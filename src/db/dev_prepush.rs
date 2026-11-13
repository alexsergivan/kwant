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
    // 0.9.7: the person's one-time approval of a bot's push, from Retry or Override on a card
    "CREATE TABLE IF NOT EXISTS dev_prepush_grants (
        id TEXT PRIMARY KEY,
        space_id TEXT NOT NULL,
        kwant_id TEXT NOT NULL,
        folder TEXT NOT NULL,
        branch TEXT NOT NULL,
        kind TEXT NOT NULL,
        card_id TEXT NOT NULL,
        granted_at INTEGER NOT NULL,
        expires_at INTEGER NOT NULL,
        used_at INTEGER,
        next_card_id TEXT
    )",
    "CREATE INDEX IF NOT EXISTS dev_prepush_grants_branch ON dev_prepush_grants (space_id, folder, branch, granted_at)",
];

/// Columns added after the first release, as `(name, type)`: added when missing (add-only).
const ADDED_COLUMNS: [(&str, &str); 2] = [
    // 0.9.7: the newer card of the branch that replaced a `superseded` one
    ("superseded_by", "TEXT"),
    // 0.9.7: what a `superseded` card was before (`blocked` or `failed`), for a push of its commit again
    ("superseded_status", "TEXT"),
];

/// The tables [`Database::delete_dev_mode_of_space`] clears with the rest of Dev mode.
pub const PREPUSH_TABLES: [&str; 2] = ["dev_prepush_cards", "dev_prepush_grants"];

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
/// A `blocked` or `failed` card of a branch that a newer card (another commit) replaced: it no
/// longer waits for the person (0.9.7). Pushing its commit again makes it what it was.
pub const SUPERSEDED: &str = "superseded";
/// How long a grant from Retry or Override waits for the push it covers (0.9.7).
pub const GRANT_MS: i64 = 2 * 60 * 60 * 1000;
/// A grant from Retry: the bot's next push of the branch whose card comes back `clear`.
pub const GRANT_RETRY: &str = "retry";
/// A grant from Override: the one push of the overridden commit.
pub const GRANT_OVERRIDE: &str = "override";
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
    /// The newer card that replaced it, when `superseded`.
    pub superseded_by: Option<String>,
    /// What it was before it was `superseded` (`blocked` or `failed`).
    pub superseded_status: Option<String>,
}

/// The person's one-time approval of a bot's push (0.9.7): Retry covers the next push of the
/// branch whose card is `clear`, Override the one push of its commit. Used at most once, and only
/// until `expires_at`.
#[derive(Debug, Clone, FromRow, PartialEq)]
pub struct PrepushGrantRow {
    pub id: String,
    pub space_id: String,
    pub kwant_id: String,
    pub folder: String,
    pub branch: String,
    /// [`GRANT_RETRY`] or [`GRANT_OVERRIDE`].
    pub kind: String,
    /// The card the person pressed.
    pub card_id: String,
    pub granted_at: i64,
    pub expires_at: i64,
    pub used_at: Option<i64>,
    /// The newest card a push of the branch made while the grant waited, then the card of the
    /// push it let go.
    pub next_card_id: Option<String>,
}

fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

const COLS: &str = "id, space_id, kwant_id, folder, branch, head_sha, status, card, overridden_by, overridden_at, override_reason, tries, created_at, \
     updated_at, superseded_by, superseded_status";

const GRANT_COLS: &str = "id, space_id, kwant_id, folder, branch, kind, card_id, granted_at, expires_at, used_at, next_card_id";

impl Database {
    pub(super) async fn migrate_prepush(&self) -> Result<(), DbError> {
        for sql in TABLES {
            sqlx::query(sql).execute(&self.pool).await?;
        }
        for (name, kind) in ADDED_COLUMNS {
            let has: Option<(String,)> =
                sqlx::query_as("SELECT name FROM pragma_table_info('dev_prepush_cards') WHERE name = ?").bind(name).fetch_optional(&self.pool).await?;
            if has.is_none() {
                sqlx::query(&format!("ALTER TABLE dev_prepush_cards ADD COLUMN {name} {kind}")).execute(&self.pool).await?;
            }
        }
        // Cards from before 0.9.7: only the newest waiting card of each branch can need the person
        self.collapse_prepush_cards().await?;
        Ok(())
    }

