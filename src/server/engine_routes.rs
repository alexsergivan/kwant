//! `GET /api/engines` and `POST /api/engines/recheck`: whether each engine can run, why not,
//! and the models it offers.

use super::AppState;
use crate::drivers::engines::{EngineReport, ProbeEnv};
use axum::{extract::State, Json};
use serde::Serialize;

#[derive(Serialize)]
pub struct EnginesResponse {
    pub engines: Vec<EngineReport>,
}

/// The cached checks; never waits on one. Engines not checked yet show as "checking".
pub async fn list(State(state): State<AppState>) -> Json<EnginesResponse> {
    Json(EnginesResponse { engines: state.engines.snapshot() })
}

/// Checks every engine again and answers once done (each check is capped at 10s).
pub async fn recheck(State(state): State<AppState>) -> Json<EnginesResponse> {
    Json(EnginesResponse { engines: recheck_all(&state).await })
}

pub async fn recheck_all(state: &AppState) -> Vec<EngineReport> {
    let env = ProbeEnv::from_settings(&state.db).await;
    state.engines.recheck(&env).await
}

/// Checks every engine in the background, so the first turns after startup see real status.
pub fn spawn_startup_check(state: AppState) {
    tokio::spawn(async move {
        let engines = recheck_all(&state).await;
        let ready: Vec<_> = engines
            .iter()
            .filter(|e| e.status == crate::drivers::engines::EngineStatus::Ready)
            .map(|e| e.name.as_str())
            .collect();
        tracing::info!(ready = ?ready, "engine checks finished");
    });
}
