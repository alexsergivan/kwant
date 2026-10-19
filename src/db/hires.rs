//! Bots proposing to hire a new bot (`propose_hire`), and how the person decides: Deny, Accept
//! once or Always accept. A proposal creates nothing by itself; the person's choice, and the
//! space's hire policy, are remembered per space and shown to its bots on their next turn.

use super::models::{Message, MessageExtra, NewMessage};
use super::spaces::SpaceId;
use super::{Database, DbError};
use chrono::{DateTime, Duration, Utc};
use serde::Serialize;
use sqlx::FromRow;
use uuid::Uuid;

/// `pending` waits for the person; `accepted` (by them) and `auto_accepted` (by the space's
/// "always accept" policy) made the bot; `denied` is remembered; `undone` means the person
/// took an accepted hire back.
pub const HIRE_STATUSES: [&str; 5] = ["pending", "accepted", "auto_accepted", "denied", "undone"];

/// Proposals one turn may make.
pub const MAX_PROPOSALS_PER_TURN: i64 = 1;
/// Bots "always accept" may create in a space in 24 hours; past it, proposals ask again.
pub const AUTO_HIRES_PER_DAY: i64 = 3;

#[derive(Debug, Clone, Serialize, FromRow, PartialEq)]
pub struct Hire {
    pub id: String,
    pub space_id: String,
    /// The bot that proposed it.
    pub kwant_id: String,
    pub turn_id: Option<String>,
    pub channel_id: Option<String>,
    pub direct_kwant_id: Option<String>,
    pub thread_id: Option<String>,
    pub name: String,
    pub role: String,
    /// `role` folded to lowercase words, for "was this kind of bot turned down?".
    pub role_key: String,
    pub instructions: String,
    pub model: String,
    /// JSON list of installed tool server ids.
    #[serde(serialize_with = "as_json")]
    pub tools: String,
    pub team_id: Option<String>,
    pub reason: String,
    /// JSON: `{basis, needs?, gap?, similar_requests?}`.
    #[serde(serialize_with = "as_json")]
    pub evidence: String,
    /// The bot said the person asked for this kind of bot (lifts an earlier Deny; never auto).
    pub requested_by_person: bool,
    /// Made by the space's "always accept" policy rather than the person's click.
    pub auto: bool,
    pub status: String,
    /// The bot it made, once accepted.
    pub hired_kwant_id: Option<String>,
    /// The `bot_actions` row Undo goes through.
    pub action_id: Option<String>,
    pub message_id: Option<String>,
    pub created_at: DateTime<Utc>,
    pub resolved_at: Option<DateTime<Utc>>,
    /// JSON list of the other bots in a team proposal: `{name, role, instructions, model, tools}`.
    pub members: Option<String>,
    /// JSON `{name, lead?}`: the team a team proposal creates, `lead` naming one of its bots.
    pub new_team: Option<String>,
    /// JSON list of every bot an accepted team proposal made, in `members` order.
    pub hired_kwant_ids: Option<String>,
}

fn as_json<S: serde::Serializer>(raw: &str, s: S) -> Result<S::Ok, S::Error> {
    let v: serde_json::Value = serde_json::from_str(raw).unwrap_or(serde_json::Value::Null);
    v.serialize(s)
}

