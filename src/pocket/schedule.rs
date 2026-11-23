//! pa7 For Friday: a pocket made "for Friday 8 pm" runs on its own. Its spec's `when` (RFC 3339
//! with an offset) gives its times: the lobby opens 30 minutes before and the pocket closes 4
//! hours after, unless the bot or the person set `opens_at`/`closes_at` themselves. The evening
//! before, at 19:00 in the person's time zone, the person gets a reminder and the pocket's card
//! is posted back into the chat it came from; when the pocket ends (the host's End, the kit's
//! final state, or its closing time), its results are posted into that chat once.
//!
//! Stored next to the pocket (`pocket_schedule`) the way letter reminders are stored next to the
//! case (`crate::documents::reminders`), not as a routine per pocket: the person never sees or
//! edits these as routines, they go when the pocket goes, and each fires at most once (claimed
//! in the database before it is sent, so End + the sweep + a restart post once). The loop is the
//! pocket supervisor's minute tick ([`super::listener`]), so it runs only while `pocket_apps` is
//! on. No notification names a guest or quotes an answer: titles and times only.

use super::{PResult, PocketError, Snapshot};
use crate::db::models::{MessageExtra, NewMessage};
use crate::db::pocket::{ScheduleRow, ENDED, OPEN};
use crate::drivers::events::RuntimeEvent;
use crate::server::continuity::{self, Notification};
use crate::server::AppState;
use axum::http::StatusCode;
use chrono::{DateTime, Duration, FixedOffset, NaiveDate, NaiveDateTime, NaiveTime, Utc};
use serde_json::{json, Value};

/// The lobby opens this many minutes before `when`.
pub const LOBBY_BEFORE_MINUTES: i64 = 30;
/// The pocket closes this many hours after `when` (the sweep then ends it).
pub const CLOSES_AFTER_HOURS: i64 = 4;
/// The reminder's wall-clock time, the evening before.
pub const REMIND_HOUR: u32 = 19;
/// The notification kind (`continuity::NOTIFY_KINDS`).
pub const NOTIFY_KIND: &str = "pocket_app";
/// Longest pocket title a notification or a chat line repeats.
const TITLE_CHARS: usize = 60;

/// The chat a pocket came from: a bot's direct chat (no `channel_id`) or a channel, optionally a
/// thread in it. Its card, the reminder and the results go back there.
#[derive(Debug, Clone, Default, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Origin {
    pub kwant_id: String,
    #[serde(default)]
    pub channel_id: Option<String>,
    #[serde(default)]
    pub thread_id: Option<String>,
}

/// Where local times are read: the person's IANA zone, else the offset `when` was written in.
#[derive(Debug, Clone, PartialEq)]
pub enum Zone {
    Named(String),
    Fixed(FixedOffset),
}

impl Zone {
    /// From what `pocket_schedule.zone` stores: `+HH:MM`/`-HH:MM`, else an IANA name.
    pub fn parse(s: &str) -> Zone {
        let s = s.trim();
        if s.starts_with('+') || s.starts_with('-') {
            if let Ok(t) = DateTime::parse_from_rfc3339(&format!("2000-01-01T00:00:00{s}")) {
                return Zone::Fixed(*t.offset());
            }
        }
        if s.is_empty() || crate::routines::now_in(s, Utc::now()).is_err() {
            return Zone::Fixed(FixedOffset::east_opt(0).expect("zero offset"));
        }
        Zone::Named(s.to_string())
    }

    pub fn text(&self) -> String {
        match self {
            Zone::Named(n) => n.clone(),
            Zone::Fixed(o) => o.to_string(),
        }
    }

    /// The wall clock at `at`.
    pub fn local(&self, at: DateTime<Utc>) -> NaiveDateTime {
        match self {
            Zone::Named(n) => crate::routines::local_datetime(n, at).ok().flatten().unwrap_or_else(|| at.naive_utc()),
            Zone::Fixed(o) => at.with_timezone(o).naive_local(),
        }
    }

    /// `date` at wall-clock `time` (a time skipped by a DST change moves forward, as routines do).
    pub fn instant(&self, date: NaiveDate, time: NaiveTime) -> DateTime<Utc> {
        match self {
            Zone::Named(n) => crate::routines::local_instant(n, date, time).ok().flatten().unwrap_or_else(|| date.and_time(time).and_utc()),
            Zone::Fixed(o) => (date.and_time(time) - Duration::seconds(i64::from(o.local_minus_utc()))).and_utc(),
        }
    }
}

/// The spec's `when`: RFC 3339 with its offset (pa2 schema), `None` when missing or unreadable.
pub fn when(spec: &Value) -> Option<DateTime<FixedOffset>> {
    DateTime::parse_from_rfc3339(spec.get("when")?.as_str()?.trim()).ok()
}

