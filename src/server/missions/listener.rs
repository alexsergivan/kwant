//! Feeds the one task board from what already happens: hand-offs (`delegation`), routine runs
//! (`routine`) and work-ahead posts (`work_ahead`) become tasks, and the existing question,
//! approval and human-step cards move a task to `needs_you` and back. Only while `mission_board`
//! is on. Each update re-reads the source row, so an event that arrives late or twice (or is lost
//! and found again by [`reconcile`]) leaves the same result.

use super::checks::run_task_checks;
use super::{lease_ms, transition, Actor};
use crate::db::missions::{NewTask, Task, TaskChange};
use crate::db::spaces::SpaceId;
use crate::drivers::events::RuntimeEvent;
use crate::server::AppState;
use std::sync::LazyLock;
use tokio::sync::Mutex;
use tokio_util::sync::CancellationToken;

/// One update at a time, so two events about one hand-off can't both create its task.
static SERIAL: LazyLock<Mutex<()>> = LazyLock::new(|| Mutex::new(()));

/// Listens on the event bus until `shutdown`.
pub fn spawn(state: AppState, shutdown: CancellationToken) {
    let mut rx = state.event_sender.subscribe();
    tokio::spawn(async move {
        loop {
            let ev = tokio::select! {
                _ = shutdown.cancelled() => return,
                ev = rx.recv() => ev,
            };
            match ev {
                Ok(ev) => handle(&state, ev).await,
                // Missed events: the next reconcile catches up
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => reconcile(&state).await,
                Err(tokio::sync::broadcast::error::RecvError::Closed) => return,
            }
        }
    });
}

async fn handle(state: &AppState, ev: RuntimeEvent) {
    match ev {
        RuntimeEvent::DelegationUpdated { delegation } => {
            if let Some(id) = delegation["id"].as_str() {
                on_delegation(state, id).await;
            }
        }
        RuntimeEvent::RoutineRun { routine_id, run_id, status } => on_routine_run(state, &routine_id, &run_id, &status).await,
        RuntimeEvent::QuestionUpdated { .. }
        | RuntimeEvent::HumanStepUpdated { .. }
        | RuntimeEvent::ApprovalRequired { .. }
        | RuntimeEvent::ApprovalWithdrawn { .. } => refresh_needs_you(state).await,
        _ => {}
    }
}

/// Catches up with anything missed: every system-held task against its source, every open task's
/// cards.
pub async fn reconcile(state: &AppState) {
    if !super::enabled(state).await {
        return;
    }
    // A hand-off card the person (or AutoFix) put back in Ready, whose hand-off had ended
    for t in state.db.tasks_in_state("ready").await.unwrap_or_default() {
        if t.source_kind == "delegation" && t.held_at.is_none() {
            if let Some(id) = t.source_id.as_deref() {
                on_delegation(state, id).await;
            }
        }
    }
    for t in state.db.system_held().await.unwrap_or_default() {
        match (t.source_kind.as_str(), t.source_id.as_deref()) {
            ("delegation", Some(id)) => on_delegation(state, id).await,
            ("routine", Some(run)) => {
                if let Ok(Some((routine, status))) = state.db.routine_run_of(run).await {
                    on_routine_run(state, &routine, run, &status).await;
                }
            }
            _ => {}
        }
    }
    refresh_needs_you(state).await;
}

/// The mission a bot's own open claim is in, so a hand-off it starts joins that mission.
async fn mission_of_holder(state: &AppState, kwant_id: &str) -> Option<String> {
    let claims = state.db.claims_of(kwant_id).await.ok()?;
    for c in claims.iter().filter(|c| !c.system) {
        if let Ok(Some(t)) = state.db.get_task(&c.task_id).await {
            if let Some(m) = t.mission_id {
                return Some(m);
            }
        }
    }
    None
}

