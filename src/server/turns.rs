//! Running and waiting agent turns: cancellation per job and the per-agent / server-wide caps.
//!
//! A *job* is one dispatch (a user message with its handoffs or discussion, a routine run, a
//! delegation). It has its own cancel token and the keys it answers to: its message id, the chat
//! (channel, kwant or thread id), its team, its delegation. Stop for a key cancels every job that
//! holds it, and a finished job removes only itself, so concurrent turns never clobber each other.
//!
//! Every turn takes a slot from the [`Limiter`]: at most `per_agent` turns per kwant and `global`
//! overall. A turn that doesn't fit waits in FIFO order; it is never turned away. A turn that
//! answers the person also holds its chat: a second message to the same bot in the same chat waits
//! until the first reply ends, so two turns never answer side by side (2026-11-09).

use crate::drivers::events::RuntimeEvent;
use serde::Serialize;
use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::sync::{broadcast, oneshot};
use tokio_util::sync::CancellationToken;

pub const DEFAULT_PER_AGENT: usize = 3;
pub const DEFAULT_GLOBAL: usize = 6;
pub const MAX_PER_AGENT: usize = 16;
pub const MAX_GLOBAL: usize = 64;

/// Setting keys for the caps.
pub const PER_AGENT_SETTING: &str = "max_turns_per_agent";
pub const GLOBAL_SETTING: &str = "max_turns_global";

struct Job {
    keys: Vec<String>,
    token: CancellationToken,
    /// Fresh tokens (see [`crate::drivers::TurnUsage::fresh`]) its turns used so far.
    fresh_tokens: u64,
    /// A reply in it already went through the decide-first gate (see [`super::decide`]).
    decided: bool,
}

/// A turn the UI can show: running, or waiting for a slot.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct ActiveTurn {
    pub turn_id: String,
    pub kwant_id: String,
    pub context_id: String,
    pub thread_id: Option<String>,
    /// "queued" or "running".
    pub state: String,
    pub since: chrono::DateTime<chrono::Utc>,
    /// "decide" for the hidden decide-first round's turns (see [`super::decide`]).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub purpose: Option<String>,
}

#[derive(Default)]
struct Inner {
    jobs: HashMap<String, Job>,
    turns: HashMap<String, ActiveTurn>,
    /// Usage the engine reported so far in a running turn, by turn id (see [`TurnRegistry::book_usage`]).
    meters: HashMap<String, Meter>,
    /// Lead turns that hold their hand-offs' reports, by turn id (see [`TurnRegistry::hold`]).
    holds: HashMap<String, Hold>,
    /// [`HOLD_LIMIT`] unless a test shortened it.
    hold_limit: Option<Duration>,
    /// The pictures of a running turn, by turn id (see [`TurnRegistry::note_screenshot`]).
    pictures: HashMap<String, TurnPictures>,
    /// Why each bot has its status, its held sends and recent client message ids (H4/H3).
    board: super::bot_status::Board,
}

/// A bot's last screenshot in a turn, as the browser returned it.
#[derive(Debug, Clone, PartialEq)]
pub struct Screenshot {
    /// The JPEG, base64.
    pub data: String,
    pub url: String,
    pub width: u32,
    pub height: u32,
}

/// What a turn did with pictures so far: its last screenshot, and whether it put an image into
/// the chat (see [`super::picture_claims`]).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct TurnPictures {
    pub last_shot: Option<Screenshot>,
    pub shared_image: bool,
}

/// How long a lead's turn holds its hand-offs' reports at most (see [`TurnRegistry::hold`]):
/// past it they go to the lead anyway. kwnt sets no limit on a turn's length; one waiting on the
/// person's answer ends within 5 minutes.
pub const HOLD_LIMIT: Duration = Duration::from_secs(600);

/// A lead's turn that started hand-offs with the `delegate` tool.
enum Hold {
    /// Running: the hand-offs that ended meanwhile wait for it, by delegation id.
    Open(Vec<String>),
    /// Past [`HOLD_LIMIT`]: what waited was reported, and the turn holds nothing more.
    Expired,
}

/// One turn's fresh tokens: reported by the engine so far, and already booked onto a task.
#[derive(Default, Clone, Copy)]
struct Meter {
    seen: u64,
    booked: u64,
}

/// Jobs, their cancel tokens and the turns they run.
#[derive(Clone)]
pub struct TurnRegistry {
    inner: Arc<Mutex<Inner>>,
    pub limiter: Limiter,
}

impl Default for TurnRegistry {
    fn default() -> Self {
        Self::new(DEFAULT_PER_AGENT, DEFAULT_GLOBAL)
    }
}

/// Removes its job when dropped, however the job ends.
pub struct JobGuard {
    registry: TurnRegistry,
    pub id: String,
    pub token: CancellationToken,
}

impl Drop for JobGuard {
    fn drop(&mut self) {
        self.registry.finish_job(&self.id);
    }
}

impl TurnRegistry {
    pub fn new(per_agent: usize, global: usize) -> Self {
        Self { inner: Default::default(), limiter: Limiter::new(per_agent, global) }
    }

