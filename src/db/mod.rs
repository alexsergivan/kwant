pub mod models;
pub mod profiles;
pub mod continuity;
pub mod threads;
pub mod zombie;
pub mod parallel;
pub mod documents;
pub mod history;
pub mod attachments;
pub mod questions;
pub mod skills;
pub mod away;
pub mod spaces;
pub mod custom_api;
pub mod mcp_suggestions;
pub mod bot_actions;
pub mod hires;
pub mod imports;
pub mod approval_modes;
pub mod api_context;
pub mod usage;

/// What a space's #general says about itself.
pub const GENERAL_DESCRIPTION: &str = "Everyone in this space. Laya picks who answers.";
/// Default's #general description before the "kwant bots" wording; replaced only if unchanged.
const OLD_GENERAL_DESCRIPTION: &str = "Main collaboration hub for all agents";

use models::*;
use spaces::SpaceId;
use sqlx::{sqlite::SqlitePoolOptions, Pool, Sqlite};
use std::path::Path;
use thiserror::Error;
use uuid::Uuid;

#[derive(Error, Debug)]
pub enum DbError {
    #[error("Database error: {0}")]
    Sqlx(#[from] sqlx::Error),

    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    #[error("JSON serialization error: {0}")]
    Json(#[from] serde_json::Error),

    #[error("Migration error: {0}")]
    Migration(String),

    /// A write would link rows from two spaces.
    #[error("{0}")]
    OtherSpace(String),

    #[error("{0}")]
    Conflict(String),
}

#[derive(Clone, Debug)]
pub struct Database {
    pool: Pool<Sqlite>,
}

/// Built-in kwants: (name, title, persona, soul, avatar).
const STARTERS: [(&str, &str, &str, &str, &str); 3] = [
    (
        "Architect",
        "Systems Architect",
        "You are the Architect, an expert in deep systems architecture, software design, and complex multi-file engineering. You give structured, thorough, and highly insightful technical advice.",
        "Calm, deliberate, and a little philosophical about software. You think in diagrams and long time horizons, value simplicity that survives change, and distrust cleverness. You speak in measured, well-structured sentences, often naming the trade-off before the answer, and you are quietly delighted by a clean abstraction.",
        "architect",
    ),
    (
        "TechLead",
        "Staff Engineer & Prototyper",
        "You are the Tech Lead, a sharp, pragmatic engineer who excels in fast reasoning, rapid prototyping, and clear technical communication.",
        "Energetic, direct, and allergic to over-engineering. You would rather ship a small working thing today than a perfect plan next month. You keep replies short, use plain words, and are generous with credit to teammates. Your catchphrase-level instinct is \"what is the smallest step that proves it?\"",
        "tech_lead",
    ),
    (
        "DevOps",
        "Systems & Security Engineer",
        "You are the DevOps Engineer, specializing in shell commands, server infrastructure, containerization, security policies, and automation.",
        "Steady, careful, and unflappable under pressure, like someone who has been paged at 3 a.m. and lived. You value reliability, least privilege, and being able to undo anything. Dry sense of humor; you always say what a command will do before running it and never pretend something is safe when it is not.",
        "devops",
    ),
];

impl Database {
    pub async fn init(data_dir: &Path) -> Result<Self, DbError> {
        Self::open(data_dir, true).await
    }

    /// The database as a release from before spaces leaves it, for migration tests.
    #[doc(hidden)]
    pub async fn init_without_spaces(data_dir: &Path) -> Result<Self, DbError> {
        Self::open(data_dir, false).await
    }

    async fn open(data_dir: &Path, spaces: bool) -> Result<Self, DbError> {
        std::fs::create_dir_all(data_dir)?;
        let db_path = data_dir.join("kwnt.db");
        let fresh = !db_path.exists();
        let conn_str = format!("sqlite://{}?mode=rwc", db_path.to_string_lossy());

        // Migrations run on one connection, closed before the runtime pool opens. A connection
        // whose schema was loaded before another one altered a table prepares `SELECT *` from
        // the old schema; SQLite re-prepares it at the first step with the new column count,
        // and sqlx reads the row with the old column list (an index-out-of-bounds panic, then
        // RowNotFound). With one connection every statement sees every earlier change, and the
        // runtime pool's connections all start from the finished schema.
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect(&conn_str)
            .await?;

        // Enable WAL mode and foreign keys
        sqlx::query("PRAGMA journal_mode = WAL;").execute(&pool).await?;
        sqlx::query("PRAGMA synchronous = NORMAL;").execute(&pool).await?;
        sqlx::query("PRAGMA foreign_keys = ON;").execute(&pool).await?;

        let schema = include_str!("schema.sql");
        for statement in schema.split(';') {
            let stmt = statement.trim();
            if !stmt.is_empty() {
                sqlx::query(stmt).execute(&pool).await?;
            }
        }

        // Run migrations for newly added columns if existing DB
        let _ = sqlx::query("ALTER TABLE teams ADD COLUMN workspace_path TEXT;").execute(&pool).await;
        let _ = sqlx::query("ALTER TABLE teams ADD COLUMN repo_url TEXT;").execute(&pool).await;
        let _ = sqlx::query("ALTER TABLE kwants ADD COLUMN soul TEXT NOT NULL DEFAULT '';").execute(&pool).await;

        let db = Self { pool };
        db.migrate_profiles().await?;
        db.migrate_continuity().await?;
        db.migrate_threads().await?;
        db.migrate_skills().await?;
        db.migrate_questions().await?;
        db.migrate_attachments().await?;
        db.migrate_history().await?;
        db.migrate_away().await?;
        db.migrate_zombie().await?;
        db.migrate_parallel().await?;
        db.migrate_documents().await?;
        db.migrate_api_context().await?;
        db.migrate_usage().await?;
        if spaces {
            let migration = db.migrate_spaces(data_dir, fresh).await?;
            db.require_spaces_schema(&migration).await?;
            db.ensure_one_general().await?;
            db.seed_defaults().await?;
            db.migrate_space_archive().await?;
            db.migrate_custom_api().await?;
            db.migrate_mcp_suggestions().await?;
            db.migrate_bot_actions().await?;
            db.migrate_hires().await?;
            db.migrate_imports().await?;
            db.migrate_approval_modes().await?;
        }
        db.pool.close().await;

        let pool = SqlitePoolOptions::new()
            .max_connections(10)
            .connect(&conn_str)
            .await?;
        sqlx::query("PRAGMA synchronous = NORMAL;").execute(&pool).await?;
        Ok(Self { pool })
    }

    async fn seed_defaults(&self) -> Result<(), DbError> {
        // Starters seeded before souls existed get one, unless the user has rewritten them
        for (name, _, persona_prompt, soul, _) in STARTERS {
            sqlx::query("UPDATE kwants SET soul = ? WHERE name = ? AND persona_prompt = ? AND soul = ''")
                .bind(soul)
                .bind(name)
                .bind(persona_prompt)
                .execute(&self.pool)
                .await?;
        }

        // Only the Default space is seeded; other spaces start empty
        let space = SpaceId::default_space();
        let count: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM kwants WHERE space_id = ?")
            .bind(space.as_str())
            .fetch_one(&self.pool)
            .await?;

        if count.0 == 0 {
            let starters: Vec<NewKwant> = STARTERS
                .iter()
                .map(|(name, title, persona_prompt, soul, avatar_sprite)| NewKwant {
                    name: (*name).into(),
                    title: (*title).into(),
                    persona_prompt: (*persona_prompt).into(),
                    soul: (*soul).into(),
                    avatar_sprite: (*avatar_sprite).into(),
                    driver_type: "api".into(),
                    preferred_model: "auto".into(),
                })
                .collect();

            for starter in starters {
                self.create_kwant(&space, &starter).await?;
            }

            // Create initial general channel
            let kwants = self.list_kwants(&space).await?;
            if let Some(lead) = kwants.first() {
                let chan = NewChannel {
                    name: "general".into(),
                    description: Some(GENERAL_DESCRIPTION.into()),
                    lead_kwant_id: lead.id.clone(),
                    member_kwants: kwants.iter().map(|k| k.id.clone()).collect(),
                    routing_mode: "auto".into(),
                };
                self.create_channel(&space, &chan).await?;
            }
        } else {
            // The old seeded #general text, if nobody edited it
            let _ = sqlx::query("UPDATE channels SET description = ? WHERE name = 'general' AND description = ?")
                .bind(GENERAL_DESCRIPTION)
                .bind(OLD_GENERAL_DESCRIPTION)
                .execute(&self.pool)
                .await;
            // Smoothly migrate any legacy starter names and prompts
            let _ = sqlx::query("UPDATE kwants SET name = 'Architect', title = 'Systems Architect', avatar_sprite = 'architect', persona_prompt = 'You are the Architect, an expert in deep systems architecture, software design, and complex multi-file engineering. You give structured, thorough, and highly insightful technical advice.' WHERE name IN ('CyberMage', 'Architect') AND persona_prompt LIKE '%CyberMage%'").execute(&self.pool).await;
            let _ = sqlx::query("UPDATE kwants SET name = 'TechLead', title = 'Staff Engineer & Prototyper', avatar_sprite = 'tech_lead', persona_prompt = 'You are the Tech Lead, a sharp, pragmatic engineer who excels in fast reasoning, rapid prototyping, and clear technical communication.' WHERE name IN ('PixelNinja', 'TechLead') AND persona_prompt LIKE '%PixelNinja%'").execute(&self.pool).await;
            let _ = sqlx::query("UPDATE kwants SET name = 'DevOps', title = 'Systems & Security Engineer', avatar_sprite = 'devops', persona_prompt = 'You are the DevOps Engineer, specializing in shell commands, server infrastructure, containerization, security policies, and automation.' WHERE name IN ('TerminalKnight', 'DevOps') AND persona_prompt LIKE '%TerminalKnight%'").execute(&self.pool).await;
        }

        // Seed default team if none exists
        let team_count: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM teams WHERE space_id = ?")
            .bind(space.as_str())
            .fetch_one(&self.pool)
            .await
            .unwrap_or((0,));

        if team_count.0 == 0 {
            let kwants = self.list_kwants(&space).await?;
            if let Some(lead) = kwants.iter().find(|k| k.name == "Architect").or_else(|| kwants.first()) {
                let member_ids: Vec<String> = kwants.iter().map(|k| k.id.clone()).collect();
                let default_team = NewTeam {
                    name: "core-engineering".into(),
                    description: Some("Core architectural, engineering, and infrastructure team".into()),
                    icon: Some("users".into()),
                    lead_kwant_id: lead.id.clone(),
                    member_kwants: member_ids,
                    workspace_path: None,
                    repo_url: None,
                };
                self.create_team(&space, &default_team).await?;
            }
        }

        Ok(())
    }


    // Kwants CRUD
    pub async fn create_kwant(&self, space: &SpaceId, input: &NewKwant) -> Result<Kwant, DbError> {
        let id = Uuid::new_v4().to_string();
        sqlx::query(
            r#"
            INSERT INTO kwants (id, name, title, persona_prompt, soul, avatar_sprite, driver_type, preferred_model, space_id)
            VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)
            "#,
        )
        .bind(&id)
        .bind(&input.name)
        .bind(&input.title)
        .bind(&input.persona_prompt)
        .bind(&input.soul)
        .bind(&input.avatar_sprite)
        .bind(&input.driver_type)
        .bind(&input.preferred_model)
        .bind(space.as_str())
        .execute(&self.pool)
        .await?;

        self.get_kwant(&id)
            .await?
            .ok_or_else(|| DbError::Sqlx(sqlx::Error::RowNotFound))
    }

    /// The space's kwants, oldest first.
    pub async fn list_kwants(&self, space: &SpaceId) -> Result<Vec<Kwant>, DbError> {
        let rows = sqlx::query_as::<_, Kwant>("SELECT * FROM kwants WHERE space_id = ? ORDER BY created_at ASC")
            .bind(space.as_str())
            .fetch_all(&self.pool)
            .await?;
        Ok(rows)
    }

    pub async fn get_kwant(&self, id: &str) -> Result<Option<Kwant>, DbError> {
        let row = sqlx::query_as::<_, Kwant>("SELECT * FROM kwants WHERE id = ?")
            .bind(id)
            .fetch_optional(&self.pool)
            .await?;
        Ok(row)
    }

    /// Updates a kwant's profile. Its past messages follow a rename, since messages store the sender's name.
    pub async fn update_kwant(&self, id: &str, input: &UpdateKwant) -> Result<Option<Kwant>, DbError> {
        let Some(existing) = self.get_kwant(id).await? else {
            return Ok(None);
        };
        let mut tx = self.pool.begin().await?;
        sqlx::query(
            "UPDATE kwants SET name = ?, title = ?, persona_prompt = ?, soul = ?, avatar_sprite = ?, preferred_model = ? WHERE id = ?",
        )
        .bind(&input.name)
        .bind(&input.title)
        .bind(&input.persona_prompt)
        .bind(&input.soul)
        .bind(&input.avatar_sprite)
        .bind(&input.preferred_model)
        .bind(id)
        .execute(&mut *tx)
        .await?;
        if existing.name != input.name {
            sqlx::query("UPDATE messages SET sender_id = ? WHERE sender_role = 'kwant' AND sender_id = ?")
                .bind(&input.name)
                .bind(&existing.name)
                .execute(&mut *tx)
                .await?;
        }
        tx.commit().await?;
        self.get_kwant(id).await
    }

    // Channels CRUD
    /// Fails with [`DbError::OtherSpace`] when the lead or a member is another space's kwant,
    /// and [`DbError::Conflict`] for a second #general in the space.
    pub async fn create_channel(&self, space: &SpaceId, input: &NewChannel) -> Result<Channel, DbError> {
        let id = Uuid::new_v4().to_string();
        let members_json = serde_json::to_string(&input.member_kwants)?;
        self.check_members(space, &input.lead_kwant_id, &input.member_kwants).await?;
        if input.name == "general" && self.general_channel(space).await?.is_some() {
            return Err(DbError::Conflict("this space already has a #general".into()));
        }

        sqlx::query(
            r#"
            INSERT INTO channels (id, name, description, lead_kwant_id, member_kwants, routing_mode, space_id)
            VALUES (?, ?, ?, ?, ?, ?, ?)
            "#,
        )
        .bind(&id)
        .bind(&input.name)
        .bind(&input.description)
        .bind(&input.lead_kwant_id)
        .bind(&members_json)
        .bind(&input.routing_mode)
        .bind(space.as_str())
        .execute(&self.pool)
        .await?;

        self.get_channel(&id)
            .await?
            .ok_or_else(|| DbError::Sqlx(sqlx::Error::RowNotFound))
    }

    pub async fn list_channels(&self, space: &SpaceId) -> Result<Vec<Channel>, DbError> {
        let rows = sqlx::query_as::<_, Channel>("SELECT * FROM channels WHERE space_id = ? ORDER BY created_at ASC")
            .bind(space.as_str())
            .fetch_all(&self.pool)
            .await?;
        Ok(rows)
    }

    /// The space's #general (the oldest, if an old database has several).
    pub async fn general_channel(&self, space: &SpaceId) -> Result<Option<Channel>, DbError> {
        Ok(sqlx::query_as::<_, Channel>(
            "SELECT * FROM channels WHERE space_id = ? AND name = 'general' ORDER BY created_at ASC, rowid ASC LIMIT 1",
        )
        .bind(space.as_str())
        .fetch_optional(&self.pool)
        .await?)
    }

    /// Refuses a lead or member list that names another space's kwant (or no kwant at all).
    async fn check_members(&self, space: &SpaceId, lead: &str, members: &[String]) -> Result<(), DbError> {
        let ids: Vec<&str> = std::iter::once(lead).chain(members.iter().map(String::as_str)).collect();
        if self.kwants_in_space(space, &ids).await? {
            Ok(())
        } else {
            Err(DbError::OtherSpace("every lead and member must be a bot in this space".into()))
        }
    }

    pub async fn get_channel(&self, id: &str) -> Result<Option<Channel>, DbError> {
        let row = sqlx::query_as::<_, Channel>("SELECT * FROM channels WHERE id = ?")
            .bind(id)
            .fetch_optional(&self.pool)
            .await?;
        Ok(row)
    }

    // Messages CRUD
    pub async fn insert_message(&self, input: &NewMessage) -> Result<Message, DbError> {
        self.insert_message_with(input, &MessageExtra::default()).await
    }

    /// [`Self::insert_message`] into a thread and/or with kwnt's own `meta`.
    pub async fn insert_message_with(&self, input: &NewMessage, extra: &MessageExtra) -> Result<Message, DbError> {
        let mut conn = self.pool.acquire().await?;
        insert_message_on(&mut conn, input, extra).await
    }

    pub async fn list_messages(&self, channel_or_kwant_id: &str, limit: i64) -> Result<Vec<Message>, DbError> {
        let rows = sqlx::query_as::<_, Message>(
            r#"
            SELECT * FROM (
                SELECT *, rowid AS _seq FROM messages
                WHERE channel_id = ? OR direct_kwant_id = ? OR thread_id = ?
                ORDER BY created_at DESC, rowid DESC
                LIMIT ?
            )
            ORDER BY created_at ASC, _seq ASC
            "#,
        )
        .bind(channel_or_kwant_id)
        .bind(channel_or_kwant_id)
        .bind(channel_or_kwant_id)
        .bind(limit)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows)
    }

