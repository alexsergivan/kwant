//! Follow-ups on what a bot waits for from the person. When a turn ends waiting (an open question
//! card, or a reply whose last line asks the person something), one reminder is scheduled
//! `waiting_follow_up_hours` later (default 24, 0 = none). The person writing in that chat, or the
//! question closing, cancels it. The scheduler sends it once: a short note in the same chat and a
//! notification. A reminder isn't a turn and is never scheduled again, so it can't loop.
//! `GET /api/waiting` (person-only) lists everything waiting on the person.

use super::continuity::{notify, Notification};
use super::routes::TurnScope;
use super::AppState;
use crate::db::models::{Kwant, MessageExtra, NewMessage};
use crate::db::waiting::{NewWaitingItem, WaitingItem};
use crate::drivers::events::RuntimeEvent;
use axum::{
    body::Bytes,
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    Json,
};
use chrono::{DateTime, Duration, Utc};
use serde_json::{json, Value};
use std::collections::HashSet;

pub const HOURS_SETTING: &str = "waiting_follow_up_hours";
pub const DEFAULT_HOURS: i64 = 24;
pub const MAX_HOURS: i64 = 30 * 24;
/// Longest question quoted in a reminder.
const REMINDER_QUOTE_CHARS: usize = 200;

/// Hours before a reminder; 0 = no reminders.
pub async fn follow_up_hours(state: &AppState) -> i64 {
    state
        .db
        .get_setting(HOURS_SETTING)
        .await
        .ok()
        .flatten()
        .and_then(|v| v.trim().parse::<i64>().ok())
        .map(|h| h.clamp(0, MAX_HOURS))
        .unwrap_or(DEFAULT_HOURS)
}

fn last_line(text: &str) -> &str {
    text.lines().rev().map(str::trim).find(|l| !l.is_empty()).unwrap_or("")
}

/// A turn ended with `reply` (not stopped, not failed). Records what its bot now waits for:
/// the questions it asked in this turn that are still open, else (in a person's chat, not a
/// delegation) a reply that ends by asking the person something without handing on to a teammate.
pub(super) async fn turn_ended(scope: &TurnScope, kwant: &Kwant, turn_id: &str, reply: &str) {
    let state = &scope.state;
    // A reply finished here: what stopped in this chat before has moved on (0.9.5)
    super::stopped::recovered(state, &super::stopped::Place::of_turn(scope, &kwant.id)).await;
    let hours = follow_up_hours(state).await;
    let due_at = (hours > 0).then(|| Utc::now() + Duration::hours(hours));
    let open = state.db.list_questions(Some("open"), None, None).await.unwrap_or_default();
    let asked: Vec<_> = open.into_iter().filter(|q| q.turn_id.as_deref() == Some(turn_id)).collect();
    if !asked.is_empty() {
        for q in asked {
            let item = NewWaitingItem {
                kwant_id: q.kwant_id.clone(),
                channel_id: q.channel_id.clone(),
                direct_kwant_id: q.direct_kwant_id.clone(),
                thread_id: q.thread_id.clone(),
                question_id: Some(q.id.clone()),
                message_id: q.message_id.clone(),
                question: q.question.clone(),
                due_at,
            };
            if let Ok(Some(w)) = state.db.add_waiting(&item).await {
                opened(state, &w);
            }
        }
        state.scheduler_wake.notify_one();
        return;
    }
    if scope.delegation_depth > 0 {
        return;
    }
    let Some(question) = crate::router::sticky::person_question(reply) else {
        return;
    };
    let names: Vec<&str> = scope.teammates.iter().filter(|k| k.id != kwant.id).map(|k| k.name.as_str()).collect();
    if !crate::router::handoff_targets(last_line(reply), &names).is_empty() {
        return;
    }
    let item = NewWaitingItem {
        kwant_id: kwant.id.clone(),
        channel_id: scope.channel_id.clone(),
        direct_kwant_id: scope.kwant_id.clone(),
        thread_id: scope.thread_id.clone(),
        question_id: None,
        message_id: None,
        question: question.chars().take(crate::db::questions::MAX_QUESTION_CHARS).collect(),
        due_at,
    };
    if let Ok(Some(w)) = state.db.add_waiting(&item).await {
        opened(state, &w);
        state.scheduler_wake.notify_one();
    }
}

/// The person wrote in a chat: nothing there waits on them any more.
pub async fn person_wrote(state: &AppState, channel_id: Option<&str>, direct_kwant_id: Option<&str>, thread_id: Option<&str>) {
    let rows = state.db.close_waiting_in_chat(channel_id, direct_kwant_id, thread_id).await.unwrap_or_default();
    closed(state, &rows, REASON_PERSON_WROTE);
}