    /// Registers a job under `keys`; empty keys are skipped.
    pub fn start_job<I, S>(&self, keys: I) -> JobGuard
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        let id = uuid::Uuid::new_v4().to_string();
        let token = CancellationToken::new();
        let mut keys: Vec<String> = keys.into_iter().map(Into::into).filter(|k: &String| !k.is_empty()).collect();
        keys.push(id.clone());
        self.lock().jobs.insert(id.clone(), Job { keys, token: token.clone(), fresh_tokens: 0, decided: false });
        JobGuard { registry: self.clone(), id, token }
    }

    /// Adds keys to a running job (e.g. a delegation id once it exists).
    pub fn add_keys(&self, job_id: &str, keys: &[String]) {
        if let Some(job) = self.lock().jobs.get_mut(job_id) {
            job.keys.extend(keys.iter().cloned());
        }
    }

    /// Adds a finished turn's fresh tokens to its job; returns the job's new total.
    pub fn add_job_tokens(&self, job_id: &str, fresh: u64) -> u64 {
        match self.lock().jobs.get_mut(job_id) {
            Some(job) => {
                job.fresh_tokens = job.fresh_tokens.saturating_add(fresh);
                job.fresh_tokens
            }
            None => 0,
        }
    }

    /// Whether a reply in the job already went through the decide-first gate.
    pub fn job_decided(&self, job_id: &str) -> bool {
        self.lock().jobs.get(job_id).is_some_and(|j| j.decided)
    }

    /// Marks the job as through the decide-first gate.
    pub fn mark_decided(&self, job_id: &str) {
        if let Some(job) = self.lock().jobs.get_mut(job_id) {
            job.decided = true;
        }
    }

    /// Fresh tokens a job's turns used so far (0 for a finished job).
    pub fn job_tokens(&self, job_id: &str) -> u64 {
        self.lock().jobs.get(job_id).map_or(0, |j| j.fresh_tokens)
    }

    fn finish_job(&self, id: &str) {
        self.lock().jobs.remove(id);
    }

    /// Cancels every job holding `key` (or every job when `None`); returns how many.
    pub fn cancel(&self, key: Option<&str>) -> usize {
        let inner = self.lock();
        let mut n = 0;
        for job in inner.jobs.values() {
            if key.is_none_or(|k| job.keys.iter().any(|j| j == k)) {
                job.token.cancel();
                n += 1;
            }
        }
        n
    }

    pub fn has_job(&self, key: &str) -> bool {
        self.lock().jobs.values().any(|j| j.keys.iter().any(|k| k == key))
    }

    /// Every key of the job holding `key`, its turns' ids among them; empty when no job holds it.
    pub fn job_keys_holding(&self, key: &str) -> Vec<String> {
        self.lock().jobs.values().find(|j| j.keys.iter().any(|k| k == key)).map(|j| j.keys.clone()).unwrap_or_default()
    }

    pub fn set_turn(&self, turn: ActiveTurn) {
        let mut inner = self.lock();
        if turn.state == "running" {
            // A new turn ends the bot's gave-up Stuck
            if let Some(m) = inner.board.marks.get_mut(&turn.kwant_id) {
                m.gave_up = None;
            }
        }
        inner.board.nudge(&turn.kwant_id);
        inner.turns.insert(turn.turn_id.clone(), turn);
    }

    /// Marks a known turn "queued" or "running" again (an isolated reply waiting for the VM), keeping
    /// when it started.
    pub fn set_turn_state(&self, turn_id: &str, state: &str) {
        if let Some(t) = self.lock().turns.get_mut(turn_id) {
            t.state = state.into();
        }
    }

    /// Whether `turn_id` is running or waiting for its slot.
    pub fn has_turn(&self, turn_id: &str) -> bool {
        self.lock().turns.contains_key(turn_id)
    }

    pub fn end_turn(&self, turn_id: &str) {
        let mut inner = self.lock();
        let ended = inner.turns.remove(turn_id);
        inner.meters.remove(turn_id);
        inner.pictures.remove(turn_id);
        inner.board.signals.remove(turn_id);
        if let Some(t) = ended {
            if t.state == "running" {
                inner.board.marks.entry(t.kwant_id.clone()).or_default().last_finished_at = Some(chrono::Utc::now());
            }
            inner.board.nudge(&t.kwant_id);
        }
    }

    /// Reads or changes the status board (see [`super::bot_status`]).
    pub fn board<R>(&self, f: impl FnOnce(&mut super::bot_status::Board) -> R) -> R {
        f(&mut self.lock().board)
    }

    /// A streamed event of a running turn: its last step (and that its engine answered).
    pub fn saw_step(&self, turn_id: &str, e: &RuntimeEvent) {
        let mut inner = self.lock();
        if inner.turns.contains_key(turn_id) {
            inner.board.signals.entry(turn_id.to_string()).or_default().saw(e, chrono::Utc::now());
        }
    }

    /// The driver an attempt of the turn runs on: whether it streams steps, and its engine.
    pub fn note_driver(&self, turn_id: &str, live: bool, engine: Option<&'static str>) {
        let mut inner = self.lock();
        if inner.turns.contains_key(turn_id) {
            let s = inner.board.signals.entry(turn_id.to_string()).or_default();
            if s.live_steps != Some(true) {
                s.live_steps = Some(live);
            }
            s.engine = engine.or(s.engine);
        }
    }

    /// The stall watchdog fired and the turn retries now.
    pub fn note_stall(&self, turn_id: &str, kwant_id: &str, secs: u64) {
        let mut inner = self.lock();
        if inner.turns.contains_key(turn_id) {
            inner.board.signals.entry(turn_id.to_string()).or_default().stalled =
                Some(super::bot_status::Stall { at: chrono::Utc::now(), secs });
        }
        inner.board.nudge(kwant_id);
    }

    /// The turn stopped after the stall watchdog fired: its bot shows Stuck for a while.
    pub fn note_gave_up(&self, kwant_id: &str, g: super::bot_status::GaveUp) {
        let mut inner = self.lock();
        inner.board.marks.entry(kwant_id.to_string()).or_default().gave_up = Some(g);
        inner.board.nudge(kwant_id);
    }

    /// Why a queued turn waits ("agent_busy", "server_busy", "vm_busy").
    pub fn note_queue_reason(&self, turn_id: &str, reason: &str) {
        let mut inner = self.lock();
        if inner.turns.contains_key(turn_id) {
            inner.board.signals.entry(turn_id.to_string()).or_default().queue_reason = Some(reason.to_string());
        }
    }

    /// Notes who started a turn (H4 `turn.source`). Called before the turn takes its slot, so a
    /// queued turn has it too; [`TurnRegistry::end_turn`] clears it with the turn's other signals.
    pub fn note_source(&self, turn_id: &str, source: super::bot_status::TurnSource) {
        self.lock().board.signals.entry(turn_id.to_string()).or_default().source = Some(source);
    }

    /// Keeps the turn's newest screenshot (one per turn), for the reply guard.
    /// A turn that already ended keeps nothing.
    pub fn note_screenshot(&self, turn_id: &str, shot: Screenshot) {
        let mut inner = self.lock();
        if inner.turns.contains_key(turn_id) {
            inner.pictures.entry(turn_id.to_string()).or_default().last_shot = Some(shot);
        }
    }

    /// The turn put an image into its chat (shared it, or found it already there).
    pub fn note_image_shared(&self, turn_id: &str) {
        let mut inner = self.lock();
        if inner.turns.contains_key(turn_id) {
            inner.pictures.entry(turn_id.to_string()).or_default().shared_image = true;
        }
    }

    /// The turn's pictures so far, now forgotten.
    pub fn take_pictures(&self, turn_id: &str) -> TurnPictures {
        self.lock().pictures.remove(turn_id).unwrap_or_default()
    }

    /// Holds the reports of the hand-offs that `turn_id`, a running lead turn, starts until it
    /// ends, so the lead hears them all in one turn: one that ended while the lead was still
    /// handing out the next resumed it on its own, and the next one resumed it again (10 runs in
    /// 10 with a 300 ms gap between two `delegate` calls, 2026-11-02). `Some(true)` when this
    /// opened the hold, `Some(false)` when it was open; `None` when the turn isn't running or
    /// is past [`HOLD_LIMIT`].
    pub fn hold(&self, turn_id: &str) -> Option<bool> {
        let mut inner = self.lock();
        if !inner.turns.contains_key(turn_id) {
            return None;
        }
        match inner.holds.get(turn_id) {
            Some(Hold::Open(_)) => Some(false),
            Some(Hold::Expired) => None,
            None => {
                inner.holds.insert(turn_id.to_string(), Hold::Open(Vec::new()));
                Some(true)
            }
        }
    }

    /// Keeps the report of `delegation_id`, which ended, until `turn_id` ends; `false` when that
    /// turn holds nothing.
    pub fn defer(&self, turn_id: &str, delegation_id: &str) -> bool {
        match self.lock().holds.get_mut(turn_id) {
            Some(Hold::Open(held)) => {
                held.push(delegation_id.to_string());
                true
            }
            _ => false,
        }
    }

    /// Whether `turn_id` holds its hand-offs' reports.
    pub fn holding(&self, turn_id: &str) -> bool {
        matches!(self.lock().holds.get(turn_id), Some(Hold::Open(_)))
    }

    /// Ends `turn_id`'s hold as the turn ends: the hand-offs whose reports waited for it.
    pub fn end_hold(&self, turn_id: &str) -> Vec<String> {
        match self.lock().holds.remove(turn_id) {
            Some(Hold::Open(held)) => held,
            _ => Vec::new(),
        }
    }

    /// Ends `turn_id`'s hold at its limit while the turn still runs: what waited goes to the
    /// lead now, and the hand-offs it starts from here on aren't held. `None` once the turn ended.
    pub fn expire_hold(&self, turn_id: &str) -> Option<Vec<String>> {
        let mut inner = self.lock();
        let hold = inner.holds.get_mut(turn_id)?;
        match std::mem::replace(hold, Hold::Expired) {
            Hold::Open(held) => Some(held),
            Hold::Expired => None,
        }
    }

    /// The hold limit (see [`HOLD_LIMIT`]).
    pub fn hold_limit(&self) -> Duration {
        self.lock().hold_limit.unwrap_or(HOLD_LIMIT)
    }

    /// Shortens the hold limit (tests).
    pub fn set_hold_limit(&self, limit: Duration) {
        self.lock().hold_limit = Some(limit);
    }

    /// Notes the fresh tokens a running turn used so far, as its engine reports them.
    pub fn saw_usage(&self, turn_id: &str, fresh: u64) {
        let mut inner = self.lock();
        let m = inner.meters.entry(turn_id.to_string()).or_default();
        m.seen = m.seen.max(fresh);
    }

    /// Fresh tokens `kwant_id`'s running turns with `purpose` reported so far (a Skill Foundry
    /// replay's cap watches this mid-turn).
    pub fn usage_of(&self, kwant_id: &str, purpose: &str) -> u64 {
        let inner = self.lock();
        inner
            .turns
            .values()
            .filter(|t| t.kwant_id == kwant_id && t.purpose.as_deref() == Some(purpose))
            .filter_map(|t| inner.meters.get(&t.turn_id))
            .map(|m| m.seen)
            .sum()
    }

    /// Fresh tokens the engine reported so far in the turn (0 when it reported none).
    pub fn usage_seen(&self, turn_id: &str) -> u64 {
        self.lock().meters.get(turn_id).map_or(0, |m| m.seen)
    }

    /// Fresh tokens of the turn not booked yet, now marked booked: what a task the bot hands in
    /// mid-turn pays for at that moment.
    pub fn book_usage(&self, turn_id: &str) -> u64 {
        let mut inner = self.lock();
        let Some(m) = inner.meters.get_mut(turn_id) else { return 0 };
        let due = m.seen.saturating_sub(m.booked);
        m.booked = m.booked.max(m.seen);
        due
    }

    /// Tokens of the turn already booked mid-turn; forgets the turn's meter. At turn end only
    /// `fresh − booked` is still due, so the turn is counted once.
    pub fn take_booked(&self, turn_id: &str) -> u64 {
        self.lock().meters.remove(turn_id).map_or(0, |m| m.booked)
    }

    /// Running and waiting turns, oldest first.
    pub fn active_turns(&self) -> Vec<ActiveTurn> {
        let mut out: Vec<ActiveTurn> = self.lock().turns.values().cloned().collect();
        out.sort_by(|a, b| a.since.cmp(&b.since).then_with(|| a.turn_id.cmp(&b.turn_id)));
        out
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Takes a slot for one turn of `kwant_id`, waiting in line if none is free. While it waits,
    /// `turn_queued` is sent once. `None` if `cancel` fired first. `one_per_chat`: the turn also
    /// holds its chat (`tag.context_id`) for this bot, so a second message there waits for it.
    pub async fn acquire(
        &self,
        tag: &crate::drivers::events::TurnTag,
        one_per_chat: bool,
        cancel: &CancellationToken,
        events: &broadcast::Sender<RuntimeEvent>,
    ) -> Option<Slot> {
        let chat = one_per_chat.then_some(tag.context_id.as_str());
        let rx = match self.limiter.try_acquire_in(&tag.kwant_id, chat) {
            Ok(slot) => return Some(slot),
            Err(wait) => wait,
        };
        self.set_turn(ActiveTurn {
            turn_id: tag.turn_id.clone(),
            kwant_id: tag.kwant_id.clone(),
            context_id: tag.context_id.clone(),
            thread_id: tag.thread_id.clone(),
            state: "queued".into(),
            since: chrono::Utc::now(),
            purpose: None,
        });
        self.note_queue_reason(&tag.turn_id, rx.reason);
        let _ = events.send(RuntimeEvent::TurnQueued {
            turn_id: tag.turn_id.clone(),
            kwant_id: tag.kwant_id.clone(),
            context_id: Some(tag.context_id.clone()),
            thread_id: tag.thread_id.clone(),
            position: rx.position,
            reason: rx.reason.into(),
            slots: None,
        });
        tokio::select! {
            slot = rx.rx => slot.ok(),
            _ = cancel.cancelled() => None,
        }
    }
}

