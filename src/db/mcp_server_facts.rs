//! What an MCP server said about itself the last time kwnt connected to it (26f): its
//! `serverInfo.name`. Home Assistant's (`home-assistant`) makes its locks, doors and alarms
//! always ask ([`crate::mcp::home_floor`]). Kept beside `mcp_servers` rather than in it, so an
//! older kwnt reading the servers table is untouched; it goes with its server, and when the
//! server's address changes.

use super::{Database, DbError};
use chrono::Utc;

const CREATE_MCP_SERVER_FACTS: &str = "CREATE TABLE IF NOT EXISTS mcp_server_facts (
    server_id TEXT PRIMARY KEY,
    reported_name TEXT,
    seen_at TIMESTAMP NOT NULL
)";

/// A deleted server takes its facts with it, whichever path deleted it.
const SERVER_GONE: &str = "CREATE TRIGGER IF NOT EXISTS mcp_server_facts_server_gone AFTER DELETE ON mcp_servers BEGIN \
    DELETE FROM mcp_server_facts WHERE server_id = OLD.id; END";

impl Database {
    pub(super) async fn migrate_mcp_server_facts(&self) -> Result<(), DbError> {
        sqlx::query(CREATE_MCP_SERVER_FACTS).execute(&self.pool).await?;
        sqlx::query(SERVER_GONE).execute(&self.pool).await?;
        Ok(())
    }

    /// Stores what server `server_id` said its name is (`None`: it sent none).
    pub async fn save_mcp_reported_name(&self, server_id: &str, reported_name: Option<&str>) -> Result<(), DbError> {
        sqlx::query(
            "INSERT INTO mcp_server_facts (server_id, reported_name, seen_at) VALUES (?, ?, ?) \
             ON CONFLICT(server_id) DO UPDATE SET reported_name = excluded.reported_name, seen_at = excluded.seen_at",
        )
        .bind(server_id)
        .bind(reported_name)
        .bind(Utc::now())
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// What server `server_id` last said its name is: `None` when kwnt never stored it,
    /// `Some(None)` when the server sent none.
    pub async fn mcp_reported_name(&self, server_id: &str) -> Result<Option<Option<String>>, DbError> {
        let row: Option<(Option<String>,)> = sqlx::query_as("SELECT reported_name FROM mcp_server_facts WHERE server_id = ?")
            .bind(server_id)
            .fetch_optional(&self.pool)
            .await?;
        Ok(row.map(|(name,)| name))
    }

    /// Forgets what server `server_id` said about itself (its address changed).
    pub async fn forget_mcp_server_facts(&self, server_id: &str) -> Result<(), DbError> {
        sqlx::query("DELETE FROM mcp_server_facts WHERE server_id = ?").bind(server_id).execute(&self.pool).await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use crate::db::{models::McpServer, spaces::SpaceId, Database};

    #[tokio::test]
    async fn facts_are_stored_replaced_and_go_with_their_server() {
        let tmp = tempfile::tempdir().unwrap();
        let db = Database::init(tmp.path()).await.unwrap();
        let server = McpServer {
            id: String::new(),
            name: "house".into(),
            transport: "http".into(),
            command: None,
            args: vec![],
            url: Some("http://homeassistant.lan:8123/api/mcp".into()),
            env: Default::default(),
            headers: Default::default(),
            enabled: true,
            timeout_secs: 10,
            created_at: None,
        };
        let id = db.create_mcp_server(&SpaceId::default_space(), &server).await.unwrap().id;
        assert_eq!(db.mcp_reported_name(&id).await.unwrap(), None);
        db.save_mcp_reported_name(&id, Some("home-assistant")).await.unwrap();
        assert_eq!(db.mcp_reported_name(&id).await.unwrap(), Some(Some("home-assistant".into())));
        db.save_mcp_reported_name(&id, None).await.unwrap();
        assert_eq!(db.mcp_reported_name(&id).await.unwrap(), Some(None));
        db.save_mcp_reported_name(&id, Some("home-assistant")).await.unwrap();
        db.forget_mcp_server_facts(&id).await.unwrap();
        assert_eq!(db.mcp_reported_name(&id).await.unwrap(), None);
        db.save_mcp_reported_name(&id, Some("home-assistant")).await.unwrap();
        assert!(db.delete_mcp_server(&id).await.unwrap());
        assert_eq!(db.mcp_reported_name(&id).await.unwrap(), None, "goes with its server");
    }
}
