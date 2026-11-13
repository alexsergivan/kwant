//! "While you were away": what's waiting on the user, what finished and what failed or got
//! stuck, read from the tables that already record it (questions, approvals, delegations and
//! routine runs). Read-only apart from the user's last-seen marker.

use super::spaces::SpaceId;
use super::models::Message;
use super::threads::Delegation;
use super::{Database, DbError};
use chrono::{DateTime, Utc};
use sqlx::FromRow;

/// Settings row holding when the user last looked (RFC 3339). Not one of the `/api/settings`
/// keys: only `POST /api/away/seen` writes it.
pub const LAST_SEEN_KEY: &str = "away_last_seen";

/// A row behind one item, before its summary is cleaned. Every query selects these columns.
#[derive(Debug, Clone, FromRow)]
pub struct AwayRow {
    pub kind: String,
    pub id: String,
    pub kwant_id: String,
    pub kwant_name: Option<String>,
    pub title: String,
    pub status: String,
    /// Raw text the summary comes from: a result, an error or the question's choices.
    pub body: Option<String>,
    pub at: DateTime<Utc>,
    pub thread_id: Option<String>,
    pub channel_id: Option<String>,
    pub direct_kwant_id: Option<String>,
    /// Questions only: the JSON array of choices.
    pub choices: Option<String>,
}

const OPEN_QUESTIONS: &str = "
    SELECT 'question' AS kind, q.id, q.kwant_id, k.name AS kwant_name, q.question AS title, q.status,
           NULL AS body, q.created_at AS at, q.thread_id, q.channel_id, q.direct_kwant_id, q.choices
    FROM questions q LEFT JOIN kwants k ON k.id = q.kwant_id
    WHERE q.status = 'open'
      AND COALESCE(k.space_id, 'spc_default') = ?3";

/// `?1` = since. Completed delegations report the specialist's result message.
const DELEGATIONS_ENDED: &str = "
    SELECT 'delegation' AS kind, d.id, d.specialist_kwant_id AS kwant_id, k.name AS kwant_name, d.title, d.status,
           CASE WHEN d.status = 'completed' THEN COALESCE(m.text, d.error) ELSE COALESCE(d.error, m.text) END AS body,
           d.ended_at AS at, d.thread_id, NULL AS channel_id, NULL AS direct_kwant_id, NULL AS choices
    FROM delegations d
    LEFT JOIN kwants k ON k.id = d.specialist_kwant_id
    LEFT JOIN messages m ON m.id = d.result_message_id
    WHERE d.ended_at > ?1
      AND COALESCE(k.space_id, 'spc_default') = ?3";

/// `?1` = started before. Still running past the threshold.
const DELEGATIONS_STUCK: &str = "
    SELECT 'delegation' AS kind, d.id, d.specialist_kwant_id AS kwant_id, k.name AS kwant_name, d.title,
           'stuck' AS status, NULL AS body, d.started_at AS at, d.thread_id,
           NULL AS channel_id, NULL AS direct_kwant_id, NULL AS choices
    FROM delegations d LEFT JOIN kwants k ON k.id = d.specialist_kwant_id
    WHERE d.status = 'running' AND d.started_at < ?1
      AND COALESCE(k.space_id, 'spc_default') = ?3";

/// `?1` = since. A routine posts in its channel, or the kwant's direct chat when it has none.
const ROUTINE_RUNS_ENDED: &str = "
    SELECT 'routine_run' AS kind, r.id, rt.kwant_id, k.name AS kwant_name, rt.name AS title, r.status,
           CASE WHEN r.status = 'ok' THEN m.text ELSE r.error END AS body, r.ended_at AS at,
           NULL AS thread_id, rt.channel_id,
           CASE WHEN rt.channel_id IS NULL THEN rt.kwant_id END AS direct_kwant_id, NULL AS choices
    FROM routine_runs r
    JOIN routines rt ON rt.id = r.routine_id
    LEFT JOIN kwants k ON k.id = rt.kwant_id
    LEFT JOIN messages m ON m.id = r.message_id
    WHERE r.ended_at > ?1
      AND COALESCE(k.space_id, 'spc_default') = ?3";

const FINISHED_DELEGATIONS: &str = "d.status = 'completed'";
const FAILED_DELEGATIONS: &str = "d.status IN ('failed', 'interrupted')";
const FINISHED_RUNS: &str = "r.status = 'ok'";
const FAILED_RUNS: &str = "r.status = 'error'";

