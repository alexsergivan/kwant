//! Mission Control Phase D4: Automation Discovery. Reads the person's board tasks of a space, runs
//! the miner (`crate::automation_discovery`) and keeps what it finds as offers the person accepts
//! (a routine is made) or dismisses (not offered again for 30 days). Nothing is created without
//! Accept. Scans run hourly while the setting is on, or when the person asks; every route is the
//! person's only. Off by default. See docs/mission-control.md §20 and
//! mission-control/phase-d/automation-discovery.md.

// Handler helpers return the ready-made error `Response`
#![allow(clippy::result_large_err)]

use super::routes::{bad, err, internal, not_found};
use crate::automation_discovery::{self as miner, CheckSpec, Delivery, Input, LeftOut, Offer, Scan};
use crate::db::automation::{Decided, SuggestionRow};
use crate::db::spaces::SpaceId;
use crate::db::DbError;
use crate::routines::{now_in, run_time_text, Schedule};
use crate::server::continuity::{notify, Notification};
use crate::server::scheduler::{self, RoutineCheck, RoutinePayload, RoutineView};
use crate::server::spaces::{space_param, Scope};
use crate::server::AppState;
use axum::{
    body::Bytes,
    extract::{Path, Query, RawQuery, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    Json,
};
use chrono::{DateTime, Duration, Utc};
use serde::Deserialize;
use serde_json::{json, Value};
use std::collections::HashSet;
use tokio_util::sync::CancellationToken;

pub const SETTING: &str = "automation_discovery";
pub const MODES: [&str; 2] = ["off", "on"];
pub const CAP_SETTING: &str = "automation_discovery_daily_cap";
/// When the last scan of every space ran (RFC 3339): the hourly scan waits an hour from it.
pub const LAST_SCAN_SETTING: &str = "automation_discovery_last_scan";
/// How often the background loop wakes; it scans when the last scan is [`SCAN_EVERY`] old.
const TICK: std::time::Duration = std::time::Duration::from_secs(10 * 60);
const SCAN_EVERY: Duration = Duration::hours(1);
/// Statuses `GET ?status=` takes; `all` lists every one.
const STATUSES: [&str; 6] = ["offered", "accepted", "dismissed", "expired", "superseded", "all"];

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

/// Every reason no scan runs, for `GET` (empty: scans run).
async fn blockers(state: &AppState, now: DateTime<Utc>) -> Vec<&'static str> {
    let mut why = Vec::new();
    if !enabled(state).await {
        why.push("off");
    }
    if !super::enabled(state).await {
        why.push("board_off");
    }
    let tz = state.db.get_setting(crate::server::bot_actions::TIMEZONE_SETTING).await.ok().flatten().unwrap_or_default();
    if tz.trim().is_empty() || now_in(tz.trim(), now).is_err() {
        why.push("no_timezone");
    }
    why
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

/// Masks secrets in the check specs of an offer that is shown.
fn redact(checks: &mut [CheckSpec], left_out: &mut [LeftOut]) {
    for c in checks {
        c.spec = super::autofix::redact_value(&c.spec);
    }
    for c in left_out {
        c.spec = super::autofix::redact_value(&c.spec);
    }
}

/// Runs the miner over `space` at `now` in `tz`.
async fn mine(state: &AppState, space_id: &SpaceId, tz: &str, now: DateTime<Utc>) -> Result<Scan, DbError> {
    let space = space_id.as_str();
    let tasks = state.db.discovery_tasks(space, now).await?;
    let checks = state.db.discovery_checks(space, now).await?;
    let routines = state.db.discovery_routines(space).await?;
    let past = state.db.discovery_past(space, now).await?;
    let bots: HashSet<String> = state.db.list_kwants(space_id).await?.into_iter().map(|k| k.id).collect();
    Ok(miner::mine(&Input {
        tz,
        now,
        tasks: &tasks,
        checks: &checks,
        routines: &routines,
        bots: &bots,
        past: &past,
        daily_cap: usize::try_from(daily_cap(state).await).unwrap_or(0),
        max_live: miner::MAX_LIVE_PER_SPACE,
    }))
}

/// One scan of `space` at `now`, read-only: what would be offered and what was skipped.
pub async fn preview(state: &AppState, space_id: &SpaceId, now: DateTime<Utc>) -> Result<Value, DbError> {
    let tz = match blocked_by(state, now).await {
        Ok(tz) => tz,
        Err(why) => return Ok(json!({ "blocked_by": why, "preview": true, "offered": [], "skipped": [] })),
    };
    let Scan { mut offered, skipped, .. } = mine(state, space_id, &tz, now).await?;
    for o in &mut offered {
        o.delivery = deliverable(state, space_id.as_str(), o).await;
        redact(&mut o.checks, &mut o.left_out_checks);
    }
    Ok(json!({
        "blocked_by": null,
        "preview": true,
        "timezone": tz,
        "offered": offered,
        "skipped": skipped,
    }))
}

/// An offer as it is stored: where it delivers now, and only checks a routine may carry (the
/// rest are listed as left out). Specs stay unmasked; they are masked whenever shown.
async fn storable(state: &AppState, space: &str, mut o: Offer) -> Offer {
    o.delivery = deliverable(state, space, &o).await;
    let (keep, refused): (Vec<CheckSpec>, Vec<CheckSpec>) = o.checks.into_iter().partition(|c| super::validate_check(&c.kind, &c.spec, true).is_ok());
    o.checks = keep;
    o.left_out_checks.extend(refused.into_iter().map(|c| LeftOut { kind: c.kind, spec: c.spec, reason: "not allowed on a routine" }));
    o
}

/// What a scan of one space did.
#[derive(Debug, Default)]
pub struct Stored {
    /// Suggestion ids stored as new offers.
    pub offered: Vec<String>,
    /// Live offers ended because their habit no longer passes, or changed.
    pub expired: Vec<String>,
    pub superseded: Vec<String>,
    pub skipped: Vec<miner::Skip>,
}

/// One scan of `space` at `now` that keeps what it finds (design §6): a live offer whose habit no
/// longer passes is `expired`; one whose rhythm, time or bot changed is replaced, in one step, by
/// the habit as it is now (outside the daily cap and the live limit, since it adds no offer, and
/// without a second notification). New habits are stored as offers, one notification each. Two
/// scans racing store one offer per habit.
pub async fn scan_space(state: &AppState, space_id: &SpaceId, tz: &str, now: DateTime<Utc>) -> Result<Stored, DbError> {
    let space = space_id.as_str();
    let mut out = Stored::default();
    // Live offers first, then the miner: an offer another scan stores in between isn't in `live`,
    // so it can't be mistaken for one whose habit is gone (and expired, and offered twice)
    let live = state.db.list_suggestions(space, Some("offered")).await?;
    let mut scan = mine(state, space_id, tz, now).await?;
    for row in &live {
        let now_offer = scan.live.iter().find(|o| o.cluster_key == row.cluster_key);
        match (now_offer, serde_json::from_str::<Schedule>(&row.schedule)) {
            (None, _) => {
                if state.db.end_offer(&row.id, "expired", now).await? {
                    out.expired.push(row.id.clone());
                }
            }
            (Some(o), Ok(s)) if !miner::materially_changed(&s, &row.kwant_id, o) => {}
            (Some(o), _) => {
                let o = storable(state, space, o.clone()).await;
                // `None`: another scan or the person decided it first; that decision stands
                if state.db.replace_offer(&row.id, space, &o, now).await?.is_some() {
                    out.superseded.push(row.id.clone());
                }
            }
        }
    }
    // Expired offers free their habits and their room: mine again with that
    if !out.expired.is_empty() {
        scan = mine(state, space_id, tz, now).await?;
    }
    let cap = daily_cap(state).await;
    for o in scan.offered {
        let key = o.cluster_key.clone();
        let o = storable(state, space, o).await;
        match state.db.store_offer(space, &o, now, cap, miner::MAX_LIVE_PER_SPACE).await? {
            Some(id) => {
                announce(state, &o).await;
                out.offered.push(id);
            }
            // Another scan got there first, or the space filled up meanwhile
            None => out.skipped.push(miner::Skip { cluster_key: key, title: o.name.clone(), asks: o.evidence.total, reason: miner::Reason::Capped }),
        }
    }
    out.skipped.extend(scan.skipped);
    out.skipped.sort_by(|a, b| a.cluster_key.cmp(&b.cluster_key));
    Ok(out)
}

/// The one notification a new offer gets (design §10): its name and bot, nothing from a chat.
async fn announce(state: &AppState, o: &Offer) {
    let kwant_name = match state.db.get_kwant(&o.kwant_id).await {
        Ok(Some(k)) => k.name,
        _ => return,
    };
    notify(
        state,
        Notification {
            kind: "automation_suggested",
            kwant_id: o.kwant_id.clone(),
            body: format!("{kwant_name} could do this on a schedule. Accept or dismiss it in Routines."),
            title: format!("Suggested routine: {}", o.name),
            kwant_name,
            channel_id: None,
        },
    )
    .await;
}

/// The hourly scan of every space (when the setting is on), and the retention pass.
pub async fn tick(state: &AppState, now: DateTime<Utc>) -> usize {
    // Retention marks routines already accepted, so it runs whatever the setting says now
    if let Err(e) = state.db.mark_retention(now).await {
        tracing::warn!(error = %e, "automation discovery retention");
    }
    let Ok(tz) = blocked_by(state, now).await else { return 0 };
    if last_scan(state).await.is_some_and(|at| now - at < SCAN_EVERY && at <= now) {
        return 0;
    }
    let mut offered = 0;
    for space in state.db.list_spaces().await.unwrap_or_default().into_iter().filter(|s| s.archived_at.is_none()) {
        match scan_space(state, &SpaceId::new(&space.id), &tz, now).await {
            Ok(s) => offered += s.offered.len(),
            Err(e) => tracing::warn!(error = %e, space = %space.id, "automation discovery scan"),
        }
    }
    let _ = state.db.set_setting(LAST_SCAN_SETTING, &now.to_rfc3339()).await;
    offered
}

async fn last_scan(state: &AppState) -> Option<DateTime<Utc>> {
    let v = state.db.get_setting(LAST_SCAN_SETTING).await.ok().flatten()?;
    DateTime::parse_from_rfc3339(v.trim()).ok().map(|t| t.with_timezone(&Utc))
}

pub fn spawn(state: AppState, shutdown: CancellationToken) {
    tokio::spawn(async move {
        loop {
            tokio::select! {
                _ = shutdown.cancelled() => return,
                _ = tokio::time::sleep(TICK) => {}
            }
            tick(&state, Utc::now()).await;
        }
    });
}

// What the person sees

fn tier_label(tier: &str) -> &'static str {
    match tier {
        "observe" => "Observe: every change asks you first",
        "sandbox" => "Sandbox: commands that can change things ask you first",
        _ => "Act: works on its own, within its approvals",
    }
}

