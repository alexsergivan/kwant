//! 0.9.0 wave 3: the V3 task contract and the V2 verifier's verdicts (add-only tables). The rules
//! live in `server::missions::contract` and `server::missions::verifier`. Contract:
//! `/home/maus/shared/ship-software-safely/release-090/v2-v3-contract.md`, docs/mission-control.md §24.

use super::{Database, DbError};
use chrono::{DateTime, Utc};
use serde::Serialize;
use serde_json::Value;
use sqlx::FromRow;
use std::collections::HashMap;
use uuid::Uuid;

/// A task's stored contract: the JSON body (`must_not_change`, `criteria`, `open_issues`,
/// `risk_flags`) and who wrote it last.
#[derive(Debug, Clone, FromRow, PartialEq)]
pub struct ContractRow {
    pub task_id: String,
    pub body: String,
    pub updated_by: String,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, FromRow)]
struct VerdictRow {
    id: String,
    task_id: String,
    attempt: i64,
    round: i64,
    verdict: String,
    confidence: String,
    risk_tier: String,
    risk_reasons: String,
    steps: String,
    findings: String,
    couldnt_verify: String,
    model: Option<String>,
    author_model: Option<String>,
    model_differs: bool,
    difference: String,
    verifier_kwant_id: Option<String>,
    check_id: Option<String>,
    next: String,
    summary: String,
    created_at: DateTime<Utc>,
    overturned_at: Option<DateTime<Utc>>,
    overturned_to: Option<String>,
    overturned_note: Option<String>,
    criteria_status: String,
    after_run_checkpoint_id: Option<String>,
    scope_status: String,
}

/// One verdict, as the API and the inbox read it (contract §2.1): JSON columns are parsed.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct TaskVerdict {
    pub id: String,
    pub task_id: String,
    pub attempt: i64,
    pub round: i64,
    pub verdict: String,
    pub confidence: String,
    pub risk_tier: String,
    pub risk_reasons: Value,
    pub steps: Value,
    pub findings: Value,
    pub couldnt_verify: Value,
    pub model: Option<String>,
    pub author_model: Option<String>,
    pub model_differs: bool,
    pub difference: String,
    pub verifier_kwant_id: Option<String>,
    pub check_id: Option<String>,
    pub next: String,
    pub summary: String,
    pub created_at: DateTime<Utc>,
    pub overturned_at: Option<DateTime<Utc>>,
    pub overturned_to: Option<String>,
    pub overturned_note: Option<String>,
    /// `[{id, status, claim}]` for every criterion at verdict time; `[]` without a contract (§9.4).
    pub criteria_status: Value,
    /// The `after_run` checkpoint the verdict judged, if there was one (§9.4).
    pub after_run_checkpoint_id: Option<String>,
    /// `[{entry, status}]` for each must_not_change entry (§9.10).
    pub scope_status: Value,
}

/// A JSON column's text; `Value::Null` (a `Default` NewVerdict) is stored as `[]`.
fn json_or_empty(v: &Value) -> String {
    if v.is_null() {
        "[]".into()
    } else {
        v.to_string()
    }
}

fn parsed(s: &str) -> Value {
    serde_json::from_str(s).unwrap_or(Value::Array(Vec::new()))
}

impl From<VerdictRow> for TaskVerdict {
    fn from(r: VerdictRow) -> Self {
        TaskVerdict {
            risk_reasons: parsed(&r.risk_reasons),
            steps: parsed(&r.steps),
            findings: parsed(&r.findings),
            couldnt_verify: parsed(&r.couldnt_verify),
            criteria_status: parsed(&r.criteria_status),
            scope_status: parsed(&r.scope_status),
            after_run_checkpoint_id: r.after_run_checkpoint_id,
            id: r.id,
            task_id: r.task_id,
            attempt: r.attempt,
            round: r.round,
            verdict: r.verdict,
            confidence: r.confidence,
            risk_tier: r.risk_tier,
            model: r.model,
            author_model: r.author_model,
            model_differs: r.model_differs,
            difference: r.difference,
            verifier_kwant_id: r.verifier_kwant_id,
            check_id: r.check_id,
            next: r.next,
            summary: r.summary,
            created_at: r.created_at,
            overturned_at: r.overturned_at,
            overturned_to: r.overturned_to,
            overturned_note: r.overturned_note,
        }
    }
}

