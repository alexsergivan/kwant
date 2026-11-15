//! "Continue in a new chat from here" (0.9.10): `POST /api/messages/:id/continue` starts a new
//! chat with the same bot that already holds every message of the old one up to and including
//! `:id`. The old chat doesn't change. Copied rows are the whole context: no engine resumes a
//! provider session, so the new chat's next turn reads them like any other history.

use super::delegation::{emit_thread, thread_view};
use super::AppState;
use crate::db::threads::continued_title;
use axum::{
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    Json,
};
use serde_json::json;

fn error(status: StatusCode, message: &str) -> Response {
    (status, Json(json!({ "error": message }))).into_response()
}

fn internal() -> Response {
    error(StatusCode::INTERNAL_SERVER_ERROR, "couldn't start a new chat")
}

pub const ONLY_ONE_BOT: &str = "Only a chat with one bot can be continued.";

/// `POST /api/messages/:id/continue` (no body), person-only. 201 with the new thread (as
/// `GET /api/threads/:id` shows it) plus `copied` and `truncated`. 404 for an unknown message
/// (or, with `?space=`, one in another space: the row guard), 400 for a team channel, a
/// delegation or anything else that isn't a chat with one bot, 403 for a bot's token.
pub async fn continue_chat(State(state): State<AppState>, Path(id): Path<String>, headers: HeaderMap) -> Response {
    if let Err(r) = super::mcp_suggest::refuse_agent_tokens(&state, &headers, "only the person continues a chat in a new one").await {
        return r;
    }
    let msg = match state.db.get_message(&id).await {
        Ok(Some(m)) => m,
        Ok(None) => return error(StatusCode::NOT_FOUND, "message not found"),
        Err(_) => return internal(),
    };
    let only_one_bot = || error(StatusCode::BAD_REQUEST, ONLY_ONE_BOT);
    let (kwant_id, context_id, title, team_id) = if let Some(thread_id) = &msg.thread_id {
        match state.db.get_thread(thread_id).await {
            Ok(Some(t)) if t.kind == "chat" => (t.kwant_id.clone(), t.id.clone(), continued_title(&t.title), t.team_id.clone()),
            Ok(_) => return only_one_bot(),
            Err(_) => return internal(),
        }
    } else if msg.channel_id.is_some() {
        return only_one_bot();
    } else if let Some(direct) = &msg.direct_kwant_id {
        match state.db.get_kwant(direct).await {
            Ok(Some(k)) => (k.id.clone(), k.id.clone(), continued_title(&k.name), None),
            Ok(None) => return only_one_bot(),
            Err(_) => return internal(),
        }
    } else {
        return only_one_bot();
    };
    match state.db.get_kwant(&kwant_id).await {
        Ok(Some(_)) => {}
        Ok(None) => return error(StatusCode::NOT_FOUND, "bot not found"),
        Err(_) => return internal(),
    }
    let made = match state.db.continue_chat(&kwant_id, &title, team_id.as_deref(), &context_id, &msg.id).await {
        Ok(c) => c,
        Err(e) => {
            tracing::warn!("continue chat: couldn't copy {context_id} up to {}: {e}", msg.id);
            return internal();
        }
    };
    emit_thread(&state, &made.thread);
    let mut view = thread_view(&state, &made.thread).await;
    view["copied"] = made.copied.into();
    view["truncated"] = made.truncated.into();
    (StatusCode::CREATED, Json(view)).into_response()
}
