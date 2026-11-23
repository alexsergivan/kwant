//! pa3 Quizmaster: quiz drafts a bot is writing, with their last check (state, numbered
//! problems, per-question source results, dry run line) and the budget counts. Add-only.
//! The pocket itself (players, answers, the running game) is pa1's `pocket_apps`.

use super::{Database, DbError};
use sqlx::FromRow;

const TABLES: &[&str] = &[
    "CREATE TABLE IF NOT EXISTS quiz_drafts (
        id TEXT PRIMARY KEY,
        space_id TEXT NOT NULL,
        kwant_id TEXT NOT NULL,
        title TEXT NOT NULL DEFAULT '',
        spec TEXT NOT NULL,
        state TEXT NOT NULL CHECK (state IN ('writing', 'checking', 'dry_run_stopped', 'ready', 'open', 'closed')),
        problems TEXT NOT NULL DEFAULT '[]',
        sources TEXT NOT NULL DEFAULT '[]',
        dry_run TEXT,
        budget TEXT NOT NULL DEFAULT '{}',
        pocket_code TEXT,
        created_at INTEGER NOT NULL,
        updated_at INTEGER NOT NULL
    )",
    "CREATE INDEX IF NOT EXISTS quiz_drafts_space ON quiz_drafts (space_id, updated_at)",
];

const COLS: &str = "id, space_id, kwant_id, title, spec, state, problems, sources, dry_run, budget, pocket_code, created_at, updated_at";

#[derive(Debug, Clone, FromRow, PartialEq)]
pub struct QuizDraftRow {
    pub id: String,
    pub space_id: String,
    /// The bot that wrote it.
    pub kwant_id: String,
    pub title: String,
    /// The last spec handed in, as sent (a spec that doesn't validate is kept so the bot can fix it).
    pub spec: String,
    pub state: String,
    /// JSON array of numbered fix-it lines.
    pub problems: String,
    /// JSON array of `quizmaster::source_check::SourceResult`.
    pub sources: String,
    /// JSON `quizmaster::card::DryRunLine`.
    pub dry_run: Option<String>,
    /// JSON `quizmaster::budget::Budget`.
    pub budget: String,
    /// The pocket's code once opened (pa1).
    pub pocket_code: Option<String>,
    pub created_at: i64,
    pub updated_at: i64,
}

impl Database {
    pub(super) async fn migrate_quiz_drafts(&self) -> Result<(), DbError> {
        for sql in TABLES {
            sqlx::query(sql).execute(&self.pool).await?;
        }
        Ok(())
    }

    pub async fn quiz_draft_save(&self, r: &QuizDraftRow) -> Result<(), DbError> {
        sqlx::query(&format!(
            "INSERT INTO quiz_drafts ({COLS}) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
             ON CONFLICT(id) DO UPDATE SET title = excluded.title, spec = excluded.spec, state = excluded.state,
               problems = excluded.problems, sources = excluded.sources, dry_run = excluded.dry_run, budget = excluded.budget,
               pocket_code = excluded.pocket_code, updated_at = excluded.updated_at"
        ))
        .bind(&r.id)
        .bind(&r.space_id)
        .bind(&r.kwant_id)
        .bind(&r.title)
        .bind(&r.spec)
        .bind(&r.state)
        .bind(&r.problems)
        .bind(&r.sources)
        .bind(&r.dry_run)
        .bind(&r.budget)
        .bind(&r.pocket_code)
        .bind(r.created_at)
        .bind(r.updated_at)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// A draft in this space.
    pub async fn quiz_draft_get(&self, space_id: &str, id: &str) -> Result<Option<QuizDraftRow>, DbError> {
        Ok(sqlx::query_as(&format!("SELECT {COLS} FROM quiz_drafts WHERE space_id = ? AND id = ?")).bind(space_id).bind(id).fetch_optional(&self.pool).await?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn save_get_update_and_space_scoping() {
        let tmp = tempfile::tempdir().unwrap();
        let db = Database::init(tmp.path()).await.unwrap();
        let mut r = QuizDraftRow {
            id: "qd1".into(),
            space_id: "s1".into(),
            kwant_id: "k1".into(),
            title: "Friday".into(),
            spec: "{}".into(),
            state: "writing".into(),
            problems: "[]".into(),
            sources: "[]".into(),
            dry_run: None,
            budget: "{}".into(),
            pocket_code: None,
            created_at: 1,
            updated_at: 1,
        };
        db.quiz_draft_save(&r).await.unwrap();
        assert_eq!(db.quiz_draft_get("s1", "qd1").await.unwrap(), Some(r.clone()));
        assert_eq!(db.quiz_draft_get("s2", "qd1").await.unwrap(), None);
        r.state = "ready".into();
        r.updated_at = 2;
        r.kwant_id = "someone-else".into();
        db.quiz_draft_save(&r).await.unwrap();
        let got = db.quiz_draft_get("s1", "qd1").await.unwrap().unwrap();
        assert_eq!((got.state.as_str(), got.updated_at, got.kwant_id.as_str()), ("ready", 2, "k1"), "the writer never changes");
        r.state = "bogus".into();
        assert!(db.quiz_draft_save(&r).await.is_err());
    }
}
