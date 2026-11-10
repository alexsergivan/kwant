//! Goals (docs/mission-control.md §22): what should stay true, checked on a schedule. There is no
//! setting: goals show with the Mission board, and nothing runs until the person creates or
//! approves one, so an upgrade starts no background work. Quiet by default (plan B.0): a goal
//! tells the person only when its state changes, at most [`NOTICE_CAP`] times in 24 hours.

pub mod predicate;
pub mod routes;
pub mod runner;
pub mod tool;

use crate::db::goals::{Goal, GoalCheck, GoalEvent, NewGoal, NewGoalCheck, Observation};
use crate::db::missions::now_ms;
use crate::db::spaces::SpaceId;
use crate::drivers::events::RuntimeEvent;
use crate::server::continuity::{notify, notify_on, Notification};
use crate::server::AppState;
use predicate::Predicate;
use serde::Deserialize;
use serde_json::{json, Value};

pub const MODES: [&str; 3] = ["watch", "suggest", "fix"];
pub const INTERVAL_RANGE: (i64, i64) = (15, 1440);
pub const DEFAULT_INTERVAL: i64 = 60;
pub const CONFIRM_RANGE: (i64, i64) = (1, 3);
/// Consecutive failing runs before a goal is called broken (plan B.0: a single failure is never news).
pub const DEFAULT_CONFIRM: i64 = 2;
pub const FIXES_RANGE: (i64, i64) = (1, 10);
pub const DEFAULT_MAX_FIXES: i64 = 3;
pub const PRIORITY_RANGE: (i64, i64) = (1, 3);
pub const DEFAULT_PRIORITY: i64 = 2;
pub const MAX_CHECKS: usize = 10;
pub const MAX_BUDGET: i64 = 10_000_000;
/// Freshness, in minutes: at most a week.
pub const FRESHNESS_RANGE: (i64, i64) = (1, 10_080);
/// Notices per goal in any rolling 24 hours (plan B.0 rule 1).
pub const NOTICE_CAP: i64 = 3;
pub const DAY_MS: i64 = 24 * 60 * 60 * 1000;
/// Unapproved drafts are deleted silently after this (plan B.0 rule 4).
pub const DRAFT_TTL_MS: i64 = 7 * DAY_MS;
/// Runs in a row where every check errored before a met goal is called unknown: one blip is not news.
pub const ERRORS_TO_UNKNOWN: i64 = 2;
/// A goal unknown this long gets its one "can't check" notice.
pub const UNKNOWN_NOTICE_MS: i64 = DAY_MS;
/// Statuses that mean an incident is open: further failures, or a run that can't check, change nothing.
pub const INCIDENT: [&str; 4] = ["not_met", "fixing", "needs_you", "waiting"];

/// A check as the person (or `goal_propose`) writes it.
#[derive(Debug, Clone, Deserialize)]
pub struct CheckInput {
    #[serde(default)]
    pub label: String,
    pub kind: String,
    #[serde(default)]
    pub spec: Value,
    pub metric: Option<String>,
    pub operator: Option<String>,
    /// Text or a number.
    #[serde(default)]
    pub target: Value,
    /// `required` (default) or `advisory`.
    pub severity: Option<String>,
    /// The v1 spelling of severity.
    pub required: Option<bool>,
    pub freshness_minutes: Option<i64>,
}

fn in_range(v: i64, (lo, hi): (i64, i64)) -> bool {
    (lo..=hi).contains(&v)
}

/// `v` if given and in range, the default if not given.
pub fn ranged(v: Option<i64>, range: (i64, i64), default: i64, what: &str) -> Result<i64, String> {
    match v {
        None => Ok(default),
        Some(n) if in_range(n, range) => Ok(n),
        Some(_) => Err(format!("{what} must be {}–{}", range.0, range.1)),
    }
}

