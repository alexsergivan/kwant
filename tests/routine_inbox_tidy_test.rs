//! routine-inbox-tidy (Mira's UX pass 0.9.45, Minors b and g).
//! - b: an Inbox summary drops markdown bold markers anywhere in its line, not only at its start
//!   ("Shop, since yesterday 8:00:** 14 orders" read with the `**`).
//! - g: a routine that posts in one of its bot's threads opens that thread from its Inbox row and
//!   from its `routine_failed` notice, instead of the bot's main chat.
//!
//! A fake claude CLI that dies with no output makes a routine run fail; it changes PATH, so this
//! is its own test binary.

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
    db::{continuity::NewRoutine, models::{Kwant, NewMessage}, spaces::SpaceId, Database},
    drivers::events::RuntimeEvent,
    router::LayaRouter,
    server::{away::summarize, create_router, AppState},
    tools::ToolRunner,
};
use serde_json::{json, Value};
use std::path::PathBuf;
use std::sync::OnceLock;
use std::time::Duration;
use tempfile::{tempdir, TempDir};
use tower::ServiceExt;

/// Signed in, and dies with no output on every turn: each routine run ends `error`.
const FAKE_CLAUDE: &str = r#"#!/bin/sh
case "$1" in
  auth) echo '{"loggedIn":true}'; exit 0 ;;
esac
exit 3
"#;

const SECRET: &str = "sk-ant-api03-abcdefghijklmnopqrstuvwxyz0123456789";

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

async fn setup() -> (Router, AppState, TempDir, Kwant) {
    fake_cli();
    let tmp = tempdir().unwrap();
    let db = Database::init(tmp.path()).await.unwrap();
    db.set_setting("keep_going_turns", "0").await.unwrap();
    let config = Config {
        data_dir: tmp.path().to_path_buf(),
        laya_url: "http://127.0.0.1:9".into(),
        browser_cdp_url: Some("http://127.0.0.1:9".into()),
        ..Config::default()
    };
    let state = AppState::new(config.clone(), db, ToolRunner::new(tmp.path().join("workspace")), PermissionBroker::new(), LayaRouter::new(config.laya_url.clone()));
    let k = state.db.list_kwants(&SpaceId::default_space()).await.unwrap()[0].clone();
    (create_router(state.clone()), state, tmp, k)
}

async fn call(app: &Router, method: &str, uri: &str, body: Option<Value>) -> (StatusCode, Value) {
    let req = Request::builder().method(method).uri(uri).header("content-type", "application/json");
    let body = body.map(|v| Body::from(v.to_string())).unwrap_or_else(Body::empty);
    let resp = app.clone().oneshot(req.body(body).unwrap()).await.unwrap();
    let s = resp.status();
    let bytes = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    (s, serde_json::from_slice(&bytes).unwrap_or(Value::Null))
}

async fn get(app: &Router, uri: &str) -> Value {
    let (s, v) = call(app, "GET", uri, None).await;
    assert_eq!(s, StatusCode::OK, "{uri}: {v}");
    v
}

/// A routine of `k` posting in `thread` (or its direct chat), with one run that ended `status`
/// and left `reply` there. Returns the run id.
async fn ended_run(state: &AppState, k: &Kwant, name: &str, thread: Option<&str>, status: &str, reply: &str) -> String {
    let routine = state
        .db
        .create_routine(&NewRoutine {
            kwant_id: k.id.clone(),
            name: name.into(),
            instructions: "Sum up the shop's orders".into(),
            schedule: json!({"type": "interval", "every_minutes": 60}).to_string(),
            thread_id: thread.map(str::to_string),
            ..Default::default()
        })
        .await
        .unwrap();
    let run = state.db.start_routine_run(&routine.id, "schedule", "running", None).await.unwrap();
    let message = state
        .db
        .insert_message(&NewMessage {
            channel_id: None,
            direct_kwant_id: Some(k.id.clone()),
            sender_role: "kwant".into(),
            sender_id: k.id.clone(),
            text: reply.into(),
            reasoning_content: None,
            tool_calls: None,
            tokens_used: 1,
        })
        .await
        .unwrap();
    let error = (status != "ok").then_some("curl: (22) The requested URL returned error: 401");
    state.db.finish_routine_run(&run.id, status, error, Some(&message.id)).await.unwrap();
    run.id
}

