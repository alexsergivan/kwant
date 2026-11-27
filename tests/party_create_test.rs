//! party-create-from-chat (autonomy/party-create-from-chat/brief.md): "Set up an RSVP for the
//! office holiday party", "Potluck: 3 mains, 4 desserts, 2 drinks", "a theme vote" and "Set up a
//! photo wall for the party" in chat. The bot's `gather_create` and `wall_create` tools sit beside
//! `santa_create` and `desk_create`, each behind the kit-aware switch (gate `gather` / `wall`).
//! With nothing to make they answer with the fields and rules; with a whole one they make a pocket
//! of kit `gather` or `wall` owned by the bot, with the turn's chat as its origin, and answer with
//! the head, the code and the guest link. Bad input answers "Nothing was made" in the kit's own
//! words, only known fields reach the kit (never `when` for a wall), and another bot never gets
//! this bot's pocket.

mod quizmaster_support;

use chrono::{Datelike, Duration, Local, NaiveDate, Weekday};
use kwnt::db::models::{Kwant, Message, MessageExtra, NewMessage};
use kwnt::mcp::SELF_SERVER_NAME;
use kwnt::pocket::kits::Viewer;
use kwnt::server::continuity::TurnOrigin;
use quizmaster_support::{setup, tool, tool_names, turn, turn_in, Env};
use serde_json::{json, Value};

const GUEST_BASE: &str = "https://party.example.com";

/// The first Saturday at least 14 days ahead (so the tests never go stale), and its `when` line.
fn party_day() -> (NaiveDate, String) {
    let mut d = Local::now().date_naive() + Duration::days(14);
    while d.weekday() != Weekday::Sat {
        d += Duration::days(1);
    }
    (d, d.format("%a, %b %-d, 6 PM").to_string())
}

fn at(d: NaiveDate, hm: &str) -> String {
    format!("{}T{hm}:00+01:00", d.format("%Y-%m-%d"))
}

/// a01: "Set up an RSVP for the office holiday party, Sat 6 PM, EUR 15 each, ask about plus-ones
/// and food." The bot asked whether guests should see who's coming; the person said no.
fn rsvp() -> Value {
    let (d, when) = party_day();
    json!({"face": "rsvp", "title": "Office holiday party", "when": when, "place": "The Old Mill", "host_name": "Nina",
           "event_at": at(d, "18:00"), "price": "EUR 15", "fields": ["plus_ones", "food_needs"], "show_names": false})
}

/// a02: "Potluck: 3 mains, 4 desserts, 2 drinks."
fn potluck() -> Value {
    let (d, _) = party_day();
    json!({"face": "signup", "title": "Potluck", "event_at": at(d, "18:00"),
           "slots": [{"name": "Mains", "cap": 3}, {"name": "Desserts", "cap": 4}, {"name": "Drinks", "cap": 2}], "fields": ["slot_note"]})
}

const THEMES: [&str; 5] = ["Winter wonderland", "Ugly sweaters", "Casino night", "Roaring twenties", "Masquerade"];

/// a02: "and a theme vote", ranked, for the three people the person named.
fn vote() -> Value {
    let (d, _) = party_day();
    json!({"face": "vote", "title": "Party theme", "closes_at": at(d - Duration::days(7), "18:00"),
           "ballot": {"kind": "ranked", "options": THEMES}, "members": ["Ana", "Ben", "Cleo"]})
}

/// a03: "Set up a photo wall for the party."
fn photo_wall() -> Value {
    let (d, _) = party_day();
    json!({"title": "Office holiday party", "host_name": "Nina", "event_at": at(d, "18:00"), "closes_at": at(d + Duration::days(1), "12:00")})
}

async fn with_guest_address(e: &Env) {
    e.state.db.set_setting(kwnt::pocket::address::PUBLIC_URL_SETTING, GUEST_BASE).await.unwrap();
}