/// The zone for one pocket: the person's `timezone` setting when it's a known zone, else the
/// offset `when` was written in, else UTC.
pub fn zone_for(person_tz: Option<&str>, when: Option<DateTime<FixedOffset>>) -> Zone {
    match person_tz.map(str::trim).filter(|t| !t.is_empty() && crate::routines::now_in(t, Utc::now()).is_ok()) {
        Some(tz) => Zone::Named(tz.to_string()),
        None => Zone::Fixed(when.map_or_else(|| FixedOffset::east_opt(0).expect("zero offset"), |w| *w.offset())),
    }
}

/// A pocket's times from its `when` and any explicit ones.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Times {
    pub opens_at: Option<DateTime<Utc>>,
    pub closes_at: Option<DateTime<Utc>>,
    /// `when`, only while it is still ahead.
    pub starts_at: Option<DateTime<Utc>>,
    pub opens_from_when: bool,
    pub closes_from_when: bool,
}

/// A future `when` with no explicit time opens the lobby 30 minutes before it and closes 4 hours
/// after it; explicit values win. A past or missing `when` changes nothing (the pocket opens
/// now). A closing time from `when` that would not be after an explicit later opening is 4 hours
/// after that opening instead.
pub fn times(when: Option<DateTime<Utc>>, opens_at: Option<DateTime<Utc>>, closes_at: Option<DateTime<Utc>>, now: DateTime<Utc>) -> Times {
    let Some(w) = when.filter(|w| *w > now) else {
        return Times { opens_at, closes_at, starts_at: None, opens_from_when: false, closes_from_when: false };
    };
    let after = Duration::hours(CLOSES_AFTER_HOURS);
    let opens = opens_at.unwrap_or(w - Duration::minutes(LOBBY_BEFORE_MINUTES));
    let closes = closes_at.unwrap_or_else(|| (w + after).max(opens + after));
    Times { opens_at: Some(opens), closes_at: Some(closes), starts_at: Some(w), opens_from_when: opens_at.is_none(), closes_from_when: closes_at.is_none() }
}

/// 19:00 local on the day before `starts_at`; `None` once that time has passed (a pocket made
/// the evening before, after 19:00, gets no reminder).
pub fn remind_at(starts_at: DateTime<Utc>, zone: &Zone, now: DateTime<Utc>) -> Option<DateTime<Utc>> {
    let day_before = zone.local(starts_at).date().pred_opt()?;
    let at = zone.instant(day_before, NaiveTime::from_hms_opt(REMIND_HOUR, 0, 0)?);
    (at > now && at < starts_at).then_some(at)
}

/// "8 PM", "7:30 PM".
pub fn clock(t: NaiveDateTime) -> String {
    let (pm, h) = chrono::Timelike::hour12(&t);
    let m = chrono::Timelike::minute(&t);
    let ampm = if pm { "PM" } else { "AM" };
    if m == 0 {
        format!("{h} {ampm}")
    } else {
        format!("{h}:{m:02} {ampm}")
    }
}

fn short_title(title: &str) -> String {
    let t = title.trim();
    let t = if t.is_empty() { "Your pocket app" } else { t };
    if t.chars().count() <= TITLE_CHARS {
        t.to_string()
    } else {
        format!("{}…", t.chars().take(TITLE_CHARS - 1).collect::<String>().trim_end())
    }
}

/// "today" / "tomorrow" for `starts_at` seen at `now`; `None` for any other day (a reminder
/// that is late by more than a day isn't sent).
fn day_word(zone: &Zone, starts_at: DateTime<Utc>, now: DateTime<Utc>) -> Option<&'static str> {
    match (zone.local(starts_at).date() - zone.local(now).date()).num_days() {
        0 => Some("today"),
        1 => Some("tomorrow"),
        _ => None,
    }
}

/// The reminder's words: `(notification title, notification body, chat line)`. Titles and times
/// only; never a guest.
pub fn reminder_copy(title: &str, zone: &Zone, starts_at: DateTime<Utc>, opens_at: Option<DateTime<Utc>>, now: DateTime<Utc>, has_chat: bool) -> Option<(String, String, String)> {
    let day = day_word(zone, starts_at, now)?;
    let t = short_title(title);
    let start = clock(zone.local(starts_at));
    let opens = opens_at.filter(|o| *o > now).map(|o| clock(zone.local(o)));
    let lobby = opens.as_ref().map(|o| format!(" The lobby opens at {o} by itself.")).unwrap_or_default();
    let body = if has_chat { format!("{t} {day} at {start}. The card is in your chat.") } else { format!("{t} {day} at {start}.{lobby}") };
    let line = format!("Reminder: {t} is {day} at {start}.{lobby}");
    Some((format!("{t} {day}"), body, line))
}

