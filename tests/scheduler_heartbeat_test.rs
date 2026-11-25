//! scheduler-heartbeat (0.9.53): `/api/health` tells whether the scheduler's one background loop
//! (routines, follow-ups, parked turns, missions, goals…) is still finishing passes, as a new
//! `scheduler` field only. HTTP 200, `"status": "ok"` and the `"app":"kwnt"` text that install.sh,
//! Homebrew, the Linux smoke test and the Mac app look for never change, stale or not.

use axum::{
    body::Body,
    http::{Request, StatusCode},
    Router,
};
use chrono::{DateTime, Utc};
use kwnt::{
    broker::PermissionBroker,
    config::Config,
    db::Database,
    router::LayaRouter,
    server::{create_router, scheduler, scheduler_heartbeat::STALE_AFTER, AppState},
    tools::ToolRunner,
};
use serde_json::Value;
use std::time::{Duration, Instant};
use tempfile::TempDir;
use tokio_util::sync::CancellationToken;
use tower::ServiceExt;

async fn setup() -> (TempDir, AppState, Router) {
    let tmp = tempfile::tempdir().unwrap();
    let db = Database::init(tmp.path()).await.unwrap();
    let config = Config { data_dir: tmp.path().to_path_buf(), browser_cdp_url: Some("http://127.0.0.1:9".into()), ..Config::default() };
    let tools = ToolRunner::new(tmp.path().join("workspace"));
    let router = LayaRouter::new(config.laya_url.clone());
    let state = AppState::new(config, db, tools, PermissionBroker::new(), router);
    let app = create_router(state.clone());
    (tmp, state, app)
}

/// The status, the raw body and the parsed body of `GET /api/health`.
async fn health(app: &Router) -> (StatusCode, String, Value) {
    let req = Request::builder().uri("/api/health").body(Body::empty()).unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    let status = resp.status();
    let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    let text = String::from_utf8(bytes.to_vec()).unwrap();
    let json = serde_json::from_str(&text).unwrap();
    (status, text, json)
}

fn assert_unchanged_shape(status: StatusCode, text: &str, body: &Value) {
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["status"], "ok");
    assert_eq!(body["app"], "kwnt");
    assert_eq!(body["version"], env!("CARGO_PKG_VERSION"));
    assert!(body.get("laya_url").is_some() && body.get("laya").is_some());
    // install.sh, Homebrew and the Linux smoke test grep the raw text for this
    assert!(text.contains(r#""app":"kwnt""#), "{text}");
}

fn now_ms() -> i64 {
    Utc::now().timestamp_millis()
}

#[tokio::test]
async fn after_a_pass_health_shows_a_recent_tick_and_not_stale() {
    let (_tmp, state, app) = setup().await;
    let cancel = CancellationToken::new();
    let handle = scheduler::spawn(state.clone(), cancel.clone());

    // An empty database: the first pass takes milliseconds; 10 s is only a ceiling for a slow CI box
    let started = Instant::now();
    let body = loop {
        let (status, text, body) = health(&app).await;
        assert_unchanged_shape(status, &text, &body);
        if !body["scheduler"]["last_tick_at"].is_null() {
            break body;
        }
        assert!(started.elapsed() < Duration::from_secs(10), "the scheduler never finished a pass: {body}");
        tokio::time::sleep(Duration::from_millis(50)).await;
    };
    let s = &body["scheduler"];
    assert_eq!(s["stale"], false, "{s}");
    let tick: DateTime<Utc> = s["last_tick_at"].as_str().unwrap().parse().unwrap();
    let age = Utc::now() - tick;
    assert!(age >= chrono::Duration::zero() - chrono::Duration::seconds(1) && age < chrono::Duration::seconds(30), "{s}");
    let pass: DateTime<Utc> = s["pass_started_at"].as_str().unwrap().parse().unwrap();
    assert!(pass <= tick, "the finished pass started before it finished: {s}");

    cancel.cancel();
    tokio::time::timeout(Duration::from_secs(5), handle).await.expect("the scheduler stops on shutdown").unwrap();
}

#[tokio::test]
async fn a_stale_scheduler_is_reported_and_health_still_says_ok() {
    let (_tmp, state, app) = setup().await;

    // Not started (as in tests and before serve starts it): nothing to report, not stale
    let (status, text, body) = health(&app).await;
    assert_unchanged_shape(status, &text, &body);
    assert_eq!(body["scheduler"], serde_json::json!({ "last_tick_at": null, "pass_started_at": null, "stale": false }));

    // Started 5 minutes ago, last finished a pass past the limit, and the next pass never finished
    let limit = STALE_AFTER.as_millis() as i64;
    let now = now_ms();
    let finished = now - limit - 5_000;
    state.scheduler_heartbeat.set_for_tests(now - 300_000, finished + 30_000, finished);
    let (status, text, body) = health(&app).await;
    assert_unchanged_shape(status, &text, &body);
    let s = &body["scheduler"];
    assert_eq!(s["stale"], true, "{s}");
    let tick: DateTime<Utc> = s["last_tick_at"].as_str().unwrap().parse().unwrap();
    assert_eq!(tick.timestamp_millis(), finished);
    assert!(s["pass_started_at"].as_str().unwrap() > s["last_tick_at"].as_str().unwrap(), "the hung pass shows: {s}");

    // Started long ago and never finished a single pass (a startup step hung): stale too
    state.scheduler_heartbeat.set_for_tests(now - limit - 5_000, 0, 0);
    let (status, text, body) = health(&app).await;
    assert_unchanged_shape(status, &text, &body);
    assert_eq!(body["scheduler"]["stale"], true);
    assert!(body["scheduler"]["last_tick_at"].is_null());

    // It finishes a pass again: recovered
    state.scheduler_heartbeat.set_for_tests(now - 300_000, now_ms(), now_ms());
    let (status, text, body) = health(&app).await;
    assert_unchanged_shape(status, &text, &body);
    assert_eq!(body["scheduler"]["stale"], false);
}
