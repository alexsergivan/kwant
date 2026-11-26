//! The scheduler's heartbeat (0.9.53, scheduler-heartbeat). All background work (routines,
//! hand-off follow-ups, parked turns, approval expiry, waiting polls, work-ahead, the idea jar,
//! missions, goals) runs from one loop in [`super::scheduler::spawn`]. If one awaited call in it
//! hangs, or the task panics, that work quietly stops. The loop stamps the start and the end of
//! every pass here, lock-free, and `/api/health` reports it as `scheduler` (a new field only:
//! `"status": "ok"` and HTTP 200 never change, since Docker, Coolify, Render and the Mac app
//! read them and a flip would restart or block a server that still answers).
//!
//! scheduler-step-watchdog (0.9.53): the loop also names the step it is about to await
//! ([`Step`]), so a stall says where it is. A step that runs past [`STEP_SLOW_AFTER`] is logged
//! once (and once more when it ends) and shows in `/api/health` as `scheduler.step`. Log only:
//! nothing is ever cancelled or timed out, because several steps take rows and then act on them
//! (parked, waiting, follow-ups) and dropping them midway would lose work.

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

/// How long one step may run before it is logged as slow. A healthy step takes milliseconds
/// (DB work; real work is `tokio::spawn`ed), the slowest legitimate one is a mission's folder
/// snapshot (seconds at the default `checkpoint_max_mb`). Below [`STALE_AFTER`], so the step is
/// named before or with the stale warning; above [`WATCH_EVERY`]'s 30 s so one late look can't
/// make a short step look slow.
pub const STEP_SLOW_AFTER: Duration = Duration::from_secs(60);

/// Each awaited call of the scheduler task, in the order it runs: the startup steps once, then
/// the pass, then the reads that decide how long to sleep.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum Step {
    StartupFailInterruptedRuns = 1,
    StartupDeleteOrphanRoutines,
    StartupLoadTurnLimits,
    StartupInterruptOpen,
    StartupRescheduleAll,
    Routines,
    FollowUps,
    BudgetRelease,
    Parked,
    ApprovalExpiry,
    Waiting,
    WorkAhead,
    IdeaJar,
    Missions,
    Goals,
    NextDue,
}

impl Step {
    pub const ALL: [Step; 16] = [
        Step::StartupFailInterruptedRuns,
        Step::StartupDeleteOrphanRoutines,
        Step::StartupLoadTurnLimits,
        Step::StartupInterruptOpen,
        Step::StartupRescheduleAll,
        Step::Routines,
        Step::FollowUps,
        Step::BudgetRelease,
        Step::Parked,
        Step::ApprovalExpiry,
        Step::Waiting,
        Step::WorkAhead,
        Step::IdeaJar,
        Step::Missions,
        Step::Goals,
        Step::NextDue,
    ];

    /// The name in logs and `/api/health`: the module of the awaited call.
    pub fn name(self) -> &'static str {
        match self {
            Step::StartupFailInterruptedRuns => "startup:fail_interrupted_runs",
            Step::StartupDeleteOrphanRoutines => "startup:delete_orphan_routines",
            Step::StartupLoadTurnLimits => "startup:load_turn_limits",
            Step::StartupInterruptOpen => "startup:interrupt_open",
            Step::StartupRescheduleAll => "startup:reschedule_all",
            Step::Routines => "routines",
            Step::FollowUps => "follow_ups",
            Step::BudgetRelease => "budget_release",
            Step::Parked => "parked",
            Step::ApprovalExpiry => "approval_expiry",
            Step::Waiting => "waiting",
            Step::WorkAhead => "work_ahead",
            Step::IdeaJar => "idea_jar",
            Step::Missions => "missions",
            Step::Goals => "goals",
            Step::NextDue => "next_due",
        }
    }

    fn from_code(code: i64) -> Option<Step> {
        Step::ALL.into_iter().find(|s| *s as i64 == code)
    }
}

/// The step and when it started live in one atomic, so a reader never pairs one step's name with
/// another's start: the code in the top 16 bits, unix ms in the low 48 (good until the year 10889).
/// 0 = no step running (asleep between passes, or not spawned).
const MS_BITS: u32 = 48;
const MS_MASK: i64 = (1 << MS_BITS) - 1;

fn pack(step: Step, ms: i64) -> i64 {
    ((step as i64) << MS_BITS) | (ms & MS_MASK)
}

fn unpack(v: i64) -> Option<(Step, i64)> {
    Step::from_code(v >> MS_BITS).map(|s| (s, v & MS_MASK))
}

