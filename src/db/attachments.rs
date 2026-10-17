//! Files in chats: what the user attached to a message and what an agent shared. The bytes live
//! content-addressed under `<data dir>/attachments/<sha[0..2]>/<sha>`; a row names one use of them.

use super::{Database, DbError};
use chrono::{DateTime, Utc};
use serde::Serialize;
use serde_json::{json, Value};
use sqlx::FromRow;
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, FromRow, PartialEq)]
pub struct Attachment {
    pub id: String,
    pub sha256: String,
    /// Display name (sanitized: no path, no control characters).
    pub name: String,
    /// Sniffed from the bytes, never taken from the client.
    pub mime: String,
    pub size: i64,
    /// "user" (uploaded) or "agent" (shared by `kwant_id`).
    pub source: String,
    pub kwant_id: Option<String>,
    /// The chat it belongs to (channel, thread or direct-chat kwant id); null until a user
    /// upload is sent with a message.
    pub context_id: Option<String>,
    pub channel_id: Option<String>,
    pub direct_kwant_id: Option<String>,
    pub thread_id: Option<String>,
    pub message_id: Option<String>,
    pub created_at: DateTime<Utc>,
}

impl Attachment {
    /// What a message's `meta.attachments` holds: enough for a card, nothing else.
    pub fn card(&self) -> Value {
        json!({"id": self.id, "name": self.name, "mime": self.mime, "size": self.size})
    }
}

/// A new row: the stored blob and who/where it came from.
#[derive(Debug, Clone, Default)]
pub struct NewAttachment {
    pub sha256: String,
    pub name: String,
    pub mime: String,
    pub size: i64,
    pub source: String,
    pub kwant_id: Option<String>,
    pub context_id: Option<String>,
    pub channel_id: Option<String>,
    pub direct_kwant_id: Option<String>,
    pub thread_id: Option<String>,
}

impl Database {
    pub(super) async fn migrate_attachments(&self) -> Result<(), DbError> {
        for stmt in [
            "CREATE TABLE IF NOT EXISTS attachments (
                id TEXT PRIMARY KEY,
                sha256 TEXT NOT NULL,
                name TEXT NOT NULL,
                mime TEXT NOT NULL,
                size INTEGER NOT NULL,
                source TEXT NOT NULL CHECK (source IN ('user', 'agent')),
                kwant_id TEXT,
                context_id TEXT,
                channel_id TEXT,
                direct_kwant_id TEXT,
                thread_id TEXT,
                message_id TEXT,
                created_at TIMESTAMP NOT NULL
            )",
            "CREATE INDEX IF NOT EXISTS attachments_by_context ON attachments (context_id, created_at)",
        ] {
            sqlx::query(stmt).execute(&self.pool).await?;
        }
        Ok(())
    }

    pub async fn insert_attachment(&self, a: &NewAttachment) -> Result<Attachment, DbError> {
        let id = Uuid::new_v4().to_string();
        sqlx::query(
            "INSERT INTO attachments (id, sha256, name, mime, size, source, kwant_id, context_id, channel_id, direct_kwant_id, thread_id, created_at)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(&id)
        .bind(&a.sha256)
        .bind(&a.name)
        .bind(&a.mime)
        .bind(a.size)
        .bind(&a.source)
        .bind(&a.kwant_id)
        .bind(&a.context_id)
        .bind(&a.channel_id)
        .bind(&a.direct_kwant_id)
        .bind(&a.thread_id)
        .bind(Utc::now())
        .execute(&self.pool)
        .await?;
        Ok(self.get_attachment(&id).await?.expect("just inserted"))
    }

    pub async fn get_attachment(&self, id: &str) -> Result<Option<Attachment>, DbError> {
        Ok(sqlx::query_as::<_, Attachment>("SELECT * FROM attachments WHERE id = ?")
            .bind(id)
            .fetch_optional(&self.pool)
            .await?)
    }

    /// A chat's files, newest first.
    pub async fn list_attachments_in(&self, context_id: &str, limit: i64) -> Result<Vec<Attachment>, DbError> {
        Ok(sqlx::query_as::<_, Attachment>(
            "SELECT * FROM attachments WHERE context_id = ? ORDER BY created_at DESC LIMIT ?",
        )
        .bind(context_id)
        .bind(limit)
        .fetch_all(&self.pool)
        .await?)
    }

    /// Ties attachments to the message they were sent or shared with, and to its chat. Only rows
    /// not yet tied to a message change; returns how many did.
    pub async fn link_attachments(
        &self,
        ids: &[String],
        message_id: &str,
        context_id: &str,
        channel_id: Option<&str>,
        direct_kwant_id: Option<&str>,
        thread_id: Option<&str>,
    ) -> Result<u64, DbError> {
        let mut n = 0;
        for id in ids {
            n += sqlx::query(
                "UPDATE attachments SET message_id = ?, context_id = ?, channel_id = ?, direct_kwant_id = ?, thread_id = ?
                 WHERE id = ? AND message_id IS NULL",
            )
            .bind(message_id)
            .bind(context_id)
            .bind(channel_id)
            .bind(direct_kwant_id)
            .bind(thread_id)
            .bind(id)
            .execute(&self.pool)
            .await?
            .rows_affected();
        }
        Ok(n)
    }
}
