//! Agent continuity: pinned team/channel instructions, full-text search over messages, per-kwant
//! memory, and routines with their run history.

use super::spaces::SpaceId;
use super::{Database, DbError};
use chrono::{DateTime, Utc};
use serde::Serialize;
use sqlx::{FromRow, QueryBuilder, Sqlite};
use uuid::Uuid;

/// Longest pinned instructions for a team or channel.
pub const MAX_INSTRUCTIONS_CHARS: usize = 4000;
/// Longest single memory.
pub const MAX_MEMORY_CHARS: usize = 500;
/// Most memories one kwant may keep.
pub const MAX_MEMORIES_PER_KWANT: i64 = 100;
/// Most pinned memories per kwant. Pinned notes always reach the prompt, so ten notes of at most
/// [`MAX_MEMORY_CHARS`] keep them inside the prompt's memory budget for ordinary text.
pub const MAX_PINNED_PER_KWANT: i64 = 10;
/// Most results from one memory search.
pub const MAX_MEMORY_SEARCH_LIMIT: i64 = 20;
/// Columns a [`Memory`] row is read with.
const MEMORY_COLUMNS: &str = "id, kwant_id, text, source, pinned_by, imported, created_at, updated_at";
/// Undoes the pinning and memory-search migration (see [`Database::migrate_continuity`]): the
/// table goes back to the shape older releases read. Kept for rollbacks and tested. The Truth
/// Ledger's mirror reads the dropped columns, so it goes first; the next start brings it back.
pub const MEMORY_PINS_DOWN: &str = "DROP TRIGGER IF EXISTS kwant_memories_ledger_ai; \
    DROP TRIGGER IF EXISTS kwant_memories_ledger_au; \
    DROP TRIGGER IF EXISTS kwant_memories_ledger_pin; \
    DROP TRIGGER IF EXISTS kwant_memories_ledger_ad; \
    DROP TRIGGER IF EXISTS kwant_memories_fts_ai; \
    DROP TRIGGER IF EXISTS kwant_memories_fts_ad; \
    DROP TRIGGER IF EXISTS kwant_memories_fts_au; \
    DROP TABLE IF EXISTS kwant_memories_fts; \
    ALTER TABLE kwant_memories DROP COLUMN pinned_by; \
    ALTER TABLE kwant_memories DROP COLUMN imported";
/// Most search results per request.
pub const MAX_SEARCH_LIMIT: i64 = 50;
/// Most runs kept per routine; older ones are pruned when a run starts.
pub const MAX_RUNS_KEPT: i64 = 200;

#[derive(Debug, Clone, Serialize, FromRow, PartialEq)]
pub struct Memory {
    pub id: String,
    pub kwant_id: String,
    pub text: String,
    pub source: String,
    /// Who pinned it ('user' or 'agent'); `None` when it isn't pinned. Pinned notes always load first.
    pub pinned_by: Option<String>,
    /// Came in with an import (the person's, `source = 'user'`, but not words they wrote here).
    /// Cleared when the person edits it.
    pub imported: bool,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl Memory {
    pub fn pinned(&self) -> bool {
        self.pinned_by.is_some()
    }
}

/// A pin change: who asks, and whether it should be pinned afterwards.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PinChange {
    pub pinned: bool,
    pub by_person: bool,
}

/// A chat turn in progress: where its reply would go.
#[derive(Debug, Clone, FromRow, PartialEq)]
pub struct RunningTurn {
    pub turn_id: String,
    pub kwant_id: String,
    pub kwant_name: String,
    pub channel_id: Option<String>,
    pub direct_kwant_id: Option<String>,
    pub thread_id: Option<String>,
    pub started_at: DateTime<Utc>,
}

#[derive(Debug, PartialEq)]
pub enum MemoryError {
    Empty,
    TooLong,
    TooMany,
    TooManyPinned,
    /// A bot tried to unpin a note the person pinned
    PersonPinned,
    NotFound,
    /// A bot tried to change a memory the person wrote
    PersonWritten,
    Db(String),
}

impl From<DbError> for MemoryError {
    fn from(e: DbError) -> Self {
        MemoryError::Db(e.to_string())
    }
}

impl From<sqlx::Error> for MemoryError {
    fn from(e: sqlx::Error) -> Self {
        MemoryError::Db(e.to_string())
    }
}

impl std::fmt::Display for MemoryError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            MemoryError::Empty => write!(f, "memory text is empty"),
            MemoryError::TooLong => write!(
                f,
                "memory text is longer than {MAX_MEMORY_CHARS} characters"
            ),
            MemoryError::TooMany => write!(
                f,
                "this bot already has {MAX_MEMORIES_PER_KWANT} memories; remove one first"
            ),
            MemoryError::TooManyPinned => write!(
                f,
                "{MAX_PINNED_PER_KWANT} notes are already pinned; unpin one first"
            ),
            MemoryError::PersonPinned => write!(
                f,
                "the person pinned this note, so only they can unpin it"
            ),
            MemoryError::NotFound => write!(f, "no such memory"),
            MemoryError::PersonWritten => write!(
                f,
                "the person wrote this memory, so only they can change or remove it. \
                 Ask them in your reply if you think it should change"
            ),
            MemoryError::Db(_) => write!(f, "database error"),
        }
    }
}

