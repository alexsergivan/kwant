//! 34g: signing in again closes the "needs you" line and the stopped job carries on
//! (autonomy/connection-signin-resumes/brief.md, Mira's ux-pass-0942-connections report §1).
//! Full turns on the fake claude, sign-ins against the mock OAuth + MCP server, and a fake
//! registry for suggestions. Nothing here touches the network beyond 127.0.0.1.
//!
//! Rules:
//! - signing in to a connection closes every bot's open line on that connection, and only those;
//! - each chat with a closed line gets one new turn that carries on, however many lines it had;
//! - the first sign-in (or the install, when none is needed) of a server a bot suggested carries
//!   the suggestion's chat on once; a later sign-in doesn't again;
//! - deleting the connection closes its lines as removed, and starts no turn.

mod common;

use axum::{
    extract::{Path, Query},
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use common::*;
use kwnt::db::models::{McpServer, Message};
use kwnt::db::spaces::SpaceId;
use kwnt::mcp::{oauth_mock, registry::McpRegistry};
use kwnt::server::continuity::{SelfScope, TurnOrigin};
use kwnt::server::mcp_runtime;
use serde_json::{json, Value};
use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;
use std::time::Duration;

// ---- helpers ------------------------------------------------------------------------------

async fn listen() -> (tokio::net::TcpListener, String) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    (listener, base)
}

/// The mock OAuth provider with its MCP server; returns the server's URL.
async fn signin_mcp() -> String {
    let (listener, base) = listen().await;
    tokio::spawn(oauth_mock::serve(listener, oauth_mock::MockOptions::default()));
    format!("{base}/mcp")
}

/// A streamable-HTTP MCP server with one `echo` tool and no sign-in.
async fn open_mcp() -> String {
    async fn mcp(Json(msg): Json<Value>) -> Response {
        let id = msg["id"].clone();
        let result = match msg["method"].as_str().unwrap_or("") {
            "initialize" => json!({"protocolVersion": "2025-06-18", "capabilities": {"tools": {}},
                                   "serverInfo": {"name": "fake-echo", "version": "1.0.0"}}),
            "tools/list" => json!({"tools": [{"name": "echo", "description": "Echoes text",
                                   "inputSchema": {"type": "object", "properties": {"text": {"type": "string"}}}}]}),
            "tools/call" => json!({"content": [{"type": "text", "text": "echo"}]}),
            _ => return StatusCode::ACCEPTED.into_response(),
        };
        Json(json!({"jsonrpc": "2.0", "id": id, "result": result})).into_response()
    }
    let (listener, base) = listen().await;
    tokio::spawn(async move { axum::serve(listener, Router::new().route("/mcp", post(mcp))).await });
    format!("{base}/mcp")
}

/// An http server in the default space, switched on for every bot.
async fn http_server(env: &Env, name: &str, url: &str) -> String {
    let (s, v) = call(&env.app, "POST", "/api/mcp/servers", Some(json!({"name": name, "transport": "http", "url": url}))).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    v["id"].as_str().unwrap().to_string()
}

