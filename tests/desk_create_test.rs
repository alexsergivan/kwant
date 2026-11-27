//! desk-create-from-chat (autonomy/desk-create-from-chat/brief.md): "set up a front desk for my
//! guesthouse" or "let people book a 30-minute haircut, Tue-Sat 10-6" in chat. The bot's one
//! `desk_create` tool sits beside `santa_create`, behind the kit-aware switch (gate "desk"). With
//! no desk it answers with the fields and rules; a guide desk or a booking desk (its times as a
//! weekly `fill` rule, expanded by kwant) makes a pocket of kit `desk` owned by the bot, with the
//! turn's chat as its origin, and the reply that carries the call gets a `pocket` block with a
//! `desk` part. Bad times answer `bad_slots` in the validator's words. Nothing in the block ever
//! carries a PIN or key, and another bot's reply never carries this bot's desk.

mod quizmaster_support;

use chrono::{Datelike, Duration, Local, NaiveDate, NaiveDateTime, Weekday};
use kwnt::db::models::{Kwant, Message, MessageExtra, NewMessage};
use kwnt::mcp::SELF_SERVER_NAME;
use kwnt::pocket::kits::Viewer;
use kwnt::server::continuity::TurnOrigin;
use quizmaster_support::{setup, tool, tool_names, turn, turn_in, Env};
use serde_json::{json, Value};

/// a01: "Set up a front desk for my guesthouse: check-in 3 PM, Wi-Fi password on the fridge, ask
/// me about late check-out." The bot asked about the cap; the person said 30.
fn guesthouse() -> Value {
    json!({
        "mode": "guide",
        "title": "Sea View Guesthouse",
        "host_name": "Nika",
        "guide": "Check-in is from 3 PM.\nThe Wi-Fi password is on the fridge.\nLate check-out: ask the host.",
        "starters": ["Check-in time?", "Wi-Fi?"],
        "daily_cap": 30,
    })
}

fn haircut_fill(weeks: u32) -> Value {
    json!({"days": ["tue", "wed", "thu", "fri", "sat"], "from": "10:00", "to": "18:00", "minutes": 30, "seats": 1, "weeks": weeks})
}

/// a02: "Let people book a 30-minute haircut, Tue-Sat 10-6."
fn haircut(weeks: u32) -> Value {
    json!({"mode": "booking", "title": "Book a haircut", "what": "30-minute haircut", "fill": haircut_fill(weeks)})
}

/// A turn's record of one of its own tool calls, as the self server writes it.
fn call(args: Value, status: &str, result: &str) -> Value {
    json!({"id": "call-desk_create", "name": format!("{SELF_SERVER_NAME}__desk_create"), "server": SELF_SERVER_NAME, "tool": "desk_create",
           "arguments": args, "status": status, "result": result})
}

/// A call as the Claude CLI engine records it: "allowed", with no `result`.
fn claude_call(args: Value) -> Value {
    json!({"id": "toolu-desk_create", "name": format!("mcp__{SELF_SERVER_NAME}__desk_create"), "server": SELF_SERVER_NAME,
           "tool": "desk_create", "arguments": args, "status": "allowed"})
}

/// `bot`'s final reply in `direct`'s chat, with the turn's tool calls.
async fn reply_as(e: &Env, bot: &Kwant, direct: &str, calls: Vec<Value>) -> Message {
    let msg = NewMessage {
        channel_id: None,
        direct_kwant_id: Some(direct.into()),
        sender_role: "kwant".into(),
        sender_id: bot.name.clone(),
        text: "Your front desk is open: share the guest link.".into(),
        reasoning_content: None,
        tool_calls: Some(Value::Array(calls)),
        tokens_used: 0,
    };
    e.state.db.insert_message_with(&msg, &MessageExtra::default()).await.unwrap()
}

/// The person's message in a bot's direct chat: the ask a turn answers.
async fn ask_in(e: &Env, direct: &str) {
    let msg = NewMessage {
        channel_id: None,
        direct_kwant_id: Some(direct.into()),
        sender_role: "user".into(),
        sender_id: "user".into(),
        text: "Set up a front desk for my guesthouse".into(),
        reasoning_content: None,
        tool_calls: None,
        tokens_used: 0,
    };
    e.state.db.insert_message_with(&msg, &MessageExtra::default()).await.unwrap();
}

