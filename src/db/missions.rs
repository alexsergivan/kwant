//! Mission Control: missions, the one `tasks` table every kind of work lands in, their
//! dependencies, claims (leases), evidence, outcome checks and the append-only `task_events` log.
//! Contract: docs/mission-control.md. The rules that matter are enforced twice: in
//! [`crate::server::missions`] and by SQLite triggers here, so no code path (or a stray UPDATE) can
//! verify a task on its owner's word, start work without a live lease, make a task ready before its
//! dependencies are verified, or rewrite the event log.

use super::spaces::SpaceId;
use super::{Database, DbError};
use chrono::{DateTime, TimeZone, Utc};
use serde::Serialize;
use serde_json::Value;
use sqlx::FromRow;
use uuid::Uuid;

pub const TASK_STATES: [&str; 9] = ["backlog", "ready", "working", "needs_you", "review", "verified", "blocked", "failed", "cancelled"];
pub const MISSION_STATUSES: [&str; 4] = ["draft", "active", "done", "cancelled"];
pub const SOURCE_KINDS: [&str; 7] = ["mission", "person", "delegation", "routine", "zombie", "work_ahead", "chat"];
pub const CHECK_KINDS: [&str; 7] = ["command", "http", "file_exists", "file_contains", "file_sha256", "peer_review", "person"];
/// Checks kwant runs itself (the rest are recorded by a teammate or the person).
pub const KWANT_CHECK_KINDS: [&str; 5] = ["command", "http", "file_exists", "file_contains", "file_sha256"];
/// States a task can't leave.
pub const TERMINAL: [&str; 2] = ["verified", "cancelled"];

#[derive(Debug, Clone, Serialize, FromRow, PartialEq)]
pub struct Mission {
    pub id: String,
    pub space_id: String,
    pub title: String,
    pub goal: String,
    pub status: String,
    pub lead_kwant_id: String,
    pub channel_id: Option<String>,
    pub direct_kwant_id: Option<String>,
    pub thread_id: Option<String>,
    pub workspace_path: String,
    pub budget_tokens: i64,
    pub spent_tokens: i64,
    pub created_by: String,
    pub approved_at: Option<DateTime<Utc>>,
    pub approved_by: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub finished_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Serialize, FromRow, PartialEq)]
pub struct Task {
    pub id: String,
    pub space_id: String,
    pub mission_id: Option<String>,
    pub title: String,
    pub acceptance: String,
    pub state: String,
    pub priority: i64,
    pub owner_kwant_id: Option<String>,
    pub suggested_kwant_id: Option<String>,
    pub budget_tokens: i64,
    pub spent_tokens: i64,
    pub attempts: i64,
    pub max_attempts: i64,
    pub blocked_reason: Option<String>,
    pub source_kind: String,
    pub source_id: Option<String>,
    pub channel_id: Option<String>,
    pub direct_kwant_id: Option<String>,
    pub thread_id: Option<String>,
    pub created_by: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub finished_at: Option<DateTime<Utc>>,
}

/// A lease on a task. Times are unix milliseconds so the SQLite triggers can compare them.
#[derive(Debug, Clone, FromRow, PartialEq)]
pub struct Claim {
    pub id: String,
    pub task_id: String,
    pub kwant_id: String,
    pub turn_id: Option<String>,
    /// The chat whose turns spend on this task; `None` = any turn of the bot.
    pub context_id: Option<String>,
    /// Held by kwant for a hand-off or routine run, not taken by the bot itself.
    pub system: bool,
    pub claimed_at: i64,
    pub expires_at: i64,
    pub released_at: Option<i64>,
    pub release_reason: Option<String>,
}

impl Claim {
    pub fn claimed_at_utc(&self) -> DateTime<Utc> {
        ms_to_utc(self.claimed_at)
    }
    pub fn expires_at_utc(&self) -> DateTime<Utc> {
        ms_to_utc(self.expires_at)
    }
}

pub fn ms_to_utc(ms: i64) -> DateTime<Utc> {
    Utc.timestamp_millis_opt(ms).single().unwrap_or_else(Utc::now)
}

#[derive(Debug, Clone, Serialize, FromRow, PartialEq)]
pub struct Evidence {
    pub id: String,
    pub task_id: String,
    pub kind: String,
    pub label: String,
    pub uri: Option<String>,
    pub path: Option<String>,
    pub sha256: Option<String>,
    pub bytes: Option<i64>,
    pub excerpt: Option<String>,
    pub captured_by: String,
    pub run_id: Option<String>,
    pub captured_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, FromRow, PartialEq)]
pub struct OutcomeCheck {
    pub id: String,
    pub task_id: String,
    pub kind: String,
    /// JSON text; see [`OutcomeCheck::spec_value`].
    #[serde(skip)]
    pub spec: String,
    pub required: bool,
    pub approved: bool,
    pub status: String,
    pub output_excerpt: Option<String>,
    pub checked_by: Option<String>,
    pub requested_by: String,
    pub evidence_id: Option<String>,
    pub created_at: DateTime<Utc>,
    pub started_at: Option<DateTime<Utc>>,
    pub finished_at: Option<DateTime<Utc>>,
}

impl OutcomeCheck {
    pub fn spec_value(&self) -> Value {
        serde_json::from_str(&self.spec).unwrap_or(Value::Null)
    }
}

#[derive(Debug, Clone, Serialize, FromRow, PartialEq)]
pub struct TaskEvent {
    pub id: i64,
    pub task_id: String,
    pub mission_id: Option<String>,
    pub kind: String,
    pub actor: String,
    pub from_state: Option<String>,
    pub to_state: Option<String>,
    #[serde(skip)]
    pub detail: String,
    pub created_at: DateTime<Utc>,
}

impl TaskEvent {
    pub fn detail_value(&self) -> Value {
        serde_json::from_str(&self.detail).unwrap_or(Value::Null)
    }
}

#[derive(Debug, Clone, Default)]
pub struct NewMission {
    pub title: String,
    pub goal: String,
    pub status: String,
    pub lead_kwant_id: String,
    pub channel_id: Option<String>,
    pub direct_kwant_id: Option<String>,
    pub thread_id: Option<String>,
    pub workspace_path: String,
    pub budget_tokens: i64,
    pub created_by: String,
}

