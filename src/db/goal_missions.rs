//! Goal remediation (docs/mission-control.md §22): the missions a goal opened to restore itself
//! (`goal_mission_links`), the notices sent about it (for the 3-per-day cap) and the re-checks
//! asked after a fix (for the conflict rule). Add-only, like [`super::goals`]: new tables, nothing
//! changed in an existing one, and running the migration twice changes nothing. No foreign keys,
//! so [`Database::delete_goal_remediation`] removes a goal's rows itself. Times are milliseconds.

use super::{Database, DbError};
use serde::Serialize;
use sqlx::FromRow;

const TABLES: &[&str] = &[
    "CREATE TABLE IF NOT EXISTS goal_mission_links (
        id INTEGER PRIMARY KEY AUTOINCREMENT,
        goal_id TEXT NOT NULL,
        mission_id TEXT NOT NULL,
        investigate_task_id TEXT NOT NULL DEFAULT '',
        fix_task_id TEXT NOT NULL DEFAULT '',
        reason TEXT NOT NULL DEFAULT '',
        attempt INTEGER NOT NULL DEFAULT 1,
        incident_at INTEGER NOT NULL,
        opened_at INTEGER NOT NULL,
        closed_at INTEGER,
        outcome TEXT,
        declined_at INTEGER
    )",
    // At most one open remediation per goal, enforced here as well as in the opener
    "CREATE UNIQUE INDEX IF NOT EXISTS idx_goal_mission_links_open ON goal_mission_links(goal_id) WHERE closed_at IS NULL",
    "CREATE INDEX IF NOT EXISTS idx_goal_mission_links_goal ON goal_mission_links(goal_id, id)",
    "CREATE INDEX IF NOT EXISTS idx_goal_mission_links_mission ON goal_mission_links(mission_id)",
    "CREATE TABLE IF NOT EXISTS goal_rechecks (
        goal_id TEXT NOT NULL,
        after_goal_id TEXT NOT NULL,
        created_at INTEGER NOT NULL,
        PRIMARY KEY (goal_id, after_goal_id)
    )",
    // The goal "Keep this true" made from a task: one per task, so a repeat finds it
    "CREATE TABLE IF NOT EXISTS goal_from_tasks (
        task_id TEXT PRIMARY KEY,
        goal_id TEXT NOT NULL,
        created_at INTEGER NOT NULL
    )",
];

/// One remediation attempt of one goal. Goals that break together share one mission: each has its
/// own link to it.
#[derive(Debug, Clone, PartialEq, Eq, FromRow, Serialize)]
pub struct GoalLink {
    pub id: i64,
    pub goal_id: String,
    pub mission_id: String,
    pub investigate_task_id: String,
    pub fix_task_id: String,
    /// Why it opened, in plain words ("a check failed", "the last attempt failed").
    pub reason: String,
    pub attempt: i64,
    /// When the incident this attempt belongs to started (see `goals_remediation::incident_start`).
    pub incident_at: i64,
    pub opened_at: i64,
    pub closed_at: Option<i64>,
    /// `verified`, `failed`, `cancelled`, `not_needed`, `unmet` (the fix passed its checks but the
    /// goal still wasn't met) or `joined_closed`; None while open.
    pub outcome: Option<String>,
    /// When the person chose Not now for this incident; the held fix stays, nothing asks again.
    pub declined_at: Option<i64>,
}

/// A task a bot finished (verified) whose "Keep this true" goal later stopped holding: work marked
/// done that didn't hold. Times are milliseconds.
#[derive(Debug, Clone, PartialEq, Eq, FromRow, Serialize)]
pub struct GoalLapse {
    pub goal_id: String,
    pub goal_title: String,
    pub task_id: String,
    pub task_title: String,
    pub task_created_by: String,
    pub task_finished_at: i64,
    /// When the goal was confirmed not met (the newest time, if it lapsed more than once).
    pub lapsed_at: i64,
    /// The kind of the required check that failed, when kwant still has that observation.
    pub check_kind: Option<String>,
}

