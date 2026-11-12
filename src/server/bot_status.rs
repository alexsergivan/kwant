//! Why each bot has its status, and since when (H4); the quiet signal (P4); never sending to a
//! stuck or waiting bot, and no double send (H3). The contract the frontend reads is
//! `release-090/h4-contract.md`: the copy lives there, the server sends codes, times and names.
//!
//! The turn registry keeps the live signals ([`Board`]): each running turn's last streamed step,
//! its stall, and its quiet snooze; each bot's gave-up mark and last finished turn; the held sends;
//! the client message ids seen lately. [`compute`] turns one bot's signals into its status object
//! and [`hold_reason`] says whether a send to it waits. Both are pure and tested below. A watcher
//! ([`spawn_watcher`]) pushes `bot_status` on change and sends the 15 min quiet FYI once per turn.

use super::turns::ActiveTurn;
use super::AppState;
use crate::db::spaces::SpaceId;
use crate::drivers::events::RuntimeEvent;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use chrono::{DateTime, Duration as Span, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};
use tokio_util::sync::CancellationToken;

/// A live-engine turn with no step for this long is Quiet.
pub const QUIET_AFTER_MIN: i64 = 5;
/// From here Quiet is "long" (amber) and the Inbox FYI goes out, once per turn.
pub const QUIET_LONG_MIN: i64 = 15;
/// An engine that reports only at the end is Quiet after this long running.
pub const NO_LIVE_QUIET_AFTER_MIN: i64 = 15;
/// How long a turn that gave up after a stall keeps its bot Stuck.
pub const GAVE_UP_SHOWS_MIN: i64 = 30;
/// "Check again in 30 min".
pub const SNOOZE_MIN: i64 = 30;
/// Text and thinking deltas move the last step at most this often.
pub const STEP_BATCH_SECS: i64 = 10;
/// The same client message id in the same chat within this window is sent once.
pub const DEDUPE_MIN: i64 = 10;
pub const MAX_CLIENT_ID_CHARS: usize = 100;
/// Other turns listed beside the one the status is about.
const MAX_ALSO: usize = 2;
/// How often a held send checks whether its bot can take it.
const HOLD_POLL: std::time::Duration = std::time::Duration::from_millis(500);
/// How often the watcher re-checks every busy bot for quiet crossings, besides on events.
const WATCH_SWEEP: std::time::Duration = std::time::Duration::from_secs(15);

// ---------------------------------------------------------------------------------------------
// Live signals, kept by the turn registry

/// The stall watchdog fired in this turn and its retry hasn't answered yet.
#[derive(Debug, Clone, PartialEq)]
pub struct Stall {
    pub at: DateTime<Utc>,
    pub secs: u64,
}

/// What a running (or queued) turn showed so far.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct TurnSignals {
    pub last_step_at: Option<DateTime<Utc>>,
    /// The tool of the last step; `None` for text and thinking.
    pub last_step_title: Option<String>,
    /// `Some(false)`: the engine reports only at the end (`streams_progress() == false`). Any
    /// streamed step proves otherwise.
    pub live_steps: Option<bool>,
    /// "Claude", "Codex", …
    pub engine: Option<&'static str>,
    pub stalled: Option<Stall>,
    pub snooze_until: Option<DateTime<Utc>>,
    pub muted: bool,
    pub fyi_sent: bool,
    /// Why a queued turn waits: "agent_busy", "server_busy" or "vm_busy".
    pub queue_reason: Option<String>,
    /// Who started the turn, when that is certain (see [`TurnSource`]).
    pub source: Option<TurnSource>,
}

/// The longest `turn.source_name` sent: bot and routine names are the person's own text.
pub const MAX_SOURCE_NAME_CHARS: usize = 100;

/// Who started a turn, for the Why popover's "This turn" row ("from your message"). Set only where
/// the start is certain: a chat message (the person's, or a bot's sent with its own token), a
/// bot's hand-off, a routine's run. kwant has
/// no webhook or app-event starts in v1, so "webhook" and "app_event" are reserved in the
/// contract and never sent. Other turns (kwant's own follow-ups, a lead hearing its hand-offs'
/// results, retries of parked turns) have no source, and the row reads without one.
#[derive(Debug, Clone, PartialEq)]
pub struct TurnSource {
    /// "person", "bot" or "routine".
    pub kind: &'static str,
    /// The bot that handed the turn over, or the routine; never for "person".
    pub name: Option<String>,
}

impl TurnSource {
    pub fn person() -> Self {
        Self { kind: "person", name: None }
    }

    /// Who sent a chat message that starts a turn: the person, or a bot posting with its own
    /// token (its message is saved as sent by a bot; the token doesn't name the bot here).
    pub fn sender(from_person: bool) -> Self {
        if from_person {
            Self::person()
        } else {
            Self { kind: "bot", name: None }
        }
    }

    pub fn bot(name: &str) -> Self {
        Self { kind: "bot", name: clean_name(name) }
    }

    pub fn routine(name: &str) -> Self {
        Self { kind: "routine", name: clean_name(name) }
    }
}

/// A name trimmed to one line of at most [`MAX_SOURCE_NAME_CHARS`]; `None` when blank.
fn clean_name(name: &str) -> Option<String> {
    let one_line: String = name.split_whitespace().collect::<Vec<_>>().join(" ");
    let clipped: String = one_line.chars().take(MAX_SOURCE_NAME_CHARS).collect();
    (!clipped.is_empty()).then_some(clipped)
}

impl TurnSignals {
    /// Notes one streamed driver event. Tool events always count as a step; text, thinking and
    /// driver progress at most once per [`STEP_BATCH_SECS`].
    pub fn saw(&mut self, e: &RuntimeEvent, now: DateTime<Utc>) {
        let (title, batched) = match e {
            RuntimeEvent::ToolCallPending { name, .. } => (Some(Some(name.clone())), false),
            RuntimeEvent::ToolCallCompleted { .. } => (None, false),
            RuntimeEvent::TextDelta(_) | RuntimeEvent::ThinkingDelta(_) => (Some(None), true),
            RuntimeEvent::Progress => (None, true),
            _ => return,
        };
        // The engine answered: a stall's retry got through, and it streams after all
        self.stalled = None;
        self.live_steps = Some(true);
        if batched && self.last_step_at.is_some_and(|t| now - t < Span::seconds(STEP_BATCH_SECS)) {
            return;
        }
        self.last_step_at = Some(now);
        if let Some(t) = title {
            self.last_step_title = t;
        }
    }
}

/// A bot's turn stopped after the stall watchdog fired (`turn_stalled`).
#[derive(Debug, Clone, PartialEq)]
pub struct GaveUp {
    pub at: DateTime<Utc>,
    pub tries: u32,
    pub secs: u64,
    pub turn_id: String,
    pub context_id: String,
    pub thread_id: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct BotMarks {
    pub gave_up: Option<GaveUp>,
    pub last_finished_at: Option<DateTime<Utc>>,
}

/// A send that waits for its bot (H3): the person's message, or a bot's hand-off.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct HeldItem {
    /// "message" or "handoff".
    pub kind: &'static str,
    /// The user message id, or the delegation id.
    pub id: String,
    #[serde(skip)]
    pub kwant_id: String,
    pub since: DateTime<Utc>,
    pub reason_code: &'static str,
    pub context_id: String,
}

