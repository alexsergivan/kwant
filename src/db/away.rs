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

/// 22i: what [`Database::dismiss_attention`] did.
#[derive(Debug)]
pub enum Dismissed {
    /// Closed now. `kind` is "waiting", "delegation" or "routine_run"; `waiting` the closed row.
    Closed { kind: String, kwant_id: Option<String>, waiting: Option<Box<super::waiting::WaitingItem>> },
    /// It had already left Needs attention (a second Dismiss, or it closed by itself first).
    AlreadyClosed { kind: String },
    NotFound,
    /// It isn't the person's to dismiss: the code names what to use instead.
    Refused(&'static str),
    /// `kind` is none of the kinds `/api/away` lists.
    BadKind,
}

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
    /// Routine runs only: the run's routine, so a failed one can offer Run again
    /// (`POST /api/routines/:id/run`). `NULL` on every other kind.
    pub routine_id: Option<String>,
}

const OPEN_QUESTIONS: &str = "
    SELECT 'question' AS kind, q.id, q.kwant_id, k.name AS kwant_name, q.question AS title, q.status,
           NULL AS body, q.created_at AS at, q.thread_id, q.channel_id, q.direct_kwant_id, q.choices,
           NULL AS routine_id
    FROM questions q LEFT JOIN kwants k ON k.id = q.kwant_id
    WHERE q.status = 'open'
      AND COALESCE(k.space_id, 'spc_default') = ?3";

/// `?1` = since. Completed delegations report the specialist's result message.
const DELEGATIONS_ENDED: &str = "
    SELECT 'delegation' AS kind, d.id, d.specialist_kwant_id AS kwant_id, k.name AS kwant_name, d.title, d.status,
           CASE WHEN d.status = 'completed' THEN COALESCE(m.text, d.error) ELSE COALESCE(d.error, m.text) END AS body,
           d.ended_at AS at, d.thread_id, NULL AS channel_id, NULL AS direct_kwant_id, NULL AS choices,
           NULL AS routine_id
    FROM delegations d
    LEFT JOIN kwants k ON k.id = d.specialist_kwant_id
    LEFT JOIN messages m ON m.id = d.result_message_id
    WHERE d.ended_at > ?1
      AND COALESCE(k.space_id, 'spc_default') = ?3";

/// `?1` = started before. Still running past the threshold.
const DELEGATIONS_STUCK: &str = "
    SELECT 'delegation' AS kind, d.id, d.specialist_kwant_id AS kwant_id, k.name AS kwant_name, d.title,
           'stuck' AS status, NULL AS body, d.started_at AS at, d.thread_id,
           NULL AS channel_id, NULL AS direct_kwant_id, NULL AS choices, NULL AS routine_id
    FROM delegations d LEFT JOIN kwants k ON k.id = d.specialist_kwant_id
    WHERE d.status = 'running' AND d.started_at < ?1
      AND COALESCE(k.space_id, 'spc_default') = ?3
      AND k.id IS NOT NULL
      AND EXISTS (SELECT 1 FROM threads t WHERE t.id = d.thread_id)
      AND NOT EXISTS (SELECT 1 FROM attention_closed x WHERE x.kind = 'delegation' AND x.ref_id = d.id)";

/// `?1` = since. A routine posts in its channel, or the kwant's direct chat when it has none.
const ROUTINE_RUNS_ENDED: &str = "
    SELECT 'routine_run' AS kind, r.id, rt.kwant_id, k.name AS kwant_name, rt.name AS title, r.status,
           CASE WHEN r.status = 'ok' THEN m.text ELSE r.error END AS body, r.ended_at AS at,
           NULL AS thread_id, rt.channel_id,
           CASE WHEN rt.channel_id IS NULL THEN rt.kwant_id END AS direct_kwant_id, NULL AS choices,
           r.routine_id
    FROM routine_runs r
    JOIN routines rt ON rt.id = r.routine_id
    LEFT JOIN kwants k ON k.id = rt.kwant_id
    LEFT JOIN messages m ON m.id = r.message_id
    WHERE r.ended_at > ?1
      AND COALESCE(k.space_id, 'spc_default') = ?3";

const FINISHED_DELEGATIONS: &str = "d.status = 'completed'";
/// 22i close rules for a failed hand-off (each one drops it from Needs attention, the Inbox and
/// the badge): its bot or thread is gone, a later hand-off to the same bot in that thread started
/// (a retry), or it is in `attention_closed` (a later reply of the bot there finished, or the
/// person dismissed it).
const FAILED_DELEGATIONS: &str = "d.status IN ('failed', 'interrupted')
      AND k.id IS NOT NULL
      AND EXISTS (SELECT 1 FROM threads t WHERE t.id = d.thread_id)
      AND NOT EXISTS (SELECT 1 FROM delegations d2 WHERE d2.specialist_kwant_id = d.specialist_kwant_id
                      AND d2.thread_id = d.thread_id AND d2.created_at > d.created_at AND d2.status != 'cancelled')
      AND NOT EXISTS (SELECT 1 FROM attention_closed x WHERE x.kind = 'delegation' AND x.ref_id = d.id)";
