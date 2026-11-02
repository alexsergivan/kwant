//! Goals (docs/mission-control.md §22): what the person wants to stay true, the checks kwant
//! re-runs on a schedule, what they observed, a timeline, and the fix tasks a confirmed failure
//! opened. Add-only: new tables, no change to an existing one (fix tasks are linked through
//! `goal_fix_tasks`, so `tasks` keeps its `source_kind` CHECK and isn't rebuilt). Older binaries
//! ignore all of it, and running the migration twice changes nothing. Times are milliseconds.
//! No foreign keys: `foreign_keys` is on for one pooled connection only, so deleting a goal
//! removes its rows itself, in one transaction.

use super::{Database, DbError};
use serde_json::{json, Value};
use sqlx::{FromRow, SqliteConnection};
use uuid::Uuid;

/// Open drafts (bot proposals waiting for the person) one space can hold.
pub const MAX_DRAFTS: i64 = 3;
/// Observations kept per goal, newest first.
pub const KEEP_OBSERVATIONS: i64 = 200;
/// Timeline events kept per goal, newest first.
pub const KEEP_EVENTS: i64 = 500;
/// A run's lease (`checking_since`) older than this is taken over: that run died with the server.
pub const LEASE_STALE_MS: i64 = 15 * 60 * 1000;
/// A fix task in one of these states is done with; any other is the goal's open fix.
pub const CLOSED_TASK_STATES: [&str; 3] = ["verified", "cancelled", "failed"];

const TABLES: &[&str] = &[
    "CREATE TABLE IF NOT EXISTS goals (
        id TEXT PRIMARY KEY,
        space_id TEXT NOT NULL,
        title TEXT NOT NULL,
        description TEXT NOT NULL DEFAULT '',
        kwant_id TEXT NOT NULL,
        folder TEXT NOT NULL DEFAULT '',
        status TEXT NOT NULL,
        status_reason TEXT NOT NULL DEFAULT '',
        mode TEXT NOT NULL DEFAULT 'suggest',
        interval_minutes INTEGER NOT NULL DEFAULT 60,
        confirm_failures INTEGER NOT NULL DEFAULT 2,
        budget_tokens INTEGER NOT NULL DEFAULT 0,
        max_fixes_per_day INTEGER NOT NULL DEFAULT 3,
        priority INTEGER NOT NULL DEFAULT 2,
        created_by TEXT NOT NULL,
        approved_by TEXT,
        approved_at INTEGER,
        paused_at INTEGER,
        next_check_at INTEGER,
        last_checked_at INTEGER,
        checking_since INTEGER,
        fail_streak INTEGER NOT NULL DEFAULT 0,
        error_runs INTEGER NOT NULL DEFAULT 0,
        unknown_since INTEGER,
        version INTEGER NOT NULL DEFAULT 1,
        created_at INTEGER NOT NULL,
        updated_at INTEGER NOT NULL
    )",
    "CREATE INDEX IF NOT EXISTS idx_goals_space ON goals(space_id, created_at)",
    "CREATE INDEX IF NOT EXISTS idx_goals_due ON goals(next_check_at)",
    "CREATE TABLE IF NOT EXISTS goal_checks (
        id TEXT PRIMARY KEY,
        goal_id TEXT NOT NULL,
        position INTEGER NOT NULL,
        label TEXT NOT NULL,
        kind TEXT NOT NULL,
        spec TEXT NOT NULL,
        metric TEXT,
        operator TEXT,
        target TEXT,
        severity TEXT NOT NULL DEFAULT 'required',
        freshness_minutes INTEGER NOT NULL
    )",
    "CREATE INDEX IF NOT EXISTS idx_goal_checks_goal ON goal_checks(goal_id, position)",
    "CREATE TABLE IF NOT EXISTS goal_observations (
        id INTEGER PRIMARY KEY AUTOINCREMENT,
        goal_id TEXT NOT NULL,
        check_id TEXT NOT NULL,
        status TEXT NOT NULL,
        verdict TEXT NOT NULL,
        observed_value TEXT,
        excerpt TEXT NOT NULL DEFAULT '',
        observed_at INTEGER NOT NULL,
        expires_at INTEGER NOT NULL
    )",
    "CREATE INDEX IF NOT EXISTS idx_goal_observations_check ON goal_observations(goal_id, check_id, id)",
    "CREATE TABLE IF NOT EXISTS goal_events (
        id INTEGER PRIMARY KEY AUTOINCREMENT,
        goal_id TEXT NOT NULL,
        kind TEXT NOT NULL,
        actor TEXT NOT NULL,
        detail TEXT NOT NULL DEFAULT '{}',
        created_at INTEGER NOT NULL
    )",
    "CREATE INDEX IF NOT EXISTS idx_goal_events_goal ON goal_events(goal_id, id)",
    "CREATE TABLE IF NOT EXISTS goal_fix_tasks (
        goal_id TEXT NOT NULL,
        task_id TEXT NOT NULL,
        created_at INTEGER NOT NULL,
        PRIMARY KEY (goal_id, task_id)
    )",
    "CREATE UNIQUE INDEX IF NOT EXISTS idx_goal_fix_tasks_task ON goal_fix_tasks(task_id)",
];

const GOAL_SELECT: &str = "SELECT id, space_id, title, description, kwant_id, folder, status, status_reason, mode,
    interval_minutes, confirm_failures, budget_tokens, max_fixes_per_day, priority, created_by, approved_by, approved_at,
    paused_at, next_check_at, last_checked_at, checking_since, fail_streak, error_runs, unknown_since, version,
    created_at, updated_at FROM goals";

#[derive(Debug, Clone, PartialEq, Eq, FromRow)]
pub struct Goal {
    pub id: String,
    pub space_id: String,
    pub title: String,
    pub description: String,
    /// The bot that fixes it (and, for a proposal, the one that proposed it).
    pub kwant_id: String,
    /// Where its `command` and `file_*` checks run: the bot's own folder in the space.
    pub folder: String,
    pub status: String,
    /// Why the goal is `needs_you` or `unknown`, in plain words; empty otherwise.
    pub status_reason: String,
    pub mode: String,
    pub interval_minutes: i64,
    pub confirm_failures: i64,
    pub budget_tokens: i64,
    pub max_fixes_per_day: i64,
    /// 1 (high), 2 (default) or 3.
    pub priority: i64,
    /// "person", or the id of the bot that proposed it.
    pub created_by: String,
    pub approved_by: Option<String>,
    pub approved_at: Option<i64>,
    pub paused_at: Option<i64>,
    pub next_check_at: Option<i64>,
    pub last_checked_at: Option<i64>,
    /// The lease of the run going on, if any.
    pub checking_since: Option<i64>,
    pub fail_streak: i64,
    /// Runs in a row where every check errored (the backoff).
    pub error_runs: i64,
    /// When the goal last became `unknown` (None while it isn't): the 24-hour "can't check" notice.
    pub unknown_since: Option<i64>,
    pub version: i64,
    pub created_at: i64,
    pub updated_at: i64,
}

