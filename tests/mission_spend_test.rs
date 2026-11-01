//! Mission Control spend through the real turn path: the mission runner starts a turn on a fake
//! claude CLI, which streams its usage so far, writes the task's outcome and calls `task_update
//! review` through the turn's own MCP server (so the claim is released and the check verifies the
//! task inside the turn), and only then ends with its final usage. The usage known at hand-in must
//! be on the task the moment it is Verified, the turn must add up to its final usage exactly (no
//! double count), and the mission budget must use that number. Its own test binary: it changes
//! PATH and listens on a real port the CLI calls back on.

#[path = "common/exe.rs"]
mod exe;

use axum::{
    body::{to_bytes, Body},
    http::{Request, StatusCode},
    Router,
};
use kwnt::{
    broker::PermissionBroker,
    config::Config,
    db::Database,
    router::LayaRouter,
    server::{create_router, AppState},
    tools::ToolRunner,
};
use serde_json::{json, Value};
use std::path::PathBuf;
use std::sync::OnceLock;
use std::time::Duration;
use tower::ServiceExt;

/// On a Mission Control prompt: streams one model call's usage so far (12 new input, 8,211
/// cache-write, 83,544 cache-read, 300 output: 8,523 fresh), writes notes.md, moves the task it
/// holds to review through the kwant tools in its `--mcp-config`, works on for a while, then
/// reports the turn's final usage: 487 output, so 8,710 fresh.
const FAKE_CLAUDE: &str = r#"#!/usr/bin/env python3
import json, re, sys, time, urllib.request
args = sys.argv[1:]
if args[:1] == ["auth"]:
    print('{"loggedIn":true}')
    sys.exit(0)
prompt = args[-1] if args else ""
held = re.search(r"You hold task (\S+) of", prompt)
if held and "--mcp-config" in args:
    start = {"type": "message_start", "message": {"id": "msg_1", "usage": {"input_tokens": 12,
             "cache_creation_input_tokens": 8211, "cache_read_input_tokens": 83544, "output_tokens": 1}}}
    print(json.dumps({"type": "stream_event", "event": start}), flush=True)
    print(json.dumps({"type": "stream_event", "event": {"type": "message_delta", "usage": {"output_tokens": 300}}}), flush=True)
    time.sleep(0.5)
    with open("notes.md", "w") as f:
        f.write("done")
    with open(args[args.index("--mcp-config") + 1]) as f:
        servers = json.load(f).get("mcpServers", {})
    for s in servers.values():
        if s.get("url", "").endswith("/api/mcp/self"):
            call = {"jsonrpc": "2.0", "id": 1, "method": "tools/call",
                    "params": {"name": "task_update", "arguments": {"task_id": held.group(1), "status": "review"}}}
            headers = dict(s.get("headers", {}), **{"content-type": "application/json"})
            req = urllib.request.Request(s["url"], data=json.dumps(call).encode(), headers=headers)
            urllib.request.urlopen(req, timeout=30).read()
    time.sleep(3)
print(json.dumps({"type": "result", "is_error": False, "num_turns": 1, "result": "done, sent for review",
                  "total_cost_usd": 0.42,
                  "usage": {"input_tokens": 12, "cache_creation_input_tokens": 8211,
                            "cache_read_input_tokens": 83544, "output_tokens": 487},
                  "modelUsage": {"claude-opus-5-5": {"outputTokens": 487}}}))
"#;

/// Fresh tokens the CLI reported before it handed the task in.
const FRESH_AT_HAND_IN: i64 = 12 + 8_211 + 300;
const FRESH_PER_TURN: i64 = 12 + 8_211 + 487;

fn fake_cli() {
    static DONE: OnceLock<PathBuf> = OnceLock::new();
    DONE.get_or_init(|| {
        let bin = tempfile::tempdir().unwrap().keep().join("bin");
        std::fs::create_dir_all(&bin).unwrap();
        let fake = bin.join("claude");
        exe::write_executable(&fake, FAKE_CLAUDE);
        std::env::set_var("PATH", format!("{}:{}", bin.display(), std::env::var("PATH").unwrap_or_default()));
        bin
    });
}

async fn call(app: &Router, method: &str, uri: &str, body: Option<Value>) -> (StatusCode, Value) {
    let req = Request::builder().method(method).uri(uri).header("content-type", "application/json");
    let body = body.map(|v| Body::from(v.to_string())).unwrap_or_else(Body::empty);
    let resp = app.clone().oneshot(req.body(body).unwrap()).await.unwrap();
    let s = resp.status();
    let bytes = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    (s, serde_json::from_slice(&bytes).unwrap_or(Value::Null))
}

async fn ok(app: &Router, method: &str, uri: &str, body: Option<Value>) -> Value {
    let (s, v) = call(app, method, uri, body).await;
    assert!(s.is_success(), "{method} {uri}: {s} {v}");
    v
}

