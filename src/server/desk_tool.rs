//! desk-create-from-chat (autonomy/desk-create-from-chat/brief.md): the bot's front desk tool,
//! `desk_create`, offered beside `santa_create` and behind the same switch (`pocket_apps`, on or
//! naming `desk`). Before it, "Set up a front desk for my guesthouse…" or "Let people book a
//! 30-minute haircut, Tue-Sat 10-6" ended with the bot saying it couldn't, and a desk made through
//! `POST /api/pockets` never showed its card in chat (Mira's ux-pass-front-desk fix #1, and the
//! server half of #2).
//!
//! - With no desk in the arguments (no `title`) it answers with the fields and rules ([`RULES`]).
//! - With one it builds the spec from the fields it knows (never `answer_bot`: the desk's bot is
//!   always the calling bot), expands a booking desk's `fill` rule into times the way the card's
//!   QuickFill does ([`expand_fill`]), checks the times with the kit's own `parse_slots` (its
//!   refusal comes back as `bad_slots` with the slot's index, why and the kit's words), and makes
//!   the pocket with `pocket::create`: kit `desk`, owned by the calling bot, with the turn's own
//!   chat as its origin (never one named in the arguments).
//! - Its answer starts with [`CARD_TEXT_HEAD`] and the pocket's code. The reply that carries the
//!   call gets a `pocket` block with a `desk` part ([`card`]), built by `smart_results::pocket` from
//!   the stored pocket, never from the call's text. The part holds counts, first names and booking
//!   times; never a guest's words, a PIN or a key ([`card`] drops the guest-typed fields).
//!
//! Each bot reaches only the desks it made (the card is attached only to the owner's own replies).

use super::continuity::SelfScope;
use super::quizmaster_tools::PocketOrigin;
use super::AppState;
use crate::pocket::kits::{self, KitCtx, Player, Viewer};
use crate::pocket::schedule;
use crate::pocket::{desk, CreateRequest};
use chrono::{Datelike, Duration, NaiveDate, NaiveDateTime, NaiveTime, Timelike, Utc, Weekday};
use serde_json::{json, Map, Value};

type ToolResult = (String, bool);

pub const TOOL: &str = "desk_create";
/// What a made desk's answer starts with, then its code and `]`.
pub const CARD_TEXT_HEAD: &str = "Pocket app · Front desk [";
/// The largest set of arguments a bot may hand in (a guide of 12,000 characters plus 200 times).
const MAX_ARGS_BYTES: usize = 64 * 1024;
/// The most weeks one `fill` may cover.
pub const FILL_WEEKS_MAX: u32 = 4;
/// The fields `desk_create` passes on to the kit; anything else in the arguments is dropped.
const FIELDS: [&str; 9] = ["mode", "title", "place", "host_name", "what", "guide", "starters", "daily_cap", "slots"];

pub fn handles(name: &str) -> bool {
    name == TOOL
}

/// Whether the front desk is on: the switch is `on` or names `desk` (pocket-switch-santa-alone).
pub async fn enabled(state: &AppState) -> bool {
    crate::pocket::enabled_for(&state.db, desk::ID).await
}

pub fn tool() -> Value {
    json!({
        "name": TOOL,
        "description": "Set up a front desk: a guest page where your bot answers guests' questions from the facts the person gives \
            (a guest guide for a guesthouse, Airbnb or holiday flat: check-in, Wi-Fi, parking, house rules), or a booking desk where \
            people ask for a time (appointments: a haircut, a lesson, a table) and the person confirms each one. Guests open it on their \
            phones; the person sees every conversation and answers what the bot can't. Call it first with no arguments to get the fields \
            and rules, then with the whole desk: it checks it, opens the desk and shows its card, with the guest link, in the person's chat. \
            Never invent a Wi-Fi password, door code or price, and never promise a time or a late check-out yourself.",
        "inputSchema": {"type": "object", "properties": {
            "mode": {"type": "string", "enum": ["guide", "booking"], "description": "guide (answers questions) or booking (guests ask for a time)"},
            "title": {"type": "string", "description": "What guests see, up to 80 characters"},
            "place": {"type": "string", "description": "The place's name, up to 60 characters"},
            "host_name": {"type": "string", "description": "The person's first name, up to 40 characters"},
            "what": {"type": "string", "description": "Booking desk: what guests book, like \"30-minute haircut\" (up to 40 characters)"},
            "guide": {"type": "string", "description": "The facts the bot answers from, in the person's own words"},
            "starters": {"type": "array", "items": {"type": "string"}, "description": "Up to 3 short questions guests can tap"},
            "daily_cap": {"type": "integer", "description": "Most bot replies a day, 10 to 200 (default 50)"},
            "slots": {"type": "array", "items": {"type": "object"}, "description": "Booking desk: the times, each {start: \"YYYY-MM-DDTHH:MM\", minutes, seats}"},
            "fill": {"type": "object", "description": "Booking desk: a weekly rule kwant turns into times, {days: [\"tue\", …], from: \"10:00\", to: \"18:00\", minutes: 30, seats: 1, weeks: 1}"}
        }},
    })
}

