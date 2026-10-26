//! One row per model API call an API-key bot makes: what it was for and the tokens it used, so
//! the person can see per bot what caching, summaries and tools save (see `crate::drivers::api`).

use super::{Database, DbError};
use chrono::{DateTime, Utc};
use serde::Serialize;
use sqlx::FromRow;

/// Why a call was made.
pub const PURPOSE_REPLY: &str = "reply";
/// A follow-up call that hands the model its tool results.
pub const PURPOSE_TOOL_ROUND: &str = "tool_round";
/// Summarising older messages (see `crate::drivers::api`).
pub const PURPOSE_SUMMARY: &str = "summary";

/// A call to record. Token fields are 0 when the provider sent no counts (`reported` false).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct NewUsage {
    pub kwant_id: String,
    pub context_id: String,
    pub purpose: &'static str,
    pub model: String,
    /// Input tokens not read from or written to the cache.
    pub fresh_input: i64,
    pub cache_read: i64,
    pub cache_write: i64,
    pub output: i64,
    pub reported: bool,
    /// Size of the request's tool schemas, system prompt and messages, in characters of JSON.
    pub tools_chars: i64,
    pub system_chars: i64,
    pub messages_chars: i64,
    /// What the provider said the call cost; `None` when it didn't say.
    pub cost_usd: Option<f64>,
}

#[derive(Debug, Clone, Serialize, FromRow)]
pub struct UsageRow {
    pub id: i64,
    pub context_id: String,
    pub purpose: String,
    pub model: String,
    pub fresh_input_tokens: i64,
    pub cache_read_tokens: i64,
    pub cache_write_tokens: i64,
    pub output_tokens: i64,
    pub reported: bool,
    pub tools_chars: i64,
    pub system_chars: i64,
    pub messages_chars: i64,
    pub cost_usd: Option<f64>,
    pub created_at: DateTime<Utc>,
}

/// Sums over a set of calls.
#[derive(Debug, Clone, Default, Serialize, FromRow, PartialEq)]
pub struct UsageTotals {
    pub calls: i64,
    pub fresh_input_tokens: i64,
    pub cache_read_tokens: i64,
    pub cache_write_tokens: i64,
    pub output_tokens: i64,
    pub tools_chars: i64,
    pub system_chars: i64,
    pub messages_chars: i64,
    /// Summed over the calls that reported a cost; `None` when none did.
    pub cost_usd: Option<f64>,
}

#[derive(Debug, Clone, Serialize, FromRow)]
pub struct PurposeTotals {
    pub purpose: String,
    #[sqlx(flatten)]
    #[serde(flatten)]
    pub totals: UsageTotals,
}

const SUMS: &str = "COUNT(*) AS calls, COALESCE(SUM(fresh_input), 0) AS fresh_input_tokens, \
    COALESCE(SUM(cache_read), 0) AS cache_read_tokens, COALESCE(SUM(cache_write), 0) AS cache_write_tokens, \
    COALESCE(SUM(output), 0) AS output_tokens, COALESCE(SUM(tools_chars), 0) AS tools_chars, \
    COALESCE(SUM(system_chars), 0) AS system_chars, COALESCE(SUM(messages_chars), 0) AS messages_chars, \
    SUM(cost_usd) AS cost_usd";

/// One bot's calls on one day (UTC), for Settings → Usage.
#[derive(Debug, Clone, Serialize, FromRow, PartialEq)]
pub struct DailyUsage {
    /// `YYYY-MM-DD`, UTC.
    pub day: String,
    pub kwant_id: String,
    /// None once the bot is deleted.
    pub kwant_name: Option<String>,
    #[sqlx(flatten)]
    #[serde(flatten)]
    pub totals: UsageTotals,
}