fn code_after<'a>(text: &'a str, head: &str) -> &'a str {
    let rest = text.strip_prefix(head).unwrap_or_else(|| panic!("not made by {head}: {text}"));
    &rest[..rest.find(']').unwrap()]
}

/// Makes `args` with `tool_name` as `bot` in its own direct chat; returns (code, the tool's text).
async fn make_as(e: &Env, bot: &Kwant, tool_name: &str, args: Value) -> (String, String) {
    let origin = TurnOrigin { direct_kwant_id: Some(bot.id.clone()), ..Default::default() };
    let token = turn_in(e, bot, &format!("t-{tool_name}-{}-{}", bot.id, rand_id()), origin).await;
    let (text, err) = tool(e, &token, tool_name, args).await;
    assert!(!err, "{text}");
    let head = if tool_name == "wall_create" { "Pocket app · Photo wall [" } else { "Pocket app · Gather [" };
    (code_after(&text, head).to_string(), text)
}

fn rand_id() -> String {
    format!("{:x}", std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos())
}

async fn make(e: &Env, tool_name: &str, args: Value) -> (String, String) {
    let bot = e.bot.clone();
    make_as(e, &bot, tool_name, args).await
}

async fn stored_spec(e: &Env, code: &str) -> Value {
    let row = e.state.db.pocket_get(code).await.unwrap().expect("the pocket exists");
    serde_json::from_str(&row.spec).unwrap()
}

/// The pocket's host view, as the person's card and remote read it.
async fn host_view(e: &Env, code: &str) -> Value {
    let s = kwnt::pocket::snapshot(&e.state, code).await.unwrap_or_else(|err| panic!("{}", err.message)).expect("a snapshot");
    kwnt::pocket::view_json(&s, &Viewer::Host, chrono::Utc::now())
}

/// `bot`'s final reply in `direct`'s chat, with the turn's tool calls.
async fn reply_as(e: &Env, bot: &Kwant, direct: &str, tool_name: &str, args: Value, result: &str) -> Message {
    let call = json!({"id": format!("call-{tool_name}"), "name": format!("{SELF_SERVER_NAME}__{tool_name}"), "server": SELF_SERVER_NAME,
                      "tool": tool_name, "arguments": args, "status": "ok", "result": result});
    let msg = NewMessage {
        channel_id: None,
        direct_kwant_id: Some(direct.into()),
        sender_role: "kwant".into(),
        sender_id: bot.name.clone(),
        text: "It's open: send the link to everyone.".into(),
        reasoning_content: None,
        tool_calls: Some(json!([call])),
        tokens_used: 0,
    };
    e.state.db.insert_message_with(&msg, &MessageExtra::default()).await.unwrap()
}

fn pocket_block(m: &Message) -> Option<Value> {
    let b = m.results.as_ref()?.get("blocks")?.get(0)?.clone();
    (b["type"] == json!("pocket")).then_some(b)
}

/// Whether `v` holds, anywhere, an array of exactly these numbers.
fn holds_numbers(v: &Value, want: &[u64]) -> bool {
    match v {
        Value::Array(a) => a.iter().map(Value::as_u64).collect::<Option<Vec<u64>>>().as_deref() == Some(want) || a.iter().any(|x| holds_numbers(x, want)),
        Value::Object(o) => o.values().any(|x| holds_numbers(x, want)),
        _ => false,
    }
}

