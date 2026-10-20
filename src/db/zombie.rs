//! Zombie Mission Hunter's tables: the cases it found (`zombie_cases`), the mailboxes and GitHub
//! accounts it reads (`zombie_sources`, never a secret: those are in the password vault), what
//! the person muted (`zombie_mutes`) and the weekly reviews (`zombie_reviews`).

use super::{Database, DbError};
use chrono::{DateTime, Utc};
use serde::Serialize;
use sqlx::FromRow;

/// `open` (in the queue), `snoozed` (until `snoozed_until`), `dismissed` (never comes back),
/// `waiting_reply` (the person followed up; reopens if nobody answers), `closed` (done or resolved).
pub const CASE_STATUSES: [&str; 5] = ["open", "snoozed", "dismissed", "waiting_reply", "closed"];

#[derive(Debug, Clone, FromRow, PartialEq)]
pub struct CaseRow {
    pub id: String,
    pub dedupe_key: String,
    pub source: String,
    pub source_id: String,
    pub source_ref: String,
    pub kind: String,
    pub conv_kind: String,
    pub title: String,
    pub counterpart: Option<String>,
    pub counterpart_name: Option<String>,
    pub reference: Option<String>,
    pub link: Option<String>,
    pub why: String,
    /// JSON array of `zombie::Evidence`.
    pub evidence: String,
    /// JSON `zombie::draft::Draft`.
    pub draft: Option<String>,
    pub status: String,
    pub snoozed_until: Option<DateTime<Utc>>,
    pub waiting_since: Option<DateTime<Utc>>,
    pub follow_ups: i64,
    pub first_seen: DateTime<Utc>,
    pub last_seen: DateTime<Utc>,
    pub last_activity_at: DateTime<Utc>,
    pub status_changed_at: DateTime<Utc>,
    /// Why it closed: `done` (the person), `resolved` (an answer came), `source_removed`.
    pub resolution: Option<String>,
    pub dismiss_reason: Option<String>,
    pub deadline: Option<String>,
    pub reply_to_message_id: Option<String>,
    pub space_id: Option<String>,
    /// Wiped by retention: only the key and status are left.
    pub redacted: bool,
}

#[derive(Debug, Clone, FromRow, PartialEq, Serialize)]
pub struct SourceRow {
    pub id: String,
    /// `imap` or `github`.
    pub kind: String,
    pub label: String,
    /// JSON settings, never a password or token.
    pub config: String,
    pub enabled: bool,
    pub last_scan_at: Option<DateTime<Utc>>,
    /// `ok`, `error`, or null before the first scan.
    pub last_status: Option<String>,
    pub last_error: Option<String>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, FromRow, PartialEq, Serialize)]
pub struct ReviewRow {
    pub id: String,
    /// `schedule` or `manual`.
    pub trigger: String,
    pub started_at: DateTime<Utc>,
    pub ended_at: Option<DateTime<Utc>>,
    /// `running`, `ok`, `error`.
    pub status: String,
    /// Open cases after the scan.
    pub found: i64,
    /// JSON array of the case ids the review showed.
    pub shown: String,
    pub error: Option<String>,
}


/// A chat thread whose last message is the person's: the bot never answered.
#[derive(Debug, Clone, FromRow)]
pub struct KwantThreadRow {
    pub thread_id: String,
    pub title: String,
    pub kwant_id: String,
    pub kwant_name: String,
    pub space_id: Option<String>,
    pub text: String,
    pub created_at: DateTime<Utc>,
}

/// A task handed to a bot that failed or was cut short, with no later run of the same task.
#[derive(Debug, Clone, FromRow)]
pub struct KwantDelegationRow {
    pub id: String,
    pub thread_id: String,
    pub title: String,
    pub task: String,
    pub status: String,
    pub error: Option<String>,
    pub kwant_name: String,
    pub space_id: Option<String>,
    pub created_at: DateTime<Utc>,
    pub ended_at: DateTime<Utc>,
}

