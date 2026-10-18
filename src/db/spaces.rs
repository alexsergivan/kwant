//! Spaces: the `spaces` table and a `space_id` on every top-level row (S0), and the lookups that
//! keep each space to itself (S1). Lists and lookups that could reach into another space take a
//! required [`SpaceId`], so a caller that hasn't decided its space doesn't compile.
//!
//! The migration runs once, gated on `PRAGMA user_version`. It first writes a full copy of the
//! database to `<data_dir>/backups/` and does nothing else if that fails. The schema change then
//! runs in one transaction that rolls back on any error or on a new foreign-key violation, so a
//! failed migration leaves the old schema, which this release still runs on, and retries at the
//! next start. A binary from before spaces still runs on the migrated database while only the
//! Default space exists: its inserts get the column default and its queries ignore the column.

use super::{Database, DbError};
use serde::{Deserialize, Serialize};
use sqlx::{Connection, FromRow, SqliteConnection};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

/// The space every row created before spaces belongs to.
pub const DEFAULT_SPACE_ID: &str = "spc_default";

/// Where a space keeps its files (team work folders, sandboxes). The Default space keeps the
/// data dir itself, so existing folders stay where they are; others get `spaces/<id>`, with
/// anything but ASCII letters, digits, `_` and `-` in the id replaced.
pub fn space_data_dir(data_dir: &std::path::Path, space: &SpaceId) -> std::path::PathBuf {
    if space.is_default() {
        return data_dir.to_path_buf();
    }
    let safe: String = space
        .as_str()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() || c == '_' || c == '-' { c } else { '_' })
        .collect();
    data_dir.join("spaces").join(safe)
}

/// The id of a space. Required by every list or lookup that could otherwise return rows from
/// another space; build one from a row you already have (`kwant.space_id`, …) or, for requests
/// that don't name a space yet, with [`SpaceId::default_space`].
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct SpaceId(String);

impl SpaceId {
    pub fn new(id: impl Into<String>) -> Self {
        Self(id.into())
    }

    /// The Default space, which holds everything from before spaces.
    pub fn default_space() -> Self {
        Self(DEFAULT_SPACE_ID.to_string())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn is_default(&self) -> bool {
        self.0 == DEFAULT_SPACE_ID
    }
}

impl std::fmt::Display for SpaceId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// Serde default for the `space_id` field of rows read from older JSON.
pub fn default_space_id() -> String {
    DEFAULT_SPACE_ID.to_string()
}

#[derive(Debug, Clone, Serialize, Deserialize, FromRow, PartialEq)]
pub struct Space {
    pub id: String,
    pub name: String,
    pub icon: String,
    pub color: Option<String>,
    pub sort: i64,
    pub archived_at: Option<chrono::DateTime<chrono::Utc>>,
    pub created_at: Option<chrono::DateTime<chrono::Utc>>,
}

/// `PRAGMA user_version` once the spaces schema is in place.
pub const SPACES_SCHEMA_VERSION: i64 = 1;

/// Tables that only gain a column. Their uniqueness rules don't involve names.
const ADD_COLUMN_TABLES: [&str; 4] = ["kwants", "channels", "routines", "skills"];

/// Tables with a name that was unique app-wide and becomes unique per space. SQLite can't drop a
/// table-level UNIQUE, so these are rebuilt (create new, copy, drop old, rename). Each statement
/// keeps every column and rule of the old table, plus `space_id`; the old case sensitivity is kept.
const REBUILDS: [(&str, &str, &[&str]); 3] = [
    (
        "teams",
        "CREATE TABLE teams__spaces_new (
            id TEXT PRIMARY KEY,
            name TEXT NOT NULL,
            description TEXT,
            icon TEXT NOT NULL DEFAULT 'users',
            lead_kwant_id TEXT NOT NULL,
            member_kwants TEXT NOT NULL,
            workspace_path TEXT,
            repo_url TEXT,
            created_at DATETIME DEFAULT CURRENT_TIMESTAMP,
            instructions TEXT,
            space_id TEXT NOT NULL DEFAULT 'spc_default',
            UNIQUE (space_id, name),
            FOREIGN KEY(lead_kwant_id) REFERENCES kwants(id)
        )",
        &[],
    ),
    (
        "mcp_servers",
        "CREATE TABLE mcp_servers__spaces_new (
            id TEXT PRIMARY KEY,
            name TEXT NOT NULL,
            transport TEXT NOT NULL,
            command TEXT,
            args TEXT NOT NULL DEFAULT '[]',
            url TEXT,
            env TEXT NOT NULL DEFAULT '{}',
            headers TEXT NOT NULL DEFAULT '{}',
            enabled INTEGER NOT NULL DEFAULT 1,
            timeout_secs INTEGER NOT NULL DEFAULT 30,
            created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
            space_id TEXT NOT NULL DEFAULT 'spc_default',
            UNIQUE (space_id, name)
        )",
        &[],
    ),
    (
        "llm_profiles",
        "CREATE TABLE llm_profiles__spaces_new (
            id TEXT PRIMARY KEY,
            engine TEXT NOT NULL CHECK (engine IN ('claude', 'codex', 'grok')),
            name TEXT NOT NULL,
            kind TEXT NOT NULL CHECK (kind IN ('system', 'cli_home', 'api_key')),
            home_dir TEXT UNIQUE,
            priority INTEGER NOT NULL DEFAULT 100,
            enabled INTEGER NOT NULL DEFAULT 1,
            is_default INTEGER NOT NULL DEFAULT 0,
            created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
            space_id TEXT NOT NULL DEFAULT 'spc_default',
            UNIQUE (space_id, engine, name COLLATE NOCASE)
        )",
        // Dropping the old table dropped its one-default index; it comes back per space
        &["CREATE UNIQUE INDEX llm_profiles_one_default ON llm_profiles (space_id, engine) WHERE is_default = 1"],
    ),
];

