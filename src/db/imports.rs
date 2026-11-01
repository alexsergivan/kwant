//! Imports: applying a checked [`Bundle`] in one transaction, remembering exactly what it
//! created (ids only, never the content), undoing it, and exporting bots back out as bundles.

use super::models::Kwant;
use super::spaces::SpaceId;
use super::{Database, DbError};
use crate::import::{scrub, BotSpec, Bundle, RoutineSpec, SkillSpec, Source, TeamSpec, BUNDLE_FORMAT};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use sqlx::FromRow;
use std::collections::HashMap;
use uuid::Uuid;

/// The avatars kwant draws; an import's `avatar_hint` is used when it names one.
pub const AVATARS: [&str; 22] = [
    "default", "architect", "tech_lead", "devops", "security", "fast", "designer", "data", "qa", "product", "writer", "research",
    "teacher", "marketing", "legal", "coach", "traveler", "chef", "fitness", "party", "builder", "zen",
];

/// One bot an import created, and what it looked like then (to tell if it was edited since).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct CreatedKwant {
    #[serde(rename = "ref")]
    pub ref_: String,
    pub id: String,
    pub name: String,
    pub fingerprint: String,
}

/// Everything an import created, by id.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Created {
    pub kwants: Vec<CreatedKwant>,
    pub skills: Vec<String>,
    pub routines: Vec<String>,
    pub teams: Vec<String>,
    pub channels: Vec<String>,
    pub memories: usize,
}

#[derive(Debug, Clone, FromRow)]
struct ImportRow {
    id: String,
    space_id: String,
    source_kind: String,
    created: String,
    status: String,
    created_at: DateTime<Utc>,
    undone_at: Option<DateTime<Utc>>,
}

/// An import as the API shows it.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct ImportRecord {
    pub id: String,
    pub space_id: String,
    pub source_kind: String,
    /// `applied` or `undone`.
    pub status: String,
    pub created: Created,
    pub created_at: DateTime<Utc>,
    pub undone_at: Option<DateTime<Utc>>,
}

impl TryFrom<ImportRow> for ImportRecord {
    type Error = DbError;
    fn try_from(r: ImportRow) -> Result<Self, DbError> {
        Ok(ImportRecord {
            id: r.id,
            space_id: r.space_id,
            source_kind: r.source_kind,
            status: r.status,
            created: serde_json::from_str(&r.created)?,
            created_at: r.created_at,
            undone_at: r.undone_at,
        })
    }
}

/// Why an import can't be undone.
#[derive(Debug)]
pub enum UndoError {
    NotFound,
    AlreadyUndone,
    /// Bots changed since the import; undoing again with `force` removes them anyway.
    Edited(Vec<String>),
    /// Bots that have been used (chats, threads, files, …): never removed by undo.
    InUse(Vec<String>),
    Db(DbError),
}

impl From<DbError> for UndoError {
    fn from(e: DbError) -> Self {
        UndoError::Db(e)
    }
}

impl From<sqlx::Error> for UndoError {
    fn from(e: sqlx::Error) -> Self {
        UndoError::Db(e.into())
    }
}

/// What a bot looks like, as a hash: name, title, instructions, soul, avatar and model.
fn fingerprint(name: &str, title: &str, persona: &str, soul: &str, avatar: &str, model: &str) -> String {
    let mut h = Sha256::new();
    for part in [name, title, persona, soul, avatar, model] {
        h.update((part.len() as u64).to_le_bytes());
        h.update(part.as_bytes());
    }
    format!("{:x}", h.finalize())
}

fn kwant_fingerprint(k: &Kwant) -> String {
    fingerprint(&k.name, &k.title, &k.persona_prompt, &k.soul, &k.avatar_sprite, &k.preferred_model)
}

