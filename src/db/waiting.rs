//! What a bot is waiting on the person for: an open question (its card) or a reply that ended by
//! asking the person something. Each gets at most one reminder, `due_at` after the turn ended
//! (none when reminders are off). The person writing in that chat, or the question closing, ends it.

use super::{Database, DbError};
use chrono::{DateTime, Utc};
use serde::Serialize;
use sqlx::FromRow;
use uuid::Uuid;

/// "open" (still waiting), "reminded" (its one reminder went out, still waiting) or "closed".
#[derive(Debug, Clone, Serialize, FromRow, PartialEq)]
pub struct WaitingItem {
    pub id: String,
    pub kwant_id: String,
    pub channel_id: Option<String>,
    pub direct_kwant_id: Option<String>,
    pub thread_id: Option<String>,
    /// Set when the bot asked with a question card.
    pub question_id: Option<String>,
    /// The reply or card that asked.
    pub message_id: Option<String>,
    pub question: String,
    /// When the reminder goes out; `None` when reminders were off as the turn ended.
    pub due_at: Option<DateTime<Utc>>,
    pub status: String,
    pub created_at: DateTime<Utc>,
    pub reminded_at: Option<DateTime<Utc>>,
}

pub struct NewWaitingItem {
    pub kwant_id: String,
    pub channel_id: Option<String>,
    pub direct_kwant_id: Option<String>,
    pub thread_id: Option<String>,
    pub question_id: Option<String>,
    pub message_id: Option<String>,
    pub question: String,
    pub due_at: Option<DateTime<Utc>>,
}

/// The same bot in the same chat, matched with NULLs as "".
const SAME_CHAT: &str = "kwant_id = ? AND COALESCE(channel_id, '') = ? AND COALESCE(direct_kwant_id, '') = ? \
     AND COALESCE(thread_id, '') = ?";

