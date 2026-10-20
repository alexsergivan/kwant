//! Parallel Universe API. Person-only (see `auth::person_only`): a scenario points the harness
//! at a directory and starts containers on this machine, so bots never call it.
//!
//! - `GET    /api/parallel/scenarios` → `{scenarios: [Scenario]}`.
//! - `POST   /api/parallel/scenarios` Scenario (no id) → 201 Scenario; 400 `{error}` when invalid.
//! - `GET    /api/parallel/scenarios/:id` → Scenario.
//! - `PUT    /api/parallel/scenarios/:id` Scenario → Scenario (keeps id and created_at).
//! - `DELETE /api/parallel/scenarios/:id` → `{deleted: true}`.
//! - `GET    /api/parallel/prices` → `{prices: [PriceEntry], stale_after_days}`.
//! - `POST   /api/parallel/estimate {plan, traffic}` → `{figures: [Figure]}` (no Docker, no network).

#![allow(clippy::result_large_err)]

use super::AppState;
use crate::parallel::{prices, Scenario, Traffic};
use axum::{
    extract::{rejection::JsonRejection, Path, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    Json,
};
use chrono::Utc;
use serde::Deserialize;
use serde_json::json;

const REFUSAL: &str = "only the person can manage Parallel Universe scenarios";

type Res = Result<Response, Response>;

fn error(status: StatusCode, message: &str) -> Response {
    (status, Json(json!({"error": message}))).into_response()
}

fn db_error(e: crate::db::DbError) -> Response {
    tracing::warn!("parallel api: database error: {e}");
    error(StatusCode::INTERNAL_SERVER_ERROR, "kwant could not read or save its scenarios")
}

async fn person(state: &AppState, headers: &HeaderMap) -> Result<(), Response> {
    super::mcp_suggest::refuse_agent_tokens(state, headers, REFUSAL).await
}

fn body<T>(b: Result<Json<T>, JsonRejection>) -> Result<T, Response> {
    b.map(|Json(v)| v).map_err(|e| error(StatusCode::BAD_REQUEST, &format!("invalid scenario: {}", e.body_text())))
}

pub async fn list(State(state): State<AppState>, headers: HeaderMap) -> Res {
    person(&state, &headers).await?;
    let scenarios = state.db.pu_scenarios().await.map_err(db_error)?;
    Ok(Json(json!({"scenarios": scenarios})).into_response())
}

pub async fn create(State(state): State<AppState>, headers: HeaderMap, b: Result<Json<Scenario>, JsonRejection>) -> Res {
    person(&state, &headers).await?;
    let mut s = body(b)?;
    s.validate().map_err(|e| error(StatusCode::BAD_REQUEST, &e))?;
    let now = Utc::now();
    s.id = uuid::Uuid::new_v4().to_string();
    s.title = s.title.trim().to_string();
    s.created_at = now;
    s.updated_at = now;
    state.db.pu_put_scenario(&s).await.map_err(db_error)?;
    Ok((StatusCode::CREATED, Json(s)).into_response())
}

async fn scenario_or_404(state: &AppState, id: &str) -> Result<Scenario, Response> {
    state.db.pu_scenario(id).await.map_err(db_error)?.ok_or_else(|| error(StatusCode::NOT_FOUND, "no such scenario"))
}

pub async fn get_one(State(state): State<AppState>, headers: HeaderMap, Path(id): Path<String>) -> Res {
    person(&state, &headers).await?;
    Ok(Json(scenario_or_404(&state, &id).await?).into_response())
}

pub async fn update(State(state): State<AppState>, headers: HeaderMap, Path(id): Path<String>, b: Result<Json<Scenario>, JsonRejection>) -> Res {
    person(&state, &headers).await?;
    let old = scenario_or_404(&state, &id).await?;
    let mut s = body(b)?;
    s.validate().map_err(|e| error(StatusCode::BAD_REQUEST, &e))?;
    s.id = old.id;
    s.title = s.title.trim().to_string();
    s.created_at = old.created_at;
    s.updated_at = Utc::now();
    state.db.pu_put_scenario(&s).await.map_err(db_error)?;
    Ok(Json(s).into_response())
}

pub async fn delete(State(state): State<AppState>, headers: HeaderMap, Path(id): Path<String>) -> Res {
    person(&state, &headers).await?;
    if !state.db.pu_delete_scenario(&id).await.map_err(db_error)? {
        return Err(error(StatusCode::NOT_FOUND, "no such scenario"));
    }
    Ok(Json(json!({"deleted": true})).into_response())
}

pub async fn list_prices(State(state): State<AppState>, headers: HeaderMap) -> Res {
    person(&state, &headers).await?;
    Ok(Json(json!({"prices": prices::table(), "stale_after_days": prices::STALE_AFTER_DAYS})).into_response())
}

#[derive(Deserialize)]
pub struct EstimateBody {
    pub plan: Option<String>,
    pub traffic: Traffic,
}

pub async fn estimate(State(state): State<AppState>, headers: HeaderMap, b: Result<Json<EstimateBody>, JsonRejection>) -> Res {
    person(&state, &headers).await?;
    let b = body(b)?;
    let t = &b.traffic;
    if t.monthly_requests > 100_000_000_000 || [t.peak_rps, t.avg_response_kb, t.avg_duration_ms].iter().any(|v| !v.is_finite() || *v < 0.0 || *v > 1e7) {
        return Err(error(StatusCode::BAD_REQUEST, "traffic numbers must be from 0 and not huge"));
    }
    if let Some(p) = &b.plan {
        if prices::find(p).is_none() {
            return Err(error(StatusCode::BAD_REQUEST, &format!("no price-table plan called {p}")));
        }
    }
    Ok(Json(json!({"figures": [prices::cost_figure(b.plan.as_deref(), t)]})).into_response())
}
