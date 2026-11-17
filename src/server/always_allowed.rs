//! The person's list of "always allow" answers for one bot (25e): which tool runs without a card
//! in which conversation, and taking one or all of them back. Person-only (`auth::person_only`):
//! a bot's turn token, the chat API's keys and bots' own tools can't reach either route.
//!
//! - `GET /api/kwants/:id/always-allowed` → `[{tool_name, conversation, conversation_kind,
//!   conversation_label, created_at}]`, newest first.
//! - `DELETE /api/kwants/:id/always-allowed` with `{conversation, tool_name}` takes back that one,
//!   with `{}` all of them → `{revoked: <count>, always_allowed: [...what is left]}`. The very
//!   next call asks again, no restart needed.

use super::AppState;
use axum::{
    body::Bytes,
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    Json,
};
use serde::Deserialize;
use serde_json::{json, Value};

const REFUSAL: &str = "only the person sees and takes back \"always allow\" answers";

fn error(status: StatusCode, message: &str) -> Response {
    (status, Json(json!({"error": message}))).into_response()
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RevokeBody {
    conversation: Option<String>,
    tool_name: Option<String>,
}

#[allow(clippy::result_large_err)]
async fn known_bot(state: &AppState, id: &str) -> Result<(), Response> {
    match state.db.space_of_kwant(id).await {
        Ok(Some(_)) => Ok(()),
        Ok(None) => Err(error(StatusCode::NOT_FOUND, "no such bot")),
        Err(e) => {
            tracing::warn!(kwant = %id, error = %e, "could not look up a bot for its \"always allow\" answers");
            Err(StatusCode::INTERNAL_SERVER_ERROR.into_response())
        }
    }
}

#[allow(clippy::result_large_err)]
async fn listed(state: &AppState, id: &str) -> Result<Value, Response> {
    match state.db.list_always_allowed(id).await {
        Ok(rows) => Ok(json!(rows)),
        Err(e) => {
            tracing::warn!(kwant = %id, error = %e, "could not list a bot's \"always allow\" answers");
            Err(StatusCode::INTERNAL_SERVER_ERROR.into_response())
        }
    }
}

pub async fn list(State(state): State<AppState>, headers: HeaderMap, Path(id): Path<String>) -> Response {
    if let Err(r) = super::mcp_suggest::refuse_agent_tokens(&state, &headers, REFUSAL).await {
        return r;
    }
    if let Err(r) = known_bot(&state, &id).await {
        return r;
    }
    // Answers stored before this start are in memory too from here on
    state.broker.load_saved().await;
    match listed(&state, &id).await {
        Ok(v) => Json(v).into_response(),
        Err(r) => r,
    }
}

pub async fn revoke(State(state): State<AppState>, headers: HeaderMap, Path(id): Path<String>, body: Bytes) -> Response {
    if let Err(r) = super::mcp_suggest::refuse_agent_tokens(&state, &headers, REFUSAL).await {
        return r;
    }
    // An empty body is not "all of them": the caller says so with `{}`
    let body: RevokeBody = match serde_json::from_slice(&body) {
        Ok(b) => b,
        Err(_) => return error(StatusCode::BAD_REQUEST, "send {\"conversation\", \"tool_name\"} for one, or {} for all of them"),
    };
    if let Err(r) = known_bot(&state, &id).await {
        return r;
    }
    let revoked = match (body.conversation.as_deref(), body.tool_name.as_deref()) {
        (Some(conversation), Some(tool_name)) => state.broker.revoke(&id, conversation, tool_name).await.map(u64::from),
        (None, None) => state.broker.revoke_all(&id).await,
        _ => return error(StatusCode::BAD_REQUEST, "conversation and tool_name go together; send {} for all of them"),
    };
    let revoked = match revoked {
        Ok(n) => n,
        Err(e) => {
            // Taken out of memory already, so it asks again until a restart; say it may come back
            tracing::warn!(kwant = %id, error = %e, "could not remove a stored \"always allow\"");
            return error(StatusCode::INTERNAL_SERVER_ERROR, "it asks again now, but couldn't be removed for good: try again");
        }
    };
    tracing::info!(kwant = %id, revoked, one = body.tool_name.is_some(), "the person took back \"always allow\" answers");
    match listed(&state, &id).await {
        Ok(left) => Json(json!({"revoked": revoked, "always_allowed": left})).into_response(),
        Err(r) => r,
    }
}