/// The results' words: `(notification title, notification body, chat line)`. Counts only, the
/// same for every kit (Mira's copy check: the title says which pocket, the body the outcome).
pub fn results_copy(title: &str, players: usize, has_chat: bool) -> (String, String, String) {
    let t = short_title(title);
    let who = match players {
        0 => "Nobody joined.".to_string(),
        1 => "1 player took part.".to_string(),
        n => format!("{n} players took part."),
    };
    let body = if has_chat && players > 0 { format!("{who} The results are in your chat.") } else { who.clone() };
    (format!("{t} has ended"), body, format!("{t} has ended. {who}"))
}

/// The line under the card's name: "Fri, Nov 27 · 8 PM".
fn when_line(zone: &Zone, starts_at: DateTime<Utc>) -> String {
    let l = zone.local(starts_at);
    format!("{} · {}", l.format("%a, %b %-d"), clock(l))
}

/// The guests' names leave a posted results block: each team's `members` empties and
/// `results.names_deleted_at` records when. Team names, points and the winner stay (Mira's
/// copy check: the Ended card promises "Names and answers are deleted on …").
pub fn blank_names(block: &mut Value, at: DateTime<Utc>) {
    let Some(results) = block.get_mut("results").and_then(Value::as_object_mut) else { return };
    if let Some(teams) = results.get_mut("teams").and_then(Value::as_array_mut) {
        for team in teams.iter_mut().filter_map(Value::as_object_mut) {
            if team.contains_key("members") {
                team.insert("members".into(), json!([]));
            }
        }
    }
    results.insert("names_deleted_at".into(), json!(at));
}

/// The kit's results when they fit the card's shape (1-12 teams), else `Null`.
fn fitting_results(v: Value) -> Value {
    match v.get("teams").and_then(Value::as_array) {
        Some(t) if (1..=12).contains(&t.len()) => v,
        _ => Value::Null,
    }
}

/// The ended pocket as the chat's pocket card (Pixel's pa4/impl.md "PocketBlock: the shape
/// Vega's pa3 emits"): state `ended` with `results`. Stored in the message's `meta.block` until
/// the server's Smart Results learn the `pocket` block (pa3/pa4).
pub fn ended_block(s: &Snapshot, sched: Option<&ScheduleRow>, zone: &Zone) -> Value {
    let results = s
        .kit()
        .map(|k| k.results(&super::kits::KitCtx::new(&s.spec, &s.players, Utc::now()), &s.kit_state))
        .map_or(Value::Null, fitting_results);
    let winner = results["teams"].as_array().and_then(|a| a.first()).map(|t| json!({ "name": t["name"], "points": t["points"] }));
    json!({
        "id": format!("pocket-{}-ended", s.row.code),
        "type": "pocket",
        "title": null,
        "source": { "kind": "slot" },
        "provenance": { "kind": "kwant_data", "ref": format!("pocket:{}", s.row.code) },
        "code": s.row.code,
        "kit": s.row.kit,
        "name": s.row.title.chars().take(80).collect::<String>(),
        "when": sched.and_then(|r| r.starts_at).map(|w| when_line(zone, w)),
        "state": "ended",
        "progress": null,
        "rounds": [],
        "checks": null,
        "dry_run": null,
        "eta": null,
        "opens_line": null,
        "live": null,
        "join": null,
        "ended": { "at": s.row.ended_at, "winner": winner, "stats": null, "deletes_on": s.row.expires_at },
        "budget": null,
        "results": results,
    })
}

// ---- Planning (create and a new `when`) ----

async fn person_tz(state: &AppState) -> Option<String> {
    state.db.get_setting(crate::server::bot_actions::TIMEZONE_SETTING).await.ok().flatten()
}

/// Checks an origin names a bot, channel and thread that exist.
pub async fn check_origin(state: &AppState, o: &Origin) -> PResult<()> {
    let bad = || PocketError::new(StatusCode::BAD_REQUEST, "bad_origin", "The chat this pocket came from doesn't exist.");
    let ok_id = |s: &str| !s.is_empty() && s.len() <= 128;
    if !ok_id(&o.kwant_id) || state.db.get_kwant(&o.kwant_id).await?.is_none() {
        return Err(bad());
    }
    if let Some(c) = &o.channel_id {
        if !ok_id(c) || state.db.get_channel(c).await?.is_none() {
            return Err(bad());
        }
    }
    if let Some(t) = &o.thread_id {
        if !ok_id(t) || state.db.get_thread(t).await?.is_none() {
            return Err(bad());
        }
    }
    Ok(())
}

