//! One-tap questions (batch 2 / C). An agent calls `ask_user` mid-turn: the question is saved as
//! a card in its chat, the UI gets `question_updated` + `message_added` and a `question_asked`
//! notification, and the tool waits for the answer while the turn stays live (and stoppable).
//! Only that turn waits: the agent's other chats and threads run as usual, since each turn has
//! its own job and slot. If nobody answers in time the tool tells the agent to wrap up, and a
//! later answer starts a new turn in the same chat with it.

use super::continuity::{notify, Notification, SelfScope};
use super::routes::{scope_for, try_kwant_turn, TurnOpts};
use super::AppState;
use crate::db::models::{MessageExtra, NewMessage};
use crate::db::questions::{clean_question, Question, QuestionError, QuestionPlace, MAX_ANSWER_CHARS, QUESTION_STATUSES};
use crate::drivers::events::RuntimeEvent;
use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use serde::Deserialize;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;
use std::time::Duration;
use tokio::sync::oneshot;

/// What a waiting `ask_user` call is woken with.
#[derive(Debug)]
pub enum Reply {
    Answer(String),
    Dismissed,
}

/// The `ask_user` calls waiting for an answer, by question id. Empty (no cost) when unused.
pub struct QuestionHub {
    waiters: Mutex<HashMap<String, oneshot::Sender<Reply>>>,
    wait_ms: AtomicU64,
}

impl Default for QuestionHub {
    fn default() -> Self {
        Self {
            waiters: Mutex::default(),
            // Shorter than the CLIs' tool timeout (approval timeout + 30 s), so the tool answers first
            wait_ms: AtomicU64::new((crate::mcp::APPROVAL_TIMEOUT_SECS - 10) * 1000),
        }
    }
}

impl QuestionHub {
    /// How long `ask_user` waits before telling the agent to wrap up (tests shorten it).
    pub fn set_wait(&self, wait: Duration) {
        self.wait_ms.store(wait.as_millis() as u64, Ordering::Relaxed);
    }

    fn wait(&self) -> Duration {
        Duration::from_millis(self.wait_ms.load(Ordering::Relaxed))
    }

    fn register(&self, id: &str) -> oneshot::Receiver<Reply> {
        let (tx, rx) = oneshot::channel();
        self.waiters.lock().unwrap().insert(id.to_string(), tx);
        rx
    }

    fn take(&self, id: &str) -> Option<oneshot::Sender<Reply>> {
        self.waiters.lock().unwrap().remove(id)
    }

    /// Whether a live turn is waiting on this question.
    pub fn is_waiting(&self, id: &str) -> bool {
        self.waiters.lock().unwrap().contains_key(id)
    }
}

/// Removes the waiter when the tool call ends any way (also when the CLI hangs up).
struct WaiterGuard<'a> {
    hub: &'a QuestionHub,
    id: String,
}

impl Drop for WaiterGuard<'_> {
    fn drop(&mut self) {
        self.hub.take(&self.id);
    }
}

fn card_meta(q: &Question) -> Value {
    json!({
        "kind": "ask_user",
        "question_id": q.id,
        "question": q.question,
        "choices": q.choice_list(),
        "status": q.status,
        "answer": q.answer,
    })
}

fn place_message(q: &Question, role: &str, sender: &str, text: String) -> (NewMessage, MessageExtra) {
    (
        NewMessage {
            channel_id: q.channel_id.clone(),
            direct_kwant_id: q.direct_kwant_id.clone(),
            sender_role: role.into(),
            sender_id: sender.into(),
            text,
            reasoning_content: None,
            tool_calls: None,
            tokens_used: 0,
        },
        MessageExtra { thread_id: q.thread_id.clone(), meta: None },
    )
}

async fn save_message(state: &AppState, q: &Question, role: &str, sender: &str, text: String, meta: Value) -> Option<String> {
    let (msg, extra) = place_message(q, role, sender, text);
    let saved = state.db.insert_message_with(&msg, &MessageExtra { meta: Some(meta), ..extra }).await.ok()?;
    if let Some(t) = &q.thread_id {
        let _ = state.db.touch_thread(t).await;
    }
    let _ = state.event_sender.send(RuntimeEvent::MessageAdded { message: serde_json::to_value(&saved).unwrap_or(Value::Null) });
    Some(saved.id)
}

