//! CI and review comments back to the author bot (0.9.0 wave 1, Juno; `server::dev::fix_it`).
//! `dev_fix_seen` holds every failed check (`ci:<head>:<check>`), review comment (`comment:<id>`)
//! and green head (`verdict:<head>`) kwant has already sent for a PR, so a later poll, a restart
//! or two polls at once never send it twice. `dev_fix_items` is one row per item sent. Kept
//! outside exports like the other Dev mode tables. Add-only.

use super::{Database, DbError};
use sqlx::FromRow;

const TABLES: &[&str] = &[
    "CREATE TABLE IF NOT EXISTS dev_fix_items (
        id TEXT PRIMARY KEY,
        space_id TEXT NOT NULL,
        repo_key TEXT NOT NULL,
        pr INTEGER NOT NULL,
        head_sha TEXT NOT NULL,
        kwant_id TEXT NOT NULL,
        thread_id TEXT,
        kind TEXT NOT NULL,
        item TEXT NOT NULL,
        turn TEXT NOT NULL,
        created_at INTEGER NOT NULL
    )",
    "CREATE INDEX IF NOT EXISTS dev_fix_items_pr ON dev_fix_items (space_id, repo_key, pr, created_at)",
    "CREATE TABLE IF NOT EXISTS dev_fix_seen (
        space_id TEXT NOT NULL,
        repo_key TEXT NOT NULL,
        pr INTEGER NOT NULL,
        key TEXT NOT NULL,
        item_id TEXT NOT NULL,
        created_at INTEGER NOT NULL,
        PRIMARY KEY (space_id, repo_key, pr, key)
    )",
];

/// The tables [`Database::delete_dev_mode_of_space`] clears with the rest of Dev mode.
pub const FIX_TABLES: [&str; 2] = ["dev_fix_items", "dev_fix_seen"];

/// A Fix-it item: failed checks and review comments, sent to the author bot.
pub const FIX_IT: &str = "fix_it";
/// The fresh-eyes reviewer's advisory MERGE / HOLD / DO NOT MERGE on a green head.
pub const VERDICT: &str = "verdict";

/// The item started a turn of the author bot.
pub const TURN_STARTED: &str = "started";
/// Today's Fix-it turns for this PR were used up: the item waits in the thread and in Needs you.
pub const TURN_HELD: &str = "held";
/// Today's spending stop held the item's turn: it waits in the thread and in Needs you, and the
/// first poll after the stop lifts (Continue, or the day's end) starts it.
pub const TURN_SPEND_HELD: &str = "spend_held";
/// The person archived the PR's Fix-it chat: nothing is sent to that bot for this PR any more.
pub const TURN_STOPPED: &str = "stopped";
/// No turn for this item (a verdict is for the person).
pub const TURN_NONE: &str = "none";

#[derive(Debug, Clone, FromRow, PartialEq)]
pub struct FixItemRow {
    pub id: String,
    pub space_id: String,
    pub repo_key: String,
    pub pr: i64,
    pub head_sha: String,
    pub kwant_id: String,
    /// The author bot's Fix-it chat for this PR; none when nothing was posted (`stopped`).
    pub thread_id: Option<String>,
    /// [`FIX_IT`] or [`VERDICT`].
    pub kind: String,
    /// The item as JSON (what the chat line's `meta.item` carries).
    pub item: String,
    /// [`TURN_STARTED`], [`TURN_HELD`], [`TURN_SPEND_HELD`], [`TURN_STOPPED`] or [`TURN_NONE`].
    pub turn: String,
    pub created_at: i64,
}

const COLS: &str = "id, space_id, repo_key, pr, head_sha, kwant_id, thread_id, kind, item, turn, created_at";

impl Database {
    pub(super) async fn migrate_fix_it(&self) -> Result<(), DbError> {
        for sql in TABLES {
            sqlx::query(sql).execute(&self.pool).await?;
        }
        Ok(())
    }

    /// The keys already sent for a PR (see the module doc).
    pub async fn fix_seen_keys(&self, space_id: &str, repo_key: &str, pr: i64) -> Result<Vec<String>, DbError> {
        let rows: Vec<(String,)> = sqlx::query_as("SELECT key FROM dev_fix_seen WHERE space_id = ? AND repo_key = ? AND pr = ?")
            .bind(space_id)
            .bind(repo_key)
            .bind(pr)
            .fetch_all(&self.pool)
            .await?;
        Ok(rows.into_iter().map(|(k,)| k).collect())
    }

