//! Talk to your bots from Telegram (0.7.8, next-077 spec §G): one link per space. The bot token
//! is never here: it is in the vault (`crate::server::telegram::VAULT_OWNER`). The pairing code
//! is kept only as its sha256. All tables are add-only.

use super::{Database, DbError};
use chrono::{DateTime, Utc};
use serde::Serialize;
use sqlx::FromRow;

/// Setup states (`GET /api/connections/telegram` `state`; "off" is "no row").
pub const TOKEN_SAVED: &str = "token_saved";
pub const PAIRING: &str = "pairing";
pub const PAIRED: &str = "paired";
/// Telegram said the saved token no longer works (401 on getUpdates).
pub const REJECTED: &str = "rejected";

/// The needs-you kinds and their defaults (design §G5, decision A20).
pub const DEFAULT_KINDS: &str = r#"{"question":true,"approval":true,"human_step":true,"stuck":true,"done":false}"#;

#[derive(Debug, Clone, Serialize, FromRow, PartialEq)]
pub struct TelegramLink {
    pub space_id: String,
    pub state: String,
    pub bot_id: Option<i64>,
    pub bot_username: Option<String>,
    pub bot_name: Option<String>,
    #[serde(skip)]
    pub pair_code_sha256: Option<String>,
    pub pair_expires_at: Option<DateTime<Utc>>,
    pub pair_failures: i64,
    pub pair_failures_since: Option<DateTime<Utc>>,
    pub pair_locked: bool,
    pub user_id: Option<i64>,
    pub chat_id: Option<i64>,
    pub user_name: Option<String>,
    pub user_username: Option<String>,
    pub paired_at: Option<DateTime<Utc>>,
    /// The pairing an Unpair ended, as JSON, for the 10 s Undo (design "Connected card").
    #[serde(skip)]
    pub prev_pairing: Option<String>,
    pub prev_until: Option<DateTime<Utc>>,
    /// JSON object: question, approval, human_step, stuck, done.
    pub kinds: String,
    pub allow_approvals: bool,
    pub allow_since: Option<DateTime<Utc>>,
    /// The next getUpdates offset: one past the last update kwant has stored.
    pub update_offset: i64,
    /// When the person last wrote from Telegram: bot replies in #general after it go back (A19).
    pub reply_window_from: Option<DateTime<Utc>>,
    /// The newest #general message already sent to Telegram (created_at, then id).
    pub reply_cursor_at: Option<DateTime<Utc>>,
    pub reply_cursor_id: Option<String>,
    pub last_message_at: Option<DateTime<Utc>>,
    /// Messages from chats other than the paired one: only this count is kept (Q-G1).
    pub strangers: i64,
    pub last_error: Option<String>,
    /// Where kwant is open, `https://host[:port]` only: the "Open in kwant" buttons link here.
    /// Telegram refuses link buttons to plain http, so without it messages carry no link button.
    pub app_url: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl TelegramLink {
    pub fn kinds_json(&self) -> serde_json::Value {
        let mut v: serde_json::Value = serde_json::from_str(DEFAULT_KINDS).unwrap_or_default();
        if let Ok(serde_json::Value::Object(saved)) = serde_json::from_str::<serde_json::Value>(&self.kinds) {
            for (k, on) in saved {
                if v.get(&k).is_some() && on.is_boolean() {
                    v[&k] = on;
                }
            }
        }
        v
    }

    pub fn kind_on(&self, kind: &str) -> bool {
        self.kinds_json()[kind].as_bool().unwrap_or(false)
    }

    pub fn is_paired(&self) -> bool {
        self.state == PAIRED && self.user_id.is_some() && self.chat_id.is_some()
    }
}

/// One message kwant sent that can be answered or edited later: a question, an approval or an
/// alert. `id` is the short token in its buttons' `callback_data`.
#[derive(Debug, Clone, Serialize, FromRow, PartialEq)]
pub struct TelegramAsk {
    pub id: String,
    pub space_id: String,
    /// "question", "approval" or "alert".
    pub kind: String,
    /// The question or approval id.
    pub ref_id: String,
    pub chat_id: i64,
    pub message_id: Option<i64>,
    /// The HTML that was sent, so an edit can keep it and add the "✓" line.
    pub text: String,
    /// JSON array of the choices, in button order.
    pub choices: String,
    /// Whether an Allow once button was offered.
    pub allow: bool,
    /// "open" or "closed".
    pub state: String,
    pub created_at: DateTime<Utc>,
}

impl Database {
    pub(super) async fn migrate_telegram(&self) -> Result<(), DbError> {
        for stmt in [
            "CREATE TABLE IF NOT EXISTS telegram_links (
                space_id TEXT PRIMARY KEY,
                state TEXT NOT NULL,
                bot_id INTEGER,
                bot_username TEXT,
                bot_name TEXT,
                pair_code_sha256 TEXT,
                pair_expires_at TIMESTAMP,
                pair_failures INTEGER NOT NULL DEFAULT 0,
                pair_failures_since TIMESTAMP,
                pair_locked INTEGER NOT NULL DEFAULT 0,
                user_id INTEGER,
                chat_id INTEGER,
                user_name TEXT,
                user_username TEXT,
                paired_at TIMESTAMP,
                prev_pairing TEXT,
                prev_until TIMESTAMP,
                kinds TEXT NOT NULL DEFAULT '{}',
                allow_approvals INTEGER NOT NULL DEFAULT 0,
                allow_since TIMESTAMP,
                update_offset INTEGER NOT NULL DEFAULT 0,
                reply_window_from TIMESTAMP,
                reply_cursor_at TIMESTAMP,
                reply_cursor_id TEXT,
                last_message_at TIMESTAMP,
                strangers INTEGER NOT NULL DEFAULT 0,
                last_error TEXT,
                app_url TEXT,
                created_at TIMESTAMP NOT NULL,
                updated_at TIMESTAMP NOT NULL
            )",
            "CREATE TABLE IF NOT EXISTS telegram_asks (
                id TEXT PRIMARY KEY,
                space_id TEXT NOT NULL,
                kind TEXT NOT NULL,
                ref_id TEXT NOT NULL,
                chat_id INTEGER NOT NULL,
                message_id INTEGER,
                text TEXT NOT NULL,
                choices TEXT NOT NULL DEFAULT '[]',
                allow INTEGER NOT NULL DEFAULT 0,
                state TEXT NOT NULL DEFAULT 'open',
                created_at TIMESTAMP NOT NULL
            )",
            "CREATE INDEX IF NOT EXISTS telegram_asks_ref ON telegram_asks (space_id, kind, ref_id)",
            // Updates already stored, so a restart between storing and moving the offset can't
            // post the same message twice
            "CREATE TABLE IF NOT EXISTS telegram_updates (
                space_id TEXT NOT NULL,
                update_id INTEGER NOT NULL,
                created_at TIMESTAMP NOT NULL,
                PRIMARY KEY (space_id, update_id)
            )",
            // Which kwant message each Telegram message is, both ways, for replies (G-T3)
            "CREATE TABLE IF NOT EXISTS telegram_messages (
                space_id TEXT NOT NULL,
                chat_id INTEGER NOT NULL,
                tg_message_id INTEGER NOT NULL,
                message_id TEXT NOT NULL,
                created_at TIMESTAMP NOT NULL,
                PRIMARY KEY (space_id, chat_id, tg_message_id)
            )",
        ] {
            sqlx::query(stmt).execute(&self.pool).await?;
        }
        // Where a resolved ask was answered (design "Data the design needs": `answered_via`)
        for table in ["questions", "approvals"] {
            let has: Option<(String,)> = sqlx::query_as(&format!("SELECT name FROM pragma_table_info('{table}') WHERE name = 'answered_via'"))
                .fetch_optional(&self.pool)
                .await?;
            if has.is_none() {
                sqlx::query(&format!("ALTER TABLE {table} ADD COLUMN answered_via TEXT")).execute(&self.pool).await?;
            }
        }
        Ok(())
    }

    pub async fn telegram_link(&self, space: &str) -> Result<Option<TelegramLink>, DbError> {
        Ok(sqlx::query_as::<_, TelegramLink>("SELECT * FROM telegram_links WHERE space_id = ?").bind(space).fetch_optional(&self.pool).await?)
    }

    pub async fn telegram_links(&self) -> Result<Vec<TelegramLink>, DbError> {
        Ok(sqlx::query_as::<_, TelegramLink>("SELECT * FROM telegram_links ORDER BY created_at").fetch_all(&self.pool).await?)
    }

    /// A new token was accepted: the bot is (re)set, any pairing and the approvals switch end.
    pub async fn telegram_save_bot(&self, space: &str, bot_id: i64, username: &str, name: &str) -> Result<(), DbError> {
        let now = crate::clock::now();
        sqlx::query(
            "INSERT INTO telegram_links (space_id, state, bot_id, bot_username, bot_name, kinds, created_at, updated_at)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?)
             ON CONFLICT(space_id) DO UPDATE SET state = excluded.state, bot_id = excluded.bot_id,
               bot_username = excluded.bot_username, bot_name = excluded.bot_name,
               pair_code_sha256 = NULL, pair_expires_at = NULL, pair_failures = 0, pair_failures_since = NULL,
               pair_locked = 0, user_id = NULL, chat_id = NULL, user_name = NULL, user_username = NULL,
               paired_at = NULL, prev_pairing = NULL, prev_until = NULL, allow_approvals = 0, allow_since = NULL,
               update_offset = 0, reply_window_from = NULL, last_error = NULL, updated_at = excluded.updated_at",
        )
        .bind(space)
        .bind(TOKEN_SAVED)
        .bind(bot_id)
        .bind(username)
        .bind(name)
        .bind(DEFAULT_KINDS)
        .bind(now)
        .bind(now)
        .execute(&self.pool)
        .await?;
        // A new bot can't edit the old one's messages, and none of them may allow anything now
        sqlx::query("UPDATE telegram_asks SET state = 'closed', allow = 0 WHERE space_id = ?").bind(space).execute(&self.pool).await?;
        Ok(())
    }

    /// A fresh pairing code replaces any earlier one (one outstanding code, G-P15) and lifts the
    /// lock five bad codes set (Q-G1).
    pub async fn telegram_set_code(&self, space: &str, sha256: &str, expires_at: DateTime<Utc>) -> Result<bool, DbError> {
        let r = sqlx::query(
            "UPDATE telegram_links SET state = ?, pair_code_sha256 = ?, pair_expires_at = ?, pair_failures = 0,
               pair_failures_since = NULL, pair_locked = 0, updated_at = ?
             WHERE space_id = ? AND state IN (?, ?)",
        )
        .bind(PAIRING)
        .bind(sha256)
        .bind(expires_at)
        .bind(crate::clock::now())
        .bind(space)
        .bind(TOKEN_SAVED)
        .bind(PAIRING)
        .execute(&self.pool)
        .await?;
        Ok(r.rows_affected() == 1)
    }

    /// The code is used or expired: back to "token saved".
    pub async fn telegram_clear_code(&self, space: &str, sha256: &str) -> Result<bool, DbError> {
        let r = sqlx::query(
            "UPDATE telegram_links SET state = ?, pair_code_sha256 = NULL, pair_expires_at = NULL, updated_at = ?
             WHERE space_id = ? AND state = ? AND pair_code_sha256 = ?",
        )
        .bind(TOKEN_SAVED)
        .bind(crate::clock::now())
        .bind(space)
        .bind(PAIRING)
        .bind(sha256)
        .execute(&self.pool)
        .await?;
        Ok(r.rows_affected() == 1)
    }

    /// Pairs the account that sent the right code. Only one wins: the code must still be the one
    /// on the row (G-P17).
    pub async fn telegram_pair(&self, space: &str, sha256: &str, user_id: i64, chat_id: i64, name: &str, username: Option<&str>) -> Result<bool, DbError> {
        let now = crate::clock::now();
        let r = sqlx::query(
            "UPDATE telegram_links SET state = ?, pair_code_sha256 = NULL, pair_expires_at = NULL, pair_failures = 0,
               pair_failures_since = NULL, pair_locked = 0, user_id = ?, chat_id = ?, user_name = ?, user_username = ?,
               paired_at = ?, prev_pairing = NULL, prev_until = NULL, allow_approvals = 0, allow_since = NULL,
               reply_window_from = NULL, updated_at = ?
             WHERE space_id = ? AND state = ? AND pair_code_sha256 = ?",
        )
        .bind(PAIRED)
        .bind(user_id)
        .bind(chat_id)
        .bind(name)
        .bind(username)
        .bind(now)
        .bind(now)
        .bind(space)
        .bind(PAIRING)
        .bind(sha256)
        .execute(&self.pool)
        .await?;
        Ok(r.rows_affected() == 1)
    }

    /// One more wrong or expired code. Returns the row after it (locked at the fifth in 10 min).
    pub async fn telegram_code_failed(&self, space: &str, window_start: DateTime<Utc>, max: i64) -> Result<(), DbError> {
        let now = crate::clock::now();
        sqlx::query(
            "UPDATE telegram_links SET
               pair_failures = CASE WHEN pair_failures_since IS NULL OR pair_failures_since < ? THEN 1 ELSE pair_failures + 1 END,
               pair_failures_since = CASE WHEN pair_failures_since IS NULL OR pair_failures_since < ? THEN ? ELSE pair_failures_since END,
               updated_at = ?
             WHERE space_id = ?",
        )
        .bind(window_start)
        .bind(window_start)
        .bind(now)
        .bind(now)
        .bind(space)
        .execute(&self.pool)
        .await?;
        sqlx::query("UPDATE telegram_links SET pair_locked = 1 WHERE space_id = ? AND pair_failures >= ?")
            .bind(space)
            .bind(max)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    /// Ends the pairing (Unpair, /stop). The pairing is kept for `undo_until` so Undo can bring
    /// it back; the approvals switch goes off either way (A23).
    pub async fn telegram_unpair(&self, space: &str, undo_until: Option<DateTime<Utc>>) -> Result<Option<TelegramLink>, DbError> {
        let Some(link) = self.telegram_link(space).await? else { return Ok(None) };
        if !link.is_paired() {
            return Ok(None);
        }
        let prev = serde_json::json!({
            "user_id": link.user_id, "chat_id": link.chat_id, "user_name": link.user_name,
            "user_username": link.user_username, "paired_at": link.paired_at,
        });
        sqlx::query(
            "UPDATE telegram_links SET state = ?, user_id = NULL, chat_id = NULL, user_name = NULL, user_username = NULL,
               paired_at = NULL, prev_pairing = ?, prev_until = ?, allow_approvals = 0, allow_since = NULL,
               reply_window_from = NULL, updated_at = ?
             WHERE space_id = ? AND state = ?",
        )
        .bind(TOKEN_SAVED)
        .bind(undo_until.map(|_| prev.to_string()))
        .bind(undo_until)
        .bind(crate::clock::now())
        .bind(space)
        .bind(PAIRED)
        .execute(&self.pool)
        .await?;
        sqlx::query("UPDATE telegram_asks SET state = 'closed' WHERE space_id = ?").bind(space).execute(&self.pool).await?;
        Ok(Some(link))
    }

    /// Undo of an Unpair within its window. The approvals switch stays off.
    pub async fn telegram_undo_unpair(&self, space: &str) -> Result<bool, DbError> {
        let Some(link) = self.telegram_link(space).await? else { return Ok(false) };
        let (Some(prev), Some(until)) = (link.prev_pairing.as_deref(), link.prev_until) else { return Ok(false) };
        if crate::clock::now() > until || link.state != TOKEN_SAVED {
            return Ok(false);
        }
        let p: serde_json::Value = serde_json::from_str(prev).unwrap_or_default();
        let r = sqlx::query(
            "UPDATE telegram_links SET state = ?, user_id = ?, chat_id = ?, user_name = ?, user_username = ?, paired_at = ?,
               prev_pairing = NULL, prev_until = NULL, pair_code_sha256 = NULL, pair_expires_at = NULL, updated_at = ?
             WHERE space_id = ? AND state = ?",
        )
        .bind(PAIRED)
        .bind(p["user_id"].as_i64())
        .bind(p["chat_id"].as_i64())
        .bind(p["user_name"].as_str())
        .bind(p["user_username"].as_str())
        .bind(p["paired_at"].as_str().and_then(|s| s.parse::<DateTime<Utc>>().ok()))
        .bind(crate::clock::now())
        .bind(space)
        .bind(TOKEN_SAVED)
        .execute(&self.pool)
        .await?;
        Ok(r.rows_affected() == 1)
    }

    pub async fn telegram_set_kinds(&self, space: &str, kinds: &serde_json::Value) -> Result<(), DbError> {
        sqlx::query("UPDATE telegram_links SET kinds = ?, updated_at = ? WHERE space_id = ?")
            .bind(kinds.to_string())
            .bind(crate::clock::now())
            .bind(space)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    /// The approvals switch. On needs a pairing; off always works.
    pub async fn telegram_set_allow(&self, space: &str, on: bool) -> Result<bool, DbError> {
        let now = crate::clock::now();
        let r = if on {
            sqlx::query("UPDATE telegram_links SET allow_approvals = 1, allow_since = COALESCE(allow_since, ?), updated_at = ? WHERE space_id = ? AND state = ?")
                .bind(now)
                .bind(now)
                .bind(space)
                .bind(PAIRED)
                .execute(&self.pool)
                .await?
        } else {
            let r = sqlx::query("UPDATE telegram_links SET allow_approvals = 0, allow_since = NULL, updated_at = ? WHERE space_id = ?")
                .bind(now)
                .bind(space)
                .execute(&self.pool)
                .await?;
            self.telegram_disarm_asks(space).await?;
            r
        };
        Ok(r.rows_affected() == 1)
    }

    /// Every Allow once button still out stops counting, in the database, whether or not kwant can
    /// reach Telegram to edit the messages (Lin's M1).
    pub async fn telegram_disarm_asks(&self, space: &str) -> Result<(), DbError> {
        sqlx::query("UPDATE telegram_asks SET allow = 0 WHERE space_id = ?").bind(space).execute(&self.pool).await?;
        Ok(())
    }

    pub async fn telegram_set_state(&self, space: &str, state: &str, error: Option<&str>) -> Result<(), DbError> {
        sqlx::query("UPDATE telegram_links SET state = ?, last_error = ?, allow_approvals = CASE WHEN ? = ? THEN 0 ELSE allow_approvals END, updated_at = ? WHERE space_id = ?")
            .bind(state)
            .bind(error)
            .bind(state)
            .bind(REJECTED)
            .bind(crate::clock::now())
            .bind(space)
            .execute(&self.pool)
            .await?;
        if state == REJECTED {
            sqlx::query("UPDATE telegram_asks SET allow = 0 WHERE space_id = ?").bind(space).execute(&self.pool).await?;
        }
        Ok(())
    }

    pub async fn telegram_set_error(&self, space: &str, error: Option<&str>) -> Result<(), DbError> {
        sqlx::query("UPDATE telegram_links SET last_error = ? WHERE space_id = ?").bind(error).bind(space).execute(&self.pool).await?;
        Ok(())
    }

    pub async fn telegram_set_offset(&self, space: &str, offset: i64) -> Result<(), DbError> {
        sqlx::query("UPDATE telegram_links SET update_offset = MAX(update_offset, ?) WHERE space_id = ?").bind(offset).bind(space).execute(&self.pool).await?;
        Ok(())
    }

    /// Whether this update is new (and records it). False: it was stored before a restart.
    pub async fn telegram_claim_update(&self, space: &str, update_id: i64) -> Result<bool, DbError> {
        let r = sqlx::query("INSERT OR IGNORE INTO telegram_updates (space_id, update_id, created_at) VALUES (?, ?, ?)")
            .bind(space)
            .bind(update_id)
            .bind(Utc::now())
            .execute(&self.pool)
            .await?;
        // Keep the last thousand
        sqlx::query("DELETE FROM telegram_updates WHERE space_id = ? AND update_id < ?").bind(space).bind(update_id - 1000).execute(&self.pool).await?;
        Ok(r.rows_affected() == 1)
    }

    pub async fn telegram_count_stranger(&self, space: &str) -> Result<(), DbError> {
        sqlx::query("UPDATE telegram_links SET strangers = strangers + 1 WHERE space_id = ?").bind(space).execute(&self.pool).await?;
        Ok(())
    }

    /// The person wrote from Telegram: bot replies in #general from now on go back (A19). A new
    /// window starts its cursor here, so nothing said while it was closed is sent.
    ///
    /// `at` (the test clock) opens the six-hour window. The cursor is compared with messages'
    /// `created_at`, which is real time, so it starts at the real time (Lin's L6).
    pub async fn telegram_person_wrote(&self, space: &str, at: DateTime<Utc>) -> Result<(), DbError> {
        sqlx::query(
            "UPDATE telegram_links SET reply_window_from = ?1, last_message_at = ?1,
               reply_cursor_at = CASE WHEN reply_window_from IS NULL OR reply_cursor_at IS NULL THEN ?3 ELSE reply_cursor_at END,
               reply_cursor_id = CASE WHEN reply_window_from IS NULL OR reply_cursor_at IS NULL THEN NULL ELSE reply_cursor_id END
             WHERE space_id = ?2",
        )
            .bind(at)
            .bind(space)
            .bind(Utc::now())
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    pub async fn telegram_set_cursor(&self, space: &str, at: DateTime<Utc>, id: &str) -> Result<(), DbError> {
        sqlx::query("UPDATE telegram_links SET reply_cursor_at = ?, reply_cursor_id = ? WHERE space_id = ?").bind(at).bind(id).bind(space).execute(&self.pool).await?;
        Ok(())
    }

    pub async fn telegram_touch(&self, space: &str) -> Result<(), DbError> {
        sqlx::query("UPDATE telegram_links SET last_message_at = ? WHERE space_id = ?").bind(crate::clock::now()).bind(space).execute(&self.pool).await?;
        Ok(())
    }

    /// Disconnect: the row and everything kept for it go.
    pub async fn telegram_delete(&self, space: &str) -> Result<(), DbError> {
        for t in ["telegram_links", "telegram_asks", "telegram_updates", "telegram_messages"] {
            sqlx::query(&format!("DELETE FROM {t} WHERE space_id = ?")).bind(space).execute(&self.pool).await?;
        }
        Ok(())
    }

    pub async fn telegram_add_ask(&self, ask: &TelegramAsk) -> Result<(), DbError> {
        sqlx::query("INSERT INTO telegram_asks (id, space_id, kind, ref_id, chat_id, message_id, text, choices, allow, state, created_at) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)")
            .bind(&ask.id)
            .bind(&ask.space_id)
            .bind(&ask.kind)
            .bind(&ask.ref_id)
            .bind(ask.chat_id)
            .bind(ask.message_id)
            .bind(&ask.text)
            .bind(&ask.choices)
            .bind(ask.allow)
            .bind(&ask.state)
            .bind(ask.created_at)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    pub async fn telegram_ask(&self, space: &str, id: &str) -> Result<Option<TelegramAsk>, DbError> {
        Ok(sqlx::query_as::<_, TelegramAsk>("SELECT * FROM telegram_asks WHERE space_id = ? AND id = ?").bind(space).bind(id).fetch_optional(&self.pool).await?)
    }

    pub async fn telegram_asks_for(&self, kind: &str, ref_id: &str) -> Result<Vec<TelegramAsk>, DbError> {
        Ok(sqlx::query_as::<_, TelegramAsk>("SELECT * FROM telegram_asks WHERE kind = ? AND ref_id = ? ORDER BY created_at").bind(kind).bind(ref_id).fetch_all(&self.pool).await?)
    }

    /// Open asks of `kind` in the space; `with_allow` only those that offered Allow once.
    pub async fn telegram_open_asks(&self, space: &str, kind: &str, with_allow: bool) -> Result<Vec<TelegramAsk>, DbError> {
        Ok(sqlx::query_as::<_, TelegramAsk>("SELECT * FROM telegram_asks WHERE space_id = ? AND kind = ? AND state = 'open' AND (allow = 1 OR ? = 0) ORDER BY created_at")
            .bind(space)
            .bind(kind)
            .bind(with_allow)
            .fetch_all(&self.pool)
            .await?)
    }

    pub async fn telegram_set_app_url(&self, space: &str, url: Option<&str>) -> Result<(), DbError> {
        sqlx::query("UPDATE telegram_links SET app_url = ? WHERE space_id = ?").bind(url).bind(space).execute(&self.pool).await?;
        Ok(())
    }

    pub async fn telegram_ask_sent(&self, id: &str, message_id: i64) -> Result<(), DbError> {
        sqlx::query("UPDATE telegram_asks SET message_id = ? WHERE id = ?").bind(message_id).bind(id).execute(&self.pool).await?;
        Ok(())
    }

    /// Closes an open ask. True for the one caller that closed it (an answer counts once).
    pub async fn telegram_close_ask(&self, id: &str) -> Result<bool, DbError> {
        let r = sqlx::query("UPDATE telegram_asks SET state = 'closed' WHERE id = ? AND state = 'open'").bind(id).execute(&self.pool).await?;
        Ok(r.rows_affected() == 1)
    }

    pub async fn telegram_reopen_ask(&self, id: &str) -> Result<(), DbError> {
        sqlx::query("UPDATE telegram_asks SET state = 'open' WHERE id = ?").bind(id).execute(&self.pool).await?;
        Ok(())
    }

    pub async fn telegram_ask_link_only(&self, id: &str) -> Result<(), DbError> {
        sqlx::query("UPDATE telegram_asks SET allow = 0 WHERE id = ?").bind(id).execute(&self.pool).await?;
        Ok(())
    }

    pub async fn telegram_map_message(&self, space: &str, chat_id: i64, tg_message_id: i64, message_id: &str) -> Result<(), DbError> {
        sqlx::query("INSERT OR REPLACE INTO telegram_messages (space_id, chat_id, tg_message_id, message_id, created_at) VALUES (?, ?, ?, ?, ?)")
            .bind(space)
            .bind(chat_id)
            .bind(tg_message_id)
            .bind(message_id)
            .bind(Utc::now())
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    pub async fn telegram_message_for(&self, space: &str, chat_id: i64, tg_message_id: i64) -> Result<Option<String>, DbError> {
        let row: Option<(String,)> = sqlx::query_as("SELECT message_id FROM telegram_messages WHERE space_id = ? AND chat_id = ? AND tg_message_id = ?")
            .bind(space)
            .bind(chat_id)
            .bind(tg_message_id)
            .fetch_optional(&self.pool)
            .await?;
        Ok(row.map(|r| r.0))
    }

    /// Adds keys to a message's `meta` (`source: "telegram"`, `sent_to_telegram`).
    pub async fn telegram_mark_message(&self, message_id: &str, key: &str, value: &serde_json::Value) -> Result<(), DbError> {
        sqlx::query("UPDATE messages SET meta = json_set(CASE WHEN json_valid(meta) THEN meta ELSE '{}' END, '$.' || ?, json(?)) WHERE id = ?")
            .bind(key)
            .bind(value.to_string())
            .bind(message_id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    /// Records where a question or approval was answered (`answered_via`).
    pub async fn set_answered_via(&self, table: &str, id: &str, via: &str) -> Result<(), DbError> {
        let table = if table == "approvals" { "approvals" } else { "questions" };
        sqlx::query(&format!("UPDATE {table} SET answered_via = ? WHERE id = ?")).bind(via).bind(id).execute(&self.pool).await?;
        Ok(())
    }

    /// Messages in a channel after the cursor `(at, id)`, oldest first, outside threads (the
    /// reply window's sweep, A19). `messages.created_at` is SQLite's second-precision text, so
    /// both sides are compared through `datetime()`.
    pub async fn telegram_channel_since(&self, channel_id: &str, at: DateTime<Utc>, id: &str, limit: i64) -> Result<Vec<crate::db::models::Message>, DbError> {
        Ok(sqlx::query_as::<_, crate::db::models::Message>(
            "SELECT * FROM messages WHERE channel_id = ? AND thread_id IS NULL
               AND (datetime(created_at) > datetime(?) OR (datetime(created_at) = datetime(?) AND id > ?))
             ORDER BY datetime(created_at), id LIMIT ?",
        )
        .bind(channel_id)
        .bind(at)
        .bind(at)
        .bind(id)
        .bind(limit)
        .fetch_all(&self.pool)
        .await?)
    }

    /// Closes the reply window: the person wrote in kwant, or it ran six hours.
    pub async fn telegram_close_window(&self, space: &str) -> Result<(), DbError> {
        sqlx::query("UPDATE telegram_links SET reply_window_from = NULL WHERE space_id = ?").bind(space).execute(&self.pool).await?;
        Ok(())
    }
}