/// The times `create` stores, and its schedule row (written once the pocket exists).
pub async fn plan(state: &AppState, spec: &Value, opens_at: Option<DateTime<Utc>>, closes_at: Option<DateTime<Utc>>, origin: Option<&Origin>, now: DateTime<Utc>) -> (Times, ScheduleRow) {
    let w = when(spec);
    let t = times(w.map(|w| w.with_timezone(&Utc)), opens_at, closes_at, now);
    let zone = zone_for(person_tz(state).await.as_deref(), w);
    let row = ScheduleRow {
        pocket_code: String::new(),
        origin: origin.and_then(|o| serde_json::to_string(o).ok()),
        starts_at: t.starts_at,
        zone: zone.text(),
        opens_from_when: t.opens_from_when,
        closes_from_when: t.closes_from_when,
        remind_at: t.starts_at.and_then(|s| remind_at(s, &zone, now)),
        reminded_at: None,
        results_posted_at: None,
    };
    (t, row)
}

/// A new `when` for an open pocket (pa3's update tool, `POST /api/pockets/:code/when`): sets the
/// spec's `when`, moves the times that came from the old one, and re-plans the reminder (it
/// fires again if its time moved). `None` removes `when`: times from it are cleared.
pub async fn set_when(state: &AppState, code: &str, new_when: Option<DateTime<FixedOffset>>) -> PResult<std::sync::Arc<Snapshot>> {
    let lock = state.pocket.lock_for(code);
    let _g = lock.lock().await;
    let s = super::snapshot(state, code).await?.ok_or_else(PocketError::not_found)?;
    let kit = s.kit().ok_or_else(PocketError::not_found)?;
    if s.row.state != OPEN {
        return Err(PocketError::new(StatusCode::GONE, "ended", "This pocket app has ended."));
    }
    let mut spec = s.spec.clone();
    let Some(obj) = spec.as_object_mut() else {
        return Err(PocketError::new(StatusCode::BAD_REQUEST, "bad_spec", "The spec must be a JSON object."));
    };
    match new_when {
        Some(w) => obj.insert("when".into(), json!(w.to_rfc3339())),
        None => obj.remove("when"),
    };
    kit.validate_spec(&spec).map_err(|m| PocketError::new(StatusCode::BAD_REQUEST, "bad_spec", m))?;
    let sched = state.db.pocket_schedule(code).await?;
    let (opens_derived, closes_derived) = sched.as_ref().map_or((false, false), |r| (r.opens_from_when, r.closes_from_when));
    let explicit_opens = if opens_derived { None } else { s.row.opens_at };
    let explicit_closes = if closes_derived { None } else { s.row.closes_at };
    let now = Utc::now();
    let t = times(new_when.map(|w| w.with_timezone(&Utc)), explicit_opens, explicit_closes, now);
    if let (Some(o), Some(c)) = (t.opens_at, t.closes_at) {
        if c <= o {
            return Err(PocketError::new(StatusCode::BAD_REQUEST, "bad_times", "closes_at must be after opens_at."));
        }
    }
    let zone = zone_for(person_tz(state).await.as_deref(), new_when);
    let remind = t.starts_at.and_then(|st| remind_at(st, &zone, now));
    let text = serde_json::to_string(&spec).map_err(|_| PocketError::internal())?;
    if sched.is_none() {
        // A pocket made before pa7: give it a row first
        state
            .db
            .pocket_schedule_insert(&ScheduleRow {
                pocket_code: code.into(),
                origin: None,
                starts_at: None,
                zone: zone.text(),
                opens_from_when: false,
                closes_from_when: false,
                remind_at: None,
                reminded_at: None,
                results_posted_at: None,
            })
            .await?;
    }
    if !state.db.pocket_reschedule(code, &text, t.opens_at, t.closes_at, t.starts_at, &zone.text(), remind).await? {
        return Err(PocketError::new(StatusCode::GONE, "ended", "This pocket app has ended."));
    }
    // Which times now come from `when` (an explicit one stays explicit)
    if let Some(mut row) = state.db.pocket_schedule(code).await? {
        row.opens_from_when = t.opens_from_when;
        row.closes_from_when = t.closes_from_when;
        state.db.pocket_schedule_insert(&row).await?;
    }
    super::changed(state, code).await?;
    super::snapshot(state, code).await?.ok_or_else(PocketError::not_found)
}

// ---- The minute tick ----

