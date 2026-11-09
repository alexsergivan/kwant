//! B2 (0.8.0, fresh-eyes-audit.md §4): the pre-push card. Before a bot pushes its own branch in
//! Dev mode, kwant checks the commit it would push and keeps one card per
//! `(space, folder, branch, head_sha)`. The push is refused until the card is `clear`, or the
//! person `overridden` it (for one push: then it is `pushed`). Kept outside exports like the other Dev mode tables. Add-only.

use super::{Database, DbError};
use sqlx::FromRow;

const TABLES: &[&str] = &[
    "CREATE TABLE IF NOT EXISTS dev_prepush_cards (
        id TEXT PRIMARY KEY,
        space_id TEXT NOT NULL,
        kwant_id TEXT NOT NULL,
        folder TEXT NOT NULL,
        branch TEXT NOT NULL,
        head_sha TEXT NOT NULL,
        status TEXT NOT NULL,
        card TEXT,
        overridden_by TEXT,
        overridden_at INTEGER,
        override_reason TEXT,
        tries INTEGER NOT NULL DEFAULT 1,
        created_at INTEGER NOT NULL,
        updated_at INTEGER NOT NULL,
        UNIQUE (space_id, folder, branch, head_sha)
    )",
    "CREATE INDEX IF NOT EXISTS dev_prepush_cards_space ON dev_prepush_cards (space_id, created_at)",
];

/// The tables [`Database::delete_dev_mode_of_space`] clears with the rest of Dev mode.
pub const PREPUSH_TABLES: [&str; 1] = ["dev_prepush_cards"];

/// kwant is still checking: the push waits.
pub const CHECKING: &str = "checking";
/// A check found something that must be fixed first (or the person overrides).
pub const BLOCKED: &str = "blocked";
/// Every check passed: the push goes.
pub const CLEAR: &str = "clear";
/// The person let the push go anyway: one push, used up by the first push it lets go.
pub const OVERRIDDEN: &str = "overridden";
/// An `overridden` card whose one push went. Pushing the same commit again makes it `blocked`
/// again, so the person decides each push (Rigel's QA of #456, finding C).
pub const PUSHED: &str = "pushed";
/// kwant couldn't check (no base, git failed): the push waits for the person, as blocked does.
pub const PREPUSH_FAILED: &str = "failed";
/// The person asked for the checks to run again: the next push of this commit makes a new card,
/// back at try 1.
pub const RETRIED: &str = "retried";
/// From this many blocked or failed tries in a row, the card needs the person (Mira's P4).
pub const NEEDS_YOU_TRIES: i64 = 2;

#[derive(Debug, Clone, FromRow, PartialEq)]
pub struct PrepushRow {
    pub id: String,
    pub space_id: String,
    pub kwant_id: String,
    pub folder: String,
    pub branch: String,
    pub head_sha: String,
    pub status: String,
    /// The card JSON (fresh-eyes-audit.md §4.2), none while checking.
    pub card: Option<String>,
    pub overridden_by: Option<String>,
    pub overridden_at: Option<i64>,
    /// Why the person let it go (at least 10 characters).
    pub override_reason: Option<String>,
    /// 1 for the first card of a run of blocked or failed pushes of this branch, then one more for
    /// each new commit that is blocked or failed again (see [`Database::insert_prepush_card`]).
    pub tries: i64,
    pub created_at: i64,
    pub updated_at: i64,
}

fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

const COLS: &str =
    "id, space_id, kwant_id, folder, branch, head_sha, status, card, overridden_by, overridden_at, override_reason, tries, created_at, updated_at";

impl Database {
    pub(super) async fn migrate_prepush(&self) -> Result<(), DbError> {
        for sql in TABLES {
            sqlx::query(sql).execute(&self.pool).await?;
        }
        Ok(())
    }

    /// The card of this push, if kwant has one.
    pub async fn prepush_card_of(&self, space_id: &str, folder: &str, branch: &str, head_sha: &str) -> Result<Option<PrepushRow>, DbError> {
        Ok(sqlx::query_as(&format!("SELECT {COLS} FROM dev_prepush_cards WHERE space_id = ? AND folder = ? AND branch = ? AND head_sha = ?"))
            .bind(space_id)
            .bind(folder)
            .bind(branch)
            .bind(head_sha)
            .fetch_optional(&self.pool)
            .await?)
    }

