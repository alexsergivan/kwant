//! Mission Control Phase C3: Flight School's bookkeeping. A task's class (`tasks.task_class`), the
//! tier the person gave a bot for a class (`capability_policies`, one live row per bot and class,
//! `'*'` for every class without its own) and what happened to those tiers (`capability_events`,
//! append-only). Only person-only routes write policies; the gate reads them on every call.
//! Contract: docs/mission-control.md §14.

use super::{Database, DbError};
use chrono::Utc;
use serde::Serialize;
use sqlx::FromRow;
use uuid::Uuid;

pub const TIERS: [&str; 3] = ["observe", "sandbox", "act"];
/// The class of every task nobody gave one.
pub const DEFAULT_CLASS: &str = "general";
/// The policy row that covers every class without its own.
pub const ANY_CLASS: &str = "*";

/// Undoes [`Database::migrate_flight_school`]: the tables go and `tasks` gets back the shape older
/// releases read. Kept for rollbacks and tested.
pub const FLIGHT_SCHOOL_DOWN: &str = "DROP TRIGGER IF EXISTS tier_proposals_no_delete; \
    DROP TRIGGER IF EXISTS tier_proposals_only_decide; \
    DROP INDEX IF EXISTS tier_proposals_one_open; \
    DROP TABLE IF EXISTS tier_proposals; \
    DROP TRIGGER IF EXISTS kwants_flight_school_default; \
    DROP TRIGGER IF EXISTS capability_policies_no_delete; \
    DROP TRIGGER IF EXISTS capability_policies_only_revoke; \
    DROP TRIGGER IF EXISTS capability_events_no_update; \
    DROP TRIGGER IF EXISTS capability_events_no_delete; \
    DROP TABLE IF EXISTS capability_events; \
    DROP INDEX IF EXISTS capability_live; \
    DROP TABLE IF EXISTS capability_policies; \
    ALTER TABLE tasks DROP COLUMN task_class";

#[derive(Debug, Clone, Serialize, FromRow, PartialEq)]
pub struct Policy {
    pub id: String,
    pub kwant_id: String,
    pub task_class: String,
    pub tier: String,
    pub granted_by: String,
    pub granted_at: String,
    pub revoked_at: Option<String>,
    pub note: Option<String>,
}

#[derive(Debug, Clone, Serialize, FromRow, PartialEq)]
pub struct CapabilityEvent {
    pub id: i64,
    pub kwant_id: String,
    pub task_class: String,
    pub kind: String,
    pub tier: Option<String>,
    pub actor: String,
    pub detail: Option<String>,
    pub created_at: String,
}

/// A bot's request to move up one tier for a class (C4). Only the person grants or declines it;
/// kwant marks it `stale` when the bot's tier or scorecard no longer matches what it asked from.
#[derive(Debug, Clone, Serialize, FromRow, PartialEq)]
pub struct TierProposal {
    pub id: String,
    pub space_id: String,
    pub kwant_id: String,
    pub task_class: String,
    pub from_tier: String,
    pub to_tier: String,
    /// The scorecard kwant computed when the bot asked (JSON), never the bot's own text.
    pub scorecard: String,
    /// The bot's reason, shown as the bot's words.
    pub reason: Option<String>,
    pub status: String,
    pub decided_at: Option<String>,
    /// Why kwant marked it stale, or the person's note on a decline.
    pub note: Option<String>,
    pub created_at: String,
}

/// One settled task of a bot in a class, with what the scorecard counts about it.
#[derive(Debug, Clone, FromRow, PartialEq)]
pub struct ScoreTask {
    pub id: String,
    pub state: String,
    pub attempts: i64,
    pub finished_at: chrono::DateTime<Utc>,
    pub failures: i64,
    pub denied_failures: i64,
    pub check_failures: i64,
    pub calls: i64,
    pub external_calls: i64,
    pub cards: i64,
    pub approved: i64,
    pub denied: i64,
    pub false_success: i64,
    pub interventions: i64,
    pub spent_tokens: i64,
    pub recovery_tokens: i64,
}

/// What a grant or decline found.
#[derive(Debug, Clone, PartialEq)]
pub enum Decided {
    Done(Box<TierProposal>),
    /// It wasn't `proposed` any more: its status now.
    NotOpen(String),
    Missing,
}