/// What the bot may do when the routine runs (design §5): a preview of what it already has, never
/// a grant.
async fn permissions(state: &AppState, row: &SuggestionRow, sources: &[String]) -> Value {
    let flight_school = super::flight_school::enabled(&state.db).await;
    // Off, Flight School holds nothing back: the bot acts as it does today
    let (tier, tier_from) = if flight_school { super::flight_school::tier_of(&state.db, &row.kwant_id, &row.task_class).await } else { ("act", "off") };
    let approval_mode = state.db.kwant_approvals(&row.kwant_id).await.map(|m| m.effective).unwrap_or_else(|_| "ask".into());
    let past = state.db.past_tools(sources).await.unwrap_or_default();
    json!({
        "flight_school": flight_school,
        "tier": tier,
        "tier_from": tier_from,
        "tier_label": tier_label(tier),
        "approval_mode": approval_mode,
        "payments_ask": true,
        "past": past,
    })
}

/// A stored suggestion as the API returns it (design §11): its JSON columns parsed, check specs
/// masked, and for a live offer the first run recomputed for `now` from its schedule (§5).
async fn suggestion_json(state: &AppState, row: &SuggestionRow, now: DateTime<Utc>) -> Value {
    let parse = |s: &str| serde_json::from_str::<Value>(s).unwrap_or(Value::Null);
    let mut evidence = parse(&row.evidence);
    let sources = evidence.get_mut("sources").map(Value::take).unwrap_or_else(|| json!([]));
    if let Some(e) = evidence.as_object_mut() {
        e.remove("sources");
    }
    let schedule = serde_json::from_str::<Schedule>(&row.schedule).ok();
    let mut first_run_at = row.first_run_at;
    if row.status == "offered" {
        let last_ask = evidence["last_ask_at"].as_str().and_then(|t| DateTime::parse_from_rfc3339(t).ok()).map_or(row.offered_at, |t| t.with_timezone(&Utc));
        if let Some(at) = schedule.as_ref().and_then(|s| miner::first_run_at(s, last_ask, now)) {
            first_run_at = at;
        }
    }
    let tz = schedule.as_ref().and_then(Schedule::tz);
    let masked = |s: &str| -> Value {
        let mut list: Vec<Value> = serde_json::from_str(s).unwrap_or_default();
        for c in &mut list {
            c["spec"] = super::autofix::redact_value(&c["spec"]);
        }
        Value::Array(list)
    };
    let source_ids: Vec<String> = serde_json::from_str(&row.source_task_ids).unwrap_or_default();
    let schedule_value = parse(&row.schedule);
    json!({
        "id": row.id,
        "space_id": row.space_id,
        "cluster_key": row.cluster_key,
        "kwant_id": row.kwant_id,
        "name": row.name,
        "instructions": row.instructions,
        "schedule_text": crate::server::bot_actions::schedule_text(&schedule_value),
        "schedule": schedule_value,
        "first_run_at": first_run_at,
        "first_run_text": run_time_text(first_run_at, tz),
        "delivery": parse(&row.delivery),
        "task_class": row.task_class,
        "acceptance": row.acceptance,
        "checks": masked(&row.checks),
        "left_out_checks": masked(&row.left_out_checks),
        "chain": parse(&row.chain),
        "source_task_ids": source_ids,
        "sources": sources,
        "time_spread": evidence["time_spread"].clone(),
        "evidence": evidence,
        "permissions": permissions(state, row, &source_ids).await,
        "status": row.status,
        "routine_id": row.routine_id,
        "cooldown_until": row.cooldown_until,
        "routine_deleted_at": row.routine_deleted_at,
        "retention": row.retention,
        "retention_at": row.retention_at,
        "offered_at": row.offered_at,
        "decided_at": row.decided_at,
    })
}

