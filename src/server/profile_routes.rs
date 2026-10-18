//! `/api/profiles`: LLM accounts per engine. API keys are write-only: no response ever carries
//! one, only `has_key`.

use super::spaces::Scope;
use super::AppState;
use crate::db::models::{LlmProfile, LlmProfilePatch, NewLlmProfile};
use crate::db::spaces::SpaceId;
use crate::profiles::login::LoginView;
use crate::profiles::{
    clean_key, clean_name, create_private_dir, managed_home, validate_user_home, Engine,
    ProfileStatus,
};
use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::collections::HashSet;
use std::time::Duration;

/// Most profiles one engine may have.
const MAX_PER_ENGINE: usize = 20;

fn err(status: StatusCode, msg: impl Into<String>) -> Response {
    (status, Json(json!({ "error": msg.into() }))).into_response()
}

fn db_err(e: impl std::fmt::Display) -> Response {
    let text = e.to_string();
    if text.contains("UNIQUE") {
        return err(
            StatusCode::CONFLICT,
            "a profile with this name or home_dir already exists",
        );
    }
    tracing::warn!(error = %text, "profile db error");
    err(StatusCode::INTERNAL_SERVER_ERROR, "database error")
}

#[derive(Debug, Serialize)]
pub struct ProfileOut {
    pub id: String,
    pub engine: String,
    pub name: String,
    pub kind: String,
    pub enabled: bool,
    pub is_default: bool,
    pub priority: i64,
    /// The config dir a `cli_home` profile runs with (kwnt-managed when not picked).
    pub home_dir: Option<String>,
    pub has_key: bool,
    pub signed_in: Option<bool>,
    pub account_label: Option<String>,
    pub limited_until: Option<chrono::DateTime<chrono::Utc>>,
    /// "quota" or "auth" from the last refused turn.
    pub last_error: Option<String>,
    /// A Custom API account's server and last check; `None` for every other kind.
    pub custom: Option<CustomOut>,
}

/// A Custom API account's server, as the API shows it (never its key).
#[derive(Debug, Serialize)]
pub struct CustomOut {
    /// "openai-chat" or "anthropic-messages".
    pub protocol: String,
    pub base_url: String,
    /// The model entered by hand, if any.
    pub model: Option<String>,
    /// "ready", "unreachable", "key_rejected" or "error"; `None` before the first check.
    pub status: Option<crate::drivers::custom::CheckStatus>,
    /// Readable: "Connected · 12 models", or why not.
    pub detail: Option<String>,
    /// What the server listed at the last check.
    pub models: Vec<crate::drivers::engines::EngineModel>,
    pub checked_at: Option<chrono::DateTime<chrono::Utc>>,
    /// Whether turns on it can call MCP tools (OpenAI chat: yes; Anthropic Messages: not yet).
    pub supports_tools: bool,
}

/// Profiles that count as signed in through a key: their own, or for a system profile the
/// engine's Settings key or env key, as before profiles.
pub(super) async fn keyed_ids(state: &AppState, profiles: &[LlmProfile]) -> HashSet<String> {
    let mut ids = state.db.profile_ids_with_key().await.unwrap_or_default();
    for p in profiles.iter().filter(|p| p.kind == "system") {
        let Some(engine) = Engine::parse(&p.engine) else {
            continue;
        };
        let setting = state
            .db
            .get_setting(engine.key_setting())
            .await
            .ok()
            .flatten()
            .is_some_and(|k| !k.is_empty());
        let env = engine
            .credential_env()
            .first()
            .and_then(std::env::var_os)
            .is_some_and(|v| !v.is_empty());
        if setting || env {
            ids.insert(p.id.clone());
        }
    }
    ids
}

