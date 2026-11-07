//! The brief's person-only routes, under Dev mode's guard (bot turn tokens are refused):
//!
//! - `GET  /api/spaces/:id/dev/tickets/:key/brief`: the brief, its questions and the card's state;
//! - `POST /api/spaces/:id/dev/tickets/:key/defaults`: Go with the defaults (J8);
//! - `GET  /api/spaces/:id/dev/tickets/:key/ask_jira`: the exact comment Ask on Jira would post;
//! - `POST /api/spaces/:id/dev/tickets/:key/ask_jira`: posts it, once per brief (J9);
//! - `POST /api/spaces/:id/dev/tickets/:key/ok`: the person's OK on a Brief ready card (J10).

use super::brief::{self, render, AskedOnJira, Brief};
use super::mission;
use super::hooks;
use super::seam;
use super::valid_key;
use crate::db::dev_ticket_briefs::{BriefRow, TicketQuestion};
use crate::server::dev::routes::{coded, guard, internal};
use crate::server::AppState;
use axum::{
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    Json,
};
use serde_json::json;

/// The longest name kept in the comment's greeting.
const MAX_NAME_CHARS: usize = 80;

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
    let brief: Option<Brief> = r.brief.as_deref().and_then(|j| serde_json::from_str(j).ok());
    let questions = state.db.ticket_questions(&r.space_id, &r.key).await.unwrap_or_default();
    let questions: Vec<_> = questions
        .iter()
        .map(|q| json!({"question_id": q.question_id, "idx": q.idx, "default": q.default_answer, "answer": q.answer, "answered_by": q.answered_by}))
        .collect();
    Json(json!({
        "key": r.key,
        "url": r.url,
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
        "questions": questions,
    }))
    .into_response()
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

/// The Ask on Jira comment (design.md 05): a greeting to the reporter, the open questions with
/// their choices, and how to answer. Plain text; the writer renders it for Cloud or DC.
pub fn ask_text(key: &str, reporter: Option<&str>, team: &str, questions: &[(String, Vec<String>)]) -> String {
    let clean = |s: &str, max: usize| -> String { s.split_whitespace().collect::<Vec<_>>().join(" ").chars().filter(|c| !c.is_control()).take(max).collect() };
    let hi = match reporter.map(|r| clean(r, MAX_NAME_CHARS)).filter(|r| !r.is_empty()) {
        Some(name) => format!("Hi {name}, "),
        None => "Hi, ".to_string(),
    };
    let n = questions.len();
    let mut out = format!("{hi}before we start {key} we have {} question{}:\n", n, if n == 1 { "" } else { "s" });
    for (i, (q, choices)) in questions.iter().enumerate() {
        out.push_str(&format!("\n{}. {}\n", i + 1, clean(q, 500)));
        if !choices.is_empty() {
            out.push_str(&format!("   Suggested: {}\n", choices.iter().map(|c| clean(c, 120)).collect::<Vec<_>>().join(" · ")));
        }
    }
    out.push_str("\nWe'll wait for your reply before building. Reply to this comment to answer.\n");
    out.push_str(&format!("— kwant, for the {} team", clean(team, MAX_NAME_CHARS)));
    out
}

/// The Ask on Jira comment and who it's for (the reporter, or else the assignee).
#[allow(clippy::result_large_err)]
async fn ask_comment(state: &AppState, r: &BriefRow, open: &[TicketQuestion]) -> Result<(String, Option<String>), Response> {
    let Some(site) = seam::site(&r.space_id, &r.site_id) else {
        return Err(coded(StatusCode::CONFLICT, "jira_off", "Jira isn't connected in this space"));
    };
    let brief: Brief = r.brief.as_deref().and_then(|j| serde_json::from_str(j).ok()).unwrap_or_default();
    let questions: Vec<(String, Vec<String>)> = open
        .iter()
        .filter_map(|tq| brief.questions.iter().find(|q| q.card_id.as_deref() == Some(&tq.question_id)))
        .map(|q| (q.text.clone(), q.choices.clone()))
        .collect();
    if questions.is_empty() {
        return Err(coded(StatusCode::CONFLICT, "not_waiting", "this brief has no open question"));
    }
    let reporter = site.source.issue(&r.key).await.ok().and_then(|i| i.reporter.or(i.assignee)).map(|p| p.display_name);
    let team = state.db.get_team(&r.team_id).await.ok().flatten().map(|t| t.name).unwrap_or_else(|| "team".into());
    Ok((ask_text(&r.key, reporter.as_deref(), &team, &questions), reporter))
}

