//! A CAPTCHA kwnt meets outside a browser tool call still reaches the person as a "Human check
//! needed" card with a notification: when kwnt opens the person's link before the bot's turn, and
//! when a bot drives the browser through the shell API (curl, no kwnt tools). The bot is told the
//! person was asked, not to report a block or switch sites, and the person's answer starts a new
//! turn. A fake claude CLI on PATH records its prompt and, for a bot with a `curl` behaviour, calls
//! the browser API the prompt gives it. Its own test binary: it changes PATH. Skipped when no
//! Chrome or Chromium is installed.

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
    drivers::events::RuntimeEvent,
    router::LayaRouter,
    sandbox::computer,
    server::{create_router, AppState},
    tools::ToolRunner,
};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tower::ServiceExt;

/// Records its arguments to `$KWNT_TEST_BEHAVE/<bot>.prompt`; with `<bot>.wait` holding seconds,
/// sleeps that long first (so two turns overlap); with `<bot>.curl` holding a page address,
/// opens it twice through the browser API from the system prompt, saving each answer (and its
/// HTTP status) to `<bot>.curl1` / `<bot>.curl2`. A message saying `(task-<x>)` uses
/// `<bot>-<x>.*` instead, so one bot can behave differently in two threads.
const FAKE_CLAUDE: &str = r#"#!/bin/sh
case "$1" in
  auth) echo '{"loggedIn":true}'; exit 0 ;;
esac
me=$(printf '%s\n' "$@" | sed -n 's/.*You are \([A-Za-z0-9]*\), .*/\1/p' | head -n 1)
task=$(printf '%s\n' "$@" | sed -n 's/.*(task-\([a-z]*\)).*/\1/p' | head -n 1)
me="$me${task:+-$task}"
dir="$KWNT_TEST_BEHAVE"
printf '%s\n' "$@" > "$dir/$me.prompt"
if [ -f "$dir/$me.wait" ]; then sleep "$(cat "$dir/$me.wait")"; fi
if [ -f "$dir/$me.curl" ]; then
  api=$(printf '%s\n' "$@" | sed -n 's/^Browser API: \(http[^ ]*\).*/\1/p' | head -n 1)
  url=$(cat "$dir/$me.curl")
  for n in 1 2; do
    curl -sS -o "$dir/$me.curl$n" -w '%{http_code}' -X POST "$api/navigate" -H 'Content-Type: application/json' -d "{\"url\":\"$url\"}" > "$dir/$me.status$n"
  done
fi
echo '{"type":"result","is_error":false,"num_turns":1,"result":"Waiting for you to solve the check."}'
"#;

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
                let (status, body) = match (path.contains("..") || path.contains('/'), std::fs::read_to_string(dir.join(&path))) {
                    (false, Ok(body)) => ("200 OK", body),
                    _ => ("404 Not Found", String::new()),
                };
                let reply = format!("HTTP/1.1 {status}\r\ncontent-type: text/html\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}", body.len());
                let _ = stream.write_all(reply.as_bytes()).await;
            });
        }
    });
    format!("http://127.0.0.1:{port}/")
}

struct Env {
    app: Router,
    state: AppState,
    fixtures: String,
    _chrome: ChromeGuard,
    _tmp: tempfile::TempDir,
}

/// kwnt on a real port (the shell API is curled), with a windowless Chrome as the VM browser.
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
    // Long enough that nothing times out while the test looks
    state.human_steps.set_captcha_timing(Duration::from_secs(120), Duration::from_millis(200));
    let app = create_router(state.clone());
    let served = app.clone();
    tokio::spawn(async move { axum::serve(listener, served).await.unwrap() });
    Some(Env { app, state, fixtures: serve_fixtures().await, _chrome: chrome, _tmp: tmp })
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

fn open_steps(steps: Vec<HumanStep>, bot: &str) -> Vec<HumanStep> {
    steps.into_iter().filter(|s| s.kwant_id == bot).collect()
}

