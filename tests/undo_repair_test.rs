//! Undo for a ready-made team and for an import, from a database whose chat search index is out
//! of step (h3 in tests/bot_delete_repair_test.rs, Ada's #465 review, Later 1). Both remove bots
//! with their messages or memories, so the index's delete trigger used to fail them with
//! "database disk image is malformed" and the person got a bare 500 or "database error". They
//! now get the bot delete's repair (rebuild both indexes, try once more), and an Undo that still
//! fails says why in the same plain words as the Delete dialog. All data is invented.

use axum::{
    body::{to_bytes, Body},
    http::{Request, StatusCode},
    Router,
};
use kwnt::{
    broker::PermissionBroker,
    config::Config,
    db::{models::NewMessage, spaces::SpaceId, Database},
    router::LayaRouter,
    server::{create_router, AppState},
    tools::ToolRunner,
};
use serde_json::{json, Value};
use sqlx::SqlitePool;
use std::path::PathBuf;
use tempfile::{tempdir, TempDir};
use tower::ServiceExt;

async fn setup() -> (Router, AppState, TempDir) {
    let tmp = tempdir().unwrap();
    let db = Database::init(tmp.path()).await.unwrap();
    let config = Config { data_dir: tmp.path().to_path_buf(), ..Config::default() };
    let state = AppState::new(
        config.clone(),
        db,
        ToolRunner::new(tmp.path().join("workspace")),
        PermissionBroker::new(),
        LayaRouter::new(config.laya_url.clone()),
    );
    (create_router(state.clone()), state, tmp)
}

/// A second connection to the same file, for writing the broken states the app never writes.
async fn raw(tmp: &TempDir) -> SqlitePool {
    SqlitePool::connect(&format!("sqlite://{}", tmp.path().join("kwnt.db").display())).await.unwrap()
}

async fn call(app: &Router, state: &AppState, method: &str, uri: &str, content_type: &str, body: Option<Vec<u8>>) -> (StatusCode, Value) {
    let req = Request::builder()
        .method(method)
        .uri(uri)
        .header("content-type", content_type)
        .header("authorization", format!("Bearer {}", state.api_token.reveal()));
    let resp = app.clone().oneshot(req.body(body.map(Body::from).unwrap_or_else(Body::empty)).unwrap()).await.unwrap();
    let status = resp.status();
    let bytes = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    (status, serde_json::from_slice(&bytes).unwrap_or(Value::Null))
}

async fn post(app: &Router, state: &AppState, uri: &str, body: Value) -> (StatusCode, Value) {
    call(app, state, "POST", uri, "application/json", Some(body.to_string().into_bytes())).await
}

fn dm(kwant: &str, role: &str, text: &str) -> NewMessage {
    NewMessage {
        channel_id: None,
        direct_kwant_id: Some(kwant.into()),
        sender_role: role.into(),
        sender_id: if role == "user" { "user".into() } else { kwant.into() },
        text: text.into(),
        reasoning_content: None,
        tool_calls: None,
        tokens_used: 0,
    }
}

/// h3: text changes without the search indexes hearing about it (as if written while the update
/// triggers were missing). Deleting those rows then trips the index's delete trigger.
async fn put_indexes_out_of_step(pool: &SqlitePool, bot: &str) {
    for sql in ["DROP TRIGGER messages_fts_au", "DROP TRIGGER kwant_memories_fts_au"] {
        sqlx::query(sql).execute(pool).await.unwrap();
    }
    sqlx::query("UPDATE messages SET text = text || ' edited outside the index' WHERE direct_kwant_id = ?").bind(bot).execute(pool).await.unwrap();
    sqlx::query("UPDATE kwant_memories SET text = text || ' edited outside the index' WHERE kwant_id = ?").bind(bot).execute(pool).await.unwrap();
    assert!(
        sqlx::query("INSERT INTO kwant_memories_fts(kwant_memories_fts, rank) VALUES ('integrity-check', 1)").execute(pool).await.is_err(),
        "the memory index is out of step before the Undo"
    );
}

async fn assert_search_indexes_in_step(pool: &SqlitePool) {
    for index in ["messages_fts", "kwant_memories_fts"] {
        sqlx::query(&format!("INSERT INTO {index}({index}, rank) VALUES ('integrity-check', 1)"))
            .execute(pool)
            .await
            .unwrap_or_else(|e| panic!("{index} is out of step: {e}"));
    }
}

/// Something the Undo can't get past, with SQL-ish text the person must never see.
async fn block_bot_deletes(pool: &SqlitePool) {
    sqlx::query("CREATE TRIGGER block_delete BEFORE DELETE ON kwants BEGIN SELECT RAISE(ABORT, 'secret sql text from kwants'); END")
        .execute(pool)
        .await
        .unwrap();
}

fn assert_plain(message: &str) {
    for leak in ["database error", "secret", "sql", "SQL", "kwants", "RAISE", "malformed", "code"] {
        assert!(!message.contains(leak), "{message} shows {leak}");
    }
}

/// A ready-made "inbox" team: (bot action id, a bot it made).
async fn ready_made_team(app: &Router, state: &AppState) -> (String, String) {
    let (s, v) = post(app, state, "/api/teams/from-template", json!({"template_id": "inbox", "names": {"Researcher": "Finder"}})).await;
    assert_eq!(s, StatusCode::CREATED, "{v}");
    let bot = v["bots"].as_array().unwrap().iter().find(|b| b["lead"] != true).unwrap()["id"].as_str().unwrap().to_string();
    (v["action_id"].as_str().unwrap().to_string(), bot)
}

