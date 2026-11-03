//! The goal loop. Every [`TICK`] it deletes week-old drafts, calls met goals whose evidence is out
//! of date unknown, sends the one "can't check" notice after a day of unknown, and runs the goals
//! that are due, each under its lease (`checking_since`) so two runs never overlap. With no
//! approved goals it does nothing but those queries: an upgrade starts no background work.

use super::{decide, emit, emit_goal, jitter_ms, next_interval_minutes, notice, on_change, predicate, predicate_of, Judged, RunSays};
use super::{DRAFT_TTL_MS, UNKNOWN_NOTICE_MS};
use crate::db::goals::{Goal, NewObservation, RunUpdate};
use crate::db::missions::now_ms;
use crate::server::AppState;
use serde_json::json;
use std::path::PathBuf;
use std::sync::{Arc, LazyLock};
use tokio::sync::Semaphore;
use tokio_util::sync::CancellationToken;

const TICK: std::time::Duration = std::time::Duration::from_secs(30);
/// Goals checked at once by the loop, server-wide.
const PARALLEL: usize = 4;

static RUNS: LazyLock<Arc<Semaphore>> = LazyLock::new(|| Arc::new(Semaphore::new(PARALLEL)));

pub fn spawn(state: AppState, shutdown: CancellationToken) {
    tokio::spawn(async move {
        loop {
            tokio::select! {
                _ = shutdown.cancelled() => return,
                _ = tokio::time::sleep(TICK) => {}
            }
            tick(&state).await;
        }
    });
}

/// One pass of the loop; the ids of the goals it started checking.
pub async fn tick(state: &AppState) -> Vec<String> {
    let now = now_ms();
    for (id, space) in state.db.expire_drafts(now - DRAFT_TTL_MS).await.unwrap_or_default() {
        emit(state, &id, "deleted", &space);
    }
    for id in state.db.stale_met_goals(now).await.unwrap_or_default() {
        if let Ok(Some(g)) = state.db.set_goal_status(&id, "unknown", "out_of_date", &["met"], Some(("unknown", json!({ "reason": "out_of_date" })))).await {
            emit_goal(state, &g);
        }
    }
    for g in state.db.goals_unknown_since(now - UNKNOWN_NOTICE_MS).await.unwrap_or_default() {
        let body = "kwant hasn't been able to check this for a day, so it can't say whether it's true. Open the Mission board to see what went wrong.".to_string();
        notice(state, &g, "cant_check", format!("Can't check {}", g.title), body).await;
    }
    let mut started = Vec::new();
    let free = RUNS.available_permits();
    if free == 0 {
        return started;
    }
    for id in state.db.due_goal_ids(now, free as i64).await.unwrap_or_default() {
        let Ok(permit) = RUNS.clone().try_acquire_owned() else { break };
        let Ok(Some(g)) = state.db.lease_goal(&id, now_ms(), true).await else { continue };
        started.push(g.id.clone());
        let state = state.clone();
        tokio::spawn(async move {
            run_goal(&state, g).await;
            drop(permit);
        });
    }
    started
}

/// Where a goal's command and file checks run: its folder, else its bot's.
pub fn goal_folder(state: &AppState, g: &Goal) -> Option<PathBuf> {
    if !g.folder.trim().is_empty() {
        return Some(PathBuf::from(g.folder.trim()));
    }
    if g.kwant_id.is_empty() {
        return None;
    }
    let space = crate::db::spaces::SpaceId::new(&g.space_id);
    Some(crate::tools::bot_workspace(&crate::db::spaces::space_data_dir(&state.config.data_dir, &space), &g.kwant_id))
}

/// Runs every check of a goal it holds the lease for (`g` as [`lease_goal`] returned it) and
/// writes the outcome. Gives up quietly if it loses the lease (the goal was deleted, or the run
/// took so long another run took over); if the person edited the goal meanwhile, only the lease is
/// freed and their edit stands.
///
/// [`lease_goal`]: crate::db::Database::lease_goal
pub async fn run_goal(state: &AppState, g: Goal) -> Option<Goal> {
    let mut lease = g.checking_since?;
    emit_goal(state, &g);
    let checks = state.db.goal_checks(&g.id).await.unwrap_or_default();
    let folder = goal_folder(state, &g);
    let mut obs = Vec::new();
    let mut judged = Vec::new();
    let mut failing = Vec::new();
    for c in &checks {
        let spec = c.spec_value();
        let pred = predicate_of(c);
        let out = predicate::run(&c.kind, &spec, pred.as_ref(), folder.as_deref(), true).await;
        let now = now_ms();
        if !state.db.renew_goal_lease(&g.id, lease, now).await.unwrap_or(false) {
            return None;
        }
        lease = now;
        if out.verdict == predicate::NOT_MET && c.required() {
            let says = predicate::says(&c.kind, &spec, pred.as_ref());
            failing.push(match &out.observed {
                Some(o) => format!("{says} (it saw {o})"),
                None => says,
            });
        }
        judged.push(Judged { required: c.required(), verdict: out.verdict, errored: out.status == "error" });
        obs.push(NewObservation {
            check_id: c.id.clone(),
            status: out.status,
            verdict: out.verdict.to_string(),
            observed_value: out.observed,
            excerpt: out.excerpt,
            expires_at: now + c.freshness_minutes.max(1) * 60_000,
        });
    }
    let now = now_ms();
    if !state.db.record_observations(&g.id, lease, &obs, now).await.unwrap_or(false) {
        return None;
    }
    let (status, fail_streak, error_runs, says) = decide(&g.status, g.fail_streak, g.error_runs, g.confirm_failures, &judged);
    // A met goal that had one blip stays as it was: nothing about it changed
    let held_blip = status == "met" && g.status == "met" && says == RunSays::Unknown && g.fail_streak == 0;
    let status_reason = if held_blip || (status == g.status && status != "met") {
        g.status_reason.clone()
    } else {
        match says {
            RunSays::Met => String::new(),
            RunSays::Failed => "failing".into(),
            RunSays::Unknown => "cant_check".into(),
        }
    };
    let mut events = Vec::new();
    if status != g.status {
        match status.as_str() {
            "not_met" => events.push(("not_met", json!({ "failing": failing, "confirmed_after": fail_streak }))),
            "met" => events.push(("met", json!({}))),
            "unknown" => events.push(("unknown", json!({ "reason": status_reason }))),
            _ => {}
        }
    }
    // Read before this run logs `met`, which closes the stretch the person was told about
    let owed_recovery = status == "met" && g.status != "met" && state.db.told_broken(&g.id).await.unwrap_or(false);
    let next = now + next_interval_minutes(g.interval_minutes, error_runs) * 60_000 + jitter_ms(g.interval_minutes);
    let u = RunUpdate { status, status_reason, fail_streak, error_runs, next_check_at: next, events };
    let after = state.db.finish_goal_run(&g.id, lease, g.version, &u, now).await.ok().flatten()?;
    emit_goal(state, &after);
    if after.version == g.version {
        on_change(state, &g.status, &after, &failing, owed_recovery).await;
    }
    Some(after)
}