/// The person sees the step: one open CAPTCHA step of this bot's turn, its card in the bot's
/// chat, a `human_step` notification, and the browser handed to them.
async fn assert_asked(env: &Env, bot: &str, events: &mut tokio::sync::broadcast::Receiver<RuntimeEvent>) -> HumanStep {
    assert_asked_in(env, bot, bot, events).await
}

/// [`assert_asked`] for a turn in `chat` (the bot's DM, or one of its threads).
async fn assert_asked_in(env: &Env, bot: &str, chat: &str, events: &mut tokio::sync::broadcast::Receiver<RuntimeEvent>) -> HumanStep {
    let steps = open_steps(env.state.db.list_human_steps(Some("open"), None, None).await.unwrap(), bot);
    assert_eq!(steps.len(), 1, "one card, not one per read: {steps:?}");
    let step = steps.into_iter().next().unwrap();
    assert_eq!((step.kind.as_str(), step.source.as_str()), ("captcha", "auto"));
    assert!(step.turn_id.is_some(), "the step belongs to the turn that met the check");
    let card = env.state.db.get_message(step.message_id.as_deref().unwrap()).await.unwrap().unwrap();
    assert!(card.text.starts_with("Human check needed: "), "{}", card.text);
    assert!(card.text.contains("Solve it in the browser panel"), "{}", card.text);
    let msgs = env.state.db.list_messages(chat, 50).await.unwrap();
    assert!(msgs.iter().any(|m| m.id == card.id), "the card is in the chat of the turn");
    let mut notified = false;
    while let Ok(ev) = events.try_recv() {
        if let RuntimeEvent::Notify { kind, kwant_id, title, .. } = ev {
            if kind == "human_step" && kwant_id == bot {
                assert!(title.ends_with("needs you in the browser"), "{title}");
                notified = true;
            }
        }
    }
    assert!(notified, "the person is notified");
    assert!(env.state.human_steps.turn_on(step.sandbox_target.as_deref().unwrap()).is_none(), "the turn ended and left the browser");
    step
}

/// The person's link opens on a CAPTCHA before the bot's turn starts: the card is up before the
/// bot runs, the bot hears the person was asked (not to report a block or try Bing), and Done
/// starts a new turn in the same chat.
#[tokio::test]
async fn a_link_the_person_sends_that_opens_on_a_captcha_asks_them_to_solve_it() {
    let Some(env) = setup().await else { return };
    let bot = add_bot(&env, "Wren").await;
    let mut events = env.state.event_sender.subscribe();
    let link = format!("{}px_wall.html", env.fixtures);
    let (s, v) = call(&env.app, "POST", "/api/chat/send", Some(json!({"kwant_id": bot, "prompt": format!("what does {link} say?"), "sender_id": "user"}))).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    settle(&env).await;

    let step = assert_asked(&env, &bot, &mut events).await;
    assert_eq!(step.url.as_deref(), Some(link.as_str()));
    let prompt = read("Wren", "prompt");
    assert!(prompt.contains("\"Human check needed\" card"), "{prompt}");
    assert!(prompt.contains("End your turn now"), "{prompt}");
    assert!(!prompt.contains("retry once on another engine"), "no route around the check: {prompt}");

    // The person solves it and presses Done: a new turn picks the task up
    std::fs::remove_file(behave_dir().join("Wren.prompt")).unwrap();
    let (s, v) = call(&env.app, "POST", &format!("/api/human-steps/{}/done", step.id), None).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(v["resumed"], "new_turn");
    tokio::time::sleep(Duration::from_millis(300)).await;
    settle(&env).await;
    assert!(read("Wren", "prompt").contains("The person did the browser step you asked for earlier"));
}

