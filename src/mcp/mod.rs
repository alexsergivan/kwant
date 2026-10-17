//! Model Context Protocol support: the server registry's validation and secret masking, a small
//! MCP client (stdio + streamable HTTP), and the per-turn context drivers use to offer MCP tools.

pub mod cli_config;
pub mod client;
pub mod echo_server;
pub mod oauth;
pub mod oauth_mock;
pub mod session;

pub use client::{McpClient, McpError, McpTool};
pub use oauth::OAuthStatus;

use crate::db::models::McpServer;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

/// Shown in place of every secret value; sending it back on update keeps the stored value.
pub const SECRET_MASK: &str = "********";
/// Server name kwnt uses for its own permission endpoint in agent CLI configs.
pub const RESERVED_SERVER_NAME: &str = "kwnt";
/// Server name of the agents' own continuity tools (`mcp__kwnt_tools__memory_add`, …).
pub const SELF_SERVER_NAME: &str = "kwnt_tools";
pub const DEFAULT_TIMEOUT_SECS: i64 = 30;
pub const DEFAULT_MAX_TOOL_ITERATIONS: usize = 8;
/// Longest tool result kept in a message's `tool_calls` record.
pub const RECORD_RESULT_MAX_CHARS: usize = 4000;
/// How long an MCP tool call waits for the user's approval before it is denied.
pub const APPROVAL_TIMEOUT_SECS: u64 = 300;
/// kwnt's permission-prompt tool for the claude CLI, called as `mcp__kwnt__approve`.
pub const PERMISSION_TOOL: &str = "approve";
/// Env variable that hands an agent CLI its turn's gateway token (never put in argv).
pub const GATEWAY_TOKEN_ENV: &str = "KWNT_MCP_GATEWAY_TOKEN";

/// API representation of a server: secrets replaced by [`SECRET_MASK`].
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct McpServerView {
    pub id: String,
    pub name: String,
    pub transport: String,
    pub command: Option<String>,
    pub args: Vec<String>,
    pub url: Option<String>,
    pub env: BTreeMap<String, String>,
    pub headers: BTreeMap<String, String>,
    pub enabled: bool,
    pub timeout_secs: i64,
    pub created_at: Option<chrono::DateTime<chrono::Utc>>,
    /// OAuth sign-in status (http servers); filled in by the routes, never contains tokens.
    #[serde(default)]
    pub oauth: OAuthStatus,
    /// Kwants that have this server switched off; every other kwant gets it. Filled in by the routes.
    #[serde(default)]
    pub disabled_for_kwants: Vec<String>,
}

impl From<&McpServer> for McpServerView {
    fn from(s: &McpServer) -> Self {
        let mask = |m: &BTreeMap<String, String>| m.keys().map(|k| (k.clone(), SECRET_MASK.to_string())).collect();
        Self {
            id: s.id.clone(),
            name: s.name.clone(),
            transport: s.transport.clone(),
            command: s.command.clone(),
            args: s.args.clone(),
            url: s.url.clone(),
            env: mask(&s.env),
            headers: mask(&s.headers),
            enabled: s.enabled,
            timeout_secs: s.timeout_secs,
            created_at: s.created_at,
            oauth: OAuthStatus::default(),
            disabled_for_kwants: Vec::new(),
        }
    }
}

/// Body of create and update requests.
#[derive(Debug, Clone, Deserialize)]
pub struct McpServerInput {
    pub name: String,
    pub transport: String,
    #[serde(default)]
    pub command: Option<String>,
    #[serde(default)]
    pub args: Option<Vec<String>>,
    #[serde(default)]
    pub url: Option<String>,
    #[serde(default)]
    pub env: Option<BTreeMap<String, String>>,
    #[serde(default)]
    pub headers: Option<BTreeMap<String, String>>,
    #[serde(default)]
    pub enabled: Option<bool>,
    #[serde(default)]
    pub timeout_secs: Option<i64>,
}

fn valid_server_name(name: &str) -> bool {
    let first_ok = name.chars().next().is_some_and(|c| c.is_ascii_alphanumeric());
    first_ok
        && name.len() <= 48
        && !name.contains("__")
        && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
}