#[tokio::test]
async fn h3_undoing_a_ready_made_team_rebuilds_the_index_and_succeeds() {
    let (app, state, tmp) = setup().await;
    let (action, bot) = ready_made_team(&app, &state).await;
    for i in 0..9 {
        state.db.insert_message(&dm(&bot, if i % 2 == 0 { "user" } else { "kwant" }, &format!("message {i} about the inbox"))).await.unwrap();
    }
    state.db.add_memory(&bot, "Oleks files receipts on Fridays.", "user").await.unwrap();
    let assistant = state.db.list_kwants(&SpaceId::default_space()).await.unwrap().into_iter().find(|k| k.name == "Assistant").unwrap().id;
    state.db.insert_message(&dm(&assistant, "user", "a message that stays")).await.unwrap();
    let pool = raw(&tmp).await;
    put_indexes_out_of_step(&pool, &bot).await;

    let (s, v) = post(&app, &state, &format!("/api/bot-actions/{action}/undo"), json!({})).await;
    assert!(s.is_success(), "{s} {v}");
    assert!(state.db.get_kwant(&bot).await.unwrap().is_none(), "the bot the team made is gone");
    let (left,): (i64,) = sqlx::query_as("SELECT COUNT(*) FROM messages WHERE direct_kwant_id = ?").bind(&bot).fetch_one(&pool).await.unwrap();
    assert_eq!(left, 0);
    assert_search_indexes_in_step(&pool).await;
    let (hits,): (i64,) = sqlx::query_as("SELECT COUNT(*) FROM messages_fts WHERE messages_fts MATCH 'stays'").fetch_one(&pool).await.unwrap();
    assert_eq!(hits, 1, "search still finds what is left");
}

#[tokio::test]
async fn a_ready_made_team_undo_that_still_fails_says_why_in_plain_words() {
    let (app, state, tmp) = setup().await;
    let (action, bot) = ready_made_team(&app, &state).await;
    let pool = raw(&tmp).await;
    block_bot_deletes(&pool).await;

    let (s, v) = post(&app, &state, &format!("/api/bot-actions/{action}/undo"), json!({})).await;
    assert_eq!(s, StatusCode::INTERNAL_SERVER_ERROR, "{v}");
    assert_eq!(v["code"], "undo_failed", "{v}");
    // The card puts "Couldn't undo the team: " in front of `error`
    let why = v["error"].as_str().unwrap();
    assert!(why.contains("nothing was deleted"), "{why}");
    assert_plain(why);
    let message = v["message"].as_str().unwrap();
    assert_eq!(message, format!("Couldn't undo this ready-made team: {why}"));
    assert_plain(message);
    // Rolled back whole
    assert!(state.db.get_kwant(&bot).await.unwrap().is_some());
}

fn fixture(name: &str) -> Vec<u8> {
    std::fs::read(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/import").join(name)).unwrap()
}

/// The kwant.bundle fixture applied in the default space: (import id, the bot it made, which
/// has one memory).
async fn imported(app: &Router, state: &AppState) -> (String, String) {
    let space = SpaceId::default_space().as_str().to_string();
    let (s, preview) =
        call(app, state, "POST", &format!("/api/import/preview?space={space}&filename=kwant-bundle.json"), "application/octet-stream", Some(fixture("kwant-bundle.json"))).await;
    assert_eq!(s, StatusCode::OK, "{preview}");
    let (s, applied) = post(app, state, &format!("/api/import/apply?space={space}"), json!({ "bundle": preview["bundle"], "skip": [] })).await;
    assert_eq!(s, StatusCode::CREATED, "{applied}");
    let bot = applied["import"]["created"]["kwants"][0]["id"].as_str().unwrap().to_string();
    assert_eq!(state.db.list_memories(&bot).await.unwrap().len(), 1);
    (applied["import"]["id"].as_str().unwrap().to_string(), bot)
}

#[tokio::test]
async fn h3_undoing_an_import_rebuilds_the_index_and_succeeds() {
    let (app, state, tmp) = setup().await;
    let (import, bot) = imported(&app, &state).await;
    let pool = raw(&tmp).await;
    put_indexes_out_of_step(&pool, &bot).await;

    // force: the memory's text changed, which may count as an edit
    let (s, v) = post(&app, &state, &format!("/api/imports/{import}/undo"), json!({ "force": true })).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(v["status"], "undone");
    assert!(state.db.get_kwant(&bot).await.unwrap().is_none());
    let (left,): (i64,) = sqlx::query_as("SELECT COUNT(*) FROM kwant_memories WHERE kwant_id = ?").bind(&bot).fetch_one(&pool).await.unwrap();
    assert_eq!(left, 0);
    assert_search_indexes_in_step(&pool).await;
}

#[tokio::test]
async fn an_import_undo_that_still_fails_says_why_in_plain_words() {
    let (app, state, tmp) = setup().await;
    let (import, bot) = imported(&app, &state).await;
    let pool = raw(&tmp).await;
    block_bot_deletes(&pool).await;

    let (s, v) = post(&app, &state, &format!("/api/imports/{import}/undo"), json!({ "force": true })).await;
    assert_eq!(s, StatusCode::INTERNAL_SERVER_ERROR, "{v}");
    assert_eq!(v["code"], "undo_failed", "{v}");
    let message = v["error"].as_str().unwrap();
    assert!(message.starts_with("Couldn't undo this import: "), "{message}");
    assert!(message.contains("nothing was deleted"), "{message}");
    assert_plain(message);
    // Rolled back whole: the bot and the import's status stay
    assert!(state.db.get_kwant(&bot).await.unwrap().is_some());
    let (status,): (String,) = sqlx::query_as("SELECT status FROM imports WHERE id = ?").bind(&import).fetch_one(&pool).await.unwrap();
    assert_eq!(status, "applied");
}
