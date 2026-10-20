//! Saved website passwords for a team's, channel's or bot's browser. The person only: every
//! route here needs the API token (see `auth::person_only`) and refuses bots' turn tokens.
//!
//! - `GET    /api/sandbox/:id/passwords` → `{passwords: [{id, origin, domain, username,
//!   created_at, last_used}], pending: null | {origin, username, captured_at}}`. No passwords.
//! - `POST   /api/sandbox/:id/passwords {origin | domain, username, password}` → `{status:
//!   "saved", credential: {…metadata}}`. The password is not echoed.
//! - `POST   /api/sandbox/:id/passwords/:cid/reveal` → `{password}` (no-store). With Bitwarden
//!   the body must be `{master_password}`: the API token sits in kwnt's data folder, where a bot
//!   with approvals off can read it, so it alone never shows a password. Wrong → 401
//!   `{wrong_password: true}`; after 5 in a row the vault locks.
//! - `DELETE /api/sandbox/:id/passwords/:cid` → `{status: "deleted" | "not_found"}`.
//! - `POST   /api/sandbox/:id/passwords/pending` → saves the captured password the browser holds
//!   for confirmation → `{status: "saved", credential}`; 404 when there is none.
//! - `DELETE /api/sandbox/:id/passwords/pending` → `{status: "discarded"}`.
//!
//! The vault (all owners):
//! - `GET    /api/passwords/store` → [`store_json`].
//! - `POST   /api/passwords/bitwarden/unlock {server_url?, email?, master_password, client_id?,
//!   client_secret?, two_factor_provider?, two_factor_token?, new_device_otp?}` → the store
//!   status. The first unlock sets Bitwarden up and makes it the store. 401 `{error, needs:
//!   "two_factor", providers}` / `{needs: "new_device_otp"}` / `{wrong_password: true}`; 409
//!   another account is set up; 400 bad input; 502 Bitwarden unreachable or refusing.
//! - `POST   /api/passwords/bitwarden/lock` → the store status (locked).
//! - `DELETE /api/passwords/bitwarden {master_password}` → back to the built-in store; needs the
//!   vault unlocked. Items stay in the person's Bitwarden.
//! - `POST   /api/passwords/migrate` → `{moved, failed}`: every built-in saved password moves
//!   into the unlocked Bitwarden vault, then is deleted locally.
//!
//! While Bitwarden is locked, the per-owner routes answer 423 `{error, locked: true}` and the
//! browser fills nothing.
//!
//! Approvals off and the built-in store don't mix (audit F-10): a bot that can run commands can
//! read kwnt's data folder, and the built-in store's key and the API token sit there, so it could
//! read every built-in saved password. So, while any space or bot has approvals off:
//! - saving a password into the built-in store answers 409 `{error, needs_bitwarden: true}`;
//! - going back from Bitwarden to the built-in store answers 409 `{error, needs_bitwarden: true,
//!   local_count}` unless the built-in store is empty.
//!
//! and switching approvals off answers 409 while the built-in store holds any password (see
//! [`refuse_approvals_off`]). An install already in that state is reported as `exposed: true`.
//! `local_count` counts every file in the built-in store, those of deleted teams, channels and
//! bots included, and deleting one removes its file (audit F-11).

use super::AppState;
use crate::db::spaces::SpaceId;
use crate::sandbox::bitwarden::{UnlockError, UnlockRequest};
use crate::sandbox::credentials::{migrate_legacy, normalize_origin, CredentialMeta, CredentialStore, StoreError};
use axum::{
    extract::{Path, State},
    http::{header, HeaderMap, HeaderValue, StatusCode},
    response::{IntoResponse, Response},
    Json,
};
use serde::Deserialize;
use serde_json::{json, Value};

const REFUSAL: &str = "only the person manages saved passwords";

fn error(status: StatusCode, message: &str) -> Response {
    (status, Json(json!({"error": message}))).into_response()
}

fn unavailable(e: StoreError) -> Response {
    match e {
        StoreError::Locked => (
            StatusCode::LOCKED,
            Json(json!({"error": "the password vault is locked: unlock Bitwarden first", "locked": true})),
        )
            .into_response(),
        StoreError::Remote(m) => {
            tracing::warn!(error = %m, "Bitwarden failed");
            error(StatusCode::BAD_GATEWAY, "Bitwarden could not be reached or refused the change")
        }
        e => {
            tracing::warn!(error = %e, "saved-password store failed");
            error(StatusCode::SERVICE_UNAVAILABLE, "saved passwords are unavailable right now")
        }
    }
}