/// Trims a memory and checks its length. Line breaks become spaces, so one memory is always one
/// line in the prompt and can't pose as a new section.
pub fn clean_memory_text(text: &str) -> Result<String, MemoryError> {
    let text = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if text.is_empty() {
        return Err(MemoryError::Empty);
    }
    if text.chars().count() > MAX_MEMORY_CHARS {
        return Err(MemoryError::TooLong);
    }
    Ok(text)
}

/// One search hit. `snippet` is raw message text with `<mark>…</mark>` around matches: callers
/// rendering HTML must escape it and allow only `<mark>`.
#[derive(Debug, Clone, Serialize, FromRow, PartialEq)]
pub struct SearchHit {
    pub message_id: String,
    pub channel_id: Option<String>,
    pub channel_name: Option<String>,
    pub kwant_id: Option<String>,
    pub author: String,
    pub snippet: String,
    pub created_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Default)]
pub struct SearchFilter {
    pub channel_id: Option<String>,
    /// Messages in this kwant's direct chat or written by it.
    pub kwant_id: Option<String>,
    /// When set, only these channels and `allowed_direct`'s direct chat are searched (agent access).
    pub allowed_channels: Option<Vec<String>>,
    pub allowed_direct: Option<String>,
    pub limit: i64,
}

/// Turns free text into an FTS5 query: every word must match, as a prefix. Quotes keep FTS
/// syntax (`OR`, `NEAR`, `*`, `:`) in user input from being interpreted. `None` if no words.
pub fn fts_query(q: &str) -> Option<String> {
    let terms: Vec<String> = q
        .split(|c: char| !c.is_alphanumeric() && c != '_')
        .filter(|t| !t.is_empty())
        .take(16)
        .map(|t| format!("\"{t}\"*"))
        .collect();
    (!terms.is_empty()).then(|| terms.join(" "))
}

#[derive(Debug, Clone, FromRow)]
pub struct RoutineRow {
    pub id: String,
    pub kwant_id: String,
    pub channel_id: Option<String>,
    pub name: String,
    pub instructions: String,
    pub schedule: String,
    pub enabled: bool,
    pub next_run_at: Option<DateTime<Utc>>,
    pub last_run_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    pub last_status: Option<String>,
    /// Its kwant's space; the channel, when set, is in the same one.
    pub space_id: String,
    /// Delivers into this thread of its kwant (then `channel_id` is empty).
    pub thread_id: Option<String>,
    /// A message in the delivery chat the routine follows up on; its run gets that exchange.
    pub context_message_id: Option<String>,
    /// `discovery` when Automation Discovery suggested it and the person accepted; else empty.
    pub origin: Option<String>,
    pub suggestion_id: Option<String>,
    /// What each run's board task carries: its Flight School class, a good result, and its checks
    /// (JSON `[{kind, spec, required}]`, set by the person only). Empty: as before, the
    /// instructions are the acceptance and the task has no checks.
    pub task_class: Option<String>,
    pub acceptance: Option<String>,
    pub checks: Option<String>,
}

#[derive(Debug, Clone, Serialize, FromRow, PartialEq)]
pub struct RoutineRun {
    pub id: String,
    pub routine_id: String,
    pub trigger: String,
    pub started_at: DateTime<Utc>,
    pub ended_at: Option<DateTime<Utc>>,
    pub status: String,
    pub error: Option<String>,
    pub message_id: Option<String>,
}

#[derive(Debug, Clone, Default)]
pub struct NewRoutine {
    pub kwant_id: String,
    pub channel_id: Option<String>,
    pub name: String,
    pub instructions: String,
    /// Schedule JSON, already validated.
    pub schedule: String,
    pub enabled: bool,
    pub next_run_at: Option<DateTime<Utc>>,
    pub thread_id: Option<String>,
    pub context_message_id: Option<String>,
    /// Kept through edits; only Automation Discovery's accept sets them.
    pub origin: Option<String>,
    pub suggestion_id: Option<String>,
    pub task_class: Option<String>,
    pub acceptance: Option<String>,
    /// Checks JSON, already validated.
    pub checks: Option<String>,
}

const ROUTINE_SELECT: &str = "SELECT r.id, r.kwant_id, r.channel_id, r.name, r.instructions, r.schedule, r.enabled, \
     r.next_run_at, r.last_run_at, r.created_at, r.space_id, r.thread_id, r.context_message_id, \
     r.origin, r.suggestion_id, r.task_class, r.acceptance, r.checks, \
     (SELECT status FROM routine_runs x WHERE x.routine_id = r.id ORDER BY x.started_at DESC, x.rowid DESC LIMIT 1) AS last_status \
     FROM routines r";