#[allow(clippy::result_large_err)]
async fn ask_ready(state: &AppState, headers: &HeaderMap, id: &str, key: &str) -> Result<(BriefRow, String, Option<String>, Vec<TicketQuestion>), Response> {
    let r = brief_row(state, headers, id, key).await?;
    if r.state != "needs_answer" {
        return Err(coded(StatusCode::CONFLICT, "not_waiting", "this brief has no open question"));
    }
    if r.asked_comment_id.is_some() {
        return Err(coded(StatusCode::CONFLICT, "asked", "the questions are already on Jira"));
    }
    let open: Vec<TicketQuestion> = state.db.ticket_questions(&r.space_id, &r.key).await.unwrap_or_default().into_iter().filter(|q| q.answer.is_none()).collect();
    let (text, to) = ask_comment(state, &r, &open).await?;
    Ok((r, text, to, open))
}

/// `GET …/tickets/:key/ask_jira`: the exact comment, for the preview.
pub async fn ask_preview(State(state): State<AppState>, headers: HeaderMap, Path((id, key)): Path<(String, String)>) -> Response {
    match ask_ready(&state, &headers, &id, &key).await {
        Ok((_, text, to, _)) => Json(json!({"text": text, "to": to})).into_response(),
        Err(resp) => resp,
    }
}

/// `POST …/tickets/:key/ask_jira`: posts the comment once; the reporter's reply after it answers.
pub async fn ask_jira(State(state): State<AppState>, headers: HeaderMap, Path((id, key)): Path<(String, String)>) -> Response {
    let (r, text, to, open) = match ask_ready(&state, &headers, &id, &key).await {
        Ok(v) => v,
        Err(resp) => return resp,
    };
    let Some(writer) = seam::site(&r.space_id, &r.site_id).and_then(|s| s.writer) else {
        return coded(StatusCode::CONFLICT, "no_writer", "kwant can't comment on this Jira site");
    };
    // Claim the one comment before posting, so a double tap can't post twice
    if !state.db.claim_ticket_ask(&r.space_id, &r.key).await.unwrap_or(false) {
        return coded(StatusCode::CONFLICT, "asked", "the questions are already on Jira");
    }
    match writer.add_comment(&r.key, &text).await {
        Ok(c) => {
            seam::set_asked(&state, &r.space_id, &r.key, &c.id).await;
            hooks::asked_on_jira(&state, &r.space_id, &r.key, &open, AskedOnJira { at: brief::at(chrono::Utc::now()), comment_url: brief::comment_url(&r.url, &c.id), to: to.clone().unwrap_or_default() }).await;
            tracing::info!(key = %r.key, "dev: asked the brief's questions on Jira");
            Json(json!({"comment_id": c.id, "text": text, "to": to})).into_response()
        }
        Err(e) => {
            let _ = state.db.set_ticket_brief_field(&r.space_id, &r.key, "asked_comment_id", None).await;
            tracing::warn!(key = %r.key, error = %e, "dev: Ask on Jira failed");
            coded(StatusCode::BAD_GATEWAY, "jira_failed", "Jira didn't take the comment; try again")
        }
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
    fn the_comment_lists_the_questions_with_their_choices() {
        let qs = vec![
            ("Which file should the export make? The ticket says CSV, the page says XLSX.".to_string(), vec!["CSV".to_string(), "XLSX".to_string()]),
            ("Who may export orders?".to_string(), vec!["Everyone".to_string(), "Only admins".to_string()]),
        ];
        let t = ask_text("TRAIL-102", Some("Ann\nLee"), "Shop", &qs);
        assert!(t.starts_with("Hi Ann Lee, before we start TRAIL-102 we have 2 questions:\n"), "{t}");
        assert!(t.contains("\n1. Which file"));
        assert!(t.contains("   Suggested: CSV · XLSX\n"));
        assert!(t.contains("\n2. Who may export orders?\n"));
        assert!(t.ends_with("— kwant, for the Shop team"));
        let one = ask_text("TRAIL-9", None, "Shop", &qs[..1]);
        assert!(one.starts_with("Hi, before we start TRAIL-9 we have 1 question:\n"));
    }
}
