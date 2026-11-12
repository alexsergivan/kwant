//! 0.9.5: a connection failing says so in the chat (release-095/tool-failure-line/brief.md).
//! Full turns on the fake claude, which prints the tool calls and results each test gives it.

mod common;

use axum::http::StatusCode;
use common::*;
use kwnt::db::models::{McpServer, Message};
use kwnt::db::spaces::SpaceId;
use serde_json::{json, Value};
use std::collections::BTreeMap;

async fn server(env: &Env, name: &str) -> McpServer {
    env.state
        .db
        .create_mcp_server(
            &SpaceId::default_space(),
            &McpServer {
                id: String::new(),
                name: name.into(),
                transport: "stdio".into(),
                command: Some("/usr/bin/true".into()),
                args: vec![],
                url: None,
                env: BTreeMap::new(),
                headers: BTreeMap::new(),
                enabled: true,
                timeout_secs: 30,
                created_at: None,
            },
        )
        .await
        .unwrap()
}

/// A claude tool call and its result, as `stream-json` lines. `error` makes the call fail with it.
fn call(id: &str, tool: &str, error: Option<&str>) -> [Value; 2] {
    let result = match error {
        Some(e) => {
            json!({"type": "tool_result", "tool_use_id": id, "is_error": true, "content": [{"type": "text", "text": e}]})
        }
        None => json!({"type": "tool_result", "tool_use_id": id, "content": "ok"}),
    };
    [
        json!({"type": "assistant", "message": {"content": [{"type": "tool_use", "id": id, "name": tool, "input": {}}]}}),
        json!({"type": "user", "message": {"content": [result]}}),
    ]
}

/// Runs one turn of bot A in its direct chat that prints `lines`, and waits for its reply.
async fn turn(env: &Env, tag: &str, lines: &[Value]) {
    send_direct(env, &env.a, &format!("go {} REPLYTAG:{tag}", stream_lines_marker(lines)), json!({})).await;
    wait_reply(env, &env.a, &format!("answer {tag}")).await;
}

fn meta(m: &Message) -> Value {
    m.meta.as_deref().and_then(|s| serde_json::from_str(s).ok()).unwrap_or(Value::Null)
}

async fn failure_lines(env: &Env) -> Vec<(Message, Value)> {
    messages(env, &env.a)
        .await
        .into_iter()
        .map(|m| (meta(&m), m))
        .filter(|(v, _)| v["kind"] == "tool_failure")
        .map(|(v, m)| (m, v))
        .collect()
}

async fn waiting(env: &Env) -> Vec<Value> {
    let (s, v) = common::call(&env.app, "GET", "/api/waiting", None).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    v["items"].as_array().unwrap().iter().filter(|i| i["kind"] == "connection").cloned().collect()
}

async fn status_of(env: &Env, kwant: &str) -> Value {
    let (s, v) = common::call(&env.app, "GET", "/api/bots/status", None).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    v["bots"].as_array().unwrap().iter().find(|b| b["kwant_id"] == kwant).cloned().unwrap()
}