async fn views(state: &AppState, profiles: Vec<LlmProfile>) -> Vec<ProfileOut> {
    let endpoints = state.db.list_profile_endpoints().await.unwrap_or_default();
    // Stale checks of Custom API accounts run together, each bounded by its own timeout
    futures::future::join_all(
        profiles
            .iter()
            .filter(|p| p.kind == "custom_api")
            .map(|p| super::profile_turn::refresh_custom(state, p, false)),
    )
    .await;
    let own_keys = state.db.profile_ids_with_key().await.unwrap_or_default();
    let keyed = keyed_ids(state, &profiles).await;
    let states = state.db.list_profile_states().await.unwrap_or_default();
    let statuses = state.profiles.statuses(&profiles, &keyed).await;
    profiles
        .into_iter()
        .map(|p| {
            let st = states.get(&p.id);
            let status = statuses.get(&p.id).cloned().unwrap_or_default();
            let custom = endpoints.get(&p.id).map(|e| {
                let check = state.profiles.last_custom_check(&p.id);
                CustomOut {
                    supports_tools: crate::drivers::custom::Protocol::parse(&e.protocol)
                        .is_some_and(|p| p.supports_tools()),
                    protocol: e.protocol.clone(),
                    base_url: e.base_url.clone(),
                    model: e.model.clone(),
                    status: check.as_ref().map(|c| c.status),
                    detail: check.as_ref().map(|c| c.detail.clone()),
                    checked_at: check.as_ref().map(|c| c.checked_at),
                    models: check.map(|c| c.models).unwrap_or_default(),
                }
            });
            ProfileOut {
                custom,
                home_dir: state
                    .profiles
                    .home_of(&p)
                    .map(|h| h.to_string_lossy().into_owned()),
                has_key: own_keys.contains(&p.id),
                signed_in: status.signed_in,
                account_label: status.account_label,
                limited_until: st
                    .and_then(|s| s.limited_until)
                    .filter(|t| *t > chrono::Utc::now()),
                last_error: st.and_then(|s| s.last_error.clone()),
                id: p.id,
                engine: p.engine,
                name: p.name,
                kind: p.kind,
                enabled: p.enabled,
                is_default: p.is_default,
                priority: p.priority,
            }
        })
        .collect()
}

async fn view_of(state: &AppState, id: &str) -> Response {
    match state.db.get_profile(id).await {
        Ok(Some(p)) => Json(views(state, vec![p]).await.pop()).into_response(),
        Ok(None) => err(StatusCode::NOT_FOUND, "no such profile"),
        Err(e) => db_err(e),
    }
}

#[allow(clippy::result_large_err)]
async fn load(state: &AppState, id: &str) -> Result<LlmProfile, Response> {
    match state.db.get_profile(id).await {
        Ok(Some(p)) => Ok(p),
        Ok(None) => Err(err(StatusCode::NOT_FOUND, "no such profile")),
        Err(e) => Err(db_err(e)),
    }
}

