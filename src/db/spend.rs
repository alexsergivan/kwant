//! The daily spending stop's books (see `crate::server::spend_stop`): one row per local day with
//! the fresh tokens and dollars that day's turns used and whether the person pressed "Continue
//! today", plus the notices already sent that day so each one goes out once.

use super::{Database, DbError};
use chrono::{DateTime, Utc};
use sqlx::FromRow;

/// One day's spending. `day` is the local date (`YYYY-MM-DD`) in the person's time zone.
#[derive(Debug, Clone, Default, PartialEq, FromRow)]
pub struct SpendDay {
    pub day: String,
    pub tokens: i64,
    pub usd: f64,
    /// When the person lifted the stop for this day; cleared when a limit changes.
    pub continued_at: Option<DateTime<Utc>>,
}

/// Notices older than this many days are dropped (they only dedupe within one day).
const KEEP_NOTICE_DAYS: i64 = 7;

impl Database {
    pub(super) async fn migrate_spend(&self) -> Result<(), DbError> {
        for stmt in [
            "CREATE TABLE IF NOT EXISTS daily_spend (
                day TEXT PRIMARY KEY,
                tokens INTEGER NOT NULL DEFAULT 0,
                usd REAL NOT NULL DEFAULT 0,
                continued_at TIMESTAMP,
                updated_at TIMESTAMP NOT NULL
            )",
            "CREATE TABLE IF NOT EXISTS spend_stop_notices (
                day TEXT NOT NULL,
                key TEXT NOT NULL,
                created_at TIMESTAMP NOT NULL,
                PRIMARY KEY (day, key)
            )",
        ] {
            sqlx::query(stmt).execute(&self.pool).await?;
        }
        Ok(())
    }

    /// The day's row; zeros when nothing was spent yet.
    pub async fn spend_day(&self, day: &str) -> Result<SpendDay, DbError> {
        let row: Option<SpendDay> = sqlx::query_as("SELECT day, tokens, usd, continued_at FROM daily_spend WHERE day = ?")
            .bind(day)
            .fetch_optional(&self.pool)
            .await?;
        Ok(row.unwrap_or_else(|| SpendDay { day: day.to_string(), ..Default::default() }))
    }

    /// Adds a finished turn's fresh tokens and dollars to the day.
    pub async fn add_spend(&self, day: &str, tokens: u64, usd: f64) -> Result<(), DbError> {
        let tokens = i64::try_from(tokens).unwrap_or(i64::MAX);
        let usd = if usd.is_finite() && usd > 0.0 { usd } else { 0.0 };
        sqlx::query(
            "INSERT INTO daily_spend (day, tokens, usd, updated_at) VALUES (?, ?, ?, ?) \
             ON CONFLICT(day) DO UPDATE SET tokens = MIN(tokens + excluded.tokens, 9223372036854775807), \
             usd = usd + excluded.usd, updated_at = excluded.updated_at",
        )
        .bind(day)
        .bind(tokens)
        .bind(usd)
        .bind(Utc::now())
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// Lifts (`Some`) or restores (`None`) the stop for the day.
    pub async fn set_spend_continued(&self, day: &str, at: Option<DateTime<Utc>>) -> Result<(), DbError> {
        sqlx::query(
            "INSERT INTO daily_spend (day, continued_at, updated_at) VALUES (?, ?, ?) \
             ON CONFLICT(day) DO UPDATE SET continued_at = excluded.continued_at, updated_at = excluded.updated_at",
        )
        .bind(day)
        .bind(at)
        .bind(Utc::now())
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// Notes that the notice `key` went out on `day`; true only the first time that day.
    pub async fn first_spend_notice(&self, day: &str, key: &str) -> Result<bool, DbError> {
        let now = Utc::now();
        let done = sqlx::query("INSERT OR IGNORE INTO spend_stop_notices (day, key, created_at) VALUES (?, ?, ?)")
            .bind(day)
            .bind(key)
            .bind(now)
            .execute(&self.pool)
            .await?;
        let _ = sqlx::query("DELETE FROM spend_stop_notices WHERE created_at < ?")
            .bind(now - chrono::Duration::days(KEEP_NOTICE_DAYS))
            .execute(&self.pool)
            .await;
        Ok(done.rows_affected() == 1)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn adds_up_and_dedupes() {
        let tmp = tempfile::tempdir().unwrap();
        let db = Database::init(tmp.path()).await.unwrap();
        assert_eq!(db.spend_day("2026-11-06").await.unwrap(), SpendDay { day: "2026-11-06".into(), ..Default::default() });
        db.add_spend("2026-11-06", 100, 0.5).await.unwrap();
        db.add_spend("2026-11-06", 50, f64::NAN).await.unwrap();
        db.add_spend("2026-11-07", 7, 0.0).await.unwrap();
        let d = db.spend_day("2026-11-06").await.unwrap();
        assert_eq!((d.tokens, d.usd, d.continued_at), (150, 0.5, None));
        db.set_spend_continued("2026-11-06", Some(Utc::now())).await.unwrap();
        assert!(db.spend_day("2026-11-06").await.unwrap().continued_at.is_some());
        assert_eq!(db.spend_day("2026-11-06").await.unwrap().tokens, 150);
        assert!(db.first_spend_notice("2026-11-06", "c1").await.unwrap());
        assert!(!db.first_spend_notice("2026-11-06", "c1").await.unwrap());
        assert!(db.first_spend_notice("2026-11-07", "c1").await.unwrap());
    }
}
