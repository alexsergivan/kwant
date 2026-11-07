//! The daily spending stop: two person-only settings cap what all turns together may use in one
//! day, in fresh tokens ([`crate::drivers::TurnUsage::fresh`], every engine) and in dollars (only
//! engines that report `cost_usd`). 0 means off, and both are off by default.
//!
//! The day follows the person's `timezone` setting (UTC when none or an unknown one is set), and
//! its totals are kept in the database, so a restart keeps them (see [`crate::db::spend`]).
//!
//! The check sits in one place, before a turn takes its slot ([`gate`], called from
//! `routes::turn_once`), so every turn hits it whatever starts it: a person message, a hand-off,
//! a routine, a goal draft, a mission task, AutoFix or a connected agent. Turns already running
//! finish. Over the limit:
//! - in a chat, the turn doesn't start and one notice per chat per day says so, with Continue;
//! - routines, missions and AutoFix skip with reason [`REASON`] in their own history before they
//!   start a turn, and [`needs_you_item`] puts one item in Needs you for the day. These skips are
//!   not failures: no AutoFix retry;
//! - goal checks keep running: they never ask a bot, and what a goal starts afterwards is a turn;
//! - the first thing held each day sends one `spend_stopped` notification (see [`tripped`]).
//!
//! The person lifts the stop until the day ends with `POST /api/usage/today/continue`; changing
//! either limit clears that, so the new limit applies at once (raising it past today's use
//! lifts the stop on its own).

use super::routes::TurnScope;
use super::AppState;
use crate::db::models::Kwant;
use axum::{extract::State, http::StatusCode, response::IntoResponse, Json};
use chrono::{DateTime, Duration, NaiveDate, NaiveTime, Utc};
use serde::Serialize;
use serde_json::{json, Value};

/// Setting key: fresh tokens all turns together may use in a day; 0 is off.
pub const SETTING_TOKENS: &str = "daily_spend_stop_tokens";
/// Setting key: dollars (engine-reported `cost_usd`) all turns together may use in a day; 0 is off.
pub const SETTING_USD: &str = "daily_spend_stop_usd";
pub const MAX_TOKENS: u64 = 10_000_000_000;
pub const MAX_USD: f64 = 100_000.0;
/// The skip reason routines, missions and AutoFix record, the notice's meta and notification kind, and the
/// start of the error a stopped turn returns.
pub const REASON: &str = "spend_stopped";

/// "Now" for the spending stop: the real clock, or a fixed instant a test sets.
#[derive(Default)]
pub struct Clock {
    fixed: std::sync::Mutex<Option<DateTime<Utc>>>,
}

impl Clock {
    pub fn now(&self) -> DateTime<Utc> {
        self.fixed.lock().ok().and_then(|f| *f).unwrap_or_else(Utc::now)
    }

    /// Fixes the clock at `at` (`None` goes back to the real one). For tests.
    pub fn set(&self, at: Option<DateTime<Utc>>) {
        if let Ok(mut f) = self.fixed.lock() {
            *f = at;
        }
    }
}

/// Today's spending against the limits; the body of `GET /api/usage/today`.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct Status {
    /// The local date (`YYYY-MM-DD`) in `timezone`.
    pub day: String,
    pub timezone: String,
    pub today_tokens: u64,
    pub today_usd: f64,
    /// 0 when off.
    pub stop_tokens: u64,
    /// 0 when off.
    pub stop_usd: f64,
    /// The person pressed Continue today.
    pub continued: bool,
    /// New turns are held.
    pub stopped: bool,
    /// Which limit today's spending reached: "tokens", "usd", or null.
    pub over: Option<&'static str>,
    /// When the day ends and the totals start again from 0 (the next local midnight).
    pub day_ends_at: DateTime<Utc>,
}