#[derive(Debug, Clone)]
pub struct NewLink {
    pub goal_id: String,
    pub mission_id: String,
    pub investigate_task_id: String,
    pub fix_task_id: String,
    pub reason: String,
    pub attempt: i64,
    pub incident_at: i64,
}

const LINK_SELECT: &str = "SELECT id, goal_id, mission_id, investigate_task_id, fix_task_id, reason, attempt, incident_at, opened_at, closed_at, outcome, declined_at FROM goal_mission_links";

impl Database {
    pub(super) async fn migrate_goal_missions(&self) -> Result<(), DbError> {
        for stmt in TABLES {
            sqlx::query(stmt).execute(&self.pool).await?;
        }
        Ok(())
    }

    /// Records a new open remediation. None (nothing written) when the goal already has one open.
    pub async fn open_goal_link(&self, l: &NewLink, now: i64) -> Result<Option<GoalLink>, DbError> {
        let mut tx = self.begin_write().await?;
        let open: Option<(i64,)> = sqlx::query_as("SELECT 1 FROM goal_mission_links WHERE goal_id = ? AND closed_at IS NULL").bind(&l.goal_id).fetch_optional(&mut *tx).await?;
        if open.is_some() {
            tx.rollback().await?;
            return Ok(None);
        }
        let r = sqlx::query(
            "INSERT INTO goal_mission_links (goal_id, mission_id, investigate_task_id, fix_task_id, reason, attempt, incident_at, opened_at)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(&l.goal_id)
        .bind(&l.mission_id)
        .bind(&l.investigate_task_id)
        .bind(&l.fix_task_id)
        .bind(&l.reason)
        .bind(l.attempt)
        .bind(l.incident_at)
        .bind(now)
        .execute(&mut *tx)
        .await?;
        let link = sqlx::query_as::<_, GoalLink>(&format!("{LINK_SELECT} WHERE id = ?")).bind(r.last_insert_rowid()).fetch_one(&mut *tx).await?;
        tx.commit().await?;
        Ok(Some(link))
    }

    /// The goal's open remediation, if any.
    pub async fn open_goal_link_of(&self, goal_id: &str) -> Result<Option<GoalLink>, DbError> {
        Ok(sqlx::query_as(&format!("{LINK_SELECT} WHERE goal_id = ? AND closed_at IS NULL")).bind(goal_id).fetch_optional(&self.pool).await?)
    }

    /// The goal's remediations, newest first.
    pub async fn goal_links(&self, goal_id: &str, limit: i64) -> Result<Vec<GoalLink>, DbError> {
        Ok(sqlx::query_as(&format!("{LINK_SELECT} WHERE goal_id = ? ORDER BY id DESC LIMIT ?")).bind(goal_id).bind(limit).fetch_all(&self.pool).await?)
    }

    /// Every goal's link to the mission (open or closed), oldest first.
    pub async fn links_of_mission(&self, mission_id: &str) -> Result<Vec<GoalLink>, DbError> {
        Ok(sqlx::query_as(&format!("{LINK_SELECT} WHERE mission_id = ? ORDER BY id")).bind(mission_id).fetch_all(&self.pool).await?)
    }

    /// Open remediations with the goal's `(space_id, folder, priority)`, for the whole server.
    pub async fn open_goal_links(&self) -> Result<Vec<(GoalLink, String, String, i64)>, DbError> {
        let rows = sqlx::query_as::<_, OpenRow>(
            "SELECT l.id, l.goal_id, l.mission_id, l.investigate_task_id, l.fix_task_id, l.reason, l.attempt, l.incident_at, l.opened_at,
             l.closed_at, l.outcome, l.declined_at, g.space_id AS g_space, g.folder AS g_folder, g.priority AS g_priority
             FROM goal_mission_links l JOIN goals g ON g.id = l.goal_id WHERE l.closed_at IS NULL ORDER BY l.id",
        )
        .fetch_all(&self.pool)
        .await?;
        Ok(rows.into_iter().map(OpenRow::split).collect())
    }

    /// Closes an open link with `outcome`; false if it was already closed.
    pub async fn close_goal_link(&self, id: i64, outcome: &str, now: i64) -> Result<bool, DbError> {
        let r = sqlx::query("UPDATE goal_mission_links SET closed_at = ?, outcome = ? WHERE id = ? AND closed_at IS NULL").bind(now).bind(outcome).bind(id).execute(&self.pool).await?;
        Ok(r.rows_affected() == 1)
    }

    /// Attempts opened for the goal's incident (links opened at or after `since`).
    pub async fn attempts_since(&self, goal_id: &str, since: i64) -> Result<i64, DbError> {
        let (n,): (i64,) = sqlx::query_as("SELECT COUNT(*) FROM goal_mission_links WHERE goal_id = ? AND opened_at >= ?").bind(goal_id).bind(since).fetch_one(&self.pool).await?;
        Ok(n)
    }

    /// When the goal's newest remediation closed (any incident).
    pub async fn last_link_closed_at(&self, goal_id: &str) -> Result<Option<i64>, DbError> {
        let row: Option<(Option<i64>,)> = sqlx::query_as("SELECT MAX(closed_at) FROM goal_mission_links WHERE goal_id = ?").bind(goal_id).fetch_optional(&self.pool).await?;
        Ok(row.and_then(|r| r.0))
    }

    /// Tokens the goal's remediation missions' tasks spent, over all attempts. Joined goals count
    /// the shared mission too, so a goal's budget covers what it asked for.
    pub async fn goal_spent_tokens(&self, goal_id: &str) -> Result<i64, DbError> {
        let (n,): (i64,) = sqlx::query_as(
            "SELECT COALESCE(SUM(t.spent_tokens), 0) FROM tasks t WHERE t.mission_id IN (SELECT mission_id FROM goal_mission_links WHERE goal_id = ?)",
        )
        .bind(goal_id)
        .fetch_one(&self.pool)
        .await?;
        Ok(n)
    }

    /// Clears a Not now (the person ran the fix after all, or chose Always fix).
    pub async fn clear_goal_decline(&self, id: i64) -> Result<bool, DbError> {
        let r = sqlx::query("UPDATE goal_mission_links SET declined_at = NULL WHERE id = ? AND declined_at IS NOT NULL").bind(id).execute(&self.pool).await?;
        Ok(r.rows_affected() == 1)
    }

    /// Records that the person chose Not now for the goal's open remediation.
    pub async fn decline_goal_link(&self, id: i64, now: i64) -> Result<bool, DbError> {
        let r = sqlx::query("UPDATE goal_mission_links SET declined_at = ? WHERE id = ? AND closed_at IS NULL AND declined_at IS NULL").bind(now).bind(id).execute(&self.pool).await?;
        Ok(r.rows_affected() == 1)
    }

    /// Missions whose every remediation closed `verified` or `not_needed`: off the board.
    pub async fn archived_mission_ids(&self) -> Result<Vec<String>, DbError> {
        let rows: Vec<(String,)> = sqlx::query_as(
            "SELECT DISTINCT mission_id FROM goal_mission_links WHERE outcome IN ('verified', 'not_needed')
             AND mission_id NOT IN (SELECT mission_id FROM goal_mission_links WHERE closed_at IS NULL OR outcome NOT IN ('verified', 'not_needed'))",
        )
        .fetch_all(&self.pool)
        .await?;
        Ok(rows.into_iter().map(|r| r.0).collect())
    }

    /// The first goal whose remediation a mission is, with its title: a mission's `goal_link`.
    pub async fn goal_link_of_mission(&self, mission_id: &str) -> Result<Option<(String, String)>, DbError> {
        Ok(sqlx::query_as("SELECT g.id, g.title FROM goal_mission_links l JOIN goals g ON g.id = l.goal_id WHERE l.mission_id = ? ORDER BY l.id LIMIT 1").bind(mission_id).fetch_optional(&self.pool).await?)
    }

    /// The goal "Keep this true" made from the task, if any.
    pub async fn goal_from_task(&self, task_id: &str) -> Result<Option<String>, DbError> {
        let row: Option<(String,)> = sqlx::query_as("SELECT goal_id FROM goal_from_tasks WHERE task_id = ?").bind(task_id).fetch_optional(&self.pool).await?;
        Ok(row.map(|r| r.0))
    }

    /// The goals made from tasks `kwant_id` owned and finished verified that were later confirmed
    /// not met: at `since_ms` or after, and after the task finished. Confirmed means the loop's own
    /// `not_met` event (the goal's fail streak reached its confirm count), so a goal that only went
    /// unknown, or held a blip, is never a lapse. One row per goal, its newest lapse; newest first,
    /// at most `limit`. Deleted goals have no rows left, and a draft or paused goal is left out.
    pub async fn goal_lapses_of_bot(&self, kwant_id: &str, since_ms: i64, limit: i64) -> Result<Vec<GoalLapse>, DbError> {
        // tasks.finished_at is a TIMESTAMP (UTC text): whole seconds plus the fraction, in milliseconds
        const FINISHED_MS: &str = "(CAST(strftime('%s', t.finished_at) AS INTEGER) * 1000
            + CAST(ROUND((CAST(strftime('%f', t.finished_at) AS REAL) - CAST(strftime('%S', t.finished_at) AS INTEGER)) * 1000) AS INTEGER))";
        let sql = format!(
            "SELECT g.id AS goal_id, g.title AS goal_title, t.id AS task_id, t.title AS task_title, t.created_by AS task_created_by,
                    {FINISHED_MS} AS task_finished_at, e.created_at AS lapsed_at,
                    (SELECT c.kind FROM goal_observations o JOIN goal_checks c ON c.id = o.check_id
                      WHERE o.goal_id = g.id AND o.verdict = 'not_met' AND c.severity = 'required' AND o.observed_at <= e.created_at
                      ORDER BY o.id DESC LIMIT 1) AS check_kind
             FROM goal_from_tasks f
             JOIN tasks t ON t.id = f.task_id
             JOIN goals g ON g.id = f.goal_id
             JOIN goal_events e ON e.id = (
                 SELECT MAX(x.id) FROM goal_events x
                 WHERE x.goal_id = g.id AND x.kind = 'not_met' AND x.created_at >= ?2 AND x.created_at > {FINISHED_MS})
             WHERE t.owner_kwant_id = ?1 AND t.state = 'verified' AND t.finished_at IS NOT NULL
               AND g.paused_at IS NULL AND g.status NOT IN ('draft', 'paused')
             ORDER BY e.created_at DESC, e.id DESC LIMIT ?3"
        );
        Ok(sqlx::query_as::<_, GoalLapse>(&sql).bind(kwant_id).bind(since_ms).bind(limit.max(0)).fetch_all(&self.pool).await?)
    }

