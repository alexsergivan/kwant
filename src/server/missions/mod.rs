//! Mission Control + Verified Outcomes (Phase A). Contract: docs/mission-control.md.
//!
//! One task universe: planned mission tasks, the person's tasks, hand-offs, routine runs, Loose
//! Ends taken on and work-ahead runs are all rows in `tasks`. The rules:
//!
//! - a task is `verified` only through a passing outcome check that isn't its owner's own claim
//!   ([`verdict`], plus a SQLite trigger);
//! - a task is `ready` only when its mission is active and its dependencies are verified;
//! - claims are atomic leases that expire ([`crate::db::Database::claim_task`]);
//! - `task_events` is append-only (triggers);
//! - a claim never starts past the mission's budget;
//! - nothing here widens a bot's permissions: the bot tools only touch the board, `command` checks
//!   a bot adds run only after the person approves them, and person actions are person-only routes.

pub mod checkpoints;
pub mod checks;
pub mod graph;
pub mod listener;
pub mod routes;
pub mod runner;
pub mod tools;

use super::AppState;
use crate::db::missions::{Mission, NewCheck, NewTask, OutcomeCheck, Task, TaskChange};
use crate::db::models::{MessageExtra, NewMessage};
use crate::db::spaces::SpaceId;
use crate::db::DbError;
use crate::drivers::events::RuntimeEvent;
use serde_json::{json, Value};

/// Setting key: `"true"` turns Mission Control on. Off by default.
pub const SETTING: &str = "mission_board";
pub const LEASE_SETTING: &str = "mission_lease_minutes";
pub const BUDGET_SETTING: &str = "mission_default_budget_tokens";
pub const DEFAULT_LEASE_MINUTES: i64 = 30;
pub const LEASE_RANGE: (i64, i64) = (5, 240);
pub const DEFAULT_BUDGET: i64 = 200_000;
pub const BUDGET_RANGE: (i64, i64) = (1_000, 5_000_000);
/// A task's budget when none is given (capped at its mission's budget).
pub const DEFAULT_TASK_BUDGET: i64 = 50_000;
pub const MAX_PLAN_TASKS: usize = 30;
pub const DEFAULT_MAX_ATTEMPTS: i64 = 3;
/// Longest stored check output.
pub const MAX_EXCERPT: usize = 2_000;

pub async fn enabled(state: &AppState) -> bool {
    matches!(state.db.get_setting(SETTING).await.ok().flatten().as_deref().map(str::trim), Some("true" | "1" | "on"))
}

pub(crate) async fn int_setting(state: &AppState, key: &str, range: (i64, i64), default: i64) -> i64 {
    state
        .db
        .get_setting(key)
        .await
        .ok()
        .flatten()
        .and_then(|v| v.trim().parse::<i64>().ok())
        .filter(|n| (range.0..=range.1).contains(n))
        .unwrap_or(default)
}

pub async fn lease_minutes(state: &AppState) -> i64 {
    int_setting(state, LEASE_SETTING, LEASE_RANGE, DEFAULT_LEASE_MINUTES).await
}

pub async fn lease_ms(state: &AppState) -> i64 {
    lease_minutes(state).await * 60_000
}

pub async fn default_budget(state: &AppState) -> i64 {
    int_setting(state, BUDGET_SETTING, BUDGET_RANGE, DEFAULT_BUDGET).await
}

/// Why the mission settings in a `POST /api/settings` are refused, if they are.
pub fn settings_error(lease: Option<i64>, budget: Option<i64>) -> Option<String> {
    if lease.is_some_and(|m| !(LEASE_RANGE.0..=LEASE_RANGE.1).contains(&m)) {
        return Some(format!("mission_lease_minutes must be {}–{}", LEASE_RANGE.0, LEASE_RANGE.1));
    }
    if budget.is_some_and(|n| !(BUDGET_RANGE.0..=BUDGET_RANGE.1).contains(&n)) {
        return Some(format!("mission_default_budget_tokens must be {}–{}", BUDGET_RANGE.0, BUDGET_RANGE.1));
    }
    None
}

/// Who moves a task.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Actor {
    /// The bot that holds (owns) the task.
    Owner,
    /// The person, through a person-only route.
    Person,
    /// kwant itself: checks, leases, hand-offs, routine runs, cards.
    Kwant,
}

/// Whether `actor` may move a task from `from` to `to`. Conditions beyond the pair (live claim,
/// dependencies, passing checks, budget) are checked where the move happens and again by the
/// SQLite triggers.
pub fn allowed(from: &str, to: &str, actor: Actor) -> bool {
    use Actor::*;
    let open = matches!(from, "backlog" | "ready" | "working" | "needs_you" | "review" | "blocked" | "failed");
    match (from, to) {
        ("backlog", "ready") => actor == Kwant,
        ("ready", "working") => true,
        ("working", "review") => matches!(actor, Owner | Kwant),
        ("working", "ready") => matches!(actor, Owner | Kwant),
        ("working", "needs_you") | ("needs_you", "working") => actor == Kwant,
        // A hand-off or routine run that failed or stopped
        ("working" | "needs_you", "failed") => actor == Kwant,
        ("needs_you", "ready") => actor == Kwant,
        ("backlog" | "ready" | "working" | "needs_you", "blocked") => true,
        ("blocked", "backlog") => matches!(actor, Owner | Person),
        ("review", "verified") => actor == Kwant,
        ("review", "ready") => matches!(actor, Kwant | Person),
        ("review", "failed") => actor == Kwant,
        ("failed", "ready") => actor == Person,
        (_, "cancelled") => open && matches!(actor, Person | Kwant),
        _ => false,
    }
}

/// Why a move didn't happen.
#[derive(Debug)]
pub enum MoveError {
    NotAllowed(String),
    /// The task wasn't in `from` any more (someone else moved it).
    Stale,
    Missing,
    Db(DbError),
}

impl std::fmt::Display for MoveError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            MoveError::NotAllowed(s) => write!(f, "{s}"),
            MoveError::Stale => write!(f, "the task changed meanwhile; look at it again"),
            MoveError::Missing => write!(f, "no such task"),
            MoveError::Db(DbError::Sqlx(e)) => write!(f, "refused: {}", trigger_message(&e.to_string())),
            MoveError::Db(e) => write!(f, "{e}"),
        }
    }
}

