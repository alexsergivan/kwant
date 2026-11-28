//! queue-create-tool (autonomy/queue-create-tool/brief.md): "Give my bakery a take-a-number queue
//! for Saturday morning, 8 to 12." in chat. The bot's one `queue_create` tool sits beside
//! `desk_create`, behind the kit-aware switch (gate "queue"). With no queue it answers with the
//! fields and rules; a queue makes a pocket of kit `queue` owned by the bot, with the turn's chat
//! as its origin and `until` kept as the kit's `hours.until`. A bad `call_minutes` or `until` is
//! refused in words and makes nothing.
//!
//! queue-card-in-chat (Mira's ux-pass-queue-ticket fix #2): the reply that carries the call gets a
//! `pocket` block of kit `queue` whose `queue` part is what `GET /api/pockets/:code` gives the
//! card (the kit's host view, its join and `ended_at`). Another bot's or another space's reply
//! never carries this bot's queue, and the switch off attaches no card.

mod quizmaster_support;

use kwnt::db::models::{Kwant, Message, MessageExtra, NewChannel, NewKwant, NewMessage};
use kwnt::db::spaces::{NewSpace, SpaceId};
use kwnt::mcp::SELF_SERVER_NAME;
use kwnt::server::continuity::TurnOrigin;
use quizmaster_support::{setup, tool, tool_names, turn, turn_in, Env};
use serde_json::{json, Value};

/// a01: "Give my bakery a take-a-number queue for Saturday morning, 8 to 12." The bot asked about
/// first names; the person said numbers are fine.
fn bakery() -> Value {
    json!({"title": "Tine's Bakery", "line": "Pick up at the side counter", "host_name": "Tine", "until": "12:00", "call_minutes": 3})
}

fn code_of(text: &str) -> String {
    let rest = text.strip_prefix("Pocket app · Queue [").unwrap_or_else(|| panic!("not a made queue: {text}"));
    rest[..rest.find(']').unwrap()].to_string()
}

/// Makes `queue` through the tool as `bot` in its own direct chat; returns (code, the tool's text).
async fn make_as(e: &Env, bot: &Kwant, queue: Value) -> (String, String) {
    let origin = TurnOrigin { direct_kwant_id: Some(bot.id.clone()), ..Default::default() };
    let token = turn_in(e, bot, &format!("t-queue-{}", bot.id), origin).await;
    let (text, err) = tool(e, &token, "queue_create", queue).await;
    assert!(!err, "{text}");
    (code_of(&text), text)
}

#[tokio::test]
async fn queue_create_is_offered_only_while_the_switch_lets_queues_through() {
    let e = setup(false).await;
    let names = tool_names(&e, &turn(&e, &e.bot, "t-off").await).await;
    assert!(!names.iter().any(|n| n == "queue_create"), "{names:?}");
    for (value, offered) in [("santa", false), ("desk", false), ("quiz", false), ("queue", true), ("desk, Queue", true), ("on", true), ("off", false)] {
        e.state.db.set_setting(kwnt::pocket::SETTING, value).await.unwrap();
        let names = tool_names(&e, &turn(&e, &e.bot, &format!("t-{value}")).await).await;
        assert_eq!(names.iter().any(|n| n == "queue_create"), offered, "{value}: {names:?}");
        if value == "queue" {
            // queue alone: the bot now has a pocket tool, and only this one
            assert!(!names.iter().any(|n| n == "desk_create" || n == "santa_create"), "{names:?}");
        }
    }
}

#[tokio::test]
async fn the_description_answers_every_name_for_a_queue() {
    let e = setup(true).await;
    let token = turn(&e, &e.bot, "t-desc").await;
    let v = quizmaster_support::rpc(&e, &token, "tools/list", json!({})).await;
    let t = v["result"]["tools"].as_array().unwrap().iter().find(|t| t["name"] == "queue_create").expect("offered").clone();
    let d = t["description"].as_str().unwrap();
    for w in ["take-a-number queue", "ticket", "walk-ins", "bakery", "Never promise an opening time"] {
        assert!(d.contains(w), "{w} in {d}");
    }
}

#[tokio::test]
async fn with_no_queue_it_answers_the_fields_and_rules_and_makes_nothing() {
    let e = setup(true).await;
    let token = turn(&e, &e.bot, "t-rules").await;
    for args in [json!({}), json!({"spec": null}), json!({"title": "  ", "until": "12:00"})] {
        let (text, err) = tool(&e, &token, "queue_create", args.clone()).await;
        assert!(!err, "{text}");
        for f in ["title", "until", "call_minutes", "ask_name", "at most one question", "Never promise an opening time", "Never invent a phone or SMS reminder"] {
            assert!(text.contains(f), "{f} in {text}");
        }
    }
    assert!(e.state.db.pocket_list().await.unwrap().is_empty(), "nothing made");
}

