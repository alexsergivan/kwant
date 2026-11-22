//! 33m stopped-row-names-the-job: a stopped job's Needs-you row in the Inbox is titled with the
//! person's request it was working on, so they can tell which job stopped; the reason stays its
//! summary. No request found: the title is the reason, as before. /api/away keeps the reason.

use axum::{
    body::{to_bytes, Body},
    http::{Request, StatusCode},
    Router,
};
use chrono::{DateTime, Utc};
use kwnt::{
    broker::PermissionBroker,
    config::Config,
    db::{
        models::{Kwant, MessageExtra, NewChannel, NewMessage},
        spaces::SpaceId,
        Database,
    },
    router::LayaRouter,
    server::{create_router, AppState},
    tools::ToolRunner,
};
use serde_json::Value;
use sqlx::sqlite::SqlitePoolOptions;
use tempfile::{tempdir, TempDir};
use tower::ServiceExt;

const REASON: &str = "Couldn't finish this reply: its model stopped answering. Retry in the chat sends your message again.";

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
    assert_eq!(resp.status(), StatusCode::OK);
    let bytes = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    serde_json::from_slice(&bytes).unwrap()
}

/// The Inbox's row for item `id`.
async fn inbox_row(app: &Router, id: &str) -> Value {
    let v = get(app, "/api/inbox").await;
    v["items"].as_array().unwrap().iter().find(|i| i["id"] == id).cloned().unwrap_or_else(|| panic!("no row {id}: {v}"))
}

/// The same item on /api/away (Home's needs-you strip reads it).
async fn away_row(app: &Router, id: &str) -> Value {
    let v = get(app, "/api/away").await;
    v["waiting"].as_array().unwrap().iter().find(|i| i["id"] == id).cloned().unwrap_or_else(|| panic!("no away row {id}: {v}"))
}

/// Sets a timestamp the way kwnt writes it: `messages.created_at` is SQLite's `CURRENT_TIMESTAMP`
/// text (`2026-11-22 10:00:00`), `waiting_on_person.created_at` a bound `DateTime`.
async fn set_time(state: &AppState, table: &str, id: &str, t: DateTime<Utc>) {
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect(&format!("sqlite://{}", state.config.data_dir.join("kwnt.db").display()))
        .await
        .unwrap();
    let sql = format!("UPDATE {table} SET created_at = ? WHERE id = ?");
    let q = sqlx::query(&sql);
    let q = if table == "messages" { q.bind(t.format("%Y-%m-%d %H:%M:%S").to_string()) } else { q.bind(t) };
    q.bind(id).execute(&pool).await.unwrap();
    pool.close().await;
}