/// What kwant's own history tables say about a kwant: anything here means it has been used.
const USE_CHECKS: [(&str, &str); 9] = [
    (
        "SELECT COUNT(*) FROM messages WHERE direct_kwant_id = ?1 OR (sender_role = 'kwant' AND sender_id = (SELECT name FROM kwants WHERE id = ?1) \
         AND (channel_id IN (SELECT id FROM channels WHERE space_id = (SELECT space_id FROM kwants WHERE id = ?1))))",
        "it has chat history",
    ),
    ("SELECT COUNT(*) FROM threads WHERE kwant_id = ?1", "it has threads"),
    ("SELECT COUNT(*) FROM delegations WHERE lead_kwant_id = ?1 OR specialist_kwant_id = ?1", "it has delegated work"),
    ("SELECT COUNT(*) FROM questions WHERE kwant_id = ?1", "it asked questions"),
    ("SELECT COUNT(*) FROM attachments WHERE kwant_id = ?1 OR direct_kwant_id = ?1", "it has files"),
    ("SELECT COUNT(*) FROM approvals WHERE kwant_id = ?1", "it has approvals"),
    ("SELECT COUNT(*) FROM mcp_suggestions WHERE kwant_id = ?1 OR direct_kwant_id = ?1", "it suggested tools"),
    ("SELECT COUNT(*) FROM hire_proposals WHERE kwant_id = ?1", "it proposed hiring bots"),
    ("SELECT COUNT(*) FROM routine_runs WHERE routine_id IN (SELECT id FROM routines WHERE kwant_id = ?1)", "its routines have run"),
];

fn in_list(ids: &[String]) -> String {
    vec!["?"; ids.len().max(1)].join(", ")
}