impl Database {
    /// Additive: partial indexes so the away queries never scan delegations or routine runs.
    /// An old binary just doesn't use them.
    pub(super) async fn migrate_away(&self) -> Result<(), DbError> {
        for stmt in [
            "CREATE INDEX IF NOT EXISTS delegations_by_end ON delegations (status, ended_at) WHERE ended_at IS NOT NULL",
            "CREATE INDEX IF NOT EXISTS delegations_running ON delegations (started_at) WHERE status = 'running'",
            "CREATE INDEX IF NOT EXISTS routine_runs_by_end ON routine_runs (status, ended_at) WHERE ended_at IS NOT NULL",
            // 0.9.5 B: a lead's reply that ended with `Done: …` (keep_going's `chain_done` meta), so
            // team jobs are found without scanning messages
            "CREATE INDEX IF NOT EXISTS messages_chain_done ON messages (created_at) WHERE instr(meta, '\"chain_done\"') > 0",
        ] {
            sqlx::query(stmt).execute(&self.pool).await?;
        }
        Ok(())
    }

    /// The space's open questions, newest first.
    pub async fn away_open_questions(&self, space: &SpaceId, limit: i64) -> Result<Vec<AwayRow>, DbError> {
        Ok(sqlx::query_as(&format!("{OPEN_QUESTIONS} ORDER BY q.created_at DESC, q.rowid DESC LIMIT ?2"))
            .bind(Utc::now())
            .bind(limit)
            .bind(space.as_str())
            .fetch_all(&self.pool)
            .await?)
    }

    pub async fn away_count_open_questions(&self, space: &SpaceId) -> Result<i64, DbError> {
        Ok(sqlx::query_scalar(&format!("SELECT COUNT(*) FROM ({OPEN_QUESTIONS})"))
            .bind(Utc::now())
            .bind(0)
            .bind(space.as_str())
            .fetch_one(&self.pool)
            .await?)
    }

    /// Pending approvals among `ids` (the tickets the broker still waits on), newest first.
    pub async fn away_pending_approvals(&self, ids: &[String]) -> Result<Vec<AwayRow>, DbError> {
        if ids.is_empty() {
            return Ok(Vec::new());
        }
        let marks = vec!["?"; ids.len()].join(", ");
        let sql = format!(
            "SELECT 'approval' AS kind, a.id, a.kwant_id, k.name AS kwant_name, a.tool_name AS title, a.status,
                    a.parameters AS body, a.created_at AS at, NULL AS thread_id, NULL AS channel_id,
                    NULL AS direct_kwant_id, NULL AS choices
             FROM approvals a LEFT JOIN kwants k ON k.id = a.kwant_id
             WHERE a.status = 'pending' AND a.id IN ({marks})
             ORDER BY a.created_at DESC, a.rowid DESC"
        );
        let mut q = sqlx::query_as(&sql);
        for id in ids {
            q = q.bind(id);
        }
        Ok(q.fetch_all(&self.pool).await?)
    }

    /// The space's completed delegations and successful routine runs that ended after `since`,
    /// newest first.
    pub async fn away_finished(&self, space: &SpaceId, since: DateTime<Utc>, limit: i64) -> Result<Vec<AwayRow>, DbError> {
        let mut rows = self.ended(DELEGATIONS_ENDED, FINISHED_DELEGATIONS, "d", space, since, limit).await?;
        rows.extend(self.ended(ROUTINE_RUNS_ENDED, FINISHED_RUNS, "r", space, since, limit).await?);
        Ok(newest(rows, limit))
    }

    /// Failed or interrupted delegations and failed routine runs that ended after `since`, plus
    /// delegations still running that started before `stuck_before`, newest first.
    pub async fn away_failed(
        &self,
        space: &SpaceId,
        since: DateTime<Utc>,
        stuck_before: DateTime<Utc>,
        limit: i64,
    ) -> Result<Vec<AwayRow>, DbError> {
        let mut rows = self.ended(DELEGATIONS_ENDED, FAILED_DELEGATIONS, "d", space, since, limit).await?;
        rows.extend(self.ended(ROUTINE_RUNS_ENDED, FAILED_RUNS, "r", space, since, limit).await?);
        rows.extend(
            sqlx::query_as::<_, AwayRow>(&format!("{DELEGATIONS_STUCK} ORDER BY d.started_at DESC, d.rowid DESC LIMIT ?2"))
                .bind(stuck_before)
                .bind(limit)
                .bind(space.as_str())
                .fetch_all(&self.pool)
                .await?,
        );
        Ok(newest(rows, limit))
    }

