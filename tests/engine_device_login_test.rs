//! "Sign in with ChatGPT" and "Sign in with Grok" for the Codex and Grok engines (0.7.6, O3):
//! `/api/engines/{codex,grok}/login…`, against fake CLIs that print a device link and code the
//! way `codex login --device-auth` (0.162) and `grok login --device-auth` (1.0) do, and sign in
//! once the test "approves" the code. No real account or network is used.
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
    profiles::{login::LoginManager, Engine},
    router::LayaRouter,
    server::{
        auth::{ApiToken, Require},
        create_router,
        engine_login::EngineLogin,
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

const CODEX_CODE: &str = "WXYZ-12345";
const GROK_CODE: &str = "QRST-6789";

/// Waits for `approve` (signs in) or `expire` (says the code expired) in its home.
const WAIT_LOOP: &str = r#"
    i=0
    while [ $i -lt 300 ]; do
      if [ -f "$H/approve" ]; then echo '{}' > "$H/auth.json"; echo "Successfully logged in"; exit 0; fi
      if [ -f "$H/expire" ]; then echo "Error: device code expired. Run login again."; exit 1; fi
      sleep 0.1; i=$((i+1))
    done
    exit 1 ;;"#;

fn fake_codex(home: &Path) -> String {
    format!(
        r#"#!/bin/sh
H="${{CODEX_HOME:-{home}}}"
case "$1 $2" in
  "--version "*) echo "codex-cli 9.9.9" ;;
  "login status")
    if [ -f "$H/auth.json" ] && [ ! -f "$H/unusable" ]; then echo "Logged in using ChatGPT"
    else echo "Not logged in"; exit 1; fi ;;
  "debug models") echo '{{"models":[{{"slug":"gpt-test","display_name":"GPT Test","priority":1}}]}}' ;;
  "login --device-auth")
    echo $$ > "$H/login.pid"
    printf '\nWelcome to Codex [v\033[90m9.9.9\033[0m]\n\nFollow these steps to sign in with ChatGPT using device code authorization:\n\n'
    printf '1. Open this link in your browser and sign in to your account\n   \033[94mhttps://auth.openai.com/codex/device\033[0m\n\n'
    printf '2. Enter this one-time code \033[90m(expires in 15 minutes)\033[0m\n   \033[94m{code}\033[0m\n\n'
    printf '\033[90mContinue only if you started this login in Codex.\033[0m\n'{wait}
  "logout ") rm -f "$H/auth.json"; echo "Successfully logged out" ;;
esac
"#,
        home = home.display(),
        code = CODEX_CODE,
        wait = WAIT_LOOP,
    )
}

fn fake_grok(home: &Path) -> String {
    format!(
        r#"#!/bin/sh
H="${{GROK_HOME:-{home}}}"
case "$1 $2" in
  "--version "*) echo "grok 9.9.9 (test)" ;;
  "models "*)
    if [ -f "$H/auth.json" ]; then printf 'Default model: grok-test\nAvailable models:\n  * grok-test (default)\n'
    else echo "Not authenticated. Run grok login"; exit 1; fi ;;
  "login --device-auth")
    echo $$ > "$H/login.pid"
    printf '\nTo sign in, open this URL in your browser:\n\n  https://accounts.x.ai/oauth2/device?user_code={code}\n\n'
    printf 'Confirm this code in your browser:\n\n  {code}\n\n\033[90mOnly continue with a code you requested.\033[0m\n\nWaiting for authorization...\n'{wait}
  "logout ") rm -f "$H/auth.json"; echo "Signed out" ;;
esac
"#,
        home = home.display(),
        code = GROK_CODE,
        wait = WAIT_LOOP,
    )
}

struct Env {
    app: Router,
    codex_home: PathBuf,
    grok_home: PathBuf,
    _tmp: TempDir,
}