/// A stdio server that is never started (the fake claude prints its calls).
async fn stdio_server(env: &Env, name: &str) -> McpServer {
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

/// The person signs in to server `id`: start → the mock's Allow → kwnt's callback.
async fn sign_in(env: &Env, id: &str) {
    let host = format!("127.0.0.1:{}", env.state.config.port);
    let origin = format!("http://{host}");
    let (s, started) =
        call_with(&env.app, "POST", &format!("/api/mcp/servers/{id}/oauth/start"), Some(json!({"origin": origin})), &[("host", host.as_str())])
            .await;
    assert_eq!(s, StatusCode::OK, "{started}");
    let authorize_url = started["authorize_url"].as_str().unwrap().to_string();
    let resp = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .unwrap()
        .get(&authorize_url)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let redirect = resp.headers()["x-mock-redirect"].to_str().unwrap().to_string();
    let redirect = url::Url::parse(&redirect).unwrap();
    let path_and_query = format!("{}?{}", redirect.path(), redirect.query().unwrap());
    let (s, _) = call_with(&env.app, "GET", &path_and_query, None, &[("host", host.as_str())]).await;
    assert_eq!(s, StatusCode::OK);
    assert!(env.state.db.get_mcp_oauth_token(id).await.unwrap().is_some(), "the sign-in was saved");
}

/// A claude tool call that fails with HTTP 401, as `stream-json` lines.
fn call_401(id: &str, tool: &str) -> [Value; 2] {
    [
        json!({"type": "assistant", "message": {"content": [{"type": "tool_use", "id": id, "name": tool, "input": {}}]}}),
        json!({"type": "user", "message": {"content": [{"type": "tool_result", "tool_use_id": id, "is_error": true,
            "content": [{"type": "text", "text": "HTTP 401 Unauthorized: token expired"}]}]}}),
    ]
}

/// One turn of `kwant` in its direct chat that calls `tool` and gets a 401; waits for its reply.
async fn failing_turn(env: &Env, kwant: &str, tag: &str, tool: &str) {
    let lines = call_401(&format!("{tag}_1"), tool);
    send_direct(env, kwant, &format!("go {} REPLYTAG:{tag}", stream_lines_marker(&lines)), json!({})).await;
    wait_reply(env, kwant, &format!("answer {tag}")).await;
}

fn meta(m: &Message) -> Value {
    m.meta.as_deref().and_then(|s| serde_json::from_str(s).ok()).unwrap_or(Value::Null)
}

/// The chat's connection lines on `key`.
async fn lines_on(env: &Env, chat: &str, key: &str) -> Vec<Value> {
    messages(env, chat).await.iter().map(meta).filter(|v| v["kind"] == "tool_failure" && v["key"] == key).collect()
}

/// Needs-you items for connections.
async fn waiting(env: &Env) -> Vec<Value> {
    let (s, v) = call(&env.app, "GET", "/api/waiting", None).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    v["items"].as_array().unwrap().iter().filter(|i| i["kind"] == "connection").cloned().collect()
}

async fn status_of(env: &Env, kwant: &str) -> Value {
    let (s, v) = call(&env.app, "GET", "/api/bots/status", None).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    v["bots"].as_array().unwrap().iter().find(|b| b["kwant_id"] == kwant).cloned().unwrap()
}

async fn wait_available(env: &Env, kwant: &str) {
    wait_for("the bot to be available", || async { status_of(env, kwant).await["status"] == "available" }).await;
}

/// The kwant replies in a chat that aren't connection lines.
async fn replies(env: &Env, chat: &str) -> usize {
    messages(env, chat).await.iter().filter(|m| m.sender_role == "kwant" && meta(m)["kind"] != "tool_failure").count()
}

/// The words a resumed turn starts with after a sign-in.
fn signed_in_again(label: &str) -> String {
    format!("[You can use {label} again: the person signed in.]")
}

// ---- 1: scoping, and one resume per chat ----------------------------------------------------

/// Signing in to one connection closes every bot's open lines on it and no other line, takes it
/// out of Needs you, and carries each chat on once, even a chat with two lines on it.
#[tokio::test]
async fn signing_in_closes_that_connections_lines_and_resumes_each_chat_once() {
    let env = setup().await;
    let x = http_server(&env, "tasklane", &signin_mcp().await).await;
    let notesy = stdio_server(&env, "notesy").await;
    let (kx, ky) = (format!("mcp:{x}"), format!("mcp:{}", notesy.id));

    // A: two turns that lose tasklane, one that loses notesy; B: one that loses tasklane
    failing_turn(&env, &env.a, "sr_a1", "mcp__tasklane__list_tasks").await;
    failing_turn(&env, &env.a, "sr_a2", "mcp__tasklane__list_tasks").await;
    failing_turn(&env, &env.a, "sr_a3", "mcp__notesy__search").await;
    failing_turn(&env, &env.b, "sr_b1", "mcp__tasklane__list_tasks").await;
    assert_eq!(lines_on(&env, &env.a, &kx).await.len(), 2);
    assert_eq!(lines_on(&env, &env.b, &kx).await.len(), 1);
    assert_eq!(waiting(&env).await.len(), 3, "A·tasklane, A·notesy, B·tasklane");
    // Both bots need the person now, so "available" can't be waited for: let the turns end
    tokio::time::sleep(Duration::from_millis(1000)).await;
    let (before_a, before_b) = (replies(&env, &env.a).await, replies(&env, &env.b).await);

    sign_in(&env, &x).await;

    // Every tasklane line is closed as fixed, the notesy line stays open
    wait_for("the tasklane lines to close", || async {
        let mut all = lines_on(&env, &env.a, &kx).await;
        all.extend(lines_on(&env, &env.b, &kx).await);
        all.iter().all(|v| v["resolved_at"].is_string())
    })
    .await;
    for v in lines_on(&env, &env.a, &kx).await.into_iter().chain(lines_on(&env, &env.b, &kx).await) {
        assert_ne!(v["removed"], true, "a sign-in fixes, it doesn't remove: {v}");
    }
    let notesy: Vec<Value> =
        messages(&env, &env.a).await.iter().map(meta).filter(|v| v["kind"] == "tool_failure" && v["connection"] == "notesy").collect();
    assert_eq!(notesy.len(), 1);
    assert!(notesy[0]["resolved_at"].is_null(), "another connection's line stays open: {}", notesy[0]);
    assert_eq!(notesy[0]["key"], ky.as_str());
    let items = waiting(&env).await;
    assert_eq!(items.len(), 1, "{items:?}");
    assert_eq!(items[0]["connection"], "notesy");

    // One new turn per chat: A's (two lines) once, B's once
    let marker = signed_in_again("tasklane");
    wait_for("both chats to carry on", || async {
        replies(&env, &env.a).await > before_a && replies(&env, &env.b).await > before_b
    })
    .await;
    // B needs nothing now; A still needs the person for notesy
    wait_available(&env, &env.b).await;
    assert_eq!(status_of(&env, &env.a).await["status"], "needs_you");
    tokio::time::sleep(Duration::from_millis(1500)).await;
    assert_eq!(invocations_with(&marker).len(), 2, "one resume per chat");
    assert_eq!(replies(&env, &env.a).await, before_a + 1);
    assert_eq!(replies(&env, &env.b).await, before_b + 1);
    let prompt = &invocations_with(&marker)[0];
    assert!(prompt.contains("Carry on with what you were doing when it stopped."), "{prompt}");
}

// ---- 2: delete closes as removed, no turn -----------------------------------------------------

/// Deleting the connection closes its lines as removed: out of Needs you, and no turn starts.
#[tokio::test]
async fn deleting_the_connection_closes_its_lines_as_removed_with_no_turn() {
    let env = setup().await;
    let gone = stdio_server(&env, "oldtool").await;
    let key = format!("mcp:{}", gone.id);
    failing_turn(&env, &env.a, "rm_a1", "mcp__oldtool__list").await;
    assert_eq!(lines_on(&env, &env.a, &key).await.len(), 1);
    assert_eq!(waiting(&env).await.len(), 1);
    tokio::time::sleep(Duration::from_millis(1000)).await;
    let before = messages(&env, &env.a).await.len();

    let (s, v) = call(&env.app, "DELETE", &format!("/api/mcp/servers/{}", gone.id), None).await;
    assert_eq!(s, StatusCode::OK, "{v}");

    wait_for("the line to close as removed", || async {
        let l = lines_on(&env, &env.a, &key).await;
        l.len() == 1 && l[0]["resolved_at"].is_string()
    })
    .await;
    let line = &lines_on(&env, &env.a, &key).await[0];
    assert_eq!(line["removed"], true, "{line}");
    assert!(waiting(&env).await.is_empty(), "the Inbox item goes");
    tokio::time::sleep(Duration::from_millis(1500)).await;
    assert_eq!(messages(&env, &env.a).await.len(), before, "no turn, no new message");
    assert_eq!(status_of(&env, &env.a).await["status"], "available");
    assert!(invocations_with(&signed_in_again("oldtool")).is_empty());
}

// ---- 3: suggestions carry on once after the first connect ------------------------------------

async fn registry_one(Path((name, _version)): Path<(String, String)>, axum::extract::State(entries): axum::extract::State<Arc<Vec<Value>>>) -> Response {
    match entries.iter().find(|e| e["server"]["name"] == name.as_str()) {
        Some(e) => Json(e.clone()).into_response(),
        None => StatusCode::NOT_FOUND.into_response(),
    }
}

async fn registry_list(
    Query(q): Query<HashMap<String, String>>,
    axum::extract::State(entries): axum::extract::State<Arc<Vec<Value>>>,
) -> Json<Value> {
    let search = q.get("search").cloned().unwrap_or_default();
    let servers: Vec<Value> = entries.iter().filter(|e| e["server"]["name"].as_str().unwrap().contains(search.as_str())).cloned().collect();
    Json(json!({"servers": servers, "metadata": {"count": servers.len()}}))
}

fn entry(name: &str, title: &str, url: &str) -> Value {
    json!({
        "server": {"$schema": "https://static.modelcontextprotocol.io/schemas/2025-09-29/server.schema.json",
                   "name": name, "title": title, "description": format!("{title} for tests."), "version": "1.0.0",
                   "repository": {"url": "https://github.com/example/tasks", "source": "github"},
                   "remotes": [{"type": "streamable-http", "url": url}]},
        "_meta": {"io.modelcontextprotocol.registry/official": {"status": "active", "isLatest": true, "updatedAt": "2026-01-01T00:00:00Z"}},
    })
}

/// A registry with a server that needs sign-in ("Tasklist") and one that doesn't ("Echoer").
async fn fake_registry() -> String {
    let entries = Arc::new(vec![
        entry("io.github.kwnt-test/tasklist", "Tasklist", &signin_mcp().await),
        entry("io.github.kwnt-test/echoer", "Echoer", &open_mcp().await),
    ]);
    let app = Router::new()
        .route("/v0.1/servers", get(registry_list))
        .route("/v0.1/servers/:name/versions/:version", get(registry_one))
        .with_state(entries);
    let (listener, base) = listen().await;
    tokio::spawn(async move { axum::serve(listener, app).await });
    base
}

async fn setup_with_registry() -> Env {
    let registry = fake_registry().await;
    setup_with(move |s| s.mcp_registry = Arc::new(McpRegistry::new(&registry))).await
}

/// Bot A suggests `registry_id` in its direct chat; returns the suggestion's id.
async fn suggest(env: &Env, registry_id: &str) -> String {
    let bot = env.state.db.get_kwant(&env.a).await.unwrap().unwrap();
    let turn_id = format!("t-{}", uuid::Uuid::new_v4());
    let scope = SelfScope {
        kwant_id: bot.id.clone(),
        kwant_name: bot.name.clone(),
        origin: Some(TurnOrigin { direct_kwant_id: Some(bot.id.clone()), ..Default::default() }),
        turn_id: Some(turn_id.clone()),
        ..Default::default()
    };
    let token = mcp_runtime::prepare_turn_with_self(&env.state, &bot, Some(turn_id), None, Some(scope)).await.unwrap().1;
    let rpc = json!({"jsonrpc": "2.0", "id": 1, "method": "tools/call",
                     "params": {"name": "mcp_suggest", "arguments": {"registry_id": registry_id, "reason": "You asked for your open tasks."}}});
    let auth = format!("Bearer {token}");
    let (s, v) = call_with(&env.app, "POST", "/api/mcp/self", Some(rpc), &[("authorization", auth.as_str())]).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_ne!(v["result"]["isError"], true, "{v}");
    mcp_runtime::finish_turn(&env.state, &token).await;
    let (s, list) = call(&env.app, "GET", "/api/mcp/suggestions", None).await;
    assert_eq!(s, StatusCode::OK, "{list}");
    list.as_array().unwrap().iter().find(|x| x["registry_name"] == registry_id).unwrap()["id"].as_str().unwrap().to_string()
}

async fn install(env: &Env, suggestion: &str) -> Value {
    let (s, v) = call(&env.app, "POST", &format!("/api/mcp/suggestions/{suggestion}/install"), None).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    v
}

/// A suggested server that needs sign-in: the first sign-in carries the suggestion's chat on
/// once; signing in again later doesn't.
#[tokio::test]
async fn the_first_sign_in_of_a_suggested_server_carries_its_chat_on_once() {
    let env = setup_with_registry().await;
    let id = suggest(&env, "io.github.kwnt-test/tasklist").await;
    let installed = install(&env, &id).await;
    assert_eq!(installed["oauth_required"], true, "{installed}");
    let server = installed["server"]["id"].as_str().unwrap().to_string();
    let marker = "[Tasklist is connected.]";
    tokio::time::sleep(Duration::from_millis(1000)).await;
    assert!(invocations_with(marker).is_empty(), "not before the sign-in");
    let before = replies(&env, &env.a).await;

    sign_in(&env, &server).await;
    wait_for("the suggestion's chat to carry on", || async { !invocations_with(marker).is_empty() }).await;
    wait_for("its reply", || async { replies(&env, &env.a).await > before }).await;
    wait_available(&env, &env.a).await;
    let prompts = invocations_with(marker);
    assert_eq!(prompts.len(), 1, "{prompts:?}");
    assert!(prompts[0].contains("Carry on with the request you suggested it for."), "{}", prompts[0]);

    // A later sign-in to the same server is not a first connect
    sign_in(&env, &server).await;
    tokio::time::sleep(Duration::from_millis(1500)).await;
    wait_available(&env, &env.a).await;
    assert_eq!(invocations_with(marker).len(), 1, "carried on once only");
}

/// A suggested server that needs no sign-in carries the chat on once it is installed.
#[tokio::test]
async fn installing_a_suggested_server_without_sign_in_carries_its_chat_on() {
    let env = setup_with_registry().await;
    let id = suggest(&env, "io.github.kwnt-test/echoer").await;
    let before = replies(&env, &env.a).await;
    let installed = install(&env, &id).await;
    assert_eq!(installed["oauth_required"], false, "{installed}");
    let marker = "[Echoer is connected.]";
    wait_for("the suggestion's chat to carry on", || async { !invocations_with(marker).is_empty() }).await;
    wait_for("its reply", || async { replies(&env, &env.a).await > before }).await;
    wait_available(&env, &env.a).await;
    tokio::time::sleep(Duration::from_millis(1000)).await;
    let prompts = invocations_with(marker);
    assert_eq!(prompts.len(), 1, "{prompts:?}");
    assert!(prompts[0].contains("Carry on with the request you suggested it for."), "{}", prompts[0]);
}