/// Unix milliseconds; 0 means "never".
#[derive(Default)]
pub struct Heartbeat {
    /// When the scheduler task was started; 0 while it isn't running (tests, before `serve`).
    spawned_ms: AtomicI64,
    pass_started_ms: AtomicI64,
    pass_finished_ms: AtomicI64,
    /// The step running now and since when ([`pack`]), 0 while none is.
    step: AtomicI64,
    /// The packed step last logged as slow, 0 when none is, so each slow step is logged once
    /// going slow and once when it ends, not on every check.
    slow_seen: AtomicI64,
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
    /// The step running now (a [`Step::name`]), null while the scheduler sleeps between passes.
    /// When a pass hangs, this is the call it is stuck in.
    pub step: Option<&'static str>,
    /// When `step` started (RFC 3339), null with it.
    pub step_started_at: Option<String>,
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

    /// The scheduler is about to await `step`. Only stamps: never times anything out.
    pub fn step(&self, step: Step) {
        self.step.store(pack(step, now_ms()), Ordering::Relaxed);
    }

    /// No step is running: the scheduler is going to sleep.
    pub fn idle(&self) {
        self.step.store(0, Ordering::Relaxed);
    }

    /// Test hook: a step that started at `started_ms` (unix ms), or none.
    #[doc(hidden)]
    pub fn set_step_for_tests(&self, step: Option<Step>, started_ms: i64) {
        self.step.store(step.map_or(0, |s| pack(s, started_ms)), Ordering::Relaxed);
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
        let step = unpack(self.step.load(Ordering::Relaxed));
        HeartbeatStatus {
            last_tick_at: rfc3339(finished),
            pass_started_at: rfc3339(started),
            stale,
            step: step.map(|(s, _)| s.name()),
            step_started_at: step.and_then(|(_, ms)| rfc3339(ms)),
        }
    }

    /// The status now; logs one warning when it goes stale and one info line when it recovers,
    /// and the same for a step that runs past [`STEP_SLOW_AFTER`].
    pub fn check(&self) -> HeartbeatStatus {
        let now = now_ms();
        self.note_slow_step(now);
        let status = self.status_at(now);
        self.note(&status);
        status
    }

    /// Logs a step that has run past [`STEP_SLOW_AFTER`] once, and once more when it is no
    /// longer running. Returns what it logged, for the tests.
    fn note_slow_step(&self, now: i64) -> Vec<SlowNote> {
        let mut noted = Vec::new();
        let current = self.step.load(Ordering::Relaxed);
        let slow = unpack(current)
            .filter(|(_, started)| now.saturating_sub(*started) > STEP_SLOW_AFTER.as_millis() as i64)
            .map(|_| current)
            .unwrap_or(0);
        // The step logged as slow before has ended (it can't be running again: a new run is a
        // new stamp). compare_exchange so the watcher and /api/health log it once between them.
        let seen = self.slow_seen.load(Ordering::Relaxed);
        if seen != 0 && seen != current && self.slow_seen.compare_exchange(seen, 0, Ordering::Relaxed, Ordering::Relaxed).is_ok() {
            if let Some((step, started)) = unpack(seen) {
                let secs = now.saturating_sub(started) / 1000;
                tracing::info!(
                    step = step.name(),
                    started_at = rfc3339(started).as_deref().unwrap_or("?"),
                    "scheduler step {} has finished (it ran for up to {secs} s)",
                    step.name()
                );
                noted.push(SlowNote::Ended(step));
            }
        }
        if slow != 0 && self.slow_seen.swap(slow, Ordering::Relaxed) != slow {
            if let Some((step, started)) = unpack(slow) {
                let secs = now.saturating_sub(started) / 1000;
                tracing::warn!(
                    step = step.name(),
                    started_at = rfc3339(started).as_deref().unwrap_or("?"),
                    running_secs = secs,
                    limit_secs = STEP_SLOW_AFTER.as_secs(),
                    "scheduler step {} has run for {secs} s",
                    step.name()
                );
                noted.push(SlowNote::Slow(step));
            }
        }
        noted
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
                step = status.step.unwrap_or("none"),
                limit_secs = STALE_AFTER.as_secs(),
                "the scheduler has not finished a pass in time: routines, follow-ups and parked work may have stopped"
            );
        } else {
            tracing::info!(last_tick_at, "the scheduler is finishing passes again");
        }
        Some(status.stale)
    }
}

/// What [`Heartbeat::note_slow_step`] logged.
#[derive(Debug, PartialEq)]
enum SlowNote {
    Slow(Step),
    Ended(Step),
}

#[cfg(test)]
mod tests {
    use super::*;

    const LIMIT: i64 = STALE_AFTER.as_millis() as i64;

