//! Each Claude sign-in's 5-hour and weekly limit: kept from its turns, read fresh at most once a
//! minute on "Check now", and shown as `usage` on `GET /api/profiles`. A fake claude CLI on PATH
//! prints a `rate_limit_event` like the real one. Its own test binary: it changes PATH.

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
    drivers::RuntimeEvent,
    router::LayaRouter,
    server::{create_router, AppState},
    tools::ToolRunner,
};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::time::Duration;
use tower::ServiceExt;

/// `util`, `status`, `no-usage` and `out-of-usage` in the profile's home steer what it reports;
/// every model call adds a line to `calls`.
const FAKE_CLAUDE: &str = r#"#!/bin/sh
if [ "$1" = --settings ]; then shift 2; fi
case "$1" in
  auth) echo '{"loggedIn":true,"email":"fake@example.com"}'; exit 0 ;;
esac
h="$CLAUDE_CONFIG_DIR"
echo "call" >> "$h/calls"
util=$(cat "$h/util" 2>/dev/null || echo 0.14)
st=$(cat "$h/status" 2>/dev/null || echo allowed)
echo '{"type":"system","subtype":"init","tools":[],"mcp_servers":[]}'
if [ ! -f "$h/no-usage" ]; then
  echo '{"type":"rate_limit_event","rate_limit_info":{"status":"'"$st"'","resetsAt":1794605400,"rateLimitType":"five_hour","unifiedWindows":{"five_hour":{"utilization":'"$util"',"resetsAt":1794605400,"status":"'"$st"'"},"seven_day":{"utilization":0.56,"resetsAt":1794960000,"status":"allowed"}}}}'
fi
if [ -f "$h/out-of-usage" ]; then
  echo '{"type":"result","is_error":true,"num_turns":1,"result":"Claude AI usage limit reached|'"$FAKE_RESET"'"}'
  exit 1
fi
echo '{"type":"result","is_error":false,"num_turns":1,"result":"OK","usage":{"input_tokens":3,"output_tokens":1}}'
"#;

async fn call(app: &Router, method: &str, uri: &str, body: Option<Value>) -> (StatusCode, Value, Option<String>) {
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
    let retry = resp.headers().get("retry-after").and_then(|v| v.to_str().ok()).map(str::to_string);
    let bytes = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    (s, serde_json::from_slice(&bytes).unwrap_or(Value::Null), retry)
}

async fn run_turn(app: &Router, state: &AppState, kwant_id: &str) {
    let mut rx = state.event_sender.subscribe();
    let (s, v, _) = call(app, "POST", "/api/chat/send", Some(json!({"kwant_id": kwant_id, "prompt": "hi", "sender_id": "user"}))).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    tokio::time::timeout(Duration::from_secs(60), async {
        while let Ok(e) = rx.recv().await {
            if matches!(e.unscoped(), RuntimeEvent::TurnEnded { kwant_id: k, .. } if k == kwant_id) {
                break;
            }
        }
    })
    .await
    .expect("turn did not end");
}

async fn profile(app: &Router, id: &str) -> Value {
    let (_, all, _) = call(app, "GET", "/api/profiles", None).await;
    all.as_array().unwrap().iter().find(|p| p["id"] == id).cloned().unwrap()
}

fn calls(home: &Path) -> usize {
    std::fs::read_to_string(home.join("calls")).map(|s| s.lines().count()).unwrap_or(0)
}

