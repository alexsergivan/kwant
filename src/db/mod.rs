pub mod models;

use models::*;
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
}

#[derive(Clone, Debug)]
pub struct Database {
    pool: Pool<Sqlite>,
}

impl Database {
    pub async fn init(data_dir: &Path) -> Result<Self, DbError> {
        std::fs::create_dir_all(data_dir)?;
        let db_path = data_dir.join("kwnt.db");
        let conn_str = format!("sqlite://{}?mode=rwc", db_path.to_string_lossy());

        let pool = SqlitePoolOptions::new()
            .max_connections(10)
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

        let db = Self { pool };
        db.seed_defaults().await?;

        Ok(db)
    }

    async fn seed_defaults(&self) -> Result<(), DbError> {
        let count: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM kwants")
            .fetch_one(&self.pool)
            .await?;

        if count.0 == 0 {
            let starters = vec![
                NewKwant {
                    name: "Architect".into(),
                    title: "Systems Architect".into(),
                    persona_prompt: "You are the Architect, an expert in deep systems architecture, software design, and complex multi-file engineering. You give structured, thorough, and highly insightful technical advice.".into(),
                    avatar_sprite: "architect".into(),
                    driver_type: "api".into(),
                    preferred_model: "auto".into(),
                },
                NewKwant {
                    name: "TechLead".into(),
                    title: "Staff Engineer & Prototyper".into(),
                    persona_prompt: "You are the Tech Lead, a sharp, pragmatic engineer who excels in fast reasoning, rapid prototyping, and clear technical communication.".into(),
                    avatar_sprite: "tech_lead".into(),
                    driver_type: "api".into(),
                    preferred_model: "auto".into(),
                },
                NewKwant {
                    name: "DevOps".into(),
                    title: "Systems & Security Engineer".into(),
                    persona_prompt: "You are the DevOps Engineer, specializing in shell commands, server infrastructure, containerization, security policies, and automation.".into(),
                    avatar_sprite: "devops".into(),
                    driver_type: "api".into(),
                    preferred_model: "auto".into(),
                },
            ];

            for starter in starters {
                self.create_kwant(&starter).await?;
            }

            // Create initial general channel
            let kwants = self.list_kwants().await?;
            if let Some(lead) = kwants.first() {
                let chan = NewChannel {
                    name: "general".into(),
                    description: Some("Main collaboration hub for all agents".into()),
                    lead_kwant_id: lead.id.clone(),
                    member_kwants: kwants.iter().map(|k| k.id.clone()).collect(),
                    routing_mode: "auto".into(),
                };
                self.create_channel(&chan).await?;
            }
        } else {
            // Smoothly migrate any legacy starter names and prompts
            let _ = sqlx::query("UPDATE kwants SET name = 'Architect', title = 'Systems Architect', avatar_sprite = 'architect', persona_prompt = 'You are the Architect, an expert in deep systems architecture, software design, and complex multi-file engineering. You give structured, thorough, and highly insightful technical advice.' WHERE name IN ('CyberMage', 'Architect') AND persona_prompt LIKE '%CyberMage%'").execute(&self.pool).await;
            let _ = sqlx::query("UPDATE kwants SET name = 'TechLead', title = 'Staff Engineer & Prototyper', avatar_sprite = 'tech_lead', persona_prompt = 'You are the Tech Lead, a sharp, pragmatic engineer who excels in fast reasoning, rapid prototyping, and clear technical communication.' WHERE name IN ('PixelNinja', 'TechLead') AND persona_prompt LIKE '%PixelNinja%'").execute(&self.pool).await;
            let _ = sqlx::query("UPDATE kwants SET name = 'DevOps', title = 'Systems & Security Engineer', avatar_sprite = 'devops', persona_prompt = 'You are the DevOps Engineer, specializing in shell commands, server infrastructure, containerization, security policies, and automation.' WHERE name IN ('TerminalKnight', 'DevOps') AND persona_prompt LIKE '%TerminalKnight%'").execute(&self.pool).await;
        }

        // Seed default team if none exists
        let team_count: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM teams")
            .fetch_one(&self.pool)
            .await
            .unwrap_or((0,));

        if team_count.0 == 0 {
            let kwants = self.list_kwants().await?;
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
                self.create_team(&default_team).await?;
            }
        }