#[tokio::test]
async fn a_task_verified_in_turn_pays_at_hand_in_and_the_turn_counts_once() {
    fake_cli();
    let tmp = tempfile::tempdir().unwrap();
    // The CLI reaches its kwant tools over HTTP, so kwant listens for real
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let db = Database::init(tmp.path()).await.unwrap();
    let config = Config {
        data_dir: tmp.path().to_path_buf(),
        host: "127.0.0.1".into(),
        port,
        laya_url: "http://127.0.0.1:9".into(),
        browser_cdp_url: Some("http://127.0.0.1:9".into()),
        ..Config::default()
    };
    let state = AppState::new(
        config.clone(),
        db,
        ToolRunner::new(tmp.path().join("workspace")),
        PermissionBroker::new(),
        LayaRouter::new(config.laya_url.clone()),
    );
    let app = create_router(state.clone());
    let served = app.clone();
    tokio::spawn(async move { axum::serve(listener, served).await.unwrap() });

    ok(&app, "POST", "/api/settings", Some(json!({"mission_board": true}))).await;
    let bot = json!({"name": "Ann", "title": "Ann the tester", "persona_prompt": "Test persona",
                     "avatar_sprite": "s", "driver_type": "cli", "preferred_model": "claude"});
    let ann = ok(&app, "POST", "/api/kwants", Some(bot)).await["id"].as_str().unwrap().to_string();
    let channel = ok(&app, "POST", "/api/channels", Some(json!({"name": "desk", "lead_kwant_id": ann, "member_kwants": []}))).await;
    let channel = channel["id"].as_str().unwrap().to_string();
    let body = json!({"title": "Ship it", "goal": "a release", "lead_kwant_id": ann, "budget_tokens": 10_000, "channel_id": channel,
                      "tasks": [{"title": "A", "owner": ann, "budget_tokens": 9_000,
                                 "checks": [{"kind": "file_contains", "spec": {"path": "notes.md", "text": "done"}}]},
                                {"title": "B", "owner": ann, "budget_tokens": 2_000}]});
    let m = ok(&app, "POST", "/api/missions", Some(body)).await;
    let mid = m["id"].as_str().unwrap().to_string();
    let detail = ok(&app, "GET", &format!("/api/missions/{mid}"), None).await;
    let by_title = |t: &str| detail["tasks"].as_array().unwrap().iter().find(|x| x["title"] == t).unwrap()["id"].as_str().unwrap().to_string();
    let (a, b) = (by_title("A"), by_title("B"));

    let run = ok(&app, "POST", &format!("/api/missions/{mid}/run"), Some(json!({"max_tasks": 1}))).await;
    let started = run["started"][0]["task_id"].as_str().unwrap().to_string();
    let other = if started == a { b.clone() } else { a.clone() };
    assert_eq!(started, a, "the runner starts the first task: {run}");

    // The turn hands A in and its check verifies it, while the turn is still running
    let mut t = Value::Null;
    for _ in 0..300 {
        t = ok(&app, "GET", &format!("/api/tasks/{a}"), None).await;
        if t["task"]["state"] == "verified" {
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert_eq!(t["task"]["state"], "verified", "the bot handed it in and its check passed inside the turn: {t}");
    assert!(t["task"]["claim"].is_null(), "review released the claim: {t}");
    assert_eq!(t["task"]["spent_tokens"], FRESH_AT_HAND_IN, "the usage known at hand-in is on the task the moment it is Verified: {t}");
    let detail = ok(&app, "GET", &format!("/api/missions/{mid}"), None).await;
    assert_eq!(detail["mission"]["spent_tokens"], FRESH_AT_HAND_IN, "…and on the mission: {detail}");

    // At turn end only the rest is added: the turn counts exactly once
    for _ in 0..300 {
        t = ok(&app, "GET", &format!("/api/tasks/{a}"), None).await;
        if t["task"]["spent_tokens"] != FRESH_AT_HAND_IN {
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    assert_eq!(t["task"]["spent_tokens"], FRESH_PER_TURN, "the turn's final usage, not hand-in plus final: {t}");
    let released = t["events"].as_array().unwrap().iter().any(|ev| ev["to_state"] == "review");
    assert!(released, "{t}");
    let detail = ok(&app, "GET", &format!("/api/missions/{mid}"), None).await;
    assert_eq!(detail["mission"]["spent_tokens"], FRESH_PER_TURN, "…and on the mission: {detail}");

    // The budget uses that real number: 10,000 − 8,710 can't cover B's 2,000
    let lease = 60_000;
    let refused = state.db.claim_task(&other, &ann, None, None, lease, false, &ann).await.unwrap();
    match refused {
        Err(kwnt::db::missions::ClaimRefused::Budget { remaining, needed }) => {
            assert_eq!((remaining, needed), (10_000 - FRESH_PER_TURN, 2_000));
        }
        other => panic!("B must not start past the mission budget: {other:?}"),
    }
}