/// Checks a goal's checks and fills in their defaults: a predicate for each typed kind, severity,
/// and freshness (twice the interval). At least one must be required, since only required checks
/// decide the goal. `has_folder`: command and file checks run in the bot's folder, so they need one.
pub fn validate_checks(input: &[CheckInput], interval: i64, has_folder: bool) -> Result<Vec<NewGoalCheck>, String> {
    if input.is_empty() {
        return Err("a goal needs at least one check".into());
    }
    if input.len() > MAX_CHECKS {
        return Err(format!("a goal has at most {MAX_CHECKS} checks"));
    }
    let mut out = Vec::new();
    for (i, c) in input.iter().enumerate() {
        let n = i + 1;
        let kind = c.kind.trim();
        if !predicate::metrics(kind).is_empty() || matches!(kind, "file_exists" | "file_contains" | "file_sha256") {
        } else {
            return Err(format!("check {n}: kind is one of command, http, file_exists, file_contains, file_sha256"));
        }
        if !c.spec.is_object() {
            return Err(format!("check {n}: spec must be an object"));
        }
        if kind != "http" && !has_folder {
            return Err(format!("check {n}: {kind} checks run in a bot's folder, so the goal needs a bot"));
        }
        // The person approves the goal, so an http check may reach this computer (plan B.4)
        crate::server::missions::validate_check(kind, &c.spec, true).map_err(|e| format!("check {n}: {e}"))?;
        let target = match &c.target {
            Value::Null => None,
            Value::String(s) => Some(s.clone()),
            Value::Number(x) => Some(x.to_string()),
            Value::Bool(b) => Some(b.to_string()),
            _ => return Err(format!("check {n}: target is text or a number")),
        };
        let pred = predicate::normalize(kind, &c.spec, c.metric.as_deref(), c.operator.as_deref(), target.as_deref()).map_err(|e| format!("check {n}: {e}"))?;
        let severity = match (c.severity.as_deref().map(str::trim), c.required) {
            (Some("required"), _) | (None, Some(true) | None) => "required",
            (Some("advisory"), _) | (None, Some(false)) => "advisory",
            (Some(other), _) => return Err(format!("check {n}: severity is required or advisory, not {other}")),
        };
        let freshness = ranged(c.freshness_minutes, FRESHNESS_RANGE, (interval * 2).min(FRESHNESS_RANGE.1), "freshness_minutes")
            .map_err(|e| format!("check {n}: {e}"))?;
        let label = match c.label.trim() {
            "" => predicate::says(kind, &c.spec, pred.as_ref()),
            l => l.to_string(),
        };
        out.push(NewGoalCheck {
            label: label.chars().take(200).collect(),
            kind: kind.to_string(),
            spec: c.spec.clone(),
            metric: pred.as_ref().map(|p| p.metric.clone()),
            operator: pred.as_ref().and_then(|p| p.operator.clone()),
            target: pred.as_ref().map(|p| p.target.clone()),
            severity: severity.into(),
            freshness_minutes: freshness,
        });
    }
    if !out.iter().any(|c| c.severity == "required") {
        return Err("at least one check must be required: advisory checks never decide a goal".into());
    }
    Ok(out)
}

/// A goal as the person (or `goal_propose`) writes it.
#[derive(Debug, Clone, Deserialize)]
pub struct GoalInput {
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub description: String,
    /// Its bot (id or name); the space's first bot if left out.
    pub kwant_id: Option<String>,
    pub mode: Option<String>,
    pub interval_minutes: Option<i64>,
    pub confirm_failures: Option<i64>,
    pub budget_tokens: Option<i64>,
    pub max_fixes_per_day: Option<i64>,
    pub priority: Option<i64>,
    #[serde(default)]
    pub checks: Vec<CheckInput>,
    /// "Make this a goal" from a task: Juno's route (B.2), refused here until it lands.
    pub from_task: Option<String>,
}

pub fn valid_title(t: &str) -> Result<String, String> {
    let t = t.trim();
    if t.is_empty() || t.chars().count() > 200 {
        return Err("title must be 1–200 characters".into());
    }
    Ok(t.to_string())
}