async fn setup_with(timeout: Duration) -> Env {
    logs();
    let tmp = tempdir().unwrap();
    let (codex_home, grok_home) = (tmp.path().join("codex-home"), tmp.path().join("grok-home"));
    let (codex_bin, grok_bin) = (tmp.path().join("codex"), tmp.path().join("grok"));
    exe::write_executable(&codex_bin, fake_codex(&codex_home));
    exe::write_executable(&grok_bin, fake_grok(&grok_home));
    let db = Database::init(tmp.path()).await.unwrap();
    let config = Config { data_dir: tmp.path().to_path_buf(), ..Config::default() };
    let mut state = AppState::new(
        config.clone(),
        db,
        ToolRunner::new(tmp.path().join("workspace")),
        PermissionBroker::new(),
        LayaRouter::new(config.laya_url.clone()),
    );
    state.api_token = Arc::new(ApiToken::for_tests("device-login-test-token-0123456789", Require::Auto, config.port));
    let bin = |p: &Path| Some(p.to_string_lossy().into_owned());
    state.codex_login = Arc::new(EngineLogin::for_engine(
        Engine::Codex,
        bin(&codex_bin),
        Some(codex_home.clone()),
        LoginManager::with_timeout(timeout),
    ));
    state.grok_login = Arc::new(EngineLogin::for_engine(
        Engine::Grok,
        bin(&grok_bin),
        Some(grok_home.clone()),
        LoginManager::with_timeout(timeout),
    ));
    Env { app: create_router(state), codex_home, grok_home, _tmp: tmp }
}

async fn setup() -> Env {
    setup_with(Duration::from_secs(600)).await
}

async fn call(app: &Router, method: &str, uri: &str, body: Option<Value>) -> (StatusCode, Value) {
    let b = Request::builder().method(method).uri(uri).header("content-type", "application/json");
    let resp = app
        .clone()
        .oneshot(b.body(body.map(|v| Body::from(v.to_string())).unwrap_or_else(Body::empty)).unwrap())
        .await
        .unwrap();
    let status = resp.status();
    let bytes = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    (status, serde_json::from_slice(&bytes).unwrap_or(Value::Null))
}

fn engine_status(engines: &Value, id: &str) -> String {
    engines["engines"]
        .as_array()
        .unwrap()
        .iter()
        .find(|e| e["id"] == id)
        .map(|e| e["status"].as_str().unwrap_or_default().to_string())
        .unwrap()
}

fn login_pid(home: &Path) -> String {
    std::fs::read_to_string(home.join("login.pid")).unwrap().trim().to_string()
}

fn alive(pid: &str) -> bool {
    std::process::Command::new("kill").args(["-0", pid]).status().unwrap().success()
}