const INDEXES: [&str; 7] = [
    "DROP INDEX IF EXISTS skills_live_name",
    "CREATE UNIQUE INDEX skills_live_name ON skills (space_id, IFNULL(kwant_id, ''), name)
        WHERE status IN ('enabled', 'disabled')",
    "CREATE INDEX kwants_by_space ON kwants (space_id)",
    "CREATE INDEX channels_by_space ON channels (space_id)",
    "CREATE INDEX teams_by_space ON teams (space_id)",
    "CREATE INDEX llm_profiles_by_space ON llm_profiles (space_id, engine)",
    "CREATE INDEX routines_by_space ON routines (space_id)",
];

/// What happened to the spaces migration at startup.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SpacesMigration {
    /// The schema was already at [`SPACES_SCHEMA_VERSION`].
    AlreadyDone,
    /// Migrated; `backup` is `None` only for a database created by this start.
    Migrated { backup: Option<PathBuf> },
    /// Nothing changed: the backup couldn't be written, or the schema change failed and was
    /// rolled back. Retried at the next start.
    Skipped { reason: String },
}

impl Database {
    /// `PRAGMA user_version`: 0 before spaces, [`SPACES_SCHEMA_VERSION`] after.
    pub async fn schema_version(&self) -> Result<i64, DbError> {
        let (v,): (i64,) = sqlx::query_as("PRAGMA user_version").fetch_one(&self.pool).await?;
        Ok(v)
    }

    /// Never fails startup: an error leaves the old schema in place and is logged.
    pub(super) async fn migrate_spaces(&self, data_dir: &Path, fresh: bool) -> Result<SpacesMigration, DbError> {
        if self.schema_version().await? >= SPACES_SCHEMA_VERSION {
            return Ok(SpacesMigration::AlreadyDone);
        }

        // A database this start created holds nothing worth a copy
        let backup = if fresh {
            None
        } else {
            match self.backup_before_spaces(data_dir).await {
                Ok(path) => Some(path),
                Err(e) => {
                    let reason = format!("couldn't write the pre-spaces backup: {e}");
                    tracing::error!("Spaces migration not run, the database is unchanged: {reason}");
                    return Ok(SpacesMigration::Skipped { reason });
                }
            }
        };

        let mut conn = self.pool.acquire().await?;
        // Table rebuilds need foreign keys off, and SQLite ignores this pragma inside a transaction
        sqlx::query("PRAGMA foreign_keys = OFF").execute(&mut *conn).await?;
        let result = migrate_in_transaction(&mut conn).await;
        sqlx::query("PRAGMA foreign_keys = ON").execute(&mut *conn).await?;

        match result {
            Ok(()) => {
                match &backup {
                    Some(path) => tracing::info!(backup = %path.display(), "Migrated the database to spaces"),
                    None => tracing::info!("Created the database with spaces"),
                }
                Ok(SpacesMigration::Migrated { backup })
            }
            Err(e) => {
                let reason = format!("schema change failed and was rolled back: {e}");
                tracing::error!("Spaces migration not run, the database is unchanged: {reason}");
                Ok(SpacesMigration::Skipped { reason })
            }
        }
    }

