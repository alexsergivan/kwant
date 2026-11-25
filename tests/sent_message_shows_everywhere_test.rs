//! sent-message-shows-everywhere (0.9.50): the person's message reaches the chat's live stream the
//! moment it is saved, so their other open screens show it at once, not when the bot's turn ends.
//! Before, `send_chat` saved it and linked its files but sent no `MessageAdded`: in Mira's UX pass
//! on main d35c8a52 the phone showed "Assistant · Steps" with no question above it, and her
//! recording of `/api/events` carried the message text 0 times.
//!
//! Each send here must put exactly one `MessageAdded` for it on the stream, with `sender_role`
//! "user", its chat (direct, channel or thread), its text, its files' cards, and the id the send
//! answered with: in a direct chat, a channel and a thread, with a file, when it is held (H3) and
//! when it answers the bot's open question instead of starting a turn. It never raises a
//! notification.

mod common;

use axum::{
    body::{to_bytes, Body},
    http::{Request, StatusCode},
};
use common::*;
use kwnt::RuntimeEvent;
use serde_json::{json, Value};
use std::time::Duration;
use tokio::sync::broadcast::{
    error::{RecvError, TryRecvError},
    Receiver,
};
use tower::ServiceExt;

const PNG: &[u8] = b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR-not-a-real-image-but-png-magic";

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

/// Waits (up to 10 s) until `rx` has carried the `question_asked` notification for the question
/// asked in `thread`. The question row is saved before that notification goes out, so a test that
/// only polls for the open question can subscribe in between and count the question's own
/// notification as the next message's.
async fn question_notified(rx: &mut Receiver<RuntimeEvent>, thread: &str) {
    let seen = async {
        loop {
            match rx.recv().await {
                Ok(e) => {
                    if let RuntimeEvent::Notify { kind, thread_id, .. } = e.unscoped() {
                        if kind.as_str() == "question_asked" && thread_id.as_deref() == Some(thread) {
                            return;
                        }
                    }
                }
                Err(RecvError::Lagged(_)) => continue,
                Err(RecvError::Closed) => panic!("the event stream closed before the question's notification"),
            }
        }
    };
    tokio::time::timeout(Duration::from_secs(10), seen)
        .await
        .unwrap_or_else(|_| panic!("no question_asked notification for the question in {thread} within 10 s"));
}

async fn new_thread(env: &Env, title: &str) -> String {
    let (s, t) = call(&env.app, "POST", &format!("/api/kwants/{}/threads", env.a), Some(json!({"title": title}))).await;
    assert_eq!(s, StatusCode::CREATED, "{t}");
    t["id"].as_str().unwrap().to_string()
}

async fn upload(env: &Env, name: &str, bytes: Vec<u8>) -> String {
    let req = Request::builder()
        .method("POST")
        .uri(format!("/api/attachments?name={name}"))
        .header("content-type", "application/octet-stream")
        .body(Body::from(bytes))
        .unwrap();
    let resp = env.app.clone().oneshot(req).await.unwrap();
    let s = resp.status();
    let b = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    let v: Value = serde_json::from_slice(&b).unwrap_or(Value::Null);
    assert!(s.is_success(), "upload: {s} {v}");
    v["id"].as_str().expect("attachment id").to_string()
}

async fn send(env: &Env, body: Value) -> Value {
    let (s, v) = call(&env.app, "POST", "/api/chat/send", Some(body)).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    v
}

/// What the stream carried since `rx` subscribed: the person's own messages (`sender_role`
/// "user") and how many notifications it raised.
fn drain(rx: &mut Receiver<RuntimeEvent>) -> (Vec<Value>, usize) {
    let (mut sent, mut notes) = (Vec::new(), 0);
    loop {
        match rx.try_recv() {
            Ok(e) => match e.unscoped() {
                RuntimeEvent::MessageAdded { message } if message["sender_role"] == "user" => sent.push(message.clone()),
                RuntimeEvent::Notify { .. } => notes += 1,
                _ => {}
            },
            Err(TryRecvError::Lagged(_)) => continue,
            Err(TryRecvError::Empty | TryRecvError::Closed) => return (sent, notes),
        }
    }
}

/// Exactly one live copy of the message the send answered with, in `place` (`channel_id`,
/// `direct_kwant_id` or `thread_id` = `ctx`), with `text`; no notification for it. Every caller
/// but the typed answer keeps the bot's turn slow or held, so any notification would be the
/// message's own.
fn shown_once(rx: &mut Receiver<RuntimeEvent>, reply: &Value, place: &str, ctx: &str, text: &str) -> Value {
    shown_once_with(rx, reply, place, ctx, text, true)
}

fn shown_once_with(rx: &mut Receiver<RuntimeEvent>, reply: &Value, place: &str, ctx: &str, text: &str, quiet: bool) -> Value {
    let id = reply["user_message_id"].as_str().unwrap_or_else(|| panic!("no user_message_id: {reply}"));
    let (sent, notes) = drain(rx);
    let mine: Vec<&Value> = sent.iter().filter(|m| m["id"] == id).collect();
    assert_eq!(mine.len(), 1, "exactly one MessageAdded for the person's message {id}; the stream carried {sent:?}");
    let m = mine[0].clone();
    assert_eq!(sent.len(), 1, "no other user message on the stream: {sent:?}");
    assert_eq!(m["sender_role"], "user", "{m}");
    assert_eq!(m[place], ctx, "the event names the chat it belongs to: {m}");
    assert_eq!(m["text"], text, "{m}");
    if quiet {
        assert_eq!(notes, 0, "the person's own message raises no notification");
    }
    m
}

