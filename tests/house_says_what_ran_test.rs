//! 27e house-says-what-ran: on kwant 0.9.21 with a 7B model, House replied "Unlocking the front
//! door now." after a turn that ran no tool (no card, the door stayed locked), and answered "are
//! any switches on?" from a filtered GetLiveContext that Home Assistant refused. End to end, with
//! a Custom API bot against a scripted OpenAI-compatible model and the echo MCP server with Home
//! Assistant's tools (it answers GetLiveContext like Home Assistant 2026.11.1):
//!
//! 1. A claimed unlock with no tool gets one continuation; it calls HassTurnOff on Front door, the
//!    lock card asks, the person denies, and the saved reply claims nothing.
//! 2. The claim twice with no tool: the continuation's reply ends with kwant's own line.
//! 3. A GetLiveContext filtered on `switch` fails (the model is told to read the whole home); the
//!    continuation reads it with no arguments and names the kitchen and porch lights.
//! 4. "Turn off the living room lamp" with HassTurnOff that works: no continuation and no line.

mod common;

use axum::{extract::State, http::StatusCode, routing::get, routing::post, Json, Router};
use common::*;
use kwnt::{
    broker::ApprovalDecision,
    db::{models::McpServer, spaces::SpaceId},
    drivers::RuntimeEvent,
};
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

const KEY: &str = "sk-custom-house-0123456789";
const OFF: &str = "mcp__house__intent__HassTurnOff";
const READ: &str = "mcp__house__homeassistant__GetLiveContext";
const UNLOCKING: &str = "Unlocking the front door now.";
const CONTINUATION: &str = "[kwnt: one automatic continuation";
const CHANGE_LINE: &str = "_kwant: House didn't change anything in your home in this reply. No Home Assistant step made that change (see Details)._";

/// Every chat request the fake model got.
type Seen = Arc<Mutex<Vec<Value>>>;

fn text(content: &str) -> Value {
    json!({"choices": [{"message": {"role": "assistant", "content": content}}], "usage": {"total_tokens": 9}})
}

fn tool_call(name: &str, args: Value) -> Value {
    json!({
        "choices": [{"message": {"role": "assistant", "content": "", "tool_calls": [{
            "id": format!("call_{}", name.len()), "type": "function",
            "function": {"name": name, "arguments": args.to_string()},
        }]}}],
        "usage": {"total_tokens": 9},
    })
}

fn content(m: &Value) -> String {
    match &m["content"] {
        Value::String(s) => s.clone(),
        Value::Array(parts) => parts.iter().filter_map(|p| p["text"].as_str()).collect::<Vec<_>>().join("\n"),
        _ => String::new(),
    }
}

/// The lights a whole read of the home says are on: "Kitchen light, Porch light".
fn lights_on(result: &str) -> String {
    // The result is Home Assistant's JSON; its `result` holds the list
    let list = result.find('{').and_then(|i| serde_json::from_str::<Value>(result[i..].lines().next().unwrap_or("")).ok());
    let list = list.as_ref().and_then(|v| v["result"].as_str()).unwrap_or(result).to_string();
    let mut on = Vec::new();
    let mut name = "";
    for line in list.lines() {
        if let Some(n) = line.trim().strip_prefix("- names: ") {
            name = n;
        }
        if line.trim() == "state: 'on'" {
            on.push(name.to_string());
        }
    }
    on.join(" and ")
}

/// What the scripted House answers. The person's message names its scene ("SCENE:n"); a request
/// whose last message is a tool result answers that result; one whose last user message is
/// kwant's continuation follows the scene's continuation; any other is the scene's first turn.
fn script(body: &Value) -> Value {
    let messages = body["messages"].as_array().cloned().unwrap_or_default();
    let all: String = messages.iter().map(content).collect::<Vec<_>>().join("\n");
    let scene = ["SCENE:1", "SCENE:2", "SCENE:3", "SCENE:4"].into_iter().find(|s| all.contains(s)).unwrap_or("");
    let last = messages.last().cloned().unwrap_or_default();
    let continued = messages.iter().rev().find(|m| m["role"] == "user").is_some_and(|m| content(m).contains(CONTINUATION));
    if last["role"] == "tool" {
        let result = content(&last);
        return match scene {
            "SCENE:1" => text("You declined the card, so the front door stays locked."),
            "SCENE:3" if result.contains("No exposed entities") => text("No switches are on in your home."),
            "SCENE:3" => text(&format!("On right now: {}.", lights_on(&result))),
            "SCENE:4" => text("Turned off Living room lamp."),
            _ => text("ok"),
        };
    }
    match (scene, continued) {
        ("SCENE:1", false) | ("SCENE:2", _) => text(UNLOCKING),
        ("SCENE:1", true) => tool_call(OFF, json!({"name": "Front door"})),
        ("SCENE:3", false) => tool_call(READ, json!({"domain": ["switch"]})),
        ("SCENE:3", true) => tool_call(READ, json!({})),
        ("SCENE:4", _) => tool_call(OFF, json!({"name": "Living room lamp"})),
        _ => text("ok"),
    }
}