fn valid_env_key(key: &str) -> bool {
    let first_ok = key.chars().next().is_some_and(|c| c.is_ascii_alphabetic() || c == '_');
    first_ok && key.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
}

fn valid_header_name(name: &str) -> bool {
    !name.is_empty() && name.chars().all(|c| c.is_ascii_alphanumeric() || "!#$%&'*+-.^_`|~".contains(c))
}

/// Applies the masking rules: a masked value keeps the stored one, a missing map keeps them all.
fn merge_secrets(
    incoming: Option<BTreeMap<String, String>>,
    existing: Option<&BTreeMap<String, String>>,
    what: &str,
) -> Result<BTreeMap<String, String>, String> {
    let Some(incoming) = incoming else {
        return Ok(existing.cloned().unwrap_or_default());
    };
    let mut merged = BTreeMap::new();
    for (key, value) in incoming {
        if value == SECRET_MASK {
            match existing.and_then(|e| e.get(&key)) {
                Some(stored) => merged.insert(key, stored.clone()),
                None => return Err(format!("{what} '{key}' has no stored value to keep; send the real value")),
            };
        } else {
            merged.insert(key, value);
        }
    }
    Ok(merged)
}

/// Validates a create/update body and turns it into a server, merging masked secrets with
/// `existing` on update. The returned server keeps `existing`'s id and creation time.
pub fn build_server(input: McpServerInput, existing: Option<&McpServer>) -> Result<McpServer, String> {
    let name = input.name.trim().to_string();
    if !valid_server_name(&name) {
        return Err("name must be 1-48 characters of letters, digits, '_' or '-', start with a letter or digit and not contain '__'".into());
    }
    if name.eq_ignore_ascii_case(RESERVED_SERVER_NAME) || name.eq_ignore_ascii_case(SELF_SERVER_NAME) {
        return Err(format!("name '{name}' is reserved"));
    }
    let timeout_secs = input.timeout_secs.unwrap_or(DEFAULT_TIMEOUT_SECS);
    if !(1..=600).contains(&timeout_secs) {
        return Err("timeout_secs must be between 1 and 600".into());
    }
    let env = merge_secrets(input.env, existing.map(|e| &e.env), "env")?;
    let headers = merge_secrets(input.headers, existing.map(|e| &e.headers), "header")?;
    let command = input.command.map(|c| c.trim().to_string()).filter(|c| !c.is_empty());
    let url = input.url.map(|u| u.trim().to_string()).filter(|u| !u.is_empty());
    let args = input.args.unwrap_or_default();

    match input.transport.as_str() {
        "stdio" => {
            if command.is_none() {
                return Err("stdio servers need a command".into());
            }
            if url.is_some() {
                return Err("stdio servers take no url".into());
            }
            if !headers.is_empty() {
                return Err("headers only apply to http servers".into());
            }
            if let Some(bad) = env.keys().find(|k| !valid_env_key(k)) {
                return Err(format!("invalid env variable name '{bad}'"));
            }
        }
        "http" => {
            let Some(ref u) = url else {
                return Err("http servers need a url".into());
            };
            match reqwest::Url::parse(u) {
                Ok(parsed) if matches!(parsed.scheme(), "http" | "https") && parsed.host().is_some() => {}
                _ => return Err("url must be an http:// or https:// address".into()),
            }
            if command.is_some() || !args.is_empty() {
                return Err("http servers take no command or args".into());
            }
            if !env.is_empty() {
                return Err("env only applies to stdio servers".into());
            }
            if let Some(bad) = headers.keys().find(|k| !valid_header_name(k)) {
                return Err(format!("invalid header name '{bad}'"));
            }
        }
        _ => return Err("transport must be 'stdio' or 'http'".into()),
    }

    Ok(McpServer {
        id: existing.map(|e| e.id.clone()).unwrap_or_default(),
        name,
        transport: input.transport,
        command,
        args,
        url,
        env,
        headers,
        enabled: input.enabled.unwrap_or(true),
        timeout_secs,
        created_at: existing.and_then(|e| e.created_at),
    })
}

