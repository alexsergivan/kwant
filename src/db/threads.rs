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
}
