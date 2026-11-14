//! Threads (an agent's separate conversations, each with its own history) and delegations (a lead
//! handing a task to a specialist, run in a delegation thread and reported back to the lead's chat).

use super::{Database, DbError};
use chrono::{DateTime, Utc};
use serde::Serialize;
use sqlx::FromRow;
use uuid::Uuid;

/// Longest thread or delegation title.
pub const MAX_TITLE_CHARS: usize = 120;
/// Longest delegated task.
pub const MAX_TASK_CHARS: usize = 8000;

/// Delegation states that are not over yet.
pub const OPEN_STATUSES: [&str; 2] = ["queued", "running"];

#[derive(Debug, Clone, Serialize, FromRow, PartialEq)]
pub struct Thread {
    pub id: String,
    pub kwant_id: String,
    pub title: String,
    /// "chat" (the user opened it) or "delegation".
    pub kind: String,
    pub team_id: Option<String>,
    /// "user" or the id of the kwant that delegated into it.
    pub created_by: String,
    pub archived: bool,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    /// The chat this one was continued from ("Continue in a new chat from here"): a thread id
    /// or a bot's id (its main chat). `None` for every other thread.
    #[sqlx(default)]
    pub forked_from_context: Option<String>,
    /// The message there it was continued from: the newest one copied.
    #[sqlx(default)]
    pub forked_from_message_id: Option<String>,
    /// How many messages were copied into it (Iris's ask: the "Continued from … · n messages"
    /// line needs n after a reload, not only in the 201). `None` for every other thread.
    #[sqlx(default)]
    pub forked_copied: Option<i64>,
    /// Whether only the newest [`CONTINUE_MAX_MESSAGES`] were copied. `None` for every other thread.
    #[sqlx(default)]
    pub forked_truncated: Option<bool>,
}

/// Most messages "Continue in a new chat from here" copies: the newest ones, up to the pressed one.
pub const CONTINUE_MAX_MESSAGES: usize = 1000;

/// What [`Database::continue_chat`] made.
#[derive(Debug, Clone)]
pub struct Continued {
    pub thread: Thread,
    /// Messages copied into the new thread.
    pub copied: usize,
    /// The chat had more than [`CONTINUE_MAX_MESSAGES`] up to the pressed message, so only the
    /// newest ones were copied.
    pub truncated: bool,
}

/// `<title> (continued)`, at most [`MAX_TITLE_CHARS`]: the source title is cut (not the suffix),
/// and continuing a continued chat doesn't stack a second suffix.
pub fn continued_title(source: &str) -> String {
    const SUFFIX: &str = " (continued)";
    let base = clean_title(source);
    let base = base.strip_suffix(SUFFIX).unwrap_or(&base);
    let room = MAX_TITLE_CHARS - SUFFIX.chars().count();
    let base: String = if base.chars().count() > room {
        let mut b: String = base.chars().take(room - 1).collect();
        b.push('…');
        b
    } else {
        base.to_string()
    };
    format!("{base}{SUFFIX}")
}

#[derive(Debug, Clone, Serialize, FromRow, PartialEq)]
pub struct Delegation {
    pub id: String,
    pub thread_id: String,
    pub lead_kwant_id: String,
    pub specialist_kwant_id: String,
    pub origin_channel_id: Option<String>,
    pub origin_kwant_id: Option<String>,
    pub origin_thread_id: Option<String>,
    pub task: String,
    pub title: String,
    /// "queued", "running", "completed", "failed", "cancelled" or "interrupted".
    pub status: String,
    pub continue_lead: bool,
    pub depth: i64,
    pub result_message_id: Option<String>,
    pub error: Option<String>,
    pub follow_up_at: Option<DateTime<Utc>>,
    pub followed_up: bool,
    pub created_at: DateTime<Utc>,
    pub started_at: Option<DateTime<Utc>>,
    pub ended_at: Option<DateTime<Utc>>,
    /// Times a kwnt restart sent it back to the queue (at most once).
    #[sqlx(default)]
    pub retries: i64,
    /// A delegation whose result this one waits for before it starts ("@Writer, once @Analyst
    /// has the numbers, …"). Set while it is parked as `queued`; cleared when it starts.
    #[sqlx(default)]
    pub after_delegation_id: Option<String>,
    /// The delegation whose result @mentioned this teammate, so this task was started from it.
    /// The lead hears both results in one batch (see `finished_batch`).
    #[sqlx(default)]
    pub relayed_from_delegation_id: Option<String>,
    /// The lead's turn that started it with the `delegate` tool, while that turn held its
    /// hand-offs' reports: the lead hears every hand-off of that turn in one batch (see
    /// `finished_batch`).
    #[sqlx(default)]
    pub lead_turn_id: Option<String>,
}