/// Replaces every non-empty secret in `text` with [`SECRET_MASK`].
pub fn redact(text: &str, secrets: &[String]) -> String {
    let mut out = text.to_string();
    let mut sorted: Vec<&String> = secrets.iter().filter(|s| !s.is_empty()).collect();
    // Longest first, so a secret containing another is masked whole
    sorted.sort_by_key(|s| std::cmp::Reverse(s.len()));
    for secret in sorted {
        out = out.replace(secret.as_str(), SECRET_MASK);
    }
    out
}

/// Cuts `text` to at most `max` characters, marking the cut.
pub fn truncate_text(text: &str, max: usize) -> String {
    match text.char_indices().nth(max) {
        Some((idx, _)) => format!("{}… [truncated]", &text[..idx]),
        None => text.to_string(),
    }
}

/// `mcp__<server>__<tool>`, the name agents see for an MCP tool.
pub fn namespaced_tool_name(server: &str, tool: &str) -> String {
    format!("mcp__{server}__{tool}")
}

/// Splits `mcp__<server>__<tool>`; server names cannot contain `__`, so the first split is right.
pub fn split_tool_name(name: &str) -> Option<(&str, &str)> {
    let rest = name.strip_prefix("mcp__")?;
    let (server, tool) = rest.split_once("__")?;
    (!server.is_empty() && !tool.is_empty()).then_some((server, tool))
}

/// One tool call waiting for the user's approval.
#[derive(Debug, Clone, Serialize)]
pub struct ToolApprovalRequest {
    pub kwant_id: String,
    pub kwant_name: String,
    pub turn_id: String,
    /// Namespaced name, e.g. `mcp__github__create_issue`.
    pub tool_name: String,
    pub server: String,
    pub tool: String,
    pub arguments: Value,
}

/// Decides whether an MCP tool call may run. The server implements it with the permission broker.
#[async_trait]
pub trait ToolApprover: Send + Sync {
    async fn approve(&self, request: ToolApprovalRequest) -> bool;
}

/// Approves or denies everything; for tests and tooling.
pub struct FixedApprover(pub bool);

#[async_trait]
impl ToolApprover for FixedApprover {
    async fn approve(&self, _request: ToolApprovalRequest) -> bool {
        self.0
    }
}

/// What a driver needs to offer MCP tools on one turn. Only set when the kwant has servers.
#[derive(Clone)]
pub struct McpTurnContext {
    /// The kwant's enabled servers, secrets included.
    pub servers: Vec<McpServer>,
    pub approver: Arc<dyn ToolApprover>,
    /// Private directory for per-turn agent CLI config files.
    pub config_dir: PathBuf,
    /// kwnt's permission-prompt MCP endpoint for this turn (claude CLI only).
    pub permission_url: Option<String>,
    /// kwnt's MCP gateway for this turn, for agent CLIs that can't route approvals themselves.
    pub gateway: Option<GatewayAccess>,
    pub max_tool_iterations: usize,
    /// Tool call records for the reply's `tool_calls` field; drivers and the permission endpoint append.
    pub records: Arc<Mutex<Vec<Value>>>,
}

/// Where an agent CLI reaches this turn's MCP gateway. The token is sent as a bearer token.
#[derive(Clone)]
pub struct GatewayAccess {
    pub url: String,
    pub token: String,
}

impl GatewayAccess {
    /// The gateway's grok naming mode (`POST /api/mcp/gateway/grok`): no `__` in tool names.
    pub fn grok_url(&self) -> String {
        format!("{}/grok", self.url.trim_end_matches('/'))
    }
}

impl std::fmt::Debug for GatewayAccess {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GatewayAccess").field("url", &self.url).field("token", &"<redacted>").finish()
    }
}

impl McpTurnContext {
    pub fn record(&self, record: Value) {
        if let Ok(mut r) = self.records.lock() {
            r.push(record);
        }
    }

    pub fn take_records(&self) -> Vec<Value> {
        self.records.lock().map(|mut r| std::mem::take(&mut *r)).unwrap_or_default()
    }

