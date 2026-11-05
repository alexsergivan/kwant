//! The idea jar (see [`crate::server::idea_jar`]): the person's ideas, and one row per take a bot
//! wrote (or tried to write) on one. The take rows give the daily cap per space.

use super::spaces::SpaceId;
use super::{Database, DbError};
use chrono::{DateTime, Utc};
use serde::Serialize;
use sqlx::FromRow;
use uuid::Uuid;

/// Every status an idea can have. `new` → `peeking` → `ready`, then `picked` (it has a
/// `mission_id`) or `later`. A take that failed twice leaves it `failed`. Editing sends it back
/// to `new`.
pub const IDEA_STATUSES: [&str; 6] = ["new", "peeking", "ready", "picked", "later", "failed"];

#[derive(Debug, Clone, Serialize, FromRow, PartialEq)]
pub struct Idea {
    pub id: String,
    pub space_id: String,
    /// As the person typed it. Always data, never instructions.
    pub text: String,
    pub status: String,
    /// The bot that wrote the take (or is writing it).
    pub kwant_id: Option<String>,
    /// The take, JSON (see [`crate::server::idea_jar::Take`]).
    pub take: Option<String>,
    /// The draft mission Do it made.
    pub mission_id: Option<String>,
    /// Why the last take failed, in plain words.
    pub error: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub peeked_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Serialize, FromRow, PartialEq)]
pub struct IdeaTake {
    pub id: String,
    pub idea_id: String,
    pub space_id: String,
    pub kwant_id: String,
    /// The day, `YYYY-MM-DD` in UTC.
    pub day: String,
    /// "scan" (a free bot peeked) or "look" (the person's Look now).
    pub trigger: String,
    /// "running", "ready" or "failed".
    pub status: String,
    /// Turns used: 1, or 2 after a retry.
    pub tries: i64,
    pub fresh_tokens: i64,
    /// A turn of this take used more than its fresh-token cap.
    pub over_cap: bool,
    pub error: Option<String>,
    pub started_at: DateTime<Utc>,
    pub finished_at: Option<DateTime<Utc>>,
}