// ---- 22i: every open and close is said live, with one event ----

/// Why an item left Needs you, as `waiting_updated`'s `item.reason` says it.
pub const REASON_LATER_REPLY: &str = "later_reply";
pub const REASON_PERSON_WROTE: &str = "person_wrote";
pub const REASON_ANSWERED: &str = "answered";
pub const REASON_REPLACED: &str = "replaced";
pub const REASON_RESUMED: &str = "resumed";
pub const REASON_PLACE_GONE: &str = "place_gone";
pub const REASON_DISMISSED: &str = "dismissed";

/// A waiting item's kind as the event and `GET /api/waiting` name it.
fn kind_of(w: &WaitingItem) -> &'static str {
    match (&w.question_id, w.kind.as_deref()) {
        (Some(_), _) => "question",
        (None, Some(crate::db::waiting::STOPPED)) => "stopped",
        _ => "reply",
    }
}

/// The `item` of a `waiting_updated` event.
pub fn event_item(w: &WaitingItem, reason: Option<&str>) -> Value {
    json!({
        "id": w.id,
        "kind": kind_of(w),
        "status": if w.status == "closed" { "closed" } else { "open" },
        "reason": reason,
        "kwant_id": w.kwant_id,
        "channel_id": w.channel_id,
        "direct_kwant_id": w.direct_kwant_id,
        "thread_id": w.thread_id,
    })
}

/// A new item waits on the person: clients add it to Needs you now.
pub fn opened(state: &AppState, w: &WaitingItem) {
    let _ = state.event_sender.send(RuntimeEvent::WaitingUpdated { item: event_item(w, None) });
}

/// These items closed (`reason`): clients drop them from Needs you now, with the same event a
/// new item uses.
pub fn closed(state: &AppState, rows: &[WaitingItem], reason: &str) {
    for w in rows {
        let _ = state.event_sender.send(RuntimeEvent::WaitingUpdated { item: event_item(w, Some(reason)) });
    }
}

/// Failed or stuck items (`(kind, id)`: "delegation" or "routine_run") of `kwant_id` closed.
pub fn closed_failed(state: &AppState, kwant_id: &str, place: (Option<&str>, Option<&str>, Option<&str>), items: &[(String, String)], reason: &str) {
    let (channel_id, direct_kwant_id, thread_id) = place;
    for (kind, id) in items {
        let item = json!({
            "id": id,
            "kind": kind,
            "status": "closed",
            "reason": reason,
            "kwant_id": kwant_id,
            "channel_id": channel_id,
            "direct_kwant_id": direct_kwant_id,
            "thread_id": thread_id,
        });
        let _ = state.event_sender.send(RuntimeEvent::WaitingUpdated { item });
    }
}

/// The reminder's text: one short line quoting what the bot asked.
pub fn reminder_text(question: &str) -> String {
    let q: String = question.chars().take(REMINDER_QUOTE_CHARS).collect();
    let more = if q.chars().count() < question.chars().count() { "…" } else { "" };
    format!("Still waiting on you: “{q}{more}”. Answer here when you can.")
}

/// Sends the reminders due by `now`, each once. With reminders switched off nothing is sent and
/// what was scheduled is unscheduled. Returns how many went out.
pub async fn run_due(state: &AppState, now: DateTime<Utc>) -> usize {
    if follow_up_hours(state).await == 0 {
        let _ = state.db.clear_waiting_due().await;
        return 0;
    }
    let due = state.db.take_due_waiting(now).await.unwrap_or_default();
    let mut sent = 0;
    for w in due {
        if remind(state, &w).await {
            sent += 1;
        }
    }
    sent
}

