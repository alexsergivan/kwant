//! Making a whole team in one go (next-077 spec T3/T4, D6): bots, team, team chat, skills and a
//! paused routine in one transaction, logged as one bot action with one Undo. The plan is built
//! by `server::teams` from a template (`teams::templates`) or a discovery suggestion
//! (`teams::discover`); this module only writes it.

use super::spaces::SpaceId;
use super::{Database, DbError};
use crate::teams::names;
use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use uuid::Uuid;

/// How long Undo stays available after a create (Q-T1).
pub const UNDO_HOURS: i64 = 24;
/// How long an `Idempotency-Key` replays the first answer.
pub const IDEMPOTENCY_HOURS: i64 = 24;
/// The bot action a create is logged as.
pub const ACTION: &str = "team_template";

/// What to make. Every text in it is already the server's own (template data, preset text,
/// discovery facts, or a draft from the person's words that the server checked and kept with its
/// suggestion); nothing here comes from the request except names, a handle and preset ids.
#[derive(Debug, Clone)]
pub struct Plan {
    pub template_id: Option<String>,
    pub suggestion_id: Option<String>,
    pub idempotency_key: Option<String>,
    /// SHA-256 of the request body, kept with the key: the same key with another body is
    /// refused rather than replayed (L-IDEM).
    pub request_sha256: Option<String>,
    pub target: Target,
    pub bots: Vec<PlanBot>,
    pub skills: Vec<PlanSkill>,
    pub routine: Option<PlanRoutine>,
    /// The person's time zone for the routine's cron.
    pub tz: String,
}

#[derive(Debug, Clone)]
pub enum Target {
    /// A new team called `handle`, with the chat `team-<handle>`.
    New {
        handle: String,
        icon: String,
        description: String,
        instructions: String,
        workspace_path: Option<String>,
        repo_url: Option<String>,
    },
    /// Add the bots to this team (spec D, A18). The team keeps its lead and instructions.
    Existing { team_id: String },
}

#[derive(Debug, Clone)]
pub struct PlanBot {
    /// The template slot or the suggestion row key; the response and errors use it.
    pub slot: String,
    /// Where a name error points: `names.<slot>` or `edits.rows.<key>.name`.
    pub field: String,
    pub preset: String,
    pub lead: bool,
    pub name: String,
    pub title: String,
    pub avatar: String,
    pub persona: String,
    pub soul: String,
    pub covers: Vec<String>,
    pub avoids: Vec<String>,
    /// A live bot of this space that joins unchanged instead of a new one (D6, A16).
    pub existing: Option<String>,
}

#[derive(Debug, Clone)]
pub struct PlanSkill {
    pub owner_slot: String,
    pub name: String,
    pub description: String,
    pub body: String,
}

#[derive(Debug, Clone)]
pub struct PlanRoutine {
    pub owner_slot: String,
    pub name: String,
    pub instructions: String,
    pub cron: String,
}

/// Where the create can be made to fail, to prove nothing is left behind (Q-T8). Production
/// callers pass `None`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Step {
    /// After the n-th bot (1-based).
    Bot(usize),
    Team,
    Skills,
    Routine,
    Action,
}

