//! Receipts export (roadmap Next 7): the read-only queries behind `GET /api/tasks/receipts.csv`.
//! No table of its own: it reads `tasks`, `task_events`, `outcome_checks`, `evidence`, `kwants`
//! and `missions`, a few queries per page of tasks (never one per task per column).

use super::{Database, DbError};
use crate::db::spaces::SpaceId;
use chrono::{DateTime, Utc};
use sqlx::FromRow;
use std::collections::HashMap;

/// How many task ids one `IN (…)` list carries.
const PAGE: usize = 500;

/// The task columns a receipt row needs.
#[derive(Debug, Clone, FromRow)]
pub struct ReceiptTask {
    pub id: String,
    pub title: String,
    pub state: String,
    pub mission_id: Option<String>,
    pub owner_kwant_id: Option<String>,
    pub source_kind: String,
    pub created_at: DateTime<Utc>,
    pub finished_at: Option<DateTime<Utc>>,
    pub spent_tokens: i64,
}

/// One evidence row, without its excerpt.
#[derive(Debug, Clone, FromRow)]
pub struct ReceiptEvidence {
    pub task_id: String,
    pub label: String,
    pub uri: Option<String>,
    pub path: Option<String>,
    pub sha256: Option<String>,
}

/// Everything about a page of tasks that isn't on the task row itself.
#[derive(Debug, Default)]
pub struct ReceiptParts {
    /// The actor of each task's latest move to `verified` (`person`, `kwant` or a bot id).
    pub verified_by: HashMap<String, String>,
    /// Each task's checks as `(kind, status)`, oldest first.
    pub checks: HashMap<String, Vec<(String, String)>>,
    /// Each task's evidence, oldest first.
    pub evidence: HashMap<String, Vec<ReceiptEvidence>>,
    /// Bot display names, by id, for owners and verifiers.
    pub bot_names: HashMap<String, String>,
    /// Mission titles, by id.
    pub mission_titles: HashMap<String, String>,
}

fn marks(n: usize) -> String {
    vec!["?"; n].join(", ")
}

impl Database {
    /// The space's tasks in `states` whose `finished_at` is in `[from, to)`, oldest first, at most
    /// `limit` of them.
    pub async fn receipt_tasks(
        &self,
        space: &SpaceId,
        states: &[&str],
        from: DateTime<Utc>,
        to: DateTime<Utc>,
        limit: i64,
    ) -> Result<Vec<ReceiptTask>, DbError> {
        if states.is_empty() {
            return Ok(Vec::new());
        }
        let sql = format!(
            "SELECT id, title, state, mission_id, owner_kwant_id, source_kind, created_at, finished_at, spent_tokens \
             FROM tasks WHERE space_id = ? AND state IN ({}) AND finished_at IS NOT NULL AND finished_at >= ? AND finished_at < ? \
             ORDER BY finished_at, rowid LIMIT ?",
            marks(states.len())
        );
        let mut q = sqlx::query_as::<_, ReceiptTask>(&sql).bind(space.as_str());
        for s in states {
            q = q.bind(*s);
        }
        Ok(q.bind(from).bind(to).bind(limit).fetch_all(&self.pool).await?)
    }

    /// Who verified each task, its checks and evidence, and the names of the bots and missions
    /// the rows mention: five queries per [`PAGE`] tasks.
    pub async fn receipt_parts(&self, tasks: &[ReceiptTask]) -> Result<ReceiptParts, DbError> {
        let mut out = ReceiptParts::default();
        let mut bots: Vec<String> = Vec::new();
        let mut missions: Vec<String> = Vec::new();
        for page in tasks.chunks(PAGE) {
            let ids: Vec<&str> = page.iter().map(|t| t.id.as_str()).collect();
            let m = marks(ids.len());

            let sql = format!("SELECT task_id, actor FROM task_events WHERE task_id IN ({m}) AND to_state = 'verified' ORDER BY id");
            let mut q = sqlx::query_as::<_, (String, String)>(&sql);
            for id in &ids {
                q = q.bind(*id);
            }
            for (task, actor) in q.fetch_all(&self.pool).await? {
                out.verified_by.insert(task, actor);
            }

            let sql = format!("SELECT task_id, kind, status FROM outcome_checks WHERE task_id IN ({m}) ORDER BY created_at, rowid");
            let mut q = sqlx::query_as::<_, (String, String, String)>(&sql);
            for id in &ids {
                q = q.bind(*id);
            }
            for (task, kind, status) in q.fetch_all(&self.pool).await? {
                out.checks.entry(task).or_default().push((kind, status));
            }

            let sql = format!("SELECT task_id, label, uri, path, sha256 FROM evidence WHERE task_id IN ({m}) ORDER BY captured_at, rowid");
            let mut q = sqlx::query_as::<_, ReceiptEvidence>(&sql);
            for id in &ids {
                q = q.bind(*id);
            }
            for e in q.fetch_all(&self.pool).await? {
                out.evidence.entry(e.task_id.clone()).or_default().push(e);
            }

            bots.extend(page.iter().filter_map(|t| t.owner_kwant_id.clone()));
            missions.extend(page.iter().filter_map(|t| t.mission_id.clone()));
        }
        bots.extend(out.verified_by.values().filter(|a| !matches!(a.as_str(), "person" | "kwant")).cloned());
        bots.sort();
        bots.dedup();
        missions.sort();
        missions.dedup();
        for page in bots.chunks(PAGE) {
            let sql = format!("SELECT id, name FROM kwants WHERE id IN ({})", marks(page.len()));
            let mut q = sqlx::query_as::<_, (String, String)>(&sql);
            for id in page {
                q = q.bind(id);
            }
            out.bot_names.extend(q.fetch_all(&self.pool).await?);
        }
        for page in missions.chunks(PAGE) {
            let sql = format!("SELECT id, title FROM missions WHERE id IN ({})", marks(page.len()));
            let mut q = sqlx::query_as::<_, (String, String)>(&sql);
            for id in page {
                q = q.bind(id);
            }
            out.mission_titles.extend(q.fetch_all(&self.pool).await?);
        }
        Ok(out)
    }
}
