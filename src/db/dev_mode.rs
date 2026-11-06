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

/// "Some space has Dev mode on", kept in memory so the production floor skips its reads while
/// Dev mode is off everywhere. Read at open and re-read after every write to `dev_mode_spaces`,
/// under `writes`, so two toggles can't leave it stale. A failed read counts as on (the floor runs).
#[derive(Debug, Default)]
pub struct DevOn {
    on: std::sync::atomic::AtomicBool,
    writes: tokio::sync::Mutex<()>,
}

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
    // Connections the person marked production (D10): `mcp:<server id>` or `composio:<toolkit slug>`
    "CREATE TABLE IF NOT EXISTS dev_prod_connections (
        space_id TEXT NOT NULL,
        connection_id TEXT NOT NULL,
        marked_at TIMESTAMP NOT NULL,
        PRIMARY KEY (space_id, connection_id)
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
    /// When the person last looked at what landed ("Mark seen"): the desk's rollup counts cards
    /// after it.
    pub seen_at: Option<DateTime<Utc>>,
    pub updated_at: DateTime<Utc>,
    /// When the person last opened the desk (flow F6): the row badge counts review cards after it.
    /// Its own column since 0.7.2, so opening the desk no longer empties the landed rollup.
    #[sqlx(default)]
    pub reviews_seen_at: Option<DateTime<Utc>>,
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
    /// Review cards: `{at, login, url, comments}` once posted to GitHub (post-to-github.md P1).
    /// A column of its own, so a card regenerated after CI keeps it.
    #[sqlx(default)]
    pub posted: Option<String>,
    /// Review cards: `{kind, message}` from the last post that didn't go through.
    #[sqlx(default)]
    pub post_error: Option<String>,
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

/// Adds `dev_mode_spaces.reviews_seen_at` and fills it from `seen_at` (run once, see
/// `migrate_dev_mode`). Additive only: older builds keep reading and writing the table.
const MIGRATE_REVIEWS_SEEN: [&str; 2] = [
    "ALTER TABLE dev_mode_spaces ADD COLUMN reviews_seen_at TIMESTAMP",
    "UPDATE dev_mode_spaces SET reviews_seen_at = seen_at WHERE reviews_seen_at IS NULL",
];

const CARD_COLS: &str = "id, space_id, kind, ref, head_sha, status, facts, card, created_at, updated_at, posted, post_error";

impl Database {
    pub(super) async fn migrate_dev_mode(&self) -> Result<(), DbError> {
        for sql in TABLES {
            sqlx::query(sql).execute(&self.pool).await?;
        }
        // Posting a review to GitHub came in 0.7.1; older builds name their columns, so they
        // still read this table
        for column in ["posted", "post_error"] {
            let has: Option<(String,)> =
                sqlx::query_as("SELECT name FROM pragma_table_info('dev_cards') WHERE name = ?").bind(column).fetch_optional(&self.pool).await?;
            if has.is_none() {
                sqlx::query(&format!("ALTER TABLE dev_cards ADD COLUMN {column} TEXT")).execute(&self.pool).await?;
            }
        }
        // 0.7.2: the badge's "seen" (desk opened) apart from the rollup's (Mark seen). Added once,
        // with the backfill in the same transaction, so a database from before keeps its badge
        // state and a later start never copies a Mark seen into the badge.
        let has: Option<(String,)> = sqlx::query_as("SELECT name FROM pragma_table_info('dev_mode_spaces') WHERE name = 'reviews_seen_at'")
            .fetch_optional(&self.pool)
            .await?;
        if has.is_none() {
            let mut tx = self.pool.begin().await?;
            sqlx::query(MIGRATE_REVIEWS_SEEN[0]).execute(&mut *tx).await?;
            sqlx::query(MIGRATE_REVIEWS_SEEN[1]).execute(&mut *tx).await?;
            tx.commit().await?;
        }
        Ok(())
    }

    /// False only when no space has Dev mode on: then there is no production floor anywhere.
    pub fn any_dev_mode_on(&self) -> bool {
        self.dev_on.on.load(std::sync::atomic::Ordering::SeqCst)
    }

