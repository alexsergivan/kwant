//! G2 · Bot budget: a person-only monthly budget per bot, in fresh tokens and/or dollars (see
//! `release-090/g2-contract.md`).
//!
//! - **Counting** reuses the daily spending stop's booking: [`super::spend_stop::book_turn`]
//!   books one turn's numbers into the space's day and the bot's day ([`book`]), so there is one
//!   calculation of a turn's fresh tokens and cost, not two.
//! - **The month** is the calendar month in the person's time zone (the daily stop's setting).
//! - **The check** ([`gate`]) runs before a turn starts, right after the daily stop's, in
//!   `routes::turn_once`; the running turn always finishes. At 100% the bot is *paused*: new turns
//!   are held (parked with reason `bot_budget`, released by Continue, a raise or the new month),
//!   routines collapse to one late run, missions and AutoFix skip (see
//!   [`super::spend_stop::is_stop_error`]). Other bots are not affected.
//! - **At 80%** one note in the bot's 1:1, once per budget.
//! - Only the person sets, raises, continues or pauses again: the routes are in
//!   `auth::person_only`, and the budget lives in its own table, never in `settings`.

use super::routes::TurnScope;
use super::spaces::Scope;
use super::AppState;
use crate::db::models::{Kwant, MessageExtra, NewMessage};
use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use chrono::{DateTime, Datelike, NaiveDate, NaiveTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

/// The start of the error a held turn returns, the parked reason, and H3's `reason_code`.
pub const REASON: &str = "bot_budget";
/// The meta kind of the stop notice and of the Needs-you item.
pub const STOPPED_KIND: &str = "bot_budget_stopped";
const WARN_AT: f64 = 80.0;

/// A bot's month against its budget: the body of `GET /api/kwants/:id/budget`.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct Budget {
    pub kwant_id: String,
    pub kwant_name: String,
    /// `YYYY-MM` in `timezone`.
    pub month: String,
    pub timezone: String,
    pub used_tokens: u64,
    pub used_usd: f64,
    pub today_tokens: u64,
    /// 0 when off.
    pub budget_tokens: u64,
    /// 0 when off.
    pub budget_usd: f64,
    /// The higher of the set limits' percentages; 0 when off.
    pub percent: f64,
    /// The limit at 100%: "tokens", "usd" or null.
    pub over: Option<&'static str>,
    /// "off", "under", "warn", "paused" or "continued".
    pub state: &'static str,
    pub paused_since: Option<DateTime<Utc>>,
    pub continued_month: Option<String>,
    /// Local 00:00 on the 1st of next month.
    pub resets_at: DateTime<Utc>,
    /// null before the bot's first booked turn this month; false when no turn reported usage.
    pub usage_reported: Option<bool>,
    pub some_unreported: bool,
    /// null before the first booked turn this month; else whether any turn reported a cost.
    pub cost_reported: Option<bool>,
    /// Some turn booked this month didn't report usage (Codex, or a turn that ended without it):
    /// `used_tokens` holds the daily stop's estimate for those turns, and `used_usd` has no
    /// dollars for them. The budget and the spend are still returned and still park the bot; the
    /// UI shows them as "about …" instead of hiding them.
    pub estimated: bool,
    pub engine: String,
}

/// The month `at` falls in (`YYYY-MM`), the local date, and when the month ends, in `tz`.
pub fn month_at(tz: &str, at: DateTime<Utc>) -> (String, String, NaiveDate, DateTime<Utc>) {
    let (zone, date, _) = super::spend_stop::day_at(tz, at);
    let first_next = if date.month() == 12 {
        NaiveDate::from_ymd_opt(date.year() + 1, 1, 1)
    } else {
        NaiveDate::from_ymd_opt(date.year(), date.month() + 1, 1)
    }
    .unwrap_or(date);
    let utc = first_next.and_time(NaiveTime::MIN).and_utc();
    let resets = if zone == "UTC" {
        utc
    } else {
        crate::routines::local_instant(&zone, first_next, NaiveTime::MIN).ok().flatten().unwrap_or(utc)
    };
    let resets = if resets > at { resets } else { at + chrono::Duration::hours(24) };
    (zone, date.format("%Y-%m").to_string(), date, resets)
}

