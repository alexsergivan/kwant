//! Decision notes: how the person decides, so bots can predict it. A note is saved when the person
//! answers a question card with choices (`answered`), presses Change on a choice a bot made
//! (`changed`), and for every choice a bot made itself (`bot`), which gives the rate at which the
//! person keeps the bots' own choices.

use super::spaces::SpaceId;
use super::{Database, DbError};
use chrono::{DateTime, Utc};
use serde::Serialize;
use sqlx::FromRow;
use uuid::Uuid;

/// Where a note comes from.
pub const SOURCES: [&str; 3] = ["answered", "changed", "bot"];

#[derive(Debug, Clone, Serialize, FromRow, PartialEq)]
pub struct DecisionNote {
    pub id: String,
    pub space_id: String,
    pub kwant_id: Option<String>,
    /// The message the choice was made on (the card, or the bot's reply).
    pub message_id: Option<String>,
    pub question: String,
    /// JSON array of the options offered.
    pub options: String,
    /// What was finally picked.
    pub picked: String,
    /// For `changed`: what the bot had picked.
    pub bot_pick: Option<String>,
    pub source: String,
    pub created_at: DateTime<Utc>,
}

impl DecisionNote {
    pub fn option_list(&self) -> Vec<String> {
        serde_json::from_str(&self.options).unwrap_or_default()
    }
}

pub struct NewDecisionNote<'a> {
    pub space: &'a SpaceId,
    pub kwant_id: Option<&'a str>,
    pub message_id: Option<&'a str>,
    pub question: &'a str,
    pub options: &'a [String],
    pub picked: &'a str,
    pub bot_pick: Option<&'a str>,
    pub source: &'a str,
}

/// Longest question or option kept in a note, in characters.
pub const MAX_NOTE_TEXT: usize = 300;

fn clip(text: &str) -> String {
    let t = text.split_whitespace().collect::<Vec<_>>().join(" ");
    t.chars().take(MAX_NOTE_TEXT).collect()
}

impl Database {
    pub(super) async fn migrate_decisions(&self) -> Result<(), DbError> {
        for stmt in [
            "CREATE TABLE IF NOT EXISTS decision_notes (
                id TEXT PRIMARY KEY,
                space_id TEXT NOT NULL,
                kwant_id TEXT,
                message_id TEXT,
                question TEXT NOT NULL,
                options TEXT NOT NULL,
                picked TEXT NOT NULL,
                bot_pick TEXT,
                source TEXT NOT NULL CHECK (source IN ('answered', 'changed', 'bot')),
                created_at TIMESTAMP NOT NULL
            )",
            "CREATE INDEX IF NOT EXISTS decision_notes_space ON decision_notes (space_id, created_at)",
        ] {
            sqlx::query(stmt).execute(&self.pool).await?;
        }
        Ok(())
    }

    pub async fn add_decision_note(&self, n: &NewDecisionNote<'_>) -> Result<DecisionNote, DbError> {
        let options: Vec<String> = n.options.iter().map(|o| clip(o)).collect();
        Ok(sqlx::query_as::<_, DecisionNote>(
            "INSERT INTO decision_notes (id, space_id, kwant_id, message_id, question, options, picked, bot_pick, source, created_at) \
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?) RETURNING *",
        )
        .bind(Uuid::new_v4().to_string())
        .bind(n.space.as_str())
        .bind(n.kwant_id)
        .bind(n.message_id)
        .bind(clip(n.question))
        .bind(serde_json::to_string(&options).unwrap_or_else(|_| "[]".into()))
        .bind(clip(n.picked))
        .bind(n.bot_pick.map(clip))
        .bind(n.source)
        .bind(Utc::now())
        .fetch_one(&self.pool)
        .await?)
    }

    /// The space's newest notes, newest first; `sources` limits them (all when empty).
    pub async fn list_decision_notes(&self, space: &SpaceId, sources: &[&str], limit: i64) -> Result<Vec<DecisionNote>, DbError> {
        let rows = sqlx::query_as::<_, DecisionNote>(
            "SELECT * FROM decision_notes WHERE space_id = ? ORDER BY created_at DESC, rowid DESC LIMIT ?",
        )
        .bind(space.as_str())
        .bind(limit.saturating_mul(4).max(limit))
        .fetch_all(&self.pool)
        .await?;
        Ok(rows.into_iter().filter(|n| sources.is_empty() || sources.contains(&n.source.as_str())).take(limit.max(0) as usize).collect())
    }

    /// Of the bots' own choices in the space (the newest `window`), how many the person kept:
    /// `(kept, made)`. A choice counts as changed when a `changed` note names its message and question.
    pub async fn decision_keep_rate(&self, space: &SpaceId, window: i64) -> Result<(i64, i64), DbError> {
        let (made, changed): (i64, i64) = sqlx::query_as(
            "SELECT COUNT(*), COALESCE(SUM(EXISTS (SELECT 1 FROM decision_notes c WHERE c.source = 'changed' \
                 AND c.space_id = b.space_id AND c.message_id = b.message_id AND c.question = b.question)), 0) \
             FROM (SELECT * FROM decision_notes WHERE space_id = ? AND source = 'bot' ORDER BY created_at DESC LIMIT ?) b",
        )
        .bind(space.as_str())
        .bind(window)
        .fetch_one(&self.pool)
        .await?;
        Ok((made - changed, made))
    }
}
