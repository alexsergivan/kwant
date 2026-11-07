//! Connected coding agents (0.7.4, "Use kwant from your coding agent"): a person-made token lets
//! an outside agent (Claude Code, Codex, Cursor) ask this server's bots for work over
//! `/api/mcp/agents`. Only the token's sha256 is kept. Each connection owns the threads it opened
//! and the turns it started; nothing else is reachable with it.

use super::{Database, DbError};
use chrono::{DateTime, Duration, Utc};
use serde::Serialize;
use sqlx::FromRow;
use uuid::Uuid;

/// At most this many connections that aren't revoked.
pub const MAX_LIVE_CONNECTIONS: i64 = 10;
/// Asks a connection may make in any hour.
pub const ASKS_PER_HOUR: i64 = 60;
/// Turns a connection may have running at once.
pub const TURNS_IN_FLIGHT: i64 = 3;
/// Activity rows kept per connection; older ones are pruned as each new one is written.
pub const ACTIVITY_KEPT: i64 = 1000;

#[derive(Debug, Clone, Serialize, FromRow, PartialEq)]
pub struct AgentConnection {
    pub id: String,
    pub name: String,
    /// Never serialized: the token itself is never stored, and its hash stays in the database.
    #[serde(skip)]
    pub token_sha256: String,
    /// JSON array of space ids.
    pub space_ids: String,
    /// `"all"` or a JSON array of bot ids.
    pub bots: String,
    pub created_at: DateTime<Utc>,
    pub last_used_at: Option<DateTime<Utc>>,
    pub revoked_at: Option<DateTime<Utc>>,
}

impl AgentConnection {
    pub fn space_list(&self) -> Vec<String> {
        serde_json::from_str(&self.space_ids).unwrap_or_default()
    }

    /// `None` means every bot in the connection's spaces.
    pub fn bot_list(&self) -> Option<Vec<String>> {
        if self.bots == "all" {
            return None;
        }
        Some(serde_json::from_str(&self.bots).unwrap_or_default())
    }
}

/// One turn an agent asked for. `status` is "working", "done" or "failed".
#[derive(Debug, Clone, Serialize, FromRow, PartialEq)]
pub struct AgentTurn {
    pub id: String,
    pub connection_id: String,
    pub kwant_id: String,
    pub thread_id: String,
    pub message_id: Option<String>,
    pub status: String,
    pub reply_message_id: Option<String>,
    pub error: Option<String>,
    pub created_at: DateTime<Utc>,
    pub ended_at: Option<DateTime<Utc>>,
}

/// A thread a connection opened, newest activity first.
#[derive(Debug, Clone, Serialize, FromRow, PartialEq)]
pub struct AgentThreadRow {
    pub thread_id: String,
    pub kwant_id: String,
    pub standing: bool,
    pub title: String,
    pub updated_at: DateTime<Utc>,
}

/// One tools/call (or a refused one) in a connection's activity trail.
#[derive(Debug, Clone, Serialize, FromRow, PartialEq)]
pub struct AgentCall {
    pub connection_id: String,
    pub connection_name: String,
    pub tool: String,
    pub bot: Option<String>,
    /// "ok" or "refused".
    pub outcome: String,
    pub at: DateTime<Utc>,
}

/// Why a turn couldn't be reserved.
#[derive(Debug, Clone, PartialEq)]
pub enum TurnRefusal {
    /// 60 asks in the last hour: seconds until the oldest of them leaves the window.
    RateLimited { retry_after: i64 },
    /// 3 turns still working.
    InFlight,
}