/// (percent, the limit at 100%) for `used` against the set limits.
fn percent_of(tokens: u64, usd: f64, budget_tokens: u64, budget_usd: f64) -> (f64, Option<&'static str>) {
    let pt = if budget_tokens > 0 { tokens as f64 * 100.0 / budget_tokens as f64 } else { 0.0 };
    let pu = if budget_usd > 0.0 { usd * 100.0 / budget_usd } else { 0.0 };
    let over = if budget_tokens > 0 && pt >= 100.0 {
        Some("tokens")
    } else if budget_usd > 0.0 && pu >= 100.0 {
        Some("usd")
    } else {
        None
    };
    (pt.max(pu), over)
}

fn budget_key(month: &str, tokens: u64, usd: f64) -> String {
    format!("{month}|{tokens}|{usd}")
}

/// The bot's month, at the stop's clock.
pub async fn status(state: &AppState, kwant: &Kwant) -> Budget {
    let now = state.spend.now();
    let (zone, month, date, resets_at) = month_at(&super::spend_stop::timezone(state).await, now);
    let row = state.db.bot_budget(&kwant.id).await.unwrap_or_default();
    let used = state.db.bot_use(&kwant.id, &format!("{month}-")).await.unwrap_or_default();
    let today = state.db.bot_use(&kwant.id, &date.format("%Y-%m-%d").to_string()).await.unwrap_or_default();
    let budget_tokens = u64::try_from(row.budget_tokens).unwrap_or(0);
    let budget_usd = row.budget_usd.max(0.0);
    let used_tokens = u64::try_from(used.tokens).unwrap_or(0);
    let used_usd = used.usd.max(0.0);
    let (percent, over) = percent_of(used_tokens, used_usd, budget_tokens, budget_usd);
    let continued = row.continued_month.as_deref() == Some(month.as_str());
    let state_name = if budget_tokens == 0 && budget_usd <= 0.0 {
        "off"
    } else if continued {
        "continued"
    } else if over.is_some() {
        "paused"
    } else if percent >= WARN_AT {
        "warn"
    } else {
        "under"
    };
    let turns = used.reported_turns + used.unreported_turns;
    Budget {
        kwant_id: kwant.id.clone(),
        kwant_name: kwant.name.clone(),
        month: month.clone(),
        timezone: zone,
        used_tokens,
        used_usd,
        today_tokens: u64::try_from(today.tokens).unwrap_or(0),
        budget_tokens,
        budget_usd,
        percent: (percent * 10.0).round() / 10.0,
        over,
        state: state_name,
        paused_since: (state_name == "paused" && row.paused_month.as_deref() == Some(month.as_str())).then_some(row.paused_at).flatten(),
        continued_month: row.continued_month,
        resets_at,
        usage_reported: (turns > 0).then_some(used.reported_turns > 0),
        some_unreported: used.reported_turns > 0 && used.unreported_turns > 0,
        cost_reported: (turns > 0).then_some(used.cost_turns > 0),
        estimated: used.unreported_turns > 0,
        engine: kwant.preferred_model.clone(),
    }
}

/// Short token counts: 950, 1.5k, 61k, 2.04M. One decimal below 100k, like the app's `compactTokens`,
/// so 1,500 of 1,800 reads "1.5k of 1.8k", not "1k of 1k".
fn short(n: u64) -> String {
    match n {
        0..=999 => n.to_string(),
        1_000..=99_999 => {
            let k = format!("{:.1}", n as f64 / 1_000.0);
            format!("{}k", k.trim_end_matches('0').trim_end_matches('.'))
        }
        100_000..=999_999 => format!("{}k", (n as f64 / 1_000.0).round() as u64),
        _ => {
            let m = format!("{:.2}", n as f64 / 1_000_000.0);
            format!("{}M", m.trim_end_matches('0').trim_end_matches('.'))
        }
    }
}

