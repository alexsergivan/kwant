//! The scheduler's heartbeat (0.9.53, scheduler-heartbeat). All background work (routines,
//! hand-off follow-ups, parked turns, approval expiry, waiting polls, work-ahead, the idea jar,
//! missions, goals) runs from one loop in [`super::scheduler::spawn`]. If one awaited call in it
//! hangs, or the task panics, that work quietly stops. The loop stamps the start and the end of
//! every pass here, lock-free, and `/api/health` reports it as `scheduler` (a new field only:
//! `"status": "ok"` and HTTP 200 never change, since Docker, Coolify, Render and the Mac app
//! read them and a flip would restart or block a server that still answers).

use chrono::{DateTime, SecondsFormat, TimeZone, Utc};
use serde::Serialize;
use std::sync::atomic::{AtomicBool, AtomicI64, Ordering};
use std::time::Duration;

/// How long the scheduler may go without finishing a pass before it counts as stale: twice its
/// longest sleep ([`super::scheduler::MAX_SLEEP`], 30 s) plus 30 s for a slow pass. A pass only
/// reads and writes the database and hands turns off with `tokio::spawn`, so a healthy one
/// takes well under a second; two missed wake-ups in a row is no longer a slow pass.
pub const STALE_AFTER: Duration = Duration::from_secs(90);

/// How often the watcher looks, so a stale scheduler is logged even when nobody asks `/api/health`.
pub const WATCH_EVERY: Duration = Duration::from_secs(30);

/// Unix milliseconds; 0 means "never".
#[derive(Default)]
pub struct Heartbeat {
    /// When the scheduler task was started; 0 while it isn't running (tests, before `serve`).
    spawned_ms: AtomicI64,
    pass_started_ms: AtomicI64,
    pass_finished_ms: AtomicI64,
    /// Whether the last check found it stale, so a change is logged once, not on every check.
    stale_seen: AtomicBool,
}

/// What `/api/health` shows under `scheduler`.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct HeartbeatStatus {
    /// When the scheduler last finished a full pass (RFC 3339), or null before its first one.
    pub last_tick_at: Option<String>,
    /// When the newest pass started. Later than `last_tick_at` while a pass is running, so a pass
    /// that hangs shows as started and never finished.
    pub pass_started_at: Option<String>,
    /// No finished pass for longer than [`STALE_AFTER`]. Never true before the scheduler starts,
    /// or within [`STALE_AFTER`] of it starting.
    pub stale: bool,
}

fn now_ms() -> i64 {
    Utc::now().timestamp_millis()
}

fn rfc3339(ms: i64) -> Option<String> {
    if ms == 0 {
        return None;
    }
    Utc.timestamp_millis_opt(ms)
        .single()
        .map(|t: DateTime<Utc>| t.to_rfc3339_opts(SecondsFormat::Millis, true))
}

impl Heartbeat {
    /// The scheduler task starts: the stale clock starts from here until its first pass ends.
    pub fn spawned(&self) {
        self.spawned_ms.store(now_ms(), Ordering::Relaxed);
    }

    pub fn pass_started(&self) {
        self.pass_started_ms.store(now_ms(), Ordering::Relaxed);
    }

    pub fn pass_finished(&self) {
        self.pass_finished_ms.store(now_ms(), Ordering::Relaxed);
    }

    /// Test hook: set the stored times (unix ms, 0 = never) to look at stale and recovered states
    /// without waiting for the clock.
    #[doc(hidden)]
    pub fn set_for_tests(&self, spawned_ms: i64, pass_started_ms: i64, pass_finished_ms: i64) {
        self.spawned_ms.store(spawned_ms, Ordering::Relaxed);
        self.pass_started_ms.store(pass_started_ms, Ordering::Relaxed);
        self.pass_finished_ms.store(pass_finished_ms, Ordering::Relaxed);
    }