    /// `VACUUM INTO` gives a consistent copy under WAL. The copy holds API keys and OAuth
    /// tokens, so the folder and file are owner-only.
    async fn backup_before_spaces(&self, data_dir: &Path) -> Result<PathBuf, DbError> {
        let dir = data_dir.join("backups");
        std::fs::create_dir_all(&dir)?;
        restrict(&dir, 0o700)?;
        let stamp = chrono::Utc::now().format("%Y%m%dT%H%M%SZ");
        let mut path = dir.join(format!("kwnt-pre-spaces-{stamp}.db"));
        let mut n = 1;
        while path.exists() {
            path = dir.join(format!("kwnt-pre-spaces-{stamp}-{n}.db"));
            n += 1;
        }
        // Write under a `.partial` name and rename once complete, so a kill mid-copy never
        // leaves a truncated file that looks like a real backup.
        let partial = path.with_extension("db.partial");
        let _ = std::fs::remove_file(&partial);
        sqlx::query("VACUUM INTO ?")
            .bind(partial.to_string_lossy().into_owned())
            .execute(&self.pool)
            .await?;
        restrict(&partial, 0o600)?;
        std::fs::rename(&partial, &path)?;
        remove_older_pre_spaces_backups(&dir, &path);
        Ok(path)
    }
}

/// Removes every other `kwnt-pre-spaces-*` file once a new backup is complete. Backups are only
/// written while the database is still before spaces, so an older one is from a start whose
/// migration failed and is superseded by the new copy; leftover `.partial` files are cut-off
/// copies. Without this every refused restart would leave another full copy on disk. An older
/// backup is not reused instead: the previous release may have written to the database since.
fn remove_older_pre_spaces_backups(dir: &Path, keep: &Path) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let path = entry.path();
        let ours = path
            .file_name()
            .and_then(|n| n.to_str())
            .is_some_and(|n| n.starts_with("kwnt-pre-spaces-") && (n.ends_with(".db") || n.ends_with(".db.partial")));
        if ours && path != keep {
            if let Err(e) = std::fs::remove_file(&path) {
                tracing::warn!(file = %path.display(), error = %e, "could not remove an older pre-spaces backup");
            }
        }
    }
}

#[cfg(unix)]
fn restrict(path: &Path, mode: u32) -> std::io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode))
}

#[cfg(not(unix))]
fn restrict(_path: &Path, _mode: u32) -> std::io::Result<()> {
    Ok(())
}

