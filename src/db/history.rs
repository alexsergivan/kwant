//! Chat history beyond the latest page: a window centred on one message (a search hit
//! older than what's on screen) and the page before a message.
//!
//! Order is `(created_at, rowid)`, as in [`Database::list_messages`]; `created_at` has
//! one-second resolution, so `rowid` breaks ties.

use super::models::Message;
use super::{Database, DbError};

/// Most messages one history request returns.
pub const MAX_HISTORY_PAGE: i64 = 200;

/// Messages in a chat (channel, direct chat or thread) with the anchor's position as `a.t, a.r`.
/// The anchor must be in the same chat, so an id from another chat finds nothing.
const IN_CHAT_WITH_ANCHOR: &str = r#"
    WITH a AS (
        SELECT created_at AS t, rowid AS r FROM messages
        WHERE id = ?1 AND (channel_id = ?2 OR direct_kwant_id = ?2 OR thread_id = ?2)
    )
    SELECT m.*, m.rowid AS _seq FROM messages m, a
    WHERE (m.channel_id = ?2 OR m.direct_kwant_id = ?2 OR m.thread_id = ?2)
"#;

impl Database {
    /// Additive: indexes for loading a channel's or direct chat's messages (threads have
    /// `messages_by_thread`). An old binary just doesn't use them.
    pub(super) async fn migrate_history(&self) -> Result<(), DbError> {
        for stmt in [
            "CREATE INDEX IF NOT EXISTS messages_by_channel ON messages (channel_id, created_at) WHERE channel_id IS NOT NULL",
            "CREATE INDEX IF NOT EXISTS messages_by_direct ON messages (direct_kwant_id, created_at) WHERE direct_kwant_id IS NOT NULL",
        ] {
            sqlx::query(stmt).execute(&self.pool).await?;
        }
        Ok(())
    }

    /// Up to `limit` messages around `anchor_id`, oldest first: about half before it, the
    /// anchor, and the rest after. `None` when the anchor isn't in this chat.
    pub async fn list_messages_around(
        &self,
        chat_id: &str,
        anchor_id: &str,
        limit: i64,
    ) -> Result<Option<Vec<Message>>, DbError> {
        let limit = limit.clamp(1, MAX_HISTORY_PAGE);
        let before = limit / 2;
        // Anchor and newer
        let mut newer = sqlx::query_as::<_, Message>(&format!(
            "{IN_CHAT_WITH_ANCHOR} AND (m.created_at, m.rowid) >= (a.t, a.r) ORDER BY m.created_at ASC, m.rowid ASC LIMIT ?3"
        ))
        .bind(anchor_id)
        .bind(chat_id)
        .bind(limit - before)
        .fetch_all(&self.pool)
        .await?;
        if newer.is_empty() {
            return Ok(None);
        }
        let mut out = self.older_than(chat_id, anchor_id, before).await?;
        out.append(&mut newer);
        Ok(Some(out))
    }

    /// Up to `limit` messages just before `anchor_id`, oldest first. `None` when the anchor
    /// isn't in this chat.
    pub async fn list_messages_before(
        &self,
        chat_id: &str,
        anchor_id: &str,
        limit: i64,
    ) -> Result<Option<Vec<Message>>, DbError> {
        let found: Option<(i64,)> = sqlx::query_as(
            "SELECT 1 FROM messages WHERE id = ?1 AND (channel_id = ?2 OR direct_kwant_id = ?2 OR thread_id = ?2)",
        )
        .bind(anchor_id)
        .bind(chat_id)
        .fetch_optional(&self.pool)
        .await?;
        if found.is_none() {
            return Ok(None);
        }
        Ok(Some(self.older_than(chat_id, anchor_id, limit.clamp(1, MAX_HISTORY_PAGE)).await?))
    }

    async fn older_than(&self, chat_id: &str, anchor_id: &str, limit: i64) -> Result<Vec<Message>, DbError> {
        if limit <= 0 {
            return Ok(Vec::new());
        }
        let mut rows = sqlx::query_as::<_, Message>(&format!(
            "{IN_CHAT_WITH_ANCHOR} AND (m.created_at, m.rowid) < (a.t, a.r) ORDER BY m.created_at DESC, m.rowid DESC LIMIT ?3"
        ))
        .bind(anchor_id)
        .bind(chat_id)
        .bind(limit)
        .fetch_all(&self.pool)
        .await?;
        rows.reverse();
        Ok(rows)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Both directions use the per-chat indexes rather than scanning all messages.
    #[tokio::test]
    async fn history_queries_use_the_chat_indexes() {
        let tmp = tempfile::tempdir().unwrap();
        let db = Database::init(tmp.path()).await.unwrap();
        for cmp in [">=", "<"] {
            let sql = format!(
                "EXPLAIN QUERY PLAN {IN_CHAT_WITH_ANCHOR} AND (m.created_at, m.rowid) {cmp} (a.t, a.r) ORDER BY m.created_at, m.rowid LIMIT ?3"
            );
            let plan: Vec<(i64, i64, i64, String)> =
                sqlx::query_as(&sql).bind("x").bind("c").bind(10).fetch_all(&db.pool).await.unwrap();
            let detail: Vec<&str> = plan.iter().map(|p| p.3.as_str()).collect();
            assert!(
                !detail.iter().any(|d| d.starts_with("SCAN m") || d.starts_with("SCAN messages")),
                "full scan in {cmp} plan: {detail:?}"
            );
            for idx in ["messages_by_channel", "messages_by_direct", "messages_by_thread"] {
                assert!(detail.iter().any(|d| d.contains(idx)), "{idx} unused in {cmp} plan: {detail:?}");
            }
        }
    }
}