pub fn valid_mode(m: Option<&str>, default: &str) -> Result<String, String> {
    match m.map(str::trim) {
        None => Ok(default.into()),
        Some(m) if MODES.contains(&m) => Ok(m.into()),
        Some(_) => Err("mode is watch, suggest or fix".into()),
    }
}

pub fn valid_budget(b: Option<i64>) -> Result<i64, String> {
    match b {
        None => Ok(0),
        Some(n) if (0..=MAX_BUDGET).contains(&n) => Ok(n),
        Some(_) => Err(format!("budget_tokens must be 0–{MAX_BUDGET}")),
    }
}

/// Checks a goal and fills in its defaults. `created_by`: "person" or the proposing bot's id;
/// `draft`: a bot's proposal, waiting for the person.
pub async fn build_goal(state: &AppState, space: &SpaceId, p: &GoalInput, created_by: &str, draft: bool) -> Result<NewGoal, String> {
    let title = valid_title(&p.title)?;
    if p.description.chars().count() > 2000 {
        return Err("description is at most 2000 characters".into());
    }
    let mode = valid_mode(p.mode.as_deref(), "suggest")?;
    let interval = ranged(p.interval_minutes, INTERVAL_RANGE, DEFAULT_INTERVAL, "interval_minutes")?;
    let confirm = ranged(p.confirm_failures, CONFIRM_RANGE, DEFAULT_CONFIRM, "confirm_failures")?;
    let fixes = ranged(p.max_fixes_per_day, FIXES_RANGE, DEFAULT_MAX_FIXES, "max_fixes_per_day")?;
    let priority = ranged(p.priority, PRIORITY_RANGE, DEFAULT_PRIORITY, "priority")?;
    let budget = valid_budget(p.budget_tokens)?;
    let bot = match p.kwant_id.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
        Some(who) => Some(crate::server::missions::bot_in_space(state, space, who).await.ok_or("kwant_id isn't a bot in this space")?),
        None => state.db.list_kwants(space).await.ok().and_then(|b| b.into_iter().next()),
    };
    let checks = validate_checks(&p.checks, interval, bot.is_some())?;
    let kwant_id = bot.map(|b| b.id).unwrap_or_default();
    Ok(NewGoal {
        space_id: space.as_str().to_string(),
        title,
        description: p.description.trim().to_string(),
        kwant_id,
        // Empty: its bot's folder, found when it runs
        folder: String::new(),
        mode,
        interval_minutes: interval,
        confirm_failures: confirm,
        budget_tokens: budget,
        max_fixes_per_day: fixes,
        priority,
        created_by: created_by.into(),
        draft,
        checks,
    })
}

/// The stored predicate of a check; None for the file kinds.
pub fn predicate_of(c: &GoalCheck) -> Option<Predicate> {
    Predicate::from_json(&json!({ "metric": c.metric, "operator": c.operator, "target": c.target }))
}

/// What one run of a goal said, from its checks' verdicts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RunSays {
    /// Every required check is met.
    Met,
    /// A required check is not met.
    Failed,
    /// No required check failed, but one couldn't say (an error or unreadable output).
    Unknown,
}

/// One check's verdict in a run.
#[derive(Debug, Clone, Copy)]
pub struct Judged {
    pub required: bool,
    pub verdict: &'static str,
    /// The check didn't run or got no answer.
    pub errored: bool,
}

