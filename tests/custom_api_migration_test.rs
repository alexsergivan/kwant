//! Schema version 2 (Custom API accounts): the `llm_profiles` rebuild keeps every row, index and
//! rule, and a failed change leaves version 1 running without Custom API accounts.

use kwnt::db::custom_api::{ProfileEndpoint, CUSTOM_API_SCHEMA_VERSION};
use kwnt::db::spaces::SPACES_SCHEMA_VERSION;
use kwnt::db::Database;
use sqlx::sqlite::{SqliteConnectOptions, SqlitePool, SqlitePoolOptions};
use std::path::Path;
use std::str::FromStr;

async fn raw(path: &Path) -> SqlitePool {
    let opts = SqliteConnectOptions::from_str(&format!("sqlite://{}", path.display()))
        .unwrap()
        .foreign_keys(true);
    SqlitePoolOptions::new().max_connections(1).connect_with(opts).await.unwrap()
}

async fn exec(pool: &SqlitePool, sql: &str) -> Result<(), sqlx::Error> {
    sqlx::query(sql).execute(pool).await.map(|_| ())
}

/// A database as the release before spaces leaves it, with accounts of every kind.
async fn legacy(dir: &Path) {
    Database::init_without_spaces(dir).await.unwrap();
    let pool = raw(&dir.join("kwnt.db")).await;
    for sql in [
        "UPDATE llm_profiles SET is_default = 0 WHERE id = 'prf_system_claude'",
        "INSERT INTO llm_profiles (id, engine, name, kind, home_dir, priority, enabled, is_default) VALUES
            ('prf_personal', 'claude', 'Personal', 'cli_home', '/h/personal', 1, 1, 1),
            ('prf_grok_key', 'grok', 'Key', 'api_key', NULL, 5, 0, 0)",
        "INSERT INTO llm_profile_secrets (profile_id, api_key) VALUES ('prf_grok_key', 'xai-secret')",
        "INSERT INTO llm_profile_state (profile_id, limited_until, last_error) VALUES ('prf_personal', '2030-01-01T00:00:00Z', 'quota')",
    ] {
        exec(&pool, sql).await.unwrap_or_else(|e| panic!("{sql}: {e}"));
    }
    pool.close().await;
}

async fn profiles(pool: &SqlitePool) -> Vec<(String, String, String, String, i64, bool, bool)> {
    sqlx::query_as(
        "SELECT id, engine, name, kind, priority, enabled, is_default FROM llm_profiles ORDER BY id",
    )
    .fetch_all(pool)
    .await
    .unwrap()
}

#[tokio::test]
async fn the_rebuild_keeps_every_account_and_rule_and_adds_custom() {
    let tmp = tempfile::tempdir().unwrap();
    legacy(tmp.path()).await;
    let before = profiles(&raw(&tmp.path().join("kwnt.db")).await).await;

    let db = Database::init(tmp.path()).await.unwrap();
    assert_eq!(db.schema_version().await.unwrap(), CUSTOM_API_SCHEMA_VERSION);
    assert!(db.custom_api_ready().await);
    let pool = raw(&tmp.path().join("kwnt.db")).await;
    assert_eq!(profiles(&pool).await, before, "every account kept as it was");
    let secret: (String,) = sqlx::query_as("SELECT api_key FROM llm_profile_secrets WHERE profile_id = 'prf_grok_key'")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(secret.0, "xai-secret");
    let state: (String,) = sqlx::query_as("SELECT last_error FROM llm_profile_state WHERE profile_id = 'prf_personal'")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(state.0, "quota");

    // The rules survive the rebuild: one default per engine and space, names unique per space
    // regardless of case, and only known engines and kinds
    assert!(exec(&pool, "INSERT INTO llm_profiles (id, engine, name, kind, is_default) VALUES ('p2', 'claude', 'Second', 'cli_home', 1)").await.is_err());
    assert!(exec(&pool, "INSERT INTO llm_profiles (id, engine, name, kind) VALUES ('p3', 'claude', 'PERSONAL', 'cli_home')").await.is_err());
    assert!(exec(&pool, "INSERT INTO llm_profiles (id, engine, name, kind) VALUES ('p4', 'bogus', 'X', 'api_key')").await.is_err());
    assert!(exec(&pool, "INSERT INTO llm_profiles (id, engine, name, kind) VALUES ('p5', 'custom', 'X', 'bogus')").await.is_err());
    exec(&pool, "INSERT INTO llm_profiles (id, engine, name, kind) VALUES ('p_custom', 'custom', 'Maestro', 'custom_api')").await.unwrap();
    // Same name on another engine is fine, as before
    exec(&pool, "INSERT INTO llm_profiles (id, engine, name, kind) VALUES ('p_custom2', 'custom', 'Personal', 'custom_api')").await.unwrap();
    pool.close().await;

    let endpoint = ProfileEndpoint {
        profile_id: "p_custom".into(),
        protocol: "openai-chat".into(),
        base_url: "http://localhost:23333/api/openai/v1".into(),
        model: Some("gpt-4.1".into()),
    };
    db.set_profile_endpoint(&endpoint).await.unwrap();
    assert_eq!(db.get_profile_endpoint("p_custom").await.unwrap(), Some(endpoint.clone()));
    // Upsert
    let changed = ProfileEndpoint { model: None, ..endpoint };
    db.set_profile_endpoint(&changed).await.unwrap();
    assert_eq!(db.get_profile_endpoint("p_custom").await.unwrap(), Some(changed));
    assert_eq!(db.list_profile_endpoints().await.unwrap().len(), 1);
    db.delete_profile("p_custom").await.unwrap();
    assert_eq!(db.get_profile_endpoint("p_custom").await.unwrap(), None);
    drop(db);

    // Runs once: a second start changes nothing
    let db = Database::init(tmp.path()).await.unwrap();
    assert_eq!(db.schema_version().await.unwrap(), CUSTOM_API_SCHEMA_VERSION);
    let pool = raw(&tmp.path().join("kwnt.db")).await;
    assert_eq!(profiles(&pool).await.len(), before.len() + 1);
}

#[tokio::test]
async fn a_failed_change_rolls_back_and_keeps_running_on_version_1() {
    let tmp = tempfile::tempdir().unwrap();
    legacy(tmp.path()).await;
    // A table in the way makes the change fail part-way, after the rebuild
    let pool = raw(&tmp.path().join("kwnt.db")).await;
    exec(&pool, "CREATE TABLE llm_profile_endpoints (x TEXT)").await.unwrap();
    pool.close().await;

    let db = Database::init(tmp.path()).await.expect("startup never fails on it");
    assert_eq!(db.schema_version().await.unwrap(), SPACES_SCHEMA_VERSION);
    assert!(!db.custom_api_ready().await);
    assert_eq!(db.get_profile_endpoint("x").await.unwrap(), None);
    let pool = raw(&tmp.path().join("kwnt.db")).await;
    // The rebuild was rolled back: the old rules, without custom
    assert!(exec(&pool, "INSERT INTO llm_profiles (id, engine, name, kind) VALUES ('p_c', 'custom', 'M', 'custom_api')").await.is_err());
    assert_eq!(profiles(&pool).await.len(), 5, "3 system + 2 accounts, all kept");

    // Out of the way: the next start finishes it
    exec(&pool, "DROP TABLE llm_profile_endpoints").await.unwrap();
    pool.close().await;
    drop(db);
    let db = Database::init(tmp.path()).await.unwrap();
    assert_eq!(db.schema_version().await.unwrap(), CUSTOM_API_SCHEMA_VERSION);
}