impl Hire {
    pub fn tools_json(&self) -> Vec<String> {
        serde_json::from_str(&self.tools).unwrap_or_default()
    }
    pub fn evidence_json(&self) -> serde_json::Value {
        serde_json::from_str(&self.evidence).unwrap_or(serde_json::Value::Null)
    }
    /// The other bots of a team proposal (empty for a single hire).
    pub fn members_json(&self) -> Vec<serde_json::Value> {
        self.members.as_deref().and_then(|m| serde_json::from_str(m).ok()).unwrap_or_default()
    }
    pub fn new_team_json(&self) -> Option<serde_json::Value> {
        self.new_team.as_deref().and_then(|t| serde_json::from_str(t).ok()).filter(|v: &serde_json::Value| v.is_object())
    }
    /// Every bot it made: the team's ids, or the single hire's.
    pub fn hired_ids(&self) -> Vec<String> {
        let ids: Vec<String> = self.hired_kwant_ids.as_deref().and_then(|m| serde_json::from_str(m).ok()).unwrap_or_default();
        if ids.is_empty() {
            self.hired_kwant_id.iter().cloned().collect()
        } else {
            ids
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct NewHire {
    pub space_id: String,
    pub kwant_id: String,
    pub turn_id: Option<String>,
    pub channel_id: Option<String>,
    pub direct_kwant_id: Option<String>,
    pub thread_id: Option<String>,
    pub name: String,
    pub role: String,
    pub instructions: String,
    pub model: String,
    pub tools: Vec<String>,
    pub team_id: Option<String>,
    pub reason: String,
    pub evidence: serde_json::Value,
    pub requested_by_person: bool,
    /// The other bots of a team proposal.
    pub members: Vec<serde_json::Value>,
    pub new_team: Option<serde_json::Value>,
}

/// The space's hire policy: `ask` (default) or `auto`; denials before `denials_cleared_at` no
/// longer count.
#[derive(Debug, Clone, Serialize, FromRow, PartialEq)]
pub struct HirePolicy {
    pub space_id: String,
    pub policy: String,
    pub denials_cleared_at: Option<DateTime<Utc>>,
    pub updated_at: Option<DateTime<Utc>>,
}

/// Lowercase words of a role, without filler, sorted: "Recipe helper" → "helper recipe".
pub fn role_key(role: &str) -> String {
    const FILLER: [&str; 10] = ["a", "an", "the", "and", "for", "of", "bot", "assistant", "agent", "to"];
    let mut words: Vec<String> = role
        .to_lowercase()
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty() && !FILLER.contains(w))
        .map(|w| w.strip_suffix('s').filter(|s| s.len() > 2).unwrap_or(w).to_string())
        .collect();
    words.sort();
    words.dedup();
    words.join(" ")
}

/// Whether two role keys name the same kind of bot: equal, or sharing at least half their words.
pub fn similar_roles(a: &str, b: &str) -> bool {
    if a.is_empty() || b.is_empty() {
        return false;
    }
    if a == b {
        return true;
    }
    let wa: Vec<&str> = a.split(' ').collect();
    let wb: Vec<&str> = b.split(' ').collect();
    let shared = wa.iter().filter(|w| wb.contains(w)).count();
    shared * 2 > wa.len().max(wb.len())
}

const GET: &str = "SELECT * FROM hire_proposals WHERE id = ?";

impl Database {
    pub(super) async fn migrate_hires(&self) -> Result<(), DbError> {
        for stmt in [
            "CREATE TABLE IF NOT EXISTS hire_proposals (
                id TEXT PRIMARY KEY,
                space_id TEXT NOT NULL,
                kwant_id TEXT NOT NULL,
                turn_id TEXT,
                channel_id TEXT,
                direct_kwant_id TEXT,
                thread_id TEXT,
                name TEXT NOT NULL,
                role TEXT NOT NULL,
                role_key TEXT NOT NULL,
                instructions TEXT NOT NULL,
                model TEXT NOT NULL,
                tools TEXT NOT NULL,
                team_id TEXT,
                reason TEXT NOT NULL,
                evidence TEXT NOT NULL,
                requested_by_person BOOLEAN NOT NULL DEFAULT 0,
                auto BOOLEAN NOT NULL DEFAULT 0,
                status TEXT NOT NULL CHECK (status IN ('pending', 'accepted', 'auto_accepted', 'denied', 'undone')),
                hired_kwant_id TEXT,
                action_id TEXT,
                message_id TEXT,
                created_at TIMESTAMP NOT NULL,
                resolved_at TIMESTAMP
            )",
            "CREATE INDEX IF NOT EXISTS hire_proposals_by_space ON hire_proposals (space_id, created_at)",
            "CREATE INDEX IF NOT EXISTS hire_proposals_by_turn ON hire_proposals (turn_id)",
            "CREATE TABLE IF NOT EXISTS hire_policies (
                space_id TEXT PRIMARY KEY,
                policy TEXT NOT NULL CHECK (policy IN ('ask', 'auto')),
                denials_cleared_at TIMESTAMP,
                updated_at TIMESTAMP
            )",
        ] {
            sqlx::query(stmt).execute(&self.pool).await?;
        }
        // Team proposals (added later; "duplicate column" on databases that have them)
        for col in ["members TEXT", "new_team TEXT", "hired_kwant_ids TEXT"] {
            let _ = sqlx::query(&format!("ALTER TABLE hire_proposals ADD COLUMN {col}")).execute(&self.pool).await;
        }
        Ok(())
    }

