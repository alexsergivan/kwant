//! routine-failed-row-bot-words (Mira's UX pass 0.9.45, Minor c): a routine run whose step failed
//! shows the bot's own reply as its Inbox summary ("The shop answered 401 Unauthorized, so its
//! access key may have expired.") instead of the raw step error ("curl: (22) … 401"). The raw
//! error stays the summary when there is no reply, or when the reply is only a usage-limit line.
//! Masking applies to whichever text is used; the title is the routine's name; a `needs_you` run
//! keeps its own wording (26b, unanswered-question-not-ok).

use axum::{
    body::{to_bytes, Body},
    http::{Request, StatusCode},
    Router,
};
use kwnt::{
    broker::PermissionBroker,
    config::Config,
    db::{continuity::NewRoutine, models::{Kwant, NewMessage}, spaces::SpaceId, Database},
    router::LayaRouter,
    server::{create_router, scheduler::NEEDS_YOU_ERROR, AppState},
    tools::ToolRunner,
};
use serde_json::{json, Value};
use tempfile::{tempdir, TempDir};
use tower::ServiceExt;

const SECRET: &str = "sk-ant-api03-abcdefghijklmnopqrstuvwxyz0123456789";
const CURL: &str = "curl: (22) The requested URL returned error: 401 Unauthorized";
const BOT_WORDS: &str = "The shop answered 401 Unauthorized, so its access key may have expired.";

async fn setup() -> (Router, AppState, TempDir, Kwant) {
    let tmp = tempdir().unwrap();
    let db = Database::init(tmp.path()).await.unwrap();
    db.set_setting("keep_going_turns", "0").await.unwrap();
    let config = Config { data_dir: tmp.path().to_path_buf(), ..Config::default() };
    let state = AppState::new(config.clone(), db, ToolRunner::new(tmp.path().join("workspace")), PermissionBroker::new(), LayaRouter::new(config.laya_url.clone()));
    let k = state.db.list_kwants(&SpaceId::default_space()).await.unwrap()[0].clone();
    (create_router(state.clone()), state, tmp, k)
}

async fn get(app: &Router, uri: &str) -> Value {
    let req = Request::builder().method("GET").uri(uri).body(Body::empty()).unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK, "{uri}");
    let bytes = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    serde_json::from_slice(&bytes).unwrap()
}

/// A routine of `k` named `name` with one run that ended `status` with `error`, and the bot's
/// reply `text` as the run's message when there is one. Returns the run id.
async fn ended_run(state: &AppState, k: &Kwant, name: &str, status: &str, error: Option<&str>, text: Option<&str>) -> String {
    let routine = state
        .db
        .create_routine(&NewRoutine {
            kwant_id: k.id.clone(),
            channel_id: None,
            name: name.into(),
            instructions: "Check the shop's orders".into(),
            schedule: json!({"type": "interval", "every_minutes": 60}).to_string(),
            enabled: false,
            next_run_at: None,
            thread_id: None,
            context_message_id: None,
            ..Default::default()
        })
        .await
        .unwrap();
    let run = state.db.start_routine_run(&routine.id, "schedule", "running", None).await.unwrap();
    let message_id = match text {
        Some(t) => Some(
            state
                .db
                .insert_message(&NewMessage {
                    channel_id: None,
                    direct_kwant_id: Some(k.id.clone()),
                    sender_role: "kwant".into(),
                    sender_id: k.id.clone(),
                    text: t.into(),
                    reasoning_content: None,
                    tool_calls: None,
                    tokens_used: 1,
                })
                .await
                .unwrap()
                .id,
        ),
        None => None,
    };
    state.db.finish_routine_run(&run.id, status, error, message_id.as_deref()).await.unwrap();
    run.id
}

/// The run's row in the away feed's failed list and in the Inbox, which must agree.
async fn rows(app: &Router, id: &str) -> (Value, Value) {
    let away = get(app, "/api/away").await;
    let a = away["failed"].as_array().unwrap().iter().find(|i| i["id"] == id).cloned().unwrap_or_else(|| panic!("no away row {id}: {away}"));
    let inbox = get(app, "/api/inbox").await;
    let b = inbox["items"].as_array().unwrap().iter().find(|i| i["id"] == id).cloned().unwrap_or_else(|| panic!("no inbox row {id}: {inbox}"));
    (a, b)
}

