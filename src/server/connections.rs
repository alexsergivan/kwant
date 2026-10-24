//! Connections: one list of everything kwnt's bots can reach outside the app, with its status,
//! last check and the bots that use it. MCP servers, engine accounts, website logins in the
//! vault, and the mail (IMAP) and GitHub sources of Loose ends.
//!
//! Composio is offered as an optional MCP connector, off until the person adds it. It keeps the
//! person's app sign-ins on Composio's servers, so every entry says so; logins otherwise stay in
//! kwnt's own vault. Person-only (`auth::person_only`): the list names saved logins.

use super::loose_ends_owner::{self, OwnerSettings};
use super::mcp_suggest::refuse_agent_tokens;
use super::spaces::Scope;
use super::AppState;
use crate::db::models::{Kwant, McpServer};
use crate::db::spaces::SpaceId;
use crate::mcp::{self, composio, McpServerInput};
use crate::profiles::Engine;
use crate::sandbox::credentials::{CredentialStore, StoreError};
use axum::{
    extract::State,
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    Json,
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::{BTreeMap, HashMap};
use std::sync::{Mutex, OnceLock};

const REFUSAL: &str = "only the person manages connections";

/// The name the Composio connector gets as an MCP server.
pub const COMPOSIO_NAME: &str = "composio";
/// Said wherever Composio is offered or connected.
pub const COMPOSIO_WARNING: &str = "Composio keeps your app sign-ins on Composio's servers, not in kwant's vault, \
    and your bots' tool calls through it pass through Composio.";

/// A bot that uses a connection.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct UsedBy {
    pub id: String,
    pub name: String,
}

/// One connection, as `GET /api/connections` lists it. Never holds a secret.
#[derive(Debug, Clone, Serialize)]
pub struct Connection {
    /// `<kind>:<id>`, stable across calls.
    pub id: String,
    /// `mcp`, `account`, `browser_login`, `email` or `github`.
    pub kind: &'static str,
    pub name: String,
    /// What it points at: a host, an engine, `username @ origin`. Never a secret.
    pub detail: Option<String>,
    /// `connected`, `needs_sign_in`, `limited`, `error`, `off`, `locked` or `unknown`.
    pub status: &'static str,
    pub status_detail: Option<String>,
    pub last_check: Option<DateTime<Utc>>,
    pub used_by: Vec<UsedBy>,
    /// Where its sign-in lives: `vault` (kwnt's password vault or Bitwarden), `kwnt` (kwnt's
    /// database or the engine CLI's own login folder) or `composio` (Composio's servers).
    pub logins_stored: &'static str,
    /// The API that manages it.
    pub manage: String,
}

// Last MCP test results (POST /api/mcp/servers/:id/test), kept in memory

#[derive(Debug, Clone)]
struct McpCheck {
    ok: bool,
    auth_required: bool,
    error: Option<String>,
    at: DateTime<Utc>,
}

fn mcp_checks() -> &'static Mutex<HashMap<String, McpCheck>> {
    static CHECKS: OnceLock<Mutex<HashMap<String, McpCheck>>> = OnceLock::new();
    CHECKS.get_or_init(Default::default)
}

/// Remembers the outcome of an MCP server test (already redacted) as its last check.
pub fn record_mcp_check(server_id: &str, outcome: &Value) {
    let check = McpCheck {
        ok: outcome["ok"] == json!(true),
        auth_required: outcome["auth_required"] == json!(true),
        error: outcome["error"].as_str().map(|e| cut(e, 300)),
        at: Utc::now(),
    };
    mcp_checks().lock().unwrap_or_else(|e| e.into_inner()).insert(server_id.to_string(), check);
}

fn cut(text: &str, max: usize) -> String {
    match text.char_indices().nth(max) {
        Some((i, _)) => format!("{}…", &text[..i]),
        None => text.to_string(),
    }
}

fn host_of(url: &str) -> Option<String> {
    url::Url::parse(url).ok().and_then(|u| u.host_str().map(str::to_string))
}

/// Whether `url` is one of Composio's MCP endpoints.
pub fn is_composio_url(url: &str) -> bool {
    url::Url::parse(url).ok().is_some_and(|u| {
        u.scheme() == "https" && u.host_str().is_some_and(|h| h == "composio.dev" || h.ends_with(".composio.dev"))
    })
}

