//! Mission Control Phase B1: the work graph's "may be invalid" flags, forks of a task (redo
//! without touching verified history), the drift scan's bookkeeping and HTTP idempotency keys.
//! Contract: docs/mission-control.md §11. As in Phase A, the rules hold twice: in
//! [`crate::server::missions::graph`] and by SQLite triggers here, so a flag is resolved only by
//! the person, only once, and never deleted.

use super::missions::{insert_event, Task};
use super::spaces::SpaceId;
use super::{Database, DbError};
use chrono::{DateTime, Utc};
use serde::Serialize;
use serde_json::Value;
use sqlx::FromRow;
use uuid::Uuid;

pub const FLAG_CAUSES: [&str; 3] = ["evidence_changed", "upstream_evidence_changed", "upstream_forked"];
pub const FLAG_RESOLUTIONS: [&str; 3] = ["still_valid", "redo", "cancelled"];
/// How long an `Idempotency-Key` is remembered.
pub const IDEMPOTENCY_HOURS: i64 = 24;

#[derive(Debug, Clone, Serialize, FromRow, PartialEq)]
pub struct TaskFlag {
    pub id: String,
    pub task_id: String,
    pub kind: String,
    pub cause: String,
    pub cause_task_id: String,
    /// The cause task's title (joined in).
    pub cause_title: Option<String>,
    /// JSON text; see [`TaskFlag::detail_value`].
    #[serde(skip)]
    pub detail: String,
    pub created_at: DateTime<Utc>,
    pub resolved_at: Option<DateTime<Utc>>,
    pub resolved_by: Option<String>,
    pub resolution: Option<String>,
}

impl TaskFlag {
    pub fn detail_value(&self) -> Value {
        serde_json::from_str(&self.detail).unwrap_or(Value::Null)
    }
}

/// A mission's last drift scan.
#[derive(Debug, Clone, FromRow, PartialEq)]
pub struct GraphScan {
    pub mission_id: String,
    pub scanned_at: DateTime<Utc>,
    /// JSON array of `{task_id, path, why}`.
    pub skipped: String,
}

const FLAG_SELECT: &str = "SELECT f.id, f.task_id, f.kind, f.cause, f.cause_task_id, c.title AS cause_title, f.detail, \
     f.created_at, f.resolved_at, f.resolved_by, f.resolution FROM task_flags f LEFT JOIN tasks c ON c.id = f.cause_task_id";

const TRIGGERS: [&str; 3] = [
    // A flag is resolved once, and only by the person
    "CREATE TRIGGER IF NOT EXISTS task_flags_resolve_once BEFORE UPDATE ON task_flags \
     WHEN OLD.resolved_at IS NOT NULL \
     BEGIN SELECT RAISE(ABORT, 'a flag is resolved once'); END",
    "CREATE TRIGGER IF NOT EXISTS task_flags_person_resolves BEFORE UPDATE ON task_flags \
     WHEN NEW.resolved_at IS NOT NULL AND COALESCE(NEW.resolved_by, '') != 'person' \
     BEGIN SELECT RAISE(ABORT, 'only the person resolves a flag'); END",
    // Flags are history
    "CREATE TRIGGER IF NOT EXISTS task_flags_no_delete BEFORE DELETE ON task_flags \
     BEGIN SELECT RAISE(ABORT, 'flags are history'); END",
];