async fn remind(state: &AppState, w: &WaitingItem) -> bool {
    // A question closed without its entry following (an older kwnt): nothing to remind of
    if let Some(qid) = &w.question_id {
        let open = state.db.get_question(qid).await.ok().flatten().is_some_and(|q| q.status == "open");
        if !open {
            let rows = state.db.close_waiting_for_question(qid).await.unwrap_or_default();
            closed(state, &rows, REASON_ANSWERED);
            return false;
        }
    }
    let Ok(Some(kwant)) = state.db.get_kwant(&w.kwant_id).await else {
        return false;
    };
    let msg = NewMessage {
        channel_id: w.channel_id.clone(),
        direct_kwant_id: w.direct_kwant_id.clone(),
        sender_role: "kwant".into(),
        sender_id: kwant.name.clone(),
        text: reminder_text(&w.question),
        reasoning_content: None,
        tool_calls: None,
        tokens_used: 0,
    };
    let meta = json!({"kind": "waiting_reminder", "waiting_id": w.id, "question_id": w.question_id, "question": w.question});
    let extra = MessageExtra { thread_id: w.thread_id.clone(), meta: Some(meta) };
    let Ok(saved) = state.db.insert_message_with(&msg, &extra).await else {
        return false;
    };
    if let Some(t) = &w.thread_id {
        let _ = state.db.touch_thread(t).await;
    }
    let _ = state.event_sender.send(RuntimeEvent::MessageAdded { message: serde_json::to_value(&saved).unwrap_or(Value::Null) });
    notify(
        state,
        Notification {
            kind: "question_asked",
            kwant_id: kwant.id.clone(),
            kwant_name: kwant.name.clone(),
            channel_id: w.channel_id.clone(),
            title: format!("{} is still waiting on you", kwant.name),
            body: "Open the chat to answer.".into(),
            thread_id: w.thread_id.clone(),
            card_id: None,
        },
    )
    .await;
    tracing::info!(waiting_id = %w.id, kwant_id = %kwant.id, "sent a waiting-on-you reminder");
    true
}

/// `GET /api/waiting?space=`: everything the space's bots wait on the person for, oldest first:
/// open question cards, replies that asked something, and open browser steps. `follow_up_at` is
/// when the one reminder goes out (`null` once sent or with reminders off).
pub async fn list(State(state): State<AppState>, super::spaces::Scope(space): super::spaces::Scope) -> Response {
    let (Ok(waiting), Ok(questions), Ok(steps), Ok(kwants)) = (
        state.db.list_waiting(Some(&space)).await,
        state.db.list_questions(Some("open"), None, Some(&space)).await,
        state.db.list_human_steps(Some("open"), None, Some(&space)).await,
        state.db.list_kwants(&space).await,
    ) else {
        return StatusCode::INTERNAL_SERVER_ERROR.into_response();
    };
    let name = |id: &str| kwants.iter().find(|k| k.id == id).map(|k| k.name.clone());
    let mut items = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();
    for w in &waiting {
        if let Some(q) = &w.question_id {
            // Its question was closed without the entry following: not waiting any more
            if !questions.iter().any(|x| &x.id == q) {
                continue;
            }
            seen.insert(q.clone());
        }
        let kind = match (&w.question_id, w.kind.as_deref()) {
            (Some(_), _) => "question",
            (None, Some(crate::db::waiting::STOPPED)) => "stopped",
            _ => "reply",
        };
        items.push((w.created_at, json!({
            "kind": kind,
            "id": w.id,
            "retry": w.retry.as_deref().and_then(|r| serde_json::from_str::<Value>(r).ok()),
            "kwant_id": w.kwant_id,
            "kwant_name": name(&w.kwant_id),
            "channel_id": w.channel_id,
            "direct_kwant_id": w.direct_kwant_id,
            "thread_id": w.thread_id,
            "question_id": w.question_id,
            "message_id": w.message_id,
            "text": w.question,
            "asked_at": w.created_at,
            "follow_up_at": if w.status == "open" { w.due_at } else { None },
            "reminded_at": w.reminded_at,
        })));
    }
    // Questions whose turn is still running (or asked before this list existed)
    for q in questions.iter().filter(|q| !seen.contains(&q.id)) {
        items.push((q.created_at, json!({
            "kind": "question",
            "kwant_id": q.kwant_id,
            "kwant_name": name(&q.kwant_id),
            "channel_id": q.channel_id,
            "direct_kwant_id": q.direct_kwant_id,
            "thread_id": q.thread_id,
            "question_id": q.id,
            "message_id": q.message_id,
            "text": q.question,
            "asked_at": q.created_at,
            "follow_up_at": null,
            "reminded_at": null,
        })));
    }
    for s in &steps {
        items.push((s.created_at, json!({
            "kind": "human_step",
            "kwant_id": s.kwant_id,
            "kwant_name": name(&s.kwant_id),
            "channel_id": s.channel_id,
            "direct_kwant_id": s.direct_kwant_id,
            "thread_id": s.thread_id,
            "human_step_id": s.id,
            "text": s.reason,
            "asked_at": s.created_at,
            "follow_up_at": null,
            "reminded_at": null,
        })));
    }
    // Fix-it items past their PR's daily turn cap (dev::fix_it)
    items.extend(super::dev::fix_it::needs_you(&state, space.as_str(), &kwants).await);
    items.extend(super::telegram::customers::needs_you(&state, space.as_str(), &kwants).await);
    // Connections only the person can fix (0.9.5): a sign-in that expired, an app to connect again
    items.extend(super::tool_failures::needs_you(&state, &kwants).await);
    items.sort_by_key(|(at, _)| *at);
    let mut items: Vec<Value> = items.into_iter().map(|(_, v)| v).collect();
    // Today's spending stop holds routines, goals, missions and AutoFix: one item, first, while it
    // does (its `id` names the day, so it is one item per day)
    // Bots paused at their monthly budget (G2): one item each, after the daily stop's
    let paused = super::bot_budget::needs_you_items(&state, &kwants).await;
    items.splice(0..0, paused);
    if let Some(stop) = super::spend_stop::needs_you_item(&state).await {
        items.insert(0, stop);
    }
    Json(json!({ "items": items, "follow_up_hours": follow_up_hours(&state).await })).into_response()
}

