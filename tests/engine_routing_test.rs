//! Turns run only on engines that passed their check, a refused turn marks its engine at once,
//! and with nothing usable the turn says so instead of using another engine. A fake claude CLI
//! is the only engine on PATH. Its own test binary: it changes PATH, HOME and the process env.

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
    drivers::{engines::EngineId, engines::EngineStatus, RuntimeEvent},
    router::LayaRouter,
    server::{create_router, AppState},
    tools::ToolRunner,
};
use serde_json::{json, Value};
use std::path::Path;
use std::time::Duration;
use tower::ServiceExt;

/// Signed in; a turn fails as signed out while `$FAKE_DIR/refuse` exists. Every turn is logged.
const FAKE_CLAUDE: &str = r#"#!/bin/sh
case "$1" in
  --version) echo "7.0.1 (Claude Code)"; exit 0 ;;
  auth) echo '{"loggedIn":true,"authMethod":"claude.ai"}'; exit 0 ;;
esac
echo turn >> "$FAKE_DIR/turns.log"
if [ -f "$FAKE_DIR/refuse" ]; then
  echo '{"type":"result","is_error":true,"num_turns":1,"result":"Not logged in · Please run /login"}'
  exit 1
fi
echo '{"type":"result","is_error":false,"num_turns":1,"result":"hello from fake claude","usage":{"input_tokens":3,"output_tokens":4}}'
"#;

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
    (s, serde_json::from_slice(&bytes).unwrap_or(Value::Null))
}

