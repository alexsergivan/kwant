//! LLM profiles: several accounts per engine, which kwants are bound to and fail over between.

use super::models::*;
use super::spaces::SpaceId;
use super::{Database, DbError};
use chrono::{DateTime, Utc};
use std::collections::{HashMap, HashSet};
use uuid::Uuid;

/// Engines that have profiles.
pub const PROFILE_ENGINES: [&str; 3] = ["claude", "codex", "grok"];

/// Id of an engine's built-in profile: the CLI's own login or the Settings key, as before profiles.
pub fn system_profile_id(engine: &str) -> String {
    format!("prf_system_{engine}")
}

/// Id of an engine's This machine profile in a space. The Default space keeps the old ids.
pub fn system_profile_id_in(space: &SpaceId, engine: &str) -> String {
    if space.is_default() {
        system_profile_id(engine)
    } else {
        format!("prf_system_{engine}_{space}")
    }
}

const PROFILE_COLUMNS: &str =
    "id, engine, name, kind, home_dir, priority, enabled, is_default, created_at, space_id";

impl Database {
    /// Adds the kwant binding columns and one system profile per engine. Existing setups end up
    /// with every kwant on its engine's system profile, which runs exactly as kwnt did before.
    pub(super) async fn migrate_profiles(&self) -> Result<(), DbError> {
        let _ = sqlx::query("ALTER TABLE kwants ADD COLUMN profile_id TEXT;")
            .execute(&self.pool)
            .await;
        let _ = sqlx::query(
            "ALTER TABLE kwants ADD COLUMN profile_failover INTEGER NOT NULL DEFAULT 1;",
        )
        .execute(&self.pool)
        .await;
        for engine in PROFILE_ENGINES {
            sqlx::query(
                "INSERT OR IGNORE INTO llm_profiles (id, engine, name, kind, home_dir, priority, enabled, is_default)
                 VALUES (?, ?, 'This machine', 'system', NULL, 0, 1,
                         NOT EXISTS (SELECT 1 FROM llm_profiles WHERE engine = ? AND is_default = 1))",
            )
            .bind(system_profile_id(engine))
            .bind(engine)
            .bind(engine)
            .execute(&self.pool)
            .await?;
        }
        Ok(())
    }

    /// The space's profiles, an engine's default first and then by priority.
    pub async fn list_profiles(&self, space: &SpaceId) -> Result<Vec<LlmProfile>, DbError> {
        let rows = sqlx::query_as::<_, LlmProfile>(&format!(
            "SELECT {PROFILE_COLUMNS} FROM llm_profiles WHERE space_id = ? ORDER BY engine, is_default DESC, priority, created_at, id"
        ))
        .bind(space.as_str())
        .fetch_all(&self.pool)
        .await?;
        Ok(rows)
    }

    pub async fn get_profile(&self, id: &str) -> Result<Option<LlmProfile>, DbError> {
        let row = sqlx::query_as::<_, LlmProfile>(&format!(
            "SELECT {PROFILE_COLUMNS} FROM llm_profiles WHERE id = ?"
        ))
        .bind(id)
        .fetch_optional(&self.pool)
        .await?;
        Ok(row)
    }