/// The plain sentence of a trigger's `RAISE(ABORT, …)`.
fn trigger_message(e: &str) -> String {
    for known in ["append-only", "verified only", "live lease", "ready only", "is final", "before work"] {
        if e.contains(known) {
            return e.rsplit(": ").next().unwrap_or(e).to_string();
        }
    }
    e.to_string()
}

/// Moves `task_id` from `from` to `to` as `actor` (logged as `actor_id`), then tells the board
/// and does what follows: dependents become ready, a mission whose tasks are all done is done.
pub async fn transition(
    state: &AppState,
    task_id: &str,
    from: &str,
    to: &str,
    actor: Actor,
    actor_id: &str,
    change: TaskChange,
) -> Result<Task, MoveError> {
    if !allowed(from, to, actor) {
        return Err(MoveError::NotAllowed(format!("a task can't go from {from} to {to} that way")));
    }
    if to == "verified" {
        let task = state.db.get_task(task_id).await.map_err(MoveError::Db)?.ok_or(MoveError::Missing)?;
        let checks = state.db.list_checks(task_id).await.map_err(MoveError::Db)?;
        let since = review_since(state, task_id).await;
        if verdict(&task, &checks, since) != Verdict::Pass {
            return Err(MoveError::NotAllowed("a task is verified only by a passing check that isn't its owner's".into()));
        }
    }
    let moved = state.db.move_task(task_id, from, to, actor_id, &change).await.map_err(MoveError::Db)?;
    if !moved {
        return Err(if state.db.get_task(task_id).await.ok().flatten().is_some() { MoveError::Stale } else { MoveError::Missing });
    }
    let task = state.db.get_task(task_id).await.map_err(MoveError::Db)?.ok_or(MoveError::Missing)?;
    emit_task(state, &task).await;
    // The run's claim was released: snapshot what it left (after the commit, never inside it)
    if matches!(from, "working" | "needs_you") && !matches!(to, "working" | "needs_you") {
        checkpoints::after_run(state, &task).await;
    }
    after_move(state, &task, to).await;
    Ok(task)
}

async fn after_move(state: &AppState, task: &Task, to: &str) {
    let Some(mid) = task.mission_id.clone() else {
        if matches!(to, "verified" | "failed") {
            task_chat_line(state, task, None).await;
        }
        if to == "verified" {
            for t in state.db.dependents_promotable(&task.id).await.unwrap_or_default() {
                to_ready(state, &t.id).await;
            }
        }
        return;
    };
    let mission = state.db.get_mission(&mid).await.ok().flatten();
    if matches!(to, "verified" | "failed") {
        task_chat_line(state, task, mission.as_ref()).await;
    }
    if to == "verified" || to == "cancelled" {
        promote(state, &mid).await;
        maybe_finish(state, &mid).await;
    }
    emit_mission(state, &mid).await;
}

/// Moves the mission's backlog tasks whose dependencies are all verified to ready.
pub async fn promote(state: &AppState, mission_id: &str) {
    let Ok(Some(m)) = state.db.get_mission(mission_id).await else { return };
    if m.status != "active" {
        return;
    }
    for t in state.db.promotable(mission_id).await.unwrap_or_default() {
        to_ready(state, &t.id).await;
    }
}

async fn to_ready(state: &AppState, task_id: &str) {
    let change = TaskChange { reason: Some("nothing it waits on is open".into()), ..Default::default() };
    if state.db.move_task(task_id, "backlog", "ready", "kwant", &change).await.unwrap_or(false) {
        emit_task_id(state, task_id).await;
    }
}

/// A backlog task outside a draft mission, with every dependency verified, becomes ready.
pub async fn promote_one(state: &AppState, task: &Task) {
    match &task.mission_id {
        Some(m) => promote(state, m).await,
        None => {
            let deps = state.db.dependencies_of(&task.id).await.unwrap_or_default();
            if task.state == "backlog" && deps.iter().all(|d| d.1 == "verified") {
                to_ready(state, &task.id).await;
            }
        }
    }
}

/// A mission whose tasks are all verified or cancelled, at least one verified, is done.
pub async fn maybe_finish(state: &AppState, mission_id: &str) {
    let counts = state.db.mission_counts(mission_id).await.unwrap_or_default();
    let verified = counts.iter().find(|(s, _)| s == "verified").map_or(0, |c| c.1);
    let open = counts.iter().filter(|(s, _)| s != "verified" && s != "cancelled").map(|c| c.1).sum::<i64>();
    if verified == 0 || open > 0 {
        return;
    }
    if state.db.set_mission_status(mission_id, "active", "done", Some("kwant")).await.unwrap_or(false) {
        if let Ok(Some(m)) = state.db.get_mission(mission_id).await {
            mission_chat_line(state, &m, format!("Mission \"{}\" is done: {verified} task(s) verified.", m.title)).await;
        }
    }
}

// Verification

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Verdict {
    /// Every required check passed and one passing check isn't the owner's.
    Pass,
    /// A required check failed or couldn't run.
    Fail(String),
    /// Not yet: no check, or a required one hasn't run.
    Waiting,
}

/// Checks kwant re-runs each time the task goes to review; peer reviews and the person's verify
/// count only when given since then (`since`, RFC 3339 of the last move to review).
fn fresh(c: &OutcomeCheck, since: Option<chrono::DateTime<chrono::Utc>>) -> bool {
    if crate::db::missions::KWANT_CHECK_KINDS.contains(&c.kind.as_str()) {
        return true;
    }
    match (since, c.finished_at) {
        (Some(s), Some(f)) => f >= s,
        (None, _) => true,
        (Some(_), None) => false,
    }
}

/// Whether `c` can count toward verifying `task`: approved, and not the owner's own check unless
/// the person approved it, on its own or as part of the mission's plan (a bot that becomes the
/// owner after adding a check doesn't get it free).
pub fn counts(task: &Task, c: &OutcomeCheck) -> bool {
    let own = task.owner_kwant_id.as_deref().is_some_and(|o| o == c.requested_by);
    c.approved && (!own || matches!(c.approved_by.as_deref(), Some("person" | "mission")))
}