impl Database {
    pub(super) async fn migrate_ideas(&self) -> Result<(), DbError> {
        for stmt in [
            "CREATE TABLE IF NOT EXISTS ideas (
                id TEXT PRIMARY KEY,
                space_id TEXT NOT NULL,
                text TEXT NOT NULL,
                status TEXT NOT NULL CHECK (status IN ('new', 'peeking', 'ready', 'picked', 'later', 'failed')),
                kwant_id TEXT,
                take TEXT,
                mission_id TEXT,
                error TEXT,
                created_at TIMESTAMP NOT NULL,
                updated_at TIMESTAMP NOT NULL,
                peeked_at TIMESTAMP
            )",
            "CREATE INDEX IF NOT EXISTS ideas_space_status ON ideas (space_id, status, created_at)",
            "CREATE TABLE IF NOT EXISTS idea_takes (
                id TEXT PRIMARY KEY,
                idea_id TEXT NOT NULL,
                space_id TEXT NOT NULL,
                kwant_id TEXT NOT NULL,
                day TEXT NOT NULL,
                trigger TEXT NOT NULL CHECK (trigger IN ('scan', 'look')),
                status TEXT NOT NULL CHECK (status IN ('running', 'ready', 'failed')),
                tries INTEGER NOT NULL DEFAULT 0,
                fresh_tokens INTEGER NOT NULL DEFAULT 0,
                over_cap INTEGER NOT NULL DEFAULT 0,
                error TEXT,
                started_at TIMESTAMP NOT NULL,
                finished_at TIMESTAMP
            )",
            "CREATE INDEX IF NOT EXISTS idea_takes_space_day ON idea_takes (space_id, day)",
            "CREATE INDEX IF NOT EXISTS idea_takes_idea ON idea_takes (idea_id, started_at)",
        ] {
            sqlx::query(stmt).execute(&self.pool).await?;
        }
        // A restart mid-take leaves nothing peeking: the idea goes back in the jar
        let now = Utc::now();
        sqlx::query("UPDATE idea_takes SET status = 'failed', error = 'interrupted by a restart', finished_at = ? WHERE status = 'running'")
            .bind(now)
            .execute(&self.pool)
            .await?;
        sqlx::query("UPDATE ideas SET status = 'new', kwant_id = NULL, updated_at = ? WHERE status = 'peeking'")
            .bind(now)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    pub async fn create_idea(&self, space: &SpaceId, text: &str) -> Result<Idea, DbError> {
        let now = Utc::now();
        Ok(sqlx::query_as::<_, Idea>(
            "INSERT INTO ideas (id, space_id, text, status, created_at, updated_at) VALUES (?, ?, ?, 'new', ?, ?) RETURNING *",
        )
        .bind(Uuid::new_v4().to_string())
        .bind(space.as_str())
        .bind(text)
        .bind(now)
        .bind(now)
        .fetch_one(&self.pool)
        .await?)
    }

    pub async fn get_idea(&self, id: &str) -> Result<Option<Idea>, DbError> {
        Ok(sqlx::query_as::<_, Idea>("SELECT * FROM ideas WHERE id = ?").bind(id).fetch_optional(&self.pool).await?)
    }

    /// The space's ideas with `status` (any when `None`), newest first, at most 500.
    pub async fn list_ideas(&self, space: &SpaceId, status: Option<&str>) -> Result<Vec<Idea>, DbError> {
        Ok(sqlx::query_as::<_, Idea>(
            "SELECT * FROM ideas WHERE space_id = ? AND (? IS NULL OR status = ?) ORDER BY created_at DESC, rowid DESC LIMIT 500",
        )
        .bind(space.as_str())
        .bind(status)
        .bind(status)
        .fetch_all(&self.pool)
        .await?)
    }

    /// `(status, count)` for every status the space has ideas in.
    pub async fn idea_counts(&self, space: &SpaceId) -> Result<Vec<(String, i64)>, DbError> {
        Ok(sqlx::query_as("SELECT status, COUNT(*) FROM ideas WHERE space_id = ? GROUP BY status")
            .bind(space.as_str())
            .fetch_all(&self.pool)
            .await?)
    }

    /// The space's oldest idea nobody has peeked at.
    pub async fn oldest_new_idea(&self, space: &SpaceId) -> Result<Option<Idea>, DbError> {
        Ok(sqlx::query_as::<_, Idea>("SELECT * FROM ideas WHERE space_id = ? AND status = 'new' ORDER BY created_at, rowid LIMIT 1")
            .bind(space.as_str())
            .fetch_optional(&self.pool)
            .await?)
    }

    /// Spaces with an idea nobody has peeked at, and none being peeked at.
    pub async fn spaces_with_new_ideas(&self) -> Result<Vec<String>, DbError> {
        let rows: Vec<(String,)> = sqlx::query_as(
            "SELECT DISTINCT space_id FROM ideas i WHERE status = 'new' \
             AND NOT EXISTS (SELECT 1 FROM ideas p WHERE p.space_id = i.space_id AND p.status = 'peeking')",
        )
        .fetch_all(&self.pool)
        .await?;
        Ok(rows.into_iter().map(|r| r.0).collect())
    }

    /// Moves the idea from one of `from` to `peeking` under `kwant_id`. False when it was in no
    /// such status (someone else got there first, or the person changed it).
    pub async fn claim_idea(&self, id: &str, from: &[&str], kwant_id: &str) -> Result<bool, DbError> {
        let marks = vec!["?"; from.len()].join(", ");
        let sql = format!("UPDATE ideas SET status = 'peeking', kwant_id = ?, error = NULL, updated_at = ?, peeked_at = ? WHERE id = ? AND status IN ({marks})");
        let now = Utc::now();
        let mut q = sqlx::query(&sql).bind(kwant_id).bind(now).bind(now).bind(id);
        for s in from {
            q = q.bind(*s);
        }
        Ok(q.execute(&self.pool).await?.rows_affected() == 1)
    }

    /// Ends a take: `ready` with the take's JSON, or `failed` with why. Only while it is still
    /// `peeking`: an idea deleted meanwhile stays deleted.
    pub async fn finish_idea_take(&self, id: &str, take: Result<&str, &str>) -> Result<bool, DbError> {
        let (status, take, error) = match take {
            Ok(t) => ("ready", Some(t), None),
            Err(e) => ("failed", None, Some(e)),
        };
        Ok(sqlx::query("UPDATE ideas SET status = ?, take = ?, error = ?, updated_at = ? WHERE id = ? AND status = 'peeking'")
            .bind(status)
            .bind(take)
            .bind(error)
            .bind(Utc::now())
            .bind(id)
            .execute(&self.pool)
            .await?
            .rows_affected()
            == 1)
    }

    /// New text, back in the jar for a fresh look: the old take is dropped. Not while a bot
    /// peeks or once the idea is picked.
    pub async fn edit_idea(&self, id: &str, text: &str) -> Result<bool, DbError> {
        Ok(sqlx::query(
            "UPDATE ideas SET text = ?, status = 'new', kwant_id = NULL, take = NULL, error = NULL, peeked_at = NULL, updated_at = ? \
             WHERE id = ? AND status IN ('new', 'ready', 'later', 'failed')",
        )
        .bind(text)
        .bind(Utc::now())
        .bind(id)
        .execute(&self.pool)
        .await?
        .rows_affected()
            == 1)
    }

    /// Moves the idea from `from` to `to`. False when it isn't in `from`.
    pub async fn set_idea_status(&self, id: &str, from: &str, to: &str) -> Result<bool, DbError> {
        Ok(sqlx::query("UPDATE ideas SET status = ?, updated_at = ? WHERE id = ? AND status = ?")
            .bind(to)
            .bind(Utc::now())
            .bind(id)
            .bind(from)
            .execute(&self.pool)
            .await?
            .rows_affected()
            == 1)
    }

    /// `ready` → `picked` with the draft mission. False when it wasn't ready.
    pub async fn pick_idea(&self, id: &str, mission_id: &str) -> Result<bool, DbError> {
        Ok(sqlx::query("UPDATE ideas SET status = 'picked', mission_id = ?, updated_at = ? WHERE id = ? AND status = 'ready'")
            .bind(mission_id)
            .bind(Utc::now())
            .bind(id)
            .execute(&self.pool)
            .await?
            .rows_affected()
            == 1)
    }

    pub async fn delete_idea(&self, id: &str) -> Result<bool, DbError> {
        let gone = sqlx::query("DELETE FROM ideas WHERE id = ?").bind(id).execute(&self.pool).await?.rows_affected() == 1;
        // The take rows stay: they still count towards the day's cap
        Ok(gone)
    }

    pub async fn start_idea_take(&self, idea: &Idea, kwant_id: &str, day: &str, trigger: &str) -> Result<IdeaTake, DbError> {
        Ok(sqlx::query_as::<_, IdeaTake>(
            "INSERT INTO idea_takes (id, idea_id, space_id, kwant_id, day, trigger, status, started_at) \
             VALUES (?, ?, ?, ?, ?, ?, 'running', ?) RETURNING *",
        )
        .bind(Uuid::new_v4().to_string())
        .bind(&idea.id)
        .bind(&idea.space_id)
        .bind(kwant_id)
        .bind(day)
        .bind(trigger)
        .bind(Utc::now())
        .fetch_one(&self.pool)
        .await?)
    }

    pub async fn finish_idea_take_row(&self, id: &str, status: &str, tries: i64, fresh_tokens: u64, error: Option<&str>, over_cap: bool) -> Result<(), DbError> {
        sqlx::query("UPDATE idea_takes SET status = ?, tries = ?, fresh_tokens = ?, over_cap = ?, error = ?, finished_at = ? WHERE id = ?")
            .bind(status)
            .bind(tries)
            .bind(fresh_tokens.min(i64::MAX as u64) as i64)
            .bind(over_cap)
            .bind(error)
            .bind(Utc::now())
            .bind(id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    /// Scan-started takes in `space` on `day`, any outcome (Look now doesn't count), and whether
    /// any take there that day (Look now too) went over its token cap.
    pub async fn idea_scan_takes_on(&self, space: &SpaceId, day: &str) -> Result<(i64, bool), DbError> {
        let (n, over): (i64, i64) = sqlx::query_as(
            "SELECT COALESCE(SUM(trigger = 'scan'), 0), COALESCE(MAX(over_cap), 0) FROM idea_takes WHERE space_id = ? AND day = ?",
        )
        .bind(space.as_str())
        .bind(day)
        .fetch_one(&self.pool)
        .await?;
        Ok((n, over != 0))
    }

    /// Every take started in `space` on `day`, scan or Look now: the "peeks today" the jar shows.
    pub async fn idea_takes_started_on(&self, space: &SpaceId, day: &str) -> Result<i64, DbError> {
        let n: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM idea_takes WHERE space_id = ? AND day = ?")
            .bind(space.as_str())
            .bind(day)
            .fetch_one(&self.pool)
            .await?;
        Ok(n)
    }

    /// The idea's takes, newest first.
    pub async fn idea_takes(&self, idea_id: &str) -> Result<Vec<IdeaTake>, DbError> {
        Ok(sqlx::query_as::<_, IdeaTake>("SELECT * FROM idea_takes WHERE idea_id = ? ORDER BY started_at DESC, rowid DESC")
            .bind(idea_id)
            .fetch_all(&self.pool)
            .await?)
    }

    /// When the bot's newest model call was recorded: the end of its last turn, near enough.
    pub async fn last_turn_at(&self, kwant_id: &str) -> Result<Option<DateTime<Utc>>, DbError> {
        let row: Option<(DateTime<Utc>,)> = sqlx::query_as("SELECT created_at FROM model_usage WHERE kwant_id = ? ORDER BY created_at DESC LIMIT 1")
            .bind(kwant_id)
            .fetch_optional(&self.pool)
            .await?;
        Ok(row.map(|r| r.0))
    }
}
