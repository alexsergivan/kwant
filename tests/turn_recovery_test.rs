//! Replies that can't finish: a claude that goes silent is stopped by the stall watchdog with a
//! visible reason (but not while a tool runs, and never an API reply that is just slow), and
//! replies cut short by a restart are marked interrupted on the next start.

#[path = "common/exe.rs"]
mod exe;

use axum::{
    body::{to_bytes, Body},
    http::{Request, StatusCode},
    routing::{get, post},
    Json, Router,
};
use kwnt::{
    broker::PermissionBroker,
    config::Config,
    db::{continuity::RunningTurn, Database},
    drivers::RuntimeEvent,
    router::LayaRouter,
    server::{create_router, routes, AppState},
    tools::ToolRunner,
};
use serde_json::{json, Value};
use std::path::PathBuf;
use std::sync::OnceLock;
use std::time::Duration;
use tower::ServiceExt;

/// A claude CLI that prints `stream-json` like the real one; what it does depends on a word in
/// the prompt.
const FAKE_CLAUDE: &str = r#"#!/bin/sh
case "$1" in
  auth) echo '{"loggedIn":true}'; exit 0 ;;
esac
echo '{"type":"system","subtype":"init"}'
case "$*" in
  *GO_SILENT*) exec sleep 3600 ;;
  *LONG_TOOL*)
    echo '{"type":"assistant","message":{"content":[{"type":"tool_use","id":"tu_1","name":"Bash","input":{"command":"sleep 5"}}]}}'
    sleep 5
    echo '{"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"tu_1","content":"done"}]}}'
    echo '{"type":"result","is_error":false,"num_turns":2,"result":"finished after the tool"}' ;;
  *KEEP_TALKING*)
    for i in 1 2 3 4 5; do echo '{"type":"stream_event","event":{"type":"content_block_delta"}}'; sleep 1; done
    echo '{"type":"result","is_error":false,"num_turns":1,"result":"finished talking"}' ;;
  *) echo '{"type":"result","is_error":false,"num_turns":1,"result":"answer ok"}' ;;
esac
"#;

fn fake_claude() {
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
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method(method)
                .uri(uri)
                .header("content-type", "application/json")
                .body(body.map(|v| Body::from(v.to_string())).unwrap_or_else(Body::empty))
                .unwrap(),
        )
        .await
        .unwrap();
    let s = resp.status();
    let bytes = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    (s, serde_json::from_slice(&bytes).unwrap_or_else(|_| Value::String(String::from_utf8_lossy(&bytes).into())))
}

/// An OpenAI-compatible server that answers every chat request, but only after `after`.
async fn slow_server(after: Duration) -> String {
    let app = Router::new()
        .route("/v1/models", get(|| async { Json(json!({"object": "list", "data": [{"id": "gpt-4.1"}]})) }))
        .route(
            "/v1/chat/completions",
            post(move || async move {
                tokio::time::sleep(after).await;
                Json(json!({
                    "choices": [{"message": {"role": "assistant", "content": "slow but fine"}, "finish_reason": "stop"}],
                    "usage": {"prompt_tokens": 5, "completion_tokens": 3, "total_tokens": 8},
                }))
            }),
        );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let root = format!("http://{}", listener.local_addr().unwrap());
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    root
}

struct Env {
    app: Router,
    state: AppState,
    _tmp: tempfile::TempDir,
}

async fn setup() -> Env {
    fake_claude();
    let tmp = tempfile::tempdir().unwrap();
    let data = tmp.path().join("data");
    let db = Database::init(&data).await.unwrap();
    let config = Config {
        data_dir: data.clone(),
        laya_url: "http://127.0.0.1:9".into(),
        browser_cdp_url: Some("http://127.0.0.1:9".into()),
        ..Config::default()
    };
    let state = AppState::new(
        config.clone(),
        db,
        ToolRunner::new(data.join("workspace")),
        PermissionBroker::new(),
        LayaRouter::new(config.laya_url.clone()),
    );
    Env { app: create_router(state.clone()), state, _tmp: tmp }
}

impl Env {
    /// A bot on a custom API account that points at `root`, in its own space.
    async fn api_bot(&self, root: &str) -> String {
        let (s, v) = call(
            &self.app,
            "POST",
            "/api/spaces",
            Some(json!({"name": "Stalls", "icon": "🧪", "this_machine_enabled": false})),
        )
        .await;
        assert_eq!(s, StatusCode::CREATED, "{v}");
        let space = v["id"].as_str().unwrap().to_string();
        let (s, v) = call(
            &self.app,
            "POST",
            &format!("/api/profiles?space={space}"),
            Some(json!({
                "engine": "custom", "kind": "custom_api", "name": "Silent server",
                "protocol": "openai-chat", "base_url": format!("{root}/v1"),
                "api_key": "sk-test-0123456789", "model": "gpt-4.1",
            })),
        )
        .await;
        assert_eq!(s, StatusCode::CREATED, "{v}");
        let (s, v) = call(
            &self.app,
            "POST",
            &format!("/api/kwants?space={space}"),
            Some(json!({
                "name": "Quill", "title": "Tester", "persona_prompt": "Test persona",
                "avatar_sprite": "s", "driver_type": "api", "preferred_model": "custom",
            })),
        )
        .await;
        assert!(s.is_success(), "{s} {v}");
        v["id"].as_str().unwrap().to_string()
    }