impl Database {
    pub(super) async fn migrate_mission_graph(&self) -> Result<(), DbError> {
        // The task this one redoes (a fork); fails harmlessly once the column exists
        let _ = sqlx::query("ALTER TABLE tasks ADD COLUMN forked_from TEXT").execute(&self.pool).await;
        for stmt in [
            "CREATE TABLE IF NOT EXISTS task_flags (
                id TEXT PRIMARY KEY,
                task_id TEXT NOT NULL REFERENCES tasks(id),
                kind TEXT NOT NULL CHECK (kind IN ('may_be_invalid')),
                cause TEXT NOT NULL CHECK (cause IN ('evidence_changed', 'upstream_evidence_changed', 'upstream_forked')),
                cause_task_id TEXT NOT NULL REFERENCES tasks(id),
                detail TEXT NOT NULL DEFAULT '{}',
                created_at TIMESTAMP NOT NULL,
                resolved_at TIMESTAMP,
                resolved_by TEXT,
                resolution TEXT CHECK (resolution IN ('still_valid', 'redo', 'cancelled'))
            )",
            "CREATE UNIQUE INDEX IF NOT EXISTS task_flags_one_open ON task_flags (task_id, cause_task_id, cause) WHERE resolved_at IS NULL",
            "CREATE INDEX IF NOT EXISTS task_flags_task ON task_flags (task_id, resolved_at)",
            "CREATE INDEX IF NOT EXISTS tasks_forked_from ON tasks (forked_from)",
            "CREATE TABLE IF NOT EXISTS idempotency_keys (
                key TEXT PRIMARY KEY,
                route TEXT NOT NULL,
                response TEXT NOT NULL,
                created_at TIMESTAMP NOT NULL
            )",
            "CREATE TABLE IF NOT EXISTS graph_scans (
                mission_id TEXT PRIMARY KEY,
                scanned_at TIMESTAMP NOT NULL,
                skipped TEXT NOT NULL DEFAULT '[]'
            )",
        ] {
            sqlx::query(stmt).execute(&self.pool).await?;
        }
        for t in TRIGGERS {
            sqlx::query(t).execute(&self.pool).await?;
        }
        Ok(())
    }

    // Flags

    /// Raises a flag unless the same one (task, cause task, cause) is already open. Writes a
    /// `flagged` event with the flag in the same transaction. `Ok(None)` when it was already open.
    pub async fn raise_flag(&self, task_id: &str, cause: &str, cause_task_id: &str, detail: &Value) -> Result<Option<TaskFlag>, DbError> {
        let id = Uuid::new_v4().to_string();
        let mut tx = self.pool.begin().await?;
        let r = sqlx::query(
            "INSERT OR IGNORE INTO task_flags (id, task_id, kind, cause, cause_task_id, detail, created_at) VALUES (?, ?, 'may_be_invalid', ?, ?, ?, ?)",
        )
        .bind(&id)
        .bind(task_id)
        .bind(cause)
        .bind(cause_task_id)
        .bind(detail.to_string())
        .bind(Utc::now())
        .execute(&mut *tx)
        .await?;
        if r.rows_affected() == 0 {
            return Ok(None);
        }
        let flag = sqlx::query_as::<_, TaskFlag>(&format!("{FLAG_SELECT} WHERE f.id = ?")).bind(&id).fetch_one(&mut *tx).await?;
        let event = serde_json::json!({"flag_id": flag.id, "kind": flag.kind, "cause": flag.cause, "cause_task_id": flag.cause_task_id, "detail": flag.detail_value()});
        insert_event(&mut tx, task_id, None, "flagged", "kwant", None, None, &event).await?;
        tx.commit().await?;
        Ok(Some(flag))
    }

    pub async fn get_flag(&self, id: &str) -> Result<Option<TaskFlag>, DbError> {
        Ok(sqlx::query_as::<_, TaskFlag>(&format!("{FLAG_SELECT} WHERE f.id = ?")).bind(id).fetch_optional(&self.pool).await?)
    }

    /// The task's flags, oldest first; open ones only unless `all`.
    pub async fn flags_of(&self, task_id: &str, all: bool) -> Result<Vec<TaskFlag>, DbError> {
        Ok(sqlx::query_as::<_, TaskFlag>(&format!("{FLAG_SELECT} WHERE f.task_id = ? AND (? OR f.resolved_at IS NULL) ORDER BY f.created_at, f.rowid"))
            .bind(task_id)
            .bind(all)
            .fetch_all(&self.pool)
            .await?)
    }

    /// Whether a flag (open or resolved) already records this drift: same task, cause task, cause,
    /// path and new hash. A drift the person called still valid isn't raised again.
    pub async fn drift_known(&self, task_id: &str, cause: &str, cause_task_id: &str, path: &str, new_sha256: Option<&str>) -> Result<bool, DbError> {
        let (n,): (i64,) = sqlx::query_as(
            "SELECT COUNT(*) FROM task_flags WHERE task_id = ? AND cause = ? AND cause_task_id = ? \
             AND json_extract(detail, '$.path') = ? AND json_extract(detail, '$.new_sha256') IS ?",
        )
        .bind(task_id)
        .bind(cause)
        .bind(cause_task_id)
        .bind(path)
        .bind(new_sha256)
        .fetch_one(&self.pool)
        .await?;
        Ok(n > 0)
    }

    /// Closes an open flag as the person. False when it was already resolved.
    pub async fn resolve_flag(&self, id: &str, resolution: &str, note: Option<&str>, detail: Option<&Value>) -> Result<bool, DbError> {
        let mut tx = self.pool.begin().await?;
        let r = sqlx::query("UPDATE task_flags SET resolved_at = ?, resolved_by = 'person', resolution = ? WHERE id = ? AND resolved_at IS NULL")
            .bind(Utc::now())
            .bind(resolution)
            .bind(id)
            .execute(&mut *tx)
            .await?;
        if r.rows_affected() == 0 {
            return Ok(false);
        }
        let (task_id,): (String,) = sqlx::query_as("SELECT task_id FROM task_flags WHERE id = ?").bind(id).fetch_one(&mut *tx).await?;
        let mut event = serde_json::json!({"flag_id": id, "resolution": resolution, "note": note});
        if let (Some(obj), Some(Value::Object(more))) = (event.as_object_mut(), detail) {
            obj.extend(more.clone());
        }
        insert_event(&mut tx, &task_id, None, "flag_resolved", "person", None, None, &event).await?;
        tx.commit().await?;
        Ok(true)
    }

    /// Ids of the space's tasks with an open flag.
    pub async fn flagged_task_ids(&self, space: &SpaceId) -> Result<Vec<String>, DbError> {
        let rows: Vec<(String,)> = sqlx::query_as(
            "SELECT DISTINCT f.task_id FROM task_flags f JOIN tasks t ON t.id = f.task_id WHERE t.space_id = ? AND f.resolved_at IS NULL",
        )
        .bind(space.as_str())
        .fetch_all(&self.pool)
        .await?;
        Ok(rows.into_iter().map(|r| r.0).collect())
    }

    // The graph

    /// Direct dependents of `task_id` that have started: past `ready`, attempted, or ever claimed.
    /// Cancelled ones are left alone.
    pub async fn started_dependents(&self, task_id: &str) -> Result<Vec<Task>, DbError> {
        Ok(sqlx::query_as::<_, Task>(
            "SELECT t.* FROM tasks t JOIN task_dependencies d ON d.task_id = t.id AND d.depends_on_id = ? \
             WHERE t.state != 'cancelled' AND (t.state NOT IN ('backlog', 'ready') OR t.attempts > 0 \
             OR EXISTS (SELECT 1 FROM agent_claims a WHERE a.task_id = t.id)) ORDER BY t.created_at, t.rowid",
        )
        .bind(task_id)
        .fetch_all(&self.pool)
        .await?)
    }

    /// Dependency edges among `task_ids` as (from = depends_on_id, to = task_id).
    pub async fn edges_among(&self, task_ids: &[String]) -> Result<Vec<(String, String)>, DbError> {
        if task_ids.is_empty() {
            return Ok(Vec::new());
        }
        let ids = serde_json::to_string(task_ids)?;
        Ok(sqlx::query_as(
            "SELECT d.depends_on_id, d.task_id FROM task_dependencies d \
             WHERE d.task_id IN (SELECT value FROM json_each(?1)) AND d.depends_on_id IN (SELECT value FROM json_each(?1)) \
             ORDER BY d.rowid",
        )
        .bind(ids)
        .fetch_all(&self.pool)
        .await?)
    }

    /// The space's tasks without a mission: open ones, and ones finished since `since`.
    pub async fn loose_tasks(&self, space: &SpaceId, since: DateTime<Utc>) -> Result<Vec<Task>, DbError> {
        Ok(sqlx::query_as::<_, Task>(
            "SELECT * FROM tasks WHERE space_id = ? AND mission_id IS NULL \
             AND (state NOT IN ('verified', 'cancelled') OR COALESCE(finished_at, updated_at) >= ?) \
             ORDER BY priority DESC, created_at, rowid LIMIT 500",
        )
        .bind(space.as_str())
        .bind(since)
        .fetch_all(&self.pool)
        .await?)
    }

    /// Verified tasks of the mission, for the drift scan.
    pub async fn verified_in_mission(&self, mission_id: &str) -> Result<Vec<Task>, DbError> {
        Ok(sqlx::query_as::<_, Task>("SELECT * FROM tasks WHERE mission_id = ? AND state = 'verified' ORDER BY created_at, rowid")
            .bind(mission_id)
            .fetch_all(&self.pool)
            .await?)
    }

    pub async fn graph_scan(&self, mission_id: &str) -> Result<Option<GraphScan>, DbError> {
        Ok(sqlx::query_as::<_, GraphScan>("SELECT * FROM graph_scans WHERE mission_id = ?").bind(mission_id).fetch_optional(&self.pool).await?)
    }

    pub async fn record_graph_scan(&self, mission_id: &str, skipped: &Value) -> Result<(), DbError> {
        sqlx::query("INSERT INTO graph_scans (mission_id, scanned_at, skipped) VALUES (?, ?, ?) ON CONFLICT(mission_id) DO UPDATE SET scanned_at = excluded.scanned_at, skipped = excluded.skipped")
            .bind(mission_id)
            .bind(Utc::now())
            .bind(skipped.to_string())
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    // Forks

    /// A new backlog task redoing `orig`: same mission, title, acceptance, priority, budget and
    /// attempts, suggested to the original owner, the same dependencies, and the same checks reset
    /// to pending (approval carries over only when the person or the approved plan gave it). The
    /// original is untouched. Logs `forked` on the original and `created` on the fork.
    pub async fn fork_task(&self, orig: &Task, reason: &str) -> Result<Task, DbError> {
        let now = Utc::now();
        let mut tx = self.pool.begin().await?;
        let fork = sqlx::query_as::<_, Task>(
            "INSERT INTO tasks (id, space_id, mission_id, title, acceptance, state, priority, suggested_kwant_id, budget_tokens, max_attempts, \
             source_kind, source_id, channel_id, direct_kwant_id, thread_id, created_by, created_at, updated_at, forked_from) \
             VALUES (?, ?, ?, ?, ?, 'backlog', ?, ?, ?, ?, ?, NULL, ?, ?, ?, 'person', ?, ?, ?) RETURNING *",
        )
        .bind(Uuid::new_v4().to_string())
        .bind(&orig.space_id)
        .bind(&orig.mission_id)
        .bind(&orig.title)
        .bind(&orig.acceptance)
        .bind(orig.priority)
        .bind(orig.owner_kwant_id.as_ref().or(orig.suggested_kwant_id.as_ref()))
        .bind(orig.budget_tokens)
        .bind(orig.max_attempts)
        .bind(&orig.source_kind)
        .bind(&orig.channel_id)
        .bind(&orig.direct_kwant_id)
        .bind(&orig.thread_id)
        .bind(now)
        .bind(now)
        .bind(&orig.id)
        .fetch_one(&mut *tx)
        .await?;
        sqlx::query("INSERT INTO task_dependencies (task_id, depends_on_id) SELECT ?, depends_on_id FROM task_dependencies WHERE task_id = ?")
            .bind(&fork.id)
            .bind(&orig.id)
            .execute(&mut *tx)
            .await?;
        let checks = sqlx::query_as::<_, super::missions::OutcomeCheck>("SELECT * FROM outcome_checks WHERE task_id = ? ORDER BY created_at, rowid")
            .bind(&orig.id)
            .fetch_all(&mut *tx)
            .await?;
        for c in &checks {
            let keep = c.approved && matches!(c.approved_by.as_deref(), Some("person" | "mission"));
            sqlx::query(
                "INSERT INTO outcome_checks (id, task_id, kind, spec, required, approved, status, requested_by, approved_by, created_at) \
                 VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
            )
            .bind(Uuid::new_v4().to_string())
            .bind(&fork.id)
            .bind(&c.kind)
            .bind(&c.spec)
            .bind(c.required)
            .bind(keep)
            .bind(if keep { "pending" } else { "needs_approval" })
            .bind(&c.requested_by)
            .bind(if keep { c.approved_by.clone() } else { None })
            .bind(now)
            .execute(&mut *tx)
            .await?;
        }
        insert_event(&mut tx, &orig.id, orig.mission_id.as_deref(), "forked", "person", None, None, &serde_json::json!({"fork_id": fork.id, "reason": reason})).await?;
        insert_event(
            &mut tx,
            &fork.id,
            fork.mission_id.as_deref(),
            "created",
            "person",
            None,
            Some("backlog"),
            &serde_json::json!({"source_kind": fork.source_kind, "forked_from": orig.id, "reason": reason}),
        )
        .await?;
        tx.commit().await?;
        Ok(fork)
    }

    /// Forks of `task_id`, newest first.
    pub async fn forks_of(&self, task_id: &str) -> Result<Vec<Task>, DbError> {
        Ok(sqlx::query_as::<_, Task>("SELECT * FROM tasks WHERE forked_from = ? ORDER BY created_at DESC, rowid DESC").bind(task_id).fetch_all(&self.pool).await?)
    }

    // Idempotency keys

    /// The stored response for `key` as (route, response JSON), forgetting keys older than 24 h.
    pub async fn idempotent_response(&self, key: &str) -> Result<Option<(String, String)>, DbError> {
        sqlx::query("DELETE FROM idempotency_keys WHERE created_at < ?")
            .bind(Utc::now() - chrono::Duration::hours(IDEMPOTENCY_HOURS))
            .execute(&self.pool)
            .await?;
        Ok(sqlx::query_as("SELECT route, response FROM idempotency_keys WHERE key = ?").bind(key).fetch_optional(&self.pool).await?)
    }

    pub async fn store_idempotent_response(&self, key: &str, route: &str, response: &Value) -> Result<(), DbError> {
        sqlx::query("INSERT OR IGNORE INTO idempotency_keys (key, route, response, created_at) VALUES (?, ?, ?, ?)")
            .bind(key)
            .bind(route)
            .bind(response.to_string())
            .bind(Utc::now())
            .execute(&self.pool)
            .await?;
        Ok(())
    }
}
