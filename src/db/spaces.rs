//! Spaces, step S0: the `spaces` table and a `space_id` on every top-level row, with everything
//! that exists today in the Default space. Nothing reads `space_id` yet, so behaviour is unchanged.
//!
//! The migration runs once, gated on `PRAGMA user_version`. It first writes a full copy of the
//! database to `<data_dir>/backups/` and does nothing else if that fails. The schema change then
//! runs in one transaction that rolls back on any error or on a new foreign-key violation, so a
//! failed migration leaves the old schema, which this release still runs on, and retries at the
//! next start. A binary from before spaces still runs on the migrated database while only the
//! Default space exists: its inserts get the column default and its queries ignore the column.

use super::{Database, DbError};
use sqlx::{Connection, SqliteConnection};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

/// The space every row created before spaces belongs to.
pub const DEFAULT_SPACE_ID: &str = "spc_default";

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
        sqlx::query("VACUUM INTO ?")
            .bind(path.to_string_lossy().into_owned())
            .execute(&self.pool)
            .await?;
        restrict(&path, 0o600)?;
        Ok(path)
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
async fn rebuild(conn: &mut SqliteConnection, table: &str, create: &str, after: &[&str]) -> Result<(), DbError> {
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
async fn foreign_key_violations(conn: &mut SqliteConnection) -> Result<HashMap<(String, String), usize>, DbError> {
    let rows: Vec<(String, Option<i64>, String, i64)> =
        sqlx::query_as("PRAGMA foreign_key_check").fetch_all(&mut *conn).await?;
    let mut counts = HashMap::new();
    for (table, _, parent, _) in rows {
        *counts.entry((table, parent)).or_insert(0) += 1;
    }
    Ok(counts)
}