async fn migrate_in_transaction(conn: &mut SqliteConnection) -> Result<(), DbError> {
    // Violations already in the data aren't ours to fix; only new ones abort the migration
    let violations_before = foreign_key_violations(conn).await?;

    let mut tx = conn.begin().await?;
    sqlx::query(
        "CREATE TABLE spaces (
            id TEXT PRIMARY KEY,
            name TEXT NOT NULL UNIQUE COLLATE NOCASE,
            icon TEXT NOT NULL DEFAULT 'box',
            color TEXT,
            sort INTEGER NOT NULL DEFAULT 0,
            archived_at TIMESTAMP,
            created_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP
        )",
    )
    .execute(&mut *tx)
    .await?;
    sqlx::query("INSERT INTO spaces (id, name, icon, color, sort) VALUES (?, 'Default', '💼', 'amber', 0)")
        .bind(DEFAULT_SPACE_ID)
        .execute(&mut *tx)
        .await?;

    // No REFERENCES spaces(id): SQLite refuses ADD COLUMN with a foreign key and a non-NULL
    // default. Space membership is enforced in code
    for table in ADD_COLUMN_TABLES {
        sqlx::query(&format!(
            "ALTER TABLE {table} ADD COLUMN space_id TEXT NOT NULL DEFAULT '{DEFAULT_SPACE_ID}'"
        ))
        .execute(&mut *tx)
        .await?;
    }

    for (table, create, after) in REBUILDS {
        rebuild(&mut tx, table, create, after).await?;
    }

    for stmt in INDEXES {
        sqlx::query(stmt).execute(&mut *tx).await?;
    }

    let violations_after = foreign_key_violations(&mut tx).await?;
    let new: Vec<_> = violations_after
        .iter()
        .filter(|(key, n)| **n > violations_before.get(*key).copied().unwrap_or(0))
        .collect();
    if !new.is_empty() {
        return Err(DbError::Migration(format!("foreign key check failed: {new:?}")));
    }

    sqlx::query(&format!("PRAGMA user_version = {SPACES_SCHEMA_VERSION}"))
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok(())
}

/// SQLite's documented rebuild: create the new table, copy every row, drop the old one and
/// rename. Child tables refer to the parent by name, so their foreign keys follow the rename.
/// Refuses to drop a column the new table doesn't have, rather than lose data.
pub(super) async fn rebuild(conn: &mut SqliteConnection, table: &str, create: &str, after: &[&str]) -> Result<(), DbError> {
    let new_table = format!("{table}__spaces_new");
    sqlx::query(create).execute(&mut *conn).await?;

    let old_cols = columns(conn, table).await?;
    let new_cols: HashSet<String> = columns(conn, &new_table).await?.into_iter().collect();
    if let Some(missing) = old_cols.iter().find(|c| !new_cols.contains(*c)) {
        return Err(DbError::Migration(format!("{table}.{missing} has no place in the new table")));
    }

    // rowid too, so listings without an ORDER BY keep their order
    let list = old_cols.iter().map(|c| format!("\"{c}\"")).collect::<Vec<_>>().join(", ");
    let copied = sqlx::query(&format!(
        "INSERT INTO {new_table} (rowid, {list}) SELECT rowid, {list} FROM {table}"
    ))
        .execute(&mut *conn)
        .await?
        .rows_affected();
    let (old_count,): (i64,) = sqlx::query_as(&format!("SELECT COUNT(*) FROM {table}"))
        .fetch_one(&mut *conn)
        .await?;
    if copied != old_count as u64 {
        return Err(DbError::Migration(format!("{table}: copied {copied} of {old_count} rows")));
    }

    sqlx::query(&format!("DROP TABLE {table}")).execute(&mut *conn).await?;
    sqlx::query(&format!("ALTER TABLE {new_table} RENAME TO {table}"))
        .execute(&mut *conn)
        .await?;
    for stmt in after {
        sqlx::query(stmt).execute(&mut *conn).await?;
    }
    Ok(())
}

async fn columns(conn: &mut SqliteConnection, table: &str) -> Result<Vec<String>, DbError> {
    let rows: Vec<(String,)> = sqlx::query_as("SELECT name FROM pragma_table_info(?) ORDER BY cid")
        .bind(table)
        .fetch_all(&mut *conn)
        .await?;
    Ok(rows.into_iter().map(|(name,)| name).collect())
}

/// `PRAGMA foreign_key_check` counted per (table, parent). Not per rowid: a rebuild renumbers them.
pub(super) async fn foreign_key_violations(conn: &mut SqliteConnection) -> Result<HashMap<(String, String), usize>, DbError> {
    let rows: Vec<(String, Option<i64>, String, i64)> =
        sqlx::query_as("PRAGMA foreign_key_check").fetch_all(&mut *conn).await?;
    let mut counts = HashMap::new();
    for (table, _, parent, _) in rows {
        *counts.entry((table, parent)).or_insert(0) += 1;
    }
    Ok(counts)
}