#[tokio::test]
async fn a_failed_step_shows_the_bots_words_not_the_raw_error() {
    let (app, state, _tmp, k) = setup().await;
    let id = ended_run(&state, &k, "Tuesday orders", "incomplete", Some(CURL), Some(BOT_WORDS)).await;
    let (away, inbox) = rows(&app, &id).await;
    for row in [&away, &inbox] {
        assert_eq!(row["summary"], BOT_WORDS, "{row}");
        assert_eq!(row["title"], "Tuesday orders", "{row}");
        assert_eq!(row["status"], "incomplete", "{row}");
    }
    // A run that errored and still left a reply reads the same way
    let id = ended_run(&state, &k, "Wednesday orders", "error", Some(CURL), Some(BOT_WORDS)).await;
    let (away, inbox) = rows(&app, &id).await;
    assert_eq!(away["summary"], BOT_WORDS, "{away}");
    assert_eq!(inbox["summary"], BOT_WORDS, "{inbox}");
}

#[tokio::test]
async fn the_bots_words_are_masked_and_clipped_like_any_summary() {
    let (app, state, _tmp, k) = setup().await;
    let words = format!("The shop rejected the key {SECRET}, so it may have expired.\nI stopped before posting anything.");
    let id = ended_run(&state, &k, "Thursday orders", "incomplete", Some(CURL), Some(&words)).await;
    let (away, inbox) = rows(&app, &id).await;
    for row in [&away, &inbox] {
        let s = row["summary"].as_str().unwrap();
        assert!(!s.contains(SECRET), "secret leaked: {row}");
        assert!(s.starts_with("The shop rejected the key "), "{row}");
        assert!(s.contains("[redacted]"), "{row}");
        // The first line only, as for every summary
        assert!(!s.contains("I stopped"), "{row}");
    }
    assert!(!get(&app, "/api/away").await.to_string().contains(SECRET));
}

#[tokio::test]
async fn no_reply_keeps_the_raw_error() {
    let (app, state, _tmp, k) = setup().await;
    // The model died before it answered: no message, so the step error is all there is
    let id = ended_run(&state, &k, "Friday orders", "error", Some(CURL), None).await;
    let (away, inbox) = rows(&app, &id).await;
    assert_eq!(away["summary"], CURL, "{away}");
    assert_eq!(inbox["summary"], CURL, "{inbox}");
    // An empty reply is no reply either
    let id = ended_run(&state, &k, "Saturday orders", "incomplete", Some(CURL), Some("  \n ")).await;
    let (away, inbox) = rows(&app, &id).await;
    assert_eq!(away["summary"], CURL, "{away}");
    assert_eq!(inbox["summary"], CURL, "{inbox}");
}

#[tokio::test]
async fn a_usage_limit_reply_falls_back_to_the_raw_error() {
    let (app, state, _tmp, k) = setup().await;
    let limit = "You've hit your session limit · resets 3am (Europe/Berlin)";
    let id = ended_run(&state, &k, "Sunday orders", "incomplete", Some(CURL), Some(limit)).await;
    let (away, inbox) = rows(&app, &id).await;
    assert_eq!(away["summary"], CURL, "{away}");
    assert_eq!(inbox["summary"], CURL, "{inbox}");
    // Both a limit line: never a summary, "Failed" as before
    let id = ended_run(&state, &k, "Monday orders", "error", Some(limit), Some(limit)).await;
    let (away, inbox) = rows(&app, &id).await;
    assert_eq!(away["summary"], "Failed", "{away}");
    assert_eq!(inbox["summary"], "Failed", "{inbox}");
}

#[tokio::test]
async fn a_run_waiting_on_the_person_keeps_its_own_wording() {
    let (app, state, _tmp, k) = setup().await;
    let id = ended_run(&state, &k, "Refund check", "needs_you", Some(NEEDS_YOU_ERROR), Some("I asked to send the refund email.")).await;
    let (away, inbox) = rows(&app, &id).await;
    assert_eq!(away["summary"], NEEDS_YOU_ERROR, "{away}");
    assert_eq!(inbox["summary"], NEEDS_YOU_ERROR, "{inbox}");
}

#[tokio::test]
async fn a_finished_run_still_shows_its_reply() {
    let (app, state, _tmp, k) = setup().await;
    let id = ended_run(&state, &k, "Morning briefing", "ok", None, Some("3 new orders, 1 refund.")).await;
    let away = get(&app, "/api/away").await;
    let row = away["finished"].as_array().unwrap().iter().find(|i| i["id"] == id).cloned().unwrap_or_else(|| panic!("{away}"));
    assert_eq!(row["summary"], "3 new orders, 1 refund.");
    assert_eq!(row["title"], "Morning briefing");
}
