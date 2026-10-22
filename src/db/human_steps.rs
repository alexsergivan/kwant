//! Steps only the person can do in the bot browser (`request_human_step`): sign in, solve a
//! CAPTCHA, or something else. A step is `open` until the person marks it done or declines it,
//! or its turn is stopped (`cancelled`).

use super::models::{Message, MessageExtra, NewMessage};
use super::{Database, DbError};
use chrono::{DateTime, Utc};
use serde::Serialize;
use sqlx::FromRow;
use uuid::Uuid;

pub const MAX_REASON_CHARS: usize = 300;
pub const MAX_URL_CHARS: usize = 300;
/// Most open steps per agent; an agent can't flood the person.
pub const MAX_OPEN_PER_KWANT: i64 = 5;

pub const HUMAN_STEP_KINDS: [&str; 4] = ["sign_in", "captcha", "two_factor", "other"];
/// "open", "done", "declined" or "cancelled" (its turn was stopped).
pub const HUMAN_STEP_STATUSES: [&str; 4] = ["open", "done", "declined", "cancelled"];

#[derive(Debug, Clone, Serialize, FromRow, PartialEq)]
pub struct HumanStep {
    pub id: String,
    pub kwant_id: String,
    pub channel_id: Option<String>,
    pub direct_kwant_id: Option<String>,
    pub thread_id: Option<String>,
    /// The turn that asked; it may have ended since (a late answer starts a new turn).
    pub turn_id: Option<String>,
    pub kind: String,
    pub reason: String,
    /// Where to do it: scheme, host and path only, never a query or a password.
    pub url: Option<String>,
    /// The bot browser it happens in (team, channel or kwant id).
    pub sandbox_target: Option<String>,
    /// "bot" (the agent asked) or "auto" (kwnt saw a CAPTCHA).
    pub source: String,
    pub status: String,
    /// The chat message that shows the step.
    pub message_id: Option<String>,
    pub created_at: DateTime<Utc>,
    pub closed_at: Option<DateTime<Utc>>,
}

impl HumanStep {
    /// The chat id the step was asked in: channel, thread or the direct chat's kwant.
    pub fn context_id(&self) -> String {
        self.channel_id
            .clone()
            .or_else(|| self.thread_id.clone())
            .or_else(|| self.direct_kwant_id.clone())
            .unwrap_or_else(|| self.kwant_id.clone())
    }
}

#[derive(Debug, thiserror::Error, PartialEq)]
pub enum HumanStepError {
    #[error("{0}")]
    Invalid(String),
    #[error("you already have {MAX_OPEN_PER_KWANT} steps waiting on the person; wait for them first")]
    TooManyOpen,
    #[error("database error")]
    Db,
}

impl From<DbError> for HumanStepError {
    fn from(_: DbError) -> Self {
        HumanStepError::Db
    }
}

impl From<sqlx::Error> for HumanStepError {
    fn from(_: sqlx::Error) -> Self {
        HumanStepError::Db
    }
}

/// The URL as it is kept and shown: http(s) only, without user info, query or fragment, which
/// can hold tokens. `None` when there is no usable URL.
pub fn clean_url(raw: &str) -> Option<String> {
    let mut url = url::Url::parse(raw.trim()).ok()?;
    if !matches!(url.scheme(), "http" | "https") || url.host_str().is_none() {
        return None;
    }
    let _ = url.set_username("");
    let _ = url.set_password(None);
    url.set_query(None);
    url.set_fragment(None);
    let s = url.to_string();
    (s.chars().count() <= MAX_URL_CHARS).then_some(s)
}

/// A request's kind, reason and URL, checked and trimmed.
pub fn clean_request(kind: &str, reason: &str, url: Option<&str>) -> Result<(String, String, Option<String>), HumanStepError> {
    let kind = kind.trim();
    if !HUMAN_STEP_KINDS.contains(&kind) {
        return Err(HumanStepError::Invalid("kind must be sign_in, captcha, two_factor or other".into()));
    }
    let reason = reason.split_whitespace().collect::<Vec<_>>().join(" ");
    if reason.is_empty() {
        return Err(HumanStepError::Invalid("reason is required".into()));
    }
    if reason.chars().count() > MAX_REASON_CHARS {
        return Err(HumanStepError::Invalid(format!("reason is longer than {MAX_REASON_CHARS} characters")));
    }
    let url = match url.map(str::trim).filter(|u| !u.is_empty()) {
        Some(u) => Some(clean_url(u).ok_or_else(|| HumanStepError::Invalid("url must be an http(s) address".into()))?),
        None => None,
    };
    Ok((kind.to_string(), reason, url))
}

/// Where a step is asked.
#[derive(Debug, Clone, Default)]
pub struct HumanStepPlace {
    pub kwant_id: String,
    pub channel_id: Option<String>,
    pub direct_kwant_id: Option<String>,
    pub thread_id: Option<String>,
    pub turn_id: Option<String>,
    pub sandbox_target: Option<String>,
}