    /// Remembers the goal made from a task. False if the task already had one (nothing changed).
    pub async fn set_goal_from_task(&self, task_id: &str, goal_id: &str, now: i64) -> Result<bool, DbError> {
        let r = sqlx::query("INSERT OR IGNORE INTO goal_from_tasks (task_id, goal_id, created_at) VALUES (?, ?, ?)").bind(task_id).bind(goal_id).bind(now).execute(&self.pool).await?;
        Ok(r.rows_affected() == 1)
    }

    /// Active "Restore: …" missions kwant made that no goal link points at, older than `before`
    /// (milliseconds): left by a crash between the plan and the link.
    pub async fn unlinked_restore_missions(&self, before: i64) -> Result<Vec<super::missions::Mission>, DbError> {
        Ok(sqlx::query_as::<_, super::missions::Mission>(
            "SELECT * FROM missions WHERE created_by = 'kwant' AND status = 'active' AND title LIKE 'Restore: %'
             AND (CAST(strftime('%s', created_at) AS INTEGER) * 1000) < ? AND id NOT IN (SELECT mission_id FROM goal_mission_links)",
        )
        .bind(before)
        .fetch_all(&self.pool)
        .await?)
    }

    /// Whether the goal was told something of one of `kinds` since `since`, by a notice that went
    /// out or one the daily cap held back (a held-back notice is settled: it never goes out late).
    /// Reads goal-loop events, the one place notices are recorded.
    pub async fn goal_told_since(&self, goal_id: &str, kinds: &[&str], since: i64) -> Result<bool, DbError> {
        let marks = vec!["?"; kinds.len()].join(", ");
        let sql = format!(
            "SELECT 1 FROM goal_events WHERE goal_id = ? AND created_at >= ? AND kind IN ('notified', 'notice_capped') AND json_extract(detail, '$.kind') IN ({marks}) LIMIT 1"
        );
        let mut q = sqlx::query_as::<_, (i64,)>(&sql).bind(goal_id).bind(since);
        for k in kinds {
            q = q.bind(*k);
        }
        Ok(q.fetch_optional(&self.pool).await?.is_some())
    }