/// Creates a working task held by kwant for `owner` (a hand-off or routine run), or `None`.
#[allow(clippy::too_many_arguments)]
async fn start_held(
    state: &AppState,
    space: &SpaceId,
    mission_id: Option<String>,
    title: &str,
    acceptance: &str,
    owner: &str,
    source_kind: &str,
    source_id: &str,
    place: (Option<String>, Option<String>, Option<String>),
    context_id: &str,
) -> Option<Task> {
    let budget_tokens = super::default_task_budget(state).await;
    let new = |mission_id: Option<String>| NewTask {
        mission_id,
        title: title.chars().take(200).collect(),
        acceptance: acceptance.chars().take(4000).collect(),
        state: "ready".into(),
        priority: 1,
        owner_kwant_id: None,
        suggested_kwant_id: Some(owner.into()),
        budget_tokens,
        max_attempts: super::DEFAULT_MAX_ATTEMPTS,
        source_kind: source_kind.into(),
        source_id: Some(source_id.into()),
        channel_id: place.0.clone(),
        direct_kwant_id: place.1.clone(),
        thread_id: place.2.clone(),
        created_by: "kwant".into(),
    };
    let task = match state.db.create_task(space, &new(mission_id.clone())).await {
        Ok(t) => t,
        // The mission isn't active (the trigger refuses a ready task in it): a task of its own
        Err(_) if mission_id.is_some() => state.db.create_task(space, &new(None)).await.ok()?,
        Err(e) => {
            tracing::warn!(error = %e, "could not create a {source_kind} task");
            return None;
        }
    };
    hold(state, &task, owner, context_id).await;
    state.db.get_task(&task.id).await.ok().flatten()
}

/// kwant takes `task` for `owner` (a system claim: it doesn't stop the bot from holding its own).
async fn hold(state: &AppState, task: &Task, owner: &str, context_id: &str) {
    match state.db.claim_task(&task.id, owner, None, Some(context_id), lease_ms(state).await, true, "kwant").await {
        Ok(Ok(t)) => {
            super::emit_task(state, &t).await;
            super::checkpoints::before_run(state, &t).await;
        }
        Ok(Err(why)) => {
            tracing::info!(task_id = %task.id, %why, "a held task wasn't claimed");
            super::emit_task(state, task).await;
        }
        Err(e) => tracing::warn!(error = %e, "could not claim a held task"),
    }
    if let Some(m) = &task.mission_id {
        super::emit_mission(state, m).await;
    }
}

/// The source finished: to review (and its checks), failed, cancelled or back to ready. Whether
/// the task moved.
async fn finish(state: &AppState, task: &Task, to: &str, why: &str) -> bool {
    let mut from = task.state.clone();
    if !matches!(from.as_str(), "working" | "needs_you") {
        return false;
    }
    if to == "review" && from == "needs_you" {
        // review is reached from working only
        let _ = state.db.refresh_claim(&task.id, lease_ms(state).await).await;
        let change = TaskChange { reason: Some(why.into()), ..Default::default() };
        if transition(state, &task.id, "needs_you", "working", Actor::Kwant, "kwant", change).await.is_err() {
            return false;
        }
        from = "working".into();
    }
    let change = TaskChange { reason: Some(why.into()), ..Default::default() };
    let moved = transition(state, &task.id, &from, to, Actor::Kwant, "kwant", change).await.is_ok();
    if moved && to == "review" {
        run_task_checks(state, &task.id, None).await;
    }
    moved
}

/// A hand-off card just went to Ready: the board's runner never starts hand-offs, so the
/// hand-off goes on (or the card to review) without waiting for the next tick. In the
/// background: the caller may hold [`SERIAL`].
pub fn kick_handoff(state: &AppState, task: &Task) {
    if task.source_kind != "delegation" || task.held_at.is_some() {
        return;
    }
    let Some(id) = task.source_id.clone() else { return };
    let state = state.clone();
    tokio::spawn(async move { on_delegation(&state, &id).await });
}