fn month_name(month: &str) -> String {
    NaiveDate::parse_from_str(&format!("{month}-01"), "%Y-%m-%d").map(|d| d.format("%B").to_string()).unwrap_or_else(|_| month.to_string())
}

/// "2.04M of 2M fresh tokens in November" / "$20.40 of $20.00 in November", for the limit hit
/// (or the one with the higher percentage).
pub fn used_of(b: &Budget) -> String {
    let pt = if b.budget_tokens > 0 { b.used_tokens as f64 / b.budget_tokens as f64 } else { -1.0 };
    let pu = if b.budget_usd > 0.0 { b.used_usd / b.budget_usd } else { -1.0 };
    let usd = b.over == Some("usd") || (b.over.is_none() && pu > pt);
    let about = if b.estimated { "about " } else { "" };
    if usd {
        format!("{about}${:.2} of ${:.2} in {}", b.used_usd, b.budget_usd, month_name(&b.month))
    } else {
        format!("{about}{} of {} fresh tokens in {}", short(b.used_tokens), short(b.budget_tokens), month_name(&b.month))
    }
}

fn next_month_name(b: &Budget) -> String {
    b.resets_at.format("%B").to_string()
}

pub fn stop_text(b: &Budget) -> String {
    format!(
        "{} stopped at its monthly budget. It finished what it was doing and won't start anything new until you raise the limit, continue, or {} starts.",
        b.kwant_name,
        next_month_name(b)
    )
}

fn meta(kind: &str, b: &Budget) -> Value {
    json!({
        "kind": kind,
        "kwant_id": b.kwant_id,
        "kwant_name": b.kwant_name,
        "month": b.month,
        "over": b.over,
        "percent": b.percent,
        "used_tokens": b.used_tokens,
        "budget_tokens": b.budget_tokens,
        "used_usd": b.used_usd,
        "budget_usd": b.budget_usd,
        "resets_at": b.resets_at,
        "paused_since": b.paused_since,
        "estimated": b.estimated,
        "detail": used_of(b),
    })
}

/// The error a held turn returns; starts with [`REASON`].
fn stop_error(b: &Budget) -> String {
    format!(
        "{REASON}: {} is paused at its monthly budget (used {}); it resumes at {} or when the person raises or continues it",
        b.kwant_name,
        used_of(b),
        b.resets_at.to_rfc3339()
    )
}

/// Whether a turn's error is the bot budget's hold.
pub fn is_budget_error(err: &str) -> bool {
    err.trim_start().starts_with(REASON)
}

/// Posts a system line in the bot's 1:1 (outside any turn).
async fn post_direct(state: &AppState, kwant: &Kwant, text: String, meta: Value) {
    let msg = NewMessage {
        channel_id: None,
        direct_kwant_id: Some(kwant.id.clone()),
        sender_role: "system".into(),
        sender_id: "kwnt".into(),
        text,
        reasoning_content: None,
        tool_calls: None,
        tokens_used: 0,
    };
    match state.db.insert_message_with(&msg, &MessageExtra { thread_id: None, meta: Some(meta) }).await {
        Ok(saved) => {
            let _ = state.event_sender.send(crate::drivers::events::RuntimeEvent::MessageAdded { message: serde_json::to_value(&saved).unwrap_or_default() });
        }
        Err(e) => tracing::warn!("could not post the budget note: {e}"),
    }
}

/// Notice key for the stop notice in one chat during one pause episode.
fn chat_key(context: &str, since: Option<DateTime<Utc>>) -> String {
    format!("chat:{context}:{}", since.map(|t| t.timestamp()).unwrap_or_default())
}

