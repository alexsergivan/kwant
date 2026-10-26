//! A live kwant for checking the CAPTCHA hand-off in a real browser (Playwright, or by hand on a
//! phone): a windowless Chrome as the bot browser, the UI from `frontend/dist`, and a bot whose
//! turn lands on the local TikTok-style slider fixture, so a captcha step with a box is open.
//!
//! Opt in: build the frontend, then
//! `KWNT_UI_HARNESS_DIR=/tmp/harness cargo test --test captcha_ui_harness -- --ignored --nocapture`.
//! It writes `ready.json` ({url, bot_id, bot_name}) to that folder. Each `rearm-<n>` file put
//! there starts a new turn that opens the slider again; a `stop` file (or 20 minutes) ends it.

#[path = "support/chrome_guard.rs"]
mod chrome_guard;
use axum::{
    body::{to_bytes, Body},
    http::Request,
    Router,
};
use chrome_guard::ChromeGuard;
use kwnt::{
    broker::PermissionBroker,
    config::Config,
    db::{spaces::SpaceId, Database},
    router::LayaRouter,
    sandbox::computer,
    server::{
        continuity::{SelfScope, TurnOrigin},
        create_router, mcp_runtime, AppState,
    },
    tools::ToolRunner,
};
use serde_json::{json, Value};
use std::{
    collections::HashSet,
    path::{Path, PathBuf},
    time::{Duration, Instant},
};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tower::ServiceExt;

/// Serves `tests/fixtures/captcha/<name>` on 127.0.0.1.
async fn serve_fixtures() -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/captcha");
    tokio::spawn(async move {
        while let Ok((mut stream, _)) = listener.accept().await {
            let dir = dir.clone();
            tokio::spawn(async move {
                let mut buf = [0u8; 4096];
                let n = stream.read(&mut buf).await.unwrap_or(0);
                let request = String::from_utf8_lossy(&buf[..n]);
                let path = request.split_whitespace().nth(1).unwrap_or("/").trim_start_matches('/').to_string();
                let body = if path.contains("..") || path.contains('/') { None } else { std::fs::read_to_string(dir.join(&path)).ok() };
                let (status, body) = match body { Some(b) => ("200 OK", b), None => ("404 Not Found", String::new()) };
                let reply = format!("HTTP/1.1 {status}\r\ncontent-type: text/html\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}", body.len());
                let _ = stream.write_all(reply.as_bytes()).await;
            });
        }
    });
    format!("http://127.0.0.1:{port}/")
}

async fn turn(state: &AppState, turn_id: &str) -> String {
    let bot = state.db.list_kwants(&SpaceId::default_space()).await.unwrap().remove(0);
    let scope = SelfScope {
        kwant_id: bot.id.clone(),
        kwant_name: bot.name.clone(),
        origin: Some(TurnOrigin { direct_kwant_id: Some(bot.id.clone()), ..Default::default() }),
        cancel: Some(tokio_util::sync::CancellationToken::new()),
        turn_id: Some(turn_id.into()),
        sandbox_target: Some(bot.id.clone()),
        ..Default::default()
    };
    mcp_runtime::prepare_turn_with_self(state, &bot, Some(turn_id.into()), None, Some(scope)).await.unwrap().1
}

/// The bot opens the slider page, as the CLI would mid-turn; the result is logged when the turn resumes.
fn open_slider(app: &Router, token: String, url: String, label: String) {
    let app = app.clone();
    tokio::spawn(async move {
        let body = json!({"jsonrpc": "2.0", "id": 1, "method": "tools/call", "params": {"name": "browser_open", "arguments": {"url": url}}});
        let req = Request::builder().method("POST").uri("/api/mcp/self").header("content-type", "application/json")
            .header("authorization", format!("Bearer {token}")).body(Body::from(body.to_string())).unwrap();
        let resp = app.oneshot(req).await.unwrap();
        let v: Value = serde_json::from_slice(&to_bytes(resp.into_body(), usize::MAX).await.unwrap()).unwrap_or_default();
        let text = v["result"]["content"][0]["text"].as_str().unwrap_or("").chars().take(160).collect::<String>();
        eprintln!("harness: {label} resumed: {text}");
    });
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "manual: a live UI for Playwright or a phone; see the file comment"]
async fn captcha_handoff_ui_harness() {
    let dir = PathBuf::from(std::env::var("KWNT_UI_HARNESS_DIR").expect("set KWNT_UI_HARNESS_DIR"));
    std::fs::create_dir_all(&dir).unwrap();
    let tmp = tempfile::tempdir().unwrap();
    let profile = tmp.path().join("chrome");
    let (endpoint, _) = computer::ensure_local_chrome(&profile, true).await.expect("Chrome starts");
    let _chrome = ChromeGuard(profile);
    std::env::set_var("KWNT_COMPUTER_MODE", "vm");
    let db = Database::init(tmp.path()).await.unwrap();
    let config = Config { data_dir: tmp.path().to_path_buf(), browser_cdp_url: Some(endpoint), ..Config::default() };
    let state = AppState::new(config.clone(), db, ToolRunner::new(tmp.path().join("workspace")), PermissionBroker::new(), LayaRouter::new(config.laya_url));
    state.human_steps.set_captcha_timing(Duration::from_secs(600), Duration::from_millis(1000));
    let app = create_router(state.clone());
    let port: u16 = std::env::var("KWNT_UI_HARNESS_PORT").ok().and_then(|p| p.parse().ok()).unwrap_or(8846);
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", port)).await.unwrap();
    let serve = app.clone();
    tokio::spawn(async move { axum::serve(listener, serve).await.unwrap() });
    let fixtures = serve_fixtures().await;
    let bot = state.db.list_kwants(&SpaceId::default_space()).await.unwrap().remove(0);
    let slider = format!("{fixtures}tiktok_slider.html");
    open_slider(&app, turn(&state, "harness-0").await, slider.clone(), "turn 0".into());
    std::fs::write(dir.join("ready.json"), json!({"url": format!("http://127.0.0.1:{port}/"), "bot_id": bot.id, "bot_name": bot.name, "fixture": slider}).to_string()).unwrap();
    eprintln!("harness: http://127.0.0.1:{port}/ bot {} ({})", bot.name, bot.id);

    let started = Instant::now();
    let mut armed = HashSet::new();
    while started.elapsed() < Duration::from_secs(20 * 60) && !dir.join("stop").exists() {
        for entry in std::fs::read_dir(&dir).unwrap().flatten() {
            let name = entry.file_name().to_string_lossy().to_string();
            if name.starts_with("rearm-") && armed.insert(name.clone()) {
                open_slider(&app, turn(&state, &format!("harness-{name}")).await, slider.clone(), name);
            }
        }
        tokio::time::sleep(Duration::from_millis(300)).await;
    }
}