    // Approvals CRUD
    pub async fn save_approval(&self, input: &NewApproval) -> Result<Approval, DbError> {
        let id = Uuid::new_v4().to_string();
        let params_json = serde_json::to_string(&input.parameters)?;

        sqlx::query(
            r#"
            INSERT INTO approvals (id, turn_id, kwant_id, tool_name, parameters, status)
            VALUES (?, ?, ?, ?, ?, 'pending')
            "#,
        )
        .bind(&id)
        .bind(&input.turn_id)
        .bind(&input.kwant_id)
        .bind(&input.tool_name)
        .bind(&params_json)
        .execute(&self.pool)
        .await?;

        self.get_approval(&id)
            .await?
            .ok_or_else(|| DbError::Sqlx(sqlx::Error::RowNotFound))
    }

    pub async fn get_approval(&self, id: &str) -> Result<Option<Approval>, DbError> {
        let row = sqlx::query_as::<_, Approval>("SELECT * FROM approvals WHERE id = ?")
            .bind(id)
            .fetch_optional(&self.pool)
            .await?;
        Ok(row)
    }

    pub async fn update_approval_status(&self, id: &str, status: &str) -> Result<(), DbError> {
        sqlx::query("UPDATE approvals SET status = ? WHERE id = ?")
            .bind(status)
            .bind(id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    // Settings CRUD
    pub async fn get_setting(&self, key: &str) -> Result<Option<String>, DbError> {
        let row: Option<(String,)> = sqlx::query_as("SELECT value FROM settings WHERE key = ?")
            .bind(key)
            .fetch_optional(&self.pool)
            .await?;
        Ok(row.map(|r| r.0))
    }

    pub async fn set_setting(&self, key: &str, value: &str) -> Result<(), DbError> {
        sqlx::query(
            r#"
            INSERT INTO settings (key, value) VALUES (?, ?)
            ON CONFLICT(key) DO UPDATE SET value = excluded.value
            "#,
        )
        .bind(key)
        .bind(value)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    // Teams CRUD
    /// Fails with [`DbError::OtherSpace`] when the lead or a member is another space's kwant.
    pub async fn create_team(&self, space: &SpaceId, input: &NewTeam) -> Result<Team, DbError> {
        let id = Uuid::new_v4().to_string();
        let icon = input.icon.as_deref().unwrap_or("users");
        let members_json = serde_json::to_string(&input.member_kwants)?;
        self.check_members(space, &input.lead_kwant_id, &input.member_kwants).await?;

        sqlx::query(
            r#"
            INSERT INTO teams (id, name, description, icon, lead_kwant_id, member_kwants, workspace_path, repo_url, space_id)
            VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)
            "#,
        )
        .bind(&id)
        .bind(&input.name)
        .bind(&input.description)
        .bind(icon)
        .bind(&input.lead_kwant_id)
        .bind(&members_json)
        .bind(&input.workspace_path)
        .bind(&input.repo_url)
        .bind(space.as_str())
        .execute(&self.pool)
        .await?;

        // Auto-create a team channel team-{name} if it doesn't already exist
        let channel_name = format!("team-{}", input.name);
        let channels = self.list_channels(space).await?;
        if !channels.iter().any(|c| c.name == channel_name) {
            let chan = NewChannel {
                name: channel_name,
                description: input.description.clone(),
                lead_kwant_id: input.lead_kwant_id.clone(),
                member_kwants: input.member_kwants.clone(),
                routing_mode: "auto".into(),
            };
            self.create_channel(space, &chan).await?;
        }

        self.get_team(&id)
            .await?
            .ok_or_else(|| DbError::Sqlx(sqlx::Error::RowNotFound))
    }

    pub async fn list_teams(&self, space: &SpaceId) -> Result<Vec<Team>, DbError> {
        let rows = sqlx::query_as::<_, Team>("SELECT * FROM teams WHERE space_id = ? ORDER BY created_at ASC")
            .bind(space.as_str())
            .fetch_all(&self.pool)
            .await?;
        Ok(rows)
    }

    pub async fn get_team(&self, id: &str) -> Result<Option<Team>, DbError> {
        let row = sqlx::query_as::<_, Team>("SELECT * FROM teams WHERE id = ?")
            .bind(id)
            .fetch_optional(&self.pool)
            .await?;
        Ok(row)
    }

    pub async fn get_team_by_name(&self, space: &SpaceId, name: &str) -> Result<Option<Team>, DbError> {
        let row = sqlx::query_as::<_, Team>("SELECT * FROM teams WHERE space_id = ? AND name = ?")
            .bind(space.as_str())
            .bind(name)
            .fetch_optional(&self.pool)
            .await?;
        Ok(row)
    }

    /// Fails with [`DbError::OtherSpace`] when the lead or a member is another space's kwant.
    pub async fn update_team(&self, id: &str, input: &NewTeam) -> Result<Option<Team>, DbError> {
        let icon = input.icon.as_deref().unwrap_or("users");
        let members_json = serde_json::to_string(&input.member_kwants)?;
        let Some(space) = self.space_of_team(id).await? else {
            return Ok(None);
        };
        self.check_members(&space, &input.lead_kwant_id, &input.member_kwants).await?;

        let res = sqlx::query(
            r#"
            UPDATE teams
            SET name = ?, description = ?, icon = ?, lead_kwant_id = ?, member_kwants = ?, workspace_path = ?, repo_url = ?
            WHERE id = ?
            "#,
        )
        .bind(&input.name)
        .bind(&input.description)
        .bind(icon)
        .bind(&input.lead_kwant_id)
        .bind(&members_json)
        .bind(&input.workspace_path)
        .bind(&input.repo_url)
        .bind(id)
        .execute(&self.pool)
        .await?;

        if res.rows_affected() == 0 {
            return Ok(None);
        }

        // Also update corresponding team channel if it exists
        let channel_name = format!("team-{}", input.name);
        let _ = sqlx::query("UPDATE channels SET description = ?, lead_kwant_id = ?, member_kwants = ? WHERE name = ? AND space_id = ?")
            .bind(&input.description)
            .bind(&input.lead_kwant_id)
            .bind(&members_json)
            .bind(&channel_name)
            .bind(space.as_str())
            .execute(&self.pool)
            .await;

        self.get_team(id).await
    }

    pub async fn delete_team(&self, id: &str) -> Result<bool, DbError> {
        let res = sqlx::query("DELETE FROM teams WHERE id = ?")
            .bind(id)
            .execute(&self.pool)
            .await?;
        Ok(res.rows_affected() > 0)
    }

    // MCP servers
    pub async fn list_mcp_servers(&self, space: &SpaceId) -> Result<Vec<McpServer>, DbError> {
        let rows = sqlx::query_as::<_, McpServerRow>("SELECT * FROM mcp_servers WHERE space_id = ? ORDER BY name ASC")
            .bind(space.as_str())
            .fetch_all(&self.pool)
            .await?;
        Ok(rows.into_iter().map(McpServer::from_row).collect())
    }

    pub async fn get_mcp_server(&self, id: &str) -> Result<Option<McpServer>, DbError> {
        let row = sqlx::query_as::<_, McpServerRow>("SELECT * FROM mcp_servers WHERE id = ?")
            .bind(id)
            .fetch_optional(&self.pool)
            .await?;
        Ok(row.map(McpServer::from_row))
    }

    /// Inserts `server` with a fresh id (any id passed in is ignored).
    pub async fn create_mcp_server(&self, space: &SpaceId, server: &McpServer) -> Result<McpServer, DbError> {
        let id = Uuid::new_v4().to_string();
        sqlx::query(
            "INSERT INTO mcp_servers (id, name, transport, command, args, url, env, headers, enabled, timeout_secs, space_id) \
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(&id)
        .bind(&server.name)
        .bind(&server.transport)
        .bind(&server.command)
        .bind(serde_json::to_string(&server.args)?)
        .bind(&server.url)
        .bind(serde_json::to_string(&server.env)?)
        .bind(serde_json::to_string(&server.headers)?)
        .bind(server.enabled)
        .bind(server.timeout_secs)
        .bind(space.as_str())
        .execute(&self.pool)
        .await?;
        self.get_mcp_server(&id)
            .await?
            .ok_or_else(|| DbError::Sqlx(sqlx::Error::RowNotFound))
    }

    pub async fn update_mcp_server(&self, server: &McpServer) -> Result<Option<McpServer>, DbError> {
        let res = sqlx::query(
            "UPDATE mcp_servers SET name = ?, transport = ?, command = ?, args = ?, url = ?, env = ?, headers = ?, \
             enabled = ?, timeout_secs = ? WHERE id = ?",
        )
        .bind(&server.name)
        .bind(&server.transport)
        .bind(&server.command)
        .bind(serde_json::to_string(&server.args)?)
        .bind(&server.url)
        .bind(serde_json::to_string(&server.env)?)
        .bind(serde_json::to_string(&server.headers)?)
        .bind(server.enabled)
        .bind(server.timeout_secs)
        .bind(&server.id)
        .execute(&self.pool)
        .await?;
        if res.rows_affected() == 0 {
            return Ok(None);
        }
        self.get_mcp_server(&server.id).await
    }

    /// Deletes the server, its kwant opt-outs (and legacy assignments) and its OAuth tokens and registrations. `foreign_keys` is only set on one pooled
    /// connection, so the cascade is done here rather than trusted to SQLite.
    pub async fn delete_mcp_server(&self, id: &str) -> Result<bool, DbError> {
        let mut tx = self.pool.begin().await?;
        for table in ["kwant_mcp_disabled", "kwant_mcp_servers", "mcp_oauth_tokens", "mcp_oauth_clients"] {
            sqlx::query(&format!("DELETE FROM {table} WHERE server_id = ?"))
                .bind(id)
                .execute(&mut *tx)
                .await?;
        }
        let res = sqlx::query("DELETE FROM mcp_servers WHERE id = ?")
            .bind(id)
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        Ok(res.rows_affected() > 0)
    }

    /// The servers a kwant may use, enabled or not: every server in its space it hasn't opted out of.
    pub async fn get_kwant_mcp_server_ids(&self, kwant_id: &str) -> Result<Vec<String>, DbError> {
        let rows: Vec<(String,)> = sqlx::query_as(
            "SELECT s.id FROM mcp_servers s WHERE s.space_id = (SELECT space_id FROM kwants WHERE id = ?1) AND NOT EXISTS \
             (SELECT 1 FROM kwant_mcp_disabled d WHERE d.kwant_id = ?1 AND d.server_id = s.id) ORDER BY s.name ASC",
        )
        .bind(kwant_id)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows.into_iter().map(|r| r.0).collect())
    }

    /// Makes `server_ids` the kwant's servers: every other server in its space is opted out, so
    /// ones added later start on. Callers validate that every id is a server in the kwant's space.
    pub async fn set_kwant_mcp_servers(&self, kwant_id: &str, server_ids: &[String]) -> Result<(), DbError> {
        let mut tx = self.pool.begin().await?;
        sqlx::query("DELETE FROM kwant_mcp_disabled WHERE kwant_id = ?")
            .bind(kwant_id)
            .execute(&mut *tx)
            .await?;
        let all: Vec<(String,)> =
            sqlx::query_as("SELECT id FROM mcp_servers WHERE space_id = (SELECT space_id FROM kwants WHERE id = ?)")
                .bind(kwant_id)
                .fetch_all(&mut *tx)
                .await?;
        for (server_id,) in all.into_iter().filter(|(id,)| !server_ids.contains(id)) {
            sqlx::query("INSERT OR IGNORE INTO kwant_mcp_disabled (kwant_id, server_id) VALUES (?, ?)")
                .bind(kwant_id)
                .bind(&server_id)
                .execute(&mut *tx)
                .await?;
        }
        tx.commit().await?;
        Ok(())
    }

    /// The enabled servers in the kwant's space it hasn't opted out of: what it may use on its next turn.
    pub async fn list_active_mcp_servers_for_kwant(&self, kwant_id: &str) -> Result<Vec<McpServer>, DbError> {
        let rows = sqlx::query_as::<_, McpServerRow>(
            "SELECT s.* FROM mcp_servers s WHERE s.enabled = 1 AND s.space_id = (SELECT space_id FROM kwants WHERE id = ?1) \
             AND NOT EXISTS (SELECT 1 FROM kwant_mcp_disabled d WHERE d.kwant_id = ?1 AND d.server_id = s.id) ORDER BY s.name ASC",
        )
        .bind(kwant_id)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows.into_iter().map(McpServer::from_row).collect())
    }

    /// Opt-outs by server: `server_id -> kwant ids that have it off`. Only the space's kwants are listed.
    pub async fn list_mcp_kwant_opt_outs(
        &self,
        space: &SpaceId,
    ) -> Result<std::collections::HashMap<String, Vec<String>>, DbError> {
        let rows: Vec<(String, String)> = sqlx::query_as(
            "SELECT d.server_id, d.kwant_id FROM kwant_mcp_disabled d JOIN kwants k ON k.id = d.kwant_id \
             WHERE k.space_id = ? ORDER BY k.name ASC",
        )
        .bind(space.as_str())
        .fetch_all(&self.pool)
        .await?;
        let mut map = std::collections::HashMap::<String, Vec<String>>::new();
        for (server_id, kwant_id) in rows {
            map.entry(server_id).or_default().push(kwant_id);
        }
        Ok(map)
    }
    // MCP OAuth
    pub async fn get_mcp_oauth_token(&self, server_id: &str) -> Result<Option<McpOAuthToken>, DbError> {
        Ok(sqlx::query_as::<_, McpOAuthToken>(
            "SELECT server_id, access_token, refresh_token, expires_at, token_endpoint, client_id, client_secret, \
             auth_method, resource FROM mcp_oauth_tokens WHERE server_id = ?",
        )
        .bind(server_id)
        .fetch_optional(&self.pool)
        .await?)
    }

    /// Token status for every server that has one: `server_id -> expires_at`.
    pub async fn list_mcp_oauth_expiries(
        &self,
    ) -> Result<std::collections::HashMap<String, Option<chrono::DateTime<chrono::Utc>>>, DbError> {
        let rows: Vec<(String, Option<chrono::DateTime<chrono::Utc>>)> =
            sqlx::query_as("SELECT server_id, expires_at FROM mcp_oauth_tokens").fetch_all(&self.pool).await?;
        Ok(rows.into_iter().collect())
    }

    pub async fn save_mcp_oauth_token(&self, token: &McpOAuthToken) -> Result<(), DbError> {
        sqlx::query(
            "INSERT INTO mcp_oauth_tokens (server_id, access_token, refresh_token, expires_at, token_endpoint, \
             client_id, client_secret, auth_method, resource, updated_at) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, CURRENT_TIMESTAMP) \
             ON CONFLICT(server_id) DO UPDATE SET access_token = excluded.access_token, \
             refresh_token = excluded.refresh_token, expires_at = excluded.expires_at, \
             token_endpoint = excluded.token_endpoint, client_id = excluded.client_id, \
             client_secret = excluded.client_secret, auth_method = excluded.auth_method, \
             resource = excluded.resource, updated_at = CURRENT_TIMESTAMP",
        )
        .bind(&token.server_id)
        .bind(&token.access_token)
        .bind(&token.refresh_token)
        .bind(token.expires_at)
        .bind(&token.token_endpoint)
        .bind(&token.client_id)
        .bind(&token.client_secret)
        .bind(&token.auth_method)
        .bind(&token.resource)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// Drops the server's tokens. Returns whether there were any.
    pub async fn delete_mcp_oauth_token(&self, server_id: &str) -> Result<bool, DbError> {
        let res = sqlx::query("DELETE FROM mcp_oauth_tokens WHERE server_id = ?")
            .bind(server_id)
            .execute(&self.pool)
            .await?;
        Ok(res.rows_affected() > 0)
    }

    pub async fn get_mcp_oauth_client(
        &self,
        server_id: &str,
        redirect_uri: &str,
        issuer: &str,
    ) -> Result<Option<McpOAuthClient>, DbError> {
        Ok(sqlx::query_as::<_, McpOAuthClient>(
            "SELECT server_id, redirect_uri, issuer, client_id, client_secret, auth_method FROM mcp_oauth_clients \
             WHERE server_id = ? AND redirect_uri = ? AND issuer = ?",
        )
        .bind(server_id)
        .bind(redirect_uri)
        .bind(issuer)
        .fetch_optional(&self.pool)
        .await?)
    }

    pub async fn save_mcp_oauth_client(&self, client: &McpOAuthClient) -> Result<(), DbError> {
        sqlx::query(
            "INSERT OR REPLACE INTO mcp_oauth_clients (server_id, redirect_uri, issuer, client_id, client_secret, auth_method) \
             VALUES (?, ?, ?, ?, ?, ?)",
        )
        .bind(&client.server_id)
        .bind(&client.redirect_uri)
        .bind(&client.issuer)
        .bind(&client.client_id)
        .bind(&client.client_secret)
        .bind(&client.auth_method)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// Forgets tokens and registrations, e.g. when the server's url changes.
    pub async fn clear_mcp_oauth(&self, server_id: &str) -> Result<(), DbError> {
        for table in ["mcp_oauth_tokens", "mcp_oauth_clients"] {
            sqlx::query(&format!("DELETE FROM {table} WHERE server_id = ?"))
                .bind(server_id)
                .execute(&self.pool)
                .await?;
        }
        Ok(())
    }
}

/// Inserts one message on `conn`, so it can be part of a caller's transaction.
pub(crate) async fn insert_message_on(
    conn: &mut sqlx::SqliteConnection,
    input: &NewMessage,
    extra: &MessageExtra,
) -> Result<Message, DbError> {
    let id = Uuid::new_v4().to_string();
    let tools_json = input
        .tool_calls
        .as_ref()
        .map(serde_json::to_string)
        .transpose()?;

    sqlx::query(
        r#"
        INSERT INTO messages (id, channel_id, direct_kwant_id, sender_role, sender_id, text, reasoning_content, tool_calls, tokens_used, thread_id, meta)
        VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
        "#,
    )
    .bind(&id)
    .bind(&input.channel_id)
    .bind(&input.direct_kwant_id)
    .bind(&input.sender_role)
    .bind(&input.sender_id)
    .bind(&input.text)
    .bind(&input.reasoning_content)
    .bind(&tools_json)
    .bind(input.tokens_used)
    .bind(&extra.thread_id)
    .bind(extra.meta.as_ref().map(|m| m.to_string()))
    .execute(&mut *conn)
    .await?;

    Ok(sqlx::query_as::<_, Message>("SELECT * FROM messages WHERE id = ?")
        .bind(&id)
        .fetch_one(&mut *conn)
        .await?)
}