/// The pause episode started: the stop notice in the bot's 1:1 and one notification.
async fn announce_pause(state: &AppState, kwant: &Kwant, b: &Budget) {
    tracing::info!(kwant = %kwant.id, month = %b.month, tokens = b.used_tokens, usd = b.used_usd, "bot paused at its monthly budget");
    let _ = state.db.first_bot_budget_notice(&b.month, &chat_key(&kwant.id, b.paused_since)).await;
    post_direct(state, kwant, stop_text(b), meta(STOPPED_KIND, b)).await;
    let n = super::continuity::Notification {
        kind: super::spend_stop::REASON,
        kwant_id: kwant.id.clone(),
        kwant_name: kwant.name.clone(),
        channel_id: None,
        // The stop notice is in the bot's direct chat, so a tap opens that (0.8.1 F6 fields)
        thread_id: None,
        card_id: None,
        title: format!("{} stopped at its monthly budget", kwant.name),
        body: format!("Used {}. Open to raise the budget or continue this month.", used_of(b)),
    };
    let space = state.db.space_of_kwant(&kwant.id).await.ok().flatten();
    super::continuity::notify_held(state, n, space.as_ref(), false).await;
}

/// Starts the pause episode if it hasn't started (once); returns the status with `paused_since`.
async fn ensure_paused(state: &AppState, kwant: &Kwant, mut b: Budget) -> Budget {
    let now = state.spend.now();
    if state.db.start_bot_pause(&kwant.id, &b.month, now).await.unwrap_or(false) {
        b.paused_since = Some(now);
        let _ = state.db.mark_bot_warned(&kwant.id, &budget_key(&b.month, b.budget_tokens, b.budget_usd)).await;
        announce_pause(state, kwant, &b).await;
    } else if b.paused_since.is_none() {
        b.paused_since = state.db.bot_budget(&kwant.id).await.ok().and_then(|r| r.paused_at);
    }
    b
}

/// Books one finished turn for `kwant_id` (called by `spend_stop::book_turn` with the same
/// numbers it books into the day) and moves the budget's state: the 80% note when this turn
/// crossed 80%, the pause when it reached 100% (the turn itself already finished).
pub async fn book(state: &AppState, kwant_id: &str, fresh_tokens: u64, usd: f64, reported: bool) {
    if fresh_tokens == 0 && usd <= 0.0 {
        return;
    }
    let now = state.spend.now();
    let (_, month, date, _) = month_at(&super::spend_stop::timezone(state).await, now);
    let day = date.format("%Y-%m-%d").to_string();
    if let Err(e) = state.db.add_bot_spend(&day, kwant_id, fresh_tokens, usd, reported).await {
        tracing::warn!("could not book the bot's spending: {e}");
        return;
    }
    if state.db.first_bot_budget_notice(&month, &format!("prune:{day}")).await.unwrap_or(false) {
        let _ = state.db.prune_bot_spend(date).await;
    }
    let Ok(Some(kwant)) = state.db.get_kwant(kwant_id).await else {
        return;
    };
    let after = status(state, &kwant).await;
    if after.state == "off" || after.state == "continued" {
        return;
    }
    let before_tokens = after.used_tokens.saturating_sub(fresh_tokens);
    let before_usd = (after.used_usd - usd).max(0.0);
    let (before, _) = percent_of(before_tokens, before_usd, after.budget_tokens, after.budget_usd);
    if after.state == "paused" {
        ensure_paused(state, &kwant, after).await;
    } else if after.state == "warn" && before < WARN_AT {
        let key = budget_key(&after.month, after.budget_tokens, after.budget_usd);
        if state.db.mark_bot_warned(&kwant.id, &key).await.unwrap_or(false) {
            let text = format!(
                "{} has used 80% of its {} budget. At 100% it finishes what it's doing and starts nothing new.",
                kwant.name,
                month_name(&after.month)
            );
            post_direct(state, &kwant, text, meta("bot_budget_warn", &after)).await;
        }
    }
}