/// `observe` < `sandbox` < `act`; an unknown tier is the lowest.
pub fn rank(tier: &str) -> usize {
    TIERS.iter().position(|t| *t == tier).unwrap_or(0)
}

/// A class slug: `[a-z0-9-]{1,40}` (D10).
pub fn valid_class(class: &str) -> bool {
    (1..=40).contains(&class.len()) && class.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
}

const TRIGGERS: [&str; 7] = [
    // A proposal is decided once: only `proposed` moves, to a final status, and only its status,
    // decision time and note change
    "CREATE TRIGGER IF NOT EXISTS tier_proposals_only_decide BEFORE UPDATE ON tier_proposals \
     WHEN OLD.status != 'proposed' OR NEW.status = 'proposed' OR NEW.id IS NOT OLD.id OR NEW.kwant_id IS NOT OLD.kwant_id \
       OR NEW.space_id IS NOT OLD.space_id OR NEW.task_class IS NOT OLD.task_class OR NEW.from_tier IS NOT OLD.from_tier \
       OR NEW.to_tier IS NOT OLD.to_tier OR NEW.scorecard IS NOT OLD.scorecard OR NEW.reason IS NOT OLD.reason \
       OR NEW.created_at IS NOT OLD.created_at OR NEW.decided_at IS NULL \
     BEGIN SELECT RAISE(ABORT, 'a tier proposal is decided once'); END",
    "CREATE TRIGGER IF NOT EXISTS tier_proposals_no_delete BEFORE DELETE ON tier_proposals \
     BEGIN SELECT RAISE(ABORT, 'tier proposals are kept'); END",
    // A tier row is never deleted or rewritten: revoking sets `revoked_at` once, and a new row
    // carries the new tier
    "CREATE TRIGGER IF NOT EXISTS capability_policies_no_delete BEFORE DELETE ON capability_policies \
     BEGIN SELECT RAISE(ABORT, 'tier history is kept'); END",
    "CREATE TRIGGER IF NOT EXISTS capability_policies_only_revoke BEFORE UPDATE ON capability_policies \
     WHEN OLD.revoked_at IS NOT NULL OR NEW.revoked_at IS NULL OR NEW.id IS NOT OLD.id OR NEW.kwant_id IS NOT OLD.kwant_id \
       OR NEW.task_class IS NOT OLD.task_class OR NEW.tier IS NOT OLD.tier OR NEW.granted_by IS NOT OLD.granted_by \
       OR NEW.granted_at IS NOT OLD.granted_at \
     BEGIN SELECT RAISE(ABORT, 'a tier can only be revoked'); END",
    "CREATE TRIGGER IF NOT EXISTS capability_events_no_update BEFORE UPDATE ON capability_events \
     BEGIN SELECT RAISE(ABORT, 'capability events are append-only'); END",
    "CREATE TRIGGER IF NOT EXISTS capability_events_no_delete BEFORE DELETE ON capability_events \
     BEGIN SELECT RAISE(ABORT, 'capability events are append-only'); END",
    // A bot hired while Flight School is on starts at the person's default for new bots (§3.6, D4).
    // Existing bots have no row, which is `act`: switching the feature on changes nothing for them.
    "CREATE TRIGGER IF NOT EXISTS kwants_flight_school_default AFTER INSERT ON kwants \
     WHEN (SELECT value FROM settings WHERE key = 'flight_school') = 'true' \
     BEGIN \
       INSERT INTO capability_policies (id, kwant_id, task_class, tier, granted_by, granted_at, note) \
       VALUES (lower(hex(randomblob(16))), NEW.id, '*', \
         COALESCE((SELECT value FROM settings WHERE key = 'flight_school_new_bot_tier' AND value IN ('observe', 'sandbox', 'act')), 'sandbox'), \
         'person', strftime('%Y-%m-%dT%H:%M:%fZ', 'now'), 'new bot default'); \
       INSERT INTO capability_events (kwant_id, task_class, kind, tier, actor, detail, created_at) \
       SELECT NEW.id, '*', 'new_bot_default', tier, 'person', NULL, granted_at FROM capability_policies \
       WHERE kwant_id = NEW.id AND task_class = '*' AND revoked_at IS NULL; \
     END",
];