    /// A new card for this push in `status` (`checking`, or `failed` with its card), or `None`
    /// when one is already there (two pushes at once make one card). Its `tries` is 1, or one more
    /// than the branch's newest card when that one is blocked or failed (the bot is still fixing);
    /// `fresh_start` (after the person asked for a re-check) makes it 1 again, and so does a card
    /// the person overrode that is blocked again only because its one push was used.
    #[allow(clippy::too_many_arguments)]
    pub async fn insert_prepush_card(
        &self,
        space_id: &str,
        kwant_id: &str,
        folder: &str,
        branch: &str,
        head_sha: &str,
        status: &str,
        card: Option<&str>,
        fresh_start: bool,
    ) -> Result<Option<PrepushRow>, DbError> {
        let id = format!("pp_{}", uuid::Uuid::new_v4().simple());
        let now = now_ms();
        let tries = match self.prepush_newest_on(space_id, folder, branch).await? {
            Some(last) if !fresh_start && last.overridden_by.is_none() && (last.status == BLOCKED || last.status == PREPUSH_FAILED) => last.tries + 1,
            _ => 1,
        };
        let done = sqlx::query(
            "INSERT OR IGNORE INTO dev_prepush_cards (id, space_id, kwant_id, folder, branch, head_sha, status, card, tries, created_at, updated_at)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(&id)
        .bind(space_id)
        .bind(kwant_id)
        .bind(folder)
        .bind(branch)
        .bind(head_sha)
        .bind(status)
        .bind(card)
        .bind(tries)
        .bind(now)
        .bind(now)
        .execute(&self.pool)
        .await?;
        if done.rows_affected() == 0 {
            return Ok(None);
        }
        self.prepush_card(&id).await
    }

    pub async fn prepush_card(&self, id: &str) -> Result<Option<PrepushRow>, DbError> {
        Ok(sqlx::query_as(&format!("SELECT {COLS} FROM dev_prepush_cards WHERE id = ?")).bind(id).fetch_optional(&self.pool).await?)
    }

    /// The newest card of a branch pushed from `folder`, whatever its commit.
    pub async fn prepush_newest_on(&self, space_id: &str, folder: &str, branch: &str) -> Result<Option<PrepushRow>, DbError> {
        Ok(sqlx::query_as(&format!(
            "SELECT {COLS} FROM dev_prepush_cards WHERE space_id = ? AND folder = ? AND branch = ? ORDER BY created_at DESC, rowid DESC LIMIT 1"
        ))
        .bind(space_id)
        .bind(folder)
        .bind(branch)
        .fetch_optional(&self.pool)
        .await?)
    }

    /// The cards on `branch` in a space the person overrode (standing, used or blocked again),
    /// newest first, at most `limit`: what a PR from that branch says was pushed anyway.
    pub async fn prepush_overrides_on(&self, space_id: &str, branch: &str, limit: i64) -> Result<Vec<PrepushRow>, DbError> {
        Ok(sqlx::query_as(&format!(
            "SELECT {COLS} FROM dev_prepush_cards WHERE space_id = ? AND branch = ? AND overridden_by IS NOT NULL ORDER BY overridden_at DESC, rowid DESC LIMIT ?"
        ))
        .bind(space_id)
        .bind(branch)
        .bind(limit)
        .fetch_all(&self.pool)
        .await?)
    }

    /// The newest cards of a space, at most `limit`.
    pub async fn prepush_cards(&self, space_id: &str, limit: i64) -> Result<Vec<PrepushRow>, DbError> {
        Ok(sqlx::query_as(&format!("SELECT {COLS} FROM dev_prepush_cards WHERE space_id = ? ORDER BY created_at DESC, id DESC LIMIT ?"))
            .bind(space_id)
            .bind(limit)
            .fetch_all(&self.pool)
            .await?)
    }

    /// Stores the checks' verdict on a card that is still `checking`: the card, or `None` when
    /// it was no longer checking (dropped as stuck) and nothing changed.
    pub async fn finish_prepush_card(&self, id: &str, status: &str, card: &str) -> Result<Option<PrepushRow>, DbError> {
        let r = sqlx::query("UPDATE dev_prepush_cards SET status = ?, card = ?, updated_at = ? WHERE id = ? AND status = 'checking'")
            .bind(status)
            .bind(card)
            .bind(now_ms())
            .bind(id)
            .execute(&self.pool)
            .await?;
        if r.rows_affected() == 0 {
            return Ok(None);
        }
        self.prepush_card(id).await
    }

    /// The person lets a `blocked` or `failed` push go, saying why: the card, or `None` when it
    /// isn't there or wasn't blocked or failed (nothing changed).
    pub async fn override_prepush_card(&self, id: &str, by: &str, reason: &str) -> Result<Option<PrepushRow>, DbError> {
        let now = now_ms();
        let r = sqlx::query(
            "UPDATE dev_prepush_cards SET status = 'overridden', overridden_by = ?, override_reason = ?, overridden_at = ?, updated_at = ?
             WHERE id = ? AND status IN ('blocked', 'failed')",
        )
        .bind(by)
        .bind(reason)
        .bind(now)
        .bind(now)
        .bind(id)
        .execute(&self.pool)
        .await?;
        if r.rows_affected() == 0 {
            return Ok(None);
        }
        self.prepush_card(id).await
    }

    /// Uses up the override of card `id` for the push it is letting go: `overridden` becomes
    /// `pushed`, at most once. False when it wasn't `overridden` (another push used it first).
    pub async fn use_prepush_override(&self, id: &str) -> Result<bool, DbError> {
        let r = sqlx::query("UPDATE dev_prepush_cards SET status = 'pushed', updated_at = ? WHERE id = ? AND status = 'overridden'")
            .bind(now_ms())
            .bind(id)
            .execute(&self.pool)
            .await?;
        Ok(r.rows_affected() > 0)
    }

    /// A bot pushes the commit of a `pushed` card again: the card is `blocked` again (its
    /// override history stays). The card, or `None` when it wasn't `pushed`.
    pub async fn reblock_pushed_prepush_card(&self, id: &str) -> Result<Option<PrepushRow>, DbError> {
        let r = sqlx::query("UPDATE dev_prepush_cards SET status = 'blocked', updated_at = ? WHERE id = ? AND status = 'pushed'")
            .bind(now_ms())
            .bind(id)
            .execute(&self.pool)
            .await?;
        if r.rows_affected() == 0 {
            return Ok(None);
        }
        self.prepush_card(id).await
    }

    /// The person asks for the checks of a `blocked` or `failed` card to run again: it becomes
    /// `retried`, and the next push of that commit makes a new card at try 1. `None` when it isn't
    /// there or wasn't blocked or failed.
    pub async fn retry_prepush_card(&self, id: &str) -> Result<Option<PrepushRow>, DbError> {
        let r = sqlx::query("UPDATE dev_prepush_cards SET status = 'retried', updated_at = ? WHERE id = ? AND status IN ('blocked', 'failed')")
            .bind(now_ms())
            .bind(id)
            .execute(&self.pool)
            .await?;
        if r.rows_affected() == 0 {
            return Ok(None);
        }
        self.prepush_card(id).await
    }

    /// Removes a `retried` card so its commit can get a new one. True when it was removed.
    pub async fn drop_retried_prepush_card(&self, id: &str) -> Result<bool, DbError> {
        let r = sqlx::query("DELETE FROM dev_prepush_cards WHERE id = ? AND status = 'retried'").bind(id).execute(&self.pool).await?;
        Ok(r.rows_affected() > 0)
    }

    /// Drops card `id` when it is still `checking` after `older_than_ms` (the server restarted
    /// mid-check, or the check hung), so the next push checks again. True when it was dropped.
    pub async fn drop_stuck_prepush_card(&self, id: &str, older_than_ms: i64) -> Result<bool, DbError> {
        let cutoff = now_ms() - older_than_ms;
        let r = sqlx::query("DELETE FROM dev_prepush_cards WHERE id = ? AND status = 'checking' AND updated_at < ?").bind(id).bind(cutoff).execute(&self.pool).await?;
        Ok(r.rows_affected() > 0)
    }
}
