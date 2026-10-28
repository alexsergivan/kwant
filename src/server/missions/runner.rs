//! The mission runner: on each scheduler tick (and `POST /api/missions/:id/run`) it claims the
//! highest-priority ready task of an active mission for its owner and runs one hidden turn on it.
//! At most one runner turn per mission and [`MAX_RUNNING`] server-wide.

use super::{lease_ms, transition, Actor};
use crate::db::missions::{ClaimRefused, Mission, Task, TaskChange};
use crate::server::routes::{scope_for, turn_once, TurnOpts};
use crate::server::AppState;
use std::collections::HashSet;
use std::sync::{LazyLock, Mutex};

pub const MAX_RUNNING: usize = 2;
pub const PURPOSE: &str = "mission";

/// Missions with a runner turn going.
static RUNNING: LazyLock<Mutex<HashSet<String>>> = LazyLock::new(|| Mutex::new(HashSet::new()));

/// Holds a mission's runner slot; frees it when dropped.
struct Slot(String);

impl Slot {
    fn take(mission_id: &str) -> Result<Slot, &'static str> {
        let mut set = RUNNING.lock().unwrap_or_else(|e| e.into_inner());
        if set.contains(mission_id) {
            return Err("a runner turn is already going for this mission");
        }
        if set.len() >= MAX_RUNNING {
            return Err("two missions are already running; it starts when one finishes");
        }
        set.insert(mission_id.to_string());
        Ok(Slot(mission_id.to_string()))
    }
}

impl Drop for Slot {
    fn drop(&mut self) {
        RUNNING.lock().unwrap_or_else(|e| e.into_inner()).remove(&self.0);
    }
}

pub fn is_running(mission_id: &str) -> bool {
    RUNNING.lock().unwrap_or_else(|e| e.into_inner()).contains(mission_id)
}

/// One scheduler tick: leases that ran out go back to ready, then each active mission without a
/// runner turn gets one.
pub async fn tick(state: &AppState) {
    if !super::enabled(state).await {
        return;
    }
    // Sources first, so a live hand-off or routine refreshes kwant's lease before the sweep
    super::listener::reconcile(state).await;
    super::sweep_leases(state).await;
    // AutoFix retries whose time has come, before tasks are picked (Phase B3)
    super::autofix::run_due(state).await;
    for m in state.db.active_missions().await.unwrap_or_default() {
        // Evidence drift, at most every 10 minutes per mission (Phase B1)
        super::graph::scan_if_due(state, &m).await;
        if is_running(&m.id) {
            continue;
        }
        let _ = run_mission(state, &m, 1).await;
    }
}

/// Starts runner turns for up to `max` ready tasks of `mission` (one at a time, in the
/// background). Returns `(task_id, kwant_id)` of each started, and why none were if so.
pub async fn run_mission(state: &AppState, mission: &Mission, max: usize) -> (Vec<(String, String)>, Option<String>) {
    let slot = match Slot::take(&mission.id) {
        Ok(s) => s,
        Err(why) => return (Vec::new(), Some(why.into())),
    };
    super::promote(state, &mission.id).await;
    let space = crate::db::spaces::SpaceId::new(&mission.space_id);
    let ready = state.db.list_tasks(&space, Some("ready"), Some(&mission.id), None, 100).await.unwrap_or_default();
    let mut picked: Vec<(Task, crate::db::models::Kwant)> = Vec::new();
    let mut reason = None;
    for t in ready.iter().filter(|t| matches!(t.source_kind.as_str(), "mission" | "person") && t.held_at.is_none()) {
        if picked.len() >= max {
            break;
        }
        let who = t.owner_kwant_id.clone().or_else(|| t.suggested_kwant_id.clone()).unwrap_or_else(|| mission.lead_kwant_id.clone());
        let Ok(Some(bot)) = state.db.get_kwant(&who).await else { continue };
        if picked.iter().any(|(_, k)| k.id == bot.id) {
            continue;
        }
        // Flight School (D7): a bot below act on an engine kwant can't gate doesn't run the task
        if super::flight_school::block_if_ungated(state, t, &bot, None).await {
            reason = Some(super::flight_school::ENGINE_CANT_SANDBOX.into());
            continue;
        }
        let context = mission_context(mission, &bot);
        match state.db.claim_task(&t.id, &bot.id, None, Some(&context), lease_ms(state).await, false, "kwant").await {
            Ok(Ok(task)) => {
                super::emit_task(state, &task).await;
                super::checkpoints::before_run(state, &task).await;
                picked.push((task, bot));
            }
            Ok(Err(ClaimRefused::Budget { remaining, needed })) => {
                reason = Some(format!("the mission's budget can't cover the next task ({remaining} tokens left, {needed} needed)"));
                break;
            }
            Ok(Err(why)) => reason = Some(why.to_string()),
            Err(e) => reason = Some(e.to_string()),
        }
    }
    if picked.is_empty() {
        return (Vec::new(), Some(reason.unwrap_or_else(|| "no ready task to start".into())));
    }
    super::emit_mission(state, &mission.id).await;
    let started = picked.iter().map(|(t, k)| (t.id.clone(), k.id.clone())).collect();
    let state = state.clone();
    let mission = mission.clone();
    tokio::spawn(async move {
        let _slot = slot;
        for (task, bot) in picked {
            run_one(&state, &mission, &task, &bot).await;
        }
    });
    (started, None)
}

