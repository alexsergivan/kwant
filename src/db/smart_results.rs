//! `message_results`: the Smart Results block set of a bot reply, beside the message and never in
//! its text or `meta` (see `crate::server::smart_results`).

use std::collections::HashMap;

use serde_json::Value;

use super::models::{Message, MessageExtra, NewMessage};
use super::{Database, DbError};
use crate::server::smart_results::{self as sr, ResultSet};

impl Database {
    /// Additive: one row per message that has a block set; deleting the message deletes it.
    pub(super) async fn migrate_smart_results(&self) -> Result<(), DbError> {
        sqlx::query(
            r#"
            CREATE TABLE IF NOT EXISTS message_results (
                message_id TEXT PRIMARY KEY REFERENCES messages(id) ON DELETE CASCADE,
                version INTEGER NOT NULL,
                payload TEXT NOT NULL,
                created_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP
            )
            "#,
        )
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// Sets `results` on the bot replies in `messages` that have a stored block set this build
    /// still serves. Never fails: a read error leaves them `None`, and the chat shows markdown.
    pub async fn attach_results(&self, messages: &mut [Message]) {
        let ids: Vec<&str> = messages.iter().filter(|m| m.sender_role == "kwant").map(|m| m.id.as_str()).collect();
        if ids.is_empty() {
            return;
        }
        let ids = serde_json::to_string(&ids).unwrap_or_default();
        let rows: Vec<(String, i64, String)> = match sqlx::query_as(
            "SELECT message_id, version, payload FROM message_results WHERE message_id IN (SELECT value FROM json_each(?))",
        )
        .bind(ids)
        .fetch_all(&self.pool)
        .await
        {
            Ok(rows) => rows,
            Err(e) => {
                tracing::warn!("smart results: couldn't read stored blocks: {e}");
                return;
            }
        };
        let found: HashMap<String, Value> =
            rows.into_iter().filter_map(|(id, version, payload)| read(version, &payload).map(|v| (id, v))).collect();
        for m in messages.iter_mut() {
            if let Some(v) = found.get(&m.id) {
                m.results = Some(v.clone());
            }
        }
    }
}

/// A stored payload this build can serve, or `None` (an unknown version, a type no longer
/// enabled, or a block that fails today's checks).
fn read(version: i64, payload: &str) -> Option<Value> {
    if version != i64::from(sr::VERSION) {
        return None;
    }
    let set: ResultSet = serde_json::from_str(payload).ok()?;
    sr::validate(&set, None).ok()?;
    serde_json::to_value(&set).ok()
}

/// Computes and stores the block set for a message just inserted on `conn` as `id`, and returns
/// it for the message object. Only ordinary bot replies qualify: `sender_role` "kwant" and no
/// `meta.kind` (cards and kwnt's own posts have one; a reply's `ran_on`, `usage` or `routing`
/// meta doesn't count). Never fails: a failed write is logged and the message stays as it was.
pub(super) async fn store_on(
    conn: &mut sqlx::SqliteConnection,
    input: &NewMessage,
    extra: &MessageExtra,
    id: &str,
) -> Option<Value> {
    let card = extra.meta.as_ref().is_some_and(|m| m.get("kind").is_some());
    if input.sender_role != "kwant" || card {
        return None;
    }
    let set = sr::compute(&input.text)?;
    let value = serde_json::to_value(&set).ok()?;
    let saved = sqlx::query("INSERT OR REPLACE INTO message_results (message_id, version, payload) VALUES (?, ?, ?)")
        .bind(id)
        .bind(i64::from(set.version))
        .bind(value.to_string())
        .execute(&mut *conn)
        .await;
    match saved {
        Ok(_) => Some(value),
        Err(e) => {
            tracing::warn!("smart results: couldn't store the block for message {id}: {e}");
            None
        }
    }
}