/// The rule for `review → verified`, on the task's checks: every required check that counts has
/// passed (none failed, pending or waiting for approval) and at least one of them is independent
/// of the owner. A required check waiting for approval holds the task in review, unless the
/// person verified it themselves. Optional checks are shown but never gate or verify.
pub fn verdict(task: &Task, checks: &[OutcomeCheck], since: Option<chrono::DateTime<chrono::Utc>>) -> Verdict {
    let owner = task.owner_kwant_id.as_deref().unwrap_or("");
    let required: Vec<&OutcomeCheck> = checks.iter().filter(|c| c.required).collect();
    let live: Vec<&OutcomeCheck> = required.iter().copied().filter(|c| counts(task, c) && fresh(c, since)).collect();
    if let Some(bad) = live.iter().find(|c| matches!(c.status.as_str(), "failed" | "error")) {
        return Verdict::Fail(format!("{} check {}", bad.kind, bad.status));
    }
    if live.iter().any(|c| c.status != "passed") {
        return Verdict::Waiting;
    }
    let by_person = live.iter().any(|c| c.kind == "person");
    if !by_person && required.iter().any(|c| !c.approved) {
        return Verdict::Waiting;
    }
    let independent = live.iter().any(|c| c.checked_by.as_deref().is_some_and(|by| !by.is_empty() && by != owner));
    if independent {
        Verdict::Pass
    } else {
        Verdict::Waiting
    }
}

/// When the task last went to review.
pub async fn review_since(state: &AppState, task_id: &str) -> Option<chrono::DateTime<chrono::Utc>> {
    let events = state.db.task_events(task_id, 500).await.ok()?;
    events.iter().rev().find(|e| e.to_state.as_deref() == Some("review")).map(|e| e.created_at)
}

/// Looks at a task in review after a check result: verified, back to ready, failed, or waits.
pub async fn evaluate(state: &AppState, task_id: &str) -> Option<Task> {
    let task = state.db.get_task(task_id).await.ok().flatten()?;
    if task.state != "review" {
        return Some(task);
    }
    let checks = state.db.list_checks(task_id).await.unwrap_or_default();
    let since = review_since(state, task_id).await;
    match verdict(&task, &checks, since) {
        Verdict::Pass => {
            let change = TaskChange { kind: "state".into(), reason: Some("its checks passed".into()), ..Default::default() };
            transition(state, task_id, "review", "verified", Actor::Kwant, "kwant", change).await.ok().or(Some(task))
        }
        Verdict::Fail(why) => {
            let last = task.attempts + 1 >= task.max_attempts;
            let to = if last { "failed" } else { "ready" };
            let change = TaskChange { reason: Some(why), attempt: true, ..Default::default() };
            transition(state, task_id, "review", to, Actor::Kwant, "kwant", change).await.ok().or(Some(task))
        }
        Verdict::Waiting => Some(task),
    }
}

/// `checks_summary` of a task: `passed` only when [`verdict`] could pass on them. Counts the
/// checks that count (see [`counts`]); `needs_approval` is how many wait for the person, and
/// `required_needs_approval` how many of those are required, so hold the task.
pub fn checks_summary(task: &Task, checks: &[OutcomeCheck]) -> Value {
    let live: Vec<&OutcomeCheck> = checks.iter().filter(|c| counts(task, c)).collect();
    let waiting = checks.iter().filter(|c| !c.approved).count();
    // An optional check waiting for approval is listed but doesn't hold the task (see [`verdict`])
    let required_waiting = checks.iter().filter(|c| c.required && !c.approved).count();
    let holding = required_waiting > 0;
    let gating = |c: &&&OutcomeCheck| c.required;
    let passed = live.iter().filter(|c| c.status == "passed").count();
    let failed = live.iter().filter(gating).filter(|c| matches!(c.status.as_str(), "failed" | "error")).count();
    let status = if live.is_empty() && waiting == 0 {
        "not_checked"
    } else if failed > 0 {
        "failed"
    } else if holding || live.iter().filter(gating).any(|c| c.status != "passed") || !live.iter().any(|c| c.required) {
        "pending"
    } else {
        "passed"
    };
    json!({
        "status": status,
        "passed": passed,
        "failed": failed,
        "needs_approval": waiting,
        "required_needs_approval": required_waiting,
        "total": checks.len(),
    })
}

// Budget

/// Adds a turn's fresh tokens to the task `kwant_id` worked on in `context_id`: a claim kwant
/// holds for it there (a hand-off, a routine run), else its own claim. A claim taken in `turn_id`
/// or released since `turn_started_ms` counts too, so a bot that finishes its task inside the turn
/// (`task_update review` releases the claim) still pays for that turn. Renews an open lease, and
/// blocks the task when it goes over its own budget. `booked` of the turn's tokens already landed
/// when the bot handed its task in ([`spend_at_hand_in`]), so only the rest is added here; when the
/// engine's mid-turn count ran ahead of its final one, the difference is taken back, so the turn
/// adds up to exactly `fresh`.
pub async fn spend(state: &AppState, kwant_id: &str, context_id: &str, turn_id: Option<&str>, turn_started_ms: i64, fresh: u64, booked: u64) {
    let due = i64::try_from(fresh).unwrap_or(i64::MAX).saturating_sub(i64::try_from(booked).unwrap_or(i64::MAX));
    if due == 0 {
        return;
    }
    let Ok(claims) = state.db.spend_claims(kwant_id, turn_id, turn_started_ms).await else { return };
    let here = |c: &&crate::db::missions::Claim| c.context_id.as_deref().is_none_or(|x| x == context_id);
    let claim = claims
        .iter()
        .find(|c| c.system && c.context_id.as_deref() == Some(context_id))
        .or_else(|| claims.iter().filter(here).find(|c| !c.system));
    let Some(claim) = claim else { return };
    if state.db.spend_on_task(&claim.task_id, due).await.is_err() {
        return;
    }
    let _ = state.db.renew_claim(&claim.task_id, lease_ms(state).await).await;
    let Ok(Some(task)) = state.db.get_task(&claim.task_id).await else { return };
    if task.spent_tokens > task.budget_tokens && matches!(task.state.as_str(), "working" | "needs_you") {
        let change = TaskChange {
            kind: "budget".into(),
            reason: Some("budget".into()),
            detail: Some(json!({"spent_tokens": task.spent_tokens, "budget_tokens": task.budget_tokens})),
            ..Default::default()
        };
        let _ = transition(state, &task.id, &task.state, "blocked", Actor::Kwant, "kwant", change).await;
    } else {
        emit_task(state, &task).await;
    }
    if let Some(m) = &task.mission_id {
        emit_mission(state, m).await;
    }
}

