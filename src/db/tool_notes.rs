//! P5 (0.9.0, release-090/p5-contract.md): tool notes. A bot writes down a tool problem, a tip
//! or a missing tool once ([`crate::server::tool_notes`]); every bot that has the tool sees the
//! open ones at the start of its turns, and the person edits, fixes, reopens or deletes them.
//! Bots only file and repeat: nothing here lets a bot change a note's text. Add-only.

use super::{Database, DbError};
use sqlx::FromRow;

const TABLES: &[&str] = &[
    "CREATE TABLE IF NOT EXISTS tool_notes (
        id TEXT PRIMARY KEY,
        space_id TEXT NOT NULL,
        tool TEXT NOT NULL,
        tool_key TEXT NOT NULL,
        connection TEXT,
        kind TEXT NOT NULL CHECK (kind IN ('broke', 'tip', 'missing')),
        bot_text TEXT,
        person_text TEXT,
        written_by_person INTEGER NOT NULL DEFAULT 0,
        state TEXT NOT NULL DEFAULT 'open' CHECK (state IN ('open', 'fixed')),
        count INTEGER NOT NULL DEFAULT 1,
        bots TEXT NOT NULL DEFAULT '[]',
        origin TEXT,
        created_at INTEGER NOT NULL,
        last_at INTEGER NOT NULL,
        updated_at INTEGER,
        fixed_at INTEGER
    )",
    "CREATE INDEX IF NOT EXISTS tool_notes_space ON tool_notes (space_id, state, last_at)",
    "CREATE INDEX IF NOT EXISTS tool_notes_key ON tool_notes (space_id, tool_key, kind, state)",
    // One row per file or repeat by a bot: the daily limit and the one-hour "already noted" read it
    "CREATE TABLE IF NOT EXISTS tool_note_filings (
        note_id TEXT NOT NULL,
        kwant_id TEXT NOT NULL,
        is_new INTEGER NOT NULL,
        at INTEGER NOT NULL
    )",
    "CREATE INDEX IF NOT EXISTS tool_note_filings_bot ON tool_note_filings (kwant_id, at)",
];

const COLS: &str = "id, space_id, tool, tool_key, connection, kind, bot_text, person_text, written_by_person, state, count, bots, \
     origin, created_at, last_at, updated_at, fixed_at";

#[derive(Debug, Clone, FromRow, PartialEq)]
pub struct ToolNoteRow {
    pub id: String,
    pub space_id: String,
    /// The display name, `server · tool` or free text (1–80 chars).
    pub tool: String,
    /// `tool` lowercased: the dedupe key with `kind`.
    pub tool_key: String,
    /// The MCP server the note is about, by name; `None` is built-in.
    pub connection: Option<String>,
    pub kind: String,
    /// The first bot's text; `None` for a note the person wrote.
    pub bot_text: Option<String>,
    /// The person's text: once set, it is what bots see.
    pub person_text: Option<String>,
    pub written_by_person: bool,
    pub state: String,
    pub count: i64,
    /// JSON `[{id, name}]`, first filer first.
    pub bots: String,
    /// JSON `{channel_id, direct_kwant_id, thread_id, message_id}` of the latest file or repeat.
    pub origin: Option<String>,
    pub created_at: i64,
    pub last_at: i64,
    pub updated_at: Option<i64>,
    pub fixed_at: Option<i64>,
}

impl ToolNoteRow {
    /// What bots see and the list shows: the person's text, else the bot's.
    pub fn text(&self) -> &str {
        self.person_text.as_deref().or(self.bot_text.as_deref()).unwrap_or("")
    }
}

/// A bot's filing, before dedupe.
pub struct NewFiling<'a> {
    pub space_id: &'a str,
    pub tool: &'a str,
    pub connection: Option<&'a str>,
    pub kind: &'a str,
    pub text: &'a str,
    pub kwant_id: &'a str,
    pub kwant_name: &'a str,
    pub origin: Option<String>,
    pub now: i64,
    /// An open note of the same tool and kind whose `last_at` is at or after this is repeated.
    pub repeat_since: i64,
    /// The same bot filing the same note at or after this changes nothing.
    pub same_bot_since: i64,
    /// The start of the bot's day for the new-note limit, and the limit.
    pub day_start: i64,
    pub daily_new: i64,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Filed {
    New(ToolNoteRow),
    Repeat(ToolNoteRow),
    /// The bot already filed or repeated this note within the hour.
    AlreadyNoted(ToolNoteRow),
    /// The bot filed `daily_new` new notes today.
    Limit,
}

/// The person's changes; `None` leaves a field as it is.
#[derive(Debug, Default, Clone)]
pub struct NoteEdit {
    pub person_text: Option<String>,
    pub kind: Option<String>,
    /// `Some(true)` fixes, `Some(false)` reopens.
    pub fixed: Option<bool>,
}

impl Database {
    pub(super) async fn migrate_tool_notes(&self) -> Result<(), DbError> {
        for sql in TABLES {
            sqlx::query(sql).execute(&self.pool).await?;
        }
        Ok(())
    }

