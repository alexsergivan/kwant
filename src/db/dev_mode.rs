//! Dev mode (docs/dev-mode.md): one watched GitHub repo per space, the cards kwant writes about
//! what landed on it and its open pull requests, and the backups the production floor asks for.
//! Kept in their own tables, outside exports, like approval modes, so an imported bundle can't
//! switch Dev mode on or off. Only the person writes `dev_mode_spaces` (the routes refuse agent
//! tokens). Add-only: `CREATE TABLE IF NOT EXISTS`, so a 0.6.12 build still runs on this database.

use super::{Database, DbError};
use chrono::{DateTime, Utc};
use sqlx::FromRow;

/// Most model calls Dev mode makes per space per day (plan D8).
pub const CALLS_CAP: i64 = 30;

/// Card kinds and statuses, as stored.
pub const LANDED: &str = "landed";
pub const REVIEW: &str = "review";
pub const RELEASE_NOTES: &str = "release_notes";
pub const PENDING: &str = "pending";
pub const READY: &str = "ready";
pub const FAILED: &str = "failed";
pub const STALE: &str = "stale";

const TABLES: &[&str] = &[
    "CREATE TABLE IF NOT EXISTS dev_mode_spaces (
        space_id TEXT PRIMARY KEY,
        on_ INTEGER NOT NULL DEFAULT 0,
        provider TEXT NOT NULL DEFAULT 'github',
        repo TEXT NOT NULL,
        branch TEXT NOT NULL,
        deploy_hint TEXT,
        backup_command TEXT,
        last_seen_sha TEXT,
        polled_at TIMESTAMP,
        poll_error TEXT,
        seen_at TIMESTAMP,
        updated_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP
    )",
    "CREATE TABLE IF NOT EXISTS dev_cards (
        id TEXT PRIMARY KEY,
        space_id TEXT NOT NULL,
        kind TEXT NOT NULL,
        ref TEXT NOT NULL,
        head_sha TEXT NOT NULL,
        status TEXT NOT NULL,
        facts TEXT NOT NULL,
        card TEXT,
        created_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
        updated_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
        UNIQUE(space_id, kind, ref, head_sha)
    )",
    "CREATE INDEX IF NOT EXISTS idx_dev_cards_space ON dev_cards(space_id, kind, created_at)",
    "CREATE TABLE IF NOT EXISTS dev_backups (
        id TEXT PRIMARY KEY,
        space_id TEXT NOT NULL,
        kwant_id TEXT,
        path TEXT,
        bytes INTEGER,
        ok INTEGER NOT NULL,
        error TEXT,
        taken_at TIMESTAMP NOT NULL
    )",
    "CREATE INDEX IF NOT EXISTS idx_dev_backups_space ON dev_backups(space_id, taken_at)",
    // Model calls per space per UTC day, for the D8 cap (not in exports either)
    "CREATE TABLE IF NOT EXISTS dev_model_calls (
        space_id TEXT NOT NULL,
        day TEXT NOT NULL,
        calls INTEGER NOT NULL DEFAULT 0,
        PRIMARY KEY (space_id, day)
    )",
];

/// A space's Dev mode settings and poll state.
#[derive(Debug, Clone, FromRow, PartialEq)]
pub struct DevModeRow {
    pub space_id: String,
    #[sqlx(rename = "on_")]
    pub on: bool,
    pub provider: String,
    pub repo: String,
    pub branch: String,
    pub deploy_hint: Option<String>,
    pub backup_command: Option<String>,
    pub last_seen_sha: Option<String>,
    pub polled_at: Option<DateTime<Utc>>,
    pub poll_error: Option<String>,
    pub seen_at: Option<DateTime<Utc>>,
    pub updated_at: DateTime<Utc>,
}

