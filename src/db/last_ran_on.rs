//! The engine and model each bot's newest reply ran on, for the model badge on resting rows:
//! `kwants.last_ran_on`, one nullable JSON column written as a reply's turn ends and read with
//! the bot list. A column (not a query on the newest reply's `meta.ran_on`) because replies carry
//! the bot's name, not its id, and the per-bot lookup needed a new index on every message to stay
//! fast; see autonomy/model-badge/impl.md "Lin server" for the measurements.

use super::{Database, DbError};
use chrono::{DateTime, NaiveDateTime, Utc};
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};

/// `{engine, family, model, model_label, tier, picked_by, at}` from a reply's `meta.ran_on` and
/// the time the reply was made. `picked_by` is "laya", "pinned" or "fallback" (the reply's own
/// "pinned_model" / "pinned_engine" read as "pinned"; a reply that failed over is "fallback").
pub fn last_ran_on(ran_on: &Value, failed_over: bool, at: DateTime<Utc>) -> Value {
    let picked_by = match ran_on["picked_by"].as_str() {
        _ if failed_over => Some("fallback"),
        Some("pinned_model" | "pinned_engine" | "pinned") => Some("pinned"),
        Some("laya") => Some("laya"),
        Some("fallback") => Some("fallback"),
        _ => None,
    };
    json!({
        "engine": ran_on["engine"],
        "family": ran_on["family"],
        "model": ran_on["model"],
        "model_label": ran_on["model_label"],
        "tier": ran_on["tier"],
        "picked_by": picked_by,
        "at": at.to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
    })
}

/// A stored `created_at`: SQLite's "YYYY-MM-DD HH:MM:SS" (UTC) or RFC 3339.
fn parse_at(s: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(s)
        .map(|d| d.with_timezone(&Utc))
        .ok()
        .or_else(|| NaiveDateTime::parse_from_str(s, "%Y-%m-%d %H:%M:%S%.f").ok().map(|n| n.and_utc()))
}

/// The `last_ran_on` of a saved reply's meta, when it has a `ran_on`.
fn from_meta(meta: &str, at: &str) -> Option<Value> {
    let meta: Value = serde_json::from_str(meta).ok()?;
    let ran_on = meta.get("ran_on").filter(|r| r.is_object())?;
    let failed_over = meta["failover"].as_array().is_some_and(|a| !a.is_empty());
    Some(last_ran_on(ran_on, failed_over, parse_at(at)?))
}

/// Replies with a `ran_on`, the newest per sender name; one scan of `messages`.
const NEWEST_BY_NAME: &str = "SELECT sender_id, meta, MAX(created_at) FROM messages
     WHERE sender_role = 'kwant' AND meta LIKE '%\"ran_on\"%'
       AND CASE WHEN json_valid(meta) THEN json_type(meta, '$.ran_on') = 'object' ELSE 0 END
     GROUP BY sender_id";

/// The newest reply with a `ran_on` in the bot's own direct chat or threads, for a bot whose name
/// another bot (in another space) also has.
const NEWEST_OF_BOT: &str = "SELECT meta, created_at FROM messages
     WHERE sender_role = 'kwant' AND sender_id = ?1
       AND (direct_kwant_id = ?2 OR thread_id IN (SELECT id FROM threads WHERE kwant_id = ?2))
       AND meta LIKE '%\"ran_on\"%'
       AND CASE WHEN json_valid(meta) THEN json_type(meta, '$.ran_on') = 'object' ELSE 0 END
     ORDER BY created_at DESC, rowid DESC LIMIT 1";

