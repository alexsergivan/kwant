//! 0.9.5 (#9) "Runs never stop silently": a bot's turn that ends with its browser page on a
//! sign-in form it didn't get past, and that asked the person nothing, leaves one specific ask:
//! "<Bot> needs you to sign in to <site> in the browser panel, then it continues." It is a sign-in
//! step on that turn's own job page (0.9.3 per-job pages), listed in Needs you, and its Done
//! starts the bot again. The same site on the same page is asked about once per 12 h, so a bot
//! that keeps landing on the form doesn't pile up asks. A fake claude CLI on PATH drives the
//! browser through the shell API (curl) like a bot without kwnt's tools. Its own test binary: it
//! changes PATH. Skipped when no Chrome or Chromium is installed.

#[path = "common/exe.rs"]
mod exe;
#[path = "support/chrome_guard.rs"]
mod chrome_guard;

use axum::{
    body::{to_bytes, Body},
    http::{Request, StatusCode},
    Router,
};
use chrome_guard::ChromeGuard;
use kwnt::{
    broker::PermissionBroker,
    config::Config,
    db::{human_steps::HumanStep, Database},
    router::LayaRouter,
    sandbox::computer,
    server::{create_router, AppState},
    tools::ToolRunner,
};
use serde_json::{json, Value};
use std::path::PathBuf;
use std::sync::OnceLock;
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tower::ServiceExt;

/// Records its arguments to `$KWNT_TEST_BEHAVE/<bot>.prompt`; with `<bot>.curl` holding a page
/// address, opens it through the browser API from the system prompt (saving the HTTP status to
/// `<bot>.status`), then ends its turn without asking anyone anything.
const FAKE_CLAUDE: &str = r#"#!/bin/sh
case "$1" in
  auth) echo '{"loggedIn":true}'; exit 0 ;;
esac
me=$(printf '%s\n' "$@" | sed -n 's/.*You are \([A-Za-z0-9]*\), .*/\1/p' | head -n 1)
dir="$KWNT_TEST_BEHAVE"
printf '%s\n' "$@" > "$dir/$me.prompt"
if [ -f "$dir/$me.curl" ]; then
  api=$(printf '%s\n' "$@" | sed -n 's/^Browser API: \(http[^ ]*\).*/\1/p' | head -n 1)
  url=$(cat "$dir/$me.curl")
  curl -sS -o /dev/null -w '%{http_code}' -X POST "$api/navigate" -H 'Content-Type: application/json' -d "{\"url\":\"$url\"}" > "$dir/$me.status"
fi
echo '{"type":"result","is_error":false,"num_turns":1,"result":"The page wants a login, so I stopped there."}'
"#;

const LOGIN_PAGE: &str = r#"<!doctype html><html><head><title>Log in</title></head><body>
<form><label>Email <input type="email" name="email"></label>
<label>Password <input type="password" name="password"></label><button>Log in</button></form>
</body></html>"#;

const PLAIN_PAGE: &str = r#"<!doctype html><html><head><title>Prices</title></head><body>
<h1>Prices</h1><p>Plain page, no form.</p><input type="password" style="display:none"></body></html>"#;

fn behave_dir() -> &'static PathBuf {
    static DIR: OnceLock<PathBuf> = OnceLock::new();
    DIR.get_or_init(|| {
        let root = tempfile::tempdir().unwrap().keep();
        let (bin, behave) = (root.join("bin"), root.join("behave"));
        for d in [&bin, &behave] {
            std::fs::create_dir_all(d).unwrap();
        }
        exe::write_executable(bin.join("claude"), FAKE_CLAUDE);
        std::env::set_var("KWNT_TEST_BEHAVE", &behave);
        std::env::set_var("PATH", format!("{}:{}", bin.display(), std::env::var("PATH").unwrap_or_default()));
        behave
    })
}

fn read(bot: &str, what: &str) -> String {
    std::fs::read_to_string(behave_dir().join(format!("{bot}.{what}"))).unwrap_or_default()
}

