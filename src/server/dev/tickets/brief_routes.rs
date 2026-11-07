//! The brief's person-only routes, under Dev mode's guard (bot turn tokens are refused):
//!
//! - `GET  /api/spaces/:id/dev/tickets/:key/brief`: the brief, its questions and the card's state;
//! - `PUT  /api/spaces/:id/dev/tickets/:key/brief`: the person's edits (api-decisions 14);
//! - `POST /api/spaces/:id/dev/tickets/:key/answer`: answers one question (api-decisions 13);
//! - `POST /api/spaces/:id/dev/tickets/:key/defaults`: Go with the defaults (J8);
//! - `POST /api/spaces/:id/dev/tickets/:key/ok`: the person's OK on a Brief ready card (J10).
//!
//! Ask on Jira is Juno's `tickets/:key/ask/preview` and `tickets/:key/ask` (api-decisions 16),
//! which call [`hooks::asked_on_jira`] once the comment is posted.

use super::brief::{self, render, Brief};
use super::hooks::{self, AnswerRefused, EditRefused};
use super::links;
use super::mission;
use super::seam;
use super::valid_key;
use crate::db::dev_ticket_briefs::BriefRow;
use crate::server::dev::routes::{coded, guard, internal};
use crate::server::AppState;
use axum::{
    extract::{rejection::JsonRejection, Path, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    Json,
};
use serde::Deserialize;
use serde_json::{json, Value};

#[allow(clippy::result_large_err)]
async fn brief_row(state: &AppState, headers: &HeaderMap, id: &str, key: &str) -> Result<BriefRow, Response> {
    let space = guard(state, headers, id).await?;
    if !valid_key(key) {
        return Err(coded(StatusCode::BAD_REQUEST, "bad_key", "not a Jira key"));
    }
    match state.db.ticket_brief(space.as_str(), key).await {
        Ok(Some(r)) => Ok(r),
        Ok(None) => Err(coded(StatusCode::NOT_FOUND, "no_brief", "no brief for that ticket")),
        Err(e) => Err(internal(e)),
    }
}

/// `GET …/tickets/:key/brief`.
pub async fn get_brief(State(state): State<AppState>, headers: HeaderMap, Path((id, key)): Path<(String, String)>) -> Response {
    let r = match brief_row(&state, &headers, &id, &key).await {
        Ok(r) => r,
        Err(resp) => return resp,
    };
    let mut r = r;
    // Approved from Mission Control instead of the card: the ticket is working
    if r.state == "brief_ready" {
        if let Some(id) = &r.mission_id {
            if state.db.get_mission(id).await.ok().flatten().is_some_and(|m| m.status == "active") && seam::set_state(&state, &r.space_id, &r.key, "working").await {
                r.state = "working".into();
            }
        }
    }
    Json(brief_json(&r)).into_response()
}

/// The brief card's JSON: the row, the brief and its Markdown. The questions are in
/// `brief.questions` only (api-decisions 15), each with its `card_id` (the `question_id` to
/// answer with); the ticket link only when it is on the site (api-decisions 18).
fn brief_json(r: &BriefRow) -> Value {
    let brief: Option<Brief> = r.brief.as_deref().and_then(|j| serde_json::from_str(j).ok());
    json!({
        "key": r.key,
        "url": links::keep(&r.url, &seam::origins(&r.space_id, &r.site_id)),
        "summary": r.summary,
        "needs_you": needs_you(&r.state),
        "state": r.state,
        "thread_id": r.thread_id,
        "task_id": r.task_id,
        "mission_id": r.mission_id,
        "note": r.note,
        "asked_comment_id": r.asked_comment_id,
        "markdown": brief.as_ref().map(render),
        "brief": brief,
    })
}

/// Whether the card counts as needs-you for the Dev mode badge (J10 amended): a question is open,
/// or the brief waits for the person's OK ("Wait for my OK before building").
pub fn needs_you(state: &str) -> bool {
    matches!(state, "needs_answer" | "brief_ready")
}

/// `POST …/tickets/:key/ok`: starts the work of a Brief ready card that waits for the person.
pub async fn ok(State(state): State<AppState>, headers: HeaderMap, Path((id, key)): Path<(String, String)>) -> Response {
    let r = match brief_row(&state, &headers, &id, &key).await {
        Ok(r) => r,
        Err(resp) => return resp,
    };
    if r.state != "brief_ready" || r.mission_id.is_none() {
        return coded(StatusCode::CONFLICT, "not_waiting", "this brief isn't waiting for your OK");
    }
    if !mission::approve(&state, &r).await {
        return coded(StatusCode::CONFLICT, "not_waiting", "the work couldn't start; it may have started already");
    }
    tracing::info!(key = %r.key, "dev: the person OK'd a brief");
    Json(json!({"state": "working", "mission_id": r.mission_id})).into_response()
}

/// `POST …/tickets/:key/defaults`: every open question takes its default.
pub async fn defaults(State(state): State<AppState>, headers: HeaderMap, Path((id, key)): Path<(String, String)>) -> Response {
    let r = match brief_row(&state, &headers, &id, &key).await {
        Ok(r) => r,
        Err(resp) => return resp,
    };
    if r.state != "needs_answer" {
        return coded(StatusCode::CONFLICT, "not_waiting", "this brief has no open question");
    }
    let answered = hooks::defaults(&state, &r.space_id, &r.key).await;
    let state_now = state.db.ticket_brief(&r.space_id, &r.key).await.ok().flatten().map(|r| r.state);
    Json(json!({"answered": answered, "state": state_now})).into_response()
}

/// `POST …/tickets/:key/answer {question_id, value}` (api-decisions 13): the person answers one
/// of the brief's questions from the card. `value` is one of its choices, or text of at most
/// 2,000 characters; longer is refused, never cut. Answers with the brief as it now reads; the
/// last open question moves the ticket to Brief ready.
pub async fn answer(State(state): State<AppState>, headers: HeaderMap, Path((id, key)): Path<(String, String)>, body: Result<Json<AnswerBody>, JsonRejection>) -> Response {
    let r = match brief_row(&state, &headers, &id, &key).await {
        Ok(r) => r,
        Err(resp) => return resp,
    };
    let Ok(Json(body)) = body else {
        return coded(StatusCode::BAD_REQUEST, "bad_request", "send {question_id, value}");
    };
    let brief: Brief = r.brief.as_deref().and_then(|j| serde_json::from_str(j).ok()).unwrap_or_default();
    let Some(q) = brief.questions.iter().find(|q| q.card_id.as_deref() == Some(body.question_id.as_str())) else {
        return coded(StatusCode::NOT_FOUND, "no_question", "that isn't one of this brief's questions");
    };
    let value = match answer_value(&body.value, &q.choices) {
        Ok(v) => v,
        Err(m) => return coded(StatusCode::BAD_REQUEST, "bad_answer", &m),
    };
    match hooks::answer(&state, &r.space_id, &r.key, &body.question_id, &value).await {
        Ok(()) => {}
        Err(AnswerRefused::NoQuestion) => return coded(StatusCode::NOT_FOUND, "no_question", "that isn't one of this brief's questions"),
        Err(AnswerRefused::Answered) => return coded(StatusCode::CONFLICT, "already_answered", "that question has an answer already"),
        Err(AnswerRefused::NotWaiting) => return coded(StatusCode::CONFLICT, "not_waiting", "this brief isn't waiting for answers"),
    }
    match state.db.ticket_brief(&r.space_id, &r.key).await {
        Ok(Some(r)) => Json(brief_json(&r)).into_response(),
        Ok(None) => coded(StatusCode::NOT_FOUND, "no_brief", "no brief for that ticket"),
        Err(e) => internal(e),
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AnswerBody {
    question_id: String,
    value: String,
}

/// The answer as kept: a choice in the question's own words (case aside), or the person's text,
/// trimmed, without control characters other than line breaks.
pub fn answer_value(value: &str, choices: &[String]) -> Result<String, String> {
    let v: String = value.trim().chars().filter(|c| brief::plain(*c) || *c == '\n').collect();
    if v.trim().is_empty() {
        return Err("the answer is empty".into());
    }
    if let Some(c) = choices.iter().find(|c| c.trim().eq_ignore_ascii_case(v.trim())) {
        return Ok(c.clone());
    }
    if v.chars().count() > hooks::MAX_ANSWER_CHARS {
        return Err(format!("the answer is longer than {} characters", hooks::MAX_ANSWER_CHARS));
    }
    Ok(v)
}

/// `PUT …/tickets/:key/brief` (api-decisions 14): the person's edits to the goal, done-when,
/// where, plan or assumptions ([`brief::Edits`]). They are kept apart and win over a brief read
/// again; the card shows "Edited by you". Only while the brief waits for answers or for the OK.
pub async fn put_brief(State(state): State<AppState>, headers: HeaderMap, Path((id, key)): Path<(String, String)>, body: Result<Json<brief::Edits>, JsonRejection>) -> Response {
    let r = match brief_row(&state, &headers, &id, &key).await {
        Ok(r) => r,
        Err(resp) => return resp,
    };
    let Ok(Json(edits)) = body else {
        return coded(StatusCode::BAD_REQUEST, "bad_request", "send any of goal, done_when, where, plan and assumptions");
    };
    let edits = match edits.checked() {
        Ok(e) => e,
        Err(m) => return coded(StatusCode::BAD_REQUEST, "bad_edit", &m),
    };
    if let Err(EditRefused::NotEditable) = hooks::edit(&state, &r.space_id, &r.key, edits).await {
        return coded(StatusCode::CONFLICT, "not_editable", "the brief can be changed only while it waits for answers or for your OK");
    }
    match state.db.ticket_brief(&r.space_id, &r.key).await {
        Ok(Some(r)) => Json(brief_json(&r)).into_response(),
        Ok(None) => coded(StatusCode::NOT_FOUND, "no_brief", "no brief for that ticket"),
        Err(e) => internal(e),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn needs_you_is_an_open_question_or_a_brief_waiting_for_ok() {
        for s in ["needs_answer", "brief_ready"] {
            assert!(needs_you(s), "{s}");
        }
        for s in ["waiting", "picked_up", "reading", "working", "in_review", "landed", "failed", "taken_back", "closed"] {
            assert!(!needs_you(s), "{s}");
        }
    }

    #[test]
    fn an_answer_is_a_choice_or_text_up_to_2000_characters() {
        let choices = vec!["CSV".to_string(), "XLSX".to_string()];
        assert_eq!(answer_value("  csv ", &choices).unwrap(), "CSV", "a choice in its own words");
        assert_eq!(answer_value("Both, CSV first\u{0}", &choices).unwrap(), "Both, CSV first");
        assert_eq!(answer_value("line one\nline two", &choices).unwrap(), "line one\nline two");
        assert!(answer_value(" \n\t", &choices).is_err());
        assert!(answer_value("", &[]).is_err());
        let max = "x".repeat(2000);
        assert_eq!(answer_value(&max, &choices).unwrap(), max);
        let err = answer_value(&"x".repeat(2001), &choices).unwrap_err();
        assert!(err.contains("2000"), "refused, never cut: {err}");
    }
}