impl Database {
    pub(super) async fn migrate_waiting(&self) -> Result<(), DbError> {
        for stmt in [
            "CREATE TABLE IF NOT EXISTS waiting_on_person (
                id TEXT PRIMARY KEY,
                kwant_id TEXT NOT NULL,
                channel_id TEXT,
                direct_kwant_id TEXT,
                thread_id TEXT,
                question_id TEXT,
                message_id TEXT,
                question TEXT NOT NULL,
                due_at TIMESTAMP,
                status TEXT NOT NULL CHECK (status IN ('open', 'reminded', 'closed')),
                created_at TIMESTAMP NOT NULL,
                reminded_at TIMESTAMP
            )",
            "CREATE INDEX IF NOT EXISTS waiting_on_person_due ON waiting_on_person (status, due_at)",
            // One entry per question, ever: a question is never reminded twice
            "CREATE UNIQUE INDEX IF NOT EXISTS waiting_on_person_question ON waiting_on_person (question_id) WHERE question_id IS NOT NULL",
        ] {
            sqlx::query(stmt).execute(&self.pool).await?;
        }
        Ok(())
    }

    /// Records what a bot waits for. `None` when it is already recorded: the same question, or
    /// for a plain ask, an earlier one of this bot in this chat still waiting (it keeps its time,
    /// so asking again doesn't bring a second reminder).
    pub async fn add_waiting(&self, w: &NewWaitingItem) -> Result<Option<WaitingItem>, DbError> {
        let mut tx = self.pool.begin().await?;
        let existing: Option<(String,)> = match &w.question_id {
            Some(q) => sqlx::query_as("SELECT id FROM waiting_on_person WHERE question_id = ?").bind(q).fetch_optional(&mut *tx).await?,
            None => {
                sqlx::query_as(&format!(
                    "SELECT id FROM waiting_on_person WHERE question_id IS NULL AND status != 'closed' AND {SAME_CHAT}"
                ))
                .bind(&w.kwant_id)
                .bind(w.channel_id.as_deref().unwrap_or(""))
                .bind(w.direct_kwant_id.as_deref().unwrap_or(""))
                .bind(w.thread_id.as_deref().unwrap_or(""))
                .fetch_optional(&mut *tx)
                .await?
            }
        };
        if existing.is_some() {
            return Ok(None);
        }
        let row = sqlx::query_as::<_, WaitingItem>(
            "INSERT INTO waiting_on_person (id, kwant_id, channel_id, direct_kwant_id, thread_id, question_id, message_id, question, due_at, status, created_at) \
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, 'open', ?) RETURNING *",
        )
        .bind(Uuid::new_v4().to_string())
        .bind(&w.kwant_id)
        .bind(&w.channel_id)
        .bind(&w.direct_kwant_id)
        .bind(&w.thread_id)
        .bind(&w.question_id)
        .bind(&w.message_id)
        .bind(&w.question)
        .bind(w.due_at)
        .bind(Utc::now())
        .fetch_one(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(Some(row))
    }

    pub async fn get_waiting(&self, id: &str) -> Result<Option<WaitingItem>, DbError> {
        Ok(sqlx::query_as::<_, WaitingItem>("SELECT * FROM waiting_on_person WHERE id = ?").bind(id).fetch_optional(&self.pool).await?)
    }

    /// Still waiting (open or reminded), oldest first; `space` keeps that space's bots.
    pub async fn list_waiting(&self, space: Option<&super::spaces::SpaceId>) -> Result<Vec<WaitingItem>, DbError> {
        Ok(sqlx::query_as::<_, WaitingItem>(
            "SELECT * FROM waiting_on_person WHERE status != 'closed'
               AND (?1 IS NULL OR COALESCE((SELECT k.space_id FROM kwants k WHERE k.id = waiting_on_person.kwant_id), 'spc_default') = ?1)
             ORDER BY created_at LIMIT 200",
        )
        .bind(space.map(|s| s.as_str()))
        .fetch_all(&self.pool)
        .await?)
    }

    /// The question was answered or dismissed.
    pub async fn close_waiting_for_question(&self, question_id: &str) -> Result<u64, DbError> {
        let res = sqlx::query("UPDATE waiting_on_person SET status = 'closed' WHERE question_id = ? AND status != 'closed'")
            .bind(question_id)
            .execute(&self.pool)
            .await?;
        Ok(res.rows_affected())
    }

    /// The person wrote in a chat: a thread (`thread_id`), a channel's main chat, or the direct
    /// chat with a bot (`direct_kwant_id`, no channel). Everything waiting there ends.
    pub async fn close_waiting_in_chat(
        &self,
        channel_id: Option<&str>,
        direct_kwant_id: Option<&str>,
        thread_id: Option<&str>,
    ) -> Result<u64, DbError> {
        let res = match (thread_id, channel_id, direct_kwant_id) {
            (Some(t), _, _) => {
                sqlx::query("UPDATE waiting_on_person SET status = 'closed' WHERE status != 'closed' AND thread_id = ?").bind(t).execute(&self.pool).await?
            }
            (None, Some(c), _) => sqlx::query(
                "UPDATE waiting_on_person SET status = 'closed' WHERE status != 'closed' AND channel_id = ? AND thread_id IS NULL",
            )
            .bind(c)
            .execute(&self.pool)
            .await?,
            (None, None, Some(k)) => sqlx::query(
                "UPDATE waiting_on_person SET status = 'closed' WHERE status != 'closed' AND channel_id IS NULL \
                 AND thread_id IS NULL AND direct_kwant_id = ?",
            )
            .bind(k)
            .execute(&self.pool)
            .await?,
            (None, None, None) => return Ok(0),
        };
        Ok(res.rows_affected())
    }

    /// The entries whose reminder is due by `now`, each claimed (`open` → `reminded`) so it goes
    /// out once, whoever else is looking.
    pub async fn take_due_waiting(&self, now: DateTime<Utc>) -> Result<Vec<WaitingItem>, DbError> {
        let due = sqlx::query_as::<_, WaitingItem>(
            "SELECT * FROM waiting_on_person WHERE status = 'open' AND due_at IS NOT NULL AND due_at <= ? ORDER BY due_at",
        )
        .bind(now)
        .fetch_all(&self.pool)
        .await?;
        let mut claimed = Vec::new();
        for w in due {
            let res = sqlx::query("UPDATE waiting_on_person SET status = 'reminded', reminded_at = ? WHERE id = ? AND status = 'open'")
                .bind(now)
                .bind(&w.id)
                .execute(&self.pool)
                .await?;
            if res.rows_affected() > 0 {
                claimed.push(WaitingItem { status: "reminded".into(), reminded_at: Some(now), ..w });
            }
        }
        Ok(claimed)
    }

    pub async fn next_waiting_due(&self) -> Result<Option<DateTime<Utc>>, DbError> {
        let row: Option<(DateTime<Utc>,)> = sqlx::query_as(
            "SELECT due_at FROM waiting_on_person WHERE status = 'open' AND due_at IS NOT NULL ORDER BY due_at LIMIT 1",
        )
        .fetch_optional(&self.pool)
        .await?;
        Ok(row.map(|(at,)| at))
    }

    /// Reminders were switched off: what is still waiting stays listed but is never reminded.
    pub async fn clear_waiting_due(&self) -> Result<u64, DbError> {
        let res = sqlx::query("UPDATE waiting_on_person SET due_at = NULL WHERE status = 'open' AND due_at IS NOT NULL")
            .execute(&self.pool)
            .await?;
        Ok(res.rows_affected())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Duration;

    fn item(kwant: &str, channel: Option<&str>, direct: Option<&str>, thread: Option<&str>, q: Option<&str>) -> NewWaitingItem {
        NewWaitingItem {
            kwant_id: kwant.into(),
            channel_id: channel.map(Into::into),
            direct_kwant_id: direct.map(Into::into),
            thread_id: thread.map(Into::into),
            question_id: q.map(Into::into),
            message_id: None,
            question: "Which one?".into(),
            due_at: Some(Utc::now() - Duration::seconds(1)),
        }
    }

    #[tokio::test]
    async fn one_entry_per_question_and_one_reminder() {
        let tmp = tempfile::tempdir().unwrap();
        let db = Database::init(tmp.path()).await.unwrap();
        assert!(db.add_waiting(&item("k", None, Some("k"), None, Some("q1"))).await.unwrap().is_some());
        assert!(db.add_waiting(&item("k", None, Some("k"), None, Some("q1"))).await.unwrap().is_none());
        // A plain ask in the same chat while one is waiting keeps the first
        assert!(db.add_waiting(&item("k", None, Some("k"), None, None)).await.unwrap().is_some());
        assert!(db.add_waiting(&item("k", None, Some("k"), None, None)).await.unwrap().is_none());
        assert_eq!(db.take_due_waiting(Utc::now()).await.unwrap().len(), 2);
        assert!(db.take_due_waiting(Utc::now()).await.unwrap().is_empty());
        assert_eq!(db.next_waiting_due().await.unwrap(), None);
        // Reminded entries are still listed until the person writes
        assert_eq!(db.list_waiting(None).await.unwrap().len(), 2);
        assert_eq!(db.close_waiting_in_chat(None, Some("k"), None).await.unwrap(), 2);
        assert!(db.list_waiting(None).await.unwrap().is_empty());
        // A closed question is never recorded again
        assert!(db.add_waiting(&item("k", None, Some("k"), None, Some("q1"))).await.unwrap().is_none());
    }

    #[tokio::test]
    async fn writing_closes_only_that_chat() {
        let tmp = tempfile::tempdir().unwrap();
        let db = Database::init(tmp.path()).await.unwrap();
        db.add_waiting(&item("a", Some("c1"), None, None, None)).await.unwrap();
        db.add_waiting(&item("b", Some("c1"), None, Some("t1"), None)).await.unwrap();
        db.add_waiting(&item("a", None, Some("a"), None, None)).await.unwrap();
        db.add_waiting(&item("b", Some("c2"), None, None, Some("q9"))).await.unwrap();
        assert_eq!(db.close_waiting_in_chat(Some("c1"), None, None).await.unwrap(), 1);
        assert_eq!(db.close_waiting_in_chat(None, None, Some("t1")).await.unwrap(), 1);
        assert_eq!(db.close_waiting_for_question("q9").await.unwrap(), 1);
        let left = db.list_waiting(None).await.unwrap();
        assert_eq!(left.len(), 1);
        assert_eq!(left[0].direct_kwant_id.as_deref(), Some("a"));
        assert_eq!(db.clear_waiting_due().await.unwrap(), 1);
        assert!(db.take_due_waiting(Utc::now()).await.unwrap().is_empty());
    }
}
