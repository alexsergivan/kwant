//! Mission Control Phase D2: the Swarm Auditor's records. One `task_audits` row per audited task
//! attempt (a teammate's challenge and the owner's rebuttal) and its `audit_risks`. An audit never
//! changes a task's state or checks: only outcome checks verify. See docs/mission-control.md §18.

use super::{Database, DbError};
use chrono::{DateTime, Utc};
use serde::Serialize;
use serde_json::Value;
use sqlx::FromRow;

pub const STATUSES: [&str; 7] = ["queued", "challenge", "rebuttal", "done", "skipped", "failed", "interrupted"];
/// Risk statuses; `open` and `disputed` are unresolved.
pub const RISK_STATUSES: [&str; 4] = ["open", "disputed", "resolved", "dismissed"];

#[derive(Debug, Clone, Serialize, FromRow, PartialEq)]
pub struct Audit {
    pub id: String,
    pub task_id: String,
    pub space_id: String,
    pub attempt: i64,
    /// `high_impact`, `low_confidence` or `manual`.
    pub trigger: String,
    pub challenger_kwant_id: String,
    pub owner_kwant_id: String,
    pub status: String,
    pub reason: Option<String>,
    pub challenge_tokens: i64,
    pub rebuttal_tokens: i64,
    pub created_at: DateTime<Utc>,
    pub finished_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Serialize, FromRow, PartialEq)]
pub struct Risk {
    pub id: String,
    pub audit_id: String,
    pub task_id: String,
    pub seq: i64,
    pub risk: String,
    pub evidence: String,
    /// JSON array of evidence ids on the task.
    #[serde(skip)]
    pub evidence_ids: String,
    /// The owner's stance: `disputed` or `accepted`; `None` when it didn't answer.
    pub stance: Option<String>,
    pub answer: Option<String>,
    pub answer_evidence: Option<String>,
    pub status: String,
    pub resolved_by: Option<String>,
    pub resolved_at: Option<DateTime<Utc>>,
    pub note: Option<String>,
    pub created_at: DateTime<Utc>,
}

impl Risk {
    pub fn evidence_ids(&self) -> Vec<String> {
        serde_json::from_str(&self.evidence_ids).unwrap_or_default()
    }

    pub fn unresolved(&self) -> bool {
        matches!(self.status.as_str(), "open" | "disputed")
    }
}

/// A challenge's risk before it's stored.
#[derive(Debug, Clone, PartialEq)]
pub struct NewRisk {
    pub risk: String,
    pub evidence: String,
    pub evidence_ids: Vec<String>,
}

/// The owner's answer to one risk.
#[derive(Debug, Clone, PartialEq)]
pub struct Answer {
    pub risk_id: String,
    pub stance: String,
    pub answer: String,
    pub evidence: Option<String>,
}

fn in_list(list: &[&str]) -> String {
    list.iter().map(|s| format!("'{s}'")).collect::<Vec<_>>().join(", ")
}