/// What a create made; kept as JSON on the creation row and in the action's `after`.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct Created {
    pub team_id: String,
    pub channel_id: Option<String>,
    pub new_team: bool,
    /// `#general`, when this create made it (a new space's first bots).
    pub general_id: Option<String>,
    pub template_id: Option<String>,
    pub bots: Vec<CreatedBot>,
    pub skills: Vec<CreatedSkill>,
    pub routine: Option<CreatedRoutine>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CreatedBot {
    pub slot: String,
    pub id: String,
    pub name: String,
    pub preset: String,
    pub lead: bool,
    pub existing: bool,
    /// For an existing team: whether this bot was added by the create (a reused bot that was
    /// already a member is not removed by Undo).
    #[serde(default)]
    pub joined: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CreatedSkill {
    pub id: String,
    pub name: String,
    pub owner_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CreatedRoutine {
    pub id: String,
    pub name: String,
    pub enabled: bool,
}

/// One logged create.
#[derive(Debug, Clone)]
pub struct Creation {
    pub id: String,
    pub space_id: String,
    pub team_id: String,
    pub action_id: String,
    pub template_id: Option<String>,
    pub created: Created,
    pub status: String,
    pub created_at: DateTime<Utc>,
    /// The body hash stored with its `Idempotency-Key`; `None` on rows from before it was kept.
    pub request_sha256: Option<String>,
}

impl Creation {
    pub fn undo_until(&self) -> DateTime<Utc> {
        self.created_at + Duration::hours(UNDO_HOURS)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClashKind {
    Handle,
    Name,
}

/// One taken name or handle. `field` is the request field (`names.<slot>`, `edits.rows.<key>.name`);
/// for the handle it is empty and the route fills in `handle` or `edits.handle`.
#[derive(Debug, Clone)]
pub struct Clash {
    pub kind: ClashKind,
    pub field: String,
    pub wanted: String,
    pub suggestion: String,
}

#[derive(Debug, thiserror::Error)]
pub enum CreateError {
    /// Every taken name and handle of the request, the handle first; never empty.
    #[error("{} name(s) or handle taken", .0.len())]
    Clash(Vec<Clash>),
    #[error("{field}: not a live bot of this space")]
    BadExisting { field: String },
    #[error("the team is gone")]
    TeamNotFound,
    #[error(transparent)]
    Db(#[from] DbError),
}

impl From<sqlx::Error> for CreateError {
    fn from(e: sqlx::Error) -> Self {
        CreateError::Db(e.into())
    }
}

impl From<serde_json::Error> for CreateError {
    fn from(e: serde_json::Error) -> Self {
        CreateError::Db(e.into())
    }
}

#[derive(Debug, thiserror::Error)]
pub enum UndoError {
    #[error("Undo was only available for 24 hours after the team was made")]
    Expired,
    #[error("a bot this made now leads another team or chat")]
    LeadsElsewhere(Vec<Blocked>),
    #[error("the team chat has history and no other bot in this space can keep it")]
    NoKeeper,
    #[error("not a team made from a template")]
    NotFound,
    #[error(transparent)]
    Db(#[from] DbError),
}

impl From<serde_json::Error> for UndoError {
    fn from(e: serde_json::Error) -> Self {
        UndoError::Db(e.into())
    }
}

impl super::bot_actions::SqliteCause for UndoError {
    fn sqlite_code(&self) -> Option<i32> {
        match self {
            UndoError::Db(e) => super::bot_actions::sqlite_code(e),
            _ => None,
        }
    }
}

impl From<sqlx::Error> for UndoError {
    fn from(e: sqlx::Error) -> Self {
        UndoError::Db(e.into())
    }
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct Blocked {
    pub bot: String,
    pub leads: String,
}

/// What `GET /api/teams/:id/template` shows before Undo (api.md §6).
#[derive(Debug, Clone, Serialize)]
pub struct TemplateInfo {
    pub template_id: Option<String>,
    pub action_id: String,
    pub created_at: DateTime<Utc>,
    pub undo_until: DateTime<Utc>,
    pub can_undo: bool,
    pub blocked: Vec<Blocked>,
    pub removes: Removes,
    pub chat_messages: i64,
    /// What Undo will rename the team chat to (`archived-team-<handle>`, with `-2`, `-3`… when
    /// that is taken), when it keeps the chat because it has messages. `None` when Undo deletes
    /// the chat or leaves it alone.
    pub archived_chat: Option<String>,
    pub also_in: Vec<AlsoIn>,
    pub edited_skills: Vec<String>,
    /// Needs (T5) the server can see are done. A `composer_prompt` need, such as Sales'
    /// `what_you_sell` (A5), is done once the person has posted in the team chat.
    pub needs_done: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Removes {
    pub bots: Vec<String>,
    pub skills: i64,
    pub routines: i64,
    pub handle: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct AlsoIn {
    pub bot: String,
    pub team: String,
}

/// The Undo window and the suggestion expiry read the one shared test clock (Q-X1, api.md §7).
pub use crate::clock::now;

type Tx = sqlx::Transaction<'static, sqlx::Sqlite>;

async fn name_taken(tx: &mut Tx, space: &SpaceId, name: &str) -> Result<bool, sqlx::Error> {
    let (n,): (i64,) = sqlx::query_as(
        "SELECT (SELECT COUNT(*) FROM kwants WHERE space_id = ?1 AND name = ?2 COLLATE NOCASE)
              + (SELECT COUNT(*) FROM hire_proposals WHERE space_id = ?1 AND status = 'pending' AND name = ?2 COLLATE NOCASE)",
    )
    .bind(space.as_str())
    .bind(name)
    .fetch_one(&mut **tx)
    .await?;
    Ok(n > 0)
}

async fn handle_taken(tx: &mut Tx, space: &SpaceId, handle: &str) -> Result<bool, sqlx::Error> {
    let (n,): (i64,) = sqlx::query_as(
        "SELECT (SELECT COUNT(*) FROM teams WHERE space_id = ?1 AND name = ?2 COLLATE NOCASE)
              + (SELECT COUNT(*) FROM channels WHERE space_id = ?1 AND name = 'team-' || ?2 COLLATE NOCASE)",
    )
    .bind(space.as_str())
    .bind(handle)
    .fetch_one(&mut **tx)
    .await?;
    Ok(n > 0)
}

fn seam(fail_after: Option<Step>, at: Step) -> Result<(), CreateError> {
    if fail_after == Some(at) {
        return Err(CreateError::Db(DbError::Conflict(format!("test seam: failed after {at:?}"))));
    }
    Ok(())
}

fn parse_members(json: &str) -> Vec<String> {
    serde_json::from_str(json).unwrap_or_default()
}

impl Database {
    pub(super) async fn migrate_team_templates(&self) -> Result<(), DbError> {
        for stmt in [
            "CREATE TABLE IF NOT EXISTS team_creations (
                id TEXT PRIMARY KEY,
                space_id TEXT NOT NULL,
                team_id TEXT NOT NULL,
                action_id TEXT NOT NULL,
                template_id TEXT,
                suggestion_id TEXT,
                idempotency_key TEXT,
                request_sha256 TEXT,
                created TEXT NOT NULL,
                status TEXT NOT NULL CHECK (status IN ('applied', 'undone')),
                created_at TIMESTAMP NOT NULL,
                undone_at TIMESTAMP
            )",
            "CREATE INDEX IF NOT EXISTS team_creations_by_team ON team_creations (team_id)",
            "CREATE INDEX IF NOT EXISTS team_creations_by_action ON team_creations (action_id)",
            "CREATE INDEX IF NOT EXISTS team_creations_by_key ON team_creations (space_id, idempotency_key)",
        ] {
            sqlx::query(stmt).execute(&self.pool).await?;
        }
        // Databases made by an earlier 0.7.7 build lack the body hash.
        let (has,): (i64,) = sqlx::query_as("SELECT COUNT(*) FROM pragma_table_info('team_creations') WHERE name = 'request_sha256'")
            .fetch_one(&self.pool)
            .await?;
        if has == 0 {
            sqlx::query("ALTER TABLE team_creations ADD COLUMN request_sha256 TEXT").execute(&self.pool).await?;
        }
        Ok(())
    }

    /// The names, of `wanted`, that the space already uses (bots ignoring case, pending hire
    /// proposals) and the next free one for each (`Drafter2`). Read-only; for the preview (T9).
    pub async fn free_bot_name(&self, space: &SpaceId, wanted: &str, also_taken: &HashSet<String>) -> Result<String, DbError> {
        let mut tx = self.pool.begin().await?;
        let mut taken = HashSet::new();
        // At most a few dozen probes; every candidate is checked against the database once.
        let mut candidate = wanted.to_string();
        loop {
            let busy = also_taken.contains(&candidate.to_lowercase()) || name_taken(&mut tx, space, &candidate).await?;
            if !busy {
                break;
            }
            taken.insert(candidate.to_lowercase());
            candidate = names::next_free(wanted, |n| taken.contains(&n.to_lowercase()));
        }
        tx.rollback().await?;
        Ok(candidate)
    }

    /// The next free handle: `inbox`, else `inbox2`, …; a live team or a `team-<handle>` chat holds it.
    pub async fn free_team_handle(&self, space: &SpaceId, wanted: &str) -> Result<String, DbError> {
        let mut tx = self.pool.begin().await?;
        let mut taken = HashSet::new();
        let mut candidate = wanted.to_string();
        while handle_taken(&mut tx, space, &candidate).await? {
            taken.insert(candidate.clone());
            candidate = names::next_free(wanted, |n| taken.contains(n));
        }
        tx.rollback().await?;
        Ok(candidate)
    }

    /// Whether a live team of this space was made from `template_id` (catalogue `used_in_space`),
    /// including the seeded starter team.
    pub async fn template_used_in_space(&self, space: &SpaceId, template_id: &str) -> Result<bool, DbError> {
        let (n,): (i64,) = sqlx::query_as(
            "SELECT COUNT(*) FROM team_creations c JOIN teams t ON t.id = c.team_id
             WHERE c.space_id = ? AND c.template_id = ? AND c.status = 'applied' AND c.created LIKE '%\"new_team\":true%'",
        )
        .bind(space.as_str())
        .bind(template_id)
        .fetch_one(&self.pool)
        .await?;
        if n > 0 {
            return Ok(true);
        }
        // The seeded starter team (a fresh install's @helpers) has no creation row: it counts when a
        // team here is led by a bot that still has the template lead's own persona.
        if template_id == super::STARTER_TEMPLATE {
            let (seeded,): (i64,) = sqlx::query_as(
                "SELECT COUNT(*) FROM teams t JOIN kwants k ON k.id = t.lead_kwant_id WHERE t.space_id = ? AND k.persona_prompt = ?",
            )
            .bind(space.as_str())
            .bind(&super::starter_template().lead().persona)
            .fetch_one(&self.pool)
            .await?;
            return Ok(seeded > 0);
        }
        Ok(false)
    }

    /// The create answered for `key` in the last 24 hours, if any (Idempotency-Key).
    pub async fn team_creation_by_key(&self, space: &SpaceId, key: &str) -> Result<Option<Creation>, DbError> {
        let since = now() - Duration::hours(IDEMPOTENCY_HOURS);
        let row: Option<CreationRow> = sqlx::query_as(
            "SELECT * FROM team_creations WHERE space_id = ? AND idempotency_key = ? AND status = 'applied' AND created_at > ? ORDER BY created_at DESC LIMIT 1",
        )
        .bind(space.as_str())
        .bind(key)
        .bind(since)
        .fetch_optional(&self.pool)
        .await?;
        row.map(Creation::try_from).transpose()
    }

    pub async fn team_creation_by_action(&self, action_id: &str) -> Result<Option<Creation>, DbError> {
        let row: Option<CreationRow> =
            sqlx::query_as("SELECT * FROM team_creations WHERE action_id = ?").bind(action_id).fetch_optional(&self.pool).await?;
        row.map(Creation::try_from).transpose()
    }

    /// The latest applied create that made team `team_id` or added bots to it.
    pub async fn team_creation_for_team(&self, team_id: &str) -> Result<Option<Creation>, DbError> {
        let row: Option<CreationRow> = sqlx::query_as(
            "SELECT * FROM team_creations WHERE team_id = ? AND status = 'applied' ORDER BY created_at DESC, rowid DESC LIMIT 1",
        )
        .bind(team_id)
        .fetch_optional(&self.pool)
        .await?;
        row.map(Creation::try_from).transpose()
    }

    /// Makes everything in `plan` in one transaction, or nothing (T3). Names and the handle are
    /// checked again inside the transaction, so a clash that appeared since the preview is a
    /// [`CreateError::Clash`] naming every taken name and handle with the next free one.
    pub async fn create_team_from_plan(&self, space: &SpaceId, plan: &Plan, fail_after: Option<Step>) -> Result<Creation, CreateError> {
        let mut tx = self.begin_write().await?;
        match create_in(&mut tx, space, plan, fail_after).await {
            Ok(id) => {
                tx.commit().await?;
                let row: CreationRow = sqlx::query_as("SELECT * FROM team_creations WHERE id = ?").bind(&id).fetch_one(&self.pool).await?;
                Ok(Creation::try_from(row)?)
            }
            Err(e) => {
                tx.rollback().await?;
                Err(e)
            }
        }
    }

    /// What Undo of `creation` would do, and whether it can (api.md §6).
    pub async fn team_template_info(&self, c: &Creation) -> Result<TemplateInfo, DbError> {
        let mut conn = self.pool.acquire().await?;
        let new_ids: Vec<&str> = c.created.bots.iter().filter(|b| !b.existing).map(|b| b.id.as_str()).collect();
        let blocked = blocked_by(&mut conn, c, &new_ids).await?;
        let mut bots = Vec::new();
        let (mut skills, mut routines) = (0, 0);
        let mut also_in = Vec::new();
        for id in &new_ids {
            let Some((name,)): Option<(String,)> = sqlx::query_as("SELECT name FROM kwants WHERE id = ?").bind(id).fetch_optional(&mut *conn).await?
            else {
                continue;
            };
            let (s,): (i64,) = sqlx::query_as("SELECT COUNT(*) FROM skills WHERE kwant_id = ?").bind(id).fetch_one(&mut *conn).await?;
            let (r,): (i64,) = sqlx::query_as("SELECT COUNT(*) FROM routines WHERE kwant_id = ?").bind(id).fetch_one(&mut *conn).await?;
            skills += s;
            routines += r;
            let teams: Vec<(String,)> =
                sqlx::query_as("SELECT name FROM teams WHERE id != ? AND space_id = ? AND member_kwants LIKE '%\"' || ? || '\"%' ORDER BY name")
                    .bind(&c.team_id)
                    .bind(&c.space_id)
                    .bind(id)
                    .fetch_all(&mut *conn)
                    .await?;
            also_in.extend(teams.into_iter().map(|(team,)| AlsoIn { bot: name.clone(), team }));
            bots.push(name);
        }
        let handle: Option<(String,)> = if c.created.new_team {
            sqlx::query_as("SELECT name FROM teams WHERE id = ?").bind(&c.team_id).fetch_optional(&mut *conn).await?
        } else {
            None
        };
        let chat_messages = match (&c.created.channel_id, c.created.new_team) {
            (Some(ch), true) => {
                sqlx::query_as::<_, (i64,)>("SELECT COUNT(*) FROM messages WHERE channel_id = ?").bind(ch).fetch_one(&mut *conn).await?.0
            }
            _ => 0,
        };
        // The same choice `undo_in` makes: a new team's chat with messages, still `team-…`.
        let archived_chat = match (&c.created.channel_id, chat_messages > 0) {
            (Some(ch), true) => {
                let name: Option<(String,)> = sqlx::query_as("SELECT name FROM channels WHERE id = ?").bind(ch).fetch_optional(&mut *conn).await?;
                match name {
                    Some((name,)) if name.starts_with("team-") => Some(super::archived_chat_name(&mut conn, &c.space_id, &name).await?),
                    _ => None,
                }
            }
            _ => None,
        };
        let person_posted = match &c.created.channel_id {
            Some(ch) => {
                sqlx::query_as::<_, (i64,)>("SELECT COUNT(*) FROM messages WHERE channel_id = ? AND sender_role = 'user'")
                    .bind(ch)
                    .fetch_one(&mut *conn)
                    .await?
                    .0
                    > 0
            }
            None => false,
        };
        let needs_done = c
            .template_id
            .as_deref()
            .and_then(crate::teams::templates::get)
            .map(|t| t.needs.iter().filter(|n| n.opens == "composer_prompt" && person_posted).map(|n| n.id.clone()).collect())
            .unwrap_or_default();
        let mut edited_skills = Vec::new();
        for s in &c.created.skills {
            let edited: Option<(String,)> = sqlx::query_as("SELECT name FROM skills WHERE id = ? AND updated_at > ?")
                .bind(&s.id)
                .bind(c.created_at + Duration::seconds(1))
                .fetch_optional(&mut *conn)
                .await?;
            edited_skills.extend(edited.map(|(n,)| n));
        }
        let expired = now() > c.undo_until();
        Ok(TemplateInfo {
            template_id: c.template_id.clone(),
            action_id: c.action_id.clone(),
            created_at: c.created_at,
            undo_until: c.undo_until(),
            can_undo: c.status == "applied" && !expired && blocked.is_empty(),
            blocked,
            removes: Removes { bots, skills, routines, handle: handle.map(|(h,)| h) },
            chat_messages,
            archived_chat,
            also_in,
            edited_skills,
            needs_done,
        })
    }

    /// Takes back everything `action_id`'s create made, in one transaction (T4): the new bots
    /// (with their skills, routines and history), the team, and its chat (deleted when empty,
    /// else archived as a team delete does). Reused bots only leave the team. The bot action's
    /// own status is the caller's (`server::bot_actions::resolve`).
    pub async fn undo_team_creation(&self, action_id: &str) -> Result<Created, UndoError> {
        let c = self.team_creation_by_action(action_id).await?.ok_or(UndoError::NotFound)?;
        if c.status != "applied" {
            return Err(UndoError::NotFound);
        }
        if now() > c.undo_until() {
            return Err(UndoError::Expired);
        }
        // Its bots go with their messages and memories, so an out-of-step search index gets
        // the same repair and one retry as a bot's delete
        self.retry_after_index_repair(&format!("undoing ready-made team {}", c.team_id), || self.undo_team_creation_once(&c)).await?;
        Ok(c.created)
    }

    /// One attempt of [`Self::undo_team_creation`], in one write transaction.
    async fn undo_team_creation_once(&self, c: &Creation) -> Result<(), UndoError> {
        let mut tx = self.begin_write().await?;
        match undo_in(&mut tx, c).await {
            Ok(()) => {
                tx.commit().await?;
                Ok(())
            }
            Err(e) => {
                let _ = tx.rollback().await;
                Err(e)
            }
        }
    }
}

async fn create_in(tx: &mut Tx, space: &SpaceId, plan: &Plan, fail_after: Option<Step>) -> Result<String, CreateError> {
    let created_at = Utc::now();
    let mut created = Created { template_id: plan.template_id.clone(), ..Default::default() };

    // The team first: for an existing one, who is already in it; for a new one, the handle.
    let mut existing_team: Option<(String, Vec<String>)> = None;
    // Every clash is collected, so one refusal names all of them (T-A5, T-W11).
    let mut clashes: Vec<Clash> = Vec::new();
    match &plan.target {
        Target::New { handle, .. } => {
            if handle_taken(tx, space, handle).await? {
                let mut taken = HashSet::from([handle.clone()]);
                let mut candidate = names::next_free(handle, |n| taken.contains(n));
                while handle_taken(tx, space, &candidate).await? {
                    taken.insert(candidate.clone());
                    candidate = names::next_free(handle, |n| taken.contains(n));
                }
                clashes.push(Clash { kind: ClashKind::Handle, field: String::new(), wanted: handle.clone(), suggestion: candidate });
            }
        }
        Target::Existing { team_id } => {
            let row: Option<(String, String)> = sqlx::query_as("SELECT name, member_kwants FROM teams WHERE id = ? AND space_id = ?")
                .bind(team_id)
                .bind(space.as_str())
                .fetch_optional(&mut **tx)
                .await?;
            let (name, members) = row.ok_or(CreateError::TeamNotFound)?;
            existing_team = Some((name, parse_members(&members)));
        }
    }

    // Names: every new bot's name must be free in the space and unique in the request.
    // A suggestion is never a name another bot of this request asks for or was offered.
    let mut in_request: HashSet<String> = HashSet::new();
    let mut offered: HashSet<String> = plan.bots.iter().filter(|b| b.existing.is_none()).map(|b| b.name.to_lowercase()).collect();
    for b in plan.bots.iter().filter(|b| b.existing.is_none()) {
        let lower = b.name.to_lowercase();
        if in_request.contains(&lower) || name_taken(tx, space, &b.name).await? {
            let mut candidate = names::next_free(&b.name, |n| offered.contains(&n.to_lowercase()));
            while name_taken(tx, space, &candidate).await? {
                offered.insert(candidate.to_lowercase());
                candidate = names::next_free(&b.name, |n| offered.contains(&n.to_lowercase()));
            }
            offered.insert(candidate.to_lowercase());
            clashes.push(Clash { kind: ClashKind::Name, field: b.field.clone(), wanted: b.name.clone(), suggestion: candidate });
        }
        in_request.insert(lower);
    }
    if !clashes.is_empty() {
        return Err(CreateError::Clash(clashes));
    }

    // Bots.
    for (i, b) in plan.bots.iter().enumerate() {
        let (id, name) = match &b.existing {
            Some(id) => {
                let row: Option<(String,)> = sqlx::query_as("SELECT name FROM kwants WHERE id = ? AND space_id = ?")
                    .bind(id)
                    .bind(space.as_str())
                    .fetch_optional(&mut **tx)
                    .await?;
                let (name,) = row.ok_or_else(|| CreateError::BadExisting { field: b.field.replace(".name", ".use_existing") })?;
                (id.clone(), name)
            }
            None => {
                let id = Uuid::new_v4().to_string();
                let avatar = if super::imports::AVATARS.contains(&b.avatar.as_str()) { b.avatar.as_str() } else { "default" };
                // As a bot made by hand: no MCP opt-outs, the space's tools as usual (D-I8).
                sqlx::query(
                    "INSERT INTO kwants (id, name, title, persona_prompt, soul, avatar_sprite, driver_type, preferred_model, space_id)
                     VALUES (?, ?, ?, ?, ?, ?, 'api', 'auto', ?)",
                )
                .bind(&id)
                .bind(&b.name)
                .bind(&b.title)
                .bind(&b.persona)
                .bind(&b.soul)
                .bind(avatar)
                .bind(space.as_str())
                .execute(&mut **tx)
                .await?;
                if !b.covers.is_empty() || !b.avoids.is_empty() {
                    sqlx::query("INSERT INTO kwant_scope (kwant_id, covers, avoids) VALUES (?, ?, ?)")
                        .bind(&id)
                        .bind(serde_json::to_string(&b.covers)?)
                        .bind(serde_json::to_string(&b.avoids)?)
                        .execute(&mut **tx)
                        .await?;
                }
                (id, b.name.clone())
            }
        };
        let joined = existing_team.as_ref().is_none_or(|(_, members)| !members.contains(&id));
        created.bots.push(CreatedBot {
            slot: b.slot.clone(),
            id,
            name,
            preset: b.preset.clone(),
            lead: b.lead,
            existing: b.existing.is_some(),
            joined,
        });
        seam(fail_after, Step::Bot(i + 1))?;
    }
    let bot_id = |slot: &str| created.bots.iter().find(|b| b.slot == slot).map(|b| b.id.clone());

    // The team and its chat.
    match &plan.target {
        Target::New { handle, icon, description, instructions, workspace_path, repo_url } => {
            let lead = created.bots.iter().find(|b| b.lead).or(created.bots.first()).map(|b| b.id.clone()).ok_or_else(|| {
                CreateError::Db(DbError::Conflict("a team needs at least one bot".into()))
            })?;
            let mut members: Vec<String> = Vec::new();
            for b in &created.bots {
                if !members.contains(&b.id) {
                    members.push(b.id.clone());
                }
            }
            let members_json = serde_json::to_string(&members)?;
            let team_id = Uuid::new_v4().to_string();
            sqlx::query(
                "INSERT INTO teams (id, name, description, icon, lead_kwant_id, member_kwants, workspace_path, repo_url, space_id, instructions)
                 VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
            )
            .bind(&team_id)
            .bind(handle)
            .bind(description)
            .bind(icon)
            .bind(&lead)
            .bind(&members_json)
            .bind(workspace_path)
            .bind(repo_url)
            .bind(space.as_str())
            .bind(instructions)
            .execute(&mut **tx)
            .await?;
            let channel_id = Uuid::new_v4().to_string();
            sqlx::query("INSERT INTO channels (id, name, description, lead_kwant_id, member_kwants, routing_mode, space_id) VALUES (?, ?, NULL, ?, ?, 'auto', ?)")
                .bind(&channel_id)
                .bind(format!("team-{handle}"))
                .bind(&lead)
                .bind(&members_json)
                .bind(space.as_str())
                .execute(&mut **tx)
                .await?;
            created.team_id = team_id;
            created.channel_id = Some(channel_id);
            created.new_team = true;
        }
        Target::Existing { team_id } => {
            let (name, mut members) = existing_team.clone().expect("loaded above");
            for b in created.bots.iter().filter(|b| b.joined) {
                members.push(b.id.clone());
            }
            let members_json = serde_json::to_string(&members)?;
            sqlx::query("UPDATE teams SET member_kwants = ? WHERE id = ?").bind(&members_json).bind(team_id).execute(&mut **tx).await?;
            let chat = super::team_channel_id(&mut **tx, space, &name).await?;
            if let Some(chat) = &chat {
                let (m,): (String,) = sqlx::query_as("SELECT member_kwants FROM channels WHERE id = ?").bind(chat).fetch_one(&mut **tx).await?;
                let mut cm = parse_members(&m);
                for b in created.bots.iter().filter(|b| b.joined) {
                    if !cm.contains(&b.id) {
                        cm.push(b.id.clone());
                    }
                }
                sqlx::query("UPDATE channels SET member_kwants = ? WHERE id = ?").bind(serde_json::to_string(&cm)?).bind(chat).execute(&mut **tx).await?;
            }
            created.team_id = team_id.clone();
            created.channel_id = chat;
        }
    }
    seam(fail_after, Step::Team)?;

    // Starter skills, on: they are the template's, not a bot's proposal.
    for s in &plan.skills {
        let Some(owner) = bot_id(&s.owner_slot) else { continue };
        if created.bots.iter().any(|b| b.id == owner && b.existing) {
            continue; // a reused bot is never changed (A16)
        }
        let id = Uuid::new_v4().to_string();
        sqlx::query(
            "INSERT INTO skills (id, kwant_id, name, description, body, status, source, created_at, updated_at, space_id)
             VALUES (?, ?, ?, ?, ?, 'enabled', 'user', ?, ?, ?)",
        )
        .bind(&id)
        .bind(&owner)
        .bind(&s.name)
        .bind(&s.description)
        .bind(&s.body)
        .bind(created_at)
        .bind(created_at)
        .bind(space.as_str())
        .execute(&mut **tx)
        .await?;
        created.skills.push(CreatedSkill { id, name: s.name.clone(), owner_id: owner });
    }
    seam(fail_after, Step::Skills)?;

    // The routine, paused until the person turns it on (T5).
    if let Some(r) = &plan.routine {
        if let Some(owner) = bot_id(&r.owner_slot).filter(|o| !created.bots.iter().any(|b| &b.id == o && b.existing)) {
            let schedule = crate::routines::Schedule::Cron { expression: r.cron.clone(), tz: plan.tz.clone() };
            schedule.validate().map_err(|e| CreateError::Db(DbError::Conflict(format!("routine schedule: {e}"))))?;
            let id = Uuid::new_v4().to_string();
            sqlx::query(
                "INSERT INTO routines (id, kwant_id, channel_id, name, instructions, schedule, enabled, next_run_at, created_at, space_id)
                 VALUES (?, ?, ?, ?, ?, ?, 0, NULL, ?, ?)",
            )
            .bind(&id)
            .bind(&owner)
            .bind(&created.channel_id)
            .bind(&r.name)
            .bind(&r.instructions)
            .bind(serde_json::to_string(&schedule)?)
            .bind(created_at)
            .bind(space.as_str())
            .execute(&mut **tx)
            .await?;
            created.routine = Some(CreatedRoutine { id, name: r.name.clone(), enabled: false });
        }
    }
    seam(fail_after, Step::Routine)?;

    // A new space's #general appears with its first bot, as when one is made by hand.
    if !space.is_default() {
        if let Some(first) = created.bots.iter().find(|b| b.lead).or(created.bots.first()) {
            let (general,): (i64,) = sqlx::query_as("SELECT COUNT(*) FROM channels WHERE space_id = ? AND name = 'general'")
                .bind(space.as_str())
                .fetch_one(&mut **tx)
                .await?;
            if general == 0 {
                let id = Uuid::new_v4().to_string();
                sqlx::query("INSERT INTO channels (id, name, description, lead_kwant_id, member_kwants, routing_mode, space_id) VALUES (?, 'general', ?, ?, ?, 'auto', ?)")
                    .bind(&id)
                    .bind(super::GENERAL_DESCRIPTION)
                    .bind(&first.id)
                    .bind(serde_json::to_string(&[&first.id])?)
                    .bind(space.as_str())
                    .execute(&mut **tx)
                    .await?;
                created.general_id = Some(id);
            }
        }
    }

    // One logged action, one Undo.
    let lead = created.bots.iter().find(|b| b.lead).or(created.bots.first()).map(|b| b.id.clone()).unwrap_or_default();
    let new_count = created.bots.iter().filter(|b| !b.existing).count();
    let summary = match (&plan.target, &plan.template_id) {
        (Target::New { handle, .. }, Some(t)) => format!("made the {handle} team from the {t} template ({new_count} new bots)"),
        (Target::New { handle, .. }, None) => format!("made the {handle} team from the project ({new_count} new bots)"),
        (Target::Existing { .. }, _) => format!("added {new_count} suggested bots to the team"),
    };
    let action_id = Uuid::new_v4().to_string();
    let after = serde_json::to_value(&created)?;
    sqlx::query(
        "INSERT INTO bot_actions (id, space_id, kwant_id, channel_id, action, target_kind, target_id, summary, after, status, created_at)
         VALUES (?, ?, ?, ?, ?, 'team', ?, ?, ?, 'applied', ?)",
    )
    .bind(&action_id)
    .bind(space.as_str())
    .bind(&lead)
    .bind(&created.channel_id)
    .bind(ACTION)
    .bind(&created.team_id)
    .bind(&summary)
    .bind(after.to_string())
    .bind(created_at)
    .execute(&mut **tx)
    .await?;
    seam(fail_after, Step::Action)?;

    let id = Uuid::new_v4().to_string();
    sqlx::query(
        "INSERT INTO team_creations (id, space_id, team_id, action_id, template_id, suggestion_id, idempotency_key, request_sha256, created, status, created_at)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, 'applied', ?)",
    )
    .bind(&id)
    .bind(space.as_str())
    .bind(&created.team_id)
    .bind(&action_id)
    .bind(&plan.template_id)
    .bind(&plan.suggestion_id)
    .bind(&plan.idempotency_key)
    .bind(&plan.request_sha256)
    .bind(serde_json::to_string(&created)?)
    .bind(now())
    .execute(&mut **tx)
    .await?;
    Ok(id)
}

/// The new bots that now lead a team or chat this create didn't make. An archived team chat
/// (`archived-team-…`, left by a team delete) doesn't count: nobody can pick its lead any
/// more, and the Undo hands it on like any other chat the bot led (T-U10).
async fn blocked_by(conn: &mut sqlx::SqliteConnection, c: &Creation, new_ids: &[&str]) -> Result<Vec<Blocked>, DbError> {
    let mut out = Vec::new();
    let mine: Vec<&str> = [Some(c.team_id.as_str()), c.created.channel_id.as_deref(), c.created.general_id.as_deref()]
        .into_iter()
        .flatten()
        .collect();
    for id in new_ids {
        let Some((name,)): Option<(String,)> = sqlx::query_as("SELECT name FROM kwants WHERE id = ?").bind(id).fetch_optional(&mut *conn).await?
        else {
            continue;
        };
        for (table, prefix) in [("teams", ""), ("channels", "#")] {
            let rows: Vec<(String, String)> =
                sqlx::query_as(&format!("SELECT id, name FROM {table} WHERE lead_kwant_id = ? ORDER BY name")).bind(id).fetch_all(&mut *conn).await?;
            for (row_id, row_name) in rows {
                if table == "channels" && is_archived_team_chat(&row_name) {
                    continue;
                }
                // For an existing team the team itself isn't "mine" to delete, but the lead
                // didn't change: a new bot can't lead it unless the person made it so.
                let own = mine.contains(&row_id.as_str()) && (c.created.new_team || row_id != c.team_id);
                if !own {
                    out.push(Blocked { bot: name.clone(), leads: format!("{prefix}{row_name}") });
                }
            }
        }
    }
    Ok(out)
}

/// A chat [`super::archive_team_chat`] renamed when its team was deleted.
fn is_archived_team_chat(name: &str) -> bool {
    name.get(..14).is_some_and(|p| p.eq_ignore_ascii_case("archived-team-"))
}

async fn undo_in(tx: &mut Tx, c: &Creation) -> Result<(), UndoError> {
    let space = SpaceId::new(c.space_id.clone());
    let new_ids: Vec<&str> = c.created.bots.iter().filter(|b| !b.existing).map(|b| b.id.as_str()).collect();
    let live: Vec<&str> = {
        let mut v = Vec::new();
        for id in &new_ids {
            let (n,): (i64,) = sqlx::query_as("SELECT COUNT(*) FROM kwants WHERE id = ?").bind(id).fetch_one(&mut **tx).await?;
            if n > 0 {
                v.push(*id);
            }
        }
        v
    };
    let blocked = blocked_by(&mut *tx, c, &live).await?;
    if !blocked.is_empty() {
        return Err(UndoError::LeadsElsewhere(blocked));
    }

    // Someone who stays, to keep a chat with history: a reused bot, else any other bot of the space.
    let keeper: Option<String> = {
        let reused = c.created.bots.iter().filter(|b| b.existing).map(|b| b.id.clone());
        let mut found = None;
        for id in reused {
            let (n,): (i64,) = sqlx::query_as("SELECT COUNT(*) FROM kwants WHERE id = ?").bind(&id).fetch_one(&mut **tx).await?;
            if n > 0 {
                found = Some(id);
                break;
            }
        }
        if found.is_none() {
            let others: Vec<(String,)> = sqlx::query_as("SELECT id FROM kwants WHERE space_id = ? ORDER BY created_at, id")
                .bind(space.as_str())
                .fetch_all(&mut **tx)
                .await?;
            found = others.into_iter().map(|(id,)| id).find(|id| !new_ids.contains(&id.as_str()));
        }
        found
    };

    let mut chats: Vec<(String, bool)> = Vec::new(); // (channel, made by this create and to be removed)
    if c.created.new_team {
        if let Some(ch) = &c.created.channel_id {
            chats.push((ch.clone(), true));
        }
    }
    if let Some(g) = &c.created.general_id {
        chats.push((g.clone(), false));
    }
    for (ch, is_team_chat) in &chats {
        let exists: Option<(String,)> = sqlx::query_as("SELECT name FROM channels WHERE id = ?").bind(ch).fetch_optional(&mut **tx).await?;
        let Some((chat_name,)) = exists else { continue };
        let (n,): (i64,) = sqlx::query_as("SELECT COUNT(*) FROM messages WHERE channel_id = ?").bind(ch).fetch_one(&mut **tx).await?;
        if n == 0 {
            sqlx::query("DELETE FROM channels WHERE id = ?").bind(ch).execute(&mut **tx).await?;
        } else {
            if keeper.is_none() {
                return Err(UndoError::NoKeeper);
            }
            if *is_team_chat && chat_name.starts_with("team-") {
                super::archive_team_chat(tx, &space, ch, &chat_name).await?;
            }
        }
    }

    if c.created.new_team {
        sqlx::query("DELETE FROM teams WHERE id = ?").bind(&c.team_id).execute(&mut **tx).await?;
    } else {
        // Reused bots that this create added leave the team (and its chat) again.
        let leaving: Vec<&str> = c.created.bots.iter().filter(|b| b.existing && b.joined).map(|b| b.id.as_str()).collect();
        if !leaving.is_empty() {
            let row: Option<(String, String)> =
                sqlx::query_as("SELECT name, member_kwants FROM teams WHERE id = ?").bind(&c.team_id).fetch_optional(&mut **tx).await?;
            if let Some((name, members)) = row {
                let kept: Vec<String> = parse_members(&members).into_iter().filter(|m| !leaving.contains(&m.as_str())).collect();
                sqlx::query("UPDATE teams SET member_kwants = ? WHERE id = ?").bind(serde_json::to_string(&kept)?).bind(&c.team_id).execute(&mut **tx).await?;
                if let Some(chat) = super::team_channel_id(&mut **tx, &space, &name).await? {
                    let (m,): (String,) = sqlx::query_as("SELECT member_kwants FROM channels WHERE id = ?").bind(&chat).fetch_one(&mut **tx).await?;
                    let kept: Vec<String> = parse_members(&m).into_iter().filter(|x| !leaving.contains(&x.as_str())).collect();
                    sqlx::query("UPDATE channels SET member_kwants = ? WHERE id = ?").bind(serde_json::to_string(&kept)?).bind(&chat).execute(&mut **tx).await?;
                }
            }
        }
    }

    // The new bots, with their skills, routines and history; anything they still lead
    // (an archived chat, a kept #general) goes to the keeper.
    for id in &live {
        let new_lead = keeper.clone().unwrap_or_else(|| id.to_string());
        super::bot_actions::purge_kwant(tx, id, &new_lead).await?;
    }

    sqlx::query("UPDATE team_creations SET status = 'undone', undone_at = ? WHERE id = ?")
        .bind(Utc::now())
        .bind(&c.id)
        .execute(&mut **tx)
        .await?;
    Ok(())
}

#[derive(sqlx::FromRow)]
struct CreationRow {
    id: String,
    space_id: String,
    team_id: String,
    action_id: String,
    template_id: Option<String>,
    #[allow(dead_code)]
    suggestion_id: Option<String>,
    #[allow(dead_code)]
    idempotency_key: Option<String>,
    request_sha256: Option<String>,
    created: String,
    status: String,
    created_at: DateTime<Utc>,
    #[allow(dead_code)]
    undone_at: Option<DateTime<Utc>>,
}

impl TryFrom<CreationRow> for Creation {
    type Error = DbError;
    fn try_from(r: CreationRow) -> Result<Self, DbError> {
        Ok(Creation {
            id: r.id,
            space_id: r.space_id,
            team_id: r.team_id,
            action_id: r.action_id,
            template_id: r.template_id,
            created: serde_json::from_str(&r.created)?,
            status: r.status,
            created_at: r.created_at,
            request_sha256: r.request_sha256,
        })
    }
}