/// Where a runner turn of `bot` in `m` runs: the mission's channel; its thread or direct chat only
/// for the lead (threads and direct chats are one bot's own), else the bot's direct chat.
fn mission_place(m: &Mission, bot: &crate::db::models::Kwant) -> (Option<String>, Option<String>) {
    match (&m.channel_id, &m.thread_id) {
        (Some(c), _) => (Some(c.clone()), None),
        (None, Some(t)) if bot.id == m.lead_kwant_id => (None, Some(t.clone())),
        _ => (None, None),
    }
}

/// The chat id of [`mission_place`], as `TurnScope::context_id` names it.
fn mission_context(m: &Mission, bot: &crate::db::models::Kwant) -> String {
    let (c, t) = mission_place(m, bot);
    c.or(t).unwrap_or_else(|| bot.id.clone())
}

async fn prompt(state: &AppState, m: &Mission, t: &Task) -> String {
    let checks = state.db.list_checks(&t.id).await.unwrap_or_default();
    let list: Vec<String> = checks
        .iter()
        .filter(|c| c.approved)
        .map(|c| format!("- {} {}{}", c.kind, super::autofix::redact_value(&c.spec_value()), if c.required { "" } else { " (optional)" }))
        .collect();
    let deps = state.db.dependencies_of(&t.id).await.unwrap_or_default();
    format!(
        "[Mission Control] You hold task {id} of the mission \"{mt}\" (goal: {goal}).\n\
         Task: {title}\nAcceptance: {acc}\nChecks kwant will run when you're done:\n{checks}\n{deps}\
         Work on it now. Attach what you produced with task_attach_evidence, then call task_update with \
         task_id {id} and status \"review\": kwant runs the checks and only they (or a teammate's review, \
         or the person) can verify it. If you can't finish, call task_update with status \"blocked\" and a \
         reason, or \"ready\" to let it go. Never say it is verified yourself. Budget: {budget} tokens.",
        id = t.id,
        mt = m.title,
        goal = m.goal,
        title = t.title,
        acc = if t.acceptance.is_empty() { "(none given)" } else { &t.acceptance },
        checks = if list.is_empty() { "- none: a teammate or the person reviews it".into() } else { list.join("\n") },
        deps = if deps.is_empty() { String::new() } else { format!("It builds on tasks {} (verified).\n", deps.iter().map(|d| d.0.as_str()).collect::<Vec<_>>().join(", ")) },
        budget = t.budget_tokens - t.spent_tokens,
    )
}

async fn run_one(state: &AppState, m: &Mission, task: &Task, bot: &crate::db::models::Kwant) {
    let started = chrono::Utc::now();
    let context = mission_context(m, bot);
    let job = state.turns.start_job([context.clone()]);
    let (channel, thread) = mission_place(m, bot);
    let reply = match scope_for(state, bot, channel, thread, &job, 0).await {
        Ok(scope) => {
            let mut opts = TurnOpts::default();
            (opts.unsaved, opts.quiet_failure, opts.purpose) = (true, true, Some(PURPOSE.into()));
            let mut p = prompt(state, m, task).await;
            if let Some(note) = super::autofix::retry_note(state, task).await {
                p.push_str(&note);
            }
            turn_once(&scope, bot, p, opts).await
        }
        Err(e) => Err(e),
    };
    if let Err(e) = &reply {
        tracing::info!(task_id = %task.id, error = %e, "mission runner turn ended without a reply");
    }
    let err = reply.as_ref().err().map(ToString::to_string);
    after_turn(state, &task.id, err.as_deref(), started).await;
}

/// After a runner turn of `task_id`: still `working` means the bot didn't finish or let go, so the
/// lease is released (one attempt used). With AutoFix on, a task with attempts left parks in
/// `failed` for it (§13), and the failure is recorded: the turn's error, a denied action since
/// `started`, or a stall; a task left waiting on the person records `needs_input`.
pub async fn after_turn(state: &AppState, task_id: &str, err: Option<&str>, started: chrono::DateTime<chrono::Utc>) {
    let Ok(Some(t)) = state.db.get_task(task_id).await else { return };
    if t.state == "needs_you" {
        super::autofix::record(state, &t.id, "turn", "needs_input", "", "the bot is waiting on the person").await;
        return;
    }
    if t.state != "working" {
        return;
    }
    let park = super::autofix::parks(state, &t, true).await;
    let change = TaskChange {
        kind: "released".into(),
        reason: Some("the runner turn ended without a review".into()),
        attempt: true,
        ..Default::default()
    };
    let to = if park { "failed" } else { "ready" };
    if transition(state, &t.id, "working", to, Actor::Kwant, "kwant", change).await.is_err() {
        return;
    }
    if !park {
        if let Ok(Some(t)) = state.db.get_task(task_id).await {
            if t.attempts >= t.max_attempts && t.state == "ready" {
                let change = TaskChange { reason: Some("no review after its last attempt".into()), ..Default::default() };
                let _ = transition(state, &t.id, "ready", "blocked", Actor::Kwant, "kwant", change).await;
            }
        }
    }
    let denied = state.db.denied_since(task_id, started).await.unwrap_or_default();
    match (err, denied) {
        (Some(e), _) => super::autofix::record(state, task_id, "turn", "error", e, e.lines().next().unwrap_or("the turn failed")).await,
        (None, Some(what)) => super::autofix::record(state, task_id, "turn", "denied", &what, &format!("denied: {what}")).await,
        (None, None) => super::autofix::record(state, task_id, "turn", "stalled", "", "the runner turn ended without a review").await,
    };
}
