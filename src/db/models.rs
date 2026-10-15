use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::FromRow;

#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct Kwant {
    pub id: String,
    pub name: String,
    pub title: String,
    pub persona_prompt: String,
    pub avatar_sprite: String,
    pub driver_type: String,
    pub preferred_model: String,
    pub temperature: f64,
    pub max_tokens: i64,
    pub level: i64,
    pub turns_completed: i64,
    pub created_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NewKwant {
    pub name: String,
    pub title: String,
    pub persona_prompt: String,
    pub avatar_sprite: String,
    pub driver_type: String,
    pub preferred_model: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct Channel {
    pub id: String,
    pub name: String,
    pub description: Option<String>,
    pub lead_kwant_id: String,
    pub member_kwants: String, // JSON array of string IDs
    pub routing_mode: String,
    pub created_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NewChannel {
    pub name: String,
    pub description: Option<String>,
    pub lead_kwant_id: String,
    pub member_kwants: Vec<String>,
    pub routing_mode: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct Message {
    pub id: String,
    pub channel_id: Option<String>,
    pub direct_kwant_id: Option<String>,
    pub sender_role: String, // 'user', 'kwant', 'system'
    pub sender_id: String,
    pub text: String,
    pub reasoning_content: Option<String>,
    pub tool_calls: Option<String>, // JSON serialized tool calls & outputs
    pub tokens_used: i64,
    pub created_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NewMessage {
    pub channel_id: Option<String>,
    pub direct_kwant_id: Option<String>,
    pub sender_role: String,
    pub sender_id: String,
    pub text: String,
    pub reasoning_content: Option<String>,
    pub tool_calls: Option<serde_json::Value>,
    pub tokens_used: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct Approval {
    pub id: String,
    pub turn_id: String,
    pub kwant_id: String,
    pub tool_name: String,
    pub parameters: String, // JSON string
    pub status: String,     // 'pending', 'allowed', 'denied'
    pub created_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NewApproval {
    pub turn_id: String,
    pub kwant_id: String,
    pub tool_name: String,
    pub parameters: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct Team {
    pub id: String,
    pub name: String,
    pub description: Option<String>,
    pub icon: String,
    pub lead_kwant_id: String,
    pub member_kwants: String, // JSON array string
    pub workspace_path: Option<String>,
    pub repo_url: Option<String>,
    pub created_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NewTeam {
    pub name: String,
    pub description: Option<String>,
    pub icon: Option<String>,
    pub lead_kwant_id: String,
    pub member_kwants: Vec<String>,
    #[serde(default)]
    pub workspace_path: Option<String>,
    #[serde(default)]
    pub repo_url: Option<String>,
}

