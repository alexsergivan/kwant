//! "Always allow" answers: that bot may use that tool again in that one conversation (a thread,
//! else the channel, else the direct chat) without a card, also after kwnt restarts (25e). The
//! broker keeps them in memory as its fast path and writes each one through to this table; it
//! loads the table once when it starts.
//!
//! Kept in its own table, outside exports and imports (like `approval_modes`), so a bundle can't
//! bring grants with it. A row goes when its bot, thread or channel is deleted (triggers below),
//! or when the person takes it back (`DELETE /api/kwants/:id/always-allowed`).

use super::{Database, DbError};
use chrono::{DateTime, Utc};

const CREATE_ALWAYS_ALLOWED: &str = "CREATE TABLE IF NOT EXISTS always_allowed (
    kwant_id TEXT NOT NULL,
    conversation TEXT NOT NULL,
    tool_name TEXT NOT NULL,
    created_at TIMESTAMP NOT NULL,
    PRIMARY KEY (kwant_id, conversation, tool_name)
)";

/// A deleted bot, thread or channel takes its grants with it, whichever path deleted it. Made
/// again on every start, after every step that can rebuild those tables (and drop the triggers).
const TRIGGERS: [&str; 3] = [
    "CREATE TRIGGER IF NOT EXISTS always_allowed_bot_gone AFTER DELETE ON kwants BEGIN \
        DELETE FROM always_allowed WHERE kwant_id = OLD.id; END",
    "CREATE TRIGGER IF NOT EXISTS always_allowed_thread_gone AFTER DELETE ON threads BEGIN \
        DELETE FROM always_allowed WHERE conversation = OLD.id; END",
    "CREATE TRIGGER IF NOT EXISTS always_allowed_channel_gone AFTER DELETE ON channels BEGIN \
        DELETE FROM always_allowed WHERE conversation = OLD.id; END",
];

/// One grant as the bot's settings list it.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct AlwaysAllowed {
    pub tool_name: String,
    /// The thread id, else the channel id, else the bot's id for its direct chat.
    pub conversation: String,
    /// `thread`, `channel` or `direct`; `unknown` for a conversation that no longer exists.
    pub conversation_kind: &'static str,
    /// The thread's title, `#<channel>`, or `your chat`.
    pub conversation_label: String,
    pub created_at: DateTime<Utc>,
}

impl Database {
    pub(super) async fn migrate_always_allowed(&self) -> Result<(), DbError> {
        sqlx::query(CREATE_ALWAYS_ALLOWED).execute(&self.pool).await?;
        for sql in TRIGGERS {
            sqlx::query(sql).execute(&self.pool).await?;
        }
        Ok(())
    }

    /// Stores an "always allow"; a second one for the same bot, conversation and tool keeps the
    /// first one's time.
    pub async fn save_always_allowed(&self, kwant_id: &str, conversation: &str, tool_name: &str) -> Result<(), DbError> {
        sqlx::query("INSERT OR IGNORE INTO always_allowed (kwant_id, conversation, tool_name, created_at) VALUES (?, ?, ?, ?)")
            .bind(kwant_id)
            .bind(conversation)
            .bind(tool_name)
            .bind(Utc::now())
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    /// Every stored grant as (bot, conversation, tool), for the broker's start.
    pub async fn all_always_allowed(&self) -> Result<Vec<(String, String, String)>, DbError> {
        Ok(sqlx::query_as("SELECT kwant_id, conversation, tool_name FROM always_allowed").fetch_all(&self.pool).await?)
    }

    /// One bot's grants, newest first, with where each applies in words.
    pub async fn list_always_allowed(&self, kwant_id: &str) -> Result<Vec<AlwaysAllowed>, DbError> {
        type Row = (String, String, DateTime<Utc>, Option<String>, Option<String>);
        let rows: Vec<Row> = sqlx::query_as(
            "SELECT a.tool_name, a.conversation, a.created_at, t.title, c.name
             FROM always_allowed a
             LEFT JOIN threads t ON t.id = a.conversation
             LEFT JOIN channels c ON c.id = a.conversation
             WHERE a.kwant_id = ?
             ORDER BY a.created_at DESC, a.tool_name",
        )
        .bind(kwant_id)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows
            .into_iter()
            .map(|(tool_name, conversation, created_at, thread, channel)| {
                let (conversation_kind, conversation_label) = match (thread, channel) {
                    (Some(title), _) => ("thread", title),
                    (None, Some(name)) => ("channel", format!("#{name}")),
                    (None, None) if conversation == kwant_id => ("direct", "your chat".to_string()),
                    (None, None) => ("unknown", conversation.clone()),
                };
                AlwaysAllowed { tool_name, conversation, conversation_kind, conversation_label, created_at }
            })
            .collect())
    }

    /// Takes back one grant; true when there was one.
    pub async fn delete_always_allowed(&self, kwant_id: &str, conversation: &str, tool_name: &str) -> Result<bool, DbError> {
        let done = sqlx::query("DELETE FROM always_allowed WHERE kwant_id = ? AND conversation = ? AND tool_name = ?")
            .bind(kwant_id)
            .bind(conversation)
            .bind(tool_name)
            .execute(&self.pool)
            .await?;
        Ok(done.rows_affected() > 0)
    }

    /// Takes back all of a bot's grants; how many there were.
    pub async fn delete_all_always_allowed(&self, kwant_id: &str) -> Result<u64, DbError> {
        let done = sqlx::query("DELETE FROM always_allowed WHERE kwant_id = ?").bind(kwant_id).execute(&self.pool).await?;
        Ok(done.rows_affected())
    }

    /// Forgets the grants in one conversation (its thread or channel was deleted).
    pub async fn delete_always_allowed_in(&self, conversation: &str) -> Result<u64, DbError> {
        let done = sqlx::query("DELETE FROM always_allowed WHERE conversation = ?").bind(conversation).execute(&self.pool).await?;
        Ok(done.rows_affected())
    }
}
