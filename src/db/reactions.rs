//! 👍/👎 on bot messages (`message_reactions`) and the message a person's message replies to
//! (`messages.reply_to_message_id`). See `docs/reactions-and-replies.md`.

use std::collections::HashMap;

use serde_json::{json, Value};

use super::models::Message;
use super::{Database, DbError};

/// The reactions a message can carry. kwant has one person, so a message has at most one.
pub const REACTIONS: [&str; 2] = ["up", "down"];

/// Longest `reply_to.excerpt`, in characters, the ellipsis included.
pub const EXCERPT_CHARS: usize = 200;

/// What [`Database::set_reaction`] did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReactionChange {
    /// The message had no reaction; now it has one.
    Fresh,
    /// It had the other one.
    Changed,
    /// It already had this one (or had none and still has none).
    Same,
    /// It had one; now it has none.
    Cleared,
}

/// A reaction made since a bot's last reply in a chat, for its next turn there.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewReaction {
    pub message_id: String,
    pub reaction: String,
    pub text: String,
}

impl Database {
    /// Additive and idempotent: the reactions table (a row goes with its message) and the reply
    /// column on messages.
    pub(super) async fn migrate_reactions(&self) -> Result<(), DbError> {
        sqlx::query(
            r#"
            CREATE TABLE IF NOT EXISTS message_reactions (
                message_id TEXT PRIMARY KEY REFERENCES messages(id) ON DELETE CASCADE,
                reaction TEXT NOT NULL CHECK (reaction IN ('up','down')),
                created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
                -- 1 when this reaction answered the message's question with a turn of its own,
                -- so it isn't told to the bot again as feedback
                answered INTEGER NOT NULL DEFAULT 0
            )
            "#,
        )
        .execute(&self.pool)
        .await?;
        let has_reply: bool =
            sqlx::query_scalar("SELECT COUNT(*) > 0 FROM pragma_table_info('messages') WHERE name = 'reply_to_message_id'")
                .fetch_one(&self.pool)
                .await?;
        if !has_reply {
            sqlx::query("ALTER TABLE messages ADD COLUMN reply_to_message_id TEXT").execute(&self.pool).await?;
        }
        // `told`: 1 once a turn's prompt carried the reaction as feedback, so it's told once.
        // Added here for every database, new or from before it
        let has_told: bool =
            sqlx::query_scalar("SELECT COUNT(*) > 0 FROM pragma_table_info('message_reactions') WHERE name = 'told'")
                .fetch_one(&self.pool)
                .await?;
        if !has_told {
            sqlx::query("ALTER TABLE message_reactions ADD COLUMN told INTEGER NOT NULL DEFAULT 0").execute(&self.pool).await?;
        }
        Ok(())
    }

    /// Sets (`Some("up" | "down")`) or clears (`None`) the reaction on `message_id`. A new or
    /// changed reaction gets a fresh `created_at` (so the bot hears about it on its next turn) and
    /// counts as feedback until [`Self::mark_answered`] or [`Self::mark_told`].
    pub async fn set_reaction(&self, message_id: &str, reaction: Option<&str>) -> Result<ReactionChange, DbError> {
        let Some(r) = reaction else {
            let gone = sqlx::query("DELETE FROM message_reactions WHERE message_id = ?").bind(message_id).execute(&self.pool).await?;
            return Ok(if gone.rows_affected() > 0 { ReactionChange::Cleared } else { ReactionChange::Same });
        };
        // One statement each, so two quick clicks can't both count as the first reaction
        let added = sqlx::query("INSERT INTO message_reactions (message_id, reaction) VALUES (?, ?) ON CONFLICT(message_id) DO NOTHING")
            .bind(message_id)
            .bind(r)
            .execute(&self.pool)
            .await?;
        if added.rows_affected() > 0 {
            return Ok(ReactionChange::Fresh);
        }
        let changed = sqlx::query(
            "UPDATE message_reactions SET reaction = ?1, created_at = CURRENT_TIMESTAMP, answered = 0, told = 0 \
             WHERE message_id = ?2 AND reaction <> ?1",
        )
        .bind(r)
        .bind(message_id)
        .execute(&self.pool)
        .await?;
        Ok(if changed.rows_affected() > 0 { ReactionChange::Changed } else { ReactionChange::Same })
    }

