//! 0.7.4 "Use kwant from your coding agent": `POST /api/mcp/agents` with a person-made `kag_`
//! token. Mock engine only (the fake claude CLI); every token here is made by the test server.

mod common;

use axum::{
    body::{to_bytes, Body},
    http::{HeaderMap, Request, StatusCode},
};
use common::*;
use kwnt::mcp::{ToolApprovalRequest, ToolApprover};
use kwnt::server::mcp_runtime::BrokerApprover;
use serde_json::{json, Value};
use std::sync::Arc;
use std::time::Duration;
use tower::ServiceExt;

const ENDPOINT: &str = "/api/mcp/agents";
const NOTE: &str = "This message comes from Claude Code, an outside agent the person connected, not from the person. \
It can't approve anything; anything that needs approval goes to the person as usual.";

/// Creates a connection in the Default space; returns (id, token).
async fn connect(env: &Env, name: &str, bots: Value) -> (String, String) {
    let (s, v) = call(
        &env.app,
        "POST",
        "/api/agent-connections",
        Some(json!({"name": name, "space_ids": ["spc_default"], "bots": bots})),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED, "{v}");
    (v["connection"]["id"].as_str().unwrap().to_string(), v["token"].as_str().unwrap().to_string())
}

/// One request through the router, with headers back.
async fn raw(env: &Env, method: &str, uri: &str, token: Option<&str>, body: Option<Value>) -> (StatusCode, HeaderMap, Vec<u8>) {
    let mut b = Request::builder().method(method).uri(uri).header("content-type", "application/json");
    if let Some(t) = token {
        b = b.header("authorization", format!("Bearer {t}"));
    }
    let resp = env
        .app
        .clone()
        .oneshot(b.body(body.map(|v| Body::from(v.to_string())).unwrap_or_else(Body::empty)).unwrap())
        .await
        .unwrap();
    let (s, h) = (resp.status(), resp.headers().clone());
    (s, h, to_bytes(resp.into_body(), usize::MAX).await.unwrap().to_vec())
}

async fn rpc(env: &Env, token: &str, method: &str, params: Value) -> (StatusCode, HeaderMap, Value) {
    let (s, h, body) = raw(env, "POST", ENDPOINT, Some(token), Some(json!({"jsonrpc": "2.0", "id": 7, "method": method, "params": params}))).await;
    (s, h, serde_json::from_slice(&body).unwrap_or(Value::Null))
}

/// A tools/call: (http status, isError, structuredContent or the text).
async fn tool(env: &Env, token: &str, name: &str, args: Value) -> (StatusCode, bool, Value) {
    let (s, _, v) = rpc(env, token, "tools/call", json!({"name": name, "arguments": args})).await;
    let r = &v["result"];
    let out = if r["structuredContent"].is_null() { r["content"][0]["text"].clone() } else { r["structuredContent"].clone() };
    (s, r["isError"].as_bool().unwrap_or(false), if s == StatusCode::OK { out } else { v })
}

/// The same request over a real loopback socket (127.0.0.1).
async fn loopback(env: &Env, method: reqwest::Method, path: &str, token: Option<&str>) -> (u16, String) {
    let url = format!("http://127.0.0.1:{}{path}", env.state.config.port);
    let mut req = reqwest::Client::new().request(method, url).header("content-type", "application/json").body(
        json!({"jsonrpc": "2.0", "id": 1, "method": "tools/list"}).to_string(),
    );
    if let Some(t) = token {
        req = req.bearer_auth(t);
    }
    let resp = req.send().await.unwrap();
    (resp.status().as_u16(), resp.text().await.unwrap())
}