/// What [`Database::create_space`] needs. There's no API for it before S2; tests and the
/// later `/api/spaces` call it.
#[derive(Debug, Clone)]
pub struct NewSpace {
    pub name: String,
    pub icon: String,
    pub color: Option<String>,
    /// This machine's switch in the new space, for every engine.
    pub this_machine_enabled: bool,
}

impl Database {
    /// Fails startup when the spaces migration hasn't run: S1 queries need `space_id`.
    pub(super) async fn require_spaces_schema(&self, migration: &SpacesMigration) -> Result<(), DbError> {
        if self.schema_version().await? >= SPACES_SCHEMA_VERSION {
            return Ok(());
        }
        let why = match migration {
            SpacesMigration::Skipped { reason } => reason.clone(),
            _ => "unknown".into(),
        };
        Err(DbError::Migration(format!(
            "kwnt can't start: the database is still on the schema from before spaces, and this \
             version needs spaces ({why}). Nothing was changed. Fix the cause and start kwnt again, \
             or run the previous kwnt release, which still works on this database."
        )))
    }

    /// One #general per space, as a unique index. A database that already has two channels called
    /// "general" in one space keeps working without the index (the old app never stopped a user
    /// creating a second one); [`Database::create_channel_in`] still refuses new duplicates.
    /// Returns whether the index is in place.
    pub(super) async fn ensure_one_general(&self) -> Result<bool, DbError> {
        let dupes: Vec<(String, i64)> = sqlx::query_as(
            "SELECT space_id, COUNT(*) FROM channels WHERE name = 'general' GROUP BY space_id HAVING COUNT(*) > 1",
        )
        .fetch_all(&self.pool)
        .await?;
        if !dupes.is_empty() {
            tracing::warn!(
                spaces = ?dupes,
                "Several #general channels in one space; not adding the one-#general rule until they're renamed"
            );
            return Ok(false);
        }
        sqlx::query("CREATE UNIQUE INDEX IF NOT EXISTS channels_one_general ON channels (space_id) WHERE name = 'general'")
            .execute(&self.pool)
            .await?;
        Ok(true)
    }

    /// A new, empty space with its own This machine row per engine (the space's default for
    /// each engine). Its #general and bots come with S2's create flow.
    pub async fn create_space(&self, input: &NewSpace) -> Result<Space, DbError> {
        let id = SpaceId::new(format!("spc_{}", uuid::Uuid::new_v4().simple()));
        let mut tx = self.pool.begin().await?;
        sqlx::query("INSERT INTO spaces (id, name, icon, color, sort) VALUES (?, ?, ?, ?, (SELECT IFNULL(MAX(sort), 0) + 1 FROM spaces))")
            .bind(id.as_str())
            .bind(input.name.trim())
            .bind(&input.icon)
            .bind(&input.color)
            .execute(&mut *tx)
            .await?;
        for engine in super::profiles::PROFILE_ENGINES {
            sqlx::query(
                "INSERT INTO llm_profiles (id, engine, name, kind, home_dir, priority, enabled, is_default, space_id)
                 VALUES (?, ?, 'This machine', 'system', NULL, 0, ?, 1, ?)",
            )
            .bind(super::profiles::system_profile_id_in(&id, engine))
            .bind(engine)
            .bind(input.this_machine_enabled)
            .bind(id.as_str())
            .execute(&mut *tx)
            .await?;
        }
        tx.commit().await?;
        self.get_space(&id).await?.ok_or(DbError::Sqlx(sqlx::Error::RowNotFound))
    }

    pub async fn get_space(&self, id: &SpaceId) -> Result<Option<Space>, DbError> {
        Ok(sqlx::query_as::<_, Space>("SELECT * FROM spaces WHERE id = ?")
            .bind(id.as_str())
            .fetch_optional(&self.pool)
            .await?)
    }

    pub async fn list_spaces(&self) -> Result<Vec<Space>, DbError> {
        Ok(sqlx::query_as::<_, Space>("SELECT * FROM spaces ORDER BY sort, created_at, id")
            .fetch_all(&self.pool)
            .await?)
    }