/// A turn's stall watchdog: silence for longer than the limit stops the turn, except while a
/// tool runs or while the turn waits for a spot in the VM ([`RuntimeEvent::WaitingForRoom`]),
/// which isn't the model's silence. Any event restarts the clock.
#[derive(Debug, Clone)]
pub struct StallClock {
    limit: Option<Duration>,
    last: tokio::time::Instant,
    tools: usize,
    waiting: bool,
}

impl StallClock {
    /// `limit` `None`: never stops the turn.
    pub fn new(limit: Option<Duration>, now: tokio::time::Instant) -> Self {
        Self { limit, last: now, tools: 0, waiting: false }
    }

    pub fn saw(&mut self, e: &RuntimeEvent, now: tokio::time::Instant) {
        self.last = now;
        match e {
            RuntimeEvent::ToolCallPending { .. } => self.tools += 1,
            RuntimeEvent::ToolCallCompleted { .. } => self.tools = self.tools.saturating_sub(1),
            RuntimeEvent::WaitingForRoom { .. } => self.waiting = true,
            RuntimeEvent::RoomTaken => self.waiting = false,
            _ => {}
        }
    }

    /// When the turn counts as stalled, if nothing else arrives; `None` while it can't stall.
    pub fn deadline(&self) -> Option<tokio::time::Instant> {
        self.limit.filter(|_| self.tools == 0 && !self.waiting).map(|d| self.last + d)
    }
}

