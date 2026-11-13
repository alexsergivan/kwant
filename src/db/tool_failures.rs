//! 0.9.5: the chat lines kwnt posts when a bot's connection fails (`server::tool_failures`).
//! They are ordinary messages with `meta.kind = "tool_failure"`; this adds one index so "Needs
//! you" can find the open ones without scanning every message, and the reads it needs.

use super::{Database, DbError};
use crate::db::models::Message;

/// The meta kind of a failure line (`server::tool_failures::KIND`).
pub const KIND: &str = "tool_failure";
/// Most failure lines one read returns, newest first.
pub const MAX_LINES: i64 = 200;

impl Database {
    /// Additive: an index on the meta kind of messages kwnt writes itself. An old binary ignores it.
    pub(super) async fn migrate_tool_failures(&self) -> Result<(), DbError> {
        sqlx::query(
            "CREATE INDEX IF NOT EXISTS messages_by_meta_kind ON messages (json_extract(meta, '$.kind'), created_at) \
             WHERE meta IS NOT NULL AND json_valid(meta)",
        )
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// The newest failure lines (at most [`MAX_LINES`]), newest first.
    pub async fn list_tool_failure_lines(&self) -> Result<Vec<Message>, DbError> {
        Ok(sqlx::query_as::<_, Message>(
            "SELECT * FROM messages WHERE meta IS NOT NULL AND json_valid(meta) AND json_extract(meta, '$.kind') = ? \
             ORDER BY created_at DESC LIMIT ?",
        )
        .bind(KIND)
        .bind(MAX_LINES)
        .fetch_all(&self.pool)
        .await?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The read uses the index rather than scanning all messages.
    #[tokio::test]
    async fn failure_lines_use_the_meta_kind_index() {
        let tmp = tempfile::tempdir().unwrap();
        let db = Database::init(tmp.path()).await.unwrap();
        let plan: Vec<(i64, i64, i64, String)> = sqlx::query_as(
            "EXPLAIN QUERY PLAN SELECT * FROM messages WHERE meta IS NOT NULL AND json_valid(meta) AND json_extract(meta, '$.kind') = ? \
             ORDER BY created_at DESC LIMIT ?",
        )
        .bind(KIND)
        .bind(MAX_LINES)
        .fetch_all(&db.pool)
        .await
        .unwrap();
        let detail: Vec<&str> = plan.iter().map(|p| p.3.as_str()).collect();
        assert!(detail.iter().any(|d| d.contains("messages_by_meta_kind")), "index unused: {detail:?}");
    }
}
