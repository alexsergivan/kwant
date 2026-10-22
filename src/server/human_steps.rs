//! Steps only the person can do in the bot browser: sign in, solve a CAPTCHA, or something else.
//! An agent calls `request_human_step` (or kwnt raises one itself when a browser tool lands on a
//! CAPTCHA): the step is saved as a card in its chat, the UI gets `human_step_updated` +
//! `message_added` and a `human_step` notification, the browser is held for the person, and the
//! turn waits until they press Done or Decline (`POST /api/human-steps/:id/done|decline`). If
//! nobody answers in time the tool tells the agent to wrap up, and a later answer starts a new
//! turn in the same chat with the outcome. Stopping the turn cancels the step.

use super::continuity::{notify, Notification, SelfScope};
use super::routes::{scope_for, try_kwant_turn, TurnOpts};
use super::AppState;
use crate::db::human_steps::{clean_request, HumanStep, HumanStepError, HumanStepPlace, HUMAN_STEP_STATUSES};
use crate::db::models::{MessageExtra, NewMessage};
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

pub const TOOL: &str = "request_human_step";

/// What a waiting step is woken with.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Outcome {
    Done,
    Declined,
}

impl Outcome {
    fn status(self) -> &'static str {
        match self {
            Outcome::Done => "done",
            Outcome::Declined => "declined",
        }
    }
}

/// The turns waiting on a human step, by step id. Empty (no cost) when unused.
pub struct HumanStepHub {
    waiters: Mutex<HashMap<String, oneshot::Sender<Outcome>>>,
    wait_ms: AtomicU64,
}

impl Default for HumanStepHub {
    fn default() -> Self {
        Self {
            waiters: Mutex::default(),
            // Shorter than the CLIs' tool timeout (approval timeout + 30 s), so the tool answers first
            wait_ms: AtomicU64::new((crate::mcp::APPROVAL_TIMEOUT_SECS - 10) * 1000),
        }
    }
}

impl HumanStepHub {
    /// How long a turn waits before it is told to wrap up (tests shorten it).
    pub fn set_wait(&self, wait: Duration) {
        self.wait_ms.store(wait.as_millis() as u64, Ordering::Relaxed);
    }

    fn wait(&self) -> Duration {
        Duration::from_millis(self.wait_ms.load(Ordering::Relaxed))
    }

    fn register(&self, id: &str) -> oneshot::Receiver<Outcome> {
        let (tx, rx) = oneshot::channel();
        self.waiters.lock().unwrap_or_else(|e| e.into_inner()).insert(id.to_string(), tx);
        rx
    }

    fn take(&self, id: &str) -> Option<oneshot::Sender<Outcome>> {
        self.waiters.lock().unwrap_or_else(|e| e.into_inner()).remove(id)
    }

    /// Wakes the turn waiting on this step; `false` when no live turn waits.
    pub fn wake(&self, id: &str, outcome: Outcome) -> bool {
        self.take(id).is_some_and(|tx| tx.send(outcome).is_ok())
    }

    /// Whether a live turn is waiting on this step.
    pub fn is_waiting(&self, id: &str) -> bool {
        self.waiters.lock().unwrap_or_else(|e| e.into_inner()).contains_key(id)
    }
}

/// Removes the waiter when the tool call ends any way (also when the CLI hangs up).
struct WaiterGuard<'a> {
    hub: &'a HumanStepHub,
    id: String,
}

impl Drop for WaiterGuard<'_> {
    fn drop(&mut self) {
        self.hub.take(&self.id);
    }
}

fn kind_label(kind: &str) -> &'static str {
    match kind {
        "sign_in" => "Sign in needed",
        "captcha" => "Human check needed",
        _ => "Your help is needed",
    }
}

fn card_meta(s: &HumanStep) -> Value {
    json!({
        "kind": "human_step",
        "human_step_id": s.id,
        "step_kind": s.kind,
        "reason": s.reason,
        "url": s.url,
        "sandbox_target": s.sandbox_target,
        "source": s.source,
        "status": s.status,
    })
}