#[tokio::test]
async fn each_tool_is_offered_only_while_the_switch_lets_its_kit_through() {
    let e = setup(false).await;
    let names = tool_names(&e, &turn(&e, &e.bot, "t-off").await).await;
    assert!(!names.iter().any(|n| n == "gather_create" || n == "wall_create"), "{names:?}");
    for (value, gather, wall) in [
        ("santa", false, false),
        ("quiz", false, false),
        ("desk", false, false),
        ("gather", true, false),
        ("wall", false, true),
        ("santa, Gather, WALL", true, true),
        ("on", true, true),
        ("off", false, false),
    ] {
        e.state.db.set_setting(kwnt::pocket::SETTING, value).await.unwrap();
        let names = tool_names(&e, &turn(&e, &e.bot, &format!("t-{value}")).await).await;
        assert_eq!(names.iter().any(|n| n == "gather_create"), gather, "{value}: {names:?}");
        assert_eq!(names.iter().any(|n| n == "wall_create"), wall, "{value}: {names:?}");
        if value == "santa" {
            assert!(names.iter().any(|n| n == "santa_create"), "santa alone still offers santa_create: {names:?}");
        }
    }
}

#[tokio::test]
async fn the_descriptions_answer_every_name_for_an_rsvp_a_sign_up_a_vote_and_a_photo_wall() {
    let e = setup(true).await;
    let token = turn(&e, &e.bot, "t-desc").await;
    let v = quizmaster_support::rpc(&e, &token, "tools/list", json!({})).await;
    let tools = v["result"]["tools"].as_array().unwrap();
    let desc = |name: &str| tools.iter().find(|t| t["name"] == name).unwrap_or_else(|| panic!("{name} offered"))["description"].as_str().unwrap().to_string();
    let g = desc("gather_create");
    for w in ["RSVP", "who's coming", "sign-up sheet", "potluck", "slots", "volunteer", "vote", "poll", "secret ballot", "ranked"] {
        assert!(g.contains(w), "{w} in {g}");
    }
    let w = desc("wall_create");
    for word in ["photo wall", "party photos", "slideshow", "TV"] {
        assert!(w.contains(word), "{word} in {w}");
    }
}

#[tokio::test]
async fn with_nothing_to_make_each_answers_its_fields_and_rules_and_makes_nothing() {
    let e = setup(true).await;
    let token = turn(&e, &e.bot, "t-rules").await;
    for args in [json!({}), json!({"spec": null}), json!({"title": "  "}), json!({"face": "rsvp"})] {
        let (text, err) = tool(&e, &token, "gather_create", args.clone()).await;
        assert!(!err, "{text}");
        for f in ["face", "title", "when", "event_at", "closes_at", "slots", "ballot", "members", "show_names", "templates", "at most one question"] {
            assert!(text.contains(f), "{f} in {text}");
        }
        let (text, err) = tool(&e, &token, "wall_create", args.clone()).await;
        assert!(!err, "{text}");
        for f in ["title", "event_at", "closes_at", "photo_cap_mb", "per_guest_max", "captions", "at most one question"] {
            assert!(text.contains(f), "{f} in {text}");
        }
        assert!(!text.contains("- when"), "{text}");
    }
    assert!(e.state.db.pocket_list().await.unwrap().is_empty(), "nothing made");
}

#[tokio::test]
async fn an_rsvp_is_a_gather_pocket_owned_by_the_bot_from_its_own_chat_and_the_answer_has_the_guest_link() {
    let e = setup(true).await;
    with_guest_address(&e).await;
    let (code, text) = make(&e, "gather_create", rsvp()).await;
    assert!(text.starts_with(&format!("Pocket app · Gather [{code}] RSVP \"Office holiday party\" is open.")), "{text}");
    assert!(text.contains(&format!("Guest link: {GUEST_BASE}/p/{code}")), "{text}");
    assert!(!text.contains("#remote=") && !text.contains("#screen="), "no host link in a tool answer: {text}");
    let row = e.state.db.pocket_get(&code).await.unwrap().expect("the pocket exists");
    assert_eq!(row.kit, "gather");
    assert_eq!(row.owner_kwant_id.as_deref(), Some(e.bot.id.as_str()));
    assert_eq!(row.title, "Office holiday party");
    let sched = e.state.db.pocket_schedule(&code).await.unwrap().expect("a schedule row");
    let origin: Value = serde_json::from_str(sched.origin.as_deref().expect("an origin")).unwrap();
    assert_eq!(origin["kwant_id"], json!(e.bot.id), "{origin}");
    let spec = stored_spec(&e, &code).await;
    assert_eq!((spec["face"].clone(), spec["price"].clone(), spec["show_names"].clone()), (json!("rsvp"), json!("EUR 15"), json!(false)), "{spec}");
    assert_eq!(spec["closes_at"], spec["event_at"], "answers close when the party starts: {spec}");
    let host = host_view(&e, &code).await;
    assert!(!host["view"]["paid"].is_null(), "a price means paid ticks: {host}");
}