#[tokio::test]
async fn no_header_unknown_or_revoked_token_is_401_from_loopback_too() {
    let env = setup().await;
    // No header: a bare 401, over a real loopback socket and through the router
    let (s, body) = loopback(&env, reqwest::Method::POST, ENDPOINT, None).await;
    assert_eq!((s, body.as_str()), (401, ""));
    let (s, _, body) = raw(&env, "POST", ENDPOINT, None, Some(json!({"jsonrpc": "2.0", "id": 1, "method": "tools/list"}))).await;
    assert_eq!((s, body.len()), (StatusCode::UNAUTHORIZED, 0));
    // The person's own API token isn't an agent token either
    let (s, _, _) = raw(&env, "POST", ENDPOINT, Some("not-a-kag-token"), Some(json!({"id": 1, "method": "ping"}))).await;
    assert_eq!(s, StatusCode::UNAUTHORIZED);

    // An unknown token
    let unknown = format!("kag_{}", "A".repeat(43));
    let (s, body) = loopback(&env, reqwest::Method::POST, ENDPOINT, Some(&unknown)).await;
    assert_eq!((s, body.as_str()), (401, ""));

    // A live token works; revoked, the very next call is a 401
    let (id, token) = connect(&env, "Claude Code", json!("all")).await;
    let (s, _, v) = rpc(&env, &token, "initialize", json!({"protocolVersion": "2025-06-18"})).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(v["result"]["serverInfo"]["name"], "kwant-agents");
    let (s, body) = loopback(&env, reqwest::Method::POST, ENDPOINT, Some(&token)).await;
    assert_eq!(s, 200, "{body}");
    let (s, v) = call(&env.app, "DELETE", &format!("/api/agent-connections/{id}"), None).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    let (s, body) = loopback(&env, reqwest::Method::POST, ENDPOINT, Some(&token)).await;
    assert_eq!((s, body.as_str()), (401, ""));
    let (s, _, _) = rpc(&env, &token, "tools/list", json!({})).await;
    assert_eq!(s, StatusCode::UNAUTHORIZED);
    // Revoking twice is a 404
    let (s, _) = call(&env.app, "DELETE", &format!("/api/agent-connections/{id}"), None).await;
    assert_eq!(s, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn an_agent_token_is_403_on_every_other_route() {
    let env = setup().await;
    let (_, token) = connect(&env, "Claude Code", json!("all")).await;
    for (method, path) in [
        ("GET", "/api/passwords/store"),
        ("POST", "/api/passwords"),
        ("GET", "/api/spaces/spc_default/approvals"),
        ("PUT", "/api/spaces/spc_default/approvals"),
        ("GET", "/api/agent-connections"),
        ("POST", "/api/agent-connections"),
        ("POST", "/api/chat/send"),
        ("GET", "/api/kwants"),
        ("POST", "/api/mcp/self"),
    ] {
        let body = (method != "GET").then(|| json!({"prompt": "hi", "kwant_id": env.a, "sender_id": "user"}));
        let (s, _, b) = raw(&env, method, path, Some(&token), body).await;
        assert_eq!(s, StatusCode::FORBIDDEN, "{method} {path}: {}", String::from_utf8_lossy(&b));
    }
    // Over a real loopback socket too, where no token is needed at all
    for path in ["/api/passwords/store", "/api/agent-connections", "/api/kwants"] {
        let (s, _) = loopback(&env, reqwest::Method::GET, path, Some(&token)).await;
        assert_eq!(s, 403, "{path}");
    }
    let (s, _) = loopback(&env, reqwest::Method::POST, "/api/chat/send", Some(&token)).await;
    assert_eq!(s, 403);
    // Nothing was sent
    assert!(messages(&env, &env.a).await.is_empty());
}

#[tokio::test]
async fn create_list_revoke_shows_the_token_once_and_stores_only_its_hash() {
    let env = setup().await;
    let (s, v) = call(
        &env.app,
        "POST",
        "/api/agent-connections",
        Some(json!({"name": "  Codex (laptop) ", "space_ids": ["spc_default"], "bots": [env.a]})),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED, "{v}");
    let token = v["token"].as_str().unwrap().to_string();
    let id = v["connection"]["id"].as_str().unwrap().to_string();
    assert!(token.starts_with("kag_") && token.len() == 4 + 43 && token[4..].chars().all(|c| c.is_ascii_alphanumeric()), "{token}");
    assert_eq!(v["connection"]["name"], "Codex (laptop)");
    assert_eq!(v["connection"]["bots"], json!([env.a]));
    assert_eq!(v["connection"]["space_ids"], json!(["spc_default"]));

    // The list never has the token (nor its hash)
    let (s, list) = call(&env.app, "GET", "/api/agent-connections", None).await;
    assert_eq!(s, StatusCode::OK);
    let text = list.to_string();
    assert!(!text.contains(&token) && !text.contains("token"), "{text}");
    assert_eq!(list["connections"][0]["id"], id.as_str());
    assert_eq!(list["connections"][0]["last_used_at"], Value::Null);

    // Stored: its sha256, and the token itself nowhere in the database files
    let row = env.state.db.get_agent_connection(&id).await.unwrap().unwrap();
    assert_eq!(row.token_sha256, kwnt::server::agents_connect::token_sha256(&token));
    let data = env.state.config.data_dir.clone();
    for entry in std::fs::read_dir(&data).unwrap() {
        let path = entry.unwrap().path();
        if path.is_file() {
            let bytes = std::fs::read(&path).unwrap();
            assert!(!bytes.windows(token.len()).any(|w| w == token.as_bytes()), "token found in {}", path.display());
        }
    }

    // Using it sets last_used_at
    let (s, _, _) = rpc(&env, &token, "tools/call", json!({"name": "list_bots", "arguments": {}})).await;
    assert_eq!(s, StatusCode::OK);
    let (_, list) = call(&env.app, "GET", "/api/agent-connections", None).await;
    assert!(list["connections"][0]["last_used_at"].is_string());

    // Bad input
    for bad in [
        json!({"name": "", "space_ids": ["spc_default"], "bots": "all"}),
        json!({"name": "x".repeat(41), "space_ids": ["spc_default"], "bots": "all"}),
        json!({"name": "a\nb", "space_ids": ["spc_default"], "bots": "all"}),
        json!({"name": "ok", "space_ids": [], "bots": "all"}),
        json!({"name": "ok", "space_ids": ["spc_nope"], "bots": "all"}),
        json!({"name": "ok", "space_ids": ["spc_default"], "bots": ["nope"]}),
        json!({"name": "ok", "space_ids": ["spc_default"], "bots": []}),
        json!({"name": "ok", "space_ids": ["spc_default"], "bots": "some"}),
    ] {
        let (s, v) = call(&env.app, "POST", "/api/agent-connections", Some(bad.clone())).await;
        assert_eq!(s, StatusCode::BAD_REQUEST, "{bad}: {v}");
    }

    // At most 10 live: the 11th is a 409 until one is revoked
    for i in 1..10 {
        connect(&env, &format!("agent {i}"), json!("all")).await;
    }
    let body = json!({"name": "eleventh", "space_ids": ["spc_default"], "bots": "all"});
    let (s, _) = call(&env.app, "POST", "/api/agent-connections", Some(body.clone())).await;
    assert_eq!(s, StatusCode::CONFLICT);
    let (s, _) = call(&env.app, "DELETE", &format!("/api/agent-connections/{id}"), None).await;
    assert_eq!(s, StatusCode::OK);
    let (_, list) = call(&env.app, "GET", "/api/agent-connections", None).await;
    assert_eq!(list["connections"].as_array().unwrap().len(), 9);
    let (s, _) = call(&env.app, "POST", "/api/agent-connections", Some(body)).await;
    assert_eq!(s, StatusCode::CREATED);

    // From another machine the routes are person-only: no API token, no access
    assert_eq!(call_remote(&env.app, "GET", "/api/agent-connections").await, StatusCode::UNAUTHORIZED);
    assert_eq!(call_remote(&env.app, "POST", "/api/agent-connections").await, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn ask_round_trip_with_the_mock_engine() {
    let env = setup().await;
    let (conn_id, token) = connect(&env, "Claude Code", json!([env.a])).await;

    let (s, _, v) = rpc(&env, &token, "tools/list", json!({})).await;
    assert_eq!(s, StatusCode::OK);
    let names: Vec<&str> = v["result"]["tools"].as_array().unwrap().iter().map(|t| t["name"].as_str().unwrap()).collect();
    assert_eq!(names, ["list_bots", "ask", "get_reply", "list_threads"]);

    // Only the bots in scope
    let (s, err, v) = tool(&env, &token, "list_bots", json!({})).await;
    assert_eq!((s, err), (StatusCode::OK, false), "{v}");
    let bots = v["bots"].as_array().unwrap();
    assert_eq!(bots.len(), 1, "{v}");
    assert_eq!(bots[0]["id"], env.a.as_str());
    assert_eq!(bots[0]["state"], "idle");
    for key in ["name", "role", "team", "space"] {
        assert!(bots[0].get(key).is_some(), "{key}");
    }

    // ask: the reply comes back
    let (s, err, r) = tool(&env, &token, "ask", json!({"bot": env.a, "message": "Ship it REPLYTAG:ag1", "wait_seconds": 60})).await;
    assert_eq!((s, err), (StatusCode::OK, false), "{r}");
    assert_eq!(r["status"], "done", "{r}");
    assert_eq!(r["reply_text"], "answer ag1");
    let thread_id = r["thread_id"].as_str().unwrap().to_string();
    let turn_id = r["turn_id"].as_str().unwrap().to_string();
    assert_eq!(r["attachments"], json!([]));

    // Stored in a thread named after the connection, as the agent's, with from_agent
    let thread = env.state.db.get_thread(&thread_id).await.unwrap().unwrap();
    assert_eq!((thread.title.as_str(), thread.kwant_id.as_str()), ("From Claude Code", env.a.as_str()));
    let msgs = messages(&env, &thread_id).await;
    let asked = msgs.iter().find(|m| m.sender_role == "user").unwrap();
    let meta: Value = serde_json::from_str(asked.meta.as_deref().unwrap()).unwrap();
    assert_eq!(meta["from_agent"], "Claude Code");
    assert_eq!(meta["agent_connection_id"], conn_id.as_str());
    assert_eq!(asked.sender_id, format!("agent:{conn_id}"));
    // The prompt carries the line, and the bot reads it framed when it is history
    let prompt = invocations_with("Ship it REPLYTAG:ag1");
    assert_eq!(prompt.len(), 1);
    assert!(prompt[0].contains(NOTE), "{}", prompt[0]);

    // The same standing thread next time; the earlier message is framed as the agent's
    let (_, _, r2) = tool(&env, &token, "ask", json!({"bot": "Architect", "message": "Again REPLYTAG:ag2", "wait_seconds": 60})).await;
    let r2 = if r2["status"].is_null() { panic!("{r2}") } else { r2 };
    assert_eq!(r2["status"], "done", "{r2}");
    assert_eq!(r2["thread_id"], thread_id.as_str());
    assert_eq!(r2["reply_text"], "answer ag2");
    let second = invocations_with("Again REPLYTAG:ag2");
    assert!(second[0].contains("[Message from outside agent Claude Code (connected by the person, not the person)"), "{}", second[0]);
    assert!(!second[0].contains("User: Ship it"), "{}", second[0]);

    // "new" opens another thread; the reply is redacted
    use base64::Engine;
    let reply = base64::engine::general_purpose::STANDARD.encode("Done. password=hunter2 is in /home/someone/x.txt");
    let (_, _, r3) = tool(&env, &token, "ask", json!({"bot": env.a, "message": format!("Fresh REPLYTEXT:{reply}"), "thread": "new", "wait_seconds": 60})).await;
    assert_eq!(r3["status"], "done", "{r3}");
    assert_ne!(r3["thread_id"], thread_id.as_str());
    let text = r3["reply_text"].as_str().unwrap();
    assert!(text.starts_with("Done.") && !text.contains("hunter2") && !text.contains("someone"), "{text}");

    // get_reply: this connection's own turns only
    let (_, err, g) = tool(&env, &token, "get_reply", json!({"turn_id": turn_id})).await;
    assert!(!err);
    assert_eq!((g["status"].as_str(), g["thread_id"].as_str()), (Some("done"), Some(thread_id.as_str())));
    let (_, other) = connect(&env, "Cursor", json!("all")).await;
    let (_, err, _) = tool(&env, &other, "get_reply", json!({"turn_id": turn_id})).await;
    assert!(err, "another connection's turn");

    // A bot outside the scope is refused, by id and by name
    let (s, err, v) = tool(&env, &token, "ask", json!({"bot": env.b, "message": "hi REPLYTAG:no1"})).await;
    assert_eq!((s, err), (StatusCode::OK, true), "{v}");
    assert!(v.as_str().unwrap().contains("can't ask that bot"));
    let (_, err, _) = tool(&env, &token, "ask", json!({"bot": "TechLead", "message": "hi REPLYTAG:no2"})).await;
    assert!(err);

    // Someone else's thread is refused: another connection's, and the person's own
    let (_, _, theirs) = tool(&env, &other, "ask", json!({"bot": env.a, "message": "Mine REPLYTAG:ag4", "wait_seconds": 60})).await;
    assert_eq!(theirs["status"], "done", "{theirs}");
    let (_, err, v) = tool(&env, &token, "ask", json!({"bot": env.a, "message": "hijack REPLYTAG:no3", "thread": theirs["thread_id"]})).await;
    assert!(err, "{v}");
    assert!(v.as_str().unwrap().contains("only use threads it started"));
    let person_thread = env.state.db.create_thread(&env.a, "Mine", "chat", None, "user").await.unwrap();
    let (_, err, _) = tool(&env, &token, "ask", json!({"bot": env.a, "message": "hijack REPLYTAG:no4", "thread": person_thread.id})).await;
    assert!(err);
    assert!(invocations_with("hijack").is_empty() && invocations_with("REPLYTAG:no1").is_empty(), "no refused ask ran");
    // Bad input
    for bad in [
        json!({"bot": env.a, "message": ""}),
        json!({"bot": env.a, "message": "x".repeat(8001)}),
        json!({"bot": env.a, "message": "hi", "wait_seconds": 301}),
        json!({"message": "hi"}),
    ] {
        let (_, err, _) = tool(&env, &token, "ask", bad.clone()).await;
        assert!(err, "{bad}");
    }

    // list_threads: this connection's, newest first, last reply cut to 500
    let (_, err, l) = tool(&env, &token, "list_threads", json!({})).await;
    assert!(!err, "{l}");
    let threads = l["threads"].as_array().unwrap();
    assert_eq!(threads.len(), 2, "{l}");
    assert!(threads.iter().all(|t| t["thread_id"] != theirs["thread_id"]));
    assert_eq!(threads[0]["thread_id"], r3["thread_id"]);
    let (_, _, l) = tool(&env, &token, "list_threads", json!({"bot": env.a})).await;
    assert_eq!(l["threads"].as_array().unwrap().len(), 2);

    // One activity row per call, ok or refused; never the token
    let (s, act) = call(&env.app, "GET", &format!("/api/agent-connections/{conn_id}/activity"), None).await;
    assert_eq!(s, StatusCode::OK);
    let calls = act["calls"].as_array().unwrap();
    assert!(calls.iter().any(|c| c["tool"] == "ask" && c["outcome"] == "ok" && c["bot"] == "Architect"), "{act}");
    assert!(calls.iter().any(|c| c["tool"] == "ask" && c["outcome"] == "refused"), "{act}");
    assert!(calls.iter().all(|c| c["connection_name"] == "Claude Code"));
    assert!(!act.to_string().contains(&token));
}

#[tokio::test]
async fn an_approval_card_or_a_question_returns_waiting_on_person() {
    let env = setup().await;
    let (_, token) = connect(&env, "Claude Code", json!("all")).await;
    let marker = call_tool_marker(
        "ask_user",
        json!({"question": "Deploy to prod now?", "choices": ["yes", "no"], "why_you": "irreversible", "checked": "Read the plan."}),
    );
    let (s, err, r) = tool(&env, &token, "ask", json!({"bot": env.a, "message": format!("Deploy {marker} REPLYTAG:w1"), "wait_seconds": 60})).await;
    assert_eq!((s, err), (StatusCode::OK, false), "{r}");
    assert_eq!(r["status"], "waiting_on_person", "{r}");
    assert_eq!(r["reply_text"], "The bot asked the person: Deploy to prod now?");
    let thread_id = r["thread_id"].as_str().unwrap().to_string();
    let turn_id = r["turn_id"].as_str().unwrap().to_string();

    // An approval card for the same running turn: the agent sees one line, never the card
    let running = env.state.turns.active_turns().into_iter().find(|t| t.thread_id.as_deref() == Some(thread_id.as_str())).expect("running");
    let approver = Arc::new(BrokerApprover {
        db: env.state.db.clone(),
        data_dir: env.state.config.data_dir.clone(),
        broker: env.state.broker.clone(),
        events: env.state.event_sender.clone(),
        timeout: Duration::from_secs(60),
        conversation: thread_id.clone(),
        chat: Some(json!({"thread_id": thread_id})),
        always_ask: false,
    });
    let req = ToolApprovalRequest {
        kwant_id: env.a.clone(),
        kwant_name: "Architect".into(),
        turn_id: running.turn_id.clone(),
        tool_name: "Bash".into(),
        server: String::new(),
        tool: "Bash".into(),
        arguments: json!({"command": "rm -rf SECRET_ARG_MARK", "description": "clean"}),
        builtin: None,
        smart: None,
        payment: None,
    };
    let waiter = tokio::spawn(async move { approver.decide(req).await });
    let mut seen = Value::Null;
    for _ in 0..100 {
        let (_, _, g) = tool(&env, &token, "get_reply", json!({"turn_id": turn_id})).await;
        if g["reply_text"].as_str().is_some_and(|t| t.contains("approve")) {
            seen = g;
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert_eq!(seen["status"], "waiting_on_person", "{seen}");
    assert_eq!(seen["reply_text"], "Waiting for the person to approve a step (Bash).");
    assert!(!seen.to_string().contains("SECRET_ARG_MARK") && !seen.to_string().contains("ticket"), "{seen}");

    // The person answers; the turn finishes and the agent gets the reply
    let pending = env.state.db.list_approvals(&kwnt::db::spaces::SpaceId::default_space(), Some("pending"), Some(&env.a)).await.unwrap();
    let ticket = pending.first().expect("a pending card").id.clone();
    let (s, _) = call(&env.app, "POST", &format!("/api/approvals/{ticket}/resolve"), Some(json!({"decision": "deny"}))).await;
    assert_eq!(s, StatusCode::OK);
    let _ = waiter.await;
    let (_, list) = call(&env.app, "GET", &format!("/api/questions?status=open&context_id={thread_id}"), None).await;
    let qid = list[0]["id"].as_str().expect("open question").to_string();
    let (s, _) = call(&env.app, "POST", &format!("/api/questions/{qid}/answer"), Some(json!({"choice": 1}))).await;
    assert_eq!(s, StatusCode::OK);
    let mut done = Value::Null;
    for _ in 0..200 {
        let (_, _, g) = tool(&env, &token, "get_reply", json!({"turn_id": turn_id})).await;
        if g["status"] == "done" {
            done = g;
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert!(done["reply_text"].as_str().is_some_and(|t| t.contains("answer w1")), "{done}");
}

#[tokio::test]
async fn sixty_asks_an_hour_then_429_with_retry_after() {
    let env = setup().await;
    let (conn_id, token) = connect(&env, "Claude Code", json!("all")).await;
    for _ in 0..60 {
        let t = env.state.db.reserve_agent_turn(&conn_id, &env.a, "thr_seed").await.unwrap().expect("reserved");
        env.state.db.finish_agent_turn(&t.id, "done", None, None).await.unwrap();
    }
    let (s, h, v) = rpc(&env, &token, "tools/call", json!({"name": "ask", "arguments": {"bot": env.a, "message": "one more REPLYTAG:rl"}})).await;
    assert_eq!(s, StatusCode::TOO_MANY_REQUESTS, "{v}");
    let retry = v["error"]["data"]["retry_after"].as_i64().unwrap();
    assert!((3500..=3601).contains(&retry), "{retry}");
    assert_eq!(h.get("retry-after").unwrap().to_str().unwrap(), retry.to_string());
    assert!(invocations_with("REPLYTAG:rl").is_empty());
    // Refused, in the activity trail
    let (_, act) = call(&env.app, "GET", &format!("/api/agent-connections/{conn_id}/activity"), None).await;
    assert_eq!(act["calls"][0]["outcome"], "refused");
    // Another connection has its own hour
    let (_, other) = connect(&env, "Codex", json!("all")).await;
    let (s, _, r) = tool(&env, &other, "ask", json!({"bot": env.a, "message": "fine REPLYTAG:rl2", "wait_seconds": 60})).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(r["status"], "done", "{r}");
}

#[tokio::test]
async fn three_turns_in_flight_then_429() {
    let env = setup().await;
    let (_, token) = connect(&env, "Claude Code", json!("all")).await;
    let mut turns = Vec::new();
    for i in 0..3 {
        let marker = call_tool_marker(
            "ask_user",
            json!({"question": format!("Hold {i}?"), "choices": ["yes", "no"], "why_you": "irreversible", "checked": "x"}),
        );
        let (s, err, r) = tool(&env, &token, "ask", json!({"bot": env.a, "message": format!("Hold {marker} REPLYTAG:if{i}"), "thread": "new", "wait_seconds": 0})).await;
        assert_eq!((s, err), (StatusCode::OK, false), "{r}");
        assert!(["working", "waiting_on_person"].contains(&r["status"].as_str().unwrap()), "{r}");
        turns.push(r["turn_id"].as_str().unwrap().to_string());
    }
    let (s, h, v) = rpc(&env, &token, "tools/call", json!({"name": "ask", "arguments": {"bot": env.a, "message": "fourth REPLYTAG:if4", "thread": "new"}})).await;
    assert_eq!(s, StatusCode::TOO_MANY_REQUESTS, "{v}");
    assert!(v["error"]["data"]["retry_after"].as_i64().unwrap() > 0);
    assert!(h.get("retry-after").is_some());
    assert!(invocations_with("REPLYTAG:if4").is_empty());
}