async fn suggestions_json(state: &AppState, ids: &[String], now: DateTime<Utc>) -> Vec<Value> {
    let mut out = Vec::new();
    for id in ids {
        if let Ok(Some(row)) = state.db.get_suggestion(id).await {
            out.push(suggestion_json(state, &row, now).await);
        }
    }
    out
}

// Routes

#[derive(Deserialize, Default)]
pub struct ListQuery {
    status: Option<String>,
}

/// `GET /api/automation-discovery?space=&status=` (person only): the setting, what keeps scans
/// from running, the last scan, the suggestions (`status`, default `offered`; `all`) and stats.
/// While a scan can't run, live offers are hidden.
pub async fn list(State(state): State<AppState>, Scope(space): Scope, Query(q): Query<ListQuery>) -> Response {
    let now = Utc::now();
    let status = q.status.as_deref().map(str::trim).filter(|s| !s.is_empty()).unwrap_or("offered");
    if !STATUSES.contains(&status) {
        return bad("status must be offered, accepted, dismissed, expired, superseded or all");
    }
    let blocked_by = blockers(&state, now).await;
    let rows = match state.db.list_suggestions(space.as_str(), (status != "all").then_some(status)).await {
        Ok(r) => r,
        Err(e) => return internal(e),
    };
    let mut suggestions = Vec::new();
    for row in rows.iter().filter(|r| blocked_by.is_empty() || r.status != "offered") {
        suggestions.push(suggestion_json(&state, row, now).await);
    }
    let stats = match state.db.suggestion_stats(space.as_str(), now).await {
        Ok(s) => s,
        Err(e) => return internal(e),
    };
    Json(json!({
        "setting": if enabled(&state).await { "on" } else { "off" },
        "daily_cap": daily_cap(&state).await,
        "blocked_by": blocked_by,
        "last_scan_at": last_scan(&state).await,
        "suggestions": suggestions,
        "stats": stats,
    }))
    .into_response()
}