fn pocket(m: &Message) -> Option<Value> {
    let b = m.results.as_ref()?.get("blocks")?.get(0)?.clone();
    (b["type"] == json!("pocket")).then_some(b)
}

fn code_of(text: &str) -> String {
    let rest = text.strip_prefix("Pocket app · Front desk [").unwrap_or_else(|| panic!("not a made desk: {text}"));
    rest[..rest.find(']').unwrap()].to_string()
}

/// Makes `desk` through the tool as `bot` in its own direct chat; returns (code, the tool's text).
async fn make_as(e: &Env, bot: &Kwant, desk: Value) -> (String, String) {
    let origin = TurnOrigin { direct_kwant_id: Some(bot.id.clone()), ..Default::default() };
    let token = turn_in(e, bot, &format!("t-desk-{}", bot.id), origin).await;
    let (text, err) = tool(e, &token, "desk_create", desk).await;
    assert!(!err, "{text}");
    (code_of(&text), text)
}

async fn make(e: &Env, desk: Value) -> (String, String) {
    let bot = e.bot.clone();
    make_as(e, &bot, desk).await
}

/// Keys that would let someone act as a guest or the host: none may appear in a desk block.
const SECRET_KEYS: [&str; 7] = ["pin", "pin_hash", "key", "host_key", "name_key", "guest_key", "reset_code"];

fn secret_like(v: &Value, path: &str) -> Option<String> {
    match v {
        Value::Array(a) => a.iter().enumerate().find_map(|(i, x)| secret_like(x, &format!("{path}[{i}]"))),
        Value::Object(o) => o.iter().find_map(|(k, x)| {
            if SECRET_KEYS.contains(&k.to_lowercase().as_str()) {
                Some(format!("{path}.{k}"))
            } else {
                secret_like(x, &format!("{path}.{k}"))
            }
        }),
        _ => None,
    }
}

fn text_max(v: &Value, max: usize) -> bool {
    v.as_str().is_some_and(|t| !t.trim().is_empty() && t.encode_utf16().count() <= max)
}

fn opt_text(v: &Value, max: usize) -> bool {
    v.is_null() || text_max(v, max)
}

fn count(v: &Value) -> bool {
    v.is_null() || v.as_u64().is_some()
}

/// The frontend's `validPocket` desk rules and `validDesk`, as far as a test can read them.
fn assert_valid_desk_block(b: &Value) {
    assert_eq!(b["kit"], "desk", "{b}");
    assert_eq!(b["source"], json!({"kind": "slot"}));
    assert_eq!(b["provenance"]["kind"], "kwant_data");
    assert!(b["join"].is_null() && b["results"].is_null() && b["santa"].is_null(), "no top-level join, results or santa: {b}");
    assert!(["writing", "live", "ended"].contains(&b["state"].as_str().unwrap()), "{b}");
    assert_eq!(secret_like(b, ""), None, "{b}");
    let d = &b["desk"];
    assert!(text_max(&d["title"], 80), "{d}");
    assert!(opt_text(&d["place"], 80) && opt_text(&d["host_name"], 40), "{d}");
    let c = &d["counts"];
    for k in ["guests", "waiting", "replies_today", "messages_today", "daily_cap"] {
        assert!(count(&c[k]), "{k}: {c}");
    }
    let cap = c["daily_cap"].as_u64().unwrap_or(50);
    assert!((10..=200).contains(&cap), "{c}");
    assert!(text_max(&d["answer_bot"]["name"], 40), "{d}");
    assert!(opt_text(&d["answer_bot"]["model"], 60), "{d}");
    assert!(d["answer_bot"].get("id").is_none(), "no bot id rides along: {d}");
    assert!(d["guide_preview"].is_null() || d["guide_preview"].as_str().is_some_and(|p| p.encode_utf16().count() <= 600), "{d}");
    let guests = d["guests"].as_array().map(Vec::as_slice).unwrap_or_default();
    assert!(guests.len() <= 60);
    for g in guests {
        assert!(g["id"].is_string() && text_max(&g["name"], 40) && g["waiting"].is_boolean(), "{g}");
        assert!(g["last"].is_null() && g["handoff_reason"].is_null() && g.get("booking").is_none(), "no guest words: {g}");
    }
    if c["waiting"].as_u64().unwrap_or(0) > c["guests"].as_u64().unwrap_or(0) {
        assert!(guests.is_empty(), "{c}");
    }
    if d["mode"] == "booking" {
        for s in d["slots"].as_array().expect("a booking desk lists its times") {
            let start = s["start"].as_str().unwrap();
            assert!(start.len() == 16 && NaiveDateTime::parse_from_str(start, "%Y-%m-%dT%H:%M").is_ok(), "{s}");
        }
    }
    if !d["join"].is_null() {
        let mode = d["join"]["mode"].as_str().unwrap();
        assert!(["anywhere", "wifi", "none"].contains(&mode));
        assert!(!d["join"].to_string().contains("trycloudflare"));
    }
}

