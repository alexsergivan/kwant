//! santa-create-from-chat (autonomy/santa-create-from-chat/brief.md): the bot's one Secret Santa
//! tool, `santa_create`, offered beside the Quizmaster's tools and behind the same switch
//! (`pocket_apps`). Before it, asking a bot for a Secret Santa ended with "I can only make
//! quizzes", and a Santa made through `POST /api/pockets` never showed its card in chat.
//!
//! - With no spec it answers with the spec's fields and rules ([`RULES`]).
//! - With a spec it validates it with the kit (`pocket/santa.rs`, whose refusals are already
//!   sentences a bot can act on) and makes the pocket with `pocket::create`: kit `santa`, owned by
//!   the calling bot, with the turn's own chat as its origin (never one named in the arguments).
//! - Its answer starts with [`CARD_TEXT_HEAD`] and the pocket's code. The reply that carries the
//!   call gets a `pocket` block with a `santa` part ([`card`]), built by `smart_results::pocket`
//!   from the stored pocket, never from the call's text. The block carries counts and first names
//!   only, never a match or a question's words (the kit's host view already holds none).
//!
//! There is no draft, dry run or source check: a Santa has no answers to check, and the kit's
//! validator is the whole check. Each bot reaches only the Santas it made (the card is attached
//! only to the owner's own replies).

use super::continuity::SelfScope;
use super::quizmaster_tools::{self as qt, PocketOrigin};
use super::AppState;
use crate::pocket::kits::{self, spec_date, KitCtx, Player, Viewer};
use crate::pocket::schedule::{self, Zone};
use crate::pocket::{santa, CreateRequest};
use chrono::{DateTime, NaiveDate, Utc};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

type ToolResult = (String, bool);

pub const TOOL: &str = "santa_create";
/// What a made Santa's answer starts with, then its code and `]`.
pub const CARD_TEXT_HEAD: &str = "Pocket app · Secret Santa [";
/// The largest spec a bot may hand in (a Santa of 60 names and 60 pairs is a few KB).
const MAX_SPEC_BYTES: usize = 16 * 1024;

pub fn handles(name: &str) -> bool {
    name == TOOL
}

pub fn tool() -> Value {
    json!({
        "name": TOOL,
        "description": "Make a Secret Santa (also called a gift exchange, Kris Kringle or Wichteln) for a family, office or group of friends. \
            People join on their phones with a first name and a PIN, kwant draws the names and keeps apart any couples the person names, and each \
            person sees only who they give to. Call it first with no spec to get the fields and rules, then with the whole spec: it checks it, opens \
            the Secret Santa and shows its card, with the invite, in the person's chat. Never draw names yourself or post who gets whom.",
        "inputSchema": {"type": "object", "properties": {
            "spec": {"type": "object", "description": "The Secret Santa (title, gift_on, reveal_at, people, keep_apart, budget…), written from the rules santa_create gives you"}
        }},
    })
}

/// What a call with no spec answers: the fields and rules (pocket/santa.rs's module doc, for a
/// bot) and a sample.
pub const RULES: &str = "Secret Santa: the fields and rules.

kwant runs the whole Secret Santa: people join on their phones with a first name and a 4-digit PIN, kwant draws the names (nobody gets \
themselves, and the pairs in keep_apart never get each other), and from then on each person sees only their own match. The person who asked \
(the host) sees who has joined, who has looked and who has a gift ready, never a match. If the host takes part, they join with the invite like \
everyone else. You never draw names, guess or post who gets whom, and you can't see a match either.