/// What a call with no desk answers: the fields and rules, and two samples.
pub const RULES: &str = "Front desk: the fields and rules.

kwant runs the desk: guests open its page on their phones and write to your bot there, one private conversation each. You answer them \
from the guide and nothing else. When the guide doesn't say, or a guest asks for something only the person can give, the desk hands the \
conversation to the person. The person sees every conversation, the count of replies today and, on a booking desk, every request; only the \
person confirms a booking.

Two kinds (mode):
- guide: a guest guide for a guesthouse, Airbnb, holiday flat or event. Needs guide.
- booking: people ask for a time (appointments, lessons, a table). Needs the times, as slots or as fill; guide is optional.

Fields (all at the top level of the call):
- mode (\"guide\" or \"booking\", default \"guide\").
- title (required, up to 80 characters): what guests see, like \"Sea View Guesthouse\" or \"Book a haircut\".
- place (optional, up to 60 characters): the place's name.
- host_name (optional, up to 40 characters): the person's first name.
- what (booking, optional, up to 40 characters): what guests book, like \"30-minute haircut\".
- guide (required for guide, up to 12,000 characters): the facts you answer from, one per line, in the person's own words. Write only \
what the person said. Where they said \"ask me\", write that it is a question for the host.
- starters (optional): up to 3 short questions guests can tap, up to 24 characters each, like \"Wi-Fi?\".
- daily_cap (optional, 10 to 200, default 50): the most bot replies a day across all guests. Past it, guests wait for the person.
- slots (booking): the times, each {\"start\": \"YYYY-MM-DDTHH:MM\" in the person's local time, \"minutes\": 5 to 1440, \"seats\": 1 to 60}. \
No two may overlap; up to 200.
- fill (booking, instead of slots): a weekly rule, {\"days\": [\"tue\", \"wed\", \"thu\", \"fri\", \"sat\"], \"from\": \"10:00\", \"to\": \"18:00\", \
\"minutes\": 30, \"seats\": 1, \"weeks\": 1}. kwant makes the times from today on: from `from`, one every `minutes`, the last ending by `to`, \
never one already past. weeks is 1 to 4 (default 1).

Rules:
- Ask the person at most one question before you call, and only about daily_cap if they didn't say it. Fill everything else from what \
they said: make up the title, use 50 replies a day if they don't answer.
- Never invent a Wi-Fi password, a door code, an address, a price or an opening time. Put in the guide only what the person told you.
- Never promise a late check-out, an early check-in, a discount or a time. Those go to the person.
- Money is always a hand-off: you never take a payment, quote a price the guide doesn't give, or promise a refund.
- After the desk opens, keep your reply short: its card in the chat already shows the guest link and the counts.

Sample guide desk (its facts are samples; write the person's own):
{\"mode\": \"guide\", \"title\": \"Sea View Guesthouse\", \"host_name\": \"Nika\", \"guide\": \"Check-in is from 3 PM.\\nThe Wi-Fi password is on the fridge.\\nLate check-out: ask the host.\", \"starters\": [\"Check-in time?\", \"Wi-Fi?\"], \"daily_cap\": 50}

Sample booking desk:
{\"mode\": \"booking\", \"title\": \"Book a haircut\", \"what\": \"30-minute haircut\", \"fill\": {\"days\": [\"tue\", \"wed\", \"thu\", \"fri\", \"sat\"], \"from\": \"10:00\", \"to\": \"18:00\", \"minutes\": 30, \"seats\": 1}}

Call desk_create again with the whole desk.";

fn fail(msg: impl Into<String>) -> ToolResult {
    (msg.into(), true)
}

/// The desk the arguments hold: the arguments themselves, or a `spec` object (or a string
/// holding one) when the bot wrapped them. `None` when it holds no desk yet (no `title`): then
/// the call asks for the rules.
pub(crate) fn desk_arg(args: &Value) -> Result<Option<Value>, String> {
    let v = match args.get("spec") {
        None | Some(Value::Null) => args.clone(),
        Some(Value::String(s)) => serde_json::from_str::<Value>(s).map_err(|e| format!("`spec` isn't valid JSON: {e}"))?,
        Some(v) => v.clone(),
    };
    if !v.is_object() {
        return Err("The desk must be a JSON object.".into());
    }
    if v.to_string().len() > MAX_ARGS_BYTES {
        return Err(format!("The desk is bigger than {} KB.", MAX_ARGS_BYTES / 1024));
    }
    let has_title = v.get("title").and_then(Value::as_str).is_some_and(|t| !t.trim().is_empty());
    Ok(has_title.then_some(v))
}

/// Whether a call's arguments hold a desk (what a made-this-turn card looks for).
pub(crate) fn holds_desk(args: Option<&Value>) -> bool {
    args.is_some_and(|a| matches!(desk_arg(a), Ok(Some(_))))
}

/// A `bad_slots` refusal: the kit's (or the fill rule's) words, with where and why.
fn bad_slots(index: Option<u64>, why: &str, message: &str) -> String {
    let at = index.map(|i| format!(", index: {i}")).unwrap_or_default();
    format!("Nothing was made (bad_slots, why: {why:?}{at}): {message} Fix the times and call desk_create again.")
}

/// The spec `desk_create` hands the kit, from `desk` (the known fields only), with a `fill` rule
/// expanded into `slots` from `now` (the box's local time, the clock the kit reads slots in).
/// `Err` is the bot's answer.
pub(crate) fn build_spec(desk: &Value, now: NaiveDateTime) -> Result<Value, String> {
    let mut spec = Map::new();
    for f in FIELDS {
        if let Some(v) = desk.get(f).filter(|v| !v.is_null()) {
            spec.insert(f.into(), v.clone());
        }
    }
    let booking = desk.get("mode").and_then(Value::as_str) == Some(desk::MODE_BOOKING);
    let fill = desk.get("fill").filter(|v| !v.is_null());
    if !booking {
        if fill.is_some() || spec.contains_key("slots") {
            return Err("Nothing was made: times (slots or fill) are for a booking desk. Set mode to \"booking\", or leave the times out, \
                        and call desk_create again."
                .into());
        }
        return Ok(Value::Object(spec));
    }
    match (fill, spec.contains_key("slots")) {
        (Some(_), true) => return Err(bad_slots(None, "fill", "Give the times as slots or as fill, not both.")),
        (Some(rule), false) => {
            let slots = expand_fill(rule, now).map_err(|m| bad_slots(None, "fill", &m))?;
            spec.insert("slots".into(), Value::Array(slots));
        }
        (None, true) => {}
        (None, false) => return Err(bad_slots(None, "list", "A booking desk needs its times: give slots or fill.")),
    }
    let spec = Value::Object(spec);
    // The kit's own check, with its index and words
    if let Err(kits::KitError::Coded { message, extra, .. }) = desk::parse_slots(spec.get("slots"), false) {
        return Err(bad_slots(extra["index"].as_u64(), extra["why"].as_str().unwrap_or("slots"), &message));
    }
    Ok(spec)
}

fn weekday(name: &str) -> Option<Weekday> {
    let n = name.trim().to_ascii_lowercase();
    let n = n.trim_end_matches('.');
    [
        (Weekday::Mon, "monday"),
        (Weekday::Tue, "tuesday"),
        (Weekday::Wed, "wednesday"),
        (Weekday::Thu, "thursday"),
        (Weekday::Fri, "friday"),
        (Weekday::Sat, "saturday"),
        (Weekday::Sun, "sunday"),
    ]
    .into_iter()
    .find(|(_, full)| n.len() >= 2 && full.starts_with(n))
    .map(|(d, _)| d)
}

fn clock(v: Option<&Value>, field: &str) -> Result<NaiveTime, String> {
    let t = v.and_then(Value::as_str).map(str::trim).unwrap_or("");
    NaiveTime::parse_from_str(t, "%H:%M").map_err(|_| format!("fill.{field} must be a time written HH:MM, like \"10:00\"."))
}

fn whole(v: Option<&Value>, field: &str, default: i64, lo: i64, hi: i64) -> Result<i64, String> {
    match v {
        None | Some(Value::Null) => Ok(default),
        Some(n) => n.as_i64().filter(|n| (lo..=hi).contains(n)).ok_or_else(|| format!("fill.{field} must be a whole number from {lo} to {hi}.")),
    }
}

/// A `fill` rule's times, the way the card's QuickFill makes a week (`fillWeek` in
/// frontend/src/components/results/deskBooking.ts): on each named day from today on for `weeks`
/// weeks, from `from`, one every `minutes`, the last ending by `to`, none at or before `now`.
/// The list may be longer than the kit takes; `parse_slots` refuses that with its own words.
pub(crate) fn expand_fill(rule: &Value, now: NaiveDateTime) -> Result<Vec<Value>, String> {
    if !rule.is_object() {
        return Err("fill must be an object like {\"days\": [\"tue\"], \"from\": \"10:00\", \"to\": \"18:00\", \"minutes\": 30}.".into());
    }
    let names = rule.get("days").and_then(Value::as_array).filter(|d| !d.is_empty()).ok_or("fill.days must list the days, like [\"tue\", \"sat\"].")?;
    let mut days = Vec::new();
    for d in names {
        let day = d.as_str().and_then(weekday).ok_or_else(|| format!("fill.days: {d} isn't a day of the week."))?;
        if !days.contains(&day) {
            days.push(day);
        }
    }
    let from = clock(rule.get("from"), "from")?;
    let to = clock(rule.get("to"), "to")?;
    if to <= from {
        return Err("fill.to must be later than fill.from on the same day.".into());
    }
    let minutes = whole(rule.get("minutes"), "minutes", 60, desk::SLOT_MINUTES_MIN, desk::SLOT_MINUTES_MAX)?;
    let seats = whole(rule.get("seats"), "seats", 1, 1, desk::SLOT_SEATS_MAX as i64)?;
    let weeks = whole(rule.get("weeks"), "weeks", 1, 1, FILL_WEEKS_MAX as i64)?;
    // Whole minutes since midnight: a NaiveTime wraps at midnight, so 23:30 + 60 would read as
    // 00:30, "before" `to`, and the loop below would never end.
    let (from_m, to_m) = (i64::from(from.num_seconds_from_midnight() / 60), i64::from(to.num_seconds_from_midnight() / 60));
    if from_m + minutes > to_m {
        return Err(format!("fill: a {minutes}-minute time doesn't fit between {} and {}.", from.format("%H:%M"), to.format("%H:%M")));
    }
    let today: NaiveDate = now.date();
    let mut out = Vec::new();
    for i in 0..(7 * weeks) {
        let day = today + Duration::days(i);
        if !days.contains(&day.weekday()) {
            continue;
        }
        let mut t = from_m;
        while t + minutes <= to_m {
            let start = day.and_time(NaiveTime::MIN) + Duration::minutes(t);
            if start > now {
                out.push(json!({ "start": start.format(desk::SLOT_START_FORMAT).to_string(), "minutes": minutes, "seats": seats }));
            }
            t += minutes;
        }
    }
    if out.is_empty() {
        return Err("fill made no times: every one is already past. Add weeks or other days.".into());
    }
    Ok(out)
}

pub async fn call(state: &AppState, scope: &SelfScope, args: &Value) -> ToolResult {
    if !enabled(state).await {
        return fail("Pocket apps are off on this kwant.");
    }
    let desk_in = match desk_arg(args) {
        Ok(Some(d)) => d,
        Ok(None) => return (RULES.to_string(), false),
        Err(e) => return fail(format!("Nothing was made: {e} Call desk_create again with the desk.")),
    };
    if state.db.get_kwant(&scope.kwant_id).await.ok().flatten().is_none() {
        return fail("Could not find your space.");
    }
    let spec = match build_spec(&desk_in, chrono::Local::now().naive_local()) {
        Ok(s) => s,
        Err(e) => return fail(e),
    };
    let o = PocketOrigin::of(scope);
    let req = CreateRequest {
        kit: desk::ID.into(),
        spec,
        owner_kwant_id: Some(o.kwant_id.clone()),
        opens_at: None,
        closes_at: None,
        origin: Some(schedule::Origin { kwant_id: o.kwant_id, channel_id: o.channel_id, thread_id: o.thread_id }),
    };
    // The kit's refusal is a plain sentence ("Write the house guide: the facts the bot answers from.")
    let (snap, _host_key) = match crate::pocket::create(state, req).await {
        Ok(s) => s,
        Err(e) if e.code == "bad_slots" => return fail(bad_slots(e.extra["index"].as_u64(), e.extra["why"].as_str().unwrap_or("slots"), &e.message)),
        Err(e) => return fail(format!("Nothing was made: {} Fix the desk and call desk_create again.", e.message)),
    };
    let join = super::santa_tool::santa_join(&crate::pocket::address::join(state, &snap).await);
    super::santa_tool::remember_join(&snap.row.code, join.clone());
    let booking = desk::is_booking(&snap.spec);
    let times = if booking { desk::parse_slots(snap.spec.get("slots"), false).map(|s| s.len()).unwrap_or(0) } else { 0 };
    (made_text(&snap.row.code, &snap.row.title, booking, times, desk::daily_cap(&snap.spec), join.as_ref()), false)
}

/// The answer to a call that made one: the head the chat card is found by, then what the bot
/// tells the person.
fn made_text(code: &str, title: &str, booking: bool, times: usize, cap: u64, join: Option<&Value>) -> String {
    let link = join.and_then(|j| j["anywhere_url"].as_str().or(j["wifi_url"].as_str()));
    let link_line = match (link, join.map(|j| j["mode"].as_str() == Some("wifi"))) {
        (Some(u), Some(true)) => format!(
            "Guest link: {u}\nIt works only on the same Wi-Fi as this computer; the card shows how to let guests open it from anywhere."
        ),
        (Some(u), _) => format!("Guest link: {u}"),
        (None, _) => "No guest link works yet: the card shows the person how to set one up. Don't write a link yourself.".into(),
    };
    let kind = if booking { format!("Booking desk with {times} times") } else { "Guest guide".into() };
    format!(
        "{CARD_TEXT_HEAD}{code}] \"{title}\" is open. {kind}, up to {cap} bot replies a day.\n\
         The person's chat already shows its card (the guest link, the guests and the replies today), so keep your reply to two or three \
         lines and don't repeat what's on it.\n\
         {link_line}\n\
         Guests' questions are answered from the guide only; anything it doesn't say goes to the person.{booking_line}\n\
         Never invent a Wi-Fi password, door code or price, and never promise a time or a late check-out yourself.",
        booking_line = if booking { " Only the person confirms a booking." } else { "" },
    )
}

// ---- The card ----------------------------------------------------------------------------

/// What the card is built from: the stored pocket.
pub(crate) struct CardInput<'a> {
    pub spec: &'a Value,
    pub kit_state: &'a Value,
    pub players: &'a [Player],
    pub now: chrono::DateTime<Utc>,
    /// The answering bot's model, for the card's cost line.
    pub model: Option<String>,
    pub join: Option<Value>,
}

/// The longest guide preview the card takes (`validDesk` `guide_preview`).
const PREVIEW_MAX: usize = 600;

/// Text cut to `max` UTF-16 units (the client's length).
fn cut_units(s: &str, max: usize) -> String {
    let mut out = String::new();
    let mut units = 0;
    for c in s.chars() {
        units += c.len_utf16();
        if units > max {
            break;
        }
        out.push(c);
    }
    out
}

/// The block's `desk` (frontend `validDesk`): the kit's host summary, the face (`title`,
/// `place`, `host_name`), the guide's preview and the desk's own `join`. A guest's words never
/// ride along: each guest row's `last` and hand-off reason and each booking's note are dropped
/// (the card reads them live from the host API), and nothing carries a PIN or key.
pub(crate) fn card(i: &CardInput) -> Value {
    let kit = kits::kit(desk::ID).expect("the desk kit is built in");
    let ctx = KitCtx::new(i.spec, i.players, i.now);
    let mut d = kit.host_summary(&ctx, i.kit_state);
    let face = kit.view(&ctx, i.kit_state, &Viewer::BigScreen);
    for k in ["title", "place", "host_name", "what"] {
        let v = face[k].as_str().map(str::trim).filter(|s| !s.is_empty()).map(|s| json!(s)).unwrap_or(Value::Null);
        d[k] = v;
    }
    d["mode"] = face["mode"].clone();
    d["answer_bot"] = json!({ "name": desk::bot_label(i.spec), "model": i.model });
    let guide = desk::guide(i.spec).trim();
    d["guide_preview"] = if guide.is_empty() { Value::Null } else { json!(cut_units(guide, PREVIEW_MAX)) };
    if let Some(gs) = d["guests"].as_array_mut() {
        for g in gs.iter_mut() {
            g["last"] = Value::Null;
            g["handoff_reason"] = Value::Null;
            if let Some(o) = g.as_object_mut() {
                o.remove("booking");
            }
        }
    }
    if let Some(slots) = d["slots"].as_array_mut() {
        for s in slots.iter_mut() {
            for p in s["requested"].as_array_mut().into_iter().flatten() {
                p["note"] = Value::Null;
            }
        }
    }
    if let Some(o) = d.as_object_mut() {
        o.remove("address");
    }
    d["join"] = i.join.clone().unwrap_or(Value::Null);
    d
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pocket::kits::Kit;

    /// Monday Nov 30, 2026, 08:00.
    fn monday_8am() -> NaiveDateTime {
        NaiveDate::from_ymd_opt(2026, 11, 30).unwrap().and_hms_opt(8, 0, 0).unwrap()
    }

    fn haircut_fill() -> Value {
        json!({"days": ["tue", "wed", "thu", "fri", "sat"], "from": "10:00", "to": "18:00", "minutes": 30, "seats": 1})
    }

    #[test]
    fn tue_to_sat_10_to_6_every_30_minutes_is_16_times_a_day() {
        let slots = expand_fill(&haircut_fill(), monday_8am()).unwrap();
        assert_eq!(slots.len(), 80);
        let mut per_day: std::collections::BTreeMap<String, usize> = Default::default();
        for s in &slots {
            *per_day.entry(s["start"].as_str().unwrap()[..10].to_string()).or_default() += 1;
            assert_eq!((s["minutes"].as_i64(), s["seats"].as_i64()), (Some(30), Some(1)));
        }
        assert_eq!(per_day.keys().cloned().collect::<Vec<_>>(), ["2026-12-01", "2026-12-02", "2026-12-03", "2026-12-04", "2026-12-05"]);
        assert!(per_day.values().all(|n| *n == 16), "{per_day:?}");
        assert_eq!(slots[0]["start"], "2026-12-01T10:00");
        assert_eq!(slots[15]["start"], "2026-12-01T17:30", "the last ends by 18:00");
        // and the kit takes them
        let spec = build_spec(&json!({"mode": "booking", "title": "Book a haircut", "fill": haircut_fill()}), monday_8am()).unwrap();
        assert_eq!(desk::parse_slots(spec.get("slots"), false).unwrap().len(), 80);
        assert!(spec.get("fill").is_none(), "the rule never reaches the kit: {spec}");
    }

    #[test]
    fn a_fill_never_makes_a_past_time_and_starts_today() {
        // Tuesday 14:10: today's 14:30 on, then Wed–Sat, and nothing from next Tuesday
        let now = NaiveDate::from_ymd_opt(2026, 12, 1).unwrap().and_hms_opt(14, 10, 0).unwrap();
        let slots = expand_fill(&haircut_fill(), now).unwrap();
        assert_eq!(slots[0]["start"], "2026-12-01T14:30");
        assert_eq!(slots.len(), 7 + 4 * 16);
        assert!(slots.iter().all(|s| s["start"].as_str().unwrap() < "2026-12-08"));
        // two weeks
        let mut two = haircut_fill();
        two["weeks"] = json!(2);
        assert_eq!(expand_fill(&two, monday_8am()).unwrap().len(), 160);
    }

    #[test]
    fn a_late_evening_fill_ends_by_to_and_never_wraps_past_midnight() {
        // 23:00 to 23:59 every 30 minutes: 23:00 fits, 23:30 would end at 00:00, after `to`
        let rule = json!({"days": ["tue"], "from": "23:00", "to": "23:59", "minutes": 30});
        let slots = expand_fill(&rule, monday_8am()).unwrap();
        assert_eq!(slots.iter().map(|s| s["start"].as_str().unwrap().to_string()).collect::<Vec<_>>(), ["2026-12-01T23:00"]);
        // A time longer than what is left of the day doesn't fit
        let long = json!({"days": ["tue"], "from": "23:00", "to": "23:59", "minutes": 120});
        assert!(expand_fill(&long, monday_8am()).unwrap_err().contains("doesn't fit"));
        let full = json!({"days": ["tue"], "from": "00:00", "to": "23:59", "minutes": 1430});
        assert_eq!(expand_fill(&full, monday_8am()).unwrap().len(), 1);
    }

    #[test]
    fn day_names_are_read_loosely_and_once() {
        let rule = json!({"days": ["Tuesday", "TUE", "sat."], "from": "09:00", "to": "10:00", "minutes": 60});
        let slots = expand_fill(&rule, monday_8am()).unwrap();
        assert_eq!(slots.iter().map(|s| s["start"].as_str().unwrap().to_string()).collect::<Vec<_>>(), ["2026-12-01T09:00", "2026-12-05T09:00"]);
    }

    #[test]
    fn a_bad_fill_or_bad_slots_answer_bad_slots_with_words_never_a_panic() {
        let bad = |desk: Value| build_spec(&desk, monday_8am()).unwrap_err();
        let booking = |extra: Value| {
            let mut d = json!({"mode": "booking", "title": "Book a haircut"});
            d.as_object_mut().unwrap().extend(extra.as_object().unwrap().clone());
            d
        };
        for (rule, words) in [
            (json!({"days": ["tue"], "from": "18:00", "to": "10:00", "minutes": 30}), "later than"),
            (json!({"days": ["someday"], "from": "10:00", "to": "18:00"}), "isn't a day"),
            (json!({"days": [], "from": "10:00", "to": "18:00"}), "fill.days"),
            (json!({"days": ["tue"], "from": "10", "to": "18:00"}), "HH:MM"),
            (json!({"days": ["tue"], "from": "10:00", "to": "18:00", "minutes": 2}), "fill.minutes"),
            (json!({"days": ["tue"], "from": "10:00", "to": "18:00", "seats": 0}), "fill.seats"),
            (json!({"days": ["tue"], "from": "10:00", "to": "18:00", "weeks": 9}), "fill.weeks"),
            (json!({"days": ["tue"], "from": "10:00", "to": "10:20", "minutes": 30}), "doesn't fit"),
            (json!("Tue-Sat 10-6"), "fill must be an object"),
        ] {
            let e = bad(booking(json!({ "fill": rule })));
            assert!(e.starts_with("Nothing was made (bad_slots, why: \"fill\")") && e.contains(words), "{words}: {e}");
        }
        // Every day for 4 weeks, every 5 minutes: more than the kit's 200, in the kit's words
        let e = bad(booking(json!({"fill": {"days": ["mon", "tue", "wed", "thu", "fri", "sat", "sun"], "from": "09:00", "to": "17:00", "minutes": 5, "weeks": 4}})));
        assert!(e.contains("why: \"too_many\"") && e.contains("up to 200 times"), "{e}");
        // Explicit slots go through the kit's check, with the index
        let e = bad(booking(json!({"slots": [{"start": "2026-12-01T10:00", "minutes": 60}, {"start": "2026-12-01T10:30", "minutes": 30}]})));
        assert!(e.contains("why: \"overlap\", index: 1") && e.contains("Slot 2: it overlaps"), "{e}");
        let e = bad(booking(json!({"slots": [{"start": "tomorrow at ten", "minutes": 30}]})));
        assert!(e.contains("why: \"start\", index: 0"), "{e}");
        let e = bad(booking(json!({})));
        assert!(e.contains("why: \"list\""), "{e}");
        let e = bad(booking(json!({"slots": [], "fill": haircut_fill()})));
        assert!(e.contains("not both"), "{e}");
        // Times on a guide desk
        let e = bad(json!({"title": "Sea View", "guide": "Check-in 3 PM", "fill": haircut_fill()}));
        assert!(e.contains("for a booking desk"), "{e}");
    }

    #[test]
    fn the_spec_takes_only_the_known_fields_and_never_an_answer_bot() {
        let spec = build_spec(
            &json!({"title": "Sea View", "guide": "Check-in 3 PM", "answer_bot": "k_other", "needs_pin": false, "max_guests": 1, "host_key": "x", "daily_cap": 30}),
            monday_8am(),
        )
        .unwrap();
        assert_eq!(spec, json!({"title": "Sea View", "guide": "Check-in 3 PM", "daily_cap": 30}));
        assert_eq!(desk::Desk.validate_spec(&spec), Ok(()));
    }

    #[test]
    fn no_title_asks_for_the_rules_and_a_wrapped_spec_is_read() {
        assert_eq!(desk_arg(&json!({})), Ok(None));
        assert_eq!(desk_arg(&json!({"mode": "guide"})), Ok(None));
        assert_eq!(desk_arg(&json!({"spec": null})), Ok(None));
        assert_eq!(desk_arg(&json!({"spec": {"title": "Sea View"}})), Ok(Some(json!({"title": "Sea View"}))));
        assert_eq!(desk_arg(&json!({"spec": "{\"title\": \"Sea View\"}"})), Ok(Some(json!({"title": "Sea View"}))));
        assert!(desk_arg(&json!({"spec": "not json"})).is_err());
        assert!(desk_arg(&json!({"spec": [1]})).is_err());
        assert!(holds_desk(Some(&json!({"title": "x"}))) && !holds_desk(Some(&json!({}))) && !holds_desk(None));
    }

    #[test]
    fn the_rules_and_description_name_what_a_bot_never_does_and_the_samples_are_specs_the_kit_takes() {
        for f in ["mode", "title", "place", "host_name", "what", "guide", "starters", "daily_cap", "slots", "fill"] {
            assert!(RULES.contains(&format!("- {f}")), "{f}");
        }
        for w in ["at most one question", "Never invent a Wi-Fi password, a door code", "Never promise a late check-out", "Money is always a hand-off"] {
            assert!(RULES.contains(w), "{w}");
        }
        let d = tool()["description"].as_str().unwrap().to_string();
        for w in ["front desk", "guest guide", "guesthouse", "Airbnb", "booking desk", "appointments", "ask for a time"] {
            assert!(d.contains(w), "{w}");
        }
        let mut samples = Vec::new();
        let mut rest = RULES;
        while let Some(start) = rest.find("{\"mode\"") {
            let mut depth = 0;
            let mut end = start;
            for (i, c) in rest[start..].char_indices() {
                match c {
                    '{' => depth += 1,
                    '}' => {
                        depth -= 1;
                        if depth == 0 {
                            end = start + i + 1;
                            break;
                        }
                    }
                    _ => {}
                }
            }
            samples.push(serde_json::from_str::<Value>(&rest[start..end]).unwrap());
            rest = &rest[end..];
        }
        assert_eq!(samples.len(), 2);
        for s in samples {
            let spec = build_spec(&s, monday_8am()).unwrap();
            assert_eq!(desk::Desk.validate_spec(&spec), Ok(()), "{spec}");
        }
    }

    #[test]
    fn the_card_drops_guest_words_and_carries_no_pin_or_key() {
        let spec = json!({"mode": "booking", "title": "Book a haircut", "slots": [{"id": "s1", "start": "2099-12-01T10:00", "minutes": 30}], "bot": {"name": "Bell"}, "answer_bot": "k1"});
        let st = desk::Desk.init(&spec);
        let c = card(&CardInput { spec: &spec, kit_state: &st, players: &[], now: Utc::now(), model: None, join: None });
        assert_eq!(c["title"], "Book a haircut");
        assert_eq!(c["mode"], "booking");
        assert_eq!(c["answer_bot"], json!({"name": "Bell", "model": null}));
        assert_eq!(c["slots"].as_array().unwrap().len(), 1);
        assert_eq!(c["counts"]["daily_cap"], 50);
        let s = c.to_string();
        for k in ["\"pin\"", "host_key", "\"key\"", "\"id\":\"k1\"", "\"address\""] {
            assert!(!s.contains(k), "{k} in {s}");
        }
    }
}
