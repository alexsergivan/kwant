//! queue-create-tool (autonomy/queue-create-tool/brief.md): the bot's take-a-number queue tool,
//! `queue_create`, offered beside `desk_create` and behind the same switch (`pocket_apps`, on or
//! naming `queue`). Before it, "Give my bakery a take-a-number queue for Saturday morning, 8 to
//! 12." ended with the bot saying it couldn't (Mira's ux-pass-queue-ticket fix #1), and with
//! `pocket_apps=queue` alone the bot had no pocket tool at all.
//!
//! - With no queue in the arguments (no `title`) it answers with the fields and rules ([`RULES`]).
//! - With one it builds the spec from the six fields it knows ([`build_spec`]: `until` becomes the
//!   kit's `hours.until`), and makes the pocket with `pocket::create`, the same call
//!   `POST /api/pockets` uses: kit `queue`, owned by the calling bot, with the turn's own chat as
//!   its origin (never one named in the arguments). The kit's own `validate_spec` has the last word.
//! - A queue opens the moment it's made (the kit starts `open`); `until` only sets when it closes by
//!   itself. There is no opening time yet (`opens_at` is later work), so the rules tell the bot never
//!   to promise one.
//! - Its answer starts with [`CARD_TEXT_HEAD`] and the pocket's code, the way `desk_create`'s does,
//!   so the chat card (queue-card-in-chat) can find it.
//!
//! Each bot reaches only the queues it made.

use super::continuity::SelfScope;
use super::quizmaster_tools::PocketOrigin;
use super::AppState;
use crate::pocket::schedule;
use crate::pocket::{queue, CreateRequest};
use chrono::{DateTime, Local, NaiveTime, Utc};
use serde_json::{json, Map, Value};

type ToolResult = (String, bool);

pub const TOOL: &str = "queue_create";
/// What a made queue's answer starts with, then its code and `]`.
pub const CARD_TEXT_HEAD: &str = "Pocket app · Queue [";
/// The largest set of arguments a bot may hand in (six short fields).
const MAX_ARGS_BYTES: usize = 8 * 1024;
/// The fields `queue_create` passes on to the kit as they are; `until` is read on its own.
const FIELDS: [&str; 5] = ["title", "line", "host_name", "call_minutes", "ask_name"];

pub fn handles(name: &str) -> bool {
    name == TOOL
}

/// Whether the queue is on: the switch is `on` or names `queue`.
pub async fn enabled(state: &AppState) -> bool {
    crate::pocket::enabled_for(&state.db, queue::ID).await
}

pub fn tool() -> Value {
    json!({
        "name": TOOL,
        "description": "Set up a take-a-number queue: a ticket queue for walk-ins at a bakery, shop, counter, market stall, clinic or \
            event. Customers take a number on their phones and see their place in line; the person calls the next number, pauses \
            or closes the queue. No app, no account, no phone number. Call it first with no arguments to get the fields and rules, \
            then with the whole queue: it opens the queue at once. Ask the person at most one question. Never promise an opening \
            time: the queue opens when it's made, and until only sets when it closes by itself. Never invent a phone or SMS reminder.",
        "inputSchema": {"type": "object", "properties": {
            "title": {"type": "string", "description": "What customers see, up to 40 characters"},
            "line": {"type": "string", "description": "One line under the title, like \"Pick up at the side counter\" (up to 80 characters)"},
            "host_name": {"type": "string", "description": "The person's first name, up to 40 characters"},
            "until": {"type": "string", "description": "When the queue closes by itself, \"HH:MM\" on this computer's clock, like \"12:00\""},
            "call_minutes": {"type": "integer", "description": "Minutes a called customer has to reach the counter, 1 to 10 (default 3)"},
            "ask_name": {"type": "boolean", "description": "Whether customers give a first name with their ticket (default false)"}
        }},
    })
}

/// What a call with no queue answers: the fields and rules, and a sample.
pub const RULES: &str = "Take-a-number queue: the fields and rules.

kwant runs the queue: customers open its page on their phones (from the counter sign or the link), take a number and watch their \
place in line. The person calls the next number, and can pause or close the queue. There's no app, no account and no phone number: \
a ticket is just a number (and a first name, if the person wants one), deleted when it's served, skipped or given up, or when the queue closes.

