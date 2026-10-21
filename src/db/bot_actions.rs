//! Changes agents made to the app from a chat (`routine_create`, `bot_update`, …): the audit log
//! and what Undo needs. A change applies at once (`applied`) and keeps a snapshot of what it
//! replaced; the person can undo it (`undone`). Deletions only file a request (`pending`) that
//! the person confirms (`confirmed`) or declines (`declined`).

use super::models::{Message, MessageExtra, NewMessage};
use super::spaces::SpaceId;
use super::{Database, DbError};
use chrono::{DateTime, Utc};
use serde::Serialize;
use sqlx::FromRow;
use uuid::Uuid;

/// Most changes one turn may make.
pub const MAX_PER_TURN: i64 = 10;

pub const ACTION_STATUSES: [&str; 5] = ["applied", "undone", "pending", "confirmed", "declined"];

#[derive(Debug, Clone, Serialize, FromRow, PartialEq)]
pub struct BotAction {
    pub id: String,
    pub space_id: String,
    pub kwant_id: String,
    pub turn_id: Option<String>,
    pub channel_id: Option<String>,
    pub direct_kwant_id: Option<String>,
    pub thread_id: Option<String>,
    /// The tool that made it, e.g. `routine_create`.
    pub action: String,
    /// `routine`, `bot`, `settings`, `space`, `channel` or `profile`.
    pub target_kind: String,
    pub target_id: Option<String>,
    /// One sentence for the card and the log.
    pub summary: String,
    /// What the change replaced (JSON), for Undo; `null` when there was nothing before.
    #[serde(skip)]
    pub before: Option<String>,
    /// The result (JSON), shown on the card. Never holds secrets: no tool takes one.
    #[serde(serialize_with = "as_json")]
    pub after: Option<String>,
    pub status: String,
    pub message_id: Option<String>,
    pub created_at: DateTime<Utc>,
    pub resolved_at: Option<DateTime<Utc>>,
}

fn as_json<S: serde::Serializer>(raw: &Option<String>, s: S) -> Result<S::Ok, S::Error> {
    let v: serde_json::Value = raw.as_deref().and_then(|r| serde_json::from_str(r).ok()).unwrap_or(serde_json::Value::Null);
    v.serialize(s)
}

impl BotAction {
    pub fn before_json(&self) -> serde_json::Value {
        self.before.as_deref().and_then(|r| serde_json::from_str(r).ok()).unwrap_or(serde_json::Value::Null)
    }

    pub fn after_json(&self) -> serde_json::Value {
        self.after.as_deref().and_then(|r| serde_json::from_str(r).ok()).unwrap_or(serde_json::Value::Null)
    }
}

/// An action to record; the id and time are set on insert.
#[derive(Debug, Clone, Default)]
pub struct NewBotAction {
    pub space_id: String,
    pub kwant_id: String,
    pub turn_id: Option<String>,
    pub channel_id: Option<String>,
    pub direct_kwant_id: Option<String>,
    pub thread_id: Option<String>,
    pub action: String,
    pub target_kind: String,
    pub target_id: Option<String>,
    pub summary: String,
    pub before: Option<serde_json::Value>,
    pub after: Option<serde_json::Value>,
    pub status: String,
}

const GET: &str = "SELECT * FROM bot_actions WHERE id = ?";

