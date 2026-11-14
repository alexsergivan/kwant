//! Customer chats on Telegram (roadmap 10b): a business's own customers write to one of its bots
//! through a second Telegram bot, separate from the person's own paired bot
//! (`crate::db::telegram`). One link per space, off by default ("off" is "no row"). The bot token
//! is never here: it is in the vault (`crate::server::telegram::customers::VAULT_OWNER`). All
//! tables are add-only.

use super::{Database, DbError};
use chrono::{DateTime, Duration, Utc};
use serde::Serialize;
use sqlx::FromRow;

/// The token is saved and the link listens (when a bot answers).
pub const ON: &str = "on";
/// Telegram said the saved token no longer works (401).
pub const REJECTED: &str = "rejected";

/// At most this many customer chats in a space; a new customer past it gets no reply.
pub const MAX_CHATS: i64 = 200;
/// At most this many messages from one customer chat in an hour.
pub const PER_HOUR: i64 = 20;

#[derive(Debug, Clone, Serialize, FromRow, PartialEq)]
pub struct CustomerLink {
    pub space_id: String,
    pub state: String,
    pub bot_id: Option<i64>,
    pub bot_username: Option<String>,
    pub bot_name: Option<String>,
    /// The bot that answers customers; `None` is off.
    pub kwant_id: Option<String>,
    pub update_offset: i64,
    pub last_error: Option<String>,
    /// Messages refused by the hourly limit, and new customers refused by the chat cap.
    pub limited: i64,
    pub refused_new: i64,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, FromRow, PartialEq)]
pub struct CustomerChat {
    pub space_id: String,
    pub chat_id: i64,
    pub thread_id: String,
    pub kwant_id: String,
    pub first_name: String,
    /// The customer sent /stop (or blocked the bot): no more replies.
    pub stopped_at: Option<DateTime<Utc>>,
    /// The bot handed this chat to the person: why, and when (cleared when the person replies).
    pub handoff_reason: Option<String>,
    pub handoff_at: Option<DateTime<Utc>>,
    /// When this chat last got the "too many messages" line (once an hour at most).
    pub limited_at: Option<DateTime<Utc>>,
    pub last_message_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
}

/// The counts the status view shows.
#[derive(Debug, Clone, Default, Serialize, PartialEq)]
pub struct CustomerCounts {
    pub chats: i64,
    pub active: i64,
    pub stopped: i64,
    pub waiting_on_person: i64,
}