#[tokio::test]
async fn a_queue_is_a_queue_pocket_owned_by_the_bot_from_its_own_chat_and_until_is_kept() {
    let e = setup(true).await;
    let bot = e.bot.clone();
    let (code, text) = make_as(&e, &bot, bakery()).await;
    assert!(text.contains("\"Tine's Bakery\" is made. It's open now and closes by itself at 12:00"), "{text}");
    assert!(text.contains("Never promise an opening time"), "{text}");
    let row = e.state.db.pocket_get(&code).await.unwrap().expect("the pocket exists");
    assert_eq!(row.kit, "queue");
    assert_eq!(row.owner_kwant_id.as_deref(), Some(e.bot.id.as_str()));
    assert_eq!(row.title, "Tine's Bakery");
    assert_eq!((row.opens_at, row.closes_at), (None, None), "no opening time: the queue opens when it's made");
    let spec: Value = serde_json::from_str(&row.spec).unwrap();
    assert_eq!(spec["hours"], json!({"until": "12:00"}), "{spec}");
    assert_eq!((spec["line"].clone(), spec["host_name"].clone(), spec["call_minutes"].clone()), (json!("Pick up at the side counter"), json!("Tine"), json!(3)));
    assert!(spec.get("until").is_none(), "the kit reads hours.until only: {spec}");
    let st: Value = serde_json::from_str(&row.kit_state).unwrap();
    assert_eq!(st["state"], "open", "{st}");
    assert_eq!(st["hours_until"], "12:00", "{st}");
    assert!(st["closes_at"].is_string(), "it closes by itself: {st}");
    let sched = e.state.db.pocket_schedule(&code).await.unwrap().expect("a schedule row");
    let origin: Value = serde_json::from_str(sched.origin.as_deref().expect("an origin")).unwrap();
    assert_eq!(origin["kwant_id"], json!(e.bot.id), "{origin}");

    // No until: open until the person closes it
    let (code2, text2) = make_as(&e, &bot, json!({"title": "Market stall"})).await;
    assert!(text2.contains("stays open until the person closes it"), "{text2}");
    let st: Value = serde_json::from_str(&e.state.db.pocket_get(&code2).await.unwrap().unwrap().kit_state).unwrap();
    assert!(st["closes_at"].is_null() && st["hours_until"].is_null(), "{st}");
}

#[tokio::test]
async fn a_bad_call_minutes_or_until_is_refused_in_words_and_makes_nothing() {
    let e = setup(true).await;
    let token = turn(&e, &e.bot, "t-bad").await;
    for (extra, words) in [
        (json!({"call_minutes": 0}), "call_minutes must be a whole number from 1 to 10."),
        (json!({"call_minutes": 15}), "call_minutes must be a whole number from 1 to 10."),
        (json!({"call_minutes": 2.5}), "call_minutes must be a whole number from 1 to 10."),
        (json!({"until": "noon"}), "until must be a time written HH:MM"),
        (json!({"until": "8 to 12"}), "until must be a time written HH:MM"),
        (json!({"ask_name": "yes"}), "ask_name must be true or false."),
        (json!({"title": "A very long bakery name that goes on and on"}), "title is at most 40 characters."),
    ] {
        let mut q = bakery();
        q.as_object_mut().unwrap().extend(extra.as_object().unwrap().clone());
        let (text, err) = tool(&e, &token, "queue_create", q).await;
        assert!(err, "{extra}: {text}");
        assert!(text.starts_with("Nothing was made: ") && text.contains(words), "{extra}: {text}");
    }
    assert!(e.state.db.pocket_list().await.unwrap().is_empty(), "nothing made");
}

#[tokio::test]
async fn the_switch_off_refuses_and_makes_nothing() {
    let e = setup(true).await;
    for value in ["off", "desk"] {
        e.state.db.set_setting(kwnt::pocket::SETTING, value).await.unwrap();
        let token = turn(&e, &e.bot, &format!("t-{value}2")).await;
        let (t, err) = tool(&e, &token, "queue_create", bakery()).await;
        assert!(err, "{value}: {t}");
    }
    assert!(e.state.db.pocket_list().await.unwrap().is_empty(), "nothing made");
}

/// A turn's record of its own `queue_create` call, as the self server writes it.
fn call(args: Value, status: &str, result: &str) -> Value {
    json!({"id": "call-queue_create", "name": format!("{SELF_SERVER_NAME}__queue_create"), "server": SELF_SERVER_NAME, "tool": "queue_create",
           "arguments": args, "status": status, "result": result})
}