/// Every time of `slots` by its day.
fn per_day(slots: &[Value]) -> std::collections::BTreeMap<NaiveDate, Vec<String>> {
    let mut by = std::collections::BTreeMap::new();
    for s in slots {
        let start = s["start"].as_str().unwrap();
        let at = NaiveDateTime::parse_from_str(start, "%Y-%m-%dT%H:%M").unwrap();
        by.entry(at.date()).or_insert_with(Vec::new).push(start[11..].to_string());
    }
    by
}

#[tokio::test]
async fn desk_create_is_offered_only_while_the_switch_lets_desks_through() {
    let e = setup(false).await;
    let names = tool_names(&e, &turn(&e, &e.bot, "t-off").await).await;
    assert!(!names.iter().any(|n| n == "desk_create"), "{names:?}");
    for (value, offered) in [("santa", false), ("quiz", false), ("desk", true), ("santa, Desk", true), ("on", true), ("off", false)] {
        e.state.db.set_setting(kwnt::pocket::SETTING, value).await.unwrap();
        let names = tool_names(&e, &turn(&e, &e.bot, &format!("t-{value}")).await).await;
        assert_eq!(names.iter().any(|n| n == "desk_create"), offered, "{value}: {names:?}");
        if value == "santa" {
            assert!(names.iter().any(|n| n == "santa_create"), "santa alone still offers santa_create: {names:?}");
        }
    }
}

#[tokio::test]
async fn the_description_answers_every_name_for_a_desk() {
    let e = setup(true).await;
    let token = turn(&e, &e.bot, "t-desc").await;
    let v = quizmaster_support::rpc(&e, &token, "tools/list", json!({})).await;
    let t = v["result"]["tools"].as_array().unwrap().iter().find(|t| t["name"] == "desk_create").expect("offered").clone();
    let d = t["description"].as_str().unwrap();
    for w in ["front desk", "guest guide", "guesthouse", "Airbnb", "booking desk", "appointments", "book a time"] {
        assert!(d.contains(w), "{w} in {d}");
    }
}

#[tokio::test]
async fn with_no_desk_it_answers_the_fields_and_rules_and_makes_nothing() {
    let e = setup(true).await;
    let token = turn(&e, &e.bot, "t-rules").await;
    for args in [json!({}), json!({"spec": null}), json!({"title": "  "})] {
        let (text, err) = tool(&e, &token, "desk_create", args.clone()).await;
        assert!(!err, "{text}");
        for f in ["mode", "title", "guide", "daily_cap", "slots", "fill", "at most one question", "Never invent a Wi-Fi password"] {
            assert!(text.contains(f), "{f} in {text}");
        }
        let m = reply_as(&e, &e.bot, &e.bot.id, vec![call(args, "ok", &text)]).await;
        assert_eq!(pocket(&m), None, "the rules carry no card");
    }
    assert!(e.state.db.pocket_list().await.unwrap().is_empty(), "nothing made");
}