impl Delegation {
    pub fn is_open(&self) -> bool {
        OPEN_STATUSES.contains(&self.status.as_str())
    }

    /// The chat the result goes back to: a channel, the lead's direct chat or a thread.
    pub fn origin_context(&self) -> Option<&str> {
        self.origin_channel_id
            .as_deref()
            .or(self.origin_thread_id.as_deref())
            .or(self.origin_kwant_id.as_deref())
    }
}

/// What a new delegation needs; the thread is created with it.
#[derive(Debug, Clone)]
pub struct NewDelegation {
    pub lead_kwant_id: String,
    pub specialist_kwant_id: String,
    pub origin_channel_id: Option<String>,
    pub origin_kwant_id: Option<String>,
    pub origin_thread_id: Option<String>,
    pub team_id: Option<String>,
    pub task: String,
    pub title: String,
    pub continue_lead: bool,
    pub depth: i64,
    pub follow_up_at: Option<DateTime<Utc>>,
}

/// Title from free text: one line, at most [`MAX_TITLE_CHARS`].
pub fn clean_title(text: &str) -> String {
    let line = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if line.chars().count() <= MAX_TITLE_CHARS {
        return line;
    }
    let mut out: String = line.chars().take(MAX_TITLE_CHARS - 1).collect();
    out.push('…');
    out
}

