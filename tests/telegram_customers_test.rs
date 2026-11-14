//! Customer chats on Telegram (roadmap 10b) against the mock Bot API: a business's customers
//! write to a second bot, and the answering bot's replies go back to that customer's chat only.
//! Fake tokens only, never api.telegram.org. The person's own link is tested in telegram_test.rs,
//! unchanged; here it is checked only beside customer chats.

mod common;
#[path = "support/mock_telegram.rs"]
mod mock_telegram;

use axum::http::StatusCode;
use common::{call, call_tool_marker, call_with, invocations_with, setup, Env};
use kwnt::db::spaces::SpaceId;
use kwnt::drivers::CUSTOMER_NOTE;
use kwnt::server::telegram::customers::copy;
use mock_telegram::{Bot, MockTelegram, Person, PERSON};
use serde_json::{json, Value};
use std::time::Duration;

const BASE: &str = "/api/connections/telegram/customers";
const SPACE: &str = "spc_default";

const ANN: Person = Person { id: 5101, first_name: "Ann", username: "ann_customer" };
const BEN: Person = Person { id: 5102, first_name: "Ben", username: "ben_customer" };
const CAT: Person = Person { id: 5103, first_name: "Cat", username: "cat_customer" };

/// [`common::wait_for`] with 60 s, as in telegram_test.rs (busy runners).
async fn wait_for<F, Fut>(what: &str, mut check: F)
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = bool>,
{
    for _ in 0..600 {
        if check().await {
            return;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    panic!("timed out waiting for {what}");
}

async fn status(env: &Env) -> Value {
    let (s, v) = call(&env.app, "GET", BASE, None).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    v
}

/// A fresh mock bot saved as the customer bot, answered by bot A; kwant is polling it.
async fn customers_on(env: &Env, tg: &MockTelegram, n: u32) -> Bot {
    let bot = tg.bot(n);
    let (s, v) = call(&env.app, "PUT", &format!("{BASE}/token"), Some(json!({"token": bot.token}))).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    let (s, v) = call(&env.app, "PATCH", BASE, Some(json!({"kwant_id": env.a}))).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(v["state"], "on", "{v}");
    tg.wait_call(&bot, "getUpdates", |_| true).await;
    bot
}

/// Every sendMessage to anyone but `chat`.
fn sent_elsewhere(tg: &MockTelegram, bot: &Bot, chat: i64) -> Vec<String> {
    tg.sent(bot).iter().filter(|c| c.chat() != chat).map(|c| format!("{}: {}", c.chat(), c.text())).collect()
}

async fn chat_row(env: &Env, who: &Person) -> kwnt::db::telegram_customers::CustomerChat {
    env.state.db.customer_chat(SPACE, who.id).await.unwrap().expect("a customer chat row")
}

fn meta(m: &kwnt::db::models::Message) -> Value {
    serde_json::from_str(m.meta.as_deref().unwrap_or("{}")).unwrap_or(Value::Null)
}

// --- Settings -------------------------------------------------------------------------------

#[tokio::test]
async fn off_by_default_and_on_only_with_a_token_and_a_bot() {
    let env = setup().await;
    let tg = mock_telegram::shared();
    let v = status(&env).await;
    assert_eq!(v["state"], "off", "{v}");
    assert_eq!(v["limits"], json!({"max_chats": 200, "per_hour": 20}));
    assert!(v["default_kwant_id"].is_null(), "no pack applied, so no default bot: {v}");

    // A token alone: saved, not listening (no pack lead to answer)
    let bot = tg.bot(1);
    let (s, v) = call(&env.app, "PUT", &format!("{BASE}/token"), Some(json!({"token": bot.token}))).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(v["state"], "no_bot", "{v}");
    assert_eq!(v["bot"], json!({"username": bot.username, "name": bot.name}));
    tokio::time::sleep(Duration::from_millis(500)).await;
    assert!(tg.calls_of(&bot, "getUpdates").is_empty(), "nobody answers, so nothing is read");

    // A bot from another space or a made-up id is refused
    let (s, v) = call(&env.app, "PATCH", BASE, Some(json!({"kwant_id": "nope"}))).await;
    assert_eq!((s, v["error"]["code"].as_str()), (StatusCode::BAD_REQUEST, Some("no_such_bot")), "{v}");
    let (s, v) = call(&env.app, "PATCH", BASE, Some(json!({"kwant_id": 7}))).await;
    assert_eq!((s, v["error"]["code"].as_str()), (StatusCode::BAD_REQUEST, Some("bad_kwant_id")), "{v}");

    // Picking a bot turns it on; null turns it off again
    let (s, v) = call(&env.app, "PATCH", BASE, Some(json!({"kwant_id": env.a}))).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(v["state"], "on");
    assert_eq!(v["kwant"]["id"], env.a.as_str());
    tg.wait_call(&bot, "getUpdates", |_| true).await;
    wait_for("listening", || async { status(&env).await["status"] == "listening" }).await;
    let (_, v) = call(&env.app, "PATCH", BASE, Some(json!({"kwant_id": null}))).await;
    assert_eq!(v["state"], "no_bot", "{v}");
    assert!(kwnt::server::telegram::customers::runner(&env.state, SPACE).is_none());

    // Delete forgets the token
    let (s, v) = call(&env.app, "DELETE", BASE, None).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(v["state"], "off");
    assert!(kwnt::server::telegram::customers::load_token(&env.state, SPACE).await.unwrap().is_none());
    // Its own vault owner: the person's link has no token
    assert!(kwnt::server::telegram::load_token(&env.state, SPACE).await.unwrap().is_none());
}

#[tokio::test]
async fn only_the_person_sets_up_customer_chats() {
    let env = setup().await;
    let auth = [("authorization", "Bearer not-the-person")];
    for (m, uri) in [
        ("GET", BASE.to_string()),
        ("PUT", format!("{BASE}/token")),
        ("PATCH", BASE.to_string()),
        ("DELETE", BASE.to_string()),
        ("POST", format!("{BASE}/chats/5101/reply")),
    ] {
        let body = match m {
            "PUT" => Some(json!({"token": mock_telegram::fake_token(2).1})),
            "POST" => Some(json!({"text": "hi"})),
            _ => Some(json!({})),
        };
        let (s, _) = call_with(&env.app, m, &uri, body, &auth).await;
        assert_eq!(s, StatusCode::FORBIDDEN, "{m} {uri}");
    }
    assert_ne!(common::call_remote(&env.app, "PUT", &format!("{BASE}/token")).await, StatusCode::OK);
    let (s, v) = call(&env.app, "PUT", &format!("{BASE}/token"), Some(json!({"token": "hello"}))).await;
    assert_eq!((s, v["error"]["code"].as_str()), (StatusCode::BAD_REQUEST, Some("token_shape")));
    let (_, unknown) = mock_telegram::fake_token(9_998);
    let (s, v) = call(&env.app, "PUT", &format!("{BASE}/token"), Some(json!({"token": unknown}))).await;
    assert_eq!((s, v["error"]["code"].as_str()), (StatusCode::BAD_REQUEST, Some("token_rejected")), "{v}");
    assert!(!v.to_string().contains(&unknown));
}

// --- Inbound and outbound -------------------------------------------------------------------

#[tokio::test]
async fn each_customer_gets_their_own_thread_and_only_their_reply() {
    let env = setup().await;
    let tg = mock_telegram::shared();
    let bot = customers_on(&env, tg, 10).await;

    tg.text(&bot, &ANN, "When do you open? REPLYTAG:ann10");
    tg.text(&bot, &BEN, "Do you ship to Spain? REPLYTAG:ben10");
    let to_ann = tg.wait_send(&bot, |c| c.text().contains("answer ann10")).await;
    let to_ben = tg.wait_send(&bot, |c| c.text().contains("answer ben10")).await;
    assert_eq!(to_ann.chat(), ANN.id);
    assert_eq!(to_ben.chat(), BEN.id);
    assert!(tg.sent(&bot).iter().all(|c| c.buttons().is_empty()), "plain text, no buttons");
    assert!(tg.sent(&bot).iter().all(|c| c.body.get("parse_mode").is_none()), "plain text");

    // Two threads with bot A, named after the customers
    let (ann, ben) = (chat_row(&env, &ANN).await, chat_row(&env, &BEN).await);
    assert_ne!(ann.thread_id, ben.thread_id);
    for (row, name) in [(&ann, "Ann"), (&ben, "Ben")] {
        let t = env.state.db.get_thread(&row.thread_id).await.unwrap().unwrap();
        assert_eq!((t.title.as_str(), t.kwant_id.as_str(), t.created_by.as_str()), (name, env.a.as_str(), "telegram-customer"));
        assert_eq!(row.kwant_id, env.a);
    }
    let msgs = env.state.db.list_messages(&ann.thread_id, 100).await.unwrap();
    let hers = msgs.iter().find(|m| m.sender_role == "user").expect("Ann's message");
    assert_eq!(hers.sender_id, format!("telegram-customer:{}", ANN.id));
    assert_eq!(meta(hers)["source"], "telegram-customer");
    assert_eq!(meta(hers)["from_customer"], "Ann");
    assert!(msgs.iter().any(|m| m.sender_role == "kwant" && m.text.contains("answer ann10")), "the reply is in the thread too");
    assert!(!msgs.iter().any(|m| m.text.contains("ben10")), "nothing of Ben's in Ann's thread");

    // The bot read Ann's words as a customer's, with the note
    let prompt = invocations_with("ann10").pop().expect("Ann's turn");
    assert!(prompt.contains(CUSTOMER_NOTE), "{prompt}");
    assert!(prompt.contains("outside person"), "{prompt}");

    // Ann again: the same thread
    tg.text(&bot, &ANN, "And on Sunday? REPLYTAG:ann10b");
    let again = tg.wait_send(&bot, |c| c.text().contains("answer ann10b")).await;
    assert_eq!(again.chat(), ANN.id);
    assert_eq!(chat_row(&env, &ANN).await.thread_id, ann.thread_id);
    assert_eq!(env.state.db.customer_chat_count(SPACE).await.unwrap(), 2);
    let v = status(&env).await;
    assert_eq!(v["counts"]["chats"], 2, "{v}");
    assert_eq!(v["counts"]["active"], 2, "{v}");
}

#[tokio::test]
async fn groups_channels_bots_and_forwards_are_ignored() {
    let env = setup().await;
    let tg = mock_telegram::shared();
    let bot = customers_on(&env, tg, 20).await;

    let mut group = tg.message(&bot, &ANN, "hi REPLYTAG:g20");
    group["chat"] = json!({"id": -100500, "type": "group", "title": "A group"});
    tg.inject(&bot, json!({"message": group}));
    let mut supergroup = tg.message(&bot, &ANN, "hi REPLYTAG:g20");
    supergroup["chat"] = json!({"id": -100501, "type": "supergroup", "title": "A supergroup"});
    tg.inject(&bot, json!({"message": supergroup}));
    tg.inject(&bot, json!({"channel_post": {"message_id": 1, "chat": {"id": -100502, "type": "channel"}, "date": 1, "text": "post"}}));
    let mut from_bot = tg.message(&bot, &ANN, "hi REPLYTAG:g20");
    from_bot["from"]["is_bot"] = json!(true);
    tg.inject(&bot, json!({"message": from_bot}));
    let mut fwd = tg.message(&bot, &ANN, "hi REPLYTAG:g20");
    fwd["forward_origin"] = json!({"type": "user", "date": 1, "sender_user": {"id": 1, "is_bot": false, "first_name": "X"}});
    tg.inject(&bot, json!({"message": fwd}));
    let mut old_fwd = tg.message(&bot, &ANN, "hi REPLYTAG:g20");
    old_fwd["forward_from"] = json!({"id": 1, "is_bot": false, "first_name": "X"});
    tg.inject(&bot, json!({"message": old_fwd}));
    let mut as_channel = tg.message(&bot, &ANN, "hi REPLYTAG:g20");
    as_channel["sender_chat"] = json!({"id": -100503, "type": "channel"});
    tg.inject(&bot, json!({"message": as_channel}));
    let edited = tg.message(&bot, &ANN, "hi REPLYTAG:g20");
    let last = tg.inject(&bot, json!({"edited_message": edited}));
    tg.wait_handled(&bot, last).await;
    tokio::time::sleep(Duration::from_millis(500)).await;
    assert!(tg.sent(&bot).is_empty(), "no reply to any of them: {:?}", tg.sent(&bot));
    assert_eq!(env.state.db.customer_chat_count(SPACE).await.unwrap(), 0, "no thread for any of them");
    assert!(invocations_with("g20").is_empty(), "no bot read any of them");

    // Not text: the fixed line, no thread and no turn
    let mut photo = tg.message(&bot, &BEN, "");
    photo.as_object_mut().unwrap().remove("text");
    photo["photo"] = json!([{"file_id": "x", "width": 1, "height": 1}]);
    tg.inject(&bot, json!({"message": photo}));
    let told = tg.wait_send(&bot, |c| c.text() == "I can read text messages only for now.").await;
    assert_eq!(told.chat(), BEN.id);
}

#[tokio::test]
async fn the_persons_paired_chat_still_works_beside_customer_chats() {
    let env = setup().await;
    let tg = mock_telegram::shared();
    // The person's own link, paired as in telegram_test.rs
    let mine = tg.bot(30);
    let (s, v) = call(&env.app, "PUT", "/api/connections/telegram/token", Some(json!({"token": mine.token}))).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    tg.wait_call(&mine, "getUpdates", |_| true).await;
    let (_, v) = call(&env.app, "POST", "/api/connections/telegram/pair", None).await;
    let code = v["link"].as_str().unwrap().split("start=").nth(1).unwrap().to_string();
    tg.text(&mine, &PERSON, &format!("/start {code}"));
    tg.wait_send(&mine, |c| c.text().contains("Paired with kwant")).await;

    // The person's own bot can't be the customer bot
    let (s, v) = call(&env.app, "PUT", &format!("{BASE}/token"), Some(json!({"token": mine.token}))).await;
    assert_eq!((s, v["error"]["code"].as_str()), (StatusCode::CONFLICT, Some("same_bot")), "{v}");
    assert!(kwnt::server::telegram::customers::load_token(&env.state, SPACE).await.unwrap().is_none());

    let shop = customers_on(&env, tg, 31).await;
    tg.text(&shop, &ANN, "Is the blue one in stock? REPLYTAG:ann30");
    let to_ann = tg.wait_send(&shop, |c| c.text().contains("answer ann30")).await;
    assert_eq!(to_ann.chat(), ANN.id);

    // The person's chat still works, and never gets the customer's words or the reply
    let name = env.state.db.get_kwant(&env.a).await.unwrap().unwrap().name.split_whitespace().next().unwrap().to_string();
    tg.text(&mine, &PERSON, &format!("@{name} hello from the train REPLYTAG:me30"));
    let back = tg.wait_send(&mine, |c| c.text().contains("answer me30")).await;
    assert_eq!(back.chat(), PERSON.id);
    assert!(tg.sent(&mine).iter().all(|c| !c.text().contains("ann30") && !c.text().contains("blue one")), "{:?}", tg.sent(&mine));
    assert!(sent_elsewhere(tg, &shop, ANN.id).is_empty(), "{:?}", sent_elsewhere(tg, &shop, ANN.id));
    let (_, v) = call(&env.app, "GET", "/api/connections/telegram", None).await;
    assert_eq!(v["state"], "paired", "{v}");
}

// --- Authority ------------------------------------------------------------------------------

#[tokio::test]
async fn a_tool_call_waits_for_the_person_even_with_approvals_off() {
    let env = setup().await;
    let tg = mock_telegram::shared();
    env.state.db.set_space_approvals(&SpaceId::default_space(), "off").await.unwrap();
    let bot = customers_on(&env, tg, 40).await;
    let marker = call_tool_marker("memory_search", json!({"query": "opening hours"}));
    tg.text(&bot, &ANN, &format!("Please look this up {marker} REPLYTAG:tool40"));

    // A read-only tool that runs unasked with approvals off still gets a card
        wait_for("the card", || {
        let db = env.state.db.clone();
        let a = env.a.clone();
        async move { !db.list_approvals(&SpaceId::default_space(), Some("pending"), Some(&a)).await.unwrap().is_empty() }
    })
    .await;
    let pending = env.state.db.list_approvals(&SpaceId::default_space(), Some("pending"), Some(&env.a)).await.unwrap();
    let card = pending.first().unwrap();
    assert!(card.tool_name.contains("memory_search"), "{}", card.tool_name);
    let details: Value = serde_json::from_str(&card.parameters).unwrap_or(Value::Null);
    assert_eq!(details["customer_chat"], true, "{details}");
    assert!(tg.sent(&bot).is_empty(), "nothing goes to the customer while the person decides: {:?}", tg.sent(&bot));
    assert!(env.state.db.list_approvals(&SpaceId::default_space(), Some("auto_allowed"), Some(&env.a)).await.unwrap().is_empty());

    // The person says no; the bot's reply goes out, without the card
    let mut s = StatusCode::CONFLICT;
    for _ in 0..100 {
        s = call(&env.app, "POST", &format!("/api/approvals/{}/resolve", card.id), Some(json!({"decision": "deny"}))).await.0;
        if s != StatusCode::CONFLICT {
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert_eq!(s, StatusCode::OK);
    let reply = tg.wait_send(&bot, |c| c.text().contains("answer tool40")).await;
    assert_eq!(reply.chat(), ANN.id);
    for c in tg.sent(&bot) {
        assert!(c.buttons().is_empty() && !c.text().contains(&card.id), "never an approval: {}", c.text());
    }
}

#[tokio::test]
async fn a_bot_that_cant_answer_hands_the_chat_to_the_person() {
    use base64::Engine;
    let env = setup().await;
    let tg = mock_telegram::shared();
    let bot = customers_on(&env, tg, 50).await;
    let says = base64::engine::general_purpose::STANDARD.encode("[[person]] Asks for a refund on order 1234");
    tg.text(&bot, &ANN, &format!("I want my money back REPLYTEXT:{says}"));
    let told = tg.wait_send(&bot, |c| c.text() == copy::HANDED_OFF).await;
    assert_eq!(told.chat(), ANN.id);
    assert!(tg.sent(&bot).iter().all(|c| !c.text().contains("[[person]]") && !c.text().contains("refund")), "the reason is for the person");

    // A Needs-you item that opens the thread
    let thread = chat_row(&env, &ANN).await.thread_id;
    let (_, v) = call(&env.app, "GET", "/api/waiting", None).await;
    let item = v["items"].as_array().unwrap().iter().find(|i| i["kind"] == "customer_chat").cloned().expect("a customer item");
    assert_eq!(item["thread_id"], thread.as_str());
    assert_eq!(item["customer_chat_id"], ANN.id);
    assert_eq!(item["text"], "Asks for a refund on order 1234");
    assert_eq!(status(&env).await["counts"]["waiting_on_person"], 1);

    // The person answers; it goes to Ann, is kept in the thread, and the item goes
    let (s, v) = call(&env.app, "POST", &format!("{BASE}/chats/{}/reply", ANN.id), Some(json!({"text": "Refunded, sorry about that."}))).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    let mine = tg.wait_send(&bot, |c| c.text() == "Refunded, sorry about that.").await;
    assert_eq!(mine.chat(), ANN.id);
    let msgs = env.state.db.list_messages(&thread, 100).await.unwrap();
    assert!(msgs.iter().any(|m| m.sender_id == "user" && m.text == "Refunded, sorry about that."));
    let (_, v) = call(&env.app, "GET", "/api/waiting", None).await;
    assert!(v["items"].as_array().unwrap().iter().all(|i| i["kind"] != "customer_chat"), "{v}");
    let (s, _) = call(&env.app, "POST", &format!("{BASE}/chats/999/reply"), Some(json!({"text": "x"}))).await;
    assert_eq!(s, StatusCode::NOT_FOUND);

    // Lines into kwant never reach a customer; a reply of nothing else hands off
    let link = base64::engine::general_purpose::STANDARD.encode("Sure.\nOpen http://127.0.0.1:7777/api/threads/1 to see it");
    tg.text(&bot, &BEN, &format!("Where is my order? REPLYTEXT:{link}"));
    let sure = tg.wait_send(&bot, |c| c.chat() == BEN.id).await;
    assert_eq!(sure.text(), "Sure.");
}

// --- Limits ---------------------------------------------------------------------------------

#[tokio::test]
async fn one_turn_at_a_time_per_chat_and_the_rest_wait_together() {
    let env = setup().await;
    let tg = mock_telegram::shared();
    let bot = customers_on(&env, tg, 60).await;
    tg.text(&bot, &ANN, "first SLEEPONCE:q60:2000 REPLYTAG:q60a");
    // Let the first turn start, then two more while it runs
    wait_for("the first turn", || async { !invocations_with("q60a").is_empty() }).await;
    tg.text(&bot, &ANN, "second REPLYTAG:q60b");
    let last = tg.text(&bot, &ANN, "third REPLYTAG:q60c");
    tg.wait_handled(&bot, last).await;
    tg.wait_send(&bot, |c| c.text().contains("answer q60a")).await;
    tg.wait_send(&bot, |c| c.text().contains("answer q60c")).await;
    tokio::time::sleep(Duration::from_millis(1500)).await;
    // Two turns, not three: the second and third went in one
    let turns = invocations_with("q60").len();
    assert_eq!(turns, 2, "turns: {turns}");
    assert!(tg.sent(&bot).iter().all(|c| !c.text().contains("answer q60b")));
    let thread = chat_row(&env, &ANN).await.thread_id;
    let mine: Vec<_> = env.state.db.list_messages(&thread, 100).await.unwrap().into_iter().filter(|m| m.sender_role == "user").collect();
    assert_eq!(mine.len(), 3, "every message is kept in the thread");
}

#[tokio::test]
async fn twenty_an_hour_per_chat_then_one_slow_down() {
    let env = setup().await;
    let tg = mock_telegram::shared();
    let bot = customers_on(&env, tg, 70).await;
    for i in 0..22 {
        tg.text(&bot, &ANN, &format!("message {i} REPLYTAG:r70"));
    }
    tg.wait_send(&bot, |c| c.text() == copy::SLOW_DOWN).await;
    tg.wait_send(&bot, |c| c.text().contains("answer r70")).await;
    tokio::time::sleep(Duration::from_millis(1500)).await;
    let slow = tg.sent(&bot).iter().filter(|c| c.text() == copy::SLOW_DOWN).count();
    assert_eq!(slow, 1, "told once an hour");
    let thread = chat_row(&env, &ANN).await.thread_id;
    let kept = env.state.db.list_messages(&thread, 100).await.unwrap().into_iter().filter(|m| m.sender_role == "user").count();
    assert_eq!(kept, 20, "the 21st and 22nd never reached the bot");
    let v = status(&env).await;
    assert_eq!(v["counts"]["limited"], 2, "{v}");
    // Another customer isn't limited by Ann
    tg.text(&bot, &BEN, "hello REPLYTAG:r70ben");
    assert_eq!(tg.wait_send(&bot, |c| c.text().contains("answer r70ben")).await.chat(), BEN.id);
}

#[tokio::test]
async fn at_most_two_hundred_chats_per_space() {
    let env = setup().await;
    let tg = mock_telegram::shared();
    let bot = customers_on(&env, tg, 80).await;
    for i in 0..200 {
        env.state.db.customer_save_chat(SPACE, 900_000 + i, &format!("thread-{i}"), &env.a, "Someone").await.unwrap();
    }
    let last = tg.text(&bot, &CAT, "hello REPLYTAG:cap80");
    tg.wait_handled(&bot, last).await;
    tokio::time::sleep(Duration::from_millis(500)).await;
    assert!(tg.sent(&bot).is_empty(), "{:?}", tg.sent(&bot));
    assert!(env.state.db.customer_chat(SPACE, CAT.id).await.unwrap().is_none());
    let v = status(&env).await;
    assert_eq!(v["counts"]["refused_new"], 1, "{v}");
    assert_eq!(v["counts"]["chats"], 200, "{v}");
}

#[tokio::test]
async fn stop_ends_a_chat_and_start_opens_it_again() {
    let env = setup().await;
    let tg = mock_telegram::shared();
    let bot = customers_on(&env, tg, 90).await;
    tg.text(&bot, &ANN, "hi REPLYTAG:s90");
    tg.wait_send(&bot, |c| c.text().contains("answer s90")).await;
    tg.text(&bot, &ANN, "/stop");
    tg.wait_send(&bot, |c| c.text() == copy::STOPPED).await;
    assert!(chat_row(&env, &ANN).await.stopped_at.is_some());
    let before = tg.sent(&bot).len();
    let last = tg.text(&bot, &ANN, "still there? REPLYTAG:s90b");
    tg.wait_handled(&bot, last).await;
    tokio::time::sleep(Duration::from_millis(1000)).await;
    assert_eq!(tg.sent(&bot).len(), before, "a stopped chat gets nothing");
    assert!(invocations_with("s90b").is_empty(), "and no bot reads it");
    assert_eq!(status(&env).await["counts"]["stopped"], 1);
    let (s, _) = call(&env.app, "POST", &format!("{BASE}/chats/{}/reply", ANN.id), Some(json!({"text": "x"}))).await;
    assert_eq!(s, StatusCode::CONFLICT, "the person can't write to a stopped chat either");

    tg.text(&bot, &ANN, "/start");
    tg.wait_send(&bot, |c| c.text() == copy::STARTED_AGAIN).await;
    tg.text(&bot, &ANN, "back again REPLYTAG:s90c");
    assert_eq!(tg.wait_send(&bot, |c| c.text().contains("answer s90c")).await.chat(), ANN.id);
}

// --- The token ------------------------------------------------------------------------------

#[tokio::test]
async fn the_token_never_reaches_events_the_database_files_or_a_bot() {
    let env = setup().await;
    let tg = mock_telegram::shared();
    let mut events = env.state.event_sender.subscribe();
    let bot = customers_on(&env, tg, 100).await;
    tg.text(&bot, &ANN, "hello REPLYTAG:t100");
    tg.wait_send(&bot, |c| c.text().contains("answer t100")).await;
    let secret = bot.token.split_once(':').unwrap().1.to_string();

    assert!(!status(&env).await.to_string().contains(&secret));
    let (_, list) = call(&env.app, "GET", "/api/connections", None).await;
    assert!(!list.to_string().contains(&secret));
    let mut seen = Vec::new();
    while let Ok(e) = events.try_recv() {
        seen.push(serde_json::to_string(&e).unwrap_or_default());
    }
    assert!(!seen.is_empty());
    assert!(seen.iter().all(|e| !e.contains(&secret)), "an event has the token");
    // Every file kwant wrote (the database, its WAL, logs, turn folders)
    fn scan(dir: &std::path::Path, secret: &str) {
        for entry in std::fs::read_dir(dir).into_iter().flatten().flatten() {
            let p = entry.path();
            if p.is_dir() {
                scan(&p, secret);
            } else if let Ok(bytes) = std::fs::read(&p) {
                assert!(!bytes.windows(secret.len()).any(|w| w == secret.as_bytes()), "token in {}", p.display());
            }
        }
    }
    scan(&env.state.config.data_dir, &secret);
    // Every bot invocation in this test binary (all arguments, prompts included)
    assert!(invocations_with(&secret).is_empty(), "a bot saw the token");
    // In the vault, under its own owner
    let saved = kwnt::server::telegram::customers::load_token(&env.state, SPACE).await.unwrap().unwrap();
    assert_eq!(saved.as_str(), bot.token);
}

#[tokio::test]
async fn a_revoked_token_stops_listening_and_says_so() {
    let env = setup().await;
    let tg = mock_telegram::shared();
    let bot = customers_on(&env, tg, 110).await;
    tg.revoke(&bot);
    wait_for("rejected", || async { status(&env).await["state"] == "rejected" }).await;
    assert!(kwnt::server::telegram::customers::runner(&env.state, SPACE).is_none());
    assert_eq!(status(&env).await["error"], "token_rejected");
}