pub async fn on_delegation(state: &AppState, delegation_id: &str) {
    if !super::enabled(state).await {
        return;
    }
    let _serial = SERIAL.lock().await;
    let Ok(Some(d)) = state.db.get_delegation(delegation_id).await else { return };
    let mut task = state.db.task_by_source("delegation", &d.id).await.ok().flatten();
    let live = matches!(d.status.as_str(), "queued" | "running");
    if task.is_none() {
        // A hand-off that ended before its first update got here (it failed at once) still gets
        // its card, which then takes the end below
        if !live && !matches!(d.status.as_str(), "completed" | "failed" | "cancelled" | "interrupted") {
            return;
        }
        let Ok(Some(space)) = state.db.space_of_kwant(&d.specialist_kwant_id).await else { return };
        let mission = mission_of_holder(state, &d.lead_kwant_id).await;
        let title = if d.title.trim().is_empty() { d.task.lines().next().unwrap_or("Hand-off").to_string() } else { d.title.clone() };
        let place = (None, None, Some(d.thread_id.clone()));
        task = start_held(state, &space, mission, &title, &d.task, &d.specialist_kwant_id, "delegation", &d.id, place, &d.thread_id).await;
        if live || task.is_none() {
            return;
        }
    }
    match (task, d.status.as_str()) {
        (None, _) => {}
        (Some(t), _) if live && t.state == "ready" => hold(state, &t, &d.specialist_kwant_id, &d.thread_id).await,
        // Sent back to Ready after the hand-off ended (the person unblocked or retried the card,
        // or AutoFix retried it): the board never runs hand-offs itself, so it goes on here
        (Some(t), status) if t.state == "ready" => {
            if t.held_at.is_some() {
                return;
            }
            if super::over(t.spent_tokens, t.budget_tokens) {
                // Unblocked over its token limit (kwant couldn't take it back): blocked again, so
                // the person raises or removes the limit instead of the card sitting in Ready
                let change = TaskChange {
                    kind: "budget".into(),
                    reason: Some("budget".into()),
                    detail: Some(serde_json::json!({"spent_tokens": t.spent_tokens, "budget_tokens": t.budget_tokens})),
                    ..Default::default()
                };
                let _ = transition(state, &t.id, "ready", "blocked", Actor::Kwant, "kwant", change).await;
                return;
            }
            if status == "completed" {
                // It finished while the card was blocked: its result is in, so it goes to review
                hold(state, &t, &d.specialist_kwant_id, &d.thread_id).await;
                if let Ok(Some(t)) = state.db.get_task(&t.id).await {
                    finish(state, &t, "review", "the hand-off had finished").await;
                }
            } else if !crate::server::delegation::carry_on(state, &d).await {
                let change = TaskChange { reason: Some(CANT_CARRY_ON.into()), ..Default::default() };
                let _ = transition(state, &t.id, "ready", "blocked", Actor::Kwant, "kwant", change).await;
            }
        }
        (Some(t), _) if live => {
            // Still going: keep kwant's lease on it
            let _ = state.db.refresh_claim(&t.id, lease_ms(state).await).await;
        }
        // It ended while its card was blocked (over its token limit, or by the person): the
        // card says so, and unblocking it goes on from there (see the `ready` arm)
        (Some(t), status) if t.state == "blocked" => {
            let note = if status == "completed" { "the hand-off finished; unblock to review its result" } else { "the hand-off stopped; unblock to carry on" };
            if !ended_noted(state, &t.id, status).await {
                let _ = state.db.log_task_event(&t.id, "handoff_ended", "kwant", &serde_json::json!({"status": status, "note": note})).await;
                super::emit_task(state, &t).await;
            }
        }
        (Some(t), "completed") => {
            finish(state, &t, "review", "the hand-off finished").await;
        }
        (Some(t), "failed") => {
            let why = d.error.as_deref().unwrap_or("the hand-off failed");
            if finish(state, &t, "failed", why).await {
                super::autofix::record(state, &t.id, "handoff", "error", why, why.lines().next().unwrap_or(why)).await;
            }
        }
        (Some(t), "cancelled") => {
            finish(state, &t, "cancelled", "the hand-off was cancelled").await;
        }
        // Cut short by two restarts: failed, so the card offers Retry (a Ready card would wait for
        // nobody)
        (Some(t), "interrupted") => {
            let why = "kwnt restarted twice while the hand-off ran";
            if finish(state, &t, "failed", why).await {
                super::autofix::record(state, &t.id, "handoff", "error", why, why).await;
            }
        }
        _ => {}
    }
}

/// Why a hand-off card sent back to Ready can't go on, so it is blocked for the person instead.
pub const CANT_CARRY_ON: &str = "the hand-off can't carry on (a bot is gone or it was restarted elsewhere): ask the lead again";

/// Whether the hand-off's end (`status`) was already noted on a blocked task.
async fn ended_noted(state: &AppState, task_id: &str, status: &str) -> bool {
    let events = state.db.task_events(task_id, 50).await.unwrap_or_default();
    let since_block = events.iter().rposition(|e| e.to_state.as_deref() == Some("blocked")).unwrap_or(0);
    events[since_block..].iter().any(|e| e.kind == "handoff_ended" && e.detail.contains(&format!("\"status\":\"{status}\"")))
}