impl Database {
    pub(super) async fn migrate_imports(&self) -> Result<(), DbError> {
        for stmt in [
            "CREATE TABLE IF NOT EXISTS imports (
                id TEXT PRIMARY KEY,
                space_id TEXT NOT NULL,
                source_kind TEXT NOT NULL,
                created TEXT NOT NULL,
                status TEXT NOT NULL CHECK (status IN ('applied', 'undone')),
                created_at TIMESTAMP NOT NULL,
                undone_at TIMESTAMP
            )",
            "CREATE INDEX IF NOT EXISTS imports_by_space ON imports (space_id, created_at)",
        ] {
            sqlx::query(stmt).execute(&self.pool).await?;
        }
        Ok(())
    }

    pub async fn get_import(&self, id: &str) -> Result<Option<ImportRecord>, DbError> {
        let row = sqlx::query_as::<_, ImportRow>("SELECT * FROM imports WHERE id = ?").bind(id).fetch_optional(&self.pool).await?;
        row.map(ImportRecord::try_from).transpose()
    }

    /// The space's imports, newest first.
    pub async fn list_imports(&self, space: &SpaceId) -> Result<Vec<ImportRecord>, DbError> {
        let rows = sqlx::query_as::<_, ImportRow>("SELECT * FROM imports WHERE space_id = ? ORDER BY created_at DESC, rowid DESC LIMIT 100")
            .bind(space.as_str())
            .fetch_all(&self.pool)
            .await?;
        rows.into_iter().map(ImportRecord::try_from).collect()
    }

    /// Creates everything in a bundle that [`crate::import::normalize`] checked, in one
    /// transaction: bots (no tools, model `auto`), memories, skills `pending`, routines paused,
    /// teams with their channels, and the space's #general when it has none yet. Fails with
    /// [`DbError::Conflict`] if a bot or team name was taken in the meantime.
    pub async fn apply_import(&self, space: &SpaceId, bundle: &Bundle) -> Result<ImportRecord, DbError> {
        let mut created = Created::default();
        let now = Utc::now();
        let mut tx = self.pool.begin().await?;
        let mut ids: HashMap<&str, String> = HashMap::new();
        // Known up front so the ledger can say which import each memory came from
        let import_id = Uuid::new_v4().to_string();

        for bot in &bundle.bots {
            let (taken,): (i64,) = sqlx::query_as("SELECT COUNT(*) FROM kwants WHERE space_id = ? AND name = ? COLLATE NOCASE")
                .bind(space.as_str())
                .bind(&bot.name)
                .fetch_one(&mut *tx)
                .await?;
            if taken > 0 {
                return Err(DbError::Conflict(format!("a bot called {} already exists in this space; preview again", bot.name)));
            }
            let id = Uuid::new_v4().to_string();
            let avatar = bot.avatar_hint.as_deref().filter(|a| AVATARS.contains(a)).unwrap_or("default");
            sqlx::query(
                "INSERT INTO kwants (id, name, title, persona_prompt, soul, avatar_sprite, driver_type, preferred_model, space_id,
                                     browser_allowed_sites, browser_blocked_sites, browser_block_private_network)
                 VALUES (?, ?, ?, ?, ?, ?, 'api', 'auto', ?, ?, ?, ?)",
            )
            .bind(&id)
            .bind(&bot.name)
            .bind(&bot.title)
            .bind(&bot.persona_prompt)
            .bind(&bot.soul)
            .bind(avatar)
            .bind(space.as_str())
            .bind(serde_json::to_string(&bot.browser_allowed_sites).unwrap_or_else(|_| "[]".into()))
            .bind(serde_json::to_string(&bot.browser_blocked_sites).unwrap_or_else(|_| "[]".into()))
            .bind(bot.browser_block_private_network)
            .execute(&mut *tx)
            .await?;
            // No tools: opted out of every MCP server the space has now.
            sqlx::query("INSERT OR IGNORE INTO kwant_mcp_disabled (kwant_id, server_id) SELECT ?, id FROM mcp_servers WHERE space_id = ?")
                .bind(&id)
                .bind(space.as_str())
                .execute(&mut *tx)
                .await?;
            for text in &bot.memories {
                let memory_id = Uuid::new_v4().to_string();
                super::truth_ledger::note_provenance(&mut tx, &memory_id, super::truth_ledger::Provenance::Import { import_id: &import_id }).await?;
                sqlx::query("INSERT INTO kwant_memories (id, kwant_id, text, source, imported, created_at, updated_at) VALUES (?, ?, ?, 'user', 1, ?, ?)")
                    .bind(&memory_id)
                    .bind(&id)
                    .bind(text)
                    .bind(now)
                    .bind(now)
                    .execute(&mut *tx)
                    .await?;
                created.memories += 1;
            }
            for skill in &bot.skills {
                created.skills.push(insert_pending_skill(&mut tx, space, Some(&id), skill, now).await?);
            }
            for r in &bot.routines {
                created.routines.push(insert_paused_routine(&mut tx, space, &id, r, now).await?);
            }
            created.kwants.push(CreatedKwant {
                ref_: bot.ref_.clone(),
                id: id.clone(),
                name: bot.name.clone(),
                fingerprint: fingerprint(&bot.name, &bot.title, &bot.persona_prompt, &bot.soul, avatar, "auto"),
            });
            ids.insert(bot.ref_.as_str(), id);
        }
        for skill in &bundle.skills {
            created.skills.push(insert_pending_skill(&mut tx, space, None, skill, now).await?);
        }

        for team in &bundle.teams {
            let members: Vec<String> = team.members.iter().filter_map(|r| ids.get(r.as_str()).cloned()).collect();
            let Some(lead) = team.lead.as_deref().and_then(|r| ids.get(r)).or(members.first()).cloned() else { continue };
            let name = free_team_name(&mut tx, space, &team.name).await?;
            let team_id = Uuid::new_v4().to_string();
            let members_json = serde_json::to_string(&members)?;
            sqlx::query(
                "INSERT INTO teams (id, name, description, icon, lead_kwant_id, member_kwants, space_id, instructions) VALUES (?, ?, ?, 'users', ?, ?, ?, ?)",
            )
            .bind(&team_id)
            .bind(&name)
            .bind(Some(format!("Imported from {}", bundle.source.kind)))
            .bind(&lead)
            .bind(&members_json)
            .bind(space.as_str())
            .bind(&team.instructions)
            .execute(&mut *tx)
            .await?;
            created.teams.push(team_id);
            let channel = format!("team-{name}");
            if super::team_channel_id(&mut *tx, space, &name).await?.is_none() {
                created.channels.push(insert_channel(&mut tx, space, &channel, None, &lead, &members_json).await?);
            }
        }

        // A new space's #general appears with its first bot, as when one is created by hand.
        if let (false, Some(first)) = (space.is_default(), created.kwants.first()) {
            let (general,): (i64,) = sqlx::query_as("SELECT COUNT(*) FROM channels WHERE space_id = ? AND name = 'general'")
                .bind(space.as_str())
                .fetch_one(&mut *tx)
                .await?;
            if general == 0 {
                let members = serde_json::to_string(&[&first.id])?;
                created.channels.push(insert_channel(&mut tx, space, "general", Some(super::GENERAL_DESCRIPTION), &first.id, &members).await?);
            }
        }

        let id = import_id;
        sqlx::query("INSERT INTO imports (id, space_id, source_kind, created, status, created_at) VALUES (?, ?, ?, ?, 'applied', ?)")
            .bind(&id)
            .bind(space.as_str())
            .bind(&bundle.source.kind)
            .bind(serde_json::to_string(&created)?)
            .bind(now)
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        self.get_import(&id).await?.ok_or(DbError::Sqlx(sqlx::Error::RowNotFound))
    }

    /// Removes exactly what an import created, in one transaction. Bots that were used since
    /// (chats, threads, files, other skills, leading something the import didn't make) are
    /// never removed; bots that were only edited are removed with `force`. Bots already
    /// deleted by hand are skipped.
    pub async fn undo_import(&self, id: &str, force: bool) -> Result<ImportRecord, UndoError> {
        let record = self.get_import(id).await?.ok_or(UndoError::NotFound)?;
        if record.status != "applied" {
            return Err(UndoError::AlreadyUndone);
        }
        let c = &record.created;
        let mut live = Vec::new();
        let mut edited = Vec::new();
        for k in &c.kwants {
            if let Some(now) = self.get_kwant(&k.id).await? {
                if kwant_fingerprint(&now) != k.fingerprint {
                    edited.push(now.name.clone());
                }
                live.push(now);
            }
        }
        let mut in_use = Vec::new();
        for k in &live {
            if let Some(why) = self.used_since_import(&k.id, c).await? {
                in_use.push(format!("{} ({why})", k.name));
            }
        }
        if !c.channels.is_empty() {
            let sql = format!("SELECT name FROM channels WHERE id IN ({}) AND id IN (SELECT channel_id FROM messages)", in_list(&c.channels));
            let mut q = sqlx::query_as::<_, (String,)>(&sql);
            for ch in &c.channels {
                q = q.bind(ch);
            }
            for (name,) in q.fetch_all(&self.pool).await? {
                in_use.push(format!("#{name} (it has messages)"));
            }
        }
        if !in_use.is_empty() {
            return Err(UndoError::InUse(in_use));
        }
        if !edited.is_empty() && !force {
            return Err(UndoError::Edited(edited));
        }

        let mut tx = self.pool.begin().await?;
        for table in ["teams", "channels"] {
            let list = if table == "teams" { &c.teams } else { &c.channels };
            for row in list {
                sqlx::query(&format!("DELETE FROM {table} WHERE id = ?")).bind(row).execute(&mut *tx).await?;
            }
        }
        for skill in &c.skills {
            sqlx::query("DELETE FROM skills WHERE id = ?").bind(skill).execute(&mut *tx).await?;
        }
        for routine in &c.routines {
            sqlx::query("DELETE FROM routine_runs WHERE routine_id = ?").bind(routine).execute(&mut *tx).await?;
            sqlx::query("DELETE FROM routines WHERE id = ?").bind(routine).execute(&mut *tx).await?;
        }
        for k in &live {
            for sql in [
                "DELETE FROM routine_runs WHERE routine_id IN (SELECT id FROM routines WHERE kwant_id = ?)",
                "DELETE FROM routines WHERE kwant_id = ?",
                "DELETE FROM kwant_memories WHERE kwant_id = ?",
                "DELETE FROM kwant_mcp_disabled WHERE kwant_id = ?",
                "DELETE FROM kwant_mcp_servers WHERE kwant_id = ?",
            ] {
                sqlx::query(sql).bind(&k.id).execute(&mut *tx).await?;
            }
            for sql in super::truth_ledger::PURGE_BOT_FACTS {
                sqlx::query(sql).bind(&k.id).execute(&mut *tx).await?;
            }
            // Memberships the person added since (another team or channel) go with it.
            for table in ["channels", "teams"] {
                let rows: Vec<(String, String)> = sqlx::query_as(&format!("SELECT id, member_kwants FROM {table} WHERE member_kwants LIKE '%' || ? || '%'"))
                    .bind(&k.id)
                    .fetch_all(&mut *tx)
                    .await?;
                for (row_id, members) in rows {
                    let kept: Vec<String> = serde_json::from_str::<Vec<String>>(&members).unwrap_or_default().into_iter().filter(|m| *m != k.id).collect();
                    sqlx::query(&format!("UPDATE {table} SET member_kwants = ? WHERE id = ?"))
                        .bind(serde_json::to_string(&kept).map_err(DbError::from)?)
                        .bind(&row_id)
                        .execute(&mut *tx)
                        .await?;
                }
            }
            sqlx::query("DELETE FROM kwants WHERE id = ?").bind(&k.id).execute(&mut *tx).await?;
        }
        sqlx::query("UPDATE imports SET status = 'undone', undone_at = ? WHERE id = ? AND status = 'applied'")
            .bind(Utc::now())
            .bind(id)
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        self.get_import(id).await?.ok_or(UndoError::NotFound)
    }

    /// Why an imported kwant can't be removed by undo: history, or rows the import didn't make.
    async fn used_since_import(&self, kwant_id: &str, c: &Created) -> Result<Option<&'static str>, DbError> {
        for (sql, why) in USE_CHECKS {
            let (n,): (i64,) = sqlx::query_as(sql).bind(kwant_id).fetch_one(&self.pool).await?;
            if n > 0 {
                return Ok(Some(why));
            }
        }
        let foreign: [(&str, &[String], &'static str); 3] = [
            ("SELECT COUNT(*) FROM skills WHERE kwant_id = ? AND id NOT IN ({})", &c.skills, "it has skills added after the import"),
            ("SELECT COUNT(*) FROM channels WHERE lead_kwant_id = ? AND id NOT IN ({})", &c.channels, "it leads a channel"),
            ("SELECT COUNT(*) FROM teams WHERE lead_kwant_id = ? AND id NOT IN ({})", &c.teams, "it leads a team"),
        ];
        for (sql, ours, why) in foreign {
            let sql = sql.replace("{}", &in_list(ours));
            let mut q = sqlx::query_as::<_, (i64,)>(&sql).bind(kwant_id);
            if ours.is_empty() {
                q = q.bind("");
            }
            for id in ours {
                q = q.bind(id);
            }
            if q.fetch_one(&self.pool).await?.0 > 0 {
                return Ok(Some(why));
            }
        }
        Ok(None)
    }

    /// One bot as a bundle entry: profile, memories, its live and pending skills, routines
    /// (with their schedules) and the names of its tools. Keys are scrubbed by the caller.
    async fn export_bot(&self, k: &Kwant) -> Result<BotSpec, DbError> {
        let memories = self.list_memories(&k.id).await?.into_iter().map(|m| m.text).collect();
        let skills: Vec<SkillSpec> = sqlx::query_as::<_, (String, String, String)>(
            "SELECT name, description, body FROM skills WHERE kwant_id = ? AND status IN ('enabled', 'disabled', 'pending') ORDER BY name, created_at",
        )
        .bind(&k.id)
        .fetch_all(&self.pool)
        .await?
        .into_iter()
        .map(|(name, description, body)| SkillSpec { name, description, body })
        .collect();
        type Row = (String, String, String, Option<String>, Option<String>, Option<String>);
        let routines = sqlx::query_as::<_, Row>(
            "SELECT name, instructions, schedule, task_class, acceptance, checks FROM routines WHERE kwant_id = ? ORDER BY created_at, rowid",
        )
        .bind(&k.id)
        .fetch_all(&self.pool)
        .await?
        .into_iter()
        .map(|(name, instructions, schedule, task_class, acceptance, checks)| RoutineSpec {
            name,
            instructions,
            schedule: serde_json::from_str(&schedule).ok(),
            schedule_text: None,
            task_class,
            acceptance,
            checks: shareable_checks(checks.as_deref()),
        })
        .collect();
        let tool_hints = self.list_active_mcp_servers_for_kwant(&k.id).await?.into_iter().map(|s| s.name).collect();
        let sites = self.browser_sites(&k.id).await?;
        Ok(BotSpec {
            ref_: k.name.clone(),
            name: k.name.clone(),
            title: k.title.clone(),
            persona_prompt: k.persona_prompt.clone(),
            soul: k.soul.clone(),
            avatar_hint: Some(k.avatar_sprite.clone()),
            model_hint: Some(k.preferred_model.clone()),
            memories,
            skills,
            routines,
            tool_hints,
            browser_allowed_sites: sites.browser_allowed_sites,
            browser_blocked_sites: sites.browser_blocked_sites,
            browser_block_private_network: sites.browser_block_private_network,
            ..BotSpec::default()
        })
    }

    /// A bundle with one bot.
    pub async fn export_kwant(&self, id: &str) -> Result<Option<Bundle>, DbError> {
        let Some(k) = self.get_kwant(id).await? else { return Ok(None) };
        let mut b = export_bundle(format!("bot {}", k.name));
        b.bots.push(self.export_bot(&k).await?);
        scrub::scrub_bundle(&mut b);
        Ok(Some(b))
    }

    /// A bundle with every bot and team in the space, and its shared skills.
    pub async fn export_space(&self, space: &SpaceId) -> Result<Bundle, DbError> {
        let label = self.get_space(space).await?.map(|s| s.name).unwrap_or_else(|| space.as_str().to_string());
        let mut b = export_bundle(format!("space {label}"));
        let kwants = self.list_kwants(space).await?;
        let names: HashMap<&str, &str> = kwants.iter().map(|k| (k.id.as_str(), k.name.as_str())).collect();
        for k in &kwants {
            b.bots.push(self.export_bot(k).await?);
        }
        for t in self.list_teams(space).await? {
            let members: Vec<String> = serde_json::from_str::<Vec<String>>(&t.member_kwants)
                .unwrap_or_default()
                .iter()
                .filter_map(|id| names.get(id.as_str()).map(|n| n.to_string()))
                .collect();
            b.teams.push(TeamSpec {
                name: t.name.clone(),
                lead: names.get(t.lead_kwant_id.as_str()).map(|n| n.to_string()),
                members,
                instructions: t.instructions.clone().filter(|i| !i.is_empty()),
            });
        }
        b.skills = sqlx::query_as::<_, (String, String, String)>(
            "SELECT name, description, body FROM skills WHERE kwant_id IS NULL AND space_id = ? AND status IN ('enabled', 'disabled', 'pending') ORDER BY name",
        )
        .bind(space.as_str())
        .fetch_all(&self.pool)
        .await?
        .into_iter()
        .map(|(name, description, body)| SkillSpec { name, description, body })
        .collect();
        scrub::scrub_bundle(&mut b);
        Ok(b)
    }
}