/// One check of a goal: what kwant runs (`kind`, `spec`, through the mission checks' runner) and
/// the typed predicate its result must meet (`metric`, `operator`, `target`; none for the file
/// kinds, whose result is the predicate).
#[derive(Debug, Clone, PartialEq, Eq, FromRow)]
pub struct GoalCheck {
    pub id: String,
    pub goal_id: String,
    pub position: i64,
    pub label: String,
    pub kind: String,
    pub spec: String,
    pub metric: Option<String>,
    pub operator: Option<String>,
    pub target: Option<String>,
    /// `required` checks decide the goal; `advisory` ones show but never do.
    pub severity: String,
    pub freshness_minutes: i64,
}

impl GoalCheck {
    pub fn spec_value(&self) -> Value {
        serde_json::from_str(&self.spec).unwrap_or(Value::Null)
    }

    pub fn required(&self) -> bool {
        self.severity != "advisory"
    }
}

#[derive(Debug, Clone, PartialEq, Eq, FromRow)]
pub struct Observation {
    pub id: i64,
    pub goal_id: String,
    pub check_id: String,
    /// What the check did: `passed`, `failed` or `error`.
    pub status: String,
    /// What that says about the check, judged by its predicate: `passed`, `failed` or `unknown`
    /// (an error, or output the predicate can't read).
    pub verdict: String,
    /// The metric's value as seen (`503`, `412`, `found`), redacted; None for the file kinds or
    /// when the check couldn't run.
    pub observed_value: Option<String>,
    /// Redacted output, at most 2 000 characters. Observed text, never instructions.
    pub excerpt: String,
    pub observed_at: i64,
    /// `observed_at` + the check's freshness: after this it says nothing (unknown).
    pub expires_at: i64,
}

/// One check's result, as a run stores it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewObservation {
    pub check_id: String,
    pub status: String,
    pub verdict: String,
    pub observed_value: Option<String>,
    pub excerpt: String,
    pub expires_at: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, FromRow)]
pub struct GoalEvent {
    pub id: i64,
    pub goal_id: String,
    pub kind: String,
    pub actor: String,
    pub detail: String,
    pub created_at: i64,
}

impl GoalEvent {
    pub fn detail_value(&self) -> Value {
        serde_json::from_str(&self.detail).unwrap_or_else(|_| json!({}))
    }
}