fn place_message(s: &HumanStep, role: &str, sender: &str, text: String) -> (NewMessage, MessageExtra) {
    (
        NewMessage {
            channel_id: s.channel_id.clone(),
            direct_kwant_id: s.direct_kwant_id.clone(),
            sender_role: role.into(),
            sender_id: sender.into(),
            text,
            reasoning_content: None,
            tool_calls: None,
            tokens_used: 0,
        },
        MessageExtra { thread_id: s.thread_id.clone(), meta: None },
    )
}

/// Sends `human_step_updated` and refreshes the step's card in the chat.
async fn changed(state: &AppState, s: &HumanStep) {
    if let Some(mid) = &s.message_id {
        let _ = state.db.set_message_meta(mid, &card_meta(s)).await;
    }
    let _ = state.event_sender.send(RuntimeEvent::HumanStepUpdated { step: serde_json::to_value(s).unwrap_or(Value::Null) });
}

/// Holds the browser for the person, if it is running.
async fn hold_browser(state: &AppState, target: Option<&str>) {
    let Some(target) = target else { return };
    let ctrl = state.browser_controllers.lock().await.get(target).cloned();
    if let Some(ctrl) = ctrl {
        if !ctrl.hold_for_person().await {
            tracing::info!("human step: the browser is busy or already the person's; bot actions are refused until it closes");
        }
    }
}

/// Hands the browser back once no step is open in it any more.
async fn release_browser(state: &AppState, target: Option<&str>) {
    let Some(target) = target else { return };
    if state.db.human_step_open_in(target).await.unwrap_or(false) {
        return;
    }
    if let Some(ctrl) = state.browser_controllers.lock().await.get(target) {
        ctrl.release_person_hold();
    }
}

/// Closes an open step, hands the browser back and announces it; `None` if it wasn't open.
async fn close(state: &AppState, id: &str, status: &'static str) -> Option<HumanStep> {
    if !state.db.close_human_step(id, status).await.ok()? {
        return None;
    }
    let s = state.db.get_human_step(id).await.ok().flatten()?;
    release_browser(state, s.sandbox_target.as_deref()).await;
    if let Some(turn) = s.turn_id.as_deref().and_then(super::activity::TurnActivity::find) {
        turn.settle(&s.id, if status == "cancelled" { "stopped" } else { status });
    }
    changed(state, &s).await;
    Some(s)
}

/// Whether a human step holds this bot browser: bot actions wait until it closes.
pub async fn browser_is_held(state: &AppState, target: &str) -> bool {
    state.db.human_step_open_in(target).await.unwrap_or(false)
}

// Agent tool

pub const DESCRIPTION: &str = "Ask the person to do one step in the bot browser that only they can do: sign in \
    (kind sign_in), solve a CAPTCHA or human check (kind captcha), or something else that needs them (kind other, \
    e.g. approve a 2FA prompt). Never ask for passwords or codes in chat; use this instead. The browser is handed to \
    them and your turn waits (a few minutes at most) until they press Done or Decline; the tool returns which. \
    After Done, look at the page again before you continue.";

pub fn tools() -> Vec<Value> {
    vec![json!({
        "name": TOOL,
        "description": DESCRIPTION,
        "inputSchema": {
            "type": "object",
            "properties": {
                "reason": {"type": "string", "description": "What to do and why, in one sentence (max 300 characters)"},
                "kind": {"type": "string", "enum": ["sign_in", "captcha", "other"]},
                "url": {"type": "string", "description": "The page it happens on (optional)"},
            },
            "required": ["reason", "kind"],
        },
    })]
}