fn export_bundle(label: String) -> Bundle {
    Bundle {
        format: BUNDLE_FORMAT.into(),
        source: Source { kind: "kwant".into(), label: Some(label), exported_at: Some(Utc::now().to_rfc3339()) },
        omitted: vec!["Chat history, files, tool connections and keys are never exported".into()],
        ..Bundle::default()
    }
}

async fn insert_pending_skill(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    space: &SpaceId,
    kwant_id: Option<&str>,
    skill: &SkillSpec,
    now: DateTime<Utc>,
) -> Result<String, DbError> {
    let id = Uuid::new_v4().to_string();
    sqlx::query(
        "INSERT INTO skills (id, kwant_id, name, description, body, status, source, created_at, updated_at, space_id)
         VALUES (?, ?, ?, ?, ?, 'pending', 'user', ?, ?, ?)",
    )
    .bind(&id)
    .bind(kwant_id)
    .bind(&skill.name)
    .bind(&skill.description)
    .bind(&skill.body)
    .bind(now)
    .bind(now)
    .bind(space.as_str())
    .execute(&mut **tx)
    .await?;
    Ok(id)
}

/// A routine's checks for a bundle, only when no spec holds anything that looks like a secret:
/// a bundle is shared, and a masked check would be useless.
fn shareable_checks(checks: Option<&str>) -> Option<Vec<serde_json::Value>> {
    let list: Vec<serde_json::Value> = serde_json::from_str(checks?).ok()?;
    let clean = list.iter().all(|c| crate::server::missions::autofix::redact_value(&c["spec"]) == c["spec"]);
    (clean && !list.is_empty()).then_some(list)
}

