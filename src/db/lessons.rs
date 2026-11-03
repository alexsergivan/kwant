//! Self-improving bots, round 1: the records a bot's lessons are computed from. Nothing here is
//! written: lessons are derived on read from `task_failures` (append-only, written only by
//! AutoFix's `record`) and the person's own send-backs, so a bot can't add, edit or remove one.
//! The rendering and its rails live in `server::missions::lessons`. Contract:
//! docs/mission-control.md §23.

use super::{Database, DbError};
use chrono::{DateTime, Utc};
use sqlx::FromRow;

/// The failure classes that are the bot's own doing. `quota`, `auth`, `budget`, `transient`,
/// `bad_spec` and `needs_input` are not, so they never become a lesson.
pub const LESSON_CLASSES: [&str; 4] = ["check_failed", "stalled", "denied", "unknown"];

/// One `task_failures` row of a task the bot owned and held a claim on.
#[derive(Debug, Clone, FromRow, PartialEq)]
pub struct FailureOfBot {
    pub id: String,
    pub task_id: String,
    pub source: String,
    pub class: String,
    /// Read only to find which check failed (its first line is the check's kind and redacted
    /// spec); never rendered.
    pub excerpt: Option<String>,
    pub created_at: DateTime<Utc>,
    pub title: String,
    pub created_by: String,
    pub task_state: String,
    /// An AutoFix retry of this failure succeeded.
    pub fixed_by_autofix: bool,
    /// The person moved the task from failed back to ready after this failure.
    pub person_retried: bool,
}

/// The person sent a task of the bot back from review (to ready or blocked) with a reason.
#[derive(Debug, Clone, FromRow, PartialEq)]
pub struct CorrectionOfBot {
    pub task_id: String,
    pub to_state: String,
    /// The event's detail JSON; `reason` is the person's own text.
    pub detail: String,
    pub created_at: DateTime<Utc>,
    pub title: String,
    pub created_by: String,
    pub task_state: String,
}

/// The rule `score_tasks` uses: a task counts for the bot that owns it and held a claim on it.
const OWNED: &str = "t.owner_kwant_id = ? AND EXISTS (SELECT 1 FROM agent_claims a WHERE a.task_id = t.id AND a.kwant_id = ?)";

const CLASSES_IN: &str = "('check_failed', 'stalled', 'denied', 'unknown')";

const SEND_BACK: &str = "e.kind = 'state' AND e.actor = 'person' AND e.from_state = 'review' AND e.to_state IN ('ready', 'blocked')";

impl Database {
    /// Add-only: the indexes the lesson readers use (`tasks` had no owner index and
    /// `task_failures` only `(task_id, created_at)`, so every read was a scan).
    pub(super) async fn migrate_lessons(&self) -> Result<(), DbError> {
        for stmt in [
            "CREATE INDEX IF NOT EXISTS tasks_owner_finished ON tasks (owner_kwant_id, finished_at)",
            "CREATE INDEX IF NOT EXISTS task_failures_created ON task_failures (created_at)",
            "CREATE INDEX IF NOT EXISTS task_events_kind_created ON task_events (kind, created_at)",
        ] {
            sqlx::query(stmt).execute(&self.pool).await?;
        }
        Ok(())
    }

    /// The bot's failures of the lesson classes since `since`, newest first.
    pub async fn failures_of_bot(&self, kwant_id: &str, since: DateTime<Utc>, limit: i64) -> Result<Vec<FailureOfBot>, DbError> {
        let sql = format!(
            "SELECT f.id, f.task_id, f.source, f.class, f.excerpt, f.created_at, t.title, t.created_by, t.state AS task_state, \
             EXISTS (SELECT 1 FROM autofix_attempts x WHERE x.failure_id = f.id AND x.status = 'succeeded') AS fixed_by_autofix, \
             EXISTS (SELECT 1 FROM task_events e WHERE e.task_id = f.task_id AND e.kind = 'state' AND e.actor = 'person' \
                     AND e.from_state = 'failed' AND e.to_state = 'ready' AND e.created_at >= f.created_at) AS person_retried \
             FROM task_failures f JOIN tasks t ON t.id = f.task_id \
             WHERE f.created_at >= ? AND f.class IN {CLASSES_IN} AND {OWNED} \
             ORDER BY f.created_at DESC, f.rowid DESC LIMIT ?"
        );
        Ok(sqlx::query_as::<_, FailureOfBot>(&sql).bind(since).bind(kwant_id).bind(kwant_id).bind(limit).fetch_all(&self.pool).await?)
    }

    /// The person's send-backs of the bot's tasks since `since`, newest first.
    pub async fn person_corrections_of_bot(&self, kwant_id: &str, since: DateTime<Utc>, limit: i64) -> Result<Vec<CorrectionOfBot>, DbError> {
        let sql = format!(
            "SELECT e.task_id, e.to_state, e.detail, e.created_at, t.title, t.created_by, t.state AS task_state \
             FROM task_events e JOIN tasks t ON t.id = e.task_id \
             WHERE {SEND_BACK} AND e.created_at >= ? AND {OWNED} \
             ORDER BY e.created_at DESC, e.id DESC LIMIT ?"
        );
        Ok(sqlx::query_as::<_, CorrectionOfBot>(&sql).bind(since).bind(kwant_id).bind(kwant_id).bind(limit).fetch_all(&self.pool).await?)
    }

    /// Whether the bot has anything to learn from since `since`: the cheap gate before a chat
    /// turn computes its lessons.
    pub async fn has_lessons_since(&self, kwant_id: &str, since: DateTime<Utc>) -> Result<bool, DbError> {
        let sql = format!(
            "SELECT EXISTS (SELECT 1 FROM task_failures f JOIN tasks t ON t.id = f.task_id \
                            WHERE f.created_at >= ? AND f.class IN {CLASSES_IN} AND {OWNED}) \
                 OR EXISTS (SELECT 1 FROM task_events e JOIN tasks t ON t.id = e.task_id \
                            WHERE {SEND_BACK} AND e.created_at >= ? AND {OWNED})"
        );
        let (yes,): (bool,) =
            sqlx::query_as(&sql).bind(since).bind(kwant_id).bind(kwant_id).bind(since).bind(kwant_id).bind(kwant_id).fetch_one(&self.pool).await?;
        Ok(yes)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lesson_classes_are_known_failure_classes() {
        for c in LESSON_CLASSES {
            assert!(super::super::autofix::CLASSES.contains(&c), "{c}");
            assert!(CLASSES_IN.contains(&format!("'{c}'")), "{c}");
        }
        for c in ["quota", "auth", "budget", "transient", "bad_spec", "needs_input"] {
            assert!(!LESSON_CLASSES.contains(&c), "{c}");
        }
    }
}