#[derive(Debug, Clone, Default)]
pub struct NewTask {
    pub mission_id: Option<String>,
    pub title: String,
    pub acceptance: String,
    pub state: String,
    pub priority: i64,
    pub owner_kwant_id: Option<String>,
    pub suggested_kwant_id: Option<String>,
    pub budget_tokens: i64,
    pub max_attempts: i64,
    pub source_kind: String,
    pub source_id: Option<String>,
    pub channel_id: Option<String>,
    pub direct_kwant_id: Option<String>,
    pub thread_id: Option<String>,
    pub created_by: String,
}

#[derive(Debug, Clone, Default)]
pub struct NewCheck {
    pub task_id: String,
    pub kind: String,
    pub spec: Value,
    pub required: bool,
    pub approved: bool,
    pub requested_by: String,
}

#[derive(Debug, Clone, Default)]
pub struct NewEvidence {
    pub task_id: String,
    pub kind: String,
    pub label: String,
    pub uri: Option<String>,
    pub path: Option<String>,
    pub sha256: Option<String>,
    pub bytes: Option<i64>,
    pub excerpt: Option<String>,
    pub captured_by: String,
    pub run_id: Option<String>,
}

/// Why a claim was refused.
#[derive(Debug, Clone, PartialEq)]
pub enum ClaimRefused {
    NotReady(String),
    Held,
    BotBusy(String),
    Budget { remaining: i64, needed: i64 },
    MissionNotActive,
    Missing,
}

impl std::fmt::Display for ClaimRefused {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ClaimRefused::NotReady(state) => write!(f, "the task is {state}, not ready"),
            ClaimRefused::Held => write!(f, "someone else holds this task"),
            ClaimRefused::BotBusy(task) => write!(f, "you already hold task {task}; finish or release it first"),
            ClaimRefused::Budget { remaining, needed } => {
                write!(f, "the mission's budget can't cover it: {remaining} tokens left, the task needs {needed}")
            }
            ClaimRefused::MissionNotActive => write!(f, "the mission isn't approved and active"),
            ClaimRefused::Missing => write!(f, "no such task"),
        }
    }
}

pub fn now_ms() -> i64 {
    Utc::now().timestamp_millis()
}

const TRIGGERS: [&str; 8] = [
    // The log is history: nothing rewrites or removes it
    "CREATE TRIGGER IF NOT EXISTS task_events_append_only_update BEFORE UPDATE ON task_events \
     BEGIN SELECT RAISE(ABORT, 'task_events is append-only'); END",
    "CREATE TRIGGER IF NOT EXISTS task_events_append_only_delete BEFORE DELETE ON task_events \
     BEGIN SELECT RAISE(ABORT, 'task_events is append-only'); END",
    // Verified only through a passing, approved check that isn't the owner's
    "CREATE TRIGGER IF NOT EXISTS tasks_verified_needs_check BEFORE UPDATE OF state ON tasks \
     WHEN NEW.state = 'verified' AND OLD.state != 'verified' AND NOT EXISTS ( \
        SELECT 1 FROM outcome_checks c WHERE c.task_id = NEW.id AND c.status = 'passed' AND c.approved = 1 \
        AND c.checked_by IS NOT NULL AND c.checked_by != COALESCE(NEW.owner_kwant_id, '')) \
     BEGIN SELECT RAISE(ABORT, 'a task is verified only by a passing check that is not its owner''s'); END",
    "CREATE TRIGGER IF NOT EXISTS tasks_insert_not_verified BEFORE INSERT ON tasks \
     WHEN NEW.state IN ('verified', 'working', 'needs_you') \
     BEGIN SELECT RAISE(ABORT, 'a new task starts before work'); END",
    // Working only with a live lease
    "CREATE TRIGGER IF NOT EXISTS tasks_working_needs_lease BEFORE UPDATE OF state ON tasks \
     WHEN NEW.state = 'working' AND NOT EXISTS ( \
        SELECT 1 FROM agent_claims a WHERE a.task_id = NEW.id AND a.released_at IS NULL \
        AND a.expires_at > CAST(strftime('%s', 'now') AS INTEGER) * 1000) \
     BEGIN SELECT RAISE(ABORT, 'a task is worked on only under a live lease'); END",
    // Ready only when every dependency is verified and the mission is active
    "CREATE TRIGGER IF NOT EXISTS tasks_ready_needs_deps BEFORE UPDATE OF state ON tasks \
     WHEN NEW.state = 'ready' AND (EXISTS ( \
        SELECT 1 FROM task_dependencies d JOIN tasks t ON t.id = d.depends_on_id \
        WHERE d.task_id = NEW.id AND t.state != 'verified') \
        OR EXISTS (SELECT 1 FROM missions m WHERE m.id = NEW.mission_id AND m.status != 'active')) \
     BEGIN SELECT RAISE(ABORT, 'a task is ready only when its mission is active and its dependencies are verified'); END",
    "CREATE TRIGGER IF NOT EXISTS tasks_insert_ready_needs_mission BEFORE INSERT ON tasks \
     WHEN NEW.state = 'ready' AND EXISTS (SELECT 1 FROM missions m WHERE m.id = NEW.mission_id AND m.status != 'active') \
     BEGIN SELECT RAISE(ABORT, 'a task is ready only when its mission is active'); END",
    // Terminal states stay terminal
    "CREATE TRIGGER IF NOT EXISTS tasks_terminal BEFORE UPDATE OF state ON tasks \
     WHEN OLD.state IN ('verified', 'cancelled') AND NEW.state != OLD.state \
     BEGIN SELECT RAISE(ABORT, 'a verified or cancelled task is final'); END",
];