pub async fn on_routine_run(state: &AppState, routine_id: &str, run_id: &str, status: &str) {
    if !super::enabled(state).await {
        return;
    }
    let _serial = SERIAL.lock().await;
    let task = state.db.task_by_source("routine", run_id).await.ok().flatten();
    match (task, status) {
        (None, "running") => {
            let Ok(Some(r)) = state.db.get_routine(routine_id).await else { return };
            let space = SpaceId::new(&r.space_id);
            let context = r.channel_id.clone().or_else(|| r.thread_id.clone()).unwrap_or_else(|| r.kwant_id.clone());
            let direct = if r.channel_id.is_none() && r.thread_id.is_none() { Some(r.kwant_id.clone()) } else { None };
            let place = (r.channel_id.clone(), direct, r.thread_id.clone());
            let title = format!("Routine: {}", r.name);
            start_held(state, &space, None, &title, &r.instructions, &r.kwant_id, "routine", run_id, place, &context).await;
        }
        (Some(t), "running") => {
            let _ = state.db.refresh_claim(&t.id, lease_ms(state).await).await;
        }
        (Some(t), "ok") => {
            finish(state, &t, "review", "the routine run finished").await;
        }
        (Some(t), "error") => {
            let failed = finish(state, &t, "failed", "the routine run failed").await;
            if failed {
                let err = state.db.routine_run_error(run_id).await.ok().flatten().unwrap_or_else(|| "the routine run failed".into());
                super::autofix::record(state, &t.id, "routine", "error", &err, err.lines().next().unwrap_or("the routine run failed")).await;
            }
        }
        _ => {}
    }
}

/// Working tasks whose owner waits on a card go to `needs_you`; back to `working` (fresh lease)
/// once nothing is open.
pub async fn refresh_needs_you(state: &AppState) {
    if !super::enabled(state).await {
        return;
    }
    let _serial = SERIAL.lock().await;
    let mut open = state.db.tasks_in_state("working").await.unwrap_or_default();
    open.extend(state.db.tasks_in_state("needs_you").await.unwrap_or_default());
    for t in open {
        let waiting = !super::needs_you(state, &t).await.is_empty();
        match (t.state.as_str(), waiting) {
            ("working", true) => {
                let change = TaskChange { reason: Some("waiting on the person".into()), ..Default::default() };
                let _ = transition(state, &t.id, "working", "needs_you", Actor::Kwant, "kwant", change).await;
            }
            ("needs_you", false) => {
                let _ = state.db.refresh_claim(&t.id, lease_ms(state).await).await;
                let change = TaskChange { reason: Some("the person answered".into()), ..Default::default() };
                let _ = transition(state, &t.id, "needs_you", "working", Actor::Kwant, "kwant", change).await;
            }
            _ => {}
        }
    }
}

/// `task_id` as it stands, after moving it back to `working` if it waits in `needs_you` on
/// nothing: a card answered in time sends no event, so a bot's hand-in or the turn's end can
/// come before [`refresh_needs_you`] has run.
pub async fn settle_needs_you(state: &AppState, task_id: &str) -> Option<Task> {
    let _serial = SERIAL.lock().await;
    let t = state.db.get_task(task_id).await.ok().flatten()?;
    if t.state != "needs_you" || !super::needs_you(state, &t).await.is_empty() {
        return Some(t);
    }
    let _ = state.db.refresh_claim(&t.id, lease_ms(state).await).await;
    let change = TaskChange { reason: Some("the person answered".into()), ..Default::default() };
    match transition(state, &t.id, "needs_you", "working", Actor::Kwant, "kwant", change).await {
        Ok(t) => Some(t),
        Err(_) => state.db.get_task(task_id).await.ok().flatten(),
    }
}

/// A work-ahead run posted `text`: a task in `review` owned by the lead, so the person sees it on
/// the board and verifies it (or a teammate reviews it).
pub async fn on_work_ahead_posted(state: &AppState, lead: &crate::db::models::Kwant, place: (Option<String>, Option<String>, Option<String>), run_id: &str, text: &str) {
    if !super::enabled(state).await {
        return;
    }
    let space = SpaceId::new(&lead.space_id);
    let first = text.lines().find(|l| !l.trim().is_empty()).unwrap_or("Worked ahead").trim();
    let new = NewTask {
        mission_id: None,
        title: format!("Worked ahead: {}", first.chars().take(150).collect::<String>()),
        acceptance: "The person finds the work-ahead result useful.".into(),
        state: "review".into(),
        priority: 1,
        owner_kwant_id: Some(lead.id.clone()),
        suggested_kwant_id: Some(lead.id.clone()),
        budget_tokens: super::default_task_budget(state).await,
        max_attempts: 1,
        source_kind: "work_ahead".into(),
        source_id: Some(run_id.into()),
        channel_id: place.0,
        direct_kwant_id: place.1,
        thread_id: place.2,
        created_by: "kwant".into(),
    };
    if let Err(e) = super::create_task(state, &space, new).await {
        tracing::warn!(error = %e, "could not create a work-ahead task");
    }
}