/// What one run did (for tests and logs).
#[derive(Debug, Default, PartialEq)]
pub struct Ran {
    pub ended: usize,
    pub reminded: Vec<String>,
    pub results: Vec<String>,
    /// Results messages whose guest names were blanked (the pocket's data expired or was deleted).
    pub names_deleted: usize,
}

/// One run at `now`: end pockets whose kit finished or whose closing time passed, send due
/// reminders, then post the results of ended pockets. Does nothing while the switch is off.
/// Runs before the sweep, so a pocket that keeps nothing after it ends still gets its results
/// posted before the sweep deletes it. Blanking guest names in posted results comes first and
/// runs even with the switch off: it only ever removes data.
pub async fn run_once(state: &AppState, now: DateTime<Utc>) -> Ran {
    let mut ran = Ran { names_deleted: forget_names(state, now).await, ..Ran::default() };
    if !super::enabled(&state.db).await {
        return ran;
    }
    let mut to_end = state.db.pocket_past_closing(now).await.unwrap_or_default();
    for code in state.db.pocket_codes_open().await.unwrap_or_default() {
        if let Ok(Some(s)) = super::snapshot(state, &code).await {
            if s.kit().is_some_and(|k| k.finished(&s.kit_state)) && !to_end.contains(&code) {
                to_end.push(code);
            }
        }
    }
    for code in to_end {
        if super::end(state, &code).await.is_ok() {
            ran.ended += 1;
        }
    }
    match state.db.pocket_due_reminders(now).await {
        Ok(due) => {
            for (code, at) in due {
                match remind(state, &code, at, now).await {
                    Ok(true) => ran.reminded.push(code),
                    Ok(false) => {}
                    Err(e) => tracing::warn!(code = %code, "pocket apps: reminder failed: {}", e.code),
                }
            }
        }
        Err(e) => tracing::warn!("pocket apps: couldn't list due reminders: {e}"),
    }
    match state.db.pocket_results_due().await {
        Ok(due) => {
            for code in due {
                match post_results(state, &code, now).await {
                    Ok(true) => ran.results.push(code),
                    Ok(false) => {}
                    Err(e) => tracing::warn!(code = %code, "pocket apps: results post failed: {}", e.code),
                }
            }
        }
        Err(e) => tracing::warn!("pocket apps: couldn't list ended pockets: {e}"),
    }
    ran
}

/// Blanks guest names in posted results whose pocket's data is gone or expires by `now` (the
/// sweep deletes it right after this run), and tells open chats.
async fn forget_names(state: &AppState, now: DateTime<Utc>) -> usize {
    let ids = match state.db.pocket_result_posts_expired(now).await {
        Ok(ids) => ids,
        Err(e) => {
            tracing::warn!("pocket apps: couldn't list results to clear names from: {e}");
            return 0;
        }
    };
    let mut done = 0;
    for id in ids {
        if let Ok(Some(mut m)) = state.db.get_message(&id).await {
            let mut meta: Value = m.meta.as_deref().and_then(|t| serde_json::from_str(t).ok()).unwrap_or(Value::Null);
            if meta.get("block").is_some() {
                blank_names(&mut meta["block"], now);
                if let Err(e) = state.db.set_message_meta(&id, &meta).await {
                    tracing::warn!("pocket apps: couldn't clear names from a results message: {e}");
                    continue;
                }
                m.meta = Some(meta.to_string());
                let _ = state.event_sender.send(RuntimeEvent::MessageAdded { message: serde_json::to_value(&m).unwrap_or(Value::Null) });
                done += 1;
            }
        }
        let _ = state.db.pocket_result_post_delete(&id).await;
    }
    done
}

fn origin_of(row: &ScheduleRow) -> Option<Origin> {
    row.origin.as_deref().and_then(|o| serde_json::from_str(o).ok())
}

/// The reminder for `code`, claimed first so it goes once. `false`: someone else claimed it, or
/// it is too late (the pocket starts later than tomorrow's date or has started; claimed and dropped).
async fn remind(state: &AppState, code: &str, at: DateTime<Utc>, now: DateTime<Utc>) -> PResult<bool> {
    if !state.db.pocket_claim_reminder(code, at, now).await? {
        return Ok(false);
    }
    let (Some(row), Some(s)) = (state.db.pocket_schedule(code).await?, super::snapshot(state, code).await?) else { return Ok(false) };
    if s.row.state != OPEN {
        return Ok(false);
    }
    let Some(starts_at) = row.starts_at.filter(|w| *w > now) else { return Ok(false) };
    let zone = Zone::parse(&row.zone);
    let Some((_, _, line)) = reminder_copy(&s.row.title, &zone, starts_at, s.row.opens_at, now, true) else { return Ok(false) };
    let origin = origin_of(&row);
    let posted = match &origin {
        Some(o) => post(state, o, json!({ "kind": "pocket", "reason": "reminder", "code": code, "starts_at": starts_at, "opens_at": s.row.opens_at, "block": null }), line).await.is_some(),
        None => false,
    };
    let Some((title, body, _)) = reminder_copy(&s.row.title, &zone, starts_at, s.row.opens_at, now, posted) else { return Ok(false) };
    notify(state, origin.as_ref(), &s, title, body).await;
    Ok(true)
}

