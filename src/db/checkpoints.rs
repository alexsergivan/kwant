//! Mission Control Phase B2: the Time Machine's bookkeeping. Checkpoints of a task's folder
//! (before and after each run, manual ones, and one before every restore) and the per-turn log
//! of every permission decision with its side-effect class. The files themselves live under
//! `<data_dir>/checkpoints/` (see [`crate::server::missions::checkpoints`]); no path stored here
//! ever reaches the API. Contract: docs/mission-control.md §12.

use super::missions::{insert_event, Claim};
use super::{Database, DbError};
use chrono::{DateTime, Utc};
use serde::Serialize;
use serde_json::Value;
use sqlx::FromRow;
use uuid::Uuid;

pub const CHECKPOINT_KINDS: [&str; 4] = ["before_run", "after_run", "manual", "before_restore"];
pub const EFFECTS: [&str; 3] = ["none", "workspace", "external"];
pub const DECISIONS: [&str; 4] = ["allowed", "auto_allowed", "approved", "denied"];

#[derive(Debug, Clone, Serialize, FromRow, PartialEq)]
pub struct Checkpoint {
    pub id: String,
    pub task_id: String,
    pub mission_id: Option<String>,
    pub kind: String,
    pub claim_id: Option<String>,
    pub turn_id: Option<String>,
    pub attempt: i64,
    /// Server-only.
    #[serde(skip)]
    pub manifest_path: String,
    #[serde(skip)]
    pub manifest_sha256: String,
    /// Server-only: the folder it was taken of.
    #[serde(skip)]
    pub folder: String,
    pub files: i64,
    pub bytes: i64,
    pub skipped: i64,
    pub git_head: Option<String>,
    /// Why the snapshot failed; such a checkpoint can't be restored.
    pub error: Option<String>,
    pub created_by: String,
    pub created_at: DateTime<Utc>,
}

impl Checkpoint {
    pub fn failed(&self) -> bool {
        self.error.is_some()
    }
}

pub struct NewCheckpoint<'a> {
    pub id: String,
    pub task_id: &'a str,
    pub mission_id: Option<&'a str>,
    pub kind: &'a str,
    pub claim_id: Option<&'a str>,
    pub turn_id: Option<&'a str>,
    pub attempt: i64,
    pub manifest_path: String,
    pub manifest_sha256: String,
    pub folder: String,
    pub files: i64,
    pub bytes: i64,
    pub skipped: i64,
    pub git_head: Option<String>,
    pub error: Option<String>,
    pub created_by: &'a str,
}

#[derive(Debug, Clone, Serialize, FromRow, PartialEq)]
pub struct ToolCall {
    pub id: i64,
    pub turn_id: String,
    pub kwant_id: String,
    pub task_id: Option<String>,
    pub tool_name: String,
    pub effect: String,
    pub summary: String,
    pub decision: String,
    pub approval_id: Option<String>,
    pub idempotency_key: Option<String>,
    pub created_at: DateTime<Utc>,
}

const TRIGGERS: [&str; 3] = [
    // A checkpoint is never edited; retention deletes it (logging `checkpoint_pruned`)
    "CREATE TRIGGER IF NOT EXISTS task_checkpoints_no_update BEFORE UPDATE ON task_checkpoints \
     BEGIN SELECT RAISE(ABORT, 'checkpoints are append-only'); END",
    // The side-effect ledger is history
    "CREATE TRIGGER IF NOT EXISTS turn_tool_calls_no_update BEFORE UPDATE ON turn_tool_calls \
     BEGIN SELECT RAISE(ABORT, 'the tool-call log is append-only'); END",
    "CREATE TRIGGER IF NOT EXISTS turn_tool_calls_no_delete BEFORE DELETE ON turn_tool_calls \
     BEGIN SELECT RAISE(ABORT, 'the tool-call log is append-only'); END",
];