#[derive(Deserialize, Default)]
pub struct ScanQuery {
    #[serde(default)]
    preview: Option<String>,
}

/// `POST /api/automation-discovery/scan?space=` (person only): scans now, ignoring the hourly
/// wait but not the cap, and keeps the offers. `?preview=1` only says what it would offer and
/// writes nothing. With the setting off nothing is read.
pub async fn scan(State(state): State<AppState>, Scope(space): Scope, Query(q): Query<ScanQuery>) -> Response {
    let now = Utc::now();
    if q.preview.as_deref().is_some_and(|p| matches!(p.trim(), "1" | "true" | "yes")) {
        return match preview(&state, &space, now).await {
            Ok(v) => Json(v).into_response(),
            Err(e) => internal(e),
        };
    }
    let tz = match blocked_by(&state, now).await {
        Ok(tz) => tz,
        Err(why) => return Json(json!({ "blocked_by": why, "preview": false, "offered": [], "skipped": [], "expired": [], "superseded": [] })).into_response(),
    };
    let stored = match scan_space(&state, &space, &tz, now).await {
        Ok(s) => s,
        Err(e) => return internal(e),
    };
    let _ = state.db.set_setting(LAST_SCAN_SETTING, &now.to_rfc3339()).await;
    Json(json!({
        "blocked_by": null,
        "preview": false,
        "timezone": tz,
        "offered": suggestions_json(&state, &stored.offered, now).await,
        "skipped": stored.skipped,
        "expired": stored.expired,
        "superseded": stored.superseded,
    }))
    .into_response()
}

