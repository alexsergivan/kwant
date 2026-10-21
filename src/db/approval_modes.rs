//! Whether bots ask before their tools run. The person sets it for a space (`ask`, `smart` or
//! `off`) and may override it per bot (no row inherits the space). Bots never change it.
//! `smart` ("Ask only when risky") runs ordinary work in the bot's folder without a card and asks
//! for anything risky (see `mcp::shell_risk`).
//! Kept in its own table, outside exports, so an imported bundle can't switch approvals off.

use super::{spaces::SpaceId, Database, DbError};
use chrono::Utc;

/// The default for a space with no row, and for a bot with no override.
pub const ASK: &str = "ask";
pub const OFF: &str = "off";
/// Ask only when risky.
pub const SMART: &str = "smart";

/// A bot's setting as the person chose it: `inherit` follows its space.
pub const INHERIT: &str = "inherit";

/// What `approvals` resolves to for one bot.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct ApprovalMode {
    /// The space's value: `ask`, `smart` or `off`.
    pub space: String,
    /// The bot's own choice: `inherit`, `ask`, `smart` or `off`.
    pub bot: String,
    /// The bot's override, else the space's: `ask`, `smart` or `off`.
    pub effective: String,
}

impl ApprovalMode {
    pub fn is_off(&self) -> bool {
        self.effective == OFF
    }

    pub fn is_smart(&self) -> bool {
        self.effective == SMART
    }
}

/// `ask`, `smart` or `off`; anything else is not a mode.
pub fn valid_mode(mode: &str) -> bool {
    matches!(mode, ASK | SMART | OFF)
}

const CREATE_APPROVAL_MODES: &str = "CREATE TABLE IF NOT EXISTS approval_modes (
    scope TEXT NOT NULL CHECK (scope IN ('space', 'kwant')),
    id TEXT NOT NULL,
    mode TEXT NOT NULL CHECK (mode IN ('ask', 'smart', 'off')),
    updated_at TIMESTAMP NOT NULL,
    PRIMARY KEY (scope, id)
)";

impl Database {
    pub(super) async fn migrate_approval_modes(&self) -> Result<(), DbError> {
        sqlx::query(CREATE_APPROVAL_MODES).execute(&self.pool).await?;
        // Tables made before `smart` existed only allow 'ask' and 'off': rebuild them once
        let sql: Option<(String,)> = sqlx::query_as("SELECT sql FROM sqlite_master WHERE type = 'table' AND name = 'approval_modes'")
            .fetch_optional(&self.pool)
            .await?;
        if sql.is_some_and(|(sql,)| !sql.contains("'smart'")) {
            let mut tx = self.pool.begin().await?;
            sqlx::query("ALTER TABLE approval_modes RENAME TO approval_modes_old").execute(&mut *tx).await?;
            sqlx::query(CREATE_APPROVAL_MODES).execute(&mut *tx).await?;
            sqlx::query("INSERT INTO approval_modes (scope, id, mode, updated_at) SELECT scope, id, mode, updated_at FROM approval_modes_old")
                .execute(&mut *tx)
                .await?;
            sqlx::query("DROP TABLE approval_modes_old").execute(&mut *tx).await?;
            tx.commit().await?;
        }
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

    /// The space's approvals: `ask` (the default), `smart` or `off`.
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

    /// Whether approvals are off, or only ask when risky, for any existing space or bot. Both let
    /// a bot run code without a card, so both keep passwords out of the built-in store. A space
    /// counts even when every bot in it overrides to `ask`: a new bot there would inherit it.
    pub async fn any_approvals_off(&self) -> Result<bool, DbError> {
        let row: Option<(i64,)> = sqlx::query_as(
            "SELECT 1 FROM approval_modes WHERE mode IN ('off', 'smart') AND (
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

    /// Records a call that ran without a card because approvals were off (or it was safe under
    /// "Ask only when risky").
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