    pub(super) async fn refresh_dev_on(&self) {
        let on = sqlx::query_scalar::<_, bool>("SELECT EXISTS(SELECT 1 FROM dev_mode_spaces WHERE on_ = 1)").fetch_one(&self.pool).await;
        self.dev_on.on.store(on.unwrap_or(true), std::sync::atomic::Ordering::SeqCst);
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
        let _writes = self.dev_on.writes.lock().await;
        if on {
            // Before the row, so no call slips between the write and the flag
            self.dev_on.on.store(true, std::sync::atomic::Ordering::SeqCst);
        }
        let now = Utc::now();
        let done = sqlx::query(
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
        .await;
        self.refresh_dev_on().await;
        done?;
        Ok(self.dev_mode(space_id).await?.expect("just written"))
    }

    /// Binds a repo to a space that is on with none (plan any-git.md S3, QA B1), and only then: a
    /// space turned off or given a repo meanwhile is left as the person set it. True when bound.
    pub async fn bind_dev_repo(&self, space_id: &str, repo: &str, branch: &str, deploy_hint: Option<&str>) -> Result<bool, DbError> {
        let _writes = self.dev_on.writes.lock().await;
        let done = sqlx::query(
            "UPDATE dev_mode_spaces SET repo = ?2, branch = ?3, deploy_hint = ?4, last_seen_sha = NULL, polled_at = NULL,
               poll_error = NULL, updated_at = ?5
             WHERE space_id = ?1 AND on_ = 1 AND repo = ''",
        )
        .bind(space_id)
        .bind(repo)
        .bind(branch)
        .bind(deploy_hint)
        .bind(Utc::now())
        .execute(&self.pool)
        .await?;
        Ok(done.rows_affected() == 1)
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

    /// "Mark seen" on what landed: the rollup starts over from `at`. The badge is left alone.
    pub async fn dev_mode_seen(&self, space_id: &str, at: DateTime<Utc>) -> Result<(), DbError> {
        sqlx::query("UPDATE dev_mode_spaces SET seen_at = ?2 WHERE space_id = ?1").bind(space_id).bind(at).execute(&self.pool).await?;
        Ok(())
    }

    /// The desk was opened (flow F6): the review badge starts over from `at`. The rollup is left
    /// alone, so what landed since the last Mark seen still shows.
    pub async fn dev_mode_reviews_seen(&self, space_id: &str, at: DateTime<Utc>) -> Result<(), DbError> {
        sqlx::query("UPDATE dev_mode_spaces SET reviews_seen_at = ?2 WHERE space_id = ?1").bind(space_id).bind(at).execute(&self.pool).await?;
        Ok(())
    }

    pub async fn delete_dev_mode_of_space(&self, space_id: &str) -> Result<(), DbError> {
        let _writes = self.dev_on.writes.lock().await;
        let done = self.delete_dev_rows(space_id).await;
        self.refresh_dev_on().await;
        done
    }

    async fn delete_dev_rows(&self, space_id: &str) -> Result<(), DbError> {
        let mut tx = self.pool.begin().await?;
        for t in ["dev_mode_spaces", "dev_cards", "dev_backups", "dev_model_calls", "dev_prod_connections"] {
            sqlx::query(&format!("DELETE FROM {t} WHERE space_id = ?")).bind(space_id).execute(&mut *tx).await?;
        }
        tx.commit().await?;
        Ok(())
    }

    /// The connection ids the person marked production in a space (D10).
    pub async fn prod_connections(&self, space_id: &str) -> Result<Vec<String>, DbError> {
        let rows: Vec<(String,)> = sqlx::query_as("SELECT connection_id FROM dev_prod_connections WHERE space_id = ? ORDER BY connection_id")
            .bind(space_id)
            .fetch_all(&self.pool)
            .await?;
        Ok(rows.into_iter().map(|r| r.0).collect())
    }

    /// Marks or unmarks a connection production.
    pub async fn set_prod_connection(&self, space_id: &str, connection_id: &str, production: bool) -> Result<(), DbError> {
        if production {
            sqlx::query("INSERT INTO dev_prod_connections (space_id, connection_id, marked_at) VALUES (?, ?, ?) ON CONFLICT DO NOTHING")
                .bind(space_id)
                .bind(connection_id)
                .bind(Utc::now())
                .execute(&self.pool)
                .await?;
        } else {
            sqlx::query("DELETE FROM dev_prod_connections WHERE space_id = ? AND connection_id = ?").bind(space_id).bind(connection_id).execute(&self.pool).await?;
        }
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

    /// Sets a card's status, and its card JSON when given (`None` keeps the stored one). A `stale`
    /// card stays as it is: a review that finishes after its PR merged (or moved on) must not come
    /// back to life. `None` when nothing was written (no such card, or it is stale).
    pub async fn set_dev_card(&self, id: &str, status: &str, card: Option<&str>) -> Result<Option<DevCardRow>, DbError> {
        let done = sqlx::query("UPDATE dev_cards SET status = ?2, card = COALESCE(?3, card), updated_at = ?4 WHERE id = ?1 AND status != 'stale'")
            .bind(id)
            .bind(status)
            .bind(card)
            .bind(Utc::now())
            .execute(&self.pool)
            .await?;
        if done.rows_affected() == 0 {
            return Ok(None);
        }
        self.dev_card(id).await
    }

    /// New facts for a card (CI finished after the card was made): back to pending so `generate`
    /// runs again; the card JSON stays until then.
    pub async fn set_dev_card_facts(&self, id: &str, facts: &str) -> Result<Option<DevCardRow>, DbError> {
        sqlx::query("UPDATE dev_cards SET facts = ?2, status = 'pending', updated_at = ?3 WHERE id = ?1")
            .bind(id)
            .bind(facts)
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

    /// Records a review as posted, only while none is (P6): false when another post got there
    /// first. Clears `post_error`.
    pub async fn set_dev_card_posted(&self, id: &str, posted: &str) -> Result<bool, DbError> {
        let done = sqlx::query("UPDATE dev_cards SET posted = ?2, post_error = NULL, updated_at = ?3 WHERE id = ?1 AND posted IS NULL")
            .bind(id)
            .bind(posted)
            .bind(Utc::now())
            .execute(&self.pool)
            .await?;
        Ok(done.rows_affected() == 1)
    }

    /// Records why a post didn't go through (`None` clears it). A posted card keeps no error.
    pub async fn set_dev_card_post_error(&self, id: &str, error: Option<&str>) -> Result<(), DbError> {
        sqlx::query("UPDATE dev_cards SET post_error = ?2, updated_at = ?3 WHERE id = ?1 AND posted IS NULL")
            .bind(id)
            .bind(error)
            .bind(Utc::now())
            .execute(&self.pool)
            .await?;
        Ok(())
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

    /// Model calls Dev mode made in a space today (the UTC day of `dev::now()`).
    pub async fn dev_calls_today(&self, space_id: &str) -> Result<i64, DbError> {
        self.dev_calls_on(space_id, &crate::server::dev::utc_day(crate::server::dev::now())).await
    }

    pub async fn dev_calls_on(&self, space_id: &str, day: &str) -> Result<i64, DbError> {
        let row: Option<(i64,)> = sqlx::query_as("SELECT calls FROM dev_model_calls WHERE space_id = ? AND day = ?")
            .bind(space_id)
            .bind(day)
            .fetch_optional(&self.pool)
            .await?;
        Ok(row.map(|r| r.0).unwrap_or(0))
    }

    /// Takes one model call from today's cap (`dev::call_cap()`, default [`CALLS_CAP`]): false (and
    /// nothing counted) once it's used up.
    pub async fn take_dev_call(&self, space_id: &str) -> Result<bool, DbError> {
        self.take_dev_call_at(space_id, crate::server::dev::now(), crate::server::dev::call_cap()).await
    }

    /// [`Self::take_dev_call`] on the UTC day of `now`, against `cap`.
    pub async fn take_dev_call_at(&self, space_id: &str, now: DateTime<Utc>, cap: i64) -> Result<bool, DbError> {
        if cap <= 0 {
            return Ok(false);
        }
        let done = sqlx::query(
            "INSERT INTO dev_model_calls (space_id, day, calls) VALUES (?1, ?2, 1)
             ON CONFLICT(space_id, day) DO UPDATE SET calls = calls + 1 WHERE calls < ?3",
        )
        .bind(space_id)
        .bind(crate::server::dev::utc_day(now))
        .bind(cap)
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