/// A bot's `ask_user` question still open.
#[derive(Debug, Clone, FromRow)]
pub struct KwantQuestionRow {
    pub id: String,
    pub question: String,
    pub thread_id: Option<String>,
    pub kwant_id: String,
    pub kwant_name: String,
    pub space_id: Option<String>,
    pub created_at: DateTime<Utc>,
}

const CASE_COLUMNS: &str = "id, dedupe_key, source, source_id, source_ref, kind, conv_kind, title, counterpart, \
    counterpart_name, reference, link, why, evidence, draft, status, snoozed_until, waiting_since, follow_ups, \
    first_seen, last_seen, last_activity_at, status_changed_at, resolution, dismiss_reason, deadline, \
    reply_to_message_id, space_id, redacted";

impl Database {
    pub(super) async fn migrate_zombie(&self) -> Result<(), DbError> {
        for stmt in [
            "CREATE TABLE IF NOT EXISTS zombie_cases (
                id TEXT PRIMARY KEY,
                dedupe_key TEXT NOT NULL UNIQUE,
                source TEXT NOT NULL CHECK (source IN ('kwant', 'imap', 'github')),
                source_id TEXT NOT NULL,
                source_ref TEXT NOT NULL,
                kind TEXT NOT NULL,
                conv_kind TEXT NOT NULL,
                title TEXT NOT NULL,
                counterpart TEXT,
                counterpart_name TEXT,
                reference TEXT,
                link TEXT,
                why TEXT NOT NULL,
                evidence TEXT NOT NULL DEFAULT '[]',
                draft TEXT,
                status TEXT NOT NULL CHECK (status IN ('open', 'snoozed', 'dismissed', 'waiting_reply', 'closed')),
                snoozed_until TIMESTAMP,
                waiting_since TIMESTAMP,
                follow_ups INTEGER NOT NULL DEFAULT 0,
                first_seen TIMESTAMP NOT NULL,
                last_seen TIMESTAMP NOT NULL,
                last_activity_at TIMESTAMP NOT NULL,
                status_changed_at TIMESTAMP NOT NULL,
                resolution TEXT,
                dismiss_reason TEXT,
                deadline TEXT,
                reply_to_message_id TEXT,
                space_id TEXT,
                redacted INTEGER NOT NULL DEFAULT 0
            )",
            "CREATE INDEX IF NOT EXISTS zombie_cases_status ON zombie_cases (status)",
            "CREATE INDEX IF NOT EXISTS zombie_cases_source ON zombie_cases (source_id)",
            "CREATE TABLE IF NOT EXISTS zombie_sources (
                id TEXT PRIMARY KEY,
                kind TEXT NOT NULL CHECK (kind IN ('imap', 'github')),
                label TEXT NOT NULL,
                config TEXT NOT NULL,
                enabled INTEGER NOT NULL DEFAULT 1,
                last_scan_at TIMESTAMP,
                last_status TEXT,
                last_error TEXT,
                created_at TIMESTAMP NOT NULL
            )",
            "CREATE TABLE IF NOT EXISTS zombie_mutes (
                source_id TEXT NOT NULL,
                scope TEXT NOT NULL CHECK (scope IN ('thread', 'sender')),
                value TEXT NOT NULL,
                created_at TIMESTAMP NOT NULL,
                PRIMARY KEY (source_id, scope, value)
            )",
            "CREATE TABLE IF NOT EXISTS zombie_reviews (
                id TEXT PRIMARY KEY,
                trigger TEXT NOT NULL,
                started_at TIMESTAMP NOT NULL,
                ended_at TIMESTAMP,
                status TEXT NOT NULL,
                found INTEGER NOT NULL DEFAULT 0,
                shown TEXT NOT NULL DEFAULT '[]',
                error TEXT
            )",
        ] {
            sqlx::query(stmt).execute(&self.pool).await?;
        }
        Ok(())
    }

    pub async fn zombie_case(&self, id: &str) -> Result<Option<CaseRow>, DbError> {
        Ok(sqlx::query_as(&format!("SELECT {CASE_COLUMNS} FROM zombie_cases WHERE id = ?"))
            .bind(id)
            .fetch_optional(&self.pool)
            .await?)
    }

    pub async fn zombie_case_by_key(&self, key: &str) -> Result<Option<CaseRow>, DbError> {
        Ok(sqlx::query_as(&format!("SELECT {CASE_COLUMNS} FROM zombie_cases WHERE dedupe_key = ?"))
            .bind(key)
            .fetch_optional(&self.pool)
            .await?)
    }

    /// Cases with `status` (all but redacted ones when `None`), newest activity first.
    pub async fn zombie_cases(&self, status: Option<&str>) -> Result<Vec<CaseRow>, DbError> {
        let sql = format!(
            "SELECT {CASE_COLUMNS} FROM zombie_cases WHERE redacted = 0 AND (? IS NULL OR status = ?) \
             ORDER BY last_activity_at DESC LIMIT 500"
        );
        Ok(sqlx::query_as(&sql).bind(status).bind(status).fetch_all(&self.pool).await?)
    }

    /// Every case of one source (for the scan to reconcile against).
    pub async fn zombie_cases_of_source(&self, source_id: &str) -> Result<Vec<CaseRow>, DbError> {
        Ok(sqlx::query_as(&format!("SELECT {CASE_COLUMNS} FROM zombie_cases WHERE source_id = ?"))
            .bind(source_id)
            .fetch_all(&self.pool)
            .await?)
    }

    /// Inserts `c`, or replaces every column of the case with its id.
    pub async fn zombie_put_case(&self, c: &CaseRow) -> Result<(), DbError> {
        sqlx::query(&format!(
            "INSERT INTO zombie_cases ({CASE_COLUMNS}) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
             ON CONFLICT(id) DO UPDATE SET
                source_ref = excluded.source_ref, kind = excluded.kind, conv_kind = excluded.conv_kind,
                title = excluded.title, counterpart = excluded.counterpart, counterpart_name = excluded.counterpart_name,
                reference = excluded.reference, link = excluded.link, why = excluded.why, evidence = excluded.evidence,
                draft = excluded.draft, status = excluded.status, snoozed_until = excluded.snoozed_until,
                waiting_since = excluded.waiting_since, follow_ups = excluded.follow_ups, last_seen = excluded.last_seen,
                last_activity_at = excluded.last_activity_at, status_changed_at = excluded.status_changed_at,
                resolution = excluded.resolution, dismiss_reason = excluded.dismiss_reason, deadline = excluded.deadline,
                reply_to_message_id = excluded.reply_to_message_id, space_id = excluded.space_id, redacted = excluded.redacted"
        ))
        .bind(&c.id)
        .bind(&c.dedupe_key)
        .bind(&c.source)
        .bind(&c.source_id)
        .bind(&c.source_ref)
        .bind(&c.kind)
        .bind(&c.conv_kind)
        .bind(&c.title)
        .bind(&c.counterpart)
        .bind(&c.counterpart_name)
        .bind(&c.reference)
        .bind(&c.link)
        .bind(&c.why)
        .bind(&c.evidence)
        .bind(&c.draft)
        .bind(&c.status)
        .bind(c.snoozed_until)
        .bind(c.waiting_since)
        .bind(c.follow_ups)
        .bind(c.first_seen)
        .bind(c.last_seen)
        .bind(c.last_activity_at)
        .bind(c.status_changed_at)
        .bind(&c.resolution)
        .bind(&c.dismiss_reason)
        .bind(&c.deadline)
        .bind(&c.reply_to_message_id)
        .bind(&c.space_id)
        .bind(c.redacted)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// Removes every case of a source the person deleted (their text came from it).
    pub async fn zombie_delete_cases_of_source(&self, source_id: &str) -> Result<u64, DbError> {
        Ok(sqlx::query("DELETE FROM zombie_cases WHERE source_id = ?").bind(source_id).execute(&self.pool).await?.rows_affected())
    }

    /// Wipes closed and dismissed cases whose status changed before `before` down to their key and
    /// status (so a dismissed thread stays dismissed), and drops reviews older than it.
    pub async fn zombie_apply_retention(&self, before: DateTime<Utc>) -> Result<u64, DbError> {
        let wiped = sqlx::query(
            "UPDATE zombie_cases SET title = '', counterpart = NULL, counterpart_name = NULL, reference = NULL,
                link = NULL, why = '', evidence = '[]', draft = NULL, dismiss_reason = NULL, deadline = NULL,
                reply_to_message_id = NULL, redacted = 1
             WHERE redacted = 0 AND status IN ('closed', 'dismissed') AND status_changed_at < ?",
        )
        .bind(before)
        .execute(&self.pool)
        .await?
        .rows_affected();
        sqlx::query("DELETE FROM zombie_reviews WHERE started_at < ?").bind(before).execute(&self.pool).await?;
        Ok(wiped)
    }

    pub async fn zombie_sources(&self) -> Result<Vec<SourceRow>, DbError> {
        Ok(sqlx::query_as("SELECT * FROM zombie_sources ORDER BY created_at").fetch_all(&self.pool).await?)
    }

    pub async fn zombie_source(&self, id: &str) -> Result<Option<SourceRow>, DbError> {
        Ok(sqlx::query_as("SELECT * FROM zombie_sources WHERE id = ?").bind(id).fetch_optional(&self.pool).await?)
    }

    pub async fn zombie_put_source(&self, s: &SourceRow) -> Result<(), DbError> {
        sqlx::query(
            "INSERT INTO zombie_sources (id, kind, label, config, enabled, last_scan_at, last_status, last_error, created_at)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)
             ON CONFLICT(id) DO UPDATE SET label = excluded.label, config = excluded.config, enabled = excluded.enabled,
                last_scan_at = excluded.last_scan_at, last_status = excluded.last_status, last_error = excluded.last_error",
        )
        .bind(&s.id)
        .bind(&s.kind)
        .bind(&s.label)
        .bind(&s.config)
        .bind(s.enabled)
        .bind(s.last_scan_at)
        .bind(&s.last_status)
        .bind(&s.last_error)
        .bind(s.created_at)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn zombie_record_scan(&self, id: &str, at: DateTime<Utc>, error: Option<&str>) -> Result<(), DbError> {
        sqlx::query("UPDATE zombie_sources SET last_scan_at = ?, last_status = ?, last_error = ? WHERE id = ?")
            .bind(at)
            .bind(if error.is_some() { "error" } else { "ok" })
            .bind(error)
            .bind(id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    pub async fn zombie_delete_source(&self, id: &str) -> Result<bool, DbError> {
        sqlx::query("DELETE FROM zombie_mutes WHERE source_id = ?").bind(id).execute(&self.pool).await?;
        Ok(sqlx::query("DELETE FROM zombie_sources WHERE id = ?").bind(id).execute(&self.pool).await?.rows_affected() > 0)
    }

    /// Mutes a thread or sender of a source: their conversations never become cases again.
    pub async fn zombie_mute(&self, source_id: &str, scope: &str, value: &str) -> Result<(), DbError> {
        sqlx::query("INSERT OR IGNORE INTO zombie_mutes (source_id, scope, value, created_at) VALUES (?, ?, ?, ?)")
            .bind(source_id)
            .bind(scope)
            .bind(value.to_lowercase())
            .bind(Utc::now())
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    pub async fn zombie_unmute(&self, source_id: &str, scope: &str, value: &str) -> Result<(), DbError> {
        sqlx::query("DELETE FROM zombie_mutes WHERE source_id = ? AND scope = ? AND value = ?")
            .bind(source_id)
            .bind(scope)
            .bind(value.to_lowercase())
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    /// `(scope, value)` pairs muted for a source.
    pub async fn zombie_mutes(&self, source_id: &str) -> Result<Vec<(String, String)>, DbError> {
        Ok(sqlx::query_as("SELECT scope, value FROM zombie_mutes WHERE source_id = ?").bind(source_id).fetch_all(&self.pool).await?)
    }

    pub async fn zombie_start_review(&self, id: &str, trigger: &str, at: DateTime<Utc>) -> Result<(), DbError> {
        sqlx::query("INSERT INTO zombie_reviews (id, trigger, started_at, status) VALUES (?, ?, ?, 'running')")
            .bind(id)
            .bind(trigger)
            .bind(at)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    pub async fn zombie_finish_review(&self, id: &str, found: i64, shown: &[String], error: Option<&str>) -> Result<(), DbError> {
        sqlx::query("UPDATE zombie_reviews SET ended_at = ?, status = ?, found = ?, shown = ?, error = ? WHERE id = ?")
            .bind(Utc::now())
            .bind(if error.is_some() { "error" } else { "ok" })
            .bind(found)
            .bind(serde_json::to_string(shown).unwrap_or_else(|_| "[]".into()))
            .bind(error)
            .bind(id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    pub async fn zombie_reviews(&self, limit: i64) -> Result<Vec<ReviewRow>, DbError> {
        Ok(sqlx::query_as("SELECT * FROM zombie_reviews ORDER BY started_at DESC LIMIT ?").bind(limit).fetch_all(&self.pool).await?)
    }

    /// Chat threads active since `since` whose last message (system notices aside) is the person's.
    pub async fn zombie_kwant_threads(&self, since: DateTime<Utc>) -> Result<Vec<KwantThreadRow>, DbError> {
        Ok(sqlx::query_as(
            "SELECT t.id AS thread_id, t.title, t.kwant_id, k.name AS kwant_name, k.space_id, m.text, m.created_at
             FROM threads t
             JOIN kwants k ON k.id = t.kwant_id
             JOIN messages m ON m.id = (
                SELECT id FROM messages WHERE thread_id = t.id AND sender_role != 'system'
                ORDER BY created_at DESC LIMIT 1)
             WHERE t.kind = 'chat' AND t.archived = 0 AND m.sender_role = 'user' AND m.created_at >= ?
             ORDER BY m.created_at DESC LIMIT 200",
        )
        .bind(since)
        .fetch_all(&self.pool)
        .await?)
    }

    /// Failed or interrupted tasks since `since` that were never run again (same task text) after.
    pub async fn zombie_kwant_delegations(&self, since: DateTime<Utc>) -> Result<Vec<KwantDelegationRow>, DbError> {
        Ok(sqlx::query_as(
            "SELECT d.id, d.thread_id, d.title, d.task, d.status, d.error, k.name AS kwant_name, k.space_id,
                d.created_at, COALESCE(d.ended_at, d.created_at) AS ended_at
             FROM delegations d
             JOIN kwants k ON k.id = d.specialist_kwant_id
             WHERE d.status IN ('failed', 'interrupted') AND COALESCE(d.ended_at, d.created_at) >= ?
               AND NOT EXISTS (
                SELECT 1 FROM delegations d2
                WHERE d2.task = d.task AND d2.created_at > d.created_at AND d2.status IN ('queued', 'running', 'completed'))
             ORDER BY d.created_at DESC LIMIT 200",
        )
        .bind(since)
        .fetch_all(&self.pool)
        .await?)
    }

    /// Questions bots asked since `since` that are still open.
    pub async fn zombie_kwant_questions(&self, since: DateTime<Utc>) -> Result<Vec<KwantQuestionRow>, DbError> {
        Ok(sqlx::query_as(
            "SELECT q.id, q.question, q.thread_id, q.kwant_id, k.name AS kwant_name, k.space_id, q.created_at
             FROM questions q
             JOIN kwants k ON k.id = q.kwant_id
             WHERE q.status = 'open' AND q.created_at >= ?
             ORDER BY q.created_at DESC LIMIT 200",
        )
        .bind(since)
        .fetch_all(&self.pool)
        .await?)
    }
}