async fn fake_model() -> (String, Seen) {
    let seen: Seen = Arc::default();
    let app = Router::new()
        .route("/v1/models", get(|| async { Json(json!({"data": [{"id": "qwen-7b"}]})) }))
        .route(
            "/v1/chat/completions",
            post(|State(seen): State<Seen>, Json(body): Json<Value>| async move {
                seen.lock().unwrap().push(body.clone());
                Json(script(&body))
            }),
        )
        .with_state(seen.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let root = format!("http://{}", listener.local_addr().unwrap());
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    (root, seen)
}

/// House: a Custom API bot on the fake model, with the echo server as its Home Assistant
/// (`house`), at `approvals`. Returns its id.
async fn house(env: &Env, root: &str, approvals: &str) -> String {
    let (s, space) = call(&env.app, "POST", "/api/spaces", Some(json!({"name": "Home", "icon": "🏠", "this_machine_enabled": false}))).await;
    assert_eq!(s, StatusCode::CREATED, "{space}");
    let space = space["id"].as_str().unwrap().to_string();
    let (s, acct) = call(
        &env.app,
        "POST",
        &format!("/api/profiles?space={space}"),
        Some(json!({"engine": "custom", "kind": "custom_api", "name": "Local 7B", "protocol": "openai-chat", "base_url": format!("{root}/v1"), "api_key": KEY, "model": "qwen-7b"})),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED, "{acct}");
    let (s, bot) = call(
        &env.app,
        "POST",
        &format!("/api/kwants?space={space}"),
        Some(json!({"name": "House", "title": "Home", "persona_prompt": "", "avatar_sprite": "s", "driver_type": "api", "preferred_model": "custom"})),
    )
    .await;
    assert!(s.is_success(), "{s} {bot}");
    let kid = bot["id"].as_str().unwrap().to_string();
    let server = McpServer {
        id: String::new(),
        name: "house".into(),
        transport: "stdio".into(),
        command: Some(env!("CARGO_BIN_EXE_kwnt").into()),
        args: vec!["mcp-echo-server".into(), "--home-tools".into(), "--server-name".into(), "home-assistant".into()],
        url: None,
        env: BTreeMap::new(),
        headers: BTreeMap::new(),
        enabled: true,
        timeout_secs: 10,
        created_at: None,
    };
    let server = env.state.db.create_mcp_server(&SpaceId::new(space), &server).await.unwrap();
    env.state.db.set_kwant_mcp_servers(&kid, std::slice::from_ref(&server.id)).await.unwrap();
    env.state.db.set_kwant_approvals(&kid, approvals).await.unwrap();
    kid
}

async fn replies_of(env: &Env, kwant: &str) -> Vec<kwnt::db::models::Message> {
    messages(env, kwant).await.into_iter().filter(|m| m.sender_role == "kwant").collect()
}

fn meta_of(m: &kwnt::db::models::Message) -> Value {
    m.meta.as_deref().and_then(|s| serde_json::from_str(s).ok()).unwrap_or_default()
}

/// Sends `prompt` and waits until House is idle with `replies` saved replies; answers any card
/// with `card` (and panics on a card when there is none).
async fn ask(env: &Env, kwant: &str, prompt: &str, replies: usize, card: Option<ApprovalDecision>) -> Vec<kwnt::db::models::Message> {
    let mut events = env.state.event_sender.subscribe();
    send_direct(env, kwant, prompt, json!({})).await;
    let mut cards = 0;
    let done = tokio::time::timeout(Duration::from_secs(90), async {
        loop {
            match tokio::time::timeout(Duration::from_millis(200), events.recv()).await {
                Ok(Ok(e)) => {
                    if let RuntimeEvent::ApprovalRequired { id, .. } = e.unscoped() {
                        let decision = card.clone().expect("no card was expected");
                        assert!(env.state.broker.resolve(id, decision).await);
                        cards += 1;
                    }
                }
                Ok(Err(_)) => {}
                Err(_) => {
                    let idle = env.state.turns.active_turns().iter().all(|t| t.kwant_id != kwant);
                    if idle && replies_of(env, kwant).await.len() >= replies {
                        break;
                    }
                }
            }
        }
    })
    .await;
    assert!(done.is_ok(), "House didn't finish: {:?}", replies_of(env, kwant).await.iter().map(|m| m.text.clone()).collect::<Vec<_>>());
    if card.is_some() {
        assert_eq!(cards, 1, "one card");
    }
    // Nothing more comes after
    tokio::time::sleep(Duration::from_millis(500)).await;
    replies_of(env, kwant).await
}

/// The requests that carried kwant's continuation (as their last user message).
fn continuations(seen: &Seen) -> Vec<String> {
    let seen = seen.lock().unwrap();
    let mut out: Vec<String> = seen
        .iter()
        .filter_map(|b| b["messages"].as_array()?.iter().rev().find(|m| m["role"] == "user").map(content))
        .filter(|c| c.contains(CONTINUATION))
        .collect();
    out.dedup();
    out
}

#[tokio::test]
async fn a_claimed_unlock_with_no_tool_is_continued_into_the_lock_card() {
    let env = setup().await;
    let (root, seen) = fake_model().await;
    let kid = house(&env, &root, "smart").await;

    let replies = ask(&env, &kid, "SCENE:1 unlock the front door", 2, Some(ApprovalDecision::Deny)).await;

    assert_eq!(replies.len(), 2, "{:?}", replies.iter().map(|m| &m.text).collect::<Vec<_>>());
    assert_eq!(replies[0].text, UNLOCKING);
    let asked = continuations(&seen);
    assert_eq!(asked.len(), 1, "one continuation: {asked:?}");
    assert!(asked[0].contains("Your reply says \"Unlocking the front door now.\", but no Home Assistant step changed anything in this turn"), "{}", asked[0]);
    // The continuation called HassTurnOff, the card asked, the person denied: no false claim
    let last = &replies[1].text;
    assert_eq!(last, "You declined the card, so the front door stays locked.");
    assert!(!last.contains("_kwant:"), "{last}");
    assert!(meta_of(&replies[1]).to_string().contains("HassTurnOff"), "the call shows in Details");
}

#[tokio::test]
async fn the_claim_twice_with_no_tool_ends_with_kwants_line() {
    let env = setup().await;
    let (root, seen) = fake_model().await;
    let kid = house(&env, &root, "smart").await;

    let replies = ask(&env, &kid, "SCENE:2 unlock the front door", 2, None).await;

    assert_eq!(continuations(&seen).len(), 1, "continued once, never twice");
    assert_eq!(replies.len(), 2);
    assert_eq!(replies[0].text, UNLOCKING, "the first reply is the bot's own");
    assert_eq!(replies[1].text, format!("{UNLOCKING}\n\n{CHANGE_LINE}"));
    assert_eq!(meta_of(&replies[1])["delivery"]["home_line"], json!(true));
}

#[tokio::test]
async fn a_failed_filtered_read_is_continued_into_a_whole_read() {
    let env = setup().await;
    let (root, seen) = fake_model().await;
    let kid = house(&env, &root, "off").await;

    let replies = ask(&env, &kid, "SCENE:3 are any switches on?", 2, None).await;

    // Home Assistant refused the filter, and the model was told how to read the whole home
    let hinted = {
        let seen = seen.lock().unwrap();
        seen.iter()
            .flat_map(|b| b["messages"].as_array().cloned().unwrap_or_default())
            .filter(|m| m["role"] == "tool")
            .map(|m| content(&m))
            .find(|c| c.contains("No exposed entities found in domain(s): switch"))
    };
    let hinted = hinted.expect("the failed read reached the model");
    assert!(hinted.contains("Call GetLiveContext again with no arguments"), "{hinted}");
    // ...but Details shows only Home Assistant's error
    let details = meta_of(&replies[0])["activity"].to_string();
    assert!(details.contains("No exposed entities found in domain(s): switch"), "{details}");
    assert!(!details.contains("haven't seen the home yet"), "{details}");

    let asked = continuations(&seen);
    assert_eq!(asked.len(), 1, "{asked:?}");
    assert!(
        asked[0].contains("Your Home Assistant read failed (No exposed entities found in domain(s): switch), so you haven't seen the home yet."),
        "{}",
        asked[0]
    );
    let last = &replies[1].text;
    assert_eq!(last, "On right now: Kitchen light and Porch light.");
}

#[tokio::test]
async fn turning_off_the_lamp_with_a_tool_that_worked_needs_nothing_more() {
    let env = setup().await;
    let (root, seen) = fake_model().await;
    let kid = house(&env, &root, "off").await;

    let replies = ask(&env, &kid, "SCENE:4 Turn off the living room lamp", 1, None).await;

    assert_eq!(replies.len(), 1, "{:?}", replies.iter().map(|m| &m.text).collect::<Vec<_>>());
    assert_eq!(replies[0].text, "Turned off Living room lamp.");
    assert!(continuations(&seen).is_empty());
    assert_ne!(meta_of(&replies[0])["delivery"]["home_line"], json!(true));
}
