//! Whether bots ask before their tools run. The person sets it for a space (`ask` or `off`) and
//! may override it per bot (`ask` or `off`; no row inherits the space). Bots never change it.
//! Kept in its own table, outside exports, so an imported bundle can't switch approvals off.

use super::{spaces::SpaceId, Database, DbError};
use chrono::Utc;

/// The default for a space with no row, and for a bot with no override.
pub const ASK: &str = "ask";
pub const OFF: &str = "off";

/// A bot's setting as the person chose it: `inherit` follows its space.
pub const INHERIT: &str = "inherit";

/// How long a card that outlived its turn can still be answered to resume the work.
pub const WAITING_APPROVAL_HOURS: i64 = 24;

/// What `approvals` resolves to for one bot.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct ApprovalMode {
    /// The space's value: `ask` or `off`.
    pub space: String,
    /// The bot's own choice: `inherit`, `ask` or `off`.
    pub bot: String,
    /// The bot's override, else the space's: `ask` or `off`.
    pub effective: String,
}

impl ApprovalMode {
    pub fn is_off(&self) -> bool {
        self.effective == OFF
    }
}

/// `ask` or `off`; anything else is not a mode.
pub fn valid_mode(mode: &str) -> bool {
    matches!(mode, ASK | OFF)
}

impl Database {
    pub(super) async fn migrate_approval_modes(&self) -> Result<(), DbError> {
        sqlx::query(
            "CREATE TABLE IF NOT EXISTS approval_modes (
                scope TEXT NOT NULL CHECK (scope IN ('space', 'kwant')),
                id TEXT NOT NULL,
                mode TEXT NOT NULL CHECK (mode IN ('ask', 'off')),
                updated_at TIMESTAMP NOT NULL,
                PRIMARY KEY (scope, id)
            )",
        )
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    async fn approval_mode_row(&self, scope: &str, id: &str) -> Result<Option<String>, DbError> {
        let row: Option<(String,)> = sqlx::query_as("SELECT mode FROM approval_modes WHERE scope = ? AND id = ?")
            .bind(scope)
            .bind(id)
            .fetch_optional(&self.pool)
            .await?;
        Ok(row.map(|r| r.0))
    }

    /// The space's approvals: `ask` (the default) or `off`.
    pub async fn space_approvals(&self, space: &SpaceId) -> Result<String, DbError> {
        Ok(self.approval_mode_row("space", space.as_str()).await?.unwrap_or_else(|| ASK.into()))
    }