#[tokio::test]
async fn a_guide_desk_is_a_desk_pocket_owned_by_the_bot_from_its_own_chat_and_the_reply_gets_its_card() {
    let e = setup(true).await;
    let (code, text) = make(&e, guesthouse()).await;
    assert!(text.contains("Guest guide, up to 30 bot replies a day"), "{text}");
    assert!(text.contains("Never invent a Wi-Fi password"), "{text}");
    let row = e.state.db.pocket_get(&code).await.unwrap().expect("the pocket exists");
    assert_eq!(row.kit, "desk");
    assert_eq!(row.owner_kwant_id.as_deref(), Some(e.bot.id.as_str()));
    assert_eq!(row.title, "Sea View Guesthouse");
    let sched = e.state.db.pocket_schedule(&code).await.unwrap().expect("a schedule row");
    let origin: Value = serde_json::from_str(sched.origin.as_deref().expect("an origin")).unwrap();
    assert_eq!(origin["kwant_id"], json!(e.bot.id), "{origin}");

    let m = reply_as(&e, &e.bot, &e.bot.id, vec![call(guesthouse(), "ok", &text)]).await;
    let b = pocket(&m).unwrap_or_else(|| panic!("a pocket block on the reply: {:?}", m.results));
    eprintln!("desk block: {b}");
    assert_valid_desk_block(&b);
    assert_eq!((b["code"].clone(), b["name"].clone(), b["state"].clone()), (json!(code), json!("Sea View Guesthouse"), json!("live")));
    assert_eq!(b["provenance"]["ref"], json!(format!("pocket:{code}")));
    let d = &b["desk"];
    assert_eq!((d["title"].clone(), d["host_name"].clone()), (json!("Sea View Guesthouse"), json!("Nika")), "{d}");
    assert_eq!(d["counts"]["daily_cap"], json!(30), "{d}");
    assert_eq!(d["counts"]["guests"], json!(0), "{d}");
    assert!(d["guide_preview"].as_str().unwrap().starts_with("Check-in is from 3 PM."), "{d}");
    assert_ne!(d["mode"], "booking", "{d}");
    // No quiz or Santa parts ride along
    assert!(b["draft"].is_null() && b["checks"].is_null() && b["santa"].is_null(), "{b}");
}

#[tokio::test]
async fn a_booking_fill_tue_to_sat_10_to_6_every_30_minutes_makes_16_times_a_day() {
    let e = setup(true).await;
    let (code, text) = make(&e, haircut(2)).await;
    let row = e.state.db.pocket_get(&code).await.unwrap().unwrap();
    let spec: Value = serde_json::from_str(&row.spec).unwrap();
    let slots = spec["slots"].as_array().expect("the fill became slots").clone();
    assert!(text.contains(&format!("Booking desk with {} times", slots.len())), "{text}");
    let by = per_day(&slots);
    let now = Local::now().naive_local();
    let full: Vec<String> = (0..16).map(|i| format!("{:02}:{:02}", 10 + i / 2, (i % 2) * 30)).collect();
    for (day, times) in &by {
        assert!(![Weekday::Sun, Weekday::Mon].contains(&day.weekday()), "{day}");
        if *day == now.date() {
            // Today keeps only the times still ahead
            assert!(times.len() <= 16 && times.iter().all(|t| full.contains(t)), "{day}: {times:?}");
        } else {
            assert_eq!(times, &full, "{day}");
        }
    }
    for s in &slots {
        assert_eq!((s["minutes"].clone(), s["seats"].clone()), (json!(30), json!(1)), "{s}");
        assert!(NaiveDateTime::parse_from_str(s["start"].as_str().unwrap(), "%Y-%m-%dT%H:%M").unwrap() > now, "no past time: {s}");
    }
    // Two weeks hold at least 9 whole days (10 open days, today maybe partial)
    let whole = by.iter().filter(|(d, _)| **d != now.date()).count();
    assert!((9..=10).contains(&whole), "{by:?}");
    assert!(by.keys().all(|d| *d < now.date() + Duration::days(15)), "{by:?}");

    // The card shows the week
    let m = reply_as(&e, &e.bot, &e.bot.id, vec![call(haircut(2), "ok", &text)]).await;
    let b = pocket(&m).expect("a pocket block");
    assert_valid_desk_block(&b);
    let d = &b["desk"];
    assert_eq!(d["mode"], "booking", "{d}");
    assert_eq!(d["what"], "30-minute haircut", "{d}");
    assert_eq!(d["slots"].as_array().unwrap().len(), slots.len(), "{d}");
}