    /// Claims `keys` for item `item_id`, in one transaction: the keys this call claimed, in order.
    /// A key another item claimed first is left out, so two polls at once send each key once.
    pub async fn claim_fix_keys(&self, space_id: &str, repo_key: &str, pr: i64, keys: &[String], item_id: &str) -> Result<Vec<String>, DbError> {
        let now = chrono::Utc::now().timestamp_millis();
        let mut tx = self.pool.begin().await?;
        let mut won = Vec::new();
        for k in keys {
            let r = sqlx::query("INSERT OR IGNORE INTO dev_fix_seen (space_id, repo_key, pr, key, item_id, created_at) VALUES (?, ?, ?, ?, ?, ?)")
                .bind(space_id)
                .bind(repo_key)
                .bind(pr)
                .bind(k)
                .bind(item_id)
                .bind(now)
                .execute(&mut *tx)
                .await?;
            if r.rows_affected() == 1 {
                won.push(k.clone());
            }
        }
        tx.commit().await?;
        Ok(won)
    }

    /// Gives back `keys` claimed by item `item_id` (a verdict the spending stop held), so a later
    /// poll claims them again. A key another item holds is left alone.
    pub async fn release_fix_keys(&self, space_id: &str, repo_key: &str, pr: i64, keys: &[String], item_id: &str) -> Result<(), DbError> {
        for k in keys {
            sqlx::query("DELETE FROM dev_fix_seen WHERE space_id = ? AND repo_key = ? AND pr = ? AND key = ? AND item_id = ?")
                .bind(space_id)
                .bind(repo_key)
                .bind(pr)
                .bind(k)
                .bind(item_id)
                .execute(&self.pool)
                .await?;
        }
        Ok(())
    }

    /// Moves item `id` from turn `from` to `to`; false when it wasn't at `from` (another poll
    /// moved it first), so only one caller starts its turn.
    pub async fn set_fix_item_turn(&self, id: &str, from: &str, to: &str) -> Result<bool, DbError> {
        let r = sqlx::query("UPDATE dev_fix_items SET turn = ? WHERE id = ? AND turn = ?").bind(to).bind(id).bind(from).execute(&self.pool).await?;
        Ok(r.rows_affected() == 1)
    }

    pub async fn insert_fix_item(&self, row: &FixItemRow) -> Result<(), DbError> {
        sqlx::query(&format!("INSERT INTO dev_fix_items ({COLS}) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)"))
            .bind(&row.id)
            .bind(&row.space_id)
            .bind(&row.repo_key)
            .bind(row.pr)
            .bind(&row.head_sha)
            .bind(&row.kwant_id)
            .bind(&row.thread_id)
            .bind(&row.kind)
            .bind(&row.item)
            .bind(&row.turn)
            .bind(row.created_at)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    /// A PR's items, oldest first.
    pub async fn fix_items_of_pr(&self, space_id: &str, repo_key: &str, pr: i64) -> Result<Vec<FixItemRow>, DbError> {
        Ok(sqlx::query_as(&format!("SELECT {COLS} FROM dev_fix_items WHERE space_id = ? AND repo_key = ? AND pr = ? ORDER BY created_at, rowid"))
            .bind(space_id)
            .bind(repo_key)
            .bind(pr)
            .fetch_all(&self.pool)
            .await?)
    }

    /// The items of a space made since `since_ms` with one of `turns`, newest first, at most `limit`.
    pub async fn fix_items_with_turn(&self, space_id: Option<&str>, turns: &[&str], since_ms: i64, limit: i64) -> Result<Vec<FixItemRow>, DbError> {
        let marks = vec!["?"; turns.len().max(1)].join(", ");
        let sql = format!("SELECT {COLS} FROM dev_fix_items WHERE (? IS NULL OR space_id = ?) AND turn IN ({marks}) AND created_at >= ? ORDER BY created_at DESC, rowid DESC LIMIT ?");
        let mut q = sqlx::query_as::<_, FixItemRow>(&sql).bind(space_id).bind(space_id);
        if turns.is_empty() {
            q = q.bind("");
        }
        for t in turns {
            q = q.bind(*t);
        }
        Ok(q.bind(since_ms).bind(limit).fetch_all(&self.pool).await?)
    }
}