/// The goal after a run: `(status, fail_streak, error_runs, says)`.
///
/// - Met resets both counters and the goal is met.
/// - Failed adds one to the streak. Only at `confirm` failures in a row does the goal become
///   not_met; before that it keeps its status (a met goal stays met: one failure is never news).
/// - Unknown is not a failure and breaks a run of failures: an error between two failures doesn't
///   confirm one (G-Q-02; the safer reading of Q14). The goal shows unknown.
/// - While an incident is open (not_met, fixing, needs_you, waiting), failures and unknown runs
///   keep its status, so a goal is never "broken" twice for one incident.
/// - A met goal whose checks all errored once (a dropped connection, a timeout) stays met if the run
///   before passed; only [`ERRORS_TO_UNKNOWN`] such runs in a row make it unknown. An error right
///   after a failure shows unknown at once.
/// - Advisory checks never decide.
pub fn decide(prior: &str, fail_streak: i64, error_runs: i64, confirm: i64, results: &[Judged]) -> (String, i64, i64, RunSays) {
    let required: Vec<&Judged> = results.iter().filter(|j| j.required).collect();
    let says = if required.iter().any(|j| j.verdict == predicate::NOT_MET) {
        RunSays::Failed
    } else if !required.is_empty() && required.iter().all(|j| j.verdict == predicate::MET) {
        RunSays::Met
    } else {
        RunSays::Unknown
    };
    let open = INCIDENT.contains(&prior);
    match says {
        RunSays::Met => ("met".into(), 0, 0, says),
        RunSays::Failed => {
            let streak = fail_streak + 1;
            let status = if streak >= confirm && !open { "not_met" } else { prior };
            (status.into(), streak, 0, says)
        }
        RunSays::Unknown => {
            let all_errored = !results.is_empty() && results.iter().all(|j| j.errored);
            let errors = if all_errored { error_runs + 1 } else { 0 };
            let status = if open || holds_blip(prior, fail_streak, all_errored, errors) { prior } else { "unknown" };
            (status.into(), 0, errors, says)
        }
    }
}

/// Whether a run where every check errored leaves a met goal met: only when the run before passed
/// (no failure streak) and it is not yet [`ERRORS_TO_UNKNOWN`] errors in a row. After a failure an
/// error shows unknown, so fail, error, fail, error... can't keep a goal met for ever.
pub fn holds_blip(prior: &str, fail_streak: i64, all_errored: bool, errors: i64) -> bool {
    prior == "met" && fail_streak == 0 && all_errored && errors < ERRORS_TO_UNKNOWN
}

/// Minutes to the next check: the interval, doubled for each run past the third where every check
/// errored, up to a day.
pub fn next_interval_minutes(interval: i64, error_runs: i64) -> i64 {
    if error_runs < 3 {
        return interval;
    }
    let doublings = (error_runs - 2).min(10) as u32;
    interval.saturating_mul(1 << doublings).min(1440)
}

/// Up to 10% of `minutes`, in ms, drawn fresh each time so goals made together spread out.
pub fn jitter_ms(minutes: i64) -> i64 {
    let span = minutes * 60_000 / 10;
    if span <= 0 {
        return 0;
    }
    (uuid::Uuid::new_v4().as_u128() % span as u128) as i64
}

fn parse(text: &str) -> Value {
    serde_json::from_str(text).unwrap_or(Value::Null)
}

/// A check with its newest observation. An observation at or past `expires_at` is out of date: its
/// verdict reads unknown, whatever it was.
pub fn check_json(c: &GoalCheck, latest: Option<&Observation>, now: i64, excerpt_chars: usize) -> Value {
    let spec = c.spec_value();
    let pred = predicate_of(c);
    let latest = latest.map(|o| {
        let expired = o.expires_at <= now;
        json!({
            "status": o.status,
            "verdict": if expired { predicate::UNKNOWN } else { o.verdict.as_str() },
            "expired": expired,
            "observed_value": o.observed_value,
            "excerpt": o.excerpt.chars().take(excerpt_chars).collect::<String>(),
            "observed_at": o.observed_at,
            "expires_at": o.expires_at,
        })
    });
    json!({
        "id": c.id,
        "label": c.label,
        "kind": c.kind,
        "spec": spec,
        "metric": c.metric,
        "operator": c.operator,
        "target": c.target,
        "severity": c.severity,
        "required": c.required(),
        "freshness_minutes": c.freshness_minutes,
        "says": predicate::says(&c.kind, &spec, pred.as_ref()),
        "latest": latest,
    })
}

