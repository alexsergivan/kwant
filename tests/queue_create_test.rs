//! queue-create-tool (autonomy/queue-create-tool/brief.md): "Give my bakery a take-a-number queue
//! for Saturday morning, 8 to 12." in chat. The bot's one `queue_create` tool sits beside
//! `desk_create`, behind the kit-aware switch (gate "queue"). With no queue it answers with the
//! fields and rules; a queue makes a pocket of kit `queue` owned by the bot, with the turn's chat
//! as its origin and `until` kept as the kit's `hours.until`. A bad `call_minutes` or `until` is
//! refused in words and makes nothing.

mod quizmaster_support;

use kwnt::db::models::Kwant;
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
