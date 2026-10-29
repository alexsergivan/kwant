//! Mission Control Phase D1: the Performance Market's observations. One row per settled task
//! (verified, failed, or cancelled after it ran): who did it, on which model, how long it took,
//! what it spent, how it ended and how often the person stepped in. Report-only: nothing routes
//! on these rows. See docs/mission-control.md §17.

use super::{Database, DbError};
use chrono::{DateTime, Utc};
use serde::Serialize;
use sqlx::FromRow;
use std::collections::HashMap;

/// Set once the finished tasks from before this table existed were observed.
const BACKFILL_DONE: &str = "performance_backfill_v1";

/// One model call of the owner: model, fresh input, output, cost, when.
type UsageRow = (String, i64, i64, Option<f64>, DateTime<Utc>);

#[derive(Debug, Clone, Serialize, FromRow, PartialEq)]
pub struct Observation {
    pub id: i64,
    pub task_id: String,
    pub space_id: String,
    pub mission_id: Option<String>,
    pub task_class: String,
    pub kwant_id: String,
    pub engine: String,
    pub model: String,
    pub started_at: Option<DateTime<Utc>>,
    pub finished_at: DateTime<Utc>,
    /// Time a bot held the task: the sum of its claims.
    pub latency_ms: i64,
    pub runs: i64,
    pub fresh_tokens: i64,
    /// `None` when no model call in the task's claims reported a cost.
    pub cost_usd: Option<f64>,
    /// `verified`, `failed` or `unverified` (cancelled after it ran).
    pub result: String,
    pub approvals: i64,
    pub check_approvals: i64,
    pub unblocks: i64,
    pub retries: i64,
    pub autofix_attempts: i64,
    pub interventions: i64,
    /// `live` (recorded as the task settled) or `backfill`.
    pub source: String,
    pub created_at: DateTime<Utc>,
}

/// An observation before it's stored.
#[derive(Debug, Clone, PartialEq)]
pub struct NewObservation {
    pub task_id: String,
    pub space_id: String,
    pub mission_id: Option<String>,
    pub task_class: String,
    pub kwant_id: String,
    pub engine: String,
    pub model: String,
    pub started_at: Option<DateTime<Utc>>,
    pub finished_at: DateTime<Utc>,
    pub latency_ms: i64,
    pub runs: i64,
    pub fresh_tokens: i64,
    pub cost_usd: Option<f64>,
    pub result: &'static str,
    pub approvals: i64,
    pub check_approvals: i64,
    pub unblocks: i64,
    pub retries: i64,
    pub autofix_attempts: i64,
}

/// The settled state's result; `None` while the task isn't settled.
pub fn result_of(state: &str) -> Option<&'static str> {
    match state {
        "verified" => Some("verified"),
        "failed" => Some("failed"),
        "cancelled" => Some("unverified"),
        _ => None,
    }
}

#[derive(FromRow)]
struct TaskRow {
    id: String,
    space_id: String,
    mission_id: Option<String>,
    state: String,
    owner_kwant_id: Option<String>,
    spent_tokens: i64,
    finished_at: Option<DateTime<Utc>>,
    task_class: String,
}