impl Database {
    /// Additive: instructions columns and the message search index (filled from existing messages
    /// the first time). Old binaries ignore all of it; the triggers keep the index current for them too.
    pub(super) async fn migrate_continuity(&self) -> Result<(), DbError> {
        let _ = sqlx::query("ALTER TABLE teams ADD COLUMN instructions TEXT;")
            .execute(&self.pool)
            .await;
        let _ = sqlx::query("ALTER TABLE channels ADD COLUMN instructions TEXT;")
            .execute(&self.pool)
            .await;
        // Follow-ups: a routine can deliver into a thread and carry the exchange it follows up on
        for column in ["thread_id", "context_message_id"] {
            let _ = sqlx::query(&format!("ALTER TABLE routines ADD COLUMN {column} TEXT;"))
                .execute(&self.pool)
                .await;
        }

        // Chat turns in progress, so a restart can say which replies it cut off
        sqlx::query(
            "CREATE TABLE IF NOT EXISTS running_turns (\
             turn_id TEXT PRIMARY KEY, kwant_id TEXT NOT NULL, kwant_name TEXT NOT NULL, \
             channel_id TEXT, direct_kwant_id TEXT, thread_id TEXT, started_at DATETIME NOT NULL)",
        )
        .execute(&self.pool)
        .await?;

        let had_index: Option<(String,)> = sqlx::query_as(
            "SELECT name FROM sqlite_master WHERE type = 'table' AND name = 'messages_fts'",
        )
        .fetch_optional(&self.pool)
        .await?;
        sqlx::query(
            "CREATE VIRTUAL TABLE IF NOT EXISTS messages_fts USING fts5(\
             text, content='messages', content_rowid='rowid', tokenize='unicode61 remove_diacritics 2')",
        )
        .execute(&self.pool)
        .await?;
        for trigger in [
            "CREATE TRIGGER IF NOT EXISTS messages_fts_ai AFTER INSERT ON messages BEGIN \
             INSERT INTO messages_fts(rowid, text) VALUES (new.rowid, new.text); END",
            "CREATE TRIGGER IF NOT EXISTS messages_fts_ad AFTER DELETE ON messages BEGIN \
             INSERT INTO messages_fts(messages_fts, rowid, text) VALUES ('delete', old.rowid, old.text); END",
            "CREATE TRIGGER IF NOT EXISTS messages_fts_au AFTER UPDATE OF text ON messages BEGIN \
             INSERT INTO messages_fts(messages_fts, rowid, text) VALUES ('delete', old.rowid, old.text); \
             INSERT INTO messages_fts(rowid, text) VALUES (new.rowid, new.text); END",
        ] {
            sqlx::query(trigger).execute(&self.pool).await?;
        }
        if had_index.is_none() {
            sqlx::query("INSERT INTO messages_fts(messages_fts) VALUES ('rebuild')")
                .execute(&self.pool)
                .await?;
        }
        self.migrate_memory_pins().await
    }

    /// Additive and reversible ([`MEMORY_PINS_DOWN`]): `pinned_by` and `imported` columns and a search index
    /// over memories, filled from existing notes the first time. Older binaries ignore both, and
    /// the triggers keep the index current for their writes too.
    async fn migrate_memory_pins(&self) -> Result<(), DbError> {
        let _ = sqlx::query(
            "ALTER TABLE kwant_memories ADD COLUMN pinned_by TEXT CHECK (pinned_by IN ('user', 'agent'))",
        )
        .execute(&self.pool)
        .await;
        let _ = sqlx::query("ALTER TABLE kwant_memories ADD COLUMN imported INTEGER NOT NULL DEFAULT 0")
            .execute(&self.pool)
            .await;
        let had_index: Option<(String,)> = sqlx::query_as(
            "SELECT name FROM sqlite_master WHERE type = 'table' AND name = 'kwant_memories_fts'",
        )
        .fetch_optional(&self.pool)
        .await?;
        sqlx::query(
            "CREATE VIRTUAL TABLE IF NOT EXISTS kwant_memories_fts USING fts5(\
             text, content='kwant_memories', content_rowid='rowid', tokenize='unicode61 remove_diacritics 2')",
        )
        .execute(&self.pool)
        .await?;
        for trigger in [
            "CREATE TRIGGER IF NOT EXISTS kwant_memories_fts_ai AFTER INSERT ON kwant_memories BEGIN \
             INSERT INTO kwant_memories_fts(rowid, text) VALUES (new.rowid, new.text); END",
            "CREATE TRIGGER IF NOT EXISTS kwant_memories_fts_ad AFTER DELETE ON kwant_memories BEGIN \
             INSERT INTO kwant_memories_fts(kwant_memories_fts, rowid, text) VALUES ('delete', old.rowid, old.text); END",
            "CREATE TRIGGER IF NOT EXISTS kwant_memories_fts_au AFTER UPDATE OF text ON kwant_memories BEGIN \
             INSERT INTO kwant_memories_fts(kwant_memories_fts, rowid, text) VALUES ('delete', old.rowid, old.text); \
             INSERT INTO kwant_memories_fts(rowid, text) VALUES (new.rowid, new.text); END",
        ] {
            sqlx::query(trigger).execute(&self.pool).await?;
        }
        if had_index.is_none() {
            sqlx::query("INSERT INTO kwant_memories_fts(kwant_memories_fts) VALUES ('rebuild')")
                .execute(&self.pool)
                .await?;
        }
        Ok(())
    }

    // Pinned instructions

    pub async fn team_instructions(&self, team_id: &str) -> Result<Option<String>, DbError> {
        let row: Option<(Option<String>,)> =
            sqlx::query_as("SELECT instructions FROM teams WHERE id = ?")
                .bind(team_id)
                .fetch_optional(&self.pool)
                .await?;
        Ok(row.and_then(|r| r.0).filter(|s| !s.trim().is_empty()))
    }

    pub async fn channel_instructions(&self, channel_id: &str) -> Result<Option<String>, DbError> {
        let row: Option<(Option<String>,)> =
            sqlx::query_as("SELECT instructions FROM channels WHERE id = ?")
                .bind(channel_id)
                .fetch_optional(&self.pool)
                .await?;
        Ok(row.and_then(|r| r.0).filter(|s| !s.trim().is_empty()))
    }

