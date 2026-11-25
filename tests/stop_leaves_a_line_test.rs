//! stop-leaves-a-line (0.9.51): a Stop the person presses leaves one line in the chat, "Stopped.
//! You stopped this reply.", with the steps the turn took. Before, the `if cancelled` branch of the
//! turn sent a live-only "⏹️ Cancelled by user" and saved nothing: in Mira's UX pass on main
//! d35c8a52 the question was left as the chat's last line, its steps were gone after the Stop,
//! and a queued follow-up's "Done." read as the answer to the stopped question.
//!
//! Each Stop here must save exactly one kwant message with that text and meta kind
//! `stopped_by_person`, carrying the turn's steps (so they open after a reload), and put it on
//! the live stream as `MessageAdded`. It is not a Needs-you item and raises no notification: the
//! person stopped it on purpose. A "stop all" leaves one line per bot that was running.

mod common;

use common::*;
use kwnt::db::models::Message;
use kwnt::RuntimeEvent;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::time::Duration;
use tokio::sync::broadcast::{error::RecvError, Receiver};

const STOPPED: &str = "Stopped. You stopped this reply.";

/// A turn that opens one page (done), starts on a second (still running) and then hangs, so the
/// Stop lands mid-step.
fn mid_step(tag: &str) -> String {
    let (a, b) = (format!("toolu_{tag}a"), format!("toolu_{tag}b"));
    let lines = [
        json!({"type": "assistant", "message": {"id": format!("msg_{tag}a"), "content": [
            {"type": "tool_use", "id": a, "name": "Bash", "input": {"command": "curl -s https://flexidesk.example", "description": "Opening flexidesk.example"}}]}}),
        json!({"type": "user", "message": {"content": [
            {"type": "tool_result", "tool_use_id": a, "is_error": false, "content": "FlexiDesk Pro ships to the EU"}]}}),
        json!({"type": "assistant", "message": {"id": format!("msg_{tag}b"), "content": [
            {"type": "tool_use", "id": b, "name": "Bash", "input": {"command": "curl -s https://lumo-desks.example", "description": "Opening lumo-desks.example"}}]}}),
    ];
    format!("{} HANGONCE:{tag}:30000", stream_lines_marker(&lines))
}

/// Waits until each tag's turn has started its second step; returns each tag's turn id.
async fn mid_steps(rx: &mut Receiver<RuntimeEvent>, tags: &[&str]) -> HashMap<String, String> {
    let mut seen: HashMap<String, String> = HashMap::new();
    let wait = async {
        while seen.len() < tags.len() {
            let e = match rx.recv().await {
                Ok(e) => e,
                Err(RecvError::Lagged(_)) => continue,
                Err(RecvError::Closed) => return,
            };
            let RuntimeEvent::Scoped { tag, event } = &e else { continue };
            if let RuntimeEvent::ToolCallPending { id, .. } = event.unscoped() {
                if let Some(t) = tags.iter().find(|t| *id == format!("toolu_{t}b")) {
                    seen.insert(t.to_string(), tag.turn_id.clone());
                }
            }
        }
    };
    tokio::time::timeout(Duration::from_secs(30), wait).await.unwrap_or_else(|_| panic!("the turns never reached their second step: {seen:?}"));
    seen
}

/// What reached the live stream within `within`: the bots' messages and how many notifications.
async fn live(rx: &mut Receiver<RuntimeEvent>, within: Duration) -> (Vec<Value>, usize) {
    let (mut added, mut notes) = (Vec::new(), 0);
    let _ = tokio::time::timeout(within, async {
        loop {
            match rx.recv().await {
                Ok(e) => match e.unscoped() {
                    RuntimeEvent::MessageAdded { message } if message["sender_role"] == "kwant" => added.push(message.clone()),
                    RuntimeEvent::Notify { .. } => notes += 1,
                    _ => {}
                },
                Err(RecvError::Lagged(_)) => continue,
                Err(RecvError::Closed) => return,
            }
        }
    })
    .await;
    (added, notes)
}