    #[test]
    fn a_scheduler_that_never_started_is_not_stale() {
        let h = Heartbeat::default();
        let s = h.status_at(now_ms());
        assert_eq!(
            s,
            HeartbeatStatus { last_tick_at: None, pass_started_at: None, stale: false, step: None, step_started_at: None }
        );
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

    const SLOW: i64 = STEP_SLOW_AFTER.as_millis() as i64;

    #[test]
    fn every_step_has_its_own_name_and_code() {
        let mut names: Vec<_> = Step::ALL.iter().map(|s| s.name()).collect();
        names.sort();
        names.dedup();
        assert_eq!(names.len(), Step::ALL.len());
        for s in Step::ALL {
            assert_eq!(Step::from_code(s as i64), Some(s));
        }
        assert_eq!(Step::from_code(0), None);
        assert_eq!(Step::from_code(Step::ALL.len() as i64 + 1), None);
    }

    #[test]
    fn a_step_stamp_shows_its_name_and_start_and_clears_when_idle() {
        let h = Heartbeat::default();
        let before = now_ms();
        h.step(Step::Missions);
        let s = h.status_at(now_ms());
        assert_eq!(s.step, Some("missions"));
        let started: DateTime<Utc> = s.step_started_at.unwrap().parse().unwrap();
        assert!(started.timestamp_millis() >= before && started.timestamp_millis() <= now_ms());
        h.step(Step::Goals);
        assert_eq!(h.status_at(now_ms()).step, Some("goals"));
        h.idle();
        let s = h.status_at(now_ms());
        assert_eq!((s.step, s.step_started_at), (None, None));
        // Packing keeps the exact millisecond
        assert_eq!(unpack(pack(Step::NextDue, 1_700_000_000_123)), Some((Step::NextDue, 1_700_000_000_123)));
        assert_eq!(unpack(0), None);
    }

    #[test]
    fn a_step_is_slow_only_past_sixty_seconds() {
        let h = Heartbeat::default();
        let t0 = 1_700_000_000_000;
        h.set_step_for_tests(Some(Step::Parked), t0);
        assert_eq!(h.note_slow_step(t0 + SLOW), vec![], "exactly at the limit is not slow");
        assert_eq!(h.note_slow_step(t0 + SLOW + 1), vec![SlowNote::Slow(Step::Parked)]);
    }

    #[test]
    fn a_slow_step_is_noted_once_and_once_more_when_it_ends() {
        let h = Heartbeat::default();
        let t0 = 1_700_000_000_000;
        h.set_step_for_tests(Some(Step::Missions), t0);
        assert_eq!(h.note_slow_step(t0 + SLOW + 1), vec![SlowNote::Slow(Step::Missions)]);
        assert_eq!(h.note_slow_step(t0 + SLOW + 30_000), vec![], "still running: no second warning");
        assert_eq!(h.note_slow_step(t0 + SLOW + 60_000), vec![]);
        // It ends and the loop moves on to a quick step: one info line, nothing about the new step
        h.set_step_for_tests(Some(Step::Goals), t0 + SLOW + 61_000);
        assert_eq!(h.note_slow_step(t0 + SLOW + 61_500), vec![SlowNote::Ended(Step::Missions)]);
        assert_eq!(h.note_slow_step(t0 + SLOW + 62_000), vec![]);
        // Ending in idle works the same
        h.set_step_for_tests(Some(Step::Waiting), t0);
        assert_eq!(h.note_slow_step(t0 + SLOW + 1), vec![SlowNote::Slow(Step::Waiting)]);
        h.idle();
        assert_eq!(h.note_slow_step(t0 + SLOW + 2), vec![SlowNote::Ended(Step::Waiting)]);
        assert_eq!(h.note_slow_step(t0 + SLOW + 3), vec![]);
    }

    #[test]
    fn a_new_run_of_the_same_step_is_a_new_slow_step() {
        let h = Heartbeat::default();
        let t0 = 1_700_000_000_000;
        h.set_step_for_tests(Some(Step::Missions), t0);
        assert_eq!(h.note_slow_step(t0 + SLOW + 1), vec![SlowNote::Slow(Step::Missions)]);
        // The next pass hangs in the same step again before any check saw it end
        let t1 = t0 + 200_000;
        h.set_step_for_tests(Some(Step::Missions), t1);
        assert_eq!(h.note_slow_step(t1 + 1_000), vec![SlowNote::Ended(Step::Missions)], "the old run ended");
        assert_eq!(h.note_slow_step(t1 + SLOW + 1), vec![SlowNote::Slow(Step::Missions)], "the new run is slow too");
    }

    #[test]
    fn a_slow_step_moving_straight_to_another_slow_step_logs_both() {
        let h = Heartbeat::default();
        let t0 = 1_700_000_000_000;
        h.set_step_for_tests(Some(Step::Parked), t0);
        assert_eq!(h.note_slow_step(t0 + SLOW + 1), vec![SlowNote::Slow(Step::Parked)]);
        h.set_step_for_tests(Some(Step::Waiting), t0 + 100_000);
        assert_eq!(
            h.note_slow_step(t0 + 100_000 + SLOW + 1),
            vec![SlowNote::Ended(Step::Parked), SlowNote::Slow(Step::Waiting)]
        );
    }
}
