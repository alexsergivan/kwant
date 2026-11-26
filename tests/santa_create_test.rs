//! santa-create-from-chat (autonomy/santa-create-from-chat/brief.md): "make us a Secret Santa"
//! in chat. The bot's one `santa_create` tool sits beside the Quizmaster's tools, behind the same
//! switch. With no spec it answers with the fields and rules; with Mira's spec (6 people, Anna and
//! Ben kept apart, £20, gifts on Dec 18, reveal at 7 PM in the person's time) it makes a pocket of
//! kit `santa` owned by the bot, with the turn's chat as its origin, and the reply that carries
//! the call gets a `pocket` block with a `santa` part. A bad spec answers with the kit's own words.
//! Nothing in the block ever names a match, and another bot's reply never carries this bot's
//! Santa.

mod quizmaster_support;

use chrono::{Datelike, NaiveDate, Utc};
use kwnt::db::models::{Kwant, Message, MessageExtra, NewMessage};
use kwnt::mcp::SELF_SERVER_NAME;
use kwnt::server::continuity::TurnOrigin;
use quizmaster_support::{pockets_on, setup, tool, tool_names, turn, turn_in, Env};
use serde_json::{json, Value};

/// The next Dec 18 at least a week away (Mira's gift day; the kit refuses past dates).
fn gift_day() -> NaiveDate {
    let today = Utc::now().date_naive();
    let this_year = NaiveDate::from_ymd_opt(today.year(), 12, 18).unwrap();
    if this_year - today >= chrono::Duration::days(7) {
        this_year
    } else {
        NaiveDate::from_ymd_opt(today.year() + 1, 12, 18).unwrap()
    }
}

/// Mira's office Secret Santa: 6 people, Anna and Ben kept apart, £20, gifts on Dec 18, the
/// reveal at 19:00 in the person's time (London: +00:00 in December).
fn mira_spec() -> Value {
    let d = gift_day();
    json!({
        "title": "Team Secret Santa",
        "gift_on": d.format("%Y-%m-%d").to_string(),
        "reveal": true,
        "reveal_at": format!("{}T19:00:00+00:00", d.format("%Y-%m-%d")),
        "people": ["Anna", "Ben", "Chloe", "Dev", "Erin", "Femi"],
        "keep_apart": [["Anna", "Ben"]],
        "budget": "£20",
    })
}

/// A turn's record of one of its own tool calls, as the self server writes it.
fn call(tool: &str, args: Value, status: &str, result: &str) -> Value {
    json!({"id": format!("call-{tool}"), "name": format!("{SELF_SERVER_NAME}__{tool}"), "server": SELF_SERVER_NAME, "tool": tool,
           "arguments": args, "status": status, "result": result})
}

/// `bot`'s final reply in `direct`'s chat, with the turn's tool calls.
async fn reply_as(e: &Env, bot: &Kwant, direct: &str, calls: Vec<Value>) -> Message {
    let msg = NewMessage {
        channel_id: None,
        direct_kwant_id: Some(direct.into()),
        sender_role: "kwant".into(),
        sender_id: bot.name.clone(),
        text: "Your Secret Santa is open: send the invite link to everyone.".into(),
        reasoning_content: None,
        tool_calls: Some(Value::Array(calls)),
        tokens_used: 0,
    };
    e.state.db.insert_message_with(&msg, &MessageExtra::default()).await.unwrap()
}

fn pocket(m: &Message) -> Option<Value> {
    let b = m.results.as_ref()?.get("blocks")?.get(0)?.clone();
    (b["type"] == json!("pocket")).then_some(b)
}

/// The code a made Santa's answer starts with.
fn code_of(text: &str) -> String {
    let rest = text.strip_prefix("Pocket app · Secret Santa [").unwrap_or_else(|| panic!("not a made Santa: {text}"));
    rest[..rest.find(']').unwrap()].to_string()
}

async fn person_in_london(e: &Env) {
    e.state.db.set_setting("timezone", "Europe/London").await.unwrap();
}

