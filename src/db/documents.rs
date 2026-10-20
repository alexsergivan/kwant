//! Camera-to-Completed: what was read from a letter (`document_extractions`) and the deadline
//! reminders of its case (`document_reminders`), both keyed by the `zombie_cases` id. The case
//! itself lives in `zombie_cases` (source `document`, kind `document_request`).
//!
//! The extracted text is the letter's own words: it stays here, next to the case, and goes when
//! the case is deleted or wiped by retention. It is never logged.

use super::{Database, DbError};
use chrono::{DateTime, Utc};
use sqlx::FromRow;

/// `running` (being read), `read` (a model read it), `text_only` (the PDF text, no model could
/// structure it), `unreadable` (no text and no model that sees images: the person fills the fields).
pub const EXTRACTION_STATUSES: [&str; 4] = ["running", "read", "text_only", "unreadable"];

#[derive(Debug, Clone, FromRow, PartialEq)]
pub struct ExtractionRow {
    pub case_id: String,
    pub status: String,
    /// `pdf_text`, `model`, `pdf_text+model`, or null while running / when unreadable.
    pub method: Option<String>,
    /// Why it is unreadable or text only, in kwant's words (never the letter's).
    pub note: Option<String>,
    /// The PDF text layer plus the model's transcript of the images.
    pub text: String,
    /// JSON `documents::Fields`.
    pub fields: String,
    /// JSON array of the attachment ids, in page order.
    pub attachment_ids: String,
    /// `api:<model>`, `cli` or `stub:<name>`.
    pub read_by: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, FromRow, PartialEq)]
pub struct ReminderRow {
    pub case_id: String,
    /// Days before the deadline: 7, 1 or 0.
    pub offset_days: i64,
    pub due_at: DateTime<Utc>,
    pub fired_at: Option<DateTime<Utc>>,
}