#[tokio::test]
async fn bad_fill_or_bad_slots_answer_bad_slots_in_the_validators_words_and_make_nothing() {
    let e = setup(true).await;
    let token = turn(&e, &e.bot, "t-bad").await;
    let tomorrow = (Local::now() + Duration::days(1)).format("%Y-%m-%d").to_string();
    let booking = |extra: Value| {
        let mut d = json!({"mode": "booking", "title": "Book a haircut"});
        d.as_object_mut().unwrap().extend(extra.as_object().unwrap().clone());
        d
    };
    let cases = [
        (booking(json!({"fill": {"days": ["tue"], "from": "18:00", "to": "10:00"}})), "fill"),
        (booking(json!({"fill": {"days": [], "from": "10:00", "to": "18:00"}})), "fill"),
        (booking(json!({"fill": {"days": ["funday"], "from": "10:00", "to": "18:00"}})), "fill"),
        (booking(json!({"fill": {"days": ["tue"], "from": "10:00", "to": "18:00", "minutes": 2}})), "fill"),
        (booking(json!({"fill": {"days": ["tue"], "from": "10:00", "to": "18:00", "weeks": 9}})), "fill"),
        (booking(json!({"fill": {"days": ["tue"], "from": "25:00", "to": "26:00"}})), "fill"),
        (booking(json!({})), "list"),
        (booking(json!({"slots": [{"start": format!("{tomorrow}T10:00"), "minutes": 60}, {"start": format!("{tomorrow}T10:30"), "minutes": 30}]})), "overlap"),
        (booking(json!({"slots": [{"start": "2020-01-01T10:00", "minutes": 30}]})), "start"),
        (booking(json!({"slots": [{"start": "tomorrow at ten", "minutes": 30}]})), "start"),
        (booking(json!({"slots": [{"start": format!("{tomorrow}T10:00")}], "fill": haircut_fill(1)})), "fill"),
    ];
    for (desk, why) in cases {
        let (text, err) = tool(&e, &token, "desk_create", desk.clone()).await;
        assert!(err, "{desk}: {text}");
        assert!(text.starts_with("Nothing was made (bad_slots"), "{desk}: {text}");
        assert!(text.contains(&format!("why: \"{why}")), "{why} for {desk}: {text}");
        assert!(text.ends_with("Fix the times from what the person said and call desk_create again. If their hours don't fit, ask them; never make up hours."), "{text}");
    }
    let (text, _) = tool(&e, &token, "desk_create", cases_overlap(&tomorrow)).await;
    assert!(text.contains("index: 1"), "the second time overlaps: {text}");
    // A guide desk with no guide, a cap out of range, junk: each a sentence, never a panic
    for bad in [
        json!({"title": "Sea View"}),
        json!({"title": "Sea View", "guide": "Check-in 3 PM.", "daily_cap": 5000}),
        json!({"title": "Sea View", "mode": "hotel", "guide": "x"}),
        json!({"spec": "not json {"}),
        json!({"spec": [1, 2]}),
    ] {
        let (text, err) = tool(&e, &token, "desk_create", bad.clone()).await;
        assert!(err && text.starts_with("Nothing was made") && !text.contains("panic"), "{bad}: {text}");
    }
    // The kit's own refusal (here: a guide desk with no guide) sends the bot to the person, never to writing the guide itself
    let (text, err) = tool(&e, &token, "desk_create", json!({"title": "Sea View"})).await;
    assert!(err, "{text}");
    assert!(
        text.ends_with("Fix it from what the person said and call desk_create again. If a fact is missing (like the guide), ask the person; never fill it in yourself."),
        "{text}"
    );
    assert!(e.state.db.pocket_list().await.unwrap().is_empty(), "nothing made");
    let m = reply_as(&e, &e.bot, &e.bot.id, vec![call(booking(json!({})), "error", &text)]).await;
    assert_eq!(pocket(&m), None, "a failed call carries no card");
}

fn cases_overlap(tomorrow: &str) -> Value {
    json!({"mode": "booking", "title": "Book a haircut",
           "slots": [{"start": format!("{tomorrow}T10:00"), "minutes": 60}, {"start": format!("{tomorrow}T10:30"), "minutes": 30}]})
}

