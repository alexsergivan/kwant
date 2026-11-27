//! party-create-from-chat (autonomy/party-create-from-chat/brief.md): the bot's photo wall tool,
//! `wall_create`, offered beside `gather_create` and behind the same switch (`pocket_apps`, on or
//! naming `wall`; not `wall_tools.rs`, which is about sign-in walls). Before it, "Set up a photo
//! wall for the party" reached `pocket_create`, got the Quizmaster template back and ended with the
//! bot saying it couldn't (Mira's ux-pass-office-party fix #1).
//!
//! - With no wall in the arguments (no `title`) it answers with the fields and rules ([`RULES`]).
//! - With one it builds the spec from the fields it knows ([`FIELDS`]). **Never `when`**: Juno's
//!   wall-dates-agree (#1313) is changing what a wall's `when` means, so `event_at` carries the
//!   party's time and a `when` in the call is dropped (adding it back after #1313 is one line).
//!   `event_at` is required (a wall is for one party); a missing `closes_at` is noon the day after
//!   it, in `event_at`'s own offset; `closes_at` must be after `event_at` and still ahead. Then it
//!   makes the pocket with `pocket::create`: kit `wall`, owned by the calling bot, with the turn's
//!   own chat as its origin. The kit's validator is the rest of the check.
//! - Its answer starts with [`CARD_TEXT_HEAD`] and the pocket's code, then the guest link. No big
//!   screen or host remote link: those are single-use host tokens, and a tool answer goes to the
//!   model and into the chat's history. The chat card (party-card-and-results, next) mints them.

use super::continuity::SelfScope;
use super::quizmaster_tools::PocketOrigin;
use super::AppState;
use crate::pocket::kits::spec_date;
use crate::pocket::{schedule, wall, CreateRequest};
use chrono::{DateTime, Duration, NaiveDate, NaiveTime, Utc};
use serde_json::{json, Map, Value};

type ToolResult = (String, bool);

pub const TOOL: &str = "wall_create";
/// What a made wall's answer starts with, then its code and `]`.
pub const CARD_TEXT_HEAD: &str = "Pocket app · Photo wall [";
const MAX_ARGS_BYTES: usize = 8 * 1024;
/// The fields `wall_create` passes on to the kit; anything else (`when` too) is dropped.
pub(crate) const FIELDS: [&str; 7] = ["title", "host_name", "event_at", "closes_at", "photo_cap_mb", "per_guest_max", "captions"];

pub fn handles(name: &str) -> bool {
    name == TOOL
}

/// Whether the photo wall is on: the switch is `on` or names `wall`.
pub async fn enabled(state: &AppState) -> bool {
    crate::pocket::enabled_for(&state.db, wall::ID).await
}

pub fn tool() -> Value {
    json!({
        "name": TOOL,
        "description": "Make a photo wall for a party or event: guests scan a QR or open a link on their phones and send their party photos \
            (no app, no account). Each photo waits for the person's OK; approved ones show as a slideshow on a TV or big screen, and the \
            person downloads them all as a zip. Location and camera data are removed from every photo. Call it first with no arguments to \
            get the fields and rules, then with the whole wall: it checks it, opens it and answers with the guest link. Ask the person at \
            most one question. Never invent a day or time they didn't give.",
        "inputSchema": {"type": "object", "properties": {
            "title": {"type": "string", "description": "What guests see, up to 80 characters"},
            "host_name": {"type": "string", "description": "The person's first name, up to 40 characters"},
            "event_at": {"type": "string", "description": "When the party starts: RFC 3339 with the person's offset"},
            "closes_at": {"type": "string", "description": "When uploads stop: RFC 3339 with the person's offset (default: noon the day after)"},
            "photo_cap_mb": {"type": "integer", "description": "Largest photo in MB, 1 to 25 (default 15)"},
            "per_guest_max": {"type": "integer", "description": "Most photos per guest, 1 to 500 (default 40)"},
            "captions": {"type": "boolean", "description": "Whether guests can add a short caption (default true)"}
        }},
    })
}

