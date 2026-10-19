//! MCP servers an agent suggested installing (`mcp_suggest`): a registry entry, the agent's
//! reason and the exact plan the person is shown. A suggestion is `pending` until the person
//! installs or dismisses it; only the person's install creates a server.

use super::models::{McpServer, Message, MessageExtra, NewMessage};
use super::spaces::SpaceId;
use super::{Database, DbError};
use chrono::{DateTime, Utc};
use serde::Serialize;
use sqlx::FromRow;
use uuid::Uuid;

/// Most suggestions one turn can make.
pub const MAX_PER_TURN: i64 = 3;
/// Most pending suggestions per agent; an agent can't flood the person.
pub const MAX_PENDING_PER_KWANT: i64 = 10;
pub const MAX_REASON_CHARS: usize = 300;

/// `installing` only while the person's install request runs; a restart puts it back to pending.
pub const SUGGESTION_STATUSES: [&str; 4] = ["pending", "installing", "installed", "dismissed"];

#[derive(Debug, Clone, Serialize, FromRow, PartialEq)]
pub struct McpSuggestion {
    pub id: String,
    pub space_id: String,
    pub kwant_id: String,
    pub turn_id: Option<String>,
    pub channel_id: Option<String>,
    pub direct_kwant_id: Option<String>,
    pub thread_id: Option<String>,
    pub registry_name: String,
    pub registry_version: String,
    pub title: String,
    pub description: String,
    pub repository_url: Option<String>,
    pub reason: String,
    /// The [`crate::mcp::registry::InstallPlan`] as JSON: what installing registers.
    #[serde(serialize_with = "as_json")]
    pub plan: String,
    pub status: String,
    /// The server the install created.
    pub server_id: Option<String>,
    /// Why the last install attempt failed (never contains secret values).
    pub last_error: Option<String>,
    /// The chat message that shows the suggestion card.
    pub message_id: Option<String>,
    pub created_at: DateTime<Utc>,
    pub resolved_at: Option<DateTime<Utc>>,
}

fn as_json<S: serde::Serializer>(raw: &str, s: S) -> Result<S::Ok, S::Error> {
    let v: serde_json::Value = serde_json::from_str(raw).unwrap_or(serde_json::Value::Null);
    v.serialize(s)
}

impl McpSuggestion {
    pub fn install_plan(&self) -> Option<crate::mcp::registry::InstallPlan> {
        serde_json::from_str(&self.plan).ok()
    }

    /// The chat the suggestion was made in: channel, thread or the direct chat's kwant.
    pub fn context_id(&self) -> String {
        self.channel_id
            .clone()
            .or_else(|| self.thread_id.clone())
            .or_else(|| self.direct_kwant_id.clone())
            .unwrap_or_else(|| self.kwant_id.clone())
    }
}

/// A suggestion to save; the id, status and time are set on insert.
#[derive(Debug, Clone, Default)]
pub struct NewMcpSuggestion {
    pub space_id: String,
    pub kwant_id: String,
    pub turn_id: Option<String>,
    pub channel_id: Option<String>,
    pub direct_kwant_id: Option<String>,
    pub thread_id: Option<String>,
    pub registry_name: String,
    pub registry_version: String,
    pub title: String,
    pub description: String,
    pub repository_url: Option<String>,
    pub reason: String,
    pub plan: String,
}

#[derive(Debug, thiserror::Error, PartialEq)]
pub enum SuggestionError {
    #[error("you already made {MAX_PER_TURN} suggestions this turn; carry on without the missing tools")]
    TurnLimit,
    #[error("you already have {MAX_PENDING_PER_KWANT} suggestions waiting for the person; carry on without the missing tools")]
    TooManyPending,
    #[error("database error")]
    Db,
}

impl From<DbError> for SuggestionError {
    fn from(_: DbError) -> Self {
        SuggestionError::Db
    }
}

impl From<sqlx::Error> for SuggestionError {
    fn from(_: sqlx::Error) -> Self {
        SuggestionError::Db
    }
}

const GET: &str = "SELECT * FROM mcp_suggestions WHERE id = ?";

