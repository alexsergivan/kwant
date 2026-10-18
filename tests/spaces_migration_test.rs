//! Spaces S0: the migration keeps every row, backs up first, runs once and leaves the old schema
//! when it can't finish.

use kwnt::db::custom_api::CUSTOM_API_SCHEMA_VERSION;
use kwnt::db::spaces::DEFAULT_SPACE_ID;
use kwnt::db::Database;
use sqlx::sqlite::{SqliteConnectOptions, SqlitePool, SqlitePoolOptions};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::str::FromStr;

async fn raw(path: &Path) -> SqlitePool {
    let opts = SqliteConnectOptions::from_str(&format!("sqlite://{}", path.display()))
        .unwrap()
        .foreign_keys(false);
    SqlitePoolOptions::new().max_connections(1).connect_with(opts).await.unwrap()
}

async fn exec(pool: &SqlitePool, sql: &str) {
    sqlx::query(sql).execute(pool).await.unwrap_or_else(|e| panic!("{sql}: {e}"));
}

/// A database as the release before spaces leaves it, with some of everything in it.
async fn legacy(dir: &Path) -> SqlitePool {
    Database::init_without_spaces(dir).await.unwrap();
    let pool = raw(&dir.join("kwnt.db")).await;
    for sql in [
        "INSERT INTO kwants (id, name, title, persona_prompt, avatar_sprite, profile_id) VALUES
            ('k_ada', 'Ada', 'Lead', 'p', 'architect', 'prf_personal'),
            ('k_lin', 'Lin', 'Backend', 'p', 'devops', NULL)",
        "INSERT INTO channels (id, name, lead_kwant_id, member_kwants) VALUES ('c_eng', 'eng', 'k_ada', '[\"k_ada\",\"k_lin\"]')",
        "INSERT INTO teams (id, name, lead_kwant_id, member_kwants, workspace_path, instructions) VALUES
            ('t_web', 'web', 'k_ada', '[\"k_ada\"]', '/w/web', 'ship it'),
            ('t_Web', 'Web', 'k_lin', '[\"k_lin\"]', NULL, NULL)",
        "UPDATE llm_profiles SET is_default = 0 WHERE id = 'prf_system_claude'",
        "UPDATE llm_profiles SET enabled = 0, priority = 7 WHERE id = 'prf_system_codex'",
        "INSERT INTO llm_profiles (id, engine, name, kind, home_dir, priority, enabled, is_default) VALUES
            ('prf_personal', 'claude', 'Personal', 'cli_home', '/h/personal', 1, 1, 1),
            ('prf_work', 'claude', 'Work', 'cli_home', '/h/work', 2, 0, 0),
            ('prf_grok_key', 'grok', 'Key', 'api_key', NULL, 5, 1, 0)",
        "INSERT INTO llm_profile_secrets (profile_id, api_key) VALUES ('prf_grok_key', 'xai-secret')",
        "INSERT INTO llm_profile_state (profile_id, limited_until, last_error) VALUES ('prf_work', '2030-01-01T00:00:00Z', 'auth')",
        "INSERT INTO mcp_servers (id, name, transport, command, url, env) VALUES
            ('m_fs', 'fs', 'stdio', 'mcp-fs', NULL, '{\"A\":\"1\"}'),
            ('m_gh', 'github', 'http', NULL, 'https://mcp.example', '{}')",
        "INSERT INTO kwant_mcp_disabled (kwant_id, server_id) VALUES ('k_lin', 'm_fs')",
        "INSERT INTO mcp_oauth_tokens (server_id, access_token, token_endpoint, client_id, resource) VALUES ('m_gh', 'tok', 'https://t', 'cid', 'r')",
        "INSERT INTO mcp_oauth_clients (server_id, redirect_uri, issuer, client_id) VALUES ('m_gh', 'http://cb', 'https://iss', 'cid')",
        "INSERT INTO routines (id, kwant_id, channel_id, name, instructions, schedule) VALUES ('r_1', 'k_lin', 'c_eng', 'daily', 'do', '{}')",
        "INSERT INTO routine_runs (id, routine_id, trigger, started_at, status) VALUES ('rr_1', 'r_1', 'manual', '2026-01-01', 'ok')",
        "INSERT INTO skills (id, kwant_id, name, description, body, status, source, created_at, updated_at) VALUES
            ('s_shared', NULL, 'deploy', 'd', 'b', 'enabled', 'user', '2026-01-01', '2026-01-01'),
            ('s_lin', 'k_lin', 'deploy', 'd', 'b', 'enabled', 'agent', '2026-01-01', '2026-01-01')",
        "INSERT INTO messages (id, channel_id, direct_kwant_id, sender_role, sender_id, text) VALUES
            ('msg_1', 'c_eng', NULL, 'user', 'u', 'hello migration'),
            ('msg_2', NULL, 'k_ada', 'kwant', 'k_ada', 'direct reply')",
        "INSERT INTO kwant_memories (id, kwant_id, text) VALUES ('mem_1', 'k_ada', 'likes sqlite')",
        "INSERT INTO settings (key, value) VALUES ('laya_url', 'http://laya')",
    ] {
        exec(&pool, sql).await;
    }
    pool
}

