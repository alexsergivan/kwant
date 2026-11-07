//! Show it once (0.7.9, spec §S): the person shows a bot a task once on the Take over stage,
//! kwant records the steps (masked at capture, addresses scrubbed, never pixels), and the bot's
//! own engine drafts a pending skill from them in a turn with no tools. Contract:
//! next-079/show-once/api.md.
//!
//! - [`stage`]: the recorder on the stream socket that holds Take over.
//! - [`capture`], [`mask`], [`scrub`]: what a step keeps.
//! - [`routes`]: `/api/show-once/...`, person-only.
//! - [`draft`]: Write the skill. [`trial`]: Try it once.
//!
//! Every time here comes from [`crate::clock::now`]: the 30-minute limit and every 7-day deletion.

pub mod capture;
pub mod draft;
pub mod mask;
pub mod routes;
pub mod scrub;
pub mod stage;
pub mod trial;

use super::AppState;
use crate::db::show_once::{rfc3339, Recording, StepRow, LIMIT_SECS, LIMIT_STEPS, NEAR_SECS, NEAR_STEPS_LEFT};
use crate::drivers::events::RuntimeEvent;
use mask::Mask;
use serde_json::{json, Value};
use std::time::Duration;

/// Seconds since the epoch on kwant's clock (a test can move it).
pub fn now() -> i64 {
    crate::clock::now().timestamp()
}

/// A recording as the API and the SSE event show it (api.md §2). Never any step.
pub fn recording_json(r: &Recording, now: i64, engine: Option<&str>) -> Value {
    json!({
        "id": r.id,
        "kwant_id": r.kwant_id,
        "space_id": r.space_id,
        "stage": r.stage,
        "state": r.state,
        "started_at": rfc3339(r.started_at),
        "ended_at": r.ended_at.map(rfc3339),
        "elapsed_secs": r.elapsed_at(now),
        "step_count": r.step_count,
        "note_count": r.note_count,
        "limits": {"steps": LIMIT_STEPS, "secs": LIMIT_SECS, "near_steps_left": NEAR_STEPS_LEFT, "near_secs": NEAR_SECS},
        "near": r.near_at(now),
        "stop_reason": r.stop_reason,
        "origins": r.origin_list(),
        "engine": engine,
        "write_error": r.write_error,
        "skill_id": r.skill_id,
        "steps_expire_at": r.steps_expire_at.map(rfc3339),
        "steps_deleted_at": r.steps_deleted_at.map(rfc3339),
    })
}

/// A step as the review and the stage's socket show it (api.md §3); `n` is its place in the list.
pub fn step_json(s: &StepRow, n: usize, now: i64) -> Value {
    let mask = s.masked.as_deref().and_then(Mask::parse);
    let deleted = s.deleted_at.is_some() && s.undo_until.is_some_and(|u| u > now);
    json!({
        "id": s.id,
        "n": n,
        "kind": s.kind,
        "origin": s.origin,
        "path": s.path,
        "role": s.role,
        "name": s.name,
        "label": s.label,
        // a masked step never has a value, whatever is stored
        "value": if mask.is_some() { None } else { s.value.clone() },
        "masked": mask.map(Mask::as_str),
        "placeholder": mask.map(Mask::placeholder),
        "at": rfc3339(s.at),
        "deleted": deleted,
        "undo_until": if deleted { s.undo_until.map(rfc3339) } else { None },
    })
}

/// The steps in order, numbered from 1 (A35).
pub fn steps_json(steps: &[StepRow], now: i64) -> Vec<Value> {
    steps.iter().enumerate().map(|(i, s)| step_json(s, i + 1, now)).collect()
}

/// Tells the bot's context a recording changed (SSE `skill_recording_updated`, no steps).
pub async fn emit(state: &AppState, r: &Recording) {
    let engine = engine_for(state, &r.kwant_id).await;
    let recording = recording_json(r, now(), engine.as_deref());
    let _ = state.event_sender.send(RuntimeEvent::RecordingUpdated { recording });
}

/// Emits the recording with this id, if it's still there.
pub async fn emit_id(state: &AppState, id: &str) {
    if let Ok(Some(r)) = state.db.get_recording(id).await {
        emit(state, &r).await;
    }
}