/// The status to show: a met goal whose required check is out of date (or never ran) is unknown,
/// never met.
pub fn shown_status(g: &Goal, checks: &[GoalCheck], latest: &[Observation], now: i64) -> (String, String) {
    if g.status == "met" {
        let stale = checks.iter().filter(|c| c.required()).any(|c| latest.iter().find(|o| o.check_id == c.id).is_none_or(|o| o.expires_at <= now));
        if stale {
            return ("unknown".into(), "out_of_date".into());
        }
    }
    (g.status.clone(), g.status_reason.clone())
}

pub fn goal_fields(g: &Goal, status: &str, reason: &str) -> Value {
    json!({
        "id": g.id,
        "space_id": g.space_id,
        "title": g.title,
        "description": g.description,
        "kwant_id": g.kwant_id,
        "folder": g.folder,
        "status": status,
        "status_reason": reason,
        "mode": g.mode,
        "priority": g.priority,
        "interval_minutes": g.interval_minutes,
        "confirm_failures": g.confirm_failures,
        "budget_tokens": g.budget_tokens,
        "max_fixes_per_day": g.max_fixes_per_day,
        "created_by": g.created_by,
        "approved_by": g.approved_by,
        "approved_at": g.approved_at,
        "paused_at": g.paused_at,
        "next_check_at": g.next_check_at,
        "last_checked_at": g.last_checked_at,
        "checking_since": g.checking_since,
        "fail_streak": g.fail_streak,
        "unknown_since": g.unknown_since,
        "version": g.version,
        "created_at": g.created_at,
        "updated_at": g.updated_at,
    })
}

/// A goal with its checks and their newest observations. `excerpt_chars` keeps list rows small.
pub async fn goal_json(state: &AppState, g: &Goal, excerpt_chars: usize) -> Value {
    let now = now_ms();
    let checks = state.db.goal_checks(&g.id).await.unwrap_or_default();
    let latest = state.db.recent_observations(&g.id, 1).await.unwrap_or_default();
    let (status, reason) = shown_status(g, &checks, &latest, now);
    let mut v = goal_fields(g, &status, &reason);
    v["checks"] = checks.iter().map(|c| check_json(c, latest.iter().find(|o| o.check_id == c.id), now, excerpt_chars)).collect();
    // What remediation adds: `waiting_for`, `diagnosis`, `open_mission`
    if let (Some(obj), Some(more)) = (v.as_object_mut(), crate::server::goals_remediation::extras(state, g).await.as_object()) {
        for (k, val) in more {
            obj.insert(k.clone(), val.clone());
        }
    }
    v
}

pub fn event_json(e: &GoalEvent) -> Value {
    json!({ "id": e.id, "kind": e.kind, "actor": e.actor, "detail": parse(&e.detail), "created_at": e.created_at })
}

/// Tells open boards the goal changed (`status` "deleted" when it is gone).
pub fn emit(state: &AppState, id: &str, status: &str, space_id: &str) {
    let _ = state.event_sender.send(RuntimeEvent::GoalUpdated { id: id.into(), status: status.into(), space_id: space_id.into() });
}

pub fn emit_goal(state: &AppState, g: &Goal) {
    emit(state, &g.id, &g.status, &g.space_id);
}

/// Sends one goal notice (`kind`: broken, recovered, cant_check, needs_you) unless the goal
/// already had [`NOTICE_CAP`] in the last 24 hours. A capped notice is logged, never sent: the
/// cap hides pings, not state. Returns whether it went out; a notice the person turned off in
/// Settings is neither sent nor recorded, so it never counts toward the cap.
pub async fn notice(state: &AppState, g: &Goal, kind: &str, title: String, body: String) -> bool {
    // With goal alerts off in Settings nothing is sent, so nothing is recorded or counted
    if !notify_on(state).await.iter().any(|k| k == "goal_alert") {
        return false;
    }
    let now = now_ms();
    let sent = state.db.goal_notices_since(&g.id, now - DAY_MS).await.unwrap_or(0);
    if sent >= NOTICE_CAP {
        let _ = state.db.add_goal_event(&g.id, "notice_capped", "kwant", &json!({ "kind": kind })).await;
        return false;
    }
    let bot = match state.db.get_kwant(&g.kwant_id).await.ok().flatten() {
        Some(k) => Some(k),
        None => state.db.list_kwants(&SpaceId::new(&g.space_id)).await.ok().and_then(|ks| ks.into_iter().next()),
    };
    let Some(bot) = bot else {
        return false;
    };
    let _ = state.db.add_goal_event(&g.id, "notified", "kwant", &json!({ "kind": kind })).await;
    notify(state, Notification { kind: "goal_alert", kwant_id: bot.id, kwant_name: bot.name, channel_id: None, title, body, thread_id: None, card_id: None }).await;
    true
}