/// Makes Mira's Santa through the tool in `bot`'s direct chat; returns (code, the tool's text).
async fn make(e: &Env) -> (String, String) {
    person_in_london(e).await;
    let origin = TurnOrigin { direct_kwant_id: Some(e.bot.id.clone()), ..Default::default() };
    let token = turn_in(e, &e.bot, "t-santa", origin).await;
    let (text, err) = tool(e, &token, "santa_create", json!({"spec": mira_spec()})).await;
    assert!(!err, "{text}");
    (code_of(&text), text)
}

/// The frontend's MATCH_KEYS: none may appear anywhere in a Santa block.
const MATCH_KEYS: [&str; 42] = [
    "match", "matches", "my_match", "pair", "pairs", "pairing", "pairings", "gives_to", "giving_to", "buys_for", "buying_for",
    "receives_from", "giver", "givers", "receiver", "receivers", "recipient", "recipients", "assigned", "assignment", "assignments",
    "santa_of", "my_santa", "loop", "ring", "reveal_loop", "derangement", "draw_result",
    "questions", "question", "asks", "ask", "answers", "answer", "q", "a", "text", "body", "wish", "wishes", "wish_list", "my_wish",
];

fn match_like(v: &Value, path: &str) -> Option<String> {
    match v {
        Value::Array(a) => a.iter().enumerate().find_map(|(i, x)| match_like(x, &format!("{path}[{i}]"))),
        Value::Object(o) => o.iter().find_map(|(k, x)| {
            if MATCH_KEYS.contains(&k.to_lowercase().as_str()) {
                Some(format!("{path}.{k}"))
            } else {
                match_like(x, &format!("{path}.{k}"))
            }
        }),
        _ => None,
    }
}

/// The frontend's `validSanta` and `validPocket` Santa rules, as far as a test can read them.
fn assert_valid_santa_block(b: &Value) {
    assert_eq!(b["kit"], "santa", "{b}");
    assert_eq!(b["source"], json!({"kind": "slot"}));
    assert_eq!(b["provenance"]["kind"], "kwant_data");
    assert!(b["join"].is_null() && b["results"].is_null(), "no top-level join or results: {b}");
    assert_eq!(match_like(b, ""), None, "{b}");
    let s = &b["santa"];
    let phase = s["phase"].as_str().unwrap();
    assert!(["collecting", "drawn", "reveal_day", "ended"].contains(&phase), "{s}");
    assert_eq!(b["state"], if phase == "ended" { "ended" } else { "live" });
    let joined = s["joined"].as_u64().expect("joined is a count");
    for k in ["looked", "gifts_ready"] {
        assert!(s[k].as_u64().is_none_or(|n| n <= joined), "{k}: {s}");
    }
    for p in s["people"].as_array().unwrap() {
        assert!(p["id"].is_string() && p["name"].as_str().is_some_and(|n| !n.is_empty() && n.len() <= 40), "{p}");
    }
    for (k, max) in [("draw_rule", 200), ("reveal_at", 40), ("deletes_on", 40), ("blocked", 300)] {
        assert!(s[k].is_null() || s[k].as_str().is_some_and(|t| !t.trim().is_empty() && t.encode_utf16().count() <= max), "{k}: {s}");
    }
    if !s["join"].is_null() {
        let mode = s["join"]["mode"].as_str().unwrap();
        assert!(["anywhere", "wifi", "none"].contains(&mode));
        assert!(!s["join"].to_string().contains("trycloudflare"));
        assert!(s["join"].get("quick_tunnel").is_none_or(|q| q != &json!(true)));
    }
}

#[tokio::test]
async fn santa_create_is_offered_beside_the_quizmaster_tools_only_while_pockets_are_on() {
    let e = setup(false).await;
    let token = turn(&e, &e.bot, "t-off").await;
    let names = tool_names(&e, &token).await;
    assert!(!names.iter().any(|n| n == "santa_create" || n == "pocket_create"), "{names:?}");
    pockets_on(&e).await;
    let token = turn(&e, &e.bot, "t-on").await;
    let names = tool_names(&e, &token).await;
    assert!(names.iter().any(|n| n == "santa_create"), "{names:?}");
    assert!(names.iter().any(|n| n == "pocket_create"), "{names:?}");
}