/// What a call with no wall answers: the fields and rules, and a sample.
pub const RULES: &str = "Photo wall: the fields and rules.

kwant runs the wall: guests open its link (or scan the QR on the big screen) on their phones, join with a first name and a 4-digit \
PIN, and send photos. Location, camera and time data are removed from every photo before it is saved. Each photo waits until the person \
approves it; nobody else sees it before that. The big screen shows approved photos only, and the person downloads them as a zip.

Fields (all at the top level of the call):
- title (required, up to 80 characters): what guests see, like \"Office holiday party\".
- host_name (optional, up to 40 characters): the person's first name.
- event_at (required): when the party starts, RFC 3339 with the person's offset, like 2026-12-12T18:00:00+01:00.
- closes_at (optional): when uploads stop, RFC 3339 with the person's offset, after event_at. Left out, it is noon the day after the \
party.
- photo_cap_mb (optional, 1 to 25, default 15): the largest photo in MB.
- per_guest_max (optional, 1 to 500, default 40): the most photos one guest can send.
- captions (optional, default true): whether guests can add a short caption to a photo.

Rules:
- Ask the person at most one question before you call: if they didn't say the party's day and time, ask for those, and nothing else.
- Never invent a day or a time. Write only what the person said.
- Never say a field name (event_at, closes_at) to the person.
- After it opens, keep your reply short: give the guest link, say when uploads stop, and that each photo waits for the person's OK.