/// A call as the Claude CLI engine records it: "allowed", with no `result`.
fn claude_call(args: Value) -> Value {
    json!({"id": "toolu-queue_create", "name": format!("mcp__{SELF_SERVER_NAME}__queue_create"), "server": SELF_SERVER_NAME,
           "tool": "queue_create", "arguments": args, "status": "allowed"})
}

/// `bot`'s final reply in a chat (a bot's direct chat, or a channel), with the turn's tool calls
/// and the card's `meta`.
async fn reply_in(e: &Env, bot: &Kwant, direct: Option<&str>, channel: Option<&str>, calls: Vec<Value>, meta: Option<Value>) -> Message {
    let msg = NewMessage {
        channel_id: channel.map(Into::into),
        direct_kwant_id: direct.map(Into::into),
        sender_role: "kwant".into(),
        sender_id: bot.name.clone(),
        text: "Your queue is open: share the link.".into(),
        reasoning_content: None,
        tool_calls: Some(Value::Array(calls)),
        tokens_used: 0,
    };
    e.state.db.insert_message_with(&msg, &MessageExtra { meta, ..Default::default() }).await.unwrap()
}

async fn reply_as(e: &Env, bot: &Kwant, direct: &str, calls: Vec<Value>) -> Message {
    reply_in(e, bot, Some(direct), None, calls, None).await
}