/// The team, channel or bot an id names, with its space. Ids only: names don't resolve here.
pub async fn password_owner(state: &AppState, id: &str) -> Option<(String, SpaceId)> {
    if let Ok(Some(t)) = state.db.get_team(id).await {
        return Some((t.id, SpaceId::new(&t.space_id)));
    }
    if let Ok(Some(c)) = state.db.get_channel(id).await {
        return Some((c.id, SpaceId::new(&c.space_id)));
    }
    if let Ok(Some(k)) = state.db.get_kwant(id).await {
        return Some((k.id, SpaceId::new(&k.space_id)));
    }
    None
}

#[allow(clippy::result_large_err)]
async fn owner_or_404(state: &AppState, headers: &HeaderMap, id: &str) -> Result<String, Response> {
    super::mcp_suggest::refuse_agent_tokens(state, headers, REFUSAL).await?;
    match password_owner(state, id).await {
        Some((owner, _)) => {
            migrate_owner(state, &owner).await;
            Ok(owner)
        }
        None => Err(error(StatusCode::NOT_FOUND, "no such team, channel or bot")),
    }
}

/// Moves a legacy `.saved_passwords.json` from the owner's old browser folder into the store.
async fn migrate_owner(state: &AppState, owner: &str) {
    if let Some((_, folder)) = super::routes::resolve_sandbox_target_info(state, owner).await {
        match migrate_legacy(state.credentials.as_ref(), &folder, &[owner]).await {
            Ok(0) => {}
            Ok(n) => tracing::info!(moved = n, "moved saved passwords out of a bot folder into the vault"),
            Err(e) => tracing::warn!(error = %e, "could not move a saved-passwords file into the vault"),
        }
    }
}

/// On start: every legacy `.saved_passwords.json` under a team's, channel's or bot's browser
/// folder moves into the vault (for every owner that shared the folder) and is deleted.
pub async fn migrate_all(state: &AppState) {
    let owners = all_owners(state).await;
    let mut by_folder: std::collections::BTreeMap<std::path::PathBuf, Vec<String>> = Default::default();
    for owner in owners {
        if let Some((_, folder)) = super::routes::resolve_sandbox_target_info(state, &owner).await {
            by_folder.entry(folder).or_default().push(owner);
        }
    }
    for (folder, owners) in by_folder {
        let refs: Vec<&str> = owners.iter().map(String::as_str).collect();
        match migrate_legacy(state.credentials.as_ref(), &folder, &refs).await {
            Ok(0) => {}
            Ok(n) => tracing::info!(moved = n, folder = %folder.display(), "moved saved passwords into the vault"),
            Err(e) => tracing::warn!(error = %e, folder = %folder.display(), "could not move saved passwords into the vault"),
        }
    }
}

/// Every team, channel and bot id: the owners saved passwords can belong to.
async fn all_owners(state: &AppState) -> Vec<String> {
    let Ok(spaces) = state.db.list_spaces().await else { return Vec::new() };
    let mut owners: Vec<String> = Vec::new();
    for space in spaces {
        let space = SpaceId::new(&space.id);
        owners.extend(state.db.list_teams(&space).await.unwrap_or_default().into_iter().map(|t| t.id));
        owners.extend(state.db.list_channels(&space).await.unwrap_or_default().into_iter().map(|c| c.id));
        owners.extend(state.db.list_kwants(&space).await.unwrap_or_default().into_iter().map(|k| k.id));
    }
    owners
}

fn meta_json(m: &CredentialMeta) -> Value {
    let domain = url::Url::parse(&m.origin).ok().and_then(|u| u.host_str().map(str::to_string)).unwrap_or_default();
    json!({
        "id": m.id,
        "origin": m.origin,
        "domain": domain,
        "username": m.username,
        "created_at": m.created_at,
        "last_used": m.last_used,
    })
}

/// The browser controller already open for this target, if any; listing never starts a browser.
async fn open_controller(state: &AppState, path_id: &str, owner: &str) -> Option<crate::sandbox::BrowserController> {
    let controllers = state.browser_controllers.lock().await;
    controllers.get(owner).or_else(|| controllers.get(path_id)).cloned()
}