impl Database {
    pub(super) async fn migrate_agent_connections(&self) -> Result<(), DbError> {
        for stmt in [
            "CREATE TABLE IF NOT EXISTS agent_connections (
                id TEXT PRIMARY KEY,
                name TEXT NOT NULL,
                token_sha256 TEXT NOT NULL UNIQUE,
                space_ids TEXT NOT NULL,
                bots TEXT NOT NULL,
                created_at TIMESTAMP NOT NULL,
                last_used_at TIMESTAMP,
                revoked_at TIMESTAMP
            )",
            "CREATE TABLE IF NOT EXISTS agent_threads (
                thread_id TEXT PRIMARY KEY,
                connection_id TEXT NOT NULL,
                kwant_id TEXT NOT NULL,
                standing INTEGER NOT NULL DEFAULT 0,
                created_at TIMESTAMP NOT NULL
            )",
            "CREATE UNIQUE INDEX IF NOT EXISTS agent_threads_standing ON agent_threads (connection_id, kwant_id) WHERE standing = 1",
            "CREATE TABLE IF NOT EXISTS agent_turns (
                id TEXT PRIMARY KEY,
                connection_id TEXT NOT NULL,
                kwant_id TEXT NOT NULL,
                thread_id TEXT NOT NULL,
                message_id TEXT,
                status TEXT NOT NULL CHECK (status IN ('working', 'done', 'failed')),
                reply_message_id TEXT,
                error TEXT,
                created_at TIMESTAMP NOT NULL,
                ended_at TIMESTAMP
            )",
            "CREATE INDEX IF NOT EXISTS agent_turns_by_connection ON agent_turns (connection_id, created_at)",
            "CREATE TABLE IF NOT EXISTS agent_connection_calls (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                connection_id TEXT NOT NULL,
                connection_name TEXT NOT NULL,
                tool TEXT NOT NULL,
                bot TEXT,
                outcome TEXT NOT NULL CHECK (outcome IN ('ok', 'refused')),
                at TIMESTAMP NOT NULL
            )",
            "CREATE INDEX IF NOT EXISTS agent_connection_calls_by_connection ON agent_connection_calls (connection_id, at)",
            // A revoked connection keeps its trail; it goes only with the connection's row
            "CREATE TRIGGER IF NOT EXISTS agent_connection_calls_go_with_connection AFTER DELETE ON agent_connections
             BEGIN DELETE FROM agent_connection_calls WHERE connection_id = OLD.id; END",
        ] {
            sqlx::query(stmt).execute(&self.pool).await?;
        }
        // A turn still "working" from before a restart never finishes
        sqlx::query("UPDATE agent_turns SET status = 'failed', error = 'kwant restarted before the reply', ended_at = ? WHERE status = 'working'")
            .bind(Utc::now())
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    /// Adds a connection; a `Conflict` when 10 are already live.
    pub async fn create_agent_connection(
        &self,
        name: &str,
        token_sha256: &str,
        space_ids: &[String],
        bots: Option<&[String]>,
    ) -> Result<AgentConnection, DbError> {
        let mut tx = self.begin_write().await?;
        let (live,): (i64,) = sqlx::query_as("SELECT COUNT(*) FROM agent_connections WHERE revoked_at IS NULL")
            .fetch_one(&mut *tx)
            .await?;
        if live >= MAX_LIVE_CONNECTIONS {
            return Err(DbError::Conflict(format!(
                "You already have {MAX_LIVE_CONNECTIONS} connected agents. Revoke one first."
            )));
        }
        let c = AgentConnection {
            id: format!("agc_{}", Uuid::new_v4().simple()),
            name: name.to_string(),
            token_sha256: token_sha256.to_string(),
            space_ids: serde_json::to_string(space_ids)?,
            bots: match bots {
                None => "all".into(),
                Some(ids) => serde_json::to_string(ids)?,
            },
            created_at: Utc::now(),
            last_used_at: None,
            revoked_at: None,
        };
        sqlx::query(
            "INSERT INTO agent_connections (id, name, token_sha256, space_ids, bots, created_at) VALUES (?, ?, ?, ?, ?, ?)",
        )
        .bind(&c.id)
        .bind(&c.name)
        .bind(&c.token_sha256)
        .bind(&c.space_ids)
        .bind(&c.bots)
        .bind(c.created_at)
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(c)
    }

    /// Live connections, newest first.
    pub async fn list_agent_connections(&self) -> Result<Vec<AgentConnection>, DbError> {
        Ok(sqlx::query_as::<_, AgentConnection>(
            "SELECT * FROM agent_connections WHERE revoked_at IS NULL ORDER BY created_at DESC",
        )
        .fetch_all(&self.pool)
        .await?)
    }

    pub async fn get_agent_connection(&self, id: &str) -> Result<Option<AgentConnection>, DbError> {
        Ok(sqlx::query_as::<_, AgentConnection>("SELECT * FROM agent_connections WHERE id = ?")
            .bind(id)
            .fetch_optional(&self.pool)
            .await?)
    }

    /// The live connection whose token hashes to `token_sha256`.
    pub async fn agent_connection_by_hash(&self, token_sha256: &str) -> Result<Option<AgentConnection>, DbError> {
        Ok(sqlx::query_as::<_, AgentConnection>(
            "SELECT * FROM agent_connections WHERE token_sha256 = ? AND revoked_at IS NULL",
        )
        .bind(token_sha256)
        .fetch_optional(&self.pool)
        .await?)
    }

    /// Revokes at once; false when there is no live connection with that id.
    pub async fn revoke_agent_connection(&self, id: &str) -> Result<bool, DbError> {
        let done = sqlx::query("UPDATE agent_connections SET revoked_at = ? WHERE id = ? AND revoked_at IS NULL")
            .bind(Utc::now())
            .bind(id)
            .execute(&self.pool)
            .await?;
        Ok(done.rows_affected() > 0)
    }

    pub async fn touch_agent_connection(&self, id: &str) -> Result<(), DbError> {
        sqlx::query("UPDATE agent_connections SET last_used_at = ? WHERE id = ?")
            .bind(Utc::now())
            .bind(id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    /// Adds one row to the connection's activity trail and, in the same write, drops its rows
    /// past the newest [`ACTIVITY_KEPT`].
    pub async fn log_agent_call(
        &self,
        c: &AgentConnection,
        tool: &str,
        bot: Option<&str>,
        ok: bool,
    ) -> Result<(), DbError> {
        let mut tx = self.begin_write().await?;
        sqlx::query(
            "INSERT INTO agent_connection_calls (connection_id, connection_name, tool, bot, outcome, at) VALUES (?, ?, ?, ?, ?, ?)",
        )
        .bind(&c.id)
        .bind(&c.name)
        .bind(tool)
        .bind(bot)
        .bind(if ok { "ok" } else { "refused" })
        .bind(Utc::now())
        .execute(&mut *tx)
        .await?;
        sqlx::query(
            "DELETE FROM agent_connection_calls WHERE connection_id = ?1 AND id <=
               (SELECT id FROM agent_connection_calls WHERE connection_id = ?1 ORDER BY id DESC LIMIT 1 OFFSET ?2)",
        )
        .bind(&c.id)
        .bind(ACTIVITY_KEPT)
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(())
    }

    /// A connection's activity trail, newest first.
    pub async fn list_agent_calls(&self, connection_id: &str, limit: i64) -> Result<Vec<AgentCall>, DbError> {
        Ok(sqlx::query_as::<_, AgentCall>(
            "SELECT connection_id, connection_name, tool, bot, outcome, at FROM agent_connection_calls
             WHERE connection_id = ? ORDER BY id DESC LIMIT ?",
        )
        .bind(connection_id)
        .bind(limit)
        .fetch_all(&self.pool)
        .await?)
    }

    /// The connection's standing thread with this bot, if it has one.
    pub async fn standing_agent_thread(&self, connection_id: &str, kwant_id: &str) -> Result<Option<String>, DbError> {
        let row: Option<(String,)> = sqlx::query_as(
            "SELECT a.thread_id FROM agent_threads a JOIN threads t ON t.id = a.thread_id
             WHERE a.connection_id = ? AND a.kwant_id = ? AND a.standing = 1 AND t.archived = 0",
        )
        .bind(connection_id)
        .bind(kwant_id)
        .fetch_optional(&self.pool)
        .await?;
        Ok(row.map(|r| r.0))
    }

    /// Records that `connection_id` opened `thread_id`. A standing thread replaces an earlier
    /// standing one with the same bot (it was archived).
    pub async fn add_agent_thread(
        &self,
        thread_id: &str,
        connection_id: &str,
        kwant_id: &str,
        standing: bool,
    ) -> Result<(), DbError> {
        let mut tx = self.begin_write().await?;
        if standing {
            sqlx::query("UPDATE agent_threads SET standing = 0 WHERE connection_id = ? AND kwant_id = ? AND standing = 1")
                .bind(connection_id)
                .bind(kwant_id)
                .execute(&mut *tx)
                .await?;
        }
        sqlx::query(
            "INSERT INTO agent_threads (thread_id, connection_id, kwant_id, standing, created_at) VALUES (?, ?, ?, ?, ?)",
        )
        .bind(thread_id)
        .bind(connection_id)
        .bind(kwant_id)
        .bind(standing)
        .bind(Utc::now())
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(())
    }

    /// The connection that opened `thread_id`, if an agent did.
    pub async fn agent_thread_owner(&self, thread_id: &str) -> Result<Option<String>, DbError> {
        let row: Option<(String,)> = sqlx::query_as("SELECT connection_id FROM agent_threads WHERE thread_id = ?")
            .bind(thread_id)
            .fetch_optional(&self.pool)
            .await?;
        Ok(row.map(|r| r.0))
    }

    /// The threads this connection opened, newest activity first.
    pub async fn list_agent_threads(
        &self,
        connection_id: &str,
        kwant_id: Option<&str>,
        limit: i64,
    ) -> Result<Vec<AgentThreadRow>, DbError> {
        Ok(sqlx::query_as::<_, AgentThreadRow>(
            "SELECT a.thread_id, a.kwant_id, a.standing, t.title, t.updated_at
             FROM agent_threads a JOIN threads t ON t.id = a.thread_id
             WHERE a.connection_id = ? AND (? IS NULL OR a.kwant_id = ?)
             ORDER BY t.updated_at DESC LIMIT ?",
        )
        .bind(connection_id)
        .bind(kwant_id)
        .bind(kwant_id)
        .bind(limit)
        .fetch_all(&self.pool)
        .await?)
    }

    /// Reserves a turn for the connection under both limits, in one write: 60 asks in any hour,
    /// 3 still working.
    pub async fn reserve_agent_turn(
        &self,
        connection_id: &str,
        kwant_id: &str,
        thread_id: &str,
    ) -> Result<Result<AgentTurn, TurnRefusal>, DbError> {
        let mut tx = self.begin_write().await?;
        let now = Utc::now();
        let hour_ago = now - Duration::hours(1);
        let (asks, oldest): (i64, Option<DateTime<Utc>>) = sqlx::query_as(
            "SELECT COUNT(*), MIN(created_at) FROM agent_turns WHERE connection_id = ? AND created_at > ?",
        )
        .bind(connection_id)
        .bind(hour_ago)
        .fetch_one(&mut *tx)
        .await?;
        if asks >= ASKS_PER_HOUR {
            let retry_after = oldest
                .map(|o| (o + Duration::hours(1) - now).num_seconds() + 1)
                .unwrap_or(60)
                .clamp(1, 3600);
            return Ok(Err(TurnRefusal::RateLimited { retry_after }));
        }
        let (working,): (i64,) =
            sqlx::query_as("SELECT COUNT(*) FROM agent_turns WHERE connection_id = ? AND status = 'working'")
                .bind(connection_id)
                .fetch_one(&mut *tx)
                .await?;
        if working >= TURNS_IN_FLIGHT {
            return Ok(Err(TurnRefusal::InFlight));
        }
        let t = AgentTurn {
            id: format!("agt_{}", Uuid::new_v4().simple()),
            connection_id: connection_id.to_string(),
            kwant_id: kwant_id.to_string(),
            thread_id: thread_id.to_string(),
            message_id: None,
            status: "working".into(),
            reply_message_id: None,
            error: None,
            created_at: now,
            ended_at: None,
        };
        sqlx::query(
            "INSERT INTO agent_turns (id, connection_id, kwant_id, thread_id, status, created_at) VALUES (?, ?, ?, ?, 'working', ?)",
        )
        .bind(&t.id)
        .bind(&t.connection_id)
        .bind(&t.kwant_id)
        .bind(&t.thread_id)
        .bind(t.created_at)
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(Ok(t))
    }

    pub async fn set_agent_turn_message(&self, id: &str, message_id: &str) -> Result<(), DbError> {
        sqlx::query("UPDATE agent_turns SET message_id = ? WHERE id = ?")
            .bind(message_id)
            .bind(id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    /// Ends a working turn as "done" or "failed".
    pub async fn finish_agent_turn(
        &self,
        id: &str,
        status: &str,
        reply_message_id: Option<&str>,
        error: Option<&str>,
    ) -> Result<(), DbError> {
        sqlx::query(
            "UPDATE agent_turns SET status = ?, reply_message_id = ?, error = ?, ended_at = ? WHERE id = ? AND status = 'working'",
        )
        .bind(status)
        .bind(reply_message_id)
        .bind(error)
        .bind(Utc::now())
        .bind(id)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn get_agent_turn(&self, id: &str) -> Result<Option<AgentTurn>, DbError> {
        Ok(sqlx::query_as::<_, AgentTurn>("SELECT * FROM agent_turns WHERE id = ?")
            .bind(id)
            .fetch_optional(&self.pool)
            .await?)
    }

    /// The bot's reply to the agent turn `agent_turn_id`: the reply saved with that id in its
    /// meta, never just the newest bot message (the person may be talking in the same thread).
    pub async fn agent_turn_reply_id(&self, thread_id: &str, agent_turn_id: &str) -> Result<Option<String>, DbError> {
        let row: Option<(String,)> = sqlx::query_as(
            "SELECT id FROM messages WHERE thread_id = ? AND sender_role = 'kwant'
               AND CASE WHEN json_valid(meta) THEN json_extract(meta, '$.agent_turn_id') END = ?
             ORDER BY rowid DESC LIMIT 1",
        )
        .bind(thread_id)
        .bind(agent_turn_id)
        .fetch_optional(&self.pool)
        .await?;
        Ok(row.map(|r| r.0))
    }

    /// The last bot reply in a thread (for `list_threads`).
    pub async fn last_kwant_text_in_thread(&self, thread_id: &str) -> Result<Option<String>, DbError> {
        let row: Option<(String,)> = sqlx::query_as(
            "SELECT text FROM messages WHERE thread_id = ? AND sender_role = 'kwant' ORDER BY rowid DESC LIMIT 1",
        )
        .bind(thread_id)
        .fetch_optional(&self.pool)
        .await?;
        Ok(row.map(|r| r.0))
    }

    /// What the person has to answer for the agent's own turn: an open question card or an
    /// approval card still waiting that one of `turn_ids` (the bot turns the ask started) raised,
    /// as one line with no parameters. Cards from the person's own turns in the thread never show.
    pub async fn agent_turn_waiting_on(&self, thread_id: &str, turn_ids: &[String]) -> Result<Option<String>, DbError> {
        if turn_ids.is_empty() {
            return Ok(None);
        }
        let ids = serde_json::to_string(turn_ids)?;
        let approval: Option<(String,)> = sqlx::query_as(
            "SELECT tool_name FROM approvals
             WHERE status IN ('pending', 'waiting') AND turn_id IN (SELECT value FROM json_each(?))
             ORDER BY created_at DESC LIMIT 1",
        )
        .bind(&ids)
        .fetch_optional(&self.pool)
        .await?;
        if let Some((tool,)) = approval {
            return Ok(Some(format!("Waiting for the person to approve a step ({}).", one_line(&tool, 80))));
        }
        let question: Option<(String,)> = sqlx::query_as(
            "SELECT question FROM questions WHERE thread_id = ? AND status = 'open'
               AND turn_id IN (SELECT value FROM json_each(?))
             ORDER BY created_at DESC LIMIT 1",
        )
        .bind(thread_id)
        .bind(&ids)
        .fetch_optional(&self.pool)
        .await?;
        Ok(question.map(|(q,)| format!("The bot asked the person: {}", one_line(&q, 200))))
    }

    /// Attachments the bot shared in `thread_id` between `since` and `until` (the turn's end).
    pub async fn agent_turn_attachments(
        &self,
        thread_id: &str,
        since: DateTime<Utc>,
        until: Option<DateTime<Utc>>,
    ) -> Result<Vec<super::attachments::Attachment>, DbError> {
        Ok(sqlx::query_as::<_, super::attachments::Attachment>(
            "SELECT * FROM attachments WHERE thread_id = ? AND source = 'agent' AND created_at >= ?
               AND (? IS NULL OR created_at <= ?) ORDER BY created_at",
        )
        .bind(thread_id)
        .bind(since)
        .bind(until)
        .bind(until)
        .fetch_all(&self.pool)
        .await?)
    }
}

/// `s` on one line, at most `max` characters.
fn one_line(s: &str, max: usize) -> String {
    let flat: String = s.chars().map(|c| if c.is_control() { ' ' } else { c }).collect();
    let flat = flat.split_whitespace().collect::<Vec<_>>().join(" ");
    if flat.chars().count() <= max {
        flat
    } else {
        flat.chars().take(max.saturating_sub(1)).collect::<String>() + "…"
    }
}
