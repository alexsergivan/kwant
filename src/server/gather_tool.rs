//! party-create-from-chat (autonomy/party-create-from-chat/brief.md): the bot's RSVP, sign-up
//! and vote tool, `gather_create`, offered beside `santa_create` and `desk_create` and behind the
//! same switch (`pocket_apps`, on or naming `gather`). Before it, "Set up an RSVP for the office
//! party", "Potluck: 3 mains, 4 desserts, 2 drinks" or "a theme vote" reached `pocket_create`,
//! got the Quizmaster template back and ended with the bot saying it couldn't (Mira's
//! ux-pass-office-party fix #1).
//!
//! - With no gather in the arguments (no `title`) it answers with the fields and rules ([`RULES`]).
//! - With one it builds the spec from the fields it knows ([`FIELDS`]; never an owner, key or
//!   space), checks that a `when` line naming a weekday names `event_at`'s weekday (report m6:
//!   "Fri Dec 12" when Dec 12 2026 is a Saturday), fills a missing `closes_at` with `event_at`
//!   (answers close when the event starts), and makes the pocket with `pocket::create`: kit
//!   `gather`, owned by the calling bot, with the turn's own chat as its origin (never one named
//!   in the arguments). The kit's validator is the rest of the check; its refusal comes back in
//!   its own words.
//! - Its answer starts with [`CARD_TEXT_HEAD`] and the pocket's code, then the face and the guest
//!   link. No host remote or big-screen link: those are single-use host tokens, and a tool answer
//!   goes to the model and into the chat's history. The chat card (party-card-and-results, next)
//!   mints them for the person.
//!
//! The Chaser's nudge templates are written here, once (pa10b: no model runs afterwards); left
//! out, the kit's defaults are used.

use super::continuity::SelfScope;
use super::quizmaster_tools::PocketOrigin;
use super::AppState;
use crate::pocket::{gather, schedule, CreateRequest};
use chrono::{DateTime, Datelike, Duration, NaiveDate, Weekday};
use serde_json::{json, Map, Value};

type ToolResult = (String, bool);

pub const TOOL: &str = "gather_create";
/// What a made RSVP, sign-up or vote's answer starts with, then its code and `]`. One head for
/// the three faces, so the chat card finds any of them with one prefix; the face follows the code.
pub const CARD_TEXT_HEAD: &str = "Pocket app · Gather [";
/// The largest set of arguments a bot may hand in (20 slots, 10 options, 60 members and four
/// 300-character templates are a few KB).
const MAX_ARGS_BYTES: usize = 32 * 1024;
/// The fields `gather_create` passes on to the kit; anything else in the arguments is dropped.
pub(crate) const FIELDS: [&str; 14] =
    ["face", "title", "when", "place", "host_name", "price", "closes_at", "event_at", "fields", "slots", "ballot", "members", "show_names", "templates"];

pub fn handles(name: &str) -> bool {
    name == TOOL
}

/// Whether RSVP / sign-up / vote is on: the switch is `on` or names `gather`.
pub async fn enabled(state: &AppState) -> bool {
    crate::pocket::enabled_for(&state.db, gather::ID).await
}

