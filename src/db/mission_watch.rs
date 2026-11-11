//! 0.9.0 P1, "Watch a mission": the watch set on a mission and the checks its watcher ran.
//! Add-only: two new tables, nothing else changes. See
//! `/home/maus/shared/ship-software-safely/release-090/p1-contract.md` §9.

use super::{Database, DbError};
use chrono::{DateTime, Utc};
use serde::Serialize;
use sqlx::FromRow;

/// The statuses of a check: `checking` while the watcher's turn runs; `cant_run` and
/// `over_limit` are recorded without a turn.
pub const CHECK_STATUSES: [&str; 5] = ["checking", "done", "failed", "cant_run", "over_limit"];

/// The watch on one mission. No row: the watch is off.
#[derive(Debug, Clone, Serialize, FromRow, PartialEq)]
pub struct Watch {
    pub mission_id: String,
    pub kwant_id: String,
    pub set_by: String,
    pub set_at: DateTime<Utc>,
    /// The stop the last check ran for (`None`: the next stop is checked).
    pub last_fingerprint: Option<String>,
    pub last_check_id: Option<String>,
    pub updated_at: DateTime<Utc>,
}

/// One check of a stopped mission.
#[derive(Debug, Clone, Serialize, FromRow, PartialEq)]
pub struct WatchCheck {
    pub id: String,
    pub mission_id: String,
    pub kwant_id: String,
    /// `stop` (kwant found a stop) or `manual` (the person asked).
    pub trigger: String,
    pub status: String,
    pub fingerprint: String,
    pub reason_code: Option<String>,
    pub reason_text: Option<String>,
    pub stopped_at: Option<DateTime<Utc>>,
    pub started_at: DateTime<Utc>,
    pub finished_at: Option<DateTime<Utc>>,
    /// JSON array of claims (contract §4).
    pub claims: String,
    /// JSON array of actions (contract §4).
    pub actions: String,
    pub message_id: Option<String>,
    pub turn_id: Option<String>,
    pub tokens: i64,
}

/// A new check row.
pub struct NewWatchCheck<'a> {
    pub mission_id: &'a str,
    pub kwant_id: &'a str,
    pub trigger: &'a str,
    pub status: &'a str,
    pub fingerprint: &'a str,
    pub reason: Option<(&'a str, &'a str)>,
    pub stopped_at: Option<DateTime<Utc>>,
}

/// How a check ended.
pub struct CheckEnd<'a> {
    pub status: &'a str,
    pub reason: Option<(&'a str, &'a str)>,
    pub claims: &'a str,
    pub actions: &'a str,
    pub tokens: i64,
}

