//! The person's pocket API on the main port. Every path is person-only in `auth::guard` (bots
//! and remote callers without the person's token are refused), and a pocket's guest or host key
//! is never accepted here. While the switch is off the list is empty and the rest is 404.
//!
//! - `GET    /api/pockets`                 `{enabled, pocket_port, kits, pockets: [card]}`
//! - `POST   /api/pockets`                 `{kit, spec, owner_kwant_id?, opens_at?, closes_at?, origin?}` → 201 `{pocket: card, host_key}`
//!   (pa7: a future `spec.when` sets the times not given; `origin` `{kwant_id, channel_id?, thread_id?}`)
//! - `GET    /api/pockets/:code`           `{pocket: card, summary, removed, host_view, join}`: the kit's
//!   host view (counts and ticks, never a match or a guest's words) and where guests open it
//! - `POST   /api/pockets/:code/action`    `{name, body}` → the same as `GET` (a host action, pa10a)
//! - `POST   /api/pockets/:code/remove`    `{player_id}` → the same as `GET`
//! - `POST   /api/pockets/:code/reset-pin` `{player_id}` → `{code, expires_at}` (shown once)
//! - `POST   /api/pockets/:code/end`       `{pocket: card}`
//! - `POST   /api/pockets/:code/when`      `{when: RFC 3339 | null}` → `{pocket: card}` (pa7: moves the
//!   times that came from `when` and re-plans the reminder; open pockets only)
//! - `POST   /api/pockets/:code/host-key`  `{host_key}` (the old one and every host link stop
//!   working; screen links stay)
//! - `POST   /api/pockets/:code/link`      `{role: "host"|"screen"}` → `{token, role, expires_at}`:
//!   a one-time sign-in for the remote or the big screen (pa1b, Pocket API v1.1). Revokes nothing
//! - `POST   /api/pockets/:code/tunnel`    `{on: true|false}` → the same as `GET`: join from anywhere
//!   through a quick tunnel (pa6, Pocket API v1.2). 409 `no_quick_tunnel` for a long-lived kit,
//!   `pocket_ended`, `guest_port_closed`. It starts in the background: `join.tunnel.state`
//!   goes `starting` → `on` or `failed`
//! - `DELETE /api/pockets/:code`           204, final

use super::kits::Viewer;
use super::{keys, PResult, PocketError, Snapshot};
use crate::server::AppState;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde_json::{json, Value};

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/api/pockets", get(list).post(create))
        .route("/api/pockets/:code", get(read).delete(delete))
        .route("/api/pockets/:code/end", post(end))
        .route("/api/pockets/:code/when", post(set_when))
        .route("/api/pockets/:code/host-key", post(host_key))
        .route("/api/pockets/:code/link", post(link))
        .route("/api/pockets/:code/tunnel", post(tunnel))
        .route("/api/pockets/:code/action", post(action))
        .route("/api/pockets/:code/remove", post(remove))
        .route("/api/pockets/:code/reset-pin", post(reset_pin))
}

fn off() -> PocketError {
    PocketError::new(StatusCode::NOT_FOUND, "pocket_apps_off", "Pocket apps are off. Turn them on in Settings first.")
}

async fn on(state: &AppState) -> PResult<()> {
    if super::enabled(&state.db).await {
        Ok(())
    } else {
        Err(off())
    }
}

/// A pocket as the person's list shows it: public facts and counts, never names or guest text.
pub fn card(s: &Snapshot) -> Value {
    let mut c = super::pocket_json(s);
    c["created_at"] = json!(s.row.created_at);
    c["owner_kwant_id"] = json!(s.row.owner_kwant_id);
    c["path"] = json!(format!("/p/{}", s.row.code));
    c
}

async fn list(State(state): State<AppState>) -> PResult<Json<Value>> {
    let enabled = super::enabled(&state.db).await;
    let mut pockets = Vec::new();
    if enabled {
        for row in state.db.pocket_list().await? {
            if let Some(s) = super::snapshot(&state, &row.code).await? {
                pockets.push(card(&s));
            }
        }
    }
    Ok(Json(json!({
        "enabled": enabled,
        "pocket_port": if enabled { super::pocket_port(&state.config) } else { None },
        "kits": if enabled { super::kits::kit_ids() } else { Vec::new() },
        "pockets": pockets,
    })))
}

async fn create(State(state): State<AppState>, Json(req): Json<super::CreateRequest>) -> PResult<Response> {
    on(&state).await?;
    let (s, host_key) = super::create(&state, req).await?;
    Ok((StatusCode::CREATED, Json(json!({ "pocket": card(&s), "host_key": host_key }))).into_response())
}

async fn found(state: &AppState, code: &str) -> PResult<std::sync::Arc<Snapshot>> {
    on(state).await?;
    if !keys::valid_code(code) {
        return Err(PocketError::not_found());
    }
    super::snapshot(state, code).await?.ok_or_else(PocketError::not_found)
}

