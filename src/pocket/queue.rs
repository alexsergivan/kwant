//! pa11b: the Queue ticket kit. A shop puts one QR on the counter; a customer taps "Take a
//! ticket" and sees "You're 4th"; the owner presses "Call next". Brief:
//! /home/maus/shared/ship-software-safely/autonomy/pocket-apps/pa11/queue-server-brief.md, design
//! (the contract for every name here) in queue-design.md "For the server", API in queue-impl.md.
//!
//! No model turns: nothing here (or in the core for this kit) calls an engine, so a queue keeps
//! working past any daily limit. No accounts and no contact details: a guest joins with no name
//! at all ([`Kit::nameless_join`]); a first name, when the spec asks for one, lives only on the
//! ticket and only the host sees it.
//!
//! Retention (plan section 8 rule 11): a ticket's first name is wiped the moment it is served,
//! left, removed or closed; the ticket itself (and the guest's player row, keys and events) goes
//! at the sweep [`DONE_KEEP_MINUTES`] later, so the guest's open page can still say "Served" or
//! "Closed". A skipped ticket keeps its name for the [`RECALL_MINUTES`] the host may bring it
//! back, then goes the same way. Only counts stay: today's numbers and the totals for the Done
//! row, no names and no per-ticket rows.

use super::kits::{Kit, KitCtx, KitError, KitRules, Retention, Viewer};
use super::words;
use chrono::{DateTime, Duration, Local, NaiveDate, NaiveTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

pub const ID: &str = "queue";

pub const TITLE_MAX: usize = 40;
pub const LINE_MAX: usize = 80;
pub const HOST_NAME_MAX: usize = 40;
pub const NAME_MAX: usize = 24;
pub const PAUSE_TEXT_MAX: usize = 60;
pub const CLOSE_TEXT_MAX: usize = 140;
pub const CALL_MINUTES_DEFAULT: i64 = 3;
pub const CALL_MINUTES_MIN: i64 = 1;
pub const CALL_MINUTES_MAX: i64 = 10;
pub const MAX_WAITING_DEFAULT: i64 = 100;
pub const MAX_WAITING_MIN: i64 = 10;
pub const MAX_WAITING_MAX: i64 = 200;
/// How long a skipped ticket may be brought back (`recall`).
pub const RECALL_MINUTES: i64 = 10;
/// How long a finished ticket (served, left, removed, closed), already without its name, stays so
/// the guest's open page can say what happened; then it and the guest's player go.
pub const DONE_KEEP_MINUTES: i64 = 2;
/// How long a guest who joined but holds no ticket (a refused take) keeps their key.
pub const NO_TICKET_KEEP_MINUTES: i64 = 10;
/// Players a queue takes beyond `max_waiting`: guests whose finished tickets haven't been swept
/// yet, or who joined and were refused.
pub const PLAYER_SLACK: usize = 40;
/// The most players any queue takes (`max_waiting` 200 plus the slack).
pub const PLAYER_CEILING: usize = MAX_WAITING_MAX as usize + PLAYER_SLACK;
/// Calls today before `avg_minutes` is shown (never a guessed wait).
pub const AVG_AFTER_CALLS: usize = 3;
/// Days the counts stay after the queue ends (with the pocket).
pub const KEEP_DAYS_AFTER_END: i64 = 7;
/// What the guest page shows when the host closes without a message, or the hours run out.
pub const DEFAULT_CLOSE_TEXT: &str = "Sorry, we've closed for today. Thanks for waiting, and see you next time!";

pub const OPEN: &str = "open";
pub const PAUSED: &str = "paused";
pub const CLOSED: &str = "closed";
pub const ENDED: &str = "ended";

pub const WAITING: &str = "waiting";
pub const CALLED: &str = "called";
pub const SERVED: &str = "served";
pub const SKIPPED: &str = "skipped";
pub const LEFT: &str = "left";
pub const REMOVED: &str = "removed";

const GUEST_ACTIONS: &[&str] = &["take", "leave"];
const HOST_ACTIONS: &[&str] = &["call_next", "serve", "skip", "recall", "remove", "pause", "resume", "close", "reopen", "end", "set_hours"];

pub struct Queue;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
struct Ticket {
    number: u64,
    #[serde(default)]
    name: Option<String>,
    state: String,
    taken_at: DateTime<Utc>,
    #[serde(default)]
    called_at: Option<DateTime<Utc>>,
    /// When it was served, skipped, left, removed or closed.
    #[serde(default)]
    done_at: Option<DateTime<Utc>>,
    /// The guest's player id (their key).
    guest: String,
    /// Still waiting or called when the queue closed: kept, without a name, only so the guest's
    /// page can say so.
    #[serde(default)]
    closed: bool,
}

impl Ticket {
    fn live(&self) -> bool {
        !self.closed && (self.state == WAITING || self.state == CALLED)
    }

    /// Finished: the name is gone and the ticket waits only for the sweep.
    fn done(&self) -> bool {
        self.closed || matches!(self.state.as_str(), SERVED | LEFT | REMOVED)
    }

    fn finish(&mut self, state: &str, now: DateTime<Utc>) {
        self.state = state.to_string();
        self.done_at = Some(now);
        self.name = None;
    }

    fn recall_until(&self) -> Option<DateTime<Utc>> {
        (self.state == SKIPPED && !self.closed).then(|| self.done_at.map(|d| d + Duration::minutes(RECALL_MINUTES))).flatten()
    }

    /// When the sweep may delete it (and the guest's player with it).
    fn gone_at(&self) -> Option<DateTime<Utc>> {
        if self.done() {
            self.done_at.map(|d| d + Duration::minutes(DONE_KEEP_MINUTES))
        } else {
            self.recall_until()
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct St {
    state: String,
    #[serde(default)]
    pause_text: Option<String>,
    #[serde(default)]
    close_text: Option<String>,
    #[serde(default)]
    paused_at: Option<DateTime<Utc>>,
    #[serde(default)]
    closed_at: Option<DateTime<Utc>>,
    /// Closed by `hours.until`, not by the host.
    #[serde(default)]
    auto_closed: bool,
    /// Tickets still waiting or called at the last close.
    #[serde(default)]
    closed_waiting: u64,
    /// `HH:MM` on the box's clock (the spec's `hours.until`, changed by `set_hours`).
    #[serde(default)]
    hours_until: Option<String>,
    /// The next time `hours_until` comes round while open or paused.
    #[serde(default)]
    closes_at: Option<DateTime<Utc>>,
    #[serde(default)]
    counter: u64,
    #[serde(default)]
    now_serving: Option<u64>,
    #[serde(default)]
    tickets: Vec<Ticket>,
    /// The box's date the counts below are for.
    #[serde(default)]
    day: Option<NaiveDate>,
    /// Today's waits: `called_at - taken_at` in seconds, one per call (numbers only).
    #[serde(default)]
    calls_today: Vec<i64>,
    /// Today's seconds between one call and the next (`avg_minutes`).
    #[serde(default)]
    gaps_today: Vec<i64>,
    #[serde(default)]
    last_call_at: Option<DateTime<Utc>>,
    #[serde(default)]
    served_today: u64,
    #[serde(default)]
    skipped_today: u64,
    #[serde(default)]
    left_today: u64,
    /// Totals for the Done row.
    #[serde(default)]
    days: u64,
    #[serde(default)]
    served_total: u64,
    #[serde(default)]
    gap_secs_total: i64,
    #[serde(default)]
    gaps_total: u64,
}

fn load(state: &Value) -> St {
    serde_json::from_value(state.clone()).unwrap_or_default()
}

fn store(state: &mut Value, s: &St) {
    if let Ok(v) = serde_json::to_value(s) {
        *state = v;
    }
}

/// The box's local date at `t` (the queue's day follows the box's clock, like the front desk).
fn local_day(t: DateTime<Utc>) -> NaiveDate {
    t.with_timezone(&Local).date_naive()
}

fn parse_hm(s: &str) -> Option<NaiveTime> {
    let s = s.trim();
    if s.len() != 5 || s.as_bytes()[2] != b':' {
        return None;
    }
    NaiveTime::parse_from_str(s, "%H:%M").ok()
}

/// The next time the box's clock shows `hm` after `from`.
fn next_local(from: DateTime<Utc>, hm: NaiveTime) -> DateTime<Utc> {
    let mut day = local_day(from);
    for _ in 0..3 {
        if let Some(t) = day.and_time(hm).and_local_timezone(Local).earliest().map(|t| t.with_timezone(&Utc)) {
            if t > from {
                return t;
            }
        }
        day += Duration::days(1);
    }
    from + Duration::days(1)
}

fn spec_hours(spec: &Value) -> Option<String> {
    spec.get("hours").and_then(|h| h.get("until")).and_then(Value::as_str).and_then(|s| parse_hm(s).map(|_| s.trim().to_string()))
}

fn spec_int(spec: &Value, key: &str, default: i64) -> i64 {
    spec.get(key).and_then(Value::as_i64).unwrap_or(default)
}

fn call_minutes(spec: &Value) -> i64 {
    spec_int(spec, "call_minutes", CALL_MINUTES_DEFAULT).clamp(CALL_MINUTES_MIN, CALL_MINUTES_MAX)
}

fn max_waiting(spec: &Value) -> i64 {
    spec_int(spec, "max_waiting", MAX_WAITING_DEFAULT).clamp(MAX_WAITING_MIN, MAX_WAITING_MAX)
}

fn ask_name(spec: &Value) -> bool {
    spec.get("ask_name").and_then(Value::as_bool).unwrap_or(false)
}

fn spec_str(spec: &Value, key: &str) -> Option<String> {
    spec.get(key).and_then(Value::as_str).map(str::trim).filter(|s| !s.is_empty()).map(str::to_string)
}

fn has_control(s: &str) -> bool {
    s.chars().any(|c| c.is_control() || matches!(c, '\u{200b}'..='\u{200f}' | '\u{202a}'..='\u{202e}' | '\u{2060}'..='\u{2064}' | '\u{feff}'))
}

/// An optional text field of a spec: a string of at most `max` characters, no control characters.
fn check_spec_text(spec: &Value, key: &str, max: usize, required: bool) -> Result<(), String> {
    match spec.get(key) {
        None | Some(Value::Null) if !required => Ok(()),
        Some(Value::String(s)) => {
            let t = s.trim();
            if required && t.is_empty() {
                return Err(format!("The spec needs a {key}."));
            }
            if t.chars().count() > max {
                return Err(format!("{key} is at most {max} characters."));
            }
            if has_control(s) {
                return Err(format!("{key} can't have control characters."));
            }
            Ok(())
        }
        _ if required => Err(format!("The spec needs a {key}.")),
        _ => Err(format!("{key} must be text.")),
    }
}

fn check_spec_int(spec: &Value, key: &str, min: i64, max: i64) -> Result<(), String> {
    match spec.get(key) {
        None | Some(Value::Null) => Ok(()),
        Some(v) => match v.as_i64() {
            Some(n) if (min..=max).contains(&n) => Ok(()),
            _ => Err(format!("{key} must be a whole number from {min} to {max}.")),
        },
    }
}

/// A host or guest text in an action body: `None` when missing or blank; 400 when it isn't text,
/// has control characters or is over `max`.
fn body_text(body: &Value, key: &str, max: usize) -> Result<Option<String>, KitError> {
    match body.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(s)) => {
            if has_control(s) {
                return Err(KitError::BadInput(format!("{key} can't have control characters.")));
            }
            let t = s.split_whitespace().collect::<Vec<_>>().join(" ");
            if t.chars().count() > max {
                return Err(KitError::BadInput(format!("{key} is at most {max} characters.")));
            }
            Ok(Some(t).filter(|t| !t.is_empty()))
        }
        Some(_) => Err(KitError::BadInput(format!("{key} must be text."))),
    }
}

/// A ticket number in a host body: a whole number from 1 (`2` or `2.0`), else 400.
fn ticket_no(body: &Value, key: &str) -> Result<Option<u64>, KitError> {
    match body.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(v) => v
            .as_u64()
            .or_else(|| v.as_f64().filter(|f| f.is_finite() && f.fract() == 0.0 && *f >= 1.0 && *f < 1e15).map(|f| f as u64))
            .filter(|n| *n >= 1)
            .map(Some)
            .ok_or_else(|| KitError::BadInput(format!("{key} must be a ticket number."))),
    }
}

fn required_ticket(body: &Value) -> Result<u64, KitError> {
    ticket_no(body, "ticket")?.ok_or_else(|| KitError::BadInput("ticket must be a ticket number.".into()))
}

fn coded(status: u16, code: &'static str, message: &str, extra: Value) -> KitError {
    KitError::Coded { status, code, message: message.to_string(), extra }
}

fn no_such_ticket() -> KitError {
    coded(404, "no_such_ticket", "There's no ticket with that number in the queue.", json!({}))
}

fn closed_error(s: &St) -> KitError {
    coded(410, "closed", "The queue is closed for today.", json!({ "close_text": s.close_text }))
}

fn ended_error() -> KitError {
    coded(410, "ended", "This queue has ended.", json!({}))
}

fn paused_error(s: &St) -> KitError {
    coded(409, "paused", "The queue is paused. New tickets open when it resumes.", json!({ "pause_text": s.pause_text }))
}

impl St {
    fn waiting(&self) -> impl Iterator<Item = &Ticket> {
        self.tickets.iter().filter(|t| t.live() && t.state == WAITING)
    }

    fn waiting_count(&self) -> usize {
        self.waiting().count()
    }

    fn of_guest(&self, id: &str) -> Option<&Ticket> {
        self.tickets.iter().find(|t| t.guest == id)
    }

    fn find_mut(&mut self, number: u64) -> Option<&mut Ticket> {
        self.tickets.iter_mut().find(|t| t.number == number && !t.closed)
    }

    fn avg_minutes(&self) -> Option<i64> {
        if self.calls_today.len() < AVG_AFTER_CALLS || self.gaps_today.is_empty() {
            return None;
        }
        let mean = self.gaps_today.iter().sum::<i64>() as f64 / self.gaps_today.len() as f64;
        Some(((mean / 60.0).round() as i64).max(1))
    }

    fn avg_total_minutes(&self) -> Option<i64> {
        (self.gaps_total >= (AVG_AFTER_CALLS as u64 - 1)).then(|| ((self.gap_secs_total as f64 / self.gaps_total as f64 / 60.0).round() as i64).max(1))
    }

    fn longest_wait_minutes(&self, now: DateTime<Utc>) -> Option<i64> {
        self.waiting().map(|t| (now - t.taken_at).num_minutes().max(0)).max()
    }

    /// Starts a new day's counts when the box's date has moved on. `true` when it did.
    fn roll_day(&mut self, now: DateTime<Utc>) -> bool {
        let today = local_day(now);
        if self.day == Some(today) {
            return false;
        }
        self.day = Some(today);
        self.days += 1;
        self.calls_today.clear();
        self.gaps_today.clear();
        self.last_call_at = None;
        self.served_today = 0;
        self.skipped_today = 0;
        self.left_today = 0;
        true
    }

    fn set_closes_at(&mut self, now: DateTime<Utc>) {
        self.closes_at = match (&self.state[..], self.hours_until.as_deref().and_then(parse_hm)) {
            (OPEN | PAUSED, Some(hm)) => Some(next_local(now, hm)),
            _ => None,
        };
    }

    /// One call of ticket `i`: it is `called` now and today's numbers count it.
    fn call(&mut self, i: usize, now: DateTime<Utc>) {
        let t = &mut self.tickets[i];
        t.state = CALLED.to_string();
        t.called_at = Some(now);
        t.done_at = None;
        let (number, waited) = (t.number, (now - t.taken_at).num_seconds().max(0));
        self.now_serving = Some(number);
        self.calls_today.push(waited);
        if let Some(last) = self.last_call_at {
            let gap = (now - last).num_seconds().max(0);
            self.gaps_today.push(gap);
            self.gap_secs_total += gap;
            self.gaps_total += 1;
        }
        self.last_call_at = Some(now);
    }

    fn serve(&mut self, number: u64, now: DateTime<Utc>) -> Result<(), KitError> {
        let t = self.find_mut(number).ok_or_else(no_such_ticket)?;
        if !t.live() {
            return Err(KitError::NotNow(format!("Ticket #{number} isn't waiting or called.")));
        }
        t.finish(SERVED, now);
        self.served_today += 1;
        self.served_total += 1;
        Ok(())
    }

    /// Closes the queue: every waiting or called ticket (and a skipped one the host could still
    /// bring back) loses its name and is kept only to tell its guest, until the sweep.
    fn close(&mut self, text: Option<String>, auto: bool, now: DateTime<Utc>) {
        let mut still = 0;
        for t in self.tickets.iter_mut().filter(|t| !t.done()) {
            if t.live() {
                still += 1;
            }
            t.closed = true;
            t.name = None;
            t.done_at = Some(now);
        }
        self.state = CLOSED.to_string();
        self.close_text = Some(text.unwrap_or_else(|| DEFAULT_CLOSE_TEXT.to_string()));
        self.closed_at = Some(now);
        self.closed_waiting = still;
        self.auto_closed = auto;
        self.pause_text = None;
        self.paused_at = None;
        self.closes_at = None;
        self.last_call_at = None;
    }

    fn my_ticket(&self, t: &Ticket, spec: &Value) -> Value {
        let place = (t.live() && t.state == WAITING).then(|| self.waiting().filter(|w| w.number < t.number).count() as u64 + 1);
        let call_ends_at = (t.live() && t.state == CALLED).then(|| t.called_at.map(|c| c + Duration::minutes(call_minutes(spec)))).flatten();
        json!({
            "number": t.number,
            "name": t.name,
            "place": place,
            "ahead": place.map(|p| p - 1),
            // The design shows a removed ticket to its guest as "You left the queue"
            "state": if t.state == REMOVED { LEFT } else { t.state.as_str() },
            "taken_at": t.taken_at,
            "called_at": t.called_at,
            "call_ends_at": call_ends_at,
            "done_at": t.done_at,
            "closed": t.closed,
        })
    }

    /// The facts anyone at the counter may see: numbers only, never a name.
    fn public(&self, spec: &Value, now: DateTime<Utc>) -> Value {
        json!({
            "state": self.state,
            "title": spec_str(spec, "title").unwrap_or_default(),
            "line": spec_str(spec, "line"),
            "hours_until": self.hours_until,
            "now_serving": self.now_serving,
            "waiting": self.waiting_count(),
            "avg_minutes": self.avg_minutes(),
            "call_minutes": call_minutes(spec),
            "max_waiting": max_waiting(spec),
            "ask_name": ask_name(spec),
            "host_name": spec_str(spec, "host_name"),
            "pause_text": self.pause_text,
            "close_text": self.close_text,
            "paused_at": self.paused_at,
            "now": now,
        })
    }

    fn host_summary_line(&self) -> String {
        let days = if self.days == 1 { "1 day".to_string() } else { format!("{} days", self.days) };
        match self.avg_total_minutes() {
            Some(min) => format!("{days} · {} served · average {min} min", self.served_total),
            None => format!("{days} · {} served", self.served_total),
        }
    }
}

/// `true` when this guest holds a ticket that is waiting or called (the take limiter lets them
/// through to the kit's `has_ticket` answer instead of a 429).
pub fn has_live_ticket(state: &Value, player_id: &str) -> bool {
    load(state).of_guest(player_id).is_some_and(Ticket::live)
}

impl Queue {
    fn take(ctx: &KitCtx, s: &mut St, guest: &str, body: &Value) -> Result<(), KitError> {
        match s.state.as_str() {
            ENDED => return Err(ended_error()),
            CLOSED => return Err(closed_error(s)),
            _ => {}
        }
        if let Some(t) = s.of_guest(guest).filter(|t| t.live()) {
            let ticket = s.my_ticket(t, ctx.spec);
            return Err(coded(409, "has_ticket", "You already have a ticket.", json!({ "ticket": ticket })));
        }
        if s.state == PAUSED {
            return Err(paused_error(s));
        }
        let max = max_waiting(ctx.spec);
        if s.waiting_count() as i64 >= max {
            return Err(coded(409, "full", "The queue is full right now.", json!({ "max_waiting": max })));
        }
        // A first name only when the spec asks for one; otherwise nothing sent is kept
        let name = match body_text(body, "name", NAME_MAX)? {
            Some(n) if ask_name(ctx.spec) => {
                if words::blocked(&n) {
                    return Err(KitError::BadInput("Pick another name.".into()));
                }
                words::first_name_only(&n).map_err(|e| KitError::BadInput(e.message().into()))?;
                Some(n)
            }
            _ => None,
        };
        // An old finished or skipped ticket of this guest makes way for the new one
        s.tickets.retain(|t| t.guest != guest);
        s.counter += 1;
        s.tickets.push(Ticket {
            number: s.counter,
            name,
            state: WAITING.to_string(),
            taken_at: ctx.now,
            called_at: None,
            done_at: None,
            guest: guest.to_string(),
            closed: false,
        });
        Ok(())
    }

    fn host(ctx: &KitCtx, s: &mut St, name: &str, body: &Value) -> Result<(), KitError> {
        let now = ctx.now;
        if s.state == ENDED {
            return Err(ended_error());
        }
        match name {
            "call_next" => {
                let served = ticket_no(body, "served")?;
                match s.state.as_str() {
                    CLOSED => return Err(closed_error(s)),
                    PAUSED => return Err(paused_error(s)),
                    _ => {}
                }
                let mut did_serve = false;
                if let Some(n) = served {
                    if s.tickets.iter().any(|t| t.number == n && t.live() && t.state == CALLED) {
                        s.serve(n, now)?;
                        did_serve = true;
                    }
                }
                let next = s.tickets.iter().enumerate().filter(|(_, t)| t.live() && t.state == WAITING).min_by_key(|(_, t)| t.number).map(|(i, _)| i);
                match next {
                    Some(i) => s.call(i, now),
                    // "Served" with nobody left to call still counts
                    None if did_serve => {}
                    None => return Err(coded(409, "empty", "Nobody is waiting.", json!({}))),
                }
            }
            "serve" => s.serve(required_ticket(body)?, now)?,
            "skip" => {
                let n = required_ticket(body)?;
                let t = s.find_mut(n).ok_or_else(no_such_ticket)?;
                if !(t.live() && t.state == CALLED) {
                    return Err(KitError::NotNow(format!("Ticket #{n} isn't called.")));
                }
                t.state = SKIPPED.to_string();
                t.done_at = Some(now);
                s.skipped_today += 1;
            }
            "recall" => {
                let n = required_ticket(body)?;
                if s.state == CLOSED {
                    return Err(closed_error(s));
                }
                let i = s.tickets.iter().position(|t| t.number == n && !t.closed).ok_or_else(no_such_ticket)?;
                let t = &s.tickets[i];
                if t.state != SKIPPED {
                    return Err(KitError::NotNow(format!("Ticket #{n} wasn't skipped.")));
                }
                if t.recall_until().is_none_or(|u| u <= now) {
                    return Err(coded(410, "too_late", "It's more than 10 minutes since that ticket was skipped.", json!({})));
                }
                s.skipped_today = s.skipped_today.saturating_sub(1);
                // Called again at once, ahead of everyone waiting
                let t = &mut s.tickets[i];
                t.state = CALLED.to_string();
                t.called_at = Some(now);
                t.done_at = None;
                s.now_serving = Some(n);
            }
            "remove" => {
                let n = required_ticket(body)?;
                let t = s.find_mut(n).ok_or_else(no_such_ticket)?;
                if !(t.live() && t.state == WAITING) {
                    return Err(KitError::NotNow(format!("Ticket #{n} isn't waiting.")));
                }
                t.finish(REMOVED, now);
                s.left_today += 1;
            }
            "pause" => {
                let text = body_text(body, "text", PAUSE_TEXT_MAX)?;
                if s.state != OPEN {
                    return Err(KitError::NotNow("Only an open queue can be paused.".into()));
                }
                s.state = PAUSED.to_string();
                s.pause_text = text;
                s.paused_at = Some(now);
                // The average measures turns, not the break
                s.last_call_at = None;
            }
            "resume" => {
                if s.state != PAUSED {
                    return Err(KitError::NotNow("The queue isn't paused.".into()));
                }
                s.state = OPEN.to_string();
                s.pause_text = None;
                s.paused_at = None;
            }
            "close" => {
                let text = body_text(body, "text", CLOSE_TEXT_MAX)?;
                if s.state == CLOSED {
                    return Err(KitError::NotNow("The queue is already closed.".into()));
                }
                s.close(text, false, now);
            }
            "reopen" => {
                if s.state != CLOSED {
                    return Err(KitError::NotNow("Only a closed queue can be opened again.".into()));
                }
                s.state = OPEN.to_string();
                s.counter = 0;
                s.now_serving = None;
                s.close_text = None;
                s.closed_at = None;
                s.auto_closed = false;
                s.closed_waiting = 0;
                // Closed tickets have told their guests by now; the sweep takes the guests
                s.tickets.retain(|t| !t.closed);
                s.roll_day(now);
                s.set_closes_at(now);
            }
            "end" => {
                s.state = ENDED.to_string();
                s.tickets.clear();
                s.pause_text = None;
                s.paused_at = None;
                s.closes_at = None;
                s.now_serving = None;
            }
            "set_hours" => {
                let until = match body.get("until") {
                    None | Some(Value::Null) => None,
                    Some(Value::String(v)) => Some(parse_hm(v).map(|_| v.trim().to_string()).ok_or_else(|| KitError::BadInput("until must be a time like 13:00.".into()))?),
                    Some(_) => return Err(KitError::BadInput("until must be a time like 13:00, or null.".into())),
                };
                s.hours_until = until;
                s.set_closes_at(now);
            }
            _ => return Err(KitError::BadInput("That action isn't one this pocket knows.".into())),
        }
        Ok(())
    }

    fn host_view(ctx: &KitCtx, s: &St) -> Value {
        let cm = call_minutes(ctx.spec);
        let mut shown: Vec<&Ticket> = s.tickets.iter().filter(|t| t.live() || t.recall_until().is_some_and(|u| u > ctx.now)).collect();
        shown.sort_by_key(|t| t.number);
        let tickets: Vec<Value> = shown
            .iter()
            .map(|t| {
                json!({
                    "number": t.number,
                    "name": t.name,
                    "state": t.state,
                    "taken_at": t.taken_at,
                    "called_at": t.called_at,
                    "call_ends_at": (t.state == CALLED).then(|| t.called_at.map(|c| c + Duration::minutes(cm))).flatten(),
                    "recall_until": t.recall_until(),
                })
            })
            .collect();
        json!({
            "spec": {
                "title": spec_str(ctx.spec, "title").unwrap_or_default(),
                "line": spec_str(ctx.spec, "line"),
                "hours": s.hours_until.as_ref().map(|u| json!({ "until": u })),
                "call_minutes": cm,
                "ask_name": ask_name(ctx.spec),
                "max_waiting": max_waiting(ctx.spec),
                "host_name": spec_str(ctx.spec, "host_name"),
            },
            "state": s.state,
            "pause_text": s.pause_text,
            "close_text": s.close_text,
            "paused_at": s.paused_at,
            "closed_at": s.closed_at,
            "auto_closed": s.auto_closed,
            "closed_waiting": s.closed_waiting,
            "hours_until": s.hours_until,
            "closes_at": s.closes_at,
            "now_serving": s.now_serving,
            "counter": s.counter,
            "next_number": s.counter + 1,
            "tickets": tickets,
            "summary": Self::summary(ctx, s),
            "now": ctx.now,
        })
    }

    fn summary(ctx: &KitCtx, s: &St) -> Value {
        json!({
            "served_today": s.served_today,
            "skipped_today": s.skipped_today,
            "left_today": s.left_today,
            "waiting": s.waiting_count(),
            "avg_minutes": s.avg_minutes(),
            "calls_today": s.calls_today,
            "longest_wait_minutes": s.longest_wait_minutes(ctx.now),
            "days": s.days,
            "served_total": s.served_total,
            "avg_total_minutes": s.avg_total_minutes(),
        })
    }
}

impl Kit for Queue {
    fn id(&self) -> &'static str {
        ID
    }

    fn rules(&self, spec: &Value) -> KitRules {
        KitRules {
            retention: Retention::AfterEnd { days: KEEP_DAYS_AFTER_END },
            needs_pin: false,
            max_players: max_waiting(spec) as usize + PLAYER_SLACK,
            guest_actions: GUEST_ACTIONS,
            host_actions: HOST_ACTIONS,
        }
    }

    fn validate_spec(&self, spec: &Value) -> Result<(), String> {
        if !spec.is_object() {
            return Err("The spec must be a JSON object.".into());
        }
        check_spec_text(spec, "title", TITLE_MAX, true)?;
        check_spec_text(spec, "line", LINE_MAX, false)?;
        check_spec_text(spec, "host_name", HOST_NAME_MAX, false)?;
        match spec.get("hours") {
            None | Some(Value::Null) => {}
            Some(h) => {
                let ok = h.as_object().is_some_and(|o| o.keys().all(|k| k == "until")) && h.get("until").and_then(Value::as_str).and_then(parse_hm).is_some();
                if !ok {
                    return Err("hours must be {\"until\": \"HH:MM\"} or null.".into());
                }
            }
        }
        check_spec_int(spec, "call_minutes", CALL_MINUTES_MIN, CALL_MINUTES_MAX)?;
        check_spec_int(spec, "max_waiting", MAX_WAITING_MIN, MAX_WAITING_MAX)?;
        match spec.get("ask_name") {
            None | Some(Value::Null) | Some(Value::Bool(_)) => Ok(()),
            _ => Err("ask_name must be true or false.".into()),
        }
    }

    fn init(&self, spec: &Value) -> Value {
        let now = Utc::now();
        let mut s = St { state: OPEN.to_string(), hours_until: spec_hours(spec), ..St::default() };
        s.roll_day(now);
        s.set_closes_at(now);
        serde_json::to_value(&s).unwrap_or(Value::Null)
    }

    fn act(&self, ctx: &mut KitCtx, state: &mut Value, actor: &Viewer, name: &str, body: &Value) -> Result<(), KitError> {
        let mut s = load(state);
        s.roll_day(ctx.now);
        match actor {
            Viewer::Guest(id) => match name {
                "take" => Self::take(ctx, &mut s, id, body)?,
                "leave" => {
                    if let Some(t) = s.tickets.iter_mut().find(|t| &t.guest == id && t.live()) {
                        t.finish(LEFT, ctx.now);
                        s.left_today += 1;
                    }
                }
                _ => return Err(KitError::BadInput("That action isn't one this pocket knows.".into())),
            },
            Viewer::Host => Self::host(ctx, &mut s, name, body)?,
            Viewer::BigScreen => return Err(KitError::BadInput("The big screen can't act.".into())),
        }
        store(state, &s);
        Ok(())
    }

    /// A called ticket past `call_minutes` is skipped; at `hours.until` the queue closes with the
    /// default text; a new day starts today's counts again.
    fn tick(&self, ctx: &mut KitCtx, state: &mut Value) -> bool {
        let mut s = load(state);
        if s.state == ENDED {
            return false;
        }
        let mut changed = s.roll_day(ctx.now);
        let limit = Duration::minutes(call_minutes(ctx.spec));
        let mut skipped = 0;
        for t in s.tickets.iter_mut().filter(|t| t.live() && t.state == CALLED) {
            if let Some(c) = t.called_at.filter(|c| *c + limit <= ctx.now) {
                t.state = SKIPPED.to_string();
                t.done_at = Some(c + limit);
                skipped += 1;
            }
        }
        if skipped > 0 {
            s.skipped_today += skipped;
            changed = true;
        }
        if matches!(s.state.as_str(), OPEN | PAUSED) && s.closes_at.is_some_and(|c| c <= ctx.now) {
            s.close(None, true, ctx.now);
            changed = true;
        }
        if changed {
            store(state, &s);
        }
        changed
    }

    fn ticks_unwatched(&self) -> bool {
        true
    }

    /// A printed counter sign must keep working, so no quick tunnel (a new address per start).
    fn stable_address_only(&self) -> bool {
        true
    }

    fn player_ceiling(&self, _spec: &Value) -> usize {
        PLAYER_CEILING
    }

    /// The design's `first_names_only: true`: the page says only a first name is ever asked
    /// for. The join itself takes no name ([`Kit::nameless_join`]); `take` checks the first name.
    fn first_names_only(&self, _spec: &Value) -> bool {
        true
    }

    fn nameless_join(&self) -> bool {
        true
    }

    fn view(&self, ctx: &KitCtx, state: &Value, viewer: &Viewer) -> Value {
        let s = load(state);
        match viewer {
            Viewer::Guest(id) => {
                let mut v = s.public(ctx.spec, ctx.now);
                v["my_ticket"] = s.of_guest(id).map(|t| s.my_ticket(t, ctx.spec)).unwrap_or(Value::Null);
                v
            }
            Viewer::Host => Self::host_view(ctx, &s),
            Viewer::BigScreen => {
                let mut next: Vec<u64> = s.waiting().map(|t| t.number).collect();
                next.sort_unstable();
                next.truncate(5);
                json!({
                    "state": s.state,
                    "title": spec_str(ctx.spec, "title").unwrap_or_default(),
                    "line": spec_str(ctx.spec, "line"),
                    "now_serving": s.now_serving,
                    "next": next,
                    "waiting": s.waiting_count(),
                    "avg_minutes": s.avg_minutes(),
                    "pause_text": s.pause_text,
                    "close_text": s.close_text,
                    "hours_until": s.hours_until,
                    "now": ctx.now,
                })
            }
        }
    }

    /// Before taking a ticket (`/info`, the page's boot without a key): the counter's numbers.
    fn lobby(&self, ctx: &KitCtx, state: &Value) -> Value {
        let mut v = load(state).public(ctx.spec, ctx.now);
        v["my_ticket"] = Value::Null;
        v
    }

    fn host_summary(&self, ctx: &KitCtx, state: &Value) -> Value {
        let s = load(state);
        let mut v = Self::summary(ctx, &s);
        v["state"] = json!(s.state);
        v["now_serving"] = json!(s.now_serving);
        v["called"] = json!(s.tickets.iter().filter(|t| t.live() && t.state == CALLED).count());
        v["needs_you"] = json!(false);
        v["summary"] = json!(s.host_summary_line());
        v
    }

    fn finished(&self, state: &Value) -> bool {
        load(state).state == ENDED
    }

    fn on_remove(&self, _ctx: &mut KitCtx, state: &mut Value, player_id: &str) {
        let mut s = load(state);
        let before = s.tickets.len();
        s.tickets.retain(|t| t.guest != player_id);
        if s.tickets.len() != before {
            store(state, &s);
        }
    }

    /// Guests whose ticket is finished (or skipped past the recall window), and guests who joined
    /// but hold no ticket: the sweep deletes their row, keys, events and ticket.
    fn idle_players(&self, ctx: &KitCtx, state: &Value) -> Vec<String> {
        let s = load(state);
        ctx.players
            .iter()
            .filter(|p| match s.of_guest(&p.id) {
                Some(t) => t.gone_at().is_some_and(|g| g <= ctx.now),
                None => p.joined_at + Duration::minutes(NO_TICKET_KEEP_MINUTES) <= ctx.now,
            })
            .map(|p| p.id.clone())
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pocket::kits::Player;

    fn at(min: i64) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339("2026-12-05T10:00:00Z").unwrap().with_timezone(&Utc) + Duration::minutes(min)
    }

    fn spec() -> Value {
        json!({ "title": "Tine's Bakery", "line": "Pick up at the side counter", "call_minutes": 3 })
    }

    fn players(ids: &[&str], joined: DateTime<Utc>) -> Vec<Player> {
        ids.iter().map(|id| Player { id: id.to_string(), name: String::new(), avatar: String::new(), team: None, joined_at: joined }).collect()
    }

    fn fresh(spec: &Value, now: DateTime<Utc>) -> Value {
        let mut s = St { state: OPEN.into(), hours_until: spec_hours(spec), ..St::default() };
        s.roll_day(now);
        s.set_closes_at(now);
        serde_json::to_value(&s).unwrap()
    }

    fn act(spec: &Value, st: &mut Value, now: DateTime<Utc>, who: &Viewer, name: &str, body: Value) -> Result<(), KitError> {
        let ps = players(&["a", "b", "c", "d"], at(0));
        let mut ctx = KitCtx::new(spec, &ps, now);
        Queue.act(&mut ctx, st, who, name, &body)
    }

    fn guest(id: &str) -> Viewer {
        Viewer::Guest(id.into())
    }

    fn code(e: KitError) -> (u16, String) {
        match e {
            KitError::Coded { status, code, .. } => (status, code.to_string()),
            KitError::BadInput(_) => (400, "bad_action".into()),
            KitError::NotNow(_) => (409, "not_now".into()),
        }
    }

    #[test]
    fn spec_checks() {
        assert!(Queue.validate_spec(&spec()).is_ok());
        assert!(Queue.validate_spec(&json!({ "title": "" })).is_err());
        assert!(Queue.validate_spec(&json!({ "title": "x".repeat(41) })).is_err());
        assert!(Queue.validate_spec(&json!({ "title": "A", "line": "y".repeat(81) })).is_err());
        assert!(Queue.validate_spec(&json!({ "title": "A\u{7}" })).is_err());
        assert!(Queue.validate_spec(&json!({ "title": "A", "call_minutes": 11 })).is_err());
        assert!(Queue.validate_spec(&json!({ "title": "A", "call_minutes": 2.5 })).is_err());
        assert!(Queue.validate_spec(&json!({ "title": "A", "max_waiting": 9 })).is_err());
        assert!(Queue.validate_spec(&json!({ "title": "A", "max_waiting": 200, "hours": { "until": "13:00" } })).is_ok());
        assert!(Queue.validate_spec(&json!({ "title": "A", "hours": { "until": "1pm" } })).is_err());
        assert!(Queue.validate_spec(&json!({ "title": "A", "hours": null, "ask_name": true })).is_ok());
        assert!(Queue.validate_spec(&json!({ "title": "A", "ask_name": "yes" })).is_err());
        assert_eq!(Queue.rules(&json!({ "title": "A", "max_waiting": 200 })).max_players, 240);
        assert!(Queue.stable_address_only() && Queue.nameless_join() && Queue.ticks_unwatched());
        assert!(Queue.first_names_only(&spec()));
    }

    #[test]
    fn numbers_places_and_call_next() {
        let sp = spec();
        let mut st = fresh(&sp, at(0));
        for (i, g) in ["a", "b", "c"].iter().enumerate() {
            act(&sp, &mut st, at(i as i64), &guest(g), "take", json!({})).unwrap();
        }
        let ps = players(&["a", "b", "c"], at(0));
        let ctx = KitCtx::new(&sp, &ps, at(5));
        let v = Queue.view(&ctx, &st, &guest("c"));
        assert_eq!((v["my_ticket"]["number"].as_u64(), v["my_ticket"]["place"].as_u64(), v["my_ticket"]["ahead"].as_u64()), (Some(3), Some(3), Some(2)));
        // Again: 409 has_ticket with the same ticket, no new number
        let e = act(&sp, &mut st, at(5), &guest("c"), "take", json!({})).unwrap_err();
        match e {
            KitError::Coded { status: 409, code: "has_ticket", extra, .. } => assert_eq!(extra["ticket"]["number"], 3),
            other => panic!("{other:?}"),
        }
        act(&sp, &mut st, at(6), &Viewer::Host, "call_next", json!({})).unwrap();
        act(&sp, &mut st, at(8), &Viewer::Host, "call_next", json!({ "served": 1 })).unwrap();
        let s = load(&st);
        assert_eq!((s.tickets[0].state.as_str(), s.tickets[1].state.as_str(), s.now_serving), (SERVED, CALLED, Some(2)));
        assert_eq!(s.served_today, 1);
        assert!(s.tickets[0].name.is_none());
        // One left waiting; the third call empties the queue; a fourth is 409 empty
        act(&sp, &mut st, at(9), &Viewer::Host, "call_next", json!({ "served": 2 })).unwrap();
        let e = act(&sp, &mut st, at(10), &Viewer::Host, "call_next", json!({})).unwrap_err();
        assert_eq!(code(e), (409, "empty".into()));
        // Served with nobody left still serves
        act(&sp, &mut st, at(10), &Viewer::Host, "call_next", json!({ "served": 3 })).unwrap();
        assert_eq!(load(&st).served_today, 3);
        assert_eq!(code(act(&sp, &mut st, at(10), &Viewer::Host, "serve", json!({ "ticket": "2" })).unwrap_err()).0, 400);
        assert_eq!(code(act(&sp, &mut st, at(10), &Viewer::Host, "serve", json!({ "ticket": 2.5 })).unwrap_err()).0, 400);
        assert_eq!(code(act(&sp, &mut st, at(10), &Viewer::Host, "serve", json!({ "ticket": 99 })).unwrap_err()).0, 404);
    }

    #[test]
    fn avg_only_after_three_calls() {
        let sp = spec();
        let mut st = fresh(&sp, at(0));
        for g in ["a", "b", "c", "d"] {
            act(&sp, &mut st, at(0), &guest(g), "take", json!({})).unwrap();
        }
        act(&sp, &mut st, at(1), &Viewer::Host, "call_next", json!({})).unwrap();
        act(&sp, &mut st, at(3), &Viewer::Host, "call_next", json!({ "served": 1 })).unwrap();
        assert_eq!(load(&st).avg_minutes(), None);
        act(&sp, &mut st, at(7), &Viewer::Host, "call_next", json!({ "served": 2 })).unwrap();
        let s = load(&st);
        assert_eq!(s.avg_minutes(), Some(3), "gaps of 2 and 4 minutes");
        assert_eq!(s.calls_today, vec![60, 180, 420]);
    }

    #[test]
    fn tick_skips_and_recall_window() {
        let sp = spec();
        let mut st = fresh(&sp, at(0));
        act(&sp, &mut st, at(0), &guest("a"), "take", json!({})).unwrap();
        act(&sp, &mut st, at(1), &Viewer::Host, "call_next", json!({})).unwrap();
        let ps = players(&["a"], at(0));
        let mut ctx = KitCtx::new(&sp, &ps, at(3));
        assert!(!Queue.tick(&mut ctx, &mut st), "2 of 3 minutes");
        let mut ctx = KitCtx::new(&sp, &ps, at(4));
        assert!(Queue.tick(&mut ctx, &mut st));
        assert_eq!(load(&st).tickets[0].state, SKIPPED);
        // Recall inside 10 minutes works; after it, 410 too_late
        act(&sp, &mut st, at(13), &Viewer::Host, "recall", json!({ "ticket": 1 })).unwrap();
        assert_eq!(load(&st).tickets[0].state, CALLED);
        act(&sp, &mut st, at(14), &Viewer::Host, "skip", json!({ "ticket": 1 })).unwrap();
        let e = act(&sp, &mut st, at(24), &Viewer::Host, "recall", json!({ "ticket": 1 })).unwrap_err();
        assert_eq!(code(e), (410, "too_late".into()));
        // The guest goes at the sweep after the window
        let ctx = KitCtx::new(&sp, &ps, at(23));
        assert!(Queue.idle_players(&ctx, &st).is_empty());
        let ctx = KitCtx::new(&sp, &ps, at(24));
        assert_eq!(Queue.idle_players(&ctx, &st), vec!["a".to_string()]);
    }

    #[test]
    fn close_wipes_names_and_reopen_starts_at_one() {
        let sp = json!({ "title": "A", "ask_name": true });
        let mut st = fresh(&sp, at(0));
        act(&sp, &mut st, at(0), &guest("a"), "take", json!({ "name": "Mia" })).unwrap();
        act(&sp, &mut st, at(0), &guest("b"), "take", json!({ "name": "Ben" })).unwrap();
        assert_eq!(code(act(&sp, &mut st, at(0), &guest("c"), "take", json!({ "name": "x".repeat(25) })).unwrap_err()).0, 400);
        assert_eq!(code(act(&sp, &mut st, at(0), &guest("c"), "take", json!({ "name": "Mi\u{0}a" })).unwrap_err()).0, 400);
        assert_eq!(code(act(&sp, &mut st, at(0), &guest("c"), "take", json!({ "name": "mia@example.com" })).unwrap_err()).0, 400);
        act(&sp, &mut st, at(1), &Viewer::Host, "close", json!({})).unwrap();
        let text = st.to_string();
        assert!(!text.contains("Mia") && !text.contains("Ben"), "{text}");
        assert_eq!(load(&st).close_text.as_deref(), Some(DEFAULT_CLOSE_TEXT));
        assert_eq!(code(act(&sp, &mut st, at(1), &guest("c"), "take", json!({})).unwrap_err()), (410, "closed".into()));
        act(&sp, &mut st, at(2), &Viewer::Host, "reopen", json!({})).unwrap();
        act(&sp, &mut st, at(2), &guest("c"), "take", json!({})).unwrap();
        assert_eq!(load(&st).tickets.iter().map(|t| t.number).collect::<Vec<_>>(), vec![1]);
    }

    #[test]
    fn paused_full_and_names_off() {
        let sp = json!({ "title": "A", "max_waiting": 10 });
        let mut st = fresh(&sp, at(0));
        // ask_name off: a name sent is not kept
        act(&sp, &mut st, at(0), &guest("a"), "take", json!({ "name": "Mia" })).unwrap();
        assert!(!st.to_string().contains("Mia"));
        act(&sp, &mut st, at(0), &Viewer::Host, "pause", json!({ "text": "Back in 10 minutes" })).unwrap();
        match act(&sp, &mut st, at(0), &guest("b"), "take", json!({})).unwrap_err() {
            KitError::Coded { status: 409, code: "paused", extra, .. } => assert_eq!(extra["pause_text"], "Back in 10 minutes"),
            other => panic!("{other:?}"),
        }
        assert_eq!(code(act(&sp, &mut st, at(0), &Viewer::Host, "pause", json!({ "text": "x".repeat(61) })).unwrap_err()).0, 400);
        act(&sp, &mut st, at(0), &Viewer::Host, "resume", json!({})).unwrap();
        let mut s = load(&st);
        for i in 0..9 {
            s.counter += 1;
            s.tickets.push(Ticket { number: s.counter, name: None, state: WAITING.into(), taken_at: at(0), called_at: None, done_at: None, guest: format!("g{i}"), closed: false });
        }
        store(&mut st, &s);
        match act(&sp, &mut st, at(0), &guest("b"), "take", json!({})).unwrap_err() {
            KitError::Coded { status: 409, code: "full", extra, .. } => assert_eq!(extra["max_waiting"], 10),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn views_hold_only_your_own_ticket_and_screens_no_names() {
        let sp = json!({ "title": "A", "ask_name": true });
        let mut st = fresh(&sp, at(0));
        act(&sp, &mut st, at(0), &guest("a"), "take", json!({ "name": "Mia" })).unwrap();
        act(&sp, &mut st, at(0), &guest("b"), "take", json!({ "name": "Ben" })).unwrap();
        let ps = players(&["a", "b"], at(0));
        let ctx = KitCtx::new(&sp, &ps, at(1));
        let a = Queue.view(&ctx, &st, &guest("a")).to_string();
        assert!(a.contains("Mia") && !a.contains("Ben") && !a.contains("\"b\""), "{a}");
        for v in [Queue.view(&ctx, &st, &Viewer::BigScreen), Queue.lobby(&ctx, &st), Queue.host_summary(&ctx, &st)] {
            let t = v.to_string();
            assert!(!t.contains("Mia") && !t.contains("Ben") && !t.contains("\"guest\""), "{t}");
        }
        let h = Queue.view(&ctx, &st, &Viewer::Host).to_string();
        assert!(h.contains("Mia") && h.contains("Ben") && !h.contains("\"guest\""), "the host sees names, never keys: {h}");
    }

    #[test]
    fn the_done_row_line() {
        let mut s = St { days: 1, served_total: 5, ..St::default() };
        assert_eq!(s.host_summary_line(), "1 day · 5 served");
        s.days = 3;
        s.gaps_total = 4;
        s.gap_secs_total = 4 * 190;
        assert_eq!(s.host_summary_line(), "3 days · 5 served · average 3 min");
    }
}
