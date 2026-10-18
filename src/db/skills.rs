//! Skills: short written procedures an agent follows for one kind of task. An agent proposes one
//! from something that worked (it is stored `pending`), the user reviews and enables it, and only
//! enabled skills ever reach a prompt. A skill is text only: it has no tool list and grants nothing.

use super::spaces::SpaceId;
use super::{Database, DbError};
use chrono::{DateTime, Utc};
use serde::Serialize;
use sqlx::FromRow;
use uuid::Uuid;

/// Longest skill name (a lowercase slug such as `release-checklist`).
pub const MAX_SKILL_NAME_CHARS: usize = 48;
/// Longest one-line description.
pub const MAX_SKILL_DESCRIPTION_CHARS: usize = 200;
/// Longest skill body.
pub const MAX_SKILL_BODY_CHARS: usize = 8000;
/// Most skills waiting for review per agent; a busy agent can't flood the review list.
pub const MAX_PENDING_PER_KWANT: i64 = 20;

/// "pending" (waiting for review), "enabled", "disabled", "rejected", or "superseded" (an
/// approved revision replaced it).
pub const SKILL_STATUSES: [&str; 5] = ["pending", "enabled", "disabled", "rejected", "superseded"];

#[derive(Debug, Clone, Serialize, FromRow, PartialEq)]
pub struct Skill {
    pub id: String,
    /// The agent it belongs to; `None` = every agent.
    pub kwant_id: Option<String>,
    pub name: String,
    pub description: String,
    pub body: String,
    pub status: String,
    /// "agent" (proposed with the skill_propose tool) or "user".
    pub source: String,
    /// The agent that proposed it.
    pub proposed_by: Option<String>,
    /// For a proposed revision: the skill it would replace when approved.
    pub replaces_id: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub reviewed_at: Option<DateTime<Utc>>,
    /// Shared skills (`kwant_id` = `None`) are shared within this space only.
    pub space_id: String,
}

#[derive(Debug, PartialEq)]
pub enum SkillError {
    Invalid(String),
    TooManyPending,
    NotFound,
    /// The action doesn't fit the skill's status (e.g. approving an enabled skill).
    Conflict(String),
    Db(String),
}

impl From<sqlx::Error> for SkillError {
    fn from(e: sqlx::Error) -> Self {
        SkillError::Db(e.to_string())
    }
}

impl From<DbError> for SkillError {
    fn from(e: DbError) -> Self {
        SkillError::Db(e.to_string())
    }
}

impl std::fmt::Display for SkillError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SkillError::Invalid(why) | SkillError::Conflict(why) => write!(f, "{why}"),
            SkillError::TooManyPending => write!(
                f,
                "there are already {MAX_PENDING_PER_KWANT} skills waiting for review; ask the user to review them first"
            ),
            SkillError::NotFound => write!(f, "skill not found"),
            SkillError::Db(_) => write!(f, "database error"),
        }
    }
}

/// The text a skill is made of, checked and cleaned.
#[derive(Debug, Clone, PartialEq)]
pub struct SkillText {
    pub name: String,
    pub description: String,
    pub body: String,
}

/// A name as stored: lowercase letters, digits and dashes, starting with a letter or digit.
pub fn clean_skill_name(raw: &str) -> Result<String, SkillError> {
    let name: String = raw
        .trim()
        .trim_start_matches('/')
        .to_lowercase()
        .chars()
        .map(|c| if c.is_whitespace() || c == '_' { '-' } else { c })
        .collect();
    let ok = !name.is_empty()
        && name.chars().count() <= MAX_SKILL_NAME_CHARS
        && name.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
        && !name.starts_with('-');
    if ok {
        Ok(name)
    } else {
        Err(SkillError::Invalid(format!(
            "name must be 1-{MAX_SKILL_NAME_CHARS} lowercase letters, digits or dashes (like release-checklist)"
        )))
    }
}