    /// Asks that `goal_id` be looked at after `after_goal_id`'s fix was verified: if it is then
    /// not met, that is a conflict, not a new incident.
    pub async fn add_goal_recheck(&self, goal_id: &str, after_goal_id: &str, now: i64) -> Result<(), DbError> {
        sqlx::query("INSERT OR REPLACE INTO goal_rechecks (goal_id, after_goal_id, created_at) VALUES (?, ?, ?)").bind(goal_id).bind(after_goal_id).bind(now).execute(&self.pool).await?;
        Ok(())
    }

    /// The goal whose fix the goal is waiting to be re-checked after, if asked since `since`.
    pub async fn goal_recheck_after(&self, goal_id: &str, since: i64) -> Result<Option<String>, DbError> {
        let row: Option<(String,)> = sqlx::query_as("SELECT after_goal_id FROM goal_rechecks WHERE goal_id = ? AND created_at >= ? ORDER BY created_at DESC LIMIT 1").bind(goal_id).bind(since).fetch_optional(&self.pool).await?;
        Ok(row.map(|r| r.0))
    }

    pub async fn clear_goal_rechecks(&self, goal_id: &str) -> Result<(), DbError> {
        sqlx::query("DELETE FROM goal_rechecks WHERE goal_id = ?").bind(goal_id).execute(&self.pool).await?;
        Ok(())
    }

