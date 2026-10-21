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

/// A conversation's summary of its older messages, for one bot.
#[derive(Debug, Clone, PartialEq, Eq, FromRow)]
pub struct ApiSummary {
    pub summary: String,
    /// The newest message the summary covers.
    pub through_id: String,
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
                summary TEXT NOT NULL DEFAULT '',
                summary_through TEXT,
                PRIMARY KEY (kwant_id, context_id)
            )",
        )
        .execute(&self.pool)
        .await?;
        // Summaries came after the first prefixes
        let has_summary: Option<(String,)> =
            sqlx::query_as("SELECT name FROM pragma_table_info('api_contexts') WHERE name = 'summary'").fetch_optional(&self.pool).await?;
        if has_summary.is_none() {
            sqlx::query("ALTER TABLE api_contexts ADD COLUMN summary TEXT NOT NULL DEFAULT ''").execute(&self.pool).await?;
            sqlx::query("ALTER TABLE api_contexts ADD COLUMN summary_through TEXT").execute(&self.pool).await?;
        }
        Ok(())
    }

    pub async fn api_summary(&self, kwant_id: &str, context_id: &str) -> Result<Option<ApiSummary>, DbError> {
        Ok(sqlx::query_as(
            "SELECT summary, summary_through AS through_id FROM api_contexts \
             WHERE kwant_id = ? AND context_id = ? AND summary_through IS NOT NULL",
        )
        .bind(kwant_id)
        .bind(context_id)
        .fetch_optional(&self.pool)
        .await?)
    }

    /// Saves a new summary. The saved prefix is marked unused, so the turn after a summary writes
    /// a fresh one with today's notes instead of carrying a list of changes.
    pub async fn save_api_summary(&self, kwant_id: &str, context_id: &str, summary: &ApiSummary) -> Result<(), DbError> {
        sqlx::query(
            "INSERT INTO api_contexts (kwant_id, context_id, profile, team, notes, used_at, summary, summary_through) \
             VALUES (?, ?, '', '', '', ?, ?, ?) \
             ON CONFLICT (kwant_id, context_id) DO UPDATE SET \
                summary = excluded.summary, summary_through = excluded.summary_through, used_at = excluded.used_at",
        )
        .bind(kwant_id)
        .bind(context_id)
        .bind(DateTime::<Utc>::UNIX_EPOCH)
        .bind(&summary.summary)
        .bind(&summary.through_id)
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

        assert_eq!(db.api_summary("k", "c").await.unwrap(), None);
        let summary = ApiSummary { summary: "they chose sqlite".into(), through_id: "m9".into() };
        db.save_api_summary("k", "c", &summary).await.unwrap();
        assert_eq!(db.api_summary("k", "c").await.unwrap(), Some(summary));
        let after = db.api_context("k", "c").await.unwrap().unwrap();
        assert_eq!(after.notes, "n2", "the prefix stays");
        assert_eq!(after.used_at, DateTime::<Utc>::UNIX_EPOCH, "but is written afresh next turn");
        db.save_api_context(&row).await.unwrap();
        assert!(db.api_summary("k", "c").await.unwrap().is_some(), "a new prefix keeps the summary");
        db.save_api_summary("k", "new", &ApiSummary { summary: "s".into(), through_id: "m1".into() }).await.unwrap();
        assert!(db.api_summary("k", "new").await.unwrap().is_some());
    }
}