impl Database {
    /// Adds `kwants.last_ran_on` once and fills it from each bot's newest reply. Run after every
    /// step that can rebuild `kwants`.
    pub(super) async fn migrate_last_ran_on(&self) -> Result<(), DbError> {
        let has: Option<(String,)> = sqlx::query_as("SELECT name FROM pragma_table_info('kwants') WHERE name = 'last_ran_on'")
            .fetch_optional(&self.pool)
            .await?;
        if has.is_some() {
            return Ok(());
        }
        let mut tx = self.begin_write().await?;
        sqlx::query("ALTER TABLE kwants ADD COLUMN last_ran_on TEXT").execute(&mut *tx).await?;
        let bots: Vec<(String, String)> = sqlx::query_as("SELECT id, name FROM kwants").fetch_all(&mut *tx).await?;
        let mut seen = HashSet::new();
        let shared: HashSet<String> = bots.iter().filter(|(_, n)| !seen.insert(n.as_str())).map(|(_, n)| n.clone()).collect();
        let newest: HashMap<String, (String, String)> = sqlx::query_as::<_, (String, String, String)>(NEWEST_BY_NAME)
            .fetch_all(&mut *tx)
            .await?
            .into_iter()
            .map(|(name, meta, at)| (name, (meta, at)))
            .collect();
        for (id, name) in &bots {
            let row = if shared.contains(name) {
                sqlx::query_as::<_, (String, String)>(NEWEST_OF_BOT).bind(name).bind(id).fetch_optional(&mut *tx).await?
            } else {
                // Very old replies were saved under the bot's id
                newest.get(name).or_else(|| newest.get(id)).cloned()
            };
            if let Some(v) = row.and_then(|(meta, at)| from_meta(&meta, &at)) {
                sqlx::query("UPDATE kwants SET last_ran_on = ? WHERE id = ?").bind(v.to_string()).bind(id).execute(&mut *tx).await?;
            }
        }
        tx.commit().await?;
        Ok(())
    }