#[tokio::test]
async fn no_block_carries_a_pin_or_key_or_a_guests_words_even_after_guests_join_and_ask() {
    let e = setup(true).await;
    let (code, text) = make(&e, haircut(2)).await;
    let mut ids = Vec::new();
    for (i, name) in ["Ann", "Ben"].iter().enumerate() {
        let req = serde_json::from_value(json!({"name": name, "pin": format!("{:04}", 4821 + i)})).unwrap();
        let (p, _key) = kwnt::pocket::join(&e.state, &code, req).await.unwrap_or_else(|err| panic!("{name} joins: {}", err.message));
        ids.push(p.id.clone());
    }
    // Ann asks for the first time, with a note only the person may read
    let m = reply_as(&e, &e.bot, &e.bot.id, vec![call(haircut(2), "ok", &text)]).await;
    let slot = pocket(&m).unwrap()["desk"]["slots"][0]["id"].as_str().expect("a slot id").to_string();
    kwnt::pocket::act(&e.state, &code, &Viewer::Guest(ids[0].clone()), "request", &json!({"slot": slot, "note": "door code 4821 please"}))
        .await
        .unwrap_or_else(|err| panic!("Ann asks: {}", err.message));

    let m = reply_as(&e, &e.bot, &e.bot.id, vec![call(haircut(2), "ok", &text)]).await;
    let b = pocket(&m).expect("a pocket block");
    eprintln!("desk block with guests: {b}");
    assert_valid_desk_block(&b);
    let d = &b["desk"];
    assert_eq!(d["counts"]["guests"], json!(2), "{d}");
    let names: Vec<&str> = d["guests"].as_array().unwrap().iter().map(|g| g["name"].as_str().unwrap()).collect();
    assert_eq!(names.len(), 2, "{d}");
    // No key named like a PIN or key anywhere (assert_valid_desk_block), and the note stays out.
    // (The PINs' digits aren't searched for: a random code or id could spell them by chance.)
    let s = b.to_string();
    assert!(!s.contains("door code"), "{s}");
    let stored = serde_json::to_string(&m.results).unwrap();
    assert!(!stored.contains("door code") && !stored.contains("\"pin\""), "{stored}");
}

#[tokio::test]
async fn another_bots_reply_never_carries_this_bots_desk() {
    let e = setup(true).await;
    let (_code, text) = make(&e, guesthouse()).await;
    let other = e.other.clone();
    let m = reply_as(&e, &other, &other.id, vec![call(guesthouse(), "ok", &text)]).await;
    assert_eq!(pocket(&m), None, "{:?}", m.results);
    let bot = e.bot.clone();
    let m = reply_as(&e, &bot, &other.id, vec![call(guesthouse(), "ok", &text)]).await;
    assert_eq!(pocket(&m), None, "{:?}", m.results);
}

#[tokio::test]
async fn the_switch_off_refuses_and_no_card_is_attached() {
    let e = setup(true).await;
    let (_code, text) = make(&e, guesthouse()).await;
    for value in ["off", "santa"] {
        e.state.db.set_setting(kwnt::pocket::SETTING, value).await.unwrap();
        let token = turn(&e, &e.bot, &format!("t-{value}2")).await;
        let (t, err) = tool(&e, &token, "desk_create", guesthouse()).await;
        assert!(err, "{value}: {t}");
        let m = reply_as(&e, &e.bot, &e.bot.id, vec![call(guesthouse(), "ok", &text)]).await;
        assert_eq!(pocket(&m), None, "{value}");
    }
}

#[tokio::test]
async fn a_claude_engine_reply_gets_the_desk_it_made_this_turn_and_only_its_own() {
    let e = setup(true).await;
    ask_in(&e, &e.bot.id).await;
    let (code, _) = make(&e, guesthouse()).await;
    let m = reply_as(&e, &e.bot, &e.bot.id, vec![claude_call(guesthouse())]).await;
    let b = pocket(&m).unwrap_or_else(|| panic!("the desk card under a Claude reply: {:?}", m.results));
    assert_valid_desk_block(&b);
    assert_eq!((b["code"].clone(), b["name"].clone()), (json!(code), json!("Sea View Guesthouse")), "{b}");

    // The rules and a denied call made nothing
    for args in [json!({}), json!({"spec": null}), json!({"title": ""})] {
        assert_eq!(pocket(&reply_as(&e, &e.bot, &e.bot.id, vec![claude_call(args.clone())]).await), None, "{args}");
    }
    let mut denied = claude_call(guesthouse());
    denied["status"] = json!("denied");
    assert_eq!(pocket(&reply_as(&e, &e.bot, &e.bot.id, vec![denied]).await), None, "denied");

    // A desk the other bot made isn't this bot's
    ask_in(&e, &e.other.id).await;
    let other = e.other.clone();
    let m = reply_as(&e, &other, &other.id, vec![claude_call(guesthouse())]).await;
    assert_eq!(pocket(&m), None, "the other bot made none: {:?}", m.results);

    // A desk made before the newest ask isn't this turn's (message times keep whole seconds)
    tokio::time::sleep(std::time::Duration::from_millis(1100)).await;
    ask_in(&e, &e.bot.id).await;
    let m = reply_as(&e, &e.bot, &e.bot.id, vec![claude_call(guesthouse())]).await;
    assert_eq!(pocket(&m), None, "a desk made before the ask: {:?}", m.results);
}
