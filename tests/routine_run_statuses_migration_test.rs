//! 26b: routine runs can end `needs_you` or `incomplete`. A database whose `routine_runs` still
//! has the old status rule is rebuilt once at start: every run, column and index kept, and a
//! restart still ends a run that was going as "kwnt stopped during the run".

use kwnt::db::Database;
use sqlx::sqlite::{SqliteConnectOptions, SqlitePool, SqlitePoolOptions};
use std::path::Path;
use std::str::FromStr;

async fn raw(path: &Path) -> SqlitePool {
    let opts = SqliteConnectOptions::from_str(&format!("sqlite://{}", path.display())).unwrap();
    SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(opts)
        .await
        .unwrap()
}

async fn exec(pool: &SqlitePool, sql: &str) {
    sqlx::query(sql)
        .execute(pool)
        .await
        .unwrap_or_else(|e| panic!("{sql}: {e}"));
}

/// `routine_runs` as 0.9.20 leaves it: the old status rule, a run of each kind and one going.
async fn legacy(dir: &Path) {
    Database::init(dir).await.unwrap();
    let pool = raw(&dir.join("kwnt.db")).await;
    for sql in [
        "DROP TABLE routine_runs",
        "CREATE TABLE routine_runs (
            id TEXT PRIMARY KEY,
            routine_id TEXT NOT NULL,
            trigger TEXT NOT NULL CHECK (trigger IN ('schedule', 'manual')),
            started_at TIMESTAMP NOT NULL,
            ended_at TIMESTAMP,
            status TEXT NOT NULL CHECK (status IN ('running', 'ok', 'error', 'skipped')),
            error TEXT,
            message_id TEXT
        , note TEXT)",
        "CREATE INDEX routine_runs_by_routine ON routine_runs (routine_id, started_at)",
        "CREATE INDEX routine_runs_by_end ON routine_runs (status, ended_at) WHERE ended_at IS NOT NULL",
        "INSERT INTO routine_runs VALUES
            ('run_ok', 'r1', 'schedule', '2026-11-01T09:00:00Z', '2026-11-01T09:01:00Z', 'ok', NULL, 'msg_1', NULL),
            ('run_err', 'r1', 'manual', '2026-11-02T09:00:00Z', '2026-11-02T09:01:00Z', 'error', 'boom', NULL, 'Ran late'),
            ('run_going', 'r1', 'schedule', '2026-11-03T09:00:00Z', NULL, 'running', NULL, NULL, NULL)",
    ] {
        exec(&pool, sql).await;
    }
    let refused = sqlx::query("UPDATE routine_runs SET status = 'needs_you' WHERE id = 'run_ok'")
        .execute(&pool)
        .await;
    assert!(refused.is_err(), "the old rule refuses the new statuses");
    pool.close().await;
}

type Row = (
    String,
    String,
    String,
    Option<String>,
    Option<String>,
    Option<String>,
);

async fn runs(pool: &SqlitePool) -> Vec<Row> {
    sqlx::query_as(
        "SELECT id, trigger, status, error, message_id, note FROM routine_runs ORDER BY rowid",
    )
    .fetch_all(pool)
    .await
    .unwrap()
}

#[tokio::test]
async fn old_run_history_is_kept_and_takes_the_new_statuses() {
    let tmp = tempfile::tempdir().unwrap();
    legacy(tmp.path()).await;
    let before = runs(&raw(&tmp.path().join("kwnt.db")).await).await;

    let db = Database::init(tmp.path()).await.unwrap();
    let pool = raw(&tmp.path().join("kwnt.db")).await;
    assert_eq!(runs(&pool).await, before, "every run kept as it was");
    let indexes: Vec<(String,)> = sqlx::query_as("SELECT name FROM sqlite_master WHERE type = 'index' AND tbl_name = 'routine_runs' AND sql IS NOT NULL ORDER BY name")
        .fetch_all(&pool)
        .await
        .unwrap();
    assert_eq!(
        indexes,
        vec![
            ("routine_runs_by_end".to_string(),),
            ("routine_runs_by_routine".to_string(),)
        ]
    );
    pool.close().await;

    // The new statuses are taken; anything else still isn't
    db.finish_routine_run(
        "run_ok",
        "needs_you",
        Some("A step is waiting for your approval, so it didn't run"),
        None,
    )
    .await
    .unwrap();
    db.finish_routine_run("run_err", "incomplete", Some("502"), None)
        .await
        .unwrap();
    assert!(db
        .finish_routine_run("run_err", "great", None, None)
        .await
        .is_err());

    // A restart still ends the run that was going, as before
    assert_eq!(db.fail_interrupted_runs().await.unwrap(), 1);
    let pool = raw(&tmp.path().join("kwnt.db")).await;
    let after = runs(&pool).await;
    assert_eq!(after[0].2, "needs_you");
    assert_eq!(after[1].2, "incomplete");
    assert_eq!(
        (after[2].2.as_str(), after[2].3.as_deref()),
        ("error", Some("kwnt stopped during the run"))
    );

    // Once is enough: a second start changes nothing
    drop(db);
    Database::init(tmp.path()).await.unwrap();
    assert_eq!(runs(&raw(&tmp.path().join("kwnt.db")).await).await, after);
}