impl Database {
    pub(super) async fn migrate_auditor(&self) -> Result<(), DbError> {
        let audits = format!(
            "CREATE TABLE IF NOT EXISTS task_audits (
                id TEXT PRIMARY KEY,
                task_id TEXT NOT NULL,
                space_id TEXT NOT NULL,
                attempt INTEGER NOT NULL,
                trigger TEXT NOT NULL CHECK (trigger IN ('high_impact', 'low_confidence', 'manual')),
                challenger_kwant_id TEXT NOT NULL,
                owner_kwant_id TEXT NOT NULL CHECK (owner_kwant_id <> challenger_kwant_id),
                status TEXT NOT NULL CHECK (status IN ({})),
                reason TEXT,
                challenge_tokens INTEGER NOT NULL DEFAULT 0,
                rebuttal_tokens INTEGER NOT NULL DEFAULT 0,
                created_at TIMESTAMP NOT NULL,
                finished_at TIMESTAMP,
                UNIQUE (task_id, attempt)
            )",
            in_list(&STATUSES)
        );
        let risks = format!(
            "CREATE TABLE IF NOT EXISTS audit_risks (
                id TEXT PRIMARY KEY,
                audit_id TEXT NOT NULL REFERENCES task_audits(id),
                task_id TEXT NOT NULL,
                seq INTEGER NOT NULL,
                risk TEXT NOT NULL,
                evidence TEXT NOT NULL CHECK (length(trim(evidence)) > 0),
                evidence_ids TEXT NOT NULL DEFAULT '[]',
                stance TEXT CHECK (stance IS NULL OR stance IN ('disputed', 'accepted')),
                answer TEXT,
                answer_evidence TEXT,
                status TEXT NOT NULL CHECK (status IN ({})),
                resolved_by TEXT,
                resolved_at TIMESTAMP,
                note TEXT,
                created_at TIMESTAMP NOT NULL,
                UNIQUE (audit_id, seq)
            )",
            in_list(&RISK_STATUSES)
        );
        for stmt in [
            audits.as_str(),
            "CREATE INDEX IF NOT EXISTS task_audits_task ON task_audits (task_id)",
            "CREATE INDEX IF NOT EXISTS task_audits_created ON task_audits (created_at)",
            risks.as_str(),
            "CREATE INDEX IF NOT EXISTS audit_risks_task ON audit_risks (task_id, status)",
        ] {
            sqlx::query(stmt).execute(&self.pool).await?;
        }
        // Audits a restart cut off never finish: say so instead of leaving them running
        sqlx::query(
            "UPDATE task_audits SET status = 'interrupted', reason = 'kwant restarted during the audit', finished_at = ? \
             WHERE status IN ('queued', 'challenge', 'rebuttal')",
        )
        .bind(Utc::now())
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// Opens an audit of `task_id`'s `attempt` in `queued`. `None` when that attempt has one.
    #[allow(clippy::too_many_arguments)]
    pub async fn create_audit(
        &self,
        task_id: &str,
        space_id: &str,
        attempt: i64,
        trigger: &str,
        challenger: &str,
        owner: &str,
    ) -> Result<Option<Audit>, DbError> {
        let id = uuid::Uuid::new_v4().to_string();
        let row = sqlx::query_as::<_, Audit>(
            "INSERT INTO task_audits (id, task_id, space_id, attempt, trigger, challenger_kwant_id, owner_kwant_id, status, created_at) \
             VALUES (?, ?, ?, ?, ?, ?, ?, 'queued', ?) ON CONFLICT(task_id, attempt) DO NOTHING RETURNING *",
        )
        .bind(&id)
        .bind(task_id)
        .bind(space_id)
        .bind(attempt)
        .bind(trigger)
        .bind(challenger)
        .bind(owner)
        .bind(Utc::now())
        .fetch_optional(&self.pool)
        .await?;
        Ok(row)
    }

    pub async fn get_audit(&self, id: &str) -> Result<Option<Audit>, DbError> {
        Ok(sqlx::query_as::<_, Audit>("SELECT * FROM task_audits WHERE id = ?").bind(id).fetch_optional(&self.pool).await?)
    }

    pub async fn audits_of(&self, task_id: &str) -> Result<Vec<Audit>, DbError> {
        Ok(sqlx::query_as::<_, Audit>("SELECT * FROM task_audits WHERE task_id = ? ORDER BY created_at, rowid").bind(task_id).fetch_all(&self.pool).await?)
    }

    pub async fn audit_of_attempt(&self, task_id: &str, attempt: i64) -> Result<Option<Audit>, DbError> {
        Ok(sqlx::query_as::<_, Audit>("SELECT * FROM task_audits WHERE task_id = ? AND attempt = ?")
            .bind(task_id)
            .bind(attempt)
            .fetch_optional(&self.pool)
            .await?)
    }

    /// Audits created since `since` (skipped ones included: they were started).
    pub async fn audits_since(&self, since: DateTime<Utc>) -> Result<i64, DbError> {
        let (n,): (i64,) = sqlx::query_as("SELECT COUNT(*) FROM task_audits WHERE created_at >= ?").bind(since).fetch_one(&self.pool).await?;
        Ok(n)
    }

    /// How many audits each bot challenged since `since`.
    pub async fn challenges_since(&self, since: DateTime<Utc>) -> Result<Vec<(String, i64)>, DbError> {
        Ok(sqlx::query_as("SELECT challenger_kwant_id, COUNT(*) FROM task_audits WHERE created_at >= ? GROUP BY challenger_kwant_id")
            .bind(since)
            .fetch_all(&self.pool)
            .await?)
    }

    /// Verified observations per bot in `class` (Performance Market, report data only).
    pub async fn verified_in_class(&self, space_id: &str, class: &str) -> Result<Vec<(String, i64)>, DbError> {
        Ok(sqlx::query_as(
            "SELECT kwant_id, COUNT(*) FROM performance_observations WHERE space_id = ? AND task_class = ? AND result = 'verified' GROUP BY kwant_id",
        )
        .bind(space_id)
        .bind(class)
        .fetch_all(&self.pool)
        .await?)
    }

    /// Moves an audit from `from` to `to`; false when it wasn't in `from` any more. A final status
    /// sets `finished_at`.
    pub async fn move_audit(&self, id: &str, from: &str, to: &str, reason: Option<&str>) -> Result<bool, DbError> {
        let finished = matches!(to, "done" | "skipped" | "failed" | "interrupted").then(Utc::now);
        let r = sqlx::query("UPDATE task_audits SET status = ?, reason = COALESCE(?, reason), finished_at = COALESCE(?, finished_at) WHERE id = ? AND status = ?")
            .bind(to)
            .bind(reason)
            .bind(finished)
            .bind(id)
            .bind(from)
            .execute(&self.pool)
            .await?;
        Ok(r.rows_affected() == 1)
    }

    pub async fn add_audit_tokens(&self, id: &str, stage: &str, tokens: i64) -> Result<(), DbError> {
        let col = if stage == "rebuttal" { "rebuttal_tokens" } else { "challenge_tokens" };
        sqlx::query(&format!("UPDATE task_audits SET {col} = {col} + ? WHERE id = ?")).bind(tokens.max(0)).bind(id).execute(&self.pool).await?;
        Ok(())
    }

    /// Stores the challenge and moves the audit on, in one transaction: to `rebuttal` with risks,
    /// else `done`. `None` when the audit wasn't waiting for a challenge.
    pub async fn record_challenge(&self, audit_id: &str, risks: &[NewRisk]) -> Result<Option<Vec<Risk>>, DbError> {
        let mut tx = self.pool.begin().await?;
        let (to, finished) = if risks.is_empty() { ("done", Some(Utc::now())) } else { ("rebuttal", None) };
        let reason = risks.is_empty().then_some("no risks raised");
        let moved = sqlx::query("UPDATE task_audits SET status = ?, reason = COALESCE(?, reason), finished_at = ? WHERE id = ? AND status = 'challenge'")
            .bind(to)
            .bind(reason)
            .bind(finished)
            .bind(audit_id)
            .execute(&mut *tx)
            .await?;
        if moved.rows_affected() != 1 {
            return Ok(None);
        }
        let (task_id,): (String,) = sqlx::query_as("SELECT task_id FROM task_audits WHERE id = ?").bind(audit_id).fetch_one(&mut *tx).await?;
        let mut out = Vec::new();
        for (i, r) in risks.iter().enumerate() {
            let row = sqlx::query_as::<_, Risk>(
                "INSERT INTO audit_risks (id, audit_id, task_id, seq, risk, evidence, evidence_ids, status, created_at) \
                 VALUES (?, ?, ?, ?, ?, ?, ?, 'open', ?) RETURNING *",
            )
            .bind(uuid::Uuid::new_v4().to_string())
            .bind(audit_id)
            .bind(&task_id)
            .bind(i as i64 + 1)
            .bind(&r.risk)
            .bind(&r.evidence)
            .bind(serde_json::to_string(&r.evidence_ids).unwrap_or_else(|_| "[]".into()))
            .bind(Utc::now())
            .fetch_one(&mut *tx)
            .await?;
            out.push(row);
        }
        tx.commit().await?;
        Ok(Some(out))
    }

    /// Stores the owner's answers and ends the audit, in one transaction. A disputed risk becomes
    /// `disputed`; an accepted or unanswered one stays `open`. `None` when the audit wasn't waiting
    /// for a rebuttal.
    pub async fn record_rebuttal(&self, audit_id: &str, answers: &[Answer]) -> Result<Option<Vec<Risk>>, DbError> {
        let mut tx = self.pool.begin().await?;
        let moved = sqlx::query("UPDATE task_audits SET status = 'done', finished_at = ? WHERE id = ? AND status = 'rebuttal'")
            .bind(Utc::now())
            .bind(audit_id)
            .execute(&mut *tx)
            .await?;
        if moved.rows_affected() != 1 {
            return Ok(None);
        }
        for a in answers {
            let status = if a.stance == "disputed" { "disputed" } else { "open" };
            sqlx::query("UPDATE audit_risks SET stance = ?, answer = ?, answer_evidence = ?, status = ? WHERE id = ? AND audit_id = ? AND status = 'open'")
                .bind(&a.stance)
                .bind(&a.answer)
                .bind(&a.evidence)
                .bind(status)
                .bind(&a.risk_id)
                .bind(audit_id)
                .execute(&mut *tx)
                .await?;
        }
        tx.commit().await?;
        self.risks_of_audit(audit_id).await.map(Some)
    }

    pub async fn risks_of_audit(&self, audit_id: &str) -> Result<Vec<Risk>, DbError> {
        Ok(sqlx::query_as::<_, Risk>("SELECT * FROM audit_risks WHERE audit_id = ? ORDER BY seq").bind(audit_id).fetch_all(&self.pool).await?)
    }

    pub async fn get_risk(&self, id: &str) -> Result<Option<Risk>, DbError> {
        Ok(sqlx::query_as::<_, Risk>("SELECT * FROM audit_risks WHERE id = ?").bind(id).fetch_optional(&self.pool).await?)
    }

    /// Unresolved risks (`open` or `disputed`) of tasks in `space_id`, newest first.
    pub async fn unresolved_risks(&self, space_id: &str, limit: i64) -> Result<Vec<Risk>, DbError> {
        Ok(sqlx::query_as::<_, Risk>(
            "SELECT r.* FROM audit_risks r JOIN task_audits a ON a.id = r.audit_id \
             WHERE a.space_id = ? AND r.status IN ('open', 'disputed') ORDER BY r.created_at DESC, r.seq LIMIT ?",
        )
        .bind(space_id)
        .bind(limit)
        .fetch_all(&self.pool)
        .await?)
    }

    /// The person closes an unresolved risk; false when it was already closed.
    pub async fn resolve_risk(&self, id: &str, resolution: &str, note: Option<&str>) -> Result<bool, DbError> {
        let r = sqlx::query("UPDATE audit_risks SET status = ?, resolved_by = 'person', resolved_at = ?, note = ? WHERE id = ? AND status IN ('open', 'disputed')")
            .bind(resolution)
            .bind(Utc::now())
            .bind(note)
            .bind(id)
            .execute(&self.pool)
            .await?;
        Ok(r.rows_affected() == 1)
    }

    /// Fresh input + output of `kwant_id`'s model calls between `from` and `to`.
    pub async fn tokens_between(&self, kwant_id: &str, from: DateTime<Utc>, to: DateTime<Utc>) -> Result<i64, DbError> {
        let (n,): (Option<i64>,) = sqlx::query_as("SELECT SUM(fresh_input + output) FROM model_usage WHERE kwant_id = ? AND created_at >= ? AND created_at <= ?")
            .bind(kwant_id)
            .bind(from)
            .bind(to)
            .fetch_one(&self.pool)
            .await?;
        Ok(n.unwrap_or(0))
    }
}

