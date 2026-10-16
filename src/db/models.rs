use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::FromRow;

#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct Kwant {
    pub id: String,
    pub name: String,
    pub title: String,
    pub persona_prompt: String,
    /// Personality, voice and values, kept apart from the job description in `persona_prompt`.
    #[serde(default)]
    #[sqlx(default)]
    pub soul: String,
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
    #[serde(default)]
    pub soul: String,
    pub avatar_sprite: String,
    pub driver_type: String,
    pub preferred_model: String,
}

/// Editable fields of an existing kwant.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpdateKwant {
    pub name: String,
    pub title: String,
    pub persona_prompt: String,
    #[serde(default)]
    pub soul: String,
    pub avatar_sprite: String,
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


/// Raw `mcp_servers` row; JSON columns are decoded by [`McpServer::from_row`].
#[derive(Clone, FromRow)]
pub struct McpServerRow {
    pub id: String,
    pub name: String,
    pub transport: String,
    pub command: Option<String>,
    pub args: String,
    pub url: Option<String>,
    pub env: String,
    pub headers: String,
    pub enabled: bool,
    pub timeout_secs: i64,
    pub created_at: Option<DateTime<Utc>>,
}

/// A registered MCP server, secrets included. Deliberately not `Serialize`: the API returns
/// `crate::mcp::McpServerView`, which masks `env` and `headers`.
#[derive(Clone, PartialEq)]
pub struct McpServer {
    pub id: String,
    pub name: String,
    pub transport: String, // "stdio" | "http"
    pub command: Option<String>,
    pub args: Vec<String>,
    pub url: Option<String>,
    pub env: std::collections::BTreeMap<String, String>,
    pub headers: std::collections::BTreeMap<String, String>,
    pub enabled: bool,
    pub timeout_secs: i64,
    pub created_at: Option<DateTime<Utc>>,
}

impl McpServer {
    pub fn from_row(row: McpServerRow) -> Self {
        Self {
            id: row.id,
            name: row.name,
            transport: row.transport,
            command: row.command,
            args: serde_json::from_str(&row.args).unwrap_or_default(),
            url: row.url,
            env: serde_json::from_str(&row.env).unwrap_or_default(),
            headers: serde_json::from_str(&row.headers).unwrap_or_default(),
            enabled: row.enabled,
            timeout_secs: row.timeout_secs,
            created_at: row.created_at,
        }
    }

    /// Every stored secret value, for redacting text that may echo them.
    pub fn secret_values(&self) -> Vec<String> {
        self.env.values().chain(self.headers.values()).cloned().collect()
    }
}

/// Never prints secret values.
impl std::fmt::Debug for McpServer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("McpServer")
            .field("id", &self.id)
            .field("name", &self.name)
            .field("transport", &self.transport)
            .field("command", &self.command)
            .field("args", &self.args)
            .field("url", &self.url)
            .field("env_keys", &self.env.keys().collect::<Vec<_>>())
            .field("header_keys", &self.headers.keys().collect::<Vec<_>>())
            .field("enabled", &self.enabled)
            .field("timeout_secs", &self.timeout_secs)
            .finish()
    }
}

/// A dynamic client registration with one authorization server, for one redirect URI.
#[derive(Clone, PartialEq, FromRow)]
pub struct McpOAuthClient {
    pub server_id: String,
    pub redirect_uri: String,
    pub issuer: String,
    pub client_id: String,
    pub client_secret: Option<String>,
    /// Token endpoint auth: "none", "client_secret_post" or "client_secret_basic".
    pub auth_method: String,
}

impl std::fmt::Debug for McpOAuthClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("McpOAuthClient")
            .field("server_id", &self.server_id)
            .field("redirect_uri", &self.redirect_uri)
            .field("issuer", &self.issuer)
            .field("client_id", &self.client_id)
            .field("client_secret", &self.client_secret.as_ref().map(|_| "<set>"))
            .field("auth_method", &self.auth_method)
            .finish()
    }
}

/// OAuth tokens for an http MCP server, with what a refresh needs. Deliberately not `Serialize`;
/// the API only exposes [`crate::mcp::OAuthStatus`].
#[derive(Clone, PartialEq, FromRow)]
pub struct McpOAuthToken {
    pub server_id: String,
    pub access_token: String,
    pub refresh_token: Option<String>,
    pub expires_at: Option<DateTime<Utc>>,
    pub token_endpoint: String,
    pub client_id: String,
    pub client_secret: Option<String>,
    pub auth_method: String,
    pub resource: String,
}

impl McpOAuthToken {
    pub fn secret_values(&self) -> Vec<String> {
        [Some(&self.access_token), self.refresh_token.as_ref(), self.client_secret.as_ref()]
            .into_iter()
            .flatten()
            .cloned()
            .collect()
    }
}

/// Never prints token values.
impl std::fmt::Debug for McpOAuthToken {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("McpOAuthToken")
            .field("server_id", &self.server_id)
            .field("refresh_token", &self.refresh_token.as_ref().map(|_| "<set>"))
            .field("expires_at", &self.expires_at)
            .field("token_endpoint", &self.token_endpoint)
            .field("client_id", &self.client_id)
            .field("auth_method", &self.auth_method)
            .field("resource", &self.resource)
            .finish()
    }
}