const FINISHED_RUNS: &str = "r.status = 'ok'";
/// 22i close rules for a failed routine run: its bot or channel is gone, a later run of the same
/// routine started (Run now or the next schedule), or it is in `attention_closed`. 26b: a run whose
/// step waits on the person (`needs_you`) or failed (`incomplete`) didn't finish either.
const FAILED_RUNS: &str = "r.status IN ('error', 'incomplete', 'needs_you')
      AND k.id IS NOT NULL
      AND (rt.channel_id IS NULL OR EXISTS (SELECT 1 FROM channels c WHERE c.id = rt.channel_id))
      AND NOT EXISTS (SELECT 1 FROM routine_runs r2 WHERE r2.routine_id = r.routine_id
                      AND r2.started_at > r.started_at AND r2.status != 'skipped')
      AND NOT EXISTS (SELECT 1 FROM attention_closed x WHERE x.kind = 'routine_run' AND x.ref_id = r.id)";

/// Why a failed or stuck item left Needs attention (`attention_closed.reason`).
pub const CLOSED_LATER_REPLY: &str = "later_reply";
pub const CLOSED_DISMISSED: &str = "dismissed";

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
            // 22i: failed or stuck items that left Needs attention without their row changing (a
            // later reply of the bot in that chat, or the person's Dismiss); also the Dismiss audit
            // trail for waiting items (`kind` 'waiting'). Additive: an old binary ignores it.
            "CREATE TABLE IF NOT EXISTS attention_closed (
                kind TEXT NOT NULL,
                ref_id TEXT NOT NULL,
                kwant_id TEXT,
                reason TEXT NOT NULL,
                closed_at TIMESTAMP NOT NULL,
                PRIMARY KEY (kind, ref_id)
            )",
            "CREATE INDEX IF NOT EXISTS delegations_by_specialist_thread ON delegations (specialist_kwant_id, thread_id, created_at)",
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

    /// Bots in spaces that aren't archived as `(id, name, space_id)`: those the person wrote to or
    /// heard from in a direct chat most recently first, then the rest by name (the island's quick
    /// ask, `GET /api/glance`). Reads `messages_by_direct`.
    pub async fn ask_targets(&self, limit: i64) -> Result<Vec<(String, String, String)>, DbError> {
        Ok(sqlx::query_as(
            "SELECT id, name, space_id FROM ( \
               SELECT k.id, k.name, k.space_id, \
                      (SELECT MAX(m.created_at) FROM messages m WHERE m.direct_kwant_id = k.id) AS last_at \
               FROM kwants k JOIN spaces s ON s.id = k.space_id WHERE s.archived_at IS NULL) \
             ORDER BY last_at IS NULL, last_at DESC, name, id LIMIT ?",
        )
        .bind(limit)
        .fetch_all(&self.pool)
        .await?)
    }

    /// Every bot's saved avatar sprite, by bot id (the glance puts the face on each item).
    pub async fn avatar_sprites(&self) -> Result<Vec<(String, String)>, DbError> {
        Ok(sqlx::query_as("SELECT id, avatar_sprite FROM kwants").fetch_all(&self.pool).await?)
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
                    NULL AS direct_kwant_id, NULL AS choices, NULL AS routine_id
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

    /// 22i: records that a failed or stuck item (`kind` "delegation" or "routine_run", or
    /// "waiting" for the Dismiss audit of a waiting item) left Needs attention, and why. `false`
    /// when it already had.
    pub async fn close_attention(&self, kind: &str, ref_id: &str, kwant_id: Option<&str>, reason: &str) -> Result<bool, DbError> {
        let res = sqlx::query("INSERT OR IGNORE INTO attention_closed (kind, ref_id, kwant_id, reason, closed_at) VALUES (?, ?, ?, ?, ?)")
            .bind(kind)
            .bind(ref_id)
            .bind(kwant_id)
            .bind(reason)
            .bind(Utc::now())
            .execute(&self.pool)
            .await?;
        Ok(res.rows_affected() > 0)
    }

    /// Why `(kind, ref_id)` left Needs attention, if it did through [`Self::close_attention`].
    pub async fn attention_closed_reason(&self, kind: &str, ref_id: &str) -> Result<Option<String>, DbError> {
        Ok(sqlx::query_scalar("SELECT reason FROM attention_closed WHERE kind = ? AND ref_id = ?")
            .bind(kind)
            .bind(ref_id)
            .fetch_optional(&self.pool)
            .await?)
    }

    /// The bot behind a failed or stuck item the person can dismiss: a hand-off's specialist, a
    /// routine run's bot. `None` when there is no such item.
    pub async fn attention_item_owner(&self, kind: &str, ref_id: &str) -> Result<Option<String>, DbError> {
        let sql = match kind {
            "delegation" => "SELECT specialist_kwant_id FROM delegations WHERE id = ? AND status IN ('failed', 'interrupted', 'running')",
            "routine_run" => "SELECT rt.kwant_id FROM routine_runs r JOIN routines rt ON rt.id = r.routine_id WHERE r.id = ? AND r.status IN ('error', 'incomplete', 'needs_you')",
            _ => return Ok(None),
        };
        Ok(sqlx::query_scalar(sql).bind(ref_id).fetch_optional(&self.pool).await?)
    }

    /// 22i: the board task a failed hand-off's Retry can rerun (`POST /api/tasks/:id/retry`, which
    /// moves only a `failed` task). `None` when the board has no such task (it is off by default)
    /// or the task isn't failed: the row then offers no Retry, only its thread and Dismiss.
    pub async fn retryable_hand_off_task(&self, delegation_id: &str) -> Result<Option<String>, DbError> {
        Ok(sqlx::query_scalar(
            "SELECT id FROM tasks WHERE source_kind = 'delegation' AND source_id = ? AND state = 'failed' ORDER BY created_at DESC LIMIT 1",
        )
        .bind(delegation_id)
        .fetch_optional(&self.pool)
        .await?)
    }

    /// 22i: the person's Dismiss (`POST /api/waiting/:id/dismiss`) of one Needs-attention item.
    /// `kind` is the item's `kind` from `/api/away` (`stopped`, `reply`, `delegation`,
    /// `routine_run`, …); without it, `id` is looked up as a waiting item, then a hand-off, then a
    /// routine run. Each dismiss is recorded in `attention_closed` (reason `dismissed`).
    pub async fn dismiss_attention(&self, id: &str, kind: Option<&str>) -> Result<Dismissed, DbError> {
        match kind {
            Some("question") => return Ok(Dismissed::Refused("use_question_answer")),
            Some("approval") => return Ok(Dismissed::Refused("use_approval_decide")),
            Some("human_step" | "browser") => return Ok(Dismissed::Refused("use_step_decline")),
            Some("watch") => return Ok(Dismissed::Refused("use_watch_decide")),
            Some(k @ ("delegation" | "routine_run")) => return self.dismiss_failed(k, id).await,
            None | Some("stopped" | "reply" | "waiting") => {}
            Some(_) => return Ok(Dismissed::BadKind),
        }
        if let Some(w) = self.get_waiting(id).await? {
            // Its question has an answer or a cancel that tells the bot; a dismiss would not
            if w.question_id.is_some() {
                return Ok(Dismissed::Refused("use_question_answer"));
            }
            let Some(closed) = self.claim_waiting(id).await? else {
                return Ok(Dismissed::AlreadyClosed { kind: "waiting".into() });
            };
            self.close_attention("waiting", id, Some(&closed.kwant_id), CLOSED_DISMISSED).await?;
            return Ok(Dismissed::Closed { kind: "waiting".into(), kwant_id: Some(closed.kwant_id.clone()), waiting: Some(Box::new(closed)) });
        }
        if kind.is_none() {
            for k in ["delegation", "routine_run"] {
                let r = self.dismiss_failed(k, id).await?;
                if !matches!(r, Dismissed::NotFound) {
                    return Ok(r);
                }
            }
        }
        Ok(Dismissed::NotFound)
    }

    async fn dismiss_failed(&self, kind: &str, id: &str) -> Result<Dismissed, DbError> {
        if self.attention_closed_reason(kind, id).await?.is_some() {
            return Ok(Dismissed::AlreadyClosed { kind: kind.into() });
        }
        let Some(owner) = self.attention_item_owner(kind, id).await? else {
            return Ok(Dismissed::NotFound);
        };
        if !self.close_attention(kind, id, Some(&owner), CLOSED_DISMISSED).await? {
            return Ok(Dismissed::AlreadyClosed { kind: kind.into() });
        }
        Ok(Dismissed::Closed { kind: kind.into(), kwant_id: Some(owner), waiting: None })
    }

    /// 22i: a reply of `kwant_id` finished in this chat, so its failed items there are over: a
    /// failed hand-off in this thread, a failed run of its routine that posts here. Records each
    /// in `attention_closed` and returns them as `(kind, id)`.
    pub async fn close_failed_in_chat(
        &self,
        kwant_id: &str,
        channel_id: Option<&str>,
        direct_kwant_id: Option<&str>,
        thread_id: Option<&str>,
    ) -> Result<Vec<(String, String)>, DbError> {
        let mut found: Vec<(String, String)> = Vec::new();
        if let Some(t) = thread_id {
            let ids: Vec<String> = sqlx::query_scalar(
                "SELECT d.id FROM delegations d WHERE d.specialist_kwant_id = ? AND d.thread_id = ? AND d.status IN ('failed', 'interrupted')
                   AND NOT EXISTS (SELECT 1 FROM attention_closed x WHERE x.kind = 'delegation' AND x.ref_id = d.id)",
            )
            .bind(kwant_id)
            .bind(t)
            .fetch_all(&self.pool)
            .await?;
            found.extend(ids.into_iter().map(|id| ("delegation".to_string(), id)));
        } else {
            // A routine posts in its channel's main chat, or its bot's direct chat when it has none
            let ids: Vec<String> = sqlx::query_scalar(
                "SELECT r.id FROM routine_runs r JOIN routines rt ON rt.id = r.routine_id
                 WHERE rt.kwant_id = ?1 AND r.status IN ('error', 'incomplete', 'needs_you')
                   AND ((?2 IS NOT NULL AND rt.channel_id = ?2) OR (?2 IS NULL AND rt.channel_id IS NULL AND ?3 = ?1))
                   AND NOT EXISTS (SELECT 1 FROM attention_closed x WHERE x.kind = 'routine_run' AND x.ref_id = r.id)",
            )
            .bind(kwant_id)
            .bind(channel_id)
            .bind(direct_kwant_id)
            .fetch_all(&self.pool)
            .await?;
            found.extend(ids.into_iter().map(|id| ("routine_run".to_string(), id)));
        }
        let mut closed = Vec::new();
        for (kind, id) in found {
            if self.close_attention(&kind, &id, Some(kwant_id), CLOSED_LATER_REPLY).await? {
                closed.push((kind, id));
            }
        }
        Ok(closed)
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

/// `messages.created_at` is SQLite's `CURRENT_TIMESTAMP` text (`2026-11-21 19:02:20`), while sqlx
/// writes a bound `DateTime` as `2026-11-21T19:02:20…`: as `' ' < 'T'`, every message of that day
/// compared before it (33d). Bound in the column's own format (as `db/auditor.rs` does), the
/// comparison is right and still uses the `(chat, created_at)` indexes.
fn message_time(t: DateTime<Utc>) -> String {
    t.format("%Y-%m-%d %H:%M:%S").to_string()
}

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
    "SELECT * FROM delegations WHERE lead_kwant_id = ? AND created_at >= ? AND created_at < ? ORDER BY created_at DESC, rowid DESC LIMIT 100";

impl Database {
    /// The hand-offs `lead` made between `from` and `to` (at most 100, newest first): a team
    /// job's members. `to` counts to the end of its second: a message's time is whole seconds
    /// and a hand-off's keeps its fraction, so one made in the reply's own second is still in.
    pub async fn lead_delegations(&self, lead: &str, from: DateTime<Utc>, to: DateTime<Utc>) -> Result<Vec<Delegation>, DbError> {
        use chrono::Timelike;
        let to = to.with_nanosecond(0).unwrap_or(to) + chrono::Duration::seconds(1);
        Ok(sqlx::query_as(LEAD_DELEGATIONS).bind(lead).bind(from).bind(to).fetch_all(&self.pool).await?)
    }

    /// Lead replies that ended with `Done: …` after `since`, newest first (any space: the caller
    /// keeps the ones whose lead is in its space).
    pub async fn done_replies(&self, since: DateTime<Utc>, limit: i64) -> Result<Vec<DoneReplyRow>, DbError> {
        Ok(sqlx::query_as(&format!("{DONE_REPLIES} ORDER BY created_at DESC, rowid DESC LIMIT ?2"))
            .bind(message_time(since))
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
            .bind(message_time(at))
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
            .bind(message_time(review_at))
            .bind(message_time(review_at + slack))
            .fetch_optional(&self.pool)
            .await?;
            if after.is_some() {
                return Ok(after);
            }
            let before = sqlx::query_as::<_, Message>(&task_reply_before_sql(col))
            .bind(context)
            .bind(bot_name)
            .bind(message_time(review_at))
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

    async fn bot(db: &Database, id: &str) {
        sqlx::query("INSERT INTO kwants (id, name, title, persona_prompt, avatar_sprite, space_id) VALUES (?, ?, 't', 'p', 'a', 'spc_default')")
            .bind(id)
            .bind(format!("Bot {id}"))
            .execute(&db.pool)
            .await
            .unwrap();
    }

    async fn hand_off(db: &Database, id: &str, thread: &str, specialist: &str, status: &str, mins_ago: i64) {
        let at = Utc::now() - chrono::Duration::minutes(mins_ago);
        let ended = (status != "running" && status != "queued").then_some(at + chrono::Duration::seconds(1));
        sqlx::query(
            "INSERT INTO delegations (id, thread_id, lead_kwant_id, specialist_kwant_id, task, title, status, created_at, started_at, ended_at) \
             VALUES (?, ?, 'lead', ?, 'task', ?, ?, ?, ?, ?)",
        )
        .bind(id)
        .bind(thread)
        .bind(specialist)
        .bind(format!("job {id}"))
        .bind(status)
        .bind(at)
        .bind(at)
        .bind(ended)
        .execute(&db.pool)
        .await
        .unwrap();
    }

    async fn run(db: &Database, id: &str, routine: &str, status: &str, mins_ago: i64) {
        let at = Utc::now() - chrono::Duration::minutes(mins_ago);
        sqlx::query("INSERT INTO routine_runs (id, routine_id, trigger, started_at, ended_at, status, error) VALUES (?, ?, 'schedule', ?, ?, ?, 'boom')")
            .bind(id)
            .bind(routine)
            .bind(at)
            .bind(at + chrono::Duration::seconds(1))
            .bind(status)
            .execute(&db.pool)
            .await
            .unwrap();
    }

    async fn failed_ids(db: &Database) -> Vec<String> {
        let space = SpaceId::default_space();
        let since = Utc::now() - chrono::Duration::days(7);
        let stuck_before = Utc::now() - chrono::Duration::minutes(30);
        let mut ids: Vec<String> = db.away_failed(&space, since, stuck_before, 50).await.unwrap().into_iter().map(|r| r.id).collect();
        ids.sort();
        let (_, counted) = db.away_counts(&space, since, stuck_before).await.unwrap();
        assert_eq!(counted as usize, ids.len(), "the badge counts what is listed");
        ids
    }

    /// 22i close rules for a failed hand-off: a later reply of the bot in that thread, a retry
    /// there, the person's Dismiss, or its thread or bot gone.
    #[tokio::test]
    async fn a_failed_hand_off_closes_on_a_later_reply_a_retry_dismiss_or_a_gone_place() {
        let tmp = tempfile::tempdir().unwrap();
        let db = Database::init(tmp.path()).await.unwrap();
        bot(&db, "k1").await;
        bot(&db, "k2").await;
        let mut t = Vec::new();
        for i in 0..5 {
            t.push(db.create_thread("lead", &format!("t{i}"), "delegation", None, "lead").await.unwrap().id);
        }
        hand_off(&db, "d1", &t[0], "k1", "failed", 60).await;
        hand_off(&db, "d2", &t[1], "k1", "interrupted", 60).await;
        hand_off(&db, "d4", &t[2], "k1", "failed", 60).await;
        hand_off(&db, "d5", &t[3], "k1", "failed", 60).await;
        hand_off(&db, "d6", &t[4], "k2", "failed", 60).await;
        assert_eq!(failed_ids(&db).await, vec!["d1", "d2", "d4", "d5", "d6"]);
        // A later reply of k1 in t0 finished: d1 is over (only there, only k1's)
        let closed = db.close_failed_in_chat("k1", None, None, Some(&t[0])).await.unwrap();
        assert_eq!(closed, vec![("delegation".to_string(), "d1".to_string())]);
        assert!(db.close_failed_in_chat("k1", None, None, Some(&t[0])).await.unwrap().is_empty(), "closes once");
        assert_eq!(db.attention_closed_reason("delegation", "d1").await.unwrap().as_deref(), Some(CLOSED_LATER_REPLY));
        // Retried: a later hand-off to k1 in t1
        hand_off(&db, "d3", &t[1], "k1", "queued", 1).await;
        // Dismissed by the person
        assert_eq!(db.attention_item_owner("delegation", "d4").await.unwrap().as_deref(), Some("k1"));
        assert!(db.close_attention("delegation", "d4", Some("k1"), CLOSED_DISMISSED).await.unwrap());
        assert!(!db.close_attention("delegation", "d4", Some("k1"), CLOSED_DISMISSED).await.unwrap());
        // Its thread was deleted; its bot was deleted
        sqlx::query("DELETE FROM threads WHERE id = ?").bind(&t[3]).execute(&db.pool).await.unwrap();
        sqlx::query("DELETE FROM kwants WHERE id = 'k2'").execute(&db.pool).await.unwrap();
        assert!(failed_ids(&db).await.is_empty());
    }

    /// 22i close rules for a failed routine run: a later reply of its bot in the routine's chat,
    /// a later run of the routine, the person's Dismiss, or its bot gone.
    #[tokio::test]
    async fn a_failed_routine_run_closes_on_a_later_reply_a_rerun_dismiss_or_a_gone_bot() {
        let tmp = tempfile::tempdir().unwrap();
        let db = Database::init(tmp.path()).await.unwrap();
        bot(&db, "k1").await;
        bot(&db, "k2").await;
        for (id, kwant) in [("rt1", "k1"), ("rt2", "k1"), ("rt3", "k1"), ("rt4", "k2")] {
            sqlx::query("INSERT INTO routines (id, kwant_id, name, instructions, schedule, space_id) VALUES (?, ?, ?, 'do it', '0 9 * * *', 'spc_default')")
                .bind(id)
                .bind(kwant)
                .bind(format!("routine {id}"))
                .execute(&db.pool)
                .await
                .unwrap();
        }
        run(&db, "r1", "rt1", "error", 60).await;
        run(&db, "r2", "rt2", "error", 60).await;
        run(&db, "r3", "rt3", "error", 60).await;
        run(&db, "r4", "rt4", "error", 60).await;
        assert_eq!(failed_ids(&db).await, vec!["r1", "r2", "r3", "r4"]);
        // A reply in another chat closes nothing; one in the routines' chat (k1's direct) closes k1's
        assert!(db.close_failed_in_chat("k1", Some("c-other"), None, None).await.unwrap().is_empty());
        let closed = db.close_failed_in_chat("k1", None, Some("k1"), None).await.unwrap();
        assert_eq!(closed.len(), 3);
        sqlx::query("DELETE FROM attention_closed WHERE ref_id IN ('r2', 'r3')").execute(&db.pool).await.unwrap();
        assert_eq!(failed_ids(&db).await, vec!["r2", "r3", "r4"]);
        // Run again (any later run that wasn't skipped)
        run(&db, "r2b", "rt2", "ok", 1).await;
        // Dismissed
        assert_eq!(db.attention_item_owner("routine_run", "r3").await.unwrap().as_deref(), Some("k1"));
        db.close_attention("routine_run", "r3", Some("k1"), CLOSED_DISMISSED).await.unwrap();
        // Its bot is gone
        sqlx::query("DELETE FROM kwants WHERE id = 'k2'").execute(&db.pool).await.unwrap();
        assert!(failed_ids(&db).await.is_empty());
        assert_eq!(db.attention_item_owner("routine_run", "r2b").await.unwrap(), None, "only a failed run is dismissable");
    }

    /// 22i: a hand-off still running past the stuck threshold closes when it ends (as before) or
    /// when the person dismisses it.
    #[tokio::test]
    async fn a_stuck_hand_off_closes_when_it_ends_or_is_dismissed() {
        let tmp = tempfile::tempdir().unwrap();
        let db = Database::init(tmp.path()).await.unwrap();
        bot(&db, "k1").await;
        let t = db.create_thread("lead", "t", "delegation", None, "lead").await.unwrap().id;
        let t2 = db.create_thread("lead", "t2", "delegation", None, "lead").await.unwrap().id;
        hand_off(&db, "s1", &t, "k1", "running", 120).await;
        hand_off(&db, "s2", &t2, "k1", "running", 120).await;
        assert_eq!(failed_ids(&db).await, vec!["s1", "s2"]);
        db.close_attention("delegation", "s1", Some("k1"), CLOSED_DISMISSED).await.unwrap();
        sqlx::query("UPDATE delegations SET status = 'completed', ended_at = ? WHERE id = 's2'").bind(Utc::now()).execute(&db.pool).await.unwrap();
        assert!(failed_ids(&db).await.is_empty());
    }

    /// 22i: the person's Dismiss closes a stop or a failed hand-off once, says so on a second try,
    /// refuses what has its own answer and knows nothing of an unknown id.
    #[tokio::test]
    async fn dismiss_closes_a_stop_or_failed_item_once_and_refuses_what_has_its_own_answer() {
        use crate::db::waiting::NewWaitingItem;
        let tmp = tempfile::tempdir().unwrap();
        let db = Database::init(tmp.path()).await.unwrap();
        bot(&db, "k1").await;
        let (stop, _) = db.add_stopped("k1", None, Some("k1"), None, Some("m1"), "The model sent nothing for 120 s", None).await.unwrap();
        let ask = NewWaitingItem {
            kwant_id: "k1".into(),
            channel_id: None,
            direct_kwant_id: Some("k1".into()),
            thread_id: None,
            question_id: Some("q1".into()),
            message_id: None,
            question: "Which city?".into(),
            due_at: None,
        };
        let asked = db.add_waiting(&ask).await.unwrap().unwrap();
        let t = db.create_thread("lead", "t", "delegation", None, "lead").await.unwrap().id;
        hand_off(&db, "d1", &t, "k1", "failed", 5).await;
        assert_eq!(failed_ids(&db).await, vec!["d1"]);

        match db.dismiss_attention(&stop.id, None).await.unwrap() {
            Dismissed::Closed { kind, kwant_id, waiting } => {
                assert_eq!((kind.as_str(), kwant_id.as_deref()), ("waiting", Some("k1")));
                assert_eq!(waiting.unwrap().status, "closed");
            }
            other => panic!("expected Closed, got {other:?}"),
        }
        assert_eq!(db.get_waiting(&stop.id).await.unwrap().unwrap().status, "closed");
        assert_eq!(db.attention_closed_reason("waiting", &stop.id).await.unwrap().as_deref(), Some(CLOSED_DISMISSED), "the audit row");
        assert!(matches!(db.dismiss_attention(&stop.id, Some("stopped")).await.unwrap(), Dismissed::AlreadyClosed { .. }));

        // A failed hand-off, found by id alone or with its kind
        assert!(matches!(db.dismiss_attention("d1", None).await.unwrap(), Dismissed::Closed { ref kind, .. } if kind == "delegation"));
        assert!(failed_ids(&db).await.is_empty());
        assert!(matches!(db.dismiss_attention("d1", Some("delegation")).await.unwrap(), Dismissed::AlreadyClosed { .. }));

        // A question's entry, a question, an approval: answered in their own card, so the bot hears
        assert!(matches!(db.dismiss_attention(&asked.id, None).await.unwrap(), Dismissed::Refused("use_question_answer")));
        assert_eq!(db.get_waiting(&asked.id).await.unwrap().unwrap().status, "open");
        assert!(matches!(db.dismiss_attention("a1", Some("approval")).await.unwrap(), Dismissed::Refused("use_approval_decide")));
        assert!(matches!(db.dismiss_attention("h1", Some("human_step")).await.unwrap(), Dismissed::Refused("use_step_decline")));
        assert!(matches!(db.dismiss_attention("x", Some("nonsense")).await.unwrap(), Dismissed::BadKind));
        assert!(matches!(db.dismiss_attention("nope", None).await.unwrap(), Dismissed::NotFound));
        // A finished hand-off isn't in Needs attention: nothing to dismiss
        hand_off(&db, "d2", &t, "k1", "completed", 5).await;
        assert!(matches!(db.dismiss_attention("d2", Some("delegation")).await.unwrap(), Dismissed::NotFound));
    }

    /// 22i close rules for the sources that close by their own answer: a question leaves Needs you
    /// when answered or cancelled, an approval once it isn't pending (allowed, denied, withdrawn or
    /// expired), a browser step when done or declined; and a deleted bot takes its questions along.
    #[tokio::test]
    async fn a_question_approval_or_browser_step_closes_by_its_own_answer() {
        use crate::db::human_steps::HumanStepPlace;
        use crate::db::questions::QuestionPlace;
        let tmp = tempfile::tempdir().unwrap();
        let db = Database::init(tmp.path()).await.unwrap();
        let space = SpaceId::default_space();
        bot(&db, "k1").await;
        bot(&db, "k2").await;
        let place = |k: &str| QuestionPlace { kwant_id: k.into(), channel_id: None, direct_kwant_id: Some(k.into()), thread_id: None, turn_id: None };
        let q1 = db.create_question(&place("k1"), "Which city?", &[]).await.unwrap();
        let q2 = db.create_question(&place("k1"), "Which day?", &[]).await.unwrap();
        let q3 = db.create_question(&place("k2"), "Which colour?", &[]).await.unwrap();
        let open = |rows: Vec<AwayRow>| rows.into_iter().map(|r| r.id).collect::<Vec<_>>();
        assert_eq!(open(db.away_open_questions(&space, 50).await.unwrap()).len(), 3);
        db.close_question(&q1.id, "answered", Some("Paris")).await.unwrap();
        db.close_question(&q2.id, "cancelled", None).await.unwrap();
        assert_eq!(open(db.away_open_questions(&space, 50).await.unwrap()), vec![q3.id.clone()]);
        assert_eq!(db.away_count_open_questions(&space).await.unwrap(), 1);
        db.delete_kwant_and_history("k2", "k1").await.unwrap();
        assert!(db.away_open_questions(&space, 50).await.unwrap().is_empty());

        // An approval: listed only while pending (and live in the broker: the caller's ids)
        for (id, status) in [("a1", "pending"), ("a2", "approved"), ("a3", "denied")] {
            sqlx::query("INSERT INTO approvals (id, turn_id, kwant_id, tool_name, parameters, status) VALUES (?, 't', 'k1', 'shell', '{}', ?)")
                .bind(id)
                .bind(status)
                .execute(&db.pool)
                .await
                .unwrap();
        }
        let ids: Vec<String> = ["a1", "a2", "a3"].map(String::from).to_vec();
        assert_eq!(open(db.away_pending_approvals(&ids).await.unwrap()), vec!["a1"]);
        sqlx::query("UPDATE approvals SET status = 'expired' WHERE id = 'a1'").execute(&db.pool).await.unwrap();
        assert!(db.away_pending_approvals(&ids).await.unwrap().is_empty());

        // A browser step: open until done or declined
        let step_place = HumanStepPlace {
            kwant_id: "k1".into(),
            channel_id: None,
            direct_kwant_id: Some("k1".into()),
            thread_id: None,
            turn_id: None,
            sandbox_target: None,
            page_key: None,
        };
        db.create_human_step_with_card(&step_place, "captcha", "Solve the CAPTCHA", None, None, "auto", |_| None).await.unwrap();
        let steps = db.list_human_steps(Some("open"), None, Some(&space)).await.unwrap();
        assert_eq!(steps.len(), 1);
        assert!(db.close_human_step(&steps[0].id, "declined").await.unwrap());
        assert!(db.list_human_steps(Some("open"), None, Some(&space)).await.unwrap().is_empty());
    }


    /// A message as kwnt saves it: `created_at` is SQLite's `CURRENT_TIMESTAMP` text
    /// (`2026-11-21 10:00:00`), never sqlx's `2026-11-21T10:00:00…`.
    async fn said_at(db: &Database, id: &str, role: &str, sender: &str, text: &str, at: &str, meta: Option<&str>) {
        sqlx::query("INSERT INTO messages (id, direct_kwant_id, sender_role, sender_id, text, meta, created_at) VALUES (?, 'k1', ?, ?, ?, ?, ?)")
            .bind(id)
            .bind(role)
            .bind(sender)
            .bind(text)
            .bind(meta)
            .bind(at)
            .execute(&db.pool)
            .await
            .unwrap();
    }

    fn utc(at: &str) -> DateTime<Utc> {
        chrono::NaiveDateTime::parse_from_str(at, "%Y-%m-%d %H:%M:%S").unwrap().and_utc()
    }

    /// 33d: the request a `Done:` closes is the person's message before it, not the newest one of
    /// that day (a bound `DateTime` is written with a `T`, and `' ' < 'T'`).
    #[tokio::test]
    async fn the_persons_message_before_is_not_the_newest_of_that_day() {
        let tmp = tempfile::tempdir().unwrap();
        let db = Database::init(tmp.path()).await.unwrap();
        said_at(&db, "ask", "user", "user", "Plan a 3-day trip", "2026-11-21 10:00:00", None).await;
        said_at(&db, "later", "user", "user", "And a packing list?", "2026-11-21 10:02:00", None).await;
        let before = db.last_person_message_before("k1", utc("2026-11-21 10:01:00")).await.unwrap();
        assert_eq!(before.map(|m| m.id).as_deref(), Some("ask"));
        // The same second counts as before
        let same = db.last_person_message_before("k1", utc("2026-11-21 10:02:00")).await.unwrap();
        assert_eq!(same.map(|m| m.id).as_deref(), Some("later"));
        assert!(db.last_person_message_before("k1", utc("2026-11-21 09:59:59")).await.unwrap().is_none());
    }

    /// 33d: a `Done:` reply later on the day of `since` is listed (it compared before `since`).
    #[tokio::test]
    async fn a_done_reply_later_the_same_day_is_after_since() {
        let tmp = tempfile::tempdir().unwrap();
        let db = Database::init(tmp.path()).await.unwrap();
        let done = r#"{"chain_done": {"text": "plan saved"}}"#;
        said_at(&db, "early", "kwant", "Ann", "Done: list made", "2026-11-21 08:00:00", Some(done)).await;
        said_at(&db, "late", "kwant", "Ann", "Done: plan saved", "2026-11-21 10:05:00", Some(done)).await;
        let rows = db.done_replies(utc("2026-11-21 09:00:00"), 10).await.unwrap();
        assert_eq!(rows.iter().map(|r| r.id.as_str()).collect::<Vec<_>>(), ["late"]);
    }

    /// 33d: a hand-off made in the same second as the `Done:` reply (whole seconds) still counts
    /// (its time keeps a fraction, which compared after the reply's); the next second doesn't.
    #[tokio::test]
    async fn a_hand_off_in_the_done_replys_own_second_is_a_member() {
        let tmp = tempfile::tempdir().unwrap();
        let db = Database::init(tmp.path()).await.unwrap();
        let reply = utc("2026-11-21 10:05:07");
        for (id, ms) in [("before", -400), ("same", 650), ("next", 1200)] {
            sqlx::query("INSERT INTO delegations (id, thread_id, lead_kwant_id, specialist_kwant_id, task, title, status, created_at) VALUES (?, 't', 'lead', 'spec', 'task', 'job', 'completed', ?)")
                .bind(id)
                .bind(reply + chrono::Duration::milliseconds(ms))
                .execute(&db.pool)
                .await
                .unwrap();
        }
        let ids = |v: Vec<Delegation>| v.into_iter().map(|d| d.id).collect::<Vec<_>>();
        let got = ids(db.lead_delegations("lead", utc("2026-11-21 10:05:00"), reply).await.unwrap());
        assert_eq!(got, ["same", "before"]);
        // The request's own second is in too (`from` is whole seconds as well)
        let got = ids(db.lead_delegations("lead", utc("2026-11-21 10:05:06"), utc("2026-11-21 10:05:06")).await.unwrap());
        assert_eq!(got, ["before"]);
    }

    /// 33d: a task's result reply is the bot's first reply from its review within the slack, not
    /// its newest reply of that day.
    #[tokio::test]
    async fn a_tasks_result_reply_is_the_one_after_its_review_not_the_newest_of_that_day() {
        let tmp = tempfile::tempdir().unwrap();
        let db = Database::init(tmp.path()).await.unwrap();
        said_at(&db, "morning", "kwant", "Ann", "Earlier work", "2026-11-21 09:00:00", None).await;
        said_at(&db, "result", "kwant", "Ann", "Here is the result", "2026-11-21 10:03:00", None).await;
        said_at(&db, "evening", "kwant", "Ann", "Something else", "2026-11-21 18:00:00", None).await;
        let review = utc("2026-11-21 10:02:30");
        let m = db.task_result_message("k1", "Ann", review, chrono::Duration::minutes(5)).await.unwrap();
        assert_eq!(m.map(|m| m.id).as_deref(), Some("result"));
        // Nothing within the slack: the last reply before the review
        let m = db.task_result_message("k1", "Ann", utc("2026-11-21 12:00:00"), chrono::Duration::minutes(5)).await.unwrap();
        assert_eq!(m.map(|m| m.id).as_deref(), Some("result"));
    }
}
