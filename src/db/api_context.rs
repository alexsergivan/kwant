//! What an API-key bot's conversation was last sent as its cached prompt prefix, so the next turn
//! can send the same bytes again (see `crate::drivers::api`).

use super::{Database, DbError};
use chrono::{DateTime, Utc};
use sqlx::FromRow;

/// One bot's prefix in one conversation (`context_id` is a channel, direct chat or thread).
#[derive(Debug, Clone, PartialEq, Eq, FromRow)]
pub struct ApiContextRow {
    pub kwant_id: String,
    pub context_id: String,
    /// The bot's persona, identity and soul when the prefix was written.
    pub profile: String,
    /// The team block, teammates and browser rules.
    pub team: String,
    /// Pinned instructions and memory.
    pub notes: String,
    /// When a turn last sent this prefix; the provider's cache lapses some minutes after it.
    pub used_at: DateTime<Utc>,
}

impl Database {
    pub(super) async fn migrate_api_context(&self) -> Result<(), DbError> {
        sqlx::query(
            "CREATE TABLE IF NOT EXISTS api_contexts (
                kwant_id TEXT NOT NULL,
                context_id TEXT NOT NULL,
                profile TEXT NOT NULL,
                team TEXT NOT NULL,
                notes TEXT NOT NULL,
                used_at TIMESTAMP NOT NULL,
                PRIMARY KEY (kwant_id, context_id)
            )",
        )
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn api_context(&self, kwant_id: &str, context_id: &str) -> Result<Option<ApiContextRow>, DbError> {
        Ok(sqlx::query_as(
            "SELECT kwant_id, context_id, profile, team, notes, used_at FROM api_contexts WHERE kwant_id = ? AND context_id = ?",
        )
        .bind(kwant_id)
        .bind(context_id)
        .fetch_optional(&self.pool)
        .await?)
    }

    /// Writes a new prefix, replacing the old one.
    pub async fn save_api_context(&self, row: &ApiContextRow) -> Result<(), DbError> {
        sqlx::query(
            "INSERT INTO api_contexts (kwant_id, context_id, profile, team, notes, used_at) VALUES (?, ?, ?, ?, ?, ?)
             ON CONFLICT (kwant_id, context_id) DO UPDATE SET
                profile = excluded.profile, team = excluded.team, notes = excluded.notes, used_at = excluded.used_at",
        )
        .bind(&row.kwant_id)
        .bind(&row.context_id)
        .bind(&row.profile)
        .bind(&row.team)
        .bind(&row.notes)
        .bind(row.used_at)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// Records that a turn sent the saved prefix again.
    pub async fn touch_api_context(&self, kwant_id: &str, context_id: &str, at: DateTime<Utc>) -> Result<(), DbError> {
        sqlx::query("UPDATE api_contexts SET used_at = ? WHERE kwant_id = ? AND context_id = ?")
            .bind(at)
            .bind(kwant_id)
            .bind(context_id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn a_saved_prefix_is_replaced_and_touched() {
        let tmp = tempfile::tempdir().unwrap();
        let db = Database::init(tmp.path()).await.unwrap();
        assert_eq!(db.api_context("k", "c").await.unwrap(), None);
        let t0 = Utc::now() - chrono::Duration::minutes(10);
        let mut row = ApiContextRow {
            kwant_id: "k".into(),
            context_id: "c".into(),
            profile: "p".into(),
            team: "t".into(),
            notes: "n".into(),
            used_at: t0,
        };
        db.save_api_context(&row).await.unwrap();
        row.notes = "n2".into();
        db.save_api_context(&row).await.unwrap();
        let t1 = Utc::now();
        db.touch_api_context("k", "c", t1).await.unwrap();
        let got = db.api_context("k", "c").await.unwrap().unwrap();
        assert_eq!(got.notes, "n2");
        assert_eq!(got.used_at.timestamp_millis(), t1.timestamp_millis());
        assert_eq!(db.api_context("k", "other").await.unwrap(), None);
    }
}