    /// Uncapped (finished, failed) counts for the same filters.
    pub async fn away_counts(
        &self,
        space: &SpaceId,
        since: DateTime<Utc>,
        stuck_before: DateTime<Utc>,
    ) -> Result<(i64, i64), DbError> {
        let finished = self.count(DELEGATIONS_ENDED, FINISHED_DELEGATIONS, space, since).await?
            + self.count(ROUTINE_RUNS_ENDED, FINISHED_RUNS, space, since).await?;
        let failed = self.count(DELEGATIONS_ENDED, FAILED_DELEGATIONS, space, since).await?
            + self.count(ROUTINE_RUNS_ENDED, FAILED_RUNS, space, since).await?
            + self.count(DELEGATIONS_STUCK, "1", space, stuck_before).await?;
        Ok((finished, failed))
    }

    async fn count(&self, sql: &str, cond: &str, space: &SpaceId, t: DateTime<Utc>) -> Result<i64, DbError> {
        // ?2 is the listing limit: unused here, but the slots are numbered up to ?3
        Ok(sqlx::query_scalar(&format!("SELECT COUNT(*) FROM ({sql} AND {cond})"))
            .bind(t)
            .bind(0)
            .bind(space.as_str())
            .fetch_one(&self.pool)
            .await?)
    }

    async fn ended(
        &self,
        sql: &str,
        cond: &str,
        alias: &str,
        space: &SpaceId,
        since: DateTime<Utc>,
        limit: i64,
    ) -> Result<Vec<AwayRow>, DbError> {
        Ok(sqlx::query_as(&format!(
            "{sql} AND {cond} ORDER BY {alias}.ended_at DESC, {alias}.rowid DESC LIMIT ?2"
        ))
        .bind(since)
        .bind(limit)
        .bind(space.as_str())
        .fetch_all(&self.pool)
        .await?)
    }
}

/// A lead's reply that ended with `Done: …` (0.9.5 B team jobs). `meta` holds `chain_done`.
#[derive(Debug, Clone, FromRow)]
pub struct DoneReplyRow {
    pub id: String,
    pub channel_id: Option<String>,
    pub direct_kwant_id: Option<String>,
    pub thread_id: Option<String>,
    pub sender_id: String,
    pub text: String,
    pub meta: Option<String>,
    pub created_at: DateTime<Utc>,
}

/// `?1` = since. The partial index `messages_chain_done` matches this filter exactly.
const DONE_REPLIES: &str = "
    SELECT id, channel_id, direct_kwant_id, thread_id, sender_id, text, meta, created_at
    FROM messages
    WHERE instr(meta, '\"chain_done\"') > 0 AND created_at > ?1 AND sender_role = 'kwant'";

/// The kinds of chat a message belongs to; each column has its partial index
/// (`messages_by_channel`, `messages_by_direct`, `messages_by_thread`) on `(col, created_at)`.
const CHAT_COLUMNS: [&str; 3] = ["channel_id", "direct_kwant_id", "thread_id"];

const ROUTINE_RUN_RESULT: &str =
    "SELECT r.message_id, rt.instructions, r.routine_id, r.trigger FROM routine_runs r JOIN routines rt ON rt.id = r.routine_id WHERE r.id = ?";

/// `?` = chat id, at.
fn person_before_sql(col: &str) -> String {
    format!("SELECT * FROM messages WHERE {col} = ? AND created_at <= ? AND sender_role = 'user' ORDER BY created_at DESC, rowid DESC LIMIT 1")
}

/// `?` = chat id, bot name, from, to.
fn task_reply_after_sql(col: &str) -> String {
    format!(
        "SELECT * FROM messages WHERE {col} = ? AND sender_role = 'kwant' AND sender_id = ? AND created_at >= ? AND created_at <= ? \
         ORDER BY created_at, rowid LIMIT 1"
    )
}

/// `?` = chat id, bot name, before.
fn task_reply_before_sql(col: &str) -> String {
    format!(
        "SELECT * FROM messages WHERE {col} = ? AND sender_role = 'kwant' AND sender_id = ? AND created_at < ? \
         ORDER BY created_at DESC, rowid DESC LIMIT 1"
    )
}

/// `?` = lead, from, to. `delegations_by_lead` is on `(lead_kwant_id, created_at)`.
const LEAD_DELEGATIONS: &str =
    "SELECT * FROM delegations WHERE lead_kwant_id = ? AND created_at >= ? AND created_at <= ? ORDER BY created_at DESC, rowid DESC LIMIT 100";