    pub async fn set_space_approvals(&self, space: &SpaceId, mode: &str) -> Result<(), DbError> {
        sqlx::query(
            "INSERT INTO approval_modes (scope, id, mode, updated_at) VALUES ('space', ?, ?, ?)
             ON CONFLICT(scope, id) DO UPDATE SET mode = excluded.mode, updated_at = excluded.updated_at",
        )
        .bind(space.as_str())
        .bind(mode)
        .bind(Utc::now())
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// Sets a bot's override; `inherit` removes it.
    pub async fn set_kwant_approvals(&self, kwant_id: &str, mode: &str) -> Result<(), DbError> {
        if mode == INHERIT {
            sqlx::query("DELETE FROM approval_modes WHERE scope = 'kwant' AND id = ?").bind(kwant_id).execute(&self.pool).await?;
            return Ok(());
        }
        sqlx::query(
            "INSERT INTO approval_modes (scope, id, mode, updated_at) VALUES ('kwant', ?, ?, ?)
             ON CONFLICT(scope, id) DO UPDATE SET mode = excluded.mode, updated_at = excluded.updated_at",
        )
        .bind(kwant_id)
        .bind(mode)
        .bind(Utc::now())
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// Whether approvals are off for any existing space or bot. A space that is off counts even
    /// when every bot in it overrides to `ask`: a new bot there would inherit `off`.
    pub async fn any_approvals_off(&self) -> Result<bool, DbError> {
        let row: Option<(i64,)> = sqlx::query_as(
            "SELECT 1 FROM approval_modes WHERE mode = 'off' AND (
                (scope = 'space' AND id IN (SELECT id FROM spaces))
                OR (scope = 'kwant' AND id IN (SELECT id FROM kwants))
            ) LIMIT 1",
        )
        .fetch_optional(&self.pool)
        .await?;
        Ok(row.is_some())
    }

    /// A bot's approvals in `space`: its override, else the space's.
    pub async fn kwant_approvals_in(&self, space: &SpaceId, kwant_id: &str) -> Result<ApprovalMode, DbError> {
        let space_mode = self.space_approvals(space).await?;
        let bot = self.approval_mode_row("kwant", kwant_id).await?;
        let effective = bot.clone().unwrap_or_else(|| space_mode.clone());
        Ok(ApprovalMode { space: space_mode, bot: bot.unwrap_or_else(|| INHERIT.into()), effective })
    }

    /// A bot's approvals, in the space it lives in. An unknown bot asks.
    pub async fn kwant_approvals(&self, kwant_id: &str) -> Result<ApprovalMode, DbError> {
        match self.space_of_kwant(kwant_id).await? {
            Some(space) => self.kwant_approvals_in(&space, kwant_id).await,
            None => Ok(ApprovalMode { space: ASK.into(), bot: INHERIT.into(), effective: ASK.into() }),
        }
    }

    /// The approvals rows of a space's bots, newest first (max 200), optionally of one status
    /// (`auto_allowed` lists what ran while approvals were off) or one bot.
    pub async fn list_approvals(
        &self,
        space: &SpaceId,
        status: Option<&str>,
        kwant_id: Option<&str>,
    ) -> Result<Vec<super::models::Approval>, DbError> {
        Ok(sqlx::query_as::<_, super::models::Approval>(
            "SELECT a.* FROM approvals a JOIN kwants k ON k.id = a.kwant_id
             WHERE k.space_id = ?1 AND (?2 IS NULL OR a.status = ?2) AND (?3 IS NULL OR a.kwant_id = ?3)
             ORDER BY a.created_at DESC, a.rowid DESC LIMIT 200",
        )
        .bind(space.as_str())
        .bind(status)
        .bind(kwant_id)
        .fetch_all(&self.pool)
        .await?)
    }

    /// A card nobody answered before its turn's wait ran out: it stays open as `waiting`, with
    /// the chat (`{"channel_id"|"thread_id"|"direct_kwant_id": …}`) a late Allow resumes in.
    /// False when the ticket was no longer pending.
    pub async fn park_approval(&self, id: &str, chat: &serde_json::Value) -> Result<bool, DbError> {
        let done = sqlx::query("UPDATE approvals SET status = 'waiting', resume = ? WHERE id = ? AND status = 'pending'")
            .bind(chat.to_string())
            .bind(id)
            .execute(&self.pool)
            .await?;
        Ok(done.rows_affected() == 1)
    }

    /// Answers a `waiting` card once: sets `status` and returns the ticket with its chat. `None`
    /// when it isn't waiting (answered, or never parked) or is older than [`WAITING_APPROVAL_HOURS`],
    /// which is then marked `expired`. Two answers at once: only one gets the ticket.
    pub async fn claim_waiting_approval(
        &self,
        id: &str,
        status: &str,
    ) -> Result<Option<(super::models::Approval, serde_json::Value)>, DbError> {
        let window = format!("-{WAITING_APPROVAL_HOURS} hours");
        sqlx::query("UPDATE approvals SET status = 'expired' WHERE id = ? AND status = 'waiting' AND created_at <= datetime('now', ?)")
            .bind(id)
            .bind(&window)
            .execute(&self.pool)
            .await?;
        let done = sqlx::query("UPDATE approvals SET status = ? WHERE id = ? AND status = 'waiting'")
            .bind(status)
            .bind(id)
            .execute(&self.pool)
            .await?;
        if done.rows_affected() != 1 {
            return Ok(None);
        }
        let resume: Option<(Option<String>,)> =
            sqlx::query_as("SELECT resume FROM approvals WHERE id = ?").bind(id).fetch_optional(&self.pool).await?;
        let chat = resume
            .and_then(|r| r.0)
            .and_then(|r| serde_json::from_str(&r).ok())
            .unwrap_or(serde_json::Value::Null);
        Ok(self.get_approval(id).await?.map(|a| (a, chat)))
    }

    /// Cards still `waiting` for a late answer (younger than [`WAITING_APPROVAL_HOURS`]), with
    /// the chat each one resumes in, newest first.
    pub async fn away_waiting_approvals(&self) -> Result<Vec<super::away::AwayRow>, DbError> {
        let window = format!("-{WAITING_APPROVAL_HOURS} hours");
        Ok(sqlx::query_as(
            "SELECT 'approval' AS kind, a.id, a.kwant_id, k.name AS kwant_name, a.tool_name AS title, a.status,
                    a.parameters AS body, a.created_at AS at,
                    json_extract(a.resume, '$.thread_id') AS thread_id,
                    json_extract(a.resume, '$.channel_id') AS channel_id,
                    json_extract(a.resume, '$.direct_kwant_id') AS direct_kwant_id, NULL AS choices
             FROM approvals a LEFT JOIN kwants k ON k.id = a.kwant_id
             WHERE a.status = 'waiting' AND a.created_at > datetime('now', ?)
             ORDER BY a.created_at DESC, a.rowid DESC",
        )
        .bind(window)
        .fetch_all(&self.pool)
        .await?)
    }

    /// Moves a card's `created_at` back `hours` (tests of the waiting window).
    #[doc(hidden)]
    pub async fn backdate_approval_for_test(&self, id: &str, hours: i64) -> Result<(), DbError> {
        sqlx::query("UPDATE approvals SET created_at = datetime('now', ?) WHERE id = ?")
            .bind(format!("-{hours} hours"))
            .bind(id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    /// Records a call that ran without a card because approvals were off.
    /// Records a call kwnt refused on its own (it named kwnt's secret files), as `denied`.
    pub async fn save_refused(&self, input: &super::models::NewApproval) -> Result<String, DbError> {
        let id = uuid::Uuid::new_v4().to_string();
        sqlx::query("INSERT INTO approvals (id, turn_id, kwant_id, tool_name, parameters, status) VALUES (?, ?, ?, ?, ?, 'denied')")
            .bind(&id)
            .bind(&input.turn_id)
            .bind(&input.kwant_id)
            .bind(&input.tool_name)
            .bind(serde_json::to_string(&input.parameters)?)
            .execute(&self.pool)
            .await?;
        Ok(id)
    }

    pub async fn save_auto_allowed(&self, input: &super::models::NewApproval) -> Result<String, DbError> {
        let id = uuid::Uuid::new_v4().to_string();
        sqlx::query(
            "INSERT INTO approvals (id, turn_id, kwant_id, tool_name, parameters, status) VALUES (?, ?, ?, ?, ?, 'auto_allowed')",
        )
        .bind(&id)
        .bind(&input.turn_id)
        .bind(&input.kwant_id)
        .bind(&input.tool_name)
        .bind(serde_json::to_string(&input.parameters)?)
        .execute(&self.pool)
        .await?;
        Ok(id)
    }
}