impl Database {
    pub(super) async fn migrate_missions(&self) -> Result<(), DbError> {
        for stmt in [
            "CREATE TABLE IF NOT EXISTS missions (
                id TEXT PRIMARY KEY,
                space_id TEXT NOT NULL,
                title TEXT NOT NULL,
                goal TEXT NOT NULL DEFAULT '',
                status TEXT NOT NULL CHECK (status IN ('draft', 'active', 'done', 'cancelled')),
                lead_kwant_id TEXT NOT NULL,
                channel_id TEXT,
                direct_kwant_id TEXT,
                thread_id TEXT,
                workspace_path TEXT NOT NULL DEFAULT '',
                budget_tokens INTEGER NOT NULL CHECK (budget_tokens > 0),
                spent_tokens INTEGER NOT NULL DEFAULT 0,
                created_by TEXT NOT NULL,
                approved_at TIMESTAMP,
                approved_by TEXT,
                created_at TIMESTAMP NOT NULL,
                updated_at TIMESTAMP NOT NULL,
                finished_at TIMESTAMP
            )",
            "CREATE INDEX IF NOT EXISTS missions_space ON missions (space_id, status)",
            "CREATE TABLE IF NOT EXISTS tasks (
                id TEXT PRIMARY KEY,
                space_id TEXT NOT NULL,
                mission_id TEXT REFERENCES missions(id),
                title TEXT NOT NULL,
                acceptance TEXT NOT NULL DEFAULT '',
                state TEXT NOT NULL CHECK (state IN ('backlog', 'ready', 'working', 'needs_you', 'review', 'verified', 'blocked', 'failed', 'cancelled')),
                priority INTEGER NOT NULL DEFAULT 1 CHECK (priority BETWEEN 0 AND 3),
                owner_kwant_id TEXT,
                suggested_kwant_id TEXT,
                budget_tokens INTEGER NOT NULL CHECK (budget_tokens > 0),
                spent_tokens INTEGER NOT NULL DEFAULT 0,
                attempts INTEGER NOT NULL DEFAULT 0,
                max_attempts INTEGER NOT NULL DEFAULT 3,
                blocked_reason TEXT,
                source_kind TEXT NOT NULL CHECK (source_kind IN ('mission', 'person', 'delegation', 'routine', 'zombie', 'work_ahead', 'chat')),
                source_id TEXT,
                channel_id TEXT,
                direct_kwant_id TEXT,
                thread_id TEXT,
                created_by TEXT NOT NULL,
                created_at TIMESTAMP NOT NULL,
                updated_at TIMESTAMP NOT NULL,
                finished_at TIMESTAMP
            )",
            "CREATE INDEX IF NOT EXISTS tasks_space_state ON tasks (space_id, state)",
            "CREATE INDEX IF NOT EXISTS tasks_mission ON tasks (mission_id)",
            "CREATE INDEX IF NOT EXISTS tasks_source ON tasks (source_kind, source_id)",
            "CREATE TABLE IF NOT EXISTS task_dependencies (
                task_id TEXT NOT NULL REFERENCES tasks(id),
                depends_on_id TEXT NOT NULL REFERENCES tasks(id),
                PRIMARY KEY (task_id, depends_on_id),
                CHECK (task_id != depends_on_id)
            )",
            "CREATE TABLE IF NOT EXISTS task_events (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                task_id TEXT NOT NULL,
                mission_id TEXT,
                kind TEXT NOT NULL,
                actor TEXT NOT NULL,
                from_state TEXT,
                to_state TEXT,
                detail TEXT NOT NULL DEFAULT '{}',
                created_at TIMESTAMP NOT NULL
            )",
            "CREATE INDEX IF NOT EXISTS task_events_task ON task_events (task_id, id)",
            "CREATE TABLE IF NOT EXISTS agent_claims (
                id TEXT PRIMARY KEY,
                task_id TEXT NOT NULL REFERENCES tasks(id),
                kwant_id TEXT NOT NULL,
                turn_id TEXT,
                context_id TEXT,
                system INTEGER NOT NULL DEFAULT 0,
                claimed_at INTEGER NOT NULL,
                expires_at INTEGER NOT NULL,
                released_at INTEGER,
                release_reason TEXT
            )",
            // One open claim per task, and per bot for the claims a bot takes itself
            "CREATE UNIQUE INDEX IF NOT EXISTS agent_claims_one_per_task ON agent_claims (task_id) WHERE released_at IS NULL",
            "CREATE UNIQUE INDEX IF NOT EXISTS agent_claims_one_per_bot ON agent_claims (kwant_id) WHERE released_at IS NULL AND system = 0",
            "CREATE TABLE IF NOT EXISTS evidence (
                id TEXT PRIMARY KEY,
                task_id TEXT NOT NULL REFERENCES tasks(id),
                kind TEXT NOT NULL CHECK (kind IN ('file', 'url', 'note', 'screenshot', 'check_output')),
                label TEXT NOT NULL,
                uri TEXT,
                path TEXT,
                sha256 TEXT,
                bytes INTEGER,
                excerpt TEXT,
                captured_by TEXT NOT NULL,
                run_id TEXT,
                captured_at TIMESTAMP NOT NULL
            )",
            "CREATE INDEX IF NOT EXISTS evidence_task ON evidence (task_id)",
            "CREATE TABLE IF NOT EXISTS outcome_checks (
                id TEXT PRIMARY KEY,
                task_id TEXT NOT NULL REFERENCES tasks(id),
                kind TEXT NOT NULL CHECK (kind IN ('command', 'http', 'file_exists', 'file_contains', 'file_sha256', 'peer_review', 'person')),
                spec TEXT NOT NULL,
                required INTEGER NOT NULL DEFAULT 1,
                approved INTEGER NOT NULL DEFAULT 1,
                status TEXT NOT NULL CHECK (status IN ('pending', 'needs_approval', 'running', 'passed', 'failed', 'error')),
                output_excerpt TEXT,
                checked_by TEXT,
                requested_by TEXT NOT NULL,
                evidence_id TEXT,
                created_at TIMESTAMP NOT NULL,
                started_at TIMESTAMP,
                finished_at TIMESTAMP
            )",
            "CREATE INDEX IF NOT EXISTS outcome_checks_task ON outcome_checks (task_id)",
        ] {
            sqlx::query(stmt).execute(&self.pool).await?;
        }
        for t in TRIGGERS {
            sqlx::query(t).execute(&self.pool).await?;
        }
        // A restart stops every check that was running
        sqlx::query("UPDATE outcome_checks SET status = 'error', output_excerpt = 'kwnt restarted while the check ran', finished_at = ? WHERE status = 'running'")
            .bind(Utc::now())
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    // Missions

    pub async fn create_mission(&self, space: &SpaceId, m: &NewMission) -> Result<Mission, DbError> {
        let now = Utc::now();
        let approved = m.status == "active";
        Ok(sqlx::query_as::<_, Mission>(
            "INSERT INTO missions (id, space_id, title, goal, status, lead_kwant_id, channel_id, direct_kwant_id, thread_id, \
             workspace_path, budget_tokens, created_by, approved_at, approved_by, created_at, updated_at) \
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?) RETURNING *",
        )
        .bind(Uuid::new_v4().to_string())
        .bind(space.as_str())
        .bind(&m.title)
        .bind(&m.goal)
        .bind(&m.status)
        .bind(&m.lead_kwant_id)
        .bind(&m.channel_id)
        .bind(&m.direct_kwant_id)
        .bind(&m.thread_id)
        .bind(&m.workspace_path)
        .bind(m.budget_tokens)
        .bind(&m.created_by)
        .bind(approved.then_some(now))
        .bind(approved.then(|| m.created_by.clone()))
        .bind(now)
        .bind(now)
        .fetch_one(&self.pool)
        .await?)
    }

    pub async fn get_mission(&self, id: &str) -> Result<Option<Mission>, DbError> {
        Ok(sqlx::query_as::<_, Mission>("SELECT * FROM missions WHERE id = ?").bind(id).fetch_optional(&self.pool).await?)
    }

    pub async fn list_missions(&self, space: &SpaceId, status: Option<&str>) -> Result<Vec<Mission>, DbError> {
        Ok(sqlx::query_as::<_, Mission>(
            "SELECT * FROM missions WHERE space_id = ? AND (? IS NULL OR status = ?) ORDER BY created_at DESC, rowid DESC LIMIT 200",
        )
        .bind(space.as_str())
        .bind(status)
        .bind(status)
        .fetch_all(&self.pool)
        .await?)
    }

    pub async fn active_missions(&self) -> Result<Vec<Mission>, DbError> {
        Ok(sqlx::query_as::<_, Mission>("SELECT * FROM missions WHERE status = 'active' ORDER BY created_at")
            .fetch_all(&self.pool)
            .await?)
    }

    /// Moves a mission from `from` to `to`; false when it wasn't in `from`.
    pub async fn set_mission_status(&self, id: &str, from: &str, to: &str, by: Option<&str>) -> Result<bool, DbError> {
        let now = Utc::now();
        let done = matches!(to, "done" | "cancelled");
        let r = sqlx::query(
            "UPDATE missions SET status = ?, updated_at = ?, \
             approved_at = CASE WHEN ? = 'active' THEN ? ELSE approved_at END, \
             approved_by = CASE WHEN ? = 'active' THEN ? ELSE approved_by END, \
             finished_at = CASE WHEN ? THEN ? ELSE finished_at END \
             WHERE id = ? AND status = ?",
        )
        .bind(to)
        .bind(now)
        .bind(to)
        .bind(now)
        .bind(to)
        .bind(by)
        .bind(done)
        .bind(now)
        .bind(id)
        .bind(from)
        .execute(&self.pool)
        .await?;
        Ok(r.rows_affected() == 1)
    }

    pub async fn edit_mission(&self, id: &str, title: Option<&str>, goal: Option<&str>, budget: Option<i64>) -> Result<bool, DbError> {
        let r = sqlx::query(
            "UPDATE missions SET title = COALESCE(?, title), goal = COALESCE(?, goal), budget_tokens = COALESCE(?, budget_tokens), \
             updated_at = ? WHERE id = ?",
        )
        .bind(title)
        .bind(goal)
        .bind(budget)
        .bind(Utc::now())
        .bind(id)
        .execute(&self.pool)
        .await?;
        Ok(r.rows_affected() == 1)
    }

    /// Tokens working tasks of the mission may still spend (their budget minus what they spent).
    pub async fn mission_reserved(&self, mission_id: &str) -> Result<i64, DbError> {
        let (n,): (i64,) = sqlx::query_as(
            "SELECT COALESCE(SUM(MAX(budget_tokens - spent_tokens, 0)), 0) FROM tasks \
             WHERE mission_id = ? AND state IN ('working', 'needs_you')",
        )
        .bind(mission_id)
        .fetch_one(&self.pool)
        .await?;
        Ok(n)
    }

    pub async fn mission_counts(&self, mission_id: &str) -> Result<Vec<(String, i64)>, DbError> {
        Ok(sqlx::query_as("SELECT state, COUNT(*) FROM tasks WHERE mission_id = ? GROUP BY state")
            .bind(mission_id)
            .fetch_all(&self.pool)
            .await?)
    }

    // Tasks

    pub async fn create_task(&self, space: &SpaceId, t: &NewTask) -> Result<Task, DbError> {
        let now = Utc::now();
        let mut tx = self.pool.begin().await?;
        let task = sqlx::query_as::<_, Task>(
            "INSERT INTO tasks (id, space_id, mission_id, title, acceptance, state, priority, owner_kwant_id, suggested_kwant_id, \
             budget_tokens, max_attempts, source_kind, source_id, channel_id, direct_kwant_id, thread_id, created_by, created_at, updated_at) \
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?) RETURNING *",
        )
        .bind(Uuid::new_v4().to_string())
        .bind(space.as_str())
        .bind(&t.mission_id)
        .bind(&t.title)
        .bind(&t.acceptance)
        .bind(&t.state)
        .bind(t.priority)
        .bind(&t.owner_kwant_id)
        .bind(&t.suggested_kwant_id)
        .bind(t.budget_tokens)
        .bind(t.max_attempts.max(1))
        .bind(&t.source_kind)
        .bind(&t.source_id)
        .bind(&t.channel_id)
        .bind(&t.direct_kwant_id)
        .bind(&t.thread_id)
        .bind(&t.created_by)
        .bind(now)
        .bind(now)
        .fetch_one(&mut *tx)
        .await?;
        insert_event(&mut tx, &task.id, task.mission_id.as_deref(), "created", &t.created_by, None, Some(&task.state), &serde_json::json!({"source_kind": t.source_kind, "source_id": t.source_id})).await?;
        tx.commit().await?;
        Ok(task)
    }

    pub async fn get_task(&self, id: &str) -> Result<Option<Task>, DbError> {
        Ok(sqlx::query_as::<_, Task>("SELECT * FROM tasks WHERE id = ?").bind(id).fetch_optional(&self.pool).await?)
    }

    pub async fn task_by_source(&self, kind: &str, source_id: &str) -> Result<Option<Task>, DbError> {
        Ok(sqlx::query_as::<_, Task>("SELECT * FROM tasks WHERE source_kind = ? AND source_id = ? ORDER BY created_at DESC LIMIT 1")
            .bind(kind)
            .bind(source_id)
            .fetch_optional(&self.pool)
            .await?)
    }

    pub async fn list_tasks(
        &self,
        space: &SpaceId,
        state: Option<&str>,
        mission_id: Option<&str>,
        source_kind: Option<&str>,
        limit: i64,
    ) -> Result<Vec<Task>, DbError> {
        Ok(sqlx::query_as::<_, Task>(
            "SELECT * FROM tasks WHERE space_id = ? AND (? IS NULL OR state = ?) AND (? IS NULL OR mission_id = ?) \
             AND (? IS NULL OR source_kind = ?) ORDER BY priority DESC, created_at, rowid LIMIT ?",
        )
        .bind(space.as_str())
        .bind(state)
        .bind(state)
        .bind(mission_id)
        .bind(mission_id)
        .bind(source_kind)
        .bind(source_kind)
        .bind(limit.clamp(1, 1000))
        .fetch_all(&self.pool)
        .await?)
    }

    pub async fn tasks_in_state(&self, state: &str) -> Result<Vec<Task>, DbError> {
        Ok(sqlx::query_as::<_, Task>("SELECT * FROM tasks WHERE state = ? ORDER BY created_at").bind(state).fetch_all(&self.pool).await?)
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn edit_task(
        &self,
        id: &str,
        title: Option<&str>,
        acceptance: Option<&str>,
        priority: Option<i64>,
        owner: Option<&str>,
        budget: Option<i64>,
        actor: &str,
    ) -> Result<bool, DbError> {
        let mut tx = self.pool.begin().await?;
        let r = sqlx::query(
            "UPDATE tasks SET title = COALESCE(?, title), acceptance = COALESCE(?, acceptance), priority = COALESCE(?, priority), \
             suggested_kwant_id = COALESCE(?, suggested_kwant_id), budget_tokens = COALESCE(?, budget_tokens), updated_at = ? WHERE id = ?",
        )
        .bind(title)
        .bind(acceptance)
        .bind(priority)
        .bind(owner)
        .bind(budget)
        .bind(Utc::now())
        .bind(id)
        .execute(&mut *tx)
        .await?;
        if r.rows_affected() == 1 {
            let detail = serde_json::json!({"title": title, "acceptance": acceptance.map(|_| "changed"), "priority": priority, "owner": owner, "budget_tokens": budget});
            insert_event(&mut tx, id, None, "edited", actor, None, None, &detail).await?;
        }
        tx.commit().await?;
        Ok(r.rows_affected() == 1)
    }

    /// Moves a task from `from` to `to` (compare-and-set) and logs it. `Ok(false)` when it wasn't in
    /// `from`; a trigger refusing the move is an error. Leaving `working` / `needs_you` releases the claim.
    pub async fn move_task(&self, id: &str, from: &str, to: &str, actor: &str, change: &TaskChange) -> Result<bool, DbError> {
        let now = Utc::now();
        let mut tx = self.pool.begin().await?;
        let r = sqlx::query(
            "UPDATE tasks SET state = ?, updated_at = ?, \
             attempts = attempts + ?, \
             blocked_reason = CASE WHEN ? = 'blocked' THEN ? ELSE NULL END, \
             owner_kwant_id = CASE WHEN ? THEN ? ELSE owner_kwant_id END, \
             finished_at = CASE WHEN ? IN ('verified', 'cancelled', 'failed') THEN ? ELSE NULL END \
             WHERE id = ? AND state = ?",
        )
        .bind(to)
        .bind(now)
        .bind(i64::from(change.attempt))
        .bind(to)
        .bind(&change.reason)
        .bind(change.owner.is_some())
        .bind(change.owner.as_ref().and_then(|o| o.clone()))
        .bind(to)
        .bind(now)
        .bind(id)
        .bind(from)
        .execute(&mut *tx)
        .await?;
        if r.rows_affected() != 1 {
            return Ok(false);
        }
        let releases = matches!(from, "working" | "needs_you") && !matches!(to, "working" | "needs_you");
        if releases {
            sqlx::query("UPDATE agent_claims SET released_at = ?, release_reason = ? WHERE task_id = ? AND released_at IS NULL")
                .bind(now_ms())
                .bind(to)
                .bind(id)
                .execute(&mut *tx)
                .await?;
        }
        let mission: Option<(Option<String>,)> = sqlx::query_as("SELECT mission_id FROM tasks WHERE id = ?").bind(id).fetch_optional(&mut *tx).await?;
        let mut detail = change.detail.clone().unwrap_or_else(|| serde_json::json!({}));
        if let (Some(reason), Some(obj)) = (&change.reason, detail.as_object_mut()) {
            obj.insert("reason".into(), Value::String(reason.clone()));
        }
        insert_event(&mut tx, id, mission.and_then(|m| m.0).as_deref(), &change.kind, actor, Some(from), Some(to), &detail).await?;
        tx.commit().await?;
        Ok(true)
    }

    /// Takes `task_id` for `kwant_id` with a lease of `lease_ms`, atomically: the task must be
    /// ready, its dependencies verified, its mission active and able to cover the task's budget,
    /// and the bot must hold no other claim of its own (`system` claims are kwant's, for hand-offs).
    #[allow(clippy::too_many_arguments)]
    pub async fn claim_task(
        &self,
        task_id: &str,
        kwant_id: &str,
        turn_id: Option<&str>,
        context_id: Option<&str>,
        lease_ms: i64,
        system: bool,
        actor: &str,
    ) -> Result<Result<Task, ClaimRefused>, DbError> {
        let mut tx = self.pool.begin().await?;
        // Take the write lock first, so two claims can't both read "ready"
        sqlx::query("UPDATE tasks SET updated_at = updated_at WHERE id = ?").bind(task_id).execute(&mut *tx).await?;
        let Some(task) = sqlx::query_as::<_, Task>("SELECT * FROM tasks WHERE id = ?").bind(task_id).fetch_optional(&mut *tx).await? else {
            return Ok(Err(ClaimRefused::Missing));
        };
        if task.state != "ready" {
            let held = matches!(task.state.as_str(), "working" | "needs_you");
            return Ok(Err(if held { ClaimRefused::Held } else { ClaimRefused::NotReady(task.state) }));
        }
        let now = now_ms();
        if !system {
            let busy: Option<(String,)> = sqlx::query_as(
                "SELECT task_id FROM agent_claims WHERE kwant_id = ? AND system = 0 AND released_at IS NULL AND expires_at > ?",
            )
            .bind(kwant_id)
            .bind(now)
            .fetch_optional(&mut *tx)
            .await?;
            if let Some((other,)) = busy {
                return Ok(Err(ClaimRefused::BotBusy(other)));
            }
        }
        if let Some(mid) = &task.mission_id {
            let Some(m) = sqlx::query_as::<_, Mission>("SELECT * FROM missions WHERE id = ?").bind(mid).fetch_optional(&mut *tx).await? else {
                return Ok(Err(ClaimRefused::MissionNotActive));
            };
            if m.status != "active" {
                return Ok(Err(ClaimRefused::MissionNotActive));
            }
            let (reserved,): (i64,) = sqlx::query_as(
                "SELECT COALESCE(SUM(MAX(budget_tokens - spent_tokens, 0)), 0) FROM tasks WHERE mission_id = ? AND state IN ('working', 'needs_you')",
            )
            .bind(mid)
            .fetch_one(&mut *tx)
            .await?;
            let needed = (task.budget_tokens - task.spent_tokens).max(1);
            let remaining = m.budget_tokens - m.spent_tokens - reserved;
            if needed > remaining {
                return Ok(Err(ClaimRefused::Budget { remaining: remaining.max(0), needed }));
            }
        }
        // Expired claims of this task (or the bot's own) are let go before the new one
        sqlx::query("UPDATE agent_claims SET released_at = ?, release_reason = 'expired' WHERE released_at IS NULL AND expires_at <= ? AND (task_id = ? OR kwant_id = ?)")
            .bind(now)
            .bind(now)
            .bind(task_id)
            .bind(kwant_id)
            .execute(&mut *tx)
            .await?;
        sqlx::query(
            "INSERT INTO agent_claims (id, task_id, kwant_id, turn_id, context_id, system, claimed_at, expires_at) VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(Uuid::new_v4().to_string())
        .bind(task_id)
        .bind(kwant_id)
        .bind(turn_id)
        .bind(context_id)
        .bind(system)
        .bind(now)
        .bind(now + lease_ms.max(1000))
        .execute(&mut *tx)
        .await?;
        sqlx::query("UPDATE tasks SET state = 'working', owner_kwant_id = ?, updated_at = ? WHERE id = ? AND state = 'ready'")
            .bind(kwant_id)
            .bind(Utc::now())
            .bind(task_id)
            .execute(&mut *tx)
            .await?;
        insert_event(
            &mut tx,
            task_id,
            task.mission_id.as_deref(),
            "claimed",
            actor,
            Some("ready"),
            Some("working"),
            &serde_json::json!({"kwant_id": kwant_id, "lease_ms": lease_ms, "system": system}),
        )
        .await?;
        let task = sqlx::query_as::<_, Task>("SELECT * FROM tasks WHERE id = ?").bind(task_id).fetch_one(&mut *tx).await?;
        tx.commit().await?;
        Ok(Ok(task))
    }

    /// The task's open claim, live or expired-but-not-yet-swept.
    pub async fn open_claim(&self, task_id: &str) -> Result<Option<Claim>, DbError> {
        Ok(sqlx::query_as::<_, Claim>("SELECT * FROM agent_claims WHERE task_id = ? AND released_at IS NULL")
            .bind(task_id)
            .fetch_optional(&self.pool)
            .await?)
    }

    /// Open claims of `kwant_id`: system claims first.
    pub async fn claims_of(&self, kwant_id: &str) -> Result<Vec<Claim>, DbError> {
        Ok(sqlx::query_as::<_, Claim>(
            "SELECT * FROM agent_claims WHERE kwant_id = ? AND released_at IS NULL ORDER BY system DESC, claimed_at",
        )
        .bind(kwant_id)
        .fetch_all(&self.pool)
        .await?)
    }

    /// Pushes the task's open claim out to `now + lease_ms` (never shortens it).
    pub async fn renew_claim(&self, task_id: &str, lease_ms: i64) -> Result<bool, DbError> {
        let now = now_ms();
        let r = sqlx::query("UPDATE agent_claims SET expires_at = MAX(expires_at, ?) WHERE task_id = ? AND released_at IS NULL AND expires_at > ?")
            .bind(now + lease_ms)
            .bind(task_id)
            .bind(now)
            .execute(&self.pool)
            .await?;
        Ok(r.rows_affected() == 1)
    }

    /// Working tasks whose lease ran out (needs_you tasks wait for the person and don't expire).
    pub async fn expired_working(&self) -> Result<Vec<Task>, DbError> {
        Ok(sqlx::query_as::<_, Task>(
            "SELECT t.* FROM tasks t WHERE t.state = 'working' AND NOT EXISTS ( \
               SELECT 1 FROM agent_claims a WHERE a.task_id = t.id AND a.released_at IS NULL AND a.expires_at > ?)",
        )
        .bind(now_ms())
        .fetch_all(&self.pool)
        .await?)
    }

    /// Test hook: makes the task's open claim expire now.
    #[doc(hidden)]
    pub async fn expire_claim_now(&self, task_id: &str) -> Result<(), DbError> {
        sqlx::query("UPDATE agent_claims SET expires_at = ? WHERE task_id = ? AND released_at IS NULL")
            .bind(now_ms() - 1)
            .bind(task_id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    /// Adds `tokens` to the task and its mission.
    pub async fn spend_on_task(&self, task_id: &str, tokens: i64) -> Result<(), DbError> {
        if tokens <= 0 {
            return Ok(());
        }
        let mut tx = self.pool.begin().await?;
        sqlx::query("UPDATE tasks SET spent_tokens = spent_tokens + ? WHERE id = ?").bind(tokens).bind(task_id).execute(&mut *tx).await?;
        sqlx::query("UPDATE missions SET spent_tokens = spent_tokens + ?, updated_at = ? WHERE id = (SELECT mission_id FROM tasks WHERE id = ?)")
            .bind(tokens)
            .bind(Utc::now())
            .bind(task_id)
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        Ok(())
    }

    // Dependencies

    /// Adds `task_id` → `depends_on`, refusing a cycle or a link across missions.
    pub async fn add_dependency(&self, task_id: &str, depends_on: &str) -> Result<(), DbError> {
        let a = self.get_task(task_id).await?.ok_or_else(|| DbError::Conflict("no such task".into()))?;
        let b = self.get_task(depends_on).await?.ok_or_else(|| DbError::Conflict("no such dependency".into()))?;
        if a.mission_id != b.mission_id || a.space_id != b.space_id {
            return Err(DbError::Conflict("a task can depend only on a task of the same mission".into()));
        }
        // depends_on (transitively) on task_id already → a cycle
        let (cycle,): (i64,) = sqlx::query_as(
            "WITH RECURSIVE up(id) AS (SELECT ? UNION SELECT d.depends_on_id FROM task_dependencies d JOIN up ON d.task_id = up.id) \
             SELECT COUNT(*) FROM up WHERE id = ?",
        )
        .bind(depends_on)
        .bind(task_id)
        .fetch_one(&self.pool)
        .await?;
        if cycle > 0 {
            return Err(DbError::Conflict("that dependency would make a cycle".into()));
        }
        sqlx::query("INSERT OR IGNORE INTO task_dependencies (task_id, depends_on_id) VALUES (?, ?)")
            .bind(task_id)
            .bind(depends_on)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    pub async fn dependencies_of(&self, task_id: &str) -> Result<Vec<(String, String)>, DbError> {
        Ok(sqlx::query_as("SELECT t.id, t.state FROM task_dependencies d JOIN tasks t ON t.id = d.depends_on_id WHERE d.task_id = ?")
            .bind(task_id)
            .fetch_all(&self.pool)
            .await?)
    }

    /// Backlog tasks of `mission_id` whose dependencies are all verified.
    pub async fn promotable(&self, mission_id: &str) -> Result<Vec<Task>, DbError> {
        Ok(sqlx::query_as::<_, Task>(
            "SELECT t.* FROM tasks t WHERE t.mission_id = ? AND t.state = 'backlog' AND NOT EXISTS ( \
               SELECT 1 FROM task_dependencies d JOIN tasks u ON u.id = d.depends_on_id WHERE d.task_id = t.id AND u.state != 'verified')",
        )
        .bind(mission_id)
        .fetch_all(&self.pool)
        .await?)
    }

    /// Backlog tasks waiting on `task_id` whose dependencies are now all verified.
    pub async fn dependents_promotable(&self, task_id: &str) -> Result<Vec<Task>, DbError> {
        Ok(sqlx::query_as::<_, Task>(
            "SELECT t.* FROM tasks t JOIN task_dependencies w ON w.task_id = t.id AND w.depends_on_id = ? WHERE t.state = 'backlog' AND NOT EXISTS ( \
               SELECT 1 FROM task_dependencies d JOIN tasks u ON u.id = d.depends_on_id WHERE d.task_id = t.id AND u.state != 'verified')",
        )
        .bind(task_id)
        .fetch_all(&self.pool)
        .await?)
    }

    /// A retried task gets its attempts back.
    pub async fn reset_attempts(&self, id: &str) -> Result<(), DbError> {
        sqlx::query("UPDATE tasks SET attempts = 0, updated_at = ? WHERE id = ?").bind(Utc::now()).bind(id).execute(&self.pool).await?;
        Ok(())
    }

    /// A routine run's routine and status.
    pub async fn routine_run_of(&self, run_id: &str) -> Result<Option<(String, String)>, DbError> {
        Ok(sqlx::query_as("SELECT routine_id, status FROM routine_runs WHERE id = ?").bind(run_id).fetch_optional(&self.pool).await?)
    }

    /// Open tasks kwant holds a system claim on (hand-offs, routine runs).
    pub async fn system_held(&self) -> Result<Vec<Task>, DbError> {
        Ok(sqlx::query_as::<_, Task>(
            "SELECT t.* FROM tasks t JOIN agent_claims a ON a.task_id = t.id AND a.released_at IS NULL AND a.system = 1 \
             WHERE t.state IN ('working', 'needs_you')",
        )
        .fetch_all(&self.pool)
        .await?)
    }

    /// The bot an approval card belongs to.
    pub async fn approval_kwant(&self, approval_id: &str) -> Result<Option<String>, DbError> {
        let row: Option<(String,)> = sqlx::query_as("SELECT kwant_id FROM approvals WHERE id = ?").bind(approval_id).fetch_optional(&self.pool).await?;
        Ok(row.map(|r| r.0))
    }

    // Evidence and checks

    pub async fn add_evidence(&self, e: &NewEvidence) -> Result<Evidence, DbError> {
        let mut tx = self.pool.begin().await?;
        let ev = sqlx::query_as::<_, Evidence>(
            "INSERT INTO evidence (id, task_id, kind, label, uri, path, sha256, bytes, excerpt, captured_by, run_id, captured_at) \
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?) RETURNING *",
        )
        .bind(Uuid::new_v4().to_string())
        .bind(&e.task_id)
        .bind(&e.kind)
        .bind(&e.label)
        .bind(&e.uri)
        .bind(&e.path)
        .bind(&e.sha256)
        .bind(e.bytes)
        .bind(&e.excerpt)
        .bind(&e.captured_by)
        .bind(&e.run_id)
        .bind(Utc::now())
        .fetch_one(&mut *tx)
        .await?;
        insert_event(&mut tx, &e.task_id, None, "evidence", &e.captured_by, None, None, &serde_json::json!({"evidence_id": ev.id, "kind": e.kind, "label": e.label})).await?;
        tx.commit().await?;
        Ok(ev)
    }

    pub async fn list_evidence(&self, task_id: &str) -> Result<Vec<Evidence>, DbError> {
        Ok(sqlx::query_as::<_, Evidence>("SELECT * FROM evidence WHERE task_id = ? ORDER BY captured_at, rowid").bind(task_id).fetch_all(&self.pool).await?)
    }

    pub async fn add_check(&self, c: &NewCheck) -> Result<OutcomeCheck, DbError> {
        let status = if c.approved { "pending" } else { "needs_approval" };
        let mut tx = self.pool.begin().await?;
        let check = sqlx::query_as::<_, OutcomeCheck>(
            "INSERT INTO outcome_checks (id, task_id, kind, spec, required, approved, status, requested_by, created_at) \
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?) RETURNING *",
        )
        .bind(Uuid::new_v4().to_string())
        .bind(&c.task_id)
        .bind(&c.kind)
        .bind(c.spec.to_string())
        .bind(c.required)
        .bind(c.approved)
        .bind(status)
        .bind(&c.requested_by)
        .bind(Utc::now())
        .fetch_one(&mut *tx)
        .await?;
        insert_event(&mut tx, &c.task_id, None, "check_added", &c.requested_by, None, None, &serde_json::json!({"check_id": check.id, "kind": c.kind, "approved": c.approved})).await?;
        tx.commit().await?;
        Ok(check)
    }

    pub async fn get_check(&self, id: &str) -> Result<Option<OutcomeCheck>, DbError> {
        Ok(sqlx::query_as::<_, OutcomeCheck>("SELECT * FROM outcome_checks WHERE id = ?").bind(id).fetch_optional(&self.pool).await?)
    }

    pub async fn list_checks(&self, task_id: &str) -> Result<Vec<OutcomeCheck>, DbError> {
        Ok(sqlx::query_as::<_, OutcomeCheck>("SELECT * FROM outcome_checks WHERE task_id = ? ORDER BY created_at, rowid").bind(task_id).fetch_all(&self.pool).await?)
    }

    pub async fn approve_check(&self, id: &str) -> Result<bool, DbError> {
        let r = sqlx::query("UPDATE outcome_checks SET approved = 1, status = 'pending' WHERE id = ? AND status = 'needs_approval'")
            .bind(id)
            .execute(&self.pool)
            .await?;
        Ok(r.rows_affected() == 1)
    }

    /// Marks a check running; false when it is already running or not approved.
    pub async fn start_check(&self, id: &str) -> Result<bool, DbError> {
        let r = sqlx::query("UPDATE outcome_checks SET status = 'running', started_at = ?, finished_at = NULL WHERE id = ? AND approved = 1 AND status != 'running'")
            .bind(Utc::now())
            .bind(id)
            .execute(&self.pool)
            .await?;
        Ok(r.rows_affected() == 1)
    }

    pub async fn finish_check(&self, id: &str, status: &str, output: Option<&str>, checked_by: &str, evidence_id: Option<&str>) -> Result<(), DbError> {
        let mut tx = self.pool.begin().await?;
        sqlx::query("UPDATE outcome_checks SET status = ?, output_excerpt = ?, checked_by = ?, evidence_id = ?, finished_at = ? WHERE id = ?")
            .bind(status)
            .bind(output)
            .bind(checked_by)
            .bind(evidence_id)
            .bind(Utc::now())
            .bind(id)
            .execute(&mut *tx)
            .await?;
        let task: Option<(String,)> = sqlx::query_as("SELECT task_id FROM outcome_checks WHERE id = ?").bind(id).fetch_optional(&mut *tx).await?;
        if let Some((task_id,)) = task {
            insert_event(&mut tx, &task_id, None, "check_result", checked_by, None, None, &serde_json::json!({"check_id": id, "status": status})).await?;
        }
        tx.commit().await?;
        Ok(())
    }

    // Events

    pub async fn log_task_event(&self, task_id: &str, kind: &str, actor: &str, detail: &Value) -> Result<(), DbError> {
        let mut tx = self.pool.begin().await?;
        insert_event(&mut tx, task_id, None, kind, actor, None, None, detail).await?;
        tx.commit().await?;
        Ok(())
    }

    pub async fn task_events(&self, task_id: &str, limit: i64) -> Result<Vec<TaskEvent>, DbError> {
        let mut rows = sqlx::query_as::<_, TaskEvent>("SELECT * FROM task_events WHERE task_id = ? ORDER BY id DESC LIMIT ?")
            .bind(task_id)
            .bind(limit)
            .fetch_all(&self.pool)
            .await?;
        rows.reverse();
        Ok(rows)
    }

    /// Events of `kinds` in `space` since `since`, for the Insights counts.
    pub async fn space_events_since(&self, space: &SpaceId, since: DateTime<Utc>) -> Result<Vec<TaskEvent>, DbError> {
        Ok(sqlx::query_as::<_, TaskEvent>(
            "SELECT e.* FROM task_events e JOIN tasks t ON t.id = e.task_id WHERE t.space_id = ? AND e.created_at >= ? ORDER BY e.id",
        )
        .bind(space.as_str())
        .bind(since)
        .fetch_all(&self.pool)
        .await?)
    }

    /// The bot's open question, human-step and approval cards as (kind, id, text, where): `where`
    /// is the chat (thread, channel or direct) for questions and steps, the turn for approvals.
    pub async fn open_cards_of(&self, kwant_id: &str) -> Result<Vec<(String, String, String, Option<String>)>, DbError> {
        Ok(sqlx::query_as(
            "SELECT 'question', id, question, COALESCE(thread_id, channel_id, direct_kwant_id) FROM questions WHERE kwant_id = ?1 AND status = 'open' \
             UNION ALL SELECT 'human_step', id, reason, COALESCE(thread_id, channel_id, direct_kwant_id) FROM human_steps WHERE kwant_id = ?1 AND status = 'open' \
             UNION ALL SELECT 'approval', id, tool_name, turn_id FROM approvals WHERE kwant_id = ?1 AND status = 'pending'",
        )
        .bind(kwant_id)
        .fetch_all(&self.pool)
        .await?)
    }

    /// Sets the task's open claim to expire `lease_ms` from now, live or not (a task back from
    /// `needs_you` gets a fresh lease).
    pub async fn refresh_claim(&self, task_id: &str, lease_ms: i64) -> Result<bool, DbError> {
        let r = sqlx::query("UPDATE agent_claims SET expires_at = ? WHERE task_id = ? AND released_at IS NULL")
            .bind(now_ms() + lease_ms)
            .bind(task_id)
            .execute(&self.pool)
            .await?;
        Ok(r.rows_affected() == 1)
    }

    /// Raw SQL for tests that prove the triggers hold against a direct write.
    #[doc(hidden)]
    pub async fn exec_raw_for_test(&self, sql: &str) -> Result<u64, DbError> {
        Ok(sqlx::query(sql).execute(&self.pool).await?.rows_affected())
    }
}

/// What else a state move changes.
#[derive(Debug, Clone)]
pub struct TaskChange {
    /// The event's kind ("state", "lease_expired", "review", …).
    pub kind: String,
    pub reason: Option<String>,
    /// Counts one more attempt.
    pub attempt: bool,
    /// `Some(x)` sets the owner to `x`.
    pub owner: Option<Option<String>>,
    pub detail: Option<Value>,
}

impl Default for TaskChange {
    fn default() -> Self {
        TaskChange { kind: "state".into(), reason: None, attempt: false, owner: None, detail: None }
    }
}

#[allow(clippy::too_many_arguments)]
async fn insert_event(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    task_id: &str,
    mission_id: Option<&str>,
    kind: &str,
    actor: &str,
    from: Option<&str>,
    to: Option<&str>,
    detail: &Value,
) -> Result<(), DbError> {
    let mission: Option<String> = match mission_id {
        Some(m) => Some(m.to_string()),
        None => sqlx::query_as::<_, (Option<String>,)>("SELECT mission_id FROM tasks WHERE id = ?")
            .bind(task_id)
            .fetch_optional(&mut **tx)
            .await?
            .and_then(|r| r.0),
    };
    sqlx::query("INSERT INTO task_events (task_id, mission_id, kind, actor, from_state, to_state, detail, created_at) VALUES (?, ?, ?, ?, ?, ?, ?, ?)")
        .bind(task_id)
        .bind(mission)
        .bind(kind)
        .bind(actor)
        .bind(from)
        .bind(to)
        .bind(detail.to_string())
        .bind(Utc::now())
        .execute(&mut **tx)
        .await?;
    Ok(())
}