#[tokio::test]
async fn the_description_answers_every_name_for_it_and_forbids_drawing() {
    let e = setup(true).await;
    let token = turn(&e, &e.bot, "t-desc").await;
    let v = quizmaster_support::rpc(&e, &token, "tools/list", json!({})).await;
    let t = v["result"]["tools"].as_array().unwrap().iter().find(|t| t["name"] == "santa_create").expect("offered").clone();
    let d = t["description"].as_str().unwrap();
    for w in ["Secret Santa", "gift exchange", "Kris Kringle", "Wichteln"] {
        assert!(d.contains(w), "{w} in {d}");
    }
    assert!(d.to_lowercase().contains("never draw names yourself or post who gets whom"), "{d}");
}

#[tokio::test]
async fn with_no_spec_it_answers_the_fields_and_rules_and_makes_nothing() {
    let e = setup(true).await;
    let token = turn(&e, &e.bot, "t-rules").await;
    let (text, err) = tool(&e, &token, "santa_create", json!({})).await;
    assert!(!err, "{text}");
    for f in ["title", "gift_on", "reveal_at", "people", "keep_apart", "budget"] {
        assert!(text.contains(f), "{f} in {text}");
    }
    assert!(text.contains("never draw names"), "{text}");
    assert!(e.state.db.pocket_list().await.unwrap().is_empty(), "nothing made");
    // and a reply carrying that call gets no card
    let m = reply_as(&e, &e.bot, &e.bot.id, vec![call("santa_create", json!({}), "ok", &text)]).await;
    assert_eq!(pocket(&m), None);
}

#[tokio::test]
async fn miras_spec_makes_a_santa_pocket_owned_by_the_bot_from_its_own_chat() {
    let e = setup(true).await;
    let (code, text) = make(&e).await;
    assert!(text.to_lowercase().contains("never draw names yourself"), "{text}");
    let row = e.state.db.pocket_get(&code).await.unwrap().expect("the pocket exists");
    assert_eq!(row.kit, "santa");
    assert_eq!(row.owner_kwant_id.as_deref(), Some(e.bot.id.as_str()));
    assert_eq!(row.title, "Team Secret Santa");
    let sched = e.state.db.pocket_schedule(&code).await.unwrap().expect("a schedule row");
    let origin: Value = serde_json::from_str(sched.origin.as_deref().expect("an origin")).unwrap();
    assert_eq!(origin["kwant_id"], json!(e.bot.id), "{origin}");
}

#[tokio::test]
async fn the_reply_carrying_the_call_gets_the_santa_card() {
    let e = setup(true).await;
    let (code, text) = make(&e).await;
    let m = reply_as(&e, &e.bot, &e.bot.id, vec![call("santa_create", json!({"spec": mira_spec()}), "ok", &text)]).await;
    let b = pocket(&m).unwrap_or_else(|| panic!("a pocket block on the reply: {:?}", m.results));
    eprintln!("santa block: {b}");
    assert_valid_santa_block(&b);
    assert_eq!(b["code"], json!(code));
    assert_eq!(b["provenance"]["ref"], json!(format!("pocket:{code}")));
    assert_eq!(b["name"], "Team Secret Santa");
    assert_eq!(b["state"], "live");
    let d = gift_day();
    assert_eq!(b["when"], json!(format!("£20 each · gifts on {} · reveal at 7 PM", d.format("%a, %b %-d"))), "{b}");
    let s = &b["santa"];
    assert_eq!(s["phase"], "collecting");
    assert_eq!((s["joined"].clone(), s["expected"].clone()), (json!(0), json!(6)));
    assert_eq!(s["waiting_for"], json!(["Anna", "Ben", "Chloe", "Dev", "Erin", "Femi"]));
    assert_eq!(s["keep_apart"], json!([["Anna", "Ben"]]));
    assert_eq!(s["reveal_at"], "7 PM");
    assert_eq!(s["draw_rule"], "when all 6 are in");
    // No quiz parts ride along
    assert!(b["draft"].is_null() && b["checks"].is_null() && b["dry_run"].is_null(), "{b}");
}