pub async fn list(State(state): State<AppState>, headers: HeaderMap, Path(id): Path<String>) -> Response {
    let owner = match owner_or_404(&state, &headers, &id).await {
        Ok(o) => o,
        Err(r) => return r,
    };
    let list = match state.credentials.list(&owner).await {
        Ok(l) => l,
        Err(e) => return unavailable(e),
    };
    let pending = match open_controller(&state, &id, &owner).await {
        Some(c) => c.pending().await,
        None => None,
    };
    Json(json!({
        "passwords": list.iter().map(meta_json).collect::<Vec<_>>(),
        "pending": pending,
    }))
    .into_response()
}

#[derive(Deserialize)]
pub struct SaveBody {
    pub origin: Option<String>,
    pub domain: Option<String>,
    pub username: String,
    pub password: String,
}

pub async fn save(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Json(body): Json<SaveBody>,
) -> Response {
    let owner = match owner_or_404(&state, &headers, &id).await {
        Ok(o) => o,
        Err(r) => return r,
    };
    if body.password.is_empty() {
        return error(StatusCode::BAD_REQUEST, "password is empty");
    }
    if let Some(r) = refuse_local_save(&state).await {
        return r;
    }
    let source = body.origin.filter(|o| !o.trim().is_empty()).or(body.domain).unwrap_or_default();
    let Ok(origin) = normalize_origin(&source) else {
        return error(StatusCode::BAD_REQUEST, "origin must be an http(s) address like https://example.com");
    };
    match state.credentials.save(&owner, &origin, body.username.trim(), &body.password).await {
        Ok(meta) => Json(json!({"status": "saved", "credential": meta_json(&meta)})).into_response(),
        Err(e) => unavailable(e),
    }
}

#[derive(Deserialize, Default)]
pub struct MasterPassword {
    #[serde(default)]
    pub master_password: String,
}

impl Drop for MasterPassword {
    fn drop(&mut self) {
        zeroize::Zeroize::zeroize(&mut self.master_password);
    }
}

fn master_password_of(body: &[u8]) -> MasterPassword {
    serde_json::from_slice(body).unwrap_or_default()
}

/// With Bitwarden, the master password the person typed must match before a password is shown
/// or the account is forgotten. `Err` is the response to send.
#[allow(clippy::result_large_err)]
async fn check_master_password(state: &AppState, given: &MasterPassword) -> Result<(), Response> {
    let bw = state.credentials.bitwarden();
    if given.master_password.is_empty() {
        return Err(error(StatusCode::UNAUTHORIZED, "type the Bitwarden master password"));
    }
    match bw.verify_master_password(&given.master_password).await {
        Ok(true) => Ok(()),
        Ok(false) => {
            tracing::warn!("a wrong Bitwarden master password was given");
            Err((StatusCode::UNAUTHORIZED, Json(json!({"error": "wrong master password", "wrong_password": true}))).into_response())
        }
        Err(e) => Err(unavailable(e)),
    }
}

pub async fn reveal(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((id, cid)): Path<(String, String)>,
    body: axum::body::Bytes,
) -> Response {
    let owner = match owner_or_404(&state, &headers, &id).await {
        Ok(o) => o,
        Err(r) => return r,
    };
    if state.credentials.store_name() == "bitwarden" {
        if let Err(r) = check_master_password(&state, &master_password_of(&body)).await {
            return r;
        }
    }
    match state.credentials.get(&owner, &cid).await {
        Ok(Some(c)) => {
            tracing::info!(owner = %owner, credential = %cid, "the person revealed a saved password");
            let mut resp = Json(json!({"password": c.password})).into_response();
            resp.headers_mut().insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
            resp
        }
        Ok(None) => error(StatusCode::NOT_FOUND, "no such saved password"),
        Err(e) => unavailable(e),
    }
}

pub async fn remove(State(state): State<AppState>, headers: HeaderMap, Path((id, cid)): Path<(String, String)>) -> Response {
    let owner = match owner_or_404(&state, &headers, &id).await {
        Ok(o) => o,
        Err(r) => return r,
    };
    match state.credentials.delete(&owner, &cid).await {
        Ok(removed) => Json(json!({"status": if removed { "deleted" } else { "not_found" }})).into_response(),
        Err(e) => unavailable(e),
    }
}