    /// Supersedes every `blocked` or `failed` card that isn't the newest waiting card of its
    /// `(space, folder, branch)`, pointing it at that one (0.9.7). Runs on every start, so cards
    /// saved before superseding existed collapse too; doing it again changes nothing. The number
    /// of cards it superseded.
    pub async fn collapse_prepush_cards(&self) -> Result<u64, DbError> {
        let r = sqlx::query(
            "WITH ranked AS (
                 SELECT id, FIRST_VALUE(id) OVER w AS newest, ROW_NUMBER() OVER w AS n
                 FROM dev_prepush_cards WHERE status IN ('blocked', 'failed')
                 WINDOW w AS (PARTITION BY space_id, folder, branch ORDER BY created_at DESC, rowid DESC)
             )
             UPDATE dev_prepush_cards
             SET superseded_status = status, status = 'superseded',
                 superseded_by = (SELECT newest FROM ranked WHERE ranked.id = dev_prepush_cards.id), updated_at = ?
             WHERE id IN (SELECT id FROM ranked WHERE n > 1)",
        )
        .bind(now_ms())
        .execute(&self.pool)
        .await?;
        Ok(r.rows_affected())
    }

    /// Card `newer` was made (or made waiting again) for its branch: every other `blocked` or
    /// `failed` card of the same `(space, folder, branch)` with another commit becomes
    /// `superseded` by it (0.9.7). The cards it changed, to notify.
    pub async fn supersede_prepush_cards_beside(&self, newer: &PrepushRow) -> Result<Vec<PrepushRow>, DbError> {
        let ids: Vec<(String,)> = sqlx::query_as(
            "SELECT id FROM dev_prepush_cards WHERE space_id = ? AND folder = ? AND branch = ? AND head_sha != ? AND id != ? AND status IN ('blocked', 'failed')",
        )
        .bind(&newer.space_id)
        .bind(&newer.folder)
        .bind(&newer.branch)
        .bind(&newer.head_sha)
        .bind(&newer.id)
        .fetch_all(&self.pool)
        .await?;
        let mut out = Vec::new();
        for (id,) in ids {
            let r = sqlx::query(
                "UPDATE dev_prepush_cards SET superseded_status = status, status = 'superseded', superseded_by = ?, updated_at = ?
                 WHERE id = ? AND status IN ('blocked', 'failed')",
            )
            .bind(&newer.id)
            .bind(now_ms())
            .bind(&id)
            .execute(&self.pool)
            .await?;
            if r.rows_affected() > 0 {
                if let Some(row) = self.prepush_card(&id).await? {
                    out.push(row);
                }
            }
        }
        Ok(out)
    }

    /// A bot pushes the commit of a `superseded` card again: it is what it was before (`blocked`
    /// or `failed`) and waits for the person again. The card, or `None` when it wasn't superseded.
    pub async fn revive_superseded_prepush_card(&self, id: &str) -> Result<Option<PrepushRow>, DbError> {
        let r = sqlx::query(
            "UPDATE dev_prepush_cards SET status = COALESCE(superseded_status, 'blocked'), superseded_by = NULL, superseded_status = NULL, updated_at = ?
             WHERE id = ? AND status = 'superseded'",
        )
        .bind(now_ms())
        .bind(id)
        .execute(&self.pool)
        .await?;
        if r.rows_affected() == 0 {
            return Ok(None);
        }
        self.prepush_card(id).await
    }

    /// Records the person's grant on `card` (Retry or Override, see [`PrepushGrantRow`]), for
    /// the card's bot, folder and branch, waiting [`GRANT_MS`]. Any grant of theirs on that branch
    /// still waiting ends now, so only one push is ever covered.
    pub async fn add_prepush_grant(&self, card: &PrepushRow, kind: &str) -> Result<PrepushGrantRow, DbError> {
        let now = now_ms();
        let id = format!("pg_{}", uuid::Uuid::new_v4().simple());
        let mut tx = self.pool.begin().await?;
        sqlx::query(
            "UPDATE dev_prepush_grants SET expires_at = ? WHERE space_id = ? AND kwant_id = ? AND folder = ? AND branch = ? AND used_at IS NULL AND expires_at > ?",
        )
        .bind(now)
        .bind(&card.space_id)
        .bind(&card.kwant_id)
        .bind(&card.folder)
        .bind(&card.branch)
        .bind(now)
        .execute(&mut *tx)
        .await?;
        sqlx::query(
            "INSERT INTO dev_prepush_grants (id, space_id, kwant_id, folder, branch, kind, card_id, granted_at, expires_at) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(&id)
        .bind(&card.space_id)
        .bind(&card.kwant_id)
        .bind(&card.folder)
        .bind(&card.branch)
        .bind(kind)
        .bind(&card.id)
        .bind(now)
        .bind(now + GRANT_MS)
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        self.prepush_grant(&id).await?.ok_or_else(|| DbError::Sqlx(sqlx::Error::RowNotFound))
    }

    pub async fn prepush_grant(&self, id: &str) -> Result<Option<PrepushGrantRow>, DbError> {
        Ok(sqlx::query_as(&format!("SELECT {GRANT_COLS} FROM dev_prepush_grants WHERE id = ?")).bind(id).fetch_optional(&self.pool).await?)
    }

    /// The grant of `kind` still waiting (unused, not expired) on a bot's branch, newest first.
    pub async fn live_prepush_grant(&self, space_id: &str, kwant_id: &str, folder: &str, branch: &str, kind: &str) -> Result<Option<PrepushGrantRow>, DbError> {
        Ok(sqlx::query_as(&format!(
            "SELECT {GRANT_COLS} FROM dev_prepush_grants
             WHERE space_id = ? AND kwant_id = ? AND folder = ? AND branch = ? AND kind = ? AND used_at IS NULL AND expires_at > ?
             ORDER BY granted_at DESC, rowid DESC LIMIT 1"
        ))
        .bind(space_id)
        .bind(kwant_id)
        .bind(folder)
        .bind(branch)
        .bind(kind)
        .bind(now_ms())
        .fetch_optional(&self.pool)
        .await?)
    }

    /// Points a waiting grant at `card_id`, the newest card a push of its branch made.
    pub async fn point_prepush_grant(&self, id: &str, card_id: &str) -> Result<(), DbError> {
        sqlx::query("UPDATE dev_prepush_grants SET next_card_id = ? WHERE id = ? AND used_at IS NULL").bind(card_id).bind(id).execute(&self.pool).await?;
        Ok(())
    }

    /// Uses grant `id` for the push of card `card_id`: at most once, and only before it expires.
    /// False when another push used it first or it expired (the push then asks as usual).
    pub async fn use_prepush_grant(&self, id: &str, card_id: &str) -> Result<bool, DbError> {
        let now = now_ms();
        let r = sqlx::query("UPDATE dev_prepush_grants SET used_at = ?, next_card_id = ? WHERE id = ? AND used_at IS NULL AND expires_at > ?")
            .bind(now)
            .bind(card_id)
            .bind(id)
            .bind(now)
            .execute(&self.pool)
            .await?;
        Ok(r.rows_affected() > 0)
    }

    /// The newest grant of a card: the one the person pressed on it, or the one that followed the
    /// bot's push to it.
    pub async fn prepush_grant_of_card(&self, card_id: &str) -> Result<Option<PrepushGrantRow>, DbError> {
        Ok(sqlx::query_as(&format!(
            "SELECT {GRANT_COLS} FROM dev_prepush_grants WHERE card_id = ? OR next_card_id = ? ORDER BY granted_at DESC, rowid DESC LIMIT 1"
        ))
        .bind(card_id)
        .bind(card_id)
        .fetch_optional(&self.pool)
        .await?)
    }

    /// A space's grants, newest first, at most `limit`.
    pub async fn prepush_grants(&self, space_id: &str, limit: i64) -> Result<Vec<PrepushGrantRow>, DbError> {
        Ok(sqlx::query_as(&format!("SELECT {GRANT_COLS} FROM dev_prepush_grants WHERE space_id = ? ORDER BY granted_at DESC, rowid DESC LIMIT ?"))
            .bind(space_id)
            .bind(limit)
            .fetch_all(&self.pool)
            .await?)
    }

    /// Moves grant `id`'s clock back by `ms`, for tests of a grant that expired.
    #[doc(hidden)]
    pub async fn age_prepush_grant(&self, id: &str, ms: i64) -> Result<(), DbError> {
        sqlx::query("UPDATE dev_prepush_grants SET granted_at = granted_at - ?, expires_at = expires_at - ? WHERE id = ?")
            .bind(ms)
            .bind(ms)
            .bind(id)
            .execute(&self.pool)
            .await?;
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

    /// The bot that pushed `branch` in a space last (its newest card, from any folder): who a PR
    /// from that branch goes back to (0.9.0, `dev::fix_it`).
    pub async fn prepush_pusher_of(&self, space_id: &str, branch: &str) -> Result<Option<String>, DbError> {
        let row: Option<(String,)> =
            sqlx::query_as("SELECT kwant_id FROM dev_prepush_cards WHERE space_id = ? AND branch = ? ORDER BY created_at DESC, rowid DESC LIMIT 1")
                .bind(space_id)
                .bind(branch)
                .fetch_optional(&self.pool)
                .await?;
        Ok(row.map(|(k,)| k))
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