#[tokio::test]
async fn an_rsvp_whose_weekday_isnt_event_ats_day_is_refused_and_nothing_is_made() {
    let e = setup(true).await;
    let token = turn(&e, &e.bot, "t-fri").await;
    let mut fri = rsvp();
    let (d, _) = party_day();
    fri["when"] = json!(format!("Fri {}, 6 PM", d.format("%b %-d")));
    let (text, err) = tool(&e, &token, "gather_create", fri).await;
    assert!(err, "{text}");
    assert!(text.starts_with("Nothing was made: when says Friday, but event_at is Saturday"), "{text}");
    assert!(text.contains("Ask the person which they mean"), "{text}");
    assert!(e.state.db.pocket_list().await.unwrap().is_empty());
}

#[tokio::test]
async fn a_potluck_of_3_mains_4_desserts_2_drinks_is_3_slots_with_those_caps() {
    let e = setup(true).await;
    let (code, text) = make(&e, "gather_create", potluck()).await;
    assert!(text.contains("] Sign-up \"Potluck\" is open."), "{text}");
    let spec = stored_spec(&e, &code).await;
    let caps: Vec<(String, u64)> =
        spec["slots"].as_array().unwrap().iter().map(|s| (s["name"].as_str().unwrap().to_string(), s["cap"].as_u64().unwrap())).collect();
    assert_eq!(caps, [("Mains".to_string(), 3), ("Desserts".to_string(), 4), ("Drinks".to_string(), 2)]);
    let host = host_view(&e, &code).await;
    let slots: Vec<u64> = host["view"]["slots"].as_array().expect("the host sees the slots").iter().map(|s| s["cap"].as_u64().unwrap()).collect();
    assert_eq!(slots, [3, 4, 2], "{host}");
}

#[tokio::test]
async fn a_ranked_vote_of_5_options_is_made_and_the_host_view_never_shows_a_ballots_order() {
    let e = setup(true).await;
    let (code, text) = make(&e, "gather_create", vote()).await;
    assert!(text.contains("] Vote \"Party theme\" is open.") && text.contains("secret ballot"), "{text}");
    let spec = stored_spec(&e, &code).await;
    assert_eq!(spec["ballot"]["options"].as_array().unwrap().len(), 5);
    assert_eq!(spec["members"], json!(["Ana", "Ben", "Cleo"]));
    let orders: [[u64; 5]; 2] = [[2, 0, 4, 1, 3], [3, 1, 4, 0, 2]];
    for (i, name) in ["Ana", "Ben"].iter().enumerate() {
        let req = serde_json::from_value(json!({"name": name, "pin": format!("{:04}", 2468 + i)})).unwrap();
        let (p, _key) = kwnt::pocket::join(&e.state, &code, req).await.unwrap_or_else(|err| panic!("{name} joins: {}", err.message));
        kwnt::pocket::act(&e.state, &code, &Viewer::Guest(p.id.clone()), "vote", &json!({"ranking": orders[i]}))
            .await
            .unwrap_or_else(|err| panic!("{name} votes: {}", err.message));
    }
    let host = host_view(&e, &code).await;
    let s = host.to_string();
    assert!(s.contains("Ana") && s.contains("Ben"), "the host sees who voted: {s}");
    for o in orders {
        assert!(!holds_numbers(&host, &o), "a ballot's order in the host view: {s}");
    }
    assert!(!s.contains("\"ranking\"") && !s.contains("\"ballots\""), "{s}");
}