    /// `""` clears. Returns false for an unknown team. Callers check the length.
    pub async fn set_team_instructions(&self, team_id: &str, text: &str) -> Result<bool, DbError> {
        let value = Some(text.trim()).filter(|t| !t.is_empty());
        let res = sqlx::query("UPDATE teams SET instructions = ? WHERE id = ?")
            .bind(value)
            .bind(team_id)
            .execute(&self.pool)
            .await?;
        Ok(res.rows_affected() > 0)
    }

    pub async fn set_channel_instructions(
        &self,
        channel_id: &str,
        text: &str,
    ) -> Result<bool, DbError> {
        let value = Some(text.trim()).filter(|t| !t.is_empty());
        let res = sqlx::query("UPDATE channels SET instructions = ? WHERE id = ?")
            .bind(value)
            .bind(channel_id)
            .execute(&self.pool)
            .await?;
        Ok(res.rows_affected() > 0)
    }

    // Search

    /// Best matches first (bm25), newer first among equals, from the space's channels, direct chats
    /// and threads only. An empty result for a query with no words.
    pub async fn search_messages(
        &self,
        space: &SpaceId,
        q: &str,
        filter: &SearchFilter,
    ) -> Result<Vec<SearchHit>, DbError> {
        let Some(query) = fts_query(q) else {
            return Ok(Vec::new());
        };
        let mut qb: QueryBuilder<Sqlite> = QueryBuilder::new(
            "SELECT m.id AS message_id, m.channel_id, c.name AS channel_name, \
             COALESCE(m.direct_kwant_id, k.id) AS kwant_id, m.sender_id AS author, \
             snippet(messages_fts, 0, '<mark>', '</mark>', '…', 16) AS snippet, m.created_at \
             FROM messages_fts JOIN messages m ON m.rowid = messages_fts.rowid \
             LEFT JOIN channels c ON c.id = m.channel_id \
             LEFT JOIN kwants dk ON dk.id = m.direct_kwant_id \
             LEFT JOIN threads t ON t.id = m.thread_id \
             LEFT JOIN kwants tk ON tk.id = t.kwant_id \
             LEFT JOIN kwants k ON m.sender_role = 'kwant' AND k.name = m.sender_id \
               AND k.space_id = COALESCE(c.space_id, dk.space_id, tk.space_id) \
             WHERE messages_fts MATCH ",
        );
        qb.push_bind(query);
        qb.push(" AND COALESCE(c.space_id, dk.space_id, tk.space_id) = ")
            .push_bind(space.as_str().to_string());
        if let Some(channel) = &filter.channel_id {
            qb.push(" AND m.channel_id = ").push_bind(channel.clone());
        }
        if let Some(kwant) = &filter.kwant_id {
            qb.push(" AND (m.direct_kwant_id = ")
                .push_bind(kwant.clone());
            qb.push(" OR k.id = ").push_bind(kwant.clone()).push(")");
        }
        if let Some(allowed) = &filter.allowed_channels {
            qb.push(" AND (");
            if allowed.is_empty() {
                qb.push("0");
            } else {
                qb.push("m.channel_id IN (");
                let mut list = qb.separated(", ");
                for id in allowed {
                    list.push_bind(id.clone());
                }
                list.push_unseparated(")");
            }
            if let Some(direct) = &filter.allowed_direct {
                qb.push(" OR (m.channel_id IS NULL AND m.direct_kwant_id = ")
                    .push_bind(direct.clone())
                    .push(")");
            }
            qb.push(")");
        }
        qb.push(" ORDER BY bm25(messages_fts), m.created_at DESC, m.rowid DESC LIMIT ");
        qb.push_bind(filter.limit.clamp(1, MAX_SEARCH_LIMIT));
        Ok(qb
            .build_query_as::<SearchHit>()
            .fetch_all(&self.pool)
            .await?)
    }

    // Memory

    /// Oldest first.
    pub async fn list_memories(&self, kwant_id: &str) -> Result<Vec<Memory>, DbError> {
        Ok(sqlx::query_as::<_, Memory>(&format!(
            "SELECT {MEMORY_COLUMNS} FROM kwant_memories WHERE kwant_id = ? ORDER BY created_at, rowid"
        ))
        .bind(kwant_id)
        .fetch_all(&self.pool)
        .await?)
    }

    pub async fn add_memory(
        &self,
        kwant_id: &str,
        text: &str,
        source: &str,
    ) -> Result<Memory, MemoryError> {
        self.add_memory_pinned(kwant_id, text, source, false).await
    }

    /// [`Self::add_memory`], pinned by its writer when `pinned`.
    pub async fn add_memory_pinned(
        &self,
        kwant_id: &str,
        text: &str,
        source: &str,
        pinned: bool,
    ) -> Result<Memory, MemoryError> {
        self.add_memory_from(kwant_id, text, source, pinned, None).await
    }

