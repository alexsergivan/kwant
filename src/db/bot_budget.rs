//! G2's books (see `crate::server::bot_budget`): each bot's fresh tokens and dollars per local
//! day, booked from the same call and the same numbers as the daily spending stop
//! (`spend_stop::book_turn`), each bot's monthly budget (person-only, never in `settings`), the
//! notices already sent, and the routines held while a bot was paused.

use super::{Database, DbError};
use chrono::{DateTime, Utc};
use serde::Serialize;
use sqlx::FromRow;

/// A bot's use over some days (a month, or one day).
#[derive(Debug, Clone, Default, PartialEq, FromRow, Serialize)]
pub struct BotUse {
    pub tokens: i64,
    pub usd: f64,
    pub reported_turns: i64,
    pub unreported_turns: i64,
    pub cost_turns: i64,
}

/// A bot's budget row; all zero / null when the person never set one.
#[derive(Debug, Clone, Default, PartialEq, FromRow)]
pub struct BotBudgetRow {
    pub kwant_id: String,
    pub budget_tokens: i64,
    pub budget_usd: f64,
    /// `YYYY-MM` the person pressed Continue for.
    pub continued_month: Option<String>,
    /// `YYYY-MM` of the current pause episode (set once per episode).
    pub paused_month: Option<String>,
    pub paused_at: Option<DateTime<Utc>>,
    /// The budget key (`month|tokens|usd`) the 80% note was sent (or skipped) for.
    pub warned_key: Option<String>,
}

/// Day rows older than this are pruned.
const KEEP_DAYS: i64 = 400;
/// Notices older than this are pruned (they dedupe within one month).
const KEEP_NOTICE_DAYS: i64 = 62;