    async fn space_of(&self, sql: &str, id: &str) -> Result<Option<SpaceId>, DbError> {
        let row: Option<(String,)> = sqlx::query_as(sql).bind(id).fetch_optional(&self.pool).await?;
        Ok(row.map(|(s,)| SpaceId(s)))
    }

    pub async fn space_of_kwant(&self, id: &str) -> Result<Option<SpaceId>, DbError> {
        self.space_of("SELECT space_id FROM kwants WHERE id = ?", id).await
    }

    pub async fn space_of_channel(&self, id: &str) -> Result<Option<SpaceId>, DbError> {
        self.space_of("SELECT space_id FROM channels WHERE id = ?", id).await
    }

    pub async fn space_of_team(&self, id: &str) -> Result<Option<SpaceId>, DbError> {
        self.space_of("SELECT space_id FROM teams WHERE id = ?", id).await
    }

    pub async fn space_of_profile(&self, id: &str) -> Result<Option<SpaceId>, DbError> {
        self.space_of("SELECT space_id FROM llm_profiles WHERE id = ?", id).await
    }

    pub async fn space_of_mcp_server(&self, id: &str) -> Result<Option<SpaceId>, DbError> {
        self.space_of("SELECT space_id FROM mcp_servers WHERE id = ?", id).await
    }

    pub async fn space_of_routine(&self, id: &str) -> Result<Option<SpaceId>, DbError> {
        self.space_of("SELECT space_id FROM routines WHERE id = ?", id).await
    }

    pub async fn space_of_skill(&self, id: &str) -> Result<Option<SpaceId>, DbError> {
        self.space_of("SELECT space_id FROM skills WHERE id = ?", id).await
    }

    /// A question lives in the space of the kwant that asked it.
    pub async fn space_of_question(&self, id: &str) -> Result<Option<SpaceId>, DbError> {
        self.space_of("SELECT k.space_id FROM questions q JOIN kwants k ON k.id = q.kwant_id WHERE q.id = ?", id)
            .await
    }

    pub async fn space_of_thread(&self, id: &str) -> Result<Option<SpaceId>, DbError> {
        self.space_of("SELECT k.space_id FROM threads t JOIN kwants k ON k.id = t.kwant_id WHERE t.id = ?", id)
            .await
    }

    /// A delegation lives in its lead's space (its specialist is always in the same one).
    pub async fn space_of_delegation(&self, id: &str) -> Result<Option<SpaceId>, DbError> {
        self.space_of("SELECT k.space_id FROM delegations d JOIN kwants k ON k.id = d.lead_kwant_id WHERE d.id = ?", id)
            .await
    }

    pub async fn space_of_approval(&self, id: &str) -> Result<Option<SpaceId>, DbError> {
        self.space_of("SELECT k.space_id FROM approvals a JOIN kwants k ON k.id = a.kwant_id WHERE a.id = ?", id)
            .await
    }

    /// Whether `space` exists and is archived.
    pub async fn space_archived(&self, space: &SpaceId) -> Result<bool, DbError> {
        Ok(self.get_space(space).await?.is_some_and(|s| s.archived_at.is_some()))
    }

    /// The space of whatever an event or chat id names: a channel, a kwant (its direct chat),
    /// a thread (its kwant's space) or a team.
    pub async fn space_of_context(&self, id: &str) -> Result<Option<SpaceId>, DbError> {
        self.space_of(
            "SELECT space_id FROM channels WHERE id = ?1
             UNION ALL SELECT space_id FROM kwants WHERE id = ?1
             UNION ALL SELECT k.space_id FROM threads t JOIN kwants k ON k.id = t.kwant_id WHERE t.id = ?1
             UNION ALL SELECT space_id FROM teams WHERE id = ?1
             LIMIT 1",
            id,
        )
        .await
    }

    /// Whether every one of `kwant_ids` is a kwant in `space`. Used before a write links kwants
    /// to a channel, team or routine, so nothing ever mixes two spaces.
    pub async fn kwants_in_space(&self, space: &SpaceId, kwant_ids: &[&str]) -> Result<bool, DbError> {
        for id in kwant_ids {
            if self.space_of_kwant(id).await?.as_ref() != Some(space) {
                return Ok(false);
            }
        }
        Ok(true)
    }

