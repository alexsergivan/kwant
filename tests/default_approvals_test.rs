//! New spaces start on "Ask only when risky" (`smart`); upgrading never loosens an install. A
//! space made before that default has no approvals row and still asks, explicit rows are kept,
//! a new space asks while the built-in password store holds passwords, and an imported bundle
//! can't switch approvals off.

use axum::{
    body::{to_bytes, Body},
    http::{Request, StatusCode},
    Router,
};
use kwnt::{
    broker::PermissionBroker,
    config::Config,
    db::{spaces::SpaceId, Database},
    router::LayaRouter,
    server::{create_router, AppState},
    tools::ToolRunner,
};
use serde_json::{json, Value};
use std::path::Path;
use tempfile::tempdir;
use tower::ServiceExt;

fn state_for(data: &Path, db: Database) -> AppState {
    let config = Config { data_dir: data.to_path_buf(), browser_cdp_url: Some("http://127.0.0.1:9".into()), ..Config::default() };
    AppState::new(config.clone(), db, ToolRunner::new(data.join("workspace")), PermissionBroker::new(), LayaRouter::new(config.laya_url.clone()))
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

async fn raw_pool(data: &Path) -> sqlx::SqlitePool {
    sqlx::SqlitePool::connect(&format!("sqlite://{}", data.join("kwnt.db").display())).await.unwrap()
}

#[tokio::test]
async fn a_fresh_install_and_a_new_space_ask_only_when_risky() {
    let tmp = tempdir().unwrap();
    let data = tmp.path().join("data");
    let db = Database::init(&data).await.unwrap();
    let dflt = SpaceId::default_space();
    assert_eq!(db.space_approvals(&dflt).await.unwrap(), "smart");
    let bot = db.list_kwants(&dflt).await.unwrap().remove(0);
    let m = db.kwant_approvals(&bot.id).await.unwrap();
    assert_eq!((m.bot.as_str(), m.effective.as_str()), ("inherit", "smart"));

    let state = state_for(&data, db);
    let app = create_router(state.clone());
    let (s, v) = call(&app, &state, "POST", "/api/spaces", Some(json!({"name": "Lab"}))).await;
    assert_eq!(s, StatusCode::CREATED, "{v}");
    assert_eq!(v["approvals"], "smart", "{v}");
    let (_, v) = call(&app, &state, "GET", &format!("/api/spaces/{}/approvals", v["id"].as_str().unwrap()), None).await;
    assert_eq!(v["approvals"], "smart");

    // The database API's own create path starts there too
    let other = state.db.create_space(&kwnt::db::spaces::NewSpace { name: "Other".into(), icon: "box".into(), color: None, this_machine_enabled: true }).await.unwrap();
    assert_eq!(state.db.space_approvals(&SpaceId::new(&other.id)).await.unwrap(), "smart");
}

/// An install from before the default changed: no approvals rows at all. After the upgrade its
/// spaces still ask, and explicit rows (space and bot) are exactly as they were.
#[tokio::test]
async fn an_upgrade_keeps_spaces_without_a_row_on_ask_and_explicit_rows_unchanged() {
    let tmp = tempdir().unwrap();
    let data = tmp.path().join("data");
    let db = Database::init(&data).await.unwrap();
    let dflt = SpaceId::default_space();
    let bot = db.list_kwants(&dflt).await.unwrap().remove(0);
    let other_bot = db.list_kwants(&dflt).await.unwrap().remove(1);
    drop(db);

    // Make it look like the old version wrote it: no row for Default or "Old", an explicit `off`
    // on "Kept", `smart` on a bot, `ask` on another
    let pool = raw_pool(&data).await;
    sqlx::query("DELETE FROM approval_modes").execute(&pool).await.unwrap();
    for (id, name, sort) in [("spc_old", "Old", 5), ("spc_kept", "Kept", 6)] {
        sqlx::query("INSERT INTO spaces (id, name, icon, sort) VALUES (?, ?, 'box', ?)").bind(id).bind(name).bind(sort).execute(&pool).await.unwrap();
    }
    for (scope, id, mode) in [("space", "spc_kept", "off"), ("kwant", bot.id.as_str(), "smart"), ("kwant", other_bot.id.as_str(), "ask")] {
        sqlx::query("INSERT INTO approval_modes (scope, id, mode, updated_at) VALUES (?, ?, ?, '2026-01-01T00:00:00Z')")
            .bind(scope)
            .bind(id)
            .bind(mode)
            .execute(&pool)
            .await
            .unwrap();
    }
    pool.close().await;

    // Upgrade: the new version opens the existing database, twice for good measure
    for _ in 0..2 {
        let db = Database::init(&data).await.unwrap();
        assert_eq!(db.space_approvals(&dflt).await.unwrap(), "ask");
        assert_eq!(db.space_approvals(&SpaceId::new("spc_old")).await.unwrap(), "ask");
        assert_eq!(db.space_approvals(&SpaceId::new("spc_kept")).await.unwrap(), "off");
        let m = db.kwant_approvals(&bot.id).await.unwrap();
        assert_eq!((m.space.as_str(), m.bot.as_str(), m.effective.as_str()), ("ask", "smart", "smart"));
        let m = db.kwant_approvals(&other_bot.id).await.unwrap();
        assert_eq!((m.bot.as_str(), m.effective.as_str()), ("ask", "ask"));
        let third = db.list_kwants(&dflt).await.unwrap().into_iter().find(|k| k.id != bot.id && k.id != other_bot.id);
        if let Some(k) = third {
            assert_eq!(db.kwant_approvals(&k.id).await.unwrap().effective, "ask");
        }
        drop(db);
    }
    // No row was written for the spaces that had none
    let pool = raw_pool(&data).await;
    let (n,): (i64,) = sqlx::query_as("SELECT COUNT(*) FROM approval_modes WHERE scope = 'space'").fetch_one(&pool).await.unwrap();
    assert_eq!(n, 1);
    pool.close().await;
}

/// `smart` would let a bot read the built-in store, so a space made while it holds passwords asks.
#[tokio::test]
async fn a_new_space_asks_while_the_built_in_store_holds_passwords() {
    let tmp = tempdir().unwrap();
    let data = tmp.path().join("data");
    let db = Database::init(&data).await.unwrap();
    let dflt = SpaceId::default_space();
    db.set_space_approvals(&dflt, "ask").await.unwrap();
    let bot = db.list_kwants(&dflt).await.unwrap().remove(0);
    let state = state_for(&data, db);
    let app = create_router(state.clone());
    let (s, v) = call(&app, &state, "POST", &format!("/api/sandbox/{}/passwords", bot.id), Some(json!({"origin": "https://zed.test", "username": "zed", "password": "DUMMY-DEFAULT-PW-1"}))).await;
    assert_eq!(s, StatusCode::OK, "{v}");

    let (s, v) = call(&app, &state, "POST", "/api/spaces", Some(json!({"name": "Lab"}))).await;
    assert_eq!(s, StatusCode::CREATED, "{v}");
    assert_eq!(v["approvals"], "ask", "{v}");
    let (_, st) = call(&app, &state, "GET", "/api/passwords/store", None).await;
    assert_ne!(st["exposed"], true, "{st}");
}

/// A database deleted next to a vault it left behind: the fresh Default space asks.
#[tokio::test]
async fn a_fresh_database_next_to_saved_passwords_asks() {
    let tmp = tempdir().unwrap();
    let data = tmp.path().join("data");
    std::fs::create_dir_all(data.join(kwnt::sandbox::credentials::VAULT_DIR)).unwrap();
    std::fs::write(data.join(kwnt::sandbox::credentials::VAULT_DIR).join("owner.bin"), b"x").unwrap();
    let db = Database::init(&data).await.unwrap();
    assert_eq!(db.space_approvals(&SpaceId::default_space()).await.unwrap(), "ask");
}

/// Approvals stay outside bundles: an `approvals: "off"` anywhere in one is ignored, imported
/// bots follow their space and get no override.
#[tokio::test]
async fn an_import_cant_switch_approvals_off() {
    let tmp = tempdir().unwrap();
    let data = tmp.path().join("data");
    let db = Database::init(&data).await.unwrap();
    let state = state_for(&data, db);
    let app = create_router(state.clone());
    let bundle = json!({
        "format": "kwant.bundle/1",
        "source": {"kind": "kwant", "label": "hostile"},
        "approvals": "off",
        "approval_modes": [{"scope": "space", "id": "spc_default", "mode": "off"}],
        "bots": [{"ref": "free", "name": "Free", "title": "t", "persona_prompt": "p", "soul": "", "approvals": "off", "approvals_effective": "off"}]
    });
    check_import(&app, &state, "spc_default", &bundle, "smart").await;
    let asking = state.db.create_space(&kwnt::db::spaces::NewSpace { name: "Asking".into(), icon: "box".into(), color: None, this_machine_enabled: true }).await.unwrap();
    state.db.set_space_approvals(&SpaceId::new(&asking.id), "ask").await.unwrap();
    check_import(&app, &state, &asking.id, &bundle, "ask").await;
    // Nothing in approval_modes for a bot came from either bundle
    let pool = raw_pool(&data).await;
    let (n,): (i64,) = sqlx::query_as("SELECT COUNT(*) FROM approval_modes WHERE scope = 'kwant'").fetch_one(&pool).await.unwrap();
    assert_eq!(n, 0);
    pool.close().await;
}

async fn check_import(app: &Router, state: &AppState, space: &str, bundle: &Value, expected: &str) {
    let (s, p) = call(app, state, "POST", &format!("/api/import/preview?space={space}&filename=x.kwant.json"), Some(json!({"text": bundle.to_string()}))).await;
    assert_eq!(s, StatusCode::OK, "{p}");
    let (s, v) = call(app, state, "POST", &format!("/api/import/apply?space={space}"), Some(json!({"bundle": p["bundle"], "skip": []}))).await;
    assert_eq!(s, StatusCode::CREATED, "{v}");
    let sid = SpaceId::new(space);
    assert_eq!(state.db.space_approvals(&sid).await.unwrap(), expected);
    let free = state.db.list_kwants(&sid).await.unwrap().into_iter().find(|k| k.name == "Free").expect("imported bot");
    let m = state.db.kwant_approvals(&free.id).await.unwrap();
    assert_eq!((m.bot.as_str(), m.effective.as_str()), ("inherit", expected));
}
