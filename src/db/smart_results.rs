//! `message_results`: the Smart Results block set of a bot reply, beside the message and never in
//! its text or `meta` (see `crate::server::smart_results`).

use std::collections::HashMap;

use serde_json::Value;

use super::models::{Message, MessageExtra, NewMessage};
use super::{Database, DbError};
use crate::server::smart_results::{self as sr, ResultSet};

impl Database {
    /// Additive: one row per message that has a block set; deleting the message deletes it.
    pub(super) async fn migrate_smart_results(&self) -> Result<(), DbError> {
        sqlx::query(
            r#"
            CREATE TABLE IF NOT EXISTS message_results (
                message_id TEXT PRIMARY KEY REFERENCES messages(id) ON DELETE CASCADE,
                version INTEGER NOT NULL,
                payload TEXT NOT NULL,
                created_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP
            )
            "#,
        )
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// Sets `results` on the bot replies in `messages` that have a stored block set this build
    /// still serves. Never fails: a read error leaves them `None`, and the chat shows markdown.
    pub async fn attach_results(&self, messages: &mut [Message]) {
        let ids: Vec<&str> = messages.iter().filter(|m| m.sender_role == "kwant").map(|m| m.id.as_str()).collect();
        if ids.is_empty() {
            return;
        }
        let ids = serde_json::to_string(&ids).unwrap_or_default();
        let rows: Vec<(String, i64, String)> = match sqlx::query_as(
            "SELECT message_id, version, payload FROM message_results WHERE message_id IN (SELECT value FROM json_each(?))",
        )
        .bind(ids)
        .fetch_all(&self.pool)
        .await
        {
            Ok(rows) => rows,
            Err(e) => {
                tracing::warn!("smart results: couldn't read stored blocks: {e}");
                return;
            }
        };
        let found: HashMap<String, Value> =
            rows.into_iter().filter_map(|(id, version, payload)| read(version, &payload).map(|v| (id, v))).collect();
        for m in messages.iter_mut() {
            if let Some(v) = found.get(&m.id) {
                m.results = Some(v.clone());
            }
        }
    }
}

/// A stored payload this build can serve, or `None` (an unknown version, a type no longer
/// enabled, or a block that fails today's checks).
fn read(version: i64, payload: &str) -> Option<Value> {
    if version != i64::from(sr::VERSION) {
        return None;
    }
    let set: ResultSet = serde_json::from_str(payload).ok()?;
    sr::validate(&set, None).ok()?;
    serde_json::to_value(&set).ok()
}

/// `meta.kind`s whose text is still a bot's own reply: a teammate's hand-off result, which the
/// chat draws as markdown inside its card.
const REPLY_KINDS: &[&str] = &["delegation_result"];

/// `meta.kind`s that are a card kwant builds from its own data and that have a block of their own:
/// a mission task's report line (`verification`) and an agent's shared file (`files`).
const DATA_KINDS: &[&str] = &["mission", "file"];

/// Computes and stores the block set for a message just inserted on `conn` as `id`, and returns
/// it for the message object. Only bot replies qualify: `sender_role` "kwant" and no `meta.kind`
/// other than [`REPLY_KINDS`] (cards and kwnt's own posts have one; a reply's `ran_on`, `usage`
/// or `routing` meta doesn't count). Never fails: a failed write is logged and the message stays
/// as it was.
pub(super) async fn store_on(
    conn: &mut sqlx::SqliteConnection,
    input: &NewMessage,
    extra: &MessageExtra,
    id: &str,
    root: Option<&std::path::Path>,
) -> Option<Value> {
    let kind = extra.meta.as_ref().and_then(|m| m.get("kind"));
    let card = kind.is_some_and(|k| !k.as_str().is_some_and(|k| REPLY_KINDS.contains(&k) || DATA_KINDS.contains(&k)));
    if input.sender_role != "kwant" || card {
        return None;
    }
    let set = sr::compute_for_message(conn, input, extra, root).await?;
    let value = serde_json::to_value(&set).ok()?;
    let saved = sqlx::query("INSERT OR REPLACE INTO message_results (message_id, version, payload) VALUES (?, ?, ?)")
        .bind(id)
        .bind(i64::from(set.version))
        .bind(value.to_string())
        .execute(&mut *conn)
        .await;
    match saved {
        Ok(_) => Some(value),
        Err(e) => {
            tracing::warn!("smart results: couldn't store the block for message {id}: {e}");
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const TABLE: &str = "Here are the plans.\n\n| Plan | Price | Seats |\n|---|---|---|\n| Basic | EUR 12 | 1 |\n| Team | EUR 18 | 5 |\n| Business | EUR 26 | 20 |\n\nI recommend Team.\n";

    fn message(role: &str, text: &str) -> NewMessage {
        NewMessage {
            channel_id: None,
            direct_kwant_id: Some("k1".into()),
            sender_role: role.into(),
            sender_id: if role == "kwant" { "k1".into() } else { "user".into() },
            text: text.into(),
            reasoning_content: None,
            tool_calls: None,
            tokens_used: 0,
        }
    }

    fn with_meta(meta: Value) -> MessageExtra {
        MessageExtra { meta: Some(meta), ..Default::default() }
    }

    async fn stored(db: &Database, id: &str) -> Option<(i64, String)> {
        sqlx::query_as("SELECT version, payload FROM message_results WHERE message_id = ?")
            .bind(id)
            .fetch_optional(&db.pool)
            .await
            .unwrap()
    }

    async fn served(db: &Database, id: &str) -> Option<Value> {
        let mut m = vec![db.get_message(id).await.unwrap().unwrap()];
        db.attach_results(&mut m).await;
        m.remove(0).results
    }

    #[tokio::test]
    async fn a_bot_reply_with_a_table_is_stored_and_served() {
        let tmp = tempfile::tempdir().unwrap();
        let db = Database::init(tmp.path()).await.unwrap();
        let m = db.insert_message(&message("kwant", TABLE)).await.unwrap();
        let results = m.results.clone().expect("the saved message carries its block");
        assert_eq!(results["version"], 1);
        let block = &results["blocks"][0];
        assert_eq!(block["type"], "comparison");
        assert_eq!(block["rows"][1]["cells"], json!(["Team", "EUR 18", "5"]));
        assert_eq!(block["highlight_row"], "r1");
        assert_eq!(stored(&db, &m.id).await.map(|r| r.0), Some(1));
        assert_eq!(served(&db, &m.id).await, Some(results));
        assert_eq!(m.text, TABLE, "the text is never changed");
        assert_eq!(serde_json::to_value(&m).unwrap()["results"]["blocks"][0]["type"], "comparison");
    }

    #[tokio::test]
    async fn only_ordinary_bot_replies_qualify() {
        let tmp = tempfile::tempdir().unwrap();
        let db = Database::init(tmp.path()).await.unwrap();
        let person = db.insert_message(&message("user", TABLE)).await.unwrap();
        let card = db.insert_message_with(&message("kwant", TABLE), &with_meta(json!({"kind": "routing_note"}))).await.unwrap();
        let plain = db.insert_message(&message("kwant", "Done: the build is green.")).await.unwrap();
        for m in [&person, &card, &plain] {
            assert_eq!(m.results, None, "{}", m.text);
            assert_eq!(stored(&db, &m.id).await, None, "{}", m.text);
            assert_eq!(served(&db, &m.id).await, None, "{}", m.text);
            assert_eq!(serde_json::to_value(m).unwrap()["results"], Value::Null, "results is always there");
        }
        // A reply's own meta (where it ran, what it used) doesn't make it a card
        let ran = db.insert_message_with(&message("kwant", TABLE), &with_meta(json!({"ran_on": "vm"}))).await.unwrap();
        assert!(ran.results.is_some());
        // A teammate's hand-off result is its reply; kwnt's note that the work stopped is not
        let handed = db.insert_message_with(&message("kwant", TABLE), &with_meta(json!({"kind": "delegation_result"}))).await.unwrap();
        assert!(handed.results.is_some());
        let stopped = db.insert_message_with(&message("system", TABLE), &with_meta(json!({"kind": "delegation_result"}))).await.unwrap();
        assert_eq!(stopped.results, None);
        let odd = db.insert_message_with(&message("kwant", TABLE), &with_meta(json!({"kind": ["delegation_result"]}))).await.unwrap();
        assert_eq!(odd.results, None, "a kind that isn't a string is a card");
    }

    #[tokio::test]
    async fn an_edited_or_deleted_reply_loses_its_block() {
        let tmp = tempfile::tempdir().unwrap();
        let db = Database::init(tmp.path()).await.unwrap();
        let edited = db.insert_message(&message("kwant", TABLE)).await.unwrap();
        db.set_message_text(&edited.id, "The plans changed; ask me again.").await.unwrap();
        assert_eq!(stored(&db, &edited.id).await, None);
        assert_eq!(served(&db, &edited.id).await, None);
        let deleted = db.insert_message(&message("kwant", TABLE)).await.unwrap();
        sqlx::query("DELETE FROM messages WHERE id = ?").bind(&deleted.id).execute(&db.pool).await.unwrap();
        assert_eq!(stored(&db, &deleted.id).await, None);
    }

    #[tokio::test]
    async fn a_stored_block_this_build_cannot_serve_stays_hidden() {
        let tmp = tempfile::tempdir().unwrap();
        let db = Database::init(tmp.path()).await.unwrap();
        let m = db.insert_message(&message("kwant", TABLE)).await.unwrap();
        let (_, good) = stored(&db, &m.id).await.unwrap();
        let edit = |change: &dyn Fn(&mut Value)| {
            let mut v: Value = serde_json::from_str(&good).unwrap();
            change(&mut v);
            v.to_string()
        };
        let cases = [
            (2, good.clone()),
            (1, "{not json".to_string()),
            (1, edit(&|v| v["blocks"][0]["type"] = json!("chart"))),
            (1, edit(&|v| v["blocks"][0]["highlight_row"] = json!("r9"))),
            (1, edit(&|v| v["blocks"][0]["rows"][0]["cells"][0] = json!("<img src=x onerror=alert(1)>"))),
            (1, edit(&|v| v["blocks"] = json!([]))),
        ];
        for (version, payload) in cases {
            sqlx::query("UPDATE message_results SET version = ?, payload = ? WHERE message_id = ?")
                .bind(version)
                .bind(&payload)
                .bind(&m.id)
                .execute(&db.pool)
                .await
                .unwrap();
            assert_eq!(served(&db, &m.id).await, None, "{version} {payload}");
        }
    }

    #[tokio::test]
    async fn blocks_are_attached_to_bot_replies_only() {
        let tmp = tempfile::tempdir().unwrap();
        let db = Database::init(tmp.path()).await.unwrap();
        let bot = db.insert_message(&message("kwant", TABLE)).await.unwrap();
        let person = db.insert_message(&message("user", "Which plan?")).await.unwrap();
        // Even a row stored under a person's message is never served
        sqlx::query(
            "INSERT INTO message_results (message_id, version, payload) SELECT ?, version, payload FROM message_results WHERE message_id = ?",
        )
        .bind(&person.id)
        .bind(&bot.id)
        .execute(&db.pool)
        .await
        .unwrap();
        let mut list = db.list_messages("k1", 10).await.unwrap();
        db.attach_results(&mut list).await;
        let got: Vec<(&str, bool)> = list.iter().map(|m| (m.sender_role.as_str(), m.results.is_some())).collect();
        assert_eq!(got, [("kwant", true), ("user", false)]);
    }
}
