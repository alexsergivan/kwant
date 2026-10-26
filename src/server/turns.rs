//! Running and waiting agent turns: cancellation per job and the per-agent / server-wide caps.
//!
//! A *job* is one dispatch (a user message with its handoffs or discussion, a routine run, a
//! delegation). It has its own cancel token and the keys it answers to: its message id, the chat
//! (channel, kwant or thread id), its team, its delegation. Stop for a key cancels every job that
//! holds it, and a finished job removes only itself, so concurrent turns never clobber each other.
//!
//! Every turn takes a slot from the [`Limiter`]: at most `per_agent` turns per kwant and `global`
//! overall. A turn that doesn't fit waits in FIFO order; it is never turned away.

use crate::drivers::events::RuntimeEvent;
use serde::Serialize;
use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex};
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
}

#[derive(Default)]
struct Inner {
    jobs: HashMap<String, Job>,
    turns: HashMap<String, ActiveTurn>,
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
        self.lock().jobs.insert(id.clone(), Job { keys, token: token.clone(), fresh_tokens: 0 });
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

    pub fn set_turn(&self, turn: ActiveTurn) {
        self.lock().turns.insert(turn.turn_id.clone(), turn);
    }

    pub fn end_turn(&self, turn_id: &str) {
        self.lock().turns.remove(turn_id);
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
    /// `turn_queued` is sent once. `None` if `cancel` fired first.
    pub async fn acquire(
        &self,
        tag: &crate::drivers::events::TurnTag,
        cancel: &CancellationToken,
        events: &broadcast::Sender<RuntimeEvent>,
    ) -> Option<Slot> {
        let rx = match self.limiter.try_acquire(&tag.kwant_id) {
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
        });
        let _ = events.send(RuntimeEvent::TurnQueued {
            turn_id: tag.turn_id.clone(),
            kwant_id: tag.kwant_id.clone(),
            context_id: Some(tag.context_id.clone()),
            thread_id: tag.thread_id.clone(),
            position: rx.position,
            reason: rx.reason.into(),
        });
        tokio::select! {
            slot = rx.rx => slot.ok(),
            _ = cancel.cancelled() => None,
        }
    }
}

/// One turn's place under the caps; frees it on drop.
pub struct Slot {
    limiter: Limiter,
    kwant_id: String,
}

impl Drop for Slot {
    fn drop(&mut self) {
        self.limiter.release(&self.kwant_id);
    }
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
    tx: oneshot::Sender<Slot>,
}

struct LimiterState {
    per_agent: usize,
    global: usize,
    running: HashMap<String, usize>,
    total: usize,
    waiting: VecDeque<Waiter>,
}

impl LimiterState {
    fn fits(&self, kwant_id: &str) -> bool {
        self.total < self.global && self.running.get(kwant_id).copied().unwrap_or(0) < self.per_agent
    }

    fn take(&mut self, kwant_id: &str) {
        self.total += 1;
        *self.running.entry(kwant_id.to_string()).or_default() += 1;
    }

    fn put_back(&mut self, kwant_id: &str) {
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
        let mut s = self.lock();
        let ahead_same = s.waiting.iter().any(|w| w.kwant_id == kwant_id && !w.tx.is_closed());
        if !ahead_same && s.fits(kwant_id) {
            s.take(kwant_id);
            return Ok(Slot { limiter: self.clone(), kwant_id: kwant_id.to_string() });
        }
        let agent_full = s.running.get(kwant_id).copied().unwrap_or(0) >= s.per_agent;
        let (tx, rx) = oneshot::channel();
        s.waiting.push_back(Waiter { kwant_id: kwant_id.to_string(), tx });
        let position = s.waiting.iter().filter(|w| !w.tx.is_closed()).count();
        Err(Wait { rx, position, reason: if agent_full || ahead_same { "agent_busy" } else { "server_busy" } })
    }

    fn release(&self, kwant_id: &str) {
        let returned = {
            let mut s = self.lock();
            s.put_back(kwant_id);
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
            if !s.fits(&kwant_id) {
                i += 1;
                continue;
            }
            let Some(w) = s.waiting.remove(i) else { break };
            s.take(&kwant_id);
            if let Err(slot) = w.tx.send(Slot { limiter: self.clone(), kwant_id }) {
                unclaimed.push(slot);
            }
        }
        unclaimed
    }
}

#[cfg(test)]
mod tests {
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
}