impl Database {
    /// The hand-offs `lead` made between `from` and `to` (at most 100, newest first): a team
    /// job's members.
    pub async fn lead_delegations(&self, lead: &str, from: DateTime<Utc>, to: DateTime<Utc>) -> Result<Vec<Delegation>, DbError> {
        Ok(sqlx::query_as(LEAD_DELEGATIONS).bind(lead).bind(from).bind(to).fetch_all(&self.pool).await?)
    }

    /// Lead replies that ended with `Done: …` after `since`, newest first (any space: the caller
    /// keeps the ones whose lead is in its space).
    pub async fn done_replies(&self, since: DateTime<Utc>, limit: i64) -> Result<Vec<DoneReplyRow>, DbError> {
        Ok(sqlx::query_as(&format!("{DONE_REPLIES} ORDER BY created_at DESC, rowid DESC LIMIT ?2"))
            .bind(since)
            .bind(limit)
            .fetch_all(&self.pool)
            .await?)
    }

    /// A routine run's result message, its routine's instructions, the routine's id and what
    /// started the run (`schedule`, `manual`, `event`).
    pub async fn routine_run_result(&self, run_id: &str) -> Result<Option<(Option<String>, String, String, String)>, DbError> {
        Ok(sqlx::query_as(ROUTINE_RUN_RESULT)
        .bind(run_id)
        .fetch_optional(&self.pool)
        .await?)
    }

    /// The kwant called `name` in `space` (a reply's `sender_id` is its kwant's name).
    pub async fn kwant_id_named(&self, name: &str, space: &SpaceId) -> Result<Option<String>, DbError> {
        Ok(sqlx::query_scalar(
            "SELECT id FROM kwants WHERE name = ? AND COALESCE(space_id, 'spc_default') = ? ORDER BY created_at, rowid LIMIT 1",
        )
        .bind(name)
        .bind(space.as_str())
        .fetch_optional(&self.pool)
        .await?)
    }

    /// The person's last message in a chat (channel, direct chat or thread) at or before `at`:
    /// where the request a lead's `Done:` closes started.
    pub async fn last_person_message_before(&self, context: &str, at: DateTime<Utc>) -> Result<Option<Message>, DbError> {
        // One indexed lookup per kind of chat; a context id is only ever one of them
        for col in CHAT_COLUMNS {
            let row = sqlx::query_as::<_, Message>(&person_before_sql(col))
            .bind(context)
            .bind(at)
            .fetch_optional(&self.pool)
            .await?;
            if row.is_some() {
                return Ok(row);
            }
        }
        Ok(None)
    }

    /// The bot's reply that holds a task's result: its first reply in the task's chat from
    /// `review_at` (the hand-in is a tool call, the reply lands when the turn ends) within
    /// `slack`, else its last one before.
    pub async fn task_result_message(
        &self,
        context: &str,
        bot_name: &str,
        review_at: DateTime<Utc>,
        slack: chrono::Duration,
    ) -> Result<Option<Message>, DbError> {
        for col in ["thread_id", "channel_id", "direct_kwant_id"] {
            let after = sqlx::query_as::<_, Message>(&task_reply_after_sql(col))
            .bind(context)
            .bind(bot_name)
            .bind(review_at)
            .bind(review_at + slack)
            .fetch_optional(&self.pool)
            .await?;
            if after.is_some() {
                return Ok(after);
            }
            let before = sqlx::query_as::<_, Message>(&task_reply_before_sql(col))
            .bind(context)
            .bind(bot_name)
            .bind(review_at)
            .fetch_optional(&self.pool)
            .await?;
            if before.is_some() {
                return Ok(before);
            }
        }
        Ok(None)
    }
}

