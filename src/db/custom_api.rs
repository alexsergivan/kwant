//! Custom API accounts: a profile of engine `custom` and kind `custom_api`, whose server (protocol,
//! base URL, model) lives in `llm_profile_endpoints`. Its key, if any, is stored like every other
//! profile key, in `llm_profile_secrets`.
//!
//! Schema version 2 widens `llm_profiles`' CHECKs for the new engine and kind and adds the
//! endpoints table, in one transaction that rolls back on any error. A failed migration leaves
//! version 1, which this release still runs on without Custom API accounts, and retries at the
//! next start. A release from before version 2 still starts on it but doesn't know `custom` rows:
//! it lists them under no engine and never routes to them.

use super::spaces::{foreign_key_violations, rebuild, SPACES_SCHEMA_VERSION};
use super::{Database, DbError};
use serde::{Deserialize, Serialize};
use sqlx::{Connection, FromRow, SqliteConnection};

/// `PRAGMA user_version` once Custom API accounts can be stored.
pub const CUSTOM_API_SCHEMA_VERSION: i64 = 2;

/// Where a Custom API account sends its turns.
#[derive(Debug, Clone, PartialEq, Eq, FromRow, Serialize, Deserialize)]
pub struct ProfileEndpoint {
    pub profile_id: String,
    /// "openai-chat" or "anthropic-messages".
    pub protocol: String,
    /// Normalized: no trailing slash, ends at the API root (`…/v1`).
    pub base_url: String,
    /// The model entered by hand: used when the server lists none, and as the account's
    /// default when a bot doesn't pin one.
    pub model: Option<String>,
}

const LLM_PROFILES_V2: &str = "CREATE TABLE llm_profiles__spaces_new (
    id TEXT PRIMARY KEY,
    engine TEXT NOT NULL CHECK (engine IN ('claude', 'codex', 'grok', 'custom')),
    name TEXT NOT NULL,
    kind TEXT NOT NULL CHECK (kind IN ('system', 'cli_home', 'api_key', 'custom_api')),
    home_dir TEXT UNIQUE,
    priority INTEGER NOT NULL DEFAULT 100,
    enabled INTEGER NOT NULL DEFAULT 1,
    is_default INTEGER NOT NULL DEFAULT 0,
    created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
    space_id TEXT NOT NULL DEFAULT 'spc_default',
    UNIQUE (space_id, engine, name COLLATE NOCASE)
)";

const AFTER_REBUILD: [&str; 2] = [
    "CREATE UNIQUE INDEX llm_profiles_one_default ON llm_profiles (space_id, engine) WHERE is_default = 1",
    "CREATE INDEX llm_profiles_by_space ON llm_profiles (space_id, engine)",
];

const ENDPOINTS_TABLE: &str = "CREATE TABLE llm_profile_endpoints (
    profile_id TEXT PRIMARY KEY,
    protocol TEXT NOT NULL CHECK (protocol IN ('openai-chat', 'anthropic-messages')),
    base_url TEXT NOT NULL,
    model TEXT,
    updated_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
    FOREIGN KEY(profile_id) REFERENCES llm_profiles(id) ON DELETE CASCADE
)";

impl Database {
    /// Brings a spaces database to [`CUSTOM_API_SCHEMA_VERSION`]. Never fails startup: an error
    /// is logged and the database stays as it was. Returns whether the schema is now in place.
    pub(super) async fn migrate_custom_api(&self) -> Result<bool, DbError> {
        let version = self.schema_version().await?;
        if version >= CUSTOM_API_SCHEMA_VERSION {
            return Ok(true);
        }
        if version < SPACES_SCHEMA_VERSION {
            return Ok(false);
        }
        let mut conn = self.pool.acquire().await?;
        // Table rebuilds need foreign keys off, and SQLite ignores this pragma inside a transaction
        sqlx::query("PRAGMA foreign_keys = OFF").execute(&mut *conn).await?;
        let result = custom_api_in_transaction(&mut conn).await;
        sqlx::query("PRAGMA foreign_keys = ON").execute(&mut *conn).await?;
        match result {
            Ok(()) => {
                tracing::info!("Database ready for Custom API accounts");
                Ok(true)
            }
            Err(e) => {
                tracing::error!(
                    "Custom API accounts are unavailable: the schema change failed and was rolled back ({e})"
                );
                Ok(false)
            }
        }
    }

    /// Whether Custom API accounts can be stored (the migration ran).
    pub async fn custom_api_ready(&self) -> bool {
        self.schema_version().await.is_ok_and(|v| v >= CUSTOM_API_SCHEMA_VERSION)
    }

    pub async fn get_profile_endpoint(&self, profile_id: &str) -> Result<Option<ProfileEndpoint>, DbError> {
        if !self.custom_api_ready().await {
            return Ok(None);
        }
        Ok(sqlx::query_as::<_, ProfileEndpoint>(
            "SELECT profile_id, protocol, base_url, model FROM llm_profile_endpoints WHERE profile_id = ?",
        )
        .bind(profile_id)
        .fetch_optional(&self.pool)
        .await?)
    }

    /// Every endpoint, by profile id.
    pub async fn list_profile_endpoints(
        &self,
    ) -> Result<std::collections::HashMap<String, ProfileEndpoint>, DbError> {
        if !self.custom_api_ready().await {
            return Ok(Default::default());
        }
        let rows = sqlx::query_as::<_, ProfileEndpoint>(
            "SELECT profile_id, protocol, base_url, model FROM llm_profile_endpoints",
        )
        .fetch_all(&self.pool)
        .await?;
        Ok(rows.into_iter().map(|e| (e.profile_id.clone(), e)).collect())
    }

    pub async fn set_profile_endpoint(&self, endpoint: &ProfileEndpoint) -> Result<(), DbError> {
        sqlx::query(
            "INSERT INTO llm_profile_endpoints (profile_id, protocol, base_url, model, updated_at)
             VALUES (?, ?, ?, ?, CURRENT_TIMESTAMP)
             ON CONFLICT(profile_id) DO UPDATE SET protocol = excluded.protocol,
                 base_url = excluded.base_url, model = excluded.model, updated_at = CURRENT_TIMESTAMP",
        )
        .bind(&endpoint.profile_id)
        .bind(&endpoint.protocol)
        .bind(&endpoint.base_url)
        .bind(&endpoint.model)
        .execute(&self.pool)
        .await?;
        Ok(())
    }
}

async fn custom_api_in_transaction(conn: &mut SqliteConnection) -> Result<(), DbError> {
    // Violations already in the data aren't ours to fix; only new ones abort the migration
    let before = foreign_key_violations(conn).await?;
    let mut tx = conn.begin().await?;
    rebuild(&mut tx, "llm_profiles", LLM_PROFILES_V2, &AFTER_REBUILD).await?;
    sqlx::query(ENDPOINTS_TABLE).execute(&mut *tx).await?;
    let after = foreign_key_violations(&mut tx).await?;
    let new: Vec<_> = after
        .iter()
        .filter(|(key, n)| **n > before.get(*key).copied().unwrap_or(0))
        .collect();
    if !new.is_empty() {
        return Err(DbError::Migration(format!("foreign key check failed: {new:?}")));
    }
    sqlx::query(&format!("PRAGMA user_version = {CUSTOM_API_SCHEMA_VERSION}"))
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok(())
}
