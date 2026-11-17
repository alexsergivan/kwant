//! 27d: messages the person sends while their bot waits on them are held (H3), and when the
//! person answers, all of them are released in the same tick. Each then has work to do before it
//! takes its place in the bot's line (the delivery plan's model call, up to ~3 s), so on 0.9.21
//! they were answered in whatever order that work finished: 7 of 8 runs out of order in Rigel's
//! 27c QA. Here the work after the release takes longest for the oldest message, and the four
//! must still be queued and answered in the order the person sent them: in a direct chat and in
//! a thread. Nothing is lost or merged.

mod common;

use axum::http::StatusCode;
use common::*;
use kwnt::drivers::events::RuntimeEvent;
use serde_json::{json, Value};
use std::time::Duration;

fn ask_marker(question: &str, choices: &[&str]) -> String {
    call_tool_marker(
        "ask_user",
        json!({"question": question, "choices": choices, "why_you": "irreversible", "checked": "Read the release notes."}),
    )
}

async fn open_question(env: &Env, ctx: &str) -> String {
    for _ in 0..300 {
        let (s, list) = call(&env.app, "GET", &format!("/api/questions?status=open&context_id={ctx}"), None).await;
        assert_eq!(s, StatusCode::OK);
        if let Some(q) = list.as_array().and_then(|l| l.first()) {
            return q["id"].as_str().unwrap().to_string();
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    panic!("no open question in {ctx}");
}

async fn new_thread(env: &Env, title: &str) -> String {
    let (s, t) = call(&env.app, "POST", &format!("/api/kwants/{}/threads", env.a), Some(json!({"title": title}))).await;
    assert_eq!(s, StatusCode::CREATED, "{t}");
    t["id"].as_str().unwrap().to_string()
}

/// Bot A's direct chat, or one of its threads.
enum Chat {
    Direct,
    Thread(String),
}

impl Chat {
    fn context(&self, env: &Env) -> String {
        match self {
            Chat::Direct => env.a.clone(),
            Chat::Thread(t) => t.clone(),
        }
    }

    async fn send(&self, env: &Env, prompt: &str) -> Value {
        let mut body = json!({"prompt": prompt, "sender_id": "user"});
        match self {
            Chat::Direct => body["kwant_id"] = json!(env.a),
            Chat::Thread(t) => body["thread_id"] = json!(t),
        }
        let (s, v) = call(&env.app, "POST", "/api/chat/send", Some(body)).await;
        assert_eq!(s, StatusCode::OK, "{v}");
        v
    }
}

/// Bot A is busy answering in a chat (a slow reply; a thread when `in_thread`, else its direct
/// chat) and waits on the person's answer in the other one. The person sends four messages to `chat`; all four are held. Their release work is
/// slowest for the first (`lags`, oldest first). Once the person answers, the four take their
/// places in A's line in the order sent (`turn_queued` positions 1..4) and are answered in it.
async fn four_held_messages_keep_their_order(in_thread: bool, tag: &str) {
    let env = setup().await;
    let thread = Chat::Thread(new_thread(&env, "side").await);
    let (chat, ask_in) = if in_thread { (thread, Chat::Direct) } else { (Chat::Direct, thread) };
    let ctx = chat.context(&env);
    let ask_ctx = ask_in.context(&env);

    // A slow reply holds the chat, so every released message has to queue for it
    chat.send(&env, &format!("Draft the summary. SLEEPONCE:{tag}s:20000 REPLYTAG:{tag}s")).await;
    wait_for("the slow reply to start", || async {
        env.state.turns.active_turns().iter().any(|t| t.context_id == ctx && t.state == "running")
    })
    .await;

    // The bot asks the person elsewhere, so sends to `chat` are held until the answer
    ask_in.send(&env, &format!("Ship it? {} REPLYTAG:{tag}q", ask_marker("Which branch should I ship?", &["main", "batch2"]))).await;
    let qid = open_question(&env, &ask_ctx).await;

    let mut events = env.state.event_sender.subscribe();
    let lags = [1500u64, 1000, 600, 200];
    let mut sent = Vec::new();
    for (i, lag) in lags.iter().enumerate() {
        let v = chat.send(&env, &format!("Question {}? REPLYTAG:{tag}h{}", i + 1, i + 1)).await;
        assert_eq!(v["status"], "held", "message {} is held: {v}", i + 1);
        let id = v["user_message_id"].as_str().unwrap().to_string();
        env.state.turns.board(|b| b.release_lag.insert(id.clone(), Duration::from_millis(*lag)));
        sent.push(id);
    }

    let (s, v) = call(&env.app, "POST", &format!("/api/questions/{qid}/answer"), Some(json!({"choice": 1}))).await;
    assert_eq!(s, StatusCode::OK, "{v}");

    // Every released message queues (the slow reply still holds the chat), in send order
    let mut queued: Vec<(String, usize)> = Vec::new();
    let mut started: Vec<String> = Vec::new();
    let deadline = tokio::time::Instant::now() + Duration::from_secs(60);
    while started.len() < 4 || queued.len() < 4 {
        let e = tokio::time::timeout_at(deadline, events.recv())
            .await
            .unwrap_or_else(|_| panic!("timed out: queued {queued:?}, started {started:?}"))
            .expect("event bus");
        match e {
            RuntimeEvent::TurnQueued { turn_id, context_id, position, .. } if context_id.as_deref() == Some(ctx.as_str()) => {
                assert!(started.is_empty(), "every message queued before the slow reply ended: {turn_id} at {position} after {started:?}");
                queued.push((turn_id, position));
            }
            RuntimeEvent::TurnStarted { turn_id: Some(t), context_id, .. } if context_id.as_deref() == Some(ctx.as_str()) => {
                started.push(t);
            }
            _ => {}
        }
    }
    let positions: Vec<usize> = queued.iter().map(|(_, p)| *p).collect();
    assert_eq!(positions, vec![1, 2, 3, 4], "{queued:?}");
    // The line is first come, first served: the turn queued k-th starts k-th (after the slow reply)
    let queued_ids: Vec<&str> = queued.iter().map(|(t, _)| t.as_str()).collect();
    let started_after: Vec<&str> = started.iter().map(String::as_str).filter(|t| queued_ids.contains(t)).collect();
    assert_eq!(started_after, queued_ids, "queued turns start in line order");

    // ...so the person reads the answers in the order they asked, and none is lost or merged
    wait_reply(&env, &ctx, &format!("answer {tag}h4")).await;
    wait_for("all four answers", || async {
        messages(&env, &ctx).await.iter().filter(|m| m.sender_role == "kwant" && m.text.contains(&format!("answer {tag}h"))).count() >= 4
    })
    .await;
    let msgs = messages(&env, &ctx).await;
    let answers: Vec<String> = msgs
        .iter()
        .filter(|m| m.sender_role == "kwant")
        .filter_map(|m| (1..=4).find(|i| m.text.contains(&format!("answer {tag}h{i}"))).map(|i| format!("h{i}")))
        .collect();
    assert_eq!(answers, vec!["h1", "h2", "h3", "h4"], "answers in send order");
    for id in &sent {
        assert_eq!(msgs.iter().filter(|m| &m.id == id).count(), 1, "message {id} saved once");
    }
    wait_for("the bot to be free and nothing held", || async {
        env.state.turns.active_turns().is_empty() && env.state.turns.board(|b| b.held.is_empty())
    })
    .await;
}

#[tokio::test]
async fn held_messages_in_a_direct_chat_are_queued_and_answered_in_send_order() {
    // The bot asks in a thread; the four go to its direct chat
    four_held_messages_keep_their_order(false, "ordd").await;
}

#[tokio::test]
async fn held_messages_in_a_thread_are_queued_and_answered_in_send_order() {
    // The bot asks in its direct chat; the four go to a thread
    four_held_messages_keep_their_order(true, "ordt").await;
}