/// A client message id seen in a chat: when, and the id of the message it saves. The id is
/// reserved with the send, so a duplicate racing it gets the id too; `saved` is false while
/// the first send is still in flight.
#[derive(Debug, Clone, PartialEq)]
pub struct Sent {
    pub at: DateTime<Utc>,
    pub message_id: String,
    pub saved: bool,
}

#[derive(Debug, Default)]
pub struct Board {
    pub signals: HashMap<String, TurnSignals>,
    pub marks: HashMap<String, BotMarks>,
    pub held: Vec<HeldItem>,
    sent: HashMap<String, Sent>,
    /// Bots whose status may have changed outside the event bus (a hold, a mark, a snooze).
    nudged: HashSet<String>,
}

impl Board {
    pub fn nudge(&mut self, kwant_id: &str) {
        self.nudged.insert(kwant_id.to_string());
    }

    /// 0.9.5 (no silent stops): the person writing to `kwant_id` in the chat where it gave up after
    /// a stall is that stop's Retry (the notice's Retry button resends their message; the notice
    /// says "Send your message again to retry"). It ends the gave-up Stuck like "Try again" does,
    /// so the one press runs it now instead of holding it until "Try again" is pressed as well.
    /// True when it ended one. The gave-up mark is a status hold, not a safety guard: failure
    /// streaks, budget and spending stops live elsewhere and keep their own one resume button.
    pub fn person_retries_in(&mut self, kwant_id: &str, context_id: &str) -> bool {
        let Some(m) = self.marks.get_mut(kwant_id) else {
            return false;
        };
        if !m.gave_up.as_ref().is_some_and(|g| g.context_id == context_id || g.thread_id.as_deref() == Some(context_id)) {
            return false;
        }
        m.gave_up = None;
        self.nudge(kwant_id);
        true
    }

    pub fn take_nudged(&mut self) -> HashSet<String> {
        std::mem::take(&mut self.nudged)
    }

    /// Reserves `key` (chat + client message id) for one send, whose message will be saved as
    /// `message_id`. `Err` with the first send when it was seen within [`DEDUPE_MIN`].
    pub fn reserve_send(&mut self, key: &str, now: DateTime<Utc>, message_id: &str) -> Result<(), Sent> {
        let window = Span::minutes(DEDUPE_MIN);
        self.sent.retain(|_, s| now - s.at < window);
        if let Some(s) = self.sent.get(key) {
            return Err(s.clone());
        }
        self.sent.insert(key.to_string(), Sent { at: now, message_id: message_id.to_string(), saved: false });
        Ok(())
    }

    /// The send saved its message as `message_id`: the key stays taken for the window.
    pub fn send_saved(&mut self, key: &str, message_id: &str) {
        if let Some(s) = self.sent.get_mut(key) {
            s.message_id = message_id.to_string();
            s.saved = true;
        }
    }

    /// The send failed before it was saved: the id may be sent again.
    pub fn release_send(&mut self, key: &str) {
        if self.sent.get(key).is_some_and(|s| !s.saved) {
            self.sent.remove(key);
        }
    }
}

/// A client message id is 1–100 characters of `[A-Za-z0-9_.:-]`.
pub fn valid_client_id(id: &str) -> bool {
    !id.is_empty()
        && id.chars().count() <= MAX_CLIENT_ID_CHARS
        && id.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | ':' | '-'))
}

/// The short engine name the copy uses ("the model (Claude)").
pub fn engine_label(e: crate::drivers::engines::EngineId) -> &'static str {
    use crate::drivers::engines::EngineId as E;
    match e {
        E::Claude | E::AnthropicApi => "Claude",
        E::Codex | E::OpenaiApi => "Codex",
        E::Grok | E::XaiApi => "Grok",
        E::Ollama => "Ollama",
        E::CustomApi => "Custom API",
    }
}