    /// A bot on this machine's claude CLI (the fake one on PATH).
    async fn claude_bot(&self) -> String {
        let (s, v) = call(
            &self.app,
            "POST",
            "/api/kwants",
            Some(json!({
                "name": "Clio", "title": "Tester", "persona_prompt": "Test persona",
                "avatar_sprite": "s", "driver_type": "api", "preferred_model": "claude",
            })),
        )
        .await;
        assert!(s.is_success(), "{s} {v}");
        v["id"].as_str().unwrap().to_string()
    }

    async fn turn(&self, kwant_id: &str, within: Duration) -> Vec<RuntimeEvent> {
        self.turn_with(kwant_id, "hi", within).await
    }

    async fn turn_with(&self, kwant_id: &str, prompt: &str, within: Duration) -> Vec<RuntimeEvent> {
        let mut rx = self.state.event_sender.subscribe();
        let (s, v) = call(
            &self.app,
            "POST",
            "/api/chat/send",
            Some(json!({"kwant_id": kwant_id, "prompt": prompt, "sender_id": "user"})),
        )
        .await;
        assert_eq!(s, StatusCode::OK, "{v}");
        let mut events = Vec::new();
        tokio::time::timeout(within, async {
            while let Ok(e) = rx.recv().await {
                let end = matches!(&e, RuntimeEvent::TurnEnded { kwant_id: k, .. } if k == kwant_id);
                events.push(e.unscoped().clone());
                if end {
                    break;
                }
            }
        })
        .await
        .expect("the turn did not end");
        events
    }
}

fn shown(events: &[RuntimeEvent]) -> String {
    events
        .iter()
        .filter_map(|e| match e {
            RuntimeEvent::TextDelta(t) => Some(t.as_str()),
            _ => None,
        })
        .collect()
}

#[tokio::test]
async fn a_silent_claude_is_stopped_with_a_visible_reason() {
    let env = setup().await;
    env.state.db.set_setting(routes::STALL_SETTING, "2").await.unwrap();
    let kid = env.claude_bot().await;

    let started = std::time::Instant::now();
    let events = env.turn_with(&kid, "please GO_SILENT now", Duration::from_secs(30)).await;
    let waited = started.elapsed();
    assert!(waited >= Duration::from_secs(2), "stopped before the limit: {waited:?}");
    assert!(waited < Duration::from_secs(15), "the watchdog should stop it soon after 2 s: {waited:?}");
    assert!(shown(&events).contains("sent nothing for 2 s"), "{}", shown(&events));

    // The reason is kept in the chat, tagged for a Retry button, and the bot isn't left marked as replying
    let (s, msgs) = call(&env.app, "GET", &format!("/api/channels/{kid}/messages"), None).await;
    assert_eq!(s, StatusCode::OK, "{msgs}");
    let stopped = msgs
        .as_array()
        .unwrap()
        .iter()
        .find(|m| m["text"].as_str().is_some_and(|t| t.contains("sent nothing for 2 s")))
        .unwrap_or_else(|| panic!("{msgs}"));
    let meta: Value = serde_json::from_str(stopped["meta"].as_str().unwrap()).unwrap();
    assert_eq!(meta["kind"], "turn_stalled");
    assert_eq!(meta["stalled"], true);
    assert_eq!(meta["kwant_id"], kid.as_str());
    assert_eq!(meta["stall_secs"], 2);
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert!(env.state.db.take_running_turns().await.unwrap().is_empty(), "the running row is cleared");
}

#[tokio::test]
async fn the_watchdog_waits_while_a_tool_runs() {
    let env = setup().await;
    env.state.db.set_setting(routes::STALL_SETTING, "2").await.unwrap();
    let kid = env.claude_bot().await;
    // The tool runs 5 s with no output, far past the 2 s limit
    let events = env.turn_with(&kid, "run a LONG_TOOL please", Duration::from_secs(30)).await;
    let text = shown(&events);
    assert!(text.contains("finished after the tool"), "{text}");
    assert!(!text.contains("sent nothing"), "{text}");
    assert!(events.iter().any(|e| matches!(e, RuntimeEvent::ToolCallPending { id, .. } if id == "tu_1")), "{events:?}");
    assert!(events.iter().any(|e| matches!(e, RuntimeEvent::ToolCallCompleted { id, .. } if id == "tu_1")), "{events:?}");
    assert!(!events.iter().any(|e| matches!(e, RuntimeEvent::Progress)), "progress is never broadcast");
}