/// The two limits: (fresh tokens, dollars); 0 is off.
pub async fn limits(state: &AppState) -> (u64, f64) {
    let get = |key| async move { state.db.get_setting(key).await.ok().flatten().unwrap_or_default() };
    let tokens = get(SETTING_TOKENS).await.trim().parse::<u64>().unwrap_or(0).min(MAX_TOKENS);
    let usd = get(SETTING_USD).await.trim().parse::<f64>().ok().filter(|v| v.is_finite() && *v > 0.0).unwrap_or(0.0).min(MAX_USD);
    (tokens, usd)
}

/// The day `at` falls in and when it ends, in `tz` (UTC when `tz` is empty or unknown).
pub fn day_at(tz: &str, at: DateTime<Utc>) -> (String, NaiveDate, DateTime<Utc>) {
    let tz = tz.trim();
    let local = (!tz.is_empty()).then(|| crate::routines::local_date(tz, at).ok().flatten()).flatten();
    let (zone, date) = match local {
        Some(d) => (tz, d),
        None => ("UTC", at.date_naive()),
    };
    let next = date.succ_opt().unwrap_or(date);
    let utc_midnight = next.and_time(NaiveTime::MIN).and_utc();
    let ends = if zone == "UTC" {
        utc_midnight
    } else {
        crate::routines::local_instant(zone, next, NaiveTime::MIN).ok().flatten().unwrap_or(utc_midnight)
    };
    // A zone whose midnight is odd (a DST gap) still ends the day after `at`
    let ends = if ends > at { ends } else { at + Duration::hours(24) };
    (zone.to_string(), date, ends)
}

async fn timezone(state: &AppState) -> String {
    state.db.get_setting(super::bot_actions::TIMEZONE_SETTING).await.ok().flatten().unwrap_or_default()
}

/// Today's spending and whether new turns are held.
pub async fn status(state: &AppState) -> Status {
    let now = state.spend.now();
    let (zone, date, day_ends_at) = day_at(&timezone(state).await, now);
    let day = date.format("%Y-%m-%d").to_string();
    let row = state.db.spend_day(&day).await.unwrap_or_default();
    let (stop_tokens, stop_usd) = limits(state).await;
    let today_tokens = u64::try_from(row.tokens).unwrap_or(0);
    let today_usd = row.usd.max(0.0);
    let over = if stop_tokens > 0 && today_tokens >= stop_tokens {
        Some("tokens")
    } else if stop_usd > 0.0 && today_usd >= stop_usd {
        Some("usd")
    } else {
        None
    };
    let continued = row.continued_at.is_some();
    Status { day, timezone: zone, today_tokens, today_usd, stop_tokens, stop_usd, continued, stopped: over.is_some() && !continued, over, day_ends_at }
}

/// Books a finished turn: its fresh tokens and, when the engine reported one, its cost.
/// Kept even with the stop off, so the totals are right the moment the person turns it on.
pub async fn book(state: &AppState, fresh_tokens: u64, cost_usd: Option<f64>) {
    let now = state.spend.now();
    let (_, date, _) = day_at(&timezone(state).await, now);
    let usd = cost_usd.filter(|c| c.is_finite() && *c > 0.0).unwrap_or(0.0);
    if fresh_tokens == 0 && usd == 0.0 {
        return;
    }
    if let Err(e) = state.db.add_spend(&date.format("%Y-%m-%d").to_string(), fresh_tokens, usd).await {
        tracing::warn!("could not book today's spending: {e}");
    }
}

/// Whether a turn's error is the spending stop's (a skip, not a failure).
pub fn is_stop_error(err: &str) -> bool {
    err.trim_start().starts_with(REASON)
}

/// Short token counts: 950, 12k, 1.2M.
fn short(n: u64) -> String {
    match n {
        0..=999 => n.to_string(),
        1_000..=999_999 => format!("{}k", n / 1_000),
        _ => format!("{:.1}M", n as f64 / 1_000_000.0),
    }
}

