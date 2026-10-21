//! Parallel Universe's scenarios (`pu_scenarios`) and runs (`pu_runs`). The nested parts
//! (requirements, traffic, load, criteria, options; a run's scenario copy and branches) are JSON
//! columns checked in Rust by `parallel::Scenario::validate`, so a new field needs no table rebuild.

use super::{Database, DbError};
use crate::parallel::run::{Run, RunStatus};
use crate::parallel::Scenario;
use chrono::{DateTime, Utc};
use sqlx::FromRow;

#[derive(FromRow)]
struct ScenarioRow {
    id: String,
    title: String,
    question: String,
    requirements: String,
    traffic: String,
    load: String,
    criteria: String,
    options: String,
    build_network: bool,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
}

impl ScenarioRow {
    fn into_scenario(self) -> Result<Scenario, DbError> {
        Ok(Scenario {
            id: self.id,
            title: self.title,
            question: self.question,
            requirements: serde_json::from_str(&self.requirements)?,
            traffic: serde_json::from_str(&self.traffic)?,
            load: serde_json::from_str(&self.load)?,
            criteria: serde_json::from_str(&self.criteria)?,
            options: serde_json::from_str(&self.options)?,
            build_network: self.build_network,
            created_at: self.created_at,
            updated_at: self.updated_at,
        })
    }
}

const COLUMNS: &str = "id, title, question, requirements, traffic, load, criteria, options, build_network, created_at, updated_at";

impl Database {
    pub(super) async fn migrate_parallel(&self) -> Result<(), DbError> {
        sqlx::query(
            "CREATE TABLE IF NOT EXISTS pu_scenarios (
                id TEXT PRIMARY KEY,
                title TEXT NOT NULL,
                question TEXT NOT NULL DEFAULT '',
                requirements TEXT NOT NULL DEFAULT '{}',
                traffic TEXT NOT NULL,
                load TEXT NOT NULL,
                criteria TEXT NOT NULL DEFAULT '[]',
                options TEXT NOT NULL DEFAULT '[]',
                created_at TIMESTAMP NOT NULL,
                updated_at TIMESTAMP NOT NULL
            )",
        )
        .execute(&self.pool)
        .await?;
        let has_build_network: Option<(String,)> =
            sqlx::query_as("SELECT name FROM pragma_table_info('pu_scenarios') WHERE name = 'build_network'").fetch_optional(&self.pool).await?;
        if has_build_network.is_none() {
            sqlx::query("ALTER TABLE pu_scenarios ADD COLUMN build_network BOOLEAN NOT NULL DEFAULT 0").execute(&self.pool).await?;
        }
        sqlx::query(
            "CREATE TABLE IF NOT EXISTS pu_runs (
                id TEXT PRIMARY KEY,
                scenario_id TEXT NOT NULL,
                status TEXT NOT NULL,
                scenario TEXT NOT NULL,
                branches TEXT NOT NULL DEFAULT '[]',
                cleanup_errors TEXT NOT NULL DEFAULT '[]',
                started_at TIMESTAMP NOT NULL,
                finished_at TIMESTAMP,
                swept_at TIMESTAMP
            )",
        )
        .execute(&self.pool)
        .await?;
        sqlx::query("CREATE INDEX IF NOT EXISTS pu_runs_scenario ON pu_runs (scenario_id, started_at)").execute(&self.pool).await?;
        Ok(())
    }

    /// Newest change first.
    pub async fn pu_scenarios(&self) -> Result<Vec<Scenario>, DbError> {
        let rows: Vec<ScenarioRow> = sqlx::query_as(&format!("SELECT {COLUMNS} FROM pu_scenarios ORDER BY updated_at DESC LIMIT 200"))
            .fetch_all(&self.pool)
            .await?;
        rows.into_iter().map(ScenarioRow::into_scenario).collect()
    }

    pub async fn pu_scenario(&self, id: &str) -> Result<Option<Scenario>, DbError> {
        let row: Option<ScenarioRow> = sqlx::query_as(&format!("SELECT {COLUMNS} FROM pu_scenarios WHERE id = ?"))
            .bind(id)
            .fetch_optional(&self.pool)
            .await?;
        row.map(ScenarioRow::into_scenario).transpose()
    }

    /// Inserts `s`, or replaces everything but `created_at` of the scenario with its id.
    pub async fn pu_put_scenario(&self, s: &Scenario) -> Result<(), DbError> {
        sqlx::query(&format!(
            "INSERT INTO pu_scenarios ({COLUMNS}) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
             ON CONFLICT(id) DO UPDATE SET
                title = excluded.title, question = excluded.question, requirements = excluded.requirements,
                traffic = excluded.traffic, load = excluded.load, criteria = excluded.criteria,
                options = excluded.options, build_network = excluded.build_network, updated_at = excluded.updated_at"
        ))
        .bind(&s.id)
        .bind(&s.title)
        .bind(&s.question)
        .bind(serde_json::to_string(&s.requirements)?)
        .bind(serde_json::to_string(&s.traffic)?)
        .bind(serde_json::to_string(&s.load)?)
        .bind(serde_json::to_string(&s.criteria)?)
        .bind(serde_json::to_string(&s.options)?)
        .bind(s.build_network)
        .bind(s.created_at)
        .bind(s.updated_at)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// Whether a scenario was deleted.
    pub async fn pu_delete_scenario(&self, id: &str) -> Result<bool, DbError> {
        Ok(sqlx::query("DELETE FROM pu_scenarios WHERE id = ?").bind(id).execute(&self.pool).await?.rows_affected() > 0)
    }
}