#[tokio::test]
async fn usage_is_kept_from_turns_and_checked_at_most_once_a_minute() {
    let tmp = tempfile::tempdir().unwrap();
    let bin = tmp.path().join("bin");
    std::fs::create_dir_all(&bin).unwrap();
    exe::write_executable(&bin.join("claude"), FAKE_CLAUDE);
    std::env::set_var("PATH", format!("{}:{}", bin.display(), std::env::var("PATH").unwrap_or_default()));
    std::env::set_var("FAKE_RESET", (chrono::Utc::now().timestamp() + 2 * 3600).to_string());

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

    let (_, work, _) = call(&app, "POST", "/api/profiles", Some(json!({"engine": "claude", "name": "Work", "kind": "cli_home", "is_default": true, "priority": 1}))).await;
    let (_, personal, _) = call(&app, "POST", "/api/profiles", Some(json!({"engine": "claude", "name": "Personal", "kind": "cli_home", "priority": 2}))).await;
    let (s, _, _) = call(&app, "PATCH", "/api/profiles/prf_system_claude", Some(json!({"enabled": false}))).await;
    assert_eq!(s, StatusCode::OK);
    let work_id = work["id"].as_str().unwrap().to_string();
    let personal_id = personal["id"].as_str().unwrap().to_string();
    let work_home = PathBuf::from(work["home_dir"].as_str().unwrap());
    let personal_home = PathBuf::from(personal["home_dir"].as_str().unwrap());
    std::fs::create_dir_all(&work_home).unwrap();
    std::fs::create_dir_all(&personal_home).unwrap();
    std::fs::write(personal_home.join("no-usage"), "").unwrap();

    // No reading yet: null
    assert_eq!(profile(&app, &work_id).await["usage"], Value::Null);

    let (_, kwants, _) = call(&app, "GET", "/api/kwants", None).await;
    let k = kwants[0].clone();
    let kid = k["id"].as_str().unwrap().to_string();
    let mut body = k.clone();
    body.as_object_mut()
        .unwrap()
        .retain(|key, _| ["name", "title", "persona_prompt", "soul", "avatar_sprite"].contains(&key.as_str()));
    body["preferred_model"] = json!("claude");
    let (s, v, _) = call(&app, "PUT", &format!("/api/kwants/{kid}"), Some(body)).await;
    assert_eq!(s, StatusCode::OK, "{v}");

    // 1. A turn keeps the account's windows
    run_turn(&app, &state, &kid).await;
    let u = profile(&app, &work_id).await["usage"].clone();
    assert_eq!(u["source"], "turn", "{u}");
    assert_eq!(u["status"], "ok");
    assert_eq!(
        u["windows"],
        json!([
            {"kind": "five_hour", "used_pct": 14, "resets_at": "2026-11-13T21:30:00Z"},
            {"kind": "weekly", "used_pct": 56, "resets_at": "2026-11-18T00:00:00Z"}
        ])
    );
    assert!(u["checked_at"].as_str().unwrap().ends_with('Z'));

    // 2. Check now reads it fresh: allowed_warning shows as warning
    std::fs::write(work_home.join("util"), "0.904").unwrap();
    std::fs::write(work_home.join("status"), "allowed_warning").unwrap();
    let before = calls(&work_home);
    let (s, first, _) = call(&app, "POST", &format!("/api/profiles/{work_id}/usage"), None).await;
    assert_eq!(s, StatusCode::OK, "{first}");
    assert_eq!(calls(&work_home), before + 1);
    assert_eq!(first["source"], "check");
    assert_eq!(first["status"], "warning");
    assert_eq!(first["windows"][0], json!({"kind": "five_hour", "used_pct": 90, "resets_at": "2026-11-13T21:30:00Z"}));
    assert_eq!(profile(&app, &work_id).await["usage"], first);

    // 3. A second press within the minute returns that reading and makes no call
    std::fs::write(work_home.join("util"), "0.99").unwrap();
    let (s, again, _) = call(&app, "POST", &format!("/api/profiles/{work_id}/usage"), None).await;
    assert_eq!(s, StatusCode::OK, "{again}");
    assert_eq!(again, first);
    assert_eq!(calls(&work_home), before + 1);

    // 4. An account that reports nothing: the read fails, and a press within the minute waits
    let (s, v, _) = call(&app, "POST", &format!("/api/profiles/{personal_id}/usage"), None).await;
    assert_eq!(s, StatusCode::BAD_GATEWAY, "{v}");
    let n = calls(&personal_home);
    let (s, _, retry) = call(&app, "POST", &format!("/api/profiles/{personal_id}/usage"), None).await;
    assert_eq!(s, StatusCode::TOO_MANY_REQUESTS);
    assert_eq!(retry.as_deref(), Some("60"));
    assert_eq!(calls(&personal_home), n);
    assert_eq!(profile(&app, &personal_id).await["usage"], Value::Null);

    // 5. A refused turn still keeps its windows: rejected shows as out, next to "out of usage"
    std::fs::write(work_home.join("status"), "rejected").unwrap();
    std::fs::write(work_home.join("util"), "1").unwrap();
    std::fs::write(work_home.join("out-of-usage"), "").unwrap();
    run_turn(&app, &state, &kid).await;
    let w = profile(&app, &work_id).await;
    assert!(w["limited_until"].is_string(), "{w}");
    assert_eq!(w["usage"]["source"], "turn");
    assert_eq!(w["usage"]["status"], "out");
    assert_eq!(w["usage"]["windows"][0]["used_pct"], 100);

    // 6. API-key accounts have no windows: no usage, and Check now is refused
    let (s, key, _) = call(
        &app,
        "POST",
        "/api/profiles",
        Some(json!({"engine": "claude", "name": "Work API", "kind": "api_key", "api_key": "sk-ant-api03-0123456789abcdef"})),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED, "{key}");
    let key_id = key["id"].as_str().unwrap();
    assert_eq!(profile(&app, key_id).await["usage"], Value::Null);
    let (s, _, _) = call(&app, "POST", &format!("/api/profiles/{key_id}/usage"), None).await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
    let (s, _, _) = call(&app, "POST", "/api/profiles/prf_nope/usage", None).await;
    assert_eq!(s, StatusCode::NOT_FOUND);
}

/// A served turn forgets the account's failure but keeps its usage; a new sign-in forgets both.
#[tokio::test]
async fn clearing_a_failure_keeps_the_usage_reading() {
    use kwnt::profiles::usage::{AccountUsage, UsageReading, UsageWindow};
    let tmp = tempfile::tempdir().unwrap();
    let db = Database::init(tmp.path()).await.unwrap();
    let id = "prf_system_claude";
    let reading = UsageReading {
        status: Some("out".into()),
        windows: vec![UsageWindow { kind: "five_hour".into(), used_pct: 100, resets_at: None }],
    };
    let at = chrono::Utc::now();
    let usage = AccountUsage::new(reading, "turn", chrono::DateTime::from_timestamp(at.timestamp(), 0).unwrap());
    db.record_profile_failure(id, "quota", Some(at + chrono::Duration::hours(1))).await.unwrap();
    db.record_profile_usage(id, &usage).await.unwrap();
    assert!(db.list_profile_states().await.unwrap().contains_key(id));
    assert_eq!(db.list_profile_usage().await.unwrap().get(id), Some(&usage));

    db.clear_profile_failure(id).await.unwrap();
    assert!(db.list_profile_states().await.unwrap().is_empty(), "a usage-only row is no failure");
    assert_eq!(db.list_profile_usage().await.unwrap().get(id), Some(&usage));

    // A new failure leaves the reading alone too
    db.record_profile_failure(id, "auth", None).await.unwrap();
    assert_eq!(db.list_profile_usage().await.unwrap().get(id), Some(&usage));

    db.clear_profile_state(id).await.unwrap();
    assert!(db.list_profile_usage().await.unwrap().is_empty());
}
