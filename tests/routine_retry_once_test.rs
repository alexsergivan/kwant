//! routine-retry-once: when a routine's model dies, AutoFix runs it again (twice), and the person
//! hears about it once. The retries are recorded as "retry", never as a "Run now" press they didn't
//! make; only the attempt AutoFix won't retry sends `routine_failed` (saying it was retried); and the
//! board's "Failed: … Retry it on the board" chat line isn't posted for a routine's task.
//! A fake claude CLI dies on the marker word; its own test binary: it changes PATH.

#[path = "common/exe.rs"]
mod exe;

use axum::{
    body::{to_bytes, Body},
    http::{Request, StatusCode},
    Router,
};
use chrono::Utc;
use kwnt::{
    broker::PermissionBroker,
    config::Config,
    db::{spaces::SpaceId, Database},
    drivers::events::RuntimeEvent,
    router::LayaRouter,
    server::{create_router, missions, scheduler, AppState},
    tools::ToolRunner,
};
use serde_json::{json, Value};
use std::path::PathBuf;
use std::sync::OnceLock;
use std::time::Duration;
use tokio_util::sync::CancellationToken;
use tower::ServiceExt;

/// `DIES` waits a moment (so AutoFix has linked the retry before it ends), then exits with no
/// output: a model that died. Anything else is a plain answer.
const FAKE_CLAUDE: &str = r#"#!/bin/sh
case "$1" in
  auth) echo '{"loggedIn":true}'; exit 0 ;;
esac
for a; do last="$a"; done
case "$last" in
  *DIES*) sleep 1; exit 3 ;;
  *) printf '%s\n' '{"type":"result","is_error":false,"num_turns":1,"result":"answer"}' ;;
esac
"#;

