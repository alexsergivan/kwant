//! Delivery notes (P10): how the person liked the way a result was delivered, next to the decision
//! notes. When the person's next message after a bot's reply asks to see what the reply didn't show
//! ("show me", "screenshot", "where is it") or repeats the ask, the reply gets a `miss` note naming
//! what they wanted; when they react well or move on, a `hit`. The delivery planner reads the newest
//! notes so the next plan of that kind shows what they asked for ([`crate::router::analyzer::learn`]).
//!
//! The delivery check after a turn (Vega's `delivery_check`, #469) records `check` notes here too:
//! whether the turn delivered what its plan wanted. The planner learns only from `person` notes.
//!
//! The table is new and additive; one note per reply (`message_id` is unique, a second judgement of
//! the same reply is ignored). A check note's `message_id` is `check:<uuid>`: it is made before
//! the reply is saved.
//!
//! Rollback: the table is read only by this module. Reverting the code leaves it unused;
//! `DROP TABLE delivery_notes` removes it.

use super::spaces::SpaceId;
use super::{Database, DbError};
use crate::router::analyzer::{parse_kind, parse_show, show_str, Note, Outcome};
use crate::router::delivery::Deliverable;
use chrono::{DateTime, Utc};
use serde::Serialize;
use sqlx::FromRow;
use uuid::Uuid;

/// Longest person text kept with a note, in characters.
pub const MAX_PERSON_TEXT: usize = 200;

#[derive(Debug, Clone, Serialize, FromRow, PartialEq)]
pub struct DeliveryNoteRow {
    pub id: String,
    pub space_id: String,
    /// `person` (how the person took the reply) or `check` (the delivery check after the turn).
    pub source: String,
    /// The chat, for a check note (a person note has the reply's `message_id`).
    pub context_id: Option<String>,
    pub kwant_id: Option<String>,
    /// The bot reply the note judges.
    pub message_id: String,
    /// What kind of result the reply was planned as ("ideas", "visual", "change", "explain").
    pub kind: String,
    /// "hit" or "miss".
    pub outcome: String,
    /// For a miss: what the person asked for after the reply ("image", "link"…).
    pub wanted: Option<String>,
    /// JSON array of what the reply showed.
    pub shown: String,
    /// The start of the person's message that decided it.
    pub person_text: String,
    pub created_at: DateTime<Utc>,
}

impl DeliveryNoteRow {
    /// The note for the planner, or `None` for a row it can't read.
    pub fn note(&self) -> Option<Note> {
        let kind = parse_kind(&self.kind).filter(|k| *k != Deliverable::Other)?;
        let outcome = Outcome::parse(&self.outcome)?;
        let shown: Vec<String> = serde_json::from_str(&self.shown).unwrap_or_default();
        Some(Note {
            kind,
            outcome,
            wanted: self.wanted.as_deref().and_then(parse_show),
            shown: shown.iter().filter_map(|s| parse_show(s)).collect(),
        })
    }
}

