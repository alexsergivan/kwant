//! What channel routing keeps between messages: each bot's covers/avoids lists, the choices the
//! person was asked to make on a close call, and the one hand-back a message may take.

use super::{Database, DbError};
use chrono::{DateTime, Utc};
use serde::Serialize;
use sqlx::FromRow;
use std::collections::HashMap;
use uuid::Uuid;

/// Entries a covers or avoids list keeps.
pub const MAX_SCOPE_ENTRIES: usize = 20;
/// Characters one entry keeps.
pub const MAX_SCOPE_ENTRY_CHARS: usize = 60;
/// Examples of each kind a space keeps; past it, the oldest go.
pub const MAX_ROUTE_EXAMPLES: i64 = 200;
/// Characters of a message an example keeps.
const MAX_EXAMPLE_CHARS: usize = 500;

/// A person's message and the bot that should answer it, or that the Concierge did answer.
#[derive(Debug, Clone, Serialize, FromRow, PartialEq)]
pub struct RouteExample {
    pub id: String,
    pub space_id: String,
    pub message_id: String,
    pub text: String,
    pub kwant_id: String,
    /// "correction" (the person said who should have answered), "ask" (the person chose on a
    /// card) or "concierge" (no specialist fit; counted for hire suggestions).
    pub source: String,
    /// When a hire was suggested from this request; it isn't counted again.
    pub hire_suggested_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
}

/// What routing learns from: corrections and choices the person made.
pub const LEARNED_SOURCES: [&str; 2] = ["correction", "ask"];

/// What a bot is for and not for, in the person's words ("restaurants", "travel"). Empty lists
/// leave routing as it was.
#[derive(Debug, Clone, Default, Serialize, PartialEq)]
pub struct Scope {
    pub covers: Vec<String>,
    pub avoids: Vec<String>,
}

/// Trims, drops blanks and repeats (ignoring case), and caps the list; `Err` names what's wrong.
pub fn clean_scope_list(raw: &[String]) -> Result<Vec<String>, String> {
    let mut out: Vec<String> = Vec::new();
    for e in raw {
        let e = e.trim();
        if e.is_empty() || out.iter().any(|o| o.eq_ignore_ascii_case(e)) {
            continue;
        }
        if e.chars().count() > MAX_SCOPE_ENTRY_CHARS {
            return Err(format!("each entry is at most {MAX_SCOPE_ENTRY_CHARS} characters"));
        }
        out.push(e.to_string());
    }
    if out.len() > MAX_SCOPE_ENTRIES {
        return Err(format!("at most {MAX_SCOPE_ENTRIES} entries"));
    }
    Ok(out)
}

/// A close call the person was asked to settle: the best two bots, or a general answer.
#[derive(Debug, Clone, Serialize, FromRow, PartialEq)]
pub struct RouteChoice {
    pub id: String,
    pub space_id: String,
    pub channel_id: String,
    /// The person's message waiting for the choice.
    pub user_message_id: String,
    /// What the chosen bot is sent: the message with any attachment notes.
    #[serde(skip)]
    pub prompt: String,
    /// The card that asks.
    pub card_message_id: Option<String>,
    /// JSON array of the offered bot ids, best first.
    pub choices: String,
    /// "open" or "resolved".
    pub status: String,
    pub chosen_kwant_id: Option<String>,
    /// True when the person picked a general answer.
    pub general: bool,
    pub created_at: DateTime<Utc>,
    pub resolved_at: Option<DateTime<Utc>>,
}

impl RouteChoice {
    pub fn choice_ids(&self) -> Vec<String> {
        serde_json::from_str(&self.choices).unwrap_or_default()
    }
}

