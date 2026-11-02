//! Smart Results over HTTP: a bot's final reply with a table comes back with its comparison block
//! as `results` in the chat's message list and in a thread's; every other message has
//! `results: null`.

mod common;

use axum::http::StatusCode;
use base64::Engine;
use common::*;
use kwnt::db::models::{MessageExtra, NewMessage};
use serde_json::{json, Value};

const PLANS: &str = "Here are the plans.\n\n| Plan | Price | Seats |\n|---|---|---|\n| Basic | EUR 12 | 1 |\n| Team | EUR 18 | 5 |\n| Business | EUR 26 | 20 |\n\nI recommend Team.";

async fn listed(env: &Env, uri: &str) -> Vec<Value> {
    let (s, v) = call(&env.app, "GET", uri, None).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    v.as_array().unwrap().clone()
}

#[tokio::test]
async fn a_reply_with_a_table_comes_back_with_its_comparison() {
    let env = setup().await;
    let marker = format!("REPLYTEXT:{}", base64::engine::general_purpose::STANDARD.encode(PLANS));
    send_direct(&env, &env.a, &format!("Compare the plans {marker}"), json!({})).await;
    let reply = wait_reply(&env, &env.a, "| Business | EUR 26 | 20 |").await;
    let list = listed(&env, &format!("/api/channels/{}/messages", env.a)).await;
    let got = list.iter().find(|m| m["id"] == reply.id.as_str()).expect("the reply is listed");
    assert!(got["text"].as_str().unwrap().contains(PLANS), "{got}");
    assert_eq!(got["results"]["version"], 1, "{got}");
    let block = &got["results"]["blocks"][0];
    assert_eq!(block["type"], "comparison");
    assert_eq!(block["source"], json!({"kind": "markdown_table", "index": 0}));
    assert_eq!(block["rows"][2]["cells"], json!(["Business", "EUR 26", "20"]));
    assert_eq!(block["columns"][1]["unit"], "EUR");
    assert_eq!(block["highlight_row"], "r1");
    let others: Vec<&Value> = list.iter().filter(|m| m["id"] != reply.id.as_str()).collect();
    assert!(!others.is_empty());
    for m in others {
        assert_eq!(m.get("results"), Some(&Value::Null), "{m}");
    }
}

#[tokio::test]
async fn a_plain_reply_has_null_results() {
    let env = setup().await;
    send_direct(&env, &env.a, "Hello REPLYTAG:plain", json!({})).await;
    let reply = wait_reply(&env, &env.a, "answer plain").await;
    let list = listed(&env, &format!("/api/channels/{}/messages", env.a)).await;
    let got = list.iter().find(|m| m["id"] == reply.id.as_str()).expect("the reply is listed");
    assert_eq!(got.get("results"), Some(&Value::Null), "{got}");
}

#[tokio::test]
async fn a_thread_lists_its_blocks_too() {
    let env = setup().await;
    let thread = env.state.db.create_thread(&env.a, "Plans", "chat", None, "user").await.unwrap();
    let extra = MessageExtra { thread_id: Some(thread.id.clone()), ..Default::default() };
    let reply = NewMessage {
        channel_id: None,
        direct_kwant_id: Some(env.a.clone()),
        sender_role: "kwant".into(),
        sender_id: env.a.clone(),
        text: PLANS.into(),
        reasoning_content: None,
        tool_calls: None,
        tokens_used: 0,
    };
    let saved = env.state.db.insert_message_with(&reply, &extra).await.unwrap();
    let list = listed(&env, &format!("/api/threads/{}/messages", thread.id)).await;
    let got = list.iter().find(|m| m["id"] == saved.id.as_str()).expect("the reply is listed");
    assert_eq!(got["results"]["blocks"][0]["type"], "comparison", "{got}");
}