pub fn tool() -> Value {
    json!({
        "name": TOOL,
        "description": "Make an RSVP (who's coming, plus-ones, food needs, who paid), a sign-up sheet (a potluck, volunteer shifts or any \
            slots with a limit each, with a waiting list), or a vote (a poll with one pick, or ranked; always a secret ballot: nobody, not \
            even the person, sees who voted for what). Guests open one link on their phones and answer with a first name and a PIN. Call it \
            first with no arguments to get the fields and rules, then with the whole thing: it checks it, opens it and answers with the guest \
            link for the person to send. Ask the person at most one question. Never invent a price, a place, a time or a list of people \
            they didn't give.",
        "inputSchema": {"type": "object", "properties": {
            "face": {"type": "string", "enum": ["rsvp", "signup", "vote"], "description": "rsvp, signup (slots) or vote"},
            "title": {"type": "string", "description": "What guests see, up to 80 characters"},
            "when": {"type": "string", "description": "The line guests read, like \"Sat, Dec 12, 6 PM\" (up to 60 characters; never an RFC 3339 time)"},
            "place": {"type": "string", "description": "Where, up to 60 characters"},
            "host_name": {"type": "string", "description": "The person's first name, up to 40 characters"},
            "price": {"type": "string", "description": "RSVP only: each person's share as the person said it, like \"EUR 15\" (up to 24 characters)"},
            "event_at": {"type": "string", "description": "When the event starts: RFC 3339 with the person's offset"},
            "closes_at": {"type": "string", "description": "When answers close: RFC 3339 with the person's offset (default: event_at)"},
            "fields": {"type": "array", "items": {"type": "string"}, "description": "RSVP: plus_ones, food_needs; signup: slot_note"},
            "slots": {"type": "array", "items": {"type": "object"}, "description": "Sign-up: [{\"name\": \"Mains\", \"cap\": 3}, …], 1 to 20"},
            "ballot": {"type": "object", "description": "Vote: {\"kind\": \"one_pick\" or \"ranked\", \"options\": [2 to 10 short texts]}"},
            "members": {"type": "array", "items": {"type": "string"}, "description": "The first names the person expects to answer"},
            "show_names": {"type": "boolean", "description": "Whether guests see each other's names and answers (default true)"},
            "templates": {"type": "object", "description": "Optional nudge texts: maybe, not_paid, moved_up, not_answered"}
        }},
    })
}

/// What a call with no gather answers: the fields and rules, and three samples.
pub const RULES: &str = "RSVP, sign-up and vote: the fields and rules.

kwant runs it: guests open one link on their phones, join with a first name and a 4-digit PIN, and answer. The person sees every answer \
and who hasn't answered yet, and copies ready-made nudges to send from their own chat; kwant never sends anything itself.

Three kinds (face):
- rsvp: Yes / Maybe / No, with plus-ones and food needs if the person wants them, and a \"paid\" tick per guest when there's a price.
- signup: slots with a limit each, like a potluck (3 mains, 4 desserts, 2 drinks) or volunteer shifts. A full slot has a waiting list; \
when someone leaves, the first person waiting moves in.
- vote: one pick, or ranked. Always a secret ballot: the person sees who voted (to nudge) and the result, never anyone's choice.