Sample (its facts are samples; write the person's own):
{\"title\": \"Office holiday party\", \"host_name\": \"Nina\", \"event_at\": \"2026-12-12T18:00:00+01:00\"}

Call wall_create again with the whole wall.";

fn fail(msg: impl Into<String>) -> ToolResult {
    (msg.into(), true)
}

/// The wall the arguments hold: the arguments themselves, or a `spec` object (or a string
/// holding one). `None` when it holds none yet (no `title`): then the call asks for the rules.
pub(crate) fn wall_arg(args: &Value) -> Result<Option<Value>, String> {
    let v = match args.get("spec") {
        None | Some(Value::Null) => args.clone(),
        Some(Value::String(s)) => serde_json::from_str::<Value>(s).map_err(|e| format!("`spec` isn't valid JSON: {e}"))?,
        Some(v) => v.clone(),
    };
    if !v.is_object() {
        return Err("The wall must be a JSON object.".into());
    }
    if v.to_string().len() > MAX_ARGS_BYTES {
        return Err(format!("The wall is bigger than {} KB.", MAX_ARGS_BYTES / 1024));
    }
    let has_title = v.get("title").and_then(Value::as_str).is_some_and(|t| !t.trim().is_empty());
    Ok(has_title.then_some(v))
}


/// Noon the day after `event_at`, in its own offset (a bare date: the next day).
fn noon_after(event_at: &str) -> Option<String> {
    let s = event_at.trim();
    if let Ok(t) = DateTime::parse_from_rfc3339(s) {
        let day = t.date_naive() + Duration::days(1);
        let at = day.and_time(NaiveTime::from_hms_opt(12, 0, 0)?).and_local_timezone(*t.offset()).single()?;
        return Some(at.to_rfc3339());
    }
    let d = NaiveDate::parse_from_str(s, "%Y-%m-%d").ok()?;
    Some((d + Duration::days(1)).format("%Y-%m-%d").to_string())
}

/// The spec `wall_create` hands the kit: the known fields only (never `when`), `closes_at` from
/// `event_at` when it's missing, and the two times checked against each other and `now`. `Err`
/// is the bot's answer.
pub(crate) fn build_spec(w: &Value, now: DateTime<Utc>) -> Result<Value, String> {
    let mut spec = Map::new();
    for f in FIELDS {
        if let Some(v) = w.get(f).filter(|v| !v.is_null()) {
            spec.insert(f.into(), v.clone());
        }
    }
    let event = match spec.get("event_at") {
        None => {
            return Err("Nothing was made: event_at, when the party starts, is missing. Ask the person for the day and time; never pick one \
                        yourself."
                .into())
        }
        Some(v) => spec_date(v).ok_or("Nothing was made: event_at must be an RFC 3339 time with the person's offset, like 2026-12-12T18:00:00+01:00.")?,
    };
    if !spec.contains_key("closes_at") {
        let at = spec["event_at"].as_str().and_then(noon_after).ok_or("Nothing was made: event_at must be an RFC 3339 time.")?;
        spec.insert("closes_at".into(), json!(at));
    }
    let closes = spec_date(&spec["closes_at"]).ok_or("Nothing was made: closes_at must be an RFC 3339 time with the person's offset.")?;
    if closes <= event {
        return Err("Nothing was made: closes_at, when uploads stop, must be after event_at. Leave it out for noon the day after the party.".into());
    }
    if closes <= now {
        return Err("Nothing was made: uploads would already be closed (closes_at is past). Check the day with the person.".into());
    }
    Ok(Value::Object(spec))
}

pub async fn call(state: &AppState, scope: &SelfScope, args: &Value) -> ToolResult {
    if !enabled(state).await {
        return fail("Pocket apps are off on this kwant.");
    }
    let w = match wall_arg(args) {
        Ok(Some(w)) => w,
        Ok(None) => return (RULES.to_string(), false),
        Err(e) => return fail(format!("Nothing was made: {e} Call wall_create again with the whole wall.")),
    };
    if state.db.get_kwant(&scope.kwant_id).await.ok().flatten().is_none() {
        return fail("Could not find your space.");
    }
    let spec = match build_spec(&w, Utc::now()) {
        Ok(s) => s,
        Err(e) => return fail(e),
    };
    let o = PocketOrigin::of(scope);
    let req = CreateRequest {
        kit: wall::ID.into(),
        spec,
        owner_kwant_id: Some(o.kwant_id.clone()),
        opens_at: None,
        closes_at: None,
        origin: Some(schedule::Origin { kwant_id: o.kwant_id, channel_id: o.channel_id, thread_id: o.thread_id }),
    };
    let (snap, _host_key) = match crate::pocket::create(state, req).await {
        Ok(s) => s,
        Err(e) => return fail(format!(
            "Nothing was made: {} Fix it from what the person said and call wall_create again. If a fact is missing, ask the person; \
             never fill it in yourself.",
            e.message
        )),
    };
    let join = super::santa_tool::santa_join(&crate::pocket::address::join(state, &snap).await);
    super::santa_tool::remember_join(&snap.row.code, join.clone());
    (made_text(&snap.row.code, &snap.row.title, &snap.spec, join.as_ref()), false)
}

fn made_text(code: &str, title: &str, spec: &Value, join: Option<&Value>) -> String {
    format!(
        "{CARD_TEXT_HEAD}{code}] \"{title}\" is open. Uploads stop at {closes}.\n\
         {link}\n\
         Tell the person to send the link to the guests (or show its QR on a big screen); kwant never sends it. Each photo waits for the \
         person's OK, and the big screen shows approved photos only. Keep your reply to two or three lines: the link, when uploads stop \
         (in the person's own words and time), and nothing you made up.",
        closes = spec["closes_at"].as_str().unwrap_or(""),
        link = super::gather_tool::link_line(join),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pocket::kits::Kit;

    fn nov27() -> DateTime<Utc> {
        "2026-11-27T06:00:00Z".parse().unwrap()
    }

    #[test]
    fn when_is_never_passed_and_only_known_fields_are() {
        let spec = build_spec(
            &json!({"title": "Party", "when": "2026-12-12", "event_at": "2026-12-12T18:00:00+01:00", "closes_at": "2026-12-13T12:00:00+01:00",
                    "owner": "k_other", "host_key": "x", "space": "s", "pocket_cap_mb": 4096}),
            nov27(),
        )
        .unwrap();
        assert_eq!(spec, json!({"title": "Party", "event_at": "2026-12-12T18:00:00+01:00", "closes_at": "2026-12-13T12:00:00+01:00"}));
    }

    #[test]
    fn closes_at_defaults_to_noon_the_day_after_in_event_ats_offset() {
        let spec = build_spec(&json!({"title": "P", "event_at": "2026-12-12T18:00:00+01:00"}), nov27()).unwrap();
        assert_eq!(spec["closes_at"], "2026-12-13T12:00:00+01:00");
        // Late at night: still the next calendar day of the party's own clock
        let late = build_spec(&json!({"title": "P", "event_at": "2026-12-12T23:30:00-08:00"}), nov27()).unwrap();
        assert_eq!(late["closes_at"], "2026-12-13T12:00:00-08:00");
        let bare = build_spec(&json!({"title": "P", "event_at": "2026-12-12"}), nov27()).unwrap();
        assert_eq!(bare["closes_at"], "2026-12-13");
    }

    #[test]
    fn a_missing_or_bad_event_at_and_a_closes_at_before_it_or_past_are_refused() {
        let e = build_spec(&json!({"title": "P"}), nov27()).unwrap_err();
        assert!(e.contains("event_at, when the party starts, is missing") && e.contains("never pick one yourself"), "{e}");
        let e = build_spec(&json!({"title": "P", "event_at": "next Friday"}), nov27()).unwrap_err();
        assert!(e.starts_with("Nothing was made: event_at must be"), "{e}");
        let e = build_spec(&json!({"title": "P", "event_at": "2026-12-12T18:00:00+01:00", "closes_at": "2026-12-12T17:00:00+01:00"}), nov27()).unwrap_err();
        assert!(e.contains("must be after event_at"), "{e}");
        let e = build_spec(&json!({"title": "P", "event_at": "2026-11-01T18:00:00+01:00", "closes_at": "2026-11-02T12:00:00+01:00"}), nov27()).unwrap_err();
        assert!(e.contains("already be closed"), "{e}");
    }

    #[test]
    fn no_title_asks_for_the_rules() {
        assert_eq!(wall_arg(&json!({})), Ok(None));
        assert_eq!(wall_arg(&json!({"spec": null})), Ok(None));
        assert_eq!(wall_arg(&json!({"spec": {"title": "P"}})), Ok(Some(json!({"title": "P"}))));
        assert!(wall_arg(&json!({"spec": "nope"})).is_err());
    }

    #[test]
    fn the_rules_and_description_name_what_a_bot_never_does_and_the_sample_is_a_spec_the_kit_takes() {
        for f in FIELDS {
            assert!(RULES.contains(&format!("- {f}")), "{f}");
        }
        assert!(!RULES.contains("- when"), "the wall's rules never offer when");
        for w in ["at most one question", "Never invent a day or a time", "Never say a field name", "waits until the person approves it"] {
            assert!(RULES.contains(w), "{w}");
        }
        assert!(!RULES.contains("make up"));
        let d = tool()["description"].as_str().unwrap().to_string();
        for w in ["photo wall", "party photos", "send their party photos", "slideshow", "TV", "Ask the person at most one question"] {
            assert!(d.contains(w), "{w} in {d}");
        }
        assert!(tool()["inputSchema"]["properties"].get("when").is_none());
        let start = RULES.find("{\"title\"").unwrap();
        let end = start + RULES[start..].find('}').unwrap() + 1;
        let sample: Value = serde_json::from_str(&RULES[start..end]).unwrap();
        let spec = build_spec(&sample, nov27()).unwrap();
        assert_eq!(wall::Wall.validate_spec(&spec), Ok(()), "{spec}");
    }

    #[test]
    fn the_made_text_starts_with_the_head_and_code_and_carries_no_host_link() {
        let join = json!({"mode": "wifi", "wifi_url": "http://192.168.1.20:8788/p/WALL22"});
        let t = made_text("WALL22", "Party", &json!({"closes_at": "2026-12-13T12:00:00+01:00"}), Some(&join));
        assert!(t.starts_with("Pocket app · Photo wall [WALL22] \"Party\" is open. Uploads stop at 2026-12-13T12:00:00+01:00."), "{t}");
        assert!(t.contains("Guest link: http://192.168.1.20:8788/p/WALL22") && t.contains("same Wi-Fi"), "{t}");
        assert!(!t.contains("#remote=") && !t.contains("#screen="), "{t}");
    }
}