/// What a finished verification writes.
#[derive(Debug, Clone, Default)]
pub struct NewVerdict {
    pub task_id: String,
    pub attempt: i64,
    pub round: i64,
    pub verdict: String,
    pub confidence: String,
    pub risk_tier: String,
    pub risk_reasons: Value,
    pub steps: Value,
    pub findings: Value,
    pub couldnt_verify: Value,
    pub model: Option<String>,
    pub author_model: Option<String>,
    pub model_differs: bool,
    pub difference: String,
    pub verifier_kwant_id: Option<String>,
    pub check_id: Option<String>,
    pub next: String,
    pub summary: String,
    pub criteria_status: Value,
    pub after_run_checkpoint_id: Option<String>,
    pub scope_status: Value,
}

/// How often the person overturned the verifier (contract §6).
#[derive(Debug, Clone, Default, Serialize, PartialEq)]
pub struct Accuracy {
    pub verdicts: i64,
    pub overturned: i64,
    pub by_verdict: HashMap<String, (i64, i64)>,
}

pub const VERDICTS: [&str; 3] = ["pass", "fail", "needs_you"];
pub const NEXTS: [&str; 4] = ["counted", "sent_back", "escalated", "held_for_you"];
pub const OVERTURNED_TO: [&str; 3] = ["verified", "sent_back", "rolled_back"];