/// Serves `/login` and `/plain` on 127.0.0.1.
async fn serve_pages() -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    tokio::spawn(async move {
        while let Ok((mut stream, _)) = listener.accept().await {
            tokio::spawn(async move {
                let mut buf = [0u8; 4096];
                let n = stream.read(&mut buf).await.unwrap_or(0);
                let request = String::from_utf8_lossy(&buf[..n]);
                let path = request.split_whitespace().nth(1).unwrap_or("/").to_string();
                let (status, body) = match path.as_str() {
                    "/login" => ("200 OK", LOGIN_PAGE),
                    "/plain" => ("200 OK", PLAIN_PAGE),
                    _ => ("404 Not Found", ""),
                };
                let reply = format!("HTTP/1.1 {status}\r\ncontent-type: text/html\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}", body.len());
                let _ = stream.write_all(reply.as_bytes()).await;
            });
        }
    });
    format!("http://127.0.0.1:{port}")
}

struct Env {
    app: Router,
    state: AppState,
    site: String,
    _chrome: ChromeGuard,
    _tmp: tempfile::TempDir,
}

/// kwnt on a real port (the shell API is curled), with a windowless Chrome as the VM browser and
/// kwnt's own agent tools off (the bot only has curl).
async fn setup() -> Option<Env> {
    behave_dir();
    if computer::local_chrome_path().is_none() {
        eprintln!("skipped: no Chrome or Chromium installed");
        return None;
    }
    let tmp = tempfile::tempdir().unwrap();
    let profile = tmp.path().join("chrome");
    let (endpoint, _) = computer::ensure_local_chrome(&profile, true).await.expect("Chrome starts");
    let chrome = ChromeGuard(profile);
    std::env::set_var("KWNT_COMPUTER_MODE", "vm");
    let data = tmp.path().join("data");
    let db = Database::init(&data).await.unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let config = Config {
        data_dir: data.clone(),
        port: listener.local_addr().unwrap().port(),
        laya_url: "http://127.0.0.1:9".into(),
        browser_cdp_url: Some(endpoint),
        ..Config::default()
    };
    let state = AppState::new(config.clone(), db, ToolRunner::new(data.join("workspace")), PermissionBroker::new(), LayaRouter::new(config.laya_url.clone()));
    let app = create_router(state.clone());
    let served = app.clone();
    tokio::spawn(async move { axum::serve(listener, served).await.unwrap() });
    let env = Env { app, state, site: serve_pages().await, _chrome: chrome, _tmp: tmp };
    let (s, v) = call(&env.app, "POST", "/api/settings", Some(json!({"agent_tools": false}))).await;
    assert!(s.is_success(), "{s} {v}");
    Some(env)
}

async fn call(app: &Router, method: &str, uri: &str, body: Option<Value>) -> (StatusCode, Value) {
    let req = Request::builder().method(method).uri(uri).header("content-type", "application/json");
    let resp = app.clone().oneshot(req.body(body.map(|v| Body::from(v.to_string())).unwrap_or_else(Body::empty)).unwrap()).await.unwrap();
    let s = resp.status();
    let bytes = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    (s, serde_json::from_slice(&bytes).unwrap_or(Value::Null))
}

async fn add_bot(env: &Env, name: &str) -> String {
    let (s, v) = call(&env.app, "POST", "/api/kwants", Some(json!({"name": name, "title": "Researcher", "persona_prompt": "", "soul": "", "avatar_sprite": "default", "driver_type": "cli", "preferred_model": "claude"}))).await;
    assert!(s.is_success(), "{s} {v}");
    v["id"].as_str().or(v["kwant"]["id"].as_str()).unwrap().to_string()
}