/// `request_human_step{reason, kind, url}` from the agent.
pub async fn tool_request(state: &AppState, scope: &SelfScope, args: &Value) -> (String, bool) {
    let (kind, reason, url) = match clean_request(
        args["kind"].as_str().unwrap_or(""),
        args["reason"].as_str().unwrap_or(""),
        args["url"].as_str(),
    ) {
        Ok(v) => v,
        Err(e) => return (e.to_string(), true),
    };
    match raise(state, scope, &kind, &reason, url.as_deref(), "bot").await {
        Ok(outcome) => (outcome_text(outcome), false),
        Err(e) => (e, true),
    }
}

/// What the agent is told about the outcome.
fn outcome_text(outcome: Option<Outcome>) -> String {
    match outcome {
        Some(Outcome::Done) => "The person says the step is done and handed the browser back. Look at the page \
            again (browser_page) before you continue."
            .into(),
        Some(Outcome::Declined) => "The person declined the step. Don't ask again: carry on without it if you can, or \
            end your turn and say plainly what is blocked."
            .into(),
        None => "The person hasn't done the step yet. Don't ask again: end your turn now with a short note on what \
            you're waiting for. When they finish, you'll get a new turn."
            .into(),
    }
}

/// Opens a step for the turn (or finds the one it already has open), tells the person, holds
/// the browser and waits. `Ok(None)` when the wait ran out; `Err` with what to tell the agent.
pub async fn raise(
    state: &AppState,
    scope: &SelfScope,
    kind: &str,
    reason: &str,
    url: Option<&str>,
    source: &str,
) -> Result<Option<Outcome>, String> {
    let (Some(origin), Some(cancel)) = (&scope.origin, &scope.cancel) else {
        return Err("You can only ask the person for a step from a chat.".into());
    };
    let place = HumanStepPlace {
        kwant_id: scope.kwant_id.clone(),
        channel_id: origin.channel_id.clone(),
        direct_kwant_id: origin.direct_kwant_id.clone(),
        thread_id: origin.thread_id.clone(),
        turn_id: scope.turn_id.clone(),
        sandbox_target: scope.sandbox_target.clone(),
    };
    // Like ask_user, the step, its card and the waiter appear together
    let hub = &state.human_steps;
    let mut waiter = None;
    let created = state
        .db
        .create_human_step_with_card(&place, kind, reason, url, source, |s| {
            waiter = Some((s.id.clone(), hub.register(&s.id)));
            let mut text = format!("{}: {}", kind_label(&s.kind), s.reason);
            if let Some(u) = &s.url {
                text.push_str(&format!("\n{u}"));
            }
            text.push_str("\n\nDo it in the browser panel, then press Done.");
            let (msg, extra) = place_message(s, "kwant", &scope.kwant_id, text);
            Some((msg, MessageExtra { meta: Some(card_meta(s)), ..extra }))
        })
        .await;
    let (step, is_new, card) = match created {
        Ok(v) => v,
        Err(e) => {
            if let Some((id, _)) = waiter {
                hub.take(&id);
            }
            return Err(match e {
                HumanStepError::Db => "Could not save the step.".into(),
                e => e.to_string(),
            });
        }
    };
    // The turn asked again for a step it already has open: wait on that one
    let mut rx = match waiter {
        Some((_, rx)) => rx,
        None => hub.register(&step.id),
    };
    let _guard = WaiterGuard { hub, id: step.id.clone() };
    if is_new {
        hold_browser(state, step.sandbox_target.as_deref()).await;
        if let Some(card) = card {
            if let Some(t) = &step.thread_id {
                let _ = state.db.touch_thread(t).await;
            }
            let _ = state.event_sender.send(RuntimeEvent::MessageAdded { message: serde_json::to_value(&card).unwrap_or(Value::Null) });
        }
        changed(state, &step).await;
        if let Some(turn) = step.turn_id.as_deref().and_then(super::activity::TurnActivity::find) {
            let title = format!("Waiting for you: {}", step.reason);
            turn.wait("human_step", &step.id, &title, Some(TOOL));
        }
        notify(
            state,
            Notification {
                kind: "human_step",
                kwant_id: scope.kwant_id.clone(),
                kwant_name: scope.kwant_name.clone(),
                channel_id: origin.channel_id.clone(),
                title: format!("{} needs you in the browser", scope.kwant_name),
                body: format!("{}. Open the chat, do it in the browser panel, then press Done.", kind_label(&step.kind)),
            },
        )
        .await;
        tracing::info!(human_step_id = %step.id, kwant_id = %scope.kwant_id, source, "agent asked the person for a browser step");
    }

    tokio::select! {
        r = &mut rx => Ok(r.ok()),
        _ = cancel.cancelled() => {
            close(state, &step.id, "cancelled").await;
            Err("The user stopped this turn. End it now.".into())
        }
        _ = tokio::time::sleep(hub.wait()) => {
            hub.take(&step.id);
            // An answer that arrived just as the wait ran out still counts
            Ok(rx.try_recv().ok())
        }
    }
}