/// Every row of every table except `spaces` and `llm_profile_endpoints` (both new, the second
/// from the Custom API change), without `space_id`, as quoted text.
async fn snapshot(pool: &SqlitePool) -> BTreeMap<String, Vec<String>> {
    let tables: Vec<(String,)> = sqlx::query_as(
        "SELECT name FROM sqlite_master WHERE type = 'table' AND name NOT LIKE 'sqlite_%' AND name NOT IN ('spaces', 'llm_profile_endpoints', 'mcp_suggestions') ORDER BY name",
    )
    .fetch_all(pool)
    .await
    .unwrap();
    let mut out = BTreeMap::new();
    for (table,) in tables {
        let cols: Vec<(String,)> = sqlx::query_as("SELECT name FROM pragma_table_info(?) WHERE name NOT IN ('space_id', 'paused_by_archive') ORDER BY cid")
            .bind(&table)
            .fetch_all(pool)
            .await
            .unwrap();
        let expr = cols.iter().map(|(c,)| format!("quote(\"{c}\")")).collect::<Vec<_>>().join(" || '|' || ");
        let mut rows: Vec<String> = sqlx::query_scalar(&format!("SELECT {expr} FROM \"{table}\""))
            .fetch_all(pool)
            .await
            .unwrap();
        rows.sort();
        out.insert(table, rows);
    }
    out
}

async fn scalar_i64(pool: &SqlitePool, sql: &str) -> i64 {
    sqlx::query_scalar(sql).fetch_one(pool).await.unwrap()
}

async fn has_column(pool: &SqlitePool, table: &str, col: &str) -> bool {
    scalar_i64(pool, &format!("SELECT COUNT(*) FROM pragma_table_info('{table}') WHERE name = '{col}'")).await == 1
}

fn backups(dir: &Path) -> Vec<PathBuf> {
    match std::fs::read_dir(dir.join("backups")) {
        Ok(entries) => entries.map(|e| e.unwrap().path()).collect(),
        Err(_) => vec![],
    }
}

const SPACE_TABLES: [&str; 7] = ["kwants", "channels", "teams", "llm_profiles", "mcp_servers", "routines", "skills"];