#[derive(Clone, Copy)]
enum Chat<'a> {
    Direct(&'a str),
    Channel(&'a str),
    Thread(&'a str),
}

/// A message in `chat` at `t`.
async fn say(state: &AppState, chat: Chat<'_>, role: &str, sender: &str, text: &str, t: DateTime<Utc>) -> String {
    let (channel_id, direct_kwant_id, thread_id) = match chat {
        Chat::Direct(k) => (None, Some(k.to_string()), None),
        Chat::Channel(c) => (Some(c.to_string()), None, None),
        Chat::Thread(th) => (None, None, Some(th.to_string())),
    };
    let msg = NewMessage {
        channel_id,
        direct_kwant_id,
        sender_role: role.into(),
        sender_id: sender.into(),
        text: text.into(),
        reasoning_content: None,
        tool_calls: None,
        tokens_used: 1,
    };
    let m = state.db.insert_message_with(&msg, &MessageExtra { thread_id, meta: None }).await.unwrap();
    set_time(state, "messages", &m.id, t).await;
    m.id
}

/// The bot's stop in `chat` at `t`: its notice and the stopped item, as `stopped::ask` records them.
async fn stop(state: &AppState, k: &Kwant, chat: Chat<'_>, reason: &str, t: DateTime<Utc>) -> String {
    let notice = say(state, chat, "kwant", &k.name, &format!("⚠️ [{}] {reason}", k.name), t).await;
    let (channel, direct, thread) = match chat {
        Chat::Direct(d) => (None, Some(d), None),
        Chat::Channel(c) => (Some(c), None, None),
        Chat::Thread(th) => (None, None, Some(th)),
    };
    let (w, _) = state.db.add_stopped(&k.id, channel, direct, thread, Some(&notice), reason, None).await.unwrap();
    set_time(state, "waiting_on_person", &w.id, t).await;
    w.id
}

/// Today at `h:m:s` UTC.
fn at(h: u32, m: u32, s: u32) -> DateTime<Utc> {
    Utc::now().date_naive().and_hms_opt(h, m, s).unwrap().and_utc()
}

#[tokio::test]
async fn a_stopped_job_in_a_direct_chat_is_titled_with_the_persons_request() {
    let (app, state, _tmp, k) = setup().await;
    let chat = Chat::Direct(&k.id);
    say(&state, chat, "user", "user", "Thanks, that's all for the sale.", at(9, 30, 0)).await;
    say(&state, chat, "kwant", &k.name, "You're welcome.", at(9, 30, 5)).await;
    say(&state, chat, "user", "user", "Draft the Q3 board update", at(10, 0, 0)).await;
    let id = stop(&state, &k, chat, REASON, at(10, 4, 0)).await;
    // A later message the same day is not the request this job worked on
    say(&state, chat, "user", "user", "Also book the room for Friday", at(10, 6, 0)).await;

    let row = inbox_row(&app, &id).await;
    assert_eq!(row["kind"], "stopped");
    assert_eq!(row["title"], "Draft the Q3 board update", "{row}");
    assert_eq!(row["summary"], REASON, "the reason stays the summary");
    assert_eq!(row["title_is_request"], true);
    assert_eq!(row["group"], "waiting_on_you");

    // Home's needs-you strip (/api/away) keeps the reason, the line that says what resumes it
    let away = away_row(&app, &id).await;
    assert_eq!(away["title"], REASON, "{away}");
    assert!(away.get("title_is_request").is_none(), "{away}");
}

#[tokio::test]
async fn a_stopped_job_in_a_thread_is_titled_with_the_request_in_that_thread() {
    let (app, state, _tmp, k) = setup().await;
    let thread = state.db.create_thread(&k.id, "Board pack", "chat", None, "user").await.unwrap();
    let chat = Chat::Thread(&thread.id);
    say(&state, chat, "user", "user", "Draft the Q3 board update", at(10, 0, 0)).await;
    // The direct chat's own, newer request is another job
    say(&state, Chat::Direct(&k.id), "user", "user", "Check the shop's stock", at(10, 2, 0)).await;
    let id = stop(&state, &k, chat, REASON, at(10, 4, 0)).await;

    let row = inbox_row(&app, &id).await;
    assert_eq!(row["title"], "Draft the Q3 board update", "{row}");
    assert_eq!(row["summary"], REASON);
    assert_eq!(row["link"]["thread_id"], thread.id.as_str());
}

#[tokio::test]
async fn a_stopped_job_in_a_channel_is_titled_with_the_request_in_that_channel() {
    let (app, state, _tmp, k) = setup().await;
    let chan = state
        .db
        .create_channel(&SpaceId::default_space(), &NewChannel {
            name: "board".into(),
            description: None,
            lead_kwant_id: k.id.clone(),
            member_kwants: vec![k.id.clone()],
            routing_mode: "auto".into(),
        })
        .await
        .unwrap();
    let chat = Chat::Channel(&chan.id);
    say(&state, chat, "user", "user", "Draft the Q3 board update", at(10, 0, 0)).await;
    say(&state, Chat::Direct(&k.id), "user", "user", "Check the shop's stock", at(10, 2, 0)).await;
    let id = stop(&state, &k, chat, REASON, at(10, 4, 0)).await;

    let row = inbox_row(&app, &id).await;
    assert_eq!(row["title"], "Draft the Q3 board update", "{row}");
    assert_eq!(row["summary"], REASON);
}

#[tokio::test]
async fn with_no_persons_message_before_it_the_title_is_the_reason_as_before() {
    let (app, state, _tmp, k) = setup().await;
    let chat = Chat::Direct(&k.id);
    // Long enough that the title shortens it the way it always has
    let reason = format!("Couldn't finish this reply: {}. Retry in the chat sends your message again.", "the model kept timing out on a long page ".repeat(4).trim_end());
    assert!(reason.chars().count() > 120);
    let id = stop(&state, &k, chat, &reason, at(10, 4, 0)).await;
    // Only after the stop does the person write
    say(&state, chat, "user", "user", "Draft the Q3 board update", at(10, 6, 0)).await;

    let row = inbox_row(&app, &id).await;
    let away = away_row(&app, &id).await;
    assert_eq!(row["title"], away["title"], "byte for byte today's title: {row}");
    assert!(row["title"].as_str().unwrap().ends_with("Retry in the chat sends your message again."), "{row}");
    assert_eq!(row["summary"], reason.as_str());
    assert!(row.get("title_is_request").is_none(), "{row}");
}

#[tokio::test]
async fn a_long_multi_line_request_is_one_clipped_line() {
    let (app, state, _tmp, k) = setup().await;
    let chat = Chat::Direct(&k.id);
    let first = format!("## Please **draft** the Q3 board update {}", "with revenue, churn and hiring numbers ".repeat(6));
    let request = format!("\n\n{first}\nSecond line: use last quarter's deck.\n{}", "x".repeat(200));
    assert!(request.chars().count() >= 400);
    say(&state, chat, "user", "user", &request, at(10, 0, 0)).await;
    let id = stop(&state, &k, chat, REASON, at(10, 4, 0)).await;

    let row = inbox_row(&app, &id).await;
    let title = row["title"].as_str().unwrap();
    assert!(!title.contains('\n'), "{title}");
    assert!(title.chars().count() <= 120, "{} chars: {title}", title.chars().count());
    assert!(title.starts_with("Please draft the Q3 board update with revenue"), "markdown dropped: {title}");
    assert!(title.ends_with('…'), "{title}");
    assert_eq!(row["summary"], REASON);
}

#[tokio::test]
async fn a_blank_request_keeps_the_reason() {
    let (app, state, _tmp, k) = setup().await;
    let chat = Chat::Direct(&k.id);
    say(&state, chat, "user", "user", "  \n\t ", at(10, 0, 0)).await;
    let id = stop(&state, &k, chat, REASON, at(10, 0, 30)).await;
    let row = inbox_row(&app, &id).await;
    assert_eq!(row["title"], REASON, "{row}");
    assert!(row.get("title_is_request").is_none(), "{row}");
}