async fn read(State(state): State<AppState>, Path(code): Path<String>) -> PResult<Json<Value>> {
    let s = found(&state, &code).await?;
    read_json(&state, &s).await.map(Json)
}

async fn read_json(state: &AppState, s: &Snapshot) -> PResult<Value> {
    let code = &s.row.code;
    let counts = state.db.pocket_counts(code).await?;
    let now = chrono::Utc::now();
    let ctx = super::kits::KitCtx::new(&s.spec, &s.players, now);
    let summary = s.kit().map(|k| k.host_summary(&ctx, &s.kit_state));
    let host_view = s.kit().map(|k| k.view(&ctx, &s.kit_state, &Viewer::Host));
    let join = super::address::join(state, s).await;
    Ok(json!({ "pocket": card(s), "summary": summary, "removed": counts.removed, "host_view": host_view, "join": join }))
}

/// A host action from the person's card (the same names the host remote sends).
async fn action(State(state): State<AppState>, Path(code): Path<String>, Json(req): Json<super::guest::ActionRequest>) -> PResult<Json<Value>> {
    found(&state, &code).await?;
    super::act(&state, &code, &Viewer::Host, &req.name, &req.body).await?;
    let s = found(&state, &code).await?;
    read_json(&state, &s).await.map(Json)
}

async fn remove(State(state): State<AppState>, Path(code): Path<String>, Json(req): Json<super::guest::PlayerRef>) -> PResult<Json<Value>> {
    found(&state, &code).await?;
    super::remove_player(&state, &code, &req.player_id).await?;
    let s = found(&state, &code).await?;
    read_json(&state, &s).await.map(Json)
}

/// The one-time reset code, shown once. The answer names nothing else about the player.
async fn reset_pin(State(state): State<AppState>, Path(code): Path<String>, Json(req): Json<super::guest::PlayerRef>) -> PResult<Json<Value>> {
    found(&state, &code).await?;
    let (reset, expires_at) = super::reset_pin(&state, &code, &req.player_id).await?;
    Ok(Json(json!({ "code": reset, "expires_at": expires_at })))
}

async fn end(State(state): State<AppState>, Path(code): Path<String>) -> PResult<Json<Value>> {
    found(&state, &code).await?;
    let s = super::end(&state, &code).await?;
    Ok(Json(json!({ "pocket": card(&s) })))
}

#[derive(serde::Deserialize)]
struct WhenRequest {
    when: Option<String>,
}

async fn set_when(State(state): State<AppState>, Path(code): Path<String>, Json(req): Json<WhenRequest>) -> PResult<Json<Value>> {
    found(&state, &code).await?;
    let when = match req.when.as_deref().map(str::trim).filter(|w| !w.is_empty()) {
        Some(w) => Some(chrono::DateTime::parse_from_rfc3339(w).map_err(|_| PocketError::new(StatusCode::BAD_REQUEST, "bad_when", "when must be a date and time with its offset, like 2026-11-27T20:00:00+02:00."))?),
        None => None,
    };
    let s = super::schedule::set_when(&state, &code, when).await?;
    Ok(Json(json!({ "pocket": card(&s) })))
}

async fn host_key(State(state): State<AppState>, Path(code): Path<String>) -> PResult<Json<Value>> {
    found(&state, &code).await?;
    Ok(Json(json!({ "host_key": super::rotate_host_key(&state, &code).await? })))
}

#[derive(Debug, serde::Deserialize)]
pub struct LinkRequest {
    #[serde(default)]
    pub role: String,
}

/// A one-time, 10-minute sign-in token for a remote (`host`) or a big screen (`screen`). The
/// app puts it in the link's fragment (`/p/<code>#remote=<token>` or `#screen=<token>`).
async fn link(State(state): State<AppState>, Path(code): Path<String>, Json(req): Json<LinkRequest>) -> PResult<Json<Value>> {
    found(&state, &code).await?;
    let role = super::HostRole::parse(req.role.trim())
        .ok_or_else(|| PocketError::new(StatusCode::BAD_REQUEST, "bad_role", "A link is for \"host\" (the remote) or \"screen\" (the big screen)."))?;
    let (token, expires_at) = super::mint_link(&state, &code, role, chrono::Utc::now()).await?;
    Ok(Json(json!({ "token": token, "role": role.as_str(), "expires_at": expires_at })))
}

#[derive(Debug, serde::Deserialize)]
pub struct TunnelRequest {
    pub on: bool,
}

/// Join from anywhere on or off for this pocket (pa6).
async fn tunnel(State(state): State<AppState>, Path(code): Path<String>, Json(req): Json<TunnelRequest>) -> PResult<Json<Value>> {
    found(&state, &code).await?;
    super::set_tunnel(&state, &code, req.on).await?;
    let s = found(&state, &code).await?;
    read_json(&state, &s).await.map(Json)
}

async fn delete(State(state): State<AppState>, Path(code): Path<String>) -> PResult<StatusCode> {
    found(&state, &code).await?;
    super::delete(&state, &code).await?;
    Ok(StatusCode::NO_CONTENT)
}