/// Posts the ended pocket's results once.
async fn post_results(state: &AppState, code: &str, now: DateTime<Utc>) -> PResult<bool> {
    let Some(s) = super::snapshot(state, code).await? else { return Ok(false) };
    if s.row.state != ENDED || !state.db.pocket_claim_results(code, now).await? {
        return Ok(false);
    }
    let row = state.db.pocket_schedule(code).await?;
    let zone = Zone::parse(row.as_ref().map_or("UTC", |r| r.zone.as_str()));
    let origin = row.as_ref().and_then(origin_of);
    let (_, _, line) = results_copy(&s.row.title, s.players.len(), true);
    let posted = match &origin {
        Some(o) => {
            let mut block = ended_block(&s, row.as_ref(), &zone);
            // Data that is deleted as the pocket ends (or already due) never reaches the chat with names
            let expiring = s.row.expires_at.is_some_and(|e| e <= now);
            if expiring {
                blank_names(&mut block, now);
            }
            let id = post(state, o, json!({ "kind": "pocket", "reason": "results", "code": code, "block": block }), line).await;
            if let (Some(id), false) = (&id, expiring) {
                if let Err(e) = state.db.pocket_result_post_insert(id, code).await {
                    tracing::warn!(code = %code, "pocket apps: couldn't note a results message for its names' expiry: {e}");
                }
            }
            id.is_some()
        }
        None => false,
    };
    let (title, body, _) = results_copy(&s.row.title, s.players.len(), posted);
    notify(state, origin.as_ref(), &s, title, body).await;
    Ok(true)
}

/// The bot's name for a chat line (as its replies have it), else its id.
async fn sender(state: &AppState, kwant_id: &str) -> String {
    match state.db.get_kwant(kwant_id).await {
        Ok(Some(k)) if !k.name.trim().is_empty() => k.name,
        _ => kwant_id.to_string(),
    }
}

/// Posts a card into the origin chat: the message's id, `None` when the chat is gone (the
/// notification still goes).
async fn post(state: &AppState, o: &Origin, mut meta: Value, text: String) -> Option<String> {
    if check_origin(state, o).await.is_err() {
        return None;
    }
    let message = NewMessage {
        channel_id: o.channel_id.clone(),
        direct_kwant_id: if o.channel_id.is_some() { None } else { Some(o.kwant_id.clone()) },
        sender_role: "kwant".into(),
        sender_id: sender(state, &o.kwant_id).await,
        text,
        reasoning_content: None,
        tool_calls: None,
        tokens_used: 0,
    };
    meta["kwant_id"] = json!(o.kwant_id);
    let extra = MessageExtra { thread_id: o.thread_id.clone(), meta: Some(meta) };
    match state.db.insert_message_with(&message, &extra).await {
        Ok(m) => {
            if let Some(t) = &o.thread_id {
                let _ = state.db.touch_thread(t).await;
            }
            let _ = state.event_sender.send(RuntimeEvent::MessageAdded { message: serde_json::to_value(&m).unwrap_or(Value::Null) });
            Some(m.id)
        }
        Err(e) => {
            tracing::warn!("pocket apps: couldn't post into the pocket's chat: {e}");
            None
        }
    }
}