impl Database {
    pub(super) async fn migrate_routing(&self) -> Result<(), DbError> {
        for stmt in [
            "CREATE TABLE IF NOT EXISTS kwant_scope (
                kwant_id TEXT PRIMARY KEY,
                covers TEXT NOT NULL DEFAULT '[]',
                avoids TEXT NOT NULL DEFAULT '[]'
            )",
            "CREATE TABLE IF NOT EXISTS route_choices (
                id TEXT PRIMARY KEY,
                space_id TEXT NOT NULL,
                channel_id TEXT NOT NULL,
                user_message_id TEXT NOT NULL UNIQUE,
                prompt TEXT NOT NULL,
                card_message_id TEXT,
                choices TEXT NOT NULL,
                status TEXT NOT NULL CHECK (status IN ('open', 'resolved')),
                chosen_kwant_id TEXT,
                general INTEGER NOT NULL DEFAULT 0,
                created_at TIMESTAMP NOT NULL,
                resolved_at TIMESTAMP
            )",
            // One hand-back per message, ever: a second "not my area" can't bounce it on
            "CREATE TABLE IF NOT EXISTS route_handbacks (
                id TEXT PRIMARY KEY,
                user_message_id TEXT NOT NULL UNIQUE,
                from_kwant_id TEXT NOT NULL,
                to_kwant_id TEXT NOT NULL,
                reason TEXT NOT NULL,
                created_at TIMESTAMP NOT NULL
            )",
            // What the person taught routing, and what the Concierge answered (for hire suggestions)
            "CREATE TABLE IF NOT EXISTS route_examples (
                id TEXT PRIMARY KEY,
                space_id TEXT NOT NULL,
                message_id TEXT NOT NULL,
                text TEXT NOT NULL,
                kwant_id TEXT NOT NULL,
                source TEXT NOT NULL CHECK (source IN ('correction', 'ask', 'concierge')),
                hire_suggested_at TIMESTAMP,
                created_at TIMESTAMP NOT NULL,
                UNIQUE (message_id, source)
            )",
            "CREATE INDEX IF NOT EXISTS idx_route_examples_space ON route_examples (space_id, source, created_at)",
        ] {
            sqlx::query(stmt).execute(&self.pool).await?;
        }
        Ok(())
    }

    pub async fn get_scope(&self, kwant_id: &str) -> Result<Scope, DbError> {
        Ok(self.scopes_for(&[kwant_id.to_string()]).await?.remove(kwant_id).unwrap_or_default())
    }

    /// The scopes of `kwant_ids` that have one; a bot without a row has empty lists.
    pub async fn scopes_for(&self, kwant_ids: &[String]) -> Result<HashMap<String, Scope>, DbError> {
        let rows: Vec<(String, String, String)> = sqlx::query_as("SELECT kwant_id, covers, avoids FROM kwant_scope").fetch_all(&self.pool).await?;
        Ok(rows
            .into_iter()
            .filter(|(id, _, _)| kwant_ids.contains(id))
            .map(|(id, c, a)| {
                let scope = Scope { covers: serde_json::from_str(&c).unwrap_or_default(), avoids: serde_json::from_str(&a).unwrap_or_default() };
                (id, scope)
            })
            .collect())
    }

    /// Replaces the lists given (`None` keeps that list as it is).
    pub async fn set_scope(&self, kwant_id: &str, covers: Option<&[String]>, avoids: Option<&[String]>) -> Result<Scope, DbError> {
        let mut scope = self.get_scope(kwant_id).await?;
        if let Some(c) = covers {
            scope.covers = c.to_vec();
        }
        if let Some(a) = avoids {
            scope.avoids = a.to_vec();
        }
        sqlx::query("INSERT INTO kwant_scope (kwant_id, covers, avoids) VALUES (?1, ?2, ?3) ON CONFLICT(kwant_id) DO UPDATE SET covers = ?2, avoids = ?3")
            .bind(kwant_id)
            .bind(serde_json::to_string(&scope.covers).unwrap_or_else(|_| "[]".into()))
            .bind(serde_json::to_string(&scope.avoids).unwrap_or_else(|_| "[]".into()))
            .execute(&self.pool)
            .await?;
        Ok(scope)
    }

    pub async fn add_route_choice(&self, space_id: &str, channel_id: &str, user_message_id: &str, prompt: &str, choices: &[String]) -> Result<RouteChoice, DbError> {
        let id = Uuid::new_v4().to_string();
        sqlx::query(
            "INSERT INTO route_choices (id, space_id, channel_id, user_message_id, prompt, choices, status, created_at)
             VALUES (?1, ?2, ?3, ?4, ?7, ?5, 'open', ?6)",
        )
        .bind(&id)
        .bind(space_id)
        .bind(channel_id)
        .bind(user_message_id)
        .bind(serde_json::to_string(choices).unwrap_or_else(|_| "[]".into()))
        .bind(Utc::now())
        .bind(prompt)
        .execute(&self.pool)
        .await?;
        Ok(self.get_route_choice(&id).await?.expect("just inserted"))
    }

    pub async fn set_route_choice_card(&self, id: &str, card_message_id: &str) -> Result<(), DbError> {
        sqlx::query("UPDATE route_choices SET card_message_id = ?2 WHERE id = ?1").bind(id).bind(card_message_id).execute(&self.pool).await?;
        Ok(())
    }

    pub async fn get_route_choice(&self, id: &str) -> Result<Option<RouteChoice>, DbError> {
        Ok(sqlx::query_as("SELECT * FROM route_choices WHERE id = ?1").bind(id).fetch_optional(&self.pool).await?)
    }

    /// Settles an open choice; `false` when it was already settled (a second tap does nothing).
    pub async fn resolve_route_choice(&self, id: &str, kwant_id: &str, general: bool) -> Result<bool, DbError> {
        let done = sqlx::query(
            "UPDATE route_choices SET status = 'resolved', chosen_kwant_id = ?2, general = ?3, resolved_at = ?4
             WHERE id = ?1 AND status = 'open'",
        )
        .bind(id)
        .bind(kwant_id)
        .bind(general)
        .bind(Utc::now())
        .execute(&self.pool)
        .await?;
        Ok(done.rows_affected() == 1)
    }

    /// Records the one hand-back of `user_message_id`; `false` when it already took its hop.
    pub async fn add_route_handback(&self, user_message_id: &str, from: &str, to: &str, reason: &str) -> Result<bool, DbError> {
        let done = sqlx::query(
            "INSERT OR IGNORE INTO route_handbacks (id, user_message_id, from_kwant_id, to_kwant_id, reason, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        )
        .bind(Uuid::new_v4().to_string())
        .bind(user_message_id)
        .bind(from)
        .bind(to)
        .bind(reason)
        .bind(Utc::now())
        .execute(&self.pool)
        .await?;
        Ok(done.rows_affected() == 1)
    }

    /// Records `message_id` → `kwant_id` (a later one for the same message and kind replaces it),
    /// then drops the space's oldest examples of that kind past [`MAX_ROUTE_EXAMPLES`].
    pub async fn add_route_example(&self, space_id: &str, message_id: &str, text: &str, kwant_id: &str, source: &str) -> Result<(), DbError> {
        let text: String = text.chars().take(MAX_EXAMPLE_CHARS).collect();
        sqlx::query(
            "INSERT INTO route_examples (id, space_id, message_id, text, kwant_id, source, created_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
             ON CONFLICT(message_id, source) DO UPDATE SET kwant_id = ?5, text = ?4, created_at = ?7",
        )
        .bind(Uuid::new_v4().to_string())
        .bind(space_id)
        .bind(message_id)
        .bind(&text)
        .bind(kwant_id)
        .bind(source)
        .bind(Utc::now())
        .execute(&self.pool)
        .await?;
        let kinds: &[&str] = if source == "concierge" { &["concierge"] } else { &LEARNED_SOURCES };
        let placeholders = vec!["?"; kinds.len()].join(", ");
        let sql = format!(
            "DELETE FROM route_examples WHERE id IN (SELECT id FROM route_examples WHERE space_id = ? AND source IN ({placeholders})
             ORDER BY created_at DESC LIMIT -1 OFFSET ?)"
        );
        let mut q = sqlx::query(&sql).bind(space_id);
        for k in kinds {
            q = q.bind(*k);
        }
        q.bind(MAX_ROUTE_EXAMPLES).execute(&self.pool).await?;
        Ok(())
    }

    /// The space's corrections and choices, newest first.
    pub async fn learned_examples(&self, space_id: &str) -> Result<Vec<RouteExample>, DbError> {
        Ok(sqlx::query_as("SELECT * FROM route_examples WHERE space_id = ?1 AND source IN ('correction', 'ask') ORDER BY created_at DESC")
            .bind(space_id)
            .fetch_all(&self.pool)
            .await?)
    }

    /// Requests the Concierge answered in the space since `since`, newest first.
    pub async fn concierge_examples(&self, space_id: &str, since: DateTime<Utc>) -> Result<Vec<RouteExample>, DbError> {
        Ok(sqlx::query_as("SELECT * FROM route_examples WHERE space_id = ?1 AND source = 'concierge' AND created_at >= ?2 ORDER BY created_at DESC")
            .bind(space_id)
            .bind(since)
            .fetch_all(&self.pool)
            .await?)
    }

    /// Marks the Concierge requests a hire was suggested from.
    pub async fn mark_hire_suggested(&self, message_ids: &[String]) -> Result<(), DbError> {
        for id in message_ids {
            sqlx::query("UPDATE route_examples SET hire_suggested_at = ?2 WHERE message_id = ?1 AND source = 'concierge'")
                .bind(id)
                .bind(Utc::now())
                .execute(&self.pool)
                .await?;
        }
        Ok(())
    }

    pub async fn handed_back(&self, user_message_id: &str) -> Result<bool, DbError> {
        let row: Option<(String,)> =
            sqlx::query_as("SELECT id FROM route_handbacks WHERE user_message_id = ?1").bind(user_message_id).fetch_optional(&self.pool).await?;
        Ok(row.is_some())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scope_lists_are_cleaned_and_capped() {
        let raw = vec![" food ".into(), "".into(), "Food".into(), "travel".into()];
        assert_eq!(clean_scope_list(&raw).unwrap(), vec!["food".to_string(), "travel".to_string()]);
        assert!(clean_scope_list(&["x".repeat(61)]).is_err());
        let many: Vec<String> = (0..21).map(|i| format!("t{i}")).collect();
        assert!(clean_scope_list(&many).is_err());
    }

    #[tokio::test]
    async fn examples_are_capped_per_space_and_kind() {
        let tmp = tempfile::tempdir().unwrap();
        let db = Database::init(tmp.path()).await.unwrap();
        for i in 0..(MAX_ROUTE_EXAMPLES + 5) {
            db.add_route_example("s1", &format!("m{i}"), "corn dogs", "k1", if i % 2 == 0 { "correction" } else { "ask" }).await.unwrap();
        }
        db.add_route_example("s1", "c1", "corn dogs", "conc", "concierge").await.unwrap();
        db.add_route_example("s2", "o1", "tax", "k2", "correction").await.unwrap();
        let learned = db.learned_examples("s1").await.unwrap();
        assert_eq!(learned.len() as i64, MAX_ROUTE_EXAMPLES);
        assert!(!learned.iter().any(|e| e.message_id == "m0"), "the oldest went");
        assert_eq!(db.concierge_examples("s1", Utc::now() - chrono::Duration::days(14)).await.unwrap().len(), 1);
        assert_eq!(db.learned_examples("s2").await.unwrap().len(), 1);
        // A second correction of the same message replaces the first
        db.add_route_example("s2", "o1", "tax", "k3", "correction").await.unwrap();
        assert_eq!(db.learned_examples("s2").await.unwrap()[0].kwant_id, "k3");
    }
}