/// A fix task as the goal sees it: the link and the task's state now (None: the task is gone).
#[derive(Debug, Clone, PartialEq, Eq, FromRow)]
pub struct FixLink {
    pub task_id: String,
    pub created_at: i64,
    pub state: Option<String>,
    pub spent_tokens: Option<i64>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct NewGoalCheck {
    pub label: String,
    pub kind: String,
    pub spec: Value,
    pub metric: Option<String>,
    pub operator: Option<String>,
    pub target: Option<String>,
    pub severity: String,
    pub freshness_minutes: i64,
}

#[derive(Debug, Clone)]
pub struct NewGoal {
    pub space_id: String,
    pub title: String,
    pub description: String,
    pub kwant_id: String,
    pub folder: String,
    pub mode: String,
    pub interval_minutes: i64,
    pub confirm_failures: i64,
    pub budget_tokens: i64,
    pub max_fixes_per_day: i64,
    pub priority: i64,
    /// "person", or the proposing bot's id.
    pub created_by: String,
    /// A bot's proposal, waiting for the person; otherwise created approved by `created_by`.
    pub draft: bool,
    pub checks: Vec<NewGoalCheck>,
}

#[derive(Debug)]
pub enum Created {
    Goal(Box<Goal>),
    /// The space already has [`MAX_DRAFTS`] drafts.
    TooManyDrafts,
}

/// What a person's edit changes, decided by the caller with the goal as it is inside the write
/// lock (see [`Database::edit_goal`]).
#[derive(Debug, Default)]
pub struct Edit {
    /// Replaces the checks; a check with the same kind and spec as one before keeps its id, so
    /// its observations and streak stay.
    pub checks: Option<Vec<NewGoalCheck>>,
    /// Timeline events to add: `(kind, detail)`.
    pub events: Vec<(&'static str, Value)>,
}

/// What a finished run writes (see [`Database::finish_goal_run`]).
#[derive(Debug, Clone, Default)]
pub struct RunUpdate {
    pub status: String,
    pub status_reason: String,
    pub fail_streak: i64,
    pub error_runs: i64,
    pub next_check_at: i64,
    pub events: Vec<(&'static str, Value)>,
}

async fn goal_in(conn: &mut SqliteConnection, id: &str) -> Result<Option<Goal>, sqlx::Error> {
    sqlx::query_as::<_, Goal>(&format!("{GOAL_SELECT} WHERE id = ?")).bind(id).fetch_optional(conn).await
}

async fn event_in(conn: &mut SqliteConnection, goal_id: &str, kind: &str, actor: &str, detail: &Value, now: i64) -> Result<(), sqlx::Error> {
    sqlx::query("INSERT INTO goal_events (goal_id, kind, actor, detail, created_at) VALUES (?, ?, ?, ?, ?)")
        .bind(goal_id)
        .bind(kind)
        .bind(actor)
        .bind(detail.to_string())
        .bind(now)
        .execute(conn)
        .await?;
    Ok(())
}

/// Writes `checks` as the goal's checks. One with the same kind, spec and predicate as an old
/// check keeps that check's id (so its observations still count; a new label, severity or
/// freshness doesn't change what they say); the old ones not kept go with their observations.
async fn put_checks(conn: &mut SqliteConnection, goal_id: &str, checks: &[NewGoalCheck]) -> Result<(), sqlx::Error> {
    let old: Vec<GoalCheck> = sqlx::query_as("SELECT id, goal_id, position, label, kind, spec, metric, operator, target, severity, freshness_minutes FROM goal_checks WHERE goal_id = ?")
        .bind(goal_id)
        .fetch_all(&mut *conn)
        .await?;
    let mut kept: Vec<String> = Vec::new();
    let mut rows = Vec::new();
    for (i, c) in checks.iter().enumerate() {
        let spec = c.spec.to_string();
        let reuse = old.iter().find(|o| {
            o.kind == c.kind && o.spec_value() == c.spec && o.metric == c.metric && o.operator == c.operator && o.target == c.target && !kept.contains(&o.id)
        });
        let id = match reuse {
            Some(o) => o.id.clone(),
            None => Uuid::new_v4().to_string(),
        };
        kept.push(id.clone());
        rows.push((id, i as i64, c, spec));
    }
    for o in old.iter().filter(|o| !kept.contains(&o.id)) {
        sqlx::query("DELETE FROM goal_observations WHERE goal_id = ? AND check_id = ?").bind(goal_id).bind(&o.id).execute(&mut *conn).await?;
    }
    sqlx::query("DELETE FROM goal_checks WHERE goal_id = ?").bind(goal_id).execute(&mut *conn).await?;
    for (id, position, c, spec) in rows {
        sqlx::query(
            "INSERT INTO goal_checks (id, goal_id, position, label, kind, spec, metric, operator, target, severity, freshness_minutes)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(id)
        .bind(goal_id)
        .bind(position)
        .bind(&c.label)
        .bind(&c.kind)
        .bind(spec)
        .bind(&c.metric)
        .bind(&c.operator)
        .bind(&c.target)
        .bind(&c.severity)
        .bind(c.freshness_minutes)
        .execute(&mut *conn)
        .await?;
    }
    Ok(())
}

impl Database {
    pub(super) async fn migrate_goals(&self) -> Result<(), DbError> {
        for stmt in TABLES {
            sqlx::query(stmt).execute(&self.pool).await?;
        }
        // A database made by the first goals build (never released) lacks this column
        let _ = sqlx::query("ALTER TABLE goals ADD COLUMN unknown_since INTEGER").execute(&self.pool).await;
        Ok(())
    }

    /// Creates a goal with its checks and a `created` event. A draft is refused when the space
    /// already holds [`MAX_DRAFTS`]: counted and inserted under one write lock, so two bots
    /// proposing at once can't pass the cap.
    pub async fn create_goal(&self, g: &NewGoal, now: i64) -> Result<Created, DbError> {
        let mut tx = self.begin_write().await?;
        if g.draft {
            let (n,): (i64,) = sqlx::query_as("SELECT COUNT(*) FROM goals WHERE space_id = ? AND status = 'draft'")
                .bind(&g.space_id)
                .fetch_one(&mut *tx)
                .await?;
            if n >= MAX_DRAFTS {
                tx.rollback().await?;
                return Ok(Created::TooManyDrafts);
            }
        }
        let id = Uuid::new_v4().to_string();
        let (status, approved_by, approved_at, next) = if g.draft {
            ("draft", None, None, None)
        } else {
            ("checking", Some(g.created_by.as_str()), Some(now), Some(now))
        };
        sqlx::query(
            "INSERT INTO goals (id, space_id, title, description, kwant_id, folder, status, mode, interval_minutes,
             confirm_failures, budget_tokens, max_fixes_per_day, priority, created_by, approved_by, approved_at,
             next_check_at, created_at, updated_at) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(&id)
        .bind(&g.space_id)
        .bind(&g.title)
        .bind(&g.description)
        .bind(&g.kwant_id)
        .bind(&g.folder)
        .bind(status)
        .bind(&g.mode)
        .bind(g.interval_minutes)
        .bind(g.confirm_failures)
        .bind(g.budget_tokens)
        .bind(g.max_fixes_per_day)
        .bind(g.priority)
        .bind(&g.created_by)
        .bind(approved_by)
        .bind(approved_at)
        .bind(next)
        .bind(now)
        .bind(now)
        .execute(&mut *tx)
        .await?;
        put_checks(&mut tx, &id, &g.checks).await?;
        event_in(&mut tx, &id, "created", &g.created_by, &json!({ "draft": g.draft }), now).await?;
        let goal = goal_in(&mut tx, &id).await?;
        tx.commit().await?;
        goal.map(|g| Created::Goal(Box::new(g))).ok_or_else(|| DbError::Migration("the new goal wasn't stored".into()))
    }

    pub async fn get_goal(&self, id: &str) -> Result<Option<Goal>, DbError> {
        let mut conn = self.pool.acquire().await?;
        Ok(goal_in(&mut conn, id).await?)
    }

    /// The space's goals, drafts first, then oldest first.
    pub async fn list_goals(&self, space_id: &str) -> Result<Vec<Goal>, DbError> {
        Ok(sqlx::query_as::<_, Goal>(&format!("{GOAL_SELECT} WHERE space_id = ? ORDER BY status = 'draft' DESC, created_at, id"))
            .bind(space_id)
            .fetch_all(&self.pool)
            .await?)
    }

    pub async fn count_draft_goals(&self, space_id: &str) -> Result<i64, DbError> {
        let (n,): (i64,) = sqlx::query_as("SELECT COUNT(*) FROM goals WHERE space_id = ? AND status = 'draft'").bind(space_id).fetch_one(&self.pool).await?;
        Ok(n)
    }

    pub async fn goal_checks(&self, goal_id: &str) -> Result<Vec<GoalCheck>, DbError> {
        Ok(sqlx::query_as("SELECT id, goal_id, position, label, kind, spec, metric, operator, target, severity, freshness_minutes FROM goal_checks WHERE goal_id = ? ORDER BY position")
            .bind(goal_id)
            .fetch_all(&self.pool)
            .await?)
    }

    /// Up to `per_check` newest observations of each of the goal's checks, newest first.
    pub async fn recent_observations(&self, goal_id: &str, per_check: i64) -> Result<Vec<Observation>, DbError> {
        Ok(sqlx::query_as(
            "SELECT id, goal_id, check_id, status, verdict, observed_value, excerpt, observed_at, expires_at FROM (
                SELECT o.*, ROW_NUMBER() OVER (PARTITION BY o.check_id ORDER BY o.id DESC) AS n
                FROM goal_observations o WHERE o.goal_id = ?
             ) WHERE n <= ? ORDER BY id DESC",
        )
        .bind(goal_id)
        .bind(per_check)
        .fetch_all(&self.pool)
        .await?)
    }

    /// The goal's newest events, newest first.
    pub async fn goal_events(&self, goal_id: &str, limit: i64) -> Result<Vec<GoalEvent>, DbError> {
        Ok(sqlx::query_as("SELECT id, goal_id, kind, actor, detail, created_at FROM goal_events WHERE goal_id = ? ORDER BY id DESC LIMIT ?")
            .bind(goal_id)
            .bind(limit)
            .fetch_all(&self.pool)
            .await?)
    }

    /// The newest of the goal's `met` / `not_met` events: which side of an outage it is on.
    pub async fn last_goal_mark(&self, goal_id: &str) -> Result<Option<String>, DbError> {
        let row: Option<(String,)> = sqlx::query_as("SELECT kind FROM goal_events WHERE goal_id = ? AND kind IN ('met', 'not_met') ORDER BY id DESC LIMIT 1")
            .bind(goal_id)
            .fetch_optional(&self.pool)
            .await?;
        Ok(row.map(|r| r.0))
    }

    pub async fn add_goal_event(&self, goal_id: &str, kind: &str, actor: &str, detail: &Value) -> Result<(), DbError> {
        let mut conn = self.pool.acquire().await?;
        event_in(&mut conn, goal_id, kind, actor, detail, super::missions::now_ms()).await?;
        Ok(())
    }

    /// Approves a draft of the space: it is checked from now on. None if it isn't a draft there.
    pub async fn approve_goal(&self, id: &str, space_id: &str, by: &str, now: i64) -> Result<Option<Goal>, DbError> {
        let mut tx = self.begin_write().await?;
        let r = sqlx::query(
            "UPDATE goals SET status = 'checking', status_reason = '', approved_by = ?, approved_at = ?, next_check_at = ?,
             version = version + 1, updated_at = ? WHERE id = ? AND space_id = ? AND status = 'draft'",
        )
        .bind(by)
        .bind(now)
        .bind(now)
        .bind(now)
        .bind(id)
        .bind(space_id)
        .execute(&mut *tx)
        .await?;
        if r.rows_affected() != 1 {
            tx.rollback().await?;
            return Ok(None);
        }
        event_in(&mut tx, id, "approved", by, &json!({}), now).await?;
        let goal = goal_in(&mut tx, id).await?;
        tx.commit().await?;
        Ok(goal)
    }

    /// A person's edit: `decide` sees the goal of the space as it is under the write lock, changes
    /// it in place (or refuses with a message) and says what else to write. The version goes up,
    /// so a run that started before the edit doesn't overwrite it. `Ok(None)`: no such goal.
    pub async fn edit_goal<F>(&self, id: &str, space_id: &str, actor: &str, now: i64, decide: F) -> Result<Option<Result<Goal, String>>, DbError>
    where
        F: FnOnce(&mut Goal) -> Result<Edit, String>,
    {
        let mut tx = self.begin_write().await?;
        let Some(mut g) = goal_in(&mut tx, id).await?.filter(|g| g.space_id == space_id) else {
            tx.rollback().await?;
            return Ok(None);
        };
        let edit = match decide(&mut g) {
            Ok(e) => e,
            Err(why) => {
                tx.rollback().await?;
                return Ok(Some(Err(why)));
            }
        };
        sqlx::query(
            "UPDATE goals SET title = ?, description = ?, kwant_id = ?, folder = ?, status = ?, status_reason = ?, mode = ?,
             interval_minutes = ?, confirm_failures = ?, budget_tokens = ?, max_fixes_per_day = ?, priority = ?,
             paused_at = ?, next_check_at = ?, fail_streak = ?, error_runs = ?,
             unknown_since = CASE WHEN ? = 'unknown' THEN COALESCE(unknown_since, ?) ELSE NULL END,
             version = version + 1, updated_at = ? WHERE id = ?",
        )
        .bind(&g.title)
        .bind(&g.description)
        .bind(&g.kwant_id)
        .bind(&g.folder)
        .bind(&g.status)
        .bind(&g.status_reason)
        .bind(&g.mode)
        .bind(g.interval_minutes)
        .bind(g.confirm_failures)
        .bind(g.budget_tokens)
        .bind(g.max_fixes_per_day)
        .bind(g.priority)
        .bind(g.paused_at)
        .bind(g.next_check_at)
        .bind(g.fail_streak)
        .bind(g.error_runs)
        .bind(&g.status)
        .bind(now)
        .bind(now)
        .bind(id)
        .execute(&mut *tx)
        .await?;
        if let Some(checks) = &edit.checks {
            put_checks(&mut tx, id, checks).await?;
        }
        for (kind, detail) in &edit.events {
            event_in(&mut tx, id, kind, actor, detail, now).await?;
        }
        let goal = goal_in(&mut tx, id).await?;
        tx.commit().await?;
        Ok(goal.map(Ok))
    }

    /// Deletes a goal of the space with its checks, observations, events and fix-task links (the
    /// tasks stay on the board). False if there was none.
    pub async fn delete_goal(&self, id: &str, space_id: &str) -> Result<bool, DbError> {
        let mut tx = self.begin_write().await?;
        let r = sqlx::query("DELETE FROM goals WHERE id = ? AND space_id = ?").bind(id).bind(space_id).execute(&mut *tx).await?;
        if r.rows_affected() != 1 {
            tx.rollback().await?;
            return Ok(false);
        }
        for table in ["goal_checks", "goal_observations", "goal_events", "goal_fix_tasks"] {
            sqlx::query(&format!("DELETE FROM {table} WHERE goal_id = ?")).bind(id).execute(&mut *tx).await?;
        }
        tx.commit().await?;
        Ok(true)
    }

    /// Goals whose check is due at `now` and not running: approved, not paused, no live lease.
    pub async fn due_goal_ids(&self, now: i64, limit: i64) -> Result<Vec<String>, DbError> {
        let rows: Vec<(String,)> = sqlx::query_as(
            "SELECT id FROM goals WHERE next_check_at IS NOT NULL AND next_check_at <= ? AND status NOT IN ('draft', 'paused')
             AND (checking_since IS NULL OR checking_since < ?) ORDER BY next_check_at LIMIT ?",
        )
        .bind(now)
        .bind(now - LEASE_STALE_MS)
        .bind(limit)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows.into_iter().map(|r| r.0).collect())
    }