/// Sends a prompt and returns the text the turn streamed.
async fn turn_text(app: &Router, state: &AppState, kwant_id: &str) -> String {
    let mut rx = state.event_sender.subscribe();
    let (s, v) = call(
        app,
        "POST",
        "/api/chat/send",
        Some(json!({"kwant_id": kwant_id, "prompt": "hi", "sender_id": "user"})),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    let mut text = String::new();
    tokio::time::timeout(Duration::from_secs(60), async {
        while let Ok(e) = rx.recv().await {
            match e.unscoped() {
                RuntimeEvent::TextDelta(t) => text.push_str(t),
                RuntimeEvent::TurnEnded { kwant_id: k, .. } if k == kwant_id => break,
                _ => {}
            }
        }
    })
    .await
    .expect("turn did not end");
    text
}

fn turns_run(dir: &Path) -> usize {
    std::fs::read_to_string(dir.join("turns.log")).map(|s| s.lines().count()).unwrap_or(0)
}

#[tokio::test]
async fn turns_skip_engines_that_cannot_run() {
    let tmp = tempfile::tempdir().unwrap();
    let bin = tmp.path().join("bin");
    std::fs::create_dir_all(&bin).unwrap();
    let fake = bin.join("claude");
    exe::write_executable(&fake, FAKE_CLAUDE);
    // Only the fake claude: no real CLIs from PATH or ~/.local/bin, no host keys
    std::env::set_var("PATH", format!("{}:/usr/bin:/bin", bin.display()));
    std::env::set_var("HOME", tmp.path());
    std::env::set_var("FAKE_DIR", tmp.path());
    for var in ["ANTHROPIC_API_KEY", "OPENAI_API_KEY", "XAI_API_KEY", "OLLAMA_MODEL", "CLAUDE_CONFIG_DIR"] {
        std::env::remove_var(var);
    }
    std::env::set_var("OLLAMA_URL", "http://127.0.0.1:9");

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
    let app = create_router(state.clone());

    // Before any check every engine is "checking"
    let (s, v) = call(&app, "GET", "/api/engines", None).await;
    assert_eq!(s, StatusCode::OK);
    let engines = v["engines"].as_array().unwrap();
    assert_eq!(engines.len(), 8, "7 engines + Custom API");
    assert!(engines.iter().all(|e| e["status"] == "checking" && e["checked_at"].is_null()));

    // 1. Re-check: only the fake claude is ready; the shape is the agreed contract
    let (s, v) = call(&app, "POST", "/api/engines/recheck", None).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    let by_id = |v: &Value, id: &str| v["engines"].as_array().unwrap().iter().find(|e| e["id"] == id).cloned().unwrap();
    let claude = by_id(&v, "claude");
    assert_eq!(claude["status"], "ready", "{claude}");
    assert_eq!(claude["name"], "Claude Code CLI");
    assert_eq!(claude["kind"], "cli");
    assert_eq!(claude["version"], "7.0.1");
    assert!(claude["checked_at"].as_str().unwrap().contains('T'));
    assert!(claude["models"].as_array().unwrap().iter().any(|m| m["id"] == "opus" && m["default"] == true && m["label"].is_string()));
    for (id, kind) in [("codex", "cli"), ("grok", "cli"), ("anthropic_api", "api"), ("openai_api", "api"), ("xai_api", "api"), ("ollama", "local")] {
        let e = by_id(&v, id);
        assert_eq!(e["status"], "not_installed", "{e}");
        assert_eq!(e["kind"], kind);
        assert!(e["detail"].is_string());
        assert_eq!(e["models"], json!([]));
    }
    // Custom API has no CLI: it runs on the space's accounts and says where to add one
    let custom = by_id(&v, "custom_api");
    assert_eq!((custom["name"].clone(), custom["kind"].clone(), custom["status"].clone()), (json!("Custom API"), json!("api"), json!("not_installed")));
    assert!(custom["detail"].as_str().unwrap().contains("Custom API accounts"), "{custom}");
    let (_, listed) = call(&app, "GET", "/api/engines", None).await;
    assert_eq!(listed, v);
    // The old Settings fields stay
    let (_, settings) = call(&app, "GET", "/api/settings", None).await;
    assert_eq!(settings["claude_cli_found"], true);
    assert_eq!(settings["codex_cli_found"], false);

    // 2. A turn pinned to claude runs on the ready CLI
    let (_, kwants) = call(&app, "GET", "/api/kwants", None).await;
    let k = kwants[0].clone();
    let kid = k["id"].as_str().unwrap().to_string();
    let mut body = k.clone();
    body.as_object_mut()
        .unwrap()
        .retain(|key, _| ["name", "title", "persona_prompt", "soul", "avatar_sprite"].contains(&key.as_str()));
    body["preferred_model"] = json!("claude");
    let (s, r) = call(&app, "PUT", &format!("/api/kwants/{kid}"), Some(body.clone())).await;
    assert_eq!(s, StatusCode::OK, "{r}");
    let text = turn_text(&app, &state, &kid).await;
    assert!(text.contains("hello from fake claude"), "{text}");
    assert_eq!(turns_run(tmp.path()), 1);

    // 3. Signed out mid-turn: the engine is marked at once, without a re-check
    std::fs::write(tmp.path().join("refuse"), "").unwrap();
    let text = turn_text(&app, &state, &kid).await;
    assert!(text.contains("signed out") || text.contains("refused"), "{text}");
    assert_eq!(turns_run(tmp.path()), 2);
    assert_eq!(state.engines.status(EngineId::Claude), EngineStatus::SignedOut);
    let (_, v) = call(&app, "GET", "/api/engines", None).await;
    assert_eq!(by_id(&v, "claude")["status"], "signed_out");
    assert_eq!(by_id(&v, "claude")["models"], json!([]));

    // 4. Nothing can run now: a clear message, and no CLI is started
    let text = turn_text(&app, &state, &kid).await;
    assert!(text.contains("No account can run bots here yet. Add one in Settings › This space › Accounts."), "{text}");
    assert_eq!(turns_run(tmp.path()), 2);

    // 5. Signed in again and re-checked: claude is back. A kwant pinned to codex (not installed)
    // is told why, and never quietly runs on claude instead (the old fallback did).
    std::fs::remove_file(tmp.path().join("refuse")).unwrap();
    call(&app, "POST", "/api/engines/recheck", None).await;
    assert_eq!(state.engines.status(EngineId::Claude), EngineStatus::Ready);
    body["preferred_model"] = json!("codex");
    let (s, _) = call(&app, "PUT", &format!("/api/kwants/{kid}"), Some(body.clone())).await;
    assert_eq!(s, StatusCode::OK);
    let text = turn_text(&app, &state, &kid).await;
    assert!(text.contains("'codex' can't run right now"), "{text}");
    assert!(text.contains("Codex isn't installed") && text.contains("No API key"), "{text}");
    assert_eq!(turns_run(tmp.path()), 2);

    // 6. A rejected Settings key is skipped for the signed-in CLI
    state.db.set_setting("anthropic_api_key", "sk-ant-rejected").await.unwrap();
    body["preferred_model"] = json!("claude");
    call(&app, "PUT", &format!("/api/kwants/{kid}"), Some(body)).await;
    state.engines.store(vec![(
        EngineId::AnthropicApi,
        kwnt::drivers::engines::api_refusal(401, "invalid x-api-key"),
    )]);
    let text = turn_text(&app, &state, &kid).await;
    assert!(text.contains("hello from fake claude"), "{text}");
    assert_eq!(turns_run(tmp.path()), 3);
}