impl Database {
    pub(super) async fn migrate_checkpoints(&self) -> Result<(), DbError> {
        for stmt in [
            "CREATE TABLE IF NOT EXISTS task_checkpoints (
                id TEXT PRIMARY KEY,
                task_id TEXT NOT NULL REFERENCES tasks(id),
                mission_id TEXT,
                kind TEXT NOT NULL CHECK (kind IN ('before_run', 'after_run', 'manual', 'before_restore')),
                claim_id TEXT,
                turn_id TEXT,
                attempt INTEGER NOT NULL,
                manifest_path TEXT NOT NULL,
                manifest_sha256 TEXT NOT NULL,
                folder TEXT NOT NULL DEFAULT '',
                files INTEGER NOT NULL,
                bytes INTEGER NOT NULL,
                skipped INTEGER NOT NULL DEFAULT 0,
                git_head TEXT,
                error TEXT,
                created_by TEXT NOT NULL,
                created_at TIMESTAMP NOT NULL
            )",
            "CREATE INDEX IF NOT EXISTS task_checkpoints_task ON task_checkpoints (task_id, created_at)",
            "CREATE TABLE IF NOT EXISTS turn_tool_calls (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                turn_id TEXT NOT NULL,
                kwant_id TEXT NOT NULL,
                task_id TEXT,
                tool_name TEXT NOT NULL,
                effect TEXT NOT NULL CHECK (effect IN ('none', 'workspace', 'external')),
                summary TEXT NOT NULL,
                decision TEXT NOT NULL CHECK (decision IN ('allowed', 'auto_allowed', 'approved', 'denied')),
                approval_id TEXT,
                idempotency_key TEXT,
                created_at TIMESTAMP NOT NULL
            )",
            "CREATE INDEX IF NOT EXISTS turn_tool_calls_task ON turn_tool_calls (task_id, id)",
        ] {
            sqlx::query(stmt).execute(&self.pool).await?;
        }
        for t in TRIGGERS {
            sqlx::query(t).execute(&self.pool).await?;
        }
        Ok(())
    }

    // Checkpoints

    /// Stores a checkpoint and its task event (`checkpoint`, or `checkpoint_failed` when the
    /// snapshot failed) in one transaction.
    pub async fn insert_checkpoint(&self, c: &NewCheckpoint<'_>) -> Result<Checkpoint, DbError> {
        let mut tx = self.pool.begin().await?;
        let row = sqlx::query_as::<_, Checkpoint>(
            "INSERT INTO task_checkpoints (id, task_id, mission_id, kind, claim_id, turn_id, attempt, manifest_path, manifest_sha256, folder, \
             files, bytes, skipped, git_head, error, created_by, created_at) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?) RETURNING *",
        )
        .bind(&c.id)
        .bind(c.task_id)
        .bind(c.mission_id)
        .bind(c.kind)
        .bind(c.claim_id)
        .bind(c.turn_id)
        .bind(c.attempt)
        .bind(&c.manifest_path)
        .bind(&c.manifest_sha256)
        .bind(&c.folder)
        .bind(c.files)
        .bind(c.bytes)
        .bind(c.skipped)
        .bind(&c.git_head)
        .bind(&c.error)
        .bind(c.created_by)
        .bind(Utc::now())
        .fetch_one(&mut *tx)
        .await?;
        let (kind, detail) = match &c.error {
            Some(e) => ("checkpoint_failed", serde_json::json!({"checkpoint_id": row.id, "kind": row.kind, "error": e})),
            None => (
                "checkpoint",
                serde_json::json!({"checkpoint_id": row.id, "kind": row.kind, "files": row.files, "bytes": row.bytes, "skipped": row.skipped}),
            ),
        };
        insert_event(&mut tx, c.task_id, c.mission_id, kind, c.created_by, None, None, &detail).await?;
        tx.commit().await?;
        Ok(row)
    }

    pub async fn get_checkpoint(&self, id: &str) -> Result<Option<Checkpoint>, DbError> {
        Ok(sqlx::query_as::<_, Checkpoint>("SELECT * FROM task_checkpoints WHERE id = ?").bind(id).fetch_optional(&self.pool).await?)
    }

    /// The task's checkpoints, oldest first.
    pub async fn checkpoints_of(&self, task_id: &str) -> Result<Vec<Checkpoint>, DbError> {
        Ok(sqlx::query_as::<_, Checkpoint>("SELECT * FROM task_checkpoints WHERE task_id = ? ORDER BY created_at, rowid")
            .bind(task_id)
            .fetch_all(&self.pool)
            .await?)
    }

    /// Run checkpoints (`before_run` / `after_run`) of other tasks, oldest first.
    pub async fn run_checkpoints_except(&self, task_id: &str) -> Result<Vec<Checkpoint>, DbError> {
        Ok(sqlx::query_as::<_, Checkpoint>(
            "SELECT * FROM task_checkpoints WHERE task_id != ? AND kind IN ('before_run', 'after_run') AND error IS NULL ORDER BY created_at, rowid",
        )
        .bind(task_id)
        .fetch_all(&self.pool)
        .await?)
    }

    /// Every checkpoint, oldest first (for the total cap and blob GC).
    pub async fn all_checkpoints(&self) -> Result<Vec<Checkpoint>, DbError> {
        Ok(sqlx::query_as::<_, Checkpoint>("SELECT * FROM task_checkpoints ORDER BY created_at, rowid").fetch_all(&self.pool).await?)
    }

    /// `(count, newest created_at)` of the task's checkpoints.
    pub async fn checkpoint_stats(&self, task_id: &str) -> Result<(i64, Option<DateTime<Utc>>), DbError> {
        Ok(sqlx::query_as("SELECT COUNT(*), MAX(created_at) FROM task_checkpoints WHERE task_id = ?").bind(task_id).fetch_one(&self.pool).await?)
    }

    /// Deletes a checkpoint row (retention), logging `checkpoint_pruned` on its task.
    pub async fn prune_checkpoint(&self, c: &Checkpoint, why: &str) -> Result<(), DbError> {
        let mut tx = self.pool.begin().await?;
        let r = sqlx::query("DELETE FROM task_checkpoints WHERE id = ?").bind(&c.id).execute(&mut *tx).await?;
        if r.rows_affected() == 1 {
            let detail = serde_json::json!({"checkpoint_id": c.id, "kind": c.kind, "why": why});
            insert_event(&mut tx, &c.task_id, c.mission_id.as_deref(), "checkpoint_pruned", "kwant", None, None, &detail).await?;
        }
        tx.commit().await?;
        Ok(())
    }

    /// The task's most recently released claim (the run that just ended).
    pub async fn last_released_claim(&self, task_id: &str) -> Result<Option<Claim>, DbError> {
        Ok(sqlx::query_as::<_, Claim>(
            "SELECT * FROM agent_claims WHERE task_id = ? AND released_at IS NOT NULL ORDER BY released_at DESC, claimed_at DESC LIMIT 1",
        )
        .bind(task_id)
        .fetch_optional(&self.pool)
        .await?)
    }

    // The tool-call log

    /// The task a turn's tool call belongs to: the claim taken in this turn, else the bot's own
    /// open claim, else one kwant holds for it. `None`: not a task turn, nothing is logged (D9).
    pub async fn task_of_turn(&self, kwant_id: &str, turn_id: &str) -> Result<Option<String>, DbError> {
        let row: Option<(String,)> = sqlx::query_as(
            "SELECT task_id FROM agent_claims WHERE kwant_id = ?1 AND (released_at IS NULL OR turn_id = ?2) \
             ORDER BY turn_id IS NOT ?2, released_at IS NOT NULL, system, claimed_at DESC LIMIT 1",
        )
        .bind(kwant_id)
        .bind(turn_id)
        .fetch_optional(&self.pool)
        .await?;
        Ok(row.map(|r| r.0))
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn log_tool_call(
        &self,
        turn_id: &str,
        kwant_id: &str,
        task_id: &str,
        tool_name: &str,
        effect: &str,
        summary: &str,
        decision: &str,
        approval_id: Option<&str>,
    ) -> Result<(), DbError> {
        sqlx::query(
            "INSERT INTO turn_tool_calls (turn_id, kwant_id, task_id, tool_name, effect, summary, decision, approval_id, created_at) \
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(turn_id)
        .bind(kwant_id)
        .bind(task_id)
        .bind(tool_name)
        .bind(effect)
        .bind(summary)
        .bind(decision)
        .bind(approval_id)
        .bind(Utc::now())
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// The task's logged tool calls, oldest first.
    pub async fn tool_calls_of(&self, task_id: &str) -> Result<Vec<ToolCall>, DbError> {
        Ok(sqlx::query_as::<_, ToolCall>("SELECT * FROM turn_tool_calls WHERE task_id = ? ORDER BY id").bind(task_id).fetch_all(&self.pool).await?)
    }

    /// A task event outside a state change (`restored`).
    pub async fn task_event(&self, task_id: &str, mission_id: Option<&str>, kind: &str, actor: &str, detail: &Value) -> Result<(), DbError> {
        let mut tx = self.pool.begin().await?;
        insert_event(&mut tx, task_id, mission_id, kind, actor, None, None, detail).await?;
        tx.commit().await?;
        Ok(())
    }

    /// Checkpoint test hook: the detail JSON of the task's events of `kind`.
    #[doc(hidden)]
    pub async fn event_details(&self, task_id: &str, kind: &str) -> Result<Vec<Value>, DbError> {
        let rows: Vec<(String,)> = sqlx::query_as("SELECT detail FROM task_events WHERE task_id = ? AND kind = ? ORDER BY rowid")
            .bind(task_id)
            .bind(kind)
            .fetch_all(&self.pool)
            .await?;
        Ok(rows.into_iter().map(|r| serde_json::from_str(&r.0).unwrap_or(Value::Null)).collect())
    }
}

/// A fresh checkpoint id.
pub fn new_id() -> String {
    Uuid::new_v4().to_string()
}