    /// Takes the goal's run lease (`checking_since = now`) if no live run holds it. `due_only`
    /// (the loop) also needs it approved, not paused and due; otherwise (Check now) any goal but a
    /// draft. The lease is the token the run renews and finishes with.
    pub async fn lease_goal(&self, id: &str, now: i64, due_only: bool) -> Result<Option<Goal>, DbError> {
        let due = if due_only { " AND status != 'paused' AND next_check_at IS NOT NULL AND next_check_at <= ?" } else { "" };
        let sql = format!(
            "UPDATE goals SET checking_since = ? WHERE id = ? AND status != 'draft' AND (checking_since IS NULL OR checking_since < ?){due}"
        );
        let mut q = sqlx::query(&sql).bind(now).bind(id).bind(now - LEASE_STALE_MS);
        if due_only {
            q = q.bind(now);
        }
        let mut tx = self.begin_write().await?;
        if q.execute(&mut *tx).await?.rows_affected() != 1 {
            tx.rollback().await?;
            return Ok(None);
        }
        let goal = goal_in(&mut tx, id).await?;
        tx.commit().await?;
        Ok(goal)
    }

    /// Moves the lease from `old` to `new` (a run still going); false if the run lost it (the goal
    /// was deleted, or the lease went stale and another run took it).
    pub async fn renew_goal_lease(&self, id: &str, old: i64, new: i64) -> Result<bool, DbError> {
        let r = sqlx::query("UPDATE goals SET checking_since = ? WHERE id = ? AND checking_since = ?").bind(new).bind(id).bind(old).execute(&self.pool).await?;
        Ok(r.rows_affected() == 1)
    }