fn fake_cli() {
    static DONE: OnceLock<PathBuf> = OnceLock::new();
    DONE.get_or_init(|| {
        let bin = tempfile::tempdir().unwrap().keep().join("bin");
        std::fs::create_dir_all(&bin).unwrap();
        exe::write_executable(bin.join("claude"), FAKE_CLAUDE);
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
    (s, serde_json::from_slice(&bytes).unwrap_or_else(|_| Value::String(String::from_utf8_lossy(&bytes).into())))
}

async fn ok(app: &Router, method: &str, uri: &str, body: Option<Value>) -> Value {
    let (s, v) = call(app, method, uri, body).await;
    assert!(s.is_success(), "{method} {uri}: {s} {v}");
    v
}

struct Env {
    app: Router,
    state: AppState,
    rx: tokio::sync::broadcast::Receiver<RuntimeEvent>,
    stop: CancellationToken,
    kwant_id: String,
    _tmp: tempfile::TempDir,
}

impl Drop for Env {
    fn drop(&mut self) {
        self.stop.cancel();
    }
}

/// One bot (Researcher) on the fake claude; the board's listener runs, as `serve` starts it, and
/// AutoFix is on (a fresh install's default).
async fn setup() -> Env {
    fake_cli();
    let tmp = tempfile::tempdir().unwrap();
    let db = Database::init(tmp.path()).await.unwrap();
    db.set_setting("keep_going_turns", "0").await.unwrap();
    let config = Config {
        data_dir: tmp.path().to_path_buf(),
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
    let seeded = state.db.list_kwants(&SpaceId::default_space()).await.unwrap();
    for k in seeded.iter().filter(|k| k.name != "Assistant") {
        ok(&app, "DELETE", &format!("/api/kwants/{}", k.id), None).await;
    }
    let bot = json!({"name": "Researcher", "title": "Researcher the tester", "persona_prompt": "Test persona",
        "avatar_sprite": "s", "driver_type": "cli", "preferred_model": "claude"});
    let made = ok(&app, "POST", "/api/kwants", Some(bot)).await;
    for k in seeded.iter().filter(|k| k.name == "Assistant") {
        ok(&app, "DELETE", &format!("/api/kwants/{}", k.id), None).await;
    }
    let kwant_id = made["id"].as_str().unwrap().to_string();
    assert_eq!(missions::autofix::mode(&state).await, "on");
    let rx = state.event_sender.subscribe();
    let stop = CancellationToken::new();
    missions::listener::spawn(state.clone(), stop.clone());
    Env { app, state, rx, stop, kwant_id, _tmp: tmp }
}

/// What the person was told while a routine ran: its notices and its chat lines.
#[derive(Debug, Default)]
struct Told {
    notices: Vec<(String, String, String)>,
    lines: Vec<String>,
}

impl Env {
    async fn routine(&self, instructions: &str) -> String {
        let r = ok(
            &self.app,
            "POST",
            "/api/routines",
            Some(json!({"kwant_id": self.kwant_id, "name": "Digest", "instructions": instructions,
                "schedule": {"type": "daily", "time": "09:00", "tz": "Europe/Kyiv"}})),
        )
        .await;
        r["id"].as_str().unwrap().to_string()
    }

    /// The routine's runs, oldest first.
    async fn runs(&self, routine: &str) -> Vec<kwnt::db::continuity::RoutineRun> {
        let mut runs = self.state.db.list_routine_runs(routine, 50).await.unwrap();
        runs.reverse();
        runs
    }

    /// Everything said until `n` runs of `routine` have ended and nothing more happened for 3 s
    /// (a 4th run would start well inside that: a retry follows its failure at once).
    async fn told_until_runs_end(&mut self, routine: &str, n: usize) -> Told {
        let mut told = Told::default();
        let deadline = tokio::time::Instant::now() + Duration::from_secs(120);
        let mut quiet_since = tokio::time::Instant::now();
        while tokio::time::Instant::now() < deadline {
            match tokio::time::timeout(Duration::from_millis(200), self.rx.recv()).await {
                Ok(Ok(event)) => {
                    match event.unscoped().clone() {
                        RuntimeEvent::Notify { kind, title, body, .. } => told.notices.push((kind, title, body)),
                        RuntimeEvent::MessageAdded { message } if message["sender_role"] == "kwant" => {
                            told.lines.push(message["text"].as_str().unwrap_or_default().to_string());
                        }
                        _ => {}
                    }
                    quiet_since = tokio::time::Instant::now();
                }
                Ok(Err(_)) => {}
                Err(_) => {
                    let runs = self.runs(routine).await;
                    let ended = runs.len() >= n && runs.iter().all(|r| r.status != "running");
                    if ended && quiet_since.elapsed() >= Duration::from_secs(3) {
                        return told;
                    }
                }
            }
        }
        panic!("the routine's runs didn't end: {:?}; told {told:?}", self.runs(routine).await);
    }
}

/// Mira's routine-for-a-week pass, section 1: the model dies at the routine's scheduled time.
/// AutoFix runs it again twice; the person gets one notice, on the last attempt, that says so.
#[tokio::test]
async fn a_routine_whose_model_dies_is_retried_twice_and_told_once() {
    let mut env = setup().await;
    let id = env.routine("DIES summarise yesterday.").await;
    // Its scheduled time comes
    let past = Utc::now() - chrono::Duration::minutes(1);
    env.state.db.set_routine_schedule_state(&id, Some(past), true, None).await.unwrap();
    assert_eq!(scheduler::tick(&env.state, Utc::now()).await, 1);
    let told = env.told_until_runs_end(&id, 3).await;

    let runs = env.runs(&id).await;
    let triggers: Vec<&str> = runs.iter().map(|r| r.trigger.as_str()).collect();
    assert_eq!(triggers, ["schedule", "retry", "retry"], "AutoFix's runs aren't a Run now press: {runs:?}");
    assert!(runs.iter().all(|r| matches!(r.status.as_str(), "error" | "incomplete")), "{runs:?}");

    let failed: Vec<_> = told.notices.iter().filter(|(k, ..)| k == "routine_failed").collect();
    assert_eq!(failed.len(), 1, "one failure, one notice: {told:?}");
    let (_, title, body) = failed[0];
    assert_eq!(title, "Routine \"Digest\" failed");
    assert!(body.starts_with("The routine \"Digest\" did not finish"), "{body}");
    assert!(body.ends_with("It was retried twice."), "{body}");

    let board: Vec<_> = told.lines.iter().filter(|l| l.contains("Retry it on the board")).collect();
    assert!(board.is_empty(), "no board line in the chat for a routine: {board:?}");
}

/// A Run now press is still recorded as one ("manual"), and with AutoFix off nothing retries it:
/// the one failure sends its notice with no "retried".
#[tokio::test]
async fn a_run_now_press_is_still_manual_and_with_autofix_off_it_is_told_at_once() {
    let mut env = setup().await;
    env.state.db.set_setting(missions::autofix::SETTING, "off").await.unwrap();
    let id = env.routine("DIES summarise yesterday.").await;
    let (s, run) = call(&env.app, "POST", &format!("/api/routines/{id}/run"), None).await;
    assert_eq!(s, StatusCode::ACCEPTED, "{run}");
    assert_eq!(run["trigger"], "manual");
    let told = env.told_until_runs_end(&id, 1).await;

    let runs = env.runs(&id).await;
    assert_eq!(runs.len(), 1, "AutoFix is off: {runs:?}");
    assert_eq!(runs[0].trigger, "manual");
    let failed: Vec<_> = told.notices.iter().filter(|(k, ..)| k == "routine_failed").collect();
    assert_eq!(failed.len(), 1, "{told:?}");
    assert!(!failed[0].2.contains("retried"), "{}", failed[0].2);
    assert!(told.lines.iter().all(|l| !l.contains("Retry it on the board")), "{told:?}");
}