/// Before a turn starts (`routes::turn_once`, after the daily stop's gate): `Err` when the bot is
/// paused at its budget. A turn that can wait (`parkable`: saved, not a retry, a check or a
/// discussion round) is parked until the bot can start new work again; unless `quiet`, each chat
/// gets the stop notice once per pause.
pub(super) async fn gate(
    scope: &TurnScope,
    speaker: &Kwant,
    prompt: &str,
    quiet: bool,
    parkable: bool,
    delegation_id: Option<String>,
) -> Result<(), String> {
    let b = status(&scope.state, speaker).await;
    if b.state != "paused" {
        return Ok(());
    }
    let b = ensure_paused(&scope.state, speaker, b).await;
    let context = scope.context_id(speaker);
    tracing::info!(context = %context, kwant = %speaker.id, month = %b.month, "bot budget: turn held");
    let held = if parkable && !quiet {
        super::parked::park_for(scope, speaker, prompt, false, delegation_id, Some(b.resets_at), "bot_budget").await
    } else {
        None
    };
    if !quiet && scope.state.db.first_bot_budget_notice(&b.month, &chat_key(&context, b.paused_since)).await.unwrap_or(false) {
        let mut m = meta(STOPPED_KIND, &b);
        m["held"] = json!(held.is_some());
        if let Some(saved) = scope.save(&scope.message("system", "kwnt", stop_text(&b)), Some(m)).await {
            let _ = scope.state.event_sender.send(crate::drivers::events::RuntimeEvent::MessageAdded { message: serde_json::to_value(&saved).unwrap_or_default() });
        }
    }
    Err(stop_error(&b))
}

/// For routines before they start: `Some(reason)` while `kwant_id` is paused at its budget.
pub async fn held(state: &AppState, kwant_id: &str) -> Option<String> {
    let kwant = state.db.get_kwant(kwant_id).await.ok().flatten()?;
    let b = status(state, &kwant).await;
    if b.state != "paused" {
        return None;
    }
    let b = ensure_paused(state, &kwant, b).await;
    Some(stop_error(&b))
}

/// H4: when the bot was paused at its budget (`None` when it isn't).
pub async fn paused_since(state: &AppState, kwant_id: &str) -> Option<DateTime<Utc>> {
    let kwant = state.db.get_kwant(kwant_id).await.ok().flatten()?;
    let b = status(state, &kwant).await;
    (b.state == "paused").then_some(b.paused_since).flatten()
}

/// H3: the reason code new work for the bot waits on (`Some("bot_budget")` while paused).
pub async fn hold_reason(state: &AppState, kwant_id: &str) -> Option<&'static str> {
    let kwant = state.db.get_kwant(kwant_id).await.ok().flatten()?;
    (status(state, &kwant).await.state == "paused").then_some(REASON)
}

/// The bot can start new work again (raised, continued, budget off, or a new month): ends the
/// pause, posts `card` in its 1:1, and starts what it held: parked turns oldest first, and each
/// held routine once, marked as late.
pub async fn release(state: &AppState, kwant: &Kwant, card: Option<(String, &str)>) {
    let b = status(state, kwant).await;
    if b.state == "paused" {
        return;
    }
    let was_paused = state.db.end_bot_pause(&kwant.id).await.unwrap_or(false);
    if let (true, Some((text, kind))) = (was_paused, card) {
        post_direct(state, kwant, text, meta(kind, &b)).await;
    }
    let parks = state.db.release_budget_parks(&kwant.id, Utc::now()).await.unwrap_or(0);
    if parks > 0 {
        state.scheduler_wake.notify_one();
    }
    let routines = state.db.take_budget_routines(&kwant.id).await.unwrap_or_default();
    let note = format!("Ran late: {} was paused at its monthly budget.", kwant.name);
    for id in &routines {
        if let Ok(Some(r)) = state.db.get_routine(id).await {
            if r.enabled {
                super::scheduler::start_run_noted(state, r, "schedule", Some(&note)).await;
            }
        }
    }
    if was_paused || parks > 0 || !routines.is_empty() {
        tracing::info!(kwant = %kwant.id, parks, routines = routines.len(), "bot budget released");
    }
}

/// Scheduler tick: releases bots whose pause ended on its own (the new month).
pub async fn release_due(state: &AppState) {
    let Ok(rows) = state.db.paused_or_holding_bots().await else {
        return;
    };
    for id in rows {
        if let Ok(Some(k)) = state.db.get_kwant(&id).await {
            let card = format!("{} can start new work again.", k.name);
            release(state, &k, Some((card, "bot_budget_resumed"))).await;
        }
    }
}