    /// Records what `kwant_id`'s turn just ran on (see [`last_ran_on`]).
    pub async fn set_last_ran_on(&self, kwant_id: &str, value: &Value) -> Result<(), DbError> {
        sqlx::query("UPDATE kwants SET last_ran_on = ? WHERE id = ?")
            .bind(value.to_string())
            .bind(kwant_id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    /// Every bot's `last_ran_on` by id; a bot that never replied has none.
    pub async fn last_ran_on_all(&self) -> Result<HashMap<String, Value>, DbError> {
        let rows: Vec<(String, String)> = sqlx::query_as("SELECT id, last_ran_on FROM kwants WHERE last_ran_on IS NOT NULL")
            .fetch_all(&self.pool)
            .await?;
        Ok(rows.into_iter().filter_map(|(id, v)| Some((id, serde_json::from_str(&v).ok()?))).collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_reply_ran_on_becomes_the_badge_value() {
        let at = DateTime::parse_from_rfc3339("2026-11-13T02:00:00Z").unwrap().with_timezone(&Utc);
        let ran_on = json!({"engine": "claude", "engine_name": "Claude Code CLI", "family": "claude", "model": "sonnet",
            "model_label": "Sonnet", "tier": "balanced", "picked_by": "pinned_model", "effort": "medium", "profile_id": "p", "account": "This machine"});
        assert_eq!(
            last_ran_on(&ran_on, false, at),
            json!({"engine": "claude", "family": "claude", "model": "sonnet", "model_label": "Sonnet", "tier": "balanced",
                "picked_by": "pinned", "at": "2026-11-13T02:00:00Z"})
        );
        assert_eq!(last_ran_on(&ran_on, true, at)["picked_by"], "fallback");
        assert_eq!(last_ran_on(&json!({"engine": "grok", "picked_by": "laya"}), false, at)["tier"], Value::Null);
    }

    async fn reply(db: &Database, sender: &str, direct: &str, meta: Option<Value>, at: &str) {
        let msg = crate::db::models::NewMessage {
            channel_id: None,
            direct_kwant_id: Some(direct.into()),
            sender_role: "kwant".into(),
            sender_id: sender.into(),
            text: "hi".into(),
            reasoning_content: None,
            tool_calls: None,
            tokens_used: 0,
        };
        let m = db.insert_message_with(&msg, &crate::db::models::MessageExtra { thread_id: None, meta }).await.unwrap();
        sqlx::query("UPDATE messages SET created_at = ? WHERE id = ?").bind(at).bind(&m.id).execute(&db.pool).await.unwrap();
    }

    #[tokio::test]
    async fn an_existing_database_is_backfilled_once_from_each_bots_newest_reply() {
        let tmp = tempfile::tempdir().unwrap();
        let db = Database::init(tmp.path()).await.unwrap();
        let bots: Vec<(String, String)> = sqlx::query_as("SELECT id, name FROM kwants ORDER BY name").fetch_all(&db.pool).await.unwrap();
        assert!(bots.len() >= 3, "{bots:?}");
        let ((a, a_name), (b, b_name), (c, _)) = (bots[0].clone(), bots[1].clone(), bots[2].clone());
        // A database from before the column: replies saved, no column
        sqlx::query("ALTER TABLE kwants DROP COLUMN last_ran_on").execute(&db.pool).await.unwrap();
        let ran = |engine: &str, model: &str, tier: &str| json!({"ran_on": {"engine": engine, "family": engine, "model": model, "model_label": model, "tier": tier, "picked_by": "laya"}});
        reply(&db, &a_name, &a, Some(ran("claude", "opus", "deep")), "2026-11-10 09:00:00").await;
        reply(&db, &a_name, &a, Some(ran("codex", "gpt-6.1-sol", "balanced")), "2026-11-12 09:00:00").await;
        // Newer, with no ran_on (a gave-up line) or a broken meta: skipped, never fatal
        reply(&db, &a_name, &a, None, "2026-11-12 10:00:00").await;
        reply(&db, &a_name, &a, Some(json!({"kind": "notice"})), "2026-11-12 11:00:00").await;
        sqlx::query("UPDATE messages SET meta = '{\"ran_on\": oops' WHERE created_at = '2026-11-12 11:00:00'").execute(&db.pool).await.unwrap();
        // B's name is shared with C (another space may reuse a name): each gets its own chat's reply
        sqlx::query("UPDATE kwants SET name = ? WHERE id = ?").bind(&b_name).bind(&c).execute(&db.pool).await.unwrap();
        reply(&db, &b_name, &b, Some(ran("grok", "grok-4-heavy", "deep")), "2026-11-11 09:00:00").await;
        reply(&db, &b_name, &c, Some(ran("claude", "haiku", "fast")), "2026-11-12 12:00:00").await;

        db.migrate_last_ran_on().await.unwrap();
        let all = db.last_ran_on_all().await.unwrap();
        assert_eq!(
            all[&a],
            json!({"engine": "codex", "family": "codex", "model": "gpt-6.1-sol", "model_label": "gpt-6.1-sol", "tier": "balanced",
                "picked_by": "laya", "at": "2026-11-12T09:00:00Z"})
        );
        assert_eq!((all[&b]["engine"].clone(), all[&c]["engine"].clone()), (json!("grok"), json!("claude")), "{all:?}");
        assert!(bots[3..].iter().all(|(id, _)| !all.contains_key(id)), "a bot that never replied has none: {all:?}");

        // Once: a value written since is never overwritten by a later start
        db.set_last_ran_on(&a, &json!({"engine": "grok"})).await.unwrap();
        db.migrate_last_ran_on().await.unwrap();
        assert_eq!(db.last_ran_on_all().await.unwrap()[&a], json!({"engine": "grok"}));
    }

    #[test]
    fn stored_times_and_bad_meta_read_safely() {
        assert_eq!(parse_at("2026-11-13 01:02:03").unwrap().to_rfc3339(), "2026-11-13T01:02:03+00:00");
        assert!(parse_at("2026-11-13T01:02:03.5+00:00").is_some());
        assert!(from_meta("{not json", "2026-11-13 01:02:03").is_none());
        assert!(from_meta(r#"{"ran_on": "vm"}"#, "2026-11-13 01:02:03").is_none());
        assert!(from_meta(r#"{"ran_on": {"engine": "codex"}, "failover": [{}]}"#, "2026-11-13 01:02:03").is_some_and(|v| v["picked_by"] == "fallback"));
    }
}