impl Database {
    /// Additive: thread and meta columns on messages, and the two tables. An old binary never
    /// shows thread messages (they have no channel or direct chat) and ignores the rest.
    pub(super) async fn migrate_threads(&self) -> Result<(), DbError> {
        let _ = sqlx::query("ALTER TABLE messages ADD COLUMN thread_id TEXT;")
            .execute(&self.pool)
            .await;
        let _ = sqlx::query("ALTER TABLE messages ADD COLUMN meta TEXT;")
            .execute(&self.pool)
            .await;
        for stmt in [
            "CREATE INDEX IF NOT EXISTS messages_by_thread ON messages (thread_id, created_at) WHERE thread_id IS NOT NULL",
            "CREATE TABLE IF NOT EXISTS threads (
                id TEXT PRIMARY KEY,
                kwant_id TEXT NOT NULL,
                title TEXT NOT NULL,
                kind TEXT NOT NULL CHECK (kind IN ('chat', 'delegation')),
                team_id TEXT,
                created_by TEXT NOT NULL,
                archived INTEGER NOT NULL DEFAULT 0,
                created_at TIMESTAMP NOT NULL,
                updated_at TIMESTAMP NOT NULL
            )",
            "CREATE INDEX IF NOT EXISTS threads_by_kwant ON threads (kwant_id, updated_at)",
            "CREATE TABLE IF NOT EXISTS delegations (
                id TEXT PRIMARY KEY,
                thread_id TEXT NOT NULL,
                lead_kwant_id TEXT NOT NULL,
                specialist_kwant_id TEXT NOT NULL,
                origin_channel_id TEXT,
                origin_kwant_id TEXT,
                origin_thread_id TEXT,
                task TEXT NOT NULL,
                title TEXT NOT NULL,
                status TEXT NOT NULL CHECK (status IN ('queued', 'running', 'completed', 'failed', 'cancelled', 'interrupted')),
                continue_lead INTEGER NOT NULL DEFAULT 1,
                depth INTEGER NOT NULL DEFAULT 1,
                result_message_id TEXT,
                error TEXT,
                follow_up_at TIMESTAMP,
                followed_up INTEGER NOT NULL DEFAULT 0,
                created_at TIMESTAMP NOT NULL,
                started_at TIMESTAMP,
                ended_at TIMESTAMP
            )",
            "CREATE INDEX IF NOT EXISTS delegations_by_lead ON delegations (lead_kwant_id, created_at)",
            "CREATE INDEX IF NOT EXISTS delegations_by_thread ON delegations (thread_id)",
        ] {
            sqlx::query(stmt).execute(&self.pool).await?;
        }
        let _ = sqlx::query("ALTER TABLE delegations ADD COLUMN after_delegation_id TEXT;")
            .execute(&self.pool)
            .await;
        let _ = sqlx::query("ALTER TABLE delegations ADD COLUMN relayed_from_delegation_id TEXT;")
            .execute(&self.pool)
            .await;
        let _ = sqlx::query("ALTER TABLE delegations ADD COLUMN lead_turn_id TEXT;")
            .execute(&self.pool)
            .await;
        // Continue in a new chat from here (0.9.10): where a chat was continued from
        let _ = sqlx::query("ALTER TABLE threads ADD COLUMN forked_from_context TEXT;")
            .execute(&self.pool)
            .await;
        let _ = sqlx::query("ALTER TABLE threads ADD COLUMN forked_from_message_id TEXT;")
            .execute(&self.pool)
            .await;
        let _ = sqlx::query("ALTER TABLE threads ADD COLUMN forked_copied INTEGER;")
            .execute(&self.pool)
            .await;
        let _ = sqlx::query("ALTER TABLE threads ADD COLUMN forked_truncated INTEGER;")
            .execute(&self.pool)
            .await;
        Ok(())
    }

    // Threads

    pub async fn create_thread(
        &self,
        kwant_id: &str,
        title: &str,
        kind: &str,
        team_id: Option<&str>,
        created_by: &str,
    ) -> Result<Thread, DbError> {
        let now = Utc::now();
        let thread = Thread {
            id: Uuid::new_v4().to_string(),
            kwant_id: kwant_id.into(),
            title: clean_title(title),
            kind: kind.into(),
            team_id: team_id.map(Into::into),
            created_by: created_by.into(),
            archived: false,
            created_at: now,
            updated_at: now,
            forked_from_context: None,
            forked_from_message_id: None,
            forked_copied: None,
            forked_truncated: None,
        };
        sqlx::query(
            "INSERT INTO threads (id, kwant_id, title, kind, team_id, created_by, archived, created_at, updated_at) \
             VALUES (?, ?, ?, ?, ?, ?, 0, ?, ?)",
        )
        .bind(&thread.id)
        .bind(&thread.kwant_id)
        .bind(&thread.title)
        .bind(&thread.kind)
        .bind(&thread.team_id)
        .bind(&thread.created_by)
        .bind(now)
        .bind(now)
        .execute(&self.pool)
        .await?;
        Ok(thread)
    }

    /// "Continue in a new chat from here": a new `chat` thread for `kwant_id`, made by the person,
    /// holding a copy of every message of chat `context_id` up to and including `message_id`, in
    /// one transaction. Copies get new ids and keep their sender, text, reasoning, tool calls,
    /// tokens, meta (file cards by reference) and `created_at`, oldest first, so the order holds;
    /// a reply that quotes a copied message quotes its copy. Stored Smart Results come along.
    /// Reactions, approvals and running turns don't. Past [`CONTINUE_MAX_MESSAGES`] only the
    /// newest are copied (`truncated`). The source chat is only read.
    pub async fn continue_chat(
        &self,
        kwant_id: &str,
        title: &str,
        team_id: Option<&str>,
        context_id: &str,
        message_id: &str,
    ) -> Result<Continued, DbError> {
        let mut tx = self.pool.begin().await?;
        // Newest first, up to and including the pressed message, in the order the chat shows
        // (`list_messages`: created_at, then rowid)
        let mut rows: Vec<(String, Option<String>)> = sqlx::query_as(
            "SELECT m.id, m.reply_to_message_id FROM messages m, \
             (SELECT created_at AS at, rowid AS seq FROM messages WHERE id = ?2) cut \
             WHERE (m.channel_id = ?1 OR m.direct_kwant_id = ?1 OR m.thread_id = ?1) \
             AND (m.created_at < cut.at OR (m.created_at = cut.at AND m.rowid <= cut.seq)) \
             ORDER BY m.created_at DESC, m.rowid DESC LIMIT ?3",
        )
        .bind(context_id)
        .bind(message_id)
        .bind(CONTINUE_MAX_MESSAGES as i64 + 1)
        .fetch_all(&mut *tx)
        .await?;
        let truncated = rows.len() > CONTINUE_MAX_MESSAGES;
        rows.truncate(CONTINUE_MAX_MESSAGES);
        rows.reverse();

        let now = Utc::now();
        let thread = Thread {
            id: Uuid::new_v4().to_string(),
            kwant_id: kwant_id.into(),
            title: clean_title(title),
            kind: "chat".into(),
            team_id: team_id.map(Into::into),
            created_by: "user".into(),
            archived: false,
            created_at: now,
            updated_at: now,
            forked_from_context: Some(context_id.into()),
            forked_from_message_id: Some(message_id.into()),
            forked_copied: Some(rows.len() as i64),
            forked_truncated: Some(truncated),
        };
        sqlx::query(
            "INSERT INTO threads (id, kwant_id, title, kind, team_id, created_by, archived, created_at, updated_at, \
             forked_from_context, forked_from_message_id, forked_copied, forked_truncated) \
             VALUES (?, ?, ?, 'chat', ?, 'user', 0, ?, ?, ?, ?, ?, ?)",
        )
        .bind(&thread.id)
        .bind(&thread.kwant_id)
        .bind(&thread.title)
        .bind(&thread.team_id)
        .bind(now)
        .bind(now)
        .bind(&thread.forked_from_context)
        .bind(&thread.forked_from_message_id)
        .bind(thread.forked_copied)
        .bind(thread.forked_truncated)
        .execute(&mut *tx)
        .await?;

        let new_ids: std::collections::HashMap<&str, String> =
            rows.iter().map(|(id, _)| (id.as_str(), Uuid::new_v4().to_string())).collect();
        for (old, reply_to) in &rows {
            let new = &new_ids[old.as_str()];
            // A quote of a message that wasn't copied keeps pointing at the original
            let reply_to = reply_to.as_deref().map(|r| new_ids.get(r).cloned().unwrap_or_else(|| r.to_string()));
            sqlx::query(
                "INSERT INTO messages (id, channel_id, direct_kwant_id, sender_role, sender_id, text, reasoning_content, \
                 tool_calls, tokens_used, created_at, thread_id, meta, reply_to_message_id) \
                 SELECT ?, NULL, NULL, sender_role, sender_id, text, reasoning_content, tool_calls, tokens_used, created_at, ?, meta, ? \
                 FROM messages WHERE id = ?",
            )
            .bind(new)
            .bind(&thread.id)
            .bind(reply_to)
            .bind(old)
            .execute(&mut *tx)
            .await?;
            sqlx::query(
                "INSERT INTO message_results (message_id, version, payload) SELECT ?, version, payload FROM message_results WHERE message_id = ?",
            )
            .bind(new)
            .bind(old)
            .execute(&mut *tx)
            .await?;
        }
        tx.commit().await?;
        Ok(Continued { thread, copied: rows.len(), truncated })
    }

    /// Whether message `a` comes at or before message `b` in a chat's order (created_at, then
    /// rowid). `false` when either is gone.
    pub async fn message_at_or_before(&self, a: &str, b: &str) -> Result<bool, DbError> {
        Ok(sqlx::query_scalar(
            "SELECT EXISTS (SELECT 1 FROM messages x, messages y WHERE x.id = ? AND y.id = ? \
             AND (x.created_at < y.created_at OR (x.created_at = y.created_at AND x.rowid <= y.rowid)))",
        )
        .bind(a)
        .bind(b)
        .fetch_one(&self.pool)
        .await?)
    }

    pub async fn get_thread(&self, id: &str) -> Result<Option<Thread>, DbError> {
        Ok(sqlx::query_as::<_, Thread>("SELECT * FROM threads WHERE id = ?")
            .bind(id)
            .fetch_optional(&self.pool)
            .await?)
    }

    /// Most recently active first.
    pub async fn list_threads(&self, kwant_id: &str, include_archived: bool, limit: i64) -> Result<Vec<Thread>, DbError> {
        Ok(sqlx::query_as::<_, Thread>(
            "SELECT * FROM threads WHERE kwant_id = ? AND (? OR archived = 0) ORDER BY updated_at DESC, rowid DESC LIMIT ?",
        )
        .bind(kwant_id)
        .bind(include_archived)
        .bind(limit)
        .fetch_all(&self.pool)
        .await?)
    }

    pub async fn update_thread(&self, id: &str, title: Option<&str>, archived: Option<bool>) -> Result<Option<Thread>, DbError> {
        sqlx::query(
            "UPDATE threads SET title = COALESCE(?, title), archived = COALESCE(?, archived), updated_at = ? WHERE id = ?",
        )
        .bind(title.map(clean_title))
        .bind(archived)
        .bind(Utc::now())
        .bind(id)
        .execute(&self.pool)
        .await?;
        self.get_thread(id).await
    }

    /// Moves the thread up the list when something happens in it.
    pub async fn touch_thread(&self, id: &str) -> Result<(), DbError> {
        sqlx::query("UPDATE threads SET updated_at = ? WHERE id = ?")
            .bind(Utc::now())
            .bind(id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    // Delegations

    /// Creates the delegation thread and the delegation (status `queued`) together.
    pub async fn create_delegation(&self, input: &NewDelegation) -> Result<(Thread, Delegation), DbError> {
        let mut tx = self.pool.begin().await?;
        let now = Utc::now();
        let title = clean_title(&input.title);
        let thread = Thread {
            id: Uuid::new_v4().to_string(),
            kwant_id: input.specialist_kwant_id.clone(),
            title: title.clone(),
            kind: "delegation".into(),
            team_id: input.team_id.clone(),
            created_by: input.lead_kwant_id.clone(),
            archived: false,
            created_at: now,
            updated_at: now,
            forked_from_context: None,
            forked_from_message_id: None,
            forked_copied: None,
            forked_truncated: None,
        };
        sqlx::query(
            "INSERT INTO threads (id, kwant_id, title, kind, team_id, created_by, archived, created_at, updated_at) \
             VALUES (?, ?, ?, 'delegation', ?, ?, 0, ?, ?)",
        )
        .bind(&thread.id)
        .bind(&thread.kwant_id)
        .bind(&thread.title)
        .bind(&thread.team_id)
        .bind(&thread.created_by)
        .bind(now)
        .bind(now)
        .execute(&mut *tx)
        .await?;
        let delegation = Delegation {
            id: Uuid::new_v4().to_string(),
            thread_id: thread.id.clone(),
            lead_kwant_id: input.lead_kwant_id.clone(),
            specialist_kwant_id: input.specialist_kwant_id.clone(),
            origin_channel_id: input.origin_channel_id.clone(),
            origin_kwant_id: input.origin_kwant_id.clone(),
            origin_thread_id: input.origin_thread_id.clone(),
            task: input.task.clone(),
            title,
            status: "queued".into(),
            continue_lead: input.continue_lead,
            depth: input.depth,
            result_message_id: None,
            error: None,
            follow_up_at: input.follow_up_at,
            followed_up: false,
            created_at: now,
            started_at: None,
            ended_at: None,
            retries: 0,
            after_delegation_id: None,
            relayed_from_delegation_id: None,
            lead_turn_id: None,
        };
        sqlx::query(
            "INSERT INTO delegations (id, thread_id, lead_kwant_id, specialist_kwant_id, origin_channel_id, origin_kwant_id, \
             origin_thread_id, task, title, status, continue_lead, depth, follow_up_at, created_at) \
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, 'queued', ?, ?, ?, ?)",
        )
        .bind(&delegation.id)
        .bind(&delegation.thread_id)
        .bind(&delegation.lead_kwant_id)
        .bind(&delegation.specialist_kwant_id)
        .bind(&delegation.origin_channel_id)
        .bind(&delegation.origin_kwant_id)
        .bind(&delegation.origin_thread_id)
        .bind(&delegation.task)
        .bind(&delegation.title)
        .bind(delegation.continue_lead)
        .bind(delegation.depth)
        .bind(delegation.follow_up_at)
        .bind(now)
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok((thread, delegation))
    }

    pub async fn get_delegation(&self, id: &str) -> Result<Option<Delegation>, DbError> {
        Ok(sqlx::query_as::<_, Delegation>("SELECT * FROM delegations WHERE id = ?")
            .bind(id)
            .fetch_optional(&self.pool)
            .await?)
    }

    pub async fn delegation_for_thread(&self, thread_id: &str) -> Result<Option<Delegation>, DbError> {
        Ok(sqlx::query_as::<_, Delegation>(
            "SELECT * FROM delegations WHERE thread_id = ? ORDER BY created_at DESC LIMIT 1",
        )
        .bind(thread_id)
        .fetch_optional(&self.pool)
        .await?)
    }

    /// Newest first; `lead` and `status` narrow it down.
    pub async fn list_delegations(&self, lead: Option<&str>, status: Option<&str>, limit: i64) -> Result<Vec<Delegation>, DbError> {
        Ok(sqlx::query_as::<_, Delegation>(
            "SELECT * FROM delegations WHERE (? IS NULL OR lead_kwant_id = ?) AND (? IS NULL OR status = ?) \
             ORDER BY created_at DESC, rowid DESC LIMIT ?",
        )
        .bind(lead)
        .bind(lead)
        .bind(status)
        .bind(status)
        .bind(limit)
        .fetch_all(&self.pool)
        .await?)
    }

    /// The same task from the same lead to the same specialist, from the same chat, since `since`,
    /// that is still open or finished with a result. One that failed, was cancelled or was
    /// interrupted doesn't count, so retrying it right away is allowed.
    #[allow(clippy::too_many_arguments)]
    pub async fn find_recent_delegation(
        &self,
        lead: &str,
        specialist: &str,
        origin_channel_id: Option<&str>,
        origin_kwant_id: Option<&str>,
        origin_thread_id: Option<&str>,
        task: &str,
        since: DateTime<Utc>,
    ) -> Result<Option<Delegation>, DbError> {
        Ok(sqlx::query_as::<_, Delegation>(
            "SELECT * FROM delegations WHERE lead_kwant_id = ? AND specialist_kwant_id = ? \
             AND origin_channel_id IS ? AND origin_kwant_id IS ? AND origin_thread_id IS ? \
             AND task = ? AND created_at >= ? AND status IN ('queued', 'running', 'completed') \
             ORDER BY created_at DESC LIMIT 1",
        )
        .bind(lead)
        .bind(specialist)
        .bind(origin_channel_id)
        .bind(origin_kwant_id)
        .bind(origin_thread_id)
        .bind(task)
        .bind(since)
        .fetch_optional(&self.pool)
        .await?)
    }

    pub async fn count_open_delegations(&self, lead: &str) -> Result<i64, DbError> {
        let (n,): (i64,) = sqlx::query_as(
            "SELECT COUNT(*) FROM delegations WHERE lead_kwant_id = ? AND status IN ('queued', 'running')",
        )
        .bind(lead)
        .fetch_one(&self.pool)
        .await?;
        Ok(n)
    }

    pub async fn mark_delegation_running(&self, id: &str) -> Result<bool, DbError> {
        let res = sqlx::query("UPDATE delegations SET status = 'running', started_at = ? WHERE id = ? AND status = 'queued'")
            .bind(Utc::now())
            .bind(id)
            .execute(&self.pool)
            .await?;
        Ok(res.rows_affected() > 0)
    }

    /// Ends an open delegation; `false` if it was already over (e.g. cancelled meanwhile).
    pub async fn finish_delegation(
        &self,
        id: &str,
        status: &str,
        error: Option<&str>,
        result_message_id: Option<&str>,
    ) -> Result<bool, DbError> {
        let res = sqlx::query(
            "UPDATE delegations SET status = ?, error = ?, result_message_id = COALESCE(?, result_message_id), ended_at = ? \
             WHERE id = ? AND status IN ('queued', 'running')",
        )
        .bind(status)
        .bind(error)
        .bind(result_message_id)
        .bind(Utc::now())
        .bind(id)
        .execute(&self.pool)
        .await?;
        Ok(res.rows_affected() > 0)
    }

    /// Opens an ended delegation again (failed, cancelled or interrupted) to carry on in its
    /// thread: back to `queued`, with its restart count and end cleared (and no "still working"
    /// note for its old follow-up time). `false` if it was open or completed, or someone else
    /// reopened it first.
    pub async fn reopen_delegation(&self, id: &str) -> Result<bool, DbError> {
        let res = sqlx::query(
            "UPDATE delegations SET status = 'queued', error = NULL, ended_at = NULL, started_at = NULL, retries = 0, followed_up = 1 \
             WHERE id = ? AND status IN ('failed', 'cancelled', 'interrupted')",
        )
        .bind(id)
        .execute(&self.pool)
        .await?;
        Ok(res.rows_affected() > 0)
    }

    /// Opens a completed delegation again because its lead's review sent it back.
    pub async fn reopen_reviewed_delegation(&self, id: &str) -> Result<bool, DbError> {
        let res = sqlx::query(
            "UPDATE delegations SET status = 'queued', error = NULL, result_message_id = NULL, ended_at = NULL, started_at = NULL, retries = 0, followed_up = 1 \
             WHERE id = ? AND status = 'completed'",
        )
        .bind(id)
        .execute(&self.pool)
        .await?;
        Ok(res.rows_affected() > 0)
    }

    /// Parks a queued delegation until `after` (another delegation) ends.
    pub async fn set_delegation_after(&self, id: &str, after: &str) -> Result<(), DbError> {
        sqlx::query("UPDATE delegations SET after_delegation_id = ? WHERE id = ? AND status = 'queued'")
            .bind(after)
            .bind(id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    /// Marks `id` as started from `from`'s result by an @mention (see
    /// [`Delegation::relayed_from_delegation_id`]).
    pub async fn set_delegation_relayed_from(&self, id: &str, from: &str) -> Result<(), DbError> {
        sqlx::query("UPDATE delegations SET relayed_from_delegation_id = ? WHERE id = ?")
            .bind(from)
            .bind(id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    /// Marks `id` as started by the lead's turn `turn_id` (see [`Delegation::lead_turn_id`]).
    pub async fn set_delegation_lead_turn(&self, id: &str, turn_id: &str) -> Result<(), DbError> {
        sqlx::query("UPDATE delegations SET lead_turn_id = ? WHERE id = ?")
            .bind(turn_id)
            .bind(id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    /// Delegations parked until `after` ends.
    pub async fn list_dependent_delegations(&self, after: &str) -> Result<Vec<Delegation>, DbError> {
        Ok(sqlx::query_as::<_, Delegation>(
            "SELECT * FROM delegations WHERE after_delegation_id = ? AND status = 'queued' ORDER BY created_at",
        )
        .bind(after)
        .fetch_all(&self.pool)
        .await?)
    }

    /// Every parked delegation, whatever it waits for.
    pub async fn list_parked_delegations(&self) -> Result<Vec<Delegation>, DbError> {
        Ok(sqlx::query_as::<_, Delegation>(
            "SELECT * FROM delegations WHERE after_delegation_id IS NOT NULL AND status = 'queued' ORDER BY created_at",
        )
        .fetch_all(&self.pool)
        .await?)
    }

    /// Unparks a delegation with its final `task` (the prerequisite's result added); `false`
    /// when it isn't parked any more (cancelled, or another caller started it).
    pub async fn unpark_delegation(&self, id: &str, task: &str) -> Result<bool, DbError> {
        let res = sqlx::query(
            "UPDATE delegations SET task = ?, after_delegation_id = NULL WHERE id = ? AND status = 'queued' \
             AND after_delegation_id IS NOT NULL",
        )
        .bind(task)
        .bind(id)
        .execute(&self.pool)
        .await?;
        Ok(res.rows_affected() > 0)
    }

    pub async fn set_delegation_result_message(&self, id: &str, message_id: &str) -> Result<(), DbError> {
        sqlx::query("UPDATE delegations SET result_message_id = ? WHERE id = ?")
            .bind(message_id)
            .bind(id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    /// Open delegations whose follow-up time has passed and that got no follow-up yet; marks them.
    pub async fn take_due_follow_ups(&self, now: DateTime<Utc>) -> Result<Vec<Delegation>, DbError> {
        let due = sqlx::query_as::<_, Delegation>(
            "SELECT * FROM delegations WHERE status IN ('queued', 'running') AND followed_up = 0 \
             AND after_delegation_id IS NULL AND follow_up_at IS NOT NULL AND follow_up_at <= ? ORDER BY follow_up_at",
        )
        .bind(now)
        .fetch_all(&self.pool)
        .await?;
        for d in &due {
            sqlx::query("UPDATE delegations SET followed_up = 1 WHERE id = ?")
                .bind(&d.id)
                .execute(&self.pool)
                .await?;
        }
        Ok(due)
    }

    /// Delegations left open by a kwnt that stopped. Each is restarted once: the first time it's
    /// queued again (returned in `.0`); one cut short a second time becomes `interrupted` (`.1`).
    /// One waiting out a usage limit (a parked turn) is left alone: its retry restarts it.
    pub async fn restart_open_delegations(&self) -> Result<(Vec<Delegation>, Vec<Delegation>), DbError> {
        let open = sqlx::query_as::<_, Delegation>(
            "SELECT * FROM delegations WHERE status IN ('queued', 'running') AND after_delegation_id IS NULL AND id NOT IN \
             (SELECT delegation_id FROM parked_turns WHERE status = 'pending' AND delegation_id IS NOT NULL)",
        )
        .fetch_all(&self.pool)
        .await?;
        let (mut again, mut cut) = (Vec::new(), Vec::new());
        for d in open {
            if d.retries < 1 {
                sqlx::query("UPDATE delegations SET status = 'queued', retries = retries + 1, started_at = NULL WHERE id = ?")
                    .bind(&d.id)
                    .execute(&self.pool)
                    .await?;
                again.push(d);
            } else {
                sqlx::query(
                    "UPDATE delegations SET status = 'interrupted', error = 'kwnt stopped before the task finished', ended_at = ? WHERE id = ?",
                )
                .bind(Utc::now())
                .bind(&d.id)
                .execute(&self.pool)
                .await?;
                cut.push(d);
            }
        }
        Ok((again, cut))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn titles_are_one_short_line() {
        assert_eq!(clean_title("  Fix\nthe   build "), "Fix the build");
        let long = "x".repeat(300);
        let t = clean_title(&long);
        assert_eq!(t.chars().count(), MAX_TITLE_CHARS);
        assert!(t.ends_with('…'));
    }

    #[test]
    fn continued_titles_keep_the_suffix_and_never_stack_it() {
        assert_eq!(continued_title("Wifi  help"), "Wifi help (continued)");
        assert_eq!(continued_title("Wifi help (continued)"), "Wifi help (continued)");
        let t = continued_title(&"x".repeat(300));
        assert_eq!(t.chars().count(), MAX_TITLE_CHARS);
        assert!(t.ends_with("… (continued)"), "{t}");
    }

    async fn say(db: &Database, chat: &str, role: &str, text: &str, reply_to: Option<&str>) -> String {
        let msg = crate::db::models::NewMessage {
            channel_id: None,
            direct_kwant_id: Some(chat.into()),
            sender_role: role.into(),
            sender_id: if role == "user" { "user".into() } else { "Ada".into() },
            text: text.into(),
            reasoning_content: None,
            tool_calls: None,
            tokens_used: 3,
        };
        let id = db.insert_message(&msg).await.unwrap().id;
        if let Some(r) = reply_to {
            sqlx::query("UPDATE messages SET reply_to_message_id = ? WHERE id = ?").bind(r).bind(&id).execute(&db.pool).await.unwrap();
        }
        id
    }

    #[tokio::test]
    async fn a_continued_chat_copies_up_to_the_cut_with_replies_and_results_but_not_reactions() {
        let tmp = tempfile::tempdir().unwrap();
        let db = Database::init(tmp.path()).await.unwrap();
        let first = say(&db, "k1", "user", "one", None).await;
        let second = say(&db, "k1", "kwant", "two", None).await;
        let cut = say(&db, "k1", "user", "three", Some(&second)).await;
        let quoted_before = say(&db, "k2", "user", "elsewhere", None).await;
        sqlx::query("UPDATE messages SET reply_to_message_id = ? WHERE id = ?").bind(&quoted_before).bind(&first).execute(&db.pool).await.unwrap();
        say(&db, "k1", "kwant", "four, after the cut", None).await;
        sqlx::query("INSERT INTO message_results (message_id, version, payload) VALUES (?, 1, '{\"blocks\":[]}')")
            .bind(&second)
            .execute(&db.pool)
            .await
            .unwrap();
        db.set_reaction(&second, Some("up")).await.unwrap();

        let c = db.continue_chat("k1", "Ada (continued)", None, "k1", &cut).await.unwrap();
        assert_eq!((c.copied, c.truncated), (3, false));
        assert_eq!(c.thread.kind, "chat");
        assert_eq!(c.thread.created_by, "user");
        assert_eq!(c.thread.forked_from_context.as_deref(), Some("k1"));
        assert_eq!(c.thread.forked_from_message_id.as_deref(), Some(cut.as_str()));
        // Stored, so a reload still shows "· n messages" (Iris)
        let stored = db.get_thread(&c.thread.id).await.unwrap().unwrap();
        assert_eq!((stored.forked_copied, stored.forked_truncated), (Some(c.copied as i64), Some(c.truncated)));
        assert_eq!(db.get_thread(&c.thread.id).await.unwrap().unwrap(), c.thread, "the columns read back");

        let copies = db.list_messages(&c.thread.id, 100).await.unwrap();
        assert_eq!(copies.iter().map(|m| m.text.as_str()).collect::<Vec<_>>(), ["one", "two", "three"]);
        let originals = db.list_messages("k1", 100).await.unwrap();
        for (copy, orig) in copies.iter().zip(&originals) {
            assert_ne!(copy.id, orig.id);
            assert_eq!(copy.thread_id.as_deref(), Some(c.thread.id.as_str()));
            assert!(copy.direct_kwant_id.is_none() && copy.channel_id.is_none());
            assert_eq!((&copy.sender_role, &copy.sender_id, copy.tokens_used, copy.created_at), (&orig.sender_role, &orig.sender_id, orig.tokens_used, orig.created_at));
        }
        // A quote of a copied message quotes its copy; one of a message that wasn't copied stays
        assert_eq!(copies[2].reply_to_message_id.as_deref(), Some(copies[1].id.as_str()));
        assert_eq!(copies[0].reply_to_message_id.as_deref(), Some(quoted_before.as_str()));
        let (n,): (i64,) = sqlx::query_as("SELECT COUNT(*) FROM message_results WHERE message_id = ? AND payload = '{\"blocks\":[]}'")
            .bind(&copies[1].id)
            .fetch_one(&db.pool)
            .await
            .unwrap();
        assert_eq!(n, 1, "stored results come along");
        assert_eq!(db.get_reaction(&copies[1].id).await.unwrap(), None, "reactions don't");
        assert_eq!(db.get_reaction(&second).await.unwrap().as_deref(), Some("up"));
        // The source is untouched
        assert_eq!(originals.len(), 4);
        assert_eq!(originals.iter().filter(|m| m.thread_id.is_some()).count(), 0);

        assert!(db.message_at_or_before(&first, &cut).await.unwrap());
        assert!(db.message_at_or_before(&cut, &cut).await.unwrap());
        assert!(!db.message_at_or_before(&originals[3].id, &cut).await.unwrap());
        assert!(!db.message_at_or_before("gone", &cut).await.unwrap());
    }
}