    /// [`Self::add_memory_pinned`] by a bot in turn `outside_turn`, which read outside text: the
    /// Truth Ledger mirrors the note as `outside`/`proposed` (contract §2.4).
    pub async fn add_memory_from(
        &self,
        kwant_id: &str,
        text: &str,
        source: &str,
        pinned: bool,
        outside_turn: Option<&str>,
    ) -> Result<Memory, MemoryError> {
        let text = clean_memory_text(text)?;
        let id = Uuid::new_v4().to_string();
        let now = Utc::now();
        let mut tx = self.pool.begin().await?;
        let (count,): (i64,) =
            sqlx::query_as("SELECT COUNT(*) FROM kwant_memories WHERE kwant_id = ?")
                .bind(kwant_id)
                .fetch_one(&mut *tx)
                .await?;
        if count >= MAX_MEMORIES_PER_KWANT {
            return Err(MemoryError::TooMany);
        }
        let pinned_by = pinned.then(|| source.to_string());
        if pinned && pinned_count(&mut tx, kwant_id).await? >= MAX_PINNED_PER_KWANT {
            return Err(MemoryError::TooManyPinned);
        }
        if let Some(turn_id) = outside_turn {
            super::truth_ledger::note_provenance(&mut tx, &id, super::truth_ledger::Provenance::Outside { turn_id }).await?;
        }
        sqlx::query("INSERT INTO kwant_memories (id, kwant_id, text, source, pinned_by, created_at, updated_at) VALUES (?, ?, ?, ?, ?, ?, ?)")
            .bind(&id)
            .bind(kwant_id)
            .bind(&text)
            .bind(source)
            .bind(&pinned_by)
            .bind(now)
            .bind(now)
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        Ok(Memory {
            id,
            kwant_id: kwant_id.into(),
            text,
            source: source.into(),
            pinned_by,
            imported: false,
            created_at: now,
            updated_at: now,
        })
    }

    /// The person editing a memory. Only the kwant's own memory can be changed, and the edited
    /// entry becomes the person's (`source = 'user'`), so the bot can no longer change or remove it.
    pub async fn update_memory(
        &self,
        kwant_id: &str,
        id: &str,
        text: &str,
    ) -> Result<Memory, MemoryError> {
        let text = clean_memory_text(text)?;
        let res = sqlx::query(
            "UPDATE kwant_memories SET text = ?, source = 'user', imported = 0, updated_at = ? WHERE id = ? AND kwant_id = ?",
        )
        .bind(&text)
        .bind(Utc::now())
        .bind(id)
        .bind(kwant_id)
        .execute(&self.pool)
        .await?;
        if res.rows_affected() == 0 {
            return Err(MemoryError::NotFound);
        }
        self.get_memory(id).await
    }

    /// A bot changing its own memory: like [`Self::update_memory`], but entries the person wrote
    /// are refused.
    pub async fn agent_update_memory(
        &self,
        kwant_id: &str,
        id: &str,
        text: &str,
    ) -> Result<Memory, MemoryError> {
        self.agent_update_memory_from(kwant_id, id, text, None).await
    }

    /// [`Self::agent_update_memory`] in turn `outside_turn`, which read outside text (see
    /// [`Self::add_memory_from`]).
    pub async fn agent_update_memory_from(
        &self,
        kwant_id: &str,
        id: &str,
        text: &str,
        outside_turn: Option<&str>,
    ) -> Result<Memory, MemoryError> {
        let text = clean_memory_text(text)?;
        let mut tx = self.pool.begin().await?;
        if let Some(turn_id) = outside_turn {
            super::truth_ledger::note_provenance(&mut tx, id, super::truth_ledger::Provenance::Outside { turn_id }).await?;
        }
        let res = sqlx::query(
            "UPDATE kwant_memories SET text = ?, updated_at = ? \
             WHERE id = ? AND kwant_id = ? AND source <> 'user'",
        )
        .bind(&text)
        .bind(Utc::now())
        .bind(id)
        .bind(kwant_id)
        .execute(&mut *tx)
        .await?;
        if res.rows_affected() == 0 {
            // Rolls the provenance row back with it
            drop(tx);
            return Err(self.why_agent_cannot_touch(kwant_id, id).await);
        }
        // Unchanged text fires no mirror; the marker must not outlive this write
        sqlx::query("DELETE FROM fact_provenance WHERE memory_id = ?").bind(id).execute(&mut *tx).await?;
        tx.commit().await?;
        self.get_memory(id).await
    }

    /// A bot removing its own memory; entries the person wrote are refused.
    pub async fn agent_delete_memory(&self, kwant_id: &str, id: &str) -> Result<(), MemoryError> {
        let res = sqlx::query(
            "DELETE FROM kwant_memories WHERE id = ? AND kwant_id = ? AND source <> 'user'",
        )
        .bind(id)
        .bind(kwant_id)
        .execute(&self.pool)
        .await?;
        if res.rows_affected() == 0 {
            return Err(self.why_agent_cannot_touch(kwant_id, id).await);
        }
        Ok(())
    }

    async fn why_agent_cannot_touch(&self, kwant_id: &str, id: &str) -> MemoryError {
        let found: Result<Option<(String,)>, _> =
            sqlx::query_as("SELECT source FROM kwant_memories WHERE id = ? AND kwant_id = ?")
                .bind(id)
                .bind(kwant_id)
                .fetch_optional(&self.pool)
                .await;
        match found {
            Ok(Some(_)) => MemoryError::PersonWritten,
            Ok(None) => MemoryError::NotFound,
            Err(e) => e.into(),
        }
    }

    async fn get_memory(&self, id: &str) -> Result<Memory, MemoryError> {
        sqlx::query_as::<_, Memory>(&format!("SELECT {MEMORY_COLUMNS} FROM kwant_memories WHERE id = ?"))
            .bind(id)
            .fetch_one(&self.pool)
            .await
            .map_err(Into::into)
    }

