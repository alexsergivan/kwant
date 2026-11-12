//! Steps only the person can do in the bot browser: sign in, solve a CAPTCHA, or something else.
//! An agent calls `request_human_step` (or kwnt raises one itself when a browser tool lands on a
//! CAPTCHA): the step is saved as a card in its chat, the UI gets `human_step_updated` +
//! `message_added` and a `human_step` notification, the browser is held for the person, and the
//! turn waits until they press Done or Decline (`POST /api/human-steps/:id/done|decline`). If
//! nobody answers in time the tool tells the agent to wrap up, and a later answer starts a new
//! turn in the same chat with the outcome. Stopping the turn cancels the step.
//!
//! A CAPTCHA step is also watched: the page is probed every couple of seconds, and once the check
//! is gone the step closes as `resolved_auto` and the bot carries on by itself. Nobody solving it
//! within `captcha_timeout_minutes` (default 10) closes it as `timed_out`, and the bot is told to
//! move on rather than wait.

use super::continuity::{notify, Notification, SelfScope};
use super::routes::{scope_for, try_kwant_turn, TurnOpts};
use super::AppState;
use crate::db::human_steps::{clean_request, HumanStep, HumanStepError, HumanStepPlace, HUMAN_STEP_STATUSES};
use crate::db::models::{MessageExtra, NewMessage};
use crate::drivers::events::RuntimeEvent;
use crate::sandbox::browser::{detect_bot_wall, BrowserController, CaptchaHit};
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
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::sync::oneshot;

pub const TOOL: &str = "request_human_step";

/// What a waiting step is woken with.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Outcome {
    Done,
    Declined,
    /// The CAPTCHA left the page (solved, or it passed by itself).
    ResolvedAuto,
    /// Nobody solved the CAPTCHA within the time limit.
    TimedOut,
    /// Not an answer: another turn of the bot in this chat already asked for the same sign-in and
    /// is waiting on it, so this turn steps aside. The step stays open.
    AlreadyAsked,
}

impl Outcome {
    fn status(self) -> &'static str {
        match self {
            Outcome::Done => "done",
            Outcome::Declined => "declined",
            Outcome::ResolvedAuto => "resolved_auto",
            Outcome::TimedOut => "timed_out",
            Outcome::AlreadyAsked => "open",
        }
    }
}

pub const CAPTCHA_TIMEOUT_SETTING: &str = "captcha_timeout_minutes";
pub const DEFAULT_CAPTCHA_TIMEOUT_MINUTES: u64 = 10;
pub const MAX_CAPTCHA_TIMEOUT_MINUTES: u64 = 60;
/// Consecutive probes without the check before a step counts as solved: a page re-rendering
/// between two looks doesn't close it.
const GONE_AFTER_MISSES: u32 = 2;

/// The turns waiting on a human step, by step id. Empty (no cost) when unused.
pub struct HumanStepHub {
    waiters: Mutex<HashMap<String, oneshot::Sender<Outcome>>>,
    wait_ms: AtomicU64,
    /// Tests only: CAPTCHA time limit and probe interval in ms (0 = setting / default).
    captcha_timeout_ms: AtomicU64,
    probe_ms: AtomicU64,
    /// The turns running on each bot browser (by sandbox target), oldest first, so a CAPTCHA met
    /// through the shell browser API, which carries no turn of its own, is still asked of the
    /// person in the chat of the turn that met it. A team's browser is shared, so several bots'
    /// turns can run on one at once.
    turns: Mutex<HashMap<String, Vec<(u64, SelfScope)>>>,
    next_turn: AtomicU64,
}