/// An audit as the API shows it, with its risks.
pub fn audit_json(a: &Audit, risks: &[Risk]) -> Value {
    let mut v = serde_json::to_value(a).unwrap_or_default();
    v["risks"] = Value::Array(
        risks
            .iter()
            .map(|r| {
                let mut j = serde_json::to_value(r).unwrap_or_default();
                j["evidence_ids"] = serde_json::json!(r.evidence_ids());
                j["unresolved"] = Value::Bool(r.unresolved());
                j
            })
            .collect(),
    );
    v
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn db() -> (Database, tempfile::TempDir) {
        let tmp = tempfile::tempdir().unwrap();
        (Database::init(tmp.path()).await.unwrap(), tmp)
    }

    #[tokio::test]
    async fn an_attempt_gets_one_audit_and_a_bot_never_audits_itself() {
        let (db, _t) = db().await;
        assert!(db.create_audit("t1", "spc_default", 0, "manual", "a", "b").await.unwrap().is_some());
        assert!(db.create_audit("t1", "spc_default", 0, "manual", "c", "b").await.unwrap().is_none());
        assert!(db.create_audit("t1", "spc_default", 1, "manual", "c", "b").await.unwrap().is_some());
        assert!(db.create_audit("t2", "spc_default", 0, "manual", "b", "b").await.is_err());
    }

    #[tokio::test]
    async fn a_restart_marks_running_audits_interrupted() {
        let (db, _t) = db().await;
        let a = db.create_audit("t1", "spc_default", 0, "manual", "a", "b").await.unwrap().unwrap();
        assert!(db.move_audit(&a.id, "queued", "challenge", None).await.unwrap());
        db.migrate_auditor().await.unwrap();
        let a = db.get_audit(&a.id).await.unwrap().unwrap();
        assert_eq!((a.status.as_str(), a.finished_at.is_some()), ("interrupted", true));
    }

    #[tokio::test]
    async fn a_challenge_and_rebuttal_land_once() {
        let (db, _t) = db().await;
        let a = db.create_audit("t1", "spc_default", 0, "manual", "a", "b").await.unwrap().unwrap();
        let risk = NewRisk { risk: "r".into(), evidence: "saw x".into(), evidence_ids: vec![] };
        // Not waiting for a challenge yet
        assert!(db.record_challenge(&a.id, std::slice::from_ref(&risk)).await.unwrap().is_none());
        db.move_audit(&a.id, "queued", "challenge", None).await.unwrap();
        let risks = db.record_challenge(&a.id, &[risk.clone(), risk.clone()]).await.unwrap().unwrap();
        assert!(db.record_challenge(&a.id, &[risk]).await.unwrap().is_none());
        let answers = [Answer { risk_id: risks[0].id.clone(), stance: "disputed".into(), answer: "no".into(), evidence: Some("e".into()) }];
        let after = db.record_rebuttal(&a.id, &answers).await.unwrap().unwrap();
        assert_eq!(after.iter().map(|r| r.status.as_str()).collect::<Vec<_>>(), ["disputed", "open"]);
        assert!(db.record_rebuttal(&a.id, &answers).await.unwrap().is_none());
        assert_eq!(db.get_audit(&a.id).await.unwrap().unwrap().status, "done");
        assert!(db.resolve_risk(&after[1].id, "dismissed", None).await.unwrap());
        assert!(!db.resolve_risk(&after[1].id, "resolved", None).await.unwrap());
    }
}