/// A slow reply, so the bot's turn is still running (and raises nothing) when the events are read.
fn slow(tag: &str, text: &str) -> String {
    format!("{text} SLEEPONCE:{tag}:4000 REPLYTAG:{tag}")
}

#[tokio::test]
async fn a_message_sent_in_a_direct_chat_shows_on_every_open_screen() {
    let env = setup().await;
    let mut rx = env.state.event_sender.subscribe();
    let text = slow("smdirect", "What changed in the release?");
    let v = send(&env, json!({"prompt": text, "sender_id": "user", "kwant_id": env.a})).await;
    assert_eq!(v["status"], "dispatched", "{v}");
    let m = shown_once(&mut rx, &v, "direct_kwant_id", &env.a, &text);
    assert!(m["thread_id"].is_null() && m["channel_id"].is_null(), "{m}");
}

#[tokio::test]
async fn a_message_sent_in_a_channel_shows_on_every_open_screen() {
    let env = setup().await;
    let (s, c) = call(&env.app, "POST", "/api/channels", Some(json!({"name": "desk", "member_kwants": [env.a]}))).await;
    assert!(s.is_success(), "{s} {c}");
    let cid = c["id"].as_str().unwrap().to_string();
    let mut rx = env.state.event_sender.subscribe();
    let text = slow("smchan", "Who picks up the Friday report?");
    let v = send(&env, json!({"prompt": text, "sender_id": "user", "channel_id": cid})).await;
    shown_once(&mut rx, &v, "channel_id", &cid, &text);
}

#[tokio::test]
async fn a_message_sent_in_a_thread_shows_on_every_open_screen() {
    let env = setup().await;
    let thread = new_thread(&env, "side").await;
    let mut rx = env.state.event_sender.subscribe();
    let text = slow("smthread", "And the follow-up from my phone?");
    let v = send(&env, json!({"prompt": text, "sender_id": "user", "thread_id": thread})).await;
    assert_eq!(v["status"], "dispatched", "{v}");
    let m = shown_once(&mut rx, &v, "thread_id", &thread, &text);
    assert!(m["direct_kwant_id"].is_null() && m["channel_id"].is_null(), "{m}");
}

/// The live copy carries the file's card, as a reload would show it, in a direct chat and in a thread.
#[tokio::test]
async fn a_message_with_a_file_shows_its_card_on_every_open_screen() {
    let env = setup().await;
    let thread = new_thread(&env, "files").await;
    for (tag, place, ctx) in [("smfiled", "direct_kwant_id", env.a.clone()), ("smfilet", "thread_id", thread.clone())] {
        let file = upload(&env, &format!("{tag}.png"), PNG.to_vec()).await;
        let mut rx = env.state.event_sender.subscribe();
        let text = slow(tag, "Here is the screenshot");
        let mut body = json!({"prompt": text, "sender_id": "user", "attachment_ids": [file]});
        body[if place == "thread_id" { "thread_id" } else { "kwant_id" }] = json!(ctx);
        let v = send(&env, body).await;
        let m = shown_once(&mut rx, &v, place, &ctx, &text);
        let meta: Value = serde_json::from_str(m["meta"].as_str().unwrap_or_else(|| panic!("no meta: {m}"))).unwrap();
        let cards = meta["attachments"].as_array().unwrap_or_else(|| panic!("no attachment cards: {meta}"));
        assert!(cards.iter().any(|c| c["id"] == file.as_str()), "the file's card is on the live copy: {meta}");
    }
}

/// H3: while the bot waits on the person elsewhere, a message is held (saved, no turn yet). It
/// still shows on every screen at once, in a direct chat and in a thread.
#[tokio::test]
async fn a_held_message_shows_on_every_open_screen() {
    let env = setup().await;
    let side = new_thread(&env, "ask").await;
    // Subscribed before the asking send, so the question's own notification is seen (and done
    // with) before the held messages' streams subscribe
    let mut asked = env.state.event_sender.subscribe();
    send(&env, json!({"prompt": format!("Ship it? {} REPLYTAG:smheldq", ask_marker("Which branch should I ship (smheld)?", &["main", "batch2"])), "sender_id": "user", "thread_id": side})).await;
    open_question(&env, &side).await;
    question_notified(&mut asked, &side).await;
    drop(asked);
    let other = new_thread(&env, "other").await;
    for (place, ctx) in [("direct_kwant_id", env.a.clone()), ("thread_id", other.clone())] {
        let mut rx = env.state.event_sender.subscribe();
        let text = format!("One more thing for later ({place})");
        let mut body = json!({"prompt": text, "sender_id": "user"});
        body[if place == "thread_id" { "thread_id" } else { "kwant_id" }] = json!(ctx);
        let v = send(&env, body).await;
        assert_eq!(v["status"], "held", "{v}");
        shown_once(&mut rx, &v, place, &ctx, &text);
    }
}

/// Typed instead of tapped: a message that answers the bot's open question starts no turn and
/// returns early, but it is still the person's message in the chat and shows everywhere.
#[tokio::test]
async fn a_typed_answer_shows_on_every_open_screen() {
    let env = setup().await;
    send(&env, json!({"prompt": format!("{} REPLYTAG:smtyped", ask_marker("Which offer should run this week (smtyped)?", &["15% off", "Free shipping"])), "sender_id": "user", "kwant_id": env.a})).await;
    open_question(&env, &env.a).await;
    let mut rx = env.state.event_sender.subscribe();
    let text = "free shipping over $35, please";
    let v = send(&env, json!({"prompt": text, "sender_id": "user", "kwant_id": env.a})).await;
    assert_eq!(v["status"], "answered", "{v}");
    // The waiting turn goes on at once and may end (and notify) before the events are read
    shown_once_with(&mut rx, &v, "direct_kwant_id", &env.a, text, false);
}