/// When the bot hands its task in during a turn (`task_update`), the turn's tokens its engine
/// reported so far land on the task right away, so a task verified in-turn never shows 0 spend
/// until the turn ends. [`spend`] adds only the rest at turn end. Engines that report usage only
/// at the end (codex, grok, custom CLIs) book nothing here.
pub async fn spend_at_hand_in(state: &AppState, task_id: &str, turn_id: Option<&str>) {
    let Some(turn_id) = turn_id else { return };
    let due = state.turns.book_usage(turn_id);
    if due == 0 {
        return;
    }
    let tokens = i64::try_from(due).unwrap_or(i64::MAX);
    if state.db.spend_on_task(task_id, tokens).await.is_err() {
        return;
    }
    if let Ok(Some(task)) = state.db.get_task(task_id).await {
        emit_task(state, &task).await;
        if let Some(m) = &task.mission_id {
            emit_mission(state, m).await;
        }
    }
}

/// Working tasks whose lease ran out go back to ready (one attempt used).
pub async fn sweep_leases(state: &AppState) -> usize {
    let mut n = 0;
    for t in state.db.expired_working().await.unwrap_or_default() {
        let change = TaskChange { kind: "lease_expired".into(), reason: Some("the lease ran out".into()), attempt: true, ..Default::default() };
        if transition(state, &t.id, "working", "ready", Actor::Kwant, "kwant", change).await.is_ok() {
            n += 1;
        }
    }
    n
}

// Board JSON and events

pub(crate) async fn name_of(state: &AppState, id: Option<&str>) -> Option<String> {
    match id? {
        "person" => Some("You".into()),
        "kwant" => Some("kwant".into()),
        id => state.db.get_kwant(id).await.ok().flatten().map(|k| k.name),
    }
}

/// What the task waits on the person for: the owner's open cards (questions, approvals, human
/// steps) or, in review, check approvals; then, in every state, its open "may be invalid" flags.
pub async fn needs_you(state: &AppState, task: &Task) -> Vec<Value> {
    let mut items = owner_needs_you(state, task).await;
    items.extend(graph::flag_items(state, task).await);
    items
}

async fn owner_needs_you(state: &AppState, task: &Task) -> Vec<Value> {
    if task.state == "review" {
        return check_approvals(state, task).await;
    }
    let Some(owner) = &task.owner_kwant_id else { return Vec::new() };
    if !matches!(task.state.as_str(), "working" | "needs_you") {
        return Vec::new();
    }
    let claim = state.db.open_claim(&task.id).await.ok().flatten();
    let context = claim.as_ref().and_then(|c| c.context_id.clone());
    let cards = state.db.open_cards_of(owner).await.unwrap_or_default();
    cards
        .into_iter()
        .filter(|(kind, _, _, at)| kind == "approval" || context.is_none() || at.as_deref() == context.as_deref())
        .map(|(kind, id, text, _)| json!({"kind": kind, "id": id, "text": text.chars().take(200).collect::<String>()}))
        .collect()
}

/// A task in review waits on the person for each required check that needs their approval
/// (`{"kind": "check_approval", "id": <check id>, "text"}`). An optional check can't hold the
/// task, so it never lands here; it can still be approved, and `checks_summary.needs_approval`
/// still counts it.
pub async fn check_approvals(state: &AppState, task: &Task) -> Vec<Value> {
    let checks = state.db.list_checks(&task.id).await.unwrap_or_default();
    checks
        .iter()
        .filter(|c| c.required && !c.approved)
        .map(|c| {
            let text = format!("Allow this check: {} {}", c.kind, c.spec_value());
            json!({"kind": "check_approval", "id": c.id, "text": text.chars().take(200).collect::<String>()})
        })
        .collect()
}

pub async fn task_json(state: &AppState, task: &Task) -> Value {
    let mut v = serde_json::to_value(task).unwrap_or(Value::Null);
    let deps = state.db.dependencies_of(&task.id).await.unwrap_or_default();
    let checks = state.db.list_checks(&task.id).await.unwrap_or_default();
    let claim = state.db.open_claim(&task.id).await.ok().flatten().filter(|c| c.expires_at > crate::db::missions::now_ms());
    let mission_title = match &task.mission_id {
        Some(m) => state.db.get_mission(m).await.ok().flatten().map(|m| m.title),
        None => None,
    };
    if let Some(obj) = v.as_object_mut() {
        obj.insert("owner_name".into(), json!(name_of(state, task.owner_kwant_id.as_deref()).await));
        obj.insert("mission_title".into(), json!(mission_title));
        obj.insert("depends_on".into(), json!(deps.iter().map(|d| &d.0).collect::<Vec<_>>()));
        obj.insert("waiting_on".into(), json!(deps.iter().filter(|d| d.1 != "verified").map(|d| &d.0).collect::<Vec<_>>()));
        obj.insert(
            "claim".into(),
            claim.map_or(Value::Null, |c| {
                json!({"kwant_id": c.kwant_id, "claimed_at": c.claimed_at_utc(), "expires_at": c.expires_at_utc()})
            }),
        );
        obj.insert("needs_you".into(), json!(needs_you(state, task).await));
        obj.insert("flags".into(), json!(graph::open_flags(state, task).await));
        obj.insert("checks_summary".into(), checks_summary(task, &checks));
        let (count, last) = state.db.checkpoint_stats(&task.id).await.unwrap_or((0, None));
        obj.insert("checkpoint_count".into(), json!(count));
        obj.insert("last_checkpoint_at".into(), json!(last));
    }
    v
}

