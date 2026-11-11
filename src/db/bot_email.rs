//! 0.10.0: bot email (add-only tables). One bot mailbox per space (its settings, never the
//! password: that is in the vault), each bot's on/off and last address, the addresses handed
//! out (one per use), the mail already decided, and a log of mail in and out that feeds the
//! hourly caps and the refused-reply records. The rules live in `crate::mail` and
//! `crate::server::bot_email`.

use super::{Database, DbError};
use chrono::{DateTime, Duration, Utc};
use serde::Serialize;
use sqlx::FromRow;
use std::collections::HashSet;

/// Decided mail and log rows are kept this long.
pub const KEEP_DAYS: i64 = 30;

#[derive(Debug, Clone, FromRow, Serialize, PartialEq)]
pub struct BotEmailRow {
    pub space_id: String,
    /// `crate::mail::config::MailConfig` as JSON.
    pub config: String,
    /// The setup checks' last results: `{"sign_in": "pass", "reply": "waiting", ...}`.
    pub checks: String,
    pub last_check_at: Option<DateTime<Utc>>,
    pub last_ok_at: Option<DateTime<Utc>>,
    pub last_error_at: Option<DateTime<Utc>>,
    pub last_error: Option<String>,
    pub last_error_code: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// One address handed out: a bot's sign-up, a question sent by email, or the setup test.
#[derive(Debug, Clone, FromRow, Serialize, PartialEq)]
pub struct MailTag {
    pub tag: String,
    pub space_id: String,
    pub kwant_id: Option<String>,
    /// `signup`, `ask` or `test`.
    pub kind: String,
    pub address: String,
    pub site: Option<String>,
    pub question_id: Option<String>,
    /// The Message-ID of the mail kwant sent from it (asks, the test), without brackets.
    pub message_id: Option<String>,
    /// The chat message that shows a wait (updated in place).
    pub chat_message_id: Option<String>,
    pub created_at: DateTime<Utc>,
    pub expires_at: DateTime<Utc>,
    pub used_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, FromRow, Serialize, PartialEq)]
pub struct MailLogRow {
    pub id: i64,
    pub space_id: String,
    /// `in` or `out`.
    pub direction: String,
    pub kwant_id: Option<String>,
    pub tag: Option<String>,
    pub question_id: Option<String>,
    /// The other side's base address (lowercase).
    pub counterpart: Option<String>,
    /// `code`, `answer`, `refused`, `sent`, `test`, ...
    pub outcome: String,
    /// For a refused reply: `not_on_list`, `dmarc` or `unclear`.
    pub reason: Option<String>,
    pub created_at: DateTime<Utc>,
}

