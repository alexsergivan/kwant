//! G1 line notes (0.9.0 wave 1, Juno; `server::dev::line_notes`, release-090/g1-api.md).
//!
//! `dev_line_notes` holds the person's notes on lines of a review card's diff, per PR (`repo`,
//! `ref` = `pr:N`), so they survive reloads and show on the phone. A note is written on one head
//! (`head_sha`) and keeps the lines it was written on (`code`); the server works out where they
//! sit in a newer head. `deleted_at` is a soft delete the person can undo for
//! [`RESTORE_SECS`]; after that the row is purged. `dev_line_note_sends` remembers each send by the
//! client's `send_id` for 24 h, so a retry never posts the batch twice. Kept outside exports like
//! the other Dev mode tables. Add-only.

use super::{Database, DbError};
use sqlx::FromRow;

const TABLES: &[&str] = &[
    "CREATE TABLE IF NOT EXISTS dev_line_notes (
        id TEXT PRIMARY KEY,
        space_id TEXT NOT NULL,
        repo TEXT NOT NULL,
        ref TEXT NOT NULL,
        card_id TEXT NOT NULL,
        path TEXT NOT NULL,
        start_side TEXT NOT NULL,
        start_line INTEGER NOT NULL,
        end_side TEXT NOT NULL,
        end_line INTEGER NOT NULL,
        code TEXT NOT NULL,
        text TEXT NOT NULL,
        state TEXT NOT NULL,
        head_sha TEXT NOT NULL,
        created_at INTEGER NOT NULL,
        updated_at INTEGER NOT NULL,
        sent_at INTEGER,
        message_id TEXT,
        checked_sha TEXT,
        deleted_at INTEGER
    )",
    "CREATE INDEX IF NOT EXISTS dev_line_notes_pr ON dev_line_notes (space_id, repo, ref, created_at)",
    "CREATE TABLE IF NOT EXISTS dev_line_note_sends (
        send_id TEXT PRIMARY KEY,
        space_id TEXT NOT NULL,
        repo TEXT NOT NULL,
        ref TEXT NOT NULL,
        ids TEXT,
        result TEXT NOT NULL,
        created_at INTEGER NOT NULL
    )",
];

/// The tables [`Database::delete_dev_mode_of_space`] clears with the rest of Dev mode.
pub const LINE_NOTE_TABLES: [&str; 2] = ["dev_line_notes", "dev_line_note_sends"];

/// Only the person sees it; it can be edited, deleted and sent.
pub const DRAFT: &str = "draft";
/// Sent to the author bot; waiting for its next push.
pub const SENT: &str = "sent";
/// The next push changed or deleted at least one of its lines.
pub const RESOLVED: &str = "resolved";
/// The next push left all its lines as they were.
pub const OPEN: &str = "open";

/// How long a deleted draft can be restored (the UI's Undo needs 10 s; slow phones get more).
pub const RESTORE_SECS: i64 = 60;
/// How long a send is remembered by its `send_id`.
pub const SEND_ID_HOURS: i64 = 24;

#[derive(Debug, Clone, FromRow, PartialEq)]
pub struct LineNoteRow {
    pub id: String,
    pub space_id: String,
    /// The repo key, as on the card.
    pub repo: String,
    /// The card's ref, `pr:N`.
    #[sqlx(rename = "ref")]
    pub ref_: String,
    /// The card it was written on (that head's card).
    pub card_id: String,
    pub path: String,
    /// `new` or `old`.
    pub start_side: String,
    pub start_line: i64,
    pub end_side: String,
    pub end_line: i64,
    /// The noted lines as written on, `[{kind, old, new, text}]`.
    pub code: String,
    pub text: String,
    /// [`DRAFT`], [`SENT`], [`RESOLVED`] or [`OPEN`].
    pub state: String,
    /// The head it was written on.
    pub head_sha: String,
    pub created_at: i64,
    pub updated_at: i64,
    pub sent_at: Option<i64>,
    pub message_id: Option<String>,
    /// The push a sent note was checked against.
    pub checked_sha: Option<String>,
    pub deleted_at: Option<i64>,
}

const COLS: &str = "id, space_id, repo, ref, card_id, path, start_side, start_line, end_side, end_line, code, text, state, head_sha, \
                    created_at, updated_at, sent_at, message_id, checked_sha, deleted_at";

