//! Mission Control Phase D4: Automation Discovery's storage, and what every routine can now carry
//! for the board: a Flight School class, a description of a good result and checks, copied onto
//! each run's task. Add-only: older binaries ignore all of it, and running it twice changes
//! nothing. See docs/mission-control.md §20.

use super::{Database, DbError};
use crate::automation_discovery::{self as miner, PassedCheck, Past, RoutineRef, TaskRow};
use chrono::{DateTime, Duration, Utc};
use serde_json::Value;
use std::collections::HashSet;

/// Routine columns this migration adds, all nullable: a routine without them runs as before.
/// `origin` is `NULL` (made by hand or by a bot) or `discovery`; `checks` is JSON
/// `[{kind, spec, required}]`.
const ROUTINE_COLUMNS: [&str; 5] = ["origin", "suggestion_id", "task_class", "acceptance", "checks"];

/// The tasks the miner learns from (design §4): created by the person on the board, as a task or
/// in a mission, in the space, since `?2`. Not tasks a bot, a routine, a hand-off or a chat made,
/// not retries (forks) of another task, and not cancelled ones.
const DISCOVERY_TASKS: &str = "FROM tasks t
     WHERE t.space_id = ?1 AND t.created_by = 'person' AND t.source_kind IN ('person', 'mission')
       AND t.forked_from IS NULL AND t.state <> 'cancelled' AND t.created_at >= ?2";

type TaskTuple = (String, String, String, String, Option<String>, String, Option<String>, Option<String>, Option<String>, DateTime<Utc>, Option<DateTime<Utc>>);
type RoutineTuple = (String, String, String, Option<String>, Option<String>);