/// What a turn does when its model goes silent past the stall limit (see [`StallClock`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StallStep {
    /// Start the same attempt again, with the same message and context.
    Retry,
    /// Stop the turn with the stall line.
    Stop,
}

/// A stalled turn is retried once, on its own: only a second stall stops it with the stall line.
/// `retried`: this attempt already is the retry. `acted`: the stalled attempt already ran a tool or
/// raised an approval card, so starting over could do that again; it stops as before.
pub fn after_stall(retried: bool, acted: bool) -> StallStep {
    if retried || acted {
        StallStep::Stop
    } else {
        StallStep::Retry
    }
}

/// The Details line of a turn that was started again after a stall.
pub fn stall_retry_note(secs: u64) -> String {
    format!("Retried after a stall: the model sent nothing for {secs} s, so the reply was started again.")
}

/// The line a stopped turn leaves in the chat.
pub fn stall_stop_line(kwant_name: &str, secs: u64, retried: bool) -> String {
    if retried {
        format!("⚠️ [{kwant_name}] The model sent nothing for {secs} s, twice (it was retried once), so this reply was stopped. Send your message again to retry.")
    } else {
        format!("⚠️ [{kwant_name}] The model sent nothing for {secs} s, so this reply was stopped. Send your message again to retry.")
    }
}

/// One turn's place under the caps; frees it on drop.
pub struct Slot {
    limiter: Limiter,
    kwant_id: String,
    /// The bot-and-chat key it holds, if any (see [`chat_key`]).
    chat: Option<String>,
}

impl Slot {
    /// The slot also holds its bot's chat (a turn taken with `one_per_chat`).
    pub fn holds_chat(&self) -> bool {
        self.chat.is_some()
    }
}