/// A bot without kwnt's tools (agent tools off) opens a CAPTCHA page with curl: the person gets
/// the card in that turn's chat, the bot is told so, and its next browser action waits for them.
#[tokio::test]
async fn a_captcha_met_through_the_shell_browser_api_asks_the_person() {
    let Some(env) = setup().await else { return };
    let (s, v) = call(&env.app, "POST", "/api/settings", Some(json!({"agent_tools": false}))).await;
    assert!(s.is_success(), "{s} {v}");
    let bot = add_bot(&env, "Ossi").await;
    let link = format!("{}px_wall.html", env.fixtures);
    std::fs::write(behave_dir().join("Ossi.curl"), &link).unwrap();
    let mut events = env.state.event_sender.subscribe();
    let (s, v) = call(&env.app, "POST", "/api/chat/send", Some(json!({"kwant_id": bot, "prompt": "check the store page for me", "sender_id": "user"}))).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    settle(&env).await;

    let prompt = read("Ossi", "prompt");
    assert!(prompt.contains("Browser API: http://127.0.0.1:"), "the bot drives the browser with curl: {prompt}");
    assert!(prompt.contains(&format!("/api/sandbox/{bot}/bot/{bot}/chat/{bot}")), "its own copy of the API names it and its chat: {prompt}");
    assert_eq!(read("Ossi", "status1"), "200");
    let first: Value = serde_json::from_str(&read("Ossi", "curl1")).unwrap();
    assert_eq!(first["blocked"], true, "{first}");
    assert_eq!(first["human_step"], "asked", "{first}");
    assert!(first["notice"].as_str().unwrap().contains("\"Human check needed\" card"), "{first}");
    // The browser is the person's until they finish
    assert_eq!(read("Ossi", "status2"), "409");
    assert!(read("Ossi", "curl2").contains("waiting on the person"));
    assert_asked(&env, &bot, &mut events).await;

    // The person's own panel (the plain route) is told where their step is, not told to end a turn
    let (s, v) = call(&env.app, "POST", &format!("/api/sandbox/{bot}/navigate"), Some(json!({"url": format!("{}px_wall.html", env.fixtures)}))).await;
    assert_eq!(s, StatusCode::CONFLICT, "{v}");
    let error = v["error"].as_str().unwrap();
    assert_eq!(error, kwnt::server::routes::HELD_FOR_PERSON);
    assert!(error.contains("Take control") && !error.contains("end your turn"), "{error}");
}