    /// The reaction on `message_id` started a turn that answers it: it isn't feedback any more.
    pub async fn mark_answered(&self, message_id: &str) -> Result<(), DbError> {
        sqlx::query("UPDATE message_reactions SET answered = 1 WHERE message_id = ?").bind(message_id).execute(&self.pool).await?;
        Ok(())
    }

    /// `reactions` went into a turn's prompt as feedback: later turns aren't told them again. A
    /// reaction changed meanwhile (to the other one) is news and stays untold.
    pub async fn mark_told(&self, reactions: &[NewReaction]) -> Result<(), DbError> {
        for r in reactions {
            sqlx::query("UPDATE message_reactions SET told = 1 WHERE message_id = ? AND reaction = ?")
                .bind(&r.message_id)
                .bind(&r.reaction)
                .execute(&self.pool)
                .await?;
        }
        Ok(())
    }

    /// Whether nobody (the person or a bot) wrote in `message`'s chat after it; kwant's own
    /// notes (`system`) don't count.
    pub async fn is_newest_said(&self, message: &Message) -> Result<bool, DbError> {
        let Some(chat) = message.thread_id.as_deref().or(message.channel_id.as_deref()).or(message.direct_kwant_id.as_deref()) else {
            return Ok(false);
        };
        let later: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM messages \
             WHERE (channel_id = ?1 OR direct_kwant_id = ?1 OR thread_id = ?1) AND sender_role IN ('user', 'kwant') \
               AND rowid > (SELECT rowid FROM messages WHERE id = ?2)",
        )
        .bind(chat)
        .bind(&message.id)
        .fetch_one(&self.pool)
        .await?;
        Ok(later == 0)
    }

    pub async fn get_reaction(&self, message_id: &str) -> Result<Option<String>, DbError> {
        Ok(sqlx::query_scalar("SELECT reaction FROM message_reactions WHERE message_id = ?")
            .bind(message_id)
            .fetch_optional(&self.pool)
            .await?)
    }

    /// Saves which message `message_id` replies to.
    pub async fn set_reply_to(&self, message_id: &str, reply_to: &str) -> Result<(), DbError> {
        sqlx::query("UPDATE messages SET reply_to_message_id = ? WHERE id = ?")
            .bind(reply_to)
            .bind(message_id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    /// What a chat shows beside each message: its Smart Results block, its reaction and the
    /// message it replies to. Never fails: a read error leaves the fields empty.
    pub async fn attach_chat_fields(&self, messages: &mut [Message]) {
        self.attach_results(messages).await;
        self.attach_reactions(messages).await;
        self.attach_replies(messages).await;
        self.attach_sender_names(messages).await;
    }

    /// Shows a bot's name on its older messages that stored its id as the sender (shared files
    /// before 0.8.0, which the app showed as the chat's title). Nothing is rewritten in the
    /// database; a sender that isn't a bot's id stays as it is.
    pub async fn attach_sender_names(&self, messages: &mut [Message]) {
        let ids: Vec<&str> = messages
            .iter()
            .filter(|m| m.sender_role == "kwant" && m.sender_id.len() == 36 && m.sender_id.bytes().filter(|b| *b == b'-').count() == 4)
            .map(|m| m.sender_id.as_str())
            .collect();
        if ids.is_empty() {
            return;
        }
        let ids = serde_json::to_string(&ids).unwrap_or_default();
        let rows: Vec<(String, String)> =
            match sqlx::query_as("SELECT id, name FROM kwants WHERE id IN (SELECT value FROM json_each(?))")
                .bind(ids)
                .fetch_all(&self.pool)
                .await
            {
                Ok(rows) => rows,
                Err(e) => {
                    tracing::warn!("sender names: couldn't read them: {e}");
                    return;
                }
            };
        let names: HashMap<String, String> = rows.into_iter().filter(|(_, n)| !n.trim().is_empty()).collect();
        for m in messages.iter_mut().filter(|m| m.sender_role == "kwant") {
            if let Some(name) = names.get(&m.sender_id) {
                m.sender_id = name.clone();
            }
        }
    }

    /// Sets `reaction` on the bot messages in `messages` that have one.
    pub async fn attach_reactions(&self, messages: &mut [Message]) {
        let ids: Vec<&str> = messages.iter().filter(|m| m.sender_role == "kwant").map(|m| m.id.as_str()).collect();
        if ids.is_empty() {
            return;
        }
        let ids = serde_json::to_string(&ids).unwrap_or_default();
        let rows: Vec<(String, String)> = match sqlx::query_as(
            "SELECT message_id, reaction FROM message_reactions WHERE message_id IN (SELECT value FROM json_each(?))",
        )
        .bind(ids)
        .fetch_all(&self.pool)
        .await
        {
            Ok(rows) => rows,
            Err(e) => {
                tracing::warn!("reactions: couldn't read them: {e}");
                return;
            }
        };
        let found: HashMap<String, String> = rows.into_iter().collect();
        for m in messages.iter_mut() {
            m.reaction = found.get(&m.id).cloned();
        }
    }

    /// Sets `reply_to` on the messages in `messages` that reply to another one:
    /// `{id, sender_role, sender_id, excerpt}`, or `{id, missing: true}` once that one is deleted.
    pub async fn attach_replies(&self, messages: &mut [Message]) {
        let ids: Vec<&str> = messages.iter().filter_map(|m| m.reply_to_message_id.as_deref()).collect();
        if ids.is_empty() {
            return;
        }
        let ids = serde_json::to_string(&ids).unwrap_or_default();
        let rows: Vec<(String, String, String, String)> = match sqlx::query_as(
            "SELECT id, sender_role, sender_id, text FROM messages WHERE id IN (SELECT value FROM json_each(?))",
        )
        .bind(ids)
        .fetch_all(&self.pool)
        .await
        {
            Ok(rows) => rows,
            Err(e) => {
                tracing::warn!("replies: couldn't read the quoted messages: {e}");
                return;
            }
        };
        let found: HashMap<String, (String, String, String)> =
            rows.into_iter().map(|(id, role, sender, text)| (id, (role, sender, text))).collect();
        for m in messages.iter_mut() {
            let Some(target) = m.reply_to_message_id.as_deref() else { continue };
            m.reply_to = Some(match found.get(target) {
                Some((role, sender, text)) => json!({
                    "id": target,
                    "sender_role": role,
                    "sender_id": sender,
                    "excerpt": plain_excerpt(text, EXCERPT_CHARS),
                }),
                None => json!({"id": target, "missing": true}),
            });
        }
    }

    /// The reactions in chat `context_id` on `bot_name`'s messages made since its newest message
    /// there, oldest first: what the person said about its replies since its last turn. One
    /// already told to a turn ([`Self::mark_told`]) isn't new, even when it was made in the same
    /// second as the bot's reply (timestamps here have 1-second precision).
    pub async fn reactions_since_last_reply(&self, context_id: &str, bot_name: &str) -> Result<Vec<NewReaction>, DbError> {
        let rows: Vec<(String, String, String)> = sqlx::query_as(
            r#"
            WITH chat AS (
                SELECT id, text, created_at FROM messages
                WHERE (channel_id = ?1 OR direct_kwant_id = ?1 OR thread_id = ?1)
                  AND sender_role = 'kwant' AND sender_id = ?2
            ),
            last AS (SELECT MAX(julianday(created_at)) AS at FROM chat)
            SELECT c.id, r.reaction, c.text
            FROM message_reactions r JOIN chat c ON c.id = r.message_id, last
            WHERE julianday(r.created_at) >= last.at AND r.answered = 0 AND r.told = 0
            ORDER BY julianday(r.created_at), r.rowid
            "#,
        )
        .bind(context_id)
        .bind(bot_name)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows.into_iter().map(|(message_id, reaction, text)| NewReaction { message_id, reaction, text }).collect())
    }
}