Fields (all at the top level of the call):
- title (required, up to 40 characters): what customers see, like \"Tine's Bakery\".
- line (optional, up to 80 characters): one line under the title, like \"Pick up at the side counter\".
- host_name (optional, up to 40 characters): the person's first name.
- until (optional, \"HH:MM\" on this computer's clock): when the queue closes by itself, like \"12:00\". Leave it out and the queue \
stays open until the person closes it.
- call_minutes (optional, 1 to 10, default 3): how long a called customer has to reach the counter before their number is skipped.
- ask_name (optional, true or false, default false): whether customers give a first name with their ticket.

Rules:
- Ask the person at most one question before you call: whether customers give a first name, in plain words, like \"Should \
customers give a first name with their ticket, or just take a number? If you don't mind either way, I'll use numbers only.\" If they don't say, \
use numbers only. Never say \"ask_name\" or \"call_minutes\" to the person. Pick a short title from their words.
- Never promise an opening time. The queue opens the moment you make it; kwant can't open it later. When the person gives hours, \
like \"8 to 12\", set until to the closing time and say it plainly, like \"It's open now and closes by itself at 12:00. Nobody can \
take a number until you share its link or counter sign.\"
- When the person names a later day, like \"Saturday\", don't make the queue yet: until only knows a time, so it would open now \
and close at that time tomorrow, not on their day. Say it plainly, like \"kwant can't open a queue later, so ask me on Saturday \
morning and I'll make it then.\" Make it now only when they want it today.
- Never invent a phone or SMS reminder. Customers are only told on the queue's own page, while it's open on their phone.
- Never invent a price, an address or opening hours the person didn't give.

Sample (its facts are samples; use the person's own):
{\"title\": \"Tine's Bakery\", \"line\": \"Pick up at the side counter\", \"until\": \"12:00\", \"call_minutes\": 3}

Call queue_create again with the whole queue.";

fn fail(msg: impl Into<String>) -> ToolResult {
    (msg.into(), true)
}

/// The queue the arguments hold: the arguments themselves, or a `spec` object (or a string
/// holding one) when the bot wrapped them. `None` when it holds no queue yet (no `title`): then
/// the call asks for the rules.
pub(crate) fn queue_arg(args: &Value) -> Result<Option<Value>, String> {
    let v = match args.get("spec") {
        None | Some(Value::Null) => args.clone(),
        Some(Value::String(s)) => serde_json::from_str::<Value>(s).map_err(|e| format!("`spec` isn't valid JSON: {e}"))?,
        Some(v) => v.clone(),
    };
    if !v.is_object() {
        return Err("The queue must be a JSON object.".into());
    }
    if v.to_string().len() > MAX_ARGS_BYTES {
        return Err(format!("The queue is bigger than {} KB.", MAX_ARGS_BYTES / 1024));
    }
    let has_title = v.get("title").and_then(Value::as_str).is_some_and(|t| !t.trim().is_empty());
    Ok(has_title.then_some(v))
}

/// Whether a call's arguments hold a queue (what a made-this-turn card looks for).
pub(crate) fn holds_queue(args: Option<&Value>) -> bool {
    args.is_some_and(|a| matches!(queue_arg(a), Ok(Some(_))))
}

/// `until` as the kit takes it, `HH:MM` (an `H:MM` like "8:30" gets its leading zero). Also read
/// from `hours.until`, the kit's own name for it.
fn until_of(q: &Value) -> Result<Option<String>, String> {
    let v = match q.get("until").filter(|v| !v.is_null()) {
        Some(v) => Some(v),
        None => q.get("hours").and_then(|h| h.get("until")).filter(|v| !v.is_null()),
    };
    let Some(v) = v else { return Ok(None) };
    let s = v.as_str().map(str::trim).unwrap_or("");
    let ok = s.len() <= 5 && s.contains(':');
    match NaiveTime::parse_from_str(s, "%H:%M") {
        Ok(t) if ok => Ok(Some(t.format("%H:%M").to_string())),
        _ => Err(format!(
            "until must be a time written HH:MM on a 24-hour clock, like \"12:00\" (got {v}). Take it from what the person said; \
             if they gave no closing time, leave until out."
        )),
    }
}

/// The spec `queue_create` hands the kit: the known fields only (never `max_waiting` or anything
/// else in the arguments), and `until` as `hours.until`. `Err` is the bot's answer.
pub(crate) fn build_spec(q: &Value) -> Result<Value, String> {
    let mut spec = Map::new();
    for f in FIELDS {
        if let Some(v) = q.get(f).filter(|v| !v.is_null()) {
            spec.insert(f.into(), v.clone());
        }
    }
    if let Some(until) = until_of(q)? {
        spec.insert("hours".into(), json!({ "until": until }));
    }
    Ok(Value::Object(spec))
}

pub async fn call(state: &AppState, scope: &SelfScope, args: &Value) -> ToolResult {
    if !enabled(state).await {
        return fail("Pocket apps are off on this kwant.");
    }
    let q = match queue_arg(args) {
        Ok(Some(q)) => q,
        Ok(None) => return (RULES.to_string(), false),
        Err(e) => return fail(format!("Nothing was made: {e} Call queue_create again with the queue.")),
    };
    if state.db.get_kwant(&scope.kwant_id).await.ok().flatten().is_none() {
        return fail("Could not find your space.");
    }
    let spec = match build_spec(&q) {
        Ok(s) => s,
        Err(e) => return fail(format!("Nothing was made: {e}")),
    };
    let o = PocketOrigin::of(scope);
    let req = CreateRequest {
        kit: queue::ID.into(),
        spec,
        owner_kwant_id: Some(o.kwant_id.clone()),
        opens_at: None,
        closes_at: None,
        origin: Some(schedule::Origin { kwant_id: o.kwant_id, channel_id: o.channel_id, thread_id: o.thread_id }),
    };
    // The kit's refusal is a plain sentence ("call_minutes must be a whole number from 1 to 10.")
    let (snap, _host_key) = match crate::pocket::create(state, req).await {
        Ok(s) => s,
        Err(e) => return fail(format!(
            "Nothing was made: {} Fix it from what the person said and call queue_create again. If something is missing, ask the \
             person; never fill it in yourself.",
            e.message
        )),
    };
    let join = super::santa_tool::santa_join(&crate::pocket::address::join(state, &snap).await);
    super::santa_tool::remember_join(&snap.row.code, join.clone());
    let closes_at = serde_json::from_value::<DateTime<Utc>>(snap.kit_state["closes_at"].clone()).ok();
    let closes = closes_at.map(|c| closes_label(c.with_timezone(&Local), Local::now()));
    (made_text(&snap.row.code, &snap.row.title, closes.as_deref(), join.as_ref()), false)
}

/// When a queue closes by itself, as the person reads it: "12:00" today, "12:00 tomorrow".
fn closes_label(at: DateTime<Local>, now: DateTime<Local>) -> String {
    let hm = at.format("%H:%M");
    match (at.date_naive() - now.date_naive()).num_days() {
        d if d <= 0 => hm.to_string(),
        1 => format!("{hm} tomorrow"),
        _ => format!("{hm} on {}", at.format("%a %-d %b")),
    }
}

/// The answer to a call that made one: the head the chat card is found by, then what the bot
/// tells the person.
fn made_text(code: &str, title: &str, closes: Option<&str>, join: Option<&Value>) -> String {
    let link = join.and_then(|j| j["anywhere_url"].as_str().or(j["wifi_url"].as_str()));
    let link_line = match (link, join.map(|j| j["mode"].as_str() == Some("wifi"))) {
        (Some(u), Some(true)) => format!("Customer link: {u}\nIt works only on the same Wi-Fi as this computer."),
        (Some(u), _) => format!("Customer link: {u}"),
        (None, _) => "No customer link works yet: this kwant answers only this computer, so phones can't reach the queue. Tell the person that plainly, and don't write a link yourself.".into(),
    };
    let hours = match closes {
        Some(c) => format!("It's open now and closes by itself at {c}."),
        None => "It's open now and stays open until the person closes it.".into(),
    };
    format!(
        "{CARD_TEXT_HEAD}{code}] \"{title}\" is made. {hours}\n\
         The person's chat already shows its card (who's waiting, Call next, Pause and Close for today), so don't repeat what's on it.\n\
         {link_line}\n\
         Keep your reply to two or three lines. Nobody can take a number until the person shares the link or the counter sign.\n\
         Never promise an opening time (kwant can't open a queue later) and never invent a phone or SMS reminder: customers are \
         only told on the queue's own page."
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pocket::kits::Kit;
    use chrono::TimeZone;

    fn bakery() -> Value {
        json!({"title": "Tine's Bakery", "line": "Pick up at the side counter", "until": "12:00", "call_minutes": 3})
    }

    #[test]
    fn the_spec_takes_only_the_known_fields_and_until_becomes_hours() {
        let spec = build_spec(&json!({
            "title": "Tine's Bakery", "line": "Side counter", "host_name": "Tine", "until": "12:00", "call_minutes": 4,
            "ask_name": true, "max_waiting": 10, "host_key": "x", "owner_kwant_id": "k_other", "state": "closed"
        }))
        .unwrap();
        assert_eq!(
            spec,
            json!({"title": "Tine's Bakery", "line": "Side counter", "host_name": "Tine", "hours": {"until": "12:00"}, "call_minutes": 4, "ask_name": true})
        );
        assert_eq!(queue::Queue.validate_spec(&spec), Ok(()));
        // No until: no hours at all
        assert_eq!(build_spec(&json!({"title": "A"})).unwrap(), json!({"title": "A"}));
        assert_eq!(build_spec(&json!({"title": "A", "until": null})).unwrap(), json!({"title": "A"}));
    }

    #[test]
    fn until_is_kept_as_hh_mm_and_a_loose_or_bad_one_is_refused_in_words() {
        for (given, kept) in [("12:00", "12:00"), ("8:30", "08:30"), (" 17:05 ", "17:05"), ("00:00", "00:00"), ("23:59", "23:59")] {
            assert_eq!(build_spec(&json!({"title": "A", "until": given})).unwrap()["hours"]["until"], kept, "{given}");
        }
        // The kit's own name is read too
        assert_eq!(build_spec(&json!({"title": "A", "hours": {"until": "13:00"}})).unwrap()["hours"], json!({"until": "13:00"}));
        for bad in [json!("12"), json!("noon"), json!("12pm"), json!("12:00 PM"), json!("24:00"), json!("12:60"), json!("2026-11-28T12:00"), json!(12)] {
            let e = build_spec(&json!({"title": "A", "until": bad})).unwrap_err();
            assert!(e.starts_with("until must be a time written HH:MM") && e.contains("leave until out"), "{bad}: {e}");
        }
    }

    #[test]
    fn the_kit_still_refuses_a_bad_call_minutes_or_ask_name() {
        for bad in [json!(0), json!(11), json!(2.5), json!("3")] {
            let spec = build_spec(&json!({"title": "A", "call_minutes": bad})).unwrap();
            assert_eq!(queue::Queue.validate_spec(&spec), Err("call_minutes must be a whole number from 1 to 10.".into()), "{bad}");
        }
        let spec = build_spec(&json!({"title": "A", "ask_name": "yes"})).unwrap();
        assert!(queue::Queue.validate_spec(&spec).is_err());
        let spec = build_spec(&json!({"title": "x".repeat(41)})).unwrap();
        assert!(queue::Queue.validate_spec(&spec).is_err());
    }

    #[test]
    fn no_title_asks_for_the_rules_and_a_wrapped_spec_is_read() {
        assert_eq!(queue_arg(&json!({})), Ok(None));
        assert_eq!(queue_arg(&json!({"until": "12:00"})), Ok(None));
        assert_eq!(queue_arg(&json!({"title": "  "})), Ok(None));
        assert_eq!(queue_arg(&json!({"spec": null})), Ok(None));
        assert_eq!(queue_arg(&json!({"spec": {"title": "Q"}})), Ok(Some(json!({"title": "Q"}))));
        assert_eq!(queue_arg(&json!({"spec": "{\"title\": \"Q\"}"})), Ok(Some(json!({"title": "Q"}))));
        assert!(queue_arg(&json!({"spec": "not json"})).is_err());
        assert!(queue_arg(&json!({"spec": [1]})).is_err());
        assert!(queue_arg(&json!({"title": "Q", "line": "x".repeat(9000)})).is_err());
    }

    #[test]
    fn the_made_answer_starts_with_the_head_and_never_promises_an_opening_time() {
        let t = made_text("PQ7XK2", "Tine's Bakery", Some("12:00"), None);
        assert!(t.starts_with("Pocket app · Queue [PQ7XK2] \"Tine's Bakery\" is made."), "{t}");
        assert!(t.contains("It's open now and closes by itself at 12:00."), "{t}");
        assert!(t.contains("The person's chat already shows its card (who's waiting, Call next, Pause and Close for today)"), "{t}");
        assert!(t.contains("No customer link works yet") && t.contains("so phones can't reach the queue. Tell the person that plainly, and don't write a link yourself."), "{t}");
        assert!(t.ends_with("never invent a phone or SMS reminder: customers are only told on the queue's own page."), "{t}");
        let t = made_text("PQ7XK2", "Q", None, Some(&json!({"mode": "anywhere", "anywhere_url": "https://q.example/j/PQ7XK2"})));
        assert!(t.contains("stays open until the person closes it") && t.contains("Customer link: https://q.example/j/PQ7XK2"), "{t}");
        let t = made_text("PQ7XK2", "Q", None, Some(&json!({"mode": "wifi", "wifi_url": "http://192.168.1.5:8787/j/PQ7XK2"})));
        assert!(t.contains("only on the same Wi-Fi"), "{t}");
        for t in [made_text("A", "B", Some("12:00"), None), made_text("A", "B", None, None)] {
            assert!(!t.contains("opens at") && !t.contains("SMS reminder will") && !t.contains("text them"), "{t}");
        }
    }

    #[test]
    fn the_closing_time_says_tomorrow_when_it_is_tomorrow() {
        let at = |d: u32, h: u32, m: u32| Local.with_ymd_and_hms(2026, 11, d, h, m, 0).single().unwrap();
        assert_eq!(closes_label(at(28, 12, 0), at(28, 5, 0)), "12:00");
        assert_eq!(closes_label(at(29, 12, 0), at(28, 13, 0)), "12:00 tomorrow");
        assert_eq!(closes_label(at(30, 9, 5), at(28, 13, 0)), "09:05 on Mon 30 Nov");
    }

    #[test]
    fn the_rules_and_description_name_every_field_and_what_a_bot_never_does() {
        for f in ["title", "line", "host_name", "until", "call_minutes", "ask_name"] {
            assert!(RULES.contains(&format!("- {f} (")), "{f}");
        }
        for w in [
            "at most one question",
            "Never promise an opening time",
            "When the person names a later day",
            "ask me on Saturday morning and I'll make it then.",
            "It's open now and closes by itself at 12:00.",
            "Never invent a phone or SMS reminder",
            "Never say \"ask_name\" or \"call_minutes\" to the person",
            "Pick a short title from their words",
        ] {
            assert!(RULES.contains(w), "{w}");
        }
        assert!(!RULES.contains("make up"), "nothing in the rules tells the bot to make anything up");
        assert!(!RULES.contains("opens when you tap"), "the queue opens when it's made; never promise a tap opens it");
        let d = tool()["description"].as_str().unwrap().to_string();
        for w in ["take-a-number queue", "ticket", "walk-ins", "bakery", "Ask the person at most one question", "Never promise an opening time", "SMS reminder"] {
            assert!(d.contains(w), "{w} in {d}");
        }
        let props = tool()["inputSchema"]["properties"].as_object().unwrap().keys().cloned().collect::<Vec<_>>();
        assert_eq!(props.len(), 6, "{props:?}");
        // The sample is a spec the kit takes
        let start = RULES.find("{\"title\"").unwrap();
        let end = start + RULES[start..].find('}').unwrap() + 1;
        let sample: Value = serde_json::from_str(&RULES[start..end]).unwrap();
        assert_eq!(sample, bakery());
        assert_eq!(queue::Queue.validate_spec(&build_spec(&sample).unwrap()), Ok(()));
    }
}
