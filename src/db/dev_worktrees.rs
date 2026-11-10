//! H1 + P3 (0.9.0, release-090/h1-contract.md): kwant's record of the worktrees it made, one row
//! per `git worktree add`. The row is the only proof kwant has that it made a folder, so nothing
//! without a row is ever removed or cleaned. Kept outside exports like the other Dev mode tables.
//! Add-only.

use super::{Database, DbError};
use sqlx::FromRow;

const TABLES: &[&str] = &[
    "CREATE TABLE IF NOT EXISTS dev_worktrees (
        id TEXT PRIMARY KEY,
        space_id TEXT NOT NULL,
        team_id TEXT NOT NULL,
        repo_dir TEXT NOT NULL,
        common_dir TEXT NOT NULL,
        path TEXT NOT NULL,
        branch TEXT NOT NULL,
        base TEXT,
        made_branch INTEGER NOT NULL DEFAULT 0,
        thread_id TEXT,
        task_id TEXT,
        kwant_id TEXT,
        created_at INTEGER NOT NULL,
        done_at INTEGER,
        last_turn_at INTEGER,
        keep_build INTEGER NOT NULL DEFAULT 0,
        cleaned_at INTEGER,
        cleaned_for INTEGER,
        cleaned TEXT,
        cleaned_bytes INTEGER,
        skipped TEXT,
        removed_at INTEGER
    )",
    "CREATE INDEX IF NOT EXISTS dev_worktrees_space ON dev_worktrees (space_id, created_at)",
    // One live worktree per thread, and per folder
    "CREATE UNIQUE INDEX IF NOT EXISTS dev_worktrees_thread ON dev_worktrees (thread_id) WHERE removed_at IS NULL AND thread_id IS NOT NULL",
    "CREATE UNIQUE INDEX IF NOT EXISTS dev_worktrees_path ON dev_worktrees (path) WHERE removed_at IS NULL",
];

/// The tables [`Database::delete_dev_mode_of_space`] clears with the rest of Dev mode. The folders
/// stay; git still lists them, so the desk shows them as made outside kwant.
pub const WORKTREE_TABLES: [&str; 1] = ["dev_worktrees"];

#[derive(Debug, Clone, FromRow, PartialEq)]
pub struct WorktreeRow {
    pub id: String,
    pub space_id: String,
    pub team_id: String,
    /// The `<repo>` folder under the space's worktree root (the team folder's name as a slug, with
    /// `-2`, `-3`… when another repo has that name).
    pub repo_dir: String,
    /// The main repo's git dir (canonical), as it was when kwant made the worktree.
    pub common_dir: String,
    /// The worktree's folder (absolute), `<root>/<repo_dir>/<branch slug>`.
    pub path: String,
    pub branch: String,
    pub base: Option<String>,
    pub made_branch: bool,
    pub thread_id: Option<String>,
    pub task_id: Option<String>,
    pub kwant_id: Option<String>,
    pub created_at: i64,
    /// When its work finished by hand (`POST …/done`); a task's own `finished_at` counts too.
    pub done_at: Option<i64>,
    /// When a turn last started in its thread (a later turn means the work isn't done).
    pub last_turn_at: Option<i64>,
    /// The person said "Keep it": P3 never runs on its own.
    pub keep_build: bool,
    pub cleaned_at: Option<i64>,
    /// The finish time P3 last ran for, so one finish is cleaned once.
    pub cleaned_for: Option<i64>,
    /// JSON list of the folders P3 removed (relative to the worktree).
    pub cleaned: Option<String>,
    pub cleaned_bytes: Option<i64>,
    /// JSON list of `{dir, why}` P3 kept.
    pub skipped: Option<String>,
    pub removed_at: Option<i64>,
}

/// A new row's fields.
pub struct NewWorktree<'a> {
    pub space_id: &'a str,
    pub team_id: &'a str,
    pub repo_dir: &'a str,
    pub common_dir: &'a str,
    pub path: &'a str,
    pub branch: &'a str,
    pub base: Option<&'a str>,
    pub made_branch: bool,
    pub thread_id: Option<&'a str>,
    pub task_id: Option<&'a str>,
    pub kwant_id: Option<&'a str>,
}

const COLS: &str = "id, space_id, team_id, repo_dir, common_dir, path, branch, base, made_branch, thread_id, task_id, kwant_id, created_at, \
     done_at, last_turn_at, keep_build, cleaned_at, cleaned_for, cleaned, cleaned_bytes, skipped, removed_at";

fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