impl Database {
    /// Marks a turn's history the way the bot should read it: the person's replies start with
    /// `(replying to <name>: "<excerpt>")` and bot messages the person reacted to end with
    /// `(The person reacted 👍 to this message.)`. Only the text sent to the model changes.
    pub async fn mark_history(&self, history: &mut [Message]) {
        if history.is_empty() {
            return;
        }
        self.attach_reactions(history).await;
        self.attach_replies(history).await;
        for m in history.iter_mut() {
            if m.sender_role == "user" {
                if let Some(r) = &m.reply_to {
                    m.text = format!("{} {}", reply_mark(r), m.text);
                }
            } else if m.sender_role == "kwant" {
                if let Some(e) = m.reaction.as_deref().map(emoji) {
                    m.text = format!("{}\n\n(The person reacted {e} to this message.)", m.text.trim_end());
                }
            }
        }
    }
}

/// 👍 for "up", 👎 for "down".
pub fn emoji(reaction: &str) -> &'static str {
    if reaction == "down" { "👎" } else { "👍" }
}

/// Who wrote a quoted message, for the bot reading it: a bot by its name, else the person or kwant.
pub fn author_label(sender_role: &str, sender_id: &str) -> String {
    match sender_role {
        "kwant" => crate::drivers::neutralise_frame_markers(sender_id).replace(['\n', '\r'], " "),
        "user" => "the person".into(),
        _ => "kwant".into(),
    }
}