#[tokio::test]
async fn a_wall_is_a_wall_pocket_and_a_when_in_the_call_never_reaches_the_kit() {
    let e = setup(true).await;
    with_guest_address(&e).await;
    let mut w = photo_wall();
    w["when"] = json!("2026-12-12");
    let (code, text) = make(&e, "wall_create", w.clone()).await;
    assert!(text.starts_with(&format!("Pocket app · Photo wall [{code}] \"Office holiday party\" is open. Uploads stop at {}.", w["closes_at"].as_str().unwrap())), "{text}");
    assert!(text.contains(&format!("Guest link: {GUEST_BASE}/p/{code}")), "{text}");
    assert!(!text.contains("#remote=") && !text.contains("#screen="), "{text}");
    let row = e.state.db.pocket_get(&code).await.unwrap().unwrap();
    assert_eq!(row.kit, "wall");
    assert_eq!(row.owner_kwant_id.as_deref(), Some(e.bot.id.as_str()));
    let spec = stored_spec(&e, &code).await;
    assert!(spec.get("when").is_none(), "{spec}");
    assert_eq!((spec["event_at"].clone(), spec["closes_at"].clone()), (w["event_at"].clone(), w["closes_at"].clone()), "{spec}");

    // No closes_at: noon the day after the party, in its own offset
    let mut no_close = photo_wall();
    no_close.as_object_mut().unwrap().remove("closes_at");
    let (code, _) = make(&e, "wall_create", no_close).await;
    let (d, _) = party_day();
    assert_eq!(stored_spec(&e, &code).await["closes_at"], json!(at(d + Duration::days(1), "12:00")));
}

#[tokio::test]
async fn bad_input_answers_nothing_was_made_in_the_kits_words_and_stores_nothing() {
    let e = setup(true).await;
    let token = turn(&e, &e.bot, "t-bad").await;
    let (d, _) = party_day();
    let mut rfc_when = rsvp();
    rfc_when["when"] = json!(at(d, "18:00"));
    let mut cap0 = potluck();
    cap0["slots"][1]["cap"] = json!(0);
    let mut one_option = vote();
    one_option["ballot"]["options"] = json!(["Only one"]);
    let mut past = vote();
    past["closes_at"] = json!("2020-01-01T18:00:00+01:00");
    let mut priced_vote = vote();
    priced_vote["price"] = json!("EUR 5");
    for (args, words) in [
        (rfc_when, "when is the line people read"),
        (cap0, "Desserts: cap must be a whole number from 1 to 60."),
        (one_option, "A vote has 2 to 10 options."),
        (past, "closes_at is already past."),
        (priced_vote, "price is for an RSVP"),
        (json!({"face": "party", "title": "x", "closes_at": at(d, "18:00")}), "face must be rsvp, signup or vote."),
        (json!({"face": "rsvp", "title": "No times"}), "The spec needs closes_at"),
        (json!({"spec": "not json {"}), "isn't valid JSON"),
        (json!({"spec": [1, 2]}), "must be a JSON object"),
    ] {
        let (text, err) = tool(&e, &token, "gather_create", args.clone()).await;
        assert!(err, "{args}: {text}");
        assert!(text.starts_with("Nothing was made") && text.contains(words) && !text.contains("panic"), "{words} for {args}: {text}");
    }
    let mut early = photo_wall();
    early["closes_at"] = json!(at(d, "17:00"));
    for (args, words) in [
        (json!({"title": "Party photos"}), "when the party starts, is missing"),
        (json!({"title": "Party photos", "event_at": "next Saturday"}), "event_at must be an RFC 3339 time"),
        (early, "must be after event_at"),
        (json!({"title": "Party photos", "event_at": at(d, "18:00"), "photo_cap_mb": 0}), "photo_cap_mb must be a whole number from 1 to 25."),
    ] {
        let (text, err) = tool(&e, &token, "wall_create", args.clone()).await;
        assert!(err, "{args}: {text}");
        assert!(text.starts_with("Nothing was made") && text.contains(words), "{words} for {args}: {text}");
    }
    assert!(e.state.db.pocket_list().await.unwrap().is_empty(), "nothing made");
}

