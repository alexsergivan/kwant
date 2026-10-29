//! Mission Control Phase D3: the Skill Foundry's records. A Foundry draft is an ordinary `pending`
//! skill plus a `skill_meta` row (the cluster it came from, its inputs, checks, examples, the tools
//! it uses, its replay report, its version and lineage). Draft and replay runs live in
//! `foundry_runs`; `skill_events` is each skill's history. A skill stays text only: the tools listed
//! in its meta are shown, never granted. See docs/mission-control.md §19.

use super::skills::{Skill, SkillError, SkillText, MAX_PENDING_PER_KWANT};
use super::{Database, DbError};
use chrono::{DateTime, Duration, Utc};
use serde::Serialize;
use serde_json::Value;
use sqlx::FromRow;

pub const RUN_KINDS: [&str; 2] = ["draft", "replay"];
pub const RUN_STATUSES: [&str; 6] = ["queued", "drafting", "replaying", "done", "failed", "interrupted"];
/// The miner's window, thresholds and cool-offs (design §3).
pub const WINDOW_DAYS: i64 = 60;
pub const MIN_VERIFIED: i64 = 3;
pub const MIN_RATE_PCT: i64 = 70;
pub const REJECT_COOL_OFF_DAYS: i64 = 30;
pub const DRAFT_COOL_OFF_DAYS: i64 = 7;

/// What a Foundry draft adds to its skill.
#[derive(Debug, Clone, Serialize, FromRow, PartialEq)]
pub struct SkillMeta {
    pub skill_id: String,
    pub space_id: String,
    pub kwant_id: String,
    pub task_class: String,
    /// JSON: `[{name, description, example}]`.
    pub inputs: String,
    /// JSON: `[{kind, spec}]`.
    pub checks: String,
    /// JSON: `[task_id]`.
    pub examples: String,
    /// JSON: `[tool_name]`. Shown on the card; never granted.
    pub tools: String,
    /// JSON replay report, `None` until one ran.
    pub replay: Option<String>,
    pub version: i64,
    /// The first version's skill id.
    pub lineage_id: String,
    pub created_at: DateTime<Utc>,
}

impl SkillMeta {
    pub fn json(&self) -> Value {
        let parse = |s: &str| serde_json::from_str::<Value>(s).unwrap_or(Value::Array(vec![]));
        serde_json::json!({
            "skill_id": self.skill_id,
            "kwant_id": self.kwant_id,
            "task_class": self.task_class,
            "inputs": parse(&self.inputs),
            "checks": parse(&self.checks),
            "examples": parse(&self.examples),
            "tools": parse(&self.tools),
            "replay": self.replay.as_deref().map(parse),
            "version": self.version,
            "lineage_id": self.lineage_id,
            "created_at": self.created_at,
        })
    }

    pub fn examples(&self) -> Vec<String> {
        serde_json::from_str(&self.examples).unwrap_or_default()
    }

    pub fn checks(&self) -> Vec<Value> {
        serde_json::from_str(&self.checks).unwrap_or_default()
    }
}

#[derive(Debug, Clone, Serialize, FromRow, PartialEq)]
pub struct FoundryRun {
    pub id: String,
    /// `draft` or `replay`.
    pub kind: String,
    pub space_id: String,
    pub kwant_id: String,
    pub task_class: String,
    /// The draft a replay runs (and, once drafted, the draft a draft run made).
    pub skill_id: Option<String>,
    pub status: String,
    pub reason: Option<String>,
    pub tokens: i64,
    pub created_at: DateTime<Utc>,
    pub finished_at: Option<DateTime<Utc>>,
}

impl FoundryRun {
    pub fn live(&self) -> bool {
        matches!(self.status.as_str(), "queued" | "drafting" | "replaying")
    }
}

#[derive(Debug, Clone, Serialize, FromRow, PartialEq)]
pub struct SkillEvent {
    pub id: i64,
    pub skill_id: String,
    pub kind: String,
    pub actor: String,
    #[serde(skip)]
    pub detail: String,
    pub created_at: DateTime<Utc>,
}