// ---------------------------------------------------------------------------------------------
// The status object

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct WaitsOn {
    /// "model", "tool", "person", "bot", "queue" or "budget".
    pub kind: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct TurnRef {
    pub turn_id: String,
    pub context_id: String,
    pub thread_id: Option<String>,
    pub started_at: DateTime<Utc>,
    pub live_steps: bool,
    /// "person", "bot" or "routine"; omitted when not certain (see [`TurnSource`]).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source: Option<&'static str>,
    /// The bot or routine that started the turn; omitted for "person" and when unknown.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_name: Option<String>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct Queue {
    pub position: usize,
    pub reason: String,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct Quiet {
    /// "short" (5–14 min) or "long" (15 min and more).
    pub level: &'static str,
    pub snoozed_until: Option<DateTime<Utc>>,
    pub muted: bool,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct Retry {
    pub at: DateTime<Utc>,
    #[serde(rename = "try")]
    pub attempt: u32,
    pub of: u32,
    pub stall_secs: u64,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct Stuck {
    pub gave_up_at: DateTime<Utc>,
    pub tries: u32,
    pub stall_secs: u64,
    pub turn_id: String,
    pub context_id: String,
    pub thread_id: Option<String>,
}

/// An open question of the bot.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct QuestionRef {
    pub id: String,
    pub asked_at: DateTime<Utc>,
    pub context_id: String,
    #[serde(skip)]
    pub turn_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct Also {
    pub status: &'static str,
    pub reason_code: &'static str,
    pub since: Option<DateTime<Utc>>,
    pub context_id: String,
    pub thread_id: Option<String>,
}

/// One bot's status, as the contract's §1 has it.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct Status {
    pub kwant_id: String,
    /// "working", "queued", "needs_you", "quiet", "stuck", "paused" or "available".
    pub status: &'static str,
    pub reason_code: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason_text: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub since: Option<DateTime<Utc>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_step_at: Option<DateTime<Utc>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_step_title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub waits_on: Option<WaitsOn>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub turn: Option<TurnRef>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub queue: Option<Queue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub quiet: Option<Quiet>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub retry: Option<Retry>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stuck: Option<Stuck>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub question: Option<QuestionRef>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub also: Vec<Also>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub held: Vec<HeldItem>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_finished_at: Option<DateTime<Utc>>,
}

impl Status {
    fn new(kwant_id: &str, status: &'static str, reason_code: &'static str, since: Option<DateTime<Utc>>) -> Self {
        Self {
            kwant_id: kwant_id.to_string(),
            status,
            reason_code,
            reason_text: None,
            since,
            last_step_at: None,
            last_step_title: None,
            waits_on: None,
            turn: None,
            queue: None,
            quiet: None,
            retry: None,
            stuck: None,
            question: None,
            also: Vec::new(),
            held: Vec::new(),
            last_finished_at: None,
        }
    }

    /// What a push compares: everything but the last step, which moves on every step.
    pub fn fingerprint(&self) -> Value {
        let mut v = serde_json::to_value(self).unwrap_or(Value::Null);
        if let Some(o) = v.as_object_mut() {
            o.remove("last_step_at");
            o.remove("last_step_title");
        }
        v
    }
}

/// What a running turn waits on the person for.
#[derive(Debug, Clone, PartialEq)]
pub enum Wait {
    Question(QuestionRef),
    Approval(DateTime<Utc>),
    Browser(DateTime<Utc>),
}

/// One turn of the bot, with its signals.
#[derive(Debug, Clone)]
pub struct TurnInput {
    pub turn: ActiveTurn,
    pub signals: TurnSignals,
    pub wait: Option<Wait>,
    /// The tool of the newest step that still runs.
    pub running_tool: Option<String>,
    /// 1 for the next of this bot's queued turns to start.
    pub position: usize,
}

/// Everything [`compute`] needs about one bot.
#[derive(Debug, Clone)]
pub struct BotInput {
    pub kwant_id: String,
    pub turns: Vec<TurnInput>,
    /// The bot's open questions, newest first; their turn may have ended.
    pub questions: Vec<QuestionRef>,
    pub marks: BotMarks,
    pub held: Vec<HeldItem>,
}

fn is_handoff_tool(name: &str) -> bool {
    name == "delegate" || name.ends_with("__delegate")
}

fn rank(status: &str) -> u8 {
    match status {
        "needs_you" => 0,
        "stuck" => 1,
        "quiet" => 2,
        "working" => 3,
        "queued" => 4,
        _ => 5,
    }
}

/// Whether a turn is past its quiet threshold, ignoring snoozes: `Some(level)` if so.
fn quiet_level(t: &TurnInput, now: DateTime<Utc>) -> Option<&'static str> {
    let live = t.signals.live_steps.unwrap_or(true);
    let last = t.signals.last_step_at.unwrap_or(t.turn.since);
    if live {
        let idle = now - last;
        (idle >= Span::minutes(QUIET_AFTER_MIN)).then_some(if idle >= Span::minutes(QUIET_LONG_MIN) { "long" } else { "short" })
    } else {
        (now - t.turn.since >= Span::minutes(NO_LIVE_QUIET_AFTER_MIN)).then_some("long")
    }
}

fn quiet_allowed(s: &TurnSignals, now: DateTime<Utc>) -> bool {
    !s.muted && s.snooze_until.is_none_or(|u| now >= u)
}

/// The status of one turn.
fn turn_status(kwant_id: &str, t: &TurnInput, now: DateTime<Utc>) -> Status {
    let sig = &t.signals;
    let live = sig.live_steps.unwrap_or(true);
    let model = || WaitsOn { kind: "model", label: sig.engine.map(str::to_string) };
    let turn = TurnRef {
        turn_id: t.turn.turn_id.clone(),
        context_id: t.turn.context_id.clone(),
        thread_id: t.turn.thread_id.clone(),
        started_at: t.turn.since,
        live_steps: live,
        source: sig.source.as_ref().map(|s| s.kind),
        source_name: sig.source.as_ref().and_then(|s| s.name.clone()),
    };
    if t.turn.state == "queued" {
        let mut s = Status::new(kwant_id, "queued", "in_line", Some(t.turn.since));
        s.waits_on = Some(WaitsOn { kind: "queue", label: Some(t.position.to_string()) });
        s.queue = Some(Queue { position: t.position, reason: sig.queue_reason.clone().unwrap_or_else(|| "agent_busy".into()) });
        s.turn = Some(turn);
        return s;
    }
    let mut s = match &t.wait {
        Some(Wait::Question(q)) => {
            let mut s = Status::new(kwant_id, "needs_you", "asked_question", Some(q.asked_at));
            s.question = Some(q.clone());
            s.waits_on = Some(WaitsOn { kind: "person", label: None });
            s
        }
        Some(Wait::Approval(at)) => {
            let mut s = Status::new(kwant_id, "needs_you", "wants_ok", Some(*at));
            s.waits_on = Some(WaitsOn { kind: "person", label: None });
            s
        }
        Some(Wait::Browser(at)) => {
            let mut s = Status::new(kwant_id, "needs_you", "needs_browser", Some(*at));
            s.waits_on = Some(WaitsOn { kind: "person", label: None });
            s
        }
        None => match &sig.stalled {
            Some(st) => {
                let mut s = Status::new(kwant_id, "stuck", "stalled_retrying", Some(st.at));
                s.waits_on = Some(model());
                s.retry = Some(Retry { at: st.at, attempt: 2, of: 2, stall_secs: st.secs });
                s
            }
            None => {
                let (code, text, waits) = match &t.running_tool {
                    Some(tool) if is_handoff_tool(tool) => ("waiting_for_bot", None, WaitsOn { kind: "bot", label: None }),
                    Some(tool) => ("running_tool", Some(tool.clone()), WaitsOn { kind: "tool", label: Some(tool.clone()) }),
                    None if !live => ("reports_at_end", None, model()),
                    None => ("thinking", None, model()),
                };
                let level = quiet_level(t, now);
                let mut s = match level {
                    Some(_) if quiet_allowed(sig, now) => Status::new(
                        kwant_id,
                        "quiet",
                        if live { "no_step" } else { "no_live_steps" },
                        Some(sig.last_step_at.unwrap_or(t.turn.since)),
                    ),
                    _ => Status::new(kwant_id, "working", code, Some(t.turn.since)),
                };
                s.reason_text = text;
                s.waits_on = Some(waits);
                if level.is_some() || sig.muted || sig.snooze_until.is_some_and(|u| now < u) {
                    s.quiet = Some(Quiet {
                        level: level.unwrap_or("short"),
                        snoozed_until: sig.snooze_until.filter(|u| now < *u),
                        muted: sig.muted,
                    });
                }
                s
            }
        },
    };
    s.last_step_at = sig.last_step_at;
    s.last_step_title = sig.last_step_title.clone();
    s.turn = Some(turn);
    s
}

/// The bot's status (contract §2): its most pressing turn, the others in `also`.
pub fn compute(b: &BotInput, now: DateTime<Utc>) -> Status {
    let mut parts: Vec<Status> = b.turns.iter().map(|t| turn_status(&b.kwant_id, t, now)).collect();
    // An open question whose turn ended (a late answer starts a new turn) still waits on the person
    if !parts.iter().any(|p| p.status == "needs_you") {
        if let Some(q) = b.questions.iter().max_by_key(|q| q.asked_at) {
            let mut s = Status::new(&b.kwant_id, "needs_you", "asked_question", Some(q.asked_at));
            s.question = Some(q.clone());
            s.waits_on = Some(WaitsOn { kind: "person", label: None });
            parts.push(s);
        }
    }
    if parts.is_empty() {
        if let Some(g) = b.marks.gave_up.as_ref().filter(|g| now - g.at < Span::minutes(GAVE_UP_SHOWS_MIN)) {
            let mut s = Status::new(&b.kwant_id, "stuck", "stalled_gave_up", Some(g.at));
            s.stuck = Some(Stuck {
                gave_up_at: g.at,
                tries: g.tries,
                stall_secs: g.secs,
                turn_id: g.turn_id.clone(),
                context_id: g.context_id.clone(),
                thread_id: g.thread_id.clone(),
            });
            parts.push(s);
        }
    }
    let started = |s: &Status| s.turn.as_ref().map(|t| t.started_at).or(s.since);
    parts.sort_by(|a, b| rank(a.status).cmp(&rank(b.status)).then_with(|| started(b).cmp(&started(a))));
    let mut parts = parts.into_iter();
    let mut out = parts.next().unwrap_or_else(|| Status::new(&b.kwant_id, "available", "idle", b.marks.last_finished_at));
    out.also = parts
        .take(MAX_ALSO)
        .map(|p| {
            let (context_id, thread_id) = match (&p.turn, &p.question) {
                (Some(t), _) => (t.context_id.clone(), t.thread_id.clone()),
                (None, Some(q)) => (q.context_id.clone(), None),
                _ => (String::new(), None),
            };
            Also { status: p.status, reason_code: p.reason_code, since: p.since, context_id, thread_id }
        })
        .collect();
    out.held = b.held.clone();
    out.last_finished_at = b.marks.last_finished_at;
    out
}

/// Why a send in `context_id` to this bot waits (contract §7a), or `None` to send it now.
#[derive(Debug, Clone, PartialEq)]
pub struct HoldReason {
    /// "stuck" or "waits_on_you".
    pub reason_code: &'static str,
    pub since: DateTime<Utc>,
    pub question: Option<QuestionRef>,
}

pub fn hold_reason(b: &BotInput, context_id: &str, now: DateTime<Utc>) -> Option<HoldReason> {
    for t in b.turns.iter().filter(|t| t.turn.state == "running") {
        if let Some(st) = &t.signals.stalled {
            return Some(HoldReason { reason_code: "stuck", since: st.at, question: None });
        }
    }
    if b.turns.iter().all(|t| t.turn.state != "running") {
        if let Some(g) = b.marks.gave_up.as_ref().filter(|g| now - g.at < Span::minutes(GAVE_UP_SHOWS_MIN)) {
            return Some(HoldReason { reason_code: "stuck", since: g.at, question: None });
        }
    }
    // Only a turn that still runs and waits blocks the bot; an old open question doesn't
    for t in b.turns.iter().filter(|t| t.turn.state == "running") {
        match &t.wait {
            // A message where the bot asked is the answer, never held
            Some(Wait::Question(q)) if q.context_id == context_id => {}
            Some(Wait::Question(q)) => {
                return Some(HoldReason { reason_code: "waits_on_you", since: q.asked_at, question: Some(q.clone()) })
            }
            Some(Wait::Approval(at)) | Some(Wait::Browser(at)) => {
                return Some(HoldReason { reason_code: "waits_on_you", since: *at, question: None })
            }
            None => {}
        }
    }
    None
}

/// A live-engine turn that is quiet 15 min and hasn't been told about yet: send the FYI.
pub fn fyi_due(t: &TurnInput, now: DateTime<Utc>) -> bool {
    t.turn.state == "running"
        && t.wait.is_none()
        && t.signals.stalled.is_none()
        && !t.signals.fyi_sent
        && t.signals.live_steps.unwrap_or(true)
        && quiet_allowed(&t.signals, now)
        && quiet_level(t, now) == Some("long")
}

// ---------------------------------------------------------------------------------------------
// Gathering from the server

fn context_of(q: &crate::db::questions::Question) -> String {
    q.thread_id.clone().or_else(|| q.channel_id.clone()).or_else(|| q.direct_kwant_id.clone()).unwrap_or_default()
}

fn parse_time(s: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(s).ok().map(|t| t.with_timezone(&Utc))
}

/// The inputs of `kwant_ids`, all in `space`.
pub async fn inputs(state: &AppState, space: &SpaceId, kwant_ids: &[String]) -> Vec<BotInput> {
    let turns = state.turns.active_turns();
    let questions = state.db.list_questions(Some("open"), None, Some(space)).await.unwrap_or_default();
    let (signals, marks, held) = state.turns.board(|b| {
        let wanted: HashSet<&str> = kwant_ids.iter().map(String::as_str).collect();
        let signals: HashMap<String, TurnSignals> = turns
            .iter()
            .filter(|t| wanted.contains(t.kwant_id.as_str()))
            .filter_map(|t| b.signals.get(&t.turn_id).map(|s| (t.turn_id.clone(), s.clone())))
            .collect();
        (signals, b.marks.clone(), b.held.clone())
    });
    kwant_ids
        .iter()
        .map(|k| {
            let mine: Vec<&ActiveTurn> = turns.iter().filter(|t| &t.kwant_id == k).collect();
            let mut queued: Vec<&ActiveTurn> = mine.iter().copied().filter(|t| t.state == "queued").collect();
            queued.sort_by_key(|t| t.since);
            let qs: Vec<QuestionRef> = questions
                .iter()
                .filter(|q| &q.kwant_id == k)
                .map(|q| QuestionRef { id: q.id.clone(), asked_at: q.created_at, context_id: context_of(q), turn_id: q.turn_id.clone() })
                .collect();
            let turns = mine
                .iter()
                .map(|t| {
                    let waits = super::activity::turn_waits(&t.turn_id);
                    let wait = qs
                        .iter()
                        .find(|q| q.turn_id.as_deref() == Some(t.turn_id.as_str()))
                        .map(|q| Wait::Question(q.clone()))
                        .or_else(|| {
                            waits.waiting.map(|(kind, at)| {
                                let at = parse_time(&at).unwrap_or(t.since);
                                if kind == "human_step" { Wait::Browser(at) } else { Wait::Approval(at) }
                            })
                        });
                    TurnInput {
                        turn: (*t).clone(),
                        signals: signals.get(&t.turn_id).cloned().unwrap_or_default(),
                        wait,
                        running_tool: waits.running_tool,
                        position: queued.iter().position(|q| q.turn_id == t.turn_id).map_or(0, |i| i + 1),
                    }
                })
                .collect();
            BotInput {
                kwant_id: k.clone(),
                turns,
                questions: qs,
                marks: marks.get(k).cloned().unwrap_or_default(),
                held: held.iter().filter(|h| &h.kwant_id == k).cloned().collect(),
            }
        })
        .collect()
}

async fn input_of(state: &AppState, kwant_id: &str) -> Option<BotInput> {
    let space = state.db.space_of_kwant(kwant_id).await.ok().flatten().unwrap_or_else(SpaceId::default_space);
    inputs(state, &space, &[kwant_id.to_string()]).await.pop()
}

pub async fn status_of(state: &AppState, kwant_id: &str) -> Option<Status> {
    Some(compute(&input_of(state, kwant_id).await?, Utc::now()))
}

// ---------------------------------------------------------------------------------------------
// H3: holding a send, and releasing it

/// Holds the send `item_id` ("message" or "handoff") to `kwant` in `context_id` when the bot is
/// stuck or waits on the person elsewhere; the `held` object the response carries, or `None`
/// to send it now. Run its turn after [`wait_released`].
pub async fn hold(state: &AppState, kwant: &crate::db::models::Kwant, context_id: &str, kind: &'static str, item_id: &str) -> Option<Value> {
    hold_from(state, kwant, context_id, kind, item_id, false).await
}

/// [`hold`] for a send that may be the person's own: their message in the chat where the bot gave
/// up after a stall is its Retry and goes now ([`Board::person_retries_in`]).
pub async fn hold_from(
    state: &AppState,
    kwant: &crate::db::models::Kwant,
    context_id: &str,
    kind: &'static str,
    item_id: &str,
    from_person: bool,
) -> Option<Value> {
    if from_person && kind == "message" && state.turns.board(|b| b.person_retries_in(&kwant.id, context_id)) {
        tracing::info!(kwant = %kwant.id, "the person retried in the chat where the bot stalled: it runs now");
    }
    let input = inputs(state, &SpaceId::new(&kwant.space_id), std::slice::from_ref(&kwant.id)).await.pop()?;
    let r = hold_reason(&input, context_id, Utc::now())?;
    state.turns.board(|b| {
        b.held.push(HeldItem {
            kind,
            id: item_id.to_string(),
            kwant_id: kwant.id.clone(),
            since: Utc::now(),
            reason_code: r.reason_code,
            context_id: context_id.to_string(),
        });
        b.nudge(&kwant.id);
    });
    tracing::info!(kwant = %kwant.id, kind, reason = r.reason_code, "send held until the bot can take it");
    Some(held_view(kwant, &r))
}

fn held_view(kwant: &crate::db::models::Kwant, r: &HoldReason) -> Value {
    let mut v = json!({ "kwant_id": kwant.id, "kwant_name": kwant.name, "reason_code": r.reason_code, "since": r.since });
    if let Some(q) = &r.question {
        v["question"] = json!(q);
    }
    v
}

/// Waits until the held send `item_id` may go: `true` then (or when it wasn't held), `false`
/// when it was stopped first. Either way it is no longer held.
pub async fn wait_released(state: &AppState, item_id: &str, cancel: &CancellationToken) -> bool {
    let Some(item) = state.turns.board(|b| b.held.iter().find(|h| h.id == item_id).cloned()) else {
        return true;
    };
    let unhold = || {
        state.turns.board(|b| {
            b.held.retain(|h| h.id != item_id);
            b.nudge(&item.kwant_id);
        })
    };
    loop {
        tokio::select! {
            _ = cancel.cancelled() => {
                unhold();
                return false;
            }
            _ = tokio::time::sleep(HOLD_POLL) => {}
        }
        let free = match input_of(state, &item.kwant_id).await {
            Some(input) => hold_reason(&input, &item.context_id, Utc::now()).is_none(),
            None => true,
        };
        if free {
            unhold();
            tracing::info!(kwant = %item.kwant_id, kind = item.kind, "held send released");
            return true;
        }
    }
}

/// What a bot that hands work to a held bot is told (tool result text).
pub fn handoff_held_text(name: &str, reason_code: &str) -> String {
    if reason_code == "stuck" {
        format!("{name} is stuck right now, so your request is queued, not lost. Don't send it again. It goes to {name} when {name} runs again.")
    } else {
        format!("{name} is waiting for the person to answer, so your request is queued, not lost. Don't send it again. It goes to {name} after the person answers.")
    }
}

// ---------------------------------------------------------------------------------------------
// Push

fn kwant_of(e: &RuntimeEvent) -> Option<String> {
    match e {
        RuntimeEvent::TurnStarted { kwant_id, .. } | RuntimeEvent::TurnEnded { kwant_id, .. } | RuntimeEvent::TurnQueued { kwant_id, .. } => {
            Some(kwant_id.clone())
        }
        RuntimeEvent::QuestionUpdated { question: v } | RuntimeEvent::HumanStepUpdated { step: v } => v["kwant_id"].as_str().map(str::to_string),
        RuntimeEvent::Scoped { tag, event } => match event.as_ref() {
            RuntimeEvent::Activity { .. } | RuntimeEvent::ApprovalRequired { .. } | RuntimeEvent::ApprovalWithdrawn { .. } => Some(tag.kwant_id.clone()),
            _ => None,
        },
        _ => None,
    }
}

/// Sends `bot_status` whenever a bot's status changes, and the quiet FYI once per turn.
pub fn spawn_watcher(state: AppState) -> tokio::task::JoinHandle<()> {
    let mut rx = state.event_sender.subscribe();
    tokio::spawn(async move {
        let mut last: HashMap<String, Value> = HashMap::new();
        let mut dirty: HashSet<String> = HashSet::new();
        let mut tick = tokio::time::interval(std::time::Duration::from_secs(1));
        let mut sweep = tokio::time::interval(WATCH_SWEEP);
        loop {
            tokio::select! {
                e = rx.recv() => match e {
                    Ok(e) => dirty.extend(kwant_of(&e)),
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => dirty.extend(busy(&state, &last)),
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => return,
                },
                _ = sweep.tick() => dirty.extend(busy(&state, &last)),
                _ = tick.tick() => {
                    dirty.extend(state.turns.board(|b| b.take_nudged()));
                    for k in std::mem::take(&mut dirty) {
                        push(&state, &k, &mut last).await;
                    }
                }
            }
        }
    })
}

/// Bots that may change on their own: running turns, gave-up marks, held sends, the last pushed.
fn busy(state: &AppState, last: &HashMap<String, Value>) -> HashSet<String> {
    let mut out: HashSet<String> = state.turns.active_turns().into_iter().map(|t| t.kwant_id).collect();
    out.extend(last.keys().cloned());
    state.turns.board(|b| {
        out.extend(b.marks.iter().filter(|(_, m)| m.gave_up.is_some()).map(|(k, _)| k.clone()));
        out.extend(b.held.iter().map(|h| h.kwant_id.clone()));
    });
    out
}

async fn push(state: &AppState, kwant_id: &str, last: &mut HashMap<String, Value>) {
    let Some(input) = input_of(state, kwant_id).await else { return };
    let now = Utc::now();
    for t in input.turns.iter().filter(|t| fyi_due(t, now)) {
        let first = state.turns.board(|b| match b.signals.get_mut(&t.turn.turn_id) {
            Some(s) if !s.fyi_sent => {
                s.fyi_sent = true;
                true
            }
            _ => false,
        });
        if first {
            let n = super::continuity::turn_quiet(state, kwant_id, Some(t.turn.context_id.as_str())).await;
            super::continuity::notify(state, n).await;
        }
    }
    let status = compute(&input, now);
    let fp = status.fingerprint();
    if last.get(kwant_id) == Some(&fp) {
        return;
    }
    if status.status == "available" {
        // Nothing pushed yet means the client already shows it as available
        if last.remove(kwant_id).is_none() {
            return;
        }
    } else {
        last.insert(kwant_id.to_string(), fp);
    }
    let _ = state.event_sender.send(RuntimeEvent::BotStatus { kwant_id: kwant_id.to_string(), status: json!(status) });
}

// ---------------------------------------------------------------------------------------------
// Routes

/// `GET /api/bots/status?space=`: every bot of the space with its status object.
pub async fn list(State(state): State<AppState>, super::spaces::Scope(space): super::spaces::Scope) -> Response {
    let kwants = match state.db.list_kwants(&space).await {
        Ok(k) => k,
        Err(_) => return StatusCode::INTERNAL_SERVER_ERROR.into_response(),
    };
    let ids: Vec<String> = kwants.into_iter().map(|k| k.id).collect();
    let now = Utc::now();
    let bots: Vec<Status> = inputs(&state, &space, &ids).await.iter().map(|b| compute(b, now)).collect();
    Json(json!({ "bots": bots, "now": now })).into_response()
}

#[derive(Debug, Deserialize)]
pub struct QuietAction {
    pub action: String,
}

/// `POST /api/turns/:turn_id/quiet` `{action: "snooze" | "mute"}`, the person only.
pub async fn quiet(State(state): State<AppState>, headers: HeaderMap, Path(turn_id): Path<String>, Json(body): Json<QuietAction>) -> Response {
    if let Err(r) = super::mcp_suggest::refuse_agent_tokens(&state, &headers, "only the person can snooze a bot's quiet signal").await {
        return r;
    }
    let Some(turn) = state.turns.active_turns().into_iter().find(|t| t.turn_id == turn_id && t.state == "running") else {
        return (StatusCode::NOT_FOUND, Json(json!({ "error": "That turn isn't running." }))).into_response();
    };
    let now = Utc::now();
    let set = |b: &mut Board, f: &dyn Fn(&mut TurnSignals)| {
        f(b.signals.entry(turn_id.clone()).or_default());
        b.nudge(&turn.kwant_id);
    };
    match body.action.as_str() {
        "snooze" => state.turns.board(|b| set(b, &|s| s.snooze_until = Some(now + Span::minutes(SNOOZE_MIN)))),
        "mute" => state.turns.board(|b| set(b, &|s| s.muted = true)),
        _ => return (StatusCode::BAD_REQUEST, Json(json!({ "error": "action is snooze or mute" }))).into_response(),
    }
    Json(json!(status_of(&state, &turn.kwant_id).await)).into_response()
}

/// `POST /api/kwants/:id/status/retry` ("Try again"), the person only: the bot is no longer
/// stuck, so the sends held for that go now.
pub async fn retry(State(state): State<AppState>, headers: HeaderMap, Path(kwant_id): Path<String>) -> Response {
    if let Err(r) = super::mcp_suggest::refuse_agent_tokens(&state, &headers, "only the person can retry a stuck bot").await {
        return r;
    }
    match state.db.get_kwant(&kwant_id).await {
        Ok(Some(_)) => {}
        Ok(None) => return StatusCode::NOT_FOUND.into_response(),
        Err(_) => return StatusCode::INTERNAL_SERVER_ERROR.into_response(),
    }
    let released = state.turns.board(|b| {
        if let Some(m) = b.marks.get_mut(&kwant_id) {
            m.gave_up = None;
        }
        b.nudge(&kwant_id);
        b.held.iter().filter(|h| h.kwant_id == kwant_id && h.reason_code == "stuck").count()
    });
    Json(json!({ "released": released, "status": status_of(&state, &kwant_id).await })).into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(min: i64) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339("2026-11-10T14:00:00Z").unwrap().with_timezone(&Utc) + Span::minutes(min)
    }

    fn running(id: &str, since: DateTime<Utc>) -> TurnInput {
        TurnInput {
            turn: ActiveTurn {
                turn_id: id.into(),
                kwant_id: "k1".into(),
                context_id: "c1".into(),
                thread_id: None,
                state: "running".into(),
                since,
                purpose: None,
            },
            signals: TurnSignals { engine: Some("Claude"), live_steps: Some(true), ..Default::default() },
            wait: None,
            running_tool: None,
            position: 0,
        }
    }

    fn bot(turns: Vec<TurnInput>) -> BotInput {
        BotInput { kwant_id: "k1".into(), turns, questions: Vec::new(), marks: BotMarks::default(), held: Vec::new() }
    }

    fn question(id: &str, ctx: &str, asked: DateTime<Utc>, turn: Option<&str>) -> QuestionRef {
        QuestionRef { id: id.into(), asked_at: asked, context_id: ctx.into(), turn_id: turn.map(str::to_string) }
    }

    #[test]
    fn working_since_the_turn_start_and_waits_on_the_model() {
        let mut t = running("t1", at(0));
        t.signals.last_step_at = Some(at(3));
        let s = compute(&bot(vec![t]), at(4));
        assert_eq!((s.status, s.reason_code), ("working", "thinking"));
        assert_eq!(s.since, Some(at(0)), "Working counts from the turn start, not the last step");
        assert_eq!(s.last_step_at, Some(at(3)));
        assert_eq!(s.waits_on, Some(WaitsOn { kind: "model", label: Some("Claude".into()) }));
        assert_eq!(s.turn.as_ref().unwrap().turn_id, "t1");
        assert!(s.quiet.is_none());
    }

    #[test]
    fn a_running_tool_is_the_reason_and_its_name_the_only_text() {
        let mut t = running("t1", at(0));
        t.running_tool = Some("Bash".into());
        t.signals.last_step_at = Some(at(1));
        let s = compute(&bot(vec![t.clone()]), at(2));
        assert_eq!((s.status, s.reason_code, s.reason_text.as_deref()), ("working", "running_tool", Some("Bash")));
        assert_eq!(s.waits_on.unwrap().kind, "tool");
        t.running_tool = Some("mcp__kwnt__delegate".into());
        let s = compute(&bot(vec![t]), at(2));
        assert_eq!((s.reason_code, s.reason_text), ("waiting_for_bot", None));
    }

    #[test]
    fn quiet_starts_at_5_minutes_without_a_step_and_is_long_from_15() {
        let mut t = running("t1", at(0));
        t.signals.last_step_at = Some(at(10));
        let b = bot(vec![t]);
        let at_min = |m: i64, secs: i64| compute(&b, at(m) + Span::seconds(secs));
        assert_eq!(at_min(14, 59).status, "working", "4 min 59 s is not quiet yet");
        let s = at_min(15, 0);
        assert_eq!((s.status, s.reason_code), ("quiet", "no_step"));
        assert_eq!(s.since, Some(at(10)), "Quiet counts from the last step");
        assert_eq!(s.quiet.as_ref().unwrap().level, "short");
        assert_eq!(at_min(24, 59).quiet.unwrap().level, "short");
        assert_eq!(at_min(25, 0).quiet.unwrap().level, "long");
    }

    #[test]
    fn a_turn_with_no_step_yet_is_quiet_from_its_start() {
        let s = compute(&bot(vec![running("t1", at(0))]), at(5));
        assert_eq!((s.status, s.since), ("quiet", Some(at(0))));
    }

    #[test]
    fn an_end_only_engine_is_quiet_only_at_15_minutes() {
        let mut t = running("t1", at(0));
        t.signals.live_steps = Some(false);
        t.signals.engine = Some("Codex");
        let b = bot(vec![t]);
        let s = compute(&b, at(12));
        assert_eq!((s.status, s.reason_code), ("working", "reports_at_end"));
        assert!(!s.turn.as_ref().unwrap().live_steps);
        let s = compute(&b, at(15));
        assert_eq!((s.status, s.reason_code, s.since), ("quiet", "no_live_steps", Some(at(0))));
        assert_eq!(s.quiet.unwrap().level, "long");
        assert!(!fyi_due(&b.turns[0], at(20)), "no FYI for an engine that is always quiet");
    }

    #[test]
    fn steps_move_the_clock_tools_always_text_every_10_seconds() {
        let mut s = TurnSignals { live_steps: Some(false), ..Default::default() };
        let t0 = at(0);
        s.saw(&RuntimeEvent::TextDelta("a".into()), t0);
        assert_eq!((s.last_step_at, s.live_steps), (Some(t0), Some(true)), "a streamed step proves live steps");
        s.saw(&RuntimeEvent::TextDelta("b".into()), t0 + Span::seconds(9));
        assert_eq!(s.last_step_at, Some(t0), "text within 10 s is batched");
        s.saw(&RuntimeEvent::ToolCallPending { id: "1".into(), name: "Bash".into(), args: json!({}) }, t0 + Span::seconds(9));
        assert_eq!((s.last_step_at, s.last_step_title.as_deref()), (Some(t0 + Span::seconds(9)), Some("Bash")));
        s.saw(&RuntimeEvent::ThinkingDelta("hm".into()), t0 + Span::seconds(20));
        assert_eq!((s.last_step_at, s.last_step_title.as_deref()), (Some(t0 + Span::seconds(20)), None));
        s.saw(&RuntimeEvent::UsageSoFar { fresh: 5 }, t0 + Span::seconds(60));
        assert_eq!(s.last_step_at, Some(t0 + Span::seconds(20)), "usage isn't a step");
    }

    #[test]
    fn snooze_and_mute_hold_quiet_back_for_this_turn() {
        let mut t = running("t1", at(0));
        t.signals.snooze_until = Some(at(35));
        let s = compute(&bot(vec![t.clone()]), at(20));
        assert_eq!(s.status, "working");
        assert_eq!(s.quiet.as_ref().unwrap().snoozed_until, Some(at(35)));
        assert!(!fyi_due(&t, at(20)));
        assert_eq!(compute(&bot(vec![t.clone()]), at(35)).status, "quiet", "the snooze ran out");
        t.signals.snooze_until = None;
        t.signals.muted = true;
        let s = compute(&bot(vec![t]), at(90));
        assert_eq!(s.status, "working");
        assert!(s.quiet.unwrap().muted);
    }

    #[test]
    fn the_fyi_is_due_once_at_15_minutes() {
        let mut t = running("t1", at(0));
        t.signals.last_step_at = Some(at(1));
        assert!(!fyi_due(&t, at(15)));
        assert!(fyi_due(&t, at(16)));
        t.signals.fyi_sent = true;
        assert!(!fyi_due(&t, at(30)));
    }

    #[test]
    fn a_stall_is_stuck_retrying_from_when_it_fired() {
        let mut t = running("t1", at(0));
        t.signals.stalled = Some(Stall { at: at(2), secs: 120 });
        let s = compute(&bot(vec![t.clone()]), at(3));
        assert_eq!((s.status, s.reason_code, s.since), ("stuck", "stalled_retrying", Some(at(2))));
        assert_eq!(s.retry, Some(Retry { at: at(2), attempt: 2, of: 2, stall_secs: 120 }));
        let wire = json!(s);
        assert_eq!(wire["retry"]["try"], 2);
        // The retry answered
        t.signals.saw(&RuntimeEvent::Progress, at(3));
        assert_eq!(compute(&bot(vec![t]), at(3)).status, "working");
    }

    #[test]
    fn a_turn_that_gave_up_keeps_the_bot_stuck_for_30_minutes() {
        let mut b = bot(Vec::new());
        b.marks.gave_up = Some(GaveUp { at: at(0), tries: 2, secs: 120, turn_id: "t1".into(), context_id: "c1".into(), thread_id: None });
        b.marks.last_finished_at = Some(at(0));
        let s = compute(&b, at(29));
        assert_eq!((s.status, s.reason_code, s.since), ("stuck", "stalled_gave_up", Some(at(0))));
        assert_eq!(s.stuck.as_ref().unwrap().tries, 2);
        let s = compute(&b, at(30));
        assert_eq!((s.status, s.reason_code, s.since), ("available", "idle", Some(at(0))));
        assert_eq!(s.last_finished_at, Some(at(0)));
    }

    #[test]
    fn needs_you_reasons_and_since() {
        let mut t = running("t1", at(0));
        t.wait = Some(Wait::Question(question("q1", "c1", at(3), Some("t1"))));
        let s = compute(&bot(vec![t.clone()]), at(30));
        assert_eq!((s.status, s.reason_code, s.since), ("needs_you", "asked_question", Some(at(3))));
        assert_eq!(s.question.as_ref().unwrap().id, "q1");
        assert_eq!(s.waits_on.as_ref().unwrap().kind, "person");
        t.wait = Some(Wait::Approval(at(4)));
        assert_eq!(compute(&bot(vec![t.clone()]), at(30)).reason_code, "wants_ok");
        t.wait = Some(Wait::Browser(at(4)));
        let s = compute(&bot(vec![t]), at(30));
        assert_eq!((s.reason_code, s.since), ("needs_browser", Some(at(4))));
    }

    #[test]
    fn an_open_question_of_an_ended_turn_still_needs_you() {
        let mut b = bot(Vec::new());
        b.questions = vec![question("q1", "c9", at(1), Some("gone"))];
        let s = compute(&b, at(2));
        assert_eq!((s.status, s.reason_code, s.since), ("needs_you", "asked_question", Some(at(1))));
        assert!(s.turn.is_none());
    }

    #[test]
    fn queued_has_its_place_in_line() {
        let mut t = running("t1", at(1));
        t.turn.state = "queued".into();
        t.position = 2;
        t.signals.queue_reason = Some("server_busy".into());
        let s = compute(&bot(vec![t]), at(2));
        assert_eq!((s.status, s.reason_code, s.since), ("queued", "in_line", Some(at(1))));
        assert_eq!(s.queue, Some(Queue { position: 2, reason: "server_busy".into() }));
    }

    #[test]
    fn available_with_nothing_running() {
        let s = compute(&bot(Vec::new()), at(0));
        assert_eq!((s.status, s.reason_code, s.since), ("available", "idle", None));
        let wire = json!(s);
        assert!(wire.get("since").is_none() && wire.get("turn").is_none(), "{wire}");
    }

    #[test]
    fn several_turns_show_the_most_pressing_then_the_newest() {
        let mut quiet = running("t1", at(0));
        quiet.signals.last_step_at = Some(at(0));
        let mut working = running("t2", at(9));
        working.signals.last_step_at = Some(at(9));
        working.turn.context_id = "c2".into();
        let mut newer = running("t3", at(10));
        newer.signals.last_step_at = Some(at(10));
        newer.turn.context_id = "c3".into();
        let mut asked = running("t4", at(1));
        asked.wait = Some(Wait::Approval(at(2)));
        let s = compute(&bot(vec![working.clone(), quiet.clone(), newer.clone()]), at(10));
        assert_eq!((s.status, s.turn.as_ref().unwrap().turn_id.as_str()), ("quiet", "t1"));
        assert_eq!(s.also.iter().map(|a| a.context_id.as_str()).collect::<Vec<_>>(), ["c3", "c2"], "newest first");
        let s = compute(&bot(vec![working, quiet, newer, asked]), at(10));
        assert_eq!(s.status, "needs_you");
        assert_eq!(s.also.len(), MAX_ALSO);
    }

    #[test]
    fn a_push_ignores_step_moves_but_not_status_moves() {
        let mut t = running("t1", at(0));
        t.signals.last_step_at = Some(at(1));
        let a = compute(&bot(vec![t.clone()]), at(2)).fingerprint();
        t.signals.last_step_at = Some(at(2));
        t.signals.last_step_title = Some("Read".into());
        assert_eq!(a, compute(&bot(vec![t.clone()]), at(2)).fingerprint());
        t.running_tool = Some("Read".into());
        assert_ne!(a, compute(&bot(vec![t]), at(2)).fingerprint());
    }

    // H3

    #[test]
    fn a_send_to_a_stuck_bot_is_held() {
        let mut t = running("t1", at(0));
        t.signals.stalled = Some(Stall { at: at(2), secs: 120 });
        let r = hold_reason(&bot(vec![t]), "c5", at(3)).unwrap();
        assert_eq!((r.reason_code, r.since), ("stuck", at(2)));

        let mut b = bot(Vec::new());
        b.marks.gave_up = Some(GaveUp { at: at(0), tries: 2, secs: 120, turn_id: "t1".into(), context_id: "c1".into(), thread_id: None });
        assert_eq!(hold_reason(&b, "c1", at(5)).unwrap().reason_code, "stuck", "gave up, even in its own chat");
        assert!(hold_reason(&b, "c1", at(30)).is_none(), "the gave-up window ended");
    }

    /// 0.9.5: after a stall gave up, the person's Retry in that chat (their message sent again)
    /// resumes it in one press; anywhere else, or a second time, it changes nothing.
    #[test]
    fn the_persons_retry_in_the_stalled_chat_ends_the_gave_up_hold() {
        let gave_up = |thread: Option<&str>| GaveUp { at: at(0), tries: 2, secs: 120, turn_id: "t1".into(), context_id: "c1".into(), thread_id: thread.map(str::to_string) };
        let mut board = Board::default();
        board.marks.entry("k1".into()).or_default().gave_up = Some(gave_up(None));
        assert!(!board.person_retries_in("k1", "c2"), "another chat isn't this stop's Retry");
        assert!(!board.person_retries_in("k2", "c1"), "nor another bot");
        assert!(board.marks["k1"].gave_up.is_some());
        assert!(board.take_nudged().is_empty());
        assert!(board.person_retries_in("k1", "c1"));
        assert!(board.marks["k1"].gave_up.is_none());
        assert!(board.take_nudged().contains("k1"), "the status is pushed again");
        let mut b = bot(Vec::new());
        b.marks = board.marks["k1"].clone();
        assert!(hold_reason(&b, "c1", at(1)).is_none(), "so the resent message isn't held");
        assert!(!board.person_retries_in("k1", "c1"), "nothing left to end");
        // A thread's stop: its thread id is the chat
        board.marks.entry("k1".into()).or_default().gave_up = Some(gave_up(Some("th1")));
        assert!(board.person_retries_in("k1", "th1"));
    }

    #[test]
    fn a_send_to_a_bot_waiting_on_the_person_is_held_unless_it_is_the_answer() {
        let mut t = running("t1", at(0));
        t.wait = Some(Wait::Question(question("q1", "c1", at(3), Some("t1"))));
        let b = bot(vec![t.clone()]);
        let r = hold_reason(&b, "c2", at(4)).unwrap();
        assert_eq!((r.reason_code, r.since), ("waits_on_you", at(3)));
        assert_eq!(r.question.unwrap().id, "q1");
        assert!(hold_reason(&b, "c1", at(4)).is_none(), "where it asked, the message is the answer");
        t.wait = Some(Wait::Approval(at(5)));
        assert_eq!(hold_reason(&bot(vec![t]), "c1", at(6)).unwrap().reason_code, "waits_on_you");
    }

    #[test]
    fn an_old_open_question_or_a_busy_bot_does_not_hold() {
        let mut b = bot(vec![running("t2", at(0))]);
        b.questions = vec![question("q1", "c9", at(1), Some("gone"))];
        assert!(hold_reason(&b, "c2", at(2)).is_none(), "the asking turn ended: a new message starts a turn");
        let mut quiet = running("t3", at(0));
        quiet.signals.last_step_at = Some(at(0));
        assert!(hold_reason(&bot(vec![quiet]), "c2", at(40)).is_none(), "quiet isn't stuck");
    }

    #[test]
    fn the_same_client_id_in_a_chat_goes_once_within_10_minutes() {
        let mut b = Board::default();
        assert!(b.reserve_send("c1|m1", at(0), "msg1").is_ok());
        b.send_saved("c1|m1", "msg1");
        let first = b.reserve_send("c1|m1", at(9), "msg2").unwrap_err();
        assert_eq!(first, Sent { at: at(0), message_id: "msg1".into(), saved: true });
        assert!(b.reserve_send("c2|m1", at(9), "msg3").is_ok(), "another chat is another send");
        assert!(b.reserve_send("c1|m1", at(10), "msg4").is_ok(), "the window is 10 minutes");
    }

    /// S1 (Rigel, #499): a duplicate racing the first send gets the first's reserved message id,
    /// marked as not saved yet, instead of none.
    #[test]
    fn a_racing_duplicate_gets_the_first_sends_reserved_id() {
        let mut b = Board::default();
        assert!(b.reserve_send("c1|m1", at(0), "msg1").is_ok());
        for other in ["msg2", "msg3", "msg4", "msg5", "msg6"] {
            assert_eq!(b.reserve_send("c1|m1", at(0), other), Err(Sent { at: at(0), message_id: "msg1".into(), saved: false }), "in flight");
        }
        b.send_saved("c1|m1", "msg1");
        assert_eq!(b.reserve_send("c1|m1", at(1), "msg7"), Err(Sent { at: at(0), message_id: "msg1".into(), saved: true }));
    }

    #[test]
    fn a_send_that_failed_before_saving_may_go_again() {
        let mut b = Board::default();
        assert!(b.reserve_send("c1|m1", at(0), "msg1").is_ok());
        assert!(b.reserve_send("c1|m1", at(0), "msg2").is_err(), "in flight");
        b.release_send("c1|m1");
        // N1: the duplicate that was told `pending` sends again and goes through, with its own id
        assert!(b.reserve_send("c1|m1", at(0), "msg2").is_ok());
        b.send_saved("c1|m1", "msg2");
        b.release_send("c1|m1");
        assert!(b.reserve_send("c1|m1", at(1), "msg3").is_err(), "a saved send is never released");
    }

    #[test]
    fn client_ids_are_short_and_plain() {
        assert!(valid_client_id("01HF-x_y.z:1"));
        assert!(!valid_client_id(""));
        assert!(!valid_client_id("a b"));
        assert!(!valid_client_id("ü"));
        assert!(!valid_client_id(&"a".repeat(101)));
        assert!(valid_client_id(&"a".repeat(100)));
    }

    #[test]
    fn the_sending_bot_is_told_it_is_queued_not_lost() {
        assert_eq!(
            handoff_held_text("Lin", "waits_on_you"),
            "Lin is waiting for the person to answer, so your request is queued, not lost. Don't send it again. It goes to Lin after the person answers."
        );
        assert!(handoff_held_text("Juno", "stuck").starts_with("Juno is stuck right now"));
    }

    #[test]
    fn the_turn_says_who_started_it_and_leaves_it_out_when_unknown() {
        let mut t = running("t1", at(0));
        let s = compute(&bot(vec![t.clone()]), at(1));
        let turn = serde_json::to_value(&s).unwrap()["turn"].clone();
        assert!(turn.get("source").is_none() && turn.get("source_name").is_none(), "{turn}");

        t.signals.source = Some(TurnSource::person());
        let turn = serde_json::to_value(compute(&bot(vec![t.clone()]), at(1))).unwrap()["turn"].clone();
        assert_eq!(turn["source"], "person");
        assert!(turn.get("source_name").is_none(), "the person is never named: {turn}");

        t.signals.source = Some(TurnSource::bot("Ada"));
        let turn = serde_json::to_value(compute(&bot(vec![t.clone()]), at(1))).unwrap()["turn"].clone();
        assert_eq!((turn["source"].as_str(), turn["source_name"].as_str()), (Some("bot"), Some("Ada")));

        t.signals.source = Some(TurnSource::routine("Morning digest"));
        let turn = serde_json::to_value(compute(&bot(vec![t.clone()]), at(1))).unwrap()["turn"].clone();
        assert_eq!((turn["source"].as_str(), turn["source_name"].as_str()), (Some("routine"), Some("Morning digest")));
    }

    #[test]
    fn a_queued_turn_carries_its_source_too() {
        let mut t = running("t1", at(0));
        t.turn.state = "queued".into();
        t.position = 1;
        t.signals.source = Some(TurnSource::routine("Nightly"));
        let s = compute(&bot(vec![t]), at(1));
        assert_eq!(s.status, "queued");
        assert_eq!(s.turn.as_ref().and_then(|t| t.source), Some("routine"));
    }

    #[test]
    fn a_source_name_is_one_clipped_line_and_a_blank_one_is_left_out() {
        assert_eq!(TurnSource::routine("  Daily\n  report \t now ").name.as_deref(), Some("Daily report now"));
        assert_eq!(TurnSource::bot(&"x".repeat(500)).name.map(|n| n.chars().count()), Some(MAX_SOURCE_NAME_CHARS));
        assert_eq!(TurnSource::routine(" \n ").name, None);
        assert_eq!(TurnSource::routine(" \n ").kind, "routine");
    }
}