/// The engine Write sends the steps to (Q-S2): the bot's pinned engine family, else every engine
/// it could be routed to (the bot's own routing picks among them), else `None`.
pub async fn engine_for(state: &AppState, kwant_id: &str) -> Option<String> {
    let kwant = state.db.get_kwant(kwant_id).await.ok().flatten()?;
    if let Some(family) = crate::router::parse_pin(&kwant.preferred_model).family {
        return Some(family);
    }
    let space = crate::db::spaces::SpaceId::new(&kwant.space_id);
    let profile_engines = super::profile_turn::profile_engines(state, &space).await;
    let host_off = super::profile_turn::host_off(state, &space).await;
    let models = crate::drivers::detect_available_models(&state.db, &state.engines, &profile_engines, &host_off).await;
    let mut families: Vec<&'static str> = Vec::new();
    for m in &models {
        if let Some(e) = crate::profiles::Engine::parse(&m.id) {
            if !families.contains(&e.as_str()) {
                families.push(e.as_str());
            }
        }
    }
    (!families.is_empty()).then(|| families.join(" or "))
}

/// Runs the sweep once (api.md §7): ends recordings past their 30 minutes (telling a live stage),
/// purges steps past their undo or their 7 days, and tells each bot's context what changed.
pub async fn sweep(state: &AppState) -> crate::db::show_once::SweepCounts {
    let counts = match state.db.sweep_recordings(now()).await {
        Ok(c) => c,
        Err(e) => {
            tracing::warn!("show it once: sweep failed: {e}");
            return Default::default();
        }
    };
    for id in &counts.changed {
        stage::recording_ended(id, "limit_time");
        emit_id(state, id).await;
    }
    counts
}

/// At startup: recordings a restart cut off stop (`restart`), a Write it cut off fails, then the
/// sweep catches up; after that it runs every minute until kwant stops.
pub fn spawn_sweeper(state: AppState, cancel: tokio_util::sync::CancellationToken) {
    tokio::spawn(async move {
        match state.db.stop_leftover_recordings(now()).await {
            Ok(ids) => {
                for id in &ids {
                    emit_id(&state, id).await;
                }
            }
            Err(e) => tracing::warn!("show it once: couldn't stop leftover recordings: {e}"),
        }
        let mut tick = tokio::time::interval(Duration::from_secs(60));
        loop {
            tokio::select! {
                _ = cancel.cancelled() => break,
                _ = tick.tick() => { sweep(&state).await; }
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn step(id: i64, masked: Option<&str>, value: Option<&str>) -> StepRow {
        StepRow {
            id,
            recording_id: "rec_1".into(),
            ord: id as f64,
            kind: "type".into(),
            origin: Some("https://a.example".into()),
            path: Some("/".into()),
            role: None,
            name: None,
            label: Some("Field".into()),
            value: value.map(str::to_string),
            masked: masked.map(str::to_string),
            at: 1_800_000_000,
            deleted_at: None,
            undo_until: None,
        }
    }

    #[test]
    fn a_masked_step_never_shows_a_value_and_carries_its_placeholder() {
        let v = step_json(&step(1, Some("password"), Some("leaked")), 1, 0);
        assert_eq!(v["value"], Value::Null);
        assert_eq!(v["masked"], "password");
        assert_eq!(v["placeholder"], "[your password: use the saved login]");
        let v = step_json(&step(2, None, Some("Last 7 days")), 2, 0);
        assert_eq!((v["value"].as_str(), v["placeholder"].is_null()), (Some("Last 7 days"), true));
    }

    #[test]
    fn steps_are_numbered_from_one_and_deleted_ones_show_their_undo() {
        let mut gone = step(7, None, Some("x"));
        (gone.deleted_at, gone.undo_until) = (Some(100), Some(110));
        let list = steps_json(&[step(3, None, None), gone.clone()], 105);
        assert_eq!((list[0]["n"].as_u64(), list[1]["n"].as_u64()), (Some(1), Some(2)));
        assert_eq!(list[1]["deleted"], true);
        assert!(list[1]["undo_until"].is_string());
        // past its undo it isn't shown as restorable (the sweep purges it)
        assert_eq!(step_json(&gone, 2, 111)["deleted"], false);
    }
}
