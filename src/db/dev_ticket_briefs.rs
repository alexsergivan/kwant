//! Dev mode work from Jira (0.7.6, jira-tickets brief.md): one brief per ticket, keyed by
//! `(space_id, key)`, and the questions a brief asked. Kept outside exports like the other Dev mode
//! tables, so an imported bundle can't plant a brief or answer a question. Add-only.
//!
//! `state` mirrors the ticket row's state until Juno's `dev_tickets` lands (api.md 1.4): then
//! `db.set_dev_ticket_state` is the one writer and this column is only read by older code paths.

use super::{Database, DbError};
use sqlx::FromRow;

const TABLES: &[&str] = &[
    "CREATE TABLE IF NOT EXISTS dev_ticket_briefs (
        space_id TEXT NOT NULL,
        key TEXT NOT NULL,
        site_id TEXT NOT NULL,
        team_id TEXT NOT NULL,
        url TEXT NOT NULL DEFAULT '',
        summary TEXT NOT NULL DEFAULT '',
        state TEXT NOT NULL DEFAULT 'picked_up',
        thread_id TEXT,
        task_id TEXT,
        mission_id TEXT,
        brief TEXT,
        asked_comment_id TEXT,
        note TEXT,
        created_at INTEGER NOT NULL,
        updated_at INTEGER NOT NULL,
        PRIMARY KEY (space_id, key)
    )",
    "CREATE TABLE IF NOT EXISTS dev_ticket_questions (
        question_id TEXT PRIMARY KEY,
        space_id TEXT NOT NULL,
        key TEXT NOT NULL,
        idx INTEGER NOT NULL,
        default_answer TEXT NOT NULL,
        answer TEXT,
        answered_by TEXT,
        created_at INTEGER NOT NULL
    )",
    "CREATE INDEX IF NOT EXISTS dev_ticket_questions_key ON dev_ticket_questions (space_id, key)",
    "CREATE INDEX IF NOT EXISTS dev_ticket_briefs_thread ON dev_ticket_briefs (thread_id)",
];

/// The tables [`Database::delete_dev_mode_of_space`] clears with the rest of Dev mode.
pub const BRIEF_TABLES: [&str; 2] = ["dev_ticket_briefs", "dev_ticket_questions"];

#[derive(Debug, Clone, FromRow, PartialEq)]
pub struct BriefRow {
    pub space_id: String,
    pub key: String,
    pub site_id: String,
    pub team_id: String,
    pub url: String,
    pub summary: String,
    pub state: String,
    pub thread_id: Option<String>,
    pub task_id: Option<String>,
    pub mission_id: Option<String>,
    /// The accepted brief as JSON (`tickets::brief::Brief`).
    pub brief: Option<String>,
    pub asked_comment_id: Option<String>,
    /// Why it failed or stopped, for the card.
    pub note: Option<String>,
    pub created_at: i64,
    pub updated_at: i64,
    /// The person's edits as JSON (`tickets::brief::Edits`): they win over a re-read brief.
    #[sqlx(default)]
    pub person_edits: Option<String>,
    /// The latest "Matches the ticket" verdict of a review of the ticket's PR (`met`, `not_met`,
    /// `unclear`), for the ticket row's `work.matches` (api-decisions 11).
    #[sqlx(default)]
    pub matches: Option<String>,
}

#[derive(Debug, Clone, FromRow, PartialEq)]
pub struct TicketQuestion {
    pub question_id: String,
    pub space_id: String,
    pub key: String,
    pub idx: i64,
    pub default_answer: String,
    pub answer: Option<String>,
    /// "chat", "card", "defaults", "dismissed" or "jira".
    pub answered_by: Option<String>,
    pub created_at: i64,
}

fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

impl Database {
    pub(super) async fn migrate_ticket_briefs(&self) -> Result<(), DbError> {
        for sql in TABLES {
            sqlx::query(sql).execute(&self.pool).await?;
        }
        // Add-only columns: the person's edits to a brief (api-decisions 14), apart from the brief
        // so a fresh read keeps them; and the review's verdict (api-decisions 11).
        for col in ["person_edits", "matches"] {
            let has: Option<(String,)> = sqlx::query_as("SELECT name FROM pragma_table_info('dev_ticket_briefs') WHERE name = ?").bind(col).fetch_optional(&self.pool).await?;
            if has.is_none() {
                sqlx::query(&format!("ALTER TABLE dev_ticket_briefs ADD COLUMN {col} TEXT")).execute(&self.pool).await?;
            }
        }
        Ok(())
    }

    /// Inserts the brief row for a ticket, or points an existing one at `site_id`/`team_id` again.
    /// Keeps a brief, thread and mission already there (resume).
    pub async fn upsert_ticket_brief(&self, space_id: &str, key: &str, site_id: &str, team_id: &str, url: &str, summary: &str) -> Result<BriefRow, DbError> {
        let now = now_ms();
        let row = sqlx::query_as::<_, BriefRow>(
            "INSERT INTO dev_ticket_briefs (space_id, key, site_id, team_id, url, summary, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?7)
             ON CONFLICT(space_id, key) DO UPDATE SET site_id = ?3, team_id = ?4, url = ?5, summary = ?6, updated_at = ?7
             RETURNING *",
        )
        .bind(space_id)
        .bind(key)
        .bind(site_id)
        .bind(team_id)
        .bind(url)
        .bind(summary)
        .bind(now)
        .fetch_one(&self.pool)
        .await?;
        Ok(row)
    }