/// Needs you: one item per bot paused at its budget, in `space`.
pub async fn needs_you_items(state: &AppState, kwants: &[Kwant]) -> Vec<Value> {
    let mut out = Vec::new();
    for k in kwants {
        let b = status(state, k).await;
        if b.state != "paused" {
            continue;
        }
        let mut item = meta(STOPPED_KIND, &b);
        item["id"] = json!(format!("{STOPPED_KIND}:{}:{}", k.id, b.month));
        item["text"] = json!(stop_text(&b));
        item["continue_url"] = json!(format!("/api/kwants/{}/budget/continue", k.id));
        item["raise_url"] = json!(format!("/api/kwants/{}/budget", k.id));
        out.push(item);
    }
    out
}

fn err(code: StatusCode, msg: impl Into<String>) -> Response {
    (code, Json(json!({ "error": msg.into() }))).into_response()
}

#[allow(clippy::result_large_err)]
async fn kwant_or_404(state: &AppState, id: &str) -> Result<Kwant, Response> {
    match state.db.get_kwant(id).await {
        Ok(Some(k)) => Ok(k),
        Ok(None) => Err(err(StatusCode::NOT_FOUND, "no such bot")),
        Err(_) => Err(err(StatusCode::INTERNAL_SERVER_ERROR, "could not load the bot")),
    }
}

/// `GET /api/kwants/:id/budget`.
pub async fn get_budget(State(state): State<AppState>, Path(id): Path<String>) -> Response {
    match kwant_or_404(&state, &id).await {
        Ok(k) => Json(status(&state, &k).await).into_response(),
        Err(r) => r,
    }
}

#[derive(Debug, Deserialize)]
pub struct BudgetBody {
    /// null or 0 = off; left out = unchanged.
    #[serde(default, deserialize_with = "some_value")]
    pub budget_tokens: Option<Option<f64>>,
    #[serde(default, deserialize_with = "some_value")]
    pub budget_usd: Option<Option<f64>>,
}

/// Tells a field set to null (`Some(None)`) from one left out (`None`).
fn some_value<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Option<Option<f64>>, D::Error> {
    Option::<f64>::deserialize(d).map(Some)
}

/// The body as a [`BudgetBody`], only when it is a JSON object: serde would read an array like
/// `[1]` as the struct's fields in order and save a 1-token budget (#495 S1).
pub fn parse_body(v: Value) -> Option<BudgetBody> {
    if !v.is_object() {
        return None;
    }
    serde_json::from_value(v).ok()
}

/// The limits a body asks for, validated: (tokens, usd).
pub fn validate(body: &BudgetBody, current: (u64, f64)) -> Result<(u64, f64), String> {
    let tokens = match body.budget_tokens {
        None => current.0,
        Some(None) => 0,
        Some(Some(v)) if !v.is_finite() || v < 0.0 || v.fract() != 0.0 => return Err("budget_tokens must be a whole number of tokens, or null".into()),
        Some(Some(v)) if v > super::spend_stop::MAX_TOKENS as f64 => return Err("At most 10 billion tokens a month.".into()),
        Some(Some(v)) => v as u64,
    };
    let usd = match body.budget_usd {
        None => current.1,
        Some(None) => 0.0,
        Some(Some(v)) if !v.is_finite() || v < 0.0 => return Err("budget_usd must be dollars, or null".into()),
        Some(Some(v)) if v > super::spend_stop::MAX_USD => return Err("At most $100,000 a month.".into()),
        Some(Some(v)) => (v * 100.0).round() / 100.0,
    };
    Ok((tokens, usd))
}