async fn notify(state: &AppState, origin: Option<&Origin>, s: &Snapshot, title: String, body: String) {
    let kwant_id = origin.map(|o| o.kwant_id.clone()).or_else(|| s.row.owner_kwant_id.clone()).unwrap_or_default();
    let kwant_name = if kwant_id.is_empty() { "Pocket apps".to_string() } else { sender(state, &kwant_id).await };
    continuity::notify(
        state,
        Notification {
            kind: NOTIFY_KIND,
            kwant_id,
            kwant_name,
            channel_id: origin.and_then(|o| o.channel_id.clone()),
            title,
            body,
            thread_id: origin.and_then(|o| o.thread_id.clone()),
            card_id: None,
        },
    )
    .await;
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn utc(y: i32, mo: u32, d: u32, h: u32, mi: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(y, mo, d, h, mi, 0).unwrap()
    }

    #[test]
    fn opens_thirty_minutes_before_and_closes_four_hours_after() {
        let now = utc(2026, 11, 23, 12, 0);
        let w = utc(2026, 11, 27, 20, 0);
        let t = times(Some(w), None, None, now);
        assert_eq!(t.opens_at, Some(utc(2026, 11, 27, 19, 30)));
        assert_eq!(t.closes_at, Some(utc(2026, 11, 28, 0, 0)));
        assert_eq!(t.starts_at, Some(w));
        assert!(t.opens_from_when && t.closes_from_when);
    }

    #[test]
    fn explicit_times_win_and_a_past_or_missing_when_opens_now() {
        let now = utc(2026, 11, 23, 12, 0);
        let w = utc(2026, 11, 27, 20, 0);
        let o = utc(2026, 11, 27, 18, 0);
        let c = utc(2026, 11, 27, 23, 0);
        let t = times(Some(w), Some(o), Some(c), now);
        assert_eq!((t.opens_at, t.closes_at, t.opens_from_when, t.closes_from_when), (Some(o), Some(c), false, false));
        let t = times(Some(w), Some(o), None, now);
        assert_eq!((t.opens_at, t.closes_at), (Some(o), Some(utc(2026, 11, 28, 0, 0))));
        // An explicit opening after the derived close: closes 4 h after it instead
        let late = utc(2026, 11, 28, 1, 0);
        assert_eq!(times(Some(w), Some(late), None, now).closes_at, Some(utc(2026, 11, 28, 5, 0)));
        let past = times(Some(utc(2026, 11, 20, 20, 0)), None, None, now);
        assert_eq!(past, Times { opens_at: None, closes_at: None, starts_at: None, opens_from_when: false, closes_from_when: false });
        assert_eq!(times(None, None, Some(c), now).closes_at, Some(c));
        assert_eq!(times(None, None, None, now).opens_at, None);
    }

    #[test]
    fn the_reminder_is_seven_pm_local_the_day_before() {
        let now = utc(2026, 11, 23, 12, 0);
        // Friday 27 Nov 2026 20:00 in Kyiv (UTC+2) is 18:00 UTC; Thursday 19:00 Kyiv is 17:00 UTC
        let w = DateTime::parse_from_rfc3339("2026-11-27T20:00:00+02:00").unwrap().with_timezone(&Utc);
        assert_eq!(remind_at(w, &Zone::Named("Europe/Kyiv".into()), now), Some(utc(2026, 11, 26, 17, 0)));
        // Without a named zone, the offset `when` was written in
        let z = zone_for(None, Some(DateTime::parse_from_rfc3339("2026-11-27T20:00:00+02:00").unwrap()));
        assert_eq!(z, Zone::Fixed(FixedOffset::east_opt(7200).unwrap()));
        assert_eq!(remind_at(w, &z, now), Some(utc(2026, 11, 26, 17, 0)));
        assert_eq!(Zone::parse(&z.text()), z);
        // A bad setting falls back to the offset
        assert_eq!(zone_for(Some("Not/AZone"), Some(DateTime::parse_from_rfc3339("2026-11-27T20:00:00+02:00").unwrap())), z);
    }

    #[test]
    fn none_when_made_after_seven_the_evening_before() {
        let w = utc(2026, 11, 27, 20, 0);
        let z = Zone::Named("UTC".into());
        assert_eq!(remind_at(w, &z, utc(2026, 11, 26, 18, 59)), Some(utc(2026, 11, 26, 19, 0)));
        assert_eq!(remind_at(w, &z, utc(2026, 11, 26, 19, 0)), None);
        assert_eq!(remind_at(w, &z, utc(2026, 11, 26, 20, 0)), None);
        // An event at 01:00: the reminder is still the evening before, 6 h ahead
        assert_eq!(remind_at(utc(2026, 11, 27, 1, 0), &z, utc(2026, 11, 20, 0, 0)), Some(utc(2026, 11, 26, 19, 0)));
    }

    #[test]
    fn across_a_dst_change() {
        let berlin = Zone::Named("Europe/Berlin".into());
        let before = utc(2026, 3, 1, 0, 0);
        // Spring: Sun 29 Mar 2026 20:00 CEST (+02) = 18:00 UTC; Sat 28 Mar 19:00 is still CET (+01) = 18:00 UTC
        let w = DateTime::parse_from_rfc3339("2026-03-29T20:00:00+02:00").unwrap().with_timezone(&Utc);
        assert_eq!(remind_at(w, &berlin, before), Some(utc(2026, 3, 28, 18, 0)));
        let t = times(Some(w), None, None, before);
        assert_eq!((t.opens_at, t.closes_at), (Some(utc(2026, 3, 29, 17, 30)), Some(utc(2026, 3, 29, 22, 0))));
        // Autumn: Sun 25 Oct 2026 20:00 CET (+01) = 19:00 UTC; Sat 24 Oct 19:00 CEST (+02) = 17:00 UTC
        let w = DateTime::parse_from_rfc3339("2026-10-25T20:00:00+01:00").unwrap().with_timezone(&Utc);
        assert_eq!(remind_at(w, &berlin, before), Some(utc(2026, 10, 24, 17, 0)));
        assert_eq!(clock(berlin.local(w)), "8 PM");
        assert_eq!(clock(berlin.local(w - Duration::minutes(LOBBY_BEFORE_MINUTES))), "7:30 PM");
    }

    #[test]
    fn the_words_name_the_title_and_times_only() {
        let z = Zone::Named("UTC".into());
        let w = utc(2026, 11, 27, 20, 0);
        let now = utc(2026, 11, 26, 19, 0);
        let (title, body, line) = reminder_copy("Pub quiz", &z, w, Some(w - Duration::minutes(LOBBY_BEFORE_MINUTES)), now, true).unwrap();
        assert_eq!(title, "Pub quiz tomorrow");
        assert_eq!(body, "Pub quiz tomorrow at 8 PM. The card is in your chat.");
        assert_eq!(line, "Reminder: Pub quiz is tomorrow at 8 PM. The lobby opens at 7:30 PM by itself.");
        let (_, body, _) = reminder_copy("Pub quiz", &z, w, Some(w - Duration::minutes(LOBBY_BEFORE_MINUTES)), now, false).unwrap();
        assert_eq!(body, "Pub quiz tomorrow at 8 PM. The lobby opens at 7:30 PM by itself.");
        // Late by a day (the server was off): no reminder rather than a wrong "tomorrow"
        assert!(reminder_copy("Pub quiz", &z, w, None, utc(2026, 11, 25, 19, 0), true).is_none());
        assert_eq!(reminder_copy("Pub quiz", &z, w, None, utc(2026, 11, 27, 9, 0), true).unwrap().1, "Pub quiz today at 8 PM. The card is in your chat.");
        let (title, body, line) = results_copy("Pub quiz", 9, true);
        assert_eq!((title.as_str(), body.as_str(), line.as_str()), ("Pub quiz has ended", "9 players took part. The results are in your chat.", "Pub quiz has ended. 9 players took part."));
        let s = |a: &str, b: &str, c: &str| (a.to_string(), b.to_string(), c.to_string());
        assert_eq!(results_copy("Pub quiz", 1, true), s("Pub quiz has ended", "1 player took part. The results are in your chat.", "Pub quiz has ended. 1 player took part."));
        assert_eq!(results_copy("Pub quiz", 0, true), s("Pub quiz has ended", "Nobody joined.", "Pub quiz has ended. Nobody joined."));
        assert_eq!(results_copy("Pub quiz", 9, false).1, "9 players took part.");
        assert_eq!(results_copy("", 1, false), s("Your pocket app has ended", "1 player took part.", "Your pocket app has ended. 1 player took part."));
        assert_eq!(when_line(&z, w), "Fri, Nov 27 · 8 PM");
        assert_eq!(when_line(&z, w - Duration::minutes(30)), "Fri, Nov 27 · 7:30 PM");
        let long = "x".repeat(200);
        assert_eq!(short_title(&long).chars().count(), TITLE_CHARS);
    }

    #[test]
    fn expiry_blanks_player_names_and_keeps_teams_and_scores() {
        let at = utc(2026, 12, 4, 20, 0);
        let mut block = json!({ "type": "pocket", "ended": { "winner": { "name": "Quizzly Bears", "points": 41 } }, "results": {
            "teams": [
                { "name": "Quizzly Bears", "players": 2, "points": 41, "members": ["Mia", "Zyxqv Canary"] },
                { "name": "Team 2", "players": 1, "points": 30, "members": ["Ola"] },
            ],
            "margin": 11,
        } });
        blank_names(&mut block, at);
        assert!(!block.to_string().contains("Zyxqv") && !block.to_string().contains("Mia"), "{block}");
        assert_eq!(block["results"]["teams"][0], json!({ "name": "Quizzly Bears", "players": 2, "points": 41, "members": [] }));
        assert_eq!(block["results"]["teams"][1]["points"], json!(30));
        assert_eq!(block["results"]["names_deleted_at"], json!(at));
        assert_eq!(block["ended"]["winner"]["name"], json!("Quizzly Bears"));
        // No results (a kit without them): nothing to blank
        let mut empty = json!({ "type": "pocket", "results": null });
        blank_names(&mut empty, at);
        assert_eq!(empty["results"], Value::Null);
    }
}