/// The person's message in a chat: the ask a turn answers.
async fn ask_in(e: &Env, direct: Option<&str>, channel: Option<&str>) {
    let msg = NewMessage {
        channel_id: channel.map(Into::into),
        direct_kwant_id: direct.map(Into::into),
        sender_role: "user".into(),
        sender_id: "user".into(),
        text: "Give my bakery a take-a-number queue".into(),
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

/// What the chat's `validQueue` (PocketQueue.tsx) needs to take the block, checked on the wire.
fn assert_valid_queue_block(b: &Value) {
    assert_eq!(b["kit"], "queue", "{b}");
    assert!(["writing", "live", "ended"].contains(&b["state"].as_str().unwrap()), "{b}");
    assert!(b["join"].is_null() && b["santa"].is_null() && b["desk"].is_null() && b["results"].is_null() && b["draft"].is_null(), "{b}");
    let q = &b["queue"];
    assert!(q.is_object(), "{b}");
    assert!(["open", "paused", "closed", "ended"].contains(&q["state"].as_str().unwrap()), "{q}");
    let title = q["spec"]["title"].as_str().unwrap();
    assert!(!title.trim().is_empty() && title.chars().count() <= 80, "{q}");
    let cm = q["spec"]["call_minutes"].as_u64().unwrap();
    assert!((1..=10).contains(&cm), "{q}");
    assert!(q["tickets"].is_array() && q["summary"].is_object(), "{q}");
    assert!(q["next_number"].as_u64().is_some(), "{q}");
    assert!(q.get("join").is_some() && q.get("ended_at").is_some(), "join and ended_at ride in the part: {q}");
    for k in ["guest", "guest_id", "key", "host_key", "token", "pin"] {
        assert!(!b.to_string().contains(&format!("\"{k}\":")), "{k} in {b}");
    }
}

#[tokio::test]
async fn the_reply_that_made_a_queue_gets_its_card_with_the_cards_own_host_view() {
    let e = setup(true).await;
    let bot = e.bot.clone();
    let (code, text) = make_as(&e, &bot, bakery()).await;
    assert!(text.contains("The person's chat already shows its card"), "{text}");
    let m = reply_as(&e, &bot, &bot.id, vec![call(bakery(), "ok", &text)]).await;
    let b = pocket(&m).unwrap_or_else(|| panic!("a pocket block on the reply: {:?}", m.results));
    eprintln!("queue block: {b}");
    assert_valid_queue_block(&b);
    assert_eq!((b["code"].clone(), b["name"].clone(), b["state"].clone()), (json!(code), json!("Tine's Bakery"), json!("live")));
    assert_eq!(b["provenance"]["ref"], json!(format!("pocket:{code}")));
    let q = &b["queue"];
    assert_eq!(q["state"], "open", "{q}");
    assert_eq!(q["spec"]["title"], "Tine's Bakery", "{q}");
    assert_eq!(q["spec"]["line"], "Pick up at the side counter", "{q}");
    assert_eq!(q["spec"]["host_name"], "Tine", "{q}");
    assert_eq!(q["spec"]["hours"], json!({"until": "12:00"}), "{q}");
    assert_eq!(q["hours_until"], "12:00", "{q}");
    assert_eq!((q["next_number"].clone(), q["tickets"].clone(), q["ended_at"].clone()), (json!(1), json!([]), Value::Null), "{q}");
    assert_eq!(q["summary"]["waiting"], json!(0), "{q}");

    // The card's live read (GET /api/pockets/:code) gives the host's view of the same pocket
    let snap = kwnt::pocket::snapshot(&e.state, &code).await.unwrap().expect("the pocket");
    let read = kwnt::pocket::view_json(&snap, &kwnt::pocket::kits::Viewer::Host, chrono::Utc::now());
    let hv = &read["view"];
    for k in ["spec", "state", "hours_until", "closes_at", "next_number", "tickets", "now_serving", "pause_text", "close_text"] {
        assert_eq!(q[k], hv[k], "{k}: {q} vs {hv}");
    }
    for k in ["served_today", "waiting", "days", "served_total"] {
        assert_eq!(q["summary"][k], hv["summary"][k], "summary {k}");
    }
}

#[tokio::test]
async fn another_bots_or_another_spaces_reply_never_carries_this_bots_queue() {
    let e = setup(true).await;
    let bot = e.bot.clone();
    let (code, text) = make_as(&e, &bot, bakery()).await;
    // Another bot's reply naming this bot's queue, in its own chat or in this bot's
    let other = e.other.clone();
    let m = reply_as(&e, &other, &other.id, vec![call(bakery(), "ok", &text)]).await;
    assert_eq!(pocket(&m), None, "{:?}", m.results);
    let m = reply_as(&e, &bot, &other.id, vec![call(bakery(), "ok", &text)]).await;
    assert_eq!(pocket(&m), None, "{:?}", m.results);

    // A reply under this bot's name in a channel of another space, naming this bot or not
    let side = e.state.db.create_space(&NewSpace { name: "Side".into(), icon: "S".into(), color: None, this_machine_enabled: true }).await.unwrap();
    let side_id = SpaceId::new(side.id.clone());
    let twin = e
        .state
        .db
        .create_kwant(
            &side_id,
            &NewKwant {
                name: bot.name.clone(),
                title: "Twin".into(),
                persona_prompt: String::new(),
                soul: String::new(),
                avatar_sprite: String::new(),
                driver_type: bot.driver_type.clone(),
                preferred_model: bot.preferred_model.clone(),
            },
        )
        .await
        .unwrap();
    let channel = e
        .state
        .db
        .create_channel(
            &side_id,
            &NewChannel { name: "bakery".into(), description: None, lead_kwant_id: twin.id.clone(), member_kwants: vec![twin.id.clone()], routing_mode: "lead".into() },
        )
        .await
        .unwrap();
    for meta in [None, Some(json!({"kwant_id": bot.id}))] {
        let m = reply_in(&e, &bot, None, Some(&channel.id), vec![call(bakery(), "ok", &text)], meta.clone()).await;
        assert_eq!(pocket(&m), None, "another space's queue ({meta:?}): {:?}", m.results);
        assert!(!serde_json::to_string(&m.results).unwrap().contains(&code));
    }

    // A code no queue has is no card
    let no_such = "Pocket app · Queue [NOSUCH1] \"x\" is made.";
    let m = reply_as(&e, &bot, &bot.id, vec![call(bakery(), "ok", no_such)]).await;
    assert_eq!(pocket(&m), None, "no such queue: {:?}", m.results);
}

#[tokio::test]
async fn a_claude_engine_reply_gets_the_queue_it_made_this_turn_and_the_switch_off_none() {
    let e = setup(true).await;
    let bot = e.bot.clone();
    ask_in(&e, Some(&bot.id), None).await;
    let (code, text) = make_as(&e, &bot, bakery()).await;
    let m = reply_as(&e, &bot, &bot.id, vec![claude_call(bakery())]).await;
    let b = pocket(&m).unwrap_or_else(|| panic!("the queue card under a Claude reply: {:?}", m.results));
    assert_valid_queue_block(&b);
    assert_eq!((b["code"].clone(), b["name"].clone()), (json!(code), json!("Tine's Bakery")), "{b}");
    // The rules and a denied call made nothing
    for args in [json!({}), json!({"spec": null}), json!({"title": ""})] {
        assert_eq!(pocket(&reply_as(&e, &bot, &bot.id, vec![claude_call(args.clone())]).await), None, "{args}");
    }
    let mut denied = claude_call(bakery());
    denied["status"] = json!("denied");
    assert_eq!(pocket(&reply_as(&e, &bot, &bot.id, vec![denied]).await), None, "denied");

    // Switched off, or on for other kits only: no card
    for value in ["off", "desk", "santa, quiz"] {
        e.state.db.set_setting(kwnt::pocket::SETTING, value).await.unwrap();
        let m = reply_as(&e, &bot, &bot.id, vec![call(bakery(), "ok", &text)]).await;
        assert_eq!(pocket(&m), None, "{value}");
    }
}