/// `POST …/:id/accept` body: overrides of the suggestion, all optional (design §6).
#[derive(Debug, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct AcceptBody {
    pub name: Option<String>,
    pub instructions: Option<String>,
    pub schedule: Option<Value>,
    pub kwant_id: Option<String>,
    /// A channel of the space; `null` or `""`: the bot's direct chat.
    #[serde(default, deserialize_with = "scheduler::double_option")]
    pub channel_id: Option<Option<String>>,
    /// One of the bot's own threads.
    #[serde(default, deserialize_with = "scheduler::double_option")]
    pub thread_id: Option<Option<String>>,
    #[serde(default, deserialize_with = "scheduler::double_option")]
    pub acceptance: Option<Option<String>>,
    /// Indexes into the suggestion's checks to keep (default: all). Checks can be dropped here,
    /// not added.
    pub keep_checks: Option<Vec<usize>>,
}

/// The suggestion `id`, refused (404) when an explicit `?space=` names another space.
async fn load(state: &AppState, query: Option<&str>, id: &str) -> Result<SuggestionRow, Response> {
    match state.db.get_suggestion(id).await {
        Ok(Some(s)) if space_param(query).is_none_or(|q| q.trim() == s.space_id) => Ok(s),
        Ok(_) => Err(not_found("suggestion")),
        Err(e) => Err(internal(e)),
    }
}

fn not_offered(s: &SuggestionRow) -> Response {
    (StatusCode::CONFLICT, Json(json!({ "error": format!("this suggestion was already {}", s.status), "status": s.status, "routine_id": s.routine_id }))).into_response()
}

