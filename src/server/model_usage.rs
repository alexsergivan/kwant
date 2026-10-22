//! `GET /api/kwants/:id/model-usage`: an API-key bot's model calls and tokens (see `crate::db::usage`).

use super::AppState;
use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use chrono::{DateTime, Utc};
use serde::Deserialize;
use serde_json::json;

/// Most rows `recent` returns.
const MAX_RECENT: i64 = 500;

#[derive(Debug, Deserialize)]
pub struct UsageQuery {
    /// How many of the newest calls to list (default 50, at most 500).
    pub limit: Option<i64>,
    /// Only totals for calls at or after this time (RFC 3339); all time when missing.
    pub since: Option<DateTime<Utc>>,
}

pub async fn get_usage(
    State(state): State<AppState>,
    Path(kwant_id): Path<String>,
    Query(q): Query<UsageQuery>,
) -> Response {
    match state.db.get_kwant(&kwant_id).await {
        Ok(Some(_)) => {}
        Ok(None) => return StatusCode::NOT_FOUND.into_response(),
        Err(_) => return StatusCode::INTERNAL_SERVER_ERROR.into_response(),
    }
    let limit = q.limit.unwrap_or(50).clamp(1, MAX_RECENT);
    let (totals, recent) = tokio::join!(state.db.usage_totals(&kwant_id, q.since), state.db.recent_usage(&kwant_id, limit));
    match (totals, recent) {
        (Ok((totals, by_purpose)), Ok(recent)) => Json(json!({
            "kwant_id": kwant_id,
            "since": q.since,
            "totals": totals,
            "by_purpose": by_purpose,
            "recent": recent,
        }))
        .into_response(),
        _ => StatusCode::INTERNAL_SERVER_ERROR.into_response(),
    }
}