/// One card: what landed, a review of an open PR head, or a release-notes draft.
#[derive(Debug, Clone, FromRow, PartialEq)]
pub struct DevCardRow {
    pub id: String,
    pub space_id: String,
    pub kind: String,
    #[sqlx(rename = "ref")]
    pub ref_: String,
    pub head_sha: String,
    pub status: String,
    /// `server::dev::facts::DevFacts` JSON (or `ReleaseFacts` for release notes).
    pub facts: String,
    /// The card JSON (plan §3.4) once generated.
    pub card: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, FromRow, PartialEq, serde::Serialize)]
pub struct DevBackupRow {
    pub id: String,
    pub space_id: String,
    pub kwant_id: Option<String>,
    pub path: Option<String>,
    pub bytes: Option<i64>,
    pub ok: bool,
    pub error: Option<String>,
    pub taken_at: DateTime<Utc>,
}

const CARD_COLS: &str = "id, space_id, kind, ref, head_sha, status, facts, card, created_at, updated_at";

impl Database {
    pub(super) async fn migrate_dev_mode(&self) -> Result<(), DbError> {
        for sql in TABLES {
            sqlx::query(sql).execute(&self.pool).await?;
        }
        Ok(())
    }

    pub async fn dev_mode(&self, space_id: &str) -> Result<Option<DevModeRow>, DbError> {
        Ok(sqlx::query_as("SELECT * FROM dev_mode_spaces WHERE space_id = ?").bind(space_id).fetch_optional(&self.pool).await?)
    }

    /// Every space with Dev mode on, for the poller.
    pub async fn dev_mode_on_spaces(&self) -> Result<Vec<DevModeRow>, DbError> {
        Ok(sqlx::query_as("SELECT * FROM dev_mode_spaces WHERE on_ = 1 ORDER BY space_id").fetch_all(&self.pool).await?)
    }

    /// Saves the person's settings. A new repo or branch starts the watch over (`last_seen_sha`
    /// and the poll state are cleared); the same one keeps them.
    pub async fn put_dev_mode(
        &self,
        space_id: &str,
        on: bool,
        repo: &str,
        branch: &str,
        deploy_hint: Option<&str>,
        backup_command: Option<&str>,
    ) -> Result<DevModeRow, DbError> {
        let now = Utc::now();
        sqlx::query(
            "INSERT INTO dev_mode_spaces (space_id, on_, repo, branch, deploy_hint, backup_command, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
             ON CONFLICT(space_id) DO UPDATE SET
               last_seen_sha = CASE WHEN dev_mode_spaces.repo = excluded.repo AND dev_mode_spaces.branch = excluded.branch THEN dev_mode_spaces.last_seen_sha ELSE NULL END,
               polled_at = CASE WHEN dev_mode_spaces.repo = excluded.repo AND dev_mode_spaces.branch = excluded.branch THEN dev_mode_spaces.polled_at ELSE NULL END,
               poll_error = CASE WHEN dev_mode_spaces.repo = excluded.repo AND dev_mode_spaces.branch = excluded.branch THEN dev_mode_spaces.poll_error ELSE NULL END,
               on_ = excluded.on_, repo = excluded.repo, branch = excluded.branch,
               deploy_hint = excluded.deploy_hint, backup_command = excluded.backup_command, updated_at = excluded.updated_at",
        )
        .bind(space_id)
        .bind(on)
        .bind(repo)
        .bind(branch)
        .bind(deploy_hint)
        .bind(backup_command)
        .bind(now)
        .execute(&self.pool)
        .await?;
        Ok(self.dev_mode(space_id).await?.expect("just written"))
    }