/// A body without a leading `---` front-matter block. Front matter is where other tools put
/// settings such as `allowed-tools`; kwnt ignores it entirely, so it is dropped rather than kept
/// as text that looks like it means something.
pub fn strip_front_matter(body: &str) -> &str {
    let trimmed = body.trim_start();
    let Some(rest) = trimmed.strip_prefix("---") else {
        return body;
    };
    if !rest.starts_with('\n') && !rest.starts_with("\r\n") {
        return body;
    }
    match rest.find("\n---") {
        Some(end) => {
            let after = &rest[end + 4..];
            after.trim_start_matches(['-']).trim_start_matches(['\r', '\n'])
        }
        None => body,
    }
}

pub fn clean_skill_text(name: &str, description: &str, body: &str) -> Result<SkillText, SkillError> {
    let name = clean_skill_name(name)?;
    let description = description.split_whitespace().collect::<Vec<_>>().join(" ");
    if description.is_empty() {
        return Err(SkillError::Invalid("description is required".into()));
    }
    if description.chars().count() > MAX_SKILL_DESCRIPTION_CHARS {
        return Err(SkillError::Invalid(format!(
            "description is longer than {MAX_SKILL_DESCRIPTION_CHARS} characters"
        )));
    }
    let body = strip_front_matter(body).trim().replace("\r\n", "\n");
    if body.is_empty() {
        return Err(SkillError::Invalid("body is required".into()));
    }
    if body.chars().count() > MAX_SKILL_BODY_CHARS {
        return Err(SkillError::Invalid(format!(
            "body is longer than {MAX_SKILL_BODY_CHARS} characters"
        )));
    }
    Ok(SkillText { name, description, body })
}

/// Changes to a skill; `None` leaves a field as it is.
#[derive(Debug, Default, Clone)]
pub struct SkillEdit {
    pub name: Option<String>,
    pub description: Option<String>,
    pub body: Option<String>,
    /// `Some(None)`: every agent; `Some(Some(id))`: that agent.
    pub kwant_id: Option<Option<String>>,
}