impl Database {
    pub(super) async fn migrate_mission_watch(&self) -> Result<(), DbError> {
        for stmt in [
            "CREATE TABLE IF NOT EXISTS mission_watches (
                mission_id TEXT PRIMARY KEY REFERENCES missions(id),
                kwant_id TEXT NOT NULL,
                set_by TEXT NOT NULL,
                set_at TEXT NOT NULL,
                last_fingerprint TEXT,
                last_check_id TEXT,
                updated_at TEXT NOT NULL
            )",
            "CREATE TABLE IF NOT EXISTS mission_watch_checks (
                id TEXT PRIMARY KEY,
                mission_id TEXT NOT NULL REFERENCES missions(id),
                kwant_id TEXT NOT NULL,
                trigger TEXT NOT NULL CHECK (trigger IN ('stop', 'manual')),
                status TEXT NOT NULL CHECK (status IN ('checking', 'done', 'failed', 'cant_run', 'over_limit')),
                fingerprint TEXT NOT NULL DEFAULT '',
                reason_code TEXT,
                reason_text TEXT,
                stopped_at TEXT,
                started_at TEXT NOT NULL,
                finished_at TEXT,
                claims TEXT NOT NULL DEFAULT '[]',
                actions TEXT NOT NULL DEFAULT '[]',
                message_id TEXT,
                turn_id TEXT,
                tokens INTEGER NOT NULL DEFAULT 0
            )",
            "CREATE INDEX IF NOT EXISTS mission_watch_checks_mission ON mission_watch_checks(mission_id, started_at)",
        ] {
            sqlx::query(stmt).execute(&self.pool).await?;
        }
        Ok(())
    }

    pub async fn get_watch(&self, mission_id: &str) -> Result<Option<Watch>, DbError> {
        Ok(sqlx::query_as::<_, Watch>("SELECT * FROM mission_watches WHERE mission_id = ?").bind(mission_id).fetch_optional(&self.pool).await?)
    }

    pub async fn all_watches(&self) -> Result<Vec<Watch>, DbError> {
        Ok(sqlx::query_as::<_, Watch>("SELECT * FROM mission_watches ORDER BY set_at").fetch_all(&self.pool).await?)
    }

    /// Turns the watch on, or changes the watcher. A new watcher clears the stored fingerprint, so
    /// it checks the next stop even if it is the same one.
    pub async fn set_watch(&self, mission_id: &str, kwant_id: &str, set_by: &str) -> Result<Watch, DbError> {
        let now = Utc::now();
        sqlx::query(
            "INSERT INTO mission_watches (mission_id, kwant_id, set_by, set_at, last_fingerprint, last_check_id, updated_at) \
             VALUES (?, ?, ?, ?, NULL, NULL, ?) \
             ON CONFLICT(mission_id) DO UPDATE SET \
               last_fingerprint = CASE WHEN kwant_id = excluded.kwant_id THEN last_fingerprint ELSE NULL END, \
               set_at = CASE WHEN kwant_id = excluded.kwant_id THEN set_at ELSE excluded.set_at END, \
               set_by = CASE WHEN kwant_id = excluded.kwant_id THEN set_by ELSE excluded.set_by END, \
               kwant_id = excluded.kwant_id, updated_at = excluded.updated_at",
        )
        .bind(mission_id)
        .bind(kwant_id)
        .bind(set_by)
        .bind(now)
        .bind(now)
        .execute(&self.pool)
        .await?;
        self.get_watch(mission_id).await?.ok_or_else(|| DbError::Conflict("the watch was removed meanwhile".into()))
    }

    /// Turns the watch off. True when one was on.
    pub async fn delete_watch(&self, mission_id: &str) -> Result<bool, DbError> {
        Ok(sqlx::query("DELETE FROM mission_watches WHERE mission_id = ?").bind(mission_id).execute(&self.pool).await?.rows_affected() == 1)
    }

    /// Stores the stop a check runs for. Only while `kwant_id` still watches.
    pub async fn note_watch_check(&self, mission_id: &str, kwant_id: &str, fingerprint: &str, check_id: &str) -> Result<bool, DbError> {
        let r = sqlx::query("UPDATE mission_watches SET last_fingerprint = ?, last_check_id = ?, updated_at = ? WHERE mission_id = ? AND kwant_id = ?")
            .bind(fingerprint)
            .bind(check_id)
            .bind(Utc::now())
            .bind(mission_id)
            .bind(kwant_id)
            .execute(&self.pool)
            .await?;
        Ok(r.rows_affected() == 1)
    }

    pub async fn insert_watch_check(&self, c: &NewWatchCheck<'_>) -> Result<WatchCheck, DbError> {
        let id = format!("wc-{}", uuid::Uuid::new_v4());
        let now = Utc::now();
        let finished = (c.status != "checking").then_some(now);
        sqlx::query(
            "INSERT INTO mission_watch_checks (id, mission_id, kwant_id, trigger, status, fingerprint, reason_code, reason_text, stopped_at, started_at, finished_at) \
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(&id)
        .bind(c.mission_id)
        .bind(c.kwant_id)
        .bind(c.trigger)
        .bind(c.status)
        .bind(c.fingerprint)
        .bind(c.reason.map(|r| r.0))
        .bind(c.reason.map(|r| r.1))
        .bind(c.stopped_at)
        .bind(now)
        .bind(finished)
        .execute(&self.pool)
        .await?;
        self.get_watch_check(&id).await?.ok_or_else(|| DbError::Conflict("the check was not saved".into()))
    }

    pub async fn get_watch_check(&self, id: &str) -> Result<Option<WatchCheck>, DbError> {
        Ok(sqlx::query_as::<_, WatchCheck>("SELECT * FROM mission_watch_checks WHERE id = ?").bind(id).fetch_optional(&self.pool).await?)
    }

    /// The mission's checks, newest first.
    pub async fn watch_checks(&self, mission_id: &str, limit: i64) -> Result<Vec<WatchCheck>, DbError> {
        Ok(sqlx::query_as::<_, WatchCheck>("SELECT * FROM mission_watch_checks WHERE mission_id = ? ORDER BY started_at DESC, rowid DESC LIMIT ?")
            .bind(mission_id)
            .bind(limit.clamp(1, 50))
            .fetch_all(&self.pool)
            .await?)
    }

    pub async fn latest_watch_check(&self, mission_id: &str) -> Result<Option<WatchCheck>, DbError> {
        Ok(self.watch_checks(mission_id, 1).await?.into_iter().next())
    }

    /// Checks still `checking`, every mission's.
    pub async fn live_watch_checks(&self) -> Result<Vec<WatchCheck>, DbError> {
        Ok(sqlx::query_as::<_, WatchCheck>("SELECT * FROM mission_watch_checks WHERE status = 'checking' ORDER BY started_at").fetch_all(&self.pool).await?)
    }

    /// Done checks that hold a `needs_you` action, newest first (the Inbox reads them).
    pub async fn watch_checks_needing_you(&self, limit: i64) -> Result<Vec<WatchCheck>, DbError> {
        Ok(sqlx::query_as::<_, WatchCheck>(
            "SELECT * FROM mission_watch_checks WHERE status = 'done' AND actions LIKE '%\"needs_you\"%' ORDER BY finished_at DESC, rowid DESC LIMIT ?",
        )
        .bind(limit.clamp(1, 500))
        .fetch_all(&self.pool)
        .await?)
    }

    /// Ends a `checking` check (compare-and-set). False when it had already ended.
    pub async fn finish_watch_check(&self, id: &str, end: &CheckEnd<'_>) -> Result<bool, DbError> {
        let r = sqlx::query(
            "UPDATE mission_watch_checks SET status = ?, reason_code = ?, reason_text = ?, claims = ?, actions = ?, tokens = ?, finished_at = ? \
             WHERE id = ? AND status = 'checking'",
        )
        .bind(end.status)
        .bind(end.reason.map(|r| r.0))
        .bind(end.reason.map(|r| r.1))
        .bind(end.claims)
        .bind(end.actions)
        .bind(end.tokens)
        .bind(Utc::now())
        .bind(id)
        .execute(&self.pool)
        .await?;
        Ok(r.rows_affected() == 1)
    }

    pub async fn set_watch_check_message(&self, id: &str, message_id: &str) -> Result<(), DbError> {
        sqlx::query("UPDATE mission_watch_checks SET message_id = ? WHERE id = ?").bind(message_id).bind(id).execute(&self.pool).await?;
        Ok(())
    }

    pub async fn set_watch_check_turn(&self, id: &str, turn_id: &str) -> Result<(), DbError> {
        sqlx::query("UPDATE mission_watch_checks SET turn_id = ? WHERE id = ? AND turn_id IS NULL").bind(turn_id).bind(id).execute(&self.pool).await?;
        Ok(())
    }

    /// The watcher's turn tokens count against the mission's limit.
    pub async fn add_mission_spend(&self, mission_id: &str, tokens: i64) -> Result<(), DbError> {
        if tokens <= 0 {
            return Ok(());
        }
        sqlx::query("UPDATE missions SET spent_tokens = MAX(0, spent_tokens + ?), updated_at = ? WHERE id = ?")
            .bind(tokens)
            .bind(Utc::now())
            .bind(mission_id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    /// When a task of the mission last changed: its newest event, else its newest task update.
    pub async fn mission_last_change(&self, mission_id: &str) -> Result<Option<DateTime<Utc>>, DbError> {
        let ev: Option<(Option<DateTime<Utc>>,)> =
            sqlx::query_as("SELECT MAX(e.created_at) FROM task_events e JOIN tasks t ON t.id = e.task_id WHERE t.mission_id = ?")
                .bind(mission_id)
                .fetch_optional(&self.pool)
                .await?;
        let up: Option<(Option<DateTime<Utc>>,)> =
            sqlx::query_as("SELECT MAX(updated_at) FROM tasks WHERE mission_id = ?").bind(mission_id).fetch_optional(&self.pool).await?;
        Ok(ev.and_then(|r| r.0).into_iter().chain(up.and_then(|r| r.0)).max())
    }
}