    /// The status at `now` (unix ms).
    pub fn status_at(&self, now: i64) -> HeartbeatStatus {
        let spawned = self.spawned_ms.load(Ordering::Relaxed);
        let started = self.pass_started_ms.load(Ordering::Relaxed);
        let finished = self.pass_finished_ms.load(Ordering::Relaxed);
        // Before the first pass ends, count from the start of the task, so a startup step that
        // hangs (before the loop) goes stale too
        let since = finished.max(spawned);
        let stale = spawned != 0 && now.saturating_sub(since) > STALE_AFTER.as_millis() as i64;
        HeartbeatStatus { last_tick_at: rfc3339(finished), pass_started_at: rfc3339(started), stale }
    }

    /// The status now; logs one warning when it goes stale and one info line when it recovers.
    pub fn check(&self) -> HeartbeatStatus {
        let status = self.status_at(now_ms());
        self.note(&status);
        status
    }

    /// Logs a change of `stale` once. Returns the new value when it changed.
    fn note(&self, status: &HeartbeatStatus) -> Option<bool> {
        if self.stale_seen.swap(status.stale, Ordering::Relaxed) == status.stale {
            return None;
        }
        let last_tick_at = status.last_tick_at.as_deref().unwrap_or("never");
        if status.stale {
            let running = self.pass_started_ms.load(Ordering::Relaxed) > self.pass_finished_ms.load(Ordering::Relaxed);
            tracing::warn!(
                last_tick_at,
                pass_started_at = status.pass_started_at.as_deref().unwrap_or("never"),
                pass_running = running,
                limit_secs = STALE_AFTER.as_secs(),
                "the scheduler has not finished a pass in time: routines, follow-ups and parked work may have stopped"
            );
        } else {
            tracing::info!(last_tick_at, "the scheduler is finishing passes again");
        }
        Some(status.stale)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const LIMIT: i64 = STALE_AFTER.as_millis() as i64;

    #[test]
    fn a_scheduler_that_never_started_is_not_stale() {
        let h = Heartbeat::default();
        let s = h.status_at(now_ms());
        assert_eq!(s, HeartbeatStatus { last_tick_at: None, pass_started_at: None, stale: false });
    }

    #[test]
    fn startup_is_not_stale_until_the_limit_passes_without_a_finished_pass() {
        let h = Heartbeat::default();
        let t0 = 1_700_000_000_000;
        h.set_for_tests(t0, 0, 0);
        assert!(!h.status_at(t0 + LIMIT).stale, "at the limit it is still starting");
        assert!(h.status_at(t0 + LIMIT + 1).stale, "a startup that never reaches a pass goes stale");
    }

    #[test]
    fn a_pass_that_started_and_never_finished_goes_stale() {
        let h = Heartbeat::default();
        let t0 = 1_700_000_000_000;
        // finished one pass at t0+1s, started the next at t0+31s and never finished it
        h.set_for_tests(t0, t0 + 31_000, t0 + 1_000);
        let s = h.status_at(t0 + 1_000 + LIMIT + 1);
        assert!(s.stale);
        assert!(s.pass_started_at > s.last_tick_at, "the hung pass shows as started after the last finish");
        // A finished pass clears it
        h.set_for_tests(t0, t0 + 31_000, t0 + 1_000 + LIMIT);
        assert!(!h.status_at(t0 + 1_000 + LIMIT + 1).stale);
    }

    #[test]
    fn a_change_is_noted_once_each_way() {
        let h = Heartbeat::default();
        let t0 = 1_700_000_000_000;
        h.set_for_tests(t0, t0, t0);
        let fresh = h.status_at(t0 + 1_000);
        let stale = h.status_at(t0 + LIMIT + 1);
        assert_eq!(h.note(&fresh), None, "fresh from the start: nothing to log");
        assert_eq!(h.note(&stale), Some(true));
        assert_eq!(h.note(&stale), None, "still stale: no second warning");
        assert_eq!(h.note(&fresh), Some(false), "recovered: one info line");
        assert_eq!(h.note(&fresh), None);
    }

    #[test]
    fn times_are_rfc3339_in_utc() {
        assert_eq!(rfc3339(0), None);
        assert_eq!(rfc3339(1_700_000_000_123).as_deref(), Some("2023-11-14T22:13:20.123Z"));
    }
}