/// `POST /api/kwants/:id/budget` (person only): sets the limits.
pub async fn set_budget(
    State(state): State<AppState>,
    Path(id): Path<String>,
    body: Result<Json<Value>, axum::extract::rejection::JsonRejection>,
) -> Response {
    let Some(body) = body.ok().and_then(|Json(v)| parse_body(v)) else {
        return err(StatusCode::BAD_REQUEST, "expected {budget_tokens, budget_usd}");
    };
    let kwant = match kwant_or_404(&state, &id).await {
        Ok(k) => k,
        Err(r) => return r,
    };
    let before = status(&state, &kwant).await;
    let (tokens, usd) = match validate(&body, (before.budget_tokens, before.budget_usd)) {
        Ok(v) => v,
        Err(e) => return err(StatusCode::BAD_REQUEST, e),
    };
    if let Err(e) = state.db.set_bot_budget(&kwant.id, i64::try_from(tokens).unwrap_or(i64::MAX), usd).await {
        tracing::warn!("could not save the bot budget: {e}");
        return err(StatusCode::INTERNAL_SERVER_ERROR, "Couldn't save the budget. Try again.");
    }
    let after = status(&state, &kwant).await;
    // A save that leaves use already at or past 80% doesn't send the 80% note for the new budget
    if after.percent >= WARN_AT {
        let _ = state.db.mark_bot_warned(&kwant.id, &budget_key(&after.month, after.budget_tokens, after.budget_usd)).await;
    }
    tracing::info!(kwant = %kwant.id, tokens, usd, state = after.state, "bot budget saved");
    let after = if after.state == "paused" {
        ensure_paused(&state, &kwant, after).await
    } else {
        release(&state, &kwant, Some((format!("{} can start new work again.", kwant.name), "bot_budget_resumed"))).await;
        status(&state, &kwant).await
    };
    Json(after).into_response()
}

/// `POST /api/kwants/:id/budget/continue` (person only): ignores the budget until the month ends.
pub async fn continue_month(State(state): State<AppState>, Path(id): Path<String>) -> Response {
    let kwant = match kwant_or_404(&state, &id).await {
        Ok(k) => k,
        Err(r) => return r,
    };
    let b = status(&state, &kwant).await;
    if b.state == "off" {
        return err(StatusCode::CONFLICT, "this bot has no budget");
    }
    if let Err(e) = state.db.set_bot_budget_continued(&kwant.id, Some(&b.month)).await {
        tracing::warn!("could not continue the bot budget: {e}");
        return err(StatusCode::INTERNAL_SERVER_ERROR, "Couldn't continue. Try again.");
    }
    let text = format!(
        "Continued for {}. {} can start new work again; the budget applies again on {}.",
        month_name(&b.month),
        kwant.name,
        b.resets_at.format("%B %-d")
    );
    release(&state, &kwant, Some((text, "bot_budget_continued"))).await;
    tracing::info!(kwant = %kwant.id, month = %b.month, "bot budget continued for the month");
    Json(status(&state, &kwant).await).into_response()
}

/// `DELETE /api/kwants/:id/budget/continue` (person only): pause at the budget again.
pub async fn pause_again(State(state): State<AppState>, Path(id): Path<String>) -> Response {
    let kwant = match kwant_or_404(&state, &id).await {
        Ok(k) => k,
        Err(r) => return r,
    };
    if let Err(e) = state.db.set_bot_budget_continued(&kwant.id, None).await {
        tracing::warn!("could not undo continue: {e}");
        return err(StatusCode::INTERNAL_SERVER_ERROR, "could not save");
    }
    let b = status(&state, &kwant).await;
    let b = if b.state == "paused" { ensure_paused(&state, &kwant, b).await } else { b };
    Json(b).into_response()
}

/// `GET /api/usage/budgets`: every bot in the space, paused first, then by percent, then name.
pub async fn list_budgets(State(state): State<AppState>, Scope(space): Scope) -> Response {
    let Ok(kwants) = state.db.list_kwants(&space).await else {
        return err(StatusCode::INTERNAL_SERVER_ERROR, "could not load the bots");
    };
    let mut bots = Vec::with_capacity(kwants.len());
    for k in &kwants {
        bots.push(status(&state, k).await);
    }
    sort_for_overview(&mut bots);
    let now = state.spend.now();
    let (zone, month, _, resets_at) = month_at(&super::spend_stop::timezone(&state).await, now);
    Json(json!({ "month": month, "timezone": zone, "resets_at": resets_at, "bots": bots })).into_response()
}