#[derive(FromRow)]
struct RunRow {
    id: String,
    scenario_id: String,
    status: String,
    scenario: String,
    branches: String,
    cleanup_errors: String,
    started_at: DateTime<Utc>,
    finished_at: Option<DateTime<Utc>>,
    swept_at: Option<DateTime<Utc>>,
}

impl RunRow {
    fn into_run(self) -> Result<Run, DbError> {
        Ok(Run {
            id: self.id,
            scenario_id: self.scenario_id,
            status: serde_json::from_value(serde_json::Value::String(self.status))?,
            scenario: serde_json::from_str(&self.scenario)?,
            branches: serde_json::from_str(&self.branches)?,
            cleanup_errors: serde_json::from_str(&self.cleanup_errors)?,
            started_at: self.started_at,
            finished_at: self.finished_at,
            swept_at: self.swept_at,
        })
    }
}

const RUN_COLUMNS: &str = "id, scenario_id, status, scenario, branches, cleanup_errors, started_at, finished_at, swept_at";

impl Database {
    /// Inserts `r`, or replaces everything but `started_at` of the run with its id.
    pub async fn pu_put_run(&self, r: &Run) -> Result<(), DbError> {
        sqlx::query(&format!(
            "INSERT INTO pu_runs ({RUN_COLUMNS}) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)
             ON CONFLICT(id) DO UPDATE SET
                status = excluded.status, branches = excluded.branches, cleanup_errors = excluded.cleanup_errors,
                finished_at = excluded.finished_at, swept_at = excluded.swept_at"
        ))
        .bind(&r.id)
        .bind(&r.scenario_id)
        .bind(r.status.as_str())
        .bind(serde_json::to_string(&r.scenario)?)
        .bind(serde_json::to_string(&r.branches)?)
        .bind(serde_json::to_string(&r.cleanup_errors)?)
        .bind(r.started_at)
        .bind(r.finished_at)
        .bind(r.swept_at)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn pu_run(&self, id: &str) -> Result<Option<Run>, DbError> {
        let row: Option<RunRow> =
            sqlx::query_as(&format!("SELECT {RUN_COLUMNS} FROM pu_runs WHERE id = ?")).bind(id).fetch_optional(&self.pool).await?;
        row.map(RunRow::into_run).transpose()
    }

    /// Newest first; one scenario's runs, or every run.
    pub async fn pu_runs(&self, scenario_id: Option<&str>, limit: u32) -> Result<Vec<Run>, DbError> {
        let rows: Vec<RunRow> = match scenario_id {
            Some(id) => sqlx::query_as(&format!("SELECT {RUN_COLUMNS} FROM pu_runs WHERE scenario_id = ? ORDER BY started_at DESC LIMIT ?"))
                .bind(id)
                .bind(limit)
                .fetch_all(&self.pool)
                .await?,
            None => sqlx::query_as(&format!("SELECT {RUN_COLUMNS} FROM pu_runs ORDER BY started_at DESC LIMIT ?")).bind(limit).fetch_all(&self.pool).await?,
        };
        rows.into_iter().map(RunRow::into_run).collect()
    }

    /// Runs a previous process left behind: still `running`, or finished with cleanup errors and
    /// not swept since.
    pub async fn pu_runs_to_sweep(&self) -> Result<Vec<Run>, DbError> {
        let rows: Vec<RunRow> = sqlx::query_as(&format!(
            "SELECT {RUN_COLUMNS} FROM pu_runs WHERE status = ? OR (cleanup_errors != '[]' AND swept_at IS NULL) ORDER BY started_at LIMIT 500"
        ))
        .bind(RunStatus::Running.as_str())
        .fetch_all(&self.pool)
        .await?;
        rows.into_iter().map(RunRow::into_run).collect()
    }
}