pub async fn save_pending(State(state): State<AppState>, headers: HeaderMap, Path(id): Path<String>) -> Response {
    let owner = match owner_or_404(&state, &headers, &id).await {
        Ok(o) => o,
        Err(r) => return r,
    };
    let Some(ctrl) = open_controller(&state, &id, &owner).await else {
        return error(StatusCode::NOT_FOUND, "no password is waiting to be saved");
    };
    // Refused before it's taken, so it keeps waiting until Bitwarden is set up
    if let Some(r) = refuse_local_save(&state).await {
        return r;
    }
    let Some(p) = ctrl.take_pending().await else {
        return error(StatusCode::NOT_FOUND, "no password is waiting to be saved");
    };
    match state.credentials.save(&owner, &p.origin, &p.username, &p.password).await {
        Ok(meta) => Json(json!({"status": "saved", "credential": meta_json(&meta)})).into_response(),
        Err(e) => {
            // Locked or offline: keep it waiting so the person can save it after unlocking
            ctrl.hold_for_confirmation(p).await;
            unavailable(e)
        }
    }
}

pub async fn discard_pending(State(state): State<AppState>, headers: HeaderMap, Path(id): Path<String>) -> Response {
    let owner = match owner_or_404(&state, &headers, &id).await {
        Ok(o) => o,
        Err(r) => return r,
    };
    if let Some(ctrl) = open_controller(&state, &id, &owner).await {
        ctrl.take_pending().await;
    }
    Json(json!({"status": "discarded"})).into_response()
}

/// `{store: "local" | "bitwarden", locked, bitwarden: null | {server_url, email, login:
/// "password" | "api_key", client_id, unlocked, unlocked_at, item_count, last_sync},
/// local_count, approvals_off, exposed}`. `local_count` is how many passwords the built-in store
/// still holds (what `migrate` would move). `approvals_off`: any space or bot has approvals off.
/// `exposed`: both at once, so a bot could read those passwords; nothing changes it on its own.
async fn store_json(state: &AppState) -> Value {
    let vault = &state.credentials;
    let local_count = local_count(state).await;
    let approvals_off = approvals_off_anywhere(state).await;
    json!({
        "store": vault.store_name(),
        "locked": vault.is_locked().await,
        "bitwarden": vault.bitwarden().status().await,
        "local_count": local_count,
        "approvals_off": approvals_off,
        "exposed": approvals_off && local_count > 0,
    })
}

/// How many passwords the built-in store holds, deleted owners' files included (audit F-11). In
/// Bitwarden mode these are leftovers that were never moved, still readable with the built-in key.
pub async fn local_count(state: &AppState) -> usize {
    state.credentials.local_count().await
}

/// After a team, channel or bot is deleted: removes its file from the built-in store, so no
/// password outlives its owner unseen (audit F-11). An id that still exists is left alone.
pub async fn forget_deleted_owners(state: &AppState, ids: &[&str]) {
    if ids.is_empty() {
        return;
    }
    let live = all_owners(state).await;
    for id in ids.iter().filter(|id| !live.iter().any(|l| l == *id)) {
        if let Err(e) = state.credentials.forget_local_owner(id).await {
            tracing::warn!(error = %e, owner = %id, "could not delete a deleted owner's saved passwords");
        }
    }
}

/// Whether any space or bot has approvals off. A failed read counts as off.
pub async fn approvals_off_anywhere(state: &AppState) -> bool {
    state.db.any_approvals_off().await.unwrap_or(true)
}

/// The 409 for switching approvals off while the built-in store holds passwords: a bot with
/// approvals off could read them (audit F-10). Bitwarden mode with nothing left in the built-in
/// store is fine. `None` when approvals may go off.
pub async fn refuse_approvals_off(state: &AppState) -> Option<Response> {
    let n = local_count(state).await;
    if n == 0 {
        return None;
    }
    tracing::info!(local_count = n, "refused to switch approvals off while the built-in store holds passwords");
    Some(
        (
            StatusCode::CONFLICT,
            Json(json!({
                "error": "approvals can't be switched off while passwords are saved on this server: a bot that can run commands could read them. Move them to Bitwarden first.",
                "needs_bitwarden": true,
                "local_count": n,
            })),
        )
            .into_response(),
    )
}

/// The 409 for saving into the built-in store while approvals are off anywhere.
async fn refuse_local_save(state: &AppState) -> Option<Response> {
    if state.credentials.store_name() != "local" || !approvals_off_anywhere(state).await {
        return None;
    }
    Some(
        (
            StatusCode::CONFLICT,
            Json(json!({
                "error": "passwords can't be saved on this server while approvals are off: a bot that can run commands could read them. Use Bitwarden, or turn approvals back on.",
                "needs_bitwarden": true,
            })),
        )
            .into_response(),
    )
}