fn is_composio(server: &McpServer) -> bool {
    server.url.as_deref().is_some_and(is_composio_url)
}

fn used(kwants: &[&Kwant]) -> Vec<UsedBy> {
    let mut out: Vec<UsedBy> = kwants.iter().map(|k| UsedBy { id: k.id.clone(), name: k.name.clone() }).collect();
    out.sort_by_key(|u| (u.name.to_lowercase(), u.id.clone()));
    out.dedup_by(|a, b| a.id == b.id);
    out
}

/// The status of an MCP server from its switch, sign-in and last test.
fn mcp_status(server: &McpServer, oauth_connected: bool, check: Option<&McpCheck>) -> (&'static str, Option<String>) {
    if !server.enabled {
        return ("off", None);
    }
    match check {
        Some(c) if c.ok => ("connected", None),
        Some(c) if c.auth_required => ("needs_sign_in", Some("Sign-in required".into())),
        Some(c) => ("error", c.error.clone()),
        None if oauth_connected => ("connected", Some("Signed in; not tested since kwant started".into())),
        None => ("unknown", Some("Not tested since kwant started".into())),
    }
}

async fn mcp_connections(state: &AppState, space: &SpaceId, kwants: &[Kwant]) -> Vec<Connection> {
    let servers = state.db.list_mcp_servers(space).await.unwrap_or_default();
    let oauth = super::mcp_oauth::statuses(state).await;
    let opt_outs = state.db.list_mcp_kwant_opt_outs(space).await.unwrap_or_default();
    let checks = mcp_checks().lock().unwrap_or_else(|e| e.into_inner()).clone();
    servers
        .iter()
        .map(|s| {
            let (status, status_detail) =
                mcp_status(s, oauth.get(&s.id).is_some_and(|o| o.connected), checks.get(&s.id));
            let off_for = opt_outs.get(&s.id).cloned().unwrap_or_default();
            let users: Vec<&Kwant> = if s.enabled { kwants.iter().filter(|k| !off_for.contains(&k.id)).collect() } else { Vec::new() };
            let detail = match s.transport.as_str() {
                "http" => s.url.as_deref().and_then(host_of),
                // the command only: arguments can carry tokens
                _ => s.command.clone(),
            };
            Connection {
                id: format!("mcp:{}", s.id),
                kind: "mcp",
                name: s.name.clone(),
                detail,
                status,
                status_detail,
                last_check: checks.get(&s.id).map(|c| c.at),
                used_by: used(&users),
                logins_stored: if is_composio(s) { "composio" } else { "kwnt" },
                manage: format!("/api/mcp/servers/{}", s.id),
            }
        })
        .collect()
}

/// Bots that run on an account: those whose turns on its engine start there. A bot on "auto"
/// may run on any engine, a pinned one only on its own; on that engine the bot's bound account
/// comes first, else the engine's default (`preference_order`, as a turn picks it).
fn account_users<'a>(
    profile: &crate::db::models::LlmProfile,
    profiles: &[crate::db::models::LlmProfile],
    kwants: &'a [Kwant],
    bindings: &HashMap<String, (Option<String>, bool)>,
) -> Vec<&'a Kwant> {
    let Some(engine) = Engine::parse(&profile.engine) else {
        return Vec::new();
    };
    kwants
        .iter()
        .filter(|k| match crate::router::parse_pin(&k.preferred_model).family {
            None => true,
            Some(family) => family == engine.as_str(),
        })
        .filter(|k| {
            let bound = bindings.get(&k.id).and_then(|(p, _)| p.as_deref());
            crate::profiles::preference_order(profiles, engine, bound).first().is_some_and(|p| p.id == profile.id)
        })
        .collect()
}

