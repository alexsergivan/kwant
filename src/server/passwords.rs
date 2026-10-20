//! Saved website passwords for a team's, channel's or bot's browser. The person only: every
//! route here needs the API token (see `auth::person_only`) and refuses bots' turn tokens.
//!
//! - `GET    /api/sandbox/:id/passwords` → `{passwords: [{id, origin, domain, username,
//!   created_at, last_used}], pending: null | {origin, username, captured_at}}`. No passwords.
//! - `POST   /api/sandbox/:id/passwords {origin | domain, username, password}` → `{status:
//!   "saved", credential: {…metadata}}`. The password is not echoed.
//! - `POST   /api/sandbox/:id/passwords/:cid/reveal` → `{password}` (no-store).
//! - `DELETE /api/sandbox/:id/passwords/:cid` → `{status: "deleted" | "not_found"}`.
//! - `POST   /api/sandbox/:id/passwords/pending` → saves the captured password the browser holds
//!   for confirmation → `{status: "saved", credential}`; 404 when there is none.
//! - `DELETE /api/sandbox/:id/passwords/pending` → `{status: "discarded"}`.

use super::AppState;
use crate::db::spaces::SpaceId;
use crate::sandbox::credentials::{migrate_legacy, normalize_origin, CredentialMeta, StoreError};
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
    tracing::warn!(error = %e, "saved-password store failed");
    error(StatusCode::SERVICE_UNAVAILABLE, "saved passwords are unavailable right now")
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
        match migrate_legacy(state.credentials.as_ref(), &folder, &[owner]) {
            Ok(0) => {}
            Ok(n) => tracing::info!(moved = n, "moved saved passwords out of a bot folder into the vault"),
            Err(e) => tracing::warn!(error = %e, "could not move a saved-passwords file into the vault"),
        }
    }
}

/// On start: every legacy `.saved_passwords.json` under a team's, channel's or bot's browser
/// folder moves into the vault (for every owner that shared the folder) and is deleted.
pub async fn migrate_all(state: &AppState) {
    let Ok(spaces) = state.db.list_spaces().await else { return };
    let mut owners: Vec<String> = Vec::new();
    for space in spaces {
        let space = SpaceId::new(&space.id);
        owners.extend(state.db.list_teams(&space).await.unwrap_or_default().into_iter().map(|t| t.id));
        owners.extend(state.db.list_channels(&space).await.unwrap_or_default().into_iter().map(|c| c.id));
        owners.extend(state.db.list_kwants(&space).await.unwrap_or_default().into_iter().map(|k| k.id));
    }
    let mut by_folder: std::collections::BTreeMap<std::path::PathBuf, Vec<String>> = Default::default();
    for owner in owners {
        if let Some((_, folder)) = super::routes::resolve_sandbox_target_info(state, &owner).await {
            by_folder.entry(folder).or_default().push(owner);
        }
    }
    for (folder, owners) in by_folder {
        let refs: Vec<&str> = owners.iter().map(String::as_str).collect();
        match migrate_legacy(state.credentials.as_ref(), &folder, &refs) {
            Ok(0) => {}
            Ok(n) => tracing::info!(moved = n, folder = %folder.display(), "moved saved passwords into the vault"),
            Err(e) => tracing::warn!(error = %e, folder = %folder.display(), "could not move saved passwords into the vault"),
        }
    }
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
    let list = match state.credentials.list(&owner) {
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
    let source = body.origin.filter(|o| !o.trim().is_empty()).or(body.domain).unwrap_or_default();
    let Ok(origin) = normalize_origin(&source) else {
        return error(StatusCode::BAD_REQUEST, "origin must be an http(s) address like https://example.com");
    };
    match state.credentials.save(&owner, &origin, body.username.trim(), &body.password) {
        Ok(meta) => Json(json!({"status": "saved", "credential": meta_json(&meta)})).into_response(),
        Err(e) => unavailable(e),
    }
}

pub async fn reveal(State(state): State<AppState>, headers: HeaderMap, Path((id, cid)): Path<(String, String)>) -> Response {
    let owner = match owner_or_404(&state, &headers, &id).await {
        Ok(o) => o,
        Err(r) => return r,
    };
    match state.credentials.get(&owner, &cid) {
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
    match state.credentials.delete(&owner, &cid) {
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
    let Some(p) = ctrl.take_pending().await else {
        return error(StatusCode::NOT_FOUND, "no password is waiting to be saved");
    };
    match state.credentials.save(&owner, &p.origin, &p.username, &p.password) {
        Ok(meta) => Json(json!({"status": "saved", "credential": meta_json(&meta)})).into_response(),
        Err(e) => unavailable(e),
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
