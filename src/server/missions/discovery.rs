//! Mission Control Phase D4: Automation Discovery's scan. Reads the person's board tasks of one
//! space, runs the miner (`crate::automation_discovery`) and says which routines it would suggest
//! and why every other habit gets none. Step 2 is a preview: it stores nothing, neither an offer
//! nor a routine, and offering, Accept and Dismiss come in step 3. Off by default. See
//! docs/mission-control.md §20 and mission-control/phase-d/automation-discovery.md.

use super::routes::internal;
use crate::automation_discovery::{self as miner, Delivery, Input, Offer, Scan};
use crate::db::spaces::SpaceId;
use crate::db::DbError;
use crate::routines::now_in;
use crate::server::spaces::Scope;
use crate::server::AppState;
use axum::{
    extract::State,
    response::{IntoResponse, Response},
    Json,
};
use chrono::{DateTime, Utc};
use serde_json::{json, Value};
use std::collections::HashSet;

pub const SETTING: &str = "automation_discovery";
pub const MODES: [&str; 2] = ["off", "on"];
pub const CAP_SETTING: &str = "automation_discovery_daily_cap";

pub async fn enabled(state: &AppState) -> bool {
    matches!(state.db.get_setting(SETTING).await.ok().flatten().as_deref().map(str::trim), Some("on"))
}

/// Offers made per space per 24 hours (0–10; 0 offers none).
pub async fn daily_cap(state: &AppState) -> i64 {
    super::int_setting(state, CAP_SETTING, miner::DAILY_CAP_RANGE, miner::DEFAULT_DAILY_CAP).await
}

pub fn settings_error(mode: Option<&str>, cap: Option<i64>) -> Option<String> {
    if let Some(m) = mode.filter(|m| !MODES.contains(m)) {
        return Some(format!("{SETTING} must be off or on (got {m})"));
    }
    let (lo, hi) = miner::DAILY_CAP_RANGE;
    if let Some(n) = cap.filter(|n| !(lo..=hi).contains(n)) {
        return Some(format!("{CAP_SETTING} must be {lo}–{hi} (got {n})"));
    }
    None
}

/// Why no scan runs: `off` (the setting), `board_off` (the Mission board is off, so there are no
/// board tasks to learn from) or `no_timezone` (habits are local times, and none is set, or it is
/// one this machine can't read). `Ok` with the zone when a scan can run.
async fn blocked_by(state: &AppState, now: DateTime<Utc>) -> Result<String, &'static str> {
    if !enabled(state).await {
        return Err("off");
    }
    if !super::enabled(state).await {
        return Err("board_off");
    }
    let tz = state.db.get_setting(crate::server::bot_actions::TIMEZONE_SETTING).await.ok().flatten().map(|t| t.trim().to_string()).unwrap_or_default();
    if tz.is_empty() || now_in(&tz, now).is_err() {
        return Err("no_timezone");
    }
    Ok(tz)
}

/// Where an offer would deliver, if that place is still there: its bot's own unarchived thread,
/// or a channel of the space; else the bot's direct chat.
async fn deliverable(state: &AppState, space: &str, offer: &Offer) -> Delivery {
    let direct = Delivery { direct_kwant_id: Some(offer.kwant_id.clone()), ..Default::default() };
    if let Some(t) = &offer.delivery.thread_id {
        return match state.db.get_thread(t).await {
            Ok(Some(t)) if t.kwant_id == offer.kwant_id && !t.archived => offer.delivery.clone(),
            _ => direct,
        };
    }
    if let Some(c) = &offer.delivery.channel_id {
        return match state.db.get_channel(c).await {
            Ok(Some(c)) if c.space_id == space => offer.delivery.clone(),
            _ => direct,
        };
    }
    direct
}

/// One scan of `space` at `now`, read-only: what would be offered and what was skipped.
pub async fn preview(state: &AppState, space_id: &SpaceId, now: DateTime<Utc>) -> Result<Value, DbError> {
    let space = space_id.as_str();
    let tz = match blocked_by(state, now).await {
        Ok(tz) => tz,
        Err(why) => return Ok(json!({ "blocked_by": why, "preview": true, "offered": [], "skipped": [] })),
    };
    let tasks = state.db.discovery_tasks(space, now).await?;
    let checks = state.db.discovery_checks(space, now).await?;
    let routines = state.db.discovery_routines(space).await?;
    let past = state.db.discovery_past(space, now).await?;
    let bots: HashSet<String> = state.db.list_kwants(space_id).await?.into_iter().map(|k| k.id).collect();
    let Scan { mut offered, skipped } = miner::mine(&Input {
        tz: &tz,
        now,
        tasks: &tasks,
        checks: &checks,
        routines: &routines,
        bots: &bots,
        past: &past,
        daily_cap: usize::try_from(daily_cap(state).await).unwrap_or(0),
        max_live: miner::MAX_LIVE_PER_SPACE,
    });
    for o in &mut offered {
        o.delivery = deliverable(state, space, o).await;
        // Shown on a card: secrets in a check's spec never are
        for c in &mut o.checks {
            c.spec = super::autofix::redact_value(&c.spec);
        }
        for c in &mut o.left_out_checks {
            c.spec = super::autofix::redact_value(&c.spec);
        }
    }
    Ok(json!({
        "blocked_by": null,
        "preview": true,
        "timezone": tz,
        "offered": offered,
        "skipped": skipped,
    }))
}

/// `POST /api/automation-discovery/scan?space=` (person only): what a scan would offer now, and
/// why every other habit gets none. Writes nothing; with the setting off it reads nothing either.
pub async fn scan(State(state): State<AppState>, Scope(space): Scope) -> Response {
    match preview(&state, &space, Utc::now()).await {
        Ok(v) => Json(v).into_response(),
        Err(e) => internal(e),
    }
}