pub async fn mission_json(state: &AppState, m: &Mission) -> Value {
    let mut v = serde_json::to_value(m).unwrap_or(Value::Null);
    let reserved = state.db.mission_reserved(&m.id).await.unwrap_or(0);
    let mut counts = serde_json::Map::new();
    for s in crate::db::missions::TASK_STATES {
        counts.insert(s.into(), json!(0));
    }
    for (s, n) in state.db.mission_counts(&m.id).await.unwrap_or_default() {
        counts.insert(s, json!(n));
    }
    if let Some(obj) = v.as_object_mut() {
        obj.remove("workspace_path");
        obj.insert("lead_name".into(), json!(name_of(state, Some(&m.lead_kwant_id)).await));
        obj.insert("reserved_tokens".into(), json!(reserved));
        obj.insert("over_budget".into(), json!(m.spent_tokens >= m.budget_tokens));
        obj.insert("counts".into(), Value::Object(counts));
    }
    v
}

pub fn check_json(task: &Task, c: &OutcomeCheck, checked_by_name: Option<String>) -> Value {
    let mut v = serde_json::to_value(c).unwrap_or(Value::Null);
    if let Some(obj) = v.as_object_mut() {
        obj.insert("spec".into(), c.spec_value());
        obj.insert("checked_by_name".into(), json!(checked_by_name));
        obj.insert("counts".into(), json!(counts(task, c)));
    }
    v
}

pub async fn checks_json(state: &AppState, task: &Task, checks: &[OutcomeCheck]) -> Vec<Value> {
    let mut out = Vec::new();
    for c in checks {
        out.push(check_json(task, c, name_of(state, c.checked_by.as_deref()).await));
    }
    out
}

pub async fn emit_task(state: &AppState, task: &Task) {
    let task = task_json(state, task).await;
    let _ = state.event_sender.send(RuntimeEvent::TaskUpdated { task });
}

pub async fn emit_task_id(state: &AppState, task_id: &str) {
    if let Ok(Some(t)) = state.db.get_task(task_id).await {
        emit_task(state, &t).await;
    }
}

pub async fn emit_mission(state: &AppState, mission_id: &str) {
    if let Ok(Some(m)) = state.db.get_mission(mission_id).await {
        let mission = mission_json(state, &m).await;
        let _ = state.event_sender.send(RuntimeEvent::MissionUpdated { mission });
    }
}

// Chat lines

async fn post_line(state: &AppState, channel_id: Option<String>, direct_kwant_id: Option<String>, thread_id: Option<String>, sender: &str, text: String, meta: Value) {
    if channel_id.is_none() && direct_kwant_id.is_none() && thread_id.is_none() {
        return;
    }
    let msg = NewMessage {
        channel_id: if thread_id.is_some() { None } else { channel_id },
        direct_kwant_id: if thread_id.is_some() { None } else { direct_kwant_id },
        sender_role: "kwant".into(),
        sender_id: sender.into(),
        text,
        reasoning_content: None,
        tool_calls: None,
        tokens_used: 0,
    };
    let extra = MessageExtra { thread_id: thread_id.clone(), meta: Some(meta) };
    let Ok(saved) = state.db.insert_message_with(&msg, &extra).await else { return };
    if let Some(t) = &thread_id {
        let _ = state.db.touch_thread(t).await;
    }
    let _ = state.event_sender.send(RuntimeEvent::MessageAdded { message: serde_json::to_value(&saved).unwrap_or(Value::Null) });
}

/// A short line in the mission's chat (`meta.kind = "mission"`).
pub async fn mission_chat_line(state: &AppState, m: &Mission, text: String) {
    let meta = json!({"kind": "mission", "mission_id": m.id, "status": m.status});
    post_line(state, m.channel_id.clone(), m.direct_kwant_id.clone(), m.thread_id.clone(), &m.lead_kwant_id, text, meta).await;
}

async fn task_chat_line(state: &AppState, t: &Task, m: Option<&Mission>) {
    let text = match t.state.as_str() {
        "verified" => format!("Verified: \"{}\" passed its checks.", t.title),
        "failed" => format!("Failed: \"{}\" didn't pass its checks after {} attempts. Retry it on the board.", t.title, t.attempts),
        _ => return,
    };
    let meta = json!({"kind": "mission", "task_id": t.id, "mission_id": t.mission_id, "state": t.state});
    let sender = t.owner_kwant_id.clone().or_else(|| m.map(|m| m.lead_kwant_id.clone())).unwrap_or_else(|| "kwant".into());
    match m {
        Some(m) => post_line(state, m.channel_id.clone(), m.direct_kwant_id.clone(), m.thread_id.clone(), &sender, text, meta).await,
        None => post_line(state, t.channel_id.clone(), t.direct_kwant_id.clone(), t.thread_id.clone(), &sender, text, meta).await,
    }
}

// Plans: shared by the `mission_plan` tool and `POST /api/missions`

#[derive(Debug, Clone, serde::Deserialize)]
pub struct PlanTask {
    pub key: Option<String>,
    pub title: String,
    #[serde(default)]
    pub acceptance: String,
    pub owner: Option<String>,
    pub priority: Option<i64>,
    #[serde(default)]
    pub depends_on: Vec<String>,
    pub budget_tokens: Option<i64>,
    #[serde(default)]
    pub checks: Vec<PlanCheck>,
}

#[derive(Debug, Clone, serde::Deserialize)]
pub struct PlanCheck {
    pub kind: String,
    #[serde(default)]
    pub spec: Value,
    pub required: Option<bool>,
}

/// Who made a plan, and so whether its `command` checks are approved with the mission.
pub struct PlanBy<'a> {
    pub created_by: &'a str,
    /// Active at once (the person's plan) or a draft waiting for the person.
    pub active: bool,
}

/// Where a mission lives.
#[derive(Default, Clone)]
pub struct PlanPlace {
    pub channel_id: Option<String>,
    pub direct_kwant_id: Option<String>,
    pub thread_id: Option<String>,
    pub workspace_path: String,
}