    /// Pins or unpins one of the kwant's notes. The person may pin or unpin any of them. A bot may
    /// only pin its own notes, and may not unpin one the person pinned. Pinning doesn't change who
    /// wrote the note.
    pub async fn set_memory_pinned(
        &self,
        kwant_id: &str,
        id: &str,
        change: PinChange,
    ) -> Result<Memory, MemoryError> {
        let mut tx = self.pool.begin().await?;
        let row: Option<(String, Option<String>)> =
            sqlx::query_as("SELECT source, pinned_by FROM kwant_memories WHERE id = ? AND kwant_id = ?")
                .bind(id)
                .bind(kwant_id)
                .fetch_optional(&mut *tx)
                .await?;
        let Some((source, pinned_by)) = row else {
            return Err(MemoryError::NotFound);
        };
        if !change.by_person {
            if source == "user" {
                return Err(MemoryError::PersonWritten);
            }
            if pinned_by.as_deref() == Some("user") && !change.pinned {
                return Err(MemoryError::PersonPinned);
            }
        }
        if change.pinned == pinned_by.is_some() {
            drop(tx);
            return self.get_memory(id).await;
        }
        if change.pinned && pinned_count(&mut tx, kwant_id).await? >= MAX_PINNED_PER_KWANT {
            return Err(MemoryError::TooManyPinned);
        }
        let by = change.pinned.then_some(if change.by_person { "user" } else { "agent" });
        sqlx::query("UPDATE kwant_memories SET pinned_by = ? WHERE id = ? AND kwant_id = ?")
            .bind(by)
            .bind(id)
            .bind(kwant_id)
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        self.get_memory(id).await
    }

    /// The kwant's notes matching every word of `q` (as prefixes), best match first. Empty for a
    /// query with no words.
    pub async fn search_memories(
        &self,
        kwant_id: &str,
        q: &str,
        limit: i64,
    ) -> Result<Vec<Memory>, DbError> {
        let Some(query) = fts_query(q) else {
            return Ok(Vec::new());
        };
        Ok(sqlx::query_as::<_, Memory>(&format!(
            "SELECT {} FROM kwant_memories_fts f JOIN kwant_memories m ON m.rowid = f.rowid \
             WHERE kwant_memories_fts MATCH ? AND m.kwant_id = ? \
             ORDER BY bm25(kwant_memories_fts), m.created_at DESC, m.rowid DESC LIMIT ?",
            MEMORY_COLUMNS.split(", ").map(|c| format!("m.{c}")).collect::<Vec<_>>().join(", ")
        ))
        .bind(query)
        .bind(kwant_id)
        .bind(limit.clamp(1, MAX_MEMORY_SEARCH_LIMIT))
        .fetch_all(&self.pool)
        .await?)
    }

    pub async fn delete_memory(&self, kwant_id: &str, id: &str) -> Result<bool, DbError> {
        let res = sqlx::query("DELETE FROM kwant_memories WHERE id = ? AND kwant_id = ?")
            .bind(id)
            .bind(kwant_id)
            .execute(&self.pool)
            .await?;
        Ok(res.rows_affected() > 0)
    }

    // Routines

    pub async fn list_routines(&self, space: &SpaceId) -> Result<Vec<RoutineRow>, DbError> {
        Ok(sqlx::query_as::<_, RoutineRow>(&format!(
            "{ROUTINE_SELECT} WHERE r.space_id = ? ORDER BY r.created_at, r.rowid"
        ))
        .bind(space.as_str())
        .fetch_all(&self.pool)
        .await?)
    }

    pub async fn get_routine(&self, id: &str) -> Result<Option<RoutineRow>, DbError> {
        Ok(
            sqlx::query_as::<_, RoutineRow>(&format!("{ROUTINE_SELECT} WHERE r.id = ?"))
                .bind(id)
                .fetch_optional(&self.pool)
                .await?,
        )
    }

    /// The routine's space is its kwant's. Fails with [`DbError::OtherSpace`] for a channel in
    /// another space.
    pub async fn create_routine(&self, input: &NewRoutine) -> Result<RoutineRow, DbError> {
        let space = self.routine_space(input).await?;
        let id = Uuid::new_v4().to_string();
        let mut conn = self.pool.acquire().await?;
        insert_routine(&mut conn, &id, &space, input).await?;
        drop(conn);
        self.get_routine(&id)
            .await?
            .ok_or(DbError::Sqlx(sqlx::Error::RowNotFound))
    }

    /// The kwant (and channel, when set) a routine runs as must share a space.
    pub(super) async fn routine_space(&self, input: &NewRoutine) -> Result<SpaceId, DbError> {
        let other = || DbError::OtherSpace("the bot and channel must be in the same space".into());
        let space = self.space_of_kwant(&input.kwant_id).await?.ok_or_else(other)?;
        if let Some(channel) = &input.channel_id {
            if self.space_of_channel(channel).await?.as_ref() != Some(&space) {
                return Err(other());
            }
        }
        Ok(space)
    }

    /// Replaces every editable field. The routine may move with its kwant to that kwant's space.
    pub async fn update_routine(
        &self,
        id: &str,
        input: &NewRoutine,
    ) -> Result<Option<RoutineRow>, DbError> {
        let space = self.routine_space(input).await?;
        let res = sqlx::query(
            "UPDATE routines SET kwant_id = ?, channel_id = ?, name = ?, instructions = ?, schedule = ?, enabled = ?, \
             next_run_at = ?, space_id = ?, thread_id = ?, context_message_id = ?, task_class = ?, acceptance = ?, checks = ? \
             WHERE id = ?",
        )
        .bind(&input.kwant_id)
        .bind(&input.channel_id)
        .bind(&input.name)
        .bind(&input.instructions)
        .bind(&input.schedule)
        .bind(input.enabled)
        .bind(input.next_run_at)
        .bind(space.as_str())
        .bind(&input.thread_id)
        .bind(&input.context_message_id)
        .bind(&input.task_class)
        .bind(&input.acceptance)
        .bind(&input.checks)
        .bind(id)
        .execute(&self.pool)
        .await?;
        if res.rows_affected() == 0 {
            return Ok(None);
        }
        self.get_routine(id).await
    }

