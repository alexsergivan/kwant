//! Mission Control Phase D4: Automation Discovery's storage, and what every routine can now carry
//! for the board: a Flight School class, a description of a good result and checks, copied onto
//! each run's task. Add-only: older binaries ignore all of it, and running it twice changes
//! nothing. See docs/mission-control.md §20.

use super::continuity::{insert_routine, NewRoutine, RoutineRow};
use super::{Database, DbError};
use crate::automation_discovery::{self as miner, Offer, PassedCheck, Past, RoutineRef, TaskRow};
use chrono::{DateTime, Duration, Utc};
use serde_json::{json, Value};
use sqlx::{FromRow, SqliteConnection};
use std::collections::HashSet;
use uuid::Uuid;

/// An accepted suggestion's routine counts as kept (`retained`) or not (`dropped`) this long after
/// Accept (design §10).
pub const RETENTION_DAYS: i64 = 14;

/// One stored offer, as `routine_suggestions` holds it (JSON columns as text).
#[derive(Debug, Clone, FromRow)]
pub struct SuggestionRow {
    pub id: String,
    pub space_id: String,
    pub cluster_key: String,
    pub kwant_id: String,
    pub name: String,
    pub instructions: String,
    pub schedule: String,
    pub first_run_at: DateTime<Utc>,
    pub delivery: String,
    pub task_class: String,
    pub acceptance: String,
    pub checks: String,
    pub left_out_checks: String,
    pub chain: String,
    pub source_task_ids: String,
    pub evidence: String,
    pub status: String,
    pub routine_id: Option<String>,
    pub cooldown_until: Option<DateTime<Utc>>,
    pub routine_deleted_at: Option<DateTime<Utc>>,
    pub retention: Option<String>,
    pub retention_at: Option<DateTime<Utc>>,
    pub offered_at: DateTime<Utc>,
    pub decided_at: Option<DateTime<Utc>>,
}

/// What Accept or Dismiss did.
#[derive(Debug)]
pub enum Decided {
    /// Done: the suggestion now, and for Accept the routine it made.
    Done(Box<SuggestionRow>, Option<Box<RoutineRow>>),
    /// It wasn't a live offer any more: as it is now.
    NotOffered(Box<SuggestionRow>),
    /// No such suggestion in the space.
    Missing,
}

/// Counts for Insights (design §10).
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize)]
pub struct SuggestionStats {
    pub offered_30d: i64,
    pub accepted_30d: i64,
    pub dismissed_30d: i64,
    pub retained: i64,
    pub dropped: i64,
}

/// What the source tasks' turns did (design §5): tool calls with an outside effect, with a
/// workspace effect, and how many of them the person had to approve.
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize)]
pub struct PastTools {
    pub external: i64,
    pub workspace: i64,
    pub needed_ok: i64,
}

const SUGGESTION_SELECT: &str = "SELECT id, space_id, cluster_key, kwant_id, name, instructions, schedule, first_run_at, delivery,
    task_class, acceptance, checks, left_out_checks, chain, source_task_ids, evidence, status, routine_id, cooldown_until,
    routine_deleted_at, retention, retention_at, offered_at, decided_at FROM routine_suggestions";

fn text<T: serde::Serialize + ?Sized>(v: &T) -> String {
    serde_json::to_string(v).unwrap_or_default()
}

/// Appends to a suggestion's history.
async fn event(conn: &mut SqliteConnection, suggestion: &str, kind: &str, actor: &str, detail: Value, now: DateTime<Utc>) -> Result<(), DbError> {
    sqlx::query("INSERT INTO routine_suggestion_events (suggestion_id, kind, actor, detail, created_at) VALUES (?, ?, ?, ?, ?)")
        .bind(suggestion)
        .bind(kind)
        .bind(actor)
        .bind(detail.to_string())
        .bind(now)
        .execute(conn)
        .await?;
    Ok(())
}