impl Database {
    pub(super) async fn migrate_documents(&self) -> Result<(), DbError> {
        for stmt in [
            "CREATE TABLE IF NOT EXISTS document_extractions (
                case_id TEXT PRIMARY KEY,
                status TEXT NOT NULL CHECK (status IN ('running', 'read', 'text_only', 'unreadable')),
                method TEXT,
                note TEXT,
                text TEXT NOT NULL DEFAULT '',
                fields TEXT NOT NULL DEFAULT '{}',
                attachment_ids TEXT NOT NULL DEFAULT '[]',
                read_by TEXT,
                created_at TIMESTAMP NOT NULL,
                updated_at TIMESTAMP NOT NULL
            )",
            "CREATE TABLE IF NOT EXISTS document_reminders (
                case_id TEXT NOT NULL,
                offset_days INTEGER NOT NULL,
                due_at TIMESTAMP NOT NULL,
                fired_at TIMESTAMP,
                PRIMARY KEY (case_id, offset_days)
            )",
            "CREATE INDEX IF NOT EXISTS document_reminders_due ON document_reminders (fired_at, due_at)",
        ] {
            sqlx::query(stmt).execute(&self.pool).await?;
        }
        Ok(())
    }

    pub async fn document_extraction(&self, case_id: &str) -> Result<Option<ExtractionRow>, DbError> {
        Ok(sqlx::query_as("SELECT * FROM document_extractions WHERE case_id = ?").bind(case_id).fetch_optional(&self.pool).await?)
    }

    /// Inserts `e`, or replaces the row of its case.
    pub async fn document_put_extraction(&self, e: &ExtractionRow) -> Result<(), DbError> {
        sqlx::query(
            "INSERT INTO document_extractions (case_id, status, method, note, text, fields, attachment_ids, read_by, created_at, updated_at)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
             ON CONFLICT(case_id) DO UPDATE SET status = excluded.status, method = excluded.method, note = excluded.note,
                text = excluded.text, fields = excluded.fields, attachment_ids = excluded.attachment_ids,
                read_by = excluded.read_by, updated_at = excluded.updated_at",
        )
        .bind(&e.case_id)
        .bind(&e.status)
        .bind(&e.method)
        .bind(&e.note)
        .bind(&e.text)
        .bind(&e.fields)
        .bind(&e.attachment_ids)
        .bind(&e.read_by)
        .bind(e.created_at)
        .bind(e.updated_at)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// Extractions a restart cut short become unreadable, so the person can fill them in or retry.
    pub async fn document_mark_interrupted(&self, note: &str, at: DateTime<Utc>) -> Result<u64, DbError> {
        Ok(sqlx::query("UPDATE document_extractions SET status = 'unreadable', note = ?, updated_at = ? WHERE status = 'running'")
            .bind(note)
            .bind(at)
            .execute(&self.pool)
            .await?
            .rows_affected())
    }

    pub async fn document_reminders(&self, case_id: &str) -> Result<Vec<ReminderRow>, DbError> {
        Ok(sqlx::query_as("SELECT * FROM document_reminders WHERE case_id = ? ORDER BY offset_days DESC")
            .bind(case_id)
            .fetch_all(&self.pool)
            .await?)
    }

    /// Replaces the reminders of a case with `due` (offset days, due time), none fired.
    pub async fn document_set_reminders(&self, case_id: &str, due: &[(i64, DateTime<Utc>)]) -> Result<(), DbError> {
        let mut tx = self.pool.begin().await?;
        sqlx::query("DELETE FROM document_reminders WHERE case_id = ?").bind(case_id).execute(&mut *tx).await?;
        for (offset, at) in due {
            sqlx::query("INSERT INTO document_reminders (case_id, offset_days, due_at) VALUES (?, ?, ?)")
                .bind(case_id)
                .bind(offset)
                .bind(at)
                .execute(&mut *tx)
                .await?;
        }
        tx.commit().await?;
        Ok(())
    }

    pub async fn document_clear_reminders(&self, case_id: &str) -> Result<u64, DbError> {
        Ok(sqlx::query("DELETE FROM document_reminders WHERE case_id = ?").bind(case_id).execute(&self.pool).await?.rows_affected())
    }

    /// Reminders not yet fired that are due at `now`.
    pub async fn document_due_reminders(&self, now: DateTime<Utc>) -> Result<Vec<ReminderRow>, DbError> {
        Ok(sqlx::query_as("SELECT * FROM document_reminders WHERE fired_at IS NULL AND due_at <= ? ORDER BY case_id, offset_days")
            .bind(now)
            .fetch_all(&self.pool)
            .await?)
    }

    pub async fn document_mark_fired(&self, case_id: &str, offset_days: i64, at: DateTime<Utc>) -> Result<(), DbError> {
        sqlx::query("UPDATE document_reminders SET fired_at = ? WHERE case_id = ? AND offset_days = ?")
            .bind(at)
            .bind(case_id)
            .bind(offset_days)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    /// Deletes a letter's case and everything read from it. Returns the attachment rows it removed,
    /// so the caller can delete the blobs nothing else uses.
    pub async fn document_delete(&self, case_id: &str) -> Result<Vec<super::attachments::Attachment>, DbError> {
        let context = format!("document:{case_id}");
        let mut tx = self.pool.begin().await?;
        let attachments: Vec<super::attachments::Attachment> =
            sqlx::query_as("SELECT * FROM attachments WHERE context_id = ?").bind(&context).fetch_all(&mut *tx).await?;
        sqlx::query("DELETE FROM attachments WHERE context_id = ?").bind(&context).execute(&mut *tx).await?;
        sqlx::query("DELETE FROM document_reminders WHERE case_id = ?").bind(case_id).execute(&mut *tx).await?;
        sqlx::query("DELETE FROM document_extractions WHERE case_id = ?").bind(case_id).execute(&mut *tx).await?;
        sqlx::query("DELETE FROM zombie_cases WHERE id = ? AND source = 'document'").bind(case_id).execute(&mut *tx).await?;
        tx.commit().await?;
        Ok(attachments)
    }

    /// Whether any attachment row still uses the blob `sha256`.
    pub async fn attachment_blob_in_use(&self, sha256: &str) -> Result<bool, DbError> {
        let n: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM attachments WHERE sha256 = ?").bind(sha256).fetch_one(&self.pool).await?;
        Ok(n.0 > 0)
    }

    /// Retention: a wiped (redacted) letter case loses its extracted text and reminders too.
    pub async fn document_apply_retention(&self) -> Result<u64, DbError> {
        sqlx::query("DELETE FROM document_reminders WHERE case_id IN (SELECT id FROM zombie_cases WHERE redacted = 1)")
            .execute(&self.pool)
            .await?;
        Ok(sqlx::query("DELETE FROM document_extractions WHERE case_id IN (SELECT id FROM zombie_cases WHERE redacted = 1)")
            .execute(&self.pool)
            .await?
            .rows_affected())
    }
}
