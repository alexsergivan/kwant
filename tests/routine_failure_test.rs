//! A routine whose engine can't run records why in its history, not just that it failed. Uses a
//! signed-out fake claude CLI on PATH, so it is its own test binary: it changes the process env.

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
use std::time::Duration;
use tower::ServiceExt;

/// Signed out, as the real CLI reports it; or, with `$CRASH` set, dying with no output at all.
const FAKE_CLAUDE: &str = r#"#!/bin/sh
case "$1" in
  auth) echo '{"loggedIn":false}'; exit 1 ;;
esac
[ -f "$CRASH" ] && exit 3
echo '{"type":"result","is_error":true,"num_turns":1,"result":"Invalid API key · Please run /login"}'
exit 1
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

#[tokio::test]
async fn failed_routine_run_records_the_engine_error() {
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
    let crash = tmp.path().join("crash");
    std::env::set_var("CRASH", &crash);
    std::env::set_var(
        "PATH",
        format!(
            "{}:{}",
            bin.display(),
            std::env::var("PATH").unwrap_or_default()
        ),
    );

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

    let (_, kwants) = call(&app, "GET", "/api/kwants", None).await;
    let k = kwants[0].clone();
    let kid = k["id"].as_str().unwrap().to_string();
    let mut body = k.clone();
    body.as_object_mut().unwrap().retain(|key, _| {
        ["name", "title", "persona_prompt", "soul", "avatar_sprite"].contains(&key.as_str())
    });
    body["preferred_model"] = json!("claude");
    let (s, v) = call(&app, "PUT", &format!("/api/kwants/{kid}"), Some(body)).await;
    assert_eq!(s, StatusCode::OK, "{v}");

    let (s, r) = call(
        &app,
        "POST",
        "/api/routines",
        Some(json!({
            "kwant_id": kid,
            "name": "Digest",
            "instructions": "Summarise yesterday.",
            "schedule": {"type": "daily", "time": "09:00", "tz": "Europe/Kiev"},
        })),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED, "{r}");
    // Stored under the current name, so the schedule survives a host without tzdata-legacy
    assert_eq!(r["schedule"]["tz"], "Europe/Kyiv");
    let id = r["id"].as_str().unwrap().to_string();

    let error = run_once(&app, &id).await;
    assert!(error.starts_with("the agent's turn failed: "), "{error}");
    assert!(error.contains("is signed out"), "{error}");
    assert!(error.chars().count() <= 600, "{error}");

    // A CLI that dies silently used to be recorded as an ok run with an empty reply
    std::fs::write(&crash, "").unwrap();
    let error = run_once(&app, &id).await;
    assert!(error.contains("exited with status 3 and no output"), "{error}");
}

/// Runs the routine now and returns the run's error, which it must have.
async fn run_once(app: &Router, id: &str) -> String {
    let (_, before) = call(app, "GET", &format!("/api/routines/{id}/runs"), None).await;
    let seen = before.as_array().map_or(0, Vec::len);
    let (s, _) = call(app, "POST", &format!("/api/routines/{id}/run"), None).await;
    assert_eq!(s, StatusCode::ACCEPTED);
    let run = tokio::time::timeout(Duration::from_secs(60), async {
        loop {
            let (_, runs) = call(app, "GET", &format!("/api/routines/{id}/runs"), None).await;
            let runs = runs.as_array().cloned().unwrap_or_default();
            if let Some(run) = runs.first().filter(|r| runs.len() > seen && r["status"] != "running") {
                return run.clone();
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    })
    .await
    .expect("routine run did not finish");
    assert_eq!(run["status"], "error", "{run}");
    run["error"].as_str().unwrap_or_default().to_string()
}