/// On start: one warning, without secrets, when approvals are off and the built-in store holds
/// passwords. Approvals are left as the person set them.
pub async fn warn_if_exposed(state: &AppState) {
    let n = local_count(state).await;
    if n > 0 && approvals_off_anywhere(state).await {
        tracing::warn!(
            local_count = n,
            "approvals are off while passwords are saved on this server: a bot that can run commands could read them. Move them to Bitwarden or turn approvals back on."
        );
    }
}

#[allow(clippy::result_large_err)]
async fn person(state: &AppState, headers: &HeaderMap) -> Result<(), Response> {
    super::mcp_suggest::refuse_agent_tokens(state, headers, REFUSAL).await
}

pub async fn store_status(State(state): State<AppState>, headers: HeaderMap) -> Response {
    if let Err(r) = person(&state, &headers).await {
        return r;
    }
    Json(store_json(&state).await).into_response()
}

pub async fn unlock_bitwarden(State(state): State<AppState>, headers: HeaderMap, body: axum::body::Bytes) -> Response {
    if let Err(r) = person(&state, &headers).await {
        return r;
    }
    let Ok(req) = serde_json::from_slice::<UnlockRequest>(&body) else {
        return error(StatusCode::BAD_REQUEST, "send {master_password} (and server_url and email the first time)");
    };
    drop(body);
    match state.credentials.bitwarden().unlock(req).await {
        Ok(_) => Json(store_json(&state).await).into_response(),
        Err(e) => {
            let message = e.to_string();
            let (status, extra) = match e {
                UnlockError::Wrong => (StatusCode::UNAUTHORIZED, json!({"wrong_password": true})),
                UnlockError::TwoFactor(providers) => (StatusCode::UNAUTHORIZED, json!({"needs": "two_factor", "providers": providers})),
                UnlockError::NewDeviceOtp => (StatusCode::UNAUTHORIZED, json!({"needs": "new_device_otp"})),
                UnlockError::OtherAccount => (StatusCode::CONFLICT, json!({})),
                UnlockError::Invalid(_) | UnlockError::Unsupported(_) => (StatusCode::BAD_REQUEST, json!({})),
                UnlockError::Unreachable(_) | UnlockError::Server(_) => (StatusCode::BAD_GATEWAY, json!({})),
            };
            tracing::warn!(error = %message, "Bitwarden unlock failed");
            let mut body = extra;
            body["error"] = json!(message);
            (status, Json(body)).into_response()
        }
    }
}

pub async fn lock_bitwarden(State(state): State<AppState>, headers: HeaderMap) -> Response {
    if let Err(r) = person(&state, &headers).await {
        return r;
    }
    state.credentials.bitwarden().lock().await;
    tracing::info!("the person locked Bitwarden");
    Json(store_json(&state).await).into_response()
}

pub async fn forget_bitwarden(State(state): State<AppState>, headers: HeaderMap, body: axum::body::Bytes) -> Response {
    if let Err(r) = person(&state, &headers).await {
        return r;
    }
    if !state.credentials.bitwarden().is_configured() {
        return Json(store_json(&state).await).into_response();
    }
    // Back to the built-in store would expose what it still holds to bots with approvals off
    let n = local_count(&state).await;
    if n > 0 && approvals_off_anywhere(&state).await {
        return (
            StatusCode::CONFLICT,
            Json(json!({
                "error": "approvals are off, so this server can't go back to its built-in password store while it still holds passwords. Turn approvals back on first.",
                "needs_bitwarden": true,
                "local_count": n,
            })),
        )
            .into_response();
    }
    if let Err(r) = check_master_password(&state, &master_password_of(&body)).await {
        return r;
    }
    if let Err(e) = state.credentials.bitwarden().forget().await {
        return unavailable(e);
    }
    Json(store_json(&state).await).into_response()
}

pub async fn migrate_to_bitwarden(State(state): State<AppState>, headers: HeaderMap) -> Response {
    if let Err(r) = person(&state, &headers).await {
        return r;
    }
    match state.credentials.move_local_to_bitwarden().await {
        Ok(done) => {
            tracing::info!(moved = done.moved, failed = done.failed, "the person moved saved passwords to Bitwarden");
            Json(json!(done)).into_response()
        }
        Err(e) => unavailable(e),
    }
}