#[tokio::test]
async fn an_owner_key_or_space_in_the_input_is_dropped_and_the_bot_owns_it() {
    let e = setup(true).await;
    let other = e.other.clone();
    let extra = json!({"owner": other.id, "owner_kwant_id": other.id, "host_key": "hk_planted", "space": "elsewhere", "origin": {"kwant_id": other.id}});
    for (tool_name, base) in [("gather_create", rsvp()), ("wall_create", photo_wall())] {
        let mut args = base;
        args.as_object_mut().unwrap().extend(extra.as_object().unwrap().clone());
        let (code, _) = make(&e, tool_name, args).await;
        let row = e.state.db.pocket_get(&code).await.unwrap().unwrap();
        assert_eq!(row.owner_kwant_id.as_deref(), Some(e.bot.id.as_str()), "{tool_name}");
        let spec = stored_spec(&e, &code).await;
        for k in ["owner", "owner_kwant_id", "host_key", "space", "origin"] {
            assert!(spec.get(k).is_none(), "{tool_name}: {k} in {spec}");
        }
        let sched = e.state.db.pocket_schedule(&code).await.unwrap().unwrap();
        let origin: Value = serde_json::from_str(sched.origin.as_deref().unwrap()).unwrap();
        assert_eq!(origin["kwant_id"], json!(e.bot.id), "{tool_name}: {origin}");
        assert!(!row.spec.contains("hk_planted"));
    }
}

#[tokio::test]
async fn another_bot_never_gets_this_bots_pocket() {
    let e = setup(true).await;
    with_guest_address(&e).await;
    let (mine, text) = make(&e, "gather_create", rsvp()).await;
    let other = e.other.clone();
    // The other bot's own call makes its own pocket, and its answer never names this bot's
    let (theirs, their_text) = make_as(&e, &other, "gather_create", rsvp()).await;
    assert_ne!(mine, theirs);
    assert!(!their_text.contains(&mine), "{their_text}");
    let row = e.state.db.pocket_get(&theirs).await.unwrap().unwrap();
    assert_eq!(row.owner_kwant_id.as_deref(), Some(other.id.as_str()));
    // A reply of the other bot carrying this bot's answer gets no pocket from it
    let m = reply_as(&e, &other, &other.id, "gather_create", rsvp(), &text).await;
    assert_eq!(pocket_block(&m), None, "{:?}", m.results);
    let (_wall, wall_text) = make(&e, "wall_create", photo_wall()).await;
    let m = reply_as(&e, &other, &other.id, "wall_create", photo_wall(), &wall_text).await;
    assert_eq!(pocket_block(&m), None, "{:?}", m.results);
}

#[tokio::test]
async fn the_switch_without_the_kit_refuses_the_call_and_makes_nothing() {
    let e = setup(true).await;
    for (value, tool_name, args) in [
        ("off", "gather_create", rsvp()),
        ("santa", "gather_create", rsvp()),
        ("wall", "gather_create", rsvp()),
        ("off", "wall_create", photo_wall()),
        ("santa, desk", "wall_create", photo_wall()),
        ("gather", "wall_create", photo_wall()),
    ] {
        e.state.db.set_setting(kwnt::pocket::SETTING, value).await.unwrap();
        let token = turn(&e, &e.bot, &format!("t-{value}-{tool_name}")).await;
        let (text, err) = tool(&e, &token, tool_name, args).await;
        assert!(err, "{value} {tool_name}: {text}");
    }
    assert!(e.state.db.pocket_list().await.unwrap().is_empty(), "nothing made");
}