/// A new step (with `true`), or the turn's step that is already open (with `false`).
pub type Created = (HumanStep, bool, Option<Message>);

impl Database {
    pub(super) async fn migrate_human_steps(&self) -> Result<(), DbError> {
        for stmt in [
            "CREATE TABLE IF NOT EXISTS human_steps (
                id TEXT PRIMARY KEY,
                kwant_id TEXT NOT NULL,
                channel_id TEXT,
                direct_kwant_id TEXT,
                thread_id TEXT,
                turn_id TEXT,
                kind TEXT NOT NULL CHECK (kind IN ('sign_in', 'captcha', 'two_factor', 'other')),
                reason TEXT NOT NULL,
                url TEXT,
                sandbox_target TEXT,
                source TEXT NOT NULL CHECK (source IN ('bot', 'auto')),
                status TEXT NOT NULL CHECK (status IN ('open', 'done', 'declined', 'cancelled')),
                message_id TEXT,
                created_at TIMESTAMP NOT NULL,
                closed_at TIMESTAMP
            )",
            "CREATE INDEX IF NOT EXISTS human_steps_by_status ON human_steps (status, kwant_id)",
        ] {
            sqlx::query(stmt).execute(&self.pool).await?;
        }
        Ok(())
    }

    /// Saves a step together with the chat message that shows it (built by `card`), in one
    /// transaction. A turn that asks again while its step is open gets that step back, so a
    /// retrying agent (or the CAPTCHA check firing on every page) doesn't pile up cards.
    pub async fn create_human_step_with_card(
        &self,
        place: &HumanStepPlace,
        kind: &str,
        reason: &str,
        url: Option<&str>,
        source: &str,
        card: impl FnOnce(&HumanStep) -> Option<(NewMessage, MessageExtra)>,
    ) -> Result<Created, HumanStepError> {
        let mut tx = self.pool.begin().await?;
        let get = "SELECT * FROM human_steps WHERE id = ?";
        // Writes first so the transaction holds SQLite's write lock before it reads
        let id = Uuid::new_v4().to_string();
        let inserted = sqlx::query(
            "INSERT INTO human_steps (id, kwant_id, channel_id, direct_kwant_id, thread_id, turn_id, kind, reason, url, sandbox_target, source, status, created_at)
             SELECT ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, 'open', ?12
             WHERE (SELECT COUNT(*) FROM human_steps WHERE kwant_id = ?2 AND status = 'open') < ?13
               AND NOT EXISTS (SELECT 1 FROM human_steps WHERE status = 'open' AND ?6 IS NOT NULL AND turn_id = ?6)",
        )
        .bind(&id)
        .bind(&place.kwant_id)
        .bind(&place.channel_id)
        .bind(&place.direct_kwant_id)
        .bind(&place.thread_id)
        .bind(&place.turn_id)
        .bind(kind)
        .bind(reason)
        .bind(url)
        .bind(&place.sandbox_target)
        .bind(source)
        .bind(Utc::now())
        .bind(MAX_OPEN_PER_KWANT)
        .execute(&mut *tx)
        .await?;
        if inserted.rows_affected() == 0 {
            let existing = match &place.turn_id {
                Some(t) => {
                    sqlx::query_as::<_, HumanStep>("SELECT * FROM human_steps WHERE status = 'open' AND turn_id = ? LIMIT 1")
                        .bind(t)
                        .fetch_optional(&mut *tx)
                        .await?
                }
                None => None,
            };
            tx.commit().await?;
            return match existing {
                Some(step) => Ok((step, false, None)),
                None => Err(HumanStepError::TooManyOpen),
            };
        }
        let mut step = sqlx::query_as::<_, HumanStep>(get).bind(&id).fetch_one(&mut *tx).await?;
        let mut message = None;
        if let Some((msg, extra)) = card(&step) {
            let saved = super::insert_message_on(&mut tx, &msg, &extra).await?;
            sqlx::query("UPDATE human_steps SET message_id = ? WHERE id = ?").bind(&saved.id).bind(&id).execute(&mut *tx).await?;
            step = sqlx::query_as::<_, HumanStep>(get).bind(&id).fetch_one(&mut *tx).await?;
            message = Some(saved);
        }
        tx.commit().await?;
        Ok((step, true, message))
    }

    pub async fn get_human_step(&self, id: &str) -> Result<Option<HumanStep>, DbError> {
        Ok(sqlx::query_as::<_, HumanStep>("SELECT * FROM human_steps WHERE id = ?")
            .bind(id)
            .fetch_optional(&self.pool)
            .await?)
    }

    /// Steps, newest first, at most 100: by status and/or chat (channel, thread or direct-chat
    /// kwant id); `space` keeps the steps of that space's kwants (all when `None`).
    pub async fn list_human_steps(
        &self,
        status: Option<&str>,
        context_id: Option<&str>,
        space: Option<&super::spaces::SpaceId>,
    ) -> Result<Vec<HumanStep>, DbError> {
        Ok(sqlx::query_as::<_, HumanStep>(
            "SELECT * FROM human_steps
             WHERE (?1 IS NULL OR status = ?1)
               AND (?2 IS NULL OR channel_id = ?2 OR thread_id = ?2 OR direct_kwant_id = ?2)
               AND (?3 IS NULL OR COALESCE((SELECT k.space_id FROM kwants k WHERE k.id = human_steps.kwant_id), 'spc_default') = ?3)
             ORDER BY created_at DESC LIMIT 100",
        )
        .bind(status)
        .bind(context_id)
        .bind(space.map(|s| s.as_str()))
        .fetch_all(&self.pool)
        .await?)
    }

    /// The open steps of one turn.
    pub async fn open_human_steps_for_turn(&self, turn_id: &str) -> Result<Vec<HumanStep>, DbError> {
        Ok(sqlx::query_as::<_, HumanStep>("SELECT * FROM human_steps WHERE status = 'open' AND turn_id = ?")
            .bind(turn_id)
            .fetch_all(&self.pool)
            .await?)
    }

    /// Whether a step is open in this bot browser (it stays the person's until then).
    pub async fn human_step_open_in(&self, sandbox_target: &str) -> Result<bool, DbError> {
        let n: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM human_steps WHERE status = 'open' AND sandbox_target = ?")
            .bind(sandbox_target)
            .fetch_one(&self.pool)
            .await?;
        Ok(n > 0)
    }

    /// Closes an open step; `false` when it wasn't open any more (someone else closed it).
    pub async fn close_human_step(&self, id: &str, status: &str) -> Result<bool, DbError> {
        let done = sqlx::query("UPDATE human_steps SET status = ?, closed_at = ? WHERE id = ? AND status = 'open'")
            .bind(status)
            .bind(Utc::now())
            .bind(id)
            .execute(&self.pool)
            .await?;
        Ok(done.rows_affected() == 1)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn requests_are_checked_and_urls_lose_their_secrets() {
        assert!(clean_request("sign_in", "Log in to GitHub", None).is_ok());
        assert!(clean_request("login", "Log in", None).is_err());
        assert!(clean_request("other", "  ", None).is_err());
        assert!(clean_request("other", &"r".repeat(301), None).is_err());
        assert!(clean_request("captcha", "Solve it", Some("javascript:alert(1)")).is_err());
        let (_, reason, url) =
            clean_request("sign_in", " Log in\n to  GitHub ", Some("https://me:pw@github.com/login?return_to=x&token=abc#f")).unwrap();
        assert_eq!(reason, "Log in to GitHub");
        assert_eq!(url.as_deref(), Some("https://github.com/login"));
    }

    #[tokio::test]
    async fn a_step_closes_once_and_a_turn_gets_its_open_step_back() {
        let tmp = tempfile::tempdir().unwrap();
        let db = Database::init(tmp.path()).await.unwrap();
        let place = |turn: &str| HumanStepPlace {
            kwant_id: "k".into(),
            direct_kwant_id: Some("k".into()),
            turn_id: Some(turn.into()),
            sandbox_target: Some("k".into()),
            ..Default::default()
        };
        let (s, new, _) = db.create_human_step_with_card(&place("t1"), "sign_in", "Log in", None, "bot", |_| None).await.unwrap();
        assert!(new);
        assert_eq!(s.context_id(), "k");
        assert!(db.human_step_open_in("k").await.unwrap());
        let (again, new, _) = db.create_human_step_with_card(&place("t1"), "captcha", "Solve", None, "auto", |_| None).await.unwrap();
        assert!(!new);
        assert_eq!(again.id, s.id);
        assert_eq!(db.open_human_steps_for_turn("t1").await.unwrap().len(), 1);
        assert!(db.close_human_step(&s.id, "done").await.unwrap());
        assert!(!db.close_human_step(&s.id, "declined").await.unwrap());
        let closed = db.get_human_step(&s.id).await.unwrap().unwrap();
        assert_eq!(closed.status, "done");
        assert!(closed.closed_at.is_some());
        assert!(!db.human_step_open_in("k").await.unwrap());
        for i in 0..MAX_OPEN_PER_KWANT {
            db.create_human_step_with_card(&place(&format!("t{}", i + 10)), "other", "Do it", None, "bot", |_| None).await.unwrap();
        }
        let full = db.create_human_step_with_card(&place("t99"), "other", "Do it", None, "bot", |_| None).await;
        assert_eq!(full.err(), Some(HumanStepError::TooManyOpen));
        assert_eq!(db.list_human_steps(Some("open"), Some("k"), None).await.unwrap().len() as i64, MAX_OPEN_PER_KWANT);
    }
}
