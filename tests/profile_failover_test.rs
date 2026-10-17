//! A turn whose profile is out of usage moves to the engine's next profile, with a fake claude
//! CLI on PATH. Its own test binary: it changes PATH and the process env.

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
if [ -f "$CLAUDE_CONFIG_DIR/out-of-usage" ]; then
  echo '{"type":"result","is_error":true,"num_turns":1,"result":"Claude AI usage limit reached|'"$FAKE_RESET"'"}'
  exit 1
fi
echo "{\"type\":\"result\",\"is_error\":false,\"num_turns\":1,\"result\":\"reply from $(basename "$CLAUDE_CONFIG_DIR") key=${ANTHROPIC_API_KEY:-none}\",\"usage\":{\"input_tokens\":3,\"output_tokens\":4}}"
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
            events.push(e);
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
async fn out_of_usage_profile_fails_over_once_and_pinned_turns_give_up() {
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
    std::env::set_var(
        "PATH",
        format!(
            "{}:{}",
            bin.display(),
            std::env::var("PATH").unwrap_or_default()
        ),
    );
    // Must not reach a profile's CLI
    std::env::set_var("ANTHROPIC_API_KEY", "sk-ant-host-key-should-not-leak");
    let reset = chrono::Utc::now().timestamp() + 2 * 3600;
    std::env::set_var("FAKE_RESET", reset.to_string());

    let data = tmp.path().join("data");
    let db = Database::init(&data).await.unwrap();
    // Laya is unreachable: routing falls back to the kwant's preferred model
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

    let (_, a) = call(&app, "POST", "/api/profiles", Some(json!({"engine": "claude", "name": "Work", "kind": "cli_home", "is_default": true, "priority": 1}))).await;
    let (_, b) = call(
        &app,
        "POST",
        "/api/profiles",
        Some(json!({"engine": "claude", "name": "Personal", "kind": "cli_home", "priority": 2})),
    )
    .await;
    let (s, _) = call(
        &app,
        "PATCH",
        "/api/profiles/prf_system_claude",
        Some(json!({"enabled": false})),
    )
    .await;
    assert_eq!(s, StatusCode::OK);
    let a_id = a["id"].as_str().unwrap().to_string();
    let b_id = b["id"].as_str().unwrap().to_string();
    std::fs::write(
        std::path::Path::new(a["home_dir"].as_str().unwrap()).join("out-of-usage"),
        "",
    )
    .unwrap();

    let (_, kwants) = call(&app, "GET", "/api/kwants", None).await;
    let k = kwants[0].clone();
    let kid = k["id"].as_str().unwrap().to_string();
    let mut body = k.clone();
    body.as_object_mut().unwrap().retain(|key, _| {
        ["name", "title", "persona_prompt", "soul", "avatar_sprite"].contains(&key.as_str())
    });
    body["preferred_model"] = json!("claude");
    let (s, v) = call(
        &app,
        "PUT",
        &format!("/api/kwants/{kid}"),
        Some(body.clone()),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{v}");

    // 1. Work is out of usage: the turn completes on Personal, with one switch
    let events = run_turn(&app, &state, &kid).await;
    let started = events
        .iter()
        .filter(|e| matches!(e, RuntimeEvent::TurnStarted { kwant_id, .. } if *kwant_id == kid))
        .count();
    assert_eq!(started, 1);
    let switches: Vec<_> = events
        .iter()
        .filter(|e| matches!(e, RuntimeEvent::ProfileSwitched { .. }))
        .collect();
    assert_eq!(switches.len(), 1, "{events:?}");
    match switches[0] {
        RuntimeEvent::ProfileSwitched {
            kwant_id,
            from_profile_id,
            to_profile_id,
            to_name,
            class,
            retry_at,
            ..
        } => {
            assert_eq!(kwant_id, &kid);
            assert_eq!(from_profile_id, &a_id);
            assert_eq!(to_profile_id, &b_id);
            assert_eq!(to_name, "Personal");
            assert_eq!(class, "quota");
            assert_eq!(retry_at.unwrap().timestamp(), reset);
        }
        _ => unreachable!(),
    }
    let completes: Vec<_> = events
        .iter()
        .filter(|e| matches!(e, RuntimeEvent::TurnComplete { .. }))
        .collect();
    assert_eq!(
        completes.len(),
        1,
        "the refused attempt's completion is not sent: {events:?}"
    );
    assert!(
        matches!(completes[0], RuntimeEvent::TurnComplete { kwant_id: Some(k), profile_id: Some(p), tokens: 7, .. } if *k == kid && *p == b_id)
    );
    assert!(
        matches!(events.last(), Some(RuntimeEvent::TurnEnded { outcome, .. }) if outcome == "completed")
    );
    let text: String = events
        .iter()
        .filter_map(|e| match e {
            RuntimeEvent::TextDelta(t) => Some(t.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(text, format!("reply from {b_id} key=none"));
    assert!(!text.contains("usage limit"));

    let saved = state.db.list_messages(&kid, 10).await.unwrap();
    let reply = saved
        .iter()
        .rev()
        .find(|m| m.sender_role == "kwant")
        .unwrap();
    assert_eq!(reply.text, format!("reply from {b_id} key=none"));
    assert!(
        reply
            .reasoning_content
            .as_deref()
            .unwrap_or("")
            .contains("switching to \"Personal\""),
        "{reply:?}"
    );

    // Work's state: out of usage until the provider's reset time; the list shows it
    let (_, list) = call(&app, "GET", "/api/profiles", None).await;
    let work = list
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["id"] == a_id.as_str())
        .unwrap()
        .clone();
    assert_eq!(work["last_error"], "quota");
    let until: chrono::DateTime<chrono::Utc> =
        work["limited_until"].as_str().unwrap().parse().unwrap();
    assert_eq!(until.timestamp(), reset);
    assert_eq!(work["signed_in"], true);
    assert_eq!(work["account_label"], "fake@example.com");

    // 2. Next turn skips Work up front: no switch at all
    let events = run_turn(&app, &state, &kid).await;
    assert!(
        !events
            .iter()
            .any(|e| matches!(e, RuntimeEvent::ProfileSwitched { .. })),
        "{events:?}"
    );
    assert!(events.iter().any(
        |e| matches!(e, RuntimeEvent::TurnComplete { profile_id: Some(p), .. } if *p == b_id)
    ));

    // 3. Pinned to Work without failover: the turn says why and stops
    body["profile_id"] = json!(a_id);
    body["profile_failover"] = json!(false);
    let (s, v) = call(&app, "PUT", &format!("/api/kwants/{kid}"), Some(body)).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    let events = run_turn(&app, &state, &kid).await;
    assert!(!events
        .iter()
        .any(|e| matches!(e, RuntimeEvent::ProfileSwitched { .. })));
    let text: String = events
        .iter()
        .filter_map(|e| match e {
            RuntimeEvent::TextDelta(t) => Some(t.as_str()),
            _ => None,
        })
        .collect();
    assert!(text.contains("\"Work\" is out of usage until"), "{text}");
    assert!(!text.contains("sk-ant"), "{text}");
    assert!(
        matches!(events.last(), Some(RuntimeEvent::TurnEnded { outcome, .. }) if outcome == "failed")
    );
    let saved = state.db.list_messages(&kid, 10).await.unwrap();
    assert!(saved
        .iter()
        .rev()
        .find(|m| m.sender_role == "kwant")
        .unwrap()
        .text
        .contains("out of usage"));
}
