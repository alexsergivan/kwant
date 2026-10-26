//! Work-ahead runs: one row per hidden run in a quiet chat (see [`crate::server::work_ahead`]).
//! The rows give the caps (runs per space per day, one per chat per day) and keep a chat from
//! being worked on again before the person writes there.

use super::spaces::SpaceId;
use super::{Database, DbError};
use chrono::{DateTime, Utc};
use serde::Serialize;
use sqlx::FromRow;
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, FromRow, PartialEq)]
pub struct WorkAheadRun {
    pub id: String,
    pub space_id: String,
    /// The chat: a channel, a thread or a bot's direct chat.
    pub context_id: String,
    pub kwant_id: String,
    /// The day, `YYYY-MM-DD` in UTC.
    pub day: String,
    /// "running", "posted", "nothing" or "failed".
    pub status: String,
    pub message_id: Option<String>,
    pub fresh_tokens: i64,
    pub started_at: DateTime<Utc>,
    pub finished_at: Option<DateTime<Utc>>,
}

impl Database {
    pub(super) async fn migrate_work_ahead(&self) -> Result<(), DbError> {
        for stmt in [
            "CREATE TABLE IF NOT EXISTS work_ahead_runs (
                id TEXT PRIMARY KEY,
                space_id TEXT NOT NULL,
                context_id TEXT NOT NULL,
                kwant_id TEXT NOT NULL,
                day TEXT NOT NULL,
                status TEXT NOT NULL CHECK (status IN ('running', 'posted', 'nothing', 'failed')),
                message_id TEXT,
                fresh_tokens INTEGER NOT NULL DEFAULT 0,
                started_at TIMESTAMP NOT NULL,
                finished_at TIMESTAMP
            )",
            "CREATE INDEX IF NOT EXISTS work_ahead_runs_space_day ON work_ahead_runs (space_id, day)",
            "CREATE INDEX IF NOT EXISTS work_ahead_runs_context ON work_ahead_runs (context_id, started_at)",
        ] {
            sqlx::query(stmt).execute(&self.pool).await?;
        }
        // A restart mid-run leaves nothing running
        sqlx::query("UPDATE work_ahead_runs SET status = 'failed', finished_at = ? WHERE status = 'running'")
            .bind(Utc::now())
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    pub async fn start_work_ahead_run(&self, space: &SpaceId, context_id: &str, kwant_id: &str, day: &str) -> Result<WorkAheadRun, DbError> {
        Ok(sqlx::query_as::<_, WorkAheadRun>(
            "INSERT INTO work_ahead_runs (id, space_id, context_id, kwant_id, day, status, started_at) \
             VALUES (?, ?, ?, ?, ?, 'running', ?) RETURNING *",
        )
        .bind(Uuid::new_v4().to_string())
        .bind(space.as_str())
        .bind(context_id)
        .bind(kwant_id)
        .bind(day)
        .bind(Utc::now())
        .fetch_one(&self.pool)
        .await?)
    }

    pub async fn finish_work_ahead_run(&self, id: &str, status: &str, message_id: Option<&str>, fresh_tokens: u64) -> Result<(), DbError> {
        sqlx::query("UPDATE work_ahead_runs SET status = ?, message_id = ?, fresh_tokens = ?, finished_at = ? WHERE id = ?")
            .bind(status)
            .bind(message_id)
            .bind(fresh_tokens.min(i64::MAX as u64) as i64)
            .bind(Utc::now())
            .bind(id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    /// Runs in `space` on `day`, any outcome, and the most fresh tokens one of them used.
    pub async fn work_ahead_runs_on(&self, space: &SpaceId, day: &str) -> Result<(i64, i64), DbError> {
        Ok(sqlx::query_as("SELECT COUNT(*), COALESCE(MAX(fresh_tokens), 0) FROM work_ahead_runs WHERE space_id = ? AND day = ?")
            .bind(space.as_str())
            .bind(day)
            .fetch_one(&self.pool)
            .await?)
    }

    /// The chat's newest run, if any.
    pub async fn last_work_ahead_run(&self, context_id: &str) -> Result<Option<WorkAheadRun>, DbError> {
        Ok(sqlx::query_as::<_, WorkAheadRun>(
            "SELECT * FROM work_ahead_runs WHERE context_id = ? ORDER BY started_at DESC, rowid DESC LIMIT 1",
        )
        .bind(context_id)
        .fetch_optional(&self.pool)
        .await?)
    }

    /// Chats with the newest messages, newest first, at most `limit`: (thread, channel, direct chat).
    pub async fn recent_chats(&self, limit: i64) -> Result<Vec<(Option<String>, Option<String>, Option<String>)>, DbError> {
        let rows: Vec<(Option<String>, Option<String>, Option<String>)> = sqlx::query_as(
            "SELECT thread_id, channel_id, direct_kwant_id FROM messages ORDER BY rowid DESC LIMIT ?",
        )
        .bind(limit.saturating_mul(20))
        .fetch_all(&self.pool)
        .await?;
        let mut out: Vec<(Option<String>, Option<String>, Option<String>)> = Vec::new();
        for r in rows {
            if !out.contains(&r) {
                out.push(r);
            }
            if out.len() as i64 >= limit {
                break;
            }
        }
        Ok(out)
    }
}