impl Database {
    pub(super) async fn migrate_worktrees(&self) -> Result<(), DbError> {
        for sql in TABLES {
            sqlx::query(sql).execute(&self.pool).await?;
        }
        Ok(())
    }

    /// Records a worktree kwant just made. `Ok(None)`: the thread or the folder already has a live
    /// one (two creates at once).
    pub async fn insert_worktree(&self, w: &NewWorktree<'_>) -> Result<Option<WorktreeRow>, DbError> {
        let id = format!("wt_{}", uuid::Uuid::new_v4().simple());
        let done = sqlx::query(
            "INSERT OR IGNORE INTO dev_worktrees (id, space_id, team_id, repo_dir, common_dir, path, branch, base, made_branch, thread_id, task_id, kwant_id, created_at)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(&id)
        .bind(w.space_id)
        .bind(w.team_id)
        .bind(w.repo_dir)
        .bind(w.common_dir)
        .bind(w.path)
        .bind(w.branch)
        .bind(w.base)
        .bind(w.made_branch)
        .bind(w.thread_id)
        .bind(w.task_id)
        .bind(w.kwant_id)
        .bind(now_ms())
        .execute(&self.pool)
        .await?;
        if done.rows_affected() == 0 {
            return Ok(None);
        }
        self.worktree(&id).await
    }

    pub async fn worktree(&self, id: &str) -> Result<Option<WorktreeRow>, DbError> {
        Ok(sqlx::query_as(&format!("SELECT {COLS} FROM dev_worktrees WHERE id = ?")).bind(id).fetch_optional(&self.pool).await?)
    }

    /// A space's worktrees, newest first; removed ones only with `removed`.
    pub async fn worktrees_of_space(&self, space_id: &str, removed: bool) -> Result<Vec<WorktreeRow>, DbError> {
        let only_live = if removed { "" } else { " AND removed_at IS NULL" };
        Ok(sqlx::query_as(&format!("SELECT {COLS} FROM dev_worktrees WHERE space_id = ?{only_live} ORDER BY created_at DESC, id DESC"))
            .bind(space_id)
            .fetch_all(&self.pool)
            .await?)
    }

    /// Every live worktree on this server (the P3 sweep).
    pub async fn live_worktrees(&self) -> Result<Vec<WorktreeRow>, DbError> {
        Ok(sqlx::query_as(&format!("SELECT {COLS} FROM dev_worktrees WHERE removed_at IS NULL ORDER BY created_at")).fetch_all(&self.pool).await?)
    }

    /// The live worktree a thread works in, if any.
    pub async fn worktree_of_thread(&self, thread_id: &str) -> Result<Option<WorktreeRow>, DbError> {
        Ok(sqlx::query_as(&format!("SELECT {COLS} FROM dev_worktrees WHERE thread_id = ? AND removed_at IS NULL"))
            .bind(thread_id)
            .fetch_optional(&self.pool)
            .await?)
    }

    /// A turn starts in the worktree's thread: the work isn't done any more.
    pub async fn worktree_turn_started(&self, id: &str) -> Result<(), DbError> {
        sqlx::query("UPDATE dev_worktrees SET last_turn_at = ?, done_at = NULL WHERE id = ? AND removed_at IS NULL").bind(now_ms()).bind(id).execute(&self.pool).await?;
        Ok(())
    }

    /// `POST …/done`: the first time stays. `false` when there's no live row (removed, or none).
    pub async fn worktree_done(&self, id: &str) -> Result<bool, DbError> {
        let r = sqlx::query("UPDATE dev_worktrees SET done_at = COALESCE(done_at, ?) WHERE id = ? AND removed_at IS NULL").bind(now_ms()).bind(id).execute(&self.pool).await?;
        Ok(r.rows_affected() > 0)
    }

    pub async fn worktree_keep_build(&self, id: &str, keep: bool) -> Result<(), DbError> {
        sqlx::query("UPDATE dev_worktrees SET keep_build = ? WHERE id = ? AND removed_at IS NULL").bind(keep).bind(id).execute(&self.pool).await?;
        Ok(())
    }

    /// P3 ran: what it removed (JSON list), how many bytes, what it kept (JSON), and the finish
    /// time it ran for (`None` for a "Clean now").
    pub async fn worktree_cleaned(&self, id: &str, cleaned: &str, bytes: i64, skipped: &str, done_for: Option<i64>) -> Result<(), DbError> {
        sqlx::query("UPDATE dev_worktrees SET cleaned_at = ?, cleaned = ?, cleaned_bytes = ?, skipped = ?, cleaned_for = COALESCE(?, cleaned_for) WHERE id = ?")
            .bind(now_ms())
            .bind(cleaned)
            .bind(bytes)
            .bind(skipped)
            .bind(done_for)
            .bind(id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    pub async fn worktree_removed(&self, id: &str) -> Result<(), DbError> {
        sqlx::query("UPDATE dev_worktrees SET removed_at = ? WHERE id = ? AND removed_at IS NULL").bind(now_ms()).bind(id).execute(&self.pool).await?;
        Ok(())
    }

    /// The `finished_at` (ms) of a task, `None` while it isn't finished (or there's no such task).
    pub async fn task_finished_ms(&self, task_id: &str) -> Result<Option<i64>, DbError> {
        let row: Option<(Option<chrono::DateTime<chrono::Utc>>,)> =
            sqlx::query_as("SELECT finished_at FROM tasks WHERE id = ?").bind(task_id).fetch_optional(&self.pool).await?;
        Ok(row.and_then(|(f,)| f).map(|t| t.timestamp_millis()))
    }

    /// The title of a task, for the desk's row.
    pub async fn task_title(&self, task_id: &str) -> Result<Option<String>, DbError> {
        let row: Option<(String,)> = sqlx::query_as("SELECT title FROM tasks WHERE id = ?").bind(task_id).fetch_optional(&self.pool).await?;
        Ok(row.map(|(t,)| t))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn new<'a>(path: &'a str, thread: Option<&'a str>) -> NewWorktree<'a> {
        NewWorktree {
            space_id: "space_demo",
            team_id: "team_web",
            repo_dir: "web",
            common_dir: "/r/web/.git",
            path,
            branch: "lin/x",
            base: Some("main"),
            made_branch: true,
            thread_id: thread,
            task_id: None,
            kwant_id: None,
        }
    }

    /// One live worktree per thread and per folder; a removed one frees both. A turn clears the
    /// finish, "done" keeps its first time, and P3's result is kept with the finish it ran for.
    #[tokio::test]
    async fn worktree_rows_keep_one_live_per_thread_and_track_the_finish() {
        let tmp = tempfile::tempdir().unwrap();
        let db = Database::init(tmp.path()).await.unwrap();
        let a = db.insert_worktree(&new("/wt/web/a", Some("thr_1"))).await.unwrap().unwrap();
        assert!(db.insert_worktree(&new("/wt/web/b", Some("thr_1"))).await.unwrap().is_none(), "a second for the thread");
        assert!(db.insert_worktree(&new("/wt/web/a", Some("thr_2"))).await.unwrap().is_none(), "a second at the folder");
        assert_eq!(db.worktree_of_thread("thr_1").await.unwrap().map(|w| w.id), Some(a.id.clone()));

        assert!(db.worktree_done(&a.id).await.unwrap());
        let first = db.worktree(&a.id).await.unwrap().unwrap().done_at.unwrap();
        db.worktree_done(&a.id).await.unwrap();
        assert_eq!(db.worktree(&a.id).await.unwrap().unwrap().done_at, Some(first), "the first time stays");
        db.worktree_turn_started(&a.id).await.unwrap();
        let w = db.worktree(&a.id).await.unwrap().unwrap();
        assert!(w.done_at.is_none() && w.last_turn_at.is_some(), "a turn means it isn't done");

        db.worktree_cleaned(&a.id, r#"["node_modules"]"#, 15, "[]", Some(42)).await.unwrap();
        db.worktree_cleaned(&a.id, "[]", 0, "[]", None).await.unwrap();
        let w = db.worktree(&a.id).await.unwrap().unwrap();
        assert_eq!((w.cleaned_for, w.cleaned_bytes), (Some(42), Some(0)), "Clean now keeps the finish P3 ran for");
        db.worktree_keep_build(&a.id, true).await.unwrap();
        assert!(db.worktree(&a.id).await.unwrap().unwrap().keep_build);

        db.worktree_removed(&a.id).await.unwrap();
        let before = db.worktree(&a.id).await.unwrap().unwrap().done_at;
        assert!(!db.worktree_done(&a.id).await.unwrap(), "done on a removed row: no live row");
        assert_eq!(db.worktree(&a.id).await.unwrap().unwrap().done_at, before, "and nothing stamped");
        assert!(!db.worktree_done("wt_nope").await.unwrap());
        assert!(db.worktree_of_thread("thr_1").await.unwrap().is_none());
        assert!(db.live_worktrees().await.unwrap().is_empty());
        assert_eq!(db.worktrees_of_space("space_demo", true).await.unwrap().len(), 1);
        assert!(db.insert_worktree(&new("/wt/web/a", Some("thr_1"))).await.unwrap().is_some(), "freed by the removal");
    }
}
