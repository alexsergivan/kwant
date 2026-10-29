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
    sweep_unreviewed(state, REVIEW_GRACE).await;
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
                // The lead's review failed it: the teammate fixes it in the hand-off's thread
                if let Some(note) = sent_back_by_review(state, &t).await {
                    if !crate::server::delegation::send_back(state, &d, &note).await {
                        let change = TaskChange { reason: Some(CANT_CARRY_ON.into()), ..Default::default() };
                        let _ = transition(state, &t.id, "ready", "blocked", Actor::Kwant, "kwant", change).await;
                    }
                    return;
                }
                // It finished while the card was blocked: its result is in, so it goes to review
                hold(state, &t, &d.specialist_kwant_id, &d.thread_id).await;
                if let Ok(Some(t)) = state.db.get_task(&t.id).await {
                    if !d.continue_lead {
                        finish(state, &t, "cancelled", NOTICE_CLOSED).await;
                        return;
                    }
                    attach_result(state, &t, &d).await;
                    // The lead's results turn ran while the card was blocked, so it never saw the card
                    if finish(state, &t, "review", "the hand-off had finished").await {
                        crate::server::delegation::spawn_review_turn(state, &d, false);
                    }
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
        // A one-way hand-off: nobody is resumed with its result, so nobody would review it
        (Some(t), "completed") if !d.continue_lead => {
            finish(state, &t, "cancelled", NOTICE_CLOSED).await;
        }
        (Some(t), "completed") => {
            attach_result(state, &t, &d).await;
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

/// Label of the evidence that carries a hand-off's result (see [`attach_result`]).
pub const RESULT_LABEL: &str = "Hand-off result";

/// The finished hand-off's reply as note evidence on its card, once per finish, so the lead's
/// review can cite it (a pass needs evidence; evidence alone never verifies).
async fn attach_result(state: &AppState, task: &Task, d: &crate::db::threads::Delegation) {
    if !matches!(task.state.as_str(), "working" | "needs_you") {
        return;
    }
    let run = format!("{}@{}", d.id, d.ended_at.map(|t| t.timestamp_millis()).unwrap_or_default());
    let on_task = state.db.list_evidence(&task.id).await.unwrap_or_default();
    if on_task.iter().any(|e| e.run_id.as_deref() == Some(run.as_str())) {
        return;
    }
    let text = match &d.result_message_id {
        Some(m) => state.db.get_message(m).await.ok().flatten().map(|m| m.text),
        None => None,
    };
    let e = crate::db::missions::NewEvidence {
        task_id: task.id.clone(),
        kind: "note".into(),
        label: RESULT_LABEL.into(),
        excerpt: Some(text.unwrap_or_else(|| "(The result is in the hand-off's thread.)".into()).chars().take(super::MAX_EXCERPT).collect()),
        captured_by: d.specialist_kwant_id.clone(),
        run_id: Some(run),
        ..Default::default()
    };
    if let Err(err) = state.db.add_evidence(&e).await {
        tracing::warn!(error = %err, task_id = %task.id, "could not attach a hand-off's result");
    }
}

/// The note of the review that sent `task` from review back to Ready, if that was its last move.
pub async fn sent_back_by_review(state: &AppState, task: &Task) -> Option<String> {
    let events = state.db.task_events(&task.id, 500).await.ok()?;
    let last = events.iter().rev().find(|e| e.to_state.is_some())?;
    if last.from_state.as_deref() != Some("review") || last.to_state.as_deref() != Some("ready") {
        return None;
    }
    let since = super::review_since(state, &task.id).await;
    let checks = state.db.list_checks(&task.id).await.ok()?;
    checks
        .iter()
        .filter(|c| c.kind == "peer_review" && c.status == "failed")
        .filter(|c| since.is_none_or(|s| c.finished_at.is_some_and(|f| f >= s)))
        .max_by_key(|c| c.finished_at)
        .map(|c| c.output_excerpt.clone().unwrap_or_else(|| "it isn't done yet".into()))
}

/// For the lead's results turn: the board card of a finished hand-off `d` waiting in review, and
/// how the lead reviews it. `None` when the board is off or the card isn't in review.
pub async fn lead_review_note(state: &AppState, d: &crate::db::threads::Delegation, who: &str) -> Option<String> {
    if !super::enabled(state).await || d.status != "completed" {
        return None;
    }
    // The board may not have seen the end yet: its card goes to review first. Not through
    // [`on_delegation`], whose send-back runs the hand-off, whose end comes back here.
    let t = {
        let _serial = SERIAL.lock().await;
        let t = state.db.task_by_source("delegation", &d.id).await.ok().flatten()?;
        if matches!(t.state.as_str(), "working" | "needs_you") {
            attach_result(state, &t, d).await;
            finish(state, &t, "review", "the hand-off finished").await;
        }
        state.db.get_task(&t.id).await.ok().flatten()?
    };
    if t.state != "review" {
        return None;
    }
    let ev = state.db.list_evidence(&t.id).await.ok()?.into_iter().filter(|e| e.label == RESULT_LABEL).max_by_key(|e| e.captured_at)?;
    let detail = serde_json::json!({"delegation_id": d.id, "lead_kwant_id": d.lead_kwant_id});
    if let Err(e) = state.db.log_task_event(&t.id, REVIEW_ASKED, "kwant", &detail).await {
        tracing::warn!(error = %e, task_id = %t.id, "could not log a lead review ask");
    }
    Some(format!(
        "(Board card {task} is in Review and you, who handed it out, review it: {who} can't approve its own work. Call \
        task_review with task_id \"{task}\": verdict \"pass\" with evidence_ids [\"{ev}\"] (its result) when it does what you \
        asked, or verdict \"fail\" with a note that says exactly what is missing or wrong. A fail sends it back to {who} \
        with your note, so don't also @mention {who} for that correction.)",
        task = t.id,
        ev = ev.id,
    ))
}

/// Task event logged each time the lead is asked to review a hand-off card (its results turn,
/// a review turn or the reminder).
pub const REVIEW_ASKED: &str = "lead_review_asked";

/// How often the lead is asked per stay in Review: its results turn and one reminder. After
/// that the card waits ("Waiting for <lead> to review") for the lead or the person.
pub const MAX_REVIEW_ASKS: usize = 2;

/// Why a finished one-way hand-off (the lead isn't resumed with its result) is closed instead of
/// going to Review, where nobody would look at it.
pub const NOTICE_CLOSED: &str = "one-way hand-off: its result was posted to the chat, and nobody was asked to review it";

/// How long a hand-off card may sit in Review with no lead review running before
/// [`sweep_unreviewed`] asks the lead (again): long enough for the lead's batch turn to start.
pub const REVIEW_GRACE: std::time::Duration = std::time::Duration::from_secs(120);

/// How often the lead was asked to review `task_id` since it last went to Review, and when last.
pub async fn review_asks(state: &AppState, task_id: &str) -> (usize, Option<chrono::DateTime<chrono::Utc>>) {
    let since = super::review_since(state, task_id).await;
    let events = state.db.task_events(task_id, 500).await.unwrap_or_default();
    let asks: Vec<_> = events.iter().filter(|e| e.kind == REVIEW_ASKED && since.is_none_or(|s| e.created_at >= s)).collect();
    (asks.len(), asks.last().map(|e| e.created_at))
}

/// Whether `task` is a hand-off card in Review that its lead should still be asked about:
/// its result is on the card (older cards, from before leads reviewed, are left alone) and the
/// lead hasn't been asked [`MAX_REVIEW_ASKS`] times this stay.
pub async fn wants_lead_review(state: &AppState, task: &Task) -> bool {
    if task.state != "review" || task.source_kind != "delegation" {
        return false;
    }
    let has_result = state.db.list_evidence(&task.id).await.unwrap_or_default().iter().any(|e| e.label == RESULT_LABEL);
    has_result && review_asks(state, &task.id).await.0 < MAX_REVIEW_ASKS
}

/// Hand-off cards in Review whose lead review never came: kwnt restarted before or during the
/// lead's turn, or the reminder was lost. Each gets a review turn once nothing has asked for
/// `grace` and no turn for its hand-off is running; one whose batch still runs waits for it.
/// Called from [`reconcile`] with [`REVIEW_GRACE`].
pub async fn sweep_unreviewed(state: &AppState, grace: std::time::Duration) {
    let now = chrono::Utc::now();
    for t in state.db.tasks_in_state("review").await.unwrap_or_default() {
        let Some(id) = t.source_id.as_deref().filter(|_| t.source_kind == "delegation") else { continue };
        let Ok(Some(d)) = state.db.get_delegation(id).await else { continue };
        if d.status != "completed" || !d.continue_lead || state.turns.has_job(&d.id) {
            continue;
        }
        if !wants_lead_review(state, &t).await || crate::server::delegation::batch_running(state, &d).await {
            continue;
        }
        let (asks, last) = review_asks(state, &t.id).await;
        let quiet_since = last.or(super::review_since(state, &t.id).await).unwrap_or(t.updated_at);
        if (now - quiet_since).to_std().unwrap_or_default() < grace {
            continue;
        }
        crate::server::delegation::spawn_review_turn(state, &d, asks > 0);
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