    pub async fn get_hire(&self, id: &str) -> Result<Option<Hire>, DbError> {
        Ok(sqlx::query_as::<_, Hire>(GET).bind(id).fetch_optional(&self.pool).await?)
    }

    /// The space's proposals, newest first (max 200).
    pub async fn list_hires(&self, space: &SpaceId, status: Option<&str>) -> Result<Vec<Hire>, DbError> {
        Ok(sqlx::query_as::<_, Hire>(
            "SELECT * FROM hire_proposals WHERE space_id = ? AND (?2 IS NULL OR status = ?2) ORDER BY created_at DESC LIMIT 200",
        )
        .bind(space.as_str())
        .bind(status)
        .fetch_all(&self.pool)
        .await?)
    }

    pub async fn hires_in_turn(&self, turn_id: &str) -> Result<i64, DbError> {
        let (n,): (i64,) =
            sqlx::query_as("SELECT COUNT(*) FROM hire_proposals WHERE turn_id = ?").bind(turn_id).fetch_one(&self.pool).await?;
        Ok(n)
    }

    /// Bots the space's policy created in the last 24 hours (undone ones count too).
    pub async fn auto_hires_today(&self, space: &SpaceId) -> Result<i64, DbError> {
        let (n,): (i64,) = sqlx::query_as(
            "SELECT COUNT(*) FROM hire_proposals WHERE space_id = ? AND auto = 1 AND resolved_at > ?",
        )
        .bind(space.as_str())
        .bind(Utc::now() - Duration::hours(24))
        .fetch_one(&self.pool)
        .await?;
        Ok(n)
    }

    pub async fn hire_policy(&self, space: &SpaceId) -> Result<HirePolicy, DbError> {
        let row = sqlx::query_as::<_, HirePolicy>("SELECT * FROM hire_policies WHERE space_id = ?")
            .bind(space.as_str())
            .fetch_optional(&self.pool)
            .await?;
        Ok(row.unwrap_or(HirePolicy { space_id: space.as_str().into(), policy: "ask".into(), denials_cleared_at: None, updated_at: None }))
    }

    /// Sets the policy; `clear_denials` makes earlier Denies stop counting.
    pub async fn set_hire_policy(&self, space: &SpaceId, policy: &str, clear_denials: bool) -> Result<HirePolicy, DbError> {
        let now = Utc::now();
        let current = self.hire_policy(space).await?;
        let cleared = if clear_denials { Some(now) } else { current.denials_cleared_at };
        sqlx::query(
            "INSERT INTO hire_policies (space_id, policy, denials_cleared_at, updated_at) VALUES (?, ?, ?, ?)
             ON CONFLICT(space_id) DO UPDATE SET policy = excluded.policy, denials_cleared_at = excluded.denials_cleared_at,
             updated_at = excluded.updated_at",
        )
        .bind(space.as_str())
        .bind(policy)
        .bind(cleared)
        .bind(now)
        .execute(&self.pool)
        .await?;
        self.hire_policy(space).await
    }

    /// Denials that still count in the space, newest first.
    pub async fn active_denials(&self, space: &SpaceId) -> Result<Vec<Hire>, DbError> {
        let policy = self.hire_policy(space).await?;
        let list = self.list_hires(space, Some("denied")).await?;
        Ok(list.into_iter().filter(|h| policy.denials_cleared_at.is_none_or(|c| h.resolved_at.is_some_and(|r| r > c))).collect())
    }

