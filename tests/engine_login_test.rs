//! "Sign in with Claude" for the Claude engine (`/api/engines/claude/login…` and `/logout`),
//! against a fake `claude` that prints a sign-in link and waits for the pasted code.
#![cfg(unix)]

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
    profiles::login::LoginManager,
    router::LayaRouter,
    server::{
        auth::{ApiToken, Require},
        create_router,
        engine_login::ClaudeLogin,
        AppState,
    },
    tools::ToolRunner,
};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tempfile::{tempdir, TempDir};
use tower::ServiceExt;

const TOKEN: &str = "engine-login-test-token-0123456789";
const GOOD: &str = "GOODCODE123#STATE987";

/// Prints a link, writes its pid, and signs in (writes `.credentials.json`) only for [`GOOD`].
const FAKE_CLAUDE: &str = r#"#!/bin/sh
# A profile's claude gets kwnt's provider overrides first, as `claude --settings <json> auth …`
if [ "$1" = --settings ]; then over="$2"; shift 2; fi
case "$1 $2" in
  "--version "*) echo "9.9.9 (Claude Code)" ;;
  "auth status")
    if [ -f "$CLAUDE_CONFIG_DIR/.credentials.json" ]; then echo '{"loggedIn":true,"authMethod":"claude.ai"}'
    else echo '{"loggedIn":false,"authMethod":"none"}'; exit 1; fi ;;
  "auth login")
    echo $$ > "$CLAUDE_CONFIG_DIR/login.pid"
    echo "Opening browser to sign in…"
    echo "If the browser didn't open, visit: https://claude.com/cai/oauth/authorize?code=true&client_id=x&code_challenge=CHALLENGE987&state=STATE987"
    printf 'Paste code here if prompted > '
    read code
    if [ "$code" = "GOODCODE123#STATE987" ]; then echo '{}' > "$CLAUDE_CONFIG_DIR/.credentials.json"; echo "Login successful."
    else echo "Login failed: Request failed with status code 400 for $code"; exit 1; fi ;;
  "auth logout") rm -f "$CLAUDE_CONFIG_DIR/.credentials.json"; echo "Successfully logged out" ;;
esac
"#;

struct Env {
    app: Router,
    home: PathBuf,
    _tmp: TempDir,
}

async fn setup_with(timeout: Duration) -> Env {
    logs();
    let tmp = tempdir().unwrap();
    let bin = tmp.path().join("claude");
    exe::write_executable(&bin, FAKE_CLAUDE);
    let home = tmp.path().join("claude-home");
    let db = Database::init(tmp.path()).await.unwrap();
    let config = Config { data_dir: tmp.path().to_path_buf(), ..Config::default() };
    let mut state = AppState::new(
        config.clone(),
        db,
        ToolRunner::new(tmp.path().join("workspace")),
        PermissionBroker::new(),
        LayaRouter::new(config.laya_url.clone()),
    );
    state.api_token = Arc::new(ApiToken::for_tests(TOKEN, Require::Auto, config.port));
    state.claude_login = Arc::new(ClaudeLogin::new(
        Some(bin.to_string_lossy().into_owned()),
        Some(home.clone()),
        LoginManager::with_timeout(timeout),
    ));
    Env { app: create_router(state), home, _tmp: tmp }
}

async fn setup() -> Env {
    setup_with(Duration::from_secs(600)).await
}

async fn req(app: &Router, method: &str, uri: &str, body: Option<Value>, headers: &[(&str, &str)]) -> (StatusCode, Value) {
    let mut b = Request::builder().method(method).uri(uri).header("content-type", "application/json");
    for (k, v) in headers {
        b = b.header(*k, *v);
    }
    let resp = app
        .clone()
        .oneshot(b.body(body.map(|v| Body::from(v.to_string())).unwrap_or_else(Body::empty)).unwrap())
        .await
        .unwrap();
    let status = resp.status();
    let bytes = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    (status, serde_json::from_slice(&bytes).unwrap_or(Value::Null))
}

async fn call(app: &Router, method: &str, uri: &str, body: Option<Value>) -> (StatusCode, Value) {
    req(app, method, uri, body, &[]).await
}

fn claude_status(engines: &Value) -> String {
    engines["engines"]
        .as_array()
        .unwrap()
        .iter()
        .find(|e| e["id"] == "claude")
        .map(|e| e["status"].as_str().unwrap_or_default().to_string())
        .unwrap()
}

fn login_pid(home: &Path) -> String {
    std::fs::read_to_string(home.join("login.pid")).unwrap().trim().to_string()
}