impl Database {
    pub(super) async fn migrate_usage(&self) -> Result<(), DbError> {
        for stmt in [
            "CREATE TABLE IF NOT EXISTS model_usage (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                kwant_id TEXT NOT NULL,
                context_id TEXT NOT NULL,
                purpose TEXT NOT NULL,
                model TEXT NOT NULL,
                fresh_input INTEGER NOT NULL DEFAULT 0,
                cache_read INTEGER NOT NULL DEFAULT 0,
                cache_write INTEGER NOT NULL DEFAULT 0,
                output INTEGER NOT NULL DEFAULT 0,
                reported INTEGER NOT NULL DEFAULT 0,
                tools_chars INTEGER NOT NULL DEFAULT 0,
                system_chars INTEGER NOT NULL DEFAULT 0,
                messages_chars INTEGER NOT NULL DEFAULT 0,
                created_at TIMESTAMP NOT NULL
            )",
            "CREATE INDEX IF NOT EXISTS model_usage_kwant ON model_usage (kwant_id, created_at)",
        ] {
            sqlx::query(stmt).execute(&self.pool).await?;
        }
        // Added later; fails harmlessly when the column is already there
        let _ = sqlx::query("ALTER TABLE model_usage ADD COLUMN cost_usd REAL").execute(&self.pool).await;
        Ok(())
    }

    pub async fn record_usage(&self, u: &NewUsage) -> Result<(), DbError> {
        sqlx::query(
            "INSERT INTO model_usage (kwant_id, context_id, purpose, model, fresh_input, cache_read, cache_write, output, \
             reported, tools_chars, system_chars, messages_chars, cost_usd, created_at) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(&u.kwant_id)
        .bind(&u.context_id)
        .bind(u.purpose)
        .bind(&u.model)
        .bind(u.fresh_input)
        .bind(u.cache_read)
        .bind(u.cache_write)
        .bind(u.output)
        .bind(u.reported)
        .bind(u.tools_chars)
        .bind(u.system_chars)
        .bind(u.messages_chars)
        .bind(u.cost_usd)
        .bind(Utc::now())
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// A bot's newest calls first, at most `limit`.
    pub async fn recent_usage(&self, kwant_id: &str, limit: i64) -> Result<Vec<UsageRow>, DbError> {
        Ok(sqlx::query_as(
            "SELECT id, context_id, purpose, model, fresh_input AS fresh_input_tokens, cache_read AS cache_read_tokens, \
             cache_write AS cache_write_tokens, output AS output_tokens, reported, tools_chars, system_chars, messages_chars, cost_usd, created_at \
             FROM model_usage WHERE kwant_id = ? ORDER BY id DESC LIMIT ?",
        )
        .bind(kwant_id)
        .bind(limit)
        .fetch_all(&self.pool)
        .await?)
    }

    /// A bot's totals since `since` (all time when `None`), overall and per purpose.
    pub async fn usage_totals(
        &self,
        kwant_id: &str,
        since: Option<DateTime<Utc>>,
    ) -> Result<(UsageTotals, Vec<PurposeTotals>), DbError> {
        let since = since.unwrap_or(DateTime::<Utc>::MIN_UTC);
        let all = sqlx::query_as(&format!("SELECT {SUMS} FROM model_usage WHERE kwant_id = ? AND created_at >= ?"))
            .bind(kwant_id)
            .bind(since)
            .fetch_one(&self.pool)
            .await?;
        let by_purpose = sqlx::query_as(&format!(
            "SELECT purpose, {SUMS} FROM model_usage WHERE kwant_id = ? AND created_at >= ? GROUP BY purpose ORDER BY purpose"
        ))
        .bind(kwant_id)
        .bind(since)
        .fetch_all(&self.pool)
        .await?;
        Ok((all, by_purpose))
    }

    /// Every bot's totals per UTC day since `since`, newest day first.
    pub async fn daily_usage(&self, since: DateTime<Utc>) -> Result<Vec<DailyUsage>, DbError> {
        Ok(sqlx::query_as(&format!(
            "SELECT substr(created_at, 1, 10) AS day, kwant_id, \
             (SELECT name FROM kwants WHERE kwants.id = model_usage.kwant_id) AS kwant_name, {SUMS} \
             FROM model_usage WHERE created_at >= ? GROUP BY day, kwant_id ORDER BY day DESC, kwant_id"
        ))
        .bind(since)
        .fetch_all(&self.pool)
        .await?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn usage_is_summed_per_bot_and_purpose() {
        let tmp = tempfile::tempdir().unwrap();
        let db = Database::init(tmp.path()).await.unwrap();
        let call = |kwant: &str, purpose, fresh, read| NewUsage {
            kwant_id: kwant.into(),
            context_id: "c".into(),
            purpose,
            model: "claude-opus-5-5".into(),
            fresh_input: fresh,
            cache_read: read,
            cache_write: 10,
            output: 5,
            reported: true,
            tools_chars: 100,
            system_chars: 200,
            messages_chars: 300,
            cost_usd: None,
        };
        db.record_usage(&call("k1", PURPOSE_REPLY, 50, 1000)).await.unwrap();
        db.record_usage(&call("k1", PURPOSE_TOOL_ROUND, 20, 1200)).await.unwrap();
        db.record_usage(&call("k1", PURPOSE_REPLY, 30, 0)).await.unwrap();
        db.record_usage(&call("k2", PURPOSE_REPLY, 999, 999)).await.unwrap();

        let (all, by) = db.usage_totals("k1", None).await.unwrap();
        assert_eq!((all.calls, all.fresh_input_tokens, all.cache_read_tokens, all.cache_write_tokens, all.output_tokens), (3, 100, 2200, 30, 15));
        assert_eq!(all.tools_chars, 300);
        let reply = by.iter().find(|p| p.purpose == PURPOSE_REPLY).unwrap();
        assert_eq!((reply.totals.calls, reply.totals.fresh_input_tokens), (2, 80));
        assert_eq!(by.len(), 2);

        let recent = db.recent_usage("k1", 2).await.unwrap();
        assert_eq!(recent.len(), 2);
        assert_eq!((recent[0].fresh_input_tokens, recent[1].purpose.as_str()), (30, PURPOSE_TOOL_ROUND), "newest first");

        let later = Utc::now() + chrono::Duration::minutes(1);
        assert_eq!(db.usage_totals("k1", Some(later)).await.unwrap().0, UsageTotals::default());
        assert_eq!(db.usage_totals("nobody", None).await.unwrap().0.calls, 0);
    }
}