/// "{used} of {limit}" for the limit today reached.
fn used_of(s: &Status) -> String {
    if s.over == Some("usd") {
        format!("${:.2} of ${:.2}", s.today_usd, s.stop_usd)
    } else {
        format!("{} of {} tokens", short(s.today_tokens), short(s.stop_tokens))
    }
}

/// The text of the chat notice.
pub fn notice_text(s: &Status) -> String {
    format!("Today's spending stop: used {}. Continue today, or change it in Settings › All spaces › Usage.", used_of(s))
}

/// The error a held turn returns; starts with [`REASON`].
fn stop_error(s: &Status) -> String {
    format!("{REASON}: today's spending stop is reached (used {}); it lifts at {} or when the person continues today", used_of(s), s.day_ends_at.to_rfc3339())
}

/// The status, or the error a turn starting now gets.
pub async fn check(state: &AppState) -> Result<(), String> {
    let s = status(state).await;
    if !s.stopped {
        return Ok(());
    }
    tripped(state, &s).await;
    Err(stop_error(&s))
}

/// Key in `spend_stop_notices` for the day's one notification.
const NOTIFY_KEY: &str = "notify";

/// The stop held something: the first time each day, one `spend_stopped` notification (UI,
/// push, webhook; off when the person unticks it), so routines going quiet overnight are heard.
/// It names no bot (the stop holds them all); a tap opens Needs you, where Continue is.
async fn tripped(state: &AppState, s: &Status) {
    if !state.db.first_spend_notice(&s.day, NOTIFY_KEY).await.unwrap_or(false) {
        return;
    }
    let n = super::continuity::Notification {
        kind: REASON,
        kwant_id: String::new(),
        kwant_name: "kwant".into(),
        channel_id: None,
        title: "Today's spending limit is reached".into(),
        body: format!("Used {}, so bots won't start new work until tomorrow. Open to continue today.", used_of(s)),
    };
    super::continuity::notify(state, n).await;
}

/// Before a turn takes its slot (`routes::turn_once`): `Err` when today's spending stop holds new
/// turns. Unless the turn is `quiet` (its caller records its own skip), the chat gets one notice
/// a day, with Continue.
pub(super) async fn gate(scope: &TurnScope, speaker: &Kwant, quiet: bool) -> Result<(), String> {
    let s = status(&scope.state).await;
    if !s.stopped {
        return Ok(());
    }
    let context = scope.context_id(speaker);
    tracing::info!(context = %context, day = %s.day, tokens = s.today_tokens, usd = s.today_usd, "daily spending stop: turn not started");
    tripped(&scope.state, &s).await;
    if !quiet && scope.state.db.first_spend_notice(&s.day, &format!("chat:{context}")).await.unwrap_or(false) {
        let meta = json!({
            "kind": REASON,
            "day": s.day,
            "over": s.over,
            "used_tokens": s.today_tokens,
            "stop_tokens": s.stop_tokens,
            "used_usd": s.today_usd,
            "stop_usd": s.stop_usd,
            "day_ends_at": s.day_ends_at,
        });
        if let Some(saved) = scope.save(&scope.message("system", "kwnt", notice_text(&s)), Some(meta)).await {
            let _ = scope.state.event_sender.send(crate::drivers::events::RuntimeEvent::MessageAdded { message: serde_json::to_value(&saved).unwrap_or_default() });
        }
    }
    Err(stop_error(&s))
}

/// For routines, missions and AutoFix before they start work: `Some(reason)` when the
/// stop holds it (the reason starts with [`REASON`]).
pub async fn held(state: &AppState) -> Option<String> {
    check(state).await.err()
}

/// True the first time `key` is skipped today, so a skip is written to its history once a day
/// (a routine due every 5 minutes doesn't fill its history).
pub async fn first_skip_today(state: &AppState, key: &str) -> bool {
    let s = status(state).await;
    state.db.first_spend_notice(&s.day, key).await.unwrap_or(true)
}