/// In the transaction that deletes routine `routine`: when it was discovered, its suggestion
/// notes the delete (so its habit cools off for 30 days) and the history gets `deleted`.
pub(super) async fn routine_deleted(conn: &mut SqliteConnection, routine: &str, now: DateTime<Utc>) -> Result<(), DbError> {
    let ids: Vec<(String,)> = sqlx::query_as("UPDATE routine_suggestions SET routine_deleted_at = ? WHERE routine_id = ? AND routine_deleted_at IS NULL RETURNING id")
        .bind(now)
        .bind(routine)
        .fetch_all(&mut *conn)
        .await?;
    for (id,) in ids {
        event(conn, &id, "deleted", "kwant", json!({ "routine_id": routine }), now).await?;
    }
    Ok(())
}

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

    /// Stores `offer` (its delivery and checks as they should be kept, specs unmasked) as a live
    /// offer of `space`, unless the habit already has one or the space is at its limits: fewer
    /// than `max_live` live offers and fewer than `daily_cap` offered in the last 24 hours. One
    /// statement checks and inserts, so racing scans can't pass a limit or make two. The id, when
    /// stored.
    pub async fn store_offer(&self, space: &str, offer: &Offer, now: DateTime<Utc>, daily_cap: i64, max_live: usize) -> Result<Option<String>, DbError> {
        let id = format!("sug_{}", Uuid::new_v4().simple());
        let mut evidence = serde_json::to_value(&offer.evidence).unwrap_or_default();
        evidence["sources"] = serde_json::to_value(&offer.sources).unwrap_or_default();
        let mut tx = self.pool.begin().await?;
        let res = sqlx::query(
            "INSERT INTO routine_suggestions (id, space_id, cluster_key, kwant_id, name, instructions, schedule, first_run_at, delivery,
                task_class, acceptance, checks, left_out_checks, chain, source_task_ids, evidence, status, offered_at)
             SELECT ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, 'offered', ?17
             WHERE (SELECT COUNT(*) FROM routine_suggestions WHERE space_id = ?2 AND status = 'offered') < ?18
               AND (SELECT COUNT(*) FROM routine_suggestions WHERE space_id = ?2 AND offered_at > ?19) < ?20
             ON CONFLICT DO NOTHING",
        )
        .bind(&id)
        .bind(space)
        .bind(&offer.cluster_key)
        .bind(&offer.kwant_id)
        .bind(&offer.name)
        .bind(&offer.instructions)
        .bind(text(&offer.schedule))
        .bind(offer.first_run_at)
        .bind(text(&offer.delivery))
        .bind(&offer.task_class)
        .bind(&offer.acceptance)
        .bind(text(&offer.checks))
        .bind(text(&offer.left_out_checks))
        .bind(text(&offer.chain))
        .bind(text(&offer.source_task_ids))
        .bind(evidence.to_string())
        .bind(now)
        .bind(i64::try_from(max_live).unwrap_or(i64::MAX))
        .bind(now - Duration::hours(24))
        .bind(daily_cap)
        .execute(&mut *tx)
        .await?;
        if res.rows_affected() == 0 {
            tx.rollback().await?;
            return Ok(None);
        }
        event(&mut tx, &id, "offered", "kwant", json!({ "cluster_key": offer.cluster_key }), now).await?;
        tx.commit().await?;
        Ok(Some(id))
    }

    pub async fn get_suggestion(&self, id: &str) -> Result<Option<SuggestionRow>, DbError> {
        Ok(sqlx::query_as::<_, SuggestionRow>(&format!("{SUGGESTION_SELECT} WHERE id = ?")).bind(id).fetch_optional(&self.pool).await?)
    }

    /// The space's suggestions, newest first; `status` filters (`None`: all).
    pub async fn list_suggestions(&self, space: &str, status: Option<&str>) -> Result<Vec<SuggestionRow>, DbError> {
        Ok(sqlx::query_as::<_, SuggestionRow>(&format!(
            "{SUGGESTION_SELECT} WHERE space_id = ?1 AND (?2 IS NULL OR status = ?2) ORDER BY offered_at DESC, id"
        ))
        .bind(space)
        .bind(status)
        .fetch_all(&self.pool)
        .await?)
    }

    /// Ends live offer `id` as `expired` or `superseded` (a scan found its habit gone or
    /// changed). False when it wasn't live.
    pub async fn end_offer(&self, id: &str, kind: &str, now: DateTime<Utc>) -> Result<bool, DbError> {
        let mut tx = self.pool.begin().await?;
        let res = sqlx::query("UPDATE routine_suggestions SET status = ?, decided_at = ? WHERE id = ? AND status = 'offered'")
            .bind(kind)
            .bind(now)
            .bind(id)
            .execute(&mut *tx)
            .await?;
        if res.rows_affected() == 0 {
            tx.rollback().await?;
            return Ok(false);
        }
        event(&mut tx, id, kind, "kwant", json!({}), now).await?;
        tx.commit().await?;
        Ok(true)
    }

    /// The decision made on a suggestion that is no longer live, or that isn't in `space`.
    async fn not_offered(&self, id: &str, space: &str) -> Result<Decided, DbError> {
        Ok(match self.get_suggestion(id).await? {
            Some(s) if s.space_id == space => Decided::NotOffered(Box::new(s)),
            _ => Decided::Missing,
        })
    }

    /// Accept (design §6): in one transaction the live offer becomes `accepted`, `routine` (built
    /// and validated by the caller, `origin` and `suggestion_id` set) is created and linked, and
    /// the history gets `accepted`. Either all of it happens or none.
    pub async fn accept_suggestion(&self, id: &str, space: &str, routine: &NewRoutine, now: DateTime<Utc>) -> Result<Decided, DbError> {
        let routine_space = self.routine_space(routine).await?;
        if routine_space.as_str() != space {
            return Err(DbError::OtherSpace("the bot is in another space".into()));
        }
        let routine_id = Uuid::new_v4().to_string();
        let mut tx = self.pool.begin().await?;
        // A write first, so the transaction holds the lock from its first statement. From here on
        // the suggestion's first run is the routine's
        let res = sqlx::query(
            "UPDATE routine_suggestions SET status = 'accepted', decided_at = ?, routine_id = ?, first_run_at = COALESCE(?, first_run_at)
             WHERE id = ? AND space_id = ? AND status = 'offered'",
        )
        .bind(now)
        .bind(&routine_id)
        .bind(routine.next_run_at)
        .bind(id)
        .bind(space)
        .execute(&mut *tx)
        .await?;
        if res.rows_affected() == 0 {
            tx.rollback().await?;
            return self.not_offered(id, space).await;
        }
        insert_routine(&mut tx, &routine_id, &routine_space, routine).await?;
        event(&mut tx, id, "accepted", "person", json!({ "routine_id": routine_id }), now).await?;
        tx.commit().await?;
        let s = self.get_suggestion(id).await?.ok_or(DbError::Sqlx(sqlx::Error::RowNotFound))?;
        let r = self.get_routine(&routine_id).await?.ok_or(DbError::Sqlx(sqlx::Error::RowNotFound))?;
        Ok(Decided::Done(Box::new(s), Some(Box::new(r))))
    }

    /// Dismiss (design §6): the habit isn't offered again for [`miner::DISMISS_COOL_OFF_DAYS`].
    pub async fn dismiss_suggestion(&self, id: &str, space: &str, now: DateTime<Utc>) -> Result<Decided, DbError> {
        let until = now + Duration::days(miner::DISMISS_COOL_OFF_DAYS);
        let mut tx = self.pool.begin().await?;
        let res = sqlx::query(
            "UPDATE routine_suggestions SET status = 'dismissed', decided_at = ?, cooldown_until = ? WHERE id = ? AND space_id = ? AND status = 'offered'",
        )
        .bind(now)
        .bind(until)
        .bind(id)
        .bind(space)
        .execute(&mut *tx)
        .await?;
        if res.rows_affected() == 0 {
            tx.rollback().await?;
            return self.not_offered(id, space).await;
        }
        event(&mut tx, id, "dismissed", "person", json!({ "cooldown_until": until }), now).await?;
        tx.commit().await?;
        let s = self.get_suggestion(id).await?.ok_or(DbError::Sqlx(sqlx::Error::RowNotFound))?;
        Ok(Decided::Done(Box::new(s), None))
    }

    /// A discovered routine switched on or off (design §8): `enabled` or `disabled` in its
    /// suggestion's history.
    pub async fn suggestion_switched(&self, suggestion: &str, enabled: bool, actor: &str) -> Result<(), DbError> {
        let mut conn = self.pool.acquire().await?;
        event(&mut conn, suggestion, if enabled { "enabled" } else { "disabled" }, actor, json!({}), Utc::now()).await
    }

    /// An undone delete of a discovered routine: the new routine `routine` is again the one
    /// accepted from `suggestion`, so it keeps "Suggested by kwant" and its habit stops cooling
    /// off. Only for that suggestion's accepted, space-matching link, and only while no other
    /// routine holds it. False when nothing was linked.
    pub async fn relink_discovered(&self, routine: &str, suggestion: &str) -> Result<bool, DbError> {
        let mut tx = self.pool.begin().await?;
        let res = sqlx::query(
            "UPDATE routines SET origin = 'discovery', suggestion_id = ?1 WHERE id = ?2
               AND EXISTS (SELECT 1 FROM routine_suggestions s WHERE s.id = ?1 AND s.status = 'accepted' AND s.space_id = routines.space_id)
               AND NOT EXISTS (SELECT 1 FROM routines o WHERE o.suggestion_id = ?1 AND o.id <> ?2)",
        )
        .bind(suggestion)
        .bind(routine)
        .execute(&mut *tx)
        .await?;
        if res.rows_affected() == 0 {
            tx.rollback().await?;
            return Ok(false);
        }
        sqlx::query("UPDATE routine_suggestions SET routine_id = ?, routine_deleted_at = NULL WHERE id = ?")
            .bind(routine)
            .bind(suggestion)
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        Ok(true)
    }

    /// The daily retention pass (design §10), for every space: accepted suggestions decided at
    /// least [`RETENTION_DAYS`] ago get `retained` if their routine exists and is on now, else
    /// `dropped`. Written once, never recomputed. How many it marked.
    pub async fn mark_retention(&self, now: DateTime<Utc>) -> Result<usize, DbError> {
        let mut tx = self.pool.begin().await?;
        let marked: Vec<(String, String)> = sqlx::query_as(
            "UPDATE routine_suggestions SET retention = CASE WHEN EXISTS
                 (SELECT 1 FROM routines r WHERE r.id = routine_suggestions.routine_id AND r.enabled) THEN 'retained' ELSE 'dropped' END,
               retention_at = ?1
             WHERE status = 'accepted' AND retention IS NULL AND decided_at <= ?2
             RETURNING id, retention",
        )
        .bind(now)
        .bind(now - Duration::days(RETENTION_DAYS))
        .fetch_all(&mut *tx)
        .await?;
        for (id, retention) in &marked {
            event(&mut tx, id, retention, "kwant", json!({}), now).await?;
        }
        tx.commit().await?;
        Ok(marked.len())
    }

    pub async fn suggestion_stats(&self, space: &str, now: DateTime<Utc>) -> Result<SuggestionStats, DbError> {
        let since = now - Duration::days(30);
        let (offered_30d, accepted_30d, dismissed_30d, retained, dropped): (i64, i64, i64, i64, i64) = sqlx::query_as(
            "SELECT COALESCE(SUM(offered_at > ?2), 0),
                    COALESCE(SUM(status = 'accepted' AND decided_at > ?2), 0),
                    COALESCE(SUM(status = 'dismissed' AND decided_at > ?2), 0),
                    COALESCE(SUM(retention = 'retained'), 0),
                    COALESCE(SUM(retention = 'dropped'), 0)
             FROM routine_suggestions WHERE space_id = ?1",
        )
        .bind(space)
        .bind(since)
        .fetch_one(&self.pool)
        .await?;
        Ok(SuggestionStats { offered_30d, accepted_30d, dismissed_30d, retained, dropped })
    }

    /// What the turns of `tasks` did (design §5), from `turn_tool_calls`.
    pub async fn past_tools(&self, tasks: &[String]) -> Result<PastTools, DbError> {
        if tasks.is_empty() {
            return Ok(PastTools::default());
        }
        let list = serde_json::to_string(tasks).unwrap_or_default();
        let (external, workspace, needed_ok): (i64, i64, i64) = sqlx::query_as(
            "SELECT COALESCE(SUM(effect = 'external'), 0), COALESCE(SUM(effect = 'workspace'), 0), COALESCE(SUM(decision = 'approved'), 0)
             FROM turn_tool_calls WHERE task_id IN (SELECT value FROM json_each(?))",
        )
        .bind(list)
        .fetch_one(&self.pool)
        .await?;
        Ok(PastTools { external, workspace, needed_ok })
    }
}