impl Database {
    pub(super) async fn migrate_skills(&self) -> Result<(), DbError> {
        for stmt in [
            "CREATE TABLE IF NOT EXISTS skills (
                id TEXT PRIMARY KEY,
                kwant_id TEXT,
                name TEXT NOT NULL,
                description TEXT NOT NULL,
                body TEXT NOT NULL,
                status TEXT NOT NULL CHECK (status IN ('pending', 'enabled', 'disabled', 'rejected', 'superseded')),
                source TEXT NOT NULL CHECK (source IN ('agent', 'user')),
                proposed_by TEXT,
                replaces_id TEXT,
                created_at TIMESTAMP NOT NULL,
                updated_at TIMESTAMP NOT NULL,
                reviewed_at TIMESTAMP
            )",
            "CREATE INDEX IF NOT EXISTS skills_by_status ON skills (status, kwant_id)",
            // One live skill per name for an agent (and one per name among the shared ones)
            "CREATE UNIQUE INDEX IF NOT EXISTS skills_live_name ON skills (IFNULL(kwant_id, ''), name)
                WHERE status IN ('enabled', 'disabled')",
        ] {
            sqlx::query(stmt).execute(&self.pool).await?;
        }
        Ok(())
    }

    pub async fn get_skill(&self, id: &str) -> Result<Option<Skill>, DbError> {
        Ok(sqlx::query_as::<_, Skill>("SELECT * FROM skills WHERE id = ?")
            .bind(id)
            .fetch_optional(&self.pool)
            .await?)
    }

    /// The space's skills, newest first; `kwant_id` also matches the shared ones.
    pub async fn list_skills(
        &self,
        space: &SpaceId,
        kwant_id: Option<&str>,
        status: Option<&str>,
    ) -> Result<Vec<Skill>, DbError> {
        Ok(sqlx::query_as::<_, Skill>(
            "SELECT * FROM skills
             WHERE space_id = ?3 AND (?1 IS NULL OR kwant_id = ?1 OR kwant_id IS NULL) AND (?2 IS NULL OR status = ?2)
             ORDER BY updated_at DESC LIMIT 500",
        )
        .bind(kwant_id)
        .bind(status)
        .bind(space.as_str())
        .fetch_all(&self.pool)
        .await?)
    }

    /// The enabled skills `kwant_id` may use: its own and the ones shared in its space, by name.
    pub async fn enabled_skills_for(&self, kwant_id: &str) -> Result<Vec<Skill>, DbError> {
        Ok(sqlx::query_as::<_, Skill>(
            "SELECT * FROM skills WHERE status = 'enabled' AND (kwant_id = ?1 OR kwant_id IS NULL)
             AND space_id = (SELECT space_id FROM kwants WHERE id = ?1) ORDER BY name",
        )
        .bind(kwant_id)
        .fetch_all(&self.pool)
        .await?)
    }

    /// The live (enabled or disabled) skill called `name` for this agent, if any.
    async fn live_skill(&self, space: &SpaceId, kwant_id: Option<&str>, name: &str) -> Result<Option<Skill>, DbError> {
        Ok(sqlx::query_as::<_, Skill>(
            "SELECT * FROM skills WHERE space_id = ? AND IFNULL(kwant_id, '') = IFNULL(?, '') AND name = ? AND status IN ('enabled', 'disabled')",
        )
        .bind(space.as_str())
        .bind(kwant_id)
        .bind(name)
        .fetch_optional(&self.pool)
        .await?)
    }

    /// An agent's proposal, stored `pending`. Proposing a name the agent already has makes a
    /// revision of that skill; proposing again before review rewrites the waiting proposal.
    pub async fn propose_skill(&self, kwant_id: &str, text: SkillText) -> Result<Skill, SkillError> {
        let now = Utc::now();
        let space = self.space_of_kwant(kwant_id).await?.ok_or(SkillError::NotFound)?;
        let replaces = self.live_skill(&space, Some(kwant_id), &text.name).await?.map(|s| s.id);
        let waiting = sqlx::query_as::<_, Skill>(
            "SELECT * FROM skills WHERE status = 'pending' AND kwant_id = ? AND name = ? ORDER BY created_at DESC LIMIT 1",
        )
        .bind(kwant_id)
        .bind(&text.name)
        .fetch_optional(&self.pool)
        .await?;
        if let Some(p) = waiting {
            sqlx::query("UPDATE skills SET description = ?, body = ?, replaces_id = ?, updated_at = ? WHERE id = ?")
                .bind(&text.description)
                .bind(&text.body)
                .bind(&replaces)
                .bind(now)
                .bind(&p.id)
                .execute(&self.pool)
                .await?;
            return self.get_skill(&p.id).await?.ok_or(SkillError::NotFound);
        }
        let (count,): (i64,) = sqlx::query_as("SELECT COUNT(*) FROM skills WHERE status = 'pending' AND kwant_id = ?")
            .bind(kwant_id)
            .fetch_one(&self.pool)
            .await?;
        if count >= MAX_PENDING_PER_KWANT {
            return Err(SkillError::TooManyPending);
        }
        let id = Uuid::new_v4().to_string();
        sqlx::query(
            "INSERT INTO skills (id, kwant_id, name, description, body, status, source, proposed_by, replaces_id, created_at, updated_at, space_id)
             VALUES (?, ?, ?, ?, ?, 'pending', 'agent', ?, ?, ?, ?, ?)",
        )
        .bind(&id)
        .bind(kwant_id)
        .bind(&text.name)
        .bind(&text.description)
        .bind(&text.body)
        .bind(kwant_id)
        .bind(&replaces)
        .bind(now)
        .bind(now)
        .bind(space.as_str())
        .execute(&self.pool)
        .await?;
        self.get_skill(&id).await?.ok_or(SkillError::NotFound)
    }

    /// A skill the user wrote; it is enabled at once (the user is the reviewer). `kwant_id` must
    /// be one of the space's kwants.
    pub async fn create_user_skill(
        &self,
        space: &SpaceId,
        kwant_id: Option<&str>,
        text: SkillText,
    ) -> Result<Skill, SkillError> {
        self.check_skill_owner(space, kwant_id).await?;
        if self.live_skill(space, kwant_id, &text.name).await?.is_some() {
            return Err(SkillError::Conflict(format!("a skill called {} already exists", text.name)));
        }
        let id = Uuid::new_v4().to_string();
        let now = Utc::now();
        sqlx::query(
            "INSERT INTO skills (id, kwant_id, name, description, body, status, source, created_at, updated_at, reviewed_at, space_id)
             VALUES (?, ?, ?, ?, ?, 'enabled', 'user', ?, ?, ?, ?)",
        )
        .bind(&id)
        .bind(kwant_id)
        .bind(&text.name)
        .bind(&text.description)
        .bind(&text.body)
        .bind(now)
        .bind(now)
        .bind(now)
        .bind(space.as_str())
        .execute(&self.pool)
        .await?;
        self.get_skill(&id).await?.ok_or(SkillError::NotFound)
    }

    /// A skill's owner, when it has one, must be a kwant in the skill's space.
    async fn check_skill_owner(&self, space: &SpaceId, kwant_id: Option<&str>) -> Result<(), SkillError> {
        match kwant_id {
            Some(id) if !self.kwants_in_space(space, &[id]).await? => {
                Err(SkillError::Invalid("that bot is in another space".into()))
            }
            _ => Ok(()),
        }
    }

    /// Enables a pending skill (with the user's edits, if any); the skill it revises is superseded.
    pub async fn approve_skill(&self, id: &str, edit: &SkillEdit) -> Result<Skill, SkillError> {
        let skill = self.get_skill(id).await?.ok_or(SkillError::NotFound)?;
        if skill.status != "pending" {
            return Err(SkillError::Conflict(format!("only a pending skill can be approved; this one is {}", skill.status)));
        }
        let text = merged_text(&skill, edit)?;
        let kwant_id = edit.kwant_id.clone().unwrap_or(skill.kwant_id.clone());
        self.check_skill_owner(&SpaceId::new(&skill.space_id), kwant_id.as_deref()).await?;
        let now = Utc::now();
        let mut tx = self.pool.begin().await?;
        // The revised skill steps aside; any other live skill with the target name blocks approval
        if let Some(old) = &skill.replaces_id {
            sqlx::query("UPDATE skills SET status = 'superseded', updated_at = ? WHERE id = ? AND status IN ('enabled', 'disabled')")
                .bind(now)
                .bind(old)
                .execute(&mut *tx)
                .await?;
        }
        let clash: Option<(String,)> = sqlx::query_as(
            "SELECT id FROM skills WHERE space_id = ? AND IFNULL(kwant_id, '') = IFNULL(?, '') AND name = ? AND status IN ('enabled', 'disabled') AND id != ?",
        )
        .bind(&skill.space_id)
        .bind(&kwant_id)
        .bind(&text.name)
        .bind(id)
        .fetch_optional(&mut *tx)
        .await?;
        if clash.is_some() {
            return Err(SkillError::Conflict(format!("a skill called {} already exists; rename this one or disable that one first", text.name)));
        }
        sqlx::query(
            "UPDATE skills SET kwant_id = ?, name = ?, description = ?, body = ?, status = 'enabled', updated_at = ?, reviewed_at = ? WHERE id = ?",
        )
        .bind(&kwant_id)
        .bind(&text.name)
        .bind(&text.description)
        .bind(&text.body)
        .bind(now)
        .bind(now)
        .bind(id)
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        self.get_skill(id).await?.ok_or(SkillError::NotFound)
    }

    /// Sets a status: reject a pending skill, or enable / disable a reviewed one.
    pub async fn set_skill_status(&self, id: &str, status: &str) -> Result<Skill, SkillError> {
        let skill = self.get_skill(id).await?.ok_or(SkillError::NotFound)?;
        let allowed = match status {
            "rejected" => skill.status == "pending",
            "enabled" | "disabled" => matches!(skill.status.as_str(), "enabled" | "disabled"),
            _ => false,
        };
        if !allowed {
            return Err(SkillError::Conflict(format!("a {} skill can't become {status}", skill.status)));
        }
        let now = Utc::now();
        sqlx::query("UPDATE skills SET status = ?, updated_at = ?, reviewed_at = ? WHERE id = ?")
            .bind(status)
            .bind(now)
            .bind(now)
            .bind(id)
            .execute(&self.pool)
            .await?;
        self.get_skill(id).await?.ok_or(SkillError::NotFound)
    }

    /// The user's edit of a skill's text or owner; its status stays as it is.
    pub async fn edit_skill(&self, id: &str, edit: &SkillEdit) -> Result<Skill, SkillError> {
        let skill = self.get_skill(id).await?.ok_or(SkillError::NotFound)?;
        if matches!(skill.status.as_str(), "rejected" | "superseded") {
            return Err(SkillError::Conflict(format!("a {} skill can't be edited", skill.status)));
        }
        let text = merged_text(&skill, edit)?;
        let kwant_id = edit.kwant_id.clone().unwrap_or(skill.kwant_id.clone());
        self.check_skill_owner(&SpaceId::new(&skill.space_id), kwant_id.as_deref()).await?;
        let res = sqlx::query("UPDATE skills SET kwant_id = ?, name = ?, description = ?, body = ?, updated_at = ? WHERE id = ?")
            .bind(&kwant_id)
            .bind(&text.name)
            .bind(&text.description)
            .bind(&text.body)
            .bind(Utc::now())
            .bind(id)
            .execute(&self.pool)
            .await;
        match res {
            Ok(_) => self.get_skill(id).await?.ok_or(SkillError::NotFound),
            Err(sqlx::Error::Database(e)) if e.is_unique_violation() => {
                Err(SkillError::Conflict(format!("a skill called {} already exists", text.name)))
            }
            Err(e) => Err(e.into()),
        }
    }

    pub async fn delete_skill(&self, id: &str) -> Result<bool, DbError> {
        let res = sqlx::query("DELETE FROM skills WHERE id = ?").bind(id).execute(&self.pool).await?;
        Ok(res.rows_affected() > 0)
    }
}

