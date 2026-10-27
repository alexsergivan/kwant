//! Mission Control Phase B3: AutoFix's bookkeeping. Every failure of a task (`task_failures`,
//! append-only), the retries AutoFix plans or runs for it (`autofix_attempts`, one per failure and
//! strategy, at most one live per task) and the permanent changes it or a bot proposes
//! (`workflow_changes`, applied once and only after the person approves). The diagnostics files
//! live under `<data_dir>/autofix/`; no path stored here ever reaches the API. Contract:
//! docs/mission-control.md §16.

use super::missions::{insert_event, Task};
use super::{Database, DbError};
use chrono::{DateTime, Utc};
use serde::Serialize;
use serde_json::Value;
use sqlx::FromRow;
use uuid::Uuid;

pub const SOURCES: [&str; 6] = ["check", "turn", "lease", "budget", "routine", "handoff"];
pub const CLASSES: [&str; 9] = ["transient", "stalled", "check_failed", "auth", "budget", "denied", "bad_spec", "needs_input", "unknown"];
pub const STRATEGIES: [&str; 6] = ["retry_same", "retry_with_diagnostics", "retry_from_checkpoint", "wait_and_retry", "propose_change", "none"];
pub const TARGETS: [&str; 5] = ["routine_instructions", "routine_schedule", "check_spec", "task_acceptance", "max_attempts"];

#[derive(Debug, Clone, Serialize, FromRow, PartialEq)]
pub struct Failure {
    pub id: String,
    pub task_id: String,
    pub attempt: i64,
    pub source: String,
    pub class: String,
    pub summary: String,
    pub excerpt: Option<String>,
    /// Server-only.
    #[serde(skip)]
    pub diagnostics_path: Option<String>,
    pub checkpoint_id: Option<String>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, FromRow, PartialEq)]
pub struct Attempt {
    pub id: String,
    pub task_id: String,
    pub failure_id: String,
    pub strategy: String,
    #[serde(skip)]
    pub idempotency_key: String,
    pub status: String,
    pub not_before: Option<DateTime<Utc>>,
    pub reason: String,
    pub retry_task_id: Option<String>,
    pub tokens_spent: i64,
    pub created_at: DateTime<Utc>,
    pub finished_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Serialize, FromRow, PartialEq)]
pub struct WorkflowChange {
    pub id: String,
    pub task_id: Option<String>,
    pub routine_id: Option<String>,
    pub target: String,
    pub before: String,
    pub after: String,
    pub reason: String,
    pub proposed_by: String,
    pub status: String,
    pub decided_by: Option<String>,
    pub decided_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
}

pub struct NewFailure<'a> {
    pub task: &'a Task,
    pub source: &'a str,
    pub class: &'a str,
    pub summary: String,
    pub excerpt: Option<String>,
    pub diagnostics_path: Option<String>,
    pub checkpoint_id: Option<String>,
}

pub struct NewAttempt<'a> {
    pub task: &'a Task,
    pub failure_id: &'a str,
    pub strategy: &'a str,
    pub idempotency_key: String,
    pub status: &'a str,
    pub not_before: Option<DateTime<Utc>>,
    pub reason: String,
}