/// H5 order: paused first, then percent high to low (bots with a budget), then the rest by name.
pub fn sort_for_overview(bots: &mut [Budget]) {
    bots.sort_by(|a, b| {
        let rank = |x: &Budget| match x.state {
            "paused" => 0,
            "off" => 2,
            _ => 1,
        };
        rank(a)
            .cmp(&rank(b))
            .then(b.percent.partial_cmp(&a.percent).unwrap_or(std::cmp::Ordering::Equal))
            .then_with(|| a.kwant_name.to_lowercase().cmp(&b.kwant_name.to_lowercase()))
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(s: &str) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339(s).unwrap().with_timezone(&Utc)
    }

    #[test]
    fn month_follows_the_zone() {
        let (zone, month, _, resets) = month_at("Europe/Ljubljana", at("2026-11-30T23:30:00Z"));
        assert_eq!((zone.as_str(), month.as_str()), ("Europe/Ljubljana", "2026-12"));
        assert_eq!(resets, at("2026-12-31T23:00:00Z"));
        let (zone, month, _, resets) = month_at("", at("2026-12-31T23:59:00Z"));
        assert_eq!((zone.as_str(), month.as_str(), resets), ("UTC", "2026-12", at("2027-01-01T00:00:00Z")));
    }

    #[test]
    fn percent_and_over() {
        assert_eq!(percent_of(1_600, 0.0, 2_000, 0.0), (80.0, None));
        assert_eq!(percent_of(2_040, 0.0, 2_000, 0.0).1, Some("tokens"));
        assert_eq!(percent_of(10, 20.0, 2_000, 20.0), (100.0, Some("usd")));
        assert_eq!(percent_of(99, 99.0, 0, 0.0), (0.0, None));
    }

    #[test]
    fn only_an_object_is_a_budget_body() {
        for bad in [json!([1]), json!([1000, 2.5]), json!(1), json!("x"), json!(null), json!(true)] {
            assert!(parse_body(bad.clone()).is_none(), "{bad}");
        }
        let b = parse_body(json!({"budget_tokens": 5})).unwrap();
        assert_eq!((b.budget_tokens, b.budget_usd), (Some(Some(5.0)), None));
        assert!(parse_body(json!({})).is_some(), "an empty object changes nothing, as before");
        assert!(parse_body(json!({"budget_tokens": "lots"})).is_none());
    }

    #[test]
    fn validates_limits() {
        let body = |t: Option<Option<f64>>, u: Option<Option<f64>>| BudgetBody { budget_tokens: t, budget_usd: u };
        assert_eq!(validate(&body(Some(Some(2_000_000.0)), None), (0, 5.0)), Ok((2_000_000, 5.0)));
        assert_eq!(validate(&body(Some(None), Some(Some(2.505))), (9, 5.0)), Ok((0, 2.51)));
        assert!(validate(&body(Some(Some(-1.0)), None), (0, 0.0)).is_err());
        assert!(validate(&body(Some(Some(1.5)), None), (0, 0.0)).is_err());
        assert!(validate(&body(Some(Some(10_000_000_001.0)), None), (0, 0.0)).is_err());
        assert!(validate(&body(None, Some(Some(100_000.01))), (0, 0.0)).is_err());
        assert!(validate(&body(None, Some(Some(f64::NAN))), (0, 0.0)).is_err());
    }

    #[test]
    fn short_counts() {
        assert_eq!(short(2_040_000), "2.04M");
        assert_eq!(short(2_000_000), "2M");
        assert_eq!(short(61_000), "61k");
    }

    #[test]
    fn short_counts_keep_one_decimal_below_100k() {
        assert_eq!(short(1_500), "1.5k");
        assert_eq!(short(1_800), "1.8k");
        assert_eq!(short(61_000), "61k");
    }

    #[test]
    fn budget_errors_are_stop_errors() {
        assert!(is_budget_error("bot_budget: Pixel is paused"));
        assert!(super::super::spend_stop::is_stop_error("bot_budget: Pixel is paused"));
        assert!(!is_budget_error("spend_stopped: today's"));
    }
}