    /// Every re-check asked and not yet settled: `(goal_id, after_goal_id, created_at)`.
    pub async fn pending_rechecks(&self) -> Result<Vec<(String, String, i64)>, DbError> {
        Ok(sqlx::query_as("SELECT goal_id, after_goal_id, created_at FROM goal_rechecks ORDER BY created_at, goal_id").fetch_all(&self.pool).await?)
    }

    pub async fn delete_goal_recheck(&self, goal_id: &str, after_goal_id: &str) -> Result<(), DbError> {
        sqlx::query("DELETE FROM goal_rechecks WHERE goal_id = ? AND after_goal_id = ?").bind(goal_id).bind(after_goal_id).execute(&self.pool).await?;
        Ok(())
    }

    /// When the goal's current incident started: the first `not_met` event after its newest `met`
    /// one. None if it never logged a `not_met`.
    pub async fn goal_incident_start(&self, goal_id: &str) -> Result<Option<i64>, DbError> {
        let row: Option<(Option<i64>,)> = sqlx::query_as(
            "SELECT MIN(created_at) FROM goal_events WHERE goal_id = ? AND kind = 'not_met'
             AND id > COALESCE((SELECT MAX(id) FROM goal_events WHERE goal_id = ? AND kind = 'met'), 0)",
        )
        .bind(goal_id)
        .bind(goal_id)
        .fetch_optional(&self.pool)
        .await?;
        Ok(row.and_then(|r| r.0))
    }