impl Database {
    pub(super) async fn migrate_delivery_notes(&self) -> Result<(), DbError> {
        for stmt in [
            "CREATE TABLE IF NOT EXISTS delivery_notes (
                id TEXT PRIMARY KEY,
                space_id TEXT NOT NULL,
                source TEXT NOT NULL DEFAULT 'person' CHECK (source IN ('person', 'check')),
                context_id TEXT,
                kwant_id TEXT,
                message_id TEXT NOT NULL UNIQUE,
                kind TEXT NOT NULL,
                outcome TEXT NOT NULL CHECK (outcome IN ('hit', 'miss')),
                wanted TEXT,
                shown TEXT NOT NULL,
                person_text TEXT NOT NULL,
                created_at TIMESTAMP NOT NULL
            )",
            "CREATE INDEX IF NOT EXISTS delivery_notes_space ON delivery_notes (space_id, source, created_at)",
        ] {
            sqlx::query(stmt).execute(&self.pool).await?;
        }
        Ok(())
    }

    /// Saves `note` about the bot reply `message_id`; `false` when that reply already has one.
    pub async fn add_delivery_note(
        &self,
        space: &SpaceId,
        kwant_id: Option<&str>,
        message_id: &str,
        note: &Note,
        person_text: &str,
    ) -> Result<bool, DbError> {
        let kind = serde_json::to_value(note.kind).ok().and_then(|v| v.as_str().map(str::to_string)).unwrap_or_default();
        let shown: Vec<&str> = note.shown.iter().map(|s| show_str(*s)).collect();
        let person: String = person_text.split_whitespace().collect::<Vec<_>>().join(" ").chars().take(MAX_PERSON_TEXT).collect();
        let done = sqlx::query(
            "INSERT OR IGNORE INTO delivery_notes (id, space_id, kwant_id, message_id, kind, outcome, wanted, shown, person_text, created_at) \
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(Uuid::new_v4().to_string())
        .bind(space.as_str())
        .bind(kwant_id)
        .bind(message_id)
        .bind(kind)
        .bind(note.outcome.as_str())
        .bind(note.wanted.map(show_str))
        .bind(serde_json::to_string(&shown).unwrap_or_else(|_| "[]".into()))
        .bind(person)
        .bind(Utc::now())
        .execute(&self.pool)
        .await?;
        Ok(done.rows_affected() == 1)
    }

    /// Saves the delivery check's `note` about a turn of `kwant_id` in chat `context_id`.
    pub async fn add_delivery_check(&self, space: &SpaceId, kwant_id: &str, context_id: &str, note: &Note) -> Result<(), DbError> {
        let kind = serde_json::to_value(note.kind).ok().and_then(|v| v.as_str().map(str::to_string)).unwrap_or_default();
        let shown: Vec<&str> = note.shown.iter().map(|s| show_str(*s)).collect();
        sqlx::query(
            "INSERT INTO delivery_notes (id, space_id, source, context_id, kwant_id, message_id, kind, outcome, wanted, shown, person_text, created_at) \
             VALUES (?, ?, 'check', ?, ?, ?, ?, ?, ?, ?, '', ?)",
        )
        .bind(Uuid::new_v4().to_string())
        .bind(space.as_str())
        .bind(context_id)
        .bind(kwant_id)
        .bind(format!("check:{}", Uuid::new_v4()))
        .bind(kind)
        .bind(note.outcome.as_str())
        .bind(note.wanted.map(show_str))
        .bind(serde_json::to_string(&shown).unwrap_or_else(|_| "[]".into()))
        .bind(Utc::now())
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// The space's newest notes from the person's own reactions, newest first: what the planner
    /// learns from.
    pub async fn list_person_delivery_notes(&self, space: &SpaceId, limit: i64) -> Result<Vec<DeliveryNoteRow>, DbError> {
        Ok(sqlx::query_as::<_, DeliveryNoteRow>(
            "SELECT * FROM delivery_notes WHERE space_id = ? AND source = 'person' ORDER BY created_at DESC, rowid DESC LIMIT ?",
        )
        .bind(space.as_str())
        .bind(limit)
        .fetch_all(&self.pool)
        .await?)
    }

    /// The space's newest delivery notes, newest first.
    pub async fn list_delivery_notes(&self, space: &SpaceId, limit: i64) -> Result<Vec<DeliveryNoteRow>, DbError> {
        Ok(sqlx::query_as::<_, DeliveryNoteRow>(
            "SELECT * FROM delivery_notes WHERE space_id = ? ORDER BY created_at DESC, rowid DESC LIMIT ?",
        )
        .bind(space.as_str())
        .bind(limit)
        .fetch_all(&self.pool)
        .await?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::router::delivery::Show;

    #[tokio::test]
    async fn a_note_round_trips_once_per_reply() {
        let tmp = tempfile::tempdir().unwrap();
        let db = Database::init(tmp.path()).await.unwrap();
        let space = SpaceId::default_space();
        let miss = Note { kind: Deliverable::Explain, outcome: Outcome::Miss, wanted: Some(Show::Image), shown: vec![Show::Text] };
        assert!(db.add_delivery_note(&space, Some("k1"), "m1", &miss, "show me\n please").await.unwrap());
        let hit = Note { kind: Deliverable::Visual, outcome: Outcome::Hit, wanted: None, shown: vec![Show::Image, Show::Text] };
        assert!(!db.add_delivery_note(&space, Some("k1"), "m1", &hit, "again").await.unwrap(), "one note per reply");
        assert!(db.add_delivery_note(&space, None, "m2", &hit, "great").await.unwrap());
        let rows = db.list_delivery_notes(&space, 10).await.unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].message_id, "m2");
        assert_eq!(rows[0].note(), Some(hit));
        assert_eq!(rows[1].note(), Some(miss));
        assert_eq!(rows[1].person_text, "show me please");
        assert_eq!(rows[1].kind, "explain");
        assert!(db.list_delivery_notes(&SpaceId::new("other"), 10).await.unwrap().is_empty());
        assert_eq!(rows[0].source, "person");
    }

    #[tokio::test]
    async fn a_check_note_is_kept_apart_from_the_persons_notes() {
        let tmp = tempfile::tempdir().unwrap();
        let db = Database::init(tmp.path()).await.unwrap();
        let space = SpaceId::default_space();
        let person = Note { kind: Deliverable::Explain, outcome: Outcome::Miss, wanted: Some(Show::Image), shown: vec![Show::Text] };
        db.add_delivery_note(&space, Some("k1"), "m1", &person, "show me").await.unwrap();
        let check = Note { kind: Deliverable::Visual, outcome: Outcome::Miss, wanted: Some(Show::Image), shown: vec![] };
        db.add_delivery_check(&space, "k1", "chat-1", &check).await.unwrap();
        db.add_delivery_check(&space, "k1", "chat-1", &check).await.unwrap();
        let all = db.list_delivery_notes(&space, 10).await.unwrap();
        assert_eq!(all.len(), 3, "every check is kept");
        let c = all.iter().find(|r| r.source == "check").unwrap();
        assert!(c.message_id.starts_with("check:"));
        assert_eq!((c.context_id.as_deref(), c.kind.as_str(), c.outcome.as_str(), c.wanted.as_deref()), (Some("chat-1"), "visual", "miss", Some("image")));
        assert_eq!(c.note(), Some(check));
        let learned = db.list_person_delivery_notes(&space, 10).await.unwrap();
        assert_eq!(learned.len(), 1);
        assert_eq!(learned[0].message_id, "m1");
    }
}
