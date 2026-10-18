//! Oleks's setup: "This machine" turned off, Personal on. A turn runs on Personal's own login
//! even when this machine's env points Claude Code at another provider, and its first status
//! line names the account. Its own test binary: it changes PATH and the process env.

use axum::{
    body::{to_bytes, Body},
    http::{Request, StatusCode},
    Router,
};
use kwnt::{
    broker::PermissionBroker,
    config::Config,
    db::Database,
    drivers::RuntimeEvent,
    router::LayaRouter,
    server::{create_router, AppState},
    tools::ToolRunner,
};
use serde_json::{json, Value};
use std::time::Duration;
use tower::ServiceExt;

const FAKE_CLAUDE: &str = r#"#!/bin/sh
case "$1" in
  auth) echo '{"loggedIn":true,"email":"fake@example.com"}'; exit 0 ;;
esac
echo "{\"type\":\"result\",\"is_error\":false,\"num_turns\":1,\"result\":\"home=$(basename "${CLAUDE_CONFIG_DIR:-host}") bedrock=${CLAUDE_CODE_USE_BEDROCK:-none} base=${ANTHROPIC_BASE_URL:-none}\",\"usage\":{\"input_tokens\":3,\"output_tokens\":4}}"
"#;

async fn call(app: &Router, method: &str, uri: &str, body: Option<Value>) -> (StatusCode, Value) {
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method(method)
                .uri(uri)
                .header("content-type", "application/json")
                .body(
                    body.map(|v| Body::from(v.to_string()))
                        .unwrap_or_else(Body::empty),
                )
                .unwrap(),
        )
        .await
        .unwrap();
    let s = resp.status();
    let bytes = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    (s, serde_json::from_slice(&bytes).unwrap_or(Value::Null))
}

/// Sends a prompt and collects the turn's events up to `turn_ended`.
async fn run_turn(app: &Router, state: &AppState, kwant_id: &str) -> Vec<RuntimeEvent> {
    let mut rx = state.event_sender.subscribe();
    let (s, v) = call(
        app,
        "POST",
        "/api/chat/send",
        Some(json!({"kwant_id": kwant_id, "prompt": "hi", "sender_id": "user"})),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    let mut events = Vec::new();
    tokio::time::timeout(Duration::from_secs(60), async {
        while let Ok(e) = rx.recv().await {
            let end = matches!(&e, RuntimeEvent::TurnEnded { kwant_id: k, .. } if k == kwant_id);
            // Turn events arrive tagged with their turn; the test looks at what happened
            events.push(e.unscoped().clone());
            if end {
                break;
            }
        }
    })
    .await
    .expect("turn did not end");
    events
}

#[tokio::test]
async fn personal_turn_names_its_account_and_ignores_this_machines_provider() {
    let tmp = tempfile::tempdir().unwrap();
    let bin = tmp.path().join("bin");
    std::fs::create_dir_all(&bin).unwrap();
    let fake = bin.join("claude");
    std::fs::write(&fake, FAKE_CLAUDE).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&fake, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    std::env::set_var("PATH", format!("{}:{}", bin.display(), std::env::var("PATH").unwrap_or_default()));
    // This machine's Claude Code runs on Bedrock through a proxy; Personal must not
    std::env::set_var("CLAUDE_CODE_USE_BEDROCK", "1");
    std::env::set_var("ANTHROPIC_BASE_URL", "http://127.0.0.1:9/proxy");

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

    let (s, personal) = call(
        &app,
        "POST",
        "/api/profiles",
        Some(json!({"engine": "claude", "name": "Personal", "kind": "cli_home", "is_default": true, "priority": 100})),
    )
    .await;
    assert!(s.is_success(), "{personal}");
    let personal_id = personal["id"].as_str().unwrap().to_string();
    let (s, _) = call(&app, "PATCH", "/api/profiles/prf_system_claude", Some(json!({"enabled": false}))).await;
    assert_eq!(s, StatusCode::OK);

    let (_, kwants) = call(&app, "GET", "/api/kwants", None).await;
    let k = kwants[0].clone();
    let kid = k["id"].as_str().unwrap().to_string();
    let mut body = k.clone();
    body.as_object_mut().unwrap().retain(|key, _| {
        ["name", "title", "persona_prompt", "soul", "avatar_sprite"].contains(&key.as_str())
    });
    body["preferred_model"] = json!("claude:haiku");
    let (s, v) = call(&app, "PUT", &format!("/api/kwants/{kid}"), Some(body)).await;
    assert_eq!(s, StatusCode::OK, "{v}");

    let events = run_turn(&app, &state, &kid).await;
    let text: String = events
        .iter()
        .filter_map(|e| match e {
            RuntimeEvent::TextDelta(t) => Some(t.as_str()),
            _ => None,
        })
        .collect();
    let notes: Vec<&str> = events
        .iter()
        .filter_map(|e| match e {
            RuntimeEvent::ThinkingDelta(t) => Some(t.as_str()),
            _ => None,
        })
        .collect();
    // Ran with Personal's own home, and without this machine's provider or proxy
    assert!(text.contains(&format!("home={personal_id}")), "{text}");
    assert!(text.contains("bedrock=none base=none"), "{text}");
    // The status line says which account, and never shows the CLI's path
    assert!(
        notes.iter().any(|n| n.starts_with("Running on Claude Code CLI · Personal · haiku")),
        "{notes:?}"
    );
    assert!(!notes.iter().any(|n| n.contains("This machine") || n.contains("binary")), "{notes:?}");
}
