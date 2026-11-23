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
        updated_at INTEGER NOT NULL,
        own_keys TEXT NOT NULL DEFAULT '[]'
    )",
    "CREATE INDEX IF NOT EXISTS quiz_drafts_space ON quiz_drafts (space_id, updated_at)",
];

const COLS: &str = "id, space_id, kwant_id, title, spec, state, problems, sources, dry_run, budget, pocket_code, created_at, updated_at, own_keys";

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
    /// pa4c: JSON array of `quizmaster::source_check::own_key`s, the questions the person wrote or
    /// changed through the draft edit API. They aren't source-checked, and a bot's update keeps them.
    pub own_keys: String,
}

impl Database {
    pub(super) async fn migrate_quiz_drafts(&self) -> Result<(), DbError> {
        for sql in TABLES {
            sqlx::query(sql).execute(&self.pool).await?;
        }
        // pa4c: the person's own questions came after the first drafts
        let has: Option<(String,)> = sqlx::query_as("SELECT name FROM pragma_table_info('quiz_drafts') WHERE name = 'own_keys'").fetch_optional(&self.pool).await?;
        if has.is_none() {
            sqlx::query("ALTER TABLE quiz_drafts ADD COLUMN own_keys TEXT NOT NULL DEFAULT '[]'").execute(&self.pool).await?;
        }
        Ok(())
    }

    pub async fn quiz_draft_save(&self, r: &QuizDraftRow) -> Result<(), DbError> {
        self.quiz_draft_save_if(r, None).await.map(|_| ())
    }

    /// Saves `r` only while the stored draft's `updated_at` is still `base` (pa4c: the person's
    /// edit doesn't overwrite a bot's update that landed in between); `None` always saves.
    /// Returns whether it saved.
    pub async fn quiz_draft_save_if(&self, r: &QuizDraftRow, base: Option<i64>) -> Result<bool, DbError> {
        let guard = if base.is_some() { " WHERE quiz_drafts.updated_at = ?" } else { "" };
        let q = format!(
            "INSERT INTO quiz_drafts ({COLS}) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
             ON CONFLICT(id) DO UPDATE SET title = excluded.title, spec = excluded.spec, state = excluded.state,
               problems = excluded.problems, sources = excluded.sources, dry_run = excluded.dry_run, budget = excluded.budget,
               pocket_code = excluded.pocket_code, updated_at = excluded.updated_at, own_keys = excluded.own_keys{guard}"
        );
        let mut query = sqlx::query(&q)
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
        .bind(&r.own_keys);
        if let Some(b) = base {
            query = query.bind(b);
        }
        Ok(query.execute(&self.pool).await?.rows_affected() > 0)
    }

    /// A draft in this space.
    pub async fn quiz_draft_get(&self, space_id: &str, id: &str) -> Result<Option<QuizDraftRow>, DbError> {
        Ok(sqlx::query_as(&format!("SELECT {COLS} FROM quiz_drafts WHERE space_id = ? AND id = ?")).bind(space_id).bind(id).fetch_optional(&self.pool).await?)
    }

    /// The draft a pocket was opened from (pa3b: the live judge reads its allowance here).
    pub async fn quiz_draft_by_pocket(&self, pocket_code: &str) -> Result<Option<QuizDraftRow>, DbError> {
        Ok(sqlx::query_as(&format!("SELECT {COLS} FROM quiz_drafts WHERE pocket_code = ? ORDER BY updated_at DESC LIMIT 1"))
            .bind(pocket_code)
            .fetch_optional(&self.pool)
            .await?)
    }

    /// pa3b (pa3 qa F3): the judge calls live play has used, written into the budget JSON so the
    /// card counts them and a restart resumes the allowance. Never lowers the stored count (two
    /// judging rounds may finish out of order). Changes nothing else in the row.
    pub async fn quiz_draft_set_judge_calls(&self, id: &str, calls: u32) -> Result<bool, DbError> {
        let r = sqlx::query(
            "UPDATE quiz_drafts SET budget = json_set(CASE WHEN json_valid(budget) THEN budget ELSE '{}' END, '$.judge_calls',
               MAX(?, COALESCE(CASE WHEN json_valid(budget) THEN json_extract(budget, '$.judge_calls') END, 0)))
             WHERE id = ?",
        )
        .bind(i64::from(calls))
        .bind(id)
        .execute(&self.pool)
        .await?;
        Ok(r.rows_affected() > 0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn judge_calls_go_into_the_budget_and_the_pocket_finds_its_draft() {
        let tmp = tempfile::tempdir().unwrap();
        let db = Database::init(tmp.path()).await.unwrap();
        let r = QuizDraftRow {
            id: "qd9".into(),
            space_id: "s1".into(),
            kwant_id: "k1".into(),
            title: "Friday".into(),
            spec: "{}".into(),
            state: "open".into(),
            problems: "[]".into(),
            sources: "[]".into(),
            dry_run: None,
            budget: r#"{"writing_calls":2,"source_fetches":4,"judge_allowance":16}"#.into(),
            pocket_code: Some("ABCD".into()),
            created_at: 1,
            updated_at: 1,
        };
        db.quiz_draft_save(&r).await.unwrap();
        assert_eq!(db.quiz_draft_by_pocket("ABCD").await.unwrap().map(|r| r.id), Some("qd9".into()));
        assert_eq!(db.quiz_draft_by_pocket("NOPE").await.unwrap(), None);
        assert!(db.quiz_draft_set_judge_calls("qd9", 3).await.unwrap());
        assert!(!db.quiz_draft_set_judge_calls("nope", 3).await.unwrap());
        assert!(db.quiz_draft_set_judge_calls("qd9", 2).await.unwrap(), "a late, lower count doesn't lower it");
        let b: serde_json::Value = serde_json::from_str(&db.quiz_draft_get("s1", "qd9").await.unwrap().unwrap().budget).unwrap();
        assert_eq!(b, serde_json::json!({"writing_calls": 2, "source_fetches": 4, "judge_allowance": 16, "judge_calls": 3}));
    }

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
            own_keys: "[]".into(),
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
        // pa4c: a save based on an older draft does nothing
        r.state = "checking".into();
        r.own_keys = r#"["abc"]"#.into();
        assert!(!db.quiz_draft_save_if(&r, Some(1)).await.unwrap());
        assert_eq!(db.quiz_draft_get("s1", "qd1").await.unwrap().unwrap().state, "ready");
        r.updated_at = 3;
        assert!(db.quiz_draft_save_if(&r, Some(2)).await.unwrap());
        let got = db.quiz_draft_get("s1", "qd1").await.unwrap().unwrap();
        assert_eq!((got.state.as_str(), got.own_keys.as_str()), ("checking", r#"["abc"]"#));
        r.state = "bogus".into();
        assert!(db.quiz_draft_save(&r).await.is_err());
    }
}