    /// Whether the run holding `lease` still holds it.
    pub async fn holds_goal_lease(&self, id: &str, lease: i64) -> Result<bool, DbError> {
        let row: Option<(i64,)> = sqlx::query_as("SELECT 1 FROM goals WHERE id = ? AND checking_since = ?").bind(id).bind(lease).fetch_optional(&self.pool).await?;
        Ok(row.is_some())
    }

    /// Stores a run's observations while the run holds `lease`, then keeps the newest
    /// [`KEEP_OBSERVATIONS`]. False (nothing stored) if the run lost its lease.
    pub async fn record_observations(&self, goal_id: &str, lease: i64, obs: &[NewObservation], now: i64) -> Result<bool, DbError> {
        let mut tx = self.begin_write().await?;
        let held: Option<(i64,)> = sqlx::query_as("SELECT 1 FROM goals WHERE id = ? AND checking_since = ?").bind(goal_id).bind(lease).fetch_optional(&mut *tx).await?;
        if held.is_none() {
            tx.rollback().await?;
            return Ok(false);
        }
        for o in obs {
            sqlx::query(
                "INSERT INTO goal_observations (goal_id, check_id, status, verdict, observed_value, excerpt, observed_at, expires_at)
                 VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
            )
            .bind(goal_id)
            .bind(&o.check_id)
            .bind(&o.status)
            .bind(&o.verdict)
            .bind(&o.observed_value)
            .bind(&o.excerpt)
            .bind(now)
            .bind(o.expires_at)
            .execute(&mut *tx)
            .await?;
        }
        sqlx::query(
            "DELETE FROM goal_observations WHERE goal_id = ? AND id NOT IN
             (SELECT id FROM goal_observations WHERE goal_id = ? ORDER BY id DESC LIMIT ?)",
        )
        .bind(goal_id)
        .bind(goal_id)
        .bind(KEEP_OBSERVATIONS)
        .execute(&mut *tx)
        .await?;
        sqlx::query("UPDATE goals SET last_checked_at = ? WHERE id = ?").bind(now).bind(goal_id).execute(&mut *tx).await?;
        tx.commit().await?;
        Ok(true)
    }

    /// Ends a run: writes its outcome and frees the lease. If the person edited, paused or resumed
    /// the goal since the run started (`version` moved on), only the lease is freed, so their
    /// change stands (an edit already asked for a check now). Returns the goal as it is now; None
    /// if the run lost its lease.
    pub async fn finish_goal_run(&self, id: &str, lease: i64, version: i64, u: &RunUpdate, now: i64) -> Result<Option<Goal>, DbError> {
        let mut tx = self.begin_write().await?;
        let r = sqlx::query(
            "UPDATE goals SET status = ?, status_reason = ?, fail_streak = ?, error_runs = ?, next_check_at = ?,
             unknown_since = CASE WHEN ? = 'unknown' THEN COALESCE(unknown_since, ?) ELSE NULL END,
             checking_since = NULL, updated_at = ? WHERE id = ? AND checking_since = ? AND version = ?",
        )
        .bind(&u.status)
        .bind(&u.status_reason)
        .bind(u.fail_streak)
        .bind(u.error_runs)
        .bind(u.next_check_at)
        .bind(&u.status)
        .bind(now)
        .bind(now)
        .bind(id)
        .bind(lease)
        .bind(version)
        .execute(&mut *tx)
        .await?;
        if r.rows_affected() == 1 {
            for (kind, detail) in &u.events {
                event_in(&mut tx, id, kind, "kwant", detail, now).await?;
            }
            sqlx::query(
                "DELETE FROM goal_events WHERE goal_id = ? AND id NOT IN
                 (SELECT id FROM goal_events WHERE goal_id = ? ORDER BY id DESC LIMIT ?)",
            )
            .bind(id)
            .bind(id)
            .bind(KEEP_EVENTS)
            .execute(&mut *tx)
            .await?;
        } else {
            let r = sqlx::query("UPDATE goals SET checking_since = NULL WHERE id = ? AND checking_since = ?").bind(id).bind(lease).execute(&mut *tx).await?;
            if r.rows_affected() != 1 {
                tx.rollback().await?;
                return Ok(None);
            }
        }
        let goal = goal_in(&mut tx, id).await?;
        tx.commit().await?;
        Ok(goal)
    }