/// The item `id` in `/api/away`'s `list` and in the Inbox (which must agree).
async fn rows(app: &Router, list: &str, id: &str) -> (Value, Option<Value>) {
    let away = get(app, "/api/away").await;
    let a = away[list].as_array().unwrap().iter().find(|i| i["id"] == id).cloned().unwrap_or_else(|| panic!("no away {list} row {id}: {away}"));
    let inbox = get(app, "/api/inbox").await;
    let b = inbox["items"].as_array().unwrap().iter().find(|i| i["id"] == id).cloned();
    (a, b)
}

#[test]
fn summaries_drop_bold_markers_anywhere_in_the_line() {
    // Mira's two snippets (c1-d-inbox)
    assert_eq!(summarize("**Shop, since yesterday 8:00:** 14 orders (€1,240)"), "Shop, since yesterday 8:00: 14 orders (€1,240)");
    assert_eq!(summarize("Shop, since yesterday 8:00:** 14 orders"), "Shop, since yesterday 8:00: 14 orders");
    assert_eq!(summarize("**Done: Morning shop digest** runs every weekday"), "Done: Morning shop digest runs every weekday");
    assert_eq!(summarize("Done: Morning shop digest** runs every weekday"), "Done: Morning shop digest runs every weekday");
    // Pairs in the middle, underscores, bold italic, a list bullet
    assert_eq!(summarize("Orders: **14**, refunds: **1**"), "Orders: 14, refunds: 1");
    assert_eq!(summarize("__Weekly__ report is __ready__"), "Weekly report is ready");
    assert_eq!(summarize("***Heads up:*** the shop is down"), "Heads up: the shop is down");
    assert_eq!(summarize("- **3 new orders** since 8:00"), "3 new orders since 8:00");
    // A line that is only markers is no summary line: the next one is
    assert_eq!(summarize("___\n**Done:** sent"), "Done: sent");
}

#[test]
fn single_markers_and_words_with_markers_inside_stay() {
    assert_eq!(summarize("Saved file_name and my_report.csv"), "Saved file_name and my_report.csv");
    assert_eq!(summarize("2*3 is 6, and *italic* stays"), "2*3 is 6, and *italic* stays");
    assert_eq!(summarize("2**10 is 1024 and snake__case stays"), "2**10 is 1024 and snake__case stays");
    // A fill-in blank is not a marker
    assert_eq!(summarize("Name: ____ please"), "Name: ____ please");
}

#[test]
fn a_secret_in_bold_is_still_masked_and_the_cut_is_unchanged() {
    let s = summarize(&format!("**Key:** {SECRET} was refused"));
    assert!(!s.contains(SECRET) && !s.contains("sk-ant"), "{s}");
    assert_eq!(s, "Key: [redacted] was refused");
    let s = summarize(&format!("The key **{SECRET}** was refused"));
    assert!(!s.contains("sk-ant") && !s.contains("**"), "{s}");
    let long = summarize(&format!("**{}**", "word ".repeat(100)));
    assert_eq!(long.chars().count(), kwnt::server::away::SUMMARY_CHARS);
    assert!(long.ends_with('…') && !long.contains('*'), "{long}");
}

#[tokio::test]
async fn a_thread_routines_runs_open_its_thread() {
    let (app, state, _tmp, k) = setup().await;
    let thread = state.db.create_thread(&k.id, "Shop digest", "chat", None, "user").await.unwrap().id;
    let failed = ended_run(&state, &k, "Morning shop digest", Some(&thread), "error", "The shop answered 401.").await;
    let (away, inbox) = rows(&app, "failed", &failed).await;
    let inbox = inbox.unwrap_or_else(|| panic!("no inbox row {failed}"));
    for row in [&away, &inbox] {
        assert_eq!(row["link"]["thread_id"], thread.as_str(), "{row}");
        // The rest of the link is as before, so an old client still lands in the bot's chat
        assert!(row["link"]["channel_id"].is_null(), "{row}");
        assert_eq!(row["link"]["direct_kwant_id"], k.id.as_str(), "{row}");
        // The thread is the bot's own: no other owner to look it up under
        assert!(row["link"].get("thread_kwant_id").is_none(), "{row}");
    }
    // A finished run of a thread routine reads from the same SELECT
    let ok = ended_run(&state, &k, "Evening shop digest", Some(&thread), "ok", "14 orders.").await;
    let (away, _) = rows(&app, "finished", &ok).await;
    assert_eq!(away["link"]["thread_id"], thread.as_str(), "{away}");
}