fn clock(ms: i64) -> String {
    chrono::DateTime::from_timestamp_millis(ms).map(|t| t.format("%H:%M UTC").to_string()).unwrap_or_default()
}

/// The notices a run's change of state earns (plan B.0 rule 1): broken when an incident opens,
/// recovered when a goal the person was told about is met again. `owed_recovery` is read before
/// the run wrote its `met` event. Remediation (B.2) adds one decision notice once the diagnosis is in.
pub async fn on_change(state: &AppState, before: &str, after: &Goal, failing: &[String], owed_recovery: bool) {
    if after.status == "not_met" && !INCIDENT.contains(&before) {
        // Goals of one folder that break together are one incident: the first one's notice is the notice
        if crate::server::goals_remediation::grouped_with_told(state, after).await {
            let _ = state.db.add_goal_event(&after.id, "grouped", "kwant", &json!({})).await;
            return;
        }
        let what = failing.first().cloned().unwrap_or_else(|| "a check".into());
        let body = format!("Since {}, this isn't true: {what}. kwant checked {} times in a row. Open the Mission board to see what it saw.", clock(now_ms()), after.confirm_failures.max(1));
        notice(state, after, "broken", format!("{} is failing", after.title), body).await;
    } else if after.status == "met" && before != "met" && owed_recovery {
        notice(state, after, "recovered", format!("{} is OK again", after.title), "Its checks pass again. Nothing else to do.".into()).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn j(required: bool, verdict: &'static str) -> Judged {
        Judged { required, verdict, errored: verdict == predicate::UNKNOWN }
    }

    const MET: &str = predicate::MET;
    const NOT: &str = predicate::NOT_MET;
    const UNK: &str = predicate::UNKNOWN;

    /// Runs `seq` from a fresh goal; returns the statuses after each run.
    fn walk(confirm: i64, seq: &[&[Judged]]) -> Vec<String> {
        let (mut status, mut streak, mut errs) = ("checking".to_string(), 0, 0);
        let mut out = Vec::new();
        for r in seq {
            let (s, st, e, _) = decide(&status, streak, errs, confirm, r);
            (status, streak, errs) = (s, st, e);
            out.push(status.clone());
        }
        out
    }

    #[test]
    fn one_failure_is_never_news_and_two_confirm() {
        let ok: &[Judged] = &[j(true, MET)];
        let bad: &[Judged] = &[j(true, NOT)];
        // G-V-02, G-V-03
        assert_eq!(walk(2, &[ok, bad]), ["met", "met"]);
        assert_eq!(walk(2, &[ok, bad, bad]), ["met", "met", "not_met"]);
        // G-V-04: a pass resets the streak
        assert_eq!(walk(2, &[ok, bad, ok, bad]), ["met", "met", "met", "met"]);
        // G-V-09: confirm 1 and 3
        assert_eq!(walk(1, &[ok, bad]), ["met", "not_met"]);
        assert_eq!(walk(3, &[ok, bad, bad, bad]), ["met", "met", "met", "not_met"]);
        // A new goal is never met before a pass (G-V-01)
        assert_eq!(walk(2, &[bad]), ["checking"]);
    }

    #[test]
    fn an_error_is_unknown_and_never_confirms_a_failure() {
        let ok: &[Judged] = &[j(true, MET)];
        let bad: &[Judged] = &[j(true, NOT)];
        let err: &[Judged] = &[j(true, UNK)];
        // G-T-05, G-V-05, G-Q-02: fail, error, fail doesn't confirm (the error after a failure shows unknown)
        assert_eq!(walk(2, &[ok, bad, err, bad]), ["met", "met", "unknown", "unknown"]);
        assert_eq!(walk(2, &[ok, bad, err, err, bad]), ["met", "met", "unknown", "unknown", "unknown"]);
        // api-M7: one error on a met goal changes nothing; two in a row are unknown; a pass between resets
        assert_eq!(walk(2, &[ok, err]), ["met", "met"]);
        assert_eq!(walk(2, &[ok, err, err]), ["met", "met", "unknown"]);
        assert_eq!(walk(2, &[ok, err, ok, err]), ["met", "met", "met", "met"]);
        // A goal that never met has nothing to keep
        assert_eq!(walk(2, &[err]), ["unknown"]);
        // Output the predicate couldn't read isn't a dropped connection: unknown at once
        let unreadable = [Judged { required: true, verdict: UNK, errored: false }];
        assert_eq!(decide("met", 0, 0, 2, &unreadable).0, "unknown");
        // Once confirmed, an error during the incident keeps it open: no second "broken"
        assert_eq!(walk(2, &[ok, bad, bad, err, bad]), ["met", "met", "not_met", "not_met", "not_met"]);
        // Recovery closes it
        assert_eq!(walk(2, &[bad, bad, ok]), ["checking", "not_met", "met"]);
    }

    #[test]
    fn alternating_fail_and_error_never_leaves_a_goal_met() {
        let ok: &[Judged] = &[j(true, MET)];
        let bad: &[Judged] = &[j(true, NOT)];
        let err: &[Judged] = &[j(true, UNK)];
        // A blip is held only when the run before passed. After a failure the error shows unknown,
        // and the fail/error pattern can't keep the goal met for ever.
        for confirm in [2, 3] {
            let seq: Vec<&[Judged]> = std::iter::once(ok).chain((0..4).flat_map(|_| [bad, err])).collect();
            let seen = walk(confirm, &seq);
            assert_eq!(seen[0], "met");
            assert_eq!(seen[1], "met", "one failure is not news");
            assert!(seen[2..].iter().all(|s| s == "unknown"), "confirm {confirm}: fail, error, fail, error... gave {seen:?}");
        }
        // A pass between the failures does hold a blip: fail, pass, error, fail is still met
        assert_eq!(walk(2, &[ok, bad, ok, err, bad]), ["met", "met", "met", "met", "met"]);
    }

    #[test]
    fn advisory_checks_never_decide() {
        // G-T-07, G-V-07, G-V-08
        let adv_fails: &[Judged] = &[j(true, MET), j(false, NOT)];
        let adv_unknown: &[Judged] = &[j(true, MET), j(false, UNK)];
        let one_unknown: &[Judged] = &[j(true, MET), j(true, UNK)];
        assert_eq!(decide("met", 0, 0, 1, adv_fails).3, RunSays::Met);
        assert_eq!(decide("met", 0, 0, 1, adv_unknown).3, RunSays::Met);
        assert_eq!(decide("met", 0, 0, 1, one_unknown).0, "unknown");
        assert_eq!(decide("met", 0, 0, 1, &[j(false, MET)]).0, "unknown");
    }

    #[test]
    fn the_backoff_counts_only_runs_where_everything_errored() {
        let all_err: &[Judged] = &[j(true, UNK), j(false, UNK)];
        let partly = [Judged { required: true, verdict: UNK, errored: false }, j(false, MET)];
        assert_eq!(decide("unknown", 0, 2, 2, all_err).2, 3);
        assert_eq!(decide("unknown", 0, 2, 2, &partly).2, 0);
        assert_eq!(decide("unknown", 0, 5, 2, &[j(true, MET)]).2, 0);
        // G-V-10
        assert_eq!(next_interval_minutes(60, 2), 60);
        assert_eq!(next_interval_minutes(60, 3), 120);
        assert_eq!(next_interval_minutes(60, 4), 240);
        assert_eq!(next_interval_minutes(60, 40), 1440);
        for _ in 0..50 {
            assert!((0..360_000).contains(&jitter_ms(60)));
        }
    }

    fn input(v: Value) -> Vec<CheckInput> {
        serde_json::from_value(v).unwrap()
    }

    #[test]
    fn checks_are_validated_with_their_defaults() {
        let c = validate_checks(&input(json!([{"kind": "http", "spec": {"url": "https://example.com"}}])), 60, false).unwrap();
        assert_eq!((c[0].metric.as_deref(), c[0].operator.as_deref(), c[0].target.as_deref()), (Some("status"), Some("eq"), Some("200")));
        assert_eq!((c[0].severity.as_str(), c[0].freshness_minutes), ("required", 120));
        assert_eq!(c[0].label, "https://example.com answers with status 200");
        let c = validate_checks(&input(json!([{"kind": "http", "spec": {"url": "https://e.x"}, "metric": "latency_ms", "operator": "lt", "target": 500}])), 15, false).unwrap();
        assert_eq!(c[0].target.as_deref(), Some("500"));
        // G-T-07: only advisory is refused; G-T-08
        for (bad, why) in [
            (json!([{"kind": "http", "spec": {"url": "https://e.x"}, "severity": "advisory"}]), "at least one check must be required"),
            (json!([{"kind": "http", "spec": {"url": "https://e.x"}, "metric": "latency_ms", "operator": "eq", "target": 5}]), "latency_ms takes"),
            (json!([{"kind": "http", "spec": {"url": "https://e.x"}, "metric": "stdout_number", "target": 5}]), "metric is one of"),
            (json!([{"kind": "http", "spec": {"url": "https://e.x"}, "metric": "latency_ms"}]), "needs a target"),
            (json!([{"kind": "command", "spec": {"command": "true"}}]), "needs a bot"),
            (json!([{"kind": "ftp", "spec": {}}]), "kind is one of"),
            (json!([{"kind": "http", "spec": {"url": "https://e.x"}, "severity": "maybe"}]), "severity is required or advisory"),
            (json!([{"kind": "http", "spec": {"url": "https://e.x"}, "freshness_minutes": 0}]), "freshness_minutes"),
            (json!([]), "at least one check"),
        ] {
            let e = validate_checks(&input(bad.clone()), 60, false).unwrap_err();
            assert!(e.contains(why), "{bad}: {e}");
        }
        assert!(validate_checks(&input(json!([{"kind": "command", "spec": {"command": "true"}}])), 60, true).is_ok());
    }

    fn obs(check: &str, expires_at: i64) -> Observation {
        Observation {
            id: 1,
            goal_id: "g".into(),
            check_id: check.into(),
            status: "passed".into(),
            verdict: "met".into(),
            observed_value: Some("200".into()),
            excerpt: "x".repeat(50),
            observed_at: 0,
            expires_at,
        }
    }

    fn check(id: &str, severity: &str) -> GoalCheck {
        GoalCheck {
            id: id.into(),
            goal_id: "g".into(),
            position: 0,
            label: "l".into(),
            kind: "http".into(),
            spec: r#"{"url":"https://e.x"}"#.into(),
            metric: Some("status".into()),
            operator: Some("eq".into()),
            target: Some("200".into()),
            severity: severity.into(),
            freshness_minutes: 120,
        }
    }

    #[test]
    fn an_out_of_date_observation_is_unknown_never_met() {
        // G-T-06: expired at the boundary too
        let v = check_json(&check("c", "required"), Some(&obs("c", 1000)), 1000, 10);
        assert_eq!((v["latest"]["verdict"].as_str(), v["latest"]["expired"].as_bool()), (Some("unknown"), Some(true)));
        assert_eq!(v["latest"]["excerpt"].as_str().map(str::len), Some(10));
        let v = check_json(&check("c", "required"), Some(&obs("c", 1001)), 1000, 10);
        assert_eq!(v["latest"]["verdict"], "met");
    }
}