    /// Frees a run's lease without an outcome (the run found nothing to do).
    pub async fn release_goal_lease(&self, id: &str, lease: i64, next_check_at: Option<i64>) -> Result<(), DbError> {
        sqlx::query("UPDATE goals SET checking_since = NULL, next_check_at = COALESCE(?, next_check_at) WHERE id = ? AND checking_since = ?")
            .bind(next_check_at)
            .bind(id)
            .bind(lease)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    /// Sets the goal's status from outside a run (a fix task moved), unless it is a draft or
    /// paused. `only_from`: change it only from one of these. Returns the goal if it changed.
    pub async fn set_goal_status(&self, id: &str, status: &str, reason: &str, only_from: &[&str], event: Option<(&str, Value)>) -> Result<Option<Goal>, DbError> {
        let now = super::missions::now_ms();
        let mut tx = self.begin_write().await?;
        let Some(g) = goal_in(&mut tx, id).await? else {
            tx.rollback().await?;
            return Ok(None);
        };
        if matches!(g.status.as_str(), "draft" | "paused") || (!only_from.is_empty() && !only_from.contains(&g.status.as_str())) {
            tx.rollback().await?;
            return Ok(None);
        }
        sqlx::query(
            "UPDATE goals SET status = ?, status_reason = ?,
             unknown_since = CASE WHEN ? = 'unknown' THEN COALESCE(unknown_since, ?) ELSE NULL END, updated_at = ? WHERE id = ?",
        )
            .bind(status)
            .bind(reason)
            .bind(status)
            .bind(now)
            .bind(now)
            .bind(id)
            .execute(&mut *tx)
            .await?;
        if let Some((kind, detail)) = event {
            event_in(&mut tx, id, kind, "kwant", &detail, now).await?;
        }
        let goal = goal_in(&mut tx, id).await?;
        tx.commit().await?;
        Ok(goal)
    }

    /// Asks for a check soon: `next_check_at = at` unless one is already due earlier.
    pub async fn check_goal_soon(&self, id: &str, at: i64) -> Result<(), DbError> {
        sqlx::query("UPDATE goals SET next_check_at = MIN(COALESCE(next_check_at, ?), ?) WHERE id = ? AND status NOT IN ('draft', 'paused')")
            .bind(at)
            .bind(at)
            .bind(id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    /// Links a new fix task to the goal and logs `fix_opened`, while the run that opened it still
    /// holds `lease`. False (nothing linked) if it doesn't: the caller cancels the task.
    pub async fn link_fix_task(&self, goal_id: &str, lease: i64, task_id: &str, detail: &Value, now: i64) -> Result<bool, DbError> {
        let mut tx = self.begin_write().await?;
        let held: Option<(i64,)> = sqlx::query_as("SELECT 1 FROM goals WHERE id = ? AND checking_since = ?").bind(goal_id).bind(lease).fetch_optional(&mut *tx).await?;
        if held.is_none() {
            tx.rollback().await?;
            return Ok(false);
        }
        sqlx::query("INSERT INTO goal_fix_tasks (goal_id, task_id, created_at) VALUES (?, ?, ?)").bind(goal_id).bind(task_id).bind(now).execute(&mut *tx).await?;
        event_in(&mut tx, goal_id, "fix_opened", "kwant", detail, now).await?;
        tx.commit().await?;
        Ok(true)
    }

    /// The goal's fix tasks, newest first, with each task's state and spend now.
    pub async fn goal_fix_links(&self, goal_id: &str) -> Result<Vec<FixLink>, DbError> {
        Ok(sqlx::query_as(
            "SELECT f.task_id, f.created_at, t.state, t.spent_tokens FROM goal_fix_tasks f LEFT JOIN tasks t ON t.id = f.task_id
             WHERE f.goal_id = ? ORDER BY f.created_at DESC, f.rowid DESC",
        )
        .bind(goal_id)
        .fetch_all(&self.pool)
        .await?)
    }

    /// Fix tasks the goal opened since `since` (the daily cap counts a rolling 24 hours).
    pub async fn fixes_since(&self, goal_id: &str, since: i64) -> Result<i64, DbError> {
        let (n,): (i64,) = sqlx::query_as("SELECT COUNT(*) FROM goal_fix_tasks WHERE goal_id = ? AND created_at >= ?").bind(goal_id).bind(since).fetch_one(&self.pool).await?;
        Ok(n)
    }

    /// The goal a task was opened to fix, if any.
    pub async fn goal_of_task(&self, task_id: &str) -> Result<Option<String>, DbError> {
        let row: Option<(String,)> = sqlx::query_as("SELECT goal_id FROM goal_fix_tasks WHERE task_id = ?").bind(task_id).fetch_optional(&self.pool).await?;
        Ok(row.map(|r| r.0))
    }

    /// Goals with a fix task that is ready, not held and not running: the fix runner's work.
    pub async fn goals_with_ready_fix(&self) -> Result<Vec<(String, String)>, DbError> {
        Ok(sqlx::query_as(
            "SELECT f.goal_id, f.task_id FROM goal_fix_tasks f JOIN tasks t ON t.id = f.task_id JOIN goals g ON g.id = f.goal_id
             WHERE t.state = 'ready' AND t.held_at IS NULL AND g.status != 'paused' ORDER BY f.created_at",
        )
        .fetch_all(&self.pool)
        .await?)
    }

    /// Deletes drafts created before `before` (nobody approved them: they expire silently after
    /// 7 days, plan B.0 rule 4), with their rows. Returns `(id, space_id)` of each.
    pub async fn expire_drafts(&self, before: i64) -> Result<Vec<(String, String)>, DbError> {
        let mut tx = self.begin_write().await?;
        let gone: Vec<(String, String)> = sqlx::query_as("SELECT id, space_id FROM goals WHERE status = 'draft' AND created_at < ?").bind(before).fetch_all(&mut *tx).await?;
        for (id, _) in &gone {
            sqlx::query("DELETE FROM goals WHERE id = ? AND status = 'draft'").bind(id).execute(&mut *tx).await?;
            for table in ["goal_checks", "goal_observations", "goal_events", "goal_fix_tasks"] {
                sqlx::query(&format!("DELETE FROM {table} WHERE goal_id = ?")).bind(id).execute(&mut *tx).await?;
            }
        }
        tx.commit().await?;
        Ok(gone)
    }

    /// Goals `unknown` since `before` or longer that haven't had the "can't check" notice (sent, or
    /// held back by the cap) for this stretch of unknown.
    pub async fn goals_unknown_since(&self, before: i64) -> Result<Vec<Goal>, DbError> {
        Ok(sqlx::query_as::<_, Goal>(&format!(
            "{GOAL_SELECT} WHERE status = 'unknown' AND unknown_since IS NOT NULL AND unknown_since <= ? AND NOT EXISTS (
                SELECT 1 FROM goal_events e WHERE e.goal_id = goals.id AND e.kind IN ('notified', 'notice_capped')
                AND e.created_at >= goals.unknown_since AND json_extract(e.detail, '$.kind') = 'cant_check')"
        ))
        .bind(before)
        .fetch_all(&self.pool)
        .await?)
    }

    /// Notices sent about the goal since `since` (the cap counts a rolling 24 hours).
    pub async fn goal_notices_since(&self, goal_id: &str, since: i64) -> Result<i64, DbError> {
        let (n,): (i64,) = sqlx::query_as("SELECT COUNT(*) FROM goal_events WHERE goal_id = ? AND kind = 'notified' AND created_at >= ?")
            .bind(goal_id)
            .bind(since)
            .fetch_one(&self.pool)
            .await?;
        Ok(n)
    }

    /// Whether the person was told the goal broke, or that kwant can't check it, since it was last
    /// met (so a recovery is worth a notice).
    pub async fn told_broken(&self, goal_id: &str) -> Result<bool, DbError> {
        let row: Option<(i64,)> = sqlx::query_as(
            "SELECT 1 FROM goal_events WHERE goal_id = ? AND kind = 'notified' AND json_extract(detail, '$.kind') IN ('broken', 'cant_check')
             AND id > COALESCE((SELECT MAX(id) FROM goal_events WHERE goal_id = ? AND kind = 'met'), 0) LIMIT 1",
        )
        .bind(goal_id)
        .bind(goal_id)
        .fetch_optional(&self.pool)
        .await?;
        Ok(row.is_some())
    }

    /// Met goals, not running, with a required check whose newest observation has expired (at or
    /// before `now`) or that has none: they can't be called met any more.
    pub async fn stale_met_goals(&self, now: i64) -> Result<Vec<String>, DbError> {
        let rows: Vec<(String,)> = sqlx::query_as(
            "SELECT g.id FROM goals g WHERE g.status = 'met' AND g.checking_since IS NULL AND EXISTS (
                SELECT 1 FROM goal_checks c WHERE c.goal_id = g.id AND c.severity != 'advisory' AND COALESCE((
                    SELECT o.expires_at FROM goal_observations o WHERE o.goal_id = g.id AND o.check_id = c.id ORDER BY o.id DESC LIMIT 1
                ), 0) <= ?)",
        )
        .bind(now)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows.into_iter().map(|r| r.0).collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn db() -> (Database, tempfile::TempDir) {
        let tmp = tempfile::tempdir().unwrap();
        (Database::init(tmp.path()).await.unwrap(), tmp)
    }

    fn new_goal(space: &str, draft: bool) -> NewGoal {
        NewGoal {
            space_id: space.into(),
            title: "Site stays up".into(),
            description: "The home page answers 200".into(),
            kwant_id: "bot-1".into(),
            folder: "/tmp/x".into(),
            mode: "suggest".into(),
            interval_minutes: 60,
            confirm_failures: 2,
            budget_tokens: 0,
            max_fixes_per_day: 3,
            priority: 2,
            created_by: if draft { "bot-1".into() } else { "person".into() },
            draft,
            checks: vec![check("home", "http", json!({"url": "http://127.0.0.1:9/"}))],
        }
    }

    fn check(label: &str, kind: &str, spec: Value) -> NewGoalCheck {
        let (metric, operator, target) = if kind == "http" { (Some("status".into()), Some("eq".into()), Some("200".into())) } else { (None, None, None) };
        NewGoalCheck { label: label.into(), kind: kind.into(), spec, metric, operator, target, severity: "required".into(), freshness_minutes: 120 }
    }

    fn obs(check_id: &str, status: &str, verdict: &str, excerpt: &str) -> NewObservation {
        NewObservation {
            check_id: check_id.into(),
            status: status.into(),
            verdict: verdict.into(),
            observed_value: None,
            excerpt: excerpt.into(),
            expires_at: i64::MAX,
        }
    }

    fn created(c: Created) -> Goal {
        match c {
            Created::Goal(g) => *g,
            Created::TooManyDrafts => panic!("refused"),
        }
    }

    #[tokio::test]
    async fn drafts_are_capped_per_space_and_a_dismissed_one_frees_its_slot() {
        let (db, _dir) = db().await;
        let mut ids = Vec::new();
        for _ in 0..MAX_DRAFTS {
            ids.push(created(db.create_goal(&new_goal("s1", true), 1).await.unwrap()).id);
        }
        assert!(matches!(db.create_goal(&new_goal("s1", true), 2).await.unwrap(), Created::TooManyDrafts));
        // Another space has its own three; an approved goal doesn't count
        assert!(matches!(db.create_goal(&new_goal("s2", true), 2).await.unwrap(), Created::Goal(_)));
        assert!(matches!(db.create_goal(&new_goal("s1", false), 2).await.unwrap(), Created::Goal(_)));
        assert!(db.approve_goal(&ids[0], "s1", "person", 3).await.unwrap().is_some());
        assert!(matches!(db.create_goal(&new_goal("s1", true), 4).await.unwrap(), Created::Goal(_)));
        assert!(db.delete_goal(&ids[1], "s1").await.unwrap());
        assert!(matches!(db.create_goal(&new_goal("s1", true), 5).await.unwrap(), Created::Goal(_)));
        assert_eq!(db.count_draft_goals("s1").await.unwrap(), MAX_DRAFTS);
    }

    #[tokio::test]
    async fn concurrent_proposals_never_pass_the_cap() {
        let (db, _dir) = db().await;
        let mut handles = Vec::new();
        for _ in 0..8 {
            let db = db.clone();
            handles.push(tokio::spawn(async move { db.create_goal(&new_goal("s1", true), 1).await.unwrap() }));
        }
        let mut made = 0;
        for h in handles {
            if matches!(h.await.unwrap(), Created::Goal(_)) {
                made += 1;
            }
        }
        assert_eq!(made, MAX_DRAFTS);
        assert_eq!(db.count_draft_goals("s1").await.unwrap(), MAX_DRAFTS);
    }

    #[tokio::test]
    async fn a_draft_is_never_due_or_leased_and_approval_is_once() {
        let (db, _dir) = db().await;
        let g = created(db.create_goal(&new_goal("s1", true), 1).await.unwrap());
        assert_eq!(g.status, "draft");
        assert!(db.due_goal_ids(i64::MAX / 2, 10).await.unwrap().is_empty());
        assert!(db.lease_goal(&g.id, 10, false).await.unwrap().is_none());
        assert!(db.approve_goal(&g.id, "other-space", "person", 5).await.unwrap().is_none());
        let a = db.approve_goal(&g.id, "s1", "person", 5).await.unwrap().unwrap();
        assert_eq!((a.status.as_str(), a.approved_by.as_deref(), a.next_check_at), ("checking", Some("person"), Some(5)));
        assert!(db.approve_goal(&g.id, "s1", "person", 6).await.unwrap().is_none());
        assert_eq!(db.due_goal_ids(5, 10).await.unwrap(), vec![g.id.clone()]);
        let kinds: Vec<String> = db.goal_events(&g.id, 50).await.unwrap().into_iter().map(|e| e.kind).collect();
        assert_eq!(kinds, ["approved", "created"]);
    }

    #[tokio::test]
    async fn one_lease_at_a_time_until_it_goes_stale() {
        let (db, _dir) = db().await;
        let g = created(db.create_goal(&new_goal("s1", false), 1).await.unwrap());
        let t0 = 1_000;
        assert!(db.lease_goal(&g.id, t0, true).await.unwrap().is_some());
        assert!(db.lease_goal(&g.id, t0 + 1, false).await.unwrap().is_none());
        assert!(db.lease_goal(&g.id, t0 + LEASE_STALE_MS - 1, true).await.unwrap().is_none());
        assert!(db.due_goal_ids(t0 + 5, 10).await.unwrap().is_empty());
        // Renewed, it stays live; the old token is dead
        assert!(db.renew_goal_lease(&g.id, t0, t0 + 10).await.unwrap());
        assert!(!db.renew_goal_lease(&g.id, t0, t0 + 20).await.unwrap());
        // Stale after 15 minutes: another run takes over, and the first can't store anything
        let t1 = t0 + 10 + LEASE_STALE_MS + 1;
        assert!(db.lease_goal(&g.id, t1, true).await.unwrap().is_some());
        let o = vec![obs("c", "passed", "met", "")];
        assert!(!db.record_observations(&g.id, t0 + 10, &o, t1).await.unwrap());
        assert!(db.record_observations(&g.id, t1, &o, t1).await.unwrap());
    }

    #[tokio::test]
    async fn observations_keep_the_newest_200_per_goal() {
        let (db, _dir) = db().await;
        let g = created(db.create_goal(&new_goal("s1", false), 1).await.unwrap());
        let check = db.goal_checks(&g.id).await.unwrap()[0].id.clone();
        assert!(db.lease_goal(&g.id, 10, false).await.unwrap().is_some());
        for i in 0..210 {
            let o = vec![obs(&check, "passed", "met", &format!("run {i}"))];
            assert!(db.record_observations(&g.id, 10, &o, 100 + i).await.unwrap());
        }
        let (n,): (i64,) = sqlx::query_as("SELECT COUNT(*) FROM goal_observations WHERE goal_id = ?").bind(&g.id).fetch_one(&db.pool).await.unwrap();
        assert_eq!(n, KEEP_OBSERVATIONS);
        let newest = db.recent_observations(&g.id, 3).await.unwrap();
        assert_eq!(newest.iter().map(|o| o.excerpt.as_str()).collect::<Vec<_>>(), ["run 209", "run 208", "run 207"]);
    }

    #[tokio::test]
    async fn a_run_that_overlaps_an_edit_frees_the_lease_but_keeps_the_edit() {
        let (db, _dir) = db().await;
        let g = created(db.create_goal(&new_goal("s1", false), 1).await.unwrap());
        let leased = db.lease_goal(&g.id, 10, false).await.unwrap().unwrap();
        let paused = db
            .edit_goal(&g.id, "s1", "person", 11, |g| {
                g.status = "paused".into();
                g.paused_at = Some(11);
                Ok(Edit { events: vec![("paused", json!({}))], ..Default::default() })
            })
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        assert_eq!(paused.version, leased.version + 1);
        let u = RunUpdate { status: "met".into(), next_check_at: 999, events: vec![("met", json!({}))], ..Default::default() };
        let after = db.finish_goal_run(&g.id, 10, leased.version, &u, 12).await.unwrap().unwrap();
        assert_eq!((after.status.as_str(), after.checking_since), ("paused", None));
        assert_eq!(db.last_goal_mark(&g.id).await.unwrap(), None);
    }

    #[tokio::test]
    async fn an_edit_keeps_unchanged_checks_and_their_observations() {
        let (db, _dir) = db().await;
        let g = created(db.create_goal(&new_goal("s1", false), 1).await.unwrap());
        let old = db.goal_checks(&g.id).await.unwrap();
        assert!(db.lease_goal(&g.id, 10, false).await.unwrap().is_some());
        let o = vec![obs(&old[0].id, "failed", "not_met", "")];
        assert!(db.record_observations(&g.id, 10, &o, 10).await.unwrap());
        let mut checks = new_goal("s1", false).checks;
        checks[0].label = "renamed".into();
        checks[0].severity = "advisory".into();
        checks.push(check("file", "file_exists", json!({"path": "ok"})));
        db.edit_goal(&g.id, "s1", "person", 11, |_| Ok(Edit { checks: Some(checks), ..Default::default() })).await.unwrap().unwrap().unwrap();
        let now = db.goal_checks(&g.id).await.unwrap();
        assert_eq!(now.len(), 2);
        assert_eq!((now[0].id.as_str(), now[0].label.as_str(), now[0].required()), (old[0].id.as_str(), "renamed", false));
        assert_eq!(db.recent_observations(&g.id, 5).await.unwrap().len(), 1);
        // A changed target is a new check: what it observed answered another question
        let mut changed = new_goal("s1", false).checks;
        changed[0].target = Some("204".into());
        db.edit_goal(&g.id, "s1", "person", 12, |_| Ok(Edit { checks: Some(changed), ..Default::default() })).await.unwrap().unwrap().unwrap();
        assert_ne!(db.goal_checks(&g.id).await.unwrap()[0].id, old[0].id);
        assert!(db.recent_observations(&g.id, 5).await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn deleting_a_goal_removes_its_rows_and_a_late_run_writes_nothing() {
        let (db, _dir) = db().await;
        let g = created(db.create_goal(&new_goal("s1", false), 1).await.unwrap());
        let check = db.goal_checks(&g.id).await.unwrap()[0].id.clone();
        assert!(db.lease_goal(&g.id, 10, false).await.unwrap().is_some());
        let o = vec![obs(&check, "failed", "not_met", "")];
        assert!(db.record_observations(&g.id, 10, &o, 10).await.unwrap());
        assert!(db.link_fix_task(&g.id, 10, "task-1", &json!({}), 10).await.unwrap());
        assert!(!db.delete_goal(&g.id, "other-space").await.unwrap());
        assert!(db.delete_goal(&g.id, "s1").await.unwrap());
        for table in ["goals", "goal_checks", "goal_observations", "goal_events", "goal_fix_tasks"] {
            let col = if table == "goals" { "id" } else { "goal_id" };
            let (n,): (i64,) = sqlx::query_as(&format!("SELECT COUNT(*) FROM {table} WHERE {col} = ?")).bind(&g.id).fetch_one(&db.pool).await.unwrap();
            assert_eq!(n, 0, "{table}");
        }
        assert!(!db.record_observations(&g.id, 10, &[], 11).await.unwrap());
        assert!(!db.link_fix_task(&g.id, 10, "task-2", &json!({}), 11).await.unwrap());
        let u = RunUpdate { status: "met".into(), ..Default::default() };
        assert!(db.finish_goal_run(&g.id, 10, g.version, &u, 12).await.unwrap().is_none());
        assert!(db.get_goal(&g.id).await.unwrap().is_none());
        assert_eq!(db.goal_of_task("task-1").await.unwrap(), None);
    }

    #[tokio::test]
    async fn the_migration_runs_twice() {
        let (db, _dir) = db().await;
        db.migrate_goals().await.unwrap();
        db.migrate_goals().await.unwrap();
    }
}