/// `(replying to <name>: "<excerpt>")` for a message's `reply_to`.
fn reply_mark(reply_to: &Value) -> String {
    if reply_to["missing"] == true {
        return "(replying to a message that was deleted)".into();
    }
    let who = author_label(reply_to["sender_role"].as_str().unwrap_or(""), reply_to["sender_id"].as_str().unwrap_or(""));
    let excerpt = crate::drivers::neutralise_frame_markers(reply_to["excerpt"].as_str().unwrap_or(""));
    format!("(replying to {who}: \"{excerpt}\")")
}

/// `text` as one line of plain text: markdown marks, links and code fences stripped, whitespace
/// collapsed, and at most `max` characters, ending in "…" when cut.
pub fn plain_excerpt(text: &str, max: usize) -> String {
    let mut lines = Vec::new();
    for line in text.lines() {
        let l = line.trim();
        // Fence lines (```rust) and table rules (|---|) carry no words
        if l.starts_with("```") || l.starts_with("~~~") || (l.contains('-') && l.chars().all(|c| matches!(c, '|' | '-' | ':' | ' '))) {
            continue;
        }
        let l = l.trim_start_matches('#').trim_start_matches('>').trim_start();
        let l = strip_list_marker(l);
        lines.push(strip_inline(l));
    }
    let flat = lines.join(" ");
    let flat: String = flat.split_whitespace().collect::<Vec<_>>().join(" ");
    clip(&flat, max)
}

/// Cuts `s` to at most `max` characters, the last one "…" when anything was cut.
pub fn clip(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    let kept: String = s.chars().take(max.saturating_sub(1)).collect();
    format!("{}…", kept.trim_end())
}

fn strip_list_marker(l: &str) -> &str {
    for m in ["- [ ] ", "- [x] ", "- ", "* ", "+ "] {
        if let Some(rest) = l.strip_prefix(m) {
            return rest;
        }
    }
    let digits = l.chars().take_while(|c| c.is_ascii_digit()).count();
    if digits > 0 {
        if let Some(rest) = l[digits..].strip_prefix(". ").or_else(|| l[digits..].strip_prefix(") ")) {
            return rest;
        }
    }
    l
}