#[tokio::test]
async fn migration_keeps_every_row_and_id_and_backs_up_first() {
    let tmp = tempfile::tempdir().unwrap();
    let pool = legacy(tmp.path()).await;
    let before = snapshot(&pool).await;
    assert_eq!(scalar_i64(&pool, "PRAGMA user_version").await, 0);

    let db = Database::init(tmp.path()).await.unwrap();
    assert_eq!(db.schema_version().await.unwrap(), CUSTOM_API_SCHEMA_VERSION);

    let after = snapshot(&pool).await;
    assert_eq!(before, after, "every row and id survives, nothing added or lost");

    assert_eq!(
        sqlx::query_as::<_, (String, String)>("SELECT id, name FROM spaces").fetch_all(&pool).await.unwrap(),
        vec![(DEFAULT_SPACE_ID.to_string(), "Default".to_string())]
    );
    for table in SPACE_TABLES {
        let elsewhere = scalar_i64(&pool, &format!("SELECT COUNT(*) FROM {table} WHERE space_id != '{DEFAULT_SPACE_ID}'")).await;
        assert_eq!(elsewhere, 0, "{table} rows all in the Default space");
        assert!(scalar_i64(&pool, &format!("SELECT COUNT(*) FROM {table}")).await > 0, "{table} has rows to check");
    }

    // This machine keeps its fixed ids and settings; Personal stays the Claude default
    let profiles: Vec<(String, i64, i64, i64)> =
        sqlx::query_as("SELECT id, priority, enabled, is_default FROM llm_profiles WHERE kind = 'system' ORDER BY id")
            .fetch_all(&pool)
            .await
            .unwrap();
    assert_eq!(
        profiles,
        vec![
            ("prf_system_claude".into(), 0, 1, 0),
            ("prf_system_codex".into(), 7, 0, 1),
            ("prf_system_grok".into(), 0, 1, 1),
        ]
    );
    let claude_default = db.list_profiles(&kwnt::db::spaces::SpaceId::default_space()).await.unwrap().into_iter().find(|p| p.engine == "claude" && p.is_default);
    assert_eq!(claude_default.map(|p| p.id), Some("prf_personal".into()));

    let fk: Vec<(String, Option<i64>, String, i64)> = sqlx::query_as("PRAGMA foreign_key_check").fetch_all(&pool).await.unwrap();
    assert!(fk.is_empty(), "{fk:?}");
    let integrity: String = sqlx::query_scalar("PRAGMA integrity_check").fetch_one(&pool).await.unwrap();
    assert_eq!(integrity, "ok");

    // Child foreign keys still follow the rebuilt parents
    exec(&pool, "PRAGMA foreign_keys = ON").await;
    exec(&pool, "DELETE FROM mcp_servers WHERE id = 'm_gh'").await;
    exec(&pool, "DELETE FROM llm_profiles WHERE id = 'prf_grok_key'").await;
    assert_eq!(scalar_i64(&pool, "SELECT COUNT(*) FROM mcp_oauth_tokens").await, 0);
    assert_eq!(scalar_i64(&pool, "SELECT COUNT(*) FROM llm_profile_secrets").await, 0);
    assert!(sqlx::query("INSERT INTO kwant_mcp_disabled (kwant_id, server_id) VALUES ('k_ada', 'nope')")
        .execute(&pool)
        .await
        .is_err());

    // The backup is the database as it was before the migration touched it
    let files = backups(tmp.path());
    assert_eq!(files.len(), 1, "{files:?}");
    let backup = raw(&files[0]).await;
    assert_eq!(scalar_i64(&backup, "PRAGMA user_version").await, 0);
    assert!(!has_column(&backup, "teams", "space_id").await);
    assert_eq!(snapshot(&backup).await, before);
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = |p: &Path| std::fs::metadata(p).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode(&files[0]), 0o600, "the copy holds API keys");
        assert_eq!(mode(&tmp.path().join("backups")), 0o700);
    }
}

#[tokio::test]
async fn migration_runs_once() {
    let tmp = tempfile::tempdir().unwrap();
    let pool = legacy(tmp.path()).await;
    Database::init(tmp.path()).await.unwrap();
    let first = snapshot(&pool).await;
    let schema: Vec<(String,)> = sqlx::query_as("SELECT sql FROM sqlite_master WHERE sql IS NOT NULL ORDER BY name").fetch_all(&pool).await.unwrap();

    for _ in 0..2 {
        let db = Database::init(tmp.path()).await.unwrap();
        assert_eq!(db.schema_version().await.unwrap(), CUSTOM_API_SCHEMA_VERSION);
    }
    assert_eq!(snapshot(&pool).await, first);
    let again: Vec<(String,)> = sqlx::query_as("SELECT sql FROM sqlite_master WHERE sql IS NOT NULL ORDER BY name").fetch_all(&pool).await.unwrap();
    assert_eq!(schema, again);
    assert_eq!(scalar_i64(&pool, "SELECT COUNT(*) FROM spaces").await, 1);
    assert_eq!(backups(tmp.path()).len(), 1, "no second backup");
}

/// Starts kwnt on `dir`, expecting it to refuse; returns the reason it gave.
async fn refused_start(dir: &std::path::Path) -> String {
    match Database::init(dir).await {
        Ok(_) => panic!("startup should refuse a database below schema version 1"),
        Err(e) => e.to_string(),
    }
}