/// `GET /api/profiles`
pub async fn list(State(state): State<AppState>, Scope(space): Scope) -> Response {
    match state.db.list_profiles(&space).await {
        Ok(profiles) => Json(views(&state, profiles).await).into_response(),
        Err(e) => db_err(e),
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CreateBody {
    engine: String,
    name: String,
    kind: String,
    home_dir: Option<String>,
    priority: Option<i64>,
    enabled: Option<bool>,
    is_default: Option<bool>,
    api_key: Option<String>,
    /// Custom API only: "openai-chat" or "anthropic-messages".
    protocol: Option<String>,
    /// Custom API only: the server's API root, e.g. `http://localhost:23333/api/openai/v1`.
    base_url: Option<String>,
    /// Custom API only: a model id, for servers that don't list theirs.
    model: Option<String>,
}

/// A Custom API account's server from a create or patch body, validated.
#[allow(clippy::result_large_err)]
fn custom_endpoint(
    profile_id: &str,
    protocol: Option<&str>,
    base_url: Option<&str>,
    model: Option<&str>,
    current: Option<&crate::db::custom_api::ProfileEndpoint>,
) -> Result<crate::db::custom_api::ProfileEndpoint, Response> {
    use crate::drivers::custom::{clean_model, normalize_base, Protocol};
    let protocol = match protocol.or(current.map(|c| c.protocol.as_str())) {
        Some(raw) => Protocol::parse(raw).ok_or_else(|| {
            err(StatusCode::BAD_REQUEST, "protocol must be openai-chat or anthropic-messages")
        })?,
        None => return Err(err(StatusCode::BAD_REQUEST, "protocol is required")),
    };
    let base_url = match base_url.or(current.map(|c| c.base_url.as_str())) {
        Some(raw) => normalize_base(raw, protocol).map_err(|e| err(StatusCode::BAD_REQUEST, e))?,
        None => return Err(err(StatusCode::BAD_REQUEST, "base_url is required")),
    };
    let model = match model {
        Some(raw) => clean_model(raw).map_err(|e| err(StatusCode::BAD_REQUEST, e))?,
        None => current.and_then(|c| c.model.clone()),
    };
    Ok(crate::db::custom_api::ProfileEndpoint {
        profile_id: profile_id.to_string(),
        protocol: protocol.as_str().into(),
        base_url,
        model,
    })
}

/// `POST /api/profiles`
pub async fn create(State(state): State<AppState>, Scope(space): Scope, Json(body): Json<CreateBody>) -> Response {
    let Some(engine) = Engine::parse(&body.engine) else {
        return err(
            StatusCode::BAD_REQUEST,
            "engine must be claude, codex, grok or custom",
        );
    };
    let is_custom = engine == Engine::Custom;
    if is_custom != (body.kind == "custom_api") {
        return err(StatusCode::BAD_REQUEST, "engine custom goes with kind custom_api, and only with it");
    }
    if !is_custom && (body.protocol.is_some() || body.base_url.is_some() || body.model.is_some()) {
        return err(StatusCode::BAD_REQUEST, "protocol, base_url and model only go with kind custom_api");
    }
    if is_custom && !state.db.custom_api_ready().await {
        return err(
            StatusCode::SERVICE_UNAVAILABLE,
            "Custom API accounts are unavailable: the database couldn't be updated for them (see the log)",
        );
    }
    // Validated before anything is stored; the id is filled in once the profile exists
    let endpoint = if is_custom {
        match custom_endpoint("", body.protocol.as_deref(), body.base_url.as_deref(), body.model.as_deref(), None) {
            Ok(e) => Some(e),
            Err(r) => return r,
        }
    } else {
        None
    };
    let name = match clean_name(&body.name) {
        Ok(n) => n,
        Err(e) => return err(StatusCode::BAD_REQUEST, e),
    };
    let key = match (body.kind.as_str(), body.api_key.as_deref()) {
        ("api_key" | "custom_api", Some(k)) => match clean_key(k) {
            Ok(k) => Some(k),
            Err(e) => return err(StatusCode::BAD_REQUEST, e),
        },
        ("api_key" | "custom_api", None) => None,
        ("cli_home", Some(_)) => {
            return err(
                StatusCode::BAD_REQUEST,
                "api_key only goes with kind api_key",
            )
        }
        ("cli_home", None) => None,
        _ => return err(StatusCode::BAD_REQUEST, "kind must be cli_home, api_key or custom_api"),
    };
    let all = match state.db.list_profiles(&space).await {
        Ok(a) => a,
        Err(e) => return db_err(e),
    };
    // Home folders stay unique across every space, so two spaces never share a sign-in
    let mut everyone = Vec::new();
    for s in state.db.list_spaces().await.unwrap_or_default() {
        everyone.extend(state.db.list_profiles(&SpaceId::new(&s.id)).await.unwrap_or_default());
    }
    if all.iter().filter(|p| p.engine == engine.as_str()).count() >= MAX_PER_ENGINE {
        return err(StatusCode::BAD_REQUEST, "too many profiles for this engine");
    }
    let home_dir = match (body.kind.as_str(), body.home_dir.as_deref()) {
        ("cli_home", Some(raw)) => {
            let others: Vec<String> = everyone
                .iter()
                .filter_map(|p| state.profiles.home_of(p))
                .map(|h| h.to_string_lossy().into_owned())
                .collect();
            match validate_user_home(raw, engine, &others) {
                Ok(p) => Some(p.to_string_lossy().into_owned()),
                Err(e) => return err(StatusCode::BAD_REQUEST, e),
            }
        }
        ("api_key" | "custom_api", Some(_)) => {
            return err(
                StatusCode::BAD_REQUEST,
                "home_dir only goes with kind cli_home",
            )
        }
        _ => None,
    };
    let input = NewLlmProfile {
        engine: engine.as_str().into(),
        name,
        kind: body.kind.clone(),
        home_dir: home_dir.clone(),
        priority: body.priority.unwrap_or(100).clamp(0, 10_000),
        enabled: body.enabled.unwrap_or(true),
        is_default: body.is_default.unwrap_or(false),
    };
    let profile = match state.db.create_profile(&space, &input).await {
        Ok(p) => p,
        Err(e) => return db_err(e),
    };
    if body.kind == "cli_home" && home_dir.is_none() {
        if let Err(e) = create_private_dir(&managed_home(&state.profiles.data_dir, &profile.id)) {
            tracing::warn!(error = %e, "could not create a profile home");
            let _ = state.db.delete_profile(&profile.id).await;
            return err(
                StatusCode::INTERNAL_SERVER_ERROR,
                "could not create the profile's home dir",
            );
        }
    }
    if let Some(mut endpoint) = endpoint {
        endpoint.profile_id = profile.id.clone();
        if let Err(e) = state.db.set_profile_endpoint(&endpoint).await {
            let _ = state.db.delete_profile(&profile.id).await;
            return db_err(e);
        }
    }
    if let Some(key) = key {
        if let Err(e) = state.db.set_profile_key(&profile.id, &key).await {
            let _ = state.db.delete_profile(&profile.id).await;
            return db_err(e);
        }
    }
    // Checked at once, so the response already says whether the server answers
    super::profile_turn::refresh_custom(&state, &profile, true).await;
    let mut resp = view_of(&state, &profile.id).await;
    *resp.status_mut() = StatusCode::CREATED;
    resp
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PatchBody {
    name: Option<String>,
    priority: Option<i64>,
    enabled: Option<bool>,
    is_default: Option<bool>,
    /// Custom API only.
    protocol: Option<String>,
    base_url: Option<String>,
    /// Custom API only: `""` clears it.
    model: Option<String>,
}

/// `PATCH /api/profiles/:id`
pub async fn update(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(body): Json<PatchBody>,
) -> Response {
    let existing = match load(&state, &id).await {
        Ok(p) => p,
        Err(r) => return r,
    };
    let name = match body.name.as_deref().map(clean_name).transpose() {
        Ok(n) => n,
        Err(e) => return err(StatusCode::BAD_REQUEST, e),
    };
    if body.is_default == Some(false) && existing.is_default {
        return err(
            StatusCode::BAD_REQUEST,
            "make another profile the default instead",
        );
    }
    if body.enabled == Some(false) && (existing.is_default || body.is_default == Some(true)) {
        return err(
            StatusCode::BAD_REQUEST,
            "the default profile can't be disabled",
        );
    }
    let server_change = body.protocol.is_some() || body.base_url.is_some() || body.model.is_some();
    if server_change {
        if existing.kind != "custom_api" {
            return err(StatusCode::BAD_REQUEST, "protocol, base_url and model only go with kind custom_api");
        }
        let current = match state.db.get_profile_endpoint(&id).await {
            Ok(c) => c,
            Err(e) => return db_err(e),
        };
        let endpoint = match custom_endpoint(
            &id,
            body.protocol.as_deref(),
            body.base_url.as_deref(),
            body.model.as_deref(),
            current.as_ref(),
        ) {
            Ok(e) => e,
            Err(r) => return r,
        };
        if let Err(e) = state.db.set_profile_endpoint(&endpoint).await {
            return db_err(e);
        }
        let _ = state.db.clear_profile_state(&id).await;
        state.profiles.invalidate(&id);
    }
    let patch = LlmProfilePatch {
        name,
        priority: body.priority.map(|p| p.clamp(0, 10_000)),
        enabled: body.enabled,
        is_default: body.is_default,
    };
    match state.db.update_profile(&id, &patch).await {
        Ok(Some(p)) => {
            if server_change {
                super::profile_turn::refresh_custom(&state, &p, true).await;
            }
            view_of(&state, &id).await
        }
        Ok(None) => err(StatusCode::NOT_FOUND, "no such profile"),
        Err(e) => db_err(e),
    }
}

/// `DELETE /api/profiles/:id`. Leaves the home dir on disk: it may hold the user's own login.
pub async fn remove(State(state): State<AppState>, Path(id): Path<String>) -> Response {
    let existing = match load(&state, &id).await {
        Ok(p) => p,
        Err(r) => return r,
    };
    if existing.kind == "system" {
        return err(
            StatusCode::BAD_REQUEST,
            "the system profile can't be deleted",
        );
    }
    state.profiles.logins.cancel_profile(&id);
    match state.db.delete_profile(&id).await {
        Ok(_) => {
            state.profiles.invalidate(&id);
            StatusCode::NO_CONTENT.into_response()
        }
        Err(e) => db_err(e),
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct KeyBody {
    api_key: String,
}

/// `PUT /api/profiles/:id/key`
pub async fn set_key(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(body): Json<KeyBody>,
) -> Response {
    let existing = match load(&state, &id).await {
        Ok(p) => p,
        Err(r) => return r,
    };
    if existing.kind != "api_key" && existing.kind != "custom_api" {
        return err(StatusCode::BAD_REQUEST, "only api_key and custom_api profiles hold a key");
    }
    let key = match clean_key(&body.api_key) {
        Ok(k) => k,
        Err(e) => return err(StatusCode::BAD_REQUEST, e),
    };
    if let Err(e) = state.db.set_profile_key(&id, &key).await {
        return db_err(e);
    }
    let _ = state.db.clear_profile_state(&id).await;
    state.profiles.invalidate(&id);
    StatusCode::NO_CONTENT.into_response()
}

/// `POST /api/profiles/:id/check`: checks a Custom API account's server again now and returns
/// the account.
pub async fn check(State(state): State<AppState>, Path(id): Path<String>) -> Response {
    let p = match load(&state, &id).await {
        Ok(p) => p,
        Err(r) => return r,
    };
    if p.kind != "custom_api" {
        return err(StatusCode::BAD_REQUEST, "only custom_api profiles are checked this way");
    }
    super::profile_turn::refresh_custom(&state, &p, true).await;
    view_of(&state, &id).await
}

/// `DELETE /api/profiles/:id/key`
pub async fn delete_key(State(state): State<AppState>, Path(id): Path<String>) -> Response {
    if let Err(r) = load(&state, &id).await {
        return r;
    }
    match state.db.delete_profile_key(&id).await {
        Ok(_) => {
            state.profiles.invalidate(&id);
            StatusCode::NO_CONTENT.into_response()
        }
        Err(e) => db_err(e),
    }
}

#[allow(clippy::result_large_err)]
async fn cli_home_profile(
    state: &AppState,
    id: &str,
) -> Result<(LlmProfile, Engine, std::path::PathBuf), Response> {
    let p = load(state, id).await?;
    let engine =
        Engine::parse(&p.engine).ok_or_else(|| err(StatusCode::BAD_REQUEST, "unknown engine"))?;
    match state.profiles.home_of(&p) {
        Some(home) if p.kind == "cli_home" => Ok((p, engine, home)),
        _ => Err(err(
            StatusCode::BAD_REQUEST,
            "sign-in is only for cli_home profiles",
        )),
    }
}

fn login_for(view: Option<LoginView>, profile_id: &str) -> Response {
    match view {
        Some(v) if v.profile_id == profile_id => Json(v).into_response(),
        _ => err(StatusCode::NOT_FOUND, "no such sign-in"),
    }
}

/// `POST /api/profiles/:id/login`: starts the engine CLI's own sign-in with this profile's home.
pub async fn start_login(State(state): State<AppState>, Path(id): Path<String>) -> Response {
    let (p, engine, home) = match cli_home_profile(&state, &id).await {
        Ok(x) => x,
        Err(r) => return r,
    };
    let Some(bin) = crate::drivers::find_cli_binary(engine.as_str()) else {
        return err(
            StatusCode::BAD_REQUEST,
            format!("the {} CLI is not installed", engine.as_str()),
        );
    };
    if let Err(e) = create_private_dir(&home) {
        tracing::warn!(error = %e, "could not create a profile home");
        return err(
            StatusCode::INTERNAL_SERVER_ERROR,
            "could not create the profile's home dir",
        );
    }
    let db = state.db.clone();
    let manager = state.profiles.clone();
    let on_success: crate::profiles::login::OnSuccess =
        std::sync::Arc::new(move |view: LoginView| {
            let profile_id = view.profile_id;
            let db = db.clone();
            let manager = manager.clone();
            tokio::spawn(async move {
                let _ = db.clear_profile_state(&profile_id).await;
                manager.invalidate(&profile_id);
            });
        });
    match state
        .profiles
        .logins
        .start(engine, &bin, &p.id, &home, on_success)
        .await
    {
        Ok(view) => Json(view).into_response(),
        Err(e) => err(StatusCode::BAD_GATEWAY, e),
    }
}

/// `GET /api/profiles/:id/login/:flow_id`
pub async fn get_login(
    State(state): State<AppState>,
    Path((id, flow_id)): Path<(String, String)>,
) -> Response {
    login_for(state.profiles.logins.get(&flow_id), &id)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CodeBody {
    code: String,
}

/// `POST /api/profiles/:id/login/:flow_id/code`: the code Claude's sign-in page shows.
pub async fn submit_login_code(
    State(state): State<AppState>,
    Path((id, flow_id)): Path<(String, String)>,
    Json(body): Json<CodeBody>,
) -> Response {
    if state
        .profiles
        .logins
        .get(&flow_id)
        .is_none_or(|v| v.profile_id != id)
    {
        return err(StatusCode::NOT_FOUND, "no such sign-in");
    }
    match state
        .profiles
        .logins
        .submit_code(&flow_id, &body.code)
        .await
    {
        Ok(view) => Json(view).into_response(),
        Err(e) => err(StatusCode::BAD_REQUEST, e),
    }
}

/// `DELETE /api/profiles/:id/login/:flow_id`
pub async fn cancel_login(
    State(state): State<AppState>,
    Path((id, flow_id)): Path<(String, String)>,
) -> Response {
    if state
        .profiles
        .logins
        .get(&flow_id)
        .is_none_or(|v| v.profile_id != id)
    {
        return err(StatusCode::NOT_FOUND, "no such sign-in");
    }
    state.profiles.logins.cancel(&flow_id);
    StatusCode::NO_CONTENT.into_response()
}

/// `POST /api/profiles/:id/logout`: signs the profile's home out with the engine CLI.
pub async fn logout(State(state): State<AppState>, Path(id): Path<String>) -> Response {
    let (p, engine, home) = match cli_home_profile(&state, &id).await {
        Ok(x) => x,
        Err(r) => return r,
    };
    state.profiles.logins.cancel_profile(&p.id);
    let ok = match engine {
        Engine::Grok => {
            let file = home.join("auth.json");
            !file.exists() || std::fs::remove_file(file).is_ok()
        }
        _ => {
            let Some(bin) = crate::drivers::find_cli_binary(engine.as_str()) else {
                return err(
                    StatusCode::BAD_REQUEST,
                    format!("the {} CLI is not installed", engine.as_str()),
                );
            };
            let mut cmd = crate::profiles::CliEnv::for_home(engine, &home).command(bin);
            match engine {
                Engine::Claude => cmd.args(["auth", "logout"]),
                _ => cmd.arg("logout"),
            };
            cmd.stdin(std::process::Stdio::null())
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .kill_on_drop(true);
            matches!(tokio::time::timeout(Duration::from_secs(15), cmd.status()).await, Ok(Ok(s)) if s.success())
        }
    };
    state.profiles.set_status(
        &p.id,
        ProfileStatus {
            signed_in: Some(false),
            account_label: None,
        },
    );
    if ok {
        StatusCode::NO_CONTENT.into_response()
    } else {
        state.profiles.invalidate(&p.id);
        err(StatusCode::BAD_GATEWAY, "sign-out did not finish")
    }
}
