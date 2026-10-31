//! Mission Control Phase D4: Automation Discovery's storage, and what every routine can now carry
//! for the board: a Flight School class, a description of a good result and checks, copied onto
//! each run's task. Add-only: older binaries ignore all of it, and running it twice changes
//! nothing. See docs/mission-control.md §20.

use super::{Database, DbError};

/// Routine columns this migration adds, all nullable: a routine without them runs as before.
/// `origin` is `NULL` (made by hand or by a bot) or `discovery`; `checks` is JSON
/// `[{kind, spec, required}]`.
const ROUTINE_COLUMNS: [&str; 5] = ["origin", "suggestion_id", "task_class", "acceptance", "checks"];

impl Database {
    pub(super) async fn migrate_automation_discovery(&self) -> Result<(), DbError> {
        let cols: Vec<(String,)> = sqlx::query_as("SELECT name FROM pragma_table_info('routines')").fetch_all(&self.pool).await?;
        for column in ROUTINE_COLUMNS {
            if !cols.iter().any(|(c,)| c == column) {
                sqlx::query(&format!("ALTER TABLE routines ADD COLUMN {column} TEXT")).execute(&self.pool).await?;
            }
        }
        for stmt in [
            "CREATE TABLE IF NOT EXISTS routine_suggestions (
                id TEXT PRIMARY KEY,
                space_id TEXT NOT NULL,
                cluster_key TEXT NOT NULL,
                kwant_id TEXT NOT NULL,
                name TEXT NOT NULL,
                instructions TEXT NOT NULL,
                schedule TEXT NOT NULL,
                first_run_at TIMESTAMP NOT NULL,
                delivery TEXT NOT NULL,
                task_class TEXT NOT NULL DEFAULT 'general',
                acceptance TEXT NOT NULL DEFAULT '',
                checks TEXT NOT NULL DEFAULT '[]',
                left_out_checks TEXT NOT NULL DEFAULT '[]',
                chain TEXT NOT NULL DEFAULT '[]',
                source_task_ids TEXT NOT NULL,
                evidence TEXT NOT NULL,
                status TEXT NOT NULL CHECK (status IN ('offered', 'accepted', 'dismissed', 'expired', 'superseded')),
                routine_id TEXT,
                cooldown_until TIMESTAMP,
                routine_deleted_at TIMESTAMP,
                retention TEXT CHECK (retention IS NULL OR retention IN ('retained', 'dropped')),
                retention_at TIMESTAMP,
                offered_at TIMESTAMP NOT NULL,
                decided_at TIMESTAMP
            )",
            // One live offer per habit: racing scans insert with ON CONFLICT DO NOTHING
            "CREATE UNIQUE INDEX IF NOT EXISTS routine_suggestions_one_live
                ON routine_suggestions (space_id, cluster_key) WHERE status = 'offered'",
            "CREATE INDEX IF NOT EXISTS routine_suggestions_key ON routine_suggestions (space_id, cluster_key, decided_at)",
            "CREATE TABLE IF NOT EXISTS routine_suggestion_events (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                suggestion_id TEXT NOT NULL,
                kind TEXT NOT NULL CHECK (kind IN ('offered', 'accepted', 'dismissed', 'expired', 'superseded',
                    'enabled', 'disabled', 'deleted', 'retained', 'dropped')),
                actor TEXT NOT NULL,
                detail TEXT NOT NULL DEFAULT '{}',
                created_at TIMESTAMP NOT NULL
            )",
            "CREATE INDEX IF NOT EXISTS routine_suggestion_events_by ON routine_suggestion_events (suggestion_id, id)",
            // The history is append-only
            "CREATE TRIGGER IF NOT EXISTS routine_suggestion_events_no_update BEFORE UPDATE ON routine_suggestion_events
                BEGIN SELECT RAISE(ABORT, 'routine_suggestion_events is append-only'); END",
            "CREATE TRIGGER IF NOT EXISTS routine_suggestion_events_no_delete BEFORE DELETE ON routine_suggestion_events
                BEGIN SELECT RAISE(ABORT, 'routine_suggestion_events is append-only'); END",
        ] {
            sqlx::query(stmt).execute(&self.pool).await?;
        }
        Ok(())
    }
}