#[tokio::test]
async fn a_routine_without_a_thread_keeps_its_chat_link() {
    let (app, state, _tmp, k) = setup().await;
    let failed = ended_run(&state, &k, "Direct digest", None, "error", "The shop answered 401.").await;
    let (away, inbox) = rows(&app, "failed", &failed).await;
    for row in [Some(away), inbox].into_iter().flatten() {
        assert!(row["link"]["thread_id"].is_null(), "{row}");
        assert!(row["link"]["channel_id"].is_null(), "{row}");
        assert_eq!(row["link"]["direct_kwant_id"], k.id.as_str(), "{row}");
    }
}

/// The next `routine_failed` notice on `rx`, as (thread_id, channel_id).
async fn routine_failed(rx: &mut tokio::sync::broadcast::Receiver<RuntimeEvent>) -> (Option<String>, Option<String>) {
    tokio::time::timeout(Duration::from_secs(60), async {
        loop {
            match rx.recv().await {
                Ok(RuntimeEvent::Notify { kind, thread_id, channel_id, .. }) if kind == "routine_failed" => return (thread_id, channel_id),
                Ok(_) | Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                Err(e) => panic!("events closed: {e}"),
            }
        }
    })
    .await
    .expect("no routine_failed notice")
}

async fn make_routine(app: &Router, kid: &str, name: &str, thread: Option<&str>) -> String {
    let mut body = json!({"kwant_id": kid, "name": name, "instructions": "Sum up yesterday's orders.",
        "schedule": {"type": "daily", "time": "09:00", "tz": "Europe/Kyiv"}});
    if let Some(t) = thread {
        body["thread_id"] = json!(t);
    }
    let (s, r) = call(app, "POST", "/api/routines", Some(body)).await;
    assert_eq!(s, StatusCode::CREATED, "{r}");
    if let Some(t) = thread {
        assert_eq!(r["thread_id"], t, "{r}");
    }
    r["id"].as_str().unwrap().to_string()
}

#[tokio::test]
async fn a_thread_routines_failure_notice_opens_its_thread() {
    let (app, state, _tmp, k) = setup().await;
    let kwants = get(&app, "/api/kwants").await;
    let mut body = kwants.as_array().unwrap().iter().find(|b| b["id"] == k.id.as_str()).cloned().unwrap();
    body.as_object_mut().unwrap().retain(|key, _| ["name", "title", "persona_prompt", "soul", "avatar_sprite"].contains(&key.as_str()));
    body["preferred_model"] = json!("claude");
    let (s, v) = call(&app, "PUT", &format!("/api/kwants/{}", k.id), Some(body)).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    let thread = state.db.create_thread(&k.id, "Shop digest", "chat", None, "user").await.unwrap().id;

    let mut rx = state.event_sender.subscribe();
    let id = make_routine(&app, &k.id, "Morning shop digest", Some(&thread)).await;
    let (s, _) = call(&app, "POST", &format!("/api/routines/{id}/run"), None).await;
    assert_eq!(s, StatusCode::ACCEPTED);
    let (thread_id, channel_id) = routine_failed(&mut rx).await;
    assert_eq!(thread_id.as_deref(), Some(thread.as_str()));
    assert_eq!(channel_id, None);

    // A routine in the bot's direct chat names no thread, as before
    let id = make_routine(&app, &k.id, "Direct digest", None).await;
    let (s, _) = call(&app, "POST", &format!("/api/routines/{id}/run"), None).await;
    assert_eq!(s, StatusCode::ACCEPTED);
    let (thread_id, channel_id) = routine_failed(&mut rx).await;
    assert_eq!(thread_id, None);
    assert_eq!(channel_id, None);
}