Fields (one JSON object):
- title (required): what people see, like \"Office Secret Santa\".
- gift_on (required): the day gifts are given, YYYY-MM-DD.
- reveal (true or false, default true): whether everyone sees who gave to whom at reveal_at. Set it to false only if the person wants the \
matches kept secret for good.
- reveal_at (required while reveal is true): RFC 3339 with the person's offset. Use 7 PM on the gift day in the person's time zone unless they \
said another time, like 2026-12-18T19:00:00+00:00. It can't be in the past or more than a day before the gift day.
- people (optional): the first names you expect, like [\"Anna\", \"Ben\"]. The card shows who hasn't joined yet.
- expected (optional, 3 to 60): how many people take part, counting the person who asked if they're in it. Names are drawn as soon as that \
many have joined. Without it, the number of names in people is used; with neither, names are drawn only when the host presses Draw now on the card.
- draw_at (optional): RFC 3339; names are drawn then, even if someone hasn't joined. It must be before the gift day.
- keep_apart (optional): pairs who must not draw each other, like [[\"Anna\", \"Ben\"]] for a couple.
- budget (optional, at most 24 characters): the spending limit as the person said it, like \"£20\".
- host_name (optional): the host's first name.

Fill what you can: make up the title and set reveal_at from the gift day. If the person didn't give the gift day, ask for it in one short \
question and ask nothing else. Dates can't change once it's open, so say them back in your reply.

Sample (its names and dates are samples; write the person's own):
{\"title\": \"Team Secret Santa\", \"gift_on\": \"2026-12-18\", \"reveal\": true, \"reveal_at\": \"2026-12-18T19:00:00+00:00\", \
\"people\": [\"Anna\", \"Ben\", \"Chloe\", \"Dev\", \"Erin\", \"Femi\"], \"keep_apart\": [[\"Anna\", \"Ben\"]], \"budget\": \"£20\"}

Call santa_create again with spec set to the whole Secret Santa.";

fn fail(msg: impl Into<String>) -> ToolResult {
    (msg.into(), true)
}

/// The spec argument: an object, or a string holding one.
fn spec_arg(args: &Value) -> Result<Option<Value>, String> {
    let v = match args.get("spec") {
        None | Some(Value::Null) => return Ok(None),
        Some(Value::String(s)) => serde_json::from_str::<Value>(s).map_err(|e| format!("`spec` isn't valid JSON: {e}"))?,
        Some(v) => v.clone(),
    };
    if !v.is_object() {
        return Err("`spec` must be a JSON object".into());
    }
    if v.to_string().len() > MAX_SPEC_BYTES {
        return Err(format!("the spec is bigger than {} KB", MAX_SPEC_BYTES / 1024));
    }
    Ok(Some(v))
}

pub async fn call(state: &AppState, scope: &SelfScope, args: &Value) -> ToolResult {
    if !qt::enabled(state).await {
        return fail("Pocket apps are off on this kwant.");
    }
    let spec = match spec_arg(args) {
        Ok(Some(s)) => s,
        Ok(None) => return (RULES.to_string(), false),
        Err(e) => return fail(e),
    };
    if state.db.get_kwant(&scope.kwant_id).await.ok().flatten().is_none() {
        return fail("Could not find your space.");
    }
    let o = PocketOrigin::of(scope);
    let req = CreateRequest {
        kit: santa::ID.into(),
        spec,
        owner_kwant_id: Some(o.kwant_id.clone()),
        opens_at: None,
        closes_at: None,
        origin: Some(schedule::Origin { kwant_id: o.kwant_id, channel_id: o.channel_id, thread_id: o.thread_id }),
    };
    // The kit's refusal is a plain sentence ("The spec needs gift_on, the day gifts are given.")
    let (snap, _host_key) = match crate::pocket::create(state, req).await {
        Ok(s) => s,
        Err(e) => return fail(format!("Nothing was made: {} Fix the spec and call santa_create again.", e.message)),
    };
    let join = santa_join(&crate::pocket::address::join(state, &snap).await);
    remember_join(&snap.row.code, join.clone());
    let tz = state.db.get_setting(super::bot_actions::TIMEZONE_SETTING).await.ok().flatten();
    let players: Vec<Player> = snap.players.clone();
    let c = card(&CardInput {
        title: &snap.row.title,
        spec: &snap.spec,
        kit_state: &snap.kit_state,
        players: &players,
        ended: snap.row.state != crate::db::pocket::OPEN,
        ended_at: snap.row.ended_at,
        expires_at: snap.row.expires_at,
        tz: tz.as_deref(),
        now: Utc::now(),
        join: join.clone(),
    });
    (made_text(&snap.row.code, &c, join.as_ref()), false)
}

/// The answer to a call that made one: the head the chat card is found by, then what the bot
/// tells the person.
fn made_text(code: &str, c: &Card, join: Option<&Value>) -> String {
    let link = join.and_then(|j| j["anywhere_url"].as_str().or(j["wifi_url"].as_str()));
    let link_line = match (link, join.map(|j| j["mode"].as_str() == Some("wifi"))) {
        (Some(u), Some(true)) => format!(
            "Invite link: {u}\nIt works only on the same Wi-Fi as this computer; the card shows how to let people join from anywhere. \
             Tell the person to send it to everyone taking part."
        ),
        (Some(u), _) => format!("Invite link: {u}\nTell the person to send it to everyone taking part."),
        (None, _) => "No invite link works yet: the card shows the person how to set one up. Don't write a link yourself. \
                      Once the card has one, the person sends it to everyone taking part."
            .into(),
    };
    let rule = c.santa["draw_rule"].as_str().unwrap_or("when the person presses Draw now on the card");
    format!(
        "{CARD_TEXT_HEAD}{code}] \"{name}\" is open.\n\
         The person's chat already shows its card (who has joined, the invite and the draw), so keep your reply to two or three lines and \
         don't repeat what's on it.\n\
         {link_line}\n\
         Names are drawn {rule}.\n\
         Never draw names yourself, guess or post who gets whom: until the reveal, each person sees only their own match, and you never see any.",
        name = c.name
    )
}

// ---- The join links ----------------------------------------------------------------------

/// The join block `santa_create` worked out, by code, for the reply that carries the call (the
/// card builder has the database only, not the listener or Funnel). Kept 10 minutes; a reply
/// stored later, or after a restart, gets the card without links.
type Joins = Mutex<HashMap<String, (Instant, Option<Value>)>>;

fn joins() -> &'static Joins {
    static JOINS: std::sync::OnceLock<Joins> = std::sync::OnceLock::new();
    JOINS.get_or_init(Default::default)
}

const JOIN_KEEP: Duration = Duration::from_secs(600);

fn remember_join(code: &str, join: Option<Value>) {
    let mut m = joins().lock().unwrap_or_else(|e| e.into_inner());
    m.retain(|_, (at, _)| at.elapsed() < JOIN_KEEP);
    m.insert(code.to_string(), (Instant::now(), join));
}

/// The join `santa_create` worked out for `code`, while it's fresh.
pub(crate) fn join_for(code: &str) -> Option<Value> {
    let m = joins().lock().unwrap_or_else(|e| e.into_inner());
    m.get(code).filter(|(at, _)| at.elapsed() < JOIN_KEEP).and_then(|(_, j)| j.clone())
}

/// The most a join's Wi-Fi note may hold: the frontend's `isText(v, 40)` (Pocket.tsx).
const WIFI_NOTE_MAX: usize = 40;

/// `address::join` cut to what the Santa card reads (`validSantaJoin`): never a quick tunnel (a
/// Santa lasts weeks, and its kit refuses one), and "anywhere" only with an address and its kind.
/// The Wi-Fi note ("this_computer_only" on a box that only answers 127.0.0.1) goes through when it
/// is a short word, so the card says "only this computer" from its first paint, live read or not.
pub(crate) fn santa_join(j: &Value) -> Option<Value> {
    let url = |k: &str| {
        j[k].as_str()
            .filter(|u| u.len() <= 2048 && (u.starts_with("https://") || u.starts_with("http://")))
            .filter(|u| !u.contains(".trycloudflare.com") && !u.split('/').nth(2).unwrap_or("").contains('@'))
    };
    let kind = j["anywhere_kind"].as_str().filter(|k| matches!(*k, "https" | "funnel"));
    let anywhere = url("anywhere_url").filter(|_| kind.is_some());
    let wifi = url("wifi_url");
    let mode = if anywhere.is_some() {
        "anywhere"
    } else if wifi.is_some() {
        "wifi"
    } else {
        "none"
    };
    let wifi_note = j["wifi_note"].as_str().filter(|n| {
        !n.trim().is_empty() && n.encode_utf16().count() <= WIFI_NOTE_MAX && !n.contains(['<', '>']) && !n.chars().any(char::is_control)
    });
    Some(json!({ "mode": mode, "anywhere_kind": anywhere.and(kind), "anywhere_url": anywhere, "wifi_url": wifi, "wifi_note": wifi_note }))
}

// ---- The card ----------------------------------------------------------------------------

/// What the card is built from: the stored pocket.
pub(crate) struct CardInput<'a> {
    pub title: &'a str,
    pub spec: &'a Value,
    pub kit_state: &'a Value,
    pub players: &'a [Player],
    /// The pocket isn't open any more.
    pub ended: bool,
    pub ended_at: Option<DateTime<Utc>>,
    pub expires_at: Option<DateTime<Utc>>,
    pub tz: Option<&'a str>,
    pub now: DateTime<Utc>,
    pub join: Option<Value>,
}

/// The Santa card's parts of the `pocket` block.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Card {
    pub name: String,
    pub when: Option<String>,
    /// `live` or `ended`.
    pub state: &'static str,
    /// The block's `santa` (frontend `validSanta`, pa10a card-impl.md "Shape I read").
    pub santa: Value,
}

/// "Fri, Dec 18".
fn day(d: NaiveDate) -> String {
    d.format("%a, %b %-d").to_string()
}

/// "Sun, Dec 6 at 6 PM".
fn day_at(zone: &Zone, t: DateTime<Utc>) -> String {
    let l = zone.local(t);
    format!("{} at {}", day(l.date()), schedule::clock(l))
}

/// The gift day as the person reads it: a plain date as written, a time in their zone.
fn gift_day(spec: &Value, zone: &Zone) -> Option<NaiveDate> {
    let v = spec.get("gift_on")?;
    match v.as_str().map(str::trim).and_then(|t| NaiveDate::parse_from_str(t, "%Y-%m-%d").ok()) {
        Some(d) => Some(d),
        None => spec_date(v).map(|t| zone.local(t).date()),
    }
}

/// A short text from the kit's view, cleaned the way the chat checks texts (no `<` or `>`, no
/// control characters) and cut to `max` UTF-16 units (the client's length), or `None`.
fn short(v: &Value, max: usize) -> Option<String> {
    clean(v.as_str()?, max)
}

fn clean(s: &str, max: usize) -> Option<String> {
    let mut out = String::new();
    let mut units = 0;
    for c in s.trim().chars() {
        let c = match c {
            '<' => '\u{2039}',
            '>' => '\u{203a}',
            c if c.is_control() => ' ',
            c => c,
        };
        units += c.len_utf16();
        if units > max {
            break;
        }
        out.push(c);
    }
    let out = out.trim().to_string();
    (!out.is_empty()).then_some(out)
}

/// A count from the kit's view, never above `joined`.
fn count(v: &Value, joined: u64) -> Option<u64> {
    v.as_u64().map(|n| n.min(joined))
}

/// A guest creature id the card can draw.
fn avatar(v: &Value) -> Value {
    match v.as_str() {
        Some(a) if (1..=32).contains(&a.len()) && a.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_' || b == b'-') => json!(a),
        _ => Value::Null,
    }
}

/// The card, from the kit's own host view (counts and per-person ticks, never a match) mapped
/// onto the shape the chat draws. `None`-free: a pocket whose kit state can't be read shows as
/// collecting with nobody in.
pub(crate) fn card(i: &CardInput) -> Card {
    let kit = kits::kit(santa::ID).expect("the santa kit is built in");
    let ctx = KitCtx::new(i.spec, i.players, i.now);
    let host = kit.view(&ctx, i.kit_state, &Viewer::Host);
    let reveal_t = i.spec.get("reveal_at").and_then(spec_date);
    let reveal_fixed = i.spec.get("reveal_at").and_then(Value::as_str).and_then(|t| DateTime::parse_from_rfc3339(t.trim()).ok());
    let zone = schedule::zone_for(i.tz, reveal_fixed);
    let gift = gift_day(i.spec, &zone);
    let kit_phase = host["phase"].as_str().unwrap_or(santa::COLLECTING);
    let phase = if i.ended || kit_phase == santa::REVEALED {
        "ended"
    } else if kit_phase == santa::DRAWN && gift.is_some_and(|g| zone.local(i.now).date() >= g) {
        "reveal_day"
    } else if kit_phase == santa::DRAWN {
        "drawn"
    } else {
        "collecting"
    };
    let joined = host["joined"].as_u64().unwrap_or(i.players.len() as u64);
    let expected = host["expected"].as_u64().filter(|n| *n > 0);
    let names = |v: &Value| -> Vec<String> { v.as_array().into_iter().flatten().filter_map(|n| short(n, 40)).take(60).collect() };
    let people: Vec<Value> = host["people"]
        .as_array()
        .into_iter()
        .flatten()
        .take(60)
        .filter_map(|p| {
            Some(json!({
                "id": short(&p["id"], 64)?,
                "name": short(&p["name"], 40)?,
                "avatar": avatar(&p["avatar"]),
                "looked": p["looked"].as_bool(),
                "gift_ready": p["gift_ready"].as_bool(),
            }))
        })
        .collect();
    let keep_apart: Vec<Value> = host["keep_apart"]
        .as_array()
        .into_iter()
        .flatten()
        .take(60)
        .filter_map(|p| Some(json!([short(p.get(0)?, 40)?, short(p.get(1)?, 40)?])))
        .collect();
    let draw_at = i.spec.get("draw_at").and_then(spec_date);
    let draw_rule = match (expected, draw_at) {
        (Some(n), Some(t)) => Some(format!("when all {n} are in, or {}, whichever comes first", day_at(&zone, t))),
        (Some(n), None) => Some(format!("when all {n} are in")),
        (None, Some(t)) => Some(format!("on {}", day_at(&zone, t))),
        (None, None) => None,
    };
    let after_draw = phase != "collecting";
    let counts = &host["counts"];
    let collecting = phase == "collecting";
    let at = |v: &Value| v.as_str().and_then(|t| DateTime::parse_from_rfc3339(t).ok()).map(|t| day_at(&zone, t.with_timezone(&Utc)));
    let reveal = short(&host["reveal"], 12).filter(|r| matches!(r.as_str(), "scheduled" | "shown" | "skipped" | "never")).unwrap_or_else(|| "scheduled".into());
    let santa = json!({
        "phase": phase,
        "joined": joined,
        "expected": expected,
        "waiting_for": if collecting { names(&host["waiting_for"]) } else { Vec::new() },
        "people": people,
        "looked": if after_draw { count(&counts["looked"], joined) } else { None },
        "gifts_ready": if after_draw { count(&counts["gifts_ready"], joined) } else { None },
        "asks_asked": if after_draw { counts["questions_asked"].as_u64() } else { None },
        "asks_answered": if after_draw { counts["questions_answered"].as_u64().map(|a| a.min(counts["questions_asked"].as_u64().unwrap_or(0))) } else { None },
        "keep_apart": keep_apart,
        "draw_rule": if collecting { draw_rule } else { None },
        "drawn_at": at(&host["drawn_at"]),
        "reveal": reveal,
        "reveal_at": reveal_t.filter(|_| reveal == "scheduled").map(|t| schedule::clock(zone.local(t))),
        "revealed_at": at(&host["revealed_at"]),
        "deletes_on": i.expires_at.map(|t| day(zone.local(t).date())),
        "ended_on": if phase == "ended" { i.ended_at.map(|t| zone.local(t).date().format("%b %-d").to_string()) } else { None },
        "can_draw": collecting && host["draw"]["can_draw"].as_bool() == Some(true),
        "blocked": if collecting { short(&host["draw"]["blocked"]["message"], 300) } else { None },
        "join": if phase == "ended" { None } else { i.join.clone() },
    });
    let mut when: Vec<String> = Vec::new();
    if let Some(b) = short(&i.spec["budget"], 24) {
        when.push(format!("{b} each"));
    }
    if let Some(g) = gift {
        when.push(format!("gifts on {}", day(g)));
    }
    if let (Some(t), "scheduled") = (reveal_t, reveal.as_str()) {
        when.push(format!("reveal at {}", schedule::clock(zone.local(t))));
    }
    let name = clean(i.title, 80).unwrap_or_else(|| "Secret Santa".into());
    Card { name, when: clean(&when.join(" · "), 160), state: if phase == "ended" { "ended" } else { "live" }, santa }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pocket::kits::Kit;

    fn mira_spec() -> Value {
        json!({"title": "Team Secret Santa", "gift_on": "2099-12-18", "reveal": true, "reveal_at": "2099-12-18T19:00:00+00:00",
               "people": ["Anna", "Ben", "Chloe", "Dev", "Erin", "Femi"], "keep_apart": [["Anna", "Ben"]], "budget": "£20"})
    }

    fn input<'a>(spec: &'a Value, st: &'a Value, players: &'a [Player], join: Option<Value>) -> CardInput<'a> {
        CardInput { title: "Team Secret Santa", spec, kit_state: st, players, ended: false, ended_at: None, expires_at: None, tz: Some("Europe/London"), now: Utc::now(), join }
    }

    #[test]
    fn a_new_santa_is_collecting_with_everyone_still_to_join() {
        let spec = mira_spec();
        assert_eq!(santa::Santa.validate_spec(&spec), Ok(()));
        let st = santa::Santa.init(&spec);
        let c = card(&input(&spec, &st, &[], None));
        assert_eq!((c.state, c.name.as_str()), ("live", "Team Secret Santa"));
        assert_eq!(c.when.as_deref(), Some("£20 each · gifts on Fri, Dec 18 · reveal at 7 PM"));
        let s = &c.santa;
        assert_eq!(s["phase"], "collecting");
        assert_eq!((s["joined"].as_u64(), s["expected"].as_u64()), (Some(0), Some(6)));
        assert_eq!(s["waiting_for"], json!(["Anna", "Ben", "Chloe", "Dev", "Erin", "Femi"]));
        assert_eq!(s["keep_apart"], json!([["Anna", "Ben"]]));
        assert_eq!(s["draw_rule"], "when all 6 are in");
        assert_eq!(s["reveal_at"], "7 PM");
        assert!(s["looked"].is_null() && s["gifts_ready"].is_null(), "no counts before the draw");
    }

    #[test]
    fn a_quick_tunnel_never_reaches_the_card_and_anywhere_needs_its_kind() {
        let j = santa_join(&json!({"mode": "anywhere", "anywhere_url": "https://x.trycloudflare.com/p/abc", "anywhere_kind": "quick", "wifi_url": "http://192.168.1.2:7480/p/abc", "quick_tunnel": false, "tunnel": {"state": "off"}})).unwrap();
        assert_eq!(j, json!({"mode": "wifi", "anywhere_kind": null, "anywhere_url": null, "wifi_url": "http://192.168.1.2:7480/p/abc", "wifi_note": null}));
        let j = santa_join(&json!({"mode": "anywhere", "anywhere_url": "https://mac.ts.net/p/abc", "anywhere_kind": "funnel", "wifi_url": null})).unwrap();
        assert_eq!((j["mode"].as_str(), j["anywhere_kind"].as_str()), (Some("anywhere"), Some("funnel")));
        let j = santa_join(&json!({"mode": "none", "anywhere_url": null, "wifi_url": null})).unwrap();
        assert_eq!(j["mode"], "none");
    }

    /// santa-join-wifi-note: a box that only answers 127.0.0.1 has no Wi-Fi link, and its note
    /// reaches the card from chat, so "only this computer" shows before (or without) the live read.
    #[test]
    fn a_box_that_only_answers_this_computer_keeps_its_wifi_note() {
        let from = crate::pocket::address::choose("abcdef", None, None, None, None, Some("this_computer_only"));
        let j = santa_join(&from).unwrap();
        assert_eq!(j, json!({"mode": "none", "anywhere_kind": null, "anywhere_url": null, "wifi_url": null, "wifi_note": "this_computer_only"}));
        // ...and it is in the card's block from the first paint
        let spec = mira_spec();
        let st = santa::Santa.init(&spec);
        let c = card(&input(&spec, &st, &[], Some(j)));
        assert_eq!(c.santa["join"]["wifi_note"], "this_computer_only");
        // With Funnel on the same box the note still rides along (the card words the Wi-Fi hint)
        let from = crate::pocket::address::choose("abcdef", None, Some("https://mac.ts.net"), None, None, Some("this_computer_only"));
        let j = santa_join(&from).unwrap();
        assert_eq!((j["mode"].as_str(), j["wifi_note"].as_str()), (Some("anywhere"), Some("this_computer_only")));
        // Any other short note (no_network, listener_off) goes through as is; the card reads only its own
        let j = santa_join(&json!({"mode": "none", "anywhere_url": null, "wifi_url": null, "wifi_note": "no_network"})).unwrap();
        assert_eq!(j["wifi_note"], "no_network");
    }

    #[test]
    fn a_long_or_odd_wifi_note_is_dropped() {
        let base = |note: Value| santa_join(&json!({"mode": "none", "anywhere_url": null, "wifi_url": null, "wifi_note": note})).unwrap();
        assert_eq!(base(json!("a".repeat(WIFI_NOTE_MAX)))["wifi_note"].as_str().map(str::len), Some(WIFI_NOTE_MAX), "40 fits");
        assert!(base(json!("é".repeat(WIFI_NOTE_MAX)))["wifi_note"].is_string(), "counted in UTF-16 units, like the client, not bytes");
        for bad in [json!("a".repeat(WIFI_NOTE_MAX + 1)), json!("😀".repeat(WIFI_NOTE_MAX / 2 + 1)), json!(""), json!("   "), json!("<b>x</b>"), json!("a\nb"), json!(7), json!({"x": 1}), Value::Null] {
            assert!(base(bad.clone())["wifi_note"].is_null(), "{bad}");
        }
        // No note at all is null, never a missing key: the block keeps one shape
        let j = santa_join(&json!({"mode": "none"})).unwrap();
        assert!(j.as_object().unwrap().contains_key("wifi_note") && j["wifi_note"].is_null());
    }

    #[test]
    fn the_rules_name_every_field_and_the_one_thing_a_bot_never_does() {
        for f in ["title", "gift_on", "reveal_at", "people", "expected", "draw_at", "keep_apart", "budget", "host_name"] {
            assert!(RULES.contains(&format!("- {f}")), "{f}");
        }
        assert!(RULES.contains("never draw names"));
        let d = tool()["description"].as_str().unwrap().to_string();
        for w in ["Secret Santa", "gift exchange", "Kris Kringle", "Wichteln", "Never draw names yourself or post who gets whom"] {
            assert!(d.contains(w), "{w}");
        }
        // The sample in the rules is a spec the kit takes (with its dates moved ahead)
        let start = RULES.find("{\"title\"").unwrap();
        let end = start + RULES[start..].find('}').unwrap() + 1;
        let mut sample: Value = serde_json::from_str(&RULES[start..end]).unwrap();
        sample["gift_on"] = json!("2099-12-18");
        sample["reveal_at"] = json!("2099-12-18T19:00:00+00:00");
        assert_eq!(santa::Santa.validate_spec(&sample), Ok(()));
    }
}