impl Database {
    pub(super) async fn migrate_flight_school(&self) -> Result<(), DbError> {
        // Additive only: one new column with a default, new tables, new triggers
        let has_class: Option<(String,)> =
            sqlx::query_as("SELECT name FROM pragma_table_info('tasks') WHERE name = 'task_class'").fetch_optional(&self.pool).await?;
        if has_class.is_none() {
            sqlx::query("ALTER TABLE tasks ADD COLUMN task_class TEXT NOT NULL DEFAULT 'general'").execute(&self.pool).await?;
        }
        for stmt in [
            "CREATE TABLE IF NOT EXISTS capability_policies (
                id TEXT PRIMARY KEY,
                kwant_id TEXT NOT NULL,
                task_class TEXT NOT NULL,
                tier TEXT NOT NULL CHECK (tier IN ('observe', 'sandbox', 'act')),
                granted_by TEXT NOT NULL CHECK (granted_by = 'person'),
                granted_at TEXT NOT NULL,
                revoked_at TEXT,
                note TEXT
            )",
            "CREATE UNIQUE INDEX IF NOT EXISTS capability_live ON capability_policies (kwant_id, task_class) WHERE revoked_at IS NULL",
            "CREATE TABLE IF NOT EXISTS capability_events (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                kwant_id TEXT NOT NULL,
                task_class TEXT NOT NULL,
                kind TEXT NOT NULL,
                tier TEXT,
                actor TEXT NOT NULL,
                detail TEXT,
                created_at TEXT NOT NULL
            )",
            "CREATE INDEX IF NOT EXISTS capability_events_kwant ON capability_events (kwant_id, id)",
            "CREATE TABLE IF NOT EXISTS tier_proposals (
                id TEXT PRIMARY KEY,
                space_id TEXT NOT NULL,
                kwant_id TEXT NOT NULL,
                task_class TEXT NOT NULL,
                from_tier TEXT NOT NULL CHECK (from_tier IN ('observe', 'sandbox')),
                to_tier TEXT NOT NULL CHECK (to_tier IN ('sandbox', 'act')),
                scorecard TEXT NOT NULL,
                reason TEXT,
                status TEXT NOT NULL CHECK (status IN ('proposed', 'granted', 'declined', 'stale')),
                decided_at TEXT,
                note TEXT,
                created_at TEXT NOT NULL
            )",
            "CREATE UNIQUE INDEX IF NOT EXISTS tier_proposals_one_open ON tier_proposals (kwant_id, task_class) WHERE status = 'proposed'",
            "CREATE INDEX IF NOT EXISTS tier_proposals_space ON tier_proposals (space_id, status, created_at)",
        ] {
            sqlx::query(stmt).execute(&self.pool).await?;
        }
        for t in TRIGGERS {
            sqlx::query(t).execute(&self.pool).await?;
        }
        Ok(())
    }

    /// The task's class; `general` for a task that isn't there.
    pub async fn task_class(&self, task_id: &str) -> Result<String, DbError> {
        let row: Option<(String,)> = sqlx::query_as("SELECT task_class FROM tasks WHERE id = ?").bind(task_id).fetch_optional(&self.pool).await?;
        Ok(row.map_or_else(|| DEFAULT_CLASS.to_string(), |r| r.0))
    }

    /// Sets a task's class and records a `class_changed` task event. Callers validate the slug and
    /// who may set it (the lead or the person, never the task's own bot).
    pub async fn set_task_class(&self, task_id: &str, class: &str, actor: &str) -> Result<bool, DbError> {
        let mut tx = self.pool.begin().await?;
        let row: Option<(String, Option<String>)> =
            sqlx::query_as("SELECT task_class, mission_id FROM tasks WHERE id = ?").bind(task_id).fetch_optional(&mut *tx).await?;
        let Some((old, mission)) = row else { return Ok(false) };
        if old == class {
            return Ok(true);
        }
        sqlx::query("UPDATE tasks SET task_class = ?, updated_at = ? WHERE id = ?").bind(class).bind(Utc::now()).bind(task_id).execute(&mut *tx).await?;
        super::missions::insert_event(&mut tx, task_id, mission.as_deref(), "class_changed", actor, None, None, &serde_json::json!({"from": old, "to": class}))
            .await?;
        tx.commit().await?;
        Ok(true)
    }

    /// The live policy for `kwant_id` on `class`: its own row, else the `'*'` row. `None`: no row,
    /// which means `act`.
    pub async fn live_policy(&self, kwant_id: &str, class: &str) -> Result<Option<Policy>, DbError> {
        Ok(sqlx::query_as::<_, Policy>(
            "SELECT * FROM capability_policies WHERE kwant_id = ? AND revoked_at IS NULL AND task_class IN (?, '*') \
             ORDER BY task_class = '*' LIMIT 1",
        )
        .bind(kwant_id)
        .bind(class)
        .fetch_optional(&self.pool)
        .await?)
    }

    /// Every live policy of a bot, by class.
    pub async fn policies_of(&self, kwant_id: &str) -> Result<Vec<Policy>, DbError> {
        Ok(sqlx::query_as::<_, Policy>("SELECT * FROM capability_policies WHERE kwant_id = ? AND revoked_at IS NULL ORDER BY task_class")
            .bind(kwant_id)
            .fetch_all(&self.pool)
            .await?)
    }

    /// The classes a bot's tasks have, for the capabilities view.
    pub async fn classes_of(&self, kwant_id: &str) -> Result<Vec<String>, DbError> {
        let rows: Vec<(String,)> = sqlx::query_as(
            "SELECT DISTINCT task_class FROM tasks WHERE owner_kwant_id = ?1 OR suggested_kwant_id = ?1 ORDER BY task_class",
        )
        .bind(kwant_id)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows.into_iter().map(|r| r.0).collect())
    }

    pub async fn capability_events(&self, kwant_id: &str, limit: i64) -> Result<Vec<CapabilityEvent>, DbError> {
        Ok(sqlx::query_as::<_, CapabilityEvent>("SELECT * FROM capability_events WHERE kwant_id = ? ORDER BY id DESC LIMIT ?")
            .bind(kwant_id)
            .bind(limit)
            .fetch_all(&self.pool)
            .await?)
    }

    /// The person sets `tier` for a bot on `class` (`'*'` for every class without its own): the live
    /// row, if any, is revoked and the new one written in one transaction, with an event of `kind`
    /// (`granted` or `revoked`). The unique index keeps one live row per bot and class.
    pub async fn set_policy(&self, kwant_id: &str, class: &str, tier: &str, kind: &str, note: Option<&str>) -> Result<Policy, DbError> {
        let now = Utc::now().to_rfc3339();
        let mut tx = self.pool.begin().await?;
        sqlx::query("UPDATE capability_policies SET revoked_at = ? WHERE kwant_id = ? AND task_class = ? AND revoked_at IS NULL")
            .bind(&now)
            .bind(kwant_id)
            .bind(class)
            .execute(&mut *tx)
            .await?;
        let policy = sqlx::query_as::<_, Policy>(
            "INSERT INTO capability_policies (id, kwant_id, task_class, tier, granted_by, granted_at, note) VALUES (?, ?, ?, ?, 'person', ?, ?) RETURNING *",
        )
        .bind(Uuid::new_v4().to_string())
        .bind(kwant_id)
        .bind(class)
        .bind(tier)
        .bind(&now)
        .bind(note)
        .fetch_one(&mut *tx)
        .await?;
        sqlx::query("INSERT INTO capability_events (kwant_id, task_class, kind, tier, actor, detail, created_at) VALUES (?, ?, ?, ?, 'person', ?, ?)")
            .bind(kwant_id)
            .bind(class)
            .bind(kind)
            .bind(tier)
            .bind(note)
            .bind(&now)
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        Ok(policy)
    }

    /// A bot's settled tasks of `class` that ran (had a claim), finished since `since`, newest first,
    /// with everything the scorecard counts. Every number comes from rows kwant wrote itself: the
    /// task, its failures, its tool-call ledger, its flags and the person's moves on it.
    pub async fn score_tasks(&self, kwant_id: &str, class: &str, since: chrono::DateTime<Utc>) -> Result<Vec<ScoreTask>, DbError> {
        Ok(sqlx::query_as::<_, ScoreTask>(
            "SELECT t.id, t.state, t.attempts, t.finished_at, \
               (SELECT COUNT(*) FROM task_failures f WHERE f.task_id = t.id) AS failures, \
               (SELECT COUNT(*) FROM task_failures f WHERE f.task_id = t.id AND f.class = 'denied') AS denied_failures, \
               (SELECT COUNT(*) FROM task_failures f WHERE f.task_id = t.id AND f.class = 'check_failed') AS check_failures, \
               (SELECT COUNT(*) FROM turn_tool_calls c WHERE c.task_id = t.id) AS calls, \
               (SELECT COUNT(*) FROM turn_tool_calls c WHERE c.task_id = t.id AND c.effect = 'external') AS external_calls, \
               (SELECT COUNT(*) FROM turn_tool_calls c WHERE c.task_id = t.id AND c.approval_id IS NOT NULL) AS cards, \
               (SELECT COUNT(*) FROM turn_tool_calls c WHERE c.task_id = t.id AND c.decision = 'approved') AS approved, \
               (SELECT COUNT(*) FROM turn_tool_calls c WHERE c.task_id = t.id AND c.decision = 'denied') AS denied, \
               (t.state = 'verified' AND EXISTS (SELECT 1 FROM task_flags g WHERE g.task_id = t.id AND g.cause = 'evidence_changed' \
                  AND (g.resolution IS NULL OR g.resolution != 'still_valid'))) AS false_success, \
               (SELECT COUNT(*) FROM task_events e WHERE e.task_id = t.id AND e.actor = 'person' \
                  AND e.kind IN ('state', 'edited', 'forked', 'check_added', 'flag_resolved') AND (e.to_state IS NULL OR e.to_state != 'verified')) AS interventions, \
               t.spent_tokens, t.recovery_spent_tokens AS recovery_tokens \
             FROM tasks t \
             WHERE t.owner_kwant_id = ? AND t.task_class = ? AND t.state IN ('verified', 'failed', 'cancelled') \
               AND t.finished_at IS NOT NULL AND t.finished_at >= ? \
               AND EXISTS (SELECT 1 FROM agent_claims a WHERE a.task_id = t.id) \
             ORDER BY t.finished_at DESC, t.id",
        )
        .bind(kwant_id)
        .bind(class)
        .bind(since)
        .fetch_all(&self.pool)
        .await?)
    }

    pub async fn tier_proposal(&self, id: &str) -> Result<Option<TierProposal>, DbError> {
        Ok(sqlx::query_as::<_, TierProposal>("SELECT * FROM tier_proposals WHERE id = ?").bind(id).fetch_optional(&self.pool).await?)
    }

    /// Proposals in a space (optionally one bot's, optionally one status), newest first.
    pub async fn tier_proposals(&self, space_id: &str, kwant_id: Option<&str>, status: Option<&str>, limit: i64) -> Result<Vec<TierProposal>, DbError> {
        Ok(sqlx::query_as::<_, TierProposal>(
            "SELECT * FROM tier_proposals WHERE space_id = ? AND (?2 IS NULL OR kwant_id = ?2) AND (?3 IS NULL OR status = ?3) \
             ORDER BY created_at DESC, id LIMIT ?4",
        )
        .bind(space_id)
        .bind(kwant_id)
        .bind(status)
        .bind(limit)
        .fetch_all(&self.pool)
        .await?)
    }

    /// A bot's proposals, newest first.
    pub async fn tier_proposals_of(&self, kwant_id: &str, limit: i64) -> Result<Vec<TierProposal>, DbError> {
        Ok(sqlx::query_as::<_, TierProposal>("SELECT * FROM tier_proposals WHERE kwant_id = ? ORDER BY created_at DESC, id LIMIT ?")
            .bind(kwant_id)
            .bind(limit)
            .fetch_all(&self.pool)
            .await?)
    }

    /// The bot's open proposal for `class`, if any.
    pub async fn open_tier_proposal(&self, kwant_id: &str, class: &str) -> Result<Option<TierProposal>, DbError> {
        Ok(sqlx::query_as::<_, TierProposal>("SELECT * FROM tier_proposals WHERE kwant_id = ? AND task_class = ? AND status = 'proposed'")
            .bind(kwant_id)
            .bind(class)
            .fetch_optional(&self.pool)
            .await?)
    }

    /// When the person last declined the bot moving to `to_tier` on `class`.
    pub async fn last_declined(&self, kwant_id: &str, class: &str, to_tier: &str) -> Result<Option<String>, DbError> {
        let row: Option<(String,)> = sqlx::query_as(
            "SELECT decided_at FROM tier_proposals WHERE kwant_id = ? AND task_class = ? AND to_tier = ? AND status = 'declined' ORDER BY decided_at DESC LIMIT 1",
        )
        .bind(kwant_id)
        .bind(class)
        .bind(to_tier)
        .fetch_optional(&self.pool)
        .await?;
        Ok(row.map(|r| r.0))
    }

    /// Saves a bot's proposal. `Ok(Err(open))` when the bot already has one open for the class
    /// (one per bot and class, by the unique index; a race lands here too).
    #[allow(clippy::too_many_arguments)]
    pub async fn create_tier_proposal(
        &self,
        space_id: &str,
        kwant_id: &str,
        class: &str,
        from_tier: &str,
        to_tier: &str,
        scorecard: &serde_json::Value,
        reason: Option<&str>,
    ) -> Result<Result<TierProposal, TierProposal>, DbError> {
        let now = Utc::now().to_rfc3339();
        let mut tx = self.pool.begin().await?;
        let r = sqlx::query_as::<_, TierProposal>(
            "INSERT INTO tier_proposals (id, space_id, kwant_id, task_class, from_tier, to_tier, scorecard, reason, status, created_at) \
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, 'proposed', ?) ON CONFLICT DO NOTHING RETURNING *",
        )
        .bind(Uuid::new_v4().to_string())
        .bind(space_id)
        .bind(kwant_id)
        .bind(class)
        .bind(from_tier)
        .bind(to_tier)
        .bind(scorecard.to_string())
        .bind(reason)
        .bind(&now)
        .fetch_optional(&mut *tx)
        .await?;
        match r {
            Some(p) => {
                sqlx::query("INSERT INTO capability_events (kwant_id, task_class, kind, tier, actor, detail, created_at) VALUES (?, ?, 'proposed', ?, ?, ?, ?)")
                    .bind(kwant_id)
                    .bind(class)
                    .bind(to_tier)
                    .bind(kwant_id)
                    .bind(&p.id)
                    .bind(&now)
                    .execute(&mut *tx)
                    .await?;
                tx.commit().await?;
                Ok(Ok(p))
            }
            None => {
                drop(tx);
                match self.open_tier_proposal(kwant_id, class).await? {
                    Some(open) => Ok(Err(open)),
                    None => Err(DbError::Sqlx(sqlx::Error::RowNotFound)),
                }
            }
        }
    }

    /// Decides an open proposal: `granted` (writes the bot's tier for the class, revoking the
    /// live class row, in the same transaction), `declined` or `stale`. Claim-first: the status
    /// moves only from `proposed`, so of racing clicks exactly one changes anything.
    pub async fn decide_tier_proposal(&self, id: &str, status: &str, actor: &str, note: Option<&str>) -> Result<Decided, DbError> {
        let now = Utc::now().to_rfc3339();
        let mut tx = self.pool.begin().await?;
        let claimed = sqlx::query_as::<_, TierProposal>(
            "UPDATE tier_proposals SET status = ?, decided_at = ?, note = ? WHERE id = ? AND status = 'proposed' RETURNING *",
        )
        .bind(status)
        .bind(&now)
        .bind(note)
        .bind(id)
        .fetch_optional(&mut *tx)
        .await?;
        let Some(p) = claimed else {
            drop(tx);
            return Ok(match self.tier_proposal(id).await? {
                Some(p) => Decided::NotOpen(p.status),
                None => Decided::Missing,
            });
        };
        if status == "granted" {
            sqlx::query("UPDATE capability_policies SET revoked_at = ? WHERE kwant_id = ? AND task_class = ? AND revoked_at IS NULL")
                .bind(&now)
                .bind(&p.kwant_id)
                .bind(&p.task_class)
                .execute(&mut *tx)
                .await?;
            sqlx::query("INSERT INTO capability_policies (id, kwant_id, task_class, tier, granted_by, granted_at, note) VALUES (?, ?, ?, ?, 'person', ?, ?)")
                .bind(Uuid::new_v4().to_string())
                .bind(&p.kwant_id)
                .bind(&p.task_class)
                .bind(&p.to_tier)
                .bind(&now)
                .bind(format!("granted proposal {}", p.id))
                .execute(&mut *tx)
                .await?;
        }
        let kind = match status {
            "granted" => "granted",
            "declined" => "proposal_declined",
            _ => "proposal_stale",
        };
        sqlx::query("INSERT INTO capability_events (kwant_id, task_class, kind, tier, actor, detail, created_at) VALUES (?, ?, ?, ?, ?, ?, ?)")
            .bind(&p.kwant_id)
            .bind(&p.task_class)
            .bind(kind)
            .bind(&p.to_tier)
            .bind(actor)
            .bind(serde_json::json!({"proposal": p.id, "note": note}).to_string())
            .bind(&now)
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        Ok(Decided::Done(Box::new(p)))
    }

    /// Whether the task's open claim (the run going on now) has a usable `before_run` checkpoint:
    /// what lets a sandboxed bot write in its task folder.
    pub async fn run_has_checkpoint(&self, task_id: &str) -> Result<bool, DbError> {
        let row: Option<(i64,)> = sqlx::query_as(
            "SELECT 1 FROM task_checkpoints cp JOIN agent_claims c ON c.id = cp.claim_id \
             WHERE cp.task_id = ? AND cp.kind = 'before_run' AND cp.error IS NULL AND c.released_at IS NULL LIMIT 1",
        )
        .bind(task_id)
        .fetch_optional(&self.pool)
        .await?;
        Ok(row.is_some())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn db() -> (tempfile::TempDir, Database) {
        let tmp = tempfile::tempdir().unwrap();
        let db = Database::init(tmp.path()).await.unwrap();
        (tmp, db)
    }

    #[test]
    fn class_slugs() {
        for ok in ["general", "seo-audit", "a", "x1", &"a".repeat(40)] {
            assert!(valid_class(ok), "{ok}");
        }
        for bad in ["", "*", "Upper", "a b", "a_b", "é", &"a".repeat(41)] {
            assert!(!valid_class(bad), "{bad}");
        }
        assert!(rank("observe") < rank("sandbox") && rank("sandbox") < rank("act"));
    }

    #[tokio::test]
    async fn policies_are_revoked_never_rewritten_and_events_are_append_only() {
        let (_tmp, db) = db().await;
        db.set_policy("k1", "*", "sandbox", "granted", None).await.unwrap();
        let p = db.set_policy("k1", "*", "observe", "revoked", Some("too many denials")).await.unwrap();
        assert_eq!(db.live_policy("k1", "general").await.unwrap().unwrap().id, p.id);
        let rows: Vec<(String, Option<String>)> =
            sqlx::query_as("SELECT tier, revoked_at FROM capability_policies WHERE kwant_id = 'k1' ORDER BY granted_at").fetch_all(&db.pool).await.unwrap();
        assert_eq!(rows.len(), 2);
        assert!(rows.iter().filter(|r| r.1.is_none()).count() == 1);
        assert!(sqlx::query("UPDATE capability_policies SET tier = 'act' WHERE id = ?").bind(&p.id).execute(&db.pool).await.is_err());
        assert!(sqlx::query("DELETE FROM capability_policies").execute(&db.pool).await.is_err());
        assert!(sqlx::query("INSERT INTO capability_policies (id, kwant_id, task_class, tier, granted_by, granted_at) VALUES ('x', 'k1', 'a', 'act', 'k1', 'now')")
            .execute(&db.pool)
            .await
            .is_err());
        assert!(sqlx::query("UPDATE capability_events SET tier = 'act'").execute(&db.pool).await.is_err());
        assert!(sqlx::query("DELETE FROM capability_events").execute(&db.pool).await.is_err());
        assert_eq!(db.capability_events("k1", 10).await.unwrap().len(), 2);
    }

    #[tokio::test]
    async fn a_class_row_beats_the_star_row() {
        let (_tmp, db) = db().await;
        db.set_policy("k1", "*", "observe", "granted", None).await.unwrap();
        db.set_policy("k1", "seo", "act", "granted", None).await.unwrap();
        assert_eq!(db.live_policy("k1", "seo").await.unwrap().unwrap().tier, "act");
        assert_eq!(db.live_policy("k1", "general").await.unwrap().unwrap().tier, "observe");
        assert!(db.live_policy("k2", "general").await.unwrap().is_none());
    }

    #[tokio::test]
    async fn a_bot_hired_with_flight_school_on_starts_at_the_new_bot_tier() {
        let (_tmp, db) = db().await;
        let space = crate::db::spaces::SpaceId::new(crate::db::spaces::DEFAULT_SPACE_ID);
        let new = |name: &str| crate::db::models::NewKwant {
            name: name.into(),
            title: String::new(),
            persona_prompt: String::new(),
            soul: String::new(),
            avatar_sprite: String::new(),
            driver_type: "api".into(),
            preferred_model: "auto".into(),
        };
        let before = db.create_kwant(&space, &new("Before")).await.unwrap();
        assert!(db.policies_of(&before.id).await.unwrap().is_empty());
        db.set_setting("flight_school", "true").await.unwrap();
        let after = db.create_kwant(&space, &new("After")).await.unwrap();
        let p = db.policies_of(&after.id).await.unwrap();
        assert_eq!((p.len(), p[0].task_class.as_str(), p[0].tier.as_str()), (1, "*", "sandbox"));
        db.set_setting("flight_school_new_bot_tier", "observe").await.unwrap();
        let third = db.create_kwant(&space, &new("Third")).await.unwrap();
        assert_eq!(db.policies_of(&third.id).await.unwrap()[0].tier, "observe");
        assert!(db.policies_of(&before.id).await.unwrap().is_empty(), "existing bots stay at act");
    }

    #[tokio::test]
    async fn the_down_migration_restores_the_old_shape_and_up_runs_again() {
        let (_tmp, db) = db().await;
        sqlx::raw_sql(FLIGHT_SCHOOL_DOWN).execute(&db.pool).await.unwrap();
        let col: Option<(String,)> = sqlx::query_as("SELECT name FROM pragma_table_info('tasks') WHERE name = 'task_class'").fetch_optional(&db.pool).await.unwrap();
        assert!(col.is_none());
        let left: Vec<(String,)> = sqlx::query_as(
            "SELECT name FROM sqlite_master WHERE name LIKE 'capability%' OR name LIKE 'tier_proposals%' OR name = 'kwants_flight_school_default'",
        )
        .fetch_all(&db.pool)
        .await
        .unwrap();
        assert!(left.is_empty(), "{left:?}");
        db.migrate_flight_school().await.unwrap();
        db.migrate_flight_school().await.unwrap();
        assert_eq!(db.task_class("missing").await.unwrap(), "general");
    }

    #[tokio::test]
    async fn a_proposal_is_decided_once_and_kept() {
        let (_tmp, db) = db().await;
        let card = serde_json::json!({"tasks": {"verified": 5}});
        let p = db.create_tier_proposal("spc_default", "k1", "research", "observe", "sandbox", &card, Some("why")).await.unwrap().unwrap();
        // One open per bot and class
        let again = db.create_tier_proposal("spc_default", "k1", "research", "observe", "sandbox", &card, None).await.unwrap();
        assert_eq!(again.unwrap_err().id, p.id);
        assert!(matches!(db.decide_tier_proposal(&p.id, "declined", "person", None).await.unwrap(), Decided::Done(_)));
        assert_eq!(db.decide_tier_proposal(&p.id, "granted", "person", None).await.unwrap(), Decided::NotOpen("declined".into()));
        assert_eq!(db.decide_tier_proposal("nope", "granted", "person", None).await.unwrap(), Decided::Missing);
        assert!(db.policies_of("k1").await.unwrap().is_empty(), "a decline writes no policy");
        for bad in [
            "UPDATE tier_proposals SET status = 'granted'",
            "UPDATE tier_proposals SET scorecard = '{}'",
            "DELETE FROM tier_proposals",
        ] {
            assert!(sqlx::query(bad).execute(&db.pool).await.is_err(), "{bad}");
        }
        // A grant revokes the live class row and writes the new tier in the same transaction
        db.set_policy("k1", "ops", "observe", "granted", None).await.unwrap();
        let q = db.create_tier_proposal("spc_default", "k1", "ops", "observe", "sandbox", &card, None).await.unwrap().unwrap();
        assert!(matches!(db.decide_tier_proposal(&q.id, "granted", "person", None).await.unwrap(), Decided::Done(_)));
        let live: Vec<_> = db.policies_of("k1").await.unwrap().into_iter().filter(|x| x.task_class == "ops").collect();
        assert_eq!(live.len(), 1);
        assert_eq!(live[0].tier, "sandbox");
    }
}