Fields (all at the top level of the call):
- face (required): \"rsvp\", \"signup\" or \"vote\".
- title (required, up to 80 characters): what guests see, like \"Office holiday party\".
- when (optional, up to 60 characters): the line guests read, like \"Sat, Dec 12, 6 PM\". It is text, never an RFC 3339 time. If it \
names a day of the week, that must be event_at's day: check the calendar, and if the person's day and date don't agree, ask them which \
one they mean.
- place (optional, up to 60 characters).
- host_name (optional, up to 40 characters): the person's first name.
- price (rsvp only, optional, up to 24 characters): each person's share, as the person said it, like \"EUR 15\".
- event_at (optional): when the event starts, RFC 3339 with the person's offset, like 2026-12-12T18:00:00+01:00.
- closes_at (required unless event_at is given): when answers close, RFC 3339 with the person's offset. It can't be past or after \
event_at. Left out, it is event_at: answers close when the event starts.
- fields (optional): extra questions. rsvp: \"plus_ones\", \"food_needs\". signup: \"slot_note\" (a short note, like the dish).
- slots (signup, required, 1 to 20): [{\"name\": \"Mains\", \"cap\": 3}, …]; names up to 40 characters, cap 1 to 60.
- ballot (vote, required): {\"kind\": \"one_pick\" or \"ranked\", \"options\": [\"…\", …]}, 2 to 10 options, each up to 60 characters.
- members (optional, up to 60 first names): who the person expects to answer, so the person sees who hasn't yet. Anyone with the \
link can still answer.
- show_names (optional, default true): whether guests see each other's names and answers. With false, only the person sees who's coming.
- templates (optional): the nudges the person copies, written now because nothing writes them later. Keys: maybe, not_paid, moved_up, \
not_answered; each up to 300 characters, with {name}, {title}, {when}, {place}, {host}, {slot} and {link} filled in. Left out, kwant's own are used.

Rules:
- Ask the person at most one question before you call. If the day or time is missing (for a vote: when voting closes), or the day \
of the week and the date don't agree, ask only about that, and use the defaults for the rest: names shown, anyone with the link can \
vote. Otherwise, for an RSVP, ask whether guests should see who's coming, like \"Should guests see who's coming, or only you? I'll show \
names if that's fine.\" For a vote, ask who's voting, like \"Who's voting? Give me their first names and I'll show you who hasn't voted \
yet. Anyone with the link can vote either way.\"
- Never invent a price, a place, a time or a list of people. Write only what the person said.
- Never say a field name (show_names, closes_at, members) to the person.
- After it opens, keep your reply short: give the guest link, say when answers close, and that the person sends the link to everyone.

Sample RSVP (its facts are samples; write the person's own):
{\"face\": \"rsvp\", \"title\": \"Office holiday party\", \"when\": \"Sat, Dec 12, 6 PM\", \"event_at\": \"2026-12-12T18:00:00+01:00\", \"price\": \"EUR 15\", \"fields\": [\"plus_ones\", \"food_needs\"], \"show_names\": false}

Sample sign-up:
{\"face\": \"signup\", \"title\": \"Potluck\", \"event_at\": \"2026-12-12T18:00:00+01:00\", \"slots\": [{\"name\": \"Mains\", \"cap\": 3}, {\"name\": \"Desserts\", \"cap\": 4}, {\"name\": \"Drinks\", \"cap\": 2}], \"fields\": [\"slot_note\"]}

Sample vote:
{\"face\": \"vote\", \"title\": \"Party theme\", \"closes_at\": \"2026-12-05T18:00:00+01:00\", \"ballot\": {\"kind\": \"ranked\", \"options\": [\"Winter wonderland\", \"Ugly sweaters\", \"Casino night\"]}}

Call gather_create again with the whole thing.";

fn fail(msg: impl Into<String>) -> ToolResult {
    (msg.into(), true)
}

/// The gather the arguments hold: the arguments themselves, or a `spec` object (or a string
/// holding one) when the bot wrapped them. `None` when it holds none yet (no `title`): then the
/// call asks for the rules.
pub(crate) fn gather_arg(args: &Value) -> Result<Option<Value>, String> {
    let v = match args.get("spec") {
        None | Some(Value::Null) => args.clone(),
        Some(Value::String(s)) => serde_json::from_str::<Value>(s).map_err(|e| format!("`spec` isn't valid JSON: {e}"))?,
        Some(v) => v.clone(),
    };
    if !v.is_object() {
        return Err("It must be a JSON object.".into());
    }
    if v.to_string().len() > MAX_ARGS_BYTES {
        return Err(format!("It is bigger than {} KB.", MAX_ARGS_BYTES / 1024));
    }
    let has_title = v.get("title").and_then(Value::as_str).is_some_and(|t| !t.trim().is_empty());
    Ok(has_title.then_some(v))
}


/// The one weekday a `when` line names (Fri, Friday, fri.), or `None` when it names none or more
/// than one ("Fri night to Sat").
pub(crate) fn weekday_in(line: &str) -> Option<Weekday> {
    const DAYS: [(Weekday, &str, &[&str]); 7] = [
        (Weekday::Mon, "monday", &["mon"]),
        (Weekday::Tue, "tuesday", &["tue", "tues"]),
        (Weekday::Wed, "wednesday", &["wed"]),
        (Weekday::Thu, "thursday", &["thu", "thur", "thurs"]),
        (Weekday::Fri, "friday", &["fri"]),
        (Weekday::Sat, "saturday", &["sat"]),
        (Weekday::Sun, "sunday", &["sun"]),
    ];
    let mut found: Option<Weekday> = None;
    for w in line.split(|c: char| !c.is_alphabetic()).filter(|w| !w.is_empty()) {
        let w = w.to_lowercase();
        if let Some((d, _, _)) = DAYS.iter().find(|(_, full, short)| w == *full || short.contains(&w.as_str())) {
            match found {
                Some(f) if f != *d => return None,
                _ => found = Some(*d),
            }
        }
    }
    found
}

/// `event_at`'s own calendar day (in its own offset), or `None` when it isn't a date.
fn event_day(v: &Value) -> Option<NaiveDate> {
    let s = v.as_str()?.trim();
    DateTime::parse_from_rfc3339(s).map(|t| t.date_naive()).ok().or_else(|| NaiveDate::parse_from_str(s, "%Y-%m-%d").ok())
}

fn day_name(d: Weekday) -> &'static str {
    match d {
        Weekday::Mon => "Monday",
        Weekday::Tue => "Tuesday",
        Weekday::Wed => "Wednesday",
        Weekday::Thu => "Thursday",
        Weekday::Fri => "Friday",
        Weekday::Sat => "Saturday",
        Weekday::Sun => "Sunday",
    }
}

/// The spec `gather_create` hands the kit: the known fields only, `closes_at` from `event_at`
/// when it's missing, and a `when` naming the wrong weekday refused. `Err` is the bot's answer.
pub(crate) fn build_spec(g: &Value) -> Result<Value, String> {
    let mut spec = Map::new();
    for f in FIELDS {
        if let Some(v) = g.get(f).filter(|v| !v.is_null()) {
            spec.insert(f.into(), v.clone());
        }
    }
    if !spec.contains_key("closes_at") {
        if let Some(e) = spec.get("event_at").cloned() {
            spec.insert("closes_at".into(), e);
        }
    }
    let line = spec.get("when").and_then(Value::as_str).unwrap_or("");
    if let (Some(said), Some(day)) = (weekday_in(line), spec.get("event_at").and_then(event_day)) {
        if said != day.weekday() {
            // The nearest day with the weekday the person named (within 3 days either way), so
            // the bot can offer both dates without counting days itself.
            let d = (said.num_days_from_monday() as i64 - day.weekday().num_days_from_monday() as i64).rem_euclid(7);
            let other = day + Duration::days(if d > 3 { d - 7 } else { d });
            return Err(format!(
                "Nothing was made: when says {said}, but event_at is {actual}, {date}. Ask the person which they mean, like \"{short} is a \
                 {actual}. Do you mean {actual}, {short}, or {other}?\"; don't pick one yourself.",
                said = day_name(said),
                actual = day_name(day.weekday()),
                date = day.format("%b %-d, %Y"),
                short = day.format("%b %-d"),
                other = other.format("%A, %b %-d"),
            ));
        }
    }
    Ok(Value::Object(spec))
}

pub async fn call(state: &AppState, scope: &SelfScope, args: &Value) -> ToolResult {
    if !enabled(state).await {
        return fail("Pocket apps are off on this kwant.");
    }
    let g = match gather_arg(args) {
        Ok(Some(g)) => g,
        Ok(None) => return (RULES.to_string(), false),
        Err(e) => return fail(format!("Nothing was made: {e} Call gather_create again with the whole thing.")),
    };
    if state.db.get_kwant(&scope.kwant_id).await.ok().flatten().is_none() {
        return fail("Could not find your space.");
    }
    let spec = match build_spec(&g) {
        Ok(s) => s,
        Err(e) => return fail(e),
    };
    let o = PocketOrigin::of(scope);
    let req = CreateRequest {
        kit: gather::ID.into(),
        spec,
        owner_kwant_id: Some(o.kwant_id.clone()),
        opens_at: None,
        closes_at: None,
        origin: Some(schedule::Origin { kwant_id: o.kwant_id, channel_id: o.channel_id, thread_id: o.thread_id }),
    };
    // The kit's refusal is a plain sentence ("A sign-up needs slots, like [{\"name\": \"Desserts\", \"cap\": 2}].")
    let (snap, _host_key) = match crate::pocket::create(state, req).await {
        Ok(s) => s,
        Err(e) => return fail(format!(
            "Nothing was made: {} Fix it from what the person said and call gather_create again. If a fact is missing (a day, a time, \
             the options), ask the person; never fill it in yourself.",
            e.message
        )),
    };
    let join = super::santa_tool::santa_join(&crate::pocket::address::join(state, &snap).await);
    super::santa_tool::remember_join(&snap.row.code, join.clone());
    (made_text(&snap.row.code, &snap.row.title, &snap.spec, join.as_ref()), false)
}

/// The guest-link line every made pocket's answer carries (the Santa and desk tools' wording).
pub(crate) fn link_line(join: Option<&Value>) -> String {
    let link = join.and_then(|j| j["anywhere_url"].as_str().or(j["wifi_url"].as_str()));
    match (link, join.map(|j| j["mode"].as_str() == Some("wifi"))) {
        (Some(u), Some(true)) => format!(
            "Guest link: {u}\nIt works only on the same Wi-Fi as this computer, not from anywhere: tell the person that plainly."
        ),
        (Some(u), _) => format!("Guest link: {u}"),
        (None, _) => "No guest link works yet: guests can't reach this computer. Tell the person; don't write a link yourself.".into(),
    }
}

/// The answer to a call that made one: the head (with the code), the face, then what the bot
/// tells the person.
fn made_text(code: &str, title: &str, spec: &Value, join: Option<&Value>) -> String {
    let face = spec["face"].as_str().unwrap_or("");
    let kind = match face {
        gather::RSVP => "RSVP",
        gather::SIGNUP => "Sign-up",
        _ => "Vote",
    };
    let closes = spec["closes_at"].as_str().unwrap_or("");
    let secret = if face == gather::VOTE {
        "\nIt's a secret ballot: the person sees who voted and the result, never anyone's choice. Never ask anyone how they voted."
    } else {
        ""
    };
    format!(
        "{CARD_TEXT_HEAD}{code}] {kind} \"{title}\" is open. Answers close at {closes}.\n\
         {link}\n\
         Tell the person to send the link to everyone taking part; kwant never sends it. Keep your reply to two or three lines: the link, \
         when answers close (in the person's own words and time), and nothing you made up.{secret}",
        link = link_line(join),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pocket::kits::Kit;

    #[test]
    fn a_when_naming_another_weekday_than_event_at_is_refused() {
        // Dec 12 2026 is a Saturday (report m6)
        let fri = build_spec(&json!({"face": "rsvp", "title": "Party", "when": "Fri Dec 12, 6 PM", "event_at": "2026-12-12T18:00:00+01:00"}));
        let e = fri.unwrap_err();
        assert!(e.starts_with("Nothing was made: when says Friday, but event_at is Saturday, Dec 12, 2026."), "{e}");
        assert!(e.contains("Ask the person which they mean, like \"Dec 12 is a Saturday. Do you mean Saturday, Dec 12, or Friday, Dec 11?\""), "{e}");
        // The other date is the nearest day with the named weekday, before or after
        let sun = build_spec(&json!({"face": "rsvp", "title": "Party", "when": "Sunday", "event_at": "2026-12-12T18:00:00+01:00"})).unwrap_err();
        assert!(sun.contains("Do you mean Saturday, Dec 12, or Sunday, Dec 13?"), "{sun}");
        let wed = build_spec(&json!({"face": "rsvp", "title": "Party", "when": "Wed", "event_at": "2026-12-12T18:00:00+01:00"})).unwrap_err();
        assert!(wed.contains("or Wednesday, Dec 9?"), "{wed}");
        let tue = build_spec(&json!({"face": "rsvp", "title": "Party", "when": "Tue", "event_at": "2026-12-12T18:00:00+01:00"})).unwrap_err();
        assert!(tue.contains("or Tuesday, Dec 15?"), "{tue}");
        // Across a month end: Jan 1 2027 is a Friday
        let jan = build_spec(&json!({"face": "rsvp", "title": "P", "when": "Wed", "event_at": "2027-01-01T18:00:00+01:00"})).unwrap_err();
        assert!(jan.contains("\"Jan 1 is a Friday. Do you mean Friday, Jan 1, or Wednesday, Dec 30?\""), "{jan}");
        for ok in ["Sat Dec 12, 6 PM", "Saturday 12 December", "sat. 18:00", "Dec 12, 6 PM", "Fri night into Sat"] {
            assert!(build_spec(&json!({"face": "rsvp", "title": "Party", "when": ok, "event_at": "2026-12-12T18:00:00+01:00"})).is_ok(), "{ok}");
        }
        // The day is event_at's own: 23:30 at +01:00 on Saturday is still Saturday
        assert!(build_spec(&json!({"face": "rsvp", "title": "P", "when": "Sat late", "event_at": "2026-12-12T23:30:00+01:00"})).is_ok());
        assert!(build_spec(&json!({"face": "rsvp", "title": "P", "when": "Sat", "event_at": "2026-12-12"})).is_ok());
        // No event_at: nothing to compare
        assert!(build_spec(&json!({"face": "rsvp", "title": "P", "when": "Fri", "closes_at": "2026-12-12T18:00:00+01:00"})).is_ok());
    }

    #[test]
    fn weekday_words_are_whole_words_only() {
        assert_eq!(weekday_in("Fri, Dec 12"), Some(Weekday::Fri));
        assert_eq!(weekday_in("THURSDAY"), Some(Weekday::Thu));
        assert_eq!(weekday_in("Thurs 7pm"), Some(Weekday::Thu));
        assert_eq!(weekday_in("Sunday brunch, Sun 11"), Some(Weekday::Sun));
        assert_eq!(weekday_in("Fri to Sat"), None);
        assert_eq!(weekday_in("Saturnalia at Monaco"), None);
        assert_eq!(weekday_in("Dec 12, 6 PM"), None);
    }

    #[test]
    fn closes_at_defaults_to_event_at_and_only_known_fields_pass() {
        let spec = build_spec(&json!({
            "face": "rsvp", "title": "Party", "event_at": "2026-12-12T18:00:00+01:00",
            "owner": "k_other", "host_key": "x", "space": "s", "owner_kwant_id": "k", "show_live_count": false, "waiting_list": false
        }))
        .unwrap();
        assert_eq!(spec, json!({"face": "rsvp", "title": "Party", "event_at": "2026-12-12T18:00:00+01:00", "closes_at": "2026-12-12T18:00:00+01:00"}));
        let own = build_spec(&json!({"face": "vote", "title": "T", "event_at": "2026-12-12T18:00:00+01:00", "closes_at": "2026-12-05T18:00:00+01:00"})).unwrap();
        assert_eq!(own["closes_at"], "2026-12-05T18:00:00+01:00", "a given closes_at stays");
    }

    #[test]
    fn no_title_asks_for_the_rules_and_a_wrapped_spec_is_read() {
        assert_eq!(gather_arg(&json!({})), Ok(None));
        assert_eq!(gather_arg(&json!({"face": "rsvp"})), Ok(None));
        assert_eq!(gather_arg(&json!({"spec": null})), Ok(None));
        assert_eq!(gather_arg(&json!({"spec": {"title": "P"}})), Ok(Some(json!({"title": "P"}))));
        assert_eq!(gather_arg(&json!({"spec": "{\"title\": \"P\"}"})), Ok(Some(json!({"title": "P"}))));
        assert!(gather_arg(&json!({"spec": "not json"})).is_err());
        assert!(gather_arg(&json!({"spec": [1]})).is_err());
    }

    /// Every `{"face"` sample in RULES.
    fn samples() -> Vec<Value> {
        let mut out = Vec::new();
        let mut rest = RULES;
        while let Some(start) = rest.find("{\"face\"") {
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
            out.push(serde_json::from_str::<Value>(&rest[start..end]).unwrap());
            rest = &rest[end..];
        }
        out
    }

    #[test]
    fn the_rules_and_description_name_what_a_bot_never_does_and_the_samples_are_specs_the_kit_takes() {
        for f in FIELDS {
            assert!(RULES.contains(&format!("- {f}")), "{f}");
        }
        for w in [
            "at most one question",
            "Should guests see who's coming",
            "Who's voting?",
            "Never invent a price, a place, a time or a list of people",
            "Never say a field name",
            "If the day or time is missing (for a vote: when voting closes), or the day of the week and the date don't agree, ask only \
             about that, and use the defaults for the rest: names shown, anyone with the link can vote.",
            "\"Who's voting? Give me their first names and I'll show you who hasn't voted yet. Anyone with the link can vote either way.\"",
            "with {name}, {title}, {when}, {place}, {host}, {slot} and {link} filled in.",
            "never an RFC 3339 time",
            "ask them which one they mean",
            "written now because nothing writes them later",
            "Anyone with the link can still answer",
        ] {
            assert!(RULES.contains(w), "{w}");
        }
        assert!(!RULES.contains("make up"), "nothing in the rules tells the bot to make anything up");
        assert!(!RULES.contains("leave it open"), "naming voters never sounds like it closes the vote");
        let d = tool()["description"].as_str().unwrap().to_string();
        for w in ["RSVP", "who's coming", "sign-up sheet", "potluck", "slots", "volunteer", "vote", "poll", "secret ballot", "ranked"] {
            assert!(d.contains(w), "{w}");
        }
        for w in ["Ask the person at most one question", "Never invent a price, a place, a time or a list of people"] {
            assert!(d.contains(w), "{w} in {d}");
        }
        let s = samples();
        assert_eq!(s.len(), 3);
        let faces: Vec<&str> = s.iter().map(|x| x["face"].as_str().unwrap()).collect();
        assert_eq!(faces, ["rsvp", "signup", "vote"]);
        for x in &s {
            let spec = build_spec(x).unwrap();
            // The kit checks closes_at against the real clock: the samples' dates move to 2099 so the test never goes stale
            let later: Value = serde_json::from_str(&spec.to_string().replace("2026-12-", "2099-12-")).unwrap();
            assert_eq!(gather::Gather.validate_spec(&later), Ok(()), "{later}");
        }
        // The RSVP sample's when names event_at's day (Dec 12 2026 is a Saturday)
        assert_eq!(weekday_in(s[0]["when"].as_str().unwrap()), Some(Weekday::Sat));
    }

    #[test]
    fn the_made_text_starts_with_the_head_and_code_and_carries_no_host_link() {
        let spec = json!({"face": "vote", "closes_at": "2026-12-05T18:00:00+01:00"});
        let join = json!({"mode": "anywhere", "anywhere_url": "https://box.example.com/p/AB12CD"});
        let t = made_text("AB12CD", "Party theme", &spec, Some(&join));
        assert!(t.starts_with("Pocket app · Gather [AB12CD] Vote \"Party theme\" is open."), "{t}");
        assert!(t.contains("Guest link: https://box.example.com/p/AB12CD"), "{t}");
        assert!(t.contains("secret ballot"), "{t}");
        assert!(!t.contains("#remote=") && !t.contains("#screen="), "{t}");
        let none = made_text("AB12CD", "P", &json!({"face": "rsvp"}), None);
        assert!(none.contains("No guest link works yet") && none.contains("don't write a link yourself"), "{none}");
    }
}
