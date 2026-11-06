//! The Dev desk keeps two "seen" times (0.7.2): opening the desk clears the review badge
//! (`reviews_seen_at`, flow F6) and "Mark seen" starts the landed rollup over (`seen_at`). Before,
//! one `seen_at` served both, so opening the desk emptied the rollup before it loaded and it
//! almost always said "Nothing landed since you last looked".

mod common;

use common::*;
use kwnt::broker::PermissionBroker;
use kwnt::config::Config;
use kwnt::db::dev_mode::{LANDED, READY, REVIEW};
use kwnt::db::spaces::SpaceId;
use kwnt::db::Database;
use kwnt::router::LayaRouter;
use kwnt::server::{create_router, AppState};
use kwnt::tools::ToolRunner;
use serde_json::json;
use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
use std::path::Path;
use std::str::FromStr;
use std::time::Duration;

const REPO: &str = "demo-org/demo-app";

fn mode_uri(space: &SpaceId) -> String {
    format!("/api/spaces/{}/dev_mode", space.as_str())
}

fn landed_uri(space: &SpaceId) -> String {
    format!("/api/spaces/{}/dev/landed", space.as_str())
}

/// A ready card of `kind` made now (`ref_` doubles as its head sha).
async fn card(db: &Database, space: &SpaceId, kind: &str, ref_: &str) {
    let row = db.insert_dev_card(space.as_str(), kind, ref_, ref_, "{}").await.unwrap().unwrap();
    let body = json!({"headline": format!("Card {ref_}"), "area": "Desk"}).to_string();
    db.set_dev_card(&row.id, READY, Some(&body)).await.unwrap();
}

/// Card and seen times are compared with `>`: keep them apart.
async fn tick() {
    tokio::time::sleep(Duration::from_millis(20)).await;
}

async fn badge(app: &axum::Router, space: &SpaceId) -> serde_json::Value {
    let (s, v) = call(app, "GET", &mode_uri(space), None).await;
    assert_eq!(s.as_u16(), 200, "{v}");
    v["new_reviews"].clone()
}

async fn rollup(app: &axum::Router, space: &SpaceId) -> serde_json::Value {
    let (s, v) = call(app, "GET", &landed_uri(space), None).await;
    assert_eq!(s.as_u16(), 200, "{v}");
    v["rollup"].clone()
}

async fn post(app: &axum::Router, uri: &str) {
    let (s, v) = call(app, "POST", uri, None).await;
    assert_eq!(s.as_u16(), 200, "{uri}: {v}");
    assert!(v["seen_at"].is_string(), "{uri} still replies seen_at: {v}");
}

#[tokio::test]
async fn opening_the_desk_clears_the_badge_and_leaves_the_rollup_to_mark_seen() {
    let env = setup().await;
    let space = SpaceId::default_space();
    let db = &env.state.db;
    db.put_dev_mode(space.as_str(), true, REPO, "main", None, None).await.unwrap();
    let desk_seen = format!("{}/seen", mode_uri(&space));
    let mark_seen = format!("{}/seen", landed_uri(&space));

    // Both looked at once, then a merge and a new review arrive
    post(&env.app, &mark_seen).await;
    post(&env.app, &desk_seen).await;
    let looked = db.dev_mode(space.as_str()).await.unwrap().unwrap().seen_at.expect("Mark seen wrote seen_at");
    tick().await;
    card(db, &space, LANDED, "aaaa001").await;
    card(db, &space, REVIEW, "pr:7").await;
    assert_eq!(badge(&env.app, &space).await, 1);
    assert_eq!(rollup(&env.app, &space).await["count"], 1);

    // Opening the desk: the badge clears, the rollup still shows the merge since Mark seen
    post(&env.app, &desk_seen).await;
    assert_eq!(badge(&env.app, &space).await, 0, "opening the desk clears the badge");
    let r = rollup(&env.app, &space).await;
    assert_eq!(r["count"], 1, "the rollup still counts what landed since Mark seen: {r}");
    assert_eq!(r["areas"], json!([{"area": "Desk", "count": 1, "first_headline": "Card aaaa001"}]));
    let row = db.dev_mode(space.as_str()).await.unwrap().unwrap();
    assert_eq!(row.seen_at, Some(looked), "opening the desk never moves the rollup's seen_at");
    assert!(row.reviews_seen_at > Some(looked));

    // Mark seen: the rollup empties, and the badge is left alone
    tick().await;
    card(db, &space, REVIEW, "pr:8").await;
    assert_eq!(badge(&env.app, &space).await, 1);
    post(&env.app, &mark_seen).await;
    let r = rollup(&env.app, &space).await;
    assert_eq!((r["count"].clone(), r["areas"].clone()), (json!(0), json!([])), "Mark seen empties the rollup: {r}");
    assert_eq!(badge(&env.app, &space).await, 1, "Mark seen doesn't clear the review badge");
    post(&env.app, &desk_seen).await;
    assert_eq!(badge(&env.app, &space).await, 0);
    assert_eq!(rollup(&env.app, &space).await["count"], 0);
}

