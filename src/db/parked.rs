//! Turns a usage limit stopped before the bot did anything: parked until the limit resets
//! (`retry_at`), then run once more. A parked turn never parks again.

use super::{Database, DbError};
use chrono::{DateTime, Utc};
use serde::Serialize;
use sqlx::FromRow;
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, FromRow, PartialEq)]
pub struct ParkedTurn {
    pub id: String,
    pub kwant_id: String,
    pub channel_id: Option<String>,
    pub direct_kwant_id: Option<String>,
    pub thread_id: Option<String>,
    /// Set when the turn was a delegated task; then the delegation runs again instead.
    pub delegation_id: Option<String>,
    pub depth: i64,
    pub prompt: String,
    pub retry_at: DateTime<Utc>,
    /// "pending", "running", "done" or "dropped" (the chat moved on before the retry).
    pub status: String,
    pub created_at: DateTime<Utc>,
    /// Why it waits: "usage_limit" (the engine's limit) or "bot_budget" (G2: the bot is paused
    /// at its monthly budget; released early by Continue or a raise).
    #[sqlx(default)]
    pub reason: String,
}

/// A held prompt never grows past this; the oldest part goes first.
const MAX_HELD_PROMPT: usize = 64 * 1024;

pub struct NewParkedTurn {
    pub kwant_id: String,
    pub channel_id: Option<String>,
    pub direct_kwant_id: Option<String>,
    pub thread_id: Option<String>,
    pub delegation_id: Option<String>,
    pub depth: i64,
    pub prompt: String,
    pub retry_at: DateTime<Utc>,
    /// "usage_limit" or "bot_budget" (see [`ParkedTurn::reason`]).
    pub reason: &'static str,
}

/// `held` with `more` after it, kept under [`MAX_HELD_PROMPT`] by dropping the oldest part.
fn appended(held: &str, more: &str) -> String {
    let mut s = format!("{held}\n\n[another message arrived while the bot was paused]\n\n{more}");
    if s.len() > MAX_HELD_PROMPT {
        let mut cut = s.len() - MAX_HELD_PROMPT;
        while !s.is_char_boundary(cut) {
            cut += 1;
        }
        s = s[cut..].to_string();
    }
    s
}

/// The chat a turn ran in, matched the way it was parked.
const SAME_CHAT: &str = "kwant_id = ? AND COALESCE(channel_id, '') = ? AND COALESCE(direct_kwant_id, '') = ? \
     AND COALESCE(thread_id, '') = ?";