impl Database {
    /// Add-only: two new tables. Verdicts can't be deleted, and only the overturn columns can
    /// change, once.
    pub(super) async fn migrate_verifier(&self) -> Result<(), DbError> {
        for stmt in [
            "CREATE TABLE IF NOT EXISTS task_contracts (
                task_id TEXT PRIMARY KEY REFERENCES tasks(id),
                body TEXT NOT NULL,
                updated_by TEXT NOT NULL,
                updated_at TIMESTAMP NOT NULL
            )",
            "CREATE TABLE IF NOT EXISTS task_verdicts (
                id TEXT PRIMARY KEY,
                task_id TEXT NOT NULL REFERENCES tasks(id),
                attempt INTEGER NOT NULL,
                round INTEGER NOT NULL CHECK (round >= 1),
                verdict TEXT NOT NULL CHECK (verdict IN ('pass', 'fail', 'needs_you')),
                confidence TEXT NOT NULL CHECK (confidence IN ('high', 'medium', 'low')),
                risk_tier TEXT NOT NULL CHECK (risk_tier IN ('low', 'high')),
                risk_reasons TEXT NOT NULL DEFAULT '[]',
                steps TEXT NOT NULL DEFAULT '[]',
                findings TEXT NOT NULL DEFAULT '[]',
                couldnt_verify TEXT NOT NULL DEFAULT '[]',
                model TEXT,
                author_model TEXT,
                model_differs INTEGER NOT NULL DEFAULT 0,
                difference TEXT NOT NULL CHECK (difference IN ('model', 'tier', 'none', 'no_model_turn')),
                verifier_kwant_id TEXT,
                check_id TEXT,
                next TEXT NOT NULL CHECK (next IN ('counted', 'sent_back', 'escalated', 'held_for_you')),
                summary TEXT NOT NULL DEFAULT '',
                created_at TIMESTAMP NOT NULL,
                overturned_at TIMESTAMP,
                overturned_to TEXT CHECK (overturned_to IS NULL OR overturned_to IN ('verified', 'sent_back', 'rolled_back')),
                overturned_note TEXT,
                criteria_status TEXT NOT NULL DEFAULT '[]',
                after_run_checkpoint_id TEXT,
                scope_status TEXT NOT NULL DEFAULT '[]'
            )",
            // R8b: one row per approval a rollback stepped its owner down for, so two forks (or a
            // fork and a restore) of the same approval drop the owner once, not once each
            "CREATE TABLE IF NOT EXISTS rollback_step_downs (
                task_id TEXT NOT NULL,
                approval TEXT NOT NULL,
                checkpoint_id TEXT NOT NULL,
                created_at TIMESTAMP NOT NULL,
                PRIMARY KEY (task_id, approval)
            )",
            "CREATE INDEX IF NOT EXISTS task_verdicts_task ON task_verdicts (task_id, created_at)",
            "CREATE INDEX IF NOT EXISTS task_verdicts_created ON task_verdicts (created_at)",
            "CREATE TRIGGER IF NOT EXISTS task_verdicts_no_delete BEFORE DELETE ON task_verdicts \
             BEGIN SELECT RAISE(ABORT, 'verdicts are append-only'); END",
            "CREATE TRIGGER IF NOT EXISTS task_verdicts_only_overturn BEFORE UPDATE ON task_verdicts \
             WHEN OLD.overturned_at IS NOT NULL OR NEW.id != OLD.id OR NEW.task_id != OLD.task_id OR NEW.verdict != OLD.verdict \
               OR NEW.round != OLD.round OR NEW.findings != OLD.findings OR NEW.next != OLD.next OR NEW.check_id IS NOT OLD.check_id \
             BEGIN SELECT RAISE(ABORT, 'verdicts are append-only'); END",
        ] {
            sqlx::query(stmt).execute(&self.pool).await?;
        }
        // §9.4: added after the table first shipped to test builds, so add them where missing
        for (col, ddl) in [
            ("criteria_status", "ALTER TABLE task_verdicts ADD COLUMN criteria_status TEXT NOT NULL DEFAULT '[]'"),
            ("after_run_checkpoint_id", "ALTER TABLE task_verdicts ADD COLUMN after_run_checkpoint_id TEXT"),
            ("scope_status", "ALTER TABLE task_verdicts ADD COLUMN scope_status TEXT NOT NULL DEFAULT '[]'"),
        ] {
            let has: Option<(String,)> = sqlx::query_as("SELECT name FROM pragma_table_info('task_verdicts') WHERE name = ?").bind(col).fetch_optional(&self.pool).await?;
            if has.is_none() {
                sqlx::query(ddl).execute(&self.pool).await?;
            }
        }
        Ok(())
    }

    pub async fn get_contract(&self, task_id: &str) -> Result<Option<ContractRow>, DbError> {
        Ok(sqlx::query_as::<_, ContractRow>("SELECT * FROM task_contracts WHERE task_id = ?").bind(task_id).fetch_optional(&self.pool).await?)
    }

    /// Stores `body` as the task's contract and logs `contract_updated` in one transaction.
    pub async fn put_contract(&self, task_id: &str, body: &Value, by: &str, event: &Value) -> Result<ContractRow, DbError> {
        let now = Utc::now();
        let mut tx = self.pool.begin().await?;
        let row = sqlx::query_as::<_, ContractRow>(
            "INSERT INTO task_contracts (task_id, body, updated_by, updated_at) VALUES (?, ?, ?, ?) \
             ON CONFLICT(task_id) DO UPDATE SET body = excluded.body, updated_by = excluded.updated_by, updated_at = excluded.updated_at RETURNING *",
        )
        .bind(task_id)
        .bind(body.to_string())
        .bind(by)
        .bind(now)
        .fetch_one(&mut *tx)
        .await?;
        sqlx::query(
            "INSERT INTO task_events (task_id, mission_id, kind, actor, from_state, to_state, detail, created_at) \
             VALUES (?, (SELECT mission_id FROM tasks WHERE id = ?), 'contract_updated', ?, NULL, NULL, ?, ?)",
        )
        .bind(task_id)
        .bind(task_id)
        .bind(by)
        .bind(event.to_string())
        .bind(now)
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(row)
    }

    pub async fn add_verdict(&self, v: &NewVerdict) -> Result<TaskVerdict, DbError> {
        let row = sqlx::query_as::<_, VerdictRow>(
            "INSERT INTO task_verdicts (id, task_id, attempt, round, verdict, confidence, risk_tier, risk_reasons, steps, findings, \
             couldnt_verify, model, author_model, model_differs, difference, verifier_kwant_id, check_id, next, summary, created_at, \
             criteria_status, after_run_checkpoint_id, scope_status) \
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?) RETURNING *",
        )
        .bind(Uuid::new_v4().to_string())
        .bind(&v.task_id)
        .bind(v.attempt)
        .bind(v.round)
        .bind(&v.verdict)
        .bind(&v.confidence)
        .bind(&v.risk_tier)
        .bind(v.risk_reasons.to_string())
        .bind(v.steps.to_string())
        .bind(v.findings.to_string())
        .bind(v.couldnt_verify.to_string())
        .bind(&v.model)
        .bind(&v.author_model)
        .bind(v.model_differs)
        .bind(&v.difference)
        .bind(&v.verifier_kwant_id)
        .bind(&v.check_id)
        .bind(&v.next)
        .bind(&v.summary)
        .bind(Utc::now())
        .bind(json_or_empty(&v.criteria_status))
        .bind(&v.after_run_checkpoint_id)
        .bind(json_or_empty(&v.scope_status))
        .fetch_one(&self.pool)
        .await?;
        Ok(row.into())
    }

    /// The task's verdicts, newest first.
    pub async fn verdicts_of(&self, task_id: &str) -> Result<Vec<TaskVerdict>, DbError> {
        let rows = sqlx::query_as::<_, VerdictRow>("SELECT * FROM task_verdicts WHERE task_id = ? ORDER BY created_at DESC, rowid DESC")
            .bind(task_id)
            .fetch_all(&self.pool)
            .await?;
        Ok(rows.into_iter().map(Into::into).collect())
    }

    pub async fn latest_verdict(&self, task_id: &str) -> Result<Option<TaskVerdict>, DbError> {
        Ok(self.verdicts_of(task_id).await?.into_iter().next())
    }

    /// The newest verdict of each of `task_ids` (one query, for the inbox join).
    pub async fn latest_verdicts(&self, task_ids: &[String]) -> Result<HashMap<String, TaskVerdict>, DbError> {
        if task_ids.is_empty() {
            return Ok(HashMap::new());
        }
        let marks = vec!["?"; task_ids.len()].join(", ");
        let sql = format!("SELECT * FROM task_verdicts WHERE task_id IN ({marks}) ORDER BY created_at ASC, rowid ASC");
        let mut q = sqlx::query_as::<_, VerdictRow>(&sql);
        for id in task_ids {
            q = q.bind(id);
        }
        let mut out = HashMap::new();
        for r in q.fetch_all(&self.pool).await? {
            out.insert(r.task_id.clone(), TaskVerdict::from(r));
        }
        Ok(out)
    }

    /// The verdict that settled `check_id`, if one did.
    pub async fn verdict_of_check(&self, check_id: &str) -> Result<Option<TaskVerdict>, DbError> {
        let row = sqlx::query_as::<_, VerdictRow>("SELECT * FROM task_verdicts WHERE check_id = ? ORDER BY created_at DESC LIMIT 1")
            .bind(check_id)
            .fetch_optional(&self.pool)
            .await?;
        Ok(row.map(Into::into))
    }

    /// Model turns the verifier ran since `since` (for its daily cap).
    pub async fn verifier_turns_since(&self, since: DateTime<Utc>) -> Result<i64, DbError> {
        let (n,): (i64,) = sqlx::query_as("SELECT COUNT(*) FROM task_verdicts WHERE created_at >= ? AND model IS NOT NULL")
            .bind(since)
            .fetch_one(&self.pool)
            .await?;
        Ok(n)
    }

    /// Marks `verdict_id` overturned by the person, once and logs `verdict_overturned`. `false`
    /// when it already was.
    /// The first rollback of one approval of a task (R8b): records `(task_id, approval)` and, in the
    /// same write-locked transaction, steps the owner down (`step` = kwant_id, class, tier, detail)
    /// and logs `tier_stepped_down` on the task. `Ok(None)` when that approval was already rolled
    /// back, so nothing changes; `Ok(Some(None))` for the first rollback when there was nothing to
    /// drop. Two concurrent forks of one verified task land here once each; only one writes.
    pub async fn step_down_once(
        &self,
        task_id: &str,
        approval: &str,
        checkpoint_id: &str,
        step: Option<(&str, &str, &str, &Value)>,
    ) -> Result<Option<Option<super::flight_school::Policy>>, DbError> {
        let mut tx = self.begin_write().await?;
        let first = sqlx::query("INSERT OR IGNORE INTO rollback_step_downs (task_id, approval, checkpoint_id, created_at) VALUES (?, ?, ?, ?)")
            .bind(task_id)
            .bind(approval)
            .bind(checkpoint_id)
            .bind(Utc::now())
            .execute(&mut *tx)
            .await?
            .rows_affected()
            == 1;
        if !first {
            tx.rollback().await?;
            return Ok(None);
        }
        let mut policy = None;
        if let Some((kwant_id, class, tier, detail)) = step {
            let note = detail.to_string();
            policy = Some(super::flight_school::write_policy(&mut tx, kwant_id, class, tier, "stepped_down", Some(&note)).await?);
            sqlx::query(
                "INSERT INTO task_events (task_id, mission_id, kind, actor, from_state, to_state, detail, created_at) \
                 VALUES (?, (SELECT mission_id FROM tasks WHERE id = ?), 'tier_stepped_down', 'person', NULL, NULL, ?, ?)",
            )
            .bind(task_id)
            .bind(task_id)
            .bind(&note)
            .bind(Utc::now())
            .execute(&mut *tx)
            .await?;
        }
        tx.commit().await?;
        Ok(Some(policy))
    }

    /// Whether a rollback already used `approval` of the task (see [`Database::step_down_once`]).
    pub async fn stepped_down_for(&self, task_id: &str, approval: &str) -> Result<bool, DbError> {
        let row: Option<(String,)> = sqlx::query_as("SELECT task_id FROM rollback_step_downs WHERE task_id = ? AND approval = ?")
            .bind(task_id)
            .bind(approval)
            .fetch_optional(&self.pool)
            .await?;
        Ok(row.is_some())
    }

    pub async fn overturn_verdict(&self, verdict_id: &str, to: &str, note: Option<&str>) -> Result<bool, DbError> {
        let now = Utc::now();
        let mut tx = self.pool.begin().await?;
        let done = sqlx::query("UPDATE task_verdicts SET overturned_at = ?, overturned_to = ?, overturned_note = ? WHERE id = ? AND overturned_at IS NULL")
            .bind(now)
            .bind(to)
            .bind(note)
            .bind(verdict_id)
            .execute(&mut *tx)
            .await?
            .rows_affected()
            == 1;
        if done {
            let (task_id, from): (String, String) = sqlx::query_as("SELECT task_id, verdict FROM task_verdicts WHERE id = ?").bind(verdict_id).fetch_one(&mut *tx).await?;
            let detail = serde_json::json!({"verdict_id": verdict_id, "from": from, "to": to});
            sqlx::query(
                "INSERT INTO task_events (task_id, mission_id, kind, actor, from_state, to_state, detail, created_at) \
                 VALUES (?, (SELECT mission_id FROM tasks WHERE id = ?), 'verdict_overturned', 'person', NULL, NULL, ?, ?)",
            )
            .bind(&task_id)
            .bind(&task_id)
            .bind(detail.to_string())
            .bind(now)
            .execute(&mut *tx)
            .await?;
        }
        tx.commit().await?;
        Ok(done)
    }

    /// Verdicts since `since` and how many the person overturned, in all and per verdict, in
    /// the tasks of `space_id`.
    pub async fn verifier_accuracy(&self, space_id: &str, since: DateTime<Utc>) -> Result<Accuracy, DbError> {
        let rows: Vec<(String, i64, i64)> = sqlx::query_as(
            "SELECT v.verdict, COUNT(*), SUM(CASE WHEN v.overturned_at IS NULL THEN 0 ELSE 1 END) FROM task_verdicts v \
             JOIN tasks t ON t.id = v.task_id WHERE t.space_id = ? AND v.created_at >= ? GROUP BY v.verdict",
        )
        .bind(space_id)
        .bind(since)
        .fetch_all(&self.pool)
        .await?;
        let mut acc = Accuracy::default();
        for (verdict, n, over) in rows {
            acc.verdicts += n;
            acc.overturned += over;
            acc.by_verdict.insert(verdict, (n, over));
        }
        Ok(acc)
    }

    /// Pending verifier holds of tasks still in review with no verdict yet: a verification kwnt
    /// stopped in the middle of (contract §3.4, the restart rule).
    pub async fn orphan_verifier_holds(&self) -> Result<Vec<(String, String)>, DbError> {
        Ok(sqlx::query_as(
            "SELECT c.id, c.task_id FROM outcome_checks c JOIN tasks t ON t.id = c.task_id \
             WHERE c.kind = 'peer_review' AND c.requested_by = 'verifier' AND c.status = 'pending' AND t.state = 'review' \
             AND NOT EXISTS (SELECT 1 FROM task_verdicts v WHERE v.check_id = c.id)",
        )
        .fetch_all(&self.pool)
        .await?)
    }

    /// The task's pending verifier holds.
    pub async fn pending_verifier_holds(&self, task_id: &str) -> Result<Vec<String>, DbError> {
        let rows: Vec<(String,)> =
            sqlx::query_as("SELECT id FROM outcome_checks WHERE task_id = ? AND kind = 'peer_review' AND requested_by = 'verifier' AND status = 'pending'")
                .bind(task_id)
                .fetch_all(&self.pool)
                .await?;
        Ok(rows.into_iter().map(|r| r.0).collect())
    }

    /// Ends a hold that no longer gates: optional, finished `error` with `why`.
    pub async fn release_verifier_hold(&self, check_id: &str, why: &str) -> Result<(), DbError> {
        sqlx::query("UPDATE outcome_checks SET required = 0, status = 'error', output_excerpt = ?, checked_by = 'kwant', finished_at = ? WHERE id = ? AND status = 'pending'")
            .bind(why)
            .bind(Utc::now())
            .bind(check_id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    /// The most used model of `kwant_id` between `from` and `to` (the author's model).
    pub async fn main_model_between(&self, kwant_id: &str, from: DateTime<Utc>, to: DateTime<Utc>) -> Result<Option<String>, DbError> {
        let row: Option<(String,)> = sqlx::query_as(
            "SELECT model FROM model_usage WHERE kwant_id = ? AND created_at >= ? AND created_at <= ? AND model != '' \
             GROUP BY model ORDER BY COUNT(*) DESC, MAX(created_at) DESC LIMIT 1",
        )
        .bind(kwant_id)
        .bind(from)
        .bind(to)
        .fetch_optional(&self.pool)
        .await?;
        Ok(row.map(|r| r.0))
    }

    /// The newest `scorecard_reset` of `kwant_id` (contract §5.2).
    pub async fn last_scorecard_reset(&self, kwant_id: &str) -> Result<Option<DateTime<Utc>>, DbError> {
        let row: Option<(String,)> =
            sqlx::query_as("SELECT created_at FROM capability_events WHERE kwant_id = ? AND kind = 'scorecard_reset' ORDER BY id DESC LIMIT 1")
                .bind(kwant_id)
                .fetch_optional(&self.pool)
                .await?;
        Ok(row.and_then(|r| DateTime::parse_from_rfc3339(&r.0).ok()).map(|d| d.with_timezone(&Utc)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    async fn db() -> (tempfile::TempDir, Database) {
        let tmp = tempfile::tempdir().unwrap();
        let db = Database::init(tmp.path()).await.unwrap();
        (tmp, db)
    }

    async fn a_task(db: &Database) -> String {
        let t = crate::db::missions::NewTask {
            title: "t".into(),
            acceptance: "a".into(),
            state: "backlog".into(),
            priority: 1,
            max_attempts: 3,
            source_kind: "person".into(),
            created_by: "person".into(),
            ..Default::default()
        };
        db.create_task(&crate::db::spaces::SpaceId::default_space(), &t).await.unwrap().id
    }

    fn new(task: &str, verdict: &str) -> NewVerdict {
        NewVerdict {
            task_id: task.into(),
            round: 1,
            verdict: verdict.into(),
            confidence: "high".into(),
            risk_tier: "low".into(),
            risk_reasons: json!([]),
            steps: json!([{"step": "contract", "status": "passed"}]),
            findings: json!([]),
            couldnt_verify: json!([]),
            difference: "no_model_turn".into(),
            next: "counted".into(),
            ..Default::default()
        }
    }

    /// Contract §5.2: a model change writes one `scorecard_reset` (tier `act` with no policy row,
    /// the live `*` tier with one); saving the same model writes none.
    #[tokio::test]
    async fn a_model_change_resets_the_scorecard_once() {
        let (_t, db) = db().await;
        let bot = db.list_kwants(&crate::db::spaces::SpaceId::default_space()).await.unwrap()[0].clone();
        let edit = |model: &str| crate::db::models::UpdateKwant {
            name: bot.name.clone(),
            title: bot.title.clone(),
            persona_prompt: bot.persona_prompt.clone(),
            soul: bot.soul.clone(),
            avatar_sprite: bot.avatar_sprite.clone(),
            preferred_model: model.into(),
        };
        assert_eq!(db.last_scorecard_reset(&bot.id).await.unwrap(), None);
        db.update_kwant(&bot.id, &edit(&bot.preferred_model)).await.unwrap();
        assert_eq!(db.last_scorecard_reset(&bot.id).await.unwrap(), None, "same model: no reset");
        db.update_kwant(&bot.id, &edit("claude:opus")).await.unwrap();
        assert!(db.last_scorecard_reset(&bot.id).await.unwrap().is_some());
        db.set_policy(&bot.id, "*", "sandbox", "granted", None).await.unwrap();
        db.update_kwant(&bot.id, &edit("codex")).await.unwrap();
        let rows: Vec<(String, String, String, String)> =
            sqlx::query_as("SELECT task_class, tier, actor, detail FROM capability_events WHERE kwant_id = ? AND kind = 'scorecard_reset' ORDER BY id")
                .bind(&bot.id)
                .fetch_all(&db.pool)
                .await
                .unwrap();
        assert_eq!(rows.len(), 2, "{rows:?}");
        assert_eq!((rows[0].0.as_str(), rows[0].1.as_str(), rows[0].2.as_str()), ("*", "act", "person"));
        assert_eq!(rows[1].1, "sandbox");
        let d: Value = serde_json::from_str(&rows[1].3).unwrap();
        assert_eq!((d["from_model"].as_str(), d["to_model"].as_str()), (Some("claude:opus"), Some("codex")));
    }

    #[tokio::test]
    async fn verdicts_are_append_only_and_overturned_once() {
        let (_tmp, db) = db().await;
        let t = a_task(&db).await;
        let v = db.add_verdict(&new(&t, "pass")).await.unwrap();
        assert_eq!(v.steps[0]["step"], "contract", "JSON columns come back parsed");
        assert!(sqlx::query("DELETE FROM task_verdicts").execute(&db.pool).await.is_err());
        assert!(sqlx::query("UPDATE task_verdicts SET verdict = 'fail'").execute(&db.pool).await.is_err());
        assert!(db.overturn_verdict(&v.id, "sent_back", Some("wrong")).await.unwrap());
        assert!(!db.overturn_verdict(&v.id, "verified", None).await.unwrap(), "only once");
        let after = db.latest_verdict(&t).await.unwrap().unwrap();
        assert_eq!(after.overturned_to.as_deref(), Some("sent_back"));
        let ev: Vec<(String,)> = sqlx::query_as("SELECT kind FROM task_events WHERE task_id = ?").bind(&t).fetch_all(&db.pool).await.unwrap();
        assert!(ev.iter().any(|e| e.0 == "verdict_overturned"));
        let acc = db.verifier_accuracy(crate::db::spaces::DEFAULT_SPACE_ID, Utc::now() - chrono::Duration::days(1)).await.unwrap();
        assert_eq!((acc.verdicts, acc.overturned, acc.by_verdict["pass"]), (1, 1, (1, 1)));
    }

    #[tokio::test]
    async fn latest_verdicts_is_the_newest_per_task() {
        let (_tmp, db) = db().await;
        let (a, b) = (a_task(&db).await, a_task(&db).await);
        db.add_verdict(&new(&a, "fail")).await.unwrap();
        db.add_verdict(&new(&a, "pass")).await.unwrap();
        db.add_verdict(&new(&b, "needs_you")).await.unwrap();
        let m = db.latest_verdicts(&[a.clone(), b.clone(), "none".into()]).await.unwrap();
        assert_eq!((m[&a].verdict.as_str(), m[&b].verdict.as_str(), m.len()), ("pass", "needs_you", 2));
        assert_eq!(db.verdicts_of(&a).await.unwrap().len(), 2);
    }

    #[tokio::test]
    async fn bad_values_are_refused() {
        let (_tmp, db) = db().await;
        let t = a_task(&db).await;
        assert!(db.add_verdict(&new(&t, "maybe")).await.is_err());
        let mut v = new(&t, "pass");
        v.next = "later".into();
        assert!(db.add_verdict(&v).await.is_err());
    }

    #[tokio::test]
    async fn contract_writes_log_an_event() {
        let (_tmp, db) = db().await;
        let t = a_task(&db).await;
        db.put_contract(&t, &json!({"criteria": []}), "person", &json!({"by": "person"})).await.unwrap();
        db.put_contract(&t, &json!({"criteria": [{"id": "c1"}]}), "k1", &json!({"by": "k1"})).await.unwrap();
        let c = db.get_contract(&t).await.unwrap().unwrap();
        assert_eq!((c.updated_by.as_str(), c.body.contains("c1")), ("k1", true));
        let (n,): (i64,) = sqlx::query_as("SELECT COUNT(*) FROM task_events WHERE task_id = ? AND kind = 'contract_updated'").bind(&t).fetch_one(&db.pool).await.unwrap();
        assert_eq!(n, 2);
    }

    #[tokio::test]
    async fn verdicts_keep_criteria_scope_and_the_judged_checkpoint() {
        let (_tmp, db) = db().await;
        let t = a_task(&db).await;
        let plain = db.add_verdict(&NewVerdict { criteria_status: Value::Null, scope_status: Value::Null, ..new(&t, "pass") }).await.unwrap();
        assert_eq!((plain.criteria_status.clone(), plain.scope_status.clone(), plain.after_run_checkpoint_id.clone()), (json!([]), json!([]), None));
        let v = NewVerdict {
            criteria_status: json!([{"id": "c1", "status": "met", "claim": true}]),
            scope_status: json!([{"entry": "docs/", "status": "met"}]),
            after_run_checkpoint_id: Some("cp1".into()),
            ..new(&t, "fail")
        };
        let saved = db.add_verdict(&v).await.unwrap();
        let back = db.latest_verdict(&t).await.unwrap().unwrap();
        assert_eq!(back, saved);
        assert_eq!(back.criteria_status[0]["claim"], true);
        assert_eq!(back.scope_status[0]["entry"], "docs/");
        assert_eq!(back.after_run_checkpoint_id.as_deref(), Some("cp1"));
        let j = serde_json::to_value(&back).unwrap();
        assert!(j.get("criteria_status").is_some() && j.get("after_run_checkpoint_id").is_some() && j.get("scope_status").is_some());
        // Migrating again is a no-op
        db.migrate_verifier().await.unwrap();
    }
}