impl Database {
    pub(super) async fn migrate_bot_actions(&self) -> Result<(), DbError> {
        for stmt in [
            "CREATE TABLE IF NOT EXISTS bot_actions (
                id TEXT PRIMARY KEY,
                space_id TEXT NOT NULL,
                kwant_id TEXT NOT NULL,
                turn_id TEXT,
                channel_id TEXT,
                direct_kwant_id TEXT,
                thread_id TEXT,
                action TEXT NOT NULL,
                target_kind TEXT NOT NULL,
                target_id TEXT,
                summary TEXT NOT NULL,
                before TEXT,
                after TEXT,
                status TEXT NOT NULL CHECK (status IN ('applied', 'undone', 'pending', 'confirmed', 'declined')),
                message_id TEXT,
                created_at TIMESTAMP NOT NULL,
                resolved_at TIMESTAMP
            )",
            "CREATE INDEX IF NOT EXISTS bot_actions_by_space ON bot_actions (space_id, created_at)",
            "CREATE INDEX IF NOT EXISTS bot_actions_by_turn ON bot_actions (turn_id)",
            "CREATE INDEX IF NOT EXISTS bot_actions_by_target ON bot_actions (target_kind, target_id)",
        ] {
            sqlx::query(stmt).execute(&self.pool).await?;
        }
        Ok(())
    }

    /// How many changes the turn has made; a turn without an id counts the agent's last 10 minutes.
    pub async fn bot_actions_in_turn(&self, kwant_id: &str, turn_id: Option<&str>) -> Result<i64, DbError> {
        let (n,): (i64,) = match turn_id {
            Some(t) => sqlx::query_as("SELECT COUNT(*) FROM bot_actions WHERE turn_id = ?").bind(t).fetch_one(&self.pool).await?,
            None => {
                sqlx::query_as("SELECT COUNT(*) FROM bot_actions WHERE kwant_id = ? AND turn_id IS NULL AND created_at > ?")
                    .bind(kwant_id)
                    .bind(Utc::now() - chrono::Duration::minutes(10))
                    .fetch_one(&self.pool)
                    .await?
            }
        };
        Ok(n)
    }

    /// Records an action together with the chat message that shows it (built by `card`), in one
    /// transaction.
    pub async fn record_bot_action(
        &self,
        new: &NewBotAction,
        card: impl FnOnce(&BotAction) -> Option<(NewMessage, MessageExtra)>,
    ) -> Result<(BotAction, Option<Message>), DbError> {
        let mut tx = self.pool.begin().await?;
        let id = Uuid::new_v4().to_string();
        sqlx::query(
            "INSERT INTO bot_actions (id, space_id, kwant_id, turn_id, channel_id, direct_kwant_id, thread_id, action,
                 target_kind, target_id, summary, before, after, status, created_at)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(&id)
        .bind(&new.space_id)
        .bind(&new.kwant_id)
        .bind(&new.turn_id)
        .bind(&new.channel_id)
        .bind(&new.direct_kwant_id)
        .bind(&new.thread_id)
        .bind(&new.action)
        .bind(&new.target_kind)
        .bind(&new.target_id)
        .bind(&new.summary)
        .bind(new.before.as_ref().map(|v| v.to_string()))
        .bind(new.after.as_ref().map(|v| v.to_string()))
        .bind(&new.status)
        .bind(Utc::now())
        .execute(&mut *tx)
        .await?;
        let mut a = sqlx::query_as::<_, BotAction>(GET).bind(&id).fetch_one(&mut *tx).await?;
        let mut message = None;
        if let Some((msg, extra)) = card(&a) {
            let saved = super::insert_message_on(&mut tx, &msg, &extra).await?;
            sqlx::query("UPDATE bot_actions SET message_id = ? WHERE id = ?").bind(&saved.id).bind(&id).execute(&mut *tx).await?;
            a = sqlx::query_as::<_, BotAction>(GET).bind(&id).fetch_one(&mut *tx).await?;
            message = Some(saved);
        }
        tx.commit().await?;
        Ok((a, message))
    }

    pub async fn get_bot_action(&self, id: &str) -> Result<Option<BotAction>, DbError> {
        Ok(sqlx::query_as::<_, BotAction>(GET).bind(id).fetch_optional(&self.pool).await?)
    }

    /// A space's actions, newest first, at most 200; optionally by agent and/or status.
    pub async fn list_bot_actions(
        &self,
        space: &SpaceId,
        kwant_id: Option<&str>,
        status: Option<&str>,
    ) -> Result<Vec<BotAction>, DbError> {
        Ok(sqlx::query_as::<_, BotAction>(
            "SELECT * FROM bot_actions WHERE space_id = ?1 AND (?2 IS NULL OR kwant_id = ?2) AND (?3 IS NULL OR status = ?3)
             ORDER BY created_at DESC, rowid DESC LIMIT 200",
        )
        .bind(space.as_str())
        .bind(kwant_id)
        .bind(status)
        .fetch_all(&self.pool)
        .await?)
    }

    /// A newer applied change to the same thing, which Undo would silently overwrite.
    pub async fn newer_bot_action(&self, a: &BotAction) -> Result<Option<BotAction>, DbError> {
        Ok(sqlx::query_as::<_, BotAction>(
            "SELECT * FROM bot_actions WHERE target_kind = ? AND target_id IS ? AND status = 'applied' AND id != ?
               AND (created_at > ? OR (created_at = ? AND rowid > (SELECT rowid FROM bot_actions WHERE id = ?)))
             ORDER BY created_at DESC LIMIT 1",
        )
        .bind(&a.target_kind)
        .bind(&a.target_id)
        .bind(&a.id)
        .bind(a.created_at)
        .bind(a.created_at)
        .bind(&a.id)
        .fetch_optional(&self.pool)
        .await?)
    }

    /// Moves an action from `from` to `to`; `false` when it wasn't in `from` (someone else won).
    pub async fn resolve_bot_action(&self, id: &str, from: &str, to: &str) -> Result<bool, DbError> {
        let done = sqlx::query("UPDATE bot_actions SET status = ?, resolved_at = ? WHERE id = ? AND status = ?")
            .bind(to)
            .bind(Utc::now())
            .bind(id)
            .bind(from)
            .execute(&self.pool)
            .await?;
        Ok(done.rows_affected() == 1)
    }

    /// Puts an action back to `status` after its undo or confirm failed.
    pub async fn reset_bot_action(&self, id: &str, status: &str) -> Result<(), DbError> {
        sqlx::query("UPDATE bot_actions SET status = ?, resolved_at = NULL WHERE id = ?").bind(status).bind(id).execute(&self.pool).await?;
        Ok(())
    }

    /// Points an action at the row its undo recreated (an undone delete gets a new id).
    pub async fn retarget_bot_action(&self, id: &str, target_id: &str) -> Result<(), DbError> {
        sqlx::query("UPDATE bot_actions SET target_id = ? WHERE id = ?").bind(target_id).bind(id).execute(&self.pool).await?;
        Ok(())
    }

    /// Why a kwant can't be removed without losing anything, or `None` when it can: it leads a
    /// channel or team, or has history (messages, threads, delegations, questions, files).
    pub async fn kwant_in_use(&self, id: &str) -> Result<Option<&'static str>, DbError> {
        let checks: [(&str, &'static str); 11] = [
            ("SELECT COUNT(*) FROM channels WHERE lead_kwant_id = ?1", "it leads a channel"),
            ("SELECT COUNT(*) FROM teams WHERE lead_kwant_id = ?1", "it leads a team"),
            (
                "SELECT COUNT(*) FROM messages WHERE direct_kwant_id = ?1 OR (sender_role = 'kwant' AND sender_id = (SELECT name FROM kwants WHERE id = ?1) \
                 AND (channel_id IN (SELECT id FROM channels WHERE space_id = (SELECT space_id FROM kwants WHERE id = ?1))))",
                "it already has chat history",
            ),
            ("SELECT COUNT(*) FROM threads WHERE kwant_id = ?1", "it already has threads"),
            ("SELECT COUNT(*) FROM delegations WHERE lead_kwant_id = ?1 OR specialist_kwant_id = ?1", "it already has delegated work"),
            ("SELECT COUNT(*) FROM questions WHERE kwant_id = ?1", "it already asked questions"),
            ("SELECT COUNT(*) FROM attachments WHERE kwant_id = ?1 OR direct_kwant_id = ?1", "it already has files"),
            ("SELECT COUNT(*) FROM approvals WHERE kwant_id = ?1", "it already has approvals"),
            ("SELECT COUNT(*) FROM skills WHERE kwant_id = ?1", "it already has skills"),
            ("SELECT COUNT(*) FROM mcp_suggestions WHERE kwant_id = ?1 OR direct_kwant_id = ?1", "it already suggested tools"),
            ("SELECT COUNT(*) FROM hire_proposals WHERE kwant_id = ?1", "it already proposed hiring bots"),
        ];
        for (sql, why) in checks {
            let (n,): (i64,) = sqlx::query_as(sql).bind(id).fetch_one(&self.pool).await?;
            if n > 0 {
                return Ok(Some(why));
            }
        }
        Ok(None)
    }

    /// Removes a kwant that [`Self::kwant_in_use`] cleared, with its memories, routines and
    /// their runs, MCP opt-outs and channel/team memberships, in one transaction.
    pub async fn delete_unused_kwant(&self, id: &str) -> Result<bool, DbError> {
        let mut tx = self.pool.begin().await?;
        for sql in [
            "DELETE FROM routine_runs WHERE routine_id IN (SELECT id FROM routines WHERE kwant_id = ?)",
            "DELETE FROM routines WHERE kwant_id = ?",
            "DELETE FROM kwant_memories WHERE kwant_id = ?",
            "DELETE FROM kwant_mcp_disabled WHERE kwant_id = ?",
            "DELETE FROM kwant_mcp_servers WHERE kwant_id = ?",
        ] {
            sqlx::query(sql).bind(id).execute(&mut *tx).await?;
        }
        for table in ["channels", "teams"] {
            let rows: Vec<(String, String)> =
                sqlx::query_as(&format!("SELECT id, member_kwants FROM {table} WHERE member_kwants LIKE '%' || ? || '%'"))
                    .bind(id)
                    .fetch_all(&mut *tx)
                    .await?;
            for (row_id, members) in rows {
                let kept: Vec<String> =
                    serde_json::from_str::<Vec<String>>(&members).unwrap_or_default().into_iter().filter(|m| m != id).collect();
                sqlx::query(&format!("UPDATE {table} SET member_kwants = ? WHERE id = ?"))
                    .bind(serde_json::to_string(&kept)?)
                    .bind(&row_id)
                    .execute(&mut *tx)
                    .await?;
            }
        }
        let done = sqlx::query("DELETE FROM kwants WHERE id = ?").bind(id).execute(&mut *tx).await?;
        tx.commit().await?;
        Ok(done.rows_affected() == 1)
    }

    /// Delegated work for or from the bot is queued or running.
    pub async fn kwant_has_live_delegations(&self, id: &str) -> Result<bool, DbError> {
        let (n,): (i64,) = sqlx::query_as(
            "SELECT COUNT(*) FROM delegations WHERE (lead_kwant_id = ?1 OR specialist_kwant_id = ?1) AND status IN ('queued', 'running')",
        )
        .bind(id)
        .fetch_one(&self.pool)
        .await?;
        Ok(n > 0)
    }

    /// What deleting a bot would remove: counts by kind (for the person's confirm dialog).
    pub async fn kwant_usage(&self, id: &str) -> Result<serde_json::Value, DbError> {
        let count = |sql: &'static str| async move {
            let (n,): (i64,) = sqlx::query_as(sql).bind(id).fetch_one(&self.pool).await?;
            Ok::<i64, DbError>(n)
        };
        Ok(serde_json::json!({
            "conversations": count("SELECT (SELECT COUNT(*) FROM threads WHERE kwant_id = ?1) + \
                (SELECT COUNT(*) > 0 FROM messages WHERE direct_kwant_id = ?1)").await?,
            "messages": count("SELECT COUNT(*) FROM messages WHERE direct_kwant_id = ?1 OR thread_id IN (SELECT id FROM threads WHERE kwant_id = ?1)").await?,
            "memories": count("SELECT COUNT(*) FROM kwant_memories WHERE kwant_id = ?1").await?,
            "skills": count("SELECT COUNT(*) FROM skills WHERE kwant_id = ?1").await?,
            "routines": count("SELECT COUNT(*) FROM routines WHERE kwant_id = ?1").await?,
            "leads": count("SELECT (SELECT COUNT(*) FROM channels WHERE lead_kwant_id = ?1) + (SELECT COUNT(*) FROM teams WHERE lead_kwant_id = ?1)").await?,
        }))
    }

    /// Deletes a bot the person chose to remove, with its direct chat, threads, memories,
    /// skills, routines, questions, approvals, suggestions, proposals, cached API prompts and memberships, in one
    /// transaction. Channels and teams it led get another lead (`new_lead`, which must be in
    /// the same space). What it wrote in shared channels stays, as the room's history; so does
    /// the space's log of bot actions.
    pub async fn delete_kwant_and_history(&self, id: &str, new_lead: &str) -> Result<bool, DbError> {
        let mut tx = self.pool.begin().await?;
        for sql in [
            "DELETE FROM attachments WHERE direct_kwant_id = ?1 OR thread_id IN (SELECT id FROM threads WHERE kwant_id = ?1)",
            "DELETE FROM messages WHERE direct_kwant_id = ?1 OR thread_id IN (SELECT id FROM threads WHERE kwant_id = ?1)",
            "DELETE FROM delegations WHERE lead_kwant_id = ?1 OR specialist_kwant_id = ?1 \
                OR thread_id IN (SELECT id FROM threads WHERE kwant_id = ?1)",
            "DELETE FROM threads WHERE kwant_id = ?1",
            "DELETE FROM questions WHERE kwant_id = ?1 OR direct_kwant_id = ?1",
            "DELETE FROM approvals WHERE kwant_id = ?1",
            "DELETE FROM approval_modes WHERE scope = 'kwant' AND id = ?1",
            "DELETE FROM skills WHERE kwant_id = ?1",
            "DELETE FROM mcp_suggestions WHERE kwant_id = ?1 OR direct_kwant_id = ?1",
            "DELETE FROM hire_proposals WHERE kwant_id = ?1 OR direct_kwant_id = ?1",
            "DELETE FROM bot_actions WHERE direct_kwant_id = ?1",
            "DELETE FROM routine_runs WHERE routine_id IN (SELECT id FROM routines WHERE kwant_id = ?1)",
            "DELETE FROM routines WHERE kwant_id = ?1",
            "DELETE FROM kwant_memories WHERE kwant_id = ?1",
            "DELETE FROM kwant_mcp_disabled WHERE kwant_id = ?1",
            "DELETE FROM kwant_mcp_servers WHERE kwant_id = ?1",
            "DELETE FROM api_contexts WHERE kwant_id = ?1",
        ] {
            sqlx::query(sql).bind(id).execute(&mut *tx).await?;
        }
        // Memberships; a channel or team it led goes to its next member, else `new_lead`
        for table in ["channels", "teams"] {
            let rows: Vec<(String, String, String)> = sqlx::query_as(&format!(
                "SELECT id, lead_kwant_id, member_kwants FROM {table} WHERE lead_kwant_id = ?1 OR member_kwants LIKE '%' || ?1 || '%'"
            ))
            .bind(id)
            .fetch_all(&mut *tx)
            .await?;
            for (row_id, lead, members) in rows {
                let kept: Vec<String> =
                    serde_json::from_str::<Vec<String>>(&members).unwrap_or_default().into_iter().filter(|m| m != id).collect();
                let lead = if lead == id { kept.first().cloned().unwrap_or_else(|| new_lead.to_string()) } else { lead };
                sqlx::query(&format!("UPDATE {table} SET member_kwants = ?, lead_kwant_id = ? WHERE id = ?"))
                    .bind(serde_json::to_string(&kept)?)
                    .bind(&lead)
                    .bind(&row_id)
                    .execute(&mut *tx)
                    .await?;
            }
        }
        let done = sqlx::query("DELETE FROM kwants WHERE id = ?").bind(id).execute(&mut *tx).await?;
        tx.commit().await?;
        Ok(done.rows_affected() == 1)
    }

    /// Removes a channel nobody has written in yet; `false` when it has messages or is #general.
    pub async fn delete_empty_channel(&self, id: &str) -> Result<bool, DbError> {
        let mut tx = self.pool.begin().await?;
        let (n,): (i64,) = sqlx::query_as("SELECT COUNT(*) FROM messages WHERE channel_id = ?").bind(id).fetch_one(&mut *tx).await?;
        if n > 0 {
            return Ok(false);
        }
        let done = sqlx::query("DELETE FROM channels WHERE id = ? AND name != 'general'").bind(id).execute(&mut *tx).await?;
        tx.commit().await?;
        Ok(done.rows_affected() == 1)
    }
}