    /// The kind of the goal's newest event among `kinds`.
    pub async fn last_goal_event_of(&self, goal_id: &str, kinds: &[&str]) -> Result<Option<String>, DbError> {
        let marks = vec!["?"; kinds.len()].join(", ");
        let sql = format!("SELECT kind FROM goal_events WHERE goal_id = ? AND kind IN ({marks}) ORDER BY id DESC LIMIT 1");
        let mut q = sqlx::query_as::<_, (String,)>(&sql).bind(goal_id);
        for k in kinds {
            q = q.bind(*k);
        }
        Ok(q.fetch_optional(&self.pool).await?.map(|r| r.0))
    }

    /// Whether the goal has an event of `kind` whose detail contains `needle` (to log a thing once).
    pub async fn goal_has_event(&self, goal_id: &str, kind: &str, needle: &str) -> Result<bool, DbError> {
        let row: Option<(i64,)> = sqlx::query_as("SELECT 1 FROM goal_events WHERE goal_id = ? AND kind = ? AND instr(detail, ?) > 0 LIMIT 1")
            .bind(goal_id)
            .bind(kind)
            .bind(needle)
            .fetch_optional(&self.pool)
            .await?;
        Ok(row.is_some())
    }

    /// Removes a goal's remediation rows (its missions stay on the board). Called with the goal's
    /// own delete.
    pub async fn delete_goal_remediation(&self, goal_id: &str) -> Result<(), DbError> {
        for table in ["goal_mission_links", "goal_from_tasks"] {
            sqlx::query(&format!("DELETE FROM {table} WHERE goal_id = ?")).bind(goal_id).execute(&self.pool).await?;
        }
        sqlx::query("DELETE FROM goal_rechecks WHERE goal_id = ? OR after_goal_id = ?").bind(goal_id).bind(goal_id).execute(&self.pool).await?;
        Ok(())
    }
}

#[derive(FromRow)]
struct OpenRow {
    id: i64,
    goal_id: String,
    mission_id: String,
    investigate_task_id: String,
    fix_task_id: String,
    reason: String,
    attempt: i64,
    incident_at: i64,
    opened_at: i64,
    closed_at: Option<i64>,
    outcome: Option<String>,
    declined_at: Option<i64>,
    g_space: String,
    g_folder: String,
    g_priority: i64,
}