/// Validates a check a bot or the person adds. Returns whether it may run without the person's
/// approval: `command` needs it unless `person_ok`, an `http` check of a local or private URL too.
pub fn validate_check(kind: &str, spec: &Value, person_ok: bool) -> Result<bool, String> {
    let s = |k: &str| spec[k].as_str().map(str::trim).filter(|v| !v.is_empty());
    match kind {
        "command" => {
            let cmd = s("command").ok_or("a command check needs spec.command")?;
            if cmd.len() > 2000 {
                return Err("spec.command is too long".into());
            }
            if spec["timeout_secs"].as_u64().is_some_and(|t| !(1..=checks::MAX_COMMAND_SECS).contains(&t)) {
                return Err(format!("spec.timeout_secs must be 1–{}", checks::MAX_COMMAND_SECS));
            }
            Ok(person_ok)
        }
        "http" => {
            let url = s("url").ok_or("an http check needs spec.url")?;
            if !(url.starts_with("http://") || url.starts_with("https://")) {
                return Err("spec.url must start with http:// or https://".into());
            }
            // Stricter than WebFetch: kwnt fetches it itself, so no local dev-server exemption
            Ok(person_ok || crate::mcp::fetch_risk::public_only(url).is_ok())
        }
        "file_exists" => {
            s("path").ok_or("a file check needs spec.path")?;
            Ok(true)
        }
        "file_contains" => {
            s("path").ok_or("a file check needs spec.path")?;
            spec["text"].as_str().filter(|t| !t.is_empty()).ok_or("file_contains needs spec.text")?;
            Ok(true)
        }
        "file_sha256" => {
            s("path").ok_or("a file check needs spec.path")?;
            let h = s("sha256").ok_or("file_sha256 needs spec.sha256")?;
            if h.len() != 64 || !h.chars().all(|c| c.is_ascii_hexdigit()) {
                return Err("spec.sha256 must be 64 hex characters".into());
            }
            Ok(true)
        }
        "peer_review" | "person" => Err(format!("{kind} checks are recorded by task_review or the person's Verify, not added")),
        other => Err(format!("unknown check kind {other}")),
    }
}

/// Resolves a bot by id or name in `space`.
pub async fn bot_in_space(state: &AppState, space: &SpaceId, who: &str) -> Option<crate::db::models::Kwant> {
    let who = who.trim().trim_start_matches('@');
    let bots = state.db.list_kwants(space).await.ok()?;
    bots.iter().find(|k| k.id == who).or_else(|| bots.iter().find(|k| k.name.eq_ignore_ascii_case(who))).cloned()
}

/// Creates a mission and its tasks (backlog, or ready when active and free of dependencies).
#[allow(clippy::too_many_arguments)]
pub async fn create_plan(
    state: &AppState,
    space: &SpaceId,
    lead: &crate::db::models::Kwant,
    title: &str,
    goal: &str,
    budget: Option<i64>,
    tasks: &[PlanTask],
    by: PlanBy<'_>,
    place: PlanPlace,
) -> Result<(Mission, Vec<Task>), String> {
    let title = title.trim();
    if title.is_empty() || title.chars().count() > 200 {
        return Err("title is required (at most 200 characters)".into());
    }
    if tasks.len() > MAX_PLAN_TASKS {
        return Err(format!("a mission has at most {MAX_PLAN_TASKS} tasks"));
    }
    let budget = match budget {
        Some(b) if !(BUDGET_RANGE.0..=BUDGET_RANGE.1).contains(&b) => {
            return Err(format!("budget_tokens must be {}–{}", BUDGET_RANGE.0, BUDGET_RANGE.1))
        }
        Some(b) => b,
        None => default_budget(state).await,
    };
    // Everything is checked before anything is written
    let mut keys = std::collections::HashMap::new();
    let mut owners = Vec::new();
    let mut approvals = Vec::new();
    for (i, t) in tasks.iter().enumerate() {
        if t.title.trim().is_empty() {
            return Err(format!("task {} needs a title", i + 1));
        }
        let key = t.key.clone().unwrap_or_else(|| (i + 1).to_string());
        if keys.insert(key.clone(), i).is_some() {
            return Err(format!("two tasks have the key {key}"));
        }
        if t.priority.is_some_and(|p| !(0..=3).contains(&p)) {
            return Err(format!("task {key}: priority must be 0–3"));
        }
        if t.budget_tokens.is_some_and(|b| b < 1 || b > budget) {
            return Err(format!("task {key}: budget_tokens must be 1–{budget}"));
        }
        let owner = match &t.owner {
            Some(o) => Some(bot_in_space(state, space, o).await.ok_or_else(|| format!("task {key}: no bot {o} in this space"))?.id),
            None => None,
        };
        owners.push(owner);
        let mut ok = Vec::new();
        for c in &t.checks {
            ok.push(validate_check(&c.kind, &c.spec, true).map_err(|e| format!("task {key}: {e}"))?);
        }
        // A bot's plan is approved as a whole by the person; an http check of a private URL still asks
        let ok: Vec<bool> = t
            .checks
            .iter()
            .zip(ok)
            .map(|(c, ok)| if c.kind == "http" { validate_check("http", &c.spec, false).unwrap_or(false) || by.active } else { ok })
            .collect();
        approvals.push(ok);
    }
    for t in tasks {
        for d in &t.depends_on {
            if !keys.contains_key(d) {
                return Err(format!("depends_on names {d}, which isn't a task key in this plan"));
            }
        }
    }
    let mission = state
        .db
        .create_mission(
            space,
            &crate::db::missions::NewMission {
                title: title.into(),
                goal: goal.trim().into(),
                status: if by.active { "active".into() } else { "draft".into() },
                lead_kwant_id: lead.id.clone(),
                channel_id: place.channel_id,
                direct_kwant_id: place.direct_kwant_id,
                thread_id: place.thread_id,
                workspace_path: place.workspace_path,
                budget_tokens: budget,
                created_by: by.created_by.into(),
            },
        )
        .await
        .map_err(|e| e.to_string())?;
    let mut ids: Vec<String> = Vec::new();
    for (i, t) in tasks.iter().enumerate() {
        let row = state
            .db
            .create_task(
                space,
                &NewTask {
                    mission_id: Some(mission.id.clone()),
                    title: t.title.trim().into(),
                    acceptance: t.acceptance.trim().into(),
                    state: "backlog".into(),
                    priority: t.priority.unwrap_or(1),
                    owner_kwant_id: None,
                    suggested_kwant_id: owners[i].clone(),
                    budget_tokens: t.budget_tokens.unwrap_or(DEFAULT_TASK_BUDGET.min(budget)),
                    max_attempts: DEFAULT_MAX_ATTEMPTS,
                    source_kind: "mission".into(),
                    source_id: Some(mission.id.clone()),
                    channel_id: mission.channel_id.clone(),
                    direct_kwant_id: mission.direct_kwant_id.clone(),
                    thread_id: mission.thread_id.clone(),
                    created_by: by.created_by.into(),
                },
            )
            .await
            .map_err(|e| e.to_string())?;
        for (c, approved) in t.checks.iter().zip(&approvals[i]) {
            state
                .db
                .add_check(&NewCheck {
                    task_id: row.id.clone(),
                    kind: c.kind.clone(),
                    spec: c.spec.clone(),
                    required: c.required.unwrap_or(true),
                    approved: *approved,
                    requested_by: by.created_by.into(),
                    // The person made the plan or approves it before anything runs
                    approved_by: Some(if by.created_by == "person" { "person" } else { "mission" }.into()),
                })
                .await
                .map_err(|e| e.to_string())?;
        }
        ids.push(row.id);
    }
    for (i, t) in tasks.iter().enumerate() {
        for d in &t.depends_on {
            state.db.add_dependency(&ids[i], &ids[keys[d]]).await.map_err(|e| e.to_string())?;
        }
    }
    if by.active {
        promote(state, &mission.id).await;
    }
    let mut rows = Vec::new();
    for id in &ids {
        if let Ok(Some(t)) = state.db.get_task(id).await {
            emit_task(state, &t).await;
            rows.push(t);
        }
    }
    emit_mission(state, &mission.id).await;
    Ok((mission, rows))
}