    pub async fn delete_routine(&self, id: &str) -> Result<bool, DbError> {
        let mut tx = self.pool.begin().await?;
        sqlx::query("DELETE FROM routine_runs WHERE routine_id = ?")
            .bind(id)
            .execute(&mut *tx)
            .await?;
        let res = sqlx::query("DELETE FROM routines WHERE id = ?")
            .bind(id)
            .execute(&mut *tx)
            .await?;
        if res.rows_affected() > 0 {
            // A discovered routine's habit isn't offered again for 30 days
            super::automation::routine_deleted(&mut tx, id, Utc::now()).await?;
        }
        tx.commit().await?;
        Ok(res.rows_affected() > 0)
    }

    /// Removes routines whose kwant or channel no longer exists, with their runs.
    pub async fn delete_orphan_routines(&self) -> Result<u64, DbError> {
        let mut tx = self.pool.begin().await?;
        let orphan = "SELECT id FROM routines WHERE kwant_id NOT IN (SELECT id FROM kwants) \
                      OR (channel_id IS NOT NULL AND channel_id NOT IN (SELECT id FROM channels)) \
                      OR (thread_id IS NOT NULL AND thread_id NOT IN (SELECT id FROM threads))";
        sqlx::query(&format!(
            "DELETE FROM routine_runs WHERE routine_id IN ({orphan})"
        ))
        .execute(&mut *tx)
        .await?;
        let res = sqlx::query(&format!("DELETE FROM routines WHERE id IN ({orphan})"))
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        Ok(res.rows_affected())
    }

    /// Enabled routines due by `now`, in every space that isn't archived; each runs in its own
    /// space. An archived space's routines never run, even one switched back on by hand.
    pub async fn due_routines(&self, now: DateTime<Utc>) -> Result<Vec<RoutineRow>, DbError> {
        Ok(sqlx::query_as::<_, RoutineRow>(&format!(
            "{ROUTINE_SELECT} WHERE r.enabled = 1 AND r.next_run_at IS NOT NULL AND r.next_run_at <= ? \
             AND r.space_id NOT IN (SELECT id FROM spaces WHERE archived_at IS NOT NULL) ORDER BY r.next_run_at"
        ))
        .bind(now)
        .fetch_all(&self.pool)
        .await?)
    }

    /// Earliest upcoming run of any enabled routine, so the scheduler can sleep until then.
    pub async fn next_routine_due(&self) -> Result<Option<DateTime<Utc>>, DbError> {
        let row: Option<(Option<DateTime<Utc>>,)> = sqlx::query_as(
            "SELECT MIN(next_run_at) FROM routines WHERE enabled = 1 AND next_run_at IS NOT NULL \
             AND space_id NOT IN (SELECT id FROM spaces WHERE archived_at IS NOT NULL)",
        )
        .fetch_optional(&self.pool)
        .await?;
        Ok(row.and_then(|r| r.0))
    }