impl Drop for Slot {
    fn drop(&mut self) {
        self.limiter.release(&self.kwant_id, self.chat.as_deref());
    }
}

/// One bot in one chat: at most one turn holds it.
fn chat_key(kwant_id: &str, chat: &str) -> String {
    format!("{kwant_id}|{chat}")
}

/// Waiting for a slot.
pub struct Wait {
    rx: oneshot::Receiver<Slot>,
    pub position: usize,
    /// "agent_busy" or "server_busy".
    pub reason: &'static str,
}

struct Waiter {
    kwant_id: String,
    /// [`chat_key`] of the chat it will hold.
    chat: Option<String>,
    tx: oneshot::Sender<Slot>,
}

struct LimiterState {
    per_agent: usize,
    global: usize,
    running: HashMap<String, usize>,
    total: usize,
    waiting: VecDeque<Waiter>,
    /// [`chat_key`]s held by running turns.
    busy_chats: HashSet<String>,
}

impl LimiterState {
    fn fits(&self, kwant_id: &str, chat: Option<&str>) -> bool {
        self.total < self.global
            && self.running.get(kwant_id).copied().unwrap_or(0) < self.per_agent
            && chat.is_none_or(|c| !self.busy_chats.contains(c))
    }

    fn take(&mut self, kwant_id: &str, chat: Option<&str>) {
        self.total += 1;
        *self.running.entry(kwant_id.to_string()).or_default() += 1;
        if let Some(c) = chat {
            self.busy_chats.insert(c.to_string());
        }
    }

    fn put_back(&mut self, kwant_id: &str, chat: Option<&str>) {
        if let Some(c) = chat {
            self.busy_chats.remove(c);
        }
        self.total = self.total.saturating_sub(1);
        if let Some(n) = self.running.get_mut(kwant_id) {
            *n = n.saturating_sub(1);
            if *n == 0 {
                self.running.remove(kwant_id);
            }
        }
    }
}

/// Per-agent and server-wide caps on running turns, with a FIFO line for the rest. A waiter
/// whose agent is at its cap doesn't hold up waiters for other agents.
#[derive(Clone)]
pub struct Limiter {
    state: Arc<Mutex<LimiterState>>,
}