impl OpenRow {
    fn split(self) -> (GoalLink, String, String, i64) {
        let link = GoalLink {
            id: self.id,
            goal_id: self.goal_id,
            mission_id: self.mission_id,
            investigate_task_id: self.investigate_task_id,
            fix_task_id: self.fix_task_id,
            reason: self.reason,
            attempt: self.attempt,
            incident_at: self.incident_at,
            opened_at: self.opened_at,
            closed_at: self.closed_at,
            outcome: self.outcome,
            declined_at: self.declined_at,
        };
        (link, self.g_space, self.g_folder, self.g_priority)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn db() -> (Database, tempfile::TempDir) {
        let tmp = tempfile::tempdir().unwrap();
        (Database::init(tmp.path()).await.unwrap(), tmp)
    }

    fn link(goal: &str, mission: &str, attempt: i64) -> NewLink {
        NewLink {
            goal_id: goal.into(),
            mission_id: mission.into(),
            investigate_task_id: format!("{mission}-i"),
            fix_task_id: format!("{mission}-f"),
            reason: "a check failed".into(),
            attempt,
            incident_at: 100,
        }
    }

    #[tokio::test]
    async fn migration_twice_changes_nothing() {
        let (db, _t) = db().await;
        db.open_goal_link(&link("g1", "m1", 1), 1_000).await.unwrap().unwrap();
        db.migrate_goal_missions().await.unwrap();
        db.migrate_goal_missions().await.unwrap();
        assert_eq!(db.goal_links("g1", 10).await.unwrap().len(), 1);
    }

    #[tokio::test]
    async fn a_goal_has_one_open_remediation_at_a_time() {
        let (db, _t) = db().await;
        let first = db.open_goal_link(&link("g1", "m1", 1), 1_000).await.unwrap().unwrap();
        assert!(db.open_goal_link(&link("g1", "m2", 2), 2_000).await.unwrap().is_none(), "a second open link is refused");
        // The index itself refuses it too, even past the opener's check
        let raw = sqlx::query("INSERT INTO goal_mission_links (goal_id, mission_id, incident_at, opened_at) VALUES ('g1', 'm3', 1, 1)").execute(&db.pool).await;
        assert!(raw.is_err());
        assert!(db.close_goal_link(first.id, "failed", 3_000).await.unwrap());
        assert!(!db.close_goal_link(first.id, "failed", 4_000).await.unwrap(), "closing twice changes nothing");
        let second = db.open_goal_link(&link("g1", "m2", 2), 5_000).await.unwrap().unwrap();
        assert_eq!(db.open_goal_link_of("g1").await.unwrap().unwrap().id, second.id);
        let all = db.goal_links("g1", 10).await.unwrap();
        assert_eq!(all.iter().map(|l| l.attempt).collect::<Vec<_>>(), vec![2, 1]);
        assert_eq!(all[1].outcome.as_deref(), Some("failed"));
        assert_eq!(all[1].closed_at, Some(3_000));
    }

    #[tokio::test]
    async fn attempts_and_last_close_are_per_goal_and_incident() {
        let (db, _t) = db().await;
        let a = db.open_goal_link(&link("g1", "m1", 1), 1_000).await.unwrap().unwrap();
        db.close_goal_link(a.id, "failed", 2_000).await.unwrap();
        db.open_goal_link(&link("g1", "m2", 2), 3_000).await.unwrap().unwrap();
        db.open_goal_link(&link("g2", "m3", 1), 3_000).await.unwrap().unwrap();
        assert_eq!(db.attempts_since("g1", 0).await.unwrap(), 2);
        assert_eq!(db.attempts_since("g1", 2_500).await.unwrap(), 1, "an older incident's attempts don't count");
        assert_eq!(db.attempts_since("g2", 0).await.unwrap(), 1);
        assert_eq!(db.last_link_closed_at("g1").await.unwrap(), Some(2_000));
        assert_eq!(db.last_link_closed_at("g2").await.unwrap(), None);
        assert_eq!(db.last_link_closed_at("nobody").await.unwrap(), None);
    }

    #[tokio::test]
    async fn told_reads_sent_and_held_back_notices() {
        let (db, _t) = db().await;
        db.add_goal_event("g1", "notified", "kwant", &serde_json::json!({"kind": "broken"})).await.unwrap();
        db.add_goal_event("g1", "notice_capped", "kwant", &serde_json::json!({"kind": "needs_you"})).await.unwrap();
        assert!(db.goal_told_since("g1", &["broken"], 0).await.unwrap());
        assert!(db.goal_told_since("g1", &["decision", "needs_you"], 0).await.unwrap(), "a held-back notice is settled too");
        assert!(!db.goal_told_since("g1", &["recovered"], 0).await.unwrap());
        assert!(!db.goal_told_since("g2", &["broken"], 0).await.unwrap());
        assert!(!db.goal_told_since("g1", &["broken"], i64::MAX).await.unwrap(), "an old notice is out of the window");
    }

    #[tokio::test]
    async fn rechecks_are_asked_cleared_and_removed_with_the_goal() {
        let (db, _t) = db().await;
        db.add_goal_recheck("b", "a", 500).await.unwrap();
        assert_eq!(db.goal_recheck_after("b", 400).await.unwrap().as_deref(), Some("a"));
        assert_eq!(db.goal_recheck_after("b", 600).await.unwrap(), None, "an old ask is out of date");
        db.clear_goal_rechecks("b").await.unwrap();
        assert_eq!(db.goal_recheck_after("b", 0).await.unwrap(), None);
        db.add_goal_recheck("b", "a", 500).await.unwrap();
        db.open_goal_link(&link("a", "m1", 1), 1).await.unwrap().unwrap();
        db.delete_goal_remediation("a").await.unwrap();
        assert_eq!(db.goal_recheck_after("b", 0).await.unwrap(), None);
        assert_eq!(db.goal_links("a", 10).await.unwrap().len(), 0);
    }
}