        Ok(())
    }


    // Kwants CRUD
    pub async fn create_kwant(&self, input: &NewKwant) -> Result<Kwant, DbError> {
        let id = Uuid::new_v4().to_string();
        sqlx::query(
            r#"
            INSERT INTO kwants (id, name, title, persona_prompt, avatar_sprite, driver_type, preferred_model)
            VALUES (?, ?, ?, ?, ?, ?, ?)
            "#,
        )
        .bind(&id)
        .bind(&input.name)
        .bind(&input.title)
        .bind(&input.persona_prompt)
        .bind(&input.avatar_sprite)
        .bind(&input.driver_type)
        .bind(&input.preferred_model)
        .execute(&self.pool)
        .await?;

        self.get_kwant(&id)
            .await?
            .ok_or_else(|| DbError::Sqlx(sqlx::Error::RowNotFound))
    }

    pub async fn list_kwants(&self) -> Result<Vec<Kwant>, DbError> {
        let rows = sqlx::query_as::<_, Kwant>("SELECT * FROM kwants ORDER BY created_at ASC")
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
            "UPDATE kwants SET name = ?, title = ?, persona_prompt = ?, avatar_sprite = ?, preferred_model = ? WHERE id = ?",
        )
        .bind(&input.name)
        .bind(&input.title)
        .bind(&input.persona_prompt)
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
    pub async fn create_channel(&self, input: &NewChannel) -> Result<Channel, DbError> {
        let id = Uuid::new_v4().to_string();
        let members_json = serde_json::to_string(&input.member_kwants)?;

        sqlx::query(
            r#"
            INSERT INTO channels (id, name, description, lead_kwant_id, member_kwants, routing_mode)
            VALUES (?, ?, ?, ?, ?, ?)
            "#,
        )
        .bind(&id)
        .bind(&input.name)
        .bind(&input.description)
        .bind(&input.lead_kwant_id)
        .bind(&members_json)
        .bind(&input.routing_mode)
        .execute(&self.pool)
        .await?;

        self.get_channel(&id)
            .await?
            .ok_or_else(|| DbError::Sqlx(sqlx::Error::RowNotFound))
    }

    pub async fn list_channels(&self) -> Result<Vec<Channel>, DbError> {
        let rows = sqlx::query_as::<_, Channel>("SELECT * FROM channels ORDER BY created_at ASC")
            .fetch_all(&self.pool)
            .await?;
        Ok(rows)
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
        let id = Uuid::new_v4().to_string();
        let tools_json = input
            .tool_calls
            .as_ref()
            .map(|v| serde_json::to_string(v))
            .transpose()?;

        sqlx::query(
            r#"
            INSERT INTO messages (id, channel_id, direct_kwant_id, sender_role, sender_id, text, reasoning_content, tool_calls, tokens_used)
            VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)
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
        .execute(&self.pool)
        .await?;

        let row = sqlx::query_as::<_, Message>("SELECT * FROM messages WHERE id = ?")
            .bind(&id)
            .fetch_one(&self.pool)
            .await?;
        Ok(row)
    }

    pub async fn list_messages(&self, channel_or_kwant_id: &str, limit: i64) -> Result<Vec<Message>, DbError> {
        let rows = sqlx::query_as::<_, Message>(
            r#"
            SELECT * FROM (
                SELECT *, rowid AS _seq FROM messages
                WHERE channel_id = ? OR direct_kwant_id = ?
                ORDER BY created_at DESC, rowid DESC
                LIMIT ?
            )
            ORDER BY created_at ASC, _seq ASC
            "#,
        )
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
    pub async fn create_team(&self, input: &NewTeam) -> Result<Team, DbError> {
        let id = Uuid::new_v4().to_string();
        let icon = input.icon.as_deref().unwrap_or("users");
        let members_json = serde_json::to_string(&input.member_kwants)?;

        sqlx::query(
            r#"
            INSERT INTO teams (id, name, description, icon, lead_kwant_id, member_kwants, workspace_path, repo_url)
            VALUES (?, ?, ?, ?, ?, ?, ?, ?)
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
        .execute(&self.pool)
        .await?;

        // Auto-create a team channel team-{name} if it doesn't already exist
        let channel_name = format!("team-{}", input.name);
        let channels = self.list_channels().await?;
        if !channels.iter().any(|c| c.name == channel_name) {
            let chan = NewChannel {
                name: channel_name,
                description: input.description.clone(),
                lead_kwant_id: input.lead_kwant_id.clone(),
                member_kwants: input.member_kwants.clone(),
                routing_mode: "auto".into(),
            };
            self.create_channel(&chan).await?;
        }

        self.get_team(&id)
            .await?
            .ok_or_else(|| DbError::Sqlx(sqlx::Error::RowNotFound))
    }

    pub async fn list_teams(&self) -> Result<Vec<Team>, DbError> {
        let rows = sqlx::query_as::<_, Team>("SELECT * FROM teams ORDER BY created_at ASC")
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

    pub async fn get_team_by_name(&self, name: &str) -> Result<Option<Team>, DbError> {
        let row = sqlx::query_as::<_, Team>("SELECT * FROM teams WHERE name = ?")
            .bind(name)
            .fetch_optional(&self.pool)
            .await?;
        Ok(row)
    }

    pub async fn update_team(&self, id: &str, input: &NewTeam) -> Result<Option<Team>, DbError> {
        let icon = input.icon.as_deref().unwrap_or("users");
        let members_json = serde_json::to_string(&input.member_kwants)?;

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
        let _ = sqlx::query("UPDATE channels SET description = ?, lead_kwant_id = ?, member_kwants = ? WHERE name = ?")
            .bind(&input.description)
            .bind(&input.lead_kwant_id)
            .bind(&members_json)
            .bind(&channel_name)
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
}