/// Same space, same class (not `general`), same owner bot: verified work the miner counts.
#[derive(Debug, Clone, Serialize, FromRow, PartialEq)]
pub struct Cluster {
    pub space_id: String,
    pub kwant_id: String,
    pub task_class: String,
    pub verified: i64,
    pub total: i64,
}

impl Cluster {
    /// `{kwant_id}~{class}`: the id the routes take.
    pub fn id(&self) -> String {
        cluster_id(&self.kwant_id, &self.task_class)
    }
}

pub fn cluster_id(kwant_id: &str, class: &str) -> String {
    format!("{kwant_id}~{class}")
}

/// `(kwant_id, class)` of a cluster id.
pub fn split_cluster(id: &str) -> Option<(&str, &str)> {
    id.split_once('~').filter(|(k, c)| !k.is_empty() && !c.is_empty())
}

/// A Foundry draft before it's stored.
#[derive(Debug, Clone, PartialEq)]
pub struct NewDraft {
    pub kwant_id: String,
    pub task_class: String,
    pub text: SkillText,
    pub inputs: Value,
    pub checks: Value,
    pub examples: Vec<String>,
    pub tools: Vec<String>,
}

fn in_list(list: &[&str]) -> String {
    list.iter().map(|s| format!("'{s}'")).collect::<Vec<_>>().join(", ")
}