impl Default for HumanStepHub {
    fn default() -> Self {
        Self {
            waiters: Mutex::default(),
            // Shorter than the CLIs' tool timeout (approval timeout + 30 s), so the tool answers first
            wait_ms: AtomicU64::new((crate::mcp::APPROVAL_TIMEOUT_SECS - 10) * 1000),
            captcha_timeout_ms: AtomicU64::new(0),
            probe_ms: AtomicU64::new(0),
            turns: Mutex::default(),
            next_turn: AtomicU64::new(1),
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

    /// Tests: a short CAPTCHA time limit and probe interval instead of the setting and 2 s.
    pub fn set_captcha_timing(&self, timeout: Duration, probe: Duration) {
        self.captcha_timeout_ms.store(timeout.as_millis() as u64, Ordering::Relaxed);
        self.probe_ms.store(probe.as_millis() as u64, Ordering::Relaxed);
    }

    fn probe_interval(&self) -> Duration {
        match self.probe_ms.load(Ordering::Relaxed) {
            0 => Duration::from_secs(2),
            ms => Duration::from_millis(ms),
        }
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

    /// Records a turn now running on a bot browser until the guard drops (the turn ends). Other
    /// turns on the same browser stay recorded.
    pub fn track_turn(self: &Arc<Self>, target: &str, scope: SelfScope) -> TurnOnBrowser {
        let token = self.next_turn.fetch_add(1, Ordering::Relaxed);
        self.turns.lock().unwrap_or_else(|e| e.into_inner()).entry(target.to_string()).or_default().push((token, scope));
        TurnOnBrowser { hub: self.clone(), target: target.to_string(), token }
    }

    /// The newest live turn on this bot browser that can ask in a chat, if one runs. Only for a
    /// caller kwnt doesn't know: with two bots on a team browser it may be the other bot's.
    pub fn turn_on(&self, target: &str) -> Option<SelfScope> {
        self.newest_on(target, |_| true)
    }

    /// The live turn on this bot browser that the call came from. `by` (the bot that made it)
    /// picks that bot's newest turn there, and no other bot's; with its chat as well (the turn's
    /// thread, channel or DM, [`chat_key`]), only that bot's turn in that chat, so one bot running
    /// two threads on a team browser is asked in the thread that met the wall. Without `by`,
    /// [`Self::turn_on`]. A caller that matches no turn gets `None`: it is never asked elsewhere.
    pub fn turn_of(&self, target: &str, by: Option<Caller<'_>>) -> Option<SelfScope> {
        match by {
            Some(Caller { kwant, chat: Some(chat) }) => {
                self.newest_on(target, |s| s.kwant_id == kwant && chat_key(s).as_deref() == Some(chat))
            }
            Some(Caller { kwant, chat: None }) => self.newest_on(target, |s| s.kwant_id == kwant),
            None => self.turn_on(target),
        }
    }

    /// Whether `kwant` has a turn running in `chat` ([`chat_key`]) on the bot browser `target`:
    /// that job still uses its page, even between two browser calls.
    pub fn has_turn_in(&self, target: &str, kwant: &str, chat: &str) -> bool {
        self.newest_on(target, |s| s.kwant_id == kwant && chat_key(s).as_deref() == Some(chat)).is_some()
    }

    fn newest_on(&self, target: &str, pick: impl Fn(&SelfScope) -> bool) -> Option<SelfScope> {
        let turns = self.turns.lock().unwrap_or_else(|e| e.into_inner());
        turns.get(target)?.iter().rev().map(|(_, s)| s).find(|s| {
            pick(s) && s.origin.is_some() && !s.cancel.as_ref().is_some_and(|c| c.is_cancelled())
        }).cloned()
    }
}

/// Who made a call on a bot's own browser API (`/api/sandbox/:id/bot/:kwant_id[/chat/:chat]/...`):
/// the bot, and the chat its turn runs in when the address names one.
#[derive(Clone, Copy, Debug)]
pub struct Caller<'a> {
    pub kwant: &'a str,
    pub chat: Option<&'a str>,
}

/// The chat a turn runs in, as its browser address names it: its thread, else its channel, else
/// its DM (the bot it talks to). The same for every turn of one thread, so the address in the
/// bot's prompt never changes within a thread.
pub fn chat_key(scope: &SelfScope) -> Option<String> {
    let o = scope.origin.as_ref()?;
    o.thread_id.clone().or_else(|| o.channel_id.clone()).or_else(|| o.direct_kwant_id.clone())
}

/// Keeps a turn recorded on its bot browser while it runs ([`HumanStepHub::track_turn`]).
pub struct TurnOnBrowser {
    hub: Arc<HumanStepHub>,
    target: String,
    token: u64,
}

impl Drop for TurnOnBrowser {
    fn drop(&mut self) {
        let mut turns = self.hub.turns.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(on) = turns.get_mut(&self.target) {
            on.retain(|(t, _)| *t != self.token);
            if on.is_empty() {
                turns.remove(&self.target);
            }
        }
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
        "two_factor" => "Sign-in code needed",
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
        // the page to show for this step (`?page=` on the live view)
        "page_key": s.page(),
        "source": s.source,
        "status": s.status,
        "target": s.target.as_deref().and_then(|t| serde_json::from_str::<Value>(t).ok()),
    })
}

/// How long the person has to solve a CAPTCHA: `captcha_timeout_minutes` (1 to 60, default 10).
pub async fn captcha_timeout(state: &AppState) -> Duration {
    match state.human_steps.captcha_timeout_ms.load(Ordering::Relaxed) {
        0 => {}
        ms => return Duration::from_millis(ms),
    }
    let minutes = state.db.get_setting(CAPTCHA_TIMEOUT_SETTING).await.ok().flatten()
        .and_then(|v| v.trim().parse::<u64>().ok())
        .filter(|m| (1..=MAX_CAPTCHA_TIMEOUT_MINUTES).contains(m))
        .unwrap_or(DEFAULT_CAPTCHA_TIMEOUT_MINUTES);
    Duration::from_secs(minutes * 60)
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

/// Holds the browser page for the person, if it is running.
async fn hold_browser(state: &AppState, target: Option<&str>) {
    let Some(target) = target else { return };
    let ctrl = state.browser_controllers.lock().await.get(target).cloned();
    if let Some(ctrl) = ctrl {
        if !ctrl.hold_for_person().await {
            tracing::info!("human step: the browser is busy or already the person's; bot actions are refused until it closes");
        }
    }
}

/// Hands the browser page back once no step is open on it any more.
async fn release_browser(state: &AppState, target: Option<&str>) {
    let Some(target) = target else { return };
    if state.db.human_step_open_on(target).await.unwrap_or(false) {
        return;
    }
    let ctrl = state.browser_controllers.lock().await.get(target).cloned();
    if let Some(ctrl) = ctrl {
        // never hand the bot a page with the person's drag still held down
        ctrl.release_pointer().await;
        ctrl.release_person_hold();
        // a solved CAPTCHA usually sets a clearance cookie: keep it past a browser restart
        ctrl.persist_cookies().await;
    }
}

/// Closes an open step, hands the browser back and announces it; `None` if it wasn't open.
async fn close(state: &AppState, id: &str, status: &'static str) -> Option<HumanStep> {
    if !state.db.close_human_step(id, status).await.ok()? {
        return None;
    }
    let s = state.db.get_human_step(id).await.ok().flatten()?;
    release_browser(state, s.page()).await;
    if let Some(turn) = s.turn_id.as_deref().and_then(super::activity::TurnActivity::find) {
        turn.settle(&s.id, if status == "cancelled" { "stopped" } else { status });
    }
    changed(state, &s).await;
    Some(s)
}

/// Whether a human step holds this browser page (a job's page, or the team page): bot actions
/// on it wait until it closes. Other pages of the team are not held.
pub async fn browser_is_held(state: &AppState, page: &str) -> bool {
    state.db.human_step_open_on(page).await.unwrap_or(false)
}

// Agent tool

pub const DESCRIPTION: &str = "Ask the person to do one step in the bot browser that only they can do: sign in \
    (kind sign_in), solve a CAPTCHA or human check (kind captcha), enter a one-time code or approve a 2FA prompt \
    (kind two_factor), or something else that needs them (kind other). Never ask for passwords or codes in chat; use this instead. The browser is handed to \
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
                "kind": {"type": "string", "enum": ["sign_in", "captcha", "two_factor", "other"]},
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
    match raise(state, scope, &kind, &reason, url.as_deref(), None, "bot").await {
        Ok(outcome) => (outcome_text(outcome, kind == "captcha"), false),
        Err(e) => (e, true),
    }
}

/// What the agent is told about the outcome.
fn outcome_text(outcome: Option<Outcome>, captcha: bool) -> String {
    match outcome {
        Some(Outcome::Done) => "The person says the step is done and handed the browser back. Look at the page \
            again (browser_page) before you continue."
            .into(),
        Some(Outcome::Declined) => "The person declined the step. Don't ask for it again in this turn unless the person \
            brings it up: carry on without it if you can, or end your turn and say plainly what is blocked."
            .into(),
        Some(Outcome::ResolvedAuto) => "The human check is gone from the page (solved or passed), and the browser is \
            yours again. Look at the page again (browser_page) and continue."
            .into(),
        Some(Outcome::TimedOut) => "The person didn't solve the CAPTCHA in time, so the step was closed. Don't wait for \
            it or ask again now: skip this site and use another source (an official API or a site you're signed in to), \
            or tell the person you'll try this site later."
            .into(),
        Some(Outcome::AlreadyAsked) => "You already asked the person for this sign-in in another turn in this chat, and \
            that turn is waiting for their answer, so no new card was made. Don't ask again: end your turn now, at most \
            with one line saying you're waiting for them to sign in. The other turn carries on once they answer."
            .into(),
        None if captcha => "The person hasn't solved the CAPTCHA yet. Don't ask again: end your turn now with a short \
            note on what you're waiting for. The step closes by itself once the check is gone or the time runs out, \
            and you'll get a new turn either way."
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
    target: Option<&CaptchaHit>,
    source: &str,
) -> Result<Option<Outcome>, String> {
    let page = super::browser_pages::scope_page(scope);
    raise_step(state, scope, page, kind, reason, url, target, source, true).await
}

/// [`raise`]; `wait: false` only asks (card, notification, held browser, CAPTCHA watcher) and
/// returns at once with `Ok(None)`: no turn waits, so the answer starts a new turn in the chat
/// ([`resume`]). For a CAPTCHA kwnt meets where nothing can wait on it (before the bot's turn
/// starts, or through the shell browser API).
#[allow(clippy::too_many_arguments)]
async fn raise_step(
    state: &AppState,
    scope: &SelfScope,
    page: Option<String>,
    kind: &str,
    reason: &str,
    url: Option<&str>,
    target: Option<&CaptchaHit>,
    source: &str,
    wait_for_it: bool,
) -> Result<Option<Outcome>, String> {
    let Some(origin) = &scope.origin else {
        return Err("You can only ask the person for a step from a chat.".into());
    };
    let cancel = match (&scope.cancel, wait_for_it) {
        (Some(c), _) => c.clone(),
        (None, false) => tokio_util::sync::CancellationToken::new(),
        (None, true) => return Err("You can only ask the person for a step from a chat.".into()),
    };
    let place = HumanStepPlace {
        kwant_id: scope.kwant_id.clone(),
        channel_id: origin.channel_id.clone(),
        direct_kwant_id: origin.direct_kwant_id.clone(),
        thread_id: origin.thread_id.clone(),
        turn_id: scope.turn_id.clone(),
        sandbox_target: scope.sandbox_target.clone(),
        // only a page of this turn's own bot browser
        page_key: page.filter(|p| scope.sandbox_target.as_deref() == Some(super::browser_pages::page_team(p))),
    };
    // The browser is the person's before anyone can see the step: a bot action that lands between
    // the two would otherwise run on a page the person was just asked to use (and a test reading
    // the open step could find the browser not yet held, as CI did once on 3bfe50e)
    hold_browser(state, place.page_key.as_deref().or(place.sandbox_target.as_deref())).await;
    // Like ask_user, the step, its card and the waiter appear together
    let hub = &state.human_steps;
    let target_json = target.and_then(|t| serde_json::to_string(t).ok());
    let mut waiter = None;
    let created = state
        .db
        .create_human_step_with_card(&place, kind, reason, url, target_json.as_deref(), source, |s| {
            if wait_for_it {
                waiter = Some((s.id.clone(), hub.register(&s.id)));
            }
            let mut text = format!("{}: {}", kind_label(&s.kind), s.reason);
            if let Some(u) = &s.url {
                text.push_str(&format!("\n{u}"));
            }
            text.push_str(if s.kind == "captcha" {
                "\n\nSolve it in the browser panel. The bot carries on by itself once the check is gone (or press Done)."
            } else {
                "\n\nDo it in the browser panel, then press Done."
            });
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
            // No step after all: hand the browser back unless another step still holds it
            release_browser(state, place.page_key.as_deref().or(place.sandbox_target.as_deref())).await;
            return Err(match e {
                HumanStepError::Db => "Could not save the step.".into(),
                e => e.to_string(),
            });
        }
    };
    // Another turn of this bot asked for the same sign-in and still waits on it: one card and one
    // waiter, and this turn steps aside instead of the person being asked twice
    if !is_new && step.turn_id != place.turn_id && hub.is_waiting(&step.id) {
        tracing::info!(human_step_id = %step.id, kwant_id = %scope.kwant_id, "sign-in already asked for in another turn");
        return Ok(Some(Outcome::AlreadyAsked));
    }
    if !wait_for_it && !is_new {
        // Already asked (the card is up): nothing new to show
        return Ok(None);
    }
    // The turn asked again for a step it already has open (or one no turn waits on any more):
    // wait on that one
    let rx = match waiter {
        Some((_, rx)) => Some(rx),
        None if wait_for_it => Some(hub.register(&step.id)),
        None => None,
    };
    let _guard = rx.is_some().then(|| WaiterGuard { hub, id: step.id.clone() });
    let mut wait = hub.wait();
    let mut captcha_deadline = None;
    if step.kind == "captcha" {
        let limit = captcha_timeout(state).await;
        let deadline = step.created_at + chrono::Duration::from_std(limit).unwrap_or_default();
        captcha_deadline = Some(deadline);
        let left = (deadline - chrono::Utc::now()).to_std().unwrap_or_default();
        // A little past the limit, so the watcher's timed_out wakes the turn before the wait ends
        wait = wait.min(left + hub.probe_interval() * 2);
    }
    if is_new {
        // Again, in case another step in this browser closed (and handed it back) just before
        // this one was saved; a no-op when it is still held
        hold_browser(state, step.page()).await;
        if step.kind == "captcha" && step.sandbox_target.is_some() {
            // a CAPTCHA kwnt saw is on the page already; one the bot named must show up first
            spawn_watch_captcha(state.clone(), step.id.clone(), source == "auto");
        }
        if let Some(card) = card {
            if let Some(t) = &step.thread_id {
                let _ = state.db.touch_thread(t).await;
            }
            let _ = state.event_sender.send(RuntimeEvent::MessageAdded { message: serde_json::to_value(&card).unwrap_or(Value::Null) });
        }
        changed(state, &step).await;
        // A turn that doesn't wait on the step isn't shown as waiting
        if let Some(turn) = step.turn_id.as_deref().and_then(super::activity::TurnActivity::find).filter(|_| wait_for_it) {
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
                thread_id: step.thread_id.clone(),
                card_id: None,
            },
        )
        .await;
        tracing::info!(human_step_id = %step.id, kwant_id = %scope.kwant_id, source, wait_for_it, "agent asked the person for a browser step");
    }

    let Some(mut rx) = rx else { return Ok(None) };
    tokio::select! {
        r = &mut rx => Ok(r.ok()),
        _ = cancel.cancelled() => {
            close(state, &step.id, "cancelled").await;
            Err("The user stopped this turn. End it now.".into())
        }
        _ = tokio::time::sleep(wait) => {
            // A CAPTCHA past its time limit has timed out, whoever notices first: the watcher can
            // lag behind this wait on a busy machine, and the bot would be told to wait for a step
            // that is already over (as CI saw once on c185fc9)
            if captcha_deadline.is_some_and(|d| chrono::Utc::now() >= d) && close(state, &step.id, Outcome::TimedOut.status()).await.is_some() {
                hub.take(&step.id);
                tracing::info!(human_step_id = %step.id, "CAPTCHA step timed out at the end of the turn's wait");
                return Ok(Some(Outcome::TimedOut));
            }
            // Closed by someone else just now (the watcher, or the person): the answer is on its
            // way to this turn's waiter, which is still registered, so it reaches this turn
            // instead of starting a new one
            if state.db.get_human_step(&step.id).await.ok().flatten().is_some_and(|s| s.status != "open") {
                if let Ok(Ok(outcome)) = tokio::time::timeout(Duration::from_secs(5), &mut rx).await {
                    return Ok(Some(outcome));
                }
            }
            hub.take(&step.id);
            // An answer that arrived just as the wait ran out still counts
            if let Ok(outcome) = rx.try_recv() {
                return Ok(Some(outcome));
            }
            Ok(None)
        }
    }
}

/// Raised by kwnt when a browser tool lands on a CAPTCHA: the turn waits for the person like
/// for `request_human_step`, then gets the page again. `None` when there's no chat to ask in.
pub async fn raise_for_bot_wall(state: &AppState, scope: &SelfScope, page: &Value) -> Option<String> {
    scope.origin.as_ref()?;
    scope.cancel.as_ref()?;
    let (reason, url, target) = wall_request(page);
    Some(match raise(state, scope, "captcha", &reason, url.as_deref(), target.as_ref(), "auto").await {
        Ok(outcome) => format!("A CAPTCHA blocked the page, so the person was asked to solve it. {}", outcome_text(outcome, true)),
        Err(e) => e,
    })
}

/// What the bot hears after kwnt asked the person for a CAPTCHA it can't wait on.
pub const ASKED_NOTE: &str = "A CAPTCHA / human check blocked the page, so kwnt showed the person a \"Human check needed\" card \
    and handed them the browser. Don't try to solve, bypass or click through it, don't use the browser until it is solved, and don't \
    switch to another site to get around it. End your turn now with one short line saying you're waiting for the person to solve \
    the check. You get a new turn when it is solved, or if nobody solves it in time.";

/// [`raise_for_bot_wall`] for a CAPTCHA no tool call can wait on: kwnt opened the page before
/// the bot's turn, or the bot met it through the shell browser API. The person gets the card and
/// the notification at once; the answer starts a new turn. `None` when there's no chat to ask in
/// (the note for the bot otherwise). `on`: the browser page that showed it, which the step holds.
pub async fn ask_for_bot_wall(state: &AppState, scope: &SelfScope, on: &str, page: &Value) -> Option<String> {
    scope.origin.as_ref()?;
    let (reason, url, target) = wall_request(page);
    match raise_step(state, scope, Some(on.to_string()), "captcha", &reason, url.as_deref(), target.as_ref(), "auto", false).await {
        Ok(Some(outcome)) => Some(format!("A CAPTCHA blocked the page, so the person was asked to solve it. {}", outcome_text(Some(outcome), true))),
        Ok(None) => Some(ASKED_NOTE.to_string()),
        Err(e) => {
            tracing::warn!(kwant_id = %scope.kwant_id, error = %e, "couldn't ask the person for a CAPTCHA");
            None
        }
    }
}

/// The step to ask for on a blocked page: reason, address and the widget's box.
fn wall_request(page: &Value) -> (String, Option<String>, Option<CaptchaHit>) {
    let target = CaptchaHit::parse(&page["captcha"]).and_then(CaptchaHit::on_screen);
    let url = page["url"].as_str().and_then(crate::db::human_steps::clean_url);
    let host = url.as_deref().and_then(|u| url::Url::parse(u).ok()).and_then(|u| u.host_str().map(str::to_string));
    let reason = match host {
        Some(h) => format!("{h} is showing a CAPTCHA / human check to the bot browser. Please solve it."),
        None => "The page is showing a CAPTCHA / human check to the bot browser. Please solve it.".into(),
    };
    (reason, url, target)
}

/// Whether the page still shows a human check: a CAPTCHA widget, or a bot-wall page (Cloudflare's
/// "just a moment", Google's "unusual traffic"). `None` when the page couldn't be read.
async fn wall_on_page(ctrl: &BrowserController) -> Option<(bool, Option<CaptchaHit>)> {
    if let Some(hit) = ctrl.probe_captcha().await.ok()? {
        return Some((true, Some(hit)));
    }
    let (url, title) = ctrl.sync_location().await?;
    let text = ctrl.extract_content().await.ok()?;
    Some((detect_bot_wall(&url, &title, &text).is_some(), None))
}

/// Watches an open CAPTCHA step until it closes: keeps its box current, closes it as
/// `resolved_auto` once the check is gone, or as `timed_out` at the time limit, and tells the bot.
/// Spawns [`watch_captcha`] from a plain fn, so `raise_step`'s future type doesn't depend on the
/// watcher's (watcher → resume → a new turn → raise_step would be a type cycle).
fn spawn_watch_captcha(state: AppState, id: String, seen: bool) {
    tokio::spawn(watch_captcha(state, id, seen));
}

async fn watch_captcha(state: AppState, id: String, mut seen: bool) {
    let limit = captcha_timeout(&state).await;
    let started = tokio::time::Instant::now();
    let interval = state.human_steps.probe_interval();
    let mut misses = 0u32;
    loop {
        tokio::time::sleep(interval.min(limit.saturating_sub(started.elapsed()))).await;
        let Ok(Some(step)) = state.db.get_human_step(&id).await else { return };
        if step.status != "open" {
            return;
        }
        if started.elapsed() >= limit {
            settle(&state, &id, Outcome::TimedOut).await;
            return;
        }
        let Some(target) = step.page() else { return };
        let ctrl = state.browser_controllers.lock().await.get(target).cloned();
        let Some(ctrl) = ctrl else { continue };
        match wall_on_page(&ctrl).await {
            // unreadable (mid-navigation, say): unknown, never "gone"
            None => {}
            Some((true, hit)) => {
                seen = true;
                misses = 0;
                if let Some(target_json) = hit.and_then(CaptchaHit::on_screen).and_then(|h| serde_json::to_string(&h).ok()) {
                    if state.db.set_human_step_target(&id, &target_json).await.unwrap_or(false) {
                        if let Ok(Some(s)) = state.db.get_human_step(&id).await {
                            changed(&state, &s).await;
                        }
                    }
                }
            }
            Some((false, _)) if seen => {
                misses += 1;
                if misses >= GONE_AFTER_MISSES {
                    settle(&state, &id, Outcome::ResolvedAuto).await;
                    return;
                }
            }
            Some((false, _)) => {}
        }
    }
}

/// Closes a CAPTCHA step kwnt decided on (gone or timed out) and tells the bot: the waiting turn
/// at once, or a new turn in the chat if it already ended.
async fn settle(state: &AppState, id: &str, outcome: Outcome) {
    let Some(s) = close(state, id, outcome.status()).await else { return };
    tracing::info!(human_step_id = %s.id, status = %s.status, "CAPTCHA step closed by kwnt");
    if !state.human_steps.wake(&s.id, outcome) {
        resume(state, &s, outcome).await;
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
        Outcome::Declined => "Declined",
        _ => "Done",
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
            "[The person declined the browser step you asked for earlier]\nStep: {}\n\nDon't ask for it again unless the person brings it up. Carry on without it if you can, or say plainly what is blocked.",
            s.reason
        ),
        Outcome::ResolvedAuto => format!(
            "[The human check in the browser is gone, so the browser is yours again]\nStep: {}\n\nLook at the browser page again and continue the task.",
            s.reason
        ),
        Outcome::TimedOut => format!(
            "[Nobody solved the CAPTCHA in time, so the step was closed]\nStep: {}\n\nDon't wait for it or ask again now. Skip this site and \
             continue with another source (an official API or a site you're signed in to), or tell the person you'll try this site later.",
            s.reason
        ),
        // Not an answer: nothing to tell a new turn.
        Outcome::AlreadyAsked => return,
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

    fn turn(kwant: &str) -> SelfScope {
        SelfScope {
            kwant_id: kwant.into(),
            kwant_name: kwant.into(),
            origin: Some(Default::default()),
            cancel: None,
            turn_id: Some(format!("turn-{kwant}")),
            workspace: None,
            sandbox_target: Some("team".into()),
        }
    }

    /// Two bots on one team browser: the bot that made the call gets its own turn, never the
    /// other bot's; only an unknown caller falls back to the newest turn, and each turn ending
    /// leaves the other recorded.
    #[test]
    fn a_shared_browser_knows_each_bots_turn() {
        let hub = Arc::new(HumanStepHub::default());
        let older = hub.track_turn("team", turn("ann"));
        let newer = hub.track_turn("team", turn("bob"));
        let of = |by: Option<&str>| hub.turn_of("team", by.map(|kwant| Caller { kwant, chat: None })).map(|s| s.kwant_id);
        assert_eq!(of(Some("ann")).as_deref(), Some("ann"));
        assert_eq!(of(Some("bob")).as_deref(), Some("bob"));
        assert_eq!(of(None).as_deref(), Some("bob"), "an unknown caller: the newest turn");
        assert_eq!(of(Some("cy")), None, "a bot with no turn here is never asked in another bot's chat");
        drop(newer);
        assert_eq!(of(None).as_deref(), Some("ann"));
        assert_eq!(of(Some("bob")), None);
        let cancelled = tokio_util::sync::CancellationToken::new();
        let stopped = hub.track_turn("team", SelfScope { cancel: Some(cancelled.clone()), ..turn("ann") });
        cancelled.cancel();
        assert_eq!(hub.turn_of("team", Some(Caller { kwant: "ann", chat: None })).and_then(|s| s.cancel), None, "a stopped turn asks nobody; the live one does");
        drop((older, stopped));
        assert!(hub.turn_on("team").is_none());
        assert!(hub.turns.lock().unwrap().is_empty());
    }

    fn turn_in(kwant: &str, thread: &str) -> SelfScope {
        let mut t = turn(kwant);
        t.origin = Some(super::super::continuity::TurnOrigin { thread_id: Some(thread.into()), ..Default::default() });
        t.turn_id = Some(format!("turn-{kwant}-{thread}"));
        t
    }

    /// One bot running two threads on one team browser: an address naming the thread gets that
    /// thread's turn, even while the other thread's turn is newer; a thread with no turn there is
    /// asked nowhere; the bot-only address (an old prompt) keeps the bot's newest turn.
    #[test]
    fn a_shared_browser_knows_each_threads_turn() {
        let hub = Arc::new(HumanStepHub::default());
        let a = hub.track_turn("team", turn_in("ann", "thread-a"));
        let _b = hub.track_turn("team", turn_in("ann", "thread-b"));
        let _other = hub.track_turn("team", turn_in("bob", "thread-c"));
        let of = |kwant: &str, chat: Option<&str>| hub.turn_of("team", Some(Caller { kwant, chat })).and_then(|s| s.turn_id);
        assert_eq!(of("ann", Some("thread-a")).as_deref(), Some("turn-ann-thread-a"), "the thread that called, not the newer one");
        assert_eq!(of("ann", Some("thread-b")).as_deref(), Some("turn-ann-thread-b"));
        assert_eq!(of("ann", None).as_deref(), Some("turn-ann-thread-b"), "no chat in the address: the bot's newest turn");
        assert_eq!(of("ann", Some("thread-c")), None, "another bot's thread is never this bot's");
        assert_eq!(of("ann", Some("thread-x")), None, "a thread with no turn here asks nobody");
        drop(a);
        assert_eq!(of("ann", Some("thread-a")), None, "thread A's turn ended: nobody, never thread B");
        assert_eq!(chat_key(&turn("ann")), None, "no thread, channel or DM: no chat to name");
        let dm = SelfScope { origin: Some(super::super::continuity::TurnOrigin { direct_kwant_id: Some("ann".into()), ..Default::default() }), ..turn("ann") };
        assert_eq!(chat_key(&dm).as_deref(), Some("ann"));
    }

    /// A job (bot and chat) with a turn running keeps its page; the turn ending frees it.
    #[test]
    fn a_job_with_a_live_turn_is_known_until_the_turn_ends() {
        let hub = Arc::new(HumanStepHub::default());
        let a = hub.track_turn("team", turn_in("ann", "thread-a"));
        assert!(hub.has_turn_in("team", "ann", "thread-a"));
        assert!(!hub.has_turn_in("team", "ann", "thread-b"), "another chat of the same bot");
        assert!(!hub.has_turn_in("team", "bob", "thread-a"), "another bot");
        assert!(!hub.has_turn_in("other-team", "ann", "thread-a"), "another team's browser");
        drop(a);
        assert!(!hub.has_turn_in("team", "ann", "thread-a"));
        let stopped = tokio_util::sync::CancellationToken::new();
        stopped.cancel();
        let _s = hub.track_turn("team", SelfScope { cancel: Some(stopped), ..turn_in("ann", "thread-a") });
        assert!(!hub.has_turn_in("team", "ann", "thread-a"), "a stopped turn uses no page");
    }

    #[test]
    fn the_agent_hears_each_outcome_plainly() {
        assert!(outcome_text(Some(Outcome::Done), false).contains("Look at the page"));
        let declined = outcome_text(Some(Outcome::Declined), false);
        assert!(declined.contains("declined") && declined.contains("Don't ask for it again in this turn unless the person brings it up"));
        let asked = outcome_text(Some(Outcome::AlreadyAsked), false);
        assert!(asked.contains("already asked") && asked.contains("Don't ask again") && asked.contains("end your turn"));
        assert_eq!(Outcome::AlreadyAsked.status(), "open", "the step stays open for the turn that waits on it");
        assert!(outcome_text(None, false).contains("end your turn"));
        assert!(outcome_text(Some(Outcome::ResolvedAuto), true).contains("continue"));
        let timed_out = outcome_text(Some(Outcome::TimedOut), true);
        assert!(timed_out.contains("skip this site") && timed_out.contains("another source"));
        assert!(outcome_text(None, true).contains("closes by itself"));
        assert_eq!(Outcome::ResolvedAuto.status(), "resolved_auto");
        assert_eq!(Outcome::TimedOut.status(), "timed_out");
    }
}