async fn wait_state(app: &Router, engine: &str, sid: &str, want: &str) -> Value {
    let mut last = Value::Null;
    for _ in 0..100 {
        let (s, v) = call(app, "GET", &format!("/api/engines/{engine}/login/{sid}"), None).await;
        assert_eq!(s, StatusCode::OK, "{v}");
        if v["state"] == want {
            return v;
        }
        last = v;
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    panic!("{engine} never reached {want}: {last}");
}

fn minutes_left(v: &Value) -> f64 {
    let at = chrono::DateTime::parse_from_rfc3339(v["expires_at"].as_str().unwrap()).unwrap();
    (at.with_timezone(&chrono::Utc) - chrono::Utc::now()).num_seconds() as f64 / 60.0
}

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
async fn chatgpt_sign_in_shows_the_device_code_and_finishes_by_itself() {
    let env = setup().await;
    let app = &env.app;

    let (s, v) = call(app, "POST", "/api/engines/codex/login", None).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    // The first answer already has both the link and the code
    assert_eq!(v["state"], "waiting_for_approval", "{v}");
    assert_eq!((v["engine"].as_str(), v["kind"].as_str()), (Some("codex"), Some("device_code")), "{v}");
    assert_eq!(v["url"], "https://auth.openai.com/codex/device", "{v}");
    assert_eq!(v["user_code"], CODEX_CODE, "{v}");
    // The expiry is the code's own: Codex says 15 minutes
    let left = minutes_left(&v);
    assert!((14.5..=15.1).contains(&left), "{left} minutes left: {v}");
    let sid = v["session_id"].as_str().unwrap().to_string();
    assert!(env.codex_home.is_dir(), "CODEX_HOME was not created for the CLI");

    // A pasted code means nothing here
    let (s, _) = call(app, "POST", &format!("/api/engines/codex/login/{sid}/code"), Some(json!({ "code": "abcdefgh123" }))).await;
    assert_eq!(s, StatusCode::BAD_REQUEST);

    // The person approves on chatgpt.com: kwnt notices with no button press
    std::fs::write(env.codex_home.join("approve"), "").unwrap();
    let done = wait_state(app, "codex", &sid, "done").await;
    assert_eq!((&done["url"], &done["user_code"], &done["detail"]), (&Value::Null, &Value::Null, &Value::Null), "{done}");
    let (_, engines) = call(app, "GET", "/api/engines", None).await;
    assert_eq!(engine_status(&engines, "codex"), "ready", "{engines}");

    let logs = String::from_utf8(logs().lock().unwrap().clone()).unwrap();
    assert!(logs.contains("codex engine sign-in started"), "{logs}");
    assert!(logs.contains("codex engine sign-in finished"), "{logs}");
    for secret in [CODEX_CODE, GROK_CODE, "user_code"] {
        assert!(!logs.contains(secret), "{secret} was logged:\n{logs}");
    }

    // Sign out goes through the CLI's own `logout`
    let (s, engines) = call(app, "POST", "/api/engines/codex/logout", None).await;
    assert_eq!(s, StatusCode::OK, "{engines}");
    assert_eq!(engine_status(&engines, "codex"), "signed_out", "{engines}");
}

#[tokio::test]
async fn grok_sign_in_shows_its_own_link_and_code_and_finishes_by_itself() {
    let env = setup().await;
    let app = &env.app;
    let (s, v) = call(app, "POST", "/api/engines/grok/login", None).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(v["state"], "waiting_for_approval", "{v}");
    assert_eq!(v["kind"], "device_code");
    // The link as the CLI prints it, code in the query included
    assert_eq!(v["url"], format!("https://accounts.x.ai/oauth2/device?user_code={GROK_CODE}"), "{v}");
    assert_eq!(v["user_code"], GROK_CODE, "{v}");
    // Grok prints no lifetime: the sign-in's own timeout
    let left = minutes_left(&v);
    assert!((9.5..=10.1).contains(&left), "{left} minutes left: {v}");
    let sid = v["session_id"].as_str().unwrap().to_string();

    std::fs::write(env.grok_home.join("approve"), "").unwrap();
    wait_state(app, "grok", &sid, "done").await;
    let (_, engines) = call(app, "GET", "/api/engines", None).await;
    assert_eq!(engine_status(&engines, "grok"), "ready", "{engines}");
}

#[tokio::test]
async fn only_one_device_sign_in_runs_per_engine_and_cancel_kills_it() {
    let env = setup().await;
    let app = &env.app;
    let (_, v) = call(app, "POST", "/api/engines/codex/login", None).await;
    let sid = v["session_id"].as_str().unwrap().to_string();
    let pid = login_pid(&env.codex_home);
    assert!(alive(&pid));

    let (s, busy) = call(app, "POST", "/api/engines/codex/login", None).await;
    assert_eq!(s, StatusCode::CONFLICT, "{busy}");
    assert_eq!(busy["session_id"], sid.as_str());
    // Grok's sign-in is separate and may run meanwhile
    let (s, g) = call(app, "POST", "/api/engines/grok/login", None).await;
    assert_eq!(s, StatusCode::OK, "{g}");
    // Another engine's session id isn't found under this engine
    let (s, _) = call(app, "GET", &format!("/api/engines/grok/login/{sid}"), None).await;
    assert_eq!(s, StatusCode::NOT_FOUND);

    let (s, _) = call(app, "DELETE", &format!("/api/engines/codex/login/{sid}"), None).await;
    assert_eq!(s, StatusCode::NO_CONTENT);
    let v = wait_state(app, "codex", &sid, "cancelled").await;
    assert_eq!((&v["url"], &v["user_code"]), (&Value::Null, &Value::Null), "{v}");
    assert!(!alive(&pid), "the CLI is still running after cancel");
    assert!(!env.codex_home.join("auth.json").exists());

    let (s, v) = call(app, "POST", "/api/engines/codex/login", None).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_ne!(v["session_id"].as_str().unwrap(), sid);
    let gsid = g["session_id"].as_str().unwrap();
    call(app, "DELETE", &format!("/api/engines/grok/login/{gsid}"), None).await;
    call(app, "DELETE", &format!("/api/engines/codex/login/{}", v["session_id"].as_str().unwrap()), None).await;
}

#[tokio::test]
async fn a_code_the_cli_says_expired_is_expired_not_failed() {
    let env = setup().await;
    let app = &env.app;
    let (_, v) = call(app, "POST", "/api/engines/codex/login", None).await;
    let sid = v["session_id"].as_str().unwrap().to_string();
    std::fs::write(env.codex_home.join("expire"), "").unwrap();
    let v = wait_state(app, "codex", &sid, "expired").await;
    assert!(v["detail"].as_str().unwrap().contains("code expired"), "{v}");
    assert_eq!((&v["url"], &v["user_code"]), (&Value::Null, &Value::Null), "{v}");
}

#[tokio::test]
async fn an_unanswered_device_sign_in_expires_and_its_cli_is_killed() {
    let env = setup_with(Duration::from_millis(800)).await;
    let app = &env.app;
    let (_, v) = call(app, "POST", "/api/engines/grok/login", None).await;
    assert_eq!(v["state"], "waiting_for_approval", "{v}");
    let sid = v["session_id"].as_str().unwrap().to_string();
    let pid = login_pid(&env.grok_home);
    let v = wait_state(app, "grok", &sid, "expired").await;
    assert!(v["detail"].as_str().unwrap().contains("Get a new code"), "{v}");
    assert!(!alive(&pid), "the CLI outlived its sign-in");
}

#[tokio::test]
async fn a_sign_in_the_engine_still_cant_use_fails_with_the_reason() {
    let env = setup().await;
    let app = &env.app;
    std::fs::create_dir_all(&env.codex_home).unwrap();
    std::fs::write(env.codex_home.join("unusable"), "").unwrap();
    let (_, v) = call(app, "POST", "/api/engines/codex/login", None).await;
    let sid = v["session_id"].as_str().unwrap().to_string();
    std::fs::write(env.codex_home.join("approve"), "").unwrap();
    let v = wait_state(app, "codex", &sid, "failed").await;
    assert_eq!(
        v["detail"],
        "Signed in, but Codex can't use this account: it still shows as signed out",
        "{v}"
    );
}

#[tokio::test]
async fn engines_without_an_in_app_sign_in_are_404() {
    let env = setup().await;
    let app = &env.app;
    for engine in ["custom", "ollama", "openai_api", "nope"] {
        let (s, v) = call(app, "POST", &format!("/api/engines/{engine}/login"), None).await;
        assert_eq!(s, StatusCode::NOT_FOUND, "{engine}: {v}");
        let (s, _) = call(app, "POST", &format!("/api/engines/{engine}/logout"), None).await;
        assert_eq!(s, StatusCode::NOT_FOUND, "{engine}");
    }
    for (m, uri) in [("GET", "/api/engines/codex/login/login_nope"), ("DELETE", "/api/engines/grok/login/login_nope")] {
        assert_eq!(call(app, m, uri, None).await.0, StatusCode::NOT_FOUND, "{m} {uri}");
    }
    assert!(!env.codex_home.join("login.pid").exists());
}