#[tokio::test]
async fn backup_failure_refuses_startup_leaves_the_old_schema_and_retries_next_start() {
    let tmp = tempfile::tempdir().unwrap();
    let pool = legacy(tmp.path()).await;
    let before = snapshot(&pool).await;
    // A file where the backups folder should go makes the backup fail
    std::fs::write(tmp.path().join("backups"), "not a folder").unwrap();

    // Startup refuses rather than run without spaces, and leaves the old schema as it was
    let refused = refused_start(tmp.path()).await;
    assert!(refused.contains("can't start"), "{refused}");
    assert!(refused.contains("Nothing was changed"), "{refused}");
    assert_eq!(scalar_i64(&pool, "SELECT COUNT(*) FROM sqlite_master WHERE name = 'spaces'").await, 0);
    for table in SPACE_TABLES {
        assert!(!has_column(&pool, table, "space_id").await, "{table} untouched");
    }
    assert_eq!(snapshot(&pool).await, before);

    std::fs::remove_file(tmp.path().join("backups")).unwrap();
    let db = Database::init(tmp.path()).await.unwrap();
    assert_eq!(db.schema_version().await.unwrap(), CUSTOM_API_SCHEMA_VERSION);
    assert_eq!(backups(tmp.path()).len(), 1);
}

#[tokio::test]
async fn failed_schema_change_rolls_back_everything_and_refuses_startup() {
    let tmp = tempfile::tempdir().unwrap();
    let pool = legacy(tmp.path()).await;
    // A column the rebuilt table doesn't know: the migration must refuse rather than drop it
    exec(&pool, "ALTER TABLE mcp_servers ADD COLUMN local_note TEXT").await;
    exec(&pool, "UPDATE mcp_servers SET local_note = 'keep me'").await;
    let before = snapshot(&pool).await;

    let refused = refused_start(tmp.path()).await;
    assert!(refused.contains("rolled back"), "{refused}");
    assert_eq!(snapshot(&pool).await, before);
    assert_eq!(scalar_i64(&pool, "SELECT COUNT(*) FROM sqlite_master WHERE name = 'spaces'").await, 0);
    // Steps before the failing one were rolled back too
    assert!(!has_column(&pool, "kwants", "space_id").await);
    assert!(!has_column(&pool, "teams", "space_id").await);
    assert_eq!(scalar_i64(&pool, "SELECT COUNT(*) FROM sqlite_master WHERE name LIKE '%__spaces_new'").await, 0);
    // The backup was written before the attempt
    assert_eq!(backups(tmp.path()).len(), 1);
}

#[tokio::test]
async fn repeated_failed_starts_keep_one_current_backup() {
    let tmp = tempfile::tempdir().unwrap();
    let pool = legacy(tmp.path()).await;
    exec(&pool, "ALTER TABLE mcp_servers ADD COLUMN local_note TEXT").await;
    // A copy cut off by a kill from an earlier start
    std::fs::create_dir_all(tmp.path().join("backups")).unwrap();
    std::fs::write(tmp.path().join("backups/kwnt-pre-spaces-20200101T000000Z.db.partial"), "cut off").unwrap();

    refused_start(tmp.path()).await;
    assert_eq!(backups(tmp.path()).len(), 1, "{:?}", backups(tmp.path()));
    // The previous release may write in between: the next backup must hold that, not the old copy
    exec(&pool, "INSERT INTO settings (key, value) VALUES ('written_between_starts', 'yes')").await;
    let before = snapshot(&pool).await;
    refused_start(tmp.path()).await;

    let files = backups(tmp.path());
    assert_eq!(files.len(), 1, "two failed starts leave one backup: {files:?}");
    assert!(files[0].extension().is_some_and(|e| e == "db"), "{files:?}");
    let backup = raw(&files[0]).await;
    assert_eq!(scalar_i64(&backup, "PRAGMA user_version").await, 0);
    assert_eq!(snapshot(&backup).await, before);
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(std::fs::metadata(&files[0]).unwrap().permissions().mode() & 0o777, 0o600);
    }
}

#[tokio::test]
async fn foreign_key_problems_already_in_the_data_do_not_block_it() {
    let tmp = tempfile::tempdir().unwrap();
    let pool = legacy(tmp.path()).await;
    exec(&pool, "INSERT INTO teams (id, name, lead_kwant_id, member_kwants) VALUES ('t_orphan', 'orphan', 'k_gone', '[]')").await;
    exec(&pool, "INSERT INTO llm_profile_state (profile_id, last_error) VALUES ('prf_gone', 'auth')").await;
    let count = |pool: SqlitePool| async move {
        sqlx::query_as::<_, (String, Option<i64>, String, i64)>("PRAGMA foreign_key_check").fetch_all(&pool).await.unwrap().len()
    };
    assert_eq!(count(pool.clone()).await, 2);

    let db = Database::init(tmp.path()).await.unwrap();
    assert_eq!(db.schema_version().await.unwrap(), CUSTOM_API_SCHEMA_VERSION);
    assert_eq!(count(pool.clone()).await, 2, "the same two, none new");
}