/// Sends `question_updated` and refreshes the question's card in the chat.
async fn changed(state: &AppState, q: &Question) {
    if let Some(mid) = &q.message_id {
        let _ = state.db.set_message_meta(mid, &card_meta(q)).await;
    }
    let _ = state.event_sender.send(RuntimeEvent::QuestionUpdated { question: serde_json::to_value(q).unwrap_or(Value::Null) });
}

/// Closes an open question and announces it; `None` if it wasn't open any more.
async fn close(state: &AppState, id: &str, status: &str, answer: Option<&str>) -> Option<Question> {
    if !state.db.close_question(id, status, answer).await.ok()? {
        return None;
    }
    let q = state.db.get_question(id).await.ok().flatten()?;
    changed(state, &q).await;
    Some(q)
}

// Agent tool

/// When to ask: only for a step that can't be undone or reaches outside, or a choice with no
/// sensible default. Reversible choices are made, stated and carried on with.
pub const ASK_USER_DESCRIPTION: &str = "Ask the user one short question, only when you can't go on without their decision: \
    a step that can't be undone or reaches outside (sending, publishing, paying, deleting, contacting someone), or a choice \
    with no sensible default. For a reversible choice (wording, order, which of two fine options, a setting that can be \
    changed later) don't ask: pick the sensible option, state the assumption in your reply and continue. \
    Give 2 to 5 short choices; the user can also type their own answer. Your turn waits for the answer \
    (a few minutes at most) and the tool returns it. Don't ask what you can find out or decide yourself.";

pub fn tools() -> Vec<Value> {
    vec![json!({
        "name": "ask_user",
        "description": ASK_USER_DESCRIPTION,
        "inputSchema": {
            "type": "object",
            "properties": {
                "question": {"type": "string", "description": "The question (max 500 characters)"},
                "choices": {
                    "type": "array",
                    "items": {"type": "string"},
                    "minItems": 2,
                    "maxItems": 5,
                    "description": "2 to 5 short answers to pick from (max 80 characters each)",
                },
            },
            "required": ["question", "choices"],
        },
    })]
}

pub async fn tool_ask(state: &AppState, scope: &SelfScope, args: &Value) -> (String, bool) {
    let (Some(origin), Some(cancel)) = (&scope.origin, &scope.cancel) else {
        return ("You can only ask the user from a chat.".into(), true);
    };
    let choices: Vec<String> = match args["choices"].as_array() {
        Some(list) => list.iter().map(|c| c.as_str().unwrap_or("").to_string()).collect(),
        None => Vec::new(),
    };
    let (question, choices) = match clean_question(args["question"].as_str().unwrap_or(""), &choices) {
        Ok(v) => v,
        Err(e) => return (e.to_string(), true),
    };
    let place = QuestionPlace {
        kwant_id: scope.kwant_id.clone(),
        channel_id: origin.channel_id.clone(),
        direct_kwant_id: origin.direct_kwant_id.clone(),
        thread_id: origin.thread_id.clone(),
        turn_id: scope.turn_id.clone(),
    };
    // The question, its card and the waiter appear together: the waiter is registered inside the
    // transaction, so neither a list poll nor a very fast answer can see one without the others.
    // The text keeps the choices readable in history and for older clients; the card uses meta
    let hub = &state.questions;
    let mut waiter = None;
    let created = state
        .db
        .create_question_with_card(&place, &question, &choices, |q| {
            waiter = Some((q.id.clone(), hub.register(&q.id)));
            let text = format!("{}\n\nChoices: {}", q.question, choices.join(" · "));
            let (msg, extra) = place_message(q, "kwant", &scope.kwant_id, text);
            Some((msg, MessageExtra { meta: Some(card_meta(q)), ..extra }))
        })
        .await;
    let (q, card) = match created {
        Ok(v) => v,
        Err(e) => {
            if let Some((id, _)) = waiter {
                hub.take(&id);
            }
            return match e {
                QuestionError::Db => ("Could not save the question.".into(), true),
                e => (e.to_string(), true),
            };
        }
    };
    let Some((_, mut rx)) = waiter else {
        return ("Could not save the question.".into(), true);
    };
    let _guard = WaiterGuard { hub, id: q.id.clone() };
    if let Some(card) = card {
        if let Some(t) = &q.thread_id {
            let _ = state.db.touch_thread(t).await;
        }
        let _ = state.event_sender.send(RuntimeEvent::MessageAdded { message: serde_json::to_value(&card).unwrap_or(Value::Null) });
    }
    changed(state, &q).await;
    notify(
        state,
        Notification {
            kind: "question_asked",
            kwant_id: scope.kwant_id.clone(),
            kwant_name: scope.kwant_name.clone(),
            channel_id: origin.channel_id.clone(),
            title: format!("{} has a question", scope.kwant_name),
            body: "Open the chat to answer.".into(),
        },
    )
    .await;
    tracing::info!(question_id = %q.id, kwant_id = %scope.kwant_id, "agent asked the user");

    let reply = tokio::select! {
        r = &mut rx => r.ok(),
        _ = cancel.cancelled() => {
            close(state, &q.id, "cancelled", None).await;
            return ("The user stopped this turn. End it now.".into(), true);
        }
        _ = tokio::time::sleep(hub.wait()) => {
            hub.take(&q.id);
            // An answer that arrived just as the wait ran out still counts
            rx.try_recv().ok()
        }
    };
    match reply {
        Some(Reply::Answer(a)) => (format!("The user answered: {a}"), false),
        Some(Reply::Dismissed) => (
            "The user dismissed the question without answering. Carry on with your best judgment, or stop if you can't.".into(),
            false,
        ),
        None => (
            "The user hasn't answered yet. Don't ask again: end your turn now with a short note on what you're waiting for. \
            When they answer, you'll get a new turn with their answer."
                .into(),
            false,
        ),
    }
}