async fn account_connections(state: &AppState, space: &SpaceId, kwants: &[Kwant]) -> Vec<Connection> {
    let profiles = state.db.list_profiles(space).await.unwrap_or_default();
    let bindings = state.db.list_kwant_bindings(space).await.unwrap_or_default();
    let users: HashMap<String, Vec<UsedBy>> =
        profiles.iter().map(|p| (p.id.clone(), used(&account_users(p, &profiles, kwants, &bindings)))).collect();
    super::profile_routes::views(state, profiles)
        .await
        .into_iter()
        .map(|p| {
            let custom_status = p.custom.as_ref().and_then(|c| c.status);
            let (status, status_detail): (&'static str, Option<String>) = if !p.enabled {
                ("off", None)
            } else if let Some(until) = p.limited_until {
                ("limited", Some(format!("Out of quota until {}", until.to_rfc3339())))
            } else if let Some(s) = custom_status {
                use crate::drivers::custom::CheckStatus as C;
                match s {
                    C::Ready => ("connected", None),
                    C::KeyRejected => ("needs_sign_in", Some("The key is missing or refused".into())),
                    C::Unreachable => ("error", Some("The server can't be reached".into())),
                    C::Error => ("error", p.custom.as_ref().and_then(|c| c.detail.clone())),
                }
            } else if p.last_error.as_deref() == Some("auth") || p.signed_in == Some(false) {
                ("needs_sign_in", None)
            } else if p.signed_in == Some(true) {
                ("connected", None)
            } else if let Some(e) = p.last_error.clone() {
                ("error", Some(e))
            } else {
                ("unknown", None)
            };
            let detail = match (&p.custom, &p.account_label) {
                (Some(c), _) => host_of(&c.base_url).or_else(|| Some(c.base_url.clone())),
                (None, Some(label)) => Some(format!("{} · {label}", p.engine)),
                (None, None) => Some(p.engine.clone()),
            };
            Connection {
                id: format!("account:{}", p.id),
                kind: "account",
                name: p.name.clone(),
                detail,
                status,
                status_detail,
                last_check: p.custom.as_ref().and_then(|c| c.checked_at),
                used_by: users.get(&p.id).cloned().unwrap_or_default(),
                logins_stored: "kwnt",
                manage: format!("/api/profiles/{}", p.id),
            }
        })
        .collect()
}

fn members(lead: &str, members_json: &str) -> Vec<String> {
    let mut ids: Vec<String> = serde_json::from_str(members_json).unwrap_or_default();
    ids.push(lead.to_string());
    ids
}

/// Saved website logins of every team, channel and bot in the space. Metadata only. `Err` when
/// the vault is locked (Bitwarden) or unavailable.
async fn login_connections(state: &AppState, space: &SpaceId, kwants: &[Kwant]) -> Result<Vec<Connection>, StoreError> {
    let by_id: HashMap<&str, &Kwant> = kwants.iter().map(|k| (k.id.as_str(), k)).collect();
    let pick = |ids: Vec<String>| -> Vec<&Kwant> { ids.iter().filter_map(|id| by_id.get(id.as_str()).copied()).collect() };
    // (owner id, owner label, bots that use the owner's browser)
    let mut owners: Vec<(String, String, Vec<&Kwant>)> = Vec::new();
    for t in state.db.list_teams(space).await.unwrap_or_default() {
        owners.push((t.id.clone(), format!("team {}", t.name), pick(members(&t.lead_kwant_id, &t.member_kwants))));
    }
    for c in state.db.list_channels(space).await.unwrap_or_default() {
        owners.push((c.id.clone(), format!("channel {}", c.name), pick(members(&c.lead_kwant_id, &c.member_kwants))));
    }
    for k in kwants {
        owners.push((k.id.clone(), k.name.clone(), vec![k]));
    }
    let mut out = Vec::new();
    for (owner, label, users) in owners {
        for m in state.credentials.list(&owner).await? {
            out.push(Connection {
                id: format!("login:{owner}:{}", m.id),
                kind: "browser_login",
                name: host_of(&m.origin).unwrap_or_else(|| m.origin.clone()),
                detail: Some(format!("{} @ {} · saved for {label}", m.username, m.origin)),
                status: "connected",
                status_detail: None,
                last_check: DateTime::parse_from_rfc3339(&m.last_used).ok().map(|t| t.with_timezone(&Utc)),
                used_by: used(&users),
                logins_stored: "vault",
                manage: format!("/api/sandbox/{owner}/passwords"),
            });
        }
    }
    Ok(out)
}