/// The one Needs-you item while the stop holds turns (for `GET /api/waiting`).
pub async fn needs_you_item(state: &AppState) -> Option<Value> {
    let s = status(state).await;
    s.stopped.then(|| {
        json!({
            "kind": REASON,
            "id": format!("{REASON}:{}", s.day),
            "text": notice_text(&s),
            "day": s.day,
            "over": s.over,
            "used_tokens": s.today_tokens,
            "stop_tokens": s.stop_tokens,
            "used_usd": s.today_usd,
            "stop_usd": s.stop_usd,
            "day_ends_at": s.day_ends_at,
            "continue_url": "/api/usage/today/continue",
        })
    })
}

/// A limit changed: the person's Continue for today no longer applies.
pub async fn limits_changed(state: &AppState) {
    let s = status(state).await;
    if s.continued {
        let _ = state.db.set_spend_continued(&s.day, None).await;
    }
}

/// `GET /api/usage/today`.
pub async fn get_today(State(state): State<AppState>) -> impl IntoResponse {
    Json(status(&state).await)
}

/// `POST /api/usage/today/continue` (person only): lifts the stop until the day ends.
pub async fn continue_today(State(state): State<AppState>) -> impl IntoResponse {
    let s = status(&state).await;
    if let Err(e) = state.db.set_spend_continued(&s.day, Some(state.spend.now())).await {
        tracing::warn!("could not continue today: {e}");
        return (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({"error": "could not save"}))).into_response();
    }
    tracing::info!(day = %s.day, "daily spending stop lifted until the day ends");
    Json(status(&state).await).into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(s: &str) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339(s).unwrap().with_timezone(&Utc)
    }

    #[test]
    fn day_follows_the_zone() {
        let (zone, date, ends) = day_at("", at("2026-11-06T23:30:00Z"));
        assert_eq!((zone.as_str(), date.to_string(), ends), ("UTC", "2026-11-06".into(), at("2026-11-07T00:00:00Z")));
        let (zone, date, ends) = day_at("not/a_zone", at("2026-11-06T23:30:00Z"));
        assert_eq!((zone.as_str(), date.to_string(), ends), ("UTC", "2026-11-06".into(), at("2026-11-07T00:00:00Z")));
        // Kyiv is UTC+2 in November: 23:30 UTC is already the 7th there
        // (skipped where the host has no zone database)
        let (zone, date, ends) = day_at("Europe/Kyiv", at("2026-11-06T23:30:00Z"));
        if zone == "Europe/Kyiv" {
            assert_eq!((date.to_string(), ends), ("2026-11-07".into(), at("2026-11-07T22:00:00Z")));
        }
    }

    #[test]
    fn stop_errors_are_recognised() {
        let s = Status {
            day: "2026-11-06".into(),
            timezone: "UTC".into(),
            today_tokens: 17_420,
            today_usd: 0.84,
            stop_tokens: 10_000,
            stop_usd: 0.0,
            continued: false,
            stopped: true,
            over: Some("tokens"),
            day_ends_at: at("2026-11-07T00:00:00Z"),
        };
        assert!(is_stop_error(&stop_error(&s)));
        assert!(!is_stop_error("the driver failed"));
        assert_eq!(notice_text(&s), "Today's spending stop: used 17k of 10k tokens. Continue today, or change it in Settings › All spaces › Usage.");
        let usd = Status { over: Some("usd"), stop_usd: 0.5, ..s };
        assert!(notice_text(&usd).contains("used $0.84 of $0.50"));
    }

    #[test]
    fn clock_seam() {
        let c = Clock::default();
        c.set(Some(at("2030-01-01T00:00:00Z")));
        assert_eq!(c.now(), at("2030-01-01T00:00:00Z"));
        c.set(None);
        assert!(c.now() < at("2030-01-01T00:00:00Z"));
    }
}