impl Database {
    pub(super) async fn migrate_bot_email(&self) -> Result<(), DbError> {
        for stmt in [
            "CREATE TABLE IF NOT EXISTS bot_email_config (
                space_id TEXT PRIMARY KEY,
                config TEXT NOT NULL,
                checks TEXT NOT NULL DEFAULT '{}',
                last_check_at TIMESTAMP,
                last_ok_at TIMESTAMP,
                last_error_at TIMESTAMP,
                last_error TEXT,
                last_error_code TEXT,
                created_at TIMESTAMP NOT NULL,
                updated_at TIMESTAMP NOT NULL
            )",
            "CREATE TABLE IF NOT EXISTS bot_email_bots (
                kwant_id TEXT PRIMARY KEY,
                enabled INTEGER NOT NULL DEFAULT 1,
                last_address TEXT,
                updated_at TIMESTAMP NOT NULL
            )",
            "CREATE TABLE IF NOT EXISTS bot_email_tags (
                tag TEXT PRIMARY KEY,
                space_id TEXT NOT NULL,
                kwant_id TEXT,
                kind TEXT NOT NULL,
                address TEXT NOT NULL,
                site TEXT,
                question_id TEXT,
                message_id TEXT,
                chat_message_id TEXT,
                created_at TIMESTAMP NOT NULL,
                expires_at TIMESTAMP NOT NULL,
                used_at TIMESTAMP
            )",
            "CREATE INDEX IF NOT EXISTS bot_email_tags_space ON bot_email_tags (space_id, kind, used_at)",
            "CREATE INDEX IF NOT EXISTS bot_email_tags_question ON bot_email_tags (question_id)",
            "CREATE INDEX IF NOT EXISTS bot_email_tags_address ON bot_email_tags (space_id, address)",
            "CREATE TABLE IF NOT EXISTS bot_email_seen (
                space_id TEXT NOT NULL,
                key TEXT NOT NULL,
                outcome TEXT NOT NULL,
                seen_at TIMESTAMP NOT NULL,
                PRIMARY KEY (space_id, key)
            )",
            "CREATE TABLE IF NOT EXISTS bot_email_log (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                space_id TEXT NOT NULL,
                direction TEXT NOT NULL,
                kwant_id TEXT,
                tag TEXT,
                question_id TEXT,
                counterpart TEXT,
                outcome TEXT NOT NULL,
                reason TEXT,
                created_at TIMESTAMP NOT NULL
            )",
            "CREATE INDEX IF NOT EXISTS bot_email_log_window ON bot_email_log (space_id, direction, created_at)",
            "CREATE INDEX IF NOT EXISTS bot_email_log_question ON bot_email_log (question_id)",
        ] {
            sqlx::query(stmt).execute(&self.pool).await?;
        }
        let cutoff = Utc::now() - Duration::days(KEEP_DAYS);
        sqlx::query("DELETE FROM bot_email_seen WHERE seen_at < ?")
            .bind(cutoff)
            .execute(&self.pool)
            .await?;
        sqlx::query("DELETE FROM bot_email_log WHERE created_at < ?")
            .bind(cutoff)
            .execute(&self.pool)
            .await?;
        sqlx::query("DELETE FROM bot_email_tags WHERE expires_at < ?")
            .bind(cutoff)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    // --- The mailbox ---------------------------------------------------------------------------

    pub async fn bot_email_config(&self, space: &str) -> Result<Option<BotEmailRow>, DbError> {
        Ok(
            sqlx::query_as::<_, BotEmailRow>("SELECT * FROM bot_email_config WHERE space_id = ?")
                .bind(space)
                .fetch_optional(&self.pool)
                .await?,
        )
    }

    pub async fn bot_email_configs(&self) -> Result<Vec<BotEmailRow>, DbError> {
        Ok(
            sqlx::query_as::<_, BotEmailRow>("SELECT * FROM bot_email_config ORDER BY space_id")
                .fetch_all(&self.pool)
                .await?,
        )
    }

    /// Saves the settings and the checks; a new mailbox starts with no health yet.
    pub async fn bot_email_save(
        &self,
        space: &str,
        config: &str,
        checks: &str,
    ) -> Result<(), DbError> {
        let now = crate::clock::now();
        sqlx::query(
            "INSERT INTO bot_email_config (space_id, config, checks, last_check_at, last_ok_at, created_at, updated_at) VALUES (?1, ?2, ?3, ?4, ?4, ?4, ?4)
             ON CONFLICT(space_id) DO UPDATE SET config = ?2, checks = ?3, last_check_at = ?4, last_ok_at = ?4,
                 last_error_at = NULL, last_error = NULL, last_error_code = NULL, updated_at = ?4",
        )
        .bind(space)
        .bind(config)
        .bind(checks)
        .bind(now)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn bot_email_set_config(&self, space: &str, config: &str) -> Result<bool, DbError> {
        let r = sqlx::query(
            "UPDATE bot_email_config SET config = ?, updated_at = ? WHERE space_id = ?",
        )
        .bind(config)
        .bind(crate::clock::now())
        .bind(space)
        .execute(&self.pool)
        .await?;
        Ok(r.rows_affected() == 1)
    }

    pub async fn bot_email_set_checks(&self, space: &str, checks: &str) -> Result<(), DbError> {
        sqlx::query("UPDATE bot_email_config SET checks = ?, updated_at = ? WHERE space_id = ?")
            .bind(checks)
            .bind(crate::clock::now())
            .bind(space)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    /// Mailbox health (spec ENG-14): a read or send that worked, or the reason it didn't.
    pub async fn bot_email_health(
        &self,
        space: &str,
        error: Option<(&str, &str)>,
    ) -> Result<(), DbError> {
        let now = crate::clock::now();
        match error {
            None => sqlx::query("UPDATE bot_email_config SET last_check_at = ?1, last_ok_at = ?1 WHERE space_id = ?2").bind(now).bind(space).execute(&self.pool).await?,
            Some((code, message)) => sqlx::query("UPDATE bot_email_config SET last_check_at = ?1, last_error_at = ?1, last_error_code = ?2, last_error = ?3 WHERE space_id = ?4")
                .bind(now)
                .bind(code)
                .bind(message)
                .bind(space)
                .execute(&self.pool)
                .await?,
        };
        Ok(())
    }

    /// Disconnect: the settings and everything kept for the space go. Each bot's on/off stays.
    pub async fn bot_email_delete(&self, space: &str) -> Result<(), DbError> {
        for t in [
            "bot_email_config",
            "bot_email_tags",
            "bot_email_seen",
            "bot_email_log",
        ] {
            sqlx::query(&format!("DELETE FROM {t} WHERE space_id = ?"))
                .bind(space)
                .execute(&self.pool)
                .await?;
        }
        Ok(())
    }

    // --- Each bot ------------------------------------------------------------------------------

    /// Whether the bot may use bot email (on unless turned off, spec ENG-5) and its last address.
    pub async fn bot_email_bot(&self, kwant: &str) -> Result<(bool, Option<String>), DbError> {
        let row: Option<(bool, Option<String>)> =
            sqlx::query_as("SELECT enabled, last_address FROM bot_email_bots WHERE kwant_id = ?")
                .bind(kwant)
                .fetch_optional(&self.pool)
                .await?;
        Ok(row.unwrap_or((true, None)))
    }

    pub async fn bot_email_set_enabled(&self, kwant: &str, enabled: bool) -> Result<(), DbError> {
        sqlx::query(
            "INSERT INTO bot_email_bots (kwant_id, enabled, updated_at) VALUES (?1, ?2, ?3)
             ON CONFLICT(kwant_id) DO UPDATE SET enabled = ?2, updated_at = ?3",
        )
        .bind(kwant)
        .bind(enabled)
        .bind(crate::clock::now())
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    async fn bot_email_set_last(&self, kwant: &str, address: &str) -> Result<(), DbError> {
        sqlx::query(
            "INSERT INTO bot_email_bots (kwant_id, enabled, last_address, updated_at) VALUES (?1, 1, ?2, ?3)
             ON CONFLICT(kwant_id) DO UPDATE SET last_address = ?2, updated_at = ?3",
        )
        .bind(kwant)
        .bind(address)
        .bind(crate::clock::now())
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    // --- Addresses -----------------------------------------------------------------------------

    pub async fn bot_email_add_tag(&self, t: &MailTag) -> Result<(), DbError> {
        sqlx::query(
            "INSERT INTO bot_email_tags (tag, space_id, kwant_id, kind, address, site, question_id, message_id, chat_message_id, created_at, expires_at, used_at)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(&t.tag)
        .bind(&t.space_id)
        .bind(&t.kwant_id)
        .bind(&t.kind)
        .bind(&t.address)
        .bind(&t.site)
        .bind(&t.question_id)
        .bind(&t.message_id)
        .bind(&t.chat_message_id)
        .bind(t.created_at)
        .bind(t.expires_at)
        .bind(t.used_at)
        .execute(&self.pool)
        .await?;
        if let (Some(k), "signup") = (&t.kwant_id, t.kind.as_str()) {
            self.bot_email_set_last(k, &t.address).await?;
        }
        Ok(())
    }

    pub async fn bot_email_tag(&self, tag: &str) -> Result<Option<MailTag>, DbError> {
        Ok(
            sqlx::query_as::<_, MailTag>("SELECT * FROM bot_email_tags WHERE tag = ?")
                .bind(tag)
                .fetch_optional(&self.pool)
                .await?,
        )
    }

    /// The newest address a bot was given that is `address` (any case).
    pub async fn bot_email_tag_by_address(
        &self,
        space: &str,
        kwant: &str,
        address: &str,
    ) -> Result<Option<MailTag>, DbError> {
        Ok(sqlx::query_as::<_, MailTag>("SELECT * FROM bot_email_tags WHERE space_id = ? AND kwant_id = ? AND lower(address) = lower(?) ORDER BY created_at DESC LIMIT 1")
            .bind(space)
            .bind(kwant)
            .bind(address)
            .fetch_optional(&self.pool)
            .await?)
    }

    /// Open (unused, unexpired) addresses of a kind in a space, newest first.
    pub async fn bot_email_open_tags(
        &self,
        space: &str,
        kind: &str,
    ) -> Result<Vec<MailTag>, DbError> {
        Ok(sqlx::query_as::<_, MailTag>("SELECT * FROM bot_email_tags WHERE space_id = ? AND kind = ? AND used_at IS NULL AND expires_at > ? ORDER BY created_at DESC LIMIT 200")
            .bind(space)
            .bind(kind)
            .bind(crate::clock::now())
            .fetch_all(&self.pool)
            .await?)
    }

    pub async fn bot_email_tag_for_question(
        &self,
        question: &str,
    ) -> Result<Option<MailTag>, DbError> {
        Ok(sqlx::query_as::<_, MailTag>("SELECT * FROM bot_email_tags WHERE question_id = ? AND kind = 'ask' ORDER BY created_at DESC LIMIT 1").bind(question).fetch_optional(&self.pool).await?)
    }

    /// Marks an address used; `false` when it already was.
    pub async fn bot_email_use_tag(&self, tag: &str) -> Result<bool, DbError> {
        let r =
            sqlx::query("UPDATE bot_email_tags SET used_at = ? WHERE tag = ? AND used_at IS NULL")
                .bind(crate::clock::now())
                .bind(tag)
                .execute(&self.pool)
                .await?;
        Ok(r.rows_affected() == 1)
    }

    pub async fn bot_email_set_tag_message(
        &self,
        tag: &str,
        message_id: Option<&str>,
        chat_message_id: Option<&str>,
    ) -> Result<(), DbError> {
        sqlx::query("UPDATE bot_email_tags SET message_id = COALESCE(?, message_id), chat_message_id = COALESCE(?, chat_message_id) WHERE tag = ?")
            .bind(message_id)
            .bind(chat_message_id)
            .bind(tag)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    // --- Mail decided --------------------------------------------------------------------------

    pub async fn bot_email_seen(&self, space: &str) -> Result<HashSet<String>, DbError> {
        let rows: Vec<(String,)> =
            sqlx::query_as("SELECT key FROM bot_email_seen WHERE space_id = ?")
                .bind(space)
                .fetch_all(&self.pool)
                .await?;
        Ok(rows.into_iter().map(|(k,)| k).collect())
    }

    /// Marks a mail decided; `false` when another reader already decided it.
    pub async fn bot_email_mark_seen(
        &self,
        space: &str,
        key: &str,
        outcome: &str,
    ) -> Result<bool, DbError> {
        let r = sqlx::query("INSERT OR IGNORE INTO bot_email_seen (space_id, key, outcome, seen_at) VALUES (?, ?, ?, ?)")
            .bind(space)
            .bind(key)
            .bind(outcome)
            .bind(crate::clock::now())
            .execute(&self.pool)
            .await?;
        Ok(r.rows_affected() == 1)
    }

    // --- The log -------------------------------------------------------------------------------

    #[allow(clippy::too_many_arguments)]
    pub async fn bot_email_log(
        &self,
        space: &str,
        direction: &str,
        kwant: Option<&str>,
        tag: Option<&str>,
        question: Option<&str>,
        counterpart: Option<&str>,
        outcome: &str,
        reason: Option<&str>,
    ) -> Result<(), DbError> {
        sqlx::query("INSERT INTO bot_email_log (space_id, direction, kwant_id, tag, question_id, counterpart, outcome, reason, created_at) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)")
            .bind(space)
            .bind(direction)
            .bind(kwant)
            .bind(tag)
            .bind(question)
            .bind(counterpart.map(str::to_ascii_lowercase))
            .bind(outcome)
            .bind(reason)
            .bind(crate::clock::now())
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    /// Mail in one direction since `since`, from or to `counterpart` when given (the caps).
    pub async fn bot_email_count(
        &self,
        space: &str,
        direction: &str,
        counterpart: Option<&str>,
        since: DateTime<Utc>,
    ) -> Result<i64, DbError> {
        let (n,): (i64,) = sqlx::query_as("SELECT COUNT(*) FROM bot_email_log WHERE space_id = ?1 AND direction = ?2 AND (?3 IS NULL OR counterpart = ?3) AND created_at >= ?4")
            .bind(space)
            .bind(direction)
            .bind(counterpart.map(str::to_ascii_lowercase))
            .bind(since)
            .fetch_one(&self.pool)
            .await?;
        Ok(n)
    }

    /// Replies that didn't answer a question, newest first (spec ENG-10).
    pub async fn bot_email_refused(&self, question: &str) -> Result<Vec<MailLogRow>, DbError> {
        Ok(sqlx::query_as::<_, MailLogRow>("SELECT * FROM bot_email_log WHERE question_id = ? AND direction = 'in' AND outcome = 'refused' AND reason IN ('not_on_list', 'dmarc', 'unclear') ORDER BY created_at DESC LIMIT 20")
            .bind(question)
            .fetch_all(&self.pool)
            .await?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn db() -> (Database, tempfile::TempDir) {
        let tmp = tempfile::tempdir().unwrap();
        (Database::init(tmp.path()).await.unwrap(), tmp)
    }

    fn tag(tag: &str, kind: &str, address: &str, hours: i64) -> MailTag {
        let now = Utc::now();
        MailTag {
            tag: tag.into(),
            space_id: "s1".into(),
            kwant_id: Some("k1".into()),
            kind: kind.into(),
            address: address.into(),
            site: Some("tiktok.com".into()),
            question_id: (kind == "ask").then(|| "q1".to_string()),
            message_id: None,
            chat_message_id: None,
            created_at: now,
            expires_at: now + Duration::hours(hours),
            used_at: None,
        }
    }

    #[tokio::test]
    async fn an_address_takes_one_code_and_expired_ones_are_not_open() {
        let (db, _t) = db().await;
        db.bot_email_add_tag(&tag("kwant-a-1", "signup", "Bots+kwant-a-1@x.test", 24))
            .await
            .unwrap();
        db.bot_email_add_tag(&tag("kwant-a-2", "signup", "bots+kwant-a-2@x.test", -1))
            .await
            .unwrap();
        let open = db.bot_email_open_tags("s1", "signup").await.unwrap();
        assert_eq!(
            open.iter().map(|t| t.tag.as_str()).collect::<Vec<_>>(),
            vec!["kwant-a-1"]
        );
        // Found by address in any case, only for its own bot and space
        assert!(db
            .bot_email_tag_by_address("s1", "k1", "bots+KWANT-a-1@x.test")
            .await
            .unwrap()
            .is_some());
        assert!(db
            .bot_email_tag_by_address("s1", "k2", "bots+kwant-a-1@x.test")
            .await
            .unwrap()
            .is_none());
        assert!(db
            .bot_email_tag_by_address("s2", "k1", "bots+kwant-a-1@x.test")
            .await
            .unwrap()
            .is_none());
        assert!(db.bot_email_use_tag("kwant-a-1").await.unwrap());
        assert!(
            !db.bot_email_use_tag("kwant-a-1").await.unwrap(),
            "a second code is refused"
        );
        assert!(db
            .bot_email_open_tags("s1", "signup")
            .await
            .unwrap()
            .is_empty());
        // A sign-up address is the bot's last address
        assert_eq!(
            db.bot_email_bot("k1").await.unwrap(),
            (true, Some("bots+kwant-a-2@x.test".into()))
        );
    }

    #[tokio::test]
    async fn a_mail_is_decided_once() {
        let (db, _t) = db().await;
        assert!(db
            .bot_email_mark_seen("s1", "INBOX:7:1", "code")
            .await
            .unwrap());
        assert!(
            !db.bot_email_mark_seen("s1", "INBOX:7:1", "refused")
                .await
                .unwrap(),
            "two readers never both take it"
        );
        assert!(
            db.bot_email_mark_seen("s2", "INBOX:7:1", "code")
                .await
                .unwrap(),
            "per space"
        );
        assert_eq!(
            db.bot_email_seen("s1").await.unwrap(),
            ["INBOX:7:1".to_string()].into_iter().collect()
        );
    }

    #[tokio::test]
    async fn bots_are_on_unless_turned_off_and_disconnect_keeps_that() {
        let (db, _t) = db().await;
        assert_eq!(db.bot_email_bot("k1").await.unwrap(), (true, None));
        db.bot_email_set_enabled("k1", false).await.unwrap();
        db.bot_email_save("s1", "{}", "{}").await.unwrap();
        db.bot_email_add_tag(&tag("kwant-a-1", "signup", "a@x.test", 24))
            .await
            .unwrap();
        db.bot_email_mark_seen("s1", "k", "code").await.unwrap();
        db.bot_email_delete("s1").await.unwrap();
        assert!(db.bot_email_config("s1").await.unwrap().is_none());
        assert!(db.bot_email_tag("kwant-a-1").await.unwrap().is_none());
        assert!(db.bot_email_seen("s1").await.unwrap().is_empty());
        assert!(
            !db.bot_email_bot("k1").await.unwrap().0,
            "each bot's on/off stays"
        );
    }

    #[tokio::test]
    async fn caps_count_per_sender_in_any_case_and_refusals_list_only_replies() {
        let (db, _t) = db().await;
        let hour_ago = Utc::now() - Duration::hours(1);
        db.bot_email_log(
            "s1",
            "in",
            None,
            None,
            Some("q1"),
            Some("Sam@Person.test"),
            "refused",
            Some("dmarc"),
        )
        .await
        .unwrap();
        db.bot_email_log(
            "s1",
            "in",
            None,
            None,
            Some("q1"),
            Some("sam@person.test"),
            "refused",
            Some("automated (auto-replied)"),
        )
        .await
        .unwrap();
        db.bot_email_log(
            "s1",
            "in",
            None,
            None,
            Some("q1"),
            Some("bob@other.test"),
            "refused",
            Some("not_on_list"),
        )
        .await
        .unwrap();
        db.bot_email_log(
            "s1",
            "out",
            None,
            None,
            None,
            Some("sam@person.test"),
            "sent",
            None,
        )
        .await
        .unwrap();
        assert_eq!(
            db.bot_email_count("s1", "in", Some("SAM@person.test"), hour_ago)
                .await
                .unwrap(),
            2
        );
        assert_eq!(
            db.bot_email_count("s1", "out", None, hour_ago)
                .await
                .unwrap(),
            1
        );
        assert_eq!(
            db.bot_email_count("s2", "out", None, hour_ago)
                .await
                .unwrap(),
            0
        );
        let mut reasons: Vec<_> = db
            .bot_email_refused("q1")
            .await
            .unwrap()
            .into_iter()
            .filter_map(|r| r.reason)
            .collect();
        reasons.sort();
        assert_eq!(
            reasons,
            vec!["dmarc", "not_on_list"],
            "robots aren't shown as refused replies"
        );
    }
}