/// The mail (IMAP) and GitHub sources of Loose ends, used by the space's Loose ends bot.
async fn source_connections(state: &AppState, space: &SpaceId) -> Vec<Connection> {
    let settings = OwnerSettings::load(&state.db, space).await.unwrap_or_default();
    let owner = if settings.enabled { loose_ends_owner::owner(&state.db, space, &settings).await } else { None };
    let users: Vec<UsedBy> = owner.map(|k| vec![UsedBy { id: k.id, name: k.name }]).unwrap_or_default();
    state
        .db
        .zombie_sources()
        .await
        .unwrap_or_default()
        .into_iter()
        .filter_map(|s| {
            let config: Value = serde_json::from_str(&s.config).unwrap_or(Value::Null);
            let (kind, detail) = match s.kind.as_str() {
                "imap" => {
                    let host = config["host"].as_str().unwrap_or_default();
                    let user = config["username"].as_str().unwrap_or_default();
                    ("email", Some(format!("{user} @ {host}")))
                }
                "github" => ("github", config["api_base"].as_str().and_then(host_of).or_else(|| Some("api.github.com".into()))),
                _ => return None,
            };
            let (status, status_detail) = match (s.enabled, s.last_status.as_deref()) {
                (false, _) => ("off", None),
                (true, Some("ok")) => ("connected", None),
                (true, Some(_)) => ("error", s.last_error.clone()),
                (true, None) => ("unknown", Some("Not checked yet".into())),
            };
            Some(Connection {
                id: format!("source:{}", s.id),
                kind,
                name: s.label,
                detail,
                status,
                status_detail,
                last_check: s.last_scan_at,
                used_by: users.clone(),
                logins_stored: "vault",
                manage: format!("/api/zombie/sources/{}", s.id),
            })
        })
        .collect()
}

/// The Composio API key saved with the connector (its `x-api-key` header), if any.
fn composio_key(server: &McpServer) -> Option<&str> {
    server
        .headers
        .iter()
        .find(|(k, _)| k.eq_ignore_ascii_case("x-api-key"))
        .map(|(_, v)| v.trim())
        .filter(|v| !v.is_empty())
}

/// The optional connectors kwnt offers, with whether each is added.
fn catalog(servers: &[McpServer]) -> Value {
    let added = servers.iter().find(|s| is_composio(s));
    json!([{
        "id": "composio",
        "kind": "mcp",
        "name": "Composio",
        "description": "Hundreds of apps (Gmail, Slack, Notion, Linear…) through one MCP server you set up at composio.dev.",
        "optional": true,
        "added": added.is_some(),
        "connection_id": added.map(|s| format!("mcp:{}", s.id)),
        "enabled": added.is_some_and(|s| s.enabled),
        "logins_stored": "composio",
        "warning": COMPOSIO_WARNING,
        "add": "/api/connections/composio",
        // Finding apps needs the connector's API key; connecting one also needs the URL's user_id.
        "can_search": added.is_some_and(|s| composio_key(s).is_some()),
        "can_connect": added.is_some_and(|s| composio_key(s).is_some() && s.url.as_deref().and_then(composio::user_id_of).is_some()),
        "search": "/api/connections/composio/apps",
    }])
}

/// `GET /api/connections?space=`: every connection of the space, a vault summary and the
/// optional connectors.
pub async fn list(State(state): State<AppState>, headers: HeaderMap, Scope(space): Scope) -> Response {
    if let Err(r) = refuse_agent_tokens(&state, &headers, REFUSAL).await {
        return r;
    }
    let kwants = state.db.list_kwants(&space).await.unwrap_or_default();
    let mut connections = mcp_connections(&state, &space, &kwants).await;
    connections.extend(account_connections(&state, &space, &kwants).await);
    let vault = match login_connections(&state, &space, &kwants).await {
        Ok(logins) => {
            let n = logins.len();
            connections.extend(logins);
            json!({"status": "ok", "logins": n})
        }
        Err(StoreError::Locked) => json!({"status": "locked", "detail": "Unlock Bitwarden to list saved logins"}),
        Err(e) => {
            tracing::warn!(error = %e, "connections: could not list saved logins");
            json!({"status": "unavailable", "detail": "Saved logins can't be read right now"})
        }
    };
    connections.extend(source_connections(&state, &space).await);
    let servers = state.db.list_mcp_servers(&space).await.unwrap_or_default();
    let mut counts: BTreeMap<&str, usize> = BTreeMap::new();
    for c in &connections {
        *counts.entry(c.status).or_default() += 1;
    }
    Json(json!({
        "connections": connections,
        "counts": counts,
        "vault": vault,
        "catalog": catalog(&servers),
    }))
    .into_response()
}

