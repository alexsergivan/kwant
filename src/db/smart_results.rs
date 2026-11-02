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
    // ---- verification and files (smart-results-typed) ----

    use crate::db::attachments::NewAttachment;
    use crate::db::missions::{NewCheck, NewTask};
    use crate::db::spaces::SpaceId;
    use sha2::{Digest, Sha256};

    async fn task_with_checks(db: &Database, checks: &[(&str, Value, &str, Option<&str>)]) -> String {
        let t = db
            .create_task(
                &SpaceId::default_space(),
                &NewTask { title: "Ship it".into(), state: "failed".into(), priority: 1, max_attempts: 3, source_kind: "person".into(), created_by: "person".into(), ..Default::default() },
            )
            .await
            .unwrap();
        for (kind, spec, status, excerpt) in checks {
            let c = db
                .add_check(&NewCheck { task_id: t.id.clone(), kind: (*kind).into(), spec: spec.clone(), required: true, approved: true, requested_by: "person".into(), approved_by: Some("person".into()) })
                .await
                .unwrap();
            sqlx::query("UPDATE outcome_checks SET status = ?, output_excerpt = ?, finished_at = CASE WHEN ? IN ('passed','failed','error') THEN CURRENT_TIMESTAMP END WHERE id = ?")
                .bind(status)
                .bind(excerpt)
                .bind(status)
                .bind(&c.id)
                .execute(&db.pool)
                .await
                .unwrap();
        }
        t.id
    }

    async fn report(db: &Database, task_id: &str, text: &str) -> Message {
        db.insert_message_with(&message("kwant", text), &with_meta(json!({"kind": "mission", "task_id": task_id}))).await.unwrap()
    }

    #[tokio::test]
    async fn a_task_report_gets_its_verification_from_the_stored_checks() {
        let tmp = tempfile::tempdir().unwrap();
        let db = Database::init(tmp.path()).await.unwrap();
        let id = task_with_checks(
            &db,
            &[
                ("command", json!({"command": "cargo test"}), "passed", Some("12 passed")),
                ("http", json!({"url": "https://example.com/health"}), "failed", Some("status 500")),
                ("file_exists", json!({"path": "out.txt"}), "error", None),
                ("command", json!({"command": "make e2e"}), "running", None),
            ],
        )
        .await;
        let m = report(&db, &id, "Failed: Ship it").await;
        let block = &m.results.clone().expect("a verification block")["blocks"][0];
        assert_eq!(block["type"], "verification");
        assert_eq!(block["provenance"]["kind"], "kwant_data");
        assert_eq!(block["provenance"]["ref"], format!("task:{id}"));
        assert_eq!(block["target"], json!({"kind": "task", "id": id, "title": "Ship it"}));
        assert_eq!(block["summary"], "1 of 4 checks passed", "counts come from the rows, pending is not passed");
        let statuses: Vec<&str> = block["checks"].as_array().unwrap().iter().map(|c| c["status"].as_str().unwrap()).collect();
        assert_eq!(statuses, ["passed", "failed", "error", "pending"]);
        assert_eq!(block["checks"][0]["label"], "command cargo test");
        assert_eq!(block["checks"][1]["excerpt"], "status 500");
        assert!(block["checks"][0]["checked_at"].is_string() && block["checks"][3]["checked_at"].is_null());
        assert_eq!(served(&db, &m.id).await, m.results);
        assert_eq!(m.text, "Failed: Ship it");
    }

    #[tokio::test]
    async fn verification_needs_one_to_twenty_checks_and_a_real_task() {
        let tmp = tempfile::tempdir().unwrap();
        let db = Database::init(tmp.path()).await.unwrap();
        let none = task_with_checks(&db, &[]).await;
        assert_eq!(report(&db, &none, "Verified: Ship it").await.results, None, "no checks, no block");
        let one = task_with_checks(&db, &[("command", json!({"command": "true"}), "passed", None)]).await;
        assert!(report(&db, &one, "Verified: Ship it").await.results.is_some());
        let spec = ("command", json!({"command": "true"}), "passed", None);
        let twenty = task_with_checks(&db, &vec![spec.clone(); 20]).await;
        assert_eq!(report(&db, &twenty, "Verified: x").await.results.unwrap()["blocks"][0]["checks"].as_array().unwrap().len(), 20);
        let many = task_with_checks(&db, &vec![spec; 21]).await;
        assert_eq!(report(&db, &many, "Verified: x").await.results, None, "21 checks is over the limit");
        assert_eq!(report(&db, "no-such-task", "Verified: x").await.results, None);
        // A mission line that isn't a task report, and a person's message, never get one
        let line = db.insert_message_with(&message("kwant", TABLE), &with_meta(json!({"kind": "mission", "mission_id": "m"}))).await.unwrap();
        assert_eq!(line.results, None, "a mission line gets no selector block either");
        let person = db.insert_message_with(&message("user", "x"), &with_meta(json!({"kind": "mission", "task_id": one}))).await.unwrap();
        assert_eq!(person.results, None);
    }

    #[tokio::test]
    async fn hostile_secret_and_long_excerpts_are_made_safe() {
        let tmp = tempfile::tempdir().unwrap();
        let db = Database::init(tmp.path()).await.unwrap();
        let long = "y".repeat(5000);
        let id = task_with_checks(
            &db,
            &[
                ("command", json!({"command": "curl -H 'Authorization: Bearer sk-abcdefghijklmnopqrstuvwxyz012345'"}), "failed", Some("<img src=x onerror=alert(1)> token=ghp_abcdefghijklmnopqrstuvwxyz0123456789")),
                ("command", json!({"command": "true"}), "passed", Some(long.as_str())),
            ],
        )
        .await;
        let m = report(&db, &id, "Failed: Ship it").await;
        let text = m.results.unwrap().to_string();
        assert!(!text.contains("sk-abcdefghijklmnopqrstuvwxyz012345"), "{text}");
        assert!(!text.contains("ghp_abcdefghijklmnopqrstuvwxyz0123456789"), "{text}");
        assert!(!text.contains("<img"), "{text}");
        assert!(!text.contains(&"y".repeat(400)), "an excerpt is cut");
    }

    struct Fixture {
        db: Database,
        _tmp: tempfile::TempDir,
    }

    impl Fixture {
        async fn new() -> Self {
            let tmp = tempfile::tempdir().unwrap();
            let db = Database::init(tmp.path()).await.unwrap();
            Self { db, _tmp: tmp }
        }

        fn blob(&self, bytes: &[u8]) -> String {
            let sha = hex(&Sha256::digest(bytes));
            let dir = self._tmp.path().join("attachments").join(&sha[..2]);
            std::fs::create_dir_all(&dir).unwrap();
            std::fs::write(dir.join(&sha), bytes).unwrap();
            sha
        }

        async fn shared(&self, name: &str, mime: &str, bytes: &[u8], kwant: &str, direct: Option<&str>) -> crate::db::attachments::Attachment {
            let sha = self.blob(bytes);
            self.db
                .insert_attachment(&NewAttachment {
                    sha256: sha,
                    name: name.into(),
                    mime: mime.into(),
                    size: bytes.len() as i64,
                    source: "agent".into(),
                    kwant_id: Some(kwant.into()),
                    context_id: direct.map(Into::into),
                    direct_kwant_id: direct.map(Into::into),
                    ..Default::default()
                })
                .await
                .unwrap()
        }

        /// What `share_file` does: the card is saved first, the attachment linked after.
        async fn card(&self, sender: &str, direct: &str, attachments: &[&crate::db::attachments::Attachment]) -> Message {
            let mut msg = message("kwant", "Shared a file");
            msg.sender_id = sender.into();
            msg.direct_kwant_id = Some(direct.into());
            let cards: Vec<Value> = attachments.iter().map(|a| a.card()).collect();
            self.db.insert_message_with(&msg, &with_meta(json!({"kind": "file", "attachments": cards}))).await.unwrap()
        }
    }

    fn hex(bytes: &[u8]) -> String {
        bytes.iter().map(|b| format!("{b:02x}")).collect()
    }

    #[tokio::test]
    async fn a_shared_csv_gets_a_bounded_table_preview() {
        let f = Fixture::new().await;
        let mut csv = String::from("name,\"note, with comma\",c3,c4,c5,c6,c7,c8,c9\n");
        for i in 0..40 {
            csv.push_str(&format!("r{i},\"say \"\"hi\"\"\nline2\",<b>x</b>,=SUM(A1),4,5,6,7,8\n"));
        }
        let a = f.shared("data.csv", "text/csv", csv.as_bytes(), "k1", Some("k1")).await;
        let m = f.card("k1", "k1", &[&a]).await;
        let block = &m.results.clone().expect("files block")["blocks"][0];
        assert_eq!(block["type"], "files");
        assert_eq!(block["provenance"]["ref"], format!("attachment:{}", a.id));
        let preview = &block["items"][0]["preview"];
        assert_eq!(preview["kind"], "table");
        assert_eq!(preview["columns"].as_array().unwrap().len(), 8, "at most 8 columns");
        assert_eq!(preview["rows"].as_array().unwrap().len(), 20, "at most 20 rows");
        assert_eq!(preview["rows"][0][1], "say \"hi\"\nline2", "quoted fields are read as CSV");
        assert!(!preview.to_string().contains("<b>"), "cells are never markup");
        assert_eq!(block["items"][0]["attachment_id"], a.id);
        assert_eq!(m.text, "Shared a file");
    }

    #[tokio::test]
    async fn text_previews_and_files_without_previews() {
        let f = Fixture::new().await;
        let lines: String = (0..50).map(|i| format!("line {i}\n")).collect();
        let txt = f.shared("notes.txt", "text/plain", lines.as_bytes(), "k1", Some("k1")).await;
        let pdf = f.shared("a.pdf", "application/pdf", b"%PDF-1.4 hi", "k1", Some("k1")).await;
        let bad = f.shared("bad.csv", "text/csv", &[0xff, 0xfe, 0x00, 0xd8, 0x61], "k1", Some("k1")).await;
        let big = f.shared("big.csv", "text/csv", &vec![b'a'; 1_100_000], "k1", Some("k1")).await;
        let m = f.card("k1", "k1", &[&txt, &pdf, &bad, &big]).await;
        let items = m.results.unwrap()["blocks"][0]["items"].clone();
        assert_eq!(items[0]["preview"]["kind"], "text");
        assert_eq!(items[0]["preview"]["excerpt"].as_str().unwrap().lines().count(), 20);
        assert!(items[1]["preview"].is_null(), "a PDF is a file row only");
        assert!(items[2]["preview"].is_null(), "invalid text has no preview");
        assert!(items[3]["preview"].is_null(), "a CSV over 1 MB has no preview");
        assert_eq!(items[1]["mime"], "application/pdf");
    }

    #[tokio::test]
    async fn a_missing_blob_still_lists_the_file() {
        let f = Fixture::new().await;
        let a = f.shared("gone.csv", "text/csv", b"a,b\n1,2\n", "k1", Some("k1")).await;
        std::fs::remove_file(f._tmp.path().join("attachments").join(&a.sha256[..2]).join(&a.sha256)).unwrap();
        let m = f.card("k1", "k1", &[&a]).await;
        assert!(m.results.unwrap()["blocks"][0]["items"][0]["preview"].is_null());
    }

    #[tokio::test]
    async fn only_this_bots_own_shares_in_this_chat_are_listed() {
        let f = Fixture::new().await;
        let mine = f.shared("mine.csv", "text/csv", b"a,b\n1,2\n", "k1", Some("k1")).await;
        let other_chat = f.shared("other.csv", "text/csv", b"a,b\n3,4\n", "k1", Some("k2")).await;
        let other_bot = f.shared("bot.csv", "text/csv", b"a,b\n5,6\n", "k2", Some("k1")).await;
        let mut upload = f.shared("up.csv", "text/csv", b"a,b\n7,8\n", "k1", Some("k1")).await;
        sqlx::query("UPDATE attachments SET source = 'user' WHERE id = ?").bind(&upload.id).execute(&f.db.pool).await.unwrap();
        upload.source = "user".into();
        assert!(f.card("k1", "k1", &[&mine]).await.results.is_some());
        assert_eq!(f.card("k1", "k1", &[&mine, &other_chat]).await.results, None, "another chat's attachment");
        assert_eq!(f.card("k1", "k1", &[&other_bot]).await.results, None, "another bot's attachment");
        assert_eq!(f.card("k1", "k1", &[&upload]).await.results, None, "a person's upload");
        // Already tied to another message
        let tied = f.shared("tied.csv", "text/csv", b"a,b\n9,9\n", "k1", Some("k1")).await;
        sqlx::query("UPDATE attachments SET message_id = 'm0' WHERE id = ?").bind(&tied.id).execute(&f.db.pool).await.unwrap();
        assert_eq!(f.card("k1", "k1", &[&tied]).await.results, None);
    }

    #[tokio::test]
    async fn forged_ids_and_oversized_cards_get_no_block() {
        let f = Fixture::new().await;
        let ghost = crate::db::attachments::Attachment { id: "no-such".into(), ..f.shared("g.csv", "text/csv", b"a\n1\n", "k1", Some("k1")).await };
        assert_eq!(f.card("k1", "k1", &[&ghost]).await.results, None, "nonexistent attachment id");
        let many: Vec<_> = {
            let mut v = Vec::new();
            for i in 0..11 {
                v.push(f.shared(&format!("f{i}.txt"), "text/plain", format!("n{i}").as_bytes(), "k1", Some("k1")).await);
            }
            v
        };
        let refs: Vec<&_> = many.iter().collect();
        assert_eq!(f.card("k1", "k1", &refs).await.results, None, "11 files is over the limit");
        assert!(f.card("k1", "k1", &refs[..10]).await.results.is_some());
        // A traversal-looking hash never reaches the filesystem
        let evil = f.shared("e.txt", "text/plain", b"x", "k1", Some("k1")).await;
        sqlx::query("UPDATE attachments SET sha256 = '../../etc/passwd' WHERE id = ?").bind(&evil.id).execute(&f.db.pool).await.unwrap();
        let m = f.card("k1", "k1", &[&evil]).await;
        assert!(m.results.is_none_or(|r| r["blocks"][0]["items"][0]["preview"].is_null()));
        // A meta that isn't a list, or a bot reply that merely claims kind=file with no attachments
        let odd = db_card(&f, json!({"kind": "file", "attachments": "x"})).await;
        assert_eq!(odd.results, None);
        let none = db_card(&f, json!({"kind": "file", "attachments": []})).await;
        assert_eq!(none.results, None);
    }

    async fn db_card(f: &Fixture, meta: Value) -> Message {
        f.db.insert_message_with(&message("kwant", "Shared a file"), &with_meta(meta)).await.unwrap()
    }

    #[tokio::test]
    async fn an_image_only_share_stays_a_plain_card() {
        let f = Fixture::new().await;
        let png = f.shared("pic.png", "image/png", b"\x89PNG\r\n\x1a\nxx", "k1", Some("k1")).await;
        assert_eq!(f.card("k1", "k1", &[&png]).await.results, None, "the chat already draws it inline");
        let csv = f.shared("t.csv", "text/csv", b"a,b\n1,2\n", "k1", Some("k1")).await;
        let m = f.card("k1", "k1", &[&png, &csv]).await;
        assert_eq!(m.results.unwrap()["blocks"][0]["items"][0]["preview"]["kind"], "image");
    }

    #[tokio::test]
    async fn a_failing_hook_leaves_the_message_as_it_was() {
        let f = Fixture::new().await;
        sqlx::query("DROP TABLE outcome_checks").execute(&f.db.pool).await.unwrap();
        let m = report(&f.db, "t", "Verified: x").await;
        assert_eq!(m.results, None);
        assert_eq!(m.text, "Verified: x");
        assert!(f.db.get_message(&m.id).await.unwrap().is_some());
    }
}