impl Database {
    pub(super) async fn migrate_telegram_customers(&self) -> Result<(), DbError> {
        for stmt in [
            "CREATE TABLE IF NOT EXISTS telegram_customer_links (
                space_id TEXT PRIMARY KEY,
                state TEXT NOT NULL,
                bot_id INTEGER,
                bot_username TEXT,
                bot_name TEXT,
                kwant_id TEXT,
                update_offset INTEGER NOT NULL DEFAULT 0,
                last_error TEXT,
                limited INTEGER NOT NULL DEFAULT 0,
                refused_new INTEGER NOT NULL DEFAULT 0,
                created_at TIMESTAMP NOT NULL,
                updated_at TIMESTAMP NOT NULL
            )",
            "CREATE TABLE IF NOT EXISTS telegram_customer_chats (
                space_id TEXT NOT NULL,
                chat_id INTEGER NOT NULL,
                thread_id TEXT NOT NULL,
                kwant_id TEXT NOT NULL,
                first_name TEXT NOT NULL,
                stopped_at TIMESTAMP,
                handoff_reason TEXT,
                handoff_at TIMESTAMP,
                limited_at TIMESTAMP,
                last_message_at TIMESTAMP,
                created_at TIMESTAMP NOT NULL,
                PRIMARY KEY (space_id, chat_id)
            )",
            "CREATE INDEX IF NOT EXISTS telegram_customer_chats_by_thread ON telegram_customer_chats (thread_id)",
            // One row per message a customer chat sent, for the hourly limit (pruned as it goes)
            "CREATE TABLE IF NOT EXISTS telegram_customer_inbound (
                space_id TEXT NOT NULL,
                chat_id INTEGER NOT NULL,
                at TIMESTAMP NOT NULL
            )",
            "CREATE INDEX IF NOT EXISTS telegram_customer_inbound_by_chat ON telegram_customer_inbound (space_id, chat_id, at)",
            // Updates already handled, so a restart never answers one twice
            "CREATE TABLE IF NOT EXISTS telegram_customer_updates (
                space_id TEXT NOT NULL,
                update_id INTEGER NOT NULL,
                created_at TIMESTAMP NOT NULL,
                PRIMARY KEY (space_id, update_id)
            )",
        ] {
            sqlx::query(stmt).execute(&self.pool).await?;
        }
        Ok(())
    }

    pub async fn customer_link(&self, space: &str) -> Result<Option<CustomerLink>, DbError> {
        Ok(sqlx::query_as::<_, CustomerLink>("SELECT * FROM telegram_customer_links WHERE space_id = ?")
            .bind(space)
            .fetch_optional(&self.pool)
            .await?)
    }

    pub async fn customer_links(&self) -> Result<Vec<CustomerLink>, DbError> {
        Ok(sqlx::query_as::<_, CustomerLink>("SELECT * FROM telegram_customer_links").fetch_all(&self.pool).await?)
    }

    /// Saves the bot behind a new token. The same bot keeps its offset and answering bot; another
    /// bot starts from Telegram's first update, answered by `default_kwant` when none was chosen.
    pub async fn customer_save_bot(&self, space: &str, bot_id: i64, username: &str, name: &str, default_kwant: Option<&str>) -> Result<CustomerLink, DbError> {
        let now = crate::clock::now();
        let before = self.customer_link(space).await?;
        match before {
            Some(l) if l.bot_id == Some(bot_id) => {
                sqlx::query("UPDATE telegram_customer_links SET state = ?, bot_username = ?, bot_name = ?, last_error = NULL, updated_at = ? WHERE space_id = ?")
                    .bind(ON)
                    .bind(username)
                    .bind(name)
                    .bind(now)
                    .bind(space)
                    .execute(&self.pool)
                    .await?;
            }
            Some(l) => {
                let kwant = l.kwant_id.or(default_kwant.map(str::to_string));
                sqlx::query(
                    "UPDATE telegram_customer_links SET state = ?, bot_id = ?, bot_username = ?, bot_name = ?, kwant_id = ?, update_offset = 0, last_error = NULL, updated_at = ? WHERE space_id = ?",
                )
                .bind(ON)
                .bind(bot_id)
                .bind(username)
                .bind(name)
                .bind(kwant)
                .bind(now)
                .bind(space)
                .execute(&self.pool)
                .await?;
                sqlx::query("DELETE FROM telegram_customer_updates WHERE space_id = ?").bind(space).execute(&self.pool).await?;
            }
            None => {
                sqlx::query(
                    "INSERT INTO telegram_customer_links (space_id, state, bot_id, bot_username, bot_name, kwant_id, update_offset, created_at, updated_at)
                     VALUES (?, ?, ?, ?, ?, ?, 0, ?, ?)",
                )
                .bind(space)
                .bind(ON)
                .bind(bot_id)
                .bind(username)
                .bind(name)
                .bind(default_kwant)
                .bind(now)
                .bind(now)
                .execute(&self.pool)
                .await?;
            }
        }
        Ok(self.customer_link(space).await?.expect("just saved"))
    }

    pub async fn customer_set_kwant(&self, space: &str, kwant_id: Option<&str>) -> Result<bool, DbError> {
        let r = sqlx::query("UPDATE telegram_customer_links SET kwant_id = ?, updated_at = ? WHERE space_id = ?")
            .bind(kwant_id)
            .bind(crate::clock::now())
            .bind(space)
            .execute(&self.pool)
            .await?;
        Ok(r.rows_affected() == 1)
    }

    pub async fn customer_set_state(&self, space: &str, state: &str, error: Option<&str>) -> Result<(), DbError> {
        sqlx::query("UPDATE telegram_customer_links SET state = ?, last_error = ?, updated_at = ? WHERE space_id = ?")
            .bind(state)
            .bind(error)
            .bind(crate::clock::now())
            .bind(space)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    pub async fn customer_set_error(&self, space: &str, error: Option<&str>) -> Result<(), DbError> {
        sqlx::query("UPDATE telegram_customer_links SET last_error = ? WHERE space_id = ?")
            .bind(error)
            .bind(space)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    pub async fn customer_set_offset(&self, space: &str, offset: i64) -> Result<(), DbError> {
        sqlx::query("UPDATE telegram_customer_links SET update_offset = MAX(update_offset, ?) WHERE space_id = ?")
            .bind(offset)
            .bind(space)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    /// Ends the link: the row and the update marks go. Customer chats and their threads stay.
    pub async fn customer_delete_link(&self, space: &str) -> Result<bool, DbError> {
        let r = sqlx::query("DELETE FROM telegram_customer_links WHERE space_id = ?").bind(space).execute(&self.pool).await?;
        sqlx::query("DELETE FROM telegram_customer_updates WHERE space_id = ?").bind(space).execute(&self.pool).await?;
        Ok(r.rows_affected() == 1)
    }

    /// True the first time `update_id` is seen in this space.
    pub async fn customer_claim_update(&self, space: &str, update_id: i64) -> Result<bool, DbError> {
        let r = sqlx::query("INSERT OR IGNORE INTO telegram_customer_updates (space_id, update_id, created_at) VALUES (?, ?, ?)")
            .bind(space)
            .bind(update_id)
            .bind(crate::clock::now())
            .execute(&self.pool)
            .await?;
        sqlx::query("DELETE FROM telegram_customer_updates WHERE space_id = ? AND update_id < ?")
            .bind(space)
            .bind(update_id - 1000)
            .execute(&self.pool)
            .await?;
        Ok(r.rows_affected() == 1)
    }

    pub async fn customer_count_limited(&self, space: &str, new_chat: bool) -> Result<(), DbError> {
        let sql = if new_chat {
            "UPDATE telegram_customer_links SET refused_new = refused_new + 1 WHERE space_id = ?"
        } else {
            "UPDATE telegram_customer_links SET limited = limited + 1 WHERE space_id = ?"
        };
        sqlx::query(sql).bind(space).execute(&self.pool).await?;
        Ok(())
    }

    pub async fn customer_chat(&self, space: &str, chat_id: i64) -> Result<Option<CustomerChat>, DbError> {
        Ok(sqlx::query_as::<_, CustomerChat>("SELECT * FROM telegram_customer_chats WHERE space_id = ? AND chat_id = ?")
            .bind(space)
            .bind(chat_id)
            .fetch_optional(&self.pool)
            .await?)
    }

    pub async fn customer_chats(&self, space: &str) -> Result<Vec<CustomerChat>, DbError> {
        Ok(sqlx::query_as::<_, CustomerChat>("SELECT * FROM telegram_customer_chats WHERE space_id = ? ORDER BY created_at")
            .bind(space)
            .fetch_all(&self.pool)
            .await?)
    }

    pub async fn customer_chat_count(&self, space: &str) -> Result<i64, DbError> {
        let (n,): (i64,) = sqlx::query_as("SELECT COUNT(*) FROM telegram_customer_chats WHERE space_id = ?")
            .bind(space)
            .fetch_one(&self.pool)
            .await?;
        Ok(n)
    }

    /// Records the chat's thread with `kwant_id` (a new chat, or a new bot answering it).
    pub async fn customer_save_chat(&self, space: &str, chat_id: i64, thread_id: &str, kwant_id: &str, first_name: &str) -> Result<(), DbError> {
        sqlx::query(
            "INSERT INTO telegram_customer_chats (space_id, chat_id, thread_id, kwant_id, first_name, created_at)
             VALUES (?, ?, ?, ?, ?, ?)
             ON CONFLICT (space_id, chat_id) DO UPDATE SET thread_id = excluded.thread_id, kwant_id = excluded.kwant_id, first_name = excluded.first_name",
        )
        .bind(space)
        .bind(chat_id)
        .bind(thread_id)
        .bind(kwant_id)
        .bind(first_name)
        .bind(crate::clock::now())
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// `/stop` (or the customer blocked the bot): `true` stops, `false` (a later `/start`) opens it again.
    pub async fn customer_set_stopped(&self, space: &str, chat_id: i64, stopped: bool) -> Result<bool, DbError> {
        let at = stopped.then(crate::clock::now);
        let r = sqlx::query("UPDATE telegram_customer_chats SET stopped_at = ? WHERE space_id = ? AND chat_id = ?")
            .bind(at)
            .bind(space)
            .bind(chat_id)
            .execute(&self.pool)
            .await?;
        Ok(r.rows_affected() == 1)
    }

    pub async fn customer_set_handoff(&self, space: &str, chat_id: i64, reason: Option<&str>) -> Result<(), DbError> {
        let at = reason.map(|_| crate::clock::now());
        sqlx::query("UPDATE telegram_customer_chats SET handoff_reason = ?, handoff_at = ? WHERE space_id = ? AND chat_id = ?")
            .bind(reason)
            .bind(at)
            .bind(space)
            .bind(chat_id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    pub async fn customer_set_limited_at(&self, space: &str, chat_id: i64) -> Result<(), DbError> {
        sqlx::query("UPDATE telegram_customer_chats SET limited_at = ? WHERE space_id = ? AND chat_id = ?")
            .bind(crate::clock::now())
            .bind(space)
            .bind(chat_id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    /// How many messages this chat sent in the last hour (older rows are dropped on the way).
    pub async fn customer_inbound_last_hour(&self, space: &str, chat_id: i64) -> Result<i64, DbError> {
        let since = crate::clock::now() - Duration::hours(1);
        sqlx::query("DELETE FROM telegram_customer_inbound WHERE space_id = ? AND chat_id = ? AND at < ?")
            .bind(space)
            .bind(chat_id)
            .bind(since)
            .execute(&self.pool)
            .await?;
        let (n,): (i64,) = sqlx::query_as("SELECT COUNT(*) FROM telegram_customer_inbound WHERE space_id = ? AND chat_id = ? AND at >= ?")
            .bind(space)
            .bind(chat_id)
            .bind(since)
            .fetch_one(&self.pool)
            .await?;
        Ok(n)
    }

    pub async fn customer_note_inbound(&self, space: &str, chat_id: i64) -> Result<(), DbError> {
        let now = crate::clock::now();
        sqlx::query("INSERT INTO telegram_customer_inbound (space_id, chat_id, at) VALUES (?, ?, ?)")
            .bind(space)
            .bind(chat_id)
            .bind(now)
            .execute(&self.pool)
            .await?;
        sqlx::query("UPDATE telegram_customer_chats SET last_message_at = ? WHERE space_id = ? AND chat_id = ?")
            .bind(now)
            .bind(space)
            .bind(chat_id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    pub async fn customer_counts(&self, space: &str) -> Result<CustomerCounts, DbError> {
        let (chats, stopped, waiting): (i64, i64, i64) = sqlx::query_as(
            "SELECT COUNT(*),
                    COALESCE(SUM(CASE WHEN stopped_at IS NOT NULL THEN 1 ELSE 0 END), 0),
                    COALESCE(SUM(CASE WHEN stopped_at IS NULL AND handoff_at IS NOT NULL THEN 1 ELSE 0 END), 0)
             FROM telegram_customer_chats WHERE space_id = ?",
        )
        .bind(space)
        .fetch_one(&self.pool)
        .await?;
        Ok(CustomerCounts { chats, active: chats - stopped, stopped, waiting_on_person: waiting })
    }

    /// The lead of the space's newest team made from a pack (a template), when it still exists:
    /// the default bot to answer customers.
    pub async fn pack_lead(&self, space: &str) -> Result<Option<String>, DbError> {
        let row: Option<(String,)> = sqlx::query_as(
            "SELECT t.lead_kwant_id FROM team_creations c
               JOIN teams t ON t.id = c.team_id
               JOIN kwants k ON k.id = t.lead_kwant_id
             WHERE c.space_id = ? AND c.status = 'applied' AND c.template_id IS NOT NULL AND k.space_id = ?
             ORDER BY c.created_at DESC LIMIT 1",
        )
        .bind(space)
        .bind(space)
        .fetch_optional(&self.pool)
        .await?;
        Ok(row.map(|r| r.0))
    }

    /// The packs (template ids) of the space's teams `kwant_id` leads or belongs to, newest first.
    pub async fn packs_of_kwant(&self, space: &str, kwant_id: &str) -> Result<Vec<String>, DbError> {
        let like = format!("%\"{}\"%", kwant_id.replace(['%', '_', '"'], ""));
        let rows: Vec<(String,)> = sqlx::query_as(
            "SELECT c.template_id FROM team_creations c
               JOIN teams t ON t.id = c.team_id
             WHERE c.space_id = ? AND c.status = 'applied' AND c.template_id IS NOT NULL
               AND (t.lead_kwant_id = ? OR t.member_kwants LIKE ?)
             ORDER BY c.created_at DESC",
        )
        .bind(space)
        .bind(kwant_id)
        .bind(like)
        .fetch_all(&self.pool)
        .await?;
        let mut packs: Vec<String> = Vec::new();
        for (id,) in rows {
            if !packs.contains(&id) {
                packs.push(id);
            }
        }
        Ok(packs)
    }
}