#[derive(Debug, Deserialize)]
pub struct ComposioBody {
    /// The MCP URL Composio gives for the person's server, `https://….composio.dev/…`.
    pub url: String,
    /// Composio API key, sent as `x-api-key` (stored like any MCP header, masked in every view).
    #[serde(default)]
    pub api_key: Option<String>,
    /// Off unless the person says so: an added connector reaches no bot until it is switched on.
    #[serde(default)]
    pub enabled: bool,
}

fn error(status: StatusCode, message: &str) -> Response {
    (status, Json(json!({"error": message}))).into_response()
}

/// `POST /api/connections/composio?space=`: adds Composio as an MCP server (off by default).
pub async fn add_composio(
    State(state): State<AppState>,
    headers: HeaderMap,
    Scope(space): Scope,
    Json(body): Json<ComposioBody>,
) -> Response {
    if let Err(r) = refuse_agent_tokens(&state, &headers, REFUSAL).await {
        return r;
    }
    let url = body.url.trim();
    if !is_composio_url(url) {
        return error(StatusCode::BAD_REQUEST, "url must be an https address on composio.dev");
    }
    let servers = state.db.list_mcp_servers(&space).await.unwrap_or_default();
    if servers.iter().any(|s| is_composio(s) || s.name.eq_ignore_ascii_case(COMPOSIO_NAME)) {
        return error(StatusCode::CONFLICT, "Composio is already added; change it under its MCP server");
    }
    let headers: BTreeMap<String, String> = body
        .api_key
        .map(|k| k.trim().to_string())
        .filter(|k| !k.is_empty())
        .map(|k| BTreeMap::from([("x-api-key".to_string(), k)]))
        .unwrap_or_default();
    let input = McpServerInput {
        name: COMPOSIO_NAME.into(),
        transport: "http".into(),
        command: None,
        args: None,
        url: Some(url.to_string()),
        env: None,
        headers: Some(headers),
        enabled: Some(body.enabled),
        timeout_secs: None,
    };
    let server = match mcp::build_server(input, None) {
        Ok(s) => s,
        Err(m) => return error(StatusCode::BAD_REQUEST, &m),
    };
    match state.db.create_mcp_server(&space, &server).await {
        Ok(created) => Json(json!({
            "connection_id": format!("mcp:{}", created.id),
            "server": mcp::McpServerView::from(&created),
            "warning": COMPOSIO_WARNING,
        }))
        .into_response(),
        Err(e) => {
            tracing::warn!(error = %e, "connections: could not add Composio");
            error(StatusCode::INTERNAL_SERVER_ERROR, "could not add Composio")
        }
    }
}

/// The space's Composio connector and its API key, or the reply that explains what's missing.
/// Nothing here calls Composio.
#[allow(clippy::result_large_err)]
async fn composio_with_key(state: &AppState, space: &SpaceId) -> Result<(McpServer, String), Response> {
    let servers = state.db.list_mcp_servers(space).await.unwrap_or_default();
    let Some(server) = servers.into_iter().find(is_composio) else {
        return Err(error(StatusCode::CONFLICT, "Add the Composio connector first; kwant never contacts Composio before that"));
    };
    let Some(key) = composio_key(&server).map(str::to_string) else {
        return Err(error(StatusCode::CONFLICT, "Finding apps needs your Composio API key: add it as the x-api-key header of the Composio MCP server"));
    };
    Ok((server, key))
}

fn composio_failed(e: composio::ComposioError) -> Response {
    let status = match e {
        composio::ComposioError::KeyRefused => StatusCode::CONFLICT,
        _ => StatusCode::BAD_GATEWAY,
    };
    error(status, &e.message())
}

#[derive(Debug, Deserialize)]
pub struct AppSearch {
    #[serde(default)]
    pub q: String,
}