/// The routine Accept makes from `s` with the person's overrides `b`, built and checked like any
/// routine, then given the first run recomputed for `now` (design §5) and the discovery origin.
async fn routine_from(state: &AppState, s: &SuggestionRow, b: AcceptBody, now: DateTime<Utc>) -> Result<crate::db::continuity::NewRoutine, Response> {
    let stored_checks: Vec<RoutineCheck> = serde_json::from_str(&s.checks).unwrap_or_default();
    let checks = match &b.keep_checks {
        None => stored_checks,
        Some(keep) => {
            if let Some(i) = keep.iter().find(|i| **i >= stored_checks.len()) {
                return Err(err(StatusCode::UNPROCESSABLE_ENTITY, format!("keep_checks: no check {i} (the suggestion has {})", stored_checks.len())));
            }
            stored_checks.into_iter().enumerate().filter(|(i, _)| keep.contains(i)).map(|(_, c)| c).collect()
        }
    };
    let schedule = match b.schedule {
        Some(v) => Some(serde_json::from_value::<Schedule>(v).map_err(|e| bad(format!("invalid schedule: {e}")))?),
        None => serde_json::from_str::<Schedule>(&s.schedule).ok(),
    };
    let kwant_id = b.kwant_id.map(|k| k.trim().to_string()).unwrap_or_else(|| s.kwant_id.clone());
    match state.db.space_of_kwant(&kwant_id).await {
        Ok(Some(space)) if space.as_str() == s.space_id => {}
        Ok(Some(_)) => return Err(bad("the bot is in another space")),
        Ok(None) => return Err(bad("unknown kwant_id")),
        Err(e) => return Err(internal(e)),
    }
    // Where it delivers: the person's choice, else the suggestion's (if still there). A thread or
    // direct chat belongs to the suggested bot, so another bot gets its own direct chat.
    let (mut channel_id, mut thread_id) = (b.channel_id, b.thread_id);
    if channel_id.is_none() && thread_id.is_none() {
        let offer_place: Delivery = serde_json::from_str(&s.delivery).unwrap_or_default();
        let here = deliverable_row(state, s, offer_place).await;
        match (here.channel_id, here.thread_id) {
            (Some(c), _) => channel_id = Some(Some(c)),
            (None, Some(t)) if kwant_id == s.kwant_id => thread_id = Some(Some(t)),
            _ => channel_id = Some(None),
        }
    }
    let class = (s.task_class != crate::db::flight_school::DEFAULT_CLASS).then(|| s.task_class.clone());
    let acceptance = b.acceptance.unwrap_or_else(|| Some(s.acceptance.clone()));
    let p = RoutinePayload {
        kwant_id: Some(kwant_id),
        channel_id,
        name: Some(b.name.unwrap_or_else(|| s.name.clone())),
        instructions: Some(b.instructions.unwrap_or_else(|| s.instructions.clone())),
        schedule,
        enabled: Some(true),
        thread_id,
        context_message_id: None,
        task_class: Some(class),
        acceptance: Some(acceptance),
        checks: Some(Some(checks)),
    };
    let mut r = scheduler::build(state, p, None).await?;
    let evidence: Value = serde_json::from_str(&s.evidence).unwrap_or_default();
    let last_ask = evidence["last_ask_at"].as_str().and_then(|t| DateTime::parse_from_rfc3339(t).ok()).map_or(s.offered_at, |t| t.with_timezone(&Utc));
    let fin: Schedule = serde_json::from_str(&r.schedule).map_err(internal)?;
    if let Some(at) = miner::first_run_at(&fin, last_ask, now) {
        r.next_run_at = Some(at);
    }
    r.origin = Some("discovery".into());
    r.suggestion_id = Some(s.id.clone());
    Ok(r)
}