async fn insert_paused_routine(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    space: &SpaceId,
    kwant_id: &str,
    r: &RoutineSpec,
    now: DateTime<Utc>,
) -> Result<String, DbError> {
    let schedule = r
        .schedule
        .clone()
        .and_then(|v| serde_json::from_value::<crate::routines::Schedule>(v).ok())
        .filter(|s| s.validate().is_ok())
        .unwrap_or_else(crate::import::placeholder_schedule);
    let id = Uuid::new_v4().to_string();
    sqlx::query(
        "INSERT INTO routines (id, kwant_id, channel_id, name, instructions, schedule, enabled, next_run_at, created_at, space_id, task_class, acceptance)
         VALUES (?, ?, NULL, ?, ?, ?, 0, NULL, ?, ?, ?, ?)",
    )
    .bind(&id)
    .bind(kwant_id)
    .bind(&r.name)
    .bind(&r.instructions)
    .bind(serde_json::to_string(&schedule)?)
    .bind(now)
    .bind(space.as_str())
    .bind(&r.task_class)
    .bind(&r.acceptance)
    .execute(&mut **tx)
    .await?;
    Ok(id)
}

async fn insert_channel(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    space: &SpaceId,
    name: &str,
    description: Option<&str>,
    lead: &str,
    members_json: &str,
) -> Result<String, DbError> {
    let id = Uuid::new_v4().to_string();
    sqlx::query("INSERT INTO channels (id, name, description, lead_kwant_id, member_kwants, routing_mode, space_id) VALUES (?, ?, ?, ?, ?, 'auto', ?)")
        .bind(&id)
        .bind(name)
        .bind(description)
        .bind(lead)
        .bind(members_json)
        .bind(space.as_str())
        .execute(&mut **tx)
        .await?;
    Ok(id)
}

/// `name`, or `name 2`, `name 3`, …: the first no team in the space has (any case).
async fn free_team_name(tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>, space: &SpaceId, name: &str) -> Result<String, DbError> {
    for n in 1..1000 {
        let candidate = if n == 1 { name.to_string() } else { format!("{name} {n}") };
        let (taken,): (i64,) = sqlx::query_as("SELECT COUNT(*) FROM teams WHERE space_id = ? AND name = ? COLLATE NOCASE")
            .bind(space.as_str())
            .bind(&candidate)
            .fetch_one(&mut **tx)
            .await?;
        if taken == 0 {
            return Ok(candidate);
        }
    }
    Err(DbError::Conflict(format!("too many teams called {name}")))
}