#[tokio::test]
async fn a_claude_still_writing_is_not_stopped() {
    let env = setup().await;
    env.state.db.set_setting(routes::STALL_SETTING, "2").await.unwrap();
    let kid = env.claude_bot().await;
    // 5 s in all, but never silent for 2 s
    let events = env.turn_with(&kid, "KEEP_TALKING for a while", Duration::from_secs(30)).await;
    assert!(shown(&events).contains("finished talking"), "{}", shown(&events));
}

#[tokio::test]
async fn a_slow_api_reply_is_not_stopped_as_a_stall() {
    // The API driver's model calls aren't streamed, so a slow healthy reply is silent throughout
    let env = setup().await;
    env.state.db.set_setting(routes::STALL_SETTING, "1").await.unwrap();
    let root = slow_server(Duration::from_secs(3)).await;
    let kid = env.api_bot(&root).await;
    let events = env.turn(&kid, Duration::from_secs(30)).await;
    let text = shown(&events);
    assert!(text.contains("slow but fine"), "{text}");
    assert!(!text.contains("sent nothing"), "{text}");
}

#[tokio::test]
async fn the_stall_limit_comes_from_settings_and_zero_turns_it_off() {
    let env = setup().await;
    env.state.db.set_setting(routes::STALL_SETTING, "0").await.unwrap();
    let kid = env.claude_bot().await;
    let mut rx = env.state.event_sender.subscribe();
    let (s, _) = call(
        &env.app,
        "POST",
        "/api/chat/send",
        Some(json!({"kwant_id": kid, "prompt": "GO_SILENT", "sender_id": "user"})),
    )
    .await;
    assert_eq!(s, StatusCode::OK);
    let ended = tokio::time::timeout(Duration::from_secs(4), async {
        while let Ok(e) = rx.recv().await {
            if matches!(&e, RuntimeEvent::TurnEnded { kwant_id: k, .. } if k == &kid) {
                return;
            }
        }
    })
    .await;
    assert!(ended.is_err(), "with the watchdog off a silent turn keeps waiting");
    // While it waits, the turn is recorded as running so a restart can mark it
    let running = env.state.db.take_running_turns().await.unwrap();
    assert_eq!(running.len(), 1, "{running:?}");
    assert_eq!(running[0].kwant_id, kid);
    let _ = call(&env.app, "POST", "/api/chat/cancel", Some(json!({"kwant_id": kid}))).await;
}

#[tokio::test]
async fn replies_cut_short_by_a_restart_are_marked_interrupted() {
    let env = setup().await;
    let (s, v) = call(
        &env.app,
        "POST",
        "/api/kwants",
        Some(json!({"name": "Ivy", "title": "Tester", "persona_prompt": "p", "avatar_sprite": "s", "driver_type": "api", "preferred_model": "gpt-4.1"})),
    )
    .await;
    assert!(s.is_success(), "{s} {v}");
    let kid = v["id"].as_str().unwrap().to_string();
    // What the previous run left behind when it stopped mid-reply
    env.state
        .db
        .add_running_turn(&RunningTurn {
            turn_id: "turn-1".into(),
            kwant_id: kid.clone(),
            kwant_name: "Ivy".into(),
            channel_id: None,
            direct_kwant_id: Some(kid.clone()),
            thread_id: None,
            started_at: chrono::Utc::now(),
        })
        .await
        .unwrap();

    assert_eq!(routes::interrupt_running_turns(&env.state).await, 1);
    // Only once: a second start finds nothing left to mark
    assert_eq!(routes::interrupt_running_turns(&env.state).await, 0);

    let (s, msgs) = call(&env.app, "GET", &format!("/api/channels/{kid}/messages"), None).await;
    assert_eq!(s, StatusCode::OK, "{msgs}");
    let marked: Vec<&Value> = msgs
        .as_array()
        .unwrap()
        .iter()
        .filter(|m| m["sender_role"] == "system")
        .collect();
    assert_eq!(marked.len(), 1, "{msgs}");
    assert_eq!(marked[0]["text"], "This reply from Ivy was interrupted by a restart. Send your message again to retry.");
    let meta: Value = serde_json::from_str(marked[0]["meta"].as_str().unwrap()).unwrap();
    assert_eq!(meta["kind"], "turn_interrupted");
    assert_eq!(meta["interrupted"], true);
    assert_eq!(meta["kwant_id"], kid.as_str());
    assert_eq!(meta["turn_id"], "turn-1");
}
