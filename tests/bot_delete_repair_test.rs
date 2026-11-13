//! Deleting a bot from a database that is a little out of step (Oleks, 0.7.7 on Mac: the
//! Delete dialog said only "database error").
//! - h1: a channel or team the bot leads lists a bot that no longer exists. The lead used to go
//!   to that id and the delete failed on the lead's foreign key.
//! - h3: the chat search index is out of step with the messages (or memories) it indexes. Its
//!   delete trigger failed with "database disk image is malformed" and the delete rolled back.
//! - A delete that still fails says what happened in plain words, never SQL.
//! - Where stale ids came from: a team member added while a bot was deleted, and a team chat
//!   relinked at start-up, no longer bring back ids of bots that don't exist.

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

async fn call(app: &Router, state: &AppState, method: &str, uri: &str, body: Option<Value>) -> (StatusCode, Value) {
    let req = Request::builder()
        .method(method)
        .uri(uri)
        .header("content-type", "application/json")
        .header("authorization", format!("Bearer {}", state.api_token.reveal()));
    let body = body.map(|b| Body::from(b.to_string())).unwrap_or_else(Body::empty);
    let resp = app.clone().oneshot(req.body(body).unwrap()).await.unwrap();
    let status = resp.status();
    let bytes = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    (status, serde_json::from_slice(&bytes).unwrap_or(Value::Null))
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

/// Assistant and the others in the default space, in the order the route picks a fallback lead.
async fn assistant_and_others(state: &AppState) -> (String, Vec<String>) {
    let bots = state.db.list_kwants(&SpaceId::default_space()).await.unwrap();
    let assistant = bots.iter().find(|k| k.name == "Assistant").expect("the starter Assistant").id.clone();
    (assistant.clone(), bots.into_iter().map(|k| k.id).filter(|id| *id != assistant).collect())
}

async fn members(pool: &SqlitePool, table: &str, id: &str) -> (String, Vec<String>) {
    let (lead, raw): (String, String) =
        sqlx::query_as(&format!("SELECT lead_kwant_id, member_kwants FROM {table} WHERE id = ?")).bind(id).fetch_one(pool).await.unwrap();
    (lead, serde_json::from_str(&raw).unwrap())
}

async fn assert_foreign_keys_hold(pool: &SqlitePool) {
    let broken: Vec<(String, Option<i64>, String, i64)> = sqlx::query_as("PRAGMA foreign_key_check").fetch_all(pool).await.unwrap();
    assert!(broken.is_empty(), "PRAGMA foreign_key_check: {broken:?}");
}

async fn assert_search_indexes_in_step(pool: &SqlitePool) {
    for index in ["messages_fts", "kwant_memories_fts"] {
        sqlx::query(&format!("INSERT INTO {index}({index}, rank) VALUES ('integrity-check', 1)"))
            .execute(pool)
            .await
            .unwrap_or_else(|e| panic!("{index} is out of step: {e}"));
    }
}

#[tokio::test]
async fn h1_a_led_channel_and_team_listing_a_gone_bot_no_longer_block_the_delete() {
    let (app, state, tmp) = setup().await;
    let (assistant, others) = assistant_and_others(&state).await;
    let next = others[0].clone();

    // A channel Assistant leads, and a team where it is the only bot left besides a gone one
    let (s, v) = call(&app, &state, "POST", "/api/channels", Some(json!({"name": "plans", "description": null, "routing_mode": "auto", "lead_kwant_id": assistant, "member_kwants": [assistant, next]}))).await;
    assert!(s.is_success(), "{s} {v}");
    let channel = v["id"].as_str().unwrap().to_string();
    let (s, v) = call(&app, &state, "POST", "/api/teams", Some(json!({"name": "design", "lead_kwant_id": assistant, "member_kwants": [assistant]}))).await;
    assert!(s.is_success(), "{s} {v}");
    let team = v["id"].as_str().unwrap().to_string();
    state.db.insert_message(&dm(&assistant, "user", "hello")).await.unwrap();

    // The broken state: each lists a bot id that no longer exists right after Assistant
    let pool = raw(&tmp).await;
    sqlx::query("UPDATE channels SET member_kwants = ? WHERE id = ?").bind(json!([assistant, "gone-bot-id", next]).to_string()).bind(&channel).execute(&pool).await.unwrap();
    sqlx::query("UPDATE teams SET member_kwants = ? WHERE id = ?").bind(json!([assistant, "gone-bot-id"]).to_string()).bind(&team).execute(&pool).await.unwrap();

    let (s, v) = call(&app, &state, "DELETE", &format!("/api/kwants/{assistant}"), None).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert!(state.db.get_kwant(&assistant).await.unwrap().is_none());

    // The lead went to the next bot that exists; the gone id is dropped from the list
    assert_eq!(members(&pool, "channels", &channel).await, (next.clone(), vec![next.clone()]));
    // Nobody left in the team: the space's fallback leads it
    assert_eq!(members(&pool, "teams", &team).await, (others[0].clone(), vec![]));
    assert_foreign_keys_hold(&pool).await;
}

#[tokio::test]
async fn h3_a_search_index_out_of_step_is_rebuilt_and_the_delete_succeeds() {
    let (app, state, tmp) = setup().await;
    let (assistant, others) = assistant_and_others(&state).await;
    for i in 0..17 {
        state.db.insert_message(&dm(&assistant, if i % 2 == 0 { "user" } else { "kwant" }, &format!("message {i} about the plan"))).await.unwrap();
    }
    state.db.insert_message(&dm(&others[0], "user", "a message that stays")).await.unwrap();
    state.db.add_memory(&assistant, "Oleks prefers short answers.", "user").await.unwrap();

    // The text changes without the search indexes hearing about it (as if written while the
    // update triggers were missing)
    let pool = raw(&tmp).await;
    for sql in [
        "DROP TRIGGER messages_fts_au",
        "DROP TRIGGER kwant_memories_fts_au",
    ] {
        sqlx::query(sql).execute(&pool).await.unwrap();
    }
    sqlx::query("UPDATE messages SET text = text || ' edited outside the index' WHERE direct_kwant_id = ?").bind(&assistant).execute(&pool).await.unwrap();
    sqlx::query("UPDATE kwant_memories SET text = text || ' edited outside the index' WHERE kwant_id = ?").bind(&assistant).execute(&pool).await.unwrap();
    assert!(
        sqlx::query("INSERT INTO messages_fts(messages_fts, rank) VALUES ('integrity-check', 1)").execute(&pool).await.is_err(),
        "the index is out of step before the delete"
    );

    let (s, v) = call(&app, &state, "DELETE", &format!("/api/kwants/{assistant}"), None).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert!(state.db.get_kwant(&assistant).await.unwrap().is_none());
    let (left,): (i64,) = sqlx::query_as("SELECT COUNT(*) FROM messages WHERE direct_kwant_id = ?").bind(&assistant).fetch_one(&pool).await.unwrap();
    assert_eq!(left, 0);
    assert_eq!(state.db.list_messages(&others[0], 10).await.unwrap().len(), 1, "other chats stay");
    assert_foreign_keys_hold(&pool).await;
    assert_search_indexes_in_step(&pool).await;
    let (hits,): (i64,) = sqlx::query_as("SELECT COUNT(*) FROM messages_fts WHERE messages_fts MATCH 'stays'").fetch_one(&pool).await.unwrap();
    assert_eq!(hits, 1, "search still finds what is left");
}

#[tokio::test]
async fn a_delete_that_still_fails_says_why_in_plain_words() {
    let (app, state, tmp) = setup().await;
    let (assistant, _) = assistant_and_others(&state).await;
    state.db.insert_message(&dm(&assistant, "user", "keep me")).await.unwrap();

    // Something the delete can't get past, with SQL-ish text the dialog must not show
    let pool = raw(&tmp).await;
    sqlx::query("CREATE TRIGGER block_delete BEFORE DELETE ON kwants BEGIN SELECT RAISE(ABORT, 'secret sql text from kwants'); END")
        .execute(&pool)
        .await
        .unwrap();

    let (s, v) = call(&app, &state, "DELETE", &format!("/api/kwants/{assistant}"), None).await;
    assert_eq!(s, StatusCode::INTERNAL_SERVER_ERROR, "{v}");
    assert_eq!(v["error"], "delete_failed");
    let message = v["message"].as_str().unwrap();
    assert!(message.starts_with("Couldn't delete Assistant: "), "{message}");
    assert!(message.contains("nothing was deleted"), "{message}");
    for leak in ["database error", "secret", "sql", "SQL", "kwants", "RAISE", "code"] {
        assert!(!message.contains(leak), "{message} shows {leak}");
    }

    // Rolled back whole: the bot and its chat are still there
    assert!(state.db.get_kwant(&assistant).await.unwrap().is_some());
    assert_eq!(state.db.list_messages(&assistant, 10).await.unwrap().len(), 1);
}

#[tokio::test]
async fn adding_a_team_member_keeps_only_bots_that_exist() {
    let (app, state, tmp) = setup().await;
    let (assistant, others) = assistant_and_others(&state).await;
    let (s, v) = call(&app, &state, "POST", "/api/teams", Some(json!({"name": "build", "lead_kwant_id": assistant, "member_kwants": [assistant]}))).await;
    assert!(s.is_success(), "{s} {v}");
    let team = v["id"].as_str().unwrap().to_string();
    let chat = state.db.team_channel(&SpaceId::default_space(), "build").await.unwrap().expect("the team's chat");

    let pool = raw(&tmp).await;
    for (table, id) in [("teams", &team), ("channels", &chat)] {
        sqlx::query(&format!("UPDATE {table} SET member_kwants = ? WHERE id = ?")).bind(json!([assistant, "gone-bot-id"]).to_string()).bind(id).execute(&pool).await.unwrap();
    }

    assert!(state.db.add_team_member(&team, &others[0]).await.unwrap());
    assert!(state.db.add_team_member(&team, "not-a-bot").await.unwrap());
    for (table, id) in [("teams", &team), ("channels", &chat)] {
        assert_eq!(members(&pool, table, id).await.1, vec![assistant.clone(), others[0].clone()], "{table}");
    }
    assert!(!state.db.add_team_member("no-such-team", &others[0]).await.unwrap());
}

#[tokio::test]
async fn start_up_relinks_a_team_chat_without_ids_of_gone_bots_and_rebuilds_an_index_missing_rows() {
    let (app, state, tmp) = setup().await;
    let (assistant, others) = assistant_and_others(&state).await;
    let (s, v) = call(&app, &state, "POST", "/api/teams", Some(json!({"name": "ops", "lead_kwant_id": assistant, "member_kwants": [assistant, others[0]]}))).await;
    assert!(s.is_success(), "{s} {v}");
    let team = v["id"].as_str().unwrap().to_string();
    let chat = state.db.team_channel(&SpaceId::default_space(), "ops").await.unwrap().expect("the team's chat");

    let pool = raw(&tmp).await;
    // A chat that lost its exact name, a team listing a gone bot, and a message written while
    // the index's insert trigger was missing
    sqlx::query("UPDATE channels SET name = 'TEAM-OPS' WHERE id = ?").bind(&chat).execute(&pool).await.unwrap();
    sqlx::query("UPDATE teams SET member_kwants = ? WHERE id = ?").bind(json!([assistant, "gone-bot-id", others[0]]).to_string()).bind(&team).execute(&pool).await.unwrap();
    sqlx::query("DROP TRIGGER messages_fts_ai").execute(&pool).await.unwrap();
    state.db.insert_message(&dm(&assistant, "user", "written while the index was deaf")).await.unwrap();
    let (rows,): (i64,) = sqlx::query_as("SELECT COUNT(*) FROM messages").fetch_one(&pool).await.unwrap();
    let (indexed,): (i64,) = sqlx::query_as("SELECT COUNT(*) FROM messages_fts_docsize").fetch_one(&pool).await.unwrap();
    assert_ne!(rows, indexed);
    pool.close().await;
    drop((app, state));

    let db = Database::init(tmp.path()).await.unwrap();
    let pool = raw(&tmp).await;
    let (name, lead, raw_members): (String, String, String) =
        sqlx::query_as("SELECT name, lead_kwant_id, member_kwants FROM channels WHERE id = ?").bind(&chat).fetch_one(&pool).await.unwrap();
    assert_eq!(name, "team-ops");
    assert_eq!(lead, assistant);
    assert_eq!(serde_json::from_str::<Vec<String>>(&raw_members).unwrap(), vec![assistant.clone(), others[0].clone()]);
    let (indexed,): (i64,) = sqlx::query_as("SELECT COUNT(*) FROM messages_fts_docsize").fetch_one(&pool).await.unwrap();
    assert_eq!(indexed, rows, "the index was rebuilt");
    assert_search_indexes_in_step(&pool).await;
    let (hits,): (i64,) = sqlx::query_as("SELECT COUNT(*) FROM messages_fts WHERE messages_fts MATCH 'deaf'").fetch_one(&pool).await.unwrap();
    assert_eq!(hits, 1);
    drop(db);
}