impl Database {
    pub(super) async fn migrate_performance(&self) -> Result<(), DbError> {
        for stmt in [
            "CREATE TABLE IF NOT EXISTS performance_observations (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                task_id TEXT NOT NULL UNIQUE,
                space_id TEXT NOT NULL,
                mission_id TEXT,
                task_class TEXT NOT NULL,
                kwant_id TEXT NOT NULL,
                engine TEXT NOT NULL,
                model TEXT NOT NULL,
                started_at TIMESTAMP,
                finished_at TIMESTAMP NOT NULL,
                latency_ms INTEGER NOT NULL CHECK (latency_ms >= 0),
                runs INTEGER NOT NULL,
                fresh_tokens INTEGER NOT NULL,
                cost_usd REAL,
                result TEXT NOT NULL CHECK (result IN ('verified', 'failed', 'unverified')),
                approvals INTEGER NOT NULL,
                check_approvals INTEGER NOT NULL,
                unblocks INTEGER NOT NULL,
                retries INTEGER NOT NULL,
                autofix_attempts INTEGER NOT NULL,
                interventions INTEGER NOT NULL,
                source TEXT NOT NULL CHECK (source IN ('live', 'backfill')),
                created_at TIMESTAMP NOT NULL
            )",
            "CREATE INDEX IF NOT EXISTS performance_space_class ON performance_observations (space_id, task_class, finished_at)",
        ] {
            sqlx::query(stmt).execute(&self.pool).await?;
        }
        // Best-effort: a backfill that fails never stops kwant from starting
        if let Err(e) = self.backfill_performance().await {
            tracing::warn!(error = %e, "performance backfill failed; it runs again on the next start");
        }
        Ok(())
    }

    /// Observes every settled task once (`source = 'backfill'`), leaving rows that exist alone.
    /// Runs at most once per database (a setting marks it done). The number of rows added.
    pub async fn backfill_performance(&self) -> Result<u64, DbError> {
        if self.get_setting(BACKFILL_DONE).await?.is_some() {
            return Ok(0);
        }
        let ids: Vec<(String,)> = sqlx::query_as(
            "SELECT id FROM tasks WHERE finished_at IS NOT NULL AND state IN ('verified', 'failed', 'cancelled') ORDER BY finished_at",
        )
        .fetch_all(&self.pool)
        .await?;
        let mut added = 0;
        for (id,) in ids {
            match self.observe_task(&id).await {
                Ok(Some(o)) => added += self.store_observation(&o, "backfill").await?,
                Ok(None) => {}
                Err(e) => tracing::warn!(task_id = %id, error = %e, "task skipped in the performance backfill"),
            }
        }
        self.set_setting(BACKFILL_DONE, &Utc::now().to_rfc3339()).await?;
        Ok(added)
    }

    /// What a settled task tells about its owner. `None` while it isn't settled, when it has no
    /// owner, or when it was cancelled before any bot ran it.
    pub async fn observe_task(&self, task_id: &str) -> Result<Option<NewObservation>, DbError> {
        let Some(t) = sqlx::query_as::<_, TaskRow>(
            "SELECT id, space_id, mission_id, state, owner_kwant_id, spent_tokens, finished_at, task_class FROM tasks WHERE id = ?",
        )
        .bind(task_id)
        .fetch_optional(&self.pool)
        .await?
        else {
            return Ok(None);
        };
        let (Some(result), Some(kwant_id), Some(finished_at)) = (result_of(&t.state), t.owner_kwant_id.clone(), t.finished_at) else {
            return Ok(None);
        };
        // Claims: the windows a bot held the task (an open one ends at the settle)
        let claims: Vec<(i64, Option<i64>)> =
            sqlx::query_as("SELECT claimed_at, released_at FROM agent_claims WHERE task_id = ? ORDER BY claimed_at").bind(&t.id).fetch_all(&self.pool).await?;
        if claims.is_empty() && result == "unverified" {
            return Ok(None);
        }
        let end_ms = finished_at.timestamp_millis();
        let windows: Vec<(i64, i64)> = claims.iter().map(|&(a, b)| (a, b.unwrap_or(end_ms).max(a))).collect();
        let latency_ms = windows.iter().map(|(a, b)| b - a).sum();
        let started_at = windows.first().and_then(|(a, _)| DateTime::<Utc>::from_timestamp_millis(*a));

        // Model and cost: the owner's model calls inside the claim windows
        let (mut by_model, mut cost, mut any_cost) = (HashMap::<String, i64>::new(), 0.0, false);
        if let (Some(first), Some(last)) = (windows.iter().map(|w| w.0).min(), windows.iter().map(|w| w.1).max()) {
            // A second of slack on each side for the stored time format; the windows decide below
            let from = DateTime::<Utc>::from_timestamp_millis(first - 1000).unwrap_or(finished_at);
            let to = DateTime::<Utc>::from_timestamp_millis(last + 1000).unwrap_or(finished_at);
            let calls: Vec<UsageRow> = sqlx::query_as(
                "SELECT model, fresh_input, output, cost_usd, created_at FROM model_usage WHERE kwant_id = ? AND created_at >= ? AND created_at <= ?",
            )
            .bind(&kwant_id)
            .bind(from)
            .bind(to)
            .fetch_all(&self.pool)
            .await?;
            for (model, fresh, output, c, at) in calls {
                let ms = at.timestamp_millis();
                if !windows.iter().any(|(a, b)| ms >= *a && ms <= *b) {
                    continue;
                }
                *by_model.entry(model).or_default() += fresh + output;
                if let Some(c) = c {
                    cost += c;
                    any_cost = true;
                }
            }
        }
        let bot: Option<(String, String)> =
            sqlx::query_as("SELECT driver_type, preferred_model FROM kwants WHERE id = ?").bind(&kwant_id).fetch_optional(&self.pool).await?;
        let engine = bot.as_ref().map(|b| b.0.clone()).unwrap_or_else(|| "unknown".into());
        let model = by_model
            .into_iter()
            .max_by(|a, b| a.1.cmp(&b.1).then_with(|| b.0.cmp(&a.0)))
            .map(|(m, _)| m)
            .or_else(|| bot.map(|b| b.1).filter(|m| !m.is_empty()))
            .unwrap_or_else(|| "unknown".into());

        // Interventions
        let (unblocks, retries): (i64, i64) = sqlx::query_as(
            "SELECT \
               COALESCE(SUM(from_state IN ('blocked', 'needs_you') AND to_state NOT IN ('cancelled', 'failed')), 0), \
               COALESCE(SUM(from_state = 'failed' AND to_state = 'ready'), 0) \
             FROM task_events WHERE task_id = ? AND kind = 'state' AND actor = 'person'",
        )
        .bind(&t.id)
        .fetch_one(&self.pool)
        .await?;
        let check_approvals: (i64,) = sqlx::query_as(
            "SELECT COUNT(*) FROM task_events WHERE task_id = ? AND kind = 'check_added' AND actor = 'person' AND json_extract(detail, '$.approval') = 1",
        )
        .bind(&t.id)
        .fetch_one(&self.pool)
        .await?;
        let approvals: (i64,) = sqlx::query_as(
            "SELECT COUNT(*) FROM turn_tool_calls WHERE task_id = ? AND approval_id IS NOT NULL AND decision IN ('approved', 'denied')",
        )
        .bind(&t.id)
        .fetch_one(&self.pool)
        .await?;
        let autofix: (i64,) = sqlx::query_as(
            "SELECT COUNT(*) FROM autofix_attempts WHERE task_id = ? AND status IN ('scheduled', 'running', 'succeeded', 'failed')",
        )
        .bind(&t.id)
        .fetch_one(&self.pool)
        .await?;

        Ok(Some(NewObservation {
            task_id: t.id,
            space_id: t.space_id,
            mission_id: t.mission_id,
            task_class: t.task_class,
            kwant_id,
            engine,
            model,
            started_at,
            finished_at,
            latency_ms,
            runs: claims.len() as i64,
            fresh_tokens: t.spent_tokens,
            cost_usd: any_cost.then_some(cost),
            result,
            approvals: approvals.0,
            check_approvals: check_approvals.0,
            unblocks,
            retries,
            autofix_attempts: autofix.0,
        }))
    }

    /// Stores `o`. A `live` row replaces the task's earlier one (a task reopened and settled
    /// again is observed by its last outcome); a `backfill` row never replaces anything.
    /// The number of rows written (0 or 1).
    pub async fn store_observation(&self, o: &NewObservation, source: &str) -> Result<u64, DbError> {
        let on_conflict = if source == "live" {
            "ON CONFLICT(task_id) DO UPDATE SET space_id = excluded.space_id, mission_id = excluded.mission_id, \
             task_class = excluded.task_class, kwant_id = excluded.kwant_id, engine = excluded.engine, model = excluded.model, \
             started_at = excluded.started_at, finished_at = excluded.finished_at, latency_ms = excluded.latency_ms, \
             runs = excluded.runs, fresh_tokens = excluded.fresh_tokens, cost_usd = excluded.cost_usd, result = excluded.result, \
             approvals = excluded.approvals, check_approvals = excluded.check_approvals, unblocks = excluded.unblocks, \
             retries = excluded.retries, autofix_attempts = excluded.autofix_attempts, interventions = excluded.interventions, \
             source = excluded.source, created_at = excluded.created_at"
        } else {
            "ON CONFLICT(task_id) DO NOTHING"
        };
        let interventions = o.approvals + o.check_approvals + o.unblocks + o.retries + o.autofix_attempts;
        let r = sqlx::query(&format!(
            "INSERT INTO performance_observations (task_id, space_id, mission_id, task_class, kwant_id, engine, model, started_at, \
             finished_at, latency_ms, runs, fresh_tokens, cost_usd, result, approvals, check_approvals, unblocks, retries, \
             autofix_attempts, interventions, source, created_at) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?) {on_conflict}"
        ))
        .bind(&o.task_id)
        .bind(&o.space_id)
        .bind(&o.mission_id)
        .bind(&o.task_class)
        .bind(&o.kwant_id)
        .bind(&o.engine)
        .bind(&o.model)
        .bind(o.started_at)
        .bind(o.finished_at)
        .bind(o.latency_ms)
        .bind(o.runs)
        .bind(o.fresh_tokens)
        .bind(o.cost_usd)
        .bind(o.result)
        .bind(o.approvals)
        .bind(o.check_approvals)
        .bind(o.unblocks)
        .bind(o.retries)
        .bind(o.autofix_attempts)
        .bind(interventions)
        .bind(source)
        .bind(Utc::now())
        .execute(&self.pool)
        .await?;
        Ok(r.rows_affected())
    }

    /// Observes `task_id` as it settles and stores it (`live`). `false` when there was nothing to observe.
    pub async fn record_performance(&self, task_id: &str) -> Result<bool, DbError> {
        match self.observe_task(task_id).await? {
            Some(o) => Ok(self.store_observation(&o, "live").await? > 0),
            None => Ok(false),
        }
    }

    /// The space's observations that settled at or after `since`, optionally of one class and bot.
    pub async fn observations(&self, space_id: &str, since: DateTime<Utc>, class: Option<&str>, kwant_id: Option<&str>) -> Result<Vec<Observation>, DbError> {
        Ok(sqlx::query_as::<_, Observation>(
            "SELECT * FROM performance_observations WHERE space_id = ? AND finished_at >= ? \
             AND (? IS NULL OR task_class = ?) AND (? IS NULL OR kwant_id = ?) ORDER BY finished_at",
        )
        .bind(space_id)
        .bind(since)
        .bind(class)
        .bind(class)
        .bind(kwant_id)
        .bind(kwant_id)
        .fetch_all(&self.pool)
        .await?)
    }

    pub async fn observation_of(&self, task_id: &str) -> Result<Option<Observation>, DbError> {
        Ok(sqlx::query_as::<_, Observation>("SELECT * FROM performance_observations WHERE task_id = ?").bind(task_id).fetch_optional(&self.pool).await?)
    }
}