impl Database {
    pub(super) async fn migrate_parked(&self) -> Result<(), DbError> {
        for stmt in [
            "CREATE TABLE IF NOT EXISTS parked_turns (
                id TEXT PRIMARY KEY,
                kwant_id TEXT NOT NULL,
                channel_id TEXT,
                direct_kwant_id TEXT,
                thread_id TEXT,
                delegation_id TEXT,
                depth INTEGER NOT NULL DEFAULT 0,
                prompt TEXT NOT NULL,
                retry_at TIMESTAMP NOT NULL,
                status TEXT NOT NULL DEFAULT 'pending',
                created_at TIMESTAMP NOT NULL
            )",
            "CREATE INDEX IF NOT EXISTS parked_turns_due ON parked_turns (status, retry_at)",
            // A delegation restarted after a kwnt restart is restarted only once
            "ALTER TABLE delegations ADD COLUMN retries INTEGER NOT NULL DEFAULT 0",
        ] {
            let _ = sqlx::query(stmt).execute(&self.pool).await;
        }
        Ok(())
    }

    /// Whether a usage-limit retry or budget hold is still waiting for `kwant_id` in the chat with
    /// that channel and thread (any direct chat of the bot when both are `None`). The P1 watcher
    /// leaves a task alone while its bot has one waiting where the mission runs it (D9).
    pub async fn parked_waiting_in(&self, kwant_id: &str, channel_id: Option<&str>, thread_id: Option<&str>) -> Result<bool, DbError> {
        let n: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM parked_turns WHERE status = 'pending' AND kwant_id = ? \
             AND COALESCE(channel_id, '') = ? AND COALESCE(thread_id, '') = ?",
        )
        .bind(kwant_id)
        .bind(channel_id.unwrap_or(""))
        .bind(thread_id.unwrap_or(""))
        .fetch_one(&self.pool)
        .await?;
        Ok(n > 0)
    }

    /// Parks a turn; `None` when one is already waiting for that bot in that chat (it keeps its
    /// time, so a chat gets at most one retry however many messages hit the limit).
    pub async fn park_turn(&self, t: &NewParkedTurn) -> Result<Option<ParkedTurn>, DbError> {
        let mut tx = self.begin_write().await?;
        let waiting: Option<(String, String, String, DateTime<Utc>)> = sqlx::query_as(&format!(
            "SELECT id, prompt, reason, retry_at FROM parked_turns WHERE status = 'pending' AND {SAME_CHAT}"
        ))
        .bind(&t.kwant_id)
        .bind(t.channel_id.as_deref().unwrap_or(""))
        .bind(t.direct_kwant_id.as_deref().unwrap_or(""))
        .bind(t.thread_id.as_deref().unwrap_or(""))
        .fetch_optional(&mut *tx)
        .await?;
        if let Some((id, prompt, reason, retry_at)) = waiting {
            // A budget hold keeps every message, oldest first (G2: held, not dropped); a usage-limit
            // retry already waiting in that chat becomes a budget hold
            if t.reason == "bot_budget" {
                sqlx::query("UPDATE parked_turns SET prompt = ?, reason = 'bot_budget', retry_at = ? WHERE id = ?")
                    .bind(appended(&prompt, &t.prompt))
                    .bind(if reason == "bot_budget" { retry_at } else { retry_at.max(t.retry_at) })
                    .bind(&id)
                    .execute(&mut *tx)
                    .await?;
                tx.commit().await?;
            } else {
                tx.rollback().await?;
            }
            return Ok(None);
        }
        let row = sqlx::query_as::<_, ParkedTurn>(
            "INSERT INTO parked_turns (id, kwant_id, channel_id, direct_kwant_id, thread_id, delegation_id, depth, prompt, retry_at, status, created_at, reason) \
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, 'pending', ?, ?) RETURNING *",
        )
        .bind(Uuid::new_v4().to_string())
        .bind(&t.kwant_id)
        .bind(&t.channel_id)
        .bind(&t.direct_kwant_id)
        .bind(&t.thread_id)
        .bind(&t.delegation_id)
        .bind(t.depth)
        .bind(&t.prompt)
        .bind(t.retry_at)
        .bind(Utc::now())
        .bind(t.reason)
        .fetch_one(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(Some(row))
    }

    /// The parked turns due by `now`, each claimed (`running`) so it runs once.
    pub async fn take_due_parked(&self, now: DateTime<Utc>) -> Result<Vec<ParkedTurn>, DbError> {
        let due = sqlx::query_as::<_, ParkedTurn>("SELECT * FROM parked_turns WHERE status = 'pending' AND retry_at <= ? ORDER BY retry_at, created_at")
            .bind(now)
            .fetch_all(&self.pool)
            .await?;
        let mut claimed = Vec::new();
        for p in due {
            let res = sqlx::query("UPDATE parked_turns SET status = 'running' WHERE id = ? AND status = 'pending'")
                .bind(&p.id)
                .execute(&self.pool)
                .await?;
            if res.rows_affected() > 0 {
                claimed.push(ParkedTurn { status: "running".into(), ..p });
            }
        }
        Ok(claimed)
    }

    pub async fn next_parked_due(&self) -> Result<Option<DateTime<Utc>>, DbError> {
        let row: Option<(DateTime<Utc>,)> =
            sqlx::query_as("SELECT retry_at FROM parked_turns WHERE status = 'pending' ORDER BY retry_at LIMIT 1")
                .fetch_optional(&self.pool)
                .await?;
        Ok(row.map(|(at,)| at))
    }

    pub async fn set_parked_status(&self, id: &str, status: &str) -> Result<(), DbError> {
        sqlx::query("UPDATE parked_turns SET status = ? WHERE id = ?").bind(status).bind(id).execute(&self.pool).await?;
        Ok(())
    }

    /// A later turn in the same chat got through: the parked one would repeat it, so it's dropped.
    pub async fn drop_parked_for_chat(
        &self,
        kwant_id: &str,
        channel_id: Option<&str>,
        direct_kwant_id: Option<&str>,
        thread_id: Option<&str>,
    ) -> Result<u64, DbError> {
        let res = sqlx::query(&format!(
            "UPDATE parked_turns SET status = 'dropped' WHERE status = 'pending' AND delegation_id IS NULL AND {SAME_CHAT}"
        ))
        .bind(kwant_id)
        .bind(channel_id.unwrap_or(""))
        .bind(direct_kwant_id.unwrap_or(""))
        .bind(thread_id.unwrap_or(""))
        .execute(&self.pool)
        .await?;
        Ok(res.rows_affected())
    }

    pub async fn parked_for_delegation(&self, delegation_id: &str) -> Result<Option<ParkedTurn>, DbError> {
        Ok(sqlx::query_as::<_, ParkedTurn>("SELECT * FROM parked_turns WHERE delegation_id = ? AND status = 'pending' LIMIT 1")
            .bind(delegation_id)
            .fetch_optional(&self.pool)
            .await?)
    }

    pub async fn list_parked(&self) -> Result<Vec<ParkedTurn>, DbError> {
        Ok(sqlx::query_as::<_, ParkedTurn>("SELECT * FROM parked_turns ORDER BY created_at").fetch_all(&self.pool).await?)
    }
}
