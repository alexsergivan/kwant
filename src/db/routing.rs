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
}