#[tokio::test]
async fn a_desk_never_opened_shows_no_badge_however_often_mark_seen_is_pressed() {
    let env = setup().await;
    let space = SpaceId::default_space();
    let db = &env.state.db;
    db.put_dev_mode(space.as_str(), true, REPO, "main", None, None).await.unwrap();
    post(&env.app, &format!("{}/seen", landed_uri(&space))).await;
    tick().await;
    card(db, &space, REVIEW, "pr:9").await;
    assert_eq!(badge(&env.app, &space).await, 0, "never opened counts nothing (F6)");
}

async fn raw(db_file: &Path) -> sqlx::SqlitePool {
    let opts = SqliteConnectOptions::from_str(&format!("sqlite://{}", db_file.display())).unwrap();
    SqlitePoolOptions::new().max_connections(1).connect_with(opts).await.unwrap()
}

async fn columns(db_file: &Path) -> Vec<String> {
    let pool = raw(db_file).await;
    let cols = sqlx::query_scalar("SELECT name FROM pragma_table_info('dev_mode_spaces')").fetch_all(&pool).await.unwrap();
    pool.close().await;
    cols
}

#[tokio::test]
async fn the_migration_runs_twice_cleanly_and_backfills_only_once() {
    let tmp = tempfile::tempdir().unwrap();
    let data = tmp.path().join("data");
    let space = SpaceId::default_space();
    let db = Database::init(&data).await.unwrap();
    db.put_dev_mode(space.as_str(), true, REPO, "main", None, None).await.unwrap();
    // Mark seen before the desk was ever opened: the badge's time stays empty
    let at = chrono::Utc::now();
    db.dev_mode_seen(space.as_str(), at).await.unwrap();
    drop(db);

    for _ in 0..2 {
        let db = Database::init(&data).await.expect("a second start migrates cleanly");
        let row = db.dev_mode(space.as_str()).await.unwrap().unwrap();
        assert_eq!((row.seen_at, row.reviews_seen_at), (Some(at), None), "a restart never copies a Mark seen into the badge");
        drop(db);
    }
    let cols = columns(&data.join("kwnt.db")).await;
    assert_eq!(cols.iter().filter(|c| *c == "reviews_seen_at").count(), 1, "{cols:?}");
}

/// A server on the database in `data`, as a person's install opens it.
async fn app_on(data: &Path) -> (axum::Router, AppState) {
    prompts_dir();
    no_network_scans();
    let db = Database::init(data).await.unwrap();
    let config = Config {
        data_dir: data.to_path_buf(),
        host: "127.0.0.1".into(),
        port: 0,
        laya_url: "http://127.0.0.1:9".into(),
        browser_cdp_url: Some("http://127.0.0.1:9".into()),
        ..Config::default()
    };
    let state = AppState::new(config.clone(), db, ToolRunner::new(data.join("workspace")), PermissionBroker::new(), LayaRouter::new(config.laya_url.clone()));
    (create_router(state.clone()), state)
}

#[tokio::test]
async fn an_old_database_with_only_seen_at_keeps_its_badge() {
    let tmp = tempfile::tempdir().unwrap();
    let data = tmp.path().join("data");
    let space = SpaceId::default_space();

    // A 0.7.1 database: one seen_at, set when the desk was last opened, with a review before it
    // and one after it
    let db = Database::init(&data).await.unwrap();
    db.put_dev_mode(space.as_str(), true, REPO, "main", None, None).await.unwrap();
    card(&db, &space, REVIEW, "pr:1").await;
    tick().await;
    let opened = chrono::Utc::now();
    db.dev_mode_seen(space.as_str(), opened).await.unwrap();
    tick().await;
    card(&db, &space, REVIEW, "pr:2").await;
    drop(db);
    let pool = raw(&data.join("kwnt.db")).await;
    sqlx::query("ALTER TABLE dev_mode_spaces DROP COLUMN reviews_seen_at").execute(&pool).await.unwrap();
    pool.close().await;
    assert!(!columns(&data.join("kwnt.db")).await.contains(&"reviews_seen_at".to_string()), "set up as 0.7.1 left it");

    // 0.7.2 opens it: the badge's time is the old seen_at, so the badge is what 0.7.1 showed
    let (app, state) = app_on(&data).await;
    let row = state.db.dev_mode(space.as_str()).await.unwrap().unwrap();
    assert_eq!((row.seen_at, row.reviews_seen_at), (Some(opened), Some(opened)), "backfilled from seen_at");
    assert_eq!(badge(&app, &space).await, 1, "only the review after the last look counts, as before");
}