/// A 401 posts one line with a fix link; the bot needs the person until a call through that
/// connection works again.
#[tokio::test]
async fn a_401_posts_one_line_with_a_fix_link_and_needs_you_until_it_works() {
    let env = setup().await;
    let gh = server(&env, "github").await;
    let bot = env.state.db.get_kwant(&env.a).await.unwrap().unwrap().name;

    turn(&env, "tf401", &call("tf401_1", "mcp__github__list_issues", Some("HTTP 401 Unauthorized: Bad credentials"))).await;
    let lines = failure_lines(&env).await;
    assert_eq!(lines.len(), 1, "{lines:?}");
    let (m, v) = &lines[0];
    assert_eq!(m.sender_role, "kwant");
    assert_eq!(
        m.text,
        format!("github stopped working for {bot}: its sign-in expired or was refused. Reconnect it in Settings › Connections.")
    );
    assert_eq!(v["cause"], "sign_in");
    assert_eq!(v["needs_you"], true);
    assert_eq!(v["fix"], json!({"label": "Reconnect", "opens": "connections", "connection_id": format!("mcp:{}", gh.id)}));
    assert_eq!(v["tool"], "github · list_issues");
    // The line comes before the turn's reply
    let all = messages(&env, &env.a).await;
    let pos = |id: &str| all.iter().position(|x| x.id == id).unwrap();
    let reply = all.iter().find(|x| x.text.contains("answer tf401")).unwrap();
    assert!(pos(&m.id) < pos(&reply.id), "line after reply");

    // Needs you: in the person's list and on the bot
    let items = waiting(&env).await;
    assert_eq!(items.len(), 1, "{items:?}");
    assert_eq!(items[0]["message_id"], m.id.as_str());
    assert_eq!(items[0]["kwant_id"], env.a.as_str());
    assert_eq!(items[0]["fix"]["connection_id"], format!("mcp:{}", gh.id));
    let st = status_of(&env, &env.a).await;
    assert_eq!((st["status"].as_str(), st["reason_code"].as_str()), (Some("needs_you"), Some("needs_sign_in")), "{st}");
    assert_eq!(st["waits_on"]["label"], "github");
    assert_eq!(status_of(&env, &env.b).await["status"], "available");

    // Connected again: the next call through github works, and it no longer needs the person
    turn(&env, "tf401ok", &call("tf401_2", "mcp__github__list_issues", None)).await;
    assert!(waiting(&env).await.is_empty());
    assert_eq!(status_of(&env, &env.a).await["status"], "available");
    let lines = failure_lines(&env).await;
    assert_eq!(lines.len(), 1, "a working call posts nothing");
    assert!(lines[0].1["resolved_at"].is_string(), "{}", lines[0].1);
}

/// Two failures of one connection in a turn give one line.
#[tokio::test]
async fn two_failures_of_one_connection_give_one_line() {
    let env = setup().await;
    server(&env, "github").await;
    server(&env, "notion").await;
    let mut lines = Vec::new();
    lines.extend(call("tf2_1", "mcp__github__list_issues", Some("401 Unauthorized")));
    lines.extend(call("tf2_2", "mcp__github__get_repo", Some("401 Unauthorized")));
    lines.extend(call("tf2_3", "mcp__github__get_repo", Some("token expired")));
    // A tool's own error isn't the connection's
    lines.extend(call("tf2_4", "mcp__notion__search", Some("invalid arguments: query is required")));
    turn(&env, "tf2", &lines).await;
    let lines = failure_lines(&env).await;
    assert_eq!(lines.len(), 1, "{lines:?}");
    assert_eq!(lines[0].1["connection"], "github");
    assert_eq!(waiting(&env).await.len(), 1);
}

/// A failure the bot's retry fixed posts nothing, and the bot doesn't need the person.
#[tokio::test]
async fn a_failure_a_retry_fixed_gives_no_line() {
    let env = setup().await;
    server(&env, "github").await;
    let mut lines = Vec::new();
    lines.extend(call("tfr_1", "mcp__github__list_issues", Some("502 Bad Gateway")));
    lines.extend(call("tfr_2", "mcp__github__list_issues", None));
    turn(&env, "tfr", &lines).await;
    assert!(failure_lines(&env).await.is_empty());
    assert!(waiting(&env).await.is_empty());
    assert_eq!(status_of(&env, &env.a).await["status"], "available");
}

/// A server that failed to start says so once; one that needs a sign-in needs the person.
#[tokio::test]
async fn servers_that_did_not_start_say_so() {
    let env = setup().await;
    server(&env, "github").await;
    server(&env, "notion").await;
    let init = json!({"type": "system", "subtype": "init", "tools": [], "mcp_servers": [
        {"name": "github", "status": "needs-auth"}, {"name": "notion", "status": "failed"}, {"name": "kwnt_tools", "status": "connected"}]});
    let mut lines = vec![init];
    lines.extend(call("tfs_1", "mcp__notion__search", Some("connection closed")));
    turn(&env, "tfs", &lines).await;
    let lines = failure_lines(&env).await;
    let causes: Vec<(String, String)> =
        lines.iter().map(|(_, v)| (v["connection"].as_str().unwrap().into(), v["cause"].as_str().unwrap().into())).collect();
    assert_eq!(causes.len(), 2, "{causes:?}");
    assert!(causes.contains(&("github".into(), "sign_in".into())), "{causes:?}");
    assert!(causes.contains(&("notion".into(), "not_started".into())), "{causes:?}");
    let items = waiting(&env).await;
    assert_eq!(items.len(), 1, "{items:?}");
    assert_eq!(items[0]["connection"], "github");
}