    /// Fails with a unique-constraint error on a duplicate name or home dir.
    pub async fn create_profile(&self, space: &SpaceId, input: &NewLlmProfile) -> Result<LlmProfile, DbError> {
        let id = format!("prf_{}", Uuid::new_v4().simple());
        let mut tx = self.pool.begin().await?;
        if input.is_default {
            sqlx::query("UPDATE llm_profiles SET is_default = 0 WHERE engine = ? AND space_id = ?")
                .bind(&input.engine)
                .bind(space.as_str())
                .execute(&mut *tx)
                .await?;
        }
        sqlx::query(
            "INSERT INTO llm_profiles (id, engine, name, kind, home_dir, priority, enabled, is_default, space_id)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(&id)
        .bind(&input.engine)
        .bind(&input.name)
        .bind(&input.kind)
        .bind(&input.home_dir)
        .bind(input.priority)
        .bind(input.enabled)
        .bind(input.is_default)
        .bind(space.as_str())
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        self.get_profile(&id)
            .await?
            .ok_or(DbError::Sqlx(sqlx::Error::RowNotFound))
    }

    pub async fn update_profile(
        &self,
        id: &str,
        patch: &LlmProfilePatch,
    ) -> Result<Option<LlmProfile>, DbError> {
        let Some(existing) = self.get_profile(id).await? else {
            return Ok(None);
        };
        let mut tx = self.pool.begin().await?;
        if patch.is_default == Some(true) && !existing.is_default {
            sqlx::query("UPDATE llm_profiles SET is_default = 0 WHERE engine = ? AND space_id = ?")
                .bind(&existing.engine)
                .bind(&existing.space_id)
                .execute(&mut *tx)
                .await?;
            sqlx::query("UPDATE llm_profiles SET is_default = 1 WHERE id = ?")
                .bind(id)
                .execute(&mut *tx)
                .await?;
        }
        sqlx::query("UPDATE llm_profiles SET name = ?, priority = ?, enabled = ? WHERE id = ?")
            .bind(patch.name.as_ref().unwrap_or(&existing.name))
            .bind(patch.priority.unwrap_or(existing.priority))
            .bind(patch.enabled.unwrap_or(existing.enabled))
            .bind(id)
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        self.get_profile(id).await
    }

    /// Deletes a profile, its key and state. Kwants bound to it go back to the engine default,
    /// and a deleted default hands that role to the space's system profile. Never touches the home dir.
    pub async fn delete_profile(&self, id: &str) -> Result<bool, DbError> {
        let Some(existing) = self.get_profile(id).await? else {
            return Ok(false);
        };
        let endpoints = self.custom_api_ready().await;
        let mut tx = self.pool.begin().await?;
        if endpoints {
            sqlx::query("DELETE FROM llm_profile_endpoints WHERE profile_id = ?")
                .bind(id)
                .execute(&mut *tx)
                .await?;
        }
        for sql in [
            "DELETE FROM llm_profile_secrets WHERE profile_id = ?",
            "DELETE FROM llm_profile_state WHERE profile_id = ?",
            "UPDATE kwants SET profile_id = NULL WHERE profile_id = ?",
            "DELETE FROM llm_profiles WHERE id = ?",
        ] {
            sqlx::query(sql).bind(id).execute(&mut *tx).await?;
        }
        if existing.is_default {
            sqlx::query("UPDATE llm_profiles SET is_default = 1 WHERE id = ?")
                .bind(system_profile_id_in(&SpaceId::new(&existing.space_id), &existing.engine))
                .execute(&mut *tx)
                .await?;
        }
        tx.commit().await?;
        Ok(true)
    }

    pub async fn set_profile_key(&self, id: &str, key: &ProfileKey) -> Result<(), DbError> {
        sqlx::query(
            "INSERT INTO llm_profile_secrets (profile_id, api_key, updated_at) VALUES (?, ?, CURRENT_TIMESTAMP)
             ON CONFLICT(profile_id) DO UPDATE SET api_key = excluded.api_key, updated_at = CURRENT_TIMESTAMP",
        )
        .bind(id)
        .bind(&key.0)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn delete_profile_key(&self, id: &str) -> Result<bool, DbError> {
        let res = sqlx::query("DELETE FROM llm_profile_secrets WHERE profile_id = ?")
            .bind(id)
            .execute(&self.pool)
            .await?;
        Ok(res.rows_affected() > 0)
    }

    pub async fn get_profile_key(&self, id: &str) -> Result<Option<ProfileKey>, DbError> {
        let row: Option<(String,)> =
            sqlx::query_as("SELECT api_key FROM llm_profile_secrets WHERE profile_id = ?")
                .bind(id)
                .fetch_optional(&self.pool)
                .await?;
        Ok(row.map(|(k,)| ProfileKey(k)))
    }

    /// Ids of profiles with a stored key, without reading the keys.
    pub async fn profile_ids_with_key(&self) -> Result<HashSet<String>, DbError> {
        let rows: Vec<(String,)> = sqlx::query_as("SELECT profile_id FROM llm_profile_secrets")
            .fetch_all(&self.pool)
            .await?;
        Ok(rows.into_iter().map(|(id,)| id).collect())
    }

    pub async fn list_profile_states(&self) -> Result<HashMap<String, LlmProfileState>, DbError> {
        let rows = sqlx::query_as::<_, LlmProfileState>(
            "SELECT profile_id, limited_until, last_error FROM llm_profile_state",
        )
        .fetch_all(&self.pool)
        .await?;
        Ok(rows
            .into_iter()
            .map(|s| (s.profile_id.clone(), s))
            .collect())
    }

    /// Records why a profile failed. `limited_until` skips it in failover until then.
    pub async fn record_profile_failure(
        &self,
        id: &str,
        class: &str,
        limited_until: Option<DateTime<Utc>>,
    ) -> Result<(), DbError> {
        sqlx::query(
            "INSERT INTO llm_profile_state (profile_id, limited_until, last_error, updated_at)
             VALUES (?, ?, ?, CURRENT_TIMESTAMP)
             ON CONFLICT(profile_id) DO UPDATE SET limited_until = excluded.limited_until,
                 last_error = excluded.last_error, updated_at = CURRENT_TIMESTAMP",
        )
        .bind(id)
        .bind(limited_until)
        .bind(class)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// Forgets a profile's failure after it served a turn, or after a new login or key.
    pub async fn clear_profile_state(&self, id: &str) -> Result<(), DbError> {
        sqlx::query("DELETE FROM llm_profile_state WHERE profile_id = ?")
            .bind(id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    /// A kwant's profile binding: its profile (`None` = engine default) and whether it may fail over.
    pub async fn get_kwant_binding(
        &self,
        kwant_id: &str,
    ) -> Result<Option<(Option<String>, bool)>, DbError> {
        let row: Option<(Option<String>, bool)> =
            sqlx::query_as("SELECT profile_id, profile_failover FROM kwants WHERE id = ?")
                .bind(kwant_id)
                .fetch_optional(&self.pool)
                .await?;
        Ok(row)
    }

    pub async fn list_kwant_bindings(
        &self,
        space: &SpaceId,
    ) -> Result<HashMap<String, (Option<String>, bool)>, DbError> {
        let rows: Vec<(String, Option<String>, bool)> =
            sqlx::query_as("SELECT id, profile_id, profile_failover FROM kwants WHERE space_id = ?")
                .bind(space.as_str())
                .fetch_all(&self.pool)
                .await?;
        Ok(rows.into_iter().map(|(id, p, f)| (id, (p, f))).collect())
    }

    /// `profile_id`: `None` leaves it, `Some(None)` clears it. `failover`: `None` leaves it.
    /// Fails with [`DbError::OtherSpace`] for a profile outside the kwant's space.
    pub async fn set_kwant_binding(
        &self,
        kwant_id: &str,
        profile_id: Option<Option<&str>>,
        failover: Option<bool>,
    ) -> Result<(), DbError> {
        if let Some(Some(profile)) = profile_id {
            let kwant_space = self.space_of_kwant(kwant_id).await?;
            if kwant_space.is_none() || self.space_of_profile(profile).await? != kwant_space {
                return Err(DbError::OtherSpace("that account belongs to another space".into()));
            }
        }
        if let Some(profile_id) = profile_id {
            sqlx::query("UPDATE kwants SET profile_id = ? WHERE id = ?")
                .bind(profile_id)
                .bind(kwant_id)
                .execute(&self.pool)
                .await?;
        }
        if let Some(failover) = failover {
            sqlx::query("UPDATE kwants SET profile_failover = ? WHERE id = ?")
                .bind(failover)
                .bind(kwant_id)
                .execute(&self.pool)
                .await?;
        }
        Ok(())
    }

    pub async fn kwant_views(&self, space: &SpaceId) -> Result<Vec<KwantView>, DbError> {
        let kwants = self.list_kwants(space).await?;
        let mut bindings = self.list_kwant_bindings(space).await?;
        Ok(kwants
            .into_iter()
            .map(|kwant| {
                let (profile_id, profile_failover) =
                    bindings.remove(&kwant.id).unwrap_or((None, true));
                KwantView {
                    kwant,
                    profile_id,
                    profile_failover,
                }
            })
            .collect())
    }

    pub async fn kwant_view(&self, id: &str) -> Result<Option<KwantView>, DbError> {
        let Some(kwant) = self.get_kwant(id).await? else {
            return Ok(None);
        };
        let (profile_id, profile_failover) =
            self.get_kwant_binding(id).await?.unwrap_or((None, true));
        Ok(Some(KwantView {
            kwant,
            profile_id,
            profile_failover,
        }))
    }
}