/// `POST /api/waiting/:id/dismiss`, optional body `{"kind": "<item kind from /api/away>"}`, the
/// person only (22i): clears one stopped, failed or stuck item from Needs attention without
/// resuming it. Records why (`attention_closed`, reason `dismissed`) and says so live with the
/// `waiting_updated` event a new item uses.
///
/// 200 `{id, kind, status: "closed", already_closed}`; 400 `bad_kind` / `bad_body`; 403
/// `person_only` for a bot's token; 404 `not_found`; 409 `use_question_answer`,
/// `use_approval_decide`, `use_step_decline` or `use_watch_decide` for an item that has its own
/// answer, which tells the bot.
pub async fn dismiss(State(state): State<AppState>, headers: HeaderMap, Path(id): Path<String>, body: Bytes) -> Response {
    if let Err(r) = super::mcp_suggest::refuse_agent_tokens(&state, &headers, "only the person can dismiss what needs their attention").await {
        return r;
    }
    #[derive(serde::Deserialize, Default)]
    struct Body {
        kind: Option<String>,
    }
    let parsed: Body = if body.iter().all(u8::is_ascii_whitespace) {
        Body::default()
    } else {
        match serde_json::from_slice(&body) {
            Ok(b) => b,
            Err(_) => return dismiss_error(StatusCode::BAD_REQUEST, "bad_body", "The body must be JSON like {\"kind\": \"stopped\"}, or empty."),
        }
    };
    let kind = parsed.kind.as_deref().map(str::trim).filter(|k| !k.is_empty());
    use crate::db::away::Dismissed;
    match state.db.dismiss_attention(&id, kind).await {
        Ok(Dismissed::Closed { kind, kwant_id, waiting }) => {
            tracing::info!(target: "audit", item_id = %id, kind = %kind, kwant_id = kwant_id.as_deref().unwrap_or(""), "the person dismissed a Needs-attention item");
            match &waiting {
                Some(w) => closed(&state, std::slice::from_ref(w.as_ref()), REASON_DISMISSED),
                None => closed_failed(&state, kwant_id.as_deref().unwrap_or(""), (None, None, None), &[(kind.clone(), id.clone())], REASON_DISMISSED),
            }
            Json(json!({"id": id, "kind": kind, "status": "closed", "already_closed": false})).into_response()
        }
        Ok(Dismissed::AlreadyClosed { kind }) => Json(json!({"id": id, "kind": kind, "status": "closed", "already_closed": true})).into_response(),
        Ok(Dismissed::NotFound) => dismiss_error(StatusCode::NOT_FOUND, "not_found", "Nothing in Needs attention has that id."),
        Ok(Dismissed::Refused(code)) => dismiss_error(StatusCode::CONFLICT, code, refused_text(code)),
        Ok(Dismissed::BadKind) => dismiss_error(StatusCode::BAD_REQUEST, "bad_kind", "kind must be stopped, reply, delegation or routine_run."),
        Err(e) => {
            tracing::warn!(error = %e, "dismiss failed");
            dismiss_error(StatusCode::INTERNAL_SERVER_ERROR, "internal", "Couldn't dismiss that just now.")
        }
    }
}

fn refused_text(code: &str) -> &'static str {
    match code {
        "use_question_answer" => "Answer or cancel the question instead, so the bot knows.",
        "use_approval_decide" => "Allow or deny the approval instead, so the bot knows.",
        "use_step_decline" => "Do or decline the browser step instead, so the bot knows.",
        _ => "Decide it in its own card instead, so the bot knows.",
    }
}

fn dismiss_error(status: StatusCode, code: &str, msg: &str) -> Response {
    (status, Json(json!({"error": msg, "code": code}))).into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reminders_are_short() {
        assert_eq!(reminder_text("Which city?"), "Still waiting on you: “Which city?”. Answer here when you can.");
        let long = reminder_text(&"x".repeat(500));
        assert!(long.contains("…") && long.chars().count() < 260);
    }
}