    pub async fn dev_mode_polled(&self, space_id: &str, last_seen_sha: Option<&str>, error: Option<&str>) -> Result<(), DbError> {
        sqlx::query("UPDATE dev_mode_spaces SET last_seen_sha = COALESCE(?2, last_seen_sha), polled_at = ?3, poll_error = ?4 WHERE space_id = ?1")
            .bind(space_id)
            .bind(last_seen_sha)
            .bind(Utc::now())
            .bind(error)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    pub async fn dev_mode_seen(&self, space_id: &str, at: DateTime<Utc>) -> Result<(), DbError> {
        sqlx::query("UPDATE dev_mode_spaces SET seen_at = ?2 WHERE space_id = ?1").bind(space_id).bind(at).execute(&self.pool).await?;
        Ok(())
    }

    pub async fn delete_dev_mode_of_space(&self, space_id: &str) -> Result<(), DbError> {
        let mut tx = self.pool.begin().await?;
        for t in ["dev_mode_spaces", "dev_cards", "dev_backups", "dev_model_calls"] {
            sqlx::query(&format!("DELETE FROM {t} WHERE space_id = ?")).bind(space_id).execute(&mut *tx).await?;
        }
        tx.commit().await?;
        Ok(())
    }

    /// Adds a pending card. `None` when one with the same space, kind, ref and head already exists.
    pub async fn insert_dev_card(&self, space_id: &str, kind: &str, ref_: &str, head_sha: &str, facts: &str) -> Result<Option<DevCardRow>, DbError> {
        let id = format!("dvc_{}", uuid::Uuid::new_v4().simple());
        let now = Utc::now();
        let done = sqlx::query(
            "INSERT INTO dev_cards (id, space_id, kind, ref, head_sha, status, facts, created_at, updated_at)
             VALUES (?, ?, ?, ?, ?, 'pending', ?, ?, ?) ON CONFLICT(space_id, kind, ref, head_sha) DO NOTHING",
        )
        .bind(&id)
        .bind(space_id)
        .bind(kind)
        .bind(ref_)
        .bind(head_sha)
        .bind(facts)
        .bind(now)
        .bind(now)
        .execute(&self.pool)
        .await?;
        if done.rows_affected() == 0 {
            return Ok(None);
        }
        self.dev_card(&id).await
    }

    pub async fn dev_card(&self, id: &str) -> Result<Option<DevCardRow>, DbError> {
        Ok(sqlx::query_as(&format!("SELECT {CARD_COLS} FROM dev_cards WHERE id = ?")).bind(id).fetch_optional(&self.pool).await?)
    }

    /// Sets a card's status, and its card JSON when given (`None` keeps the stored one).
    pub async fn set_dev_card(&self, id: &str, status: &str, card: Option<&str>) -> Result<Option<DevCardRow>, DbError> {
        sqlx::query("UPDATE dev_cards SET status = ?2, card = COALESCE(?3, card), updated_at = ?4 WHERE id = ?1")
            .bind(id)
            .bind(status)
            .bind(card)
            .bind(Utc::now())
            .execute(&self.pool)
            .await?;
        self.dev_card(id).await
    }

    /// Replaces a card's JSON only while it still has `expect` (compare-and-set), so two finding
    /// actions can't overwrite each other. False when it changed in between.
    pub async fn swap_dev_card_json(&self, id: &str, expect: Option<&str>, card: &str) -> Result<bool, DbError> {
        let done = sqlx::query("UPDATE dev_cards SET card = ?3, updated_at = ?4 WHERE id = ?1 AND card IS ?2")
            .bind(id)
            .bind(expect)
            .bind(card)
            .bind(Utc::now())
            .execute(&self.pool)
            .await?;
        Ok(done.rows_affected() == 1)
    }

    /// A space's cards of one kind, newest first, created before `before` when given.
    pub async fn dev_cards(&self, space_id: &str, kind: &str, before: Option<DateTime<Utc>>, limit: i64) -> Result<Vec<DevCardRow>, DbError> {
        Ok(sqlx::query_as(&format!(
            "SELECT {CARD_COLS} FROM dev_cards WHERE space_id = ?1 AND kind = ?2 AND (?3 IS NULL OR created_at < ?3)
             ORDER BY created_at DESC, id DESC LIMIT ?4"
        ))
        .bind(space_id)
        .bind(kind)
        .bind(before)
        .bind(limit)
        .fetch_all(&self.pool)
        .await?)
    }

    /// Cards of one kind and ref (every head a PR had), newest first.
    pub async fn dev_cards_of_ref(&self, space_id: &str, kind: &str, ref_: &str) -> Result<Vec<DevCardRow>, DbError> {
        Ok(sqlx::query_as(&format!("SELECT {CARD_COLS} FROM dev_cards WHERE space_id = ? AND kind = ? AND ref = ? ORDER BY created_at DESC, id DESC"))
            .bind(space_id)
            .bind(kind)
            .bind(ref_)
            .fetch_all(&self.pool)
            .await?)
    }

    /// Cards still pending (a restart left them), oldest first.
    pub async fn pending_dev_cards(&self) -> Result<Vec<DevCardRow>, DbError> {
        Ok(sqlx::query_as(&format!("SELECT {CARD_COLS} FROM dev_cards WHERE status = 'pending' ORDER BY created_at")).fetch_all(&self.pool).await?)
    }

    /// Marks every review card of a PR whose head isn't `keep_head` stale; returns their ids.
    pub async fn stale_dev_reviews(&self, space_id: &str, ref_: &str, keep_head: Option<&str>) -> Result<Vec<String>, DbError> {
        let rows: Vec<(String,)> = sqlx::query_as(
            "UPDATE dev_cards SET status = 'stale', updated_at = ?4
             WHERE space_id = ?1 AND kind = 'review' AND ref = ?2 AND status != 'stale' AND (?3 IS NULL OR head_sha != ?3) RETURNING id",
        )
        .bind(space_id)
        .bind(ref_)
        .bind(keep_head)
        .bind(Utc::now())
        .fetch_all(&self.pool)
        .await?;
        Ok(rows.into_iter().map(|r| r.0).collect())
    }

    /// Model calls Dev mode made in a space today (UTC).
    pub async fn dev_calls_today(&self, space_id: &str) -> Result<i64, DbError> {
        let day = Utc::now().format("%Y-%m-%d").to_string();
        let row: Option<(i64,)> = sqlx::query_as("SELECT calls FROM dev_model_calls WHERE space_id = ? AND day = ?")
            .bind(space_id)
            .bind(day)
            .fetch_optional(&self.pool)
            .await?;
        Ok(row.map(|r| r.0).unwrap_or(0))
    }

    /// Takes one model call from today's [`CALLS_CAP`]: false (and nothing counted) once it's used up.
    pub async fn take_dev_call(&self, space_id: &str) -> Result<bool, DbError> {
        let day = Utc::now().format("%Y-%m-%d").to_string();
        let done = sqlx::query(
            "INSERT INTO dev_model_calls (space_id, day, calls) VALUES (?1, ?2, 1)
             ON CONFLICT(space_id, day) DO UPDATE SET calls = calls + 1 WHERE calls < ?3",
        )
        .bind(space_id)
        .bind(day)
        .bind(CALLS_CAP)
        .execute(&self.pool)
        .await?;
        Ok(done.rows_affected() == 1)
    }

    pub async fn insert_dev_backup(&self, b: &DevBackupRow) -> Result<(), DbError> {
        sqlx::query("INSERT INTO dev_backups (id, space_id, kwant_id, path, bytes, ok, error, taken_at) VALUES (?, ?, ?, ?, ?, ?, ?, ?)")
            .bind(&b.id)
            .bind(&b.space_id)
            .bind(&b.kwant_id)
            .bind(&b.path)
            .bind(b.bytes)
            .bind(b.ok)
            .bind(&b.error)
            .bind(b.taken_at)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    /// The space's newest backup, and its newest one that worked.
    pub async fn latest_dev_backups(&self, space_id: &str) -> Result<(Option<DevBackupRow>, Option<DevBackupRow>), DbError> {
        let last = sqlx::query_as("SELECT * FROM dev_backups WHERE space_id = ? ORDER BY taken_at DESC LIMIT 1").bind(space_id).fetch_optional(&self.pool).await?;
        let ok = sqlx::query_as("SELECT * FROM dev_backups WHERE space_id = ? AND ok = 1 ORDER BY taken_at DESC LIMIT 1")
            .bind(space_id)
            .fetch_optional(&self.pool)
            .await?;
        Ok((last, ok))
    }
}
