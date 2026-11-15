//! #715: a bot on an API engine reads its own folder (`bot-workspaces/<kwant id>`) with
//! `read_file`/`list_dir`; CLI engines are offered nothing new. End to end: a Custom API account
//! against a fake OpenAI-compatible server that calls `read_file` on notes.md and answers with the
//! line count it read. Then, at kwant's tool endpoint: a teammate's folder, `..`, absolute paths and
//! symlinks are refused; a bot with no space is told so; a CLI turn's list has no folder tools and a
//! call to one is refused; a customer's turn still asks before a read.

mod common;

use axum::{
    extract::State,
    http::{HeaderMap, StatusCode},
    response::IntoResponse,
    routing::{get, post},
    Json, Router,
};
use common::*;
use kwnt::drivers::RuntimeEvent;
use kwnt::server::bot_files::OUTSIDE;
use kwnt::server::continuity::{SelfScope, TurnOrigin};
use kwnt::server::mcp_runtime;
use serde_json::{json, Value};
use std::path::PathBuf;
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex};
use std::time::Duration;

const KEY: &str = "sk-custom-files-0123456789";
const LINES: usize = 7_160;

/// Every chat request the fake model got.
type Seen = Arc<Mutex<Vec<Value>>>;

/// A fake OpenAI-compatible server: lists one model; a request with `read_file` offered and no
/// tool result yet calls it on notes.md; one with a tool result answers with that result's first
/// line; anything else says no file tool was offered.
async fn fake_model() -> (String, Seen) {
    let seen: Seen = Arc::default();
    let app = Router::new()
        .route("/v1/models", get(|| async { Json(json!({"data": [{"id": "gpt-files"}]})) }))
        .route(
            "/v1/chat/completions",
            post(|State(seen): State<Seen>, h: HeaderMap, Json(body): Json<Value>| async move {
                seen.lock().unwrap().push(body.clone());
                if h.get("authorization").and_then(|v| v.to_str().ok()) != Some(&format!("Bearer {KEY}")) {
                    return (StatusCode::UNAUTHORIZED, "bad key").into_response();
                }
                let reply = |content: String| Json(json!({"choices": [{"message": {"role": "assistant", "content": content}}], "usage": {"total_tokens": 9}})).into_response();
                let messages = body["messages"].as_array().cloned().unwrap_or_default();
                if let Some(result) = messages.iter().rev().find(|m| m["role"] == "tool") {
                    let text = result["content"].as_str().unwrap_or("").lines().next().unwrap_or("").to_string();
                    return reply(format!("I read it: {text}"));
                }
                let offered = body["tools"].as_array().into_iter().flatten().any(|t| t["function"]["name"] == "mcp__kwnt_tools__read_file");
                if !offered || body["stream"] == true {
                    return reply("NO FILE TOOL".into());
                }
                Json(json!({
                    "choices": [{"message": {"role": "assistant", "content": "", "tool_calls": [{
                        "id": "call_read", "type": "function",
                        "function": {"name": "mcp__kwnt_tools__read_file", "arguments": "{\"path\":\"notes.md\"}"},
                    }]}}],
                    "usage": {"total_tokens": 9},
                }))
                .into_response()
            }),
        )
        .with_state(seen.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let root = format!("http://{}", listener.local_addr().unwrap());
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    (root, seen)
}

/// A bot's own folder, as kwant makes it.
async fn folder_of(env: &Env, kwant: &str) -> PathBuf {
    let space = env.state.db.space_of_kwant(kwant).await.unwrap().unwrap();
    kwnt::tools::bot_workspace(&kwnt::db::spaces::space_data_dir(&env.state.config.data_dir, &space), kwant)
}

/// The bot's last saved reply.
async fn last_reply(env: &Env, kwant: &str) -> String {
    let saved = env.state.db.list_messages(kwant, 10).await.unwrap();
    saved.iter().rev().find(|m| m.sender_role == "kwant").map(|m| m.text.clone()).unwrap_or_default()
}

/// Sends `prompt` to `kwant` and waits for its turn to end.
async fn turn(env: &Env, kwant: &str, prompt: &str) {
    let mut rx = env.state.event_sender.subscribe();
    send_direct(env, kwant, prompt, json!({})).await;
    tokio::time::timeout(Duration::from_secs(90), async {
        while let Ok(e) = rx.recv().await {
            if matches!(e.unscoped(), RuntimeEvent::TurnEnded { kwant_id: k, .. } if k == kwant) {
                break;
            }
        }
    })
    .await
    .expect("turn did not end");
}

#[tokio::test]
async fn a_custom_api_bot_reads_notes_md_in_its_folder_and_gets_the_line_count() {
    let env = setup().await;
    let (root, seen) = fake_model().await;
    let (s, space) = call(&env.app, "POST", "/api/spaces", Some(json!({"name": "API files", "icon": "🧪", "this_machine_enabled": false}))).await;
    assert_eq!(s, StatusCode::CREATED, "{space}");
    let space = space["id"].as_str().unwrap().to_string();
    let (s, acct) = call(
        &env.app,
        "POST",
        &format!("/api/profiles?space={space}"),
        Some(json!({"engine": "custom", "kind": "custom_api", "name": "Fake OpenAI", "protocol": "openai-chat", "base_url": format!("{root}/v1"), "api_key": KEY, "model": "gpt-files"})),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED, "{acct}");
    let (s, bot) = call(
        &env.app,
        "POST",
        &format!("/api/kwants?space={space}"),
        Some(json!({"name": "Reader", "title": "Tester", "persona_prompt": "", "avatar_sprite": "s", "driver_type": "api", "preferred_model": "custom"})),
    )
    .await;
    assert!(s.is_success(), "{s} {bot}");
    let kid = bot["id"].as_str().unwrap().to_string();
    let folder = folder_of(&env, &kid).await;
    let notes: String = (1..=LINES).map(|i| format!("note {i}\n")).collect();
    std::fs::write(folder.join("notes.md"), notes).unwrap();

    turn(&env, &kid, "read the file notes.md from your workspace and tell me how many lines it has").await;

    let reply = last_reply(&env, &kid).await;
    assert!(reply.starts_with("I read it:") && reply.contains(&format!("notes.md: {LINES} lines total. Lines 1–600:")), "{reply}");
    let seen = seen.lock().unwrap();
    let first = seen.iter().find(|b| b["tools"].is_array()).expect("a request with tools");
    let names: Vec<&str> = first["tools"].as_array().unwrap().iter().filter_map(|t| t["function"]["name"].as_str()).collect();
    for tool in ["mcp__kwnt_tools__read_file", "mcp__kwnt_tools__list_dir"] {
        assert_eq!(names.iter().filter(|n| **n == tool).count(), 1, "{tool} offered once, up front: {names:?}");
    }
    assert!(!names.iter().any(|n| n.ends_with("write_file")), "read-only: {names:?}");
    // The page went back to the model as the tool's result, with the file's own words
    let result = seen.iter().flat_map(|b| b["messages"].as_array().cloned().unwrap_or_default()).find(|m| m["role"] == "tool").expect("a tool result");
    let text = result["content"].as_str().unwrap();
    assert!(text.contains("note 1\n") && text.contains("note 600\n") && !text.contains("note 601\n"), "{}", &text[..200.min(text.len())]);
}

/// A live turn for `scope`'s bot opened the way a chat turn opens it, on an API engine or not;
/// returns its token.
async fn open_turn(env: &Env, scope: SelfScope, api: bool, opts: mcp_runtime::TurnApprovalOpts) -> String {
    let k = env.state.db.get_kwant(&env.a).await.unwrap().unwrap();
    scope.folder_tools.store(api, Ordering::Relaxed);
    let turn = scope.turn_id.clone();
    mcp_runtime::prepare_turn_with_opts(&env.state, &k, turn, None, Some(scope), opts).await.unwrap().1
}

fn scope_of(kwant: &str, turn: &str) -> SelfScope {
    SelfScope {
        kwant_id: kwant.into(),
        kwant_name: "Bot".into(),
        origin: Some(TurnOrigin { direct_kwant_id: Some(kwant.into()), ..Default::default() }),
        turn_id: Some(turn.into()),
        ..Default::default()
    }
}

async fn rpc(env: &Env, token: &str, method: &str, params: Value) -> Value {
    let (s, v) = call_with(
        &env.app,
        "POST",
        "/api/mcp/self",
        Some(json!({"jsonrpc": "2.0", "id": 1, "method": method, "params": params})),
        &[("authorization", format!("Bearer {token}").as_str())],
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    v["result"].clone()
}

/// (text, isError) of a call to kwant's own `tool`.
async fn tool(env: &Env, token: &str, name: &str, args: Value) -> (String, bool) {
    let r = rpc(env, token, "tools/call", json!({"name": name, "arguments": args})).await;
    let text = r["content"].as_array().unwrap().iter().filter_map(|c| c["text"].as_str()).collect::<Vec<_>>().join("\n");
    (text, r["isError"].as_bool().unwrap())
}

async fn listed(env: &Env, token: &str) -> Vec<String> {
    let r = rpc(env, token, "tools/list", json!({})).await;
    r["tools"].as_array().unwrap().iter().filter_map(|t| t["name"].as_str().map(String::from)).collect()
}

#[tokio::test]
async fn the_endpoint_keeps_a_bot_inside_its_own_folder() {
    let env = setup().await;
    let mine = folder_of(&env, &env.a).await;
    let theirs = folder_of(&env, &env.b).await;
    assert_eq!(mine.parent(), theirs.parent(), "two bots of one space");
    std::fs::write(mine.join("notes.md"), "mine\n").unwrap();
    std::fs::write(theirs.join("secret.md"), "B ONLY\n").unwrap();
    let tok = open_turn(&env, scope_of(&env.a, "turn-files"), true, Default::default()).await;

    let (text, err) = tool(&env, &tok, "read_file", json!({"path": "notes.md"})).await;
    assert!(!err && text.contains("1 line total") && text.ends_with("mine\n"), "{text}");
    let (text, err) = tool(&env, &tok, "list_dir", json!({})).await;
    assert!(!err && text.contains("notes.md (5 bytes)"), "{text}");

    let b_dir = theirs.file_name().unwrap().to_str().unwrap().to_string();
    let mut bad = vec![
        format!("../{b_dir}/secret.md"),
        format!("../{b_dir}"),
        theirs.join("secret.md").to_string_lossy().into_owned(),
        "../../../../etc/passwd".into(),
        "/etc/passwd".into(),
    ];
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(&theirs, mine.join("teammate")).unwrap();
        std::os::unix::fs::symlink(theirs.join("secret.md"), mine.join("their.md")).unwrap();
        bad.extend(["teammate/secret.md".into(), "teammate".into(), "their.md".into()]);
    }
    for path in &bad {
        for name in ["read_file", "list_dir"] {
            let (text, err) = tool(&env, &tok, name, json!({"path": path})).await;
            assert!(err, "{name} {path}");
            assert_eq!(text, OUTSIDE, "{name} {path}");
        }
    }
    mcp_runtime::finish_turn(&env.state, &tok).await;

    // A bot whose space can't be found
    let ghost = open_turn(&env, scope_of("ghost-bot", "turn-ghost"), true, Default::default()).await;
    assert_eq!(tool(&env, &ghost, "read_file", json!({"path": "notes.md"})).await, ("I couldn't find your space.".into(), true));
    mcp_runtime::finish_turn(&env.state, &ghost).await;
}

#[tokio::test]
async fn cli_turns_get_no_folder_tools_and_a_call_to_one_is_refused() {
    let env = setup().await;
    std::fs::write(folder_of(&env, &env.a).await.join("notes.md"), "mine\n").unwrap();
    // At the endpoint: the list a CLI turn reads has neither tool, and a call is refused
    let cli = open_turn(&env, scope_of(&env.a, "turn-cli"), false, Default::default()).await;
    let names = listed(&env, &cli).await;
    assert!(!names.iter().any(|n| n == "read_file" || n == "list_dir"), "{names:?}");
    let (text, err) = tool(&env, &cli, "read_file", json!({"path": "notes.md"})).await;
    assert!(err && text.contains("isn't offered on this engine"), "{text}");
    mcp_runtime::finish_turn(&env.state, &cli).await;
    let api = open_turn(&env, scope_of(&env.a, "turn-api"), true, Default::default()).await;
    let names = listed(&env, &api).await;
    for t in ["read_file", "list_dir"] {
        assert_eq!(names.iter().filter(|n| *n == t).count(), 1, "{t}: {names:?}");
    }
    mcp_runtime::finish_turn(&env.state, &api).await;

    // A whole turn on the fake claude CLI: the call doesn't run
    let marker = call_tool_marker("read_file", json!({"path": "notes.md"}));
    send_direct(&env, &env.a, &format!("{marker} REPLYTAG:cli"), json!({})).await;
    let reply = wait_reply(&env, &env.a, "answer cli").await;
    assert!(reply.text.contains("isn't offered on this engine") && reply.text.contains("TOOLERROR"), "{}", reply.text);
    assert!(!reply.text.contains("mine"), "{}", reply.text);
}

#[tokio::test]
async fn reads_need_no_card_in_an_always_ask_turn_but_a_customer_turn_still_asks() {
    use kwnt::mcp::{ToolApprovalRequest, ToolApprover, SELF_SERVER_NAME};
    let env = setup().await;
    std::fs::write(folder_of(&env, &env.a).await.join("notes.md"), "mine\n").unwrap();
    // A turn kwnt started on its own asks for everything but kwant's own reads
    let approver = kwnt::server::continuity::SelfToolsApprover {
        inner: Arc::new(kwnt::mcp::FixedApprover(false)),
        always_ask: true,
        browser: None,
    };
    let req = |tool: &str| ToolApprovalRequest {
        kwant_id: env.a.clone(),
        kwant_name: "A".into(),
        turn_id: "t".into(),
        tool_name: format!("mcp__{SELF_SERVER_NAME}__{tool}"),
        server: SELF_SERVER_NAME.into(),
        tool: tool.into(),
        arguments: json!({"path": "notes.md"}),
        builtin: None,
        smart: None,
        payment: None,
    };
    assert!(approver.approve(req("read_file")).await);
    assert!(approver.approve(req("list_dir")).await);
    assert!(!approver.approve(req("memory_add")).await, "the approver still asks for a write");

    // A customer's message started this turn: kwant's endpoint runs no call the approver didn't allow
    let opts = mcp_runtime::TurnApprovalOpts { every_call: true, ..Default::default() };
    let customer = open_turn(&env, scope_of(&env.a, "turn-customer"), true, opts).await;
    let (text, err) = tool(&env, &customer, "read_file", json!({"path": "notes.md"})).await;
    assert!(err, "{text}");
    assert_eq!(text, mcp_runtime::EVERY_CALL_REFUSED);
    mcp_runtime::finish_turn(&env.state, &customer).await;
}