/// Waits until no turn runs (for a while).
async fn settle(env: &Env) {
    let mut quiet = 0;
    for _ in 0..600 {
        quiet = if env.state.turns.active_turns().is_empty() { quiet + 1 } else { 0 };
        if quiet >= 10 {
            return;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    panic!("the turn never ended");
}

async fn steps_of(env: &Env, bot: &str, status: Option<&str>) -> Vec<HumanStep> {
    env.state.db.list_human_steps(status, None, None).await.unwrap().into_iter().filter(|s| s.kwant_id == bot).collect()
}

async fn send(env: &Env, bot: &str, prompt: &str) {
    let (s, v) = call(&env.app, "POST", "/api/chat/send", Some(json!({"kwant_id": bot, "prompt": prompt, "sender_id": "user"}))).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    settle(env).await;
}

#[tokio::test]
async fn a_turn_that_ends_on_a_sign_in_form_asks_the_person_once_to_sign_in() {
    let Some(env) = setup().await else { return };
    let bot = add_bot(&env, "Ben").await;
    let login = format!("{}/login", env.site);
    std::fs::write(behave_dir().join("Ben.curl"), &login).unwrap();
    send(&env, &bot, "check my orders on the shop").await;
    assert_eq!(read("Ben", "status"), "200", "the bot opened the page");
    assert!(read("Ben", "prompt").contains(&format!("/api/sandbox/{bot}/bot/{bot}/chat/{bot}")), "its own job page: {}", read("Ben", "prompt"));

    // One specific ask: who, which site, where, and what happens next
    let open = steps_of(&env, &bot, Some("open")).await;
    assert_eq!(open.len(), 1, "{open:?}");
    let step = &open[0];
    assert_eq!((step.kind.as_str(), step.source.as_str()), ("sign_in", "auto"));
    assert_eq!(step.reason, "Ben needs you to sign in to 127.0.0.1 in the browser panel, then it continues.");
    assert_eq!(step.url.as_deref(), Some(login.as_str()));
    assert!(step.turn_id.is_some(), "the step belongs to the turn that ended on the form");
    // On the turn's own page (0.9.3): the team page stays free for other jobs
    let page = kwnt::server::browser_pages::job_page_key(&bot, &bot, &bot).unwrap();
    assert_eq!(step.page_key.as_deref(), Some(page.as_str()));
    assert_eq!(step.sandbox_target.as_deref(), Some(bot.as_str()));
    // Its card is in the chat
    let card = env.state.db.get_message(step.message_id.as_deref().unwrap()).await.unwrap().unwrap();
    assert!(env.state.db.list_messages(&bot, 50).await.unwrap().iter().any(|m| m.id == card.id));
    // And in Needs you, in the same words
    let (_, away) = call(&env.app, "GET", "/api/away", None).await;
    let items: Vec<_> = away["waiting"].as_array().unwrap().iter().filter(|i| i["kind"] == "stopped").cloned().collect();
    assert_eq!(items.len(), 1, "{away}");
    assert_eq!(items[0]["human_step_id"], step.id.as_str());
    assert_eq!(items[0]["summary"], step.reason.as_str());
    assert_eq!(items[0]["link"]["direct_kwant_id"], bot.as_str());

    // The person signs in and presses Done: a new turn picks the task up, ends on the same form
    // again (still logged out), and is not asked about it a second time within 12 h
    std::fs::remove_file(behave_dir().join("Ben.prompt")).unwrap();
    let (s, v) = call(&env.app, "POST", &format!("/api/human-steps/{}/done", step.id), None).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(v["resumed"], "new_turn");
    tokio::time::sleep(Duration::from_millis(300)).await;
    settle(&env).await;
    assert!(read("Ben", "prompt").contains("The person did the browser step you asked for earlier"));
    let all = steps_of(&env, &bot, None).await;
    let seen: Vec<_> = all.iter().map(|s| (&s.kind, &s.status, &s.source, &s.url, &s.page_key, &s.sandbox_target, &s.turn_id, s.created_at)).collect();
    assert!(all.iter().all(|s| s.status != "open"), "no second ask for the same site and page: {seen:?}");
    assert_eq!(all.len(), 1, "{seen:?}");
    let (_, away) = call(&env.app, "GET", "/api/away", None).await;
    assert!(!away["waiting"].as_array().unwrap().iter().any(|i| i["kind"] == "stopped"), "{away}");
}

#[tokio::test]
async fn a_turn_that_ends_on_a_page_without_a_sign_in_form_asks_nothing() {
    let Some(env) = setup().await else { return };
    let bot = add_bot(&env, "Cleo").await;
    std::fs::write(behave_dir().join("Cleo.curl"), format!("{}/plain", env.site)).unwrap();
    send(&env, &bot, "read the prices page").await;
    assert_eq!(read("Cleo", "status"), "200");
    assert!(steps_of(&env, &bot, None).await.is_empty(), "a hidden password field is no wall");
}

#[tokio::test]
async fn a_turn_that_never_opened_the_browser_asks_nothing() {
    let Some(env) = setup().await else { return };
    let bot = add_bot(&env, "Dora").await;
    send(&env, &bot, "just say hi").await;
    assert!(steps_of(&env, &bot, None).await.is_empty());
}