// Routes

#[derive(Deserialize)]
pub struct ListQuery {
    pub status: Option<String>,
    pub context_id: Option<String>,
}

fn error(status: StatusCode, message: impl Into<String>) -> Response {
    (status, Json(json!({ "error": message.into() }))).into_response()
}

/// `GET /api/questions?status=&context_id=&space=`: the space's questions, newest first (to show open cards after a reload).
pub async fn list(
    State(state): State<AppState>,
    super::spaces::Scope(space): super::spaces::Scope,
    Query(q): Query<ListQuery>,
) -> Response {
    let status = q.status.filter(|s| !s.is_empty());
    if let Some(s) = &status {
        if !QUESTION_STATUSES.contains(&s.as_str()) {
            return error(StatusCode::BAD_REQUEST, "unknown status");
        }
    }
    let context = q.context_id.as_deref().filter(|c| !c.is_empty());
    match state.db.list_questions(status.as_deref(), context, Some(&space)).await {
        Ok(list) => Json(list).into_response(),
        Err(_) => StatusCode::INTERNAL_SERVER_ERROR.into_response(),
    }
}

async fn load(state: &AppState, id: &str) -> Result<Question, StatusCode> {
    match state.db.get_question(id).await {
        Ok(Some(q)) => Ok(q),
        Ok(None) => Err(StatusCode::NOT_FOUND),
        Err(_) => Err(StatusCode::INTERNAL_SERVER_ERROR),
    }
}

pub async fn get(State(state): State<AppState>, Path(id): Path<String>) -> Response {
    match load(&state, &id).await {
        Ok(q) => Json(json!({"question": q, "waiting": state.questions.is_waiting(&q.id)})).into_response(),
        Err(r) => r.into_response(),
    }
}

#[derive(Deserialize, Default)]
pub struct AnswerPayload {
    /// Index into the question's choices.
    pub choice: Option<usize>,
    /// The user's own answer.
    pub text: Option<String>,
}

/// `POST /api/questions/:id/answer {choice} | {text}`: the waiting turn gets it at once; if the
/// turn already ended, a new turn starts in the same chat with the answer.
pub async fn answer(State(state): State<AppState>, Path(id): Path<String>, Json(p): Json<AnswerPayload>) -> Response {
    let q = match load(&state, &id).await {
        Ok(q) => q,
        Err(r) => return r.into_response(),
    };
    let text = match (p.choice, p.text.as_deref().map(str::trim)) {
        (Some(i), None) => match q.choice_list().get(i) {
            Some(c) => c.clone(),
            None => return error(StatusCode::BAD_REQUEST, "no such choice"),
        },
        (None, Some(t)) if !t.is_empty() => {
            if t.chars().count() > MAX_ANSWER_CHARS {
                return error(StatusCode::BAD_REQUEST, format!("the answer is longer than {MAX_ANSWER_CHARS} characters"));
            }
            t.to_string()
        }
        _ => return error(StatusCode::BAD_REQUEST, "give either a choice or a non-empty text"),
    };
    if q.status != "open" {
        return error(StatusCode::CONFLICT, format!("the question is already {}", q.status));
    }
    let Some((q, live)) = answer_with(&state, &id, &text).await else {
        return error(StatusCode::CONFLICT, "the question was just closed");
    };
    Json(json!({"question": q, "resumed": if live { "turn" } else { "new_turn" }})).into_response()
}