/// Raised by kwnt when a browser tool lands on a CAPTCHA: the turn waits for the person like
/// for `request_human_step`, then gets the page again. `None` when there's no chat to ask in.
pub async fn raise_for_bot_wall(state: &AppState, scope: &SelfScope, page: &Value) -> Option<String> {
    scope.origin.as_ref()?;
    scope.cancel.as_ref()?;
    let url = page["url"].as_str().and_then(crate::db::human_steps::clean_url);
    let host = url.as_deref().and_then(|u| url::Url::parse(u).ok()).and_then(|u| u.host_str().map(str::to_string));
    let reason = match host {
        Some(h) => format!("{h} is showing a CAPTCHA / human check to the bot browser. Please solve it."),
        None => "The page is showing a CAPTCHA / human check to the bot browser. Please solve it.".into(),
    };
    Some(match raise(state, scope, "captcha", &reason, url.as_deref(), "auto").await {
        Ok(outcome) => format!("A CAPTCHA blocked the page, so the person was asked to solve it. {}", outcome_text(outcome)),
        Err(e) => e,
    })
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

/// `GET /api/human-steps?status=&context_id=&space=`: the space's steps, newest first.
pub async fn list(
    State(state): State<AppState>,
    super::spaces::Scope(space): super::spaces::Scope,
    Query(q): Query<ListQuery>,
) -> Response {
    let status = q.status.filter(|s| !s.is_empty());
    if let Some(s) = &status {
        if !HUMAN_STEP_STATUSES.contains(&s.as_str()) {
            return error(StatusCode::BAD_REQUEST, "unknown status");
        }
    }
    let context = q.context_id.as_deref().filter(|c| !c.is_empty());
    match state.db.list_human_steps(status.as_deref(), context, Some(&space)).await {
        Ok(list) => Json(list).into_response(),
        Err(_) => StatusCode::INTERNAL_SERVER_ERROR.into_response(),
    }
}

async fn load(state: &AppState, id: &str) -> Result<HumanStep, StatusCode> {
    match state.db.get_human_step(id).await {
        Ok(Some(s)) => Ok(s),
        Ok(None) => Err(StatusCode::NOT_FOUND),
        Err(_) => Err(StatusCode::INTERNAL_SERVER_ERROR),
    }
}

pub async fn get(State(state): State<AppState>, Path(id): Path<String>) -> Response {
    match load(&state, &id).await {
        Ok(s) => Json(json!({"step": s, "waiting": state.human_steps.is_waiting(&s.id)})).into_response(),
        Err(r) => r.into_response(),
    }
}

/// `POST /api/human-steps/:id/done`: the person did the step.
pub async fn done(State(state): State<AppState>, Path(id): Path<String>) -> Response {
    finish(&state, &id, Outcome::Done).await
}

/// `POST /api/human-steps/:id/decline`: the person won't do it.
pub async fn decline(State(state): State<AppState>, Path(id): Path<String>) -> Response {
    finish(&state, &id, Outcome::Declined).await
}

/// Closes the step, hands the browser back and tells the turn: the waiting turn at once, or a
/// new turn in the same chat if it already ended.
async fn finish(state: &AppState, id: &str, outcome: Outcome) -> Response {
    let s = match load(state, id).await {
        Ok(s) => s,
        Err(r) => return r.into_response(),
    };
    if s.status != "open" {
        return error(StatusCode::CONFLICT, format!("the step is already {}", s.status));
    }
    let Some(s) = close(state, id, outcome.status()).await else {
        return error(StatusCode::CONFLICT, "the step was just closed");
    };
    let text = match outcome {
        Outcome::Done => "Done",
        Outcome::Declined => "Declined",
    };
    let (msg, extra) = place_message(&s, "user", "user", text.into());
    let meta = json!({"kind": "human_step_reply", "human_step_id": s.id, "status": s.status});
    if let Ok(saved) = state.db.insert_message_with(&msg, &MessageExtra { meta: Some(meta), ..extra }).await {
        if let Some(t) = &s.thread_id {
            let _ = state.db.touch_thread(t).await;
        }
        let _ = state.event_sender.send(RuntimeEvent::MessageAdded { message: serde_json::to_value(&saved).unwrap_or(Value::Null) });
    }
    let live = state.human_steps.wake(&s.id, outcome);
    if !live {
        resume(state, &s, outcome).await;
    }
    Json(json!({"step": s, "resumed": if live { "turn" } else { "new_turn" }})).into_response()
}

/// A late answer: a new turn in the step's chat, queued like any other turn.
async fn resume(state: &AppState, s: &HumanStep, outcome: Outcome) {
    let Ok(Some(kwant)) = state.db.get_kwant(&s.kwant_id).await else {
        return;
    };
    let depth = match &s.thread_id {
        Some(t) => state.db.delegation_for_thread(t).await.ok().flatten().map(|d| d.depth).unwrap_or(0),
        None => 0,
    };
    let job = state.turns.start_job([s.context_id(), s.id.clone()]);
    let Ok(scope) = scope_for(state, &kwant, s.channel_id.clone(), s.thread_id.clone(), &job, depth).await else {
        return;
    };
    let prompt = match outcome {
        Outcome::Done => format!(
            "[The person did the browser step you asked for earlier]\nStep: {}\n\nLook at the browser page again and continue the task.",
            s.reason
        ),
        Outcome::Declined => format!(
            "[The person declined the browser step you asked for earlier]\nStep: {}\n\nDon't ask again. Carry on without it if you can, or say plainly what is blocked.",
            s.reason
        ),
    };
    tokio::spawn(async move {
        let _job = job;
        let _ = try_kwant_turn(&scope, &kwant, prompt, TurnOpts::default()).await;
    });
}

/// After a turn was stopped: its open steps are cancelled and the browser handed back.
pub async fn cancel_for_turn(state: &AppState, turn_id: &str) {
    for s in state.db.open_human_steps_for_turn(turn_id).await.unwrap_or_default() {
        drop(state.human_steps.take(&s.id));
        close(state, &s.id, "cancelled").await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn the_hub_wakes_the_waiting_turn_once() {
        let hub = HumanStepHub::default();
        let rx = hub.register("s1");
        assert!(hub.is_waiting("s1"));
        assert!(hub.wake("s1", Outcome::Declined));
        assert_eq!(rx.await.unwrap(), Outcome::Declined);
        assert!(!hub.is_waiting("s1"));
        assert!(!hub.wake("s1", Outcome::Done), "nobody waits any more: a new turn starts instead");
        let rx = hub.register("s2");
        drop(rx);
        assert!(!hub.wake("s2", Outcome::Done), "the tool call already ended");
    }

    #[test]
    fn the_agent_hears_each_outcome_plainly() {
        assert!(outcome_text(Some(Outcome::Done)).contains("Look at the page"));
        assert!(outcome_text(Some(Outcome::Declined)).contains("declined"));
        assert!(outcome_text(None).contains("end your turn"));
    }
}
