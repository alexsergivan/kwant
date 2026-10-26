//! Questions an agent asked the user mid-turn (`ask_user`): a question, 2–5 choices, and always a
//! free-text answer. A question is `open` until answered, dismissed or its turn is stopped.

use super::models::{Message, MessageExtra, NewMessage};
use super::{Database, DbError};
use chrono::{DateTime, Utc};
use serde::Serialize;
use sqlx::FromRow;
use uuid::Uuid;

pub const MAX_QUESTION_CHARS: usize = 500;
pub const MAX_CHOICE_CHARS: usize = 80;
pub const MIN_CHOICES: usize = 2;
pub const MAX_CHOICES: usize = 5;
pub const MAX_ANSWER_CHARS: usize = 2000;
/// Most open questions per agent; an agent can't flood the user.
pub const MAX_OPEN_PER_KWANT: i64 = 10;

/// "open", "answered" or "cancelled" (dismissed by the user or its turn was stopped).
pub const QUESTION_STATUSES: [&str; 3] = ["open", "answered", "cancelled"];

#[derive(Debug, Clone, Serialize, FromRow, PartialEq)]
pub struct Question {
    pub id: String,
    pub kwant_id: String,
    pub channel_id: Option<String>,
    pub direct_kwant_id: Option<String>,
    pub thread_id: Option<String>,
    /// The turn that asked; it may have ended since (a late answer starts a new turn).
    pub turn_id: Option<String>,
    pub question: String,
    /// JSON array of strings.
    #[serde(serialize_with = "as_json")]
    pub choices: String,
    pub status: String,
    pub answer: Option<String>,
    /// The chat message that shows the question.
    pub message_id: Option<String>,
    pub created_at: DateTime<Utc>,
    pub answered_at: Option<DateTime<Utc>>,
    /// Why the bot couldn't decide this itself (see `crate::server::questions::WHY_YOU`); `None` on
    /// older questions.
    #[sqlx(default)]
    pub why_you: Option<String>,
    /// What the bot checked before asking.
    #[sqlx(default)]
    pub checked: Option<String>,
}

fn as_json<S: serde::Serializer>(raw: &str, s: S) -> Result<S::Ok, S::Error> {
    let v: serde_json::Value = serde_json::from_str(raw).unwrap_or(serde_json::Value::Array(Vec::new()));
    v.serialize(s)
}

impl Question {
    pub fn choice_list(&self) -> Vec<String> {
        serde_json::from_str(&self.choices).unwrap_or_default()
    }

    /// The chat id the question was asked in: channel, thread or the direct chat's kwant.
    pub fn context_id(&self) -> String {
        self.channel_id
            .clone()
            .or_else(|| self.thread_id.clone())
            .or_else(|| self.direct_kwant_id.clone())
            .unwrap_or_else(|| self.kwant_id.clone())
    }
}

#[derive(Debug, thiserror::Error, PartialEq)]
pub enum QuestionError {
    #[error("{0}")]
    Invalid(String),
    #[error("you already have {MAX_OPEN_PER_KWANT} unanswered questions; wait for answers first")]
    TooManyOpen,
    #[error("database error")]
    Db,
}

impl From<DbError> for QuestionError {
    fn from(_: DbError) -> Self {
        QuestionError::Db
    }
}

impl From<sqlx::Error> for QuestionError {
    fn from(_: sqlx::Error) -> Self {
        QuestionError::Db
    }
}

/// A question and its choices, checked and trimmed.
pub fn clean_question(question: &str, choices: &[String]) -> Result<(String, Vec<String>), QuestionError> {
    let q = question.trim();
    if q.is_empty() {
        return Err(QuestionError::Invalid("question is required".into()));
    }
    if q.chars().count() > MAX_QUESTION_CHARS {
        return Err(QuestionError::Invalid(format!("question is longer than {MAX_QUESTION_CHARS} characters")));
    }
    let mut out: Vec<String> = Vec::new();
    for c in choices {
        let c = c.split_whitespace().collect::<Vec<_>>().join(" ");
        if c.is_empty() {
            return Err(QuestionError::Invalid("choices can't be empty".into()));
        }
        if c.chars().count() > MAX_CHOICE_CHARS {
            return Err(QuestionError::Invalid(format!("a choice is longer than {MAX_CHOICE_CHARS} characters")));
        }
        if out.iter().any(|o| o.eq_ignore_ascii_case(&c)) {
            return Err(QuestionError::Invalid("choices must differ".into()));
        }
        out.push(c);
    }
    if !(MIN_CHOICES..=MAX_CHOICES).contains(&out.len()) {
        return Err(QuestionError::Invalid(format!("give {MIN_CHOICES} to {MAX_CHOICES} choices")));
    }
    Ok((q.to_string(), out))
}