fn alive(pid: &str) -> bool {
    std::process::Command::new("kill").args(["-0", pid]).status().unwrap().success()
}

async fn wait_state(app: &Router, sid: &str, want: &str) -> Value {
    let mut last = Value::Null;
    for _ in 0..60 {
        let (s, v) = call(app, "GET", &format!("/api/engines/claude/login/{sid}"), None).await;
        assert_eq!(s, StatusCode::OK);
        if v["state"] == want {
            return v;
        }
        last = v;
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    panic!("never reached {want}: {last}");
}

/// Every log line any test in this file writes. Installed globally before the first test runs
/// kwnt code, so no callsite caches "nobody listens".
fn logs() -> Arc<Mutex<Vec<u8>>> {
    static LOGS: std::sync::OnceLock<Arc<Mutex<Vec<u8>>>> = std::sync::OnceLock::new();
    LOGS.get_or_init(install_log_capture).clone()
}

fn install_log_capture() -> Arc<Mutex<Vec<u8>>> {
    #[derive(Clone)]
    struct W(Arc<Mutex<Vec<u8>>>);
    impl std::io::Write for W {
        fn write(&mut self, b: &[u8]) -> std::io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(b);
            Ok(b.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    let buf = Arc::new(Mutex::new(Vec::new()));
    let w = W(buf.clone());
    let sub = tracing_subscriber::fmt()
        .with_max_level(tracing::Level::TRACE)
        .with_ansi(false)
        .with_writer(move || w.clone())
        .finish();
    tracing::subscriber::set_global_default(sub).unwrap();
    buf
}

#[tokio::test]
async fn sign_in_with_the_right_code_makes_claude_ready() {
    let env = setup().await;
    let app = &env.app;

    let (s, v) = call(app, "POST", "/api/engines/claude/login", None).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(v["state"], "waiting_for_code", "{v}");
    assert!(v["url"].as_str().unwrap().starts_with("https://claude.com/cai/oauth/authorize?"), "{v}");
    let sid = v["session_id"].as_str().unwrap().to_string();
    assert!(v["expires_at"].is_string());

    let (s, v) = call(app, "POST", &format!("/api/engines/claude/login/{sid}/code"), Some(json!({ "code": GOOD }))).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(v["state"], "done", "{v}");
    assert!(env.home.join(".credentials.json").exists());

    // Done means GET /api/engines already shows the new status
    let (_, engines) = call(app, "GET", "/api/engines", None).await;
    assert_eq!(claude_status(&engines), "ready", "{engines}");
    let done = wait_state(app, &sid, "done").await;
    assert_eq!((&done["detail"], &done["url"]), (&Value::Null, &Value::Null), "{done}");

    // A second code for a finished sign-in is refused
    let (s, _) = call(app, "POST", &format!("/api/engines/claude/login/{sid}/code"), Some(json!({ "code": GOOD }))).await;
    assert_eq!(s, StatusCode::BAD_REQUEST);

    // Other tests in this file log here too (wrong codes, cancels, expiry)
    let logs = String::from_utf8(logs().lock().unwrap().clone()).unwrap();
    assert!(logs.contains("claude engine sign-in started"), "{logs}");
    assert!(logs.contains("claude engine sign-in finished"), "{logs}");
    for secret in ["GOODCODE123", "WRONGCODE999", "STATE987", "CHALLENGE987", "oauth/authorize"] {
        assert!(!logs.contains(secret), "{secret} was logged:\n{logs}");
    }
}

#[tokio::test]
async fn a_wrong_code_fails_without_echoing_cli_output_and_allows_a_retry() {
    let env = setup().await;
    let app = &env.app;
    let (_, v) = call(app, "POST", "/api/engines/claude/login", None).await;
    let sid = v["session_id"].as_str().unwrap().to_string();

    // Not code-shaped: rejected before it reaches the CLI, and the sign-in keeps waiting
    let (s, v) = call(app, "POST", &format!("/api/engines/claude/login/{sid}/code"), Some(json!({ "code": "no way!" }))).await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "{v}");
    wait_state(app, &sid, "waiting_for_code").await;

    let (s, v) = call(app, "POST", &format!("/api/engines/claude/login/{sid}/code"), Some(json!({ "code": "WRONGCODE999#STATE987" }))).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(v["state"], "failed", "{v}");
    let detail = v["detail"].as_str().unwrap();
    assert!(detail.contains("didn't accept that code"), "{v}");
    // The CLI's HTTP status stays out of the text (not the whole body: a random session id can hold "400")
    assert!(!v.to_string().contains("WRONGCODE999") && !detail.contains("400"), "{v}");
    assert!(!env.home.join(".credentials.json").exists());
    let (_, engines) = call(app, "GET", "/api/engines", None).await;
    assert_ne!(claude_status(&engines), "ready");

    // The failed one doesn't block a new one
    let (s, v) = call(app, "POST", "/api/engines/claude/login", None).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_ne!(v["session_id"].as_str().unwrap(), sid);
}

#[tokio::test]
async fn only_one_sign_in_runs_and_cancel_kills_it() {
    let env = setup().await;
    let app = &env.app;
    let (_, v) = call(app, "POST", "/api/engines/claude/login", None).await;
    let sid = v["session_id"].as_str().unwrap().to_string();
    let pid = login_pid(&env.home);
    assert!(alive(&pid));

    let (s, busy) = call(app, "POST", "/api/engines/claude/login", None).await;
    assert_eq!(s, StatusCode::CONFLICT, "{busy}");
    assert_eq!(busy["session_id"], sid.as_str());

    let (s, _) = call(app, "DELETE", &format!("/api/engines/claude/login/{sid}"), None).await;
    assert_eq!(s, StatusCode::NO_CONTENT);
    assert_eq!(wait_state(app, &sid, "cancelled").await["url"], Value::Null);
    assert!(!alive(&pid), "the CLI is still running after cancel");

    // A cancelled sign-in takes no code, and a new one may start
    let (s, _) = call(app, "POST", &format!("/api/engines/claude/login/{sid}/code"), Some(json!({ "code": GOOD }))).await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
    let (s, _) = call(app, "POST", "/api/engines/claude/login", None).await;
    assert_eq!(s, StatusCode::OK);
}

#[tokio::test]
async fn an_unfinished_sign_in_expires_and_its_cli_is_killed() {
    let env = setup_with(Duration::from_millis(800)).await;
    let app = &env.app;
    let (_, v) = call(app, "POST", "/api/engines/claude/login", None).await;
    assert_eq!(v["state"], "waiting_for_code", "{v}");
    let sid = v["session_id"].as_str().unwrap().to_string();
    let pid = login_pid(&env.home);

    let v = wait_state(app, &sid, "expired").await;
    assert!(v["detail"].as_str().unwrap().contains("timed out"), "{v}");
    assert!(!alive(&pid), "the CLI outlived its sign-in");
    let (s, _) = call(app, "POST", &format!("/api/engines/claude/login/{sid}/code"), Some(json!({ "code": GOOD }))).await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn logout_signs_claude_out_and_answers_with_the_engines() {
    let env = setup().await;
    let app = &env.app;
    std::fs::create_dir_all(&env.home).unwrap();
    std::fs::write(env.home.join(".credentials.json"), "{}").unwrap();
    let (_, engines) = call(app, "POST", "/api/engines/claude", Some(json!({ "enabled": true }))).await;
    assert_eq!(claude_status(&engines), "ready", "{engines}");

    let (s, engines) = call(app, "POST", "/api/engines/claude/logout", None).await;
    assert_eq!(s, StatusCode::OK, "{engines}");
    assert_eq!(claude_status(&engines), "signed_out", "{engines}");
    assert!(!env.home.join(".credentials.json").exists());
    let (_, engines) = call(app, "GET", "/api/engines", None).await;
    assert_eq!(claude_status(&engines), "signed_out");
}

#[tokio::test]
async fn unknown_sessions_404_and_remote_callers_need_the_token() {
    let env = setup().await;
    let app = &env.app;
    for (m, uri, body) in [
        ("GET", "/api/engines/claude/login/login_nope", None),
        ("DELETE", "/api/engines/claude/login/login_nope", None),
        ("POST", "/api/engines/claude/login/login_nope/code", Some(json!({ "code": GOOD }))),
    ] {
        assert_eq!(call(app, m, uri, body).await.0, StatusCode::NOT_FOUND, "{m} {uri}");
    }

    let remote = ("host", "kwnt.example.com");
    for (m, uri) in [
        ("POST", "/api/engines/claude/login"),
        ("GET", "/api/engines/claude/login/x"),
        ("POST", "/api/engines/claude/logout"),
    ] {
        let (s, _) = req(app, m, uri, None, &[remote]).await;
        assert_eq!(s, StatusCode::UNAUTHORIZED, "{m} {uri}");
    }
    assert!(!env.home.join("login.pid").exists(), "an unauthenticated call started the CLI");
    let bearer = format!("Bearer {TOKEN}");
    let (s, v) = req(app, "POST", "/api/engines/claude/login", None, &[remote, ("authorization", &bearer)]).await;
    assert_eq!(s, StatusCode::OK, "{v}");
}