impl Database {
    pub(super) async fn migrate_bot_budget(&self) -> Result<(), DbError> {
        let existed: Option<(String,)> =
            sqlx::query_as("SELECT name FROM sqlite_master WHERE type = 'table' AND name = 'bot_spend_days'")
                .fetch_optional(&self.pool)
                .await?;
        for stmt in [
            "CREATE TABLE IF NOT EXISTS bot_spend_days (
                day TEXT NOT NULL,
                kwant_id TEXT NOT NULL,
                tokens INTEGER NOT NULL DEFAULT 0,
                usd REAL NOT NULL DEFAULT 0,
                reported_turns INTEGER NOT NULL DEFAULT 0,
                unreported_turns INTEGER NOT NULL DEFAULT 0,
                cost_turns INTEGER NOT NULL DEFAULT 0,
                updated_at TIMESTAMP NOT NULL,
                PRIMARY KEY (day, kwant_id)
            )",
            "CREATE INDEX IF NOT EXISTS bot_spend_days_kwant ON bot_spend_days (kwant_id, day)",
            "CREATE TABLE IF NOT EXISTS bot_budgets (
                kwant_id TEXT PRIMARY KEY,
                budget_tokens INTEGER NOT NULL DEFAULT 0,
                budget_usd REAL NOT NULL DEFAULT 0,
                continued_month TEXT,
                paused_month TEXT,
                paused_at TIMESTAMP,
                warned_key TEXT,
                updated_at TIMESTAMP NOT NULL
            )",
            "CREATE TABLE IF NOT EXISTS bot_budget_notices (
                month TEXT NOT NULL,
                key TEXT NOT NULL,
                created_at TIMESTAMP NOT NULL,
                PRIMARY KEY (month, key)
            )",
            "CREATE TABLE IF NOT EXISTS budget_held_routines (
                routine_id TEXT PRIMARY KEY,
                kwant_id TEXT NOT NULL,
                first_due_at TIMESTAMP NOT NULL
            )",
        ] {
            sqlx::query(stmt).execute(&self.pool).await?;
        }
        // Added with G2; fail harmlessly when already there
        let _ = sqlx::query("ALTER TABLE parked_turns ADD COLUMN reason TEXT NOT NULL DEFAULT 'usage_limit'").execute(&self.pool).await;
        let _ = sqlx::query("ALTER TABLE routine_runs ADD COLUMN note TEXT").execute(&self.pool).await;
        if existed.is_none() {
            // Once, on upgrade: the last 31 days of engine-reported calls, by UTC date, so the
            // first month isn't 0 (approximate: model_usage misses Codex and cancelled turns)
            let since = Utc::now() - chrono::Duration::days(31);
            let _ = sqlx::query(
                "INSERT OR IGNORE INTO bot_spend_days (day, kwant_id, tokens, usd, reported_turns, unreported_turns, cost_turns, updated_at) \
                 SELECT strftime('%Y-%m-%d', created_at), kwant_id, \
                        SUM(fresh_input + cache_write + output), SUM(COALESCE(cost_usd, 0)), \
                        SUM(CASE WHEN reported != 0 THEN 1 ELSE 0 END), SUM(CASE WHEN reported = 0 THEN 1 ELSE 0 END), \
                        SUM(CASE WHEN COALESCE(cost_usd, 0) > 0 THEN 1 ELSE 0 END), ? \
                 FROM model_usage WHERE created_at >= ? GROUP BY 1, 2",
            )
            .bind(Utc::now())
            .bind(since)
            .execute(&self.pool)
            .await;
        }
        Ok(())
    }

    /// Adds one booked turn to the bot's day.
    pub async fn add_bot_spend(&self, day: &str, kwant_id: &str, tokens: u64, usd: f64, reported: bool) -> Result<(), DbError> {
        let tokens = i64::try_from(tokens).unwrap_or(i64::MAX);
        let usd = if usd.is_finite() && usd > 0.0 { usd } else { 0.0 };
        sqlx::query(
            "INSERT INTO bot_spend_days (day, kwant_id, tokens, usd, reported_turns, unreported_turns, cost_turns, updated_at) \
             VALUES (?, ?, ?, ?, ?, ?, ?, ?) \
             ON CONFLICT(day, kwant_id) DO UPDATE SET tokens = MIN(tokens + excluded.tokens, 9223372036854775807), \
             usd = usd + excluded.usd, reported_turns = reported_turns + excluded.reported_turns, \
             unreported_turns = unreported_turns + excluded.unreported_turns, cost_turns = cost_turns + excluded.cost_turns, \
             updated_at = excluded.updated_at",
        )
        .bind(day)
        .bind(kwant_id)
        .bind(tokens)
        .bind(usd)
        .bind(i64::from(reported))
        .bind(i64::from(!reported))
        .bind(i64::from(usd > 0.0))
        .bind(Utc::now())
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// Drops day rows older than [`KEEP_DAYS`].
    pub async fn prune_bot_spend(&self, today: chrono::NaiveDate) -> Result<(), DbError> {
        let cutoff = (today - chrono::Duration::days(KEEP_DAYS)).format("%Y-%m-%d").to_string();
        sqlx::query("DELETE FROM bot_spend_days WHERE day < ?").bind(cutoff).execute(&self.pool).await?;
        Ok(())
    }

    /// The bot's use on the days starting with `prefix` (`YYYY-MM-` for a month, a full date for a day).
    pub async fn bot_use(&self, kwant_id: &str, prefix: &str) -> Result<BotUse, DbError> {
        let row: BotUse = sqlx::query_as(
            "SELECT COALESCE(SUM(tokens), 0) AS tokens, COALESCE(SUM(usd), 0.0) AS usd, \
             COALESCE(SUM(reported_turns), 0) AS reported_turns, COALESCE(SUM(unreported_turns), 0) AS unreported_turns, \
             COALESCE(SUM(cost_turns), 0) AS cost_turns FROM bot_spend_days WHERE kwant_id = ? AND day LIKE ? || '%'",
        )
        .bind(kwant_id)
        .bind(prefix)
        .fetch_one(&self.pool)
        .await?;
        Ok(row)
    }

    /// Sum of every bot's tokens on `day` (the invariant with `daily_spend`).
    pub async fn bot_spend_day_total(&self, day: &str) -> Result<i64, DbError> {
        let (n,): (i64,) = sqlx::query_as("SELECT COALESCE(SUM(tokens), 0) FROM bot_spend_days WHERE day = ?")
            .bind(day)
            .fetch_one(&self.pool)
            .await?;
        Ok(n)
    }

    pub async fn bot_budget(&self, kwant_id: &str) -> Result<BotBudgetRow, DbError> {
        let row: Option<BotBudgetRow> = sqlx::query_as(
            "SELECT kwant_id, budget_tokens, budget_usd, continued_month, paused_month, paused_at, warned_key FROM bot_budgets WHERE kwant_id = ?",
        )
        .bind(kwant_id)
        .fetch_optional(&self.pool)
        .await?;
        Ok(row.unwrap_or_else(|| BotBudgetRow { kwant_id: kwant_id.into(), ..Default::default() }))
    }

    /// The bots with a budget set (either limit).
    pub async fn bots_with_budget(&self) -> Result<Vec<BotBudgetRow>, DbError> {
        Ok(sqlx::query_as(
            "SELECT kwant_id, budget_tokens, budget_usd, continued_month, paused_month, paused_at, warned_key FROM bot_budgets \
             WHERE budget_tokens > 0 OR budget_usd > 0",
        )
        .fetch_all(&self.pool)
        .await?)
    }

    /// Sets the limits; a change of either clears Continue (the new budget applies at once).
    pub async fn set_bot_budget(&self, kwant_id: &str, tokens: i64, usd: f64) -> Result<(), DbError> {
        sqlx::query(
            "INSERT INTO bot_budgets (kwant_id, budget_tokens, budget_usd, updated_at) VALUES (?, ?, ?, ?) \
             ON CONFLICT(kwant_id) DO UPDATE SET \
               continued_month = CASE WHEN budget_tokens = excluded.budget_tokens AND budget_usd = excluded.budget_usd THEN continued_month ELSE NULL END, \
               budget_tokens = excluded.budget_tokens, budget_usd = excluded.budget_usd, updated_at = excluded.updated_at",
        )
        .bind(kwant_id)
        .bind(tokens)
        .bind(usd)
        .bind(Utc::now())
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn set_bot_budget_continued(&self, kwant_id: &str, month: Option<&str>) -> Result<(), DbError> {
        sqlx::query(
            "INSERT INTO bot_budgets (kwant_id, continued_month, updated_at) VALUES (?, ?, ?) \
             ON CONFLICT(kwant_id) DO UPDATE SET continued_month = excluded.continued_month, updated_at = excluded.updated_at",
        )
        .bind(kwant_id)
        .bind(month)
        .bind(Utc::now())
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// Starts a pause episode for `month`; true only for the call that started it.
    pub async fn start_bot_pause(&self, kwant_id: &str, month: &str, at: DateTime<Utc>) -> Result<bool, DbError> {
        let res = sqlx::query(
            "UPDATE bot_budgets SET paused_month = ?, paused_at = ? WHERE kwant_id = ? AND paused_month IS NOT ?",
        )
        .bind(month)
        .bind(at)
        .bind(kwant_id)
        .bind(month)
        .execute(&self.pool)
        .await?;
        Ok(res.rows_affected() == 1)
    }

    /// Ends the pause episode; true when there was one.
    pub async fn end_bot_pause(&self, kwant_id: &str) -> Result<bool, DbError> {
        let res = sqlx::query("UPDATE bot_budgets SET paused_month = NULL, paused_at = NULL WHERE kwant_id = ? AND paused_month IS NOT NULL")
            .bind(kwant_id)
            .execute(&self.pool)
            .await?;
        Ok(res.rows_affected() == 1)
    }

    /// Marks the 80% note for `key` as done; true only the first time.
    pub async fn mark_bot_warned(&self, kwant_id: &str, key: &str) -> Result<bool, DbError> {
        let res = sqlx::query("UPDATE bot_budgets SET warned_key = ? WHERE kwant_id = ? AND warned_key IS NOT ?")
            .bind(key)
            .bind(kwant_id)
            .bind(key)
            .execute(&self.pool)
            .await?;
        Ok(res.rows_affected() == 1)
    }

    /// Notes that the notice `key` went out in `month`; true only the first time.
    pub async fn first_bot_budget_notice(&self, month: &str, key: &str) -> Result<bool, DbError> {
        let now = Utc::now();
        let done = sqlx::query("INSERT OR IGNORE INTO bot_budget_notices (month, key, created_at) VALUES (?, ?, ?)")
            .bind(month)
            .bind(key)
            .bind(now)
            .execute(&self.pool)
            .await?;
        let _ = sqlx::query("DELETE FROM bot_budget_notices WHERE created_at < ?")
            .bind(now - chrono::Duration::days(KEEP_NOTICE_DAYS))
            .execute(&self.pool)
            .await;
        Ok(done.rows_affected() == 1)
    }

    /// Holds a routine whose run came due while its bot was paused; true the first time.
    pub async fn hold_budget_routine(&self, routine_id: &str, kwant_id: &str) -> Result<bool, DbError> {
        let res = sqlx::query("INSERT OR IGNORE INTO budget_held_routines (routine_id, kwant_id, first_due_at) VALUES (?, ?, ?)")
            .bind(routine_id)
            .bind(kwant_id)
            .bind(Utc::now())
            .execute(&self.pool)
            .await?;
        Ok(res.rows_affected() == 1)
    }

    /// Takes (and forgets) the routines held for `kwant_id`, oldest first.
    pub async fn take_budget_routines(&self, kwant_id: &str) -> Result<Vec<String>, DbError> {
        let mut rows: Vec<(String, DateTime<Utc>)> =
            sqlx::query_as("DELETE FROM budget_held_routines WHERE kwant_id = ? RETURNING routine_id, first_due_at")
                .bind(kwant_id)
                .fetch_all(&self.pool)
                .await?;
        rows.sort_by_key(|(_, at)| *at);
        Ok(rows.into_iter().map(|(id, _)| id).collect())
    }

    /// Makes the bot's budget-held parked turns due now; returns how many.
    pub async fn release_budget_parks(&self, kwant_id: &str, now: DateTime<Utc>) -> Result<u64, DbError> {
        let res = sqlx::query("UPDATE parked_turns SET retry_at = ? WHERE kwant_id = ? AND status = 'pending' AND reason = 'bot_budget'")
            .bind(now)
            .bind(kwant_id)
            .execute(&self.pool)
            .await?;
        Ok(res.rows_affected())
    }

    /// Bots that may have something to release: a pause episode, held routines or held turns.
    pub async fn paused_or_holding_bots(&self) -> Result<Vec<String>, DbError> {
        let rows: Vec<(String,)> = sqlx::query_as(
            "SELECT kwant_id FROM bot_budgets WHERE paused_month IS NOT NULL \
             UNION SELECT kwant_id FROM budget_held_routines \
             UNION SELECT kwant_id FROM parked_turns WHERE status = 'pending' AND reason = 'bot_budget'",
        )
        .fetch_all(&self.pool)
        .await?;
        Ok(rows.into_iter().map(|(id,)| id).collect())
    }

    /// Writes a routine run's note ("Ran late: …").
    pub async fn set_routine_run_note(&self, run_id: &str, note: &str) -> Result<(), DbError> {
        sqlx::query("UPDATE routine_runs SET note = ? WHERE id = ?").bind(note).bind(run_id).execute(&self.pool).await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn books_by_bot_and_month() {
        let tmp = tempfile::tempdir().unwrap();
        let db = Database::init(tmp.path()).await.unwrap();
        db.add_bot_spend("2026-11-06", "a", 100, 0.5, true).await.unwrap();
        db.add_bot_spend("2026-11-07", "a", 50, 0.0, false).await.unwrap();
        db.add_bot_spend("2026-10-31", "a", 7, 0.0, true).await.unwrap();
        db.add_bot_spend("2026-11-06", "b", 9, f64::NAN, true).await.unwrap();
        let m = db.bot_use("a", "2026-11-").await.unwrap();
        assert_eq!(m, BotUse { tokens: 150, usd: 0.5, reported_turns: 1, unreported_turns: 1, cost_turns: 1 });
        assert_eq!(db.bot_use("a", "2026-11-07").await.unwrap().tokens, 50);
        assert_eq!(db.bot_spend_day_total("2026-11-06").await.unwrap(), 109);
        assert_eq!(db.bot_use("nobody", "2026-11-").await.unwrap(), BotUse::default());
    }

    #[tokio::test]
    async fn pause_episode_and_warn_once() {
        let tmp = tempfile::tempdir().unwrap();
        let db = Database::init(tmp.path()).await.unwrap();
        db.set_bot_budget("a", 100, 0.0).await.unwrap();
        assert!(db.start_bot_pause("a", "2026-11", Utc::now()).await.unwrap());
        assert!(!db.start_bot_pause("a", "2026-11", Utc::now()).await.unwrap());
        assert!(db.end_bot_pause("a").await.unwrap());
        assert!(!db.end_bot_pause("a").await.unwrap());
        assert!(db.mark_bot_warned("a", "2026-11|100|0").await.unwrap());
        assert!(!db.mark_bot_warned("a", "2026-11|100|0").await.unwrap());
        assert!(db.mark_bot_warned("a", "2026-11|200|0").await.unwrap());
        db.set_bot_budget_continued("a", Some("2026-11")).await.unwrap();
        db.set_bot_budget("a", 100, 0.0).await.unwrap();
        assert_eq!(db.bot_budget("a").await.unwrap().continued_month.as_deref(), Some("2026-11"));
        db.set_bot_budget("a", 300, 0.0).await.unwrap();
        assert_eq!(db.bot_budget("a").await.unwrap().continued_month, None);
        assert!(db.hold_budget_routine("r1", "a").await.unwrap());
        assert!(!db.hold_budget_routine("r1", "a").await.unwrap());
        assert_eq!(db.take_budget_routines("a").await.unwrap(), vec!["r1".to_string()]);
        assert!(db.take_budget_routines("a").await.unwrap().is_empty());
    }
}