const TRIGGERS: [&str; 6] = [
    "CREATE TRIGGER IF NOT EXISTS task_failures_no_update BEFORE UPDATE ON task_failures \
     BEGIN SELECT RAISE(ABORT, 'task_failures is append-only'); END",
    "CREATE TRIGGER IF NOT EXISTS task_failures_no_delete BEFORE DELETE ON task_failures \
     BEGIN SELECT RAISE(ABORT, 'task_failures is append-only'); END",
    // AutoFix runs are attempts: none is scheduled or started past the task's max_attempts
    "CREATE TRIGGER IF NOT EXISTS autofix_within_attempts BEFORE INSERT ON autofix_attempts \
     WHEN NEW.status IN ('scheduled', 'running') \
     AND (SELECT attempts >= max_attempts FROM tasks WHERE id = NEW.task_id) \
     BEGIN SELECT RAISE(ABORT, 'autofix runs only within the task''s attempts'); END",
    "CREATE TRIGGER IF NOT EXISTS autofix_within_attempts_update BEFORE UPDATE OF status ON autofix_attempts \
     WHEN NEW.status = 'running' AND OLD.status != 'running' \
     AND (SELECT attempts >= max_attempts FROM tasks WHERE id = NEW.task_id) \
     BEGIN SELECT RAISE(ABORT, 'autofix runs only within the task''s attempts'); END",
    // A permanent change is decided by the person, and applied only after approval
    "CREATE TRIGGER IF NOT EXISTS workflow_change_needs_person BEFORE UPDATE ON workflow_changes \
     WHEN NEW.status IN ('approved', 'applied', 'rejected') AND COALESCE(NEW.decided_by, '') != 'person' \
     BEGIN SELECT RAISE(ABORT, 'only the person decides a workflow change'); END",
    "CREATE TRIGGER IF NOT EXISTS workflow_change_applied_after_approved BEFORE UPDATE ON workflow_changes \
     WHEN NEW.status = 'applied' AND OLD.status != 'approved' \
     BEGIN SELECT RAISE(ABORT, 'a workflow change is applied only after the person approves it'); END",
];