    pub fn secret_values(&self) -> Vec<String> {
        self.servers.iter().flat_map(|s| s.secret_values()).collect()
    }
}

impl std::fmt::Debug for McpTurnContext {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("McpTurnContext")
            .field("servers", &self.servers)
            .field("config_dir", &self.config_dir)
            .field("permission_url", &self.permission_url.as_ref().map(|_| "<set>"))
            .field("gateway", &self.gateway)
            .field("max_tool_iterations", &self.max_tool_iterations)
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn input(json: Value) -> McpServerInput {
        serde_json::from_value(json).unwrap()
    }

    #[test]
    fn validates_names_and_transports() {
        assert!(build_server(input(serde_json::json!({"name": "ok-name_1", "transport": "stdio", "command": "x"})), None).is_ok());
        for bad in ["", "-lead", "a__b", "has space", "kwnt", "KWNT", &"x".repeat(49)] {
            assert!(build_server(input(serde_json::json!({"name": bad, "transport": "stdio", "command": "x"})), None).is_err(), "{bad}");
        }
        assert!(build_server(input(serde_json::json!({"name": "a", "transport": "stdio"})), None).is_err());
        assert!(build_server(input(serde_json::json!({"name": "a", "transport": "http", "url": "ftp://x"})), None).is_err());
        assert!(build_server(input(serde_json::json!({"name": "a", "transport": "http", "url": "https://x.dev/mcp", "command": "y"})), None).is_err());
        assert!(build_server(input(serde_json::json!({"name": "a", "transport": "ws", "url": "https://x"})), None).is_err());
        assert!(build_server(input(serde_json::json!({"name": "a", "transport": "stdio", "command": "x", "env": {"1BAD": "v"}})), None).is_err());
        assert!(build_server(input(serde_json::json!({"name": "a", "transport": "stdio", "command": "x", "timeout_secs": 0})), None).is_err());
    }

    #[test]
    fn masks_and_merges_secrets() {
        let created = build_server(
            input(serde_json::json!({"name": "gh", "transport": "stdio", "command": "x", "env": {"TOKEN": "s3cret", "OTHER": "o"}})),
            None,
        )
        .unwrap();
        let view = McpServerView::from(&created);
        assert_eq!(view.env.get("TOKEN").unwrap(), SECRET_MASK);
        assert!(!serde_json::to_string(&view).unwrap().contains("s3cret"));
        assert!(!format!("{created:?}").contains("s3cret"));

        // Mask keeps, new value replaces, omitted key deletes
        let updated = build_server(
            input(serde_json::json!({"name": "gh", "transport": "stdio", "command": "x", "env": {"TOKEN": SECRET_MASK, "NEW": "n"}})),
            Some(&created),
        )
        .unwrap();
        assert_eq!(updated.env.get("TOKEN").unwrap(), "s3cret");
        assert_eq!(updated.env.get("NEW").unwrap(), "n");
        assert!(!updated.env.contains_key("OTHER"));

        // Omitted map keeps everything
        let kept = build_server(input(serde_json::json!({"name": "gh", "transport": "stdio", "command": "x"})), Some(&created)).unwrap();
        assert_eq!(kept.env, created.env);

        // A mask for a key that was never stored is an error, not an empty secret
        assert!(build_server(
            input(serde_json::json!({"name": "gh", "transport": "stdio", "command": "x", "env": {"NOPE": SECRET_MASK}})),
            Some(&created)
        )
        .is_err());
    }

    #[test]
    fn redacts_and_splits_names() {
        assert_eq!(redact("token=abc123 and abc", &["abc123".into(), "".into()]), "token=******** and abc");
        assert_eq!(split_tool_name("mcp__gh__create_issue"), Some(("gh", "create_issue")));
        assert_eq!(split_tool_name("mcp__gh__a__b"), Some(("gh", "a__b")));
        assert_eq!(split_tool_name("Bash"), None);
        assert_eq!(namespaced_tool_name("gh", "x"), "mcp__gh__x");
        assert_eq!(truncate_text("héllo", 2), "hé… [truncated]");
    }
}