/// Answers an open question as if on its card: it closes, the answer is saved where it was
/// asked, and the turn waiting on it gets it (`true`), or a new turn continues with it (`false`;
/// in a delegation's thread that goes back to the lead). `None` when it wasn't open any more.
pub async fn answer_with(state: &AppState, id: &str, text: &str) -> Option<(Question, bool)> {
    let text: String = text.trim().chars().take(MAX_ANSWER_CHARS).collect();
    let q = close(state, id, "answered", Some(&text)).await?;
    save_message(state, &q, "user", "user", text.clone(), json!({"kind": "question_answer", "question_id": q.id})).await;
    let live = match state.questions.take(&q.id) {
        Some(tx) => tx.send(Reply::Answer(text.clone())).is_ok(),
        None => false,
    };
    if !live {
        resume(state, &q, &text).await;
    }
    Some((q, live))
}

/// The person answered an open question by typing in the chat instead of on its card (their
/// message is already saved). The question closes with that text; a turn still waiting on it gets
/// the answer and `true` comes back, so no new turn is started. `false` when no turn is waiting
/// (the caller routes the message to the bot that asked) or the question was already closed.
pub async fn answer_by_message(state: &AppState, id: &str, text: &str) -> bool {
    let text: String = text.trim().chars().take(MAX_ANSWER_CHARS).collect();
    if close(state, id, "answered", Some(&text)).await.is_none() {
        return false;
    }
    match state.questions.take(id) {
        Some(tx) => tx.send(Reply::Answer(text)).is_ok(),
        None => false,
    }
}

/// A late answer: a new turn in the question's chat, queued like any other turn. In an ended
/// delegation's thread the result also goes back to the lead (see
/// [`super::delegation::resume_after_answer`]).
async fn resume(state: &AppState, q: &Question, answer: &str) {
    let Ok(Some(kwant)) = state.db.get_kwant(&q.kwant_id).await else {
        return;
    };
    let delegation = match &q.thread_id {
        Some(t) => state.db.delegation_for_thread(t).await.ok().flatten(),
        None => None,
    };
    let prompt = format!(
        "[The user answered the question you asked earlier]\nQuestion: {}\nAnswer: {answer}\n\nContinue the task with this answer.",
        q.question
    );
    // The specialist of an ended delegation asked it: its new result goes back to the lead
    if let Some(d) = delegation.clone().filter(|d| d.specialist_kwant_id == q.kwant_id) {
        if super::delegation::resume_after_answer(state, d, &q.id, prompt.clone()).await {
            return;
        }
    }
    let depth = delegation.map(|d| d.depth).unwrap_or(0);
    let job = state.turns.start_job([q.context_id(), q.id.clone()]);
    let Ok(scope) = scope_for(state, &kwant, q.channel_id.clone(), q.thread_id.clone(), &job, depth).await else {
        return;
    };
    tokio::spawn(async move {
        let _job = job;
        let _ = try_kwant_turn(&scope, &kwant, prompt, TurnOpts::default()).await;
    });
}

/// After a turn was stopped: its questions that are still open are cancelled, so a late answer
/// doesn't restart work the user stopped. One indexed query, only for stopped turns.
pub async fn cancel_for_turn(state: &AppState, turn_id: &str) {
    let open = state.db.list_questions(Some("open"), None, None).await.unwrap_or_default();
    for q in open.iter().filter(|q| q.turn_id.as_deref() == Some(turn_id)) {
        if let Some(tx) = state.questions.take(&q.id) {
            drop(tx);
        }
        close(state, &q.id, "cancelled", None).await;
    }
}

/// `POST /api/questions/:id/cancel`: the user dismisses the question; a waiting turn carries on
/// without an answer.
pub async fn cancel(State(state): State<AppState>, Path(id): Path<String>) -> Response {
    let q = match load(&state, &id).await {
        Ok(q) => q,
        Err(r) => return r.into_response(),
    };
    if q.status != "open" {
        return error(StatusCode::CONFLICT, format!("the question is already {}", q.status));
    }
    let Some(q) = close(&state, &id, "cancelled", None).await else {
        return error(StatusCode::CONFLICT, "the question was just closed");
    };
    if let Some(tx) = state.questions.take(&q.id) {
        let _ = tx.send(Reply::Dismissed);
    }
    Json(q).into_response()
}