/// Newest first, at most `limit`.
fn newest(mut rows: Vec<AwayRow>, limit: i64) -> Vec<AwayRow> {
    rows.sort_by(|a, b| b.at.cmp(&a.at).then_with(|| b.id.cmp(&a.id)));
    rows.truncate(limit.max(0) as usize);
    rows
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn plan(db: &Database, sql: &str) -> Vec<String> {
        let rows: Vec<(i64, i64, i64, String)> = sqlx::query_as(&format!("EXPLAIN QUERY PLAN {sql}"))
            .bind(Utc::now())
            .bind(50)
            .bind("spc_default")
            .fetch_all(&db.pool)
            .await
            .unwrap();
        rows.into_iter().map(|r| r.3).collect()
    }

    /// Every away query reaches its rows through an index, never by scanning a whole table
    /// (the small lookups joined in by primary key aside).
    #[tokio::test]
    async fn away_queries_use_indexes() {
        let tmp = tempfile::tempdir().unwrap();
        let db = Database::init(tmp.path()).await.unwrap();
        let cases = [
            (format!("{DELEGATIONS_ENDED} AND {FINISHED_DELEGATIONS} ORDER BY d.ended_at DESC LIMIT ?2"), "delegations_by_end"),
            (format!("{DELEGATIONS_ENDED} AND {FAILED_DELEGATIONS} ORDER BY d.ended_at DESC LIMIT ?2"), "delegations_by_end"),
            (format!("{DELEGATIONS_STUCK} ORDER BY d.started_at DESC LIMIT ?2"), "delegations_running"),
            (format!("{ROUTINE_RUNS_ENDED} AND {FINISHED_RUNS} ORDER BY r.ended_at DESC LIMIT ?2"), "routine_runs_by_end"),
            (format!("{ROUTINE_RUNS_ENDED} AND {FAILED_RUNS} ORDER BY r.ended_at DESC LIMIT ?2"), "routine_runs_by_end"),
            (format!("{OPEN_QUESTIONS} ORDER BY q.created_at DESC LIMIT ?2"), "questions_by_status"),
        ];
        for (sql, idx) in cases {
            let detail = plan(&db, &sql).await;
            assert!(!detail.iter().any(|d| d.starts_with("SCAN")), "full scan: {detail:?}\n{sql}");
            assert!(detail.iter().any(|d| d.contains(idx)), "{idx} unused: {detail:?}");
        }
        // Team jobs (0.9.5 B): two parameters, no space
        let rows: Vec<(i64, i64, i64, String)> = sqlx::query_as(&format!("EXPLAIN QUERY PLAN {DONE_REPLIES} ORDER BY created_at DESC LIMIT ?2"))
            .bind(Utc::now())
            .bind(50)
            .fetch_all(&db.pool)
            .await
            .unwrap();
        let detail: Vec<String> = rows.into_iter().map(|r| r.3).collect();
        assert!(!detail.iter().any(|d| d.starts_with("SCAN")), "full scan: {detail:?}");
        assert!(detail.iter().any(|d| d.contains("messages_chain_done")), "messages_chain_done unused: {detail:?}");
    }

    /// The plan with every `?` bound (to a string: the plan doesn't depend on the value).
    async fn plan_of(db: &Database, sql: &str) -> Vec<String> {
        let params = sql.matches('?').count();
        let explain = format!("EXPLAIN QUERY PLAN {sql}");
        let mut q = sqlx::query_as::<_, (i64, i64, i64, String)>(&explain);
        for _ in 0..params {
            q = q.bind("x");
        }
        q.fetch_all(&db.pool).await.unwrap().into_iter().map(|r| r.3).collect()
    }

    /// The per-item lookups behind the Inbox's done items (0.9.5 B) reach messages,
    /// turn_tool_calls and the rest through an index or a primary key, never by a full scan.
    #[tokio::test]
    async fn inbox_done_lookups_use_indexes() {
        let tmp = tempfile::tempdir().unwrap();
        let db = Database::init(tmp.path()).await.unwrap();
        let index_of = |col: &str| match col {
            "channel_id" => "messages_by_channel",
            "direct_kwant_id" => "messages_by_direct",
            _ => "messages_by_thread",
        };
        let mut cases: Vec<(String, &str)> = vec![
            (ROUTINE_RUN_RESULT.to_string(), "sqlite_autoindex_routine_runs_1"),
            (format!("{DONE_REPLIES} ORDER BY created_at DESC, rowid DESC LIMIT ?2"), "messages_chain_done"),
            // tool_calls_of (db/checkpoints.rs)
            ("SELECT * FROM turn_tool_calls WHERE task_id = ? ORDER BY id".to_string(), "turn_tool_calls_task"),
            (LEAD_DELEGATIONS.to_string(), "delegations_by_lead"),
        ];
        for col in CHAT_COLUMNS {
            cases.push((person_before_sql(col), index_of(col)));
            cases.push((task_reply_after_sql(col), index_of(col)));
            cases.push((task_reply_before_sql(col), index_of(col)));
        }
        for (sql, idx) in cases {
            let detail = plan_of(&db, &sql).await;
            println!("{idx}: {}", detail.join(" | "));
            assert!(!detail.iter().any(|d| d.starts_with("SCAN")), "full scan: {detail:?}\n{sql}");
            assert!(detail.iter().any(|d| d.contains(idx)), "{idx} unused: {detail:?}\n{sql}");
        }
    }
}