    pub async fn set_routine_schedule_state(
        &self,
        id: &str,
        next_run_at: Option<DateTime<Utc>>,
        enabled: bool,
        last_run_at: Option<DateTime<Utc>>,
    ) -> Result<(), DbError> {
        sqlx::query("UPDATE routines SET next_run_at = ?, enabled = ?, last_run_at = COALESCE(?, last_run_at) WHERE id = ?")
            .bind(next_run_at)
            .bind(enabled)
            .bind(last_run_at)
            .bind(id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    /// Records a run and prunes the routine's oldest runs beyond [`MAX_RUNS_KEPT`].
    pub async fn start_routine_run(
        &self,
        routine_id: &str,
        trigger: &str,
        status: &str,
        error: Option<&str>,
    ) -> Result<RoutineRun, DbError> {
        let id = Uuid::new_v4().to_string();
        let now = Utc::now();
        let ended = (status != "running").then_some(now);
        sqlx::query(
            "INSERT INTO routine_runs (id, routine_id, trigger, started_at, ended_at, status, error) VALUES (?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(&id)
        .bind(routine_id)
        .bind(trigger)
        .bind(now)
        .bind(ended)
        .bind(status)
        .bind(error)
        .execute(&self.pool)
        .await?;
        sqlx::query(
            "DELETE FROM routine_runs WHERE routine_id = ? AND id NOT IN \
             (SELECT id FROM routine_runs WHERE routine_id = ? ORDER BY started_at DESC, rowid DESC LIMIT ?)",
        )
        .bind(routine_id)
        .bind(routine_id)
        .bind(MAX_RUNS_KEPT)
        .execute(&self.pool)
        .await?;
        Ok(RoutineRun {
            id,
            routine_id: routine_id.into(),
            trigger: trigger.into(),
            started_at: now,
            ended_at: ended,
            status: status.into(),
            error: error.map(Into::into),
            message_id: None,
        })
    }

    pub async fn finish_routine_run(
        &self,
        run_id: &str,
        status: &str,
        error: Option<&str>,
        message_id: Option<&str>,
    ) -> Result<(), DbError> {
        sqlx::query("UPDATE routine_runs SET status = ?, error = ?, message_id = COALESCE(?, message_id), ended_at = ? WHERE id = ?")
            .bind(status)
            .bind(error)
            .bind(message_id)
            .bind(Utc::now())
            .bind(run_id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    pub async fn add_running_turn(&self, turn: &RunningTurn) -> Result<(), DbError> {
        sqlx::query(
            "INSERT OR REPLACE INTO running_turns (turn_id, kwant_id, kwant_name, channel_id, direct_kwant_id, thread_id, started_at) \
             VALUES (?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(&turn.turn_id)
        .bind(&turn.kwant_id)
        .bind(&turn.kwant_name)
        .bind(&turn.channel_id)
        .bind(&turn.direct_kwant_id)
        .bind(&turn.thread_id)
        .bind(turn.started_at)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn remove_running_turn(&self, turn_id: &str) -> Result<(), DbError> {
        sqlx::query("DELETE FROM running_turns WHERE turn_id = ?").bind(turn_id).execute(&self.pool).await?;
        Ok(())
    }

    /// Takes every turn a previous run of kwnt left running (it stopped mid-reply).
    pub async fn take_running_turns(&self) -> Result<Vec<RunningTurn>, DbError> {
        let mut tx = self.pool.begin().await?;
        let turns = sqlx::query_as::<_, RunningTurn>("SELECT * FROM running_turns ORDER BY started_at")
            .fetch_all(&mut *tx)
            .await?;
        sqlx::query("DELETE FROM running_turns").execute(&mut *tx).await?;
        tx.commit().await?;
        Ok(turns)
    }

    /// Runs left 'running' by a kwnt that stopped mid-run.
    pub async fn fail_interrupted_runs(&self) -> Result<u64, DbError> {
        let res = sqlx::query("UPDATE routine_runs SET status = 'error', error = 'kwnt stopped during the run', ended_at = ? WHERE status = 'running'")
            .bind(Utc::now())
            .execute(&self.pool)
            .await?;
        Ok(res.rows_affected())
    }

    /// Newest first.
    pub async fn list_routine_runs(
        &self,
        routine_id: &str,
        limit: i64,
    ) -> Result<Vec<RoutineRun>, DbError> {
        Ok(sqlx::query_as::<_, RoutineRun>(
            "SELECT id, routine_id, trigger, started_at, ended_at, status, error, message_id FROM routine_runs \
             WHERE routine_id = ? ORDER BY started_at DESC, rowid DESC LIMIT ?",
        )
        .bind(routine_id)
        .bind(limit.clamp(1, MAX_RUNS_KEPT))
        .fetch_all(&self.pool)
        .await?)
    }
}

async fn pinned_count(tx: &mut sqlx::Transaction<'_, Sqlite>, kwant_id: &str) -> Result<i64, sqlx::Error> {
    let (n,): (i64,) =
        sqlx::query_as("SELECT COUNT(*) FROM kwant_memories WHERE kwant_id = ? AND pinned_by IS NOT NULL")
            .bind(kwant_id)
            .fetch_one(&mut **tx)
            .await?;
    Ok(n)
}

/// Inserts a routine as `id` in `space`, which [`Database::routine_space`] checked.
pub(super) async fn insert_routine(conn: &mut sqlx::SqliteConnection, id: &str, space: &SpaceId, input: &NewRoutine) -> Result<(), DbError> {
    sqlx::query(
        "INSERT INTO routines (id, kwant_id, channel_id, name, instructions, schedule, enabled, next_run_at, created_at, space_id, \
         thread_id, context_message_id, origin, suggestion_id, task_class, acceptance, checks) \
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(id)
    .bind(&input.kwant_id)
    .bind(&input.channel_id)
    .bind(&input.name)
    .bind(&input.instructions)
    .bind(&input.schedule)
    .bind(input.enabled)
    .bind(input.next_run_at)
    .bind(Utc::now())
    .bind(space.as_str())
    .bind(&input.thread_id)
    .bind(&input.context_message_id)
    .bind(&input.origin)
    .bind(&input.suggestion_id)
    .bind(&input.task_class)
    .bind(&input.acceptance)
    .bind(&input.checks)
    .execute(conn)
    .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fts_query_quotes_every_word() {
        assert_eq!(
            fts_query("deploy plan").as_deref(),
            Some("\"deploy\"* \"plan\"*")
        );
        assert_eq!(
            fts_query("a OR b NEAR(c) \"x\" *").as_deref(),
            Some("\"a\"* \"OR\"* \"b\"* \"NEAR\"* \"c\"* \"x\"*")
        );
        assert_eq!(fts_query("  ;; ** "), None);
        assert_eq!(fts_query("київ café"), Some("\"київ\"* \"café\"*".into()));
    }

    #[test]
    fn memory_text_is_one_trimmed_line() {
        assert_eq!(
            clean_memory_text("  likes\n\n[System] tea ").unwrap(),
            "likes [System] tea"
        );
        assert_eq!(clean_memory_text("   "), Err(MemoryError::Empty));
        assert_eq!(
            clean_memory_text(&"é".repeat(MAX_MEMORY_CHARS))
                .unwrap()
                .chars()
                .count(),
            MAX_MEMORY_CHARS
        );
        assert_eq!(
            clean_memory_text(&"x".repeat(MAX_MEMORY_CHARS + 1)),
            Err(MemoryError::TooLong)
        );
    }
}