    /// Files a bot's note, or repeats the open one of the same tool and kind (count + 1, the bot
    /// added to `bots`, `last_at` and `origin` moved; text and kind never change). One
    /// transaction; the caller serializes filings so two at once can't both open a note.
    pub async fn file_tool_note(&self, f: &NewFiling<'_>) -> Result<Filed, DbError> {
        let key = f.tool.to_lowercase();
        let mut tx = self.pool.begin().await?;
        let open: Option<ToolNoteRow> = sqlx::query_as(&format!(
            "SELECT {COLS} FROM tool_notes WHERE space_id = ? AND tool_key = ? AND kind = ? AND state = 'open' AND last_at >= ?
             ORDER BY last_at DESC LIMIT 1"
        ))
        .bind(f.space_id)
        .bind(&key)
        .bind(f.kind)
        .bind(f.repeat_since)
        .fetch_optional(&mut *tx)
        .await?;
        if let Some(note) = open {
            let recent: Option<(i64,)> = sqlx::query_as("SELECT at FROM tool_note_filings WHERE note_id = ? AND kwant_id = ? AND at >= ? LIMIT 1")
                .bind(&note.id)
                .bind(f.kwant_id)
                .bind(f.same_bot_since)
                .fetch_optional(&mut *tx)
                .await?;
            if recent.is_some() {
                return Ok(Filed::AlreadyNoted(note));
            }
            let mut bots: Vec<serde_json::Value> = serde_json::from_str(&note.bots).unwrap_or_default();
            if !bots.iter().any(|b| b["id"] == f.kwant_id) {
                bots.push(serde_json::json!({"id": f.kwant_id, "name": f.kwant_name}));
            }
            let bots = serde_json::to_string(&bots).unwrap_or_else(|_| "[]".into());
            let origin = f.origin.clone().or(note.origin.clone());
            sqlx::query("UPDATE tool_notes SET count = count + 1, bots = ?, last_at = ?, origin = ? WHERE id = ?")
                .bind(&bots)
                .bind(f.now)
                .bind(&origin)
                .bind(&note.id)
                .execute(&mut *tx)
                .await?;
            sqlx::query("INSERT INTO tool_note_filings (note_id, kwant_id, is_new, at) VALUES (?, ?, 0, ?)")
                .bind(&note.id)
                .bind(f.kwant_id)
                .bind(f.now)
                .execute(&mut *tx)
                .await?;
            let row: ToolNoteRow = sqlx::query_as(&format!("SELECT {COLS} FROM tool_notes WHERE id = ?")).bind(&note.id).fetch_one(&mut *tx).await?;
            tx.commit().await?;
            return Ok(Filed::Repeat(row));
        }
        let (today,): (i64,) = sqlx::query_as("SELECT COUNT(*) FROM tool_note_filings WHERE kwant_id = ? AND is_new = 1 AND at >= ?")
            .bind(f.kwant_id)
            .bind(f.day_start)
            .fetch_one(&mut *tx)
            .await?;
        if today >= f.daily_new {
            return Ok(Filed::Limit);
        }
        let id = format!("tn_{}", uuid::Uuid::new_v4().simple());
        let bots = serde_json::to_string(&[serde_json::json!({"id": f.kwant_id, "name": f.kwant_name})]).unwrap_or_else(|_| "[]".into());
        sqlx::query(
            "INSERT INTO tool_notes (id, space_id, tool, tool_key, connection, kind, bot_text, bots, origin, created_at, last_at)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(&id)
        .bind(f.space_id)
        .bind(f.tool)
        .bind(&key)
        .bind(f.connection)
        .bind(f.kind)
        .bind(f.text)
        .bind(&bots)
        .bind(&f.origin)
        .bind(f.now)
        .bind(f.now)
        .execute(&mut *tx)
        .await?;
        sqlx::query("INSERT INTO tool_note_filings (note_id, kwant_id, is_new, at) VALUES (?, ?, 1, ?)")
            .bind(&id)
            .bind(f.kwant_id)
            .bind(f.now)
            .execute(&mut *tx)
            .await?;
        // Filings older than two days are only kept for the limit and the hour, which they no longer touch
        sqlx::query("DELETE FROM tool_note_filings WHERE at < ?").bind(f.day_start - 2 * 86_400_000).execute(&mut *tx).await?;
        let row: ToolNoteRow = sqlx::query_as(&format!("SELECT {COLS} FROM tool_notes WHERE id = ?")).bind(&id).fetch_one(&mut *tx).await?;
        tx.commit().await?;
        Ok(Filed::New(row))
    }

    /// A note the person wrote: never deduped.
    pub async fn add_person_tool_note(
        &self,
        space_id: &str,
        tool: &str,
        connection: Option<&str>,
        kind: &str,
        text: &str,
        now: i64,
    ) -> Result<ToolNoteRow, DbError> {
        let id = format!("tn_{}", uuid::Uuid::new_v4().simple());
        sqlx::query(
            "INSERT INTO tool_notes (id, space_id, tool, tool_key, connection, kind, person_text, written_by_person, count, created_at, last_at, updated_at)
             VALUES (?, ?, ?, ?, ?, ?, ?, 1, 1, ?, ?, ?)",
        )
        .bind(&id)
        .bind(space_id)
        .bind(tool)
        .bind(tool.to_lowercase())
        .bind(connection)
        .bind(kind)
        .bind(text)
        .bind(now)
        .bind(now)
        .bind(now)
        .execute(&self.pool)
        .await?;
        Ok(sqlx::query_as(&format!("SELECT {COLS} FROM tool_notes WHERE id = ?")).bind(&id).fetch_one(&self.pool).await?)
    }

    pub async fn tool_note(&self, id: &str) -> Result<Option<ToolNoteRow>, DbError> {
        Ok(sqlx::query_as(&format!("SELECT {COLS} FROM tool_notes WHERE id = ?")).bind(id).fetch_optional(&self.pool).await?)
    }

    /// Every note of the space, newest `last_at` first.
    pub async fn tool_notes_of_space(&self, space_id: &str) -> Result<Vec<ToolNoteRow>, DbError> {
        Ok(sqlx::query_as(&format!("SELECT {COLS} FROM tool_notes WHERE space_id = ? ORDER BY last_at DESC, id"))
            .bind(space_id)
            .fetch_all(&self.pool)
            .await?)
    }

    /// The open notes of the space, newest `last_at` first, at most `limit`.
    pub async fn open_tool_notes(&self, space_id: &str, limit: i64) -> Result<Vec<ToolNoteRow>, DbError> {
        Ok(sqlx::query_as(&format!(
            "SELECT {COLS} FROM tool_notes WHERE space_id = ? AND state = 'open' ORDER BY last_at DESC, id LIMIT ?"
        ))
        .bind(space_id)
        .bind(limit)
        .fetch_all(&self.pool)
        .await?)
    }

    /// The person's edit. `Ok(None)`: no such note.
    pub async fn edit_tool_note(&self, id: &str, edit: &NoteEdit, now: i64) -> Result<Option<ToolNoteRow>, DbError> {
        let mut tx = self.pool.begin().await?;
        let Some(_) = sqlx::query_as::<_, (String,)>("SELECT id FROM tool_notes WHERE id = ?").bind(id).fetch_optional(&mut *tx).await? else {
            return Ok(None);
        };
        if let Some(text) = &edit.person_text {
            sqlx::query("UPDATE tool_notes SET person_text = ?, updated_at = ? WHERE id = ?").bind(text).bind(now).bind(id).execute(&mut *tx).await?;
        }
        if let Some(kind) = &edit.kind {
            sqlx::query("UPDATE tool_notes SET kind = ?, updated_at = ? WHERE id = ?").bind(kind).bind(now).bind(id).execute(&mut *tx).await?;
        }
        match edit.fixed {
            Some(true) => {
                sqlx::query("UPDATE tool_notes SET state = 'fixed', fixed_at = COALESCE(fixed_at, ?) WHERE id = ?")
                    .bind(now)
                    .bind(id)
                    .execute(&mut *tx)
                    .await?;
            }
            // Reopened: it counts as fresh for the 7-day repeat window
            Some(false) => {
                sqlx::query("UPDATE tool_notes SET state = 'open', fixed_at = NULL, last_at = CASE WHEN state = 'fixed' THEN ? ELSE last_at END WHERE id = ?")
                    .bind(now)
                    .bind(id)
                    .execute(&mut *tx)
                    .await?;
            }
            None => {}
        }
        let row = sqlx::query_as(&format!("SELECT {COLS} FROM tool_notes WHERE id = ?")).bind(id).fetch_optional(&mut *tx).await?;
        tx.commit().await?;
        Ok(row)
    }

    /// Where the note's latest file or repeat happened (the chat line's message).
    pub async fn set_tool_note_origin(&self, id: &str, origin: &str) -> Result<(), DbError> {
        sqlx::query("UPDATE tool_notes SET origin = ? WHERE id = ?").bind(origin).bind(id).execute(&self.pool).await?;
        Ok(())
    }

    /// Gone at once. `false`: there was no such note.
    pub async fn delete_tool_note(&self, id: &str) -> Result<bool, DbError> {
        Ok(sqlx::query("DELETE FROM tool_notes WHERE id = ?").bind(id).execute(&self.pool).await?.rows_affected() == 1)
    }

    /// Removes notes for good: fixed before `fixed_before`, and the listed ids (notes of a removed
    /// connection past their 30 days).
    pub async fn purge_tool_notes(&self, space_id: &str, fixed_before: i64, ids: &[String]) -> Result<u64, DbError> {
        let mut n = sqlx::query("DELETE FROM tool_notes WHERE space_id = ? AND state = 'fixed' AND fixed_at < ?")
            .bind(space_id)
            .bind(fixed_before)
            .execute(&self.pool)
            .await?
            .rows_affected();
        for id in ids {
            n += sqlx::query("DELETE FROM tool_notes WHERE space_id = ? AND id = ?").bind(space_id).bind(id).execute(&self.pool).await?.rows_affected();
        }
        Ok(n)
    }
}