    /// Renames or restyles a space, or moves it in the list. `None` keeps a field.
    pub async fn update_space(&self, id: &SpaceId, input: &SpaceUpdate) -> Result<Option<Space>, DbError> {
        let res = sqlx::query(
            "UPDATE spaces SET name = COALESCE(?, name), icon = COALESCE(?, icon), \
             color = CASE WHEN ? THEN ? ELSE color END, sort = COALESCE(?, sort) WHERE id = ?",
        )
        .bind(input.name.as_deref().map(str::trim))
        .bind(&input.icon)
        .bind(input.color.is_some())
        .bind(input.color.clone().flatten())
        .bind(input.sort)
        .bind(id.as_str())
        .execute(&self.pool)
        .await?;
        if res.rows_affected() == 0 {
            return Ok(None);
        }
        self.get_space(id).await
    }

    /// Archives a space: its enabled routines are switched off and marked, so a restore brings
    /// back only those. The Default space can't be archived. Returns how many were paused.
    pub async fn archive_space(&self, id: &SpaceId) -> Result<u64, DbError> {
        if id.is_default() {
            return Err(DbError::Conflict("the Default space can't be archived".into()));
        }
        let mut tx = self.pool.begin().await?;
        let paused = sqlx::query(
            "UPDATE routines SET enabled = 0, paused_by_archive = 1 WHERE space_id = ? AND enabled = 1",
        )
        .bind(id.as_str())
        .execute(&mut *tx)
        .await?
        .rows_affected();
        sqlx::query("UPDATE spaces SET archived_at = COALESCE(archived_at, ?) WHERE id = ?")
            .bind(chrono::Utc::now())
            .bind(id.as_str())
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        Ok(paused)
    }

    /// Restores an archived space and switches back on the routines its archive paused. Their
    /// next runs are left for the caller to recompute. Returns how many came back.
    pub async fn restore_space(&self, id: &SpaceId) -> Result<u64, DbError> {
        let mut tx = self.pool.begin().await?;
        let resumed = sqlx::query(
            "UPDATE routines SET enabled = 1, paused_by_archive = 0, next_run_at = NULL \
             WHERE space_id = ? AND paused_by_archive = 1",
        )
        .bind(id.as_str())
        .execute(&mut *tx)
        .await?
        .rows_affected();
        sqlx::query("UPDATE spaces SET archived_at = NULL WHERE id = ?")
            .bind(id.as_str())
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        Ok(resumed)
    }

    /// Bots in the space and routines its archive paused.
    pub async fn space_counts(&self, id: &SpaceId) -> Result<(i64, i64), DbError> {
        let (bots,): (i64,) = sqlx::query_as("SELECT COUNT(*) FROM kwants WHERE space_id = ?")
            .bind(id.as_str())
            .fetch_one(&self.pool)
            .await?;
        let (paused,): (i64,) =
            sqlx::query_as("SELECT COUNT(*) FROM routines WHERE space_id = ? AND paused_by_archive = 1")
                .bind(id.as_str())
                .fetch_one(&self.pool)
                .await?;
        Ok((bots, paused))
    }

    /// Adds `routines.paused_by_archive` if missing. Additive, so an older binary still runs.
    pub(super) async fn migrate_space_archive(&self) -> Result<(), DbError> {
        let cols: Vec<(String,)> = sqlx::query_as("SELECT name FROM pragma_table_info('routines')")
            .fetch_all(&self.pool)
            .await?;
        if !cols.iter().any(|(c,)| c == "paused_by_archive") {
            sqlx::query("ALTER TABLE routines ADD COLUMN paused_by_archive INTEGER NOT NULL DEFAULT 0")
                .execute(&self.pool)
                .await?;
        }
        Ok(())
    }
}

/// A `PATCH /api/spaces/:id` body without the archive switch. `color: Some(None)` clears it.
#[derive(Debug, Default, Clone)]
pub struct SpaceUpdate {
    pub name: Option<String>,
    pub icon: Option<String>,
    pub color: Option<Option<String>>,
    pub sort: Option<i64>,
}