/// Two bots of one team share its browser and run at once (Rigel's N3, Oleks's "mixing
/// contexts"): each works in its own team thread, the older turn's curl meets the CAPTCHA while
/// the newer bot's turn is still running. The card, its notification and the browser hold are
/// the older bot's, in the older bot's thread; the newer bot's thread gets nothing.
#[tokio::test]
async fn a_captcha_one_bot_meets_on_a_shared_team_browser_is_asked_in_that_bots_chat() {
    let Some(env) = setup().await else { return };
    let (s, v) = call(&env.app, "POST", "/api/settings", Some(json!({"agent_tools": false}))).await;
    assert!(s.is_success(), "{s} {v}");
    let (older, newer) = (add_bot(&env, "Tove").await, add_bot(&env, "Ulla").await);
    let (s, team) = call(&env.app, "POST", "/api/teams", Some(json!({"name": "shoppers", "lead_kwant_id": older, "member_kwants": [older, newer]}))).await;
    assert!(s.is_success(), "{s} {team}");
    let team_id = team["id"].as_str().or(team["team"]["id"].as_str()).unwrap().to_string();
    let tove = env.state.db.create_thread(&older, "Store check", "chat", Some(&team_id), "user").await.unwrap().id;
    let ulla = env.state.db.create_thread(&newer, "Price list", "chat", Some(&team_id), "user").await.unwrap().id;
    // Tove starts first and curls the wall after 3 s; Ulla starts later and is still running then
    std::fs::write(behave_dir().join("Tove.wait"), "3").unwrap();
    std::fs::write(behave_dir().join("Tove.curl"), format!("{}px_wall.html", env.fixtures)).unwrap();
    std::fs::write(behave_dir().join("Ulla.wait"), "8").unwrap();
    let mut events = env.state.event_sender.subscribe();
    let send = |thread: &str| json!({"thread_id": thread, "prompt": "check the store page for me", "sender_id": "user"});
    let (s, v) = call(&env.app, "POST", "/api/chat/send", Some(send(&tove))).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    for _ in 0..50 {
        if env.state.human_steps.turn_on(&team_id).is_some() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    let (s, v) = call(&env.app, "POST", "/api/chat/send", Some(send(&ulla))).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    for _ in 0..50 {
        if env.state.human_steps.turn_on(&team_id).is_some_and(|t| t.kwant_id == newer) {
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert_eq!(env.state.human_steps.turn_on(&team_id).map(|t| t.kwant_id), Some(newer.clone()), "Ulla's turn is the newest on the team browser");
    assert_eq!(env.state.human_steps.turn_of(&team_id, Some(kwnt::server::human_steps::Caller { kwant: &older, chat: None })).map(|t| t.kwant_id), Some(older.clone()), "Tove's turn is still running");
    settle(&env).await;

    assert!(read("Tove", "prompt").contains(&format!("/api/sandbox/{team_id}/bot/{older}/chat/{tove}")), "{}", read("Tove", "prompt"));
    assert!(read("Ulla", "prompt").contains(&format!("/api/sandbox/{team_id}/bot/{newer}/chat/{ulla}")), "both drive the team's browser");
    let first: Value = serde_json::from_str(&read("Tove", "curl1")).unwrap();
    assert_eq!((first["blocked"].clone(), first["human_step"].clone()), (json!(true), json!("asked")), "{first}");
    let step = assert_asked_in(&env, &older, &tove, &mut events).await;
    assert_eq!(step.sandbox_target.as_deref(), Some(team_id.as_str()));
    assert_eq!(step.thread_id.as_deref(), Some(tove.as_str()), "the step is Tove's thread's");
    let all = env.state.db.list_human_steps(None, None, None).await.unwrap();
    assert!(all.iter().all(|s| s.kwant_id == older), "nothing is asked for Ulla: {all:?}");
    for chat in [&ulla, &newer] {
        let msgs = env.state.db.list_messages(chat, 50).await.unwrap();
        assert!(!msgs.iter().any(|m| m.text.starts_with("Human check needed")), "no card in Ulla's chats: {msgs:?}");
    }
}

/// One bot runs two team threads on the team's browser at once (Rigel's N3 on #298): thread A's
/// curl meets the CAPTCHA while the same bot's turn in thread B, started later, is still running.
/// The card, its notification and the step are thread A's; thread B and the bot's DM get nothing.
/// Before the address named the thread, the card went to thread B (the bot's newest turn).
#[tokio::test]
async fn a_captcha_met_in_one_of_a_bots_two_threads_on_a_team_browser_is_asked_in_that_thread() {
    let Some(env) = setup().await else { return };
    let (s, v) = call(&env.app, "POST", "/api/settings", Some(json!({"agent_tools": false}))).await;
    assert!(s.is_success(), "{s} {v}");
    let bot = add_bot(&env, "Vesa").await;
    let (s, team) = call(&env.app, "POST", "/api/teams", Some(json!({"name": "checkers", "lead_kwant_id": bot, "member_kwants": [bot]}))).await;
    assert!(s.is_success(), "{s} {team}");
    let team_id = team["id"].as_str().or(team["team"]["id"].as_str()).unwrap().to_string();
    let thread_a = env.state.db.create_thread(&bot, "Store check", "chat", Some(&team_id), "user").await.unwrap().id;
    let thread_b = env.state.db.create_thread(&bot, "Price list", "chat", Some(&team_id), "user").await.unwrap().id;
    // Thread A starts first and curls the wall after 3 s; thread B starts later and is still running then
    std::fs::write(behave_dir().join("Vesa-a.wait"), "3").unwrap();
    std::fs::write(behave_dir().join("Vesa-a.curl"), format!("{}px_wall.html", env.fixtures)).unwrap();
    std::fs::write(behave_dir().join("Vesa-b.wait"), "8").unwrap();
    let mut events = env.state.event_sender.subscribe();
    let send = |thread: &str, task: &str| json!({"thread_id": thread, "prompt": format!("check the store page for me (task-{task})"), "sender_id": "user"});
    let newest = |env: &Env| env.state.human_steps.turn_on(&team_id).and_then(|t| t.origin).and_then(|o| o.thread_id);
    let (s, v) = call(&env.app, "POST", "/api/chat/send", Some(send(&thread_a, "a"))).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    for _ in 0..50 {
        if newest(&env).is_some() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    let (s, v) = call(&env.app, "POST", "/api/chat/send", Some(send(&thread_b, "b"))).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    for _ in 0..50 {
        if newest(&env).as_deref() == Some(thread_b.as_str()) {
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert_eq!(newest(&env).as_deref(), Some(thread_b.as_str()), "thread B's turn is the newest on the team browser");
    let running: Vec<_> = env.state.turns.active_turns().into_iter().filter(|t| t.state == "running").filter_map(|t| t.thread_id).collect();
    assert!(running.contains(&thread_a) && running.contains(&thread_b), "both threads' turns are running: {running:?}");
    settle(&env).await;

    // Each thread's turn was given an address naming its own thread, the same bot in both
    assert!(read("Vesa-a", "prompt").contains(&format!("/api/sandbox/{team_id}/bot/{bot}/chat/{thread_a}")), "{}", read("Vesa-a", "prompt"));
    assert!(read("Vesa-b", "prompt").contains(&format!("/api/sandbox/{team_id}/bot/{bot}/chat/{thread_b}")), "{}", read("Vesa-b", "prompt"));
    assert_eq!(read("Vesa-a", "status1"), "200");
    let first: Value = serde_json::from_str(&read("Vesa-a", "curl1")).unwrap();
    assert_eq!((first["blocked"].clone(), first["human_step"].clone()), (json!(true), json!("asked")), "{first}");
    assert_eq!(read("Vesa-a", "status2"), "409", "the held browser waits for the person");
    let step = assert_asked_in(&env, &bot, &thread_a, &mut events).await;
    assert_eq!(step.sandbox_target.as_deref(), Some(team_id.as_str()));
    assert_eq!(step.thread_id.as_deref(), Some(thread_a.as_str()), "the step is thread A's");
    for chat in [&thread_b, &bot] {
        let msgs = env.state.db.list_messages(chat, 50).await.unwrap();
        assert!(!msgs.iter().any(|m| m.text.starts_with("Human check needed")), "no card in thread B or the DM: {msgs:?}");
    }
}

/// Without a turn on the browser (someone curling it by hand) nobody is asked, and the answer
/// keeps the plain notice, which never tells the bot to solve or go around the check.
#[tokio::test]
async fn the_browser_api_asks_nobody_without_a_turn() {
    let Some(env) = setup().await else { return };
    let bot = add_bot(&env, "Pim").await;
    let (s, page) = call(&env.app, "POST", &format!("/api/sandbox/{bot}/navigate"), Some(json!({"url": format!("{}px_wall.html", env.fixtures)}))).await;
    assert_eq!(s, StatusCode::OK, "{page}");
    assert_eq!(page["blocked"], true, "{page}");
    assert!(page.get("human_step").is_none(), "{page}");
    let notice = page["notice"].as_str().unwrap();
    assert!(notice.contains("never try to solve, bypass or click through the check"), "{notice}");
    assert!(notice.contains("request_human_step"), "{notice}");
    assert!(env.state.db.list_human_steps(Some("open"), None, None).await.unwrap().is_empty());
}