#[derive(Debug, Clone, FromRow, PartialEq)]
pub struct LineNoteSendRow {
    pub send_id: String,
    pub space_id: String,
    pub repo: String,
    #[sqlx(rename = "ref")]
    pub ref_: String,
    /// The ids the request named, sorted, as JSON; none when it sent every draft.
    pub ids: Option<String>,
    /// The response the first send returned.
    pub result: String,
    pub created_at: i64,
}

fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

impl Database {
    pub(super) async fn migrate_line_notes(&self) -> Result<(), DbError> {
        for sql in TABLES {
            sqlx::query(sql).execute(&self.pool).await?;
        }
        Ok(())
    }

    pub async fn insert_line_note(&self, row: &LineNoteRow) -> Result<(), DbError> {
        sqlx::query(&format!("INSERT INTO dev_line_notes ({COLS}) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)"))
            .bind(&row.id)
            .bind(&row.space_id)
            .bind(&row.repo)
            .bind(&row.ref_)
            .bind(&row.card_id)
            .bind(&row.path)
            .bind(&row.start_side)
            .bind(row.start_line)
            .bind(&row.end_side)
            .bind(row.end_line)
            .bind(&row.code)
            .bind(&row.text)
            .bind(&row.state)
            .bind(&row.head_sha)
            .bind(row.created_at)
            .bind(row.updated_at)
            .bind(row.sent_at)
            .bind(&row.message_id)
            .bind(&row.checked_sha)
            .bind(row.deleted_at)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    /// One note, deleted or not.
    pub async fn line_note(&self, id: &str) -> Result<Option<LineNoteRow>, DbError> {
        Ok(sqlx::query_as(&format!("SELECT {COLS} FROM dev_line_notes WHERE id = ?")).bind(id).fetch_optional(&self.pool).await?)
    }

    /// A PR's notes that aren't deleted, oldest first.
    pub async fn line_notes_of_pr(&self, space_id: &str, repo: &str, ref_: &str) -> Result<Vec<LineNoteRow>, DbError> {
        Ok(sqlx::query_as(&format!(
            "SELECT {COLS} FROM dev_line_notes WHERE space_id = ? AND repo = ? AND ref = ? AND deleted_at IS NULL ORDER BY created_at, rowid"
        ))
        .bind(space_id)
        .bind(repo)
        .bind(ref_)
        .fetch_all(&self.pool)
        .await?)
    }

    /// `(repo, ref, drafts, total)` for every PR of a space with notes that aren't deleted.
    pub async fn line_note_counts(&self, space_id: &str) -> Result<Vec<(String, String, i64, i64)>, DbError> {
        Ok(sqlx::query_as(
            "SELECT repo, ref, SUM(CASE WHEN state = 'draft' THEN 1 ELSE 0 END), COUNT(*) FROM dev_line_notes
             WHERE space_id = ? AND deleted_at IS NULL GROUP BY repo, ref",
        )
        .bind(space_id)
        .fetch_all(&self.pool)
        .await?)
    }

    /// Sets a draft's text. False when it isn't a draft that's not deleted.
    pub async fn set_line_note_text(&self, id: &str, text: &str) -> Result<bool, DbError> {
        let r = sqlx::query("UPDATE dev_line_notes SET text = ?, updated_at = ? WHERE id = ? AND state = 'draft' AND deleted_at IS NULL")
            .bind(text)
            .bind(now_ms())
            .bind(id)
            .execute(&self.pool)
            .await?;
        Ok(r.rows_affected() == 1)
    }

    /// Soft-deletes a draft. False when it isn't a draft that's not deleted.
    pub async fn delete_line_note(&self, id: &str) -> Result<bool, DbError> {
        let now = now_ms();
        let r = sqlx::query("UPDATE dev_line_notes SET deleted_at = ?, updated_at = ? WHERE id = ? AND state = 'draft' AND deleted_at IS NULL")
            .bind(now)
            .bind(now)
            .bind(id)
            .execute(&self.pool)
            .await?;
        Ok(r.rows_affected() == 1)
    }

    /// Soft-deletes every draft of a PR: how many.
    pub async fn discard_line_note_drafts(&self, space_id: &str, repo: &str, ref_: &str) -> Result<u64, DbError> {
        let now = now_ms();
        let r = sqlx::query(
            "UPDATE dev_line_notes SET deleted_at = ?, updated_at = ? WHERE space_id = ? AND repo = ? AND ref = ? AND state = 'draft' AND deleted_at IS NULL",
        )
        .bind(now)
        .bind(now)
        .bind(space_id)
        .bind(repo)
        .bind(ref_)
        .execute(&self.pool)
        .await?;
        Ok(r.rows_affected())
    }

    /// Undoes a soft delete made in the last [`RESTORE_SECS`]. False when there's none.
    pub async fn restore_line_note(&self, id: &str) -> Result<bool, DbError> {
        let now = now_ms();
        let r = sqlx::query("UPDATE dev_line_notes SET deleted_at = NULL, updated_at = ? WHERE id = ? AND state = 'draft' AND deleted_at >= ?")
            .bind(now)
            .bind(id)
            .bind(now - RESTORE_SECS * 1000)
            .execute(&self.pool)
            .await?;
        Ok(r.rows_affected() == 1)
    }

    /// Removes deleted notes past the restore window and sends past [`SEND_ID_HOURS`].
    pub async fn purge_line_notes(&self) -> Result<(), DbError> {
        let now = now_ms();
        sqlx::query("DELETE FROM dev_line_notes WHERE deleted_at IS NOT NULL AND deleted_at < ?").bind(now - RESTORE_SECS * 1000).execute(&self.pool).await?;
        sqlx::query("DELETE FROM dev_line_note_sends WHERE created_at < ?").bind(now - SEND_ID_HOURS * 3_600_000).execute(&self.pool).await?;
        Ok(())
    }

    /// Marks drafts sent in message `message_id`. Only drafts that aren't deleted change: the ids
    /// that did.
    pub async fn mark_line_notes_sent(&self, ids: &[String], message_id: &str, sent_at: i64) -> Result<Vec<String>, DbError> {
        let mut tx = self.pool.begin().await?;
        let mut done = Vec::new();
        for id in ids {
            let r = sqlx::query(
                "UPDATE dev_line_notes SET state = 'sent', sent_at = ?, message_id = ?, updated_at = ? WHERE id = ? AND state = 'draft' AND deleted_at IS NULL",
            )
            .bind(sent_at)
            .bind(message_id)
            .bind(sent_at)
            .bind(id)
            .execute(&mut *tx)
            .await?;
            if r.rows_affected() == 1 {
                done.push(id.clone());
            }
        }
        tx.commit().await?;
        Ok(done)
    }

    /// Sets a sent note's state after a push (`resolved` or `open`), once: false when it was
    /// already checked or isn't sent.
    pub async fn check_line_note(&self, id: &str, state: &str, checked_sha: &str) -> Result<bool, DbError> {
        let r = sqlx::query("UPDATE dev_line_notes SET state = ?, checked_sha = ?, updated_at = ? WHERE id = ? AND state = 'sent'")
            .bind(state)
            .bind(checked_sha)
            .bind(now_ms())
            .bind(id)
            .execute(&self.pool)
            .await?;
        Ok(r.rows_affected() == 1)
    }

    /// A send remembered by its `send_id` (any age; the caller checks it).
    pub async fn line_note_send(&self, send_id: &str) -> Result<Option<LineNoteSendRow>, DbError> {
        Ok(sqlx::query_as("SELECT send_id, space_id, repo, ref, ids, result, created_at FROM dev_line_note_sends WHERE send_id = ?")
            .bind(send_id)
            .fetch_optional(&self.pool)
            .await?)
    }

    pub async fn insert_line_note_send(&self, row: &LineNoteSendRow) -> Result<(), DbError> {
        sqlx::query("INSERT OR REPLACE INTO dev_line_note_sends (send_id, space_id, repo, ref, ids, result, created_at) VALUES (?, ?, ?, ?, ?, ?, ?)")
            .bind(&row.send_id)
            .bind(&row.space_id)
            .bind(&row.repo)
            .bind(&row.ref_)
            .bind(&row.ids)
            .bind(&row.result)
            .bind(row.created_at)
            .execute(&self.pool)
            .await?;
        Ok(())
    }
}