#[tokio::test]
async fn names_are_unique_per_space() {
    let tmp = tempfile::tempdir().unwrap();
    let pool = legacy(tmp.path()).await;
    Database::init(tmp.path()).await.unwrap();
    exec(&pool, "INSERT INTO spaces (id, name) VALUES ('spc_two', 'Personal')").await;

    let ok = |sql: &'static str| {
        let pool = pool.clone();
        async move { sqlx::query(sql).execute(&pool).await.map(|_| ()) }
    };
    // The same names are fine in another space
    ok("INSERT INTO teams (id, name, lead_kwant_id, member_kwants, space_id) VALUES ('t2', 'web', 'k_ada', '[]', 'spc_two')").await.unwrap();
    ok("INSERT INTO mcp_servers (id, name, transport, space_id) VALUES ('m2', 'fs', 'stdio', 'spc_two')").await.unwrap();
    ok("INSERT INTO llm_profiles (id, engine, name, kind, is_default, space_id) VALUES ('p2', 'claude', 'personal', 'cli_home', 1, 'spc_two')").await.unwrap();
    ok("INSERT INTO skills (id, name, description, body, status, source, created_at, updated_at, space_id) VALUES ('s2', 'deploy', 'd', 'b', 'enabled', 'user', 'x', 'x', 'spc_two')").await.unwrap();

    // ...and still refused within one, with the old case rules
    ok("INSERT INTO teams (id, name, lead_kwant_id, member_kwants) VALUES ('t3', 'web', 'k_ada', '[]')").await.unwrap_err();
    ok("INSERT INTO teams (id, name, lead_kwant_id, member_kwants, space_id) VALUES ('t4', 'web', 'k_ada', '[]', 'spc_two')").await.unwrap_err();
    ok("INSERT INTO mcp_servers (id, name, transport) VALUES ('m3', 'fs', 'stdio')").await.unwrap_err();
    ok("INSERT INTO llm_profiles (id, engine, name, kind) VALUES ('p3', 'claude', 'PERSONAL', 'cli_home')").await.unwrap_err();
    ok("INSERT INTO llm_profiles (id, engine, name, kind, is_default, space_id) VALUES ('p4', 'claude', 'Other', 'cli_home', 1, 'spc_two')").await.unwrap_err();
    ok("INSERT INTO skills (id, name, description, body, status, source, created_at, updated_at) VALUES ('s3', 'deploy', 'd', 'b', 'enabled', 'user', 'x', 'x')").await.unwrap_err();
    // home_dir stays unique across spaces: two accounts can't share one login folder
    ok("INSERT INTO llm_profiles (id, engine, name, kind, home_dir, space_id) VALUES ('p5', 'claude', 'Dup', 'cli_home', '/h/personal', 'spc_two')").await.unwrap_err();
}

#[tokio::test]
async fn fresh_database_starts_with_spaces_and_no_backup() {
    let tmp = tempfile::tempdir().unwrap();
    let db = Database::init(tmp.path()).await.unwrap();
    assert_eq!(db.schema_version().await.unwrap(), CUSTOM_API_SCHEMA_VERSION);
    assert!(backups(tmp.path()).is_empty());

    let pool = raw(&tmp.path().join("kwnt.db")).await;
    assert!(scalar_i64(&pool, "SELECT COUNT(*) FROM kwants").await >= 3, "starters seeded");
    for table in ["kwants", "channels", "teams", "llm_profiles"] {
        assert_eq!(scalar_i64(&pool, &format!("SELECT COUNT(*) FROM {table} WHERE space_id != '{DEFAULT_SPACE_ID}'")).await, 0);
    }
    // The existing DB layer writes through unchanged and lands in Default
    let team = db
        .create_team(&kwnt::db::spaces::SpaceId::default_space(), &kwnt::db::models::NewTeam {
            name: "web".into(),
            description: None,
            icon: None,
            lead_kwant_id: db.list_kwants(&kwnt::db::spaces::SpaceId::default_space()).await.unwrap()[0].id.clone(),
            member_kwants: vec![],
            workspace_path: None,
            repo_url: None,
        })
        .await
        .unwrap();
    let space: String = sqlx::query_scalar("SELECT space_id FROM teams WHERE id = ?").bind(&team.id).fetch_one(&pool).await.unwrap();
    assert_eq!(space, DEFAULT_SPACE_ID);
}