impl Database {
    pub(super) async fn migrate_mcp_suggestions(&self) -> Result<(), DbError> {
        for stmt in [
            "CREATE TABLE IF NOT EXISTS mcp_suggestions (
                id TEXT PRIMARY KEY,
                space_id TEXT NOT NULL,
                kwant_id TEXT NOT NULL,
                turn_id TEXT,
                channel_id TEXT,
                direct_kwant_id TEXT,
                thread_id TEXT,
                registry_name TEXT NOT NULL,
                registry_version TEXT NOT NULL,
                title TEXT NOT NULL,
                description TEXT NOT NULL,
                repository_url TEXT,
                reason TEXT NOT NULL,
                plan TEXT NOT NULL,
                status TEXT NOT NULL CHECK (status IN ('pending', 'installing', 'installed', 'dismissed')),
                server_id TEXT,
                last_error TEXT,
                message_id TEXT,
                created_at TIMESTAMP NOT NULL,
                resolved_at TIMESTAMP
            )",
            "CREATE INDEX IF NOT EXISTS mcp_suggestions_by_space ON mcp_suggestions (space_id, status)",
            "CREATE INDEX IF NOT EXISTS mcp_suggestions_by_turn ON mcp_suggestions (turn_id)",
            // An install interrupted by a restart never finished
            "UPDATE mcp_suggestions SET status = 'pending' WHERE status = 'installing'",
        ] {
            sqlx::query(stmt).execute(&self.pool).await?;
        }
        Ok(())
    }

    /// Saves a suggestion together with the chat message that shows it (built by `card`), in one
    /// transaction. A pending suggestion of the same entry by the same agent is returned instead
    /// (`true` as the third value) and nothing new is posted.
    pub async fn create_mcp_suggestion_with_card(
        &self,
        new: &NewMcpSuggestion,
        card: impl FnOnce(&McpSuggestion) -> Option<(NewMessage, MessageExtra)>,
    ) -> Result<(McpSuggestion, Option<Message>, bool), SuggestionError> {
        // The first statement writes, so the transaction takes SQLite's write lock up front
        let mut tx = self.pool.begin().await?;
        let id = Uuid::new_v4().to_string();
        let inserted = sqlx::query(
            "INSERT INTO mcp_suggestions (id, space_id, kwant_id, turn_id, channel_id, direct_kwant_id, thread_id,
                 registry_name, registry_version, title, description, repository_url, reason, plan, status, created_at)
             SELECT ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, 'pending', ?
             WHERE NOT EXISTS (SELECT 1 FROM mcp_suggestions WHERE kwant_id = ? AND registry_name = ? AND status IN ('pending', 'installing'))
               AND (SELECT COUNT(*) FROM mcp_suggestions WHERE turn_id = ?) < ?
               AND (SELECT COUNT(*) FROM mcp_suggestions WHERE kwant_id = ? AND status = 'pending') < ?",
        )
        .bind(&id)
        .bind(&new.space_id)
        .bind(&new.kwant_id)
        .bind(&new.turn_id)
        .bind(&new.channel_id)
        .bind(&new.direct_kwant_id)
        .bind(&new.thread_id)
        .bind(&new.registry_name)
        .bind(&new.registry_version)
        .bind(&new.title)
        .bind(&new.description)
        .bind(&new.repository_url)
        .bind(&new.reason)
        .bind(&new.plan)
        .bind(Utc::now())
        .bind(&new.kwant_id)
        .bind(&new.registry_name)
        .bind(&new.turn_id)
        .bind(MAX_PER_TURN)
        .bind(&new.kwant_id)
        .bind(MAX_PENDING_PER_KWANT)
        .execute(&mut *tx)
        .await?;
        if inserted.rows_affected() == 0 {
            let existing = sqlx::query_as::<_, McpSuggestion>(
                "SELECT * FROM mcp_suggestions WHERE kwant_id = ? AND registry_name = ? AND status IN ('pending', 'installing')
                 ORDER BY created_at DESC LIMIT 1",
            )
            .bind(&new.kwant_id)
            .bind(&new.registry_name)
            .fetch_optional(&mut *tx)
            .await?;
            if let Some(existing) = existing {
                return Ok((existing, None, true));
            }
            let in_turn: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM mcp_suggestions WHERE turn_id = ?")
                .bind(&new.turn_id)
                .fetch_one(&mut *tx)
                .await?;
            return Err(if new.turn_id.is_some() && in_turn.0 >= MAX_PER_TURN {
                SuggestionError::TurnLimit
            } else {
                SuggestionError::TooManyPending
            });
        }
        let mut s = sqlx::query_as::<_, McpSuggestion>(GET).bind(&id).fetch_one(&mut *tx).await?;
        let mut message = None;
        if let Some((msg, extra)) = card(&s) {
            let saved = super::insert_message_on(&mut tx, &msg, &extra).await?;
            sqlx::query("UPDATE mcp_suggestions SET message_id = ? WHERE id = ?")
                .bind(&saved.id)
                .bind(&id)
                .execute(&mut *tx)
                .await?;
            s = sqlx::query_as::<_, McpSuggestion>(GET).bind(&id).fetch_one(&mut *tx).await?;
            message = Some(saved);
        }
        tx.commit().await?;
        Ok((s, message, false))
    }

    pub async fn get_mcp_suggestion(&self, id: &str) -> Result<Option<McpSuggestion>, DbError> {
        Ok(sqlx::query_as::<_, McpSuggestion>(GET).bind(id).fetch_optional(&self.pool).await?)
    }

    /// A space's suggestions, newest first, at most 100; optionally by status and/or agent.
    pub async fn list_mcp_suggestions(
        &self,
        space: &SpaceId,
        status: Option<&str>,
        kwant_id: Option<&str>,
    ) -> Result<Vec<McpSuggestion>, DbError> {
        Ok(sqlx::query_as::<_, McpSuggestion>(
            "SELECT * FROM mcp_suggestions WHERE space_id = ?1 AND (?2 IS NULL OR status = ?2) AND (?3 IS NULL OR kwant_id = ?3)
             ORDER BY created_at DESC LIMIT 100",
        )
        .bind(space.as_str())
        .bind(status)
        .bind(kwant_id)
        .fetch_all(&self.pool)
        .await?)
    }

    /// Registry names installed through suggestions whose server still exists: `(registry name, server id)`.
    pub async fn installed_mcp_suggestions(&self, space: &SpaceId) -> Result<Vec<(String, String)>, DbError> {
        Ok(sqlx::query_as(
            "SELECT g.registry_name, g.server_id FROM mcp_suggestions g JOIN mcp_servers s ON s.id = g.server_id
             WHERE g.space_id = ? AND g.status = 'installed'",
        )
        .bind(space.as_str())
        .fetch_all(&self.pool)
        .await?)
    }

    /// Takes a pending suggestion for one install attempt; `false` when it isn't pending.
    pub async fn claim_mcp_suggestion(&self, id: &str) -> Result<bool, DbError> {
        let done = sqlx::query("UPDATE mcp_suggestions SET status = 'installing' WHERE id = ? AND status = 'pending'")
            .bind(id)
            .execute(&self.pool)
            .await?;
        Ok(done.rows_affected() == 1)
    }

    /// Puts a suggestion whose install failed back to pending, with why.
    pub async fn release_mcp_suggestion(&self, id: &str, error: &str) -> Result<(), DbError> {
        sqlx::query("UPDATE mcp_suggestions SET status = 'pending', last_error = ? WHERE id = ? AND status = 'installing'")
            .bind(error)
            .bind(id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    pub async fn mark_mcp_suggestion_installed(&self, id: &str, server_id: &str) -> Result<(), DbError> {
        sqlx::query(
            "UPDATE mcp_suggestions SET status = 'installed', server_id = ?, last_error = NULL, resolved_at = ?
             WHERE id = ? AND status = 'installing'",
        )
        .bind(server_id)
        .bind(Utc::now())
        .bind(id)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// Dismisses a pending suggestion; `false` when it wasn't pending.
    pub async fn dismiss_mcp_suggestion(&self, id: &str) -> Result<bool, DbError> {
        let done = sqlx::query(
            "UPDATE mcp_suggestions SET status = 'dismissed', resolved_at = ? WHERE id = ? AND status = 'pending'",
        )
        .bind(Utc::now())
        .bind(id)
        .execute(&self.pool)
        .await?;
        Ok(done.rows_affected() == 1)
    }

    /// Adds `server` to `space` switched on only for `kwant_id`: every other kwant in the space
    /// gets an opt-out, in the same transaction, so no other agent sees it even for a moment.
    pub async fn create_mcp_server_for_kwant(
        &self,
        space: &SpaceId,
        server: &McpServer,
        kwant_id: &str,
    ) -> Result<McpServer, DbError> {
        let id = Uuid::new_v4().to_string();
        let mut tx = self.pool.begin().await?;
        sqlx::query(
            "INSERT INTO mcp_servers (id, name, transport, command, args, url, env, headers, enabled, timeout_secs, space_id) \
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(&id)
        .bind(&server.name)
        .bind(&server.transport)
        .bind(&server.command)
        .bind(serde_json::to_string(&server.args)?)
        .bind(&server.url)
        .bind(serde_json::to_string(&server.env)?)
        .bind(serde_json::to_string(&server.headers)?)
        .bind(server.enabled)
        .bind(server.timeout_secs)
        .bind(space.as_str())
        .execute(&mut *tx)
        .await?;
        sqlx::query(
            "INSERT OR IGNORE INTO kwant_mcp_disabled (kwant_id, server_id) SELECT id, ? FROM kwants WHERE space_id = ? AND id != ?",
        )
        .bind(&id)
        .bind(space.as_str())
        .bind(kwant_id)
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        self.get_mcp_server(&id).await?.ok_or(DbError::Sqlx(sqlx::Error::RowNotFound))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn new(kwant: &str, turn: &str, name: &str) -> NewMcpSuggestion {
        NewMcpSuggestion {
            space_id: "spc_default".into(),
            kwant_id: kwant.into(),
            turn_id: Some(turn.into()),
            direct_kwant_id: Some(kwant.into()),
            registry_name: name.into(),
            registry_version: "1".into(),
            title: name.into(),
            plan: "{}".into(),
            ..Default::default()
        }
    }

    #[tokio::test]
    async fn limits_and_duplicates() {
        let tmp = tempfile::tempdir().unwrap();
        let db = Database::init(tmp.path()).await.unwrap();
        let (a, _, dup) = db.create_mcp_suggestion_with_card(&new("k", "t1", "a/one"), |_| None).await.unwrap();
        assert!(!dup && a.status == "pending");
        let (again, _, dup) = db.create_mcp_suggestion_with_card(&new("k", "t1", "a/one"), |_| None).await.unwrap();
        assert!(dup && again.id == a.id);
        db.create_mcp_suggestion_with_card(&new("k", "t1", "a/two"), |_| None).await.unwrap();
        db.create_mcp_suggestion_with_card(&new("k", "t1", "a/three"), |_| None).await.unwrap();
        let err = db.create_mcp_suggestion_with_card(&new("k", "t1", "a/four"), |_| None).await.unwrap_err();
        assert_eq!(err, SuggestionError::TurnLimit);

        assert!(db.claim_mcp_suggestion(&a.id).await.unwrap());
        assert!(!db.claim_mcp_suggestion(&a.id).await.unwrap());
        assert!(!db.dismiss_mcp_suggestion(&a.id).await.unwrap());
        db.release_mcp_suggestion(&a.id, "boom").await.unwrap();
        let back = db.get_mcp_suggestion(&a.id).await.unwrap().unwrap();
        assert_eq!((back.status.as_str(), back.last_error.as_deref()), ("pending", Some("boom")));
        assert!(db.dismiss_mcp_suggestion(&a.id).await.unwrap());

        for i in 0..MAX_PENDING_PER_KWANT {
            let _ = db.create_mcp_suggestion_with_card(&new("k", &format!("u{i}"), &format!("b/n{i}")), |_| None).await;
        }
        let err = db.create_mcp_suggestion_with_card(&new("k", "u-last", "c/x"), |_| None).await.unwrap_err();
        assert_eq!(err, SuggestionError::TooManyPending);
    }
}