impl Database {
    pub(super) async fn migrate_autofix(&self) -> Result<(), DbError> {
        let has: Option<(String,)> = sqlx::query_as("SELECT name FROM pragma_table_info('tasks') WHERE name = 'recovery_spent_tokens'").fetch_optional(&self.pool).await?;
        if has.is_none() {
            sqlx::query("ALTER TABLE tasks ADD COLUMN recovery_spent_tokens INTEGER NOT NULL DEFAULT 0").execute(&self.pool).await?;
        }
        // The AutoFix attempt a claim was taken for: its turns' spend is recovery spend
        let has: Option<(String,)> = sqlx::query_as("SELECT name FROM pragma_table_info('agent_claims') WHERE name = 'autofix_id'").fetch_optional(&self.pool).await?;
        if has.is_none() {
            sqlx::query("ALTER TABLE agent_claims ADD COLUMN autofix_id TEXT").execute(&self.pool).await?;
        }
        for stmt in [
            "CREATE TABLE IF NOT EXISTS task_failures (
                id TEXT PRIMARY KEY,
                task_id TEXT NOT NULL REFERENCES tasks(id),
                attempt INTEGER NOT NULL,
                source TEXT NOT NULL CHECK (source IN ('check', 'turn', 'lease', 'budget', 'routine', 'handoff')),
                class TEXT NOT NULL CHECK (class IN ('transient', 'stalled', 'check_failed', 'auth', 'budget', 'denied', 'bad_spec', 'needs_input', 'unknown')),
                summary TEXT NOT NULL,
                excerpt TEXT,
                diagnostics_path TEXT,
                checkpoint_id TEXT,
                created_at TIMESTAMP NOT NULL
            )",
            "CREATE INDEX IF NOT EXISTS task_failures_task ON task_failures (task_id, created_at)",
            "CREATE TABLE IF NOT EXISTS autofix_attempts (
                id TEXT PRIMARY KEY,
                task_id TEXT NOT NULL REFERENCES tasks(id),
                failure_id TEXT NOT NULL REFERENCES task_failures(id),
                strategy TEXT NOT NULL CHECK (strategy IN ('retry_same', 'retry_with_diagnostics', 'retry_from_checkpoint', 'wait_and_retry', 'propose_change', 'none')),
                idempotency_key TEXT NOT NULL UNIQUE,
                status TEXT NOT NULL CHECK (status IN ('suggested', 'scheduled', 'running', 'succeeded', 'failed', 'skipped')),
                not_before TIMESTAMP,
                reason TEXT NOT NULL,
                retry_task_id TEXT,
                tokens_spent INTEGER NOT NULL DEFAULT 0,
                created_at TIMESTAMP NOT NULL,
                finished_at TIMESTAMP
            )",
            "CREATE UNIQUE INDEX IF NOT EXISTS autofix_one_live ON autofix_attempts (task_id) WHERE status IN ('scheduled', 'running')",
            "CREATE INDEX IF NOT EXISTS autofix_attempts_task ON autofix_attempts (task_id, created_at)",
            "CREATE TABLE IF NOT EXISTS workflow_changes (
                id TEXT PRIMARY KEY,
                task_id TEXT REFERENCES tasks(id),
                routine_id TEXT,
                target TEXT NOT NULL CHECK (target IN ('routine_instructions', 'routine_schedule', 'check_spec', 'task_acceptance', 'max_attempts')),
                before TEXT NOT NULL,
                after TEXT NOT NULL,
                reason TEXT NOT NULL,
                proposed_by TEXT NOT NULL,
                status TEXT NOT NULL CHECK (status IN ('proposed', 'approved', 'rejected', 'applied', 'stale')),
                decided_by TEXT,
                decided_at TIMESTAMP,
                created_at TIMESTAMP NOT NULL
            )",
            "CREATE INDEX IF NOT EXISTS workflow_changes_status ON workflow_changes (status, created_at)",
        ] {
            sqlx::query(stmt).execute(&self.pool).await?;
        }
        for t in TRIGGERS {
            sqlx::query(t).execute(&self.pool).await?;
        }
        Ok(())
    }

    /// Records a failure and logs a `failure` event, then finishes the task's running AutoFix
    /// attempt (if any) as failed.
    pub async fn insert_failure(&self, f: &NewFailure<'_>) -> Result<Failure, DbError> {
        let id = Uuid::new_v4().to_string();
        let now = Utc::now();
        let mut tx = self.pool.begin().await?;
        sqlx::query(
            "INSERT INTO task_failures (id, task_id, attempt, source, class, summary, excerpt, diagnostics_path, checkpoint_id, created_at) \
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(&id)
        .bind(&f.task.id)
        .bind(f.task.attempts)
        .bind(f.source)
        .bind(f.class)
        .bind(&f.summary)
        .bind(&f.excerpt)
        .bind(&f.diagnostics_path)
        .bind(&f.checkpoint_id)
        .bind(now)
        .execute(&mut *tx)
        .await?;
        let detail = serde_json::json!({"failure_id": id, "source": f.source, "class": f.class, "summary": f.summary});
        insert_event(&mut tx, &f.task.id, f.task.mission_id.as_deref(), "failure", "kwant", None, None, &detail).await?;
        finish_running(&mut tx, &f.task.id, "failed", now).await?;
        tx.commit().await?;
        Ok(sqlx::query_as::<_, Failure>("SELECT * FROM task_failures WHERE id = ?").bind(&id).fetch_one(&self.pool).await?)
    }

    pub async fn failures_of(&self, task_id: &str) -> Result<Vec<Failure>, DbError> {
        Ok(sqlx::query_as::<_, Failure>("SELECT * FROM task_failures WHERE task_id = ? ORDER BY created_at, rowid").bind(task_id).fetch_all(&self.pool).await?)
    }

    pub async fn get_failure(&self, id: &str) -> Result<Option<Failure>, DbError> {
        Ok(sqlx::query_as::<_, Failure>("SELECT * FROM task_failures WHERE id = ?").bind(id).fetch_optional(&self.pool).await?)
    }

    /// How many earlier failures of `class` the task had (before `failure_id`).
    pub async fn count_failures(&self, task_id: &str, class: &str, failure_id: &str) -> Result<i64, DbError> {
        let (n,): (i64,) = sqlx::query_as(
            "SELECT COUNT(*) FROM task_failures WHERE task_id = ? AND class = ? AND id != ? \
             AND rowid < (SELECT rowid FROM task_failures WHERE id = ?)",
        )
        .bind(task_id)
        .bind(class)
        .bind(failure_id)
        .bind(failure_id)
        .fetch_one(&self.pool)
        .await?;
        Ok(n)
    }

    /// Plans an attempt once per key: `Ok(None)` when the key exists already (a repeated tick,
    /// a restart or a duplicate event). A trigger or the one-live index refusing it is an error.
    pub async fn insert_attempt(&self, a: &NewAttempt<'_>) -> Result<Option<Attempt>, DbError> {
        let id = Uuid::new_v4().to_string();
        let mut tx = self.pool.begin().await?;
        let r = sqlx::query(
            "INSERT OR IGNORE INTO autofix_attempts (id, task_id, failure_id, strategy, idempotency_key, status, not_before, reason, created_at, finished_at) \
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(&id)
        .bind(&a.task.id)
        .bind(a.failure_id)
        .bind(a.strategy)
        .bind(&a.idempotency_key)
        .bind(a.status)
        .bind(a.not_before)
        .bind(&a.reason)
        .bind(Utc::now())
        .bind(if matches!(a.status, "skipped") { Some(Utc::now()) } else { None })
        .execute(&mut *tx)
        .await?;
        if r.rows_affected() == 0 {
            return Ok(None);
        }
        let kind = if a.status == "skipped" { "autofix_finished" } else { "autofix_scheduled" };
        let detail = serde_json::json!({"autofix_id": id, "failure_id": a.failure_id, "strategy": a.strategy, "status": a.status, "not_before": a.not_before, "reason": a.reason});
        insert_event(&mut tx, &a.task.id, a.task.mission_id.as_deref(), kind, "kwant", None, None, &detail).await?;
        tx.commit().await?;
        self.get_attempt(&id).await
    }

    pub async fn get_attempt(&self, id: &str) -> Result<Option<Attempt>, DbError> {
        Ok(sqlx::query_as::<_, Attempt>("SELECT * FROM autofix_attempts WHERE id = ?").bind(id).fetch_optional(&self.pool).await?)
    }

    pub async fn attempts_of(&self, task_id: &str) -> Result<Vec<Attempt>, DbError> {
        Ok(sqlx::query_as::<_, Attempt>("SELECT * FROM autofix_attempts WHERE task_id = ? ORDER BY created_at, rowid").bind(task_id).fetch_all(&self.pool).await?)
    }

    /// The live attempt of the task, else its most recent one.
    pub async fn current_attempt(&self, task_id: &str) -> Result<Option<Attempt>, DbError> {
        Ok(sqlx::query_as::<_, Attempt>(
            "SELECT * FROM autofix_attempts WHERE task_id = ? \
             ORDER BY CASE WHEN status IN ('scheduled', 'running') THEN 0 ELSE 1 END, created_at DESC, rowid DESC LIMIT 1",
        )
        .bind(task_id)
        .fetch_optional(&self.pool)
        .await?)
    }

    /// Scheduled attempts whose time has come.
    pub async fn due_attempts(&self, now: DateTime<Utc>) -> Result<Vec<Attempt>, DbError> {
        Ok(sqlx::query_as::<_, Attempt>(
            "SELECT * FROM autofix_attempts WHERE status = 'scheduled' AND (not_before IS NULL OR not_before <= ?) ORDER BY created_at",
        )
        .bind(now)
        .fetch_all(&self.pool)
        .await?)
    }

    /// Moves an attempt `from` → `to` (compare-and-set) and logs it. `Ok(false)` when it wasn't in
    /// `from`.
    pub async fn move_attempt(&self, a: &Attempt, from: &str, to: &str, reason: Option<&str>) -> Result<bool, DbError> {
        let now = Utc::now();
        let mut tx = self.pool.begin().await?;
        let done = matches!(to, "succeeded" | "failed" | "skipped");
        let r = sqlx::query(
            "UPDATE autofix_attempts SET status = ?, reason = COALESCE(?, reason), finished_at = CASE WHEN ? THEN ? ELSE finished_at END \
             WHERE id = ? AND status = ?",
        )
        .bind(to)
        .bind(reason)
        .bind(done)
        .bind(now)
        .bind(&a.id)
        .bind(from)
        .execute(&mut *tx)
        .await?;
        if r.rows_affected() != 1 {
            return Ok(false);
        }
        let mission: Option<(Option<String>,)> = sqlx::query_as("SELECT mission_id FROM tasks WHERE id = ?").bind(&a.task_id).fetch_optional(&mut *tx).await?;
        let kind = match to {
            "running" => "autofix_started",
            "scheduled" => "autofix_scheduled",
            _ => "autofix_finished",
        };
        let detail = serde_json::json!({"autofix_id": a.id, "strategy": a.strategy, "from": from, "status": to, "reason": reason});
        insert_event(&mut tx, &a.task_id, mission.and_then(|m| m.0).as_deref(), kind, "kwant", None, None, &detail).await?;
        tx.commit().await?;
        Ok(true)
    }

    /// Finishes the task's running attempt (if any) as `status`.
    pub async fn finish_running_attempt(&self, task_id: &str, status: &str) -> Result<(), DbError> {
        let mut tx = self.pool.begin().await?;
        finish_running(&mut tx, task_id, status, Utc::now()).await?;
        tx.commit().await?;
        Ok(())
    }

    pub async fn set_retry_task(&self, attempt_id: &str, retry_task_id: &str) -> Result<(), DbError> {
        sqlx::query("UPDATE autofix_attempts SET retry_task_id = ? WHERE id = ?").bind(retry_task_id).bind(attempt_id).execute(&self.pool).await?;
        Ok(())
    }

    /// A routine retry's new task: linked to the failed one, counting its attempts (the failed run
    /// used one), and its claim is the attempt's, so its spend is recovery.
    pub async fn link_retry_task(&self, task_id: &str, failed: &Task, attempt_id: &str) -> Result<(), DbError> {
        let mut tx = self.pool.begin().await?;
        sqlx::query("UPDATE tasks SET forked_from = ?, attempts = ?, max_attempts = ? WHERE id = ? AND forked_from IS NULL")
            .bind(&failed.id)
            .bind(failed.attempts + 1)
            .bind(failed.max_attempts)
            .bind(task_id)
            .execute(&mut *tx)
            .await?;
        sqlx::query("UPDATE agent_claims SET autofix_id = ? WHERE task_id = ? AND released_at IS NULL").bind(attempt_id).bind(task_id).execute(&mut *tx).await?;
        tx.commit().await?;
        Ok(())
    }

    /// The AutoFix attempt the task's latest claim was taken for, if it was.
    pub async fn claim_autofix(&self, claim_id: &str) -> Result<Option<String>, DbError> {
        let r: Option<(Option<String>,)> = sqlx::query_as("SELECT autofix_id FROM agent_claims WHERE id = ?").bind(claim_id).fetch_optional(&self.pool).await?;
        Ok(r.and_then(|r| r.0))
    }

    pub async fn latest_claim_autofix(&self, task_id: &str) -> Result<Option<String>, DbError> {
        let r: Option<(Option<String>,)> = sqlx::query_as("SELECT autofix_id FROM agent_claims WHERE task_id = ? ORDER BY claimed_at DESC, rowid DESC LIMIT 1")
            .bind(task_id)
            .fetch_optional(&self.pool)
            .await?;
        Ok(r.and_then(|r| r.0))
    }

    /// Adds recovery spend to the task and to the attempt its claim was taken for.
    pub async fn spend_recovery(&self, task_id: &str, attempt_id: &str, tokens: i64) -> Result<(), DbError> {
        let mut tx = self.pool.begin().await?;
        sqlx::query("UPDATE tasks SET recovery_spent_tokens = MAX(recovery_spent_tokens + ?, 0) WHERE id = ?").bind(tokens).bind(task_id).execute(&mut *tx).await?;
        sqlx::query("UPDATE autofix_attempts SET tokens_spent = MAX(tokens_spent + ?, 0) WHERE id = ?").bind(tokens).bind(attempt_id).execute(&mut *tx).await?;
        tx.commit().await?;
        Ok(())
    }

    pub async fn recovery_spent(&self, task_id: &str) -> Result<i64, DbError> {
        let r: Option<(i64,)> = sqlx::query_as("SELECT recovery_spent_tokens FROM tasks WHERE id = ?").bind(task_id).fetch_optional(&self.pool).await?;
        Ok(r.map_or(0, |r| r.0))
    }

    /// Whether the task had a `denied` permission decision since `since_ms` (unix ms).
    /// A routine run's error text, for its task's failure.
    pub async fn routine_run_error(&self, run_id: &str) -> Result<Option<String>, DbError> {
        let r: Option<(Option<String>,)> = sqlx::query_as("SELECT error FROM routine_runs WHERE id = ?").bind(run_id).fetch_optional(&self.pool).await?;
        Ok(r.and_then(|r| r.0))
    }

    /// The first action denied on `task_id` since `since`, as "tool: summary".
    pub async fn denied_since(&self, task_id: &str, since: DateTime<Utc>) -> Result<Option<String>, DbError> {
        let r: Option<(String, String)> = sqlx::query_as(
            "SELECT tool_name, summary FROM turn_tool_calls WHERE task_id = ? AND decision = 'denied' AND created_at >= ? ORDER BY id LIMIT 1",
        )
        .bind(task_id)
        .bind(since)
        .fetch_optional(&self.pool)
        .await?;
        Ok(r.map(|(t, s)| format!("{t}: {s}")))
    }

    // Workflow changes

    #[allow(clippy::too_many_arguments)]
    pub async fn propose_change(
        &self,
        task: Option<&Task>,
        routine_id: Option<&str>,
        target: &str,
        before: &Value,
        after: &Value,
        reason: &str,
        proposed_by: &str,
    ) -> Result<WorkflowChange, DbError> {
        let id = Uuid::new_v4().to_string();
        let mut tx = self.pool.begin().await?;
        sqlx::query(
            "INSERT INTO workflow_changes (id, task_id, routine_id, target, before, after, reason, proposed_by, status, created_at) \
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, 'proposed', ?)",
        )
        .bind(&id)
        .bind(task.map(|t| &t.id))
        .bind(routine_id)
        .bind(target)
        .bind(before.to_string())
        .bind(after.to_string())
        .bind(reason)
        .bind(proposed_by)
        .bind(Utc::now())
        .execute(&mut *tx)
        .await?;
        if let Some(t) = task {
            let detail = serde_json::json!({"change_id": id, "target": target, "routine_id": routine_id, "reason": reason});
            insert_event(&mut tx, &t.id, t.mission_id.as_deref(), "workflow_change_proposed", proposed_by, None, None, &detail).await?;
        }
        tx.commit().await?;
        Ok(sqlx::query_as::<_, WorkflowChange>("SELECT * FROM workflow_changes WHERE id = ?").bind(&id).fetch_one(&self.pool).await?)
    }

    pub async fn get_change(&self, id: &str) -> Result<Option<WorkflowChange>, DbError> {
        Ok(sqlx::query_as::<_, WorkflowChange>("SELECT * FROM workflow_changes WHERE id = ?").bind(id).fetch_optional(&self.pool).await?)
    }

    pub async fn list_changes(&self, status: Option<&str>, task_id: Option<&str>) -> Result<Vec<WorkflowChange>, DbError> {
        Ok(sqlx::query_as::<_, WorkflowChange>(
            "SELECT * FROM workflow_changes WHERE (? IS NULL OR status = ?) AND (? IS NULL OR task_id = ?) ORDER BY created_at DESC, rowid DESC LIMIT 200",
        )
        .bind(status)
        .bind(status)
        .bind(task_id)
        .bind(task_id)
        .fetch_all(&self.pool)
        .await?)
    }

    /// The person decides a proposed change (compare-and-set). `Ok(false)` when it was decided
    /// already.
    pub async fn decide_change(&self, c: &WorkflowChange, to: &str) -> Result<bool, DbError> {
        let mut tx = self.pool.begin().await?;
        let r = sqlx::query("UPDATE workflow_changes SET status = ?, decided_by = 'person', decided_at = ? WHERE id = ? AND status = 'proposed'")
            .bind(to)
            .bind(Utc::now())
            .bind(&c.id)
            .execute(&mut *tx)
            .await?;
        if r.rows_affected() != 1 {
            return Ok(false);
        }
        change_event(&mut tx, c, "workflow_change_decided", to).await?;
        tx.commit().await?;
        Ok(true)
    }

    /// Applies an approved change once: the target is written only while it still holds `before`
    /// (else the change goes `stale` and nothing is written). Returns the final status.
    pub async fn apply_change(&self, c: &WorkflowChange, write: &ChangeWrite) -> Result<String, DbError> {
        let mut tx = self.pool.begin().await?;
        let r = match write {
            ChangeWrite::RoutineInstructions { routine_id, before, after } => {
                sqlx::query("UPDATE routines SET instructions = ? WHERE id = ? AND instructions = ?").bind(after).bind(routine_id).bind(before).execute(&mut *tx).await?
            }
            ChangeWrite::RoutineSchedule { routine_id, before, after, next_run_at } => {
                sqlx::query("UPDATE routines SET schedule = ?, next_run_at = ? WHERE id = ? AND schedule = ?")
                    .bind(after)
                    .bind(next_run_at)
                    .bind(routine_id)
                    .bind(before)
                    .execute(&mut *tx)
                    .await?
            }
            ChangeWrite::CheckSpec { check_id, before_spec, before_required, after_spec, after_required } => {
                sqlx::query("UPDATE outcome_checks SET spec = ?, required = ? WHERE id = ? AND spec = ? AND required = ?")
                    .bind(after_spec)
                    .bind(after_required)
                    .bind(check_id)
                    .bind(before_spec)
                    .bind(before_required)
                    .execute(&mut *tx)
                    .await?
            }
            ChangeWrite::Acceptance { task_id, before, after } => {
                sqlx::query("UPDATE tasks SET acceptance = ?, updated_at = ? WHERE id = ? AND acceptance = ?")
                    .bind(after)
                    .bind(Utc::now())
                    .bind(task_id)
                    .bind(before)
                    .execute(&mut *tx)
                    .await?
            }
            ChangeWrite::MaxAttempts { task_id, before, after } => {
                sqlx::query("UPDATE tasks SET max_attempts = ?, updated_at = ? WHERE id = ? AND max_attempts = ?")
                    .bind(after)
                    .bind(Utc::now())
                    .bind(task_id)
                    .bind(before)
                    .execute(&mut *tx)
                    .await?
            }
            ChangeWrite::Stale => {
                tx.rollback().await?;
                return self.mark_stale(c).await;
            }
        };
        if r.rows_affected() != 1 {
            tx.rollback().await?;
            return self.mark_stale(c).await;
        }
        let r = sqlx::query("UPDATE workflow_changes SET status = 'applied', decided_by = 'person' WHERE id = ? AND status = 'approved'").bind(&c.id).execute(&mut *tx).await?;
        if r.rows_affected() != 1 {
            tx.rollback().await?;
            return Ok(self.get_change(&c.id).await?.map_or_else(|| "missing".into(), |c| c.status));
        }
        change_event(&mut tx, c, "workflow_change_applied", "applied").await?;
        tx.commit().await?;
        Ok("applied".into())
    }

    async fn mark_stale(&self, c: &WorkflowChange) -> Result<String, DbError> {
        let mut tx = self.pool.begin().await?;
        let r = sqlx::query("UPDATE workflow_changes SET status = 'stale' WHERE id = ? AND status = 'approved'").bind(&c.id).execute(&mut *tx).await?;
        if r.rows_affected() == 1 {
            change_event(&mut tx, c, "workflow_change_decided", "stale").await?;
        }
        tx.commit().await?;
        Ok("stale".into())
    }

    /// Insights: AutoFix attempts that ran, succeeded, and their tokens; failures by class.
    pub async fn autofix_stats(&self, since: DateTime<Utc>) -> Result<(i64, i64, i64, Vec<(String, i64)>), DbError> {
        let (attempts, succeeded, tokens): (i64, i64, i64) = sqlx::query_as(
            "SELECT COUNT(*), COALESCE(SUM(status = 'succeeded'), 0), COALESCE(SUM(tokens_spent), 0) FROM autofix_attempts \
             WHERE created_at >= ? AND status IN ('running', 'succeeded', 'failed')",
        )
        .bind(since)
        .fetch_one(&self.pool)
        .await?;
        let classes: Vec<(String, i64)> = sqlx::query_as("SELECT class, COUNT(*) FROM task_failures WHERE created_at >= ? GROUP BY class ORDER BY class")
            .bind(since)
            .fetch_all(&self.pool)
            .await?;
        Ok((attempts, succeeded, tokens, classes))
    }
}

/// What applying a change writes, with the value it must still find there.
pub enum ChangeWrite {
    RoutineInstructions { routine_id: String, before: String, after: String },
    RoutineSchedule { routine_id: String, before: String, after: String, next_run_at: Option<DateTime<Utc>> },
    CheckSpec { check_id: String, before_spec: String, before_required: bool, after_spec: String, after_required: bool },
    Acceptance { task_id: String, before: String, after: String },
    MaxAttempts { task_id: String, before: i64, after: i64 },
    /// The target is gone or no longer matches.
    Stale,
}

async fn change_event(tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>, c: &WorkflowChange, kind: &str, status: &str) -> Result<(), DbError> {
    let Some(task_id) = &c.task_id else { return Ok(()) };
    let mission: Option<(Option<String>,)> = sqlx::query_as("SELECT mission_id FROM tasks WHERE id = ?").bind(task_id).fetch_optional(&mut **tx).await?;
    let detail = serde_json::json!({"change_id": c.id, "target": c.target, "status": status});
    insert_event(tx, task_id, mission.and_then(|m| m.0).as_deref(), kind, "person", None, None, &detail).await
}

async fn finish_running(tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>, task_id: &str, status: &str, now: DateTime<Utc>) -> Result<(), DbError> {
    let running: Vec<(String, String, Option<String>)> = sqlx::query_as(
        "SELECT a.id, a.strategy, t.mission_id FROM autofix_attempts a JOIN tasks t ON t.id = a.task_id \
         WHERE a.status = 'running' AND (a.task_id = ? OR a.retry_task_id = ?)",
    )
    .bind(task_id)
    .bind(task_id)
    .fetch_all(&mut **tx)
    .await?;
    for (id, strategy, mission) in running {
        let r = sqlx::query("UPDATE autofix_attempts SET status = ?, finished_at = ? WHERE id = ? AND status = 'running'")
            .bind(status)
            .bind(now)
            .bind(&id)
            .execute(&mut **tx)
            .await?;
        if r.rows_affected() == 1 {
            let detail = serde_json::json!({"autofix_id": id, "strategy": strategy, "from": "running", "status": status});
            let owner: (String,) = sqlx::query_as("SELECT task_id FROM autofix_attempts WHERE id = ?").bind(&id).fetch_one(&mut **tx).await?;
            insert_event(tx, &owner.0, mission.as_deref(), "autofix_finished", "kwant", None, None, &detail).await?;
        }
    }
    Ok(())
}