#[tokio::test]
async fn once_everyone_joined_and_names_are_drawn_the_card_still_names_no_match() {
    let e = setup(true).await;
    let (code, text) = make(&e).await;
    for (i, name) in ["Anna", "Ben", "Chloe", "Dev", "Erin", "Femi"].iter().enumerate() {
        let req = serde_json::from_value(json!({"name": name, "pin": format!("{:04}", 1000 + i)})).unwrap();
        kwnt::pocket::join(&e.state, &code, req).await.unwrap_or_else(|err| panic!("{name} joins: {}", err.message));
    }
    let m = reply_as(&e, &e.bot, &e.bot.id, vec![call("santa_create", json!({"spec": mira_spec()}), "ok", &text)]).await;
    let b = pocket(&m).expect("a pocket block");
    assert_valid_santa_block(&b);
    let s = &b["santa"];
    assert_eq!(s["phase"], "drawn", "all 6 in draws the names: {s}");
    assert_eq!(s["joined"], 6);
    assert_eq!(s["looked"], 0);
    assert_eq!(s["people"].as_array().unwrap().len(), 6);
    // The card shows names and ticks, never who gives to whom: no player id appears next to another's name
    for p in s["people"].as_array().unwrap() {
        let keys: Vec<&str> = p.as_object().unwrap().keys().map(String::as_str).collect();
        assert!(keys.iter().all(|k| ["id", "name", "avatar", "looked", "gift_ready"].contains(k)), "{p}");
    }
}

#[tokio::test]
async fn a_bad_spec_answers_with_the_kits_own_words_and_makes_nothing() {
    let e = setup(true).await;
    let token = turn(&e, &e.bot, "t-bad").await;
    let mut spec = mira_spec();
    spec.as_object_mut().unwrap().remove("gift_on");
    let (text, err) = tool(&e, &token, "santa_create", json!({"spec": spec})).await;
    assert!(err, "{text}");
    assert!(text.contains("gift_on"), "the validator's words: {text}");
    assert!(text.starts_with("Nothing was made"), "{text}");
    // A past gift day, a budget too long, too few people: each a sentence, never a panic
    let mut past = mira_spec();
    past["gift_on"] = json!("2020-12-18");
    past["reveal_at"] = json!("2020-12-18T19:00:00+00:00");
    let mut long_budget = mira_spec();
    long_budget["budget"] = json!("twenty pounds each, no more, no less, honest");
    let mut few = mira_spec();
    few["expected"] = json!(2);
    for bad in [past, long_budget, few, json!("not json {"), json!([1, 2])] {
        let (text, err) = tool(&e, &token, "santa_create", json!({"spec": bad})).await;
        assert!(err && !text.is_empty() && !text.contains("panic"), "{text}");
    }
    assert!(e.state.db.pocket_list().await.unwrap().is_empty(), "nothing made");
    // A failed call carries no card
    let m = reply_as(&e, &e.bot, &e.bot.id, vec![call("santa_create", json!({"spec": {}}), "error", &text)]).await;
    assert_eq!(pocket(&m), None);
}

#[tokio::test]
async fn another_bots_reply_never_carries_this_bots_santa() {
    let e = setup(true).await;
    let (_code, text) = make(&e).await;
    let other = e.other.clone();
    let m = reply_as(&e, &other, &other.id, vec![call("santa_create", json!({"spec": mira_spec()}), "ok", &text)]).await;
    assert_eq!(pocket(&m), None, "{:?}", m.results);
    // nor this bot's reply in another bot's chat
    let bot = e.bot.clone();
    let m = reply_as(&e, &bot, &other.id, vec![call("santa_create", json!({"spec": mira_spec()}), "ok", &text)]).await;
    assert_eq!(pocket(&m), None, "{:?}", m.results);
}

#[tokio::test]
async fn pockets_off_refuses_and_no_card_is_attached() {
    let e = setup(true).await;
    let (_code, text) = make(&e).await;
    e.state.db.set_setting(kwnt::server::quizmaster_tools::SETTING, "off").await.unwrap();
    let token = turn(&e, &e.bot, "t-off2").await;
    let (t, err) = tool(&e, &token, "santa_create", json!({"spec": mira_spec()})).await;
    assert!(err, "{t}");
    let m = reply_as(&e, &e.bot, &e.bot.id, vec![call("santa_create", json!({"spec": mira_spec()}), "ok", &text)]).await;
    assert_eq!(pocket(&m), None);
}