/// Where a question is asked.
#[derive(Debug, Clone, Default)]
pub struct QuestionPlace {
    pub kwant_id: String,
    pub channel_id: Option<String>,
    pub direct_kwant_id: Option<String>,
    pub thread_id: Option<String>,
    pub turn_id: Option<String>,
}

impl Database {
    pub(super) async fn migrate_questions(&self) -> Result<(), DbError> {
        for stmt in [
            "CREATE TABLE IF NOT EXISTS questions (
                id TEXT PRIMARY KEY,
                kwant_id TEXT NOT NULL,
                channel_id TEXT,
                direct_kwant_id TEXT,
                thread_id TEXT,
                turn_id TEXT,
                question TEXT NOT NULL,
                choices TEXT NOT NULL,
                status TEXT NOT NULL CHECK (status IN ('open', 'answered', 'cancelled')),
                answer TEXT,
                message_id TEXT,
                created_at TIMESTAMP NOT NULL,
                answered_at TIMESTAMP
            )",
            "CREATE INDEX IF NOT EXISTS questions_by_status ON questions (status, kwant_id)",
        ] {
            sqlx::query(stmt).execute(&self.pool).await?;
        }
        // Added later; fails harmlessly once the columns exist
        for col in ["why_you", "checked"] {
            let _ = sqlx::query(&format!("ALTER TABLE questions ADD COLUMN {col} TEXT")).execute(&self.pool).await;
        }
        Ok(())
    }

    /// Saves why the bot asked and what it checked first.
    pub async fn set_question_reason(&self, id: &str, why_you: Option<&str>, checked: Option<&str>) -> Result<(), DbError> {
        sqlx::query("UPDATE questions SET why_you = ?, checked = ? WHERE id = ?")
            .bind(why_you)
            .bind(checked)
            .bind(id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    pub async fn create_question(
        &self,
        place: &QuestionPlace,
        question: &str,
        choices: &[String],
    ) -> Result<Question, QuestionError> {
        let (q, _) = self.create_question_with_card(place, question, choices, |_| None).await?;
        Ok(q)
    }

    /// Saves a question together with the chat message that shows it (built by `card` from the
    /// new question), in one transaction: nobody can see the question before its card exists.
    pub async fn create_question_with_card(
        &self,
        place: &QuestionPlace,
        question: &str,
        choices: &[String],
        card: impl FnOnce(&Question) -> Option<(NewMessage, MessageExtra)>,
    ) -> Result<(Question, Option<Message>), QuestionError> {
        // The first statement writes, so the transaction takes SQLite's write lock up front
        // (waiting on busy_timeout) instead of failing to upgrade a read lock under load
        let mut tx = self.pool.begin().await?;
        let id = Uuid::new_v4().to_string();
        let inserted = sqlx::query(
            "INSERT INTO questions (id, kwant_id, channel_id, direct_kwant_id, thread_id, turn_id, question, choices, status, created_at)
             SELECT ?, ?, ?, ?, ?, ?, ?, ?, 'open', ?
             WHERE (SELECT COUNT(*) FROM questions WHERE kwant_id = ? AND status = 'open') < ?",
        )
        .bind(&id)
        .bind(&place.kwant_id)
        .bind(&place.channel_id)
        .bind(&place.direct_kwant_id)
        .bind(&place.thread_id)
        .bind(&place.turn_id)
        .bind(question)
        .bind(serde_json::to_string(choices).unwrap_or_else(|_| "[]".into()))
        .bind(Utc::now())
        .bind(&place.kwant_id)
        .bind(MAX_OPEN_PER_KWANT)
        .execute(&mut *tx)
        .await?;
        if inserted.rows_affected() == 0 {
            return Err(QuestionError::TooManyOpen);
        }
        let get = "SELECT * FROM questions WHERE id = ?";
        let mut q = sqlx::query_as::<_, Question>(get).bind(&id).fetch_one(&mut *tx).await?;
        let mut message = None;
        if let Some((msg, extra)) = card(&q) {
            let saved = super::insert_message_on(&mut tx, &msg, &extra).await?;
            sqlx::query("UPDATE questions SET message_id = ? WHERE id = ?").bind(&saved.id).bind(&id).execute(&mut *tx).await?;
            q = sqlx::query_as::<_, Question>(get).bind(&id).fetch_one(&mut *tx).await?;
            message = Some(saved);
        }
        tx.commit().await?;
        Ok((q, message))
    }

    pub async fn get_question(&self, id: &str) -> Result<Option<Question>, DbError> {
        Ok(sqlx::query_as::<_, Question>("SELECT * FROM questions WHERE id = ?")
            .bind(id)
            .fetch_optional(&self.pool)
            .await?)
    }

    /// Questions, newest first: by status and/or chat (channel, thread or direct-chat kwant id).
    /// Newest first, at most 100; `space` keeps the questions of that space's kwants (all when `None`).
    pub async fn list_questions(
        &self,
        status: Option<&str>,
        context_id: Option<&str>,
        space: Option<&super::spaces::SpaceId>,
    ) -> Result<Vec<Question>, DbError> {
        Ok(sqlx::query_as::<_, Question>(
            "SELECT * FROM questions
             WHERE (?1 IS NULL OR status = ?1)
               AND (?2 IS NULL OR channel_id = ?2 OR thread_id = ?2 OR direct_kwant_id = ?2)
               AND (?3 IS NULL OR COALESCE((SELECT k.space_id FROM kwants k WHERE k.id = questions.kwant_id), 'spc_default') = ?3)
             ORDER BY created_at DESC LIMIT 100",
        )
        .bind(status)
        .bind(context_id)
        .bind(space.map(|s| s.as_str()))
        .fetch_all(&self.pool)
        .await?)
    }

    /// Closes an open question; `false` when it wasn't open any more (someone else closed it).
    pub async fn close_question(&self, id: &str, status: &str, answer: Option<&str>) -> Result<bool, DbError> {
        let done = sqlx::query(
            "UPDATE questions SET status = ?, answer = ?, answered_at = ? WHERE id = ? AND status = 'open'",
        )
        .bind(status)
        .bind(answer)
        .bind(Utc::now())
        .bind(id)
        .execute(&self.pool)
        .await?;
        Ok(done.rows_affected() == 1)
    }

    /// Replaces a message's `meta` JSON (a question card showing its new status).
    pub async fn set_message_meta(&self, message_id: &str, meta: &serde_json::Value) -> Result<(), DbError> {
        sqlx::query("UPDATE messages SET meta = ? WHERE id = ?")
            .bind(meta.to_string())
            .bind(message_id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(v: &[&str]) -> Vec<String> {
        v.iter().map(|x| x.to_string()).collect()
    }

    #[test]
    fn questions_are_checked() {
        assert!(clean_question("Which?", &s(&["a", "b"])).is_ok());
        assert!(clean_question(" ", &s(&["a", "b"])).is_err());
        assert!(clean_question("Which?", &s(&["a"])).is_err());
        assert!(clean_question("Which?", &s(&["a", "b", "c", "d", "e", "f"])).is_err());
        assert!(clean_question("Which?", &s(&["a", "A"])).is_err());
        assert!(clean_question("Which?", &s(&["a", ""])).is_err());
        assert!(clean_question(&"q".repeat(501), &s(&["a", "b"])).is_err());
        assert!(clean_question("Which?", &s(&["a", &"b".repeat(81)])).is_err());
        let (_, c) = clean_question("Which?", &s(&["  one\n two ", "b"])).unwrap();
        assert_eq!(c[0], "one two");
    }

    #[tokio::test]
    async fn closing_is_once_only() {
        let tmp = tempfile::tempdir().unwrap();
        let db = Database::init(tmp.path()).await.unwrap();
        let place = QuestionPlace { kwant_id: "k".into(), direct_kwant_id: Some("k".into()), ..Default::default() };
        let q = db.create_question(&place, "Which?", &s(&["a", "b"])).await.unwrap();
        assert_eq!(q.choice_list(), s(&["a", "b"]));
        assert_eq!(q.context_id(), "k");
        assert!(db.close_question(&q.id, "answered", Some("a")).await.unwrap());
        assert!(!db.close_question(&q.id, "cancelled", None).await.unwrap());
        assert_eq!(db.get_question(&q.id).await.unwrap().unwrap().answer.as_deref(), Some("a"));
        for _ in 0..MAX_OPEN_PER_KWANT {
            db.create_question(&place, "Which?", &s(&["a", "b"])).await.unwrap();
        }
        assert_eq!(db.create_question(&place, "Which?", &s(&["a", "b"])).await, Err(QuestionError::TooManyOpen));
    }
}