impl Database {
    pub(super) async fn migrate_automation_discovery(&self) -> Result<(), DbError> {
        let cols: Vec<(String,)> = sqlx::query_as("SELECT name FROM pragma_table_info('routines')").fetch_all(&self.pool).await?;
        for column in ROUTINE_COLUMNS {
            if !cols.iter().any(|(c,)| c == column) {
                sqlx::query(&format!("ALTER TABLE routines ADD COLUMN {column} TEXT")).execute(&self.pool).await?;
            }
        }
        for stmt in [
            "CREATE TABLE IF NOT EXISTS routine_suggestions (
                id TEXT PRIMARY KEY,
                space_id TEXT NOT NULL,
                cluster_key TEXT NOT NULL,
                kwant_id TEXT NOT NULL,
                name TEXT NOT NULL,
                instructions TEXT NOT NULL,
                schedule TEXT NOT NULL,
                first_run_at TIMESTAMP NOT NULL,
                delivery TEXT NOT NULL,
                task_class TEXT NOT NULL DEFAULT 'general',
                acceptance TEXT NOT NULL DEFAULT '',
                checks TEXT NOT NULL DEFAULT '[]',
                left_out_checks TEXT NOT NULL DEFAULT '[]',
                chain TEXT NOT NULL DEFAULT '[]',
                source_task_ids TEXT NOT NULL,
                evidence TEXT NOT NULL,
                status TEXT NOT NULL CHECK (status IN ('offered', 'accepted', 'dismissed', 'expired', 'superseded')),
                routine_id TEXT,
                cooldown_until TIMESTAMP,
                routine_deleted_at TIMESTAMP,
                retention TEXT CHECK (retention IS NULL OR retention IN ('retained', 'dropped')),
                retention_at TIMESTAMP,
                offered_at TIMESTAMP NOT NULL,
                decided_at TIMESTAMP
            )",
            // One live offer per habit: racing scans insert with ON CONFLICT DO NOTHING
            "CREATE UNIQUE INDEX IF NOT EXISTS routine_suggestions_one_live
                ON routine_suggestions (space_id, cluster_key) WHERE status = 'offered'",
            "CREATE INDEX IF NOT EXISTS routine_suggestions_key ON routine_suggestions (space_id, cluster_key, decided_at)",
            "CREATE TABLE IF NOT EXISTS routine_suggestion_events (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                suggestion_id TEXT NOT NULL,
                kind TEXT NOT NULL CHECK (kind IN ('offered', 'accepted', 'dismissed', 'expired', 'superseded',
                    'enabled', 'disabled', 'deleted', 'retained', 'dropped')),
                actor TEXT NOT NULL,
                detail TEXT NOT NULL DEFAULT '{}',
                created_at TIMESTAMP NOT NULL
            )",
            "CREATE INDEX IF NOT EXISTS routine_suggestion_events_by ON routine_suggestion_events (suggestion_id, id)",
            // The history is append-only
            "CREATE TRIGGER IF NOT EXISTS routine_suggestion_events_no_update BEFORE UPDATE ON routine_suggestion_events
                BEGIN SELECT RAISE(ABORT, 'routine_suggestion_events is append-only'); END",
            "CREATE TRIGGER IF NOT EXISTS routine_suggestion_events_no_delete BEFORE DELETE ON routine_suggestion_events
                BEGIN SELECT RAISE(ABORT, 'routine_suggestion_events is append-only'); END",
        ] {
            sqlx::query(stmt).execute(&self.pool).await?;
        }
        Ok(())
    }

    /// The person's board tasks of the last [`miner::MONTHLY_WINDOW_DAYS`], oldest first.
    pub async fn discovery_tasks(&self, space: &str, now: DateTime<Utc>) -> Result<Vec<TaskRow>, DbError> {
        let rows: Vec<TaskTuple> = sqlx::query_as(&format!(
            "SELECT t.id, t.title, t.acceptance, t.state, t.owner_kwant_id, t.task_class, t.channel_id, t.direct_kwant_id,
                    t.thread_id, t.created_at, t.finished_at
             {DISCOVERY_TASKS} ORDER BY t.created_at, t.id"
        ))
        .bind(space)
        .bind(now - Duration::days(miner::MONTHLY_WINDOW_DAYS))
        .fetch_all(&self.pool)
        .await?;
        Ok(rows
            .into_iter()
            .map(|(id, title, acceptance, state, owner_kwant_id, task_class, channel_id, direct_kwant_id, thread_id, created_at, finished_at)| TaskRow {
                id,
                title,
                acceptance,
                state,
                owner_kwant_id,
                task_class,
                channel_id,
                direct_kwant_id,
                thread_id,
                created_at,
                finished_at,
            })
            .collect())
    }

    /// Checks that passed on those tasks.
    pub async fn discovery_checks(&self, space: &str, now: DateTime<Utc>) -> Result<Vec<PassedCheck>, DbError> {
        let rows: Vec<(String, String, String, bool)> = sqlx::query_as(&format!(
            "SELECT c.task_id, c.kind, c.spec, c.required FROM outcome_checks c
             WHERE c.status = 'passed' AND c.task_id IN (SELECT t.id {DISCOVERY_TASKS})
             ORDER BY c.created_at, c.id"
        ))
        .bind(space)
        .bind(now - Duration::days(miner::MONTHLY_WINDOW_DAYS))
        .fetch_all(&self.pool)
        .await?;
        Ok(rows.into_iter().map(|(task_id, kind, spec, required)| PassedCheck { task_id, kind, spec, required }).collect())
    }

    /// Every routine of the space, on or off, with the habit a discovered one came from.
    pub async fn discovery_routines(&self, space: &str) -> Result<Vec<RoutineRef>, DbError> {
        let rows: Vec<RoutineTuple> = sqlx::query_as(
            "SELECT r.kwant_id, r.name, r.instructions, r.task_class, s.cluster_key FROM routines r
             LEFT JOIN routine_suggestions s ON s.id = r.suggestion_id
             WHERE r.space_id = ? ORDER BY r.created_at, r.id",
        )
        .bind(space)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows
            .into_iter()
            .map(|(kwant_id, name, instructions, task_class, cluster_key)| RoutineRef { kwant_id, name, instructions, task_class, cluster_key })
            .collect())
    }

    /// What earlier offers in the space mean for a scan now: habits cooling off (dismissed, or
    /// their discovered routine deleted, within [`miner::DISMISS_COOL_OFF_DAYS`]), live offers and
    /// the follow-ups written into them, and how many were made in the last 24 hours.
    pub async fn discovery_past(&self, space: &str, now: DateTime<Utc>) -> Result<Past, DbError> {
        let cooling: Vec<(String,)> = sqlx::query_as(
            "SELECT DISTINCT cluster_key FROM routine_suggestions
             WHERE space_id = ?1 AND ((status = 'dismissed' AND cooldown_until > ?2) OR routine_deleted_at > ?3)",
        )
        .bind(space)
        .bind(now)
        .bind(now - Duration::days(miner::DISMISS_COOL_OFF_DAYS))
        .fetch_all(&self.pool)
        .await?;
        let live: Vec<(String, String)> = sqlx::query_as("SELECT cluster_key, chain FROM routine_suggestions WHERE space_id = ? AND status = 'offered'")
            .bind(space)
            .fetch_all(&self.pool)
            .await?;
        let (offered_24h,): (i64,) = sqlx::query_as("SELECT COUNT(*) FROM routine_suggestions WHERE space_id = ? AND offered_at > ?")
            .bind(space)
            .bind(now - Duration::hours(24))
            .fetch_one(&self.pool)
            .await?;
        let live_chain: HashSet<String> = live
            .iter()
            .filter_map(|(_, chain)| serde_json::from_str::<Vec<Value>>(chain).ok())
            .flatten()
            .filter(|step| step.get("in_instructions").and_then(Value::as_bool) == Some(true))
            .filter_map(|step| step.get("key").and_then(Value::as_str).map(str::to_string))
            .collect();
        Ok(Past {
            cooling: cooling.into_iter().map(|(k,)| k).collect(),
            live_count: live.len(),
            live: live.into_iter().map(|(k, _)| k).collect(),
            live_chain,
            offered_24h: usize::try_from(offered_24h).unwrap_or(0),
        })
    }
}