fn meta(m: &Message) -> Value {
    m.meta.as_deref().and_then(|s| serde_json::from_str(s).ok()).unwrap_or_default()
}

async fn stop(env: &Env, body: Option<Value>) {
    let (s, v) = call(&env.app, "POST", "/api/chat/cancel", body).await;
    assert!(s.is_success(), "{s} {v}");
    assert!(v["cancelled"].as_u64().unwrap_or(0) >= 1, "the Stop stopped something: {v}");
}

/// The one saved Stopped line in `ctx`: kind `stopped_by_person`, the turn's two steps in its
/// activity, and nothing asked of the person for it.
async fn stopped_line(env: &Env, ctx: &str, kwant: &str) -> Message {
    let mut lines = Vec::new();
    for _ in 0..50 {
        lines = messages(env, ctx).await.into_iter().filter(|m| m.text == STOPPED).collect();
        if !lines.is_empty() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    let all: Vec<String> = messages(env, ctx).await.iter().map(|m| format!("{}: {}", m.sender_role, m.text)).collect();
    assert_eq!(lines.len(), 1, "exactly one saved \"{STOPPED}\" line in {ctx} after the Stop; the chat holds {all:#?}");
    let m = lines.remove(0);
    assert_eq!(m.sender_role, "kwant", "{m:?}");
    let name = env.state.db.get_kwant(kwant).await.unwrap().unwrap().name;
    assert_eq!(m.sender_id, name, "said by the bot that was stopped: {m:?}");
    let meta = meta(&m);
    assert_eq!(meta["kind"], "stopped_by_person", "{meta}");
    let steps = meta["activity"]["steps"].as_array().cloned().unwrap_or_default();
    let tools: Vec<&Value> = steps.iter().filter(|s| s["kind"] == "tool").collect();
    assert_eq!(tools.len(), 2, "both steps it took stay openable after a reload: {meta}");
    // The person stopped it on purpose: nothing waits on them for it
    let waiting = env.state.db.list_waiting(None).await.unwrap();
    assert!(!waiting.iter().any(|w| w.kwant_id == kwant), "no Needs-you item for a Stop the person pressed: {waiting:#?}");
    m
}

/// The live copy of the saved line: exactly one, in the right chat, and no notification.
fn shown_live(added: &[Value], notes: usize, saved: &Message, place: &str, ctx: &str) {
    let mine: Vec<&Value> = added.iter().filter(|m| m["text"] == STOPPED).collect();
    assert_eq!(mine.len(), 1, "exactly one MessageAdded for the Stopped line in {ctx}; the stream carried {added:#?}");
    assert_eq!(mine[0]["id"], saved.id.as_str(), "{}", mine[0]);
    assert_eq!(mine[0][place], ctx, "the event names the chat it belongs to: {}", mine[0]);
    assert_eq!(notes, 0, "a Stop the person pressed raises no notification");
}

#[tokio::test]
async fn a_stop_mid_step_in_a_direct_chat_leaves_one_stopped_line_with_its_steps() {
    let env = setup().await;
    let a = env.a.clone();
    let mut rx = env.state.event_sender.subscribe();
    let question = format!("Thanks. Which of these deliver to Slovenia? {}", mid_step("sldirect"));
    send_direct(&env, &a, &question, json!({})).await;
    mid_steps(&mut rx, &["sldirect"]).await;

    stop(&env, Some(json!({"kwant_id": a}))).await;
    let (added, notes) = live(&mut rx, Duration::from_secs(3)).await;
    let saved = stopped_line(&env, &a, &a).await;
    shown_live(&added, notes, &saved, "direct_kwant_id", &a);
    // Under the question, as the chat's last line
    let chat = messages(&env, &a).await;
    let last_two: Vec<(&str, &str)> = chat.iter().rev().take(2).rev().map(|m| (m.sender_role.as_str(), m.text.as_str())).collect();
    assert_eq!(last_two, vec![("user", question.as_str()), ("kwant", STOPPED)], "{chat:#?}");
}

#[tokio::test]
async fn a_stop_in_a_thread_leaves_the_line_in_that_thread() {
    let env = setup().await;
    let (s, t) = call(&env.app, "POST", &format!("/api/kwants/{}/threads", env.a), Some(json!({"title": "desks"}))).await;
    assert!(s.is_success(), "{s} {t}");
    let thread = t["id"].as_str().unwrap().to_string();
    let mut rx = env.state.event_sender.subscribe();
    let (s, v) = call(&env.app, "POST", "/api/chat/send", Some(json!({"prompt": format!("Compare desks {}", mid_step("slthread")), "sender_id": "user", "thread_id": thread}))).await;
    assert!(s.is_success(), "{s} {v}");
    mid_steps(&mut rx, &["slthread"]).await;

    stop(&env, Some(json!({"thread_id": thread}))).await;
    let (added, notes) = live(&mut rx, Duration::from_secs(3)).await;
    let saved = stopped_line(&env, &thread, &env.a).await;
    shown_live(&added, notes, &saved, "thread_id", &thread);
    assert!(!messages(&env, &env.a).await.iter().any(|m| m.text == STOPPED), "only in the thread, not the direct chat");
}

/// "Stop all" (no chat named, as a team or discussion Stop does through `stop_turns`): one line
/// per bot that was running, each in its own chat.
#[tokio::test]
async fn stop_all_leaves_one_line_per_bot_that_was_running() {
    let env = setup().await;
    let (a, b) = (env.a.clone(), env.b.clone());
    let mut rx = env.state.event_sender.subscribe();
    send_direct(&env, &a, &format!("Desks? {}", mid_step("slalla")), json!({})).await;
    send_direct(&env, &b, &format!("Chairs? {}", mid_step("slallb")), json!({})).await;
    mid_steps(&mut rx, &["slalla", "slallb"]).await;

    stop(&env, None).await;
    let (added, notes) = live(&mut rx, Duration::from_secs(3)).await;
    for k in [&a, &b] {
        let saved = stopped_line(&env, k, k).await;
        let mine: Vec<&Value> = added.iter().filter(|m| m["id"] == saved.id.as_str()).collect();
        assert_eq!(mine.len(), 1, "the line in {k} is on the stream once: {added:#?}");
    }
    assert_eq!(added.iter().filter(|m| m["text"] == STOPPED).count(), 2, "{added:#?}");
    assert_eq!(notes, 0);
}

/// With a follow-up queued behind the stopped reply, the Stopped line comes before the
/// follow-up's reply: "Done." no longer reads as the answer to the stopped question.
#[tokio::test]
async fn a_queued_follow_up_is_answered_after_the_stopped_line() {
    let env = setup().await;
    let a = env.a.clone();
    let mut rx = env.state.event_sender.subscribe();
    let question = format!("Which deliver to Slovenia? {}", mid_step("slqueue"));
    send_direct(&env, &a, &question, json!({})).await;
    let turn = mid_steps(&mut rx, &["slqueue"]).await.remove("slqueue").unwrap();
    let follow_up = "Also, which of them come with assembly? REPLYTAG:slqueuef";
    send_direct(&env, &a, follow_up, json!({})).await;

    // The Stop on the running reply (its turn), as the chat's Stop button sends it
    stop(&env, Some(json!({"turn_id": turn}))).await;
    let reply = wait_reply(&env, &a, "answer slqueuef").await;
    let saved = stopped_line(&env, &a, &a).await;
    let chat = messages(&env, &a).await;
    let at = |id: &str| chat.iter().position(|m| m.id == id).unwrap();
    let q = chat.iter().position(|m| m.text == question).expect("the question");
    assert!(q < at(&saved.id), "the Stopped line is under the question: {chat:#?}");
    assert!(at(&saved.id) < at(&reply.id), "the follow-up's reply comes after the Stopped line: {chat:#?}");
    assert_eq!(chat.iter().filter(|m| m.sender_role == "kwant").count(), 2, "the Stopped line and the follow-up's reply: {chat:#?}");
}

/// After #1181 (stuck-getting-started): the silent-turn sweep cancels the same job token a Stop
/// does. Its turn must leave only the sweep's `turn_lost` line, not a "You stopped this reply."
/// the person never pressed.
#[tokio::test]
async fn a_silent_turn_the_sweep_stops_leaves_only_its_turn_lost_line() {
    let env = setup().await;
    let a = env.a.clone();
    let mut rx = env.state.event_sender.subscribe();
    // One finished step, then the engine goes quiet (no tool running, nothing waits on the person)
    let tag = "slsweep";
    let step = format!("toolu_{tag}a");
    let lines = [
        json!({"type": "assistant", "message": {"id": format!("msg_{tag}a"), "content": [
            {"type": "tool_use", "id": step, "name": "Bash", "input": {"command": "curl -s https://flexidesk.example", "description": "Opening flexidesk.example"}}]}}),
        json!({"type": "user", "message": {"content": [
            {"type": "tool_result", "tool_use_id": step, "is_error": false, "content": "FlexiDesk Pro ships to the EU"}]}}),
    ];
    let question = format!("Which deliver to Slovenia? {} HANGONCE:{tag}:30000", stream_lines_marker(&lines));
    send_direct(&env, &a, &question, json!({})).await;
    let turn = tokio::time::timeout(Duration::from_secs(30), async {
        loop {
            match rx.recv().await {
                Ok(RuntimeEvent::Scoped { tag: t, event }) => {
                    if matches!(event.unscoped(), RuntimeEvent::ToolCallCompleted { id, .. } if *id == step) {
                        return t.turn_id.clone();
                    }
                }
                Ok(_) | Err(RecvError::Lagged(_)) => continue,
                Err(RecvError::Closed) => panic!("stream closed"),
            }
        }
    })
    .await
    .expect("the turn finished its first step");

    // The sweep, looking two hours on: the turn has been silent far past the limit
    let silence = Duration::from_secs(360);
    let mut stopped = None;
    for _ in 0..50 {
        let ended = kwnt::server::lost_turns::sweep(&env.state, &mut std::collections::HashSet::new(), chrono::Utc::now() + chrono::Duration::hours(2), silence).await;
        if ended.iter().any(|(t, _)| *t == turn) {
            stopped = Some(ended);
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    assert_eq!(stopped, Some(vec![(turn.clone(), kwnt::server::lost_turns::Lost::Silent)]), "the sweep stopped the silent turn");

    // Its turn winds down on the cancel; give it the time a Stop's line takes to be saved
    for _ in 0..50 {
        if !env.state.turns.has_turn(&turn) {
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    tokio::time::sleep(Duration::from_secs(1)).await;
    let chat = messages(&env, &a).await;
    let all: Vec<String> = chat.iter().map(|m| format!("{}: {}", m.sender_role, m.text)).collect();
    assert!(!chat.iter().any(|m| m.text == STOPPED), "nobody pressed Stop: no \"{STOPPED}\" line; the chat holds {all:#?}");
    let lost: Vec<&Message> = chat.iter().filter(|m| meta(m)["kind"] == "turn_lost").collect();
    assert_eq!(lost.len(), 1, "exactly the sweep's line; the chat holds {all:#?}");
    assert!(lost[0].text.ends_with("got stuck for 6 minutes, so the work was stopped. Ask again if you still need it."), "{:?}", lost[0].text);
    assert_eq!(chat.last().map(|m| m.id.as_str()), Some(lost[0].id.as_str()), "the turn_lost line stays the chat's last line: {all:#?}");

    // A person's Stop on the next reply still leaves exactly one Stopped line
    send_direct(&env, &a, &format!("Chairs then? {}", mid_step("slsweepb")), json!({})).await;
    mid_steps(&mut rx, &["slsweepb"]).await;
    stop(&env, Some(json!({"kwant_id": a}))).await;
    stopped_line(&env, &a, &a).await;
}