impl Database {
    pub(super) async fn migrate_foundry(&self) -> Result<(), DbError> {
        let runs = format!(
            "CREATE TABLE IF NOT EXISTS foundry_runs (
                id TEXT PRIMARY KEY,
                kind TEXT NOT NULL CHECK (kind IN ({})),
                space_id TEXT NOT NULL,
                kwant_id TEXT NOT NULL,
                task_class TEXT NOT NULL,
                skill_id TEXT,
                status TEXT NOT NULL CHECK (status IN ({})),
                reason TEXT,
                tokens INTEGER NOT NULL DEFAULT 0,
                created_at TIMESTAMP NOT NULL,
                finished_at TIMESTAMP
            )",
            in_list(&RUN_KINDS),
            in_list(&RUN_STATUSES)
        );
        for stmt in [
            "CREATE TABLE IF NOT EXISTS skill_meta (
                skill_id TEXT PRIMARY KEY REFERENCES skills(id) ON DELETE CASCADE,
                space_id TEXT NOT NULL,
                kwant_id TEXT NOT NULL,
                task_class TEXT NOT NULL,
                inputs TEXT NOT NULL DEFAULT '[]',
                checks TEXT NOT NULL DEFAULT '[]',
                examples TEXT NOT NULL DEFAULT '[]',
                tools TEXT NOT NULL DEFAULT '[]',
                replay TEXT,
                version INTEGER NOT NULL CHECK (version >= 1),
                lineage_id TEXT NOT NULL,
                created_at TIMESTAMP NOT NULL
            )",
            "CREATE INDEX IF NOT EXISTS skill_meta_owner ON skill_meta (kwant_id, task_class)",
            "CREATE INDEX IF NOT EXISTS skill_meta_lineage ON skill_meta (lineage_id, version)",
            runs.as_str(),
            "CREATE INDEX IF NOT EXISTS foundry_runs_status ON foundry_runs (status, created_at)",
            "CREATE INDEX IF NOT EXISTS foundry_runs_cluster ON foundry_runs (kwant_id, task_class, kind, created_at)",
            "CREATE TABLE IF NOT EXISTS skill_events (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                skill_id TEXT NOT NULL,
                kind TEXT NOT NULL,
                actor TEXT NOT NULL,
                detail TEXT NOT NULL DEFAULT '{}',
                created_at TIMESTAMP NOT NULL
            )",
            "CREATE INDEX IF NOT EXISTS skill_events_skill ON skill_events (skill_id, id)",
        ] {
            sqlx::query(stmt).execute(&self.pool).await?;
        }
        // A restart cuts off a draft or replay turn: it never finishes, so say so. Queued runs
        // had no turn yet: the server runs them again (`foundry::resume_after_restart`).
        sqlx::query(
            "UPDATE foundry_runs SET status = 'interrupted', reason = 'kwant restarted during the run', finished_at = ?
             WHERE status IN ('drafting', 'replaying')",
        )
        .bind(Utc::now())
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// Clusters in the miner's window that pass its thresholds (≥ 3 verified, ≥ 70% verified),
    /// in `space_id` when given. Blocks and cool-offs are checked by [`Self::foundry_block`].
    pub async fn foundry_clusters(&self, space_id: Option<&str>, now: DateTime<Utc>) -> Result<Vec<Cluster>, DbError> {
        Ok(sqlx::query_as::<_, Cluster>(
            "SELECT space_id, kwant_id, task_class, SUM(result = 'verified') AS verified, COUNT(*) AS total
             FROM performance_observations
             WHERE finished_at >= ?1 AND task_class <> 'general' AND (?2 IS NULL OR space_id = ?2)
             GROUP BY space_id, kwant_id, task_class
             HAVING SUM(result = 'verified') >= ?3 AND SUM(result = 'verified') * 100 >= COUNT(*) * ?4
             ORDER BY verified DESC, kwant_id, task_class",
        )
        .bind(now - Duration::days(WINDOW_DAYS))
        .bind(space_id)
        .bind(MIN_VERIFIED)
        .bind(MIN_RATE_PCT)
        .fetch_all(&self.pool)
        .await?)
    }

    /// The cluster's task ids in the window, newest first: verified ones, then failed ones.
    pub async fn cluster_tasks(&self, space_id: &str, kwant_id: &str, class: &str, result: &str, now: DateTime<Utc>, limit: i64) -> Result<Vec<String>, DbError> {
        let rows: Vec<(String,)> = sqlx::query_as(
            "SELECT task_id FROM performance_observations
             WHERE space_id = ? AND kwant_id = ? AND task_class = ? AND result = ? AND finished_at >= ?
             ORDER BY finished_at DESC, id DESC LIMIT ?",
        )
        .bind(space_id)
        .bind(kwant_id)
        .bind(class)
        .bind(result)
        .bind(now - Duration::days(WINDOW_DAYS))
        .bind(limit)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows.into_iter().map(|r| r.0).collect())
    }

    /// Why the miner offers no draft for (bot, class) now: `has_draft` (one is waiting for
    /// review), `has_skill` (a live Foundry skill exists; the person can still ask for a new
    /// version), `rejected` (one was rejected in the last 30 days), `cooling_off` (a draft run in
    /// the last 7 days). `None` when a draft may be offered.
    pub async fn foundry_block(&self, kwant_id: &str, class: &str, now: DateTime<Utc>) -> Result<Option<&'static str>, DbError> {
        let statuses: Vec<(String, Option<DateTime<Utc>>)> = sqlx::query_as(
            "SELECT s.status, s.reviewed_at FROM skills s JOIN skill_meta m ON m.skill_id = s.id WHERE m.kwant_id = ? AND m.task_class = ?",
        )
        .bind(kwant_id)
        .bind(class)
        .fetch_all(&self.pool)
        .await?;
        if statuses.iter().any(|(s, _)| s == "pending") {
            return Ok(Some("has_draft"));
        }
        if statuses.iter().any(|(s, _)| s == "enabled" || s == "disabled") {
            return Ok(Some("has_skill"));
        }
        let cutoff = now - Duration::days(REJECT_COOL_OFF_DAYS);
        if statuses.iter().any(|(s, at)| s == "rejected" && at.is_some_and(|t| t >= cutoff)) {
            return Ok(Some("rejected"));
        }
        if let Some(last) = self.last_draft_run(kwant_id, class).await? {
            if last >= now - Duration::days(DRAFT_COOL_OFF_DAYS) {
                return Ok(Some("cooling_off"));
            }
        }
        Ok(None)
    }

    pub async fn last_draft_run(&self, kwant_id: &str, class: &str) -> Result<Option<DateTime<Utc>>, DbError> {
        let (at,): (Option<DateTime<Utc>>,) =
            sqlx::query_as("SELECT MAX(created_at) FROM foundry_runs WHERE kind = 'draft' AND kwant_id = ? AND task_class = ?")
                .bind(kwant_id)
                .bind(class)
                .fetch_one(&self.pool)
                .await?;
        Ok(at)
    }

    /// Draft runs started since `since` (the daily cap counts these).
    pub async fn draft_runs_since(&self, since: DateTime<Utc>) -> Result<i64, DbError> {
        let (n,): (i64,) = sqlx::query_as("SELECT COUNT(*) FROM foundry_runs WHERE kind = 'draft' AND created_at >= ?")
            .bind(since)
            .fetch_one(&self.pool)
            .await?;
        Ok(n)
    }

    pub async fn create_foundry_run(&self, kind: &str, space_id: &str, kwant_id: &str, class: &str, skill_id: Option<&str>) -> Result<FoundryRun, DbError> {
        Ok(sqlx::query_as::<_, FoundryRun>(
            "INSERT INTO foundry_runs (id, kind, space_id, kwant_id, task_class, skill_id, status, created_at)
             VALUES (?, ?, ?, ?, ?, ?, 'queued', ?) RETURNING *",
        )
        .bind(uuid::Uuid::new_v4().to_string())
        .bind(kind)
        .bind(space_id)
        .bind(kwant_id)
        .bind(class)
        .bind(skill_id)
        .bind(Utc::now())
        .fetch_one(&self.pool)
        .await?)
    }

    pub async fn get_foundry_run(&self, id: &str) -> Result<Option<FoundryRun>, DbError> {
        Ok(sqlx::query_as::<_, FoundryRun>("SELECT * FROM foundry_runs WHERE id = ?").bind(id).fetch_optional(&self.pool).await?)
    }

    /// Runs in `status`, oldest first.
    pub async fn foundry_runs_in(&self, status: &str) -> Result<Vec<FoundryRun>, DbError> {
        Ok(sqlx::query_as::<_, FoundryRun>("SELECT * FROM foundry_runs WHERE status = ? ORDER BY created_at, rowid")
            .bind(status)
            .fetch_all(&self.pool)
            .await?)
    }

    /// Recent runs of a space, newest first.
    pub async fn foundry_runs_of_space(&self, space_id: &str, limit: i64) -> Result<Vec<FoundryRun>, DbError> {
        Ok(sqlx::query_as::<_, FoundryRun>("SELECT * FROM foundry_runs WHERE space_id = ? ORDER BY created_at DESC, rowid DESC LIMIT ?")
            .bind(space_id)
            .bind(limit)
            .fetch_all(&self.pool)
            .await?)
    }

    /// The draft run `kwant_id` is in now, if any: the only time it is offered `skill_draft`.
    pub async fn drafting_run_of(&self, kwant_id: &str) -> Result<Option<FoundryRun>, DbError> {
        Ok(sqlx::query_as::<_, FoundryRun>("SELECT * FROM foundry_runs WHERE kind = 'draft' AND status = 'drafting' AND kwant_id = ? ORDER BY created_at DESC LIMIT 1")
            .bind(kwant_id)
            .fetch_optional(&self.pool)
            .await?)
    }

    /// A live (queued or running) run of `kind` for this cluster or skill.
    pub async fn live_foundry_run(&self, kind: &str, kwant_id: &str, class: &str, skill_id: Option<&str>) -> Result<Option<FoundryRun>, DbError> {
        Ok(sqlx::query_as::<_, FoundryRun>(
            "SELECT * FROM foundry_runs WHERE kind = ? AND kwant_id = ? AND task_class = ? AND (?4 IS NULL OR skill_id = ?4)
             AND status IN ('queued', 'drafting', 'replaying') LIMIT 1",
        )
        .bind(kind)
        .bind(kwant_id)
        .bind(class)
        .bind(skill_id)
        .fetch_optional(&self.pool)
        .await?)
    }

    /// Moves a run from `from` to `to`; false when it wasn't in `from` any more. A final status
    /// sets `finished_at`.
    pub async fn move_foundry_run(&self, id: &str, from: &str, to: &str, reason: Option<&str>) -> Result<bool, DbError> {
        let finished = matches!(to, "done" | "failed" | "interrupted").then(Utc::now);
        let r = sqlx::query("UPDATE foundry_runs SET status = ?, reason = COALESCE(?, reason), finished_at = COALESCE(?, finished_at) WHERE id = ? AND status = ?")
            .bind(to)
            .bind(reason)
            .bind(finished)
            .bind(id)
            .bind(from)
            .execute(&self.pool)
            .await?;
        Ok(r.rows_affected() == 1)
    }

    pub async fn add_foundry_tokens(&self, id: &str, tokens: i64) -> Result<(), DbError> {
        sqlx::query("UPDATE foundry_runs SET tokens = tokens + ? WHERE id = ?").bind(tokens.max(0)).bind(id).execute(&self.pool).await?;
        Ok(())
    }

    pub async fn set_foundry_run_skill(&self, id: &str, skill_id: &str) -> Result<(), DbError> {
        sqlx::query("UPDATE foundry_runs SET skill_id = ? WHERE id = ?").bind(skill_id).bind(id).execute(&self.pool).await?;
        Ok(())
    }

    pub async fn skill_meta(&self, skill_id: &str) -> Result<Option<SkillMeta>, DbError> {
        Ok(sqlx::query_as::<_, SkillMeta>("SELECT * FROM skill_meta WHERE skill_id = ?").bind(skill_id).fetch_optional(&self.pool).await?)
    }

    pub async fn set_skill_replay(&self, skill_id: &str, report: &Value) -> Result<(), DbError> {
        sqlx::query("UPDATE skill_meta SET replay = ? WHERE skill_id = ?").bind(report.to_string()).bind(skill_id).execute(&self.pool).await?;
        Ok(())
    }

    /// Every version of a lineage, oldest first, with its skill.
    pub async fn lineage(&self, lineage_id: &str) -> Result<Vec<(Skill, SkillMeta)>, DbError> {
        let metas = sqlx::query_as::<_, SkillMeta>("SELECT * FROM skill_meta WHERE lineage_id = ? ORDER BY version, created_at")
            .bind(lineage_id)
            .fetch_all(&self.pool)
            .await?;
        let mut out = Vec::new();
        for m in metas {
            if let Some(s) = self.get_skill(&m.skill_id).await? {
                out.push((s, m));
            }
        }
        Ok(out)
    }

    /// Foundry skills of a space (`pending` ones, or all), newest first.
    pub async fn foundry_skills(&self, space_id: &str, status: Option<&str>) -> Result<Vec<(Skill, SkillMeta)>, DbError> {
        let ids: Vec<(String,)> = sqlx::query_as(
            "SELECT s.id FROM skills s JOIN skill_meta m ON m.skill_id = s.id
             WHERE m.space_id = ? AND (?2 IS NULL OR s.status = ?2) ORDER BY s.created_at DESC LIMIT 200",
        )
        .bind(space_id)
        .bind(status)
        .fetch_all(&self.pool)
        .await?;
        let mut out = Vec::new();
        for (id,) in ids {
            if let (Some(s), Some(m)) = (self.get_skill(&id).await?, self.skill_meta(&id).await?) {
                out.push((s, m));
            }
        }
        Ok(out)
    }

    pub async fn log_skill_event(&self, skill_id: &str, kind: &str, actor: &str, detail: &Value) -> Result<(), DbError> {
        sqlx::query("INSERT INTO skill_events (skill_id, kind, actor, detail, created_at) VALUES (?, ?, ?, ?, ?)")
            .bind(skill_id)
            .bind(kind)
            .bind(actor)
            .bind(detail.to_string())
            .bind(Utc::now())
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    pub async fn skill_events(&self, skill_ids: &[String]) -> Result<Vec<SkillEvent>, DbError> {
        let mut out = Vec::new();
        for id in skill_ids {
            out.extend(
                sqlx::query_as::<_, SkillEvent>("SELECT * FROM skill_events WHERE skill_id = ? ORDER BY id")
                    .bind(id)
                    .fetch_all(&self.pool)
                    .await?,
            );
        }
        out.sort_by_key(|e| e.id);
        Ok(out)
    }

    /// Stores a Foundry draft as a `pending` skill with its meta and a `skill_drafted` event, in
    /// one transaction. When the bot has a live Foundry skill for the class, the draft is its next
    /// version: same lineage, and approving it supersedes the live one.
    pub async fn insert_foundry_draft(&self, space_id: &str, d: &NewDraft, run_id: &str) -> Result<(Skill, SkillMeta), SkillError> {
        let now = Utc::now();
        let mut tx = self.pool.begin().await?;
        let (pending,): (i64,) = sqlx::query_as("SELECT COUNT(*) FROM skills WHERE status = 'pending' AND kwant_id = ?")
            .bind(&d.kwant_id)
            .fetch_one(&mut *tx)
            .await?;
        if pending >= MAX_PENDING_PER_KWANT {
            return Err(SkillError::TooManyPending);
        }
        let waiting: Option<(String,)> = sqlx::query_as(
            "SELECT s.id FROM skills s JOIN skill_meta m ON m.skill_id = s.id WHERE m.kwant_id = ? AND m.task_class = ? AND s.status = 'pending'",
        )
        .bind(&d.kwant_id)
        .bind(&d.task_class)
        .fetch_optional(&mut *tx)
        .await?;
        if waiting.is_some() {
            return Err(SkillError::Conflict("a Foundry draft for this class is already waiting for review".into()));
        }
        let live: Option<(String, String)> = sqlx::query_as(
            "SELECT s.id, m.lineage_id FROM skills s JOIN skill_meta m ON m.skill_id = s.id
             WHERE m.kwant_id = ? AND m.task_class = ? AND s.status IN ('enabled', 'disabled') ORDER BY m.version DESC LIMIT 1",
        )
        .bind(&d.kwant_id)
        .bind(&d.task_class)
        .fetch_optional(&mut *tx)
        .await?;
        let id = uuid::Uuid::new_v4().to_string();
        let (replaces, lineage, version) = match &live {
            Some((live_id, lineage)) => {
                let (v,): (i64,) = sqlx::query_as("SELECT IFNULL(MAX(version), 0) FROM skill_meta WHERE lineage_id = ?")
                    .bind(lineage)
                    .fetch_one(&mut *tx)
                    .await?;
                (Some(live_id.clone()), lineage.clone(), v + 1)
            }
            None => (None, id.clone(), 1),
        };
        sqlx::query(
            "INSERT INTO skills (id, kwant_id, name, description, body, status, source, proposed_by, replaces_id, created_at, updated_at, space_id)
             VALUES (?, ?, ?, ?, ?, 'pending', 'agent', ?, ?, ?, ?, ?)",
        )
        .bind(&id)
        .bind(&d.kwant_id)
        .bind(&d.text.name)
        .bind(&d.text.description)
        .bind(&d.text.body)
        .bind(&d.kwant_id)
        .bind(&replaces)
        .bind(now)
        .bind(now)
        .bind(space_id)
        .execute(&mut *tx)
        .await?;
        sqlx::query(
            "INSERT INTO skill_meta (skill_id, space_id, kwant_id, task_class, inputs, checks, examples, tools, version, lineage_id, created_at)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(&id)
        .bind(space_id)
        .bind(&d.kwant_id)
        .bind(&d.task_class)
        .bind(d.inputs.to_string())
        .bind(d.checks.to_string())
        .bind(serde_json::to_string(&d.examples).unwrap_or_else(|_| "[]".into()))
        .bind(serde_json::to_string(&d.tools).unwrap_or_else(|_| "[]".into()))
        .bind(version)
        .bind(&lineage)
        .bind(now)
        .execute(&mut *tx)
        .await?;
        let detail = serde_json::json!({"run_id": run_id, "version": version, "replaces": replaces, "examples": d.examples});
        sqlx::query("INSERT INTO skill_events (skill_id, kind, actor, detail, created_at) VALUES (?, 'skill_drafted', ?, ?, ?)")
            .bind(&id)
            .bind(&d.kwant_id)
            .bind(detail.to_string())
            .bind(now)
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        let skill = self.get_skill(&id).await?.ok_or(SkillError::NotFound)?;
        let meta = self.skill_meta(&id).await?.ok_or(SkillError::NotFound)?;
        Ok((skill, meta))
    }

    /// Rolls a lineage back to `target_id`, a `superseded` version of it: the lineage's live
    /// version becomes `superseded` and the target `enabled`, with a `skill_rolled_back` event, in
    /// one transaction. Only within a Foundry lineage; a live skill outside it with the target's
    /// name blocks it.
    pub async fn rollback_skill(&self, target_id: &str, actor: &str) -> Result<Skill, SkillError> {
        let now = Utc::now();
        let mut tx = self.pool.begin().await?;
        let target = sqlx::query_as::<_, Skill>("SELECT * FROM skills WHERE id = ?").bind(target_id).fetch_optional(&mut *tx).await?.ok_or(SkillError::NotFound)?;
        let meta: Option<(String,)> = sqlx::query_as("SELECT lineage_id FROM skill_meta WHERE skill_id = ?").bind(target_id).fetch_optional(&mut *tx).await?;
        let Some((lineage,)) = meta else {
            return Err(SkillError::Conflict("only a Foundry skill version can be rolled back to".into()));
        };
        if target.status != "superseded" {
            return Err(SkillError::Conflict(format!("only an earlier (superseded) version can be rolled back to; this one is {}", target.status)));
        }
        let current: Vec<(String,)> = sqlx::query_as(
            "SELECT s.id FROM skills s JOIN skill_meta m ON m.skill_id = s.id WHERE m.lineage_id = ? AND s.status IN ('enabled', 'disabled')",
        )
        .bind(&lineage)
        .fetch_all(&mut *tx)
        .await?;
        for (id,) in &current {
            sqlx::query("UPDATE skills SET status = 'superseded', updated_at = ? WHERE id = ?").bind(now).bind(id).execute(&mut *tx).await?;
        }
        let clash: Option<(String,)> = sqlx::query_as(
            "SELECT id FROM skills WHERE space_id = ? AND IFNULL(kwant_id, '') = IFNULL(?, '') AND name = ? AND status IN ('enabled', 'disabled') AND id != ?",
        )
        .bind(&target.space_id)
        .bind(&target.kwant_id)
        .bind(&target.name)
        .bind(target_id)
        .fetch_optional(&mut *tx)
        .await?;
        if clash.is_some() {
            return Err(SkillError::Conflict(format!("another skill called {} is live; rename or disable it first", target.name)));
        }
        sqlx::query("UPDATE skills SET status = 'enabled', updated_at = ?, reviewed_at = ? WHERE id = ?")
            .bind(now)
            .bind(now)
            .bind(target_id)
            .execute(&mut *tx)
            .await?;
        let from: Vec<&String> = current.iter().map(|c| &c.0).collect();
        let detail = serde_json::json!({"from": from, "to": target_id});
        for id in current.iter().map(|c| c.0.as_str()).chain([target_id]) {
            sqlx::query("INSERT INTO skill_events (skill_id, kind, actor, detail, created_at) VALUES (?, 'skill_rolled_back', ?, ?, ?)")
                .bind(id)
                .bind(actor)
                .bind(detail.to_string())
                .bind(now)
                .execute(&mut *tx)
                .await?;
        }
        tx.commit().await?;
        self.get_skill(target_id).await?.ok_or(SkillError::NotFound)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn db() -> (Database, tempfile::TempDir) {
        let tmp = tempfile::tempdir().unwrap();
        (Database::init(tmp.path()).await.unwrap(), tmp)
    }

    #[test]
    fn cluster_ids_round_trip() {
        assert_eq!(split_cluster(&cluster_id("k1", "release-notes")), Some(("k1", "release-notes")));
        assert_eq!(split_cluster("nope"), None);
        assert_eq!(split_cluster("~x"), None);
    }

    #[tokio::test]
    async fn a_restart_marks_running_runs_interrupted_and_keeps_queued_ones() {
        let (db, _t) = db().await;
        let a = db.create_foundry_run("draft", "spc_default", "k", "c", None).await.unwrap();
        assert!(db.move_foundry_run(&a.id, "queued", "drafting", None).await.unwrap());
        let q = db.create_foundry_run("replay", "spc_default", "k", "c", Some("s")).await.unwrap();
        db.migrate_foundry().await.unwrap();
        let a = db.get_foundry_run(&a.id).await.unwrap().unwrap();
        assert_eq!((a.status.as_str(), a.finished_at.is_some()), ("interrupted", true));
        assert_eq!(db.get_foundry_run(&q.id).await.unwrap().unwrap().status, "queued");
    }
}