    /// Records a proposal with the card message `card` builds, in one transaction.
    pub async fn record_hire(
        &self,
        new: &NewHire,
        card: impl FnOnce(&Hire) -> Option<(NewMessage, MessageExtra)>,
    ) -> Result<(Hire, Option<Message>), DbError> {
        let mut tx = self.pool.begin().await?;
        let id = Uuid::new_v4().to_string();
        sqlx::query(
            "INSERT INTO hire_proposals (id, space_id, kwant_id, turn_id, channel_id, direct_kwant_id, thread_id, name, role, role_key,
                 instructions, model, tools, team_id, reason, evidence, requested_by_person, status, created_at, members, new_team)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, 'pending', ?, ?, ?)",
        )
        .bind(&id)
        .bind(&new.space_id)
        .bind(&new.kwant_id)
        .bind(&new.turn_id)
        .bind(&new.channel_id)
        .bind(&new.direct_kwant_id)
        .bind(&new.thread_id)
        .bind(&new.name)
        .bind(&new.role)
        .bind(role_key(&new.role))
        .bind(&new.instructions)
        .bind(&new.model)
        .bind(serde_json::to_string(&new.tools)?)
        .bind(&new.team_id)
        .bind(&new.reason)
        .bind(new.evidence.to_string())
        .bind(new.requested_by_person)
        .bind(Utc::now())
        .bind((!new.members.is_empty()).then(|| serde_json::Value::from(new.members.clone()).to_string()))
        .bind(new.new_team.as_ref().map(|t| t.to_string()))
        .execute(&mut *tx)
        .await?;
        let hire = sqlx::query_as::<_, Hire>(GET).bind(&id).fetch_one(&mut *tx).await?;
        let mut message = None;
        if let Some((msg, extra)) = card(&hire) {
            let m = super::insert_message_on(&mut tx, &msg, &extra).await?;
            sqlx::query("UPDATE hire_proposals SET message_id = ? WHERE id = ?").bind(&m.id).bind(&id).execute(&mut *tx).await?;
            message = Some(m);
        }
        tx.commit().await?;
        let hire = self.get_hire(&id).await?.ok_or(DbError::Sqlx(sqlx::Error::RowNotFound))?;
        Ok((hire, message))
    }

    /// Moves a proposal from `from` to `to`; `false` when it wasn't in `from` any more.
    pub async fn resolve_hire(&self, id: &str, from: &str, to: &str) -> Result<bool, DbError> {
        let done = sqlx::query("UPDATE hire_proposals SET status = ?, resolved_at = ? WHERE id = ? AND status = ?")
            .bind(to)
            .bind(Utc::now())
            .bind(id)
            .bind(from)
            .execute(&self.pool)
            .await?;
        Ok(done.rows_affected() == 1)
    }

    pub async fn set_hire_result(&self, id: &str, kwant_id: Option<&str>, action_id: Option<&str>) -> Result<(), DbError> {
        sqlx::query("UPDATE hire_proposals SET hired_kwant_id = ?, action_id = ? WHERE id = ?")
            .bind(kwant_id)
            .bind(action_id)
            .bind(id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    /// Every bot an accepted team proposal made, in `members` order.
    pub async fn set_hired_kwant_ids(&self, id: &str, ids: &[String]) -> Result<(), DbError> {
        sqlx::query("UPDATE hire_proposals SET hired_kwant_ids = ? WHERE id = ?")
            .bind(serde_json::to_string(ids)?)
            .bind(id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    /// Marks a proposal as decided by the space's policy, for the daily cap.
    pub async fn mark_hire_auto(&self, id: &str) -> Result<(), DbError> {
        sqlx::query("UPDATE hire_proposals SET auto = 1 WHERE id = ?")
            .bind(id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    /// The space a message belongs to (through its channel, direct chat or thread) and who sent it.
    /// When a message was posted.
    pub async fn message_created_at(&self, id: &str) -> Result<Option<DateTime<Utc>>, DbError> {
        Ok(sqlx::query_scalar("SELECT created_at FROM messages WHERE id = ?").bind(id).fetch_optional(&self.pool).await?)
    }

    /// Replaces a card message's text, so bots reading the history see the decision too.
    pub async fn set_message_text(&self, id: &str, text: &str) -> Result<(), DbError> {
        sqlx::query("UPDATE messages SET text = ? WHERE id = ?").bind(text).bind(id).execute(&self.pool).await?;
        Ok(())
    }

    pub async fn message_space_and_sender(&self, id: &str) -> Result<Option<(SpaceId, String)>, DbError> {
        type Row = (Option<String>, Option<String>, Option<String>, String);
        let row: Option<Row> =
            sqlx::query_as("SELECT channel_id, direct_kwant_id, thread_id, sender_role FROM messages WHERE id = ?")
                .bind(id)
                .fetch_optional(&self.pool)
                .await?;
        let Some((channel, direct, thread, role)) = row else {
            return Ok(None);
        };
        let space = if let Some(c) = channel {
            self.space_of_channel(&c).await?
        } else if let Some(k) = direct {
            self.space_of_kwant(&k).await?
        } else if let Some(t) = thread {
            self.space_of_thread(&t).await?
        } else {
            None
        };
        Ok(space.map(|s| (s, role)))
    }

    /// Adds a bot to a team and to the team's `team-<name>` channel.
    pub async fn add_team_member(&self, team_id: &str, kwant_id: &str) -> Result<bool, DbError> {
        let Some(team) = self.get_team(team_id).await? else {
            return Ok(false);
        };
        let mut members: Vec<String> = serde_json::from_str(&team.member_kwants).unwrap_or_default();
        if !members.iter().any(|m| m == kwant_id) {
            members.push(kwant_id.to_string());
        }
        let json = serde_json::to_string(&members)?;
        let mut tx = self.pool.begin().await?;
        sqlx::query("UPDATE teams SET member_kwants = ? WHERE id = ?").bind(&json).bind(team_id).execute(&mut *tx).await?;
        let channel: Option<(String, String)> = sqlx::query_as("SELECT id, member_kwants FROM channels WHERE name = ? AND space_id = ?")
            .bind(format!("team-{}", team.name))
            .bind(&team.space_id)
            .fetch_optional(&mut *tx)
            .await?;
        if let Some((cid, raw)) = channel {
            let mut cm: Vec<String> = serde_json::from_str(&raw).unwrap_or_default();
            if !cm.iter().any(|m| m == kwant_id) {
                cm.push(kwant_id.to_string());
            }
            sqlx::query("UPDATE channels SET member_kwants = ? WHERE id = ?")
                .bind(serde_json::to_string(&cm)?)
                .bind(cid)
                .execute(&mut *tx)
                .await?;
        }
        tx.commit().await?;
        Ok(true)
    }

    /// The team a bot leads or belongs to in its space, if any (the first one made).
    pub async fn team_of_kwant(&self, space: &SpaceId, kwant_id: &str) -> Result<Option<super::models::Team>, DbError> {
        let teams = self.list_teams(space).await?;
        Ok(teams.into_iter().find(|t| {
            t.lead_kwant_id == kwant_id || serde_json::from_str::<Vec<String>>(&t.member_kwants).unwrap_or_default().iter().any(|m| m == kwant_id)
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roles_compare_by_their_words() {
        assert_eq!(role_key("The Recipe helper bot"), "helper recipe");
        assert!(similar_roles(&role_key("Recipe helper"), &role_key("recipes helper")));
        assert!(similar_roles(&role_key("Weekly recipe planner"), &role_key("recipe planner")));
        assert!(!similar_roles(&role_key("Recipe helper"), &role_key("Recipe planner")));
        assert!(!similar_roles(&role_key("Backend engineer"), &role_key("Frontend engineer")));
        assert!(!similar_roles(&role_key("Recipe helper"), &role_key("Tax advisor")));
        assert!(!similar_roles("", "x"));
    }
}