    pub async fn ticket_brief(&self, space_id: &str, key: &str) -> Result<Option<BriefRow>, DbError> {
        Ok(sqlx::query_as::<_, BriefRow>("SELECT * FROM dev_ticket_briefs WHERE space_id = ? AND key = ?")
            .bind(space_id)
            .bind(key)
            .fetch_optional(&self.pool)
            .await?)
    }

    pub async fn ticket_brief_by_thread(&self, thread_id: &str) -> Result<Option<BriefRow>, DbError> {
        Ok(sqlx::query_as::<_, BriefRow>("SELECT * FROM dev_ticket_briefs WHERE thread_id = ?")
            .bind(thread_id)
            .fetch_optional(&self.pool)
            .await?)
    }

    /// Moves the row from `from` (any of them) to `to`; false when it wasn't in one of them.
    pub async fn move_ticket_brief(&self, space_id: &str, key: &str, from: &[&str], to: &str) -> Result<bool, DbError> {
        if from.is_empty() {
            return Ok(false);
        }
        let marks = vec!["?"; from.len()].join(", ");
        let sql = format!("UPDATE dev_ticket_briefs SET state = ?, updated_at = ? WHERE space_id = ? AND key = ? AND state IN ({marks})");
        let mut q = sqlx::query(&sql).bind(to).bind(now_ms()).bind(space_id).bind(key);
        for f in from {
            q = q.bind(*f);
        }
        Ok(q.execute(&self.pool).await?.rows_affected() == 1)
    }

    /// Sets the named optional column (`thread_id`, `task_id`, `mission_id`, `brief`,
    /// `asked_comment_id`, `note`, `person_edits`, `matches`).
    pub async fn set_ticket_brief_field(&self, space_id: &str, key: &str, field: &str, value: Option<&str>) -> Result<(), DbError> {
        if !["thread_id", "task_id", "mission_id", "brief", "asked_comment_id", "note", "person_edits", "matches"].contains(&field) {
            return Err(DbError::Conflict(format!("no brief field {field}")));
        }
        sqlx::query(&format!("UPDATE dev_ticket_briefs SET {field} = ?, updated_at = ? WHERE space_id = ? AND key = ?"))
            .bind(value)
            .bind(now_ms())
            .bind(space_id)
            .bind(key)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    /// Forgets every brief and question of a Jira site (its deletion).
    pub async fn delete_ticket_briefs_of_site(&self, space_id: &str, site_id: &str) -> Result<u64, DbError> {
        let mut tx = self.pool.begin().await?;
        sqlx::query("DELETE FROM dev_ticket_questions WHERE space_id = ? AND key IN (SELECT key FROM dev_ticket_briefs WHERE space_id = ? AND site_id = ?)")
            .bind(space_id)
            .bind(space_id)
            .bind(site_id)
            .execute(&mut *tx)
            .await?;
        let done = sqlx::query("DELETE FROM dev_ticket_briefs WHERE space_id = ? AND site_id = ?").bind(space_id).bind(site_id).execute(&mut *tx).await?;
        tx.commit().await?;
        Ok(done.rows_affected())
    }

    pub async fn add_ticket_question(&self, question_id: &str, space_id: &str, key: &str, idx: i64, default_answer: &str) -> Result<(), DbError> {
        sqlx::query("INSERT INTO dev_ticket_questions (question_id, space_id, key, idx, default_answer, created_at) VALUES (?, ?, ?, ?, ?, ?)")
            .bind(question_id)
            .bind(space_id)
            .bind(key)
            .bind(idx)
            .bind(default_answer)
            .bind(now_ms())
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    pub async fn ticket_question(&self, question_id: &str) -> Result<Option<TicketQuestion>, DbError> {
        Ok(sqlx::query_as::<_, TicketQuestion>("SELECT * FROM dev_ticket_questions WHERE question_id = ?")
            .bind(question_id)
            .fetch_optional(&self.pool)
            .await?)
    }

    /// The brief's questions, in the order it asked them.
    pub async fn ticket_questions(&self, space_id: &str, key: &str) -> Result<Vec<TicketQuestion>, DbError> {
        Ok(sqlx::query_as::<_, TicketQuestion>("SELECT * FROM dev_ticket_questions WHERE space_id = ? AND key = ? ORDER BY idx")
            .bind(space_id)
            .bind(key)
            .fetch_all(&self.pool)
            .await?)
    }

    /// Records the answer once: false when the question already had one.
    pub async fn answer_ticket_question(&self, question_id: &str, answer: &str, by: &str) -> Result<bool, DbError> {
        let done = sqlx::query("UPDATE dev_ticket_questions SET answer = ?, answered_by = ? WHERE question_id = ? AND answer IS NULL")
            .bind(answer)
            .bind(by)
            .bind(question_id)
            .execute(&self.pool)
            .await?;
        Ok(done.rows_affected() == 1)
    }

    /// Forgets a brief's questions (a fresh read after Taken back).
    pub async fn clear_ticket_questions(&self, space_id: &str, key: &str) -> Result<(), DbError> {
        sqlx::query("DELETE FROM dev_ticket_questions WHERE space_id = ? AND key = ?").bind(space_id).bind(key).execute(&self.pool).await?;
        Ok(())
    }
}