impl Limiter {
    pub fn new(per_agent: usize, global: usize) -> Self {
        Self {
            state: Arc::new(Mutex::new(LimiterState {
                per_agent: per_agent.clamp(1, MAX_PER_AGENT),
                global: global.clamp(1, MAX_GLOBAL),
                running: HashMap::new(),
                total: 0,
                waiting: VecDeque::new(),
                busy_chats: HashSet::new(),
            })),
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, LimiterState> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// New caps; raising them starts waiting turns right away.
    pub fn set_limits(&self, per_agent: usize, global: usize) {
        let returned = {
            let mut s = self.lock();
            s.per_agent = per_agent.clamp(1, MAX_PER_AGENT);
            s.global = global.clamp(1, MAX_GLOBAL);
            self.dispatch(&mut s)
        };
        drop(returned);
    }

    pub fn limits(&self) -> (usize, usize) {
        let s = self.lock();
        (s.per_agent, s.global)
    }

    /// (running for this kwant, running overall, waiting overall)
    pub fn load(&self, kwant_id: &str) -> (usize, usize, usize) {
        let s = self.lock();
        (s.running.get(kwant_id).copied().unwrap_or(0), s.total, s.waiting.len())
    }

    /// A slot now, or a place in line. Nobody jumps the line for the same agent.
    pub fn try_acquire(&self, kwant_id: &str) -> Result<Slot, Wait> {
        self.try_acquire_in(kwant_id, None)
    }

    /// [`Self::try_acquire`] for a turn that also holds `chat` for this agent: while one turn of
    /// the agent runs in that chat, the next one there waits (in line, "agent_busy") even if the
    /// agent has room. Turns of the agent in other chats, and other agents, aren't held up.
    pub fn try_acquire_in(&self, kwant_id: &str, chat: Option<&str>) -> Result<Slot, Wait> {
        let chat = chat.map(|c| chat_key(kwant_id, c));
        let (tx, mut rx) = oneshot::channel();
        let (result, returned) = {
            let mut s = self.lock();
            let ahead_same = s.waiting.iter().any(|w| w.kwant_id == kwant_id && !w.tx.is_closed());
            let agent_full = s.running.get(kwant_id).copied().unwrap_or(0) >= s.per_agent;
            let chat_busy = chat.as_ref().is_some_and(|c| s.busy_chats.contains(c));
            // Into the line, then hand out what fits in line order: a waiter ahead for the same
            // agent goes first, one held only by its chat doesn't hold up this one
            s.waiting.push_back(Waiter { kwant_id: kwant_id.to_string(), chat, tx });
            let returned = self.dispatch(&mut s);
            let result = match rx.try_recv() {
                Ok(slot) => Ok(slot),
                Err(_) => {
                    let position = s.waiting.iter().filter(|w| !w.tx.is_closed()).count();
                    let reason = if agent_full || ahead_same || chat_busy { "agent_busy" } else { "server_busy" };
                    Err((position, reason))
                }
            };
            (result, returned)
        };
        drop(returned);
        result.map_err(|(position, reason)| Wait { rx, position, reason })
    }

    fn release(&self, kwant_id: &str, chat: Option<&str>) {
        let returned = {
            let mut s = self.lock();
            s.put_back(kwant_id, chat);
            self.dispatch(&mut s)
        };
        // Slots a waiter gave up on are dropped here, outside the lock (their drop releases again)
        drop(returned);
    }

    /// Hands free slots to waiters in line order. Returns slots nobody took any more.
    fn dispatch(&self, s: &mut LimiterState) -> Vec<Slot> {
        let mut unclaimed = Vec::new();
        let mut i = 0;
        while i < s.waiting.len() && s.total < s.global {
            if s.waiting[i].tx.is_closed() {
                s.waiting.remove(i);
                continue;
            }
            let kwant_id = s.waiting[i].kwant_id.clone();
            if !s.fits(&kwant_id, s.waiting[i].chat.as_deref()) {
                i += 1;
                continue;
            }
            let Some(w) = s.waiting.remove(i) else { break };
            s.take(&kwant_id, w.chat.as_deref());
            if let Err(slot) = w.tx.send(Slot { limiter: self.clone(), kwant_id, chat: w.chat }) {
                unclaimed.push(slot);
            }
        }
        unclaimed
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn waiting_for_a_spot_in_the_vm_is_never_a_stall() {
        use crate::drivers::events::RuntimeEvent as E;
        let limit = std::time::Duration::from_secs(120);
        let t0 = tokio::time::Instant::now();
        let mut c = super::StallClock::new(Some(limit), t0);
        assert_eq!(c.deadline(), Some(t0 + limit));
        // Queued behind two other isolated replies: no deadline however long it waits
        c.saw(&E::WaitingForRoom { reason: "vm_busy".into(), position: 1, slots: 2 }, t0);
        assert_eq!(c.deadline(), None);
        // It gets its spot 10 minutes later: the clock starts from then, not from the queueing
        let t1 = t0 + std::time::Duration::from_secs(600);
        c.saw(&E::RoomTaken, t1);
        assert_eq!(c.deadline(), Some(t1 + limit));
        // A running tool still holds the clock, and the watchdog off stays off
        c.saw(&E::ToolCallPending { id: "t".into(), name: "Bash".into(), args: serde_json::json!({}) }, t1);
        assert_eq!(c.deadline(), None);
        c.saw(&E::ToolCallCompleted { id: "t".into(), result: serde_json::json!({}) }, t1);
        assert_eq!(c.deadline(), Some(t1 + limit));
        assert_eq!(super::StallClock::new(None, t0).deadline(), None);
    }

    #[test]
    fn a_turns_pictures_live_until_it_ends() {
        let reg = super::TurnRegistry::default();
        let shot = |url: &str| super::Screenshot { data: "AAAA".into(), url: url.into(), width: 390, height: 844 };
        // A turn that isn't running keeps nothing (a late call after the turn ended)
        reg.note_screenshot("t", shot("https://a.test/"));
        reg.note_image_shared("t");
        assert_eq!(reg.take_pictures("t"), super::TurnPictures::default());
        reg.set_turn(super::ActiveTurn {
            turn_id: "t".into(),
            kwant_id: "k".into(),
            context_id: "c".into(),
            thread_id: None,
            state: "running".into(),
            since: chrono::Utc::now(),
            purpose: None,
        });
        reg.note_screenshot("t", shot("https://a.test/1"));
        reg.note_screenshot("t", shot("https://a.test/2"));
        let p = reg.take_pictures("t");
        assert_eq!(p.last_shot.map(|s| s.url).as_deref(), Some("https://a.test/2"), "the newest screenshot");
        assert!(!p.shared_image);
        assert_eq!(reg.take_pictures("t"), super::TurnPictures::default(), "taken once");
        reg.note_image_shared("t");
        reg.end_turn("t");
        reg.set_turn(super::ActiveTurn {
            turn_id: "t".into(),
            kwant_id: "k".into(),
            context_id: "c".into(),
            thread_id: None,
            state: "running".into(),
            since: chrono::Utc::now(),
            purpose: None,
        });
        assert!(!reg.take_pictures("t").shared_image, "ending the turn forgot them");
    }

    #[test]
    fn usage_of_sums_a_bots_running_turns_with_a_purpose() {
        let reg = super::TurnRegistry::default();
        let turn = |id: &str, kwant: &str, purpose: Option<&str>| super::ActiveTurn {
            turn_id: id.into(),
            kwant_id: kwant.into(),
            context_id: "c".into(),
            thread_id: None,
            state: "running".into(),
            since: chrono::Utc::now(),
            purpose: purpose.map(String::from),
        };
        reg.set_turn(turn("a", "k", Some("foundry")));
        reg.set_turn(turn("b", "k", None));
        reg.set_turn(turn("c", "other", Some("foundry")));
        for (id, n) in [("a", 70), ("b", 1000), ("c", 1000)] {
            reg.saw_usage(id, n);
        }
        assert_eq!(reg.usage_of("k", "foundry"), 70);
        reg.end_turn("a");
        assert_eq!(reg.usage_of("k", "foundry"), 0);
    }

    #[test]
    fn a_turns_source_lasts_from_queued_to_running_and_goes_with_the_turn() {
        use super::super::bot_status::TurnSource;
        let reg = TurnRegistry::default();
        let turn = |state: &str| ActiveTurn {
            turn_id: "t".into(),
            kwant_id: "k".into(),
            context_id: "c".into(),
            thread_id: None,
            state: state.into(),
            since: chrono::Utc::now(),
            purpose: None,
        };
        reg.note_source("t", TurnSource::bot("Ada"));
        reg.set_turn(turn("queued"));
        reg.set_turn(turn("running"));
        let source = reg.board(|b| b.signals.get("t").and_then(|s| s.source.clone()));
        assert_eq!(source, Some(TurnSource::bot("Ada")));
        reg.end_turn("t");
        assert!(reg.board(|b| !b.signals.contains_key("t")));
    }

    #[test]
    fn usage_booked_mid_turn_is_counted_once() {
        let reg = TurnRegistry::default();
        assert_eq!(reg.book_usage("t"), 0, "nothing reported, nothing booked");
        reg.saw_usage("t", 100);
        reg.saw_usage("t", 80); // an older report never shrinks the count
        assert_eq!(reg.usage_seen("t"), 100);
        assert_eq!(reg.book_usage("t"), 100);
        assert_eq!(reg.book_usage("t"), 0, "booking twice books nothing new");
        reg.saw_usage("t", 150);
        assert_eq!(reg.book_usage("t"), 50, "a second hand-in pays only what came since");
        assert_eq!(reg.take_booked("t"), 150, "turn end subtracts what was booked");
        assert_eq!(reg.take_booked("t"), 0, "…and forgets the turn");
        reg.saw_usage("u", 10);
        reg.end_turn("u");
        assert_eq!(reg.book_usage("u"), 0, "an ended turn keeps no meter");
    }

    #[test]
    fn a_hold_keeps_what_ended_until_the_turn_ends_or_passes_its_limit() {
        let reg = TurnRegistry::default();
        assert_eq!(reg.hold("t"), None, "a turn that isn't running holds nothing");
        reg.set_turn(ActiveTurn {
            turn_id: "t".into(),
            kwant_id: "k".into(),
            context_id: "c".into(),
            thread_id: None,
            state: "running".into(),
            since: chrono::Utc::now(),
            purpose: None,
        });
        assert!(!reg.defer("t", "d1"), "nothing waits before the turn's first hand-off");
        assert_eq!(reg.hold("t"), Some(true));
        assert_eq!(reg.hold("t"), Some(false), "opened once");
        assert!(reg.defer("t", "d1") && reg.holding("t"));
        assert_eq!(reg.end_hold("t"), ["d1"]);
        assert!(!reg.holding("t") && reg.end_hold("t").is_empty(), "reported once");
        // Past the limit: what waited goes out once, and nothing the turn starts after it waits
        assert_eq!(reg.hold("t"), Some(true));
        assert!(reg.defer("t", "d2"));
        assert_eq!(reg.expire_hold("t"), Some(vec!["d2".to_string()]));
        assert_eq!(reg.expire_hold("t"), None);
        assert_eq!(reg.hold("t"), None, "an expired hold doesn't open again");
        assert!(!reg.defer("t", "d3") && !reg.holding("t"));
        reg.end_turn("t");
        assert!(reg.end_hold("t").is_empty());
        assert_eq!(reg.hold("t"), None, "nor once the turn ended");
        assert_eq!(reg.expire_hold("t"), None, "a limit that fires after the turn ended finds nothing");
    }

    use super::*;

    #[test]
    fn cancel_by_key_hits_every_job_with_it_and_only_those() {
        let r = TurnRegistry::default();
        let a = r.start_job(["chat-1", "kwant-a"]);
        let b = r.start_job(["chat-1"]);
        let c = r.start_job(["thread-9"]);
        assert_eq!(r.cancel(Some("chat-1")), 2);
        assert!(a.token.is_cancelled() && b.token.is_cancelled());
        assert!(!c.token.is_cancelled());
        drop(a);
        assert!(r.has_job("chat-1"), "b still runs");
        drop(b);
        assert!(!r.has_job("chat-1"));
        assert!(r.has_job("thread-9"));
    }

    #[test]
    fn finished_job_does_not_remove_a_newer_job_for_the_same_chat() {
        let r = TurnRegistry::default();
        let first = r.start_job(["kwant-a"]);
        let second = r.start_job(["kwant-a"]);
        drop(first);
        assert_eq!(r.cancel(Some("kwant-a")), 1);
        assert!(second.token.is_cancelled());
    }

    #[tokio::test]
    async fn per_agent_cap_queues_and_releases_in_order() {
        let l = Limiter::new(2, 10);
        let s1 = l.try_acquire("a").ok().unwrap();
        let _s2 = l.try_acquire("a").ok().unwrap();
        let w3 = l.try_acquire("a").err().unwrap();
        assert_eq!(w3.reason, "agent_busy");
        let w4 = l.try_acquire("a").err().unwrap();
        // Another agent is not held up
        let _b = l.try_acquire("b").ok().unwrap();
        drop(s1);
        let s3 = w3.rx.await.unwrap();
        assert_eq!(l.load("a").0, 2);
        drop(s3);
        let _s4 = w4.rx.await.unwrap();
        assert_eq!(l.load("a"), (2, 3, 0));
    }

    #[tokio::test]
    async fn global_cap_and_abandoned_waiters() {
        let l = Limiter::new(3, 1);
        let s1 = l.try_acquire("a").ok().unwrap();
        let w = l.try_acquire("b").err().unwrap();
        assert_eq!(w.reason, "server_busy");
        let w2 = l.try_acquire("c").err().unwrap();
        drop(w); // gave up (cancelled)
        drop(s1);
        let _s = w2.rx.await.unwrap();
        assert_eq!(l.load("c"), (1, 1, 0));
    }

    #[tokio::test]
    async fn raising_limits_starts_waiters() {
        let l = Limiter::new(1, 6);
        let _s1 = l.try_acquire("a").ok().unwrap();
        let w = l.try_acquire("a").err().unwrap();
        l.set_limits(2, 6);
        let _s2 = w.rx.await.unwrap();
        assert_eq!(l.load("a").0, 2);
    }

    #[tokio::test]
    async fn one_turn_per_bot_per_chat() {
        let l = Limiter::new(3, 10);
        let first = l.try_acquire_in("frontend", Some("chat-1")).ok().unwrap();
        // The person's second message while the first reply runs: it waits, though the bot has room
        let second = l.try_acquire_in("frontend", Some("chat-1")).err().unwrap();
        assert_eq!(second.reason, "agent_busy");
        assert_eq!(second.position, 1);
        // The same bot in another chat, another bot in this chat, and an unchatted turn (a goal
        // check, a routine) all go ahead
        let _other_chat = l.try_acquire_in("frontend", Some("chat-2")).ok().unwrap();
        let _other_bot = l.try_acquire_in("growth", Some("chat-1")).ok().unwrap();
        let _background = l.try_acquire("frontend").ok().unwrap();
        assert_eq!(l.load("frontend"), (3, 4, 1));
        // A third message queues behind the second, in order
        let third = l.try_acquire_in("frontend", Some("chat-1")).err().unwrap();
        assert_eq!(third.position, 2);
        drop(first);
        let second = second.rx.await.unwrap();
        // Only one of them runs: the third still waits for the second
        assert_eq!(l.load("frontend"), (3, 4, 1));
        drop(second);
        let _third = third.rx.await.unwrap();
        assert_eq!(l.load("frontend"), (3, 4, 0));
    }

    #[tokio::test]
    async fn a_chat_waiter_does_not_hold_up_the_bot_elsewhere() {
        let l = Limiter::new(3, 10);
        let _first = l.try_acquire_in("a", Some("c1")).ok().unwrap();
        let _waiting = l.try_acquire_in("a", Some("c1")).err().unwrap();
        // Same bot, other chat: not stuck behind the one waiting for its chat
        assert!(l.try_acquire_in("a", Some("c2")).is_ok());
        // An abandoned waiter (its turn was stopped) frees the chat's line
        let l = Limiter::new(3, 10);
        let first = l.try_acquire_in("a", Some("c1")).ok().unwrap();
        let gave_up = l.try_acquire_in("a", Some("c1")).err().unwrap();
        let next = l.try_acquire_in("a", Some("c1")).err().unwrap();
        drop(gave_up);
        drop(first);
        let _next = next.rx.await.unwrap();
        assert_eq!(l.load("a"), (1, 1, 0));
    }

    /// The bug from 2026-11-09: two messages to one bot in one chat while its first turn ran. The
    /// second now queues (visibly: `turn_queued`, "agent_busy") and starts when the first ends.
    #[tokio::test]
    async fn second_message_to_a_busy_bot_queues_visibly() {
        let reg = TurnRegistry::new(3, 6);
        let (tx, mut events) = broadcast::channel(16);
        let tag = |turn: &str| crate::drivers::events::TurnTag {
            turn_id: turn.into(),
            kwant_id: "frontend".into(),
            context_id: "chat-1".into(),
            thread_id: None,
        };
        let cancel = CancellationToken::new();
        let first = reg.acquire(&tag("t1"), true, &cancel, &tx).await.unwrap();
        let reg2 = std::sync::Arc::new(reg);
        let r = reg2.clone();
        let (tx2, cancel2) = (tx.clone(), cancel.clone());
        let second = tokio::spawn(async move { r.acquire(&tag("t2"), true, &cancel2, &tx2).await.map(|_| ()) });
        match events.recv().await.unwrap() {
            RuntimeEvent::TurnQueued { turn_id, reason, position, context_id, .. } => {
                assert_eq!((turn_id.as_str(), reason.as_str(), position), ("t2", "agent_busy", 1));
                assert_eq!(context_id.as_deref(), Some("chat-1"));
            }
            e => panic!("expected turn_queued, got {e:?}"),
        }
        assert!(reg2.active_turns().iter().any(|t| t.turn_id == "t2" && t.state == "queued"));
        tokio::time::sleep(Duration::from_millis(20)).await;
        assert!(!second.is_finished(), "the second turn started while the first ran");
        drop(first);
        assert_eq!(tokio::time::timeout(Duration::from_secs(5), second).await.unwrap().unwrap(), Some(()));
        // Without one_per_chat (a discussion round, a check) the old behaviour stays: up to the cap
        let a = reg2.acquire(&tag("t3"), false, &cancel, &tx).await;
        let b = reg2.acquire(&tag("t4"), false, &cancel, &tx).await;
        assert!(a.is_some() && b.is_some());
    }

    /// P7 (2026-11-09): a stalled turn retries exactly once, and the stop line comes only after
    /// the second stall. Walks the attempts loop's decisions for one account.
    #[test]
    fn a_stall_retries_once_and_stops_only_after_a_second_stall() {
        let run = |stalls: usize| {
            let (mut retried, mut steps) = (false, Vec::new());
            for _ in 0..stalls {
                let step = after_stall(retried, false);
                steps.push(step);
                match step {
                    StallStep::Retry => retried = true,
                    StallStep::Stop => break,
                }
            }
            steps
        };
        // One stall: retried, no stop line
        assert_eq!(run(1), vec![StallStep::Retry]);
        // Two stalls: one retry, then the stop
        assert_eq!(run(2), vec![StallStep::Retry, StallStep::Stop]);
        // However many stalls follow, it retries exactly once
        assert_eq!(run(5), vec![StallStep::Retry, StallStep::Stop]);
        // A stall after a tool ran or a card was raised isn't started over (it could act twice)
        assert_eq!(after_stall(false, true), StallStep::Stop);
        assert!(stall_retry_note(120).starts_with("Retried after a stall"));
        assert!(stall_stop_line("Frontend", 120, true).contains("twice"));
        assert!(!stall_stop_line("Frontend", 120, false).contains("twice"));
    }
}