fn merged_text(skill: &Skill, edit: &SkillEdit) -> Result<SkillText, SkillError> {
    clean_skill_text(
        edit.name.as_deref().unwrap_or(&skill.name),
        edit.description.as_deref().unwrap_or(&skill.description),
        edit.body.as_deref().unwrap_or(&skill.body),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_are_slugs() {
        assert_eq!(clean_skill_name(" Release Checklist ").unwrap(), "release-checklist");
        assert_eq!(clean_skill_name("/deploy_vps").unwrap(), "deploy-vps");
        assert!(clean_skill_name("").is_err());
        assert!(clean_skill_name("../etc").is_err());
        assert!(clean_skill_name("-x").is_err());
        assert!(clean_skill_name(&"a".repeat(MAX_SKILL_NAME_CHARS + 1)).is_err());
    }

    #[test]
    fn front_matter_is_dropped() {
        let body = "---\nname: x\nallowed-tools: Bash(*)\n---\n1. Run tests\n2. Tag";
        assert_eq!(strip_front_matter(body), "1. Run tests\n2. Tag");
        assert_eq!(strip_front_matter("no front matter"), "no front matter");
        assert_eq!(strip_front_matter("---\nunterminated"), "---\nunterminated");
        let t = clean_skill_text("x", "d", body).unwrap();
        assert!(!t.body.contains("allowed-tools"));
    }

    #[test]
    fn limits() {
        assert!(clean_skill_text("x", "", "b").is_err());
        assert!(clean_skill_text("x", "d", "  ").is_err());
        assert!(clean_skill_text("x", "d", &"b".repeat(MAX_SKILL_BODY_CHARS + 1)).is_err());
        assert!(clean_skill_text("x", &"d".repeat(MAX_SKILL_DESCRIPTION_CHARS + 1), "b").is_err());
    }
}
