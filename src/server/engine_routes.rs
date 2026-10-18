//! `GET /api/engines`, `POST /api/engines/recheck` and `POST /api/engines/{id}`: whether each
//! engine can run, why not, the models it offers, and turning one off or on.

use super::profile_turn::{self, with_accounts};
use super::spaces::Scope;
use super::AppState;
use crate::db::spaces::SpaceId;
use crate::drivers::engines::{parse_disabled, EngineId, EngineReport, ProbeEnv, DISABLED_SETTING};
use axum::{
    extract::{Path, State},
    http::StatusCode,
    Json,
};
use serde::{Deserialize, Serialize};

#[derive(Serialize)]
pub struct EnginesResponse {
    pub engines: Vec<EngineReport>,
}

/// The cached checks; never waits on one. Engines not checked yet show as "checking". Stale
/// profile sign-in checks run in the background for the next call.
pub async fn list(State(state): State<AppState>, Scope(space): Scope) -> Json<EnginesResponse> {
    let engines = with_accounts(&state, &space, state.engines.snapshot()).await;
    let bg = state.clone();
    tokio::spawn(async move { profile_turn::refresh_sign_ins(&bg).await });
    Json(EnginesResponse { engines })
}

/// Checks every engine again and answers once done (each check is capped at 10s).
pub async fn recheck(State(state): State<AppState>, Scope(space): Scope) -> Json<EnginesResponse> {
    Json(EnginesResponse { engines: recheck_all(&state, &space).await })
}

/// Every engine and every profile's sign-in, checked again; `ready` and `uses` for `space`.
pub async fn recheck_all(state: &AppState, space: &SpaceId) -> Vec<EngineReport> {
    load_disabled(state).await;
    let env = probe_env(state).await;
    let (engines, ()) = tokio::join!(state.engines.recheck(&env), profile_turn::refresh_sign_ins(state));
    with_accounts(state, space, engines).await
}

/// What the checks run with: Settings and the env, and Claude's sign-in config dir.
pub async fn probe_env(state: &AppState) -> ProbeEnv {
    state.claude_login.probe_env(ProbeEnv::from_settings(&state.db).await)
}

/// Reads which engines are off from Settings into the registry.
pub async fn load_disabled(state: &AppState) {
    let stored = state.db.get_setting(DISABLED_SETTING).await.ok().flatten();
    state.engines.set_disabled(parse_disabled(stored.as_deref()));
}

#[derive(Deserialize)]
pub struct SetEnabled {
    pub enabled: bool,
}

/// Saving one switch at a time keeps two quick toggles from overwriting each other.
static SWITCH: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

/// Turns an engine off (never checked, routed to or offered to Laya) or back on (checked at
/// once). Answers with every engine, as `GET /api/engines` does.
pub async fn set_enabled(
    State(state): State<AppState>,
    Scope(space): Scope,
    Path(id): Path<String>,
    Json(body): Json<SetEnabled>,
) -> Result<Json<EnginesResponse>, StatusCode> {
    let id = EngineId::parse(&id).ok_or(StatusCode::NOT_FOUND)?;
    {
        let _one_at_a_time = SWITCH.lock().await;
        load_disabled(&state).await;
        let off = state.engines.set_enabled(id, body.enabled);
        let saved = serde_json::to_string(&off).unwrap_or_else(|_| "[]".into());
        if state.db.set_setting(DISABLED_SETTING, &saved).await.is_err() {
            // Not saved: undo, so memory and Settings agree
            state.engines.set_enabled(id, !body.enabled);
            return Err(StatusCode::INTERNAL_SERVER_ERROR);
        }
    }
    tracing::info!(engine = id.as_str(), enabled = body.enabled, "engine switched");
    if body.enabled {
        let env = probe_env(&state).await;
        state.engines.recheck_one(id, &env).await;
    }
    Ok(Json(EnginesResponse { engines: with_accounts(&state, &space, state.engines.snapshot()).await }))
}

/// Checks every engine in the background, so the first turns after startup see real status.
pub fn spawn_startup_check(state: AppState) {
    tokio::spawn(async move {
        let engines = recheck_all(&state, &SpaceId::default_space()).await;
        let ready: Vec<_> = engines
            .iter()
            .filter(|e| e.status == crate::drivers::engines::EngineStatus::Ready)
            .map(|e| e.name.as_str())
            .collect();
        tracing::info!(ready = ?ready, "engine checks finished");
    });
}