/// [`deliverable`] for a stored suggestion.
async fn deliverable_row(state: &AppState, s: &SuggestionRow, place: Delivery) -> Delivery {
    let direct = Delivery { direct_kwant_id: Some(s.kwant_id.clone()), ..Default::default() };
    if let Some(t) = &place.thread_id {
        return match state.db.get_thread(t).await {
            Ok(Some(t)) if t.kwant_id == s.kwant_id && !t.archived => place,
            _ => direct,
        };
    }
    if let Some(c) = &place.channel_id {
        return match state.db.get_channel(c).await {
            Ok(Some(c)) if c.space_id == s.space_id => place,
            _ => direct,
        };
    }
    direct
}

/// Accept at `now`: 201-shaped `{suggestion, routine, first_run_text}` or the error response.
pub async fn accept_at(state: &AppState, query: Option<&str>, id: &str, body: AcceptBody, now: DateTime<Utc>) -> Result<Value, Response> {
    let s = load(state, query, id).await?;
    if s.status != "offered" {
        return Err(not_offered(&s));
    }
    let routine = routine_from(state, &s, body, now).await?;
    match state.db.accept_suggestion(&s.id, &s.space_id, &routine, now).await {
        Ok(Decided::Done(s, Some(r))) => {
            state.scheduler_wake.notify_one();
            let routine = RoutineView::from(*r);
            let tz = serde_json::from_value::<Schedule>(routine.schedule.clone()).ok();
            let first_run_text = routine.next_run_at.map(|at| run_time_text(at, tz.as_ref().and_then(Schedule::tz)));
            Ok(json!({
                "suggestion": suggestion_json(state, &s, now).await,
                "routine": routine,
                "first_run_text": first_run_text,
            }))
        }
        Ok(Decided::NotOffered(s)) => Err(not_offered(&s)),
        Ok(Decided::Done(..) | Decided::Missing) => Err(not_found("suggestion")),
        Err(DbError::OtherSpace(m)) => Err(bad(m)),
        Err(e) => Err(internal(e)),
    }
}

/// `POST /api/automation-discovery/:id/accept` (person only; `Idempotency-Key` required): makes
/// the routine (design §6). 201 `{suggestion, routine, first_run_text}`; the same key again
/// replays it; 409 `{routine_id}` once it isn't a live offer; 400 for an invalid override; 422
/// for `keep_checks` out of range.
pub async fn accept(State(state): State<AppState>, RawQuery(q): RawQuery, headers: HeaderMap, Path(id): Path<String>, body: Bytes) -> Response {
    if headers.get("idempotency-key").is_none() {
        return bad("Idempotency-Key is required");
    }
    // No body: the suggestion as it is. A body that isn't one (an unknown field too) is refused
    let body: AcceptBody = if body.iter().all(u8::is_ascii_whitespace) {
        AcceptBody::default()
    } else {
        match serde_json::from_slice(&body) {
            Ok(b) => b,
            Err(e) => return bad(format!("invalid body: {e}")),
        }
    };
    let route = format!("POST /api/automation-discovery/{id}/accept");
    let mut resp = super::graph::idempotent(&state, &headers, route, || accept_at(&state, q.as_deref(), &id, body, Utc::now())).await;
    if resp.status() == StatusCode::OK {
        *resp.status_mut() = StatusCode::CREATED;
    }
    resp
}

/// `POST /api/automation-discovery/:id/dismiss` (person only): not offered again for 30 days.
/// 200 `{suggestion}`; 409 once it isn't a live offer.
pub async fn dismiss(State(state): State<AppState>, RawQuery(q): RawQuery, Path(id): Path<String>) -> Response {
    let now = Utc::now();
    let s = match load(&state, q.as_deref(), &id).await {
        Ok(s) => s,
        Err(r) => return r,
    };
    match state.db.dismiss_suggestion(&s.id, &s.space_id, now).await {
        Ok(Decided::Done(s, _)) => Json(json!({ "suggestion": suggestion_json(&state, &s, now).await })).into_response(),
        Ok(Decided::NotOffered(s)) => not_offered(&s),
        Ok(Decided::Missing) => not_found("suggestion"),
        Err(e) => internal(e),
    }
}