/// `GET /api/connections/composio/apps?space=&q=`: apps in Composio's catalogue matching `q`,
/// searched with the person's own Composio key. 409 when Composio isn't added or has no key.
pub async fn search_composio_apps(
    State(state): State<AppState>,
    headers: HeaderMap,
    Scope(space): Scope,
    axum::extract::Query(query): axum::extract::Query<AppSearch>,
) -> Response {
    if let Err(r) = refuse_agent_tokens(&state, &headers, REFUSAL).await {
        return r;
    }
    let q = query.q.trim();
    if q.is_empty() || q.chars().count() > composio::MAX_QUERY_CHARS {
        return error(StatusCode::BAD_REQUEST, "type an app name, up to 60 characters");
    }
    let (server, key) = match composio_with_key(&state, &space).await {
        Ok(found) => found,
        Err(r) => return r,
    };
    match state.composio_api.search(&key, q).await {
        Ok(apps) => Json(json!({
            "apps": apps,
            "can_connect": server.url.as_deref().and_then(composio::user_id_of).is_some(),
            "warning": COMPOSIO_WARNING,
        }))
        .into_response(),
        Err(e) => composio_failed(e),
    }
}

/// `POST /api/connections/composio/apps/:slug/connect?space=`: starts signing in to one app on
/// Composio for the user in the connector's URL (`user_id`), and returns the address the person
/// opens. The sign-in itself happens on Composio's site.
pub async fn connect_composio_app(
    State(state): State<AppState>,
    headers: HeaderMap,
    Scope(space): Scope,
    axum::extract::Path(slug): axum::extract::Path<String>,
) -> Response {
    if let Err(r) = refuse_agent_tokens(&state, &headers, REFUSAL).await {
        return r;
    }
    if !composio::valid_slug(&slug) {
        return error(StatusCode::BAD_REQUEST, "not a Composio app name");
    }
    let (server, key) = match composio_with_key(&state, &space).await {
        Ok(found) => found,
        Err(r) => return r,
    };
    let Some(user_id) = server.url.as_deref().and_then(composio::user_id_of) else {
        return error(
            StatusCode::CONFLICT,
            "Your Composio URL has no user_id, so kwant can't tell which Composio user to connect the app to. Connect it on composio.dev instead",
        );
    };
    match state.composio_api.connect_link(&key, &user_id, &slug).await {
        Ok(redirect_url) => Json(json!({"redirect_url": redirect_url, "warning": COMPOSIO_WARNING})).into_response(),
        Err(e) => composio_failed(e),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn server(enabled: bool, url: Option<&str>) -> McpServer {
        McpServer {
            id: "s1".into(),
            name: "x".into(),
            transport: "http".into(),
            command: None,
            args: vec![],
            url: url.map(str::to_string),
            env: Default::default(),
            headers: Default::default(),
            enabled,
            timeout_secs: 30,
            created_at: None,
        }
    }

    #[test]
    fn composio_urls_are_https_on_composio_dev_only() {
        assert!(is_composio_url("https://backend.composio.dev/v3/mcp/abc/mcp?user_id=u"));
        assert!(is_composio_url("https://mcp.composio.dev/partner/x"));
        assert!(!is_composio_url("http://backend.composio.dev/v3/mcp/abc"), "no plain http");
        assert!(!is_composio_url("https://composio.dev.evil.example/mcp"));
        assert!(!is_composio_url("https://evilcomposio.dev/mcp"));
        assert!(!is_composio_url("not a url"));
    }

    #[test]
    fn mcp_status_follows_switch_then_last_test() {
        let check = |ok, auth_required, error: Option<&str>| McpCheck { ok, auth_required, error: error.map(str::to_string), at: Utc::now() };
        assert_eq!(mcp_status(&server(false, None), true, Some(&check(true, false, None))).0, "off");
        assert_eq!(mcp_status(&server(true, None), false, Some(&check(true, false, None))).0, "connected");
        assert_eq!(mcp_status(&server(true, None), false, Some(&check(false, true, None))).0, "needs_sign_in");
        assert_eq!(mcp_status(&server(true, None), false, Some(&check(false, false, Some("boom")))), ("error", Some("boom".into())));
        assert_eq!(mcp_status(&server(true, None), true, None).0, "connected");
        assert_eq!(mcp_status(&server(true, None), false, None).0, "unknown");
    }

    #[test]
    fn a_test_outcome_becomes_the_last_check() {
        record_mcp_check("conn-unit", &json!({"ok": false, "error": "x".repeat(400)}));
        let c = mcp_checks().lock().unwrap().get("conn-unit").cloned().unwrap();
        assert!(!c.ok && !c.auth_required);
        assert_eq!(c.error.unwrap().chars().count(), 301, "cut to 300 plus the ellipsis");
    }
}