/// Drops emphasis, inline code and table pipes, and keeps a link's or image's words.
fn strip_inline(l: &str) -> String {
    let chars: Vec<char> = l.chars().collect();
    let mut out = String::with_capacity(l.len());
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        // [words](url) and ![alt](url): keep the words
        if c == '[' || (c == '!' && chars.get(i + 1) == Some(&'[')) {
            let open = if c == '!' { i + 1 } else { i };
            if let Some(close) = (open + 1..chars.len()).find(|&j| chars[j] == ']') {
                if chars.get(close + 1) == Some(&'(') {
                    if let Some(end) = (close + 2..chars.len()).find(|&j| chars[j] == ')') {
                        out.extend(&chars[open + 1..close]);
                        i = end + 1;
                        continue;
                    }
                }
            }
        }
        match c {
            '*' | '`' | '~' => {}
            // Underscores in snake_case stay; emphasis ones (at a word's edge) go
            '_' => {
                let prev = i.checked_sub(1).map(|p| chars[p]);
                let next = chars.get(i + 1).copied();
                if prev.is_some_and(char::is_alphanumeric) && next.is_some_and(char::is_alphanumeric) {
                    out.push('_');
                }
            }
            '|' => out.push(' '),
            c => out.push(c),
        }
        i += 1;
    }
    out
}

