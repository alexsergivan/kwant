//! H4 + H3 through the routes: a bot that asks the person shows `needs_you` with why and since;
//! a message to it in another chat is held (saved, shown as held, no turn) and goes through once
//! the person answers; the same `client_message_id` is sent once. See
//! `release-090/h4-contract.md`.

mod common;

use axum::http::StatusCode;
use common::*;
use serde_json::{json, Value};
use std::time::Duration;

fn ask_marker(question: &str, choices: &[&str]) -> String {
    call_tool_marker(
        "ask_user",
        json!({"question": question, "choices": choices, "why_you": "irreversible", "checked": "Read the release notes."}),
    )
}

async fn open_question(env: &Env, ctx: &str) -> Value {
    for _ in 0..300 {
        let (s, list) = call(&env.app, "GET", &format!("/api/questions?status=open&context_id={ctx}"), None).await;
        assert_eq!(s, StatusCode::OK);
        if let Some(q) = list.as_array().and_then(|l| l.first()) {
            return q.clone();
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    panic!("no open question in {ctx}");
}

async fn status_of(env: &Env, kwant: &str) -> Value {
    let (s, v) = call(&env.app, "GET", "/api/bots/status", None).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert!(v["now"].is_string(), "{v}");
    v["bots"].as_array().unwrap().iter().find(|b| b["kwant_id"] == kwant).cloned().unwrap_or_else(|| panic!("{kwant} missing: {v}"))
}

async fn send(env: &Env, body: Value) -> (StatusCode, Value) {
    call(&env.app, "POST", "/api/chat/send", Some(body)).await
}

#[tokio::test]
async fn a_bot_waiting_on_the_person_holds_other_messages_until_the_answer_and_a_retry_is_sent_once() {
    let env = setup().await;
    let idle = status_of(&env, &env.a).await;
    assert_eq!((idle["status"].as_str(), idle["reason_code"].as_str()), (Some("available"), Some("idle")), "{idle}");

    send_direct(&env, &env.a, &format!("Ship it? {} REPLYTAG:q1", ask_marker("Which branch should I ship?", &["main", "batch2"])), json!({})).await;
    let q = open_question(&env, &env.a).await;
    let qid = q["id"].as_str().unwrap().to_string();

    // H4: needs you, because it asked, since when it asked
    let st = status_of(&env, &env.a).await;
    assert_eq!((st["status"].as_str(), st["reason_code"].as_str()), (Some("needs_you"), Some("asked_question")), "{st}");
    assert_eq!(st["question"]["id"], qid.as_str());
    assert_eq!(st["question"]["context_id"], env.a.as_str());
    assert_eq!(st["since"], st["question"]["asked_at"]);
    assert_eq!(st["waits_on"]["kind"], "person");
    assert_eq!(st["turn"]["context_id"], env.a.as_str());
    let other = status_of(&env, &env.b).await;
    assert_eq!(other["status"], "available", "{other}");

    // H3: a message in another chat of that bot is saved and held, not run
    let (s, thread) = call(&env.app, "POST", &format!("/api/kwants/{}/threads", env.a), Some(json!({"title": "side"}))).await;
    assert_eq!(s, StatusCode::CREATED);
    let tid = thread["id"].as_str().unwrap().to_string();
    let body = json!({"prompt": "and the docs? REPLYTAG:h1", "sender_id": "user", "thread_id": tid, "client_message_id": "cm-01HF.1"});
    let (s, v) = send(&env, body.clone()).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(v["status"], "held", "{v}");
    let mid = v["user_message_id"].as_str().unwrap().to_string();
    assert_eq!((v["held"]["kwant_id"].as_str(), v["held"]["reason_code"].as_str()), (Some(env.a.as_str()), Some("waits_on_you")), "{v}");
    assert_eq!(v["held"]["question"]["id"], qid.as_str());
    assert!(v["held"]["kwant_name"].is_string() && v["held"]["since"].is_string(), "{v}");
    assert!(messages(&env, &tid).await.iter().any(|m| m.id == mid), "a held message is saved and shows");

    let st = status_of(&env, &env.a).await;
    let held = st["held"].as_array().unwrap_or_else(|| panic!("no held list: {st}"));
    assert_eq!(held.len(), 1, "{st}");
    assert_eq!((held[0]["kind"].as_str(), held[0]["id"].as_str()), (Some("message"), Some(mid.as_str())));
    assert_eq!((held[0]["reason_code"].as_str(), held[0]["context_id"].as_str()), (Some("waits_on_you"), Some(tid.as_str())));

    // H3: the same client message id again (a retry after a timeout) is not sent twice
    let (s, again) = send(&env, body.clone()).await;
    assert_eq!(s, StatusCode::OK, "{again}");
    assert_eq!((again["status"].as_str(), again["deduped"].as_bool()), (Some("already_delivered"), Some(true)), "{again}");
    assert_eq!(again["user_message_id"], mid.as_str());
    assert!(again["already_delivered_at"].is_string());
    let users = |msgs: Vec<kwnt::db::models::Message>| msgs.into_iter().filter(|m| m.sender_role == "user").count();
    assert_eq!(users(messages(&env, &tid).await), 1, "nothing new saved");
    let (s, _) = send(&env, json!({"prompt": "x", "sender_id": "user", "thread_id": tid, "client_message_id": "not ok"})).await;
    assert_eq!(s, StatusCode::BAD_REQUEST);

    // Nothing ran in the thread while held
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert!(!messages(&env, &tid).await.iter().any(|m| m.sender_role == "kwant"), "held: no reply yet");

    // The answer: the question's turn goes on, then the held message gets its turn
    let (s, v) = call(&env.app, "POST", &format!("/api/questions/{qid}/answer"), Some(json!({"choice": 1}))).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    wait_reply(&env, &env.a, "answer q1").await;
    wait_reply(&env, &tid, "answer h1").await;
    wait_for("the hold to clear and the bot to be available", || async {
        let st = status_of(&env, &env.a).await;
        st["held"].as_array().is_none_or(|h| h.is_empty()) && st["status"] == "available"
    })
    .await;
    let st = status_of(&env, &env.a).await;
    assert!(st["last_finished_at"].is_string() && st["since"] == st["last_finished_at"], "{st}");

    // After the window's first send was delivered, a new id goes as usual
    let (s, v) = send(&env, json!({"prompt": "thanks REPLYTAG:h2", "sender_id": "user", "thread_id": tid, "client_message_id": "cm-01HF.2"})).await;
    assert_eq!((s, v["status"].as_str()), (StatusCode::OK, Some("dispatched")), "{v}");
    wait_reply(&env, &tid, "answer h2").await;
}

#[tokio::test]
async fn quiet_and_retry_routes_answer_for_the_person() {
    let env = setup().await;
    let (s, _) = call(&env.app, "POST", "/api/turns/no-such-turn/quiet", Some(json!({"action": "snooze"}))).await;
    assert_eq!(s, StatusCode::NOT_FOUND);
    let (s, v) = call(&env.app, "POST", &format!("/api/kwants/{}/status/retry", env.a), Some(json!({}))).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!((v["released"].as_u64(), v["status"]["status"].as_str()), (Some(0), Some("available")), "{v}");
    let (s, _) = call(&env.app, "POST", "/api/kwants/no-such-bot/status/retry", Some(json!({}))).await;
    assert_eq!(s, StatusCode::NOT_FOUND);
}