/// Creates a task outside a plan (a hand-off, a routine run, a work-ahead run, the person's).
pub async fn create_task(state: &AppState, space: &SpaceId, t: NewTask) -> Result<Task, DbError> {
    let task = state.db.create_task(space, &t).await?;
    emit_task(state, &task).await;
    if let Some(m) = &task.mission_id {
        emit_mission(state, m).await;
    }
    Ok(task)
}

/// The folder a task's file checks and evidence resolve in: its mission's, else its owner's.
pub async fn task_folder(state: &AppState, task: &Task) -> Option<std::path::PathBuf> {
    task_folder_in(&state.db, &state.config.data_dir, task).await
}

/// [`task_folder`] without the app state (the permission hook has only the database).
pub async fn task_folder_in(db: &crate::db::Database, data_dir: &std::path::Path, task: &Task) -> Option<std::path::PathBuf> {
    if let Some(mid) = &task.mission_id {
        if let Ok(Some(m)) = db.get_mission(mid).await {
            if !m.workspace_path.is_empty() {
                return Some(std::path::PathBuf::from(m.workspace_path));
            }
        }
    }
    let owner = task.owner_kwant_id.as_deref().or(task.suggested_kwant_id.as_deref())?;
    let space = SpaceId::new(&task.space_id);
    Some(crate::tools::bot_workspace(&crate::db::spaces::space_data_dir(data_dir, &space), owner))
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;

    #[test]
    fn transitions_follow_the_contract() {
        use Actor::*;
        assert!(allowed("backlog", "ready", Kwant));
        assert!(!allowed("backlog", "ready", Owner), "only kwant promotes, after the dependencies are verified");
        assert!(!allowed("backlog", "ready", Person));
        assert!(allowed("ready", "working", Owner));
        assert!(allowed("working", "review", Owner));
        for a in [Owner, Person, Kwant] {
            assert!(!allowed("working", "verified", a), "nobody skips review");
            assert!(!allowed("verified", "ready", a), "verified is final");
            assert!(!allowed("cancelled", "ready", a), "cancelled is final");
            assert!(!allowed("verified", "cancelled", a));
        }
        assert!(!allowed("review", "verified", Owner), "the owner can't verify its own work");
        assert!(!allowed("review", "verified", Person), "the person verifies through a person check");
        assert!(allowed("review", "verified", Kwant));
        assert!(!allowed("working", "needs_you", Owner));
        assert!(allowed("working", "needs_you", Kwant));
        assert!(allowed("failed", "ready", Person));
        assert!(!allowed("failed", "ready", Owner));
        assert!(!allowed("review", "ready", Owner), "the owner can't take back a review to dodge a failing check");
        assert!(allowed("ready", "cancelled", Person));
        assert!(!allowed("ready", "cancelled", Owner));
    }

    fn task(owner: &str) -> Task {
        Task {
            id: "t".into(),
            space_id: "s".into(),
            mission_id: None,
            title: "x".into(),
            acceptance: String::new(),
            state: "review".into(),
            priority: 1,
            owner_kwant_id: Some(owner.into()),
            suggested_kwant_id: None,
            budget_tokens: 10,
            spent_tokens: 0,
            attempts: 0,
            max_attempts: 3,
            blocked_reason: None,
            source_kind: "person".into(),
            source_id: None,
            channel_id: None,
            direct_kwant_id: None,
            thread_id: None,
            created_by: "person".into(),
            created_at: Utc::now(),
            updated_at: Utc::now(),
            finished_at: None,
            forked_from: None,
            held_at: None,
        }
    }

    fn check(kind: &str, status: &str, by: Option<&str>, required: bool) -> OutcomeCheck {
        OutcomeCheck {
            id: uuid::Uuid::new_v4().to_string(),
            task_id: "t".into(),
            kind: kind.into(),
            spec: "{}".into(),
            required,
            approved: true,
            status: status.into(),
            output_excerpt: None,
            checked_by: by.map(str::to_string),
            requested_by: "person".into(),
            approved_by: Some("person".into()),
            evidence_id: None,
            created_at: Utc::now(),
            started_at: None,
            finished_at: Some(Utc::now()),
        }
    }

    #[test]
    fn the_owners_word_never_verifies() {
        let t = task("bot-a");
        assert_eq!(verdict(&t, &[], None), Verdict::Waiting, "no check, no verify");
        assert_eq!(verdict(&t, &[check("peer_review", "passed", Some("bot-a"), true)], None), Verdict::Waiting, "own review");
        assert_eq!(verdict(&t, &[check("peer_review", "passed", Some("bot-b"), true)], None), Verdict::Pass);
        assert_eq!(verdict(&t, &[check("command", "passed", Some("kwant"), true)], None), Verdict::Pass);
        let failing = [check("command", "passed", Some("kwant"), true), check("http", "failed", Some("kwant"), true)];
        assert!(matches!(verdict(&t, &failing, None), Verdict::Fail(_)));
        let optional = [check("command", "passed", Some("kwant"), true), check("http", "failed", Some("kwant"), false)];
        assert_eq!(verdict(&t, &optional, None), Verdict::Pass, "an optional check failing doesn't block");
        let pending = [check("command", "passed", Some("kwant"), true), check("file_exists", "pending", None, true)];
        assert_eq!(verdict(&t, &pending, None), Verdict::Waiting);
        let mut unapproved = check("command", "failed", Some("kwant"), true);
        unapproved.approved = false;
        assert_eq!(verdict(&t, &[unapproved.clone()], None), Verdict::Waiting, "an unapproved check doesn't count either way");
        assert_eq!(verdict(&t, &[unapproved, check("person", "passed", Some("person"), true)], None), Verdict::Pass);
    }

    #[test]
    fn the_owners_own_check_counts_only_once_the_person_approves_it() {
        let t = task("bot-a");
        let mut own = check("file_exists", "passed", Some("kwant"), true);
        own.requested_by = "bot-a".into();
        own.approved_by = Some("auto".into());
        assert!(!counts(&t, &own));
        assert_eq!(verdict(&t, &[own.clone()], None), Verdict::Waiting, "its own file_exists doesn't verify it");
        // It was added before the bot became the owner, so it ran, but nothing that counts checked it
        assert_eq!(checks_summary(&t, &[own.clone()])["status"], "not_checked");
        own.approved_by = Some("person".into());
        assert!(counts(&t, &own));
        assert_eq!(verdict(&t, &[own.clone()], None), Verdict::Pass);
        // A teammate's (or the mission's) check counts as usual
        let mut mate = own.clone();
        (mate.requested_by, mate.approved_by) = ("bot-b".into(), Some("auto".into()));
        assert_eq!(verdict(&t, &[mate], None), Verdict::Pass);
    }

    #[test]
    fn every_required_check_must_pass_a_waiting_one_holds_the_task() {
        let t = task("bot-a");
        let mut waiting = check("http", "needs_approval", None, true);
        (waiting.approved, waiting.approved_by) = (false, None);
        let both = [check("file_exists", "passed", Some("kwant"), true), waiting.clone()];
        assert_eq!(verdict(&t, &both, None), Verdict::Waiting, "1 of 2 passed is not verified");
        let s = checks_summary(&t, &both);
        assert_eq!((s["status"].as_str(), s["needs_approval"].as_u64(), s["passed"].as_u64()), (Some("pending"), Some(1), Some(1)));
        // The person's own Verify is their call
        let with_person = [both[0].clone(), waiting.clone(), check("person", "passed", Some("person"), true)];
        assert_eq!(verdict(&t, &with_person, None), Verdict::Pass);
        // An optional check waiting for approval doesn't hold it
        waiting.required = false;
        assert_eq!(verdict(&t, &[both[0].clone(), waiting], None), Verdict::Pass);
    }

    #[test]
    fn summary_tells_a_required_waiting_check_from_an_optional_one() {
        let t = task("bot-a");
        let mut waiting = check("http", "needs_approval", None, true);
        (waiting.approved, waiting.approved_by) = (false, None);
        let s = checks_summary(&t, &[waiting.clone()]);
        assert_eq!((s["needs_approval"].as_u64(), s["required_needs_approval"].as_u64()), (Some(1), Some(1)));
        assert_eq!(s["status"], "pending");
        // A required check still running plus an optional one waiting: pending, but not on the person
        waiting.required = false;
        let s = checks_summary(&t, &[check("command", "running", None, true), waiting.clone()]);
        assert_eq!(s["status"], "pending");
        assert_eq!((s["needs_approval"].as_u64(), s["required_needs_approval"].as_u64()), (Some(1), Some(0)));
        assert_eq!(checks_summary(&t, &[])["required_needs_approval"], 0);
    }

    #[test]
    fn an_old_review_doesnt_count_after_rework() {
        let t = task("bot-a");
        let mut old = check("peer_review", "failed", Some("bot-b"), true);
        old.finished_at = Some(Utc::now() - chrono::Duration::hours(1));
        let since = Some(Utc::now() - chrono::Duration::minutes(1));
        assert_eq!(verdict(&t, &[old.clone()], since), Verdict::Waiting, "the failed review was of the earlier attempt");
        assert!(matches!(verdict(&t, &[old], None), Verdict::Fail(_)));
    }

    #[test]
    fn summary_says_not_checked() {
        let t = task("bot-a");
        assert_eq!(checks_summary(&t, &[])["status"], "not_checked");
        assert_eq!(checks_summary(&t, &[check("command", "passed", Some("kwant"), true)])["status"], "passed");
        assert_eq!(checks_summary(&t, &[check("command", "error", Some("kwant"), true)])["status"], "failed");
    }

    #[test]
    fn bots_need_approval_for_commands_and_private_urls() {
        assert_eq!(validate_check("command", &json!({"command": "true"}), false), Ok(false));
        assert_eq!(validate_check("command", &json!({"command": "true"}), true), Ok(true));
        assert_eq!(validate_check("http", &json!({"url": "http://127.0.0.1:8080/"}), false), Ok(false));
        assert_eq!(validate_check("file_exists", &json!({"path": "a.txt"}), false), Ok(true));
        assert!(validate_check("peer_review", &json!({}), true).is_err());
        assert!(validate_check("file_sha256", &json!({"path": "a", "sha256": "xyz"}), true).is_err());
        assert!(validate_check("http", &json!({"url": "file:///etc/passwd"}), true).is_err());
    }

    #[test]
    fn settings_ranges() {
        assert!(settings_error(Some(4), None).is_some());
        assert!(settings_error(Some(30), Some(200_000)).is_none());
        assert!(settings_error(None, Some(999)).is_some());
    }
}