/// `{"message_id","reaction"}`, the reaction route's answer.
pub fn reaction_view(message_id: &str, reaction: Option<&str>) -> Value {
    json!({"message_id": message_id, "reaction": reaction})
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::models::NewMessage;

    async fn db() -> (tempfile::TempDir, Database) {
        let tmp = tempfile::tempdir().unwrap();
        let db = Database::init(tmp.path()).await.unwrap();
        (tmp, db)
    }

    async fn say(db: &Database, chat: &str, role: &str, sender: &str, text: &str) -> Message {
        db.insert_message(&NewMessage {
            channel_id: None,
            direct_kwant_id: Some(chat.into()),
            sender_role: role.into(),
            sender_id: sender.into(),
            text: text.into(),
            reasoning_content: None,
            tool_calls: None,
            tokens_used: 0,
        })
        .await
        .unwrap()
    }

    #[tokio::test]
    async fn an_old_shared_file_shows_the_bots_name_not_its_id() {
        let (_tmp, db) = db().await;
        let space = crate::db::spaces::SpaceId::new(crate::db::spaces::DEFAULT_SPACE_ID);
        let bot = db
            .create_kwant(
                &space,
                &crate::db::models::NewKwant {
                    name: "Vega".into(),
                    title: String::new(),
                    persona_prompt: String::new(),
                    soul: String::new(),
                    avatar_sprite: String::new(),
                    driver_type: "api".into(),
                    preferred_model: "auto".into(),
                },
            )
            .await
            .unwrap();
        let old = say(&db, &bot.id, "kwant", &bot.id, "Shared a file: shot.jpg").await;
        let new = say(&db, &bot.id, "kwant", "Vega", "Done.").await;
        let person = say(&db, &bot.id, "user", &bot.id, "hi").await;
        let stranger = say(&db, &bot.id, "kwant", "00000000-0000-0000-0000-000000000000", "gone").await;
        let mut rows = vec![old, new, person, stranger];
        db.attach_chat_fields(&mut rows).await;
        assert_eq!(rows[0].sender_id, "Vega", "a bot's id as sender reads as its name");
        assert_eq!(rows[1].sender_id, "Vega");
        assert_eq!(rows[2].sender_id, bot.id, "only bot messages are touched");
        assert_eq!(rows[3].sender_id, "00000000-0000-0000-0000-000000000000", "an unknown id stays as it is");
        let stored: Vec<Message> = db.list_messages(&bot.id, 10).await.unwrap();
        assert!(stored.iter().any(|m| m.sender_id == bot.id && m.sender_role == "kwant"), "nothing is rewritten");
    }

    #[test]
    fn excerpts_are_plain_one_line_and_capped() {
        assert_eq!(plain_excerpt("## Plan\n\n- **Ship** the `fix`\n- see [the docs](https://x.y/z)", 200), "Plan Ship the fix see the docs");
        assert_eq!(plain_excerpt("```rust\nlet a_b = 1;\n```\n> _quoted_ ~~old~~", 200), "let a_b = 1; quoted old");
        assert_eq!(plain_excerpt("| a | b |\n|---|---|\n| 1 | 2 |", 200), "a b 1 2");
        assert_eq!(plain_excerpt("![chart](a.png) 1. first", 200), "chart 1. first");
        let long = "word ".repeat(100);
        let cut = plain_excerpt(&long, EXCERPT_CHARS);
        assert!(cut.chars().count() <= EXCERPT_CHARS && cut.ends_with('…'), "{cut}");
        assert_eq!(plain_excerpt("short", 200), "short");
        // Multi-byte characters are counted, never split
        let emoji = "🙂".repeat(300);
        assert_eq!(plain_excerpt(&emoji, 10).chars().count(), 10);
    }

    #[tokio::test]
    async fn the_migration_is_idempotent_and_reactions_go_with_their_message() {
        let tmp = tempfile::tempdir().unwrap();
        let db = Database::init(tmp.path()).await.unwrap();
        let m = say(&db, "k1", "kwant", "Ada", "hello").await;
        db.set_reaction(&m.id, Some("up")).await.unwrap();
        drop(db);
        // Opening again runs every migration again
        let db = Database::init(tmp.path()).await.unwrap();
        db.migrate_reactions().await.unwrap();
        assert_eq!(db.get_reaction(&m.id).await.unwrap().as_deref(), Some("up"));
        // The CHECK refuses anything else, even straight in SQL
        let bad = sqlx::query("INSERT INTO message_reactions (message_id, reaction) VALUES (?, 'love')").bind(&m.id).execute(&db.pool).await;
        assert!(bad.is_err());
        assert_eq!(db.set_reaction(&m.id, Some("down")).await.unwrap(), ReactionChange::Changed);
        assert_eq!(db.set_reaction(&m.id, Some("down")).await.unwrap(), ReactionChange::Same);
        assert_eq!(db.get_reaction(&m.id).await.unwrap().as_deref(), Some("down"));
        assert_eq!(db.set_reaction(&m.id, None).await.unwrap(), ReactionChange::Cleared);
        assert_eq!(db.set_reaction(&m.id, None).await.unwrap(), ReactionChange::Same);
        assert_eq!(db.get_reaction(&m.id).await.unwrap(), None);
        assert_eq!(db.set_reaction(&m.id, Some("up")).await.unwrap(), ReactionChange::Fresh);
        db.set_reaction(&m.id, Some("up")).await.unwrap();
        sqlx::query("DELETE FROM messages WHERE id = ?").bind(&m.id).execute(&db.pool).await.unwrap();
        let left: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM message_reactions").fetch_one(&db.pool).await.unwrap();
        assert_eq!(left, 0, "deleting the message deletes its reaction");
    }

    #[tokio::test]
    async fn lists_carry_the_reaction_and_the_quoted_message_or_that_it_is_gone() {
        let (_tmp, db) = db().await;
        let bot = say(&db, "k1", "kwant", "Ada", "**Deploy** at 9 with [the runbook](https://r)").await;
        let gone = say(&db, "k1", "kwant", "Ada", "old").await;
        let reply = say(&db, "k1", "user", "PlayerHero", "why 9?").await;
        let late = say(&db, "k1", "user", "PlayerHero", "and this?").await;
        db.set_reply_to(&reply.id, &bot.id).await.unwrap();
        db.set_reply_to(&late.id, &gone.id).await.unwrap();
        db.set_reaction(&bot.id, Some("down")).await.unwrap();
        sqlx::query("DELETE FROM messages WHERE id = ?").bind(&gone.id).execute(&db.pool).await.unwrap();
        let mut list = db.list_messages("k1", 100).await.unwrap();
        db.attach_chat_fields(&mut list).await;
        let by_id = |id: &str| list.iter().find(|m| m.id == id).unwrap().clone();
        assert_eq!(by_id(&bot.id).reaction.as_deref(), Some("down"));
        assert_eq!(by_id(&reply.id).reaction, None);
        assert_eq!(
            by_id(&reply.id).reply_to.unwrap(),
            json!({"id": bot.id, "sender_role": "kwant", "sender_id": "Ada", "excerpt": "Deploy at 9 with the runbook"})
        );
        assert_eq!(by_id(&late.id).reply_to.unwrap(), json!({"id": gone.id, "missing": true}));
        // Every message serialises both fields, null when empty
        let v = serde_json::to_value(by_id(&bot.id)).unwrap();
        assert!(v["reply_to"].is_null() && v.as_object().unwrap().contains_key("reply_to"));
        assert_eq!(v["reaction"], "down");
    }

    #[tokio::test]
    async fn only_reactions_since_the_bots_last_reply_in_that_chat_are_new() {
        let (_tmp, db) = db().await;
        let first = say(&db, "k1", "kwant", "Ada", "first answer").await;
        let other = say(&db, "k1", "kwant", "Bob", "Bob's answer").await;
        let elsewhere = say(&db, "k2", "kwant", "Ada", "another chat").await;
        // Older than every reply: made before Ada's last turn there
        db.set_reaction(&first.id, Some("up")).await.unwrap();
        sqlx::query("UPDATE message_reactions SET created_at = '2020-01-01 00:00:00'").execute(&db.pool).await.unwrap();
        assert!(db.reactions_since_last_reply("k1", "Ada").await.unwrap().is_empty());
        db.set_reaction(&first.id, Some("down")).await.unwrap();
        db.set_reaction(&other.id, Some("down")).await.unwrap();
        db.set_reaction(&elsewhere.id, Some("up")).await.unwrap();
        let new = db.reactions_since_last_reply("k1", "Ada").await.unwrap();
        assert_eq!(new, vec![NewReaction { message_id: first.id.clone(), reaction: "down".into(), text: "first answer".into() }]);
        // Ada answered again: that reaction was before her last turn
        let next = say(&db, "k1", "kwant", "Ada", "second answer").await;
        sqlx::query("UPDATE messages SET created_at = datetime('now', '+5 seconds') WHERE id = ?").bind(&next.id).execute(&db.pool).await.unwrap();
        assert!(db.reactions_since_last_reply("k1", "Ada").await.unwrap().is_empty());
        // Setting the same reaction again isn't news
        db.set_reaction(&first.id, Some("down")).await.unwrap();
        assert!(db.reactions_since_last_reply("k1", "Ada").await.unwrap().is_empty());
        // A reaction that answered a question isn't feedback; changing it afterwards is
        let ask = say(&db, "k3", "kwant", "Ada", "Deploy now?").await;
        db.set_reaction(&ask.id, Some("up")).await.unwrap();
        db.mark_answered(&ask.id).await.unwrap();
        assert!(db.reactions_since_last_reply("k3", "Ada").await.unwrap().is_empty());
        db.set_reaction(&ask.id, Some("down")).await.unwrap();
        assert_eq!(db.reactions_since_last_reply("k3", "Ada").await.unwrap().len(), 1);
    }

    #[tokio::test]
    async fn a_reaction_told_once_is_not_new_even_in_the_same_second_as_the_reply() {
        let (_tmp, db) = db().await;
        let m = say(&db, "k1", "kwant", "Ada", "Ship on Friday.").await;
        db.set_reaction(&m.id, Some("down")).await.unwrap();
        let new = db.reactions_since_last_reply("k1", "Ada").await.unwrap();
        assert_eq!(new.len(), 1);
        db.mark_told(&new).await.unwrap();
        // The bot's reply and the reaction in the same second
        let reply = say(&db, "k1", "kwant", "Ada", "Noted.").await;
        let same = "2030-01-01 12:00:00";
        sqlx::query("UPDATE message_reactions SET created_at = ?").bind(same).execute(&db.pool).await.unwrap();
        sqlx::query("UPDATE messages SET created_at = ? WHERE id = ?").bind(same).bind(&reply.id).execute(&db.pool).await.unwrap();
        assert!(db.reactions_since_last_reply("k1", "Ada").await.unwrap().is_empty(), "told once");
        // The same one again isn't news; the other one is
        db.set_reaction(&m.id, Some("down")).await.unwrap();
        assert!(db.reactions_since_last_reply("k1", "Ada").await.unwrap().is_empty());
        db.set_reaction(&m.id, Some("up")).await.unwrap();
        sqlx::query("UPDATE message_reactions SET created_at = ?").bind(same).execute(&db.pool).await.unwrap();
        let changed = db.reactions_since_last_reply("k1", "Ada").await.unwrap();
        assert_eq!(changed.iter().map(|r| r.reaction.as_str()).collect::<Vec<_>>(), ["up"]);
        // Telling a reaction that changed meanwhile leaves the new one untold
        db.mark_told(&new).await.unwrap();
        assert_eq!(db.reactions_since_last_reply("k1", "Ada").await.unwrap().len(), 1);
        // Cleared and set again: news
        db.mark_told(&changed).await.unwrap();
        db.set_reaction(&m.id, None).await.unwrap();
        db.set_reaction(&m.id, Some("up")).await.unwrap();
        sqlx::query("UPDATE message_reactions SET created_at = ?").bind(same).execute(&db.pool).await.unwrap();
        assert_eq!(db.reactions_since_last_reply("k1", "Ada").await.unwrap().len(), 1);
    }

    #[tokio::test]
    async fn a_database_from_before_told_gets_the_column_and_keeps_its_reactions() {
        let tmp = tempfile::tempdir().unwrap();
        let db = Database::init(tmp.path()).await.unwrap();
        let m = say(&db, "k1", "kwant", "Ada", "hello").await;
        db.set_reaction(&m.id, Some("down")).await.unwrap();
        // The table as #296 made it, without `told`
        for sql in [
            "ALTER TABLE message_reactions RENAME TO r_old",
            "CREATE TABLE message_reactions (message_id TEXT PRIMARY KEY REFERENCES messages(id) ON DELETE CASCADE, \
             reaction TEXT NOT NULL CHECK (reaction IN ('up','down')), created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP, \
             answered INTEGER NOT NULL DEFAULT 0)",
            "INSERT INTO message_reactions SELECT message_id, reaction, created_at, answered FROM r_old",
            "DROP TABLE r_old",
        ] {
            sqlx::query(sql).execute(&db.pool).await.unwrap();
        }
        let has_told: bool = sqlx::query_scalar("SELECT COUNT(*) > 0 FROM pragma_table_info('message_reactions') WHERE name = 'told'")
            .fetch_one(&db.pool)
            .await
            .unwrap();
        assert!(!has_told);
        drop(db);
        let db = Database::init(tmp.path()).await.unwrap();
        db.migrate_reactions().await.unwrap();
        assert_eq!(db.get_reaction(&m.id).await.unwrap().as_deref(), Some("down"));
        let new = db.reactions_since_last_reply("k1", "Ada").await.unwrap();
        assert_eq!(new.len(), 1, "an untold reaction stays news");
        db.mark_told(&new).await.unwrap();
        assert!(db.reactions_since_last_reply("k1", "Ada").await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn newest_ignores_kwants_own_notes_but_not_the_person_or_a_bot() {
        let (_tmp, db) = db().await;
        let ask = say(&db, "k1", "kwant", "Ada", "Go?").await;
        assert!(db.is_newest_said(&ask).await.unwrap());
        say(&db, "k1", "system", "kwant", "turn interrupted").await;
        say(&db, "k2", "user", "PlayerHero", "another chat").await;
        assert!(db.is_newest_said(&ask).await.unwrap());
        say(&db, "k1", "user", "PlayerHero", "wait").await;
        assert!(!db.is_newest_said(&ask).await.unwrap());
    }

    #[tokio::test]
    async fn history_marks_replies_and_reactions_for_the_model_only() {
        let (_tmp, db) = db().await;
        let bot = say(&db, "k1", "kwant", "Ada", "Ship it at **9**?").await;
        let reply = say(&db, "k1", "user", "PlayerHero", "why 9").await;
        db.set_reply_to(&reply.id, &bot.id).await.unwrap();
        db.set_reaction(&bot.id, Some("down")).await.unwrap();
        let mut history = db.list_messages("k1", 10).await.unwrap();
        db.mark_history(&mut history).await;
        assert_eq!(history[0].text, "Ship it at **9**?\n\n(The person reacted 👎 to this message.)");
        assert_eq!(history[1].text, "(replying to Ada: \"Ship it at 9?\") why 9");
        // The stored text is untouched
        assert_eq!(db.get_message(&reply.id).await.unwrap().unwrap().text, "why 9");
    }
}
