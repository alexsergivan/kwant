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
use std::sync::{Arc, Mutex, OnceLock};

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
    /// While `status` is `limited`: when the account's quota comes back. Screens format it in local time.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub limited_until: Option<DateTime<Utc>>,
    pub last_check: Option<DateTime<Utc>>,
    pub used_by: Vec<UsedBy>,
    /// Where its sign-in lives: `vault` (kwnt's password vault or Bitwarden), `kwnt` (kwnt's
    /// database or the engine CLI's own login folder) or `composio` (Composio's servers).
    pub logins_stored: &'static str,
    /// The API that manages it.
    pub manage: String,
    /// The person marked it production (D10): in a Dev mode space every call through it gets the
    /// production floor's card. Only MCP servers (and Composio apps, by toolkit slug) can be.
    pub production: bool,
    /// P5: open tool notes about this MCP server (`open`, and `broke`: "didn't work"); MCP rows only.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub notes: Option<NoteCounts>,
}

#[derive(Debug, Clone, Copy, Default, Serialize, PartialEq, Eq)]
pub struct NoteCounts {
    pub open: i64,
    pub broke: i64,
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

pub(super) fn is_composio(server: &McpServer) -> bool {
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
                limited_until: None,
                last_check: checks.get(&s.id).map(|c| c.at),
                used_by: used(&users),
                logins_stored: if is_composio(s) { "composio" } else { "kwnt" },
                manage: format!("/api/mcp/servers/{}", s.id),
                production: false,
                notes: None,
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
            } else if p.limited_until.is_some() {
                ("limited", Some("Out of quota".into()))
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
                limited_until: p.limited_until.filter(|_| status == "limited"),
                last_check: p.custom.as_ref().and_then(|c| c.checked_at),
                used_by: users.get(&p.id).cloned().unwrap_or_default(),
                logins_stored: "kwnt",
                manage: format!("/api/profiles/{}", p.id),
                production: false,
                notes: None,
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
                limited_until: None,
                last_check: DateTime::parse_from_rfc3339(&m.last_used).ok().map(|t| t.with_timezone(&Utc)),
                used_by: used(&users),
                logins_stored: "vault",
                manage: format!("/api/sandbox/{owner}/passwords"),
                production: false,
                notes: None,
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
                "forge" => {
                    let kind = match config["forge"].as_str() {
                        Some("gitlab") => "gitlab",
                        Some("forgejo") => "forgejo",
                        _ => "gitea",
                    };
                    (kind, config["url"].as_str().and_then(host_of))
                }
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
                limited_until: None,
                last_check: s.last_scan_at,
                used_by: users.clone(),
                logins_stored: "vault",
                manage: format!("/api/zombie/sources/{}", s.id),
                production: false,
                notes: None,
            })
        })
        .collect()
}

fn is_key_header(name: &str) -> bool {
    name.eq_ignore_ascii_case(composio::PROJECT_KEY_HEADER) || name.eq_ignore_ascii_case(composio::CONNECT_KEY_HEADER)
}

/// Whether the connector is a Composio Connect setup: its key is a `ck_` key, sent as
/// `x-consumer-api-key`.
fn is_connect(server: &McpServer) -> bool {
    server.headers.keys().any(|k| k.eq_ignore_ascii_case(composio::CONNECT_KEY_HEADER))
}

/// The Composio key saved with the connector (its `x-api-key` header, or `x-consumer-api-key`
/// for a Composio Connect key), if any.
pub(super) fn composio_key(server: &McpServer) -> Option<&str> {
    server
        .headers
        .iter()
        .find(|(k, _)| is_key_header(k))
        .map(|(_, v)| v.trim())
        .filter(|v| !v.is_empty())
}

/// The session kwant made for a space's Composio (the one-key setup), kept in `settings` under
/// `composio_session:<space>`. It describes the space's server only while that server's URL is
/// still `mcp_url`: a server re-added or edited by hand is a URL setup again.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
struct StoredSession {
    session_id: String,
    user_id: String,
    mcp_url: String,
}

fn session_setting(space: &SpaceId) -> String {
    format!("composio_session:{}", space.as_str())
}

async fn stored_session(state: &AppState, space: &SpaceId, server: &McpServer) -> Option<StoredSession> {
    let raw = state.db.get_setting(&session_setting(space)).await.ok().flatten()?;
    let s: StoredSession = serde_json::from_str(&raw).ok()?;
    (server.url.as_deref() == Some(s.mcp_url.as_str())).then_some(s)
}

async fn save_session(state: &AppState, space: &SpaceId, s: &composio::Session) -> Result<(), crate::db::DbError> {
    let stored = StoredSession { session_id: s.session_id.clone(), user_id: s.user_id.clone(), mcp_url: s.mcp_url.clone() };
    state.db.set_setting(&session_setting(space), &serde_json::to_string(&stored).unwrap_or_default()).await
}

/// How a space's Composio is set up: `session` (one `ak_` key; kwant made the session),
/// `connect` (one `ck_` key on Composio Connect, whose apps belong to the key's own Composio
/// account) or `url` (an MCP URL pasted from composio.dev).
#[derive(Debug, Clone)]
struct ComposioSetup {
    server: McpServer,
    key: Option<String>,
    session: Option<StoredSession>,
    connect: bool,
}

impl ComposioSetup {
    fn mode(&self) -> &'static str {
        if self.connect {
            "connect"
        } else if self.session.is_some() {
            "session"
        } else {
            "url"
        }
    }

    /// The Composio user apps are connected for: the session's, else the URL's `user_id`.
    /// Composio Connect has none: the key is the person's own account.
    fn user_id(&self) -> Option<String> {
        if self.connect {
            return None;
        }
        match &self.session {
            Some(s) => Some(s.user_id.clone()),
            None => self.server.url.as_deref().and_then(composio::user_id_of),
        }
    }

    /// Apps can be connected from kwant: a key, and someone to connect them for.
    fn can_connect(&self) -> bool {
        self.key.is_some() && (self.connect || self.user_id().is_some())
    }
}

async fn composio_setup(state: &AppState, space: &SpaceId) -> Option<ComposioSetup> {
    let servers = state.db.list_mcp_servers(space).await.unwrap_or_default();
    let server = servers.into_iter().find(is_composio)?;
    let connect = is_connect(&server);
    let session = if connect { None } else { stored_session(state, space, &server).await };
    let key = composio_key(&server).map(str::to_string);
    Some(ComposioSetup { server, key, session, connect })
}

// Apps connected on Composio, per space, kept a minute so the Connections list costs at most
// one Composio call a minute. Keyed by space and Composio client (one per server; tests make
// their own), and fingerprinted by key and user, so a new key or user starts over.

fn cache_key(state: &AppState, space: &SpaceId) -> String {
    format!("{:p}:{}", Arc::as_ptr(&state.composio_api), space.as_str())
}

const APPS_TTL: std::time::Duration = std::time::Duration::from_secs(60);

#[derive(Debug, Clone)]
struct AppsSeen {
    at: std::time::Instant,
    fingerprint: u64,
    apps: std::collections::BTreeSet<String>,
    /// The last call's failure, with the one action that fixes it.
    problem: Option<(String, &'static str)>,
}

fn apps_seen() -> &'static Mutex<HashMap<String, AppsSeen>> {
    static SEEN: OnceLock<Mutex<HashMap<String, AppsSeen>>> = OnceLock::new();
    SEEN.get_or_init(Default::default)
}

/// App names learned from catalogue searches (Composio's account list only has slugs).
fn app_names() -> &'static Mutex<HashMap<String, String>> {
    static NAMES: OnceLock<Mutex<HashMap<String, String>>> = OnceLock::new();
    NAMES.get_or_init(Default::default)
}

fn learn_names(apps: &[composio::ComposioApp]) {
    let mut names = app_names().lock().unwrap_or_else(|e| e.into_inner());
    if names.len() > 5000 {
        names.clear();
    }
    for a in apps {
        names.insert(a.slug.clone(), a.name.clone());
    }
}

/// An app's name: the catalogue's when kwant has seen it, else the popular list's, else its
/// slug in words (`google_drive` → `Google Drive`).
fn app_name(slug: &str) -> String {
    if let Some(n) = app_names().lock().unwrap_or_else(|e| e.into_inner()).get(slug) {
        return n.clone();
    }
    if let Some(a) = composio::popular_apps().iter().find(|a| a.slug == slug) {
        return a.name.clone();
    }
    slug.split(['_', '-'])
        .filter(|w| !w.is_empty())
        .map(|w| {
            let mut c = w.chars();
            c.next().map(|f| f.to_uppercase().chain(c).collect::<String>()).unwrap_or_default()
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn fingerprint(key: &str, user: &str) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    (key, user).hash(&mut h);
    h.finish()
}

fn remember_apps(space: &str, fp: u64, result: &Result<std::collections::BTreeSet<String>, composio::ComposioError>) {
    let mut seen = apps_seen().lock().unwrap_or_else(|e| e.into_inner());
    let entry = match result {
        Ok(apps) => AppsSeen { at: std::time::Instant::now(), fingerprint: fp, apps: apps.clone(), problem: None },
        Err(e) => {
            let apps = seen.get(space).filter(|s| s.fingerprint == fp).map(|s| s.apps.clone()).unwrap_or_default();
            AppsSeen { at: std::time::Instant::now(), fingerprint: fp, apps, problem: Some((e.message(), e.action())) }
        }
    };
    seen.insert(space.to_string(), entry);
}

/// Apps a status check found connected, per space and key fingerprint: with Composio Connect,
/// what the tile lists when the full check is too slow.
type ThroughKwant = HashMap<String, (u64, std::collections::BTreeSet<String>)>;

fn through_kwant() -> &'static Mutex<ThroughKwant> {
    static SEEN: OnceLock<Mutex<ThroughKwant>> = OnceLock::new();
    SEEN.get_or_init(Default::default)
}

fn apps_through_kwant(space: &str, fp: u64) -> std::collections::BTreeSet<String> {
    through_kwant().lock().unwrap_or_else(|e| e.into_inner()).get(space).filter(|(f, _)| *f == fp).map(|(_, a)| a.clone()).unwrap_or_default()
}

/// One app's status, learned from a status check: updates the cache without a call.
fn note_app(space: &str, fp: u64, slug: &str, connected: bool) {
    {
        let mut kept = through_kwant().lock().unwrap_or_else(|e| e.into_inner());
        if kept.len() > 1000 {
            kept.clear();
        }
        let entry = kept.entry(space.to_string()).or_default();
        if entry.0 != fp {
            *entry = (fp, Default::default());
        }
        if connected {
            entry.1.insert(slug.to_string());
        } else {
            entry.1.remove(slug);
        }
    }
    let mut seen = apps_seen().lock().unwrap_or_else(|e| e.into_inner());
    if let Some(s) = seen.get_mut(space).filter(|s| s.fingerprint == fp) {
        if connected {
            s.apps.insert(slug.to_string());
        } else {
            s.apps.remove(slug);
        }
    }
}

fn forget_apps(state: &AppState, space: &SpaceId) {
    apps_seen().lock().unwrap_or_else(|e| e.into_inner()).remove(&cache_key(state, space));
}

/// With Composio Connect, a check of all the popular apps that took longer than this is too slow
/// to repeat every minute: the tile then lists the apps connected through kwant for a while.
const CONNECT_SLOW: std::time::Duration = std::time::Duration::from_secs(3);
const CONNECT_SLOW_PAUSE: std::time::Duration = std::time::Duration::from_secs(10 * 60);

/// Spaces whose last full Composio Connect check was slow, until when.
fn connect_slow() -> &'static Mutex<HashMap<String, std::time::Instant>> {
    static SLOW: OnceLock<Mutex<HashMap<String, std::time::Instant>>> = OnceLock::new();
    SLOW.get_or_init(Default::default)
}

/// The apps connected with Composio Connect: one `list` call over the popular apps, timed. Under
/// [`CONNECT_SLOW`] it is the answer (with the apps connected through kwant); slower, the next
/// ten minutes list only the apps connected through kwant, with no call.
async fn connect_apps(api: &composio::ComposioApi, ck: &str, fp: u64, key: &str) -> Result<std::collections::BTreeSet<String>, composio::ComposioError> {
    let mine = apps_through_kwant(ck, fp);
    let paused = connect_slow().lock().unwrap_or_else(|e| e.into_inner()).get(ck).is_some_and(|until| std::time::Instant::now() < *until);
    if paused {
        return Ok(mine);
    }
    let slugs: Vec<&str> = composio::popular_apps().iter().map(|a| a.slug.as_str()).collect();
    let started = std::time::Instant::now();
    let result = api.connect_active(key, &slugs).await;
    let took = started.elapsed();
    tracing::info!(apps = slugs.len(), ms = took.as_millis() as u64, ok = result.is_ok(), "connections: Composio Connect status of the popular apps");
    if result.is_ok() && took > CONNECT_SLOW {
        connect_slow().lock().unwrap_or_else(|e| e.into_inner()).insert(ck.to_string(), std::time::Instant::now() + CONNECT_SLOW_PAUSE);
    }
    result.map(|mut apps| {
        apps.extend(mine);
        apps
    })
}

/// The apps connected for the setup's user: cached for a minute, else one Composio call that
/// may take up to 2 seconds here (it goes on in the background and fills the cache after).
async fn connected_apps_cached(state: &AppState, space: &SpaceId, setup: &ComposioSetup) -> Option<AppsSeen> {
    let key = setup.key.clone()?;
    let user = if setup.connect { String::new() } else { setup.user_id()? };
    let fp = fingerprint(&key, &user);
    let ck = cache_key(state, space);
    if let Some(s) = apps_seen().lock().unwrap_or_else(|e| e.into_inner()).get(&ck) {
        if s.fingerprint == fp && s.at.elapsed() < APPS_TTL {
            return Some(s.clone());
        }
    }
    let api = state.composio_api.clone();
    let ck_job = ck.clone();
    let connect = setup.connect;
    let job = tokio::spawn(async move {
        let result = if connect { connect_apps(&api, &ck_job, fp, &key).await } else { api.connected_apps(&key, &user).await };
        remember_apps(&ck_job, fp, &result);
    });
    let _ = tokio::time::timeout(std::time::Duration::from_secs(2), job).await;
    apps_seen().lock().unwrap_or_else(|e| e.into_inner()).get(&ck).filter(|s| s.fingerprint == fp).cloned()
}

/// The optional connectors kwnt offers, with whether each is added.
async fn catalog(state: &AppState, space: &SpaceId) -> Value {
    let setup = composio_setup(state, space).await;
    let seen = match &setup {
        Some(s) => connected_apps_cached(state, space, s).await,
        None => None,
    };
    let added = setup.as_ref().map(|s| &s.server);
    let has_key = setup.as_ref().is_some_and(|s| s.key.is_some());
    let can_connect = setup.as_ref().is_some_and(|s| s.can_connect());
    // Old setups get one way into the new one; a missing key or a refused one gets Replace key.
    let problem = match (&setup, &seen) {
        (None, _) => None,
        (Some(s), _) if s.key.is_none() => Some(json!({"message": "Add your Composio API key to finish setting up Composio.", "action": "replace_key"})),
        (Some(_), Some(AppsSeen { problem: Some((m, a)), .. })) => Some(json!({"message": m, "action": a})),
        (Some(s), _) if s.mode() == "url" && s.user_id().is_none() => Some(json!({
            "message": "This Composio setup doesn't say which Composio user your apps belong to. Switch to the one-key setup and kwant picks one.",
            "action": "switch",
        })),
        _ => None,
    };
    json!([{
        "id": "composio",
        "kind": "mcp",
        "name": "Composio",
        "description": "Gmail, Slack, Notion, Linear and hundreds more apps, with your free Composio key. Your bots find the tools they need themselves.",
        "optional": true,
        "added": added.is_some(),
        "connection_id": added.map(|s| format!("mcp:{}", s.id)),
        "enabled": added.is_some_and(|s| s.enabled),
        "logins_stored": "composio",
        "warning": COMPOSIO_WARNING,
        "add": "/api/connections/composio",
        "key_url": composio::KEY_PAGE,
        // `session`: one `ak_` key, kwant made the session; `connect`: one `ck_` key on Composio
        // Connect; `url`: an MCP URL pasted from composio.dev
        "mode": setup.as_ref().map(|s| s.mode()),
        "user_id": setup.as_ref().and_then(|s| s.user_id()),
        "apps_connected": seen.as_ref().map(|s| s.apps.iter().map(|slug| json!({"slug": slug, "name": app_name(slug)})).collect::<Vec<_>>()).unwrap_or_default(),
        "problem": problem,
        "can_search": has_key,
        "can_connect": can_connect,
        "can_switch": setup.as_ref().is_some_and(|s| s.mode() == "url"),
        "search": "/api/connections/composio/apps",
        "switch": "/api/connections/composio/switch",
        "replace_key": "/api/connections/composio/key",
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
    let marked = state.db.prod_connections(space.as_str()).await.unwrap_or_default();
    let notes = super::tool_notes::counts_by_connection(&state, space.as_str()).await;
    for c in connections.iter_mut() {
        c.production = marked.contains(&c.id);
        if c.kind == "mcp" {
            let (open, broke) = notes.get(&c.name).copied().unwrap_or_default();
            c.notes = Some(NoteCounts { open, broke });
        }
    }
    let mut counts: BTreeMap<&str, usize> = BTreeMap::new();
    for c in &connections {
        *counts.entry(c.status).or_default() += 1;
    }
    Json(json!({
        "connections": connections,
        "counts": counts,
        "vault": vault,
        "catalog": catalog(&state, &space).await,
        // Composio apps marked production by toolkit slug (D10), as `composio:<slug>`
        "production": marked.iter().filter(|m| m.starts_with("composio:")).collect::<Vec<_>>(),
    }))
    .into_response()
}

#[derive(Debug, Deserialize)]
pub struct ProductionBody {
    pub production: bool,
}

/// Whether a connection id can carry the production mark: an MCP server of the space, or a
/// Composio toolkit slug (`composio:github`). Accounts, logins and Loose ends sources can't.
pub fn production_target(id: &str, servers: &[McpServer]) -> Result<(), &'static str> {
    if let Some(sid) = id.strip_prefix("mcp:") {
        return if servers.iter().any(|s| s.id == sid) { Ok(()) } else { Err("No such MCP server in this space") };
    }
    if let Some(slug) = id.strip_prefix("composio:") {
        let ok = !slug.is_empty() && slug.len() <= 64 && slug.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' || c == '-');
        return if ok { Ok(()) } else { Err("A Composio app is named by its toolkit slug, like composio:github") };
    }
    Err("Only MCP servers and Composio apps can be marked production")
}

/// `PUT /api/connections/:id/production?space=` `{production}`: the person marks a connection
/// production (D10), or clears the mark. Replies `{id, production}`.
pub async fn put_production(
    State(state): State<AppState>,
    headers: HeaderMap,
    Scope(space): Scope,
    axum::extract::Path(id): axum::extract::Path<String>,
    Json(body): Json<ProductionBody>,
) -> Response {
    if let Err(r) = refuse_agent_tokens(&state, &headers, REFUSAL).await {
        return r;
    }
    let servers = state.db.list_mcp_servers(&space).await.unwrap_or_default();
    // Clearing a mark always works, so a server deleted since can be unmarked
    if body.production {
        if let Err(why) = production_target(&id, &servers) {
            return error(StatusCode::BAD_REQUEST, why);
        }
    }
    if let Err(e) = state.db.set_prod_connection(space.as_str(), &id, body.production).await {
        tracing::warn!(error = %e, "connections: could not save the production mark");
        return error(StatusCode::INTERNAL_SERVER_ERROR, "The production mark could not be saved");
    }
    Json(json!({"id": id, "production": body.production})).into_response()
}

#[derive(Debug, Deserialize)]
pub struct ComposioBody {
    /// The MCP URL Composio gives for the person's server, `https://….composio.dev/…`. Optional:
    /// without it kwant makes the server itself from the API key (the one-key setup).
    #[serde(default)]
    pub url: Option<String>,
    /// Composio project API key, sent as `x-api-key` (stored like any MCP header, masked in every view).
    #[serde(default)]
    pub api_key: Option<String>,
    /// Off unless the person says so: an added connector reaches no bot until it is switched on.
    #[serde(default)]
    pub enabled: bool,
}

fn error(status: StatusCode, message: &str) -> Response {
    (status, Json(json!({"error": message}))).into_response()
}

/// An error with the one action that fixes it: `add_composio`, `replace_key`, `try_again`,
/// `switch` or `turn_on`. A key problem also carries where to copy a key.
fn error_with(status: StatusCode, message: &str, action: &str) -> Response {
    let mut body = json!({"error": message, "action": action});
    if action == "replace_key" || action == "add_composio" {
        body["key_url"] = json!(composio::KEY_PAGE);
    }
    (status, Json(body)).into_response()
}

fn trimmed_key(key: Option<&str>) -> Option<String> {
    key.map(str::trim).filter(|k| !k.is_empty()).map(str::to_string)
}

/// Composio refused `key`: 409 with what to copy, and a word when it is the wrong kind of key.
fn key_failed(e: composio::ComposioError, key: &str) -> Response {
    if e == composio::ComposioError::KeyRefused && key.starts_with("uak_") {
        return error_with(
            StatusCode::CONFLICT,
            "That's a Composio user key (it starts with uak_). kwant needs your Composio key: copy it from your \
             Composio dashboard. It starts with ck_ or ak_.",
            "replace_key",
        );
    }
    composio_failed(e)
}

/// What a key turned out to be. A project key (`ak_`) is checked by the caller's own first call;
/// a Composio Connect key has been checked already and comes with its tools.
enum Checked {
    Project,
    Connect(Vec<String>),
}

/// Which kind `key` is. `ck_` is checked on Composio Connect (MCP `initialize` and `tools/list`);
/// `ak_` is left to the caller; any other prefix is tried as a project key, then on Composio
/// Connect. A key both refuse gets 409 `replace_key`; Composio out of reach gets 502.
#[allow(clippy::result_large_err)]
async fn check_kind(state: &AppState, key: &str) -> Result<Checked, Response> {
    let api = &state.composio_api;
    match composio::key_kind(key) {
        Some(composio::KeyKind::Project) => Ok(Checked::Project),
        Some(composio::KeyKind::Connect) => api.check_connect_key(key).await.map(Checked::Connect).map_err(|e| key_failed(e, key)),
        None => match api.check_key(key).await {
            Ok(()) => Ok(Checked::Project),
            Err(refused) if refused.is_key() => match api.check_connect_key(key).await {
                Ok(tools) => Ok(Checked::Connect(tools)),
                Err(e) if e.is_key() => Err(key_failed(refused, key)),
                Err(e) => Err(composio_failed(e)),
            },
            Err(e) => Err(composio_failed(e)),
        },
    }
}

/// `next` with `key` as its only Composio key, in the header its kind goes in.
fn with_key(mut next: McpServer, header: &str, key: &str) -> McpServer {
    next.headers.retain(|k, _| !is_key_header(k));
    next.headers.insert(header.to_string(), key.to_string());
    next
}

/// Adds Composio Connect with a checked `ck_` key: server `composio` at Composio Connect, the key
/// in `x-consumer-api-key`, no Composio user (the key is the person's own account).
async fn add_connect(state: &AppState, space: &SpaceId, key: &str, tools: Vec<String>, enabled: bool) -> Response {
    let input = McpServerInput {
        name: COMPOSIO_NAME.into(),
        transport: "http".into(),
        command: None,
        args: None,
        url: Some(composio::CONNECT_MCP_URL.into()),
        env: None,
        headers: Some(BTreeMap::from([(composio::CONNECT_KEY_HEADER.to_string(), key.to_string())])),
        enabled: Some(enabled),
        timeout_secs: None,
    };
    let server = match mcp::build_server(input, None) {
        Ok(s) => s,
        Err(m) => return error(StatusCode::BAD_REQUEST, &m),
    };
    match state.db.create_mcp_server(space, &server).await {
        Ok(created) => {
            forget_apps(state, space);
            tracing::info!(space = %space.as_str(), mode = "connect", "connections: Composio added");
            Json(json!({
                "connection_id": format!("mcp:{}", created.id),
                "server": mcp::McpServerView::from(&created),
                "mode": "connect",
                "user_id": Value::Null,
                "tools": tools,
                "warning": COMPOSIO_WARNING,
            }))
            .into_response()
        }
        Err(e) => {
            tracing::warn!(error = %e, "connections: could not add Composio");
            error(StatusCode::INTERNAL_SERVER_ERROR, "could not add Composio")
        }
    }
}

/// `POST /api/connections/composio?space=`: adds Composio (off for bots by default). A `ck_`
/// key alone is checked on Composio Connect and saved as `mode: "connect"`. With only
/// `api_key`, kwant checks the key by making a Tool Router session for the space's own Composio
/// user and saves the session's MCP server: `mode: "session"`. With `url` it saves that MCP URL as
/// before: `mode: "url"`. A refused key saves nothing.
pub async fn add_composio(
    State(state): State<AppState>,
    headers: HeaderMap,
    Scope(space): Scope,
    Json(body): Json<ComposioBody>,
) -> Response {
    if let Err(r) = refuse_agent_tokens(&state, &headers, REFUSAL).await {
        return r;
    }
    let url = body.url.as_deref().map(str::trim).filter(|u| !u.is_empty()).map(str::to_string);
    let key = trimmed_key(body.api_key.as_deref());
    if let Some(u) = &url {
        if !is_composio_url(u) {
            return error(StatusCode::BAD_REQUEST, "url must be an https address on composio.dev");
        }
    } else if key.is_none() {
        return error_with(StatusCode::BAD_REQUEST, "Paste your Composio API key.", "add_composio");
    }
    let servers = state.db.list_mcp_servers(&space).await.unwrap_or_default();
    if servers.iter().any(|s| is_composio(s) || s.name.eq_ignore_ascii_case(COMPOSIO_NAME)) {
        return error_with(StatusCode::CONFLICT, "Composio is already added. Replace its key on the Composio tile instead.", "replace_key");
    }
    if let (None, Some(k)) = (&url, &key) {
        match check_kind(&state, k).await {
            Ok(Checked::Connect(tools)) => return add_connect(&state, &space, k, tools, body.enabled).await,
            Ok(Checked::Project) => {}
            Err(r) => return r,
        }
    }
    let session = match (&url, &key) {
        (None, Some(k)) => match state.composio_api.create_session(k, &composio::space_user_id(space.as_str())).await {
            Ok(s) => Some(s),
            Err(e) => return key_failed(e, k),
        },
        _ => None,
    };
    let headers: BTreeMap<String, String> =
        key.clone().map(|k| BTreeMap::from([("x-api-key".to_string(), k)])).unwrap_or_default();
    let input = McpServerInput {
        name: COMPOSIO_NAME.into(),
        transport: "http".into(),
        command: None,
        args: None,
        url: Some(session.as_ref().map(|s| s.mcp_url.clone()).or(url).unwrap_or_default()),
        env: None,
        headers: Some(headers),
        enabled: Some(body.enabled),
        timeout_secs: None,
    };
    let server = match mcp::build_server(input, None) {
        Ok(s) => s,
        Err(m) => return error(StatusCode::BAD_REQUEST, &m),
    };
    if let Some(s) = &session {
        if let Err(e) = save_session(&state, &space, s).await {
            tracing::warn!(error = %e, "connections: could not save the Composio session");
            return error(StatusCode::INTERNAL_SERVER_ERROR, "could not add Composio");
        }
    }
    match state.db.create_mcp_server(&space, &server).await {
        Ok(created) => {
            forget_apps(&state, &space);
            tracing::info!(space = %space.as_str(), mode = if session.is_some() { "session" } else { "url" }, "connections: Composio added");
            Json(json!({
                "connection_id": format!("mcp:{}", created.id),
                "server": mcp::McpServerView::from(&created),
                "mode": if session.is_some() { "session" } else { "url" },
                "user_id": session.as_ref().map(|s| s.user_id.clone()),
                "tools": session.as_ref().map(|s| s.tools.clone()).unwrap_or_default(),
                "warning": COMPOSIO_WARNING,
            }))
            .into_response()
        }
        Err(e) => {
            tracing::warn!(error = %e, "connections: could not add Composio");
            error(StatusCode::INTERNAL_SERVER_ERROR, "could not add Composio")
        }
    }
}

#[derive(Debug, Default, Deserialize)]
pub struct KeyBody {
    #[serde(default)]
    pub api_key: Option<String>,
}

/// Points the space's Composio server at `session` with `key`, keeping its id, its switch and
/// which bots have it, so nothing the person picked changes.
#[allow(clippy::result_large_err)]
async fn move_to_session(
    state: &AppState,
    space: &SpaceId,
    server: &McpServer,
    key: &str,
    session: &composio::Session,
) -> Result<McpServer, Response> {
    let mut next = with_key(server.clone(), composio::PROJECT_KEY_HEADER, key);
    next.url = Some(session.mcp_url.clone());
    let fail = |what: &str| {
        tracing::warn!(what, "connections: could not save the Composio session");
        error(StatusCode::INTERNAL_SERVER_ERROR, "could not save the new Composio setup")
    };
    save_session(state, space, session).await.map_err(|_| fail("setting"))?;
    let saved = state.db.update_mcp_server(&next).await.map_err(|_| fail("server"))?.ok_or_else(|| fail("server gone"))?;
    forget_apps(state, space);
    Ok(saved)
}

/// `POST /api/connections/composio/switch?space=` `{api_key?}`: turns a setup made from an MCP
/// URL into the one-key setup. It uses the saved key (or `api_key`) and the URL's `user_id`, so
/// apps already connected stay connected; without one, the space's own user. Idempotent: a
/// session setup answers `switched: false` and calls nobody.
pub async fn switch_composio(
    State(state): State<AppState>,
    headers: HeaderMap,
    Scope(space): Scope,
    body: Option<Json<KeyBody>>,
) -> Response {
    if let Err(r) = refuse_agent_tokens(&state, &headers, REFUSAL).await {
        return r;
    }
    let Some(setup) = composio_setup(&state, &space).await else {
        return error_with(StatusCode::CONFLICT, "Composio isn't added yet. Add it with your free Composio API key.", "add_composio");
    };
    if setup.session.is_some() || setup.connect {
        return Json(json!({
            "connection_id": format!("mcp:{}", setup.server.id),
            "server": mcp::McpServerView::from(&setup.server),
            "mode": setup.mode(),
            "user_id": setup.user_id(),
            "switched": false,
        }))
        .into_response();
    }
    let given = body.and_then(|Json(b)| trimmed_key(b.api_key.as_deref()));
    let Some(key) = given.or(setup.key.clone()) else {
        return error_with(StatusCode::CONFLICT, "Paste your Composio API key to switch.", "replace_key");
    };
    let user_id = setup.user_id().unwrap_or_else(|| composio::space_user_id(space.as_str()));
    let session = match state.composio_api.create_session(&key, &user_id).await {
        Ok(s) => s,
        Err(e) => return key_failed(e, &key),
    };
    match move_to_session(&state, &space, &setup.server, &key, &session).await {
        Ok(saved) => {
            tracing::info!(space = %space.as_str(), "connections: Composio switched to the one-key setup");
            Json(json!({
                "connection_id": format!("mcp:{}", saved.id),
                "server": mcp::McpServerView::from(&saved),
                "mode": "session",
                "user_id": session.user_id,
                "tools": session.tools,
                "switched": true,
            }))
            .into_response()
        }
        Err(r) => r,
    }
}

/// `PUT /api/connections/composio/key?space=` `{api_key}`: replaces the Composio key with either
/// kind. The new key is checked first, and a refused one changes nothing. A `ck_` key makes the
/// setup Composio Connect; an `ak_` key in place of a `ck_` one gets a session for the space's
/// own Composio user. In the one-key setup a key of the same Composio project keeps the session;
/// a key of another project gets a new session for the same user. Replies `{connection_id, mode,
/// user_id, session_renewed}`.
pub async fn replace_composio_key(
    State(state): State<AppState>,
    headers: HeaderMap,
    Scope(space): Scope,
    Json(body): Json<KeyBody>,
) -> Response {
    if let Err(r) = refuse_agent_tokens(&state, &headers, REFUSAL).await {
        return r;
    }
    let Some(key) = trimmed_key(body.api_key.as_deref()) else {
        return error_with(StatusCode::BAD_REQUEST, "Paste your Composio API key.", "replace_key");
    };
    let Some(setup) = composio_setup(&state, &space).await else {
        return error_with(StatusCode::CONFLICT, "Composio isn't added yet. Add it with your free Composio API key.", "add_composio");
    };
    let reply = |server: &McpServer, mode: &str, user_id: Option<String>, renewed: bool| {
        Json(json!({
            "connection_id": format!("mcp:{}", server.id),
            "server": mcp::McpServerView::from(server),
            "mode": mode,
            "user_id": user_id,
            "session_renewed": renewed,
        }))
        .into_response()
    };
    match check_kind(&state, &key).await {
        Err(r) => return r,
        Ok(Checked::Connect(_)) => {
            let mut next = with_key(setup.server.clone(), composio::CONNECT_KEY_HEADER, &key);
            next.url = Some(composio::CONNECT_MCP_URL.into());
            return match state.db.update_mcp_server(&next).await {
                Ok(Some(saved)) => {
                    forget_apps(&state, &space);
                    tracing::info!(space = %space.as_str(), from = setup.mode(), "connections: Composio key replaced with a Composio Connect key");
                    reply(&saved, "connect", None, false)
                }
                _ => error(StatusCode::INTERNAL_SERVER_ERROR, "could not save the key"),
            };
        }
        Ok(Checked::Project) if setup.connect => {
            let session = match state.composio_api.create_session(&key, &composio::space_user_id(space.as_str())).await {
                Ok(s) => s,
                Err(e) => return key_failed(e, &key),
            };
            return match move_to_session(&state, &space, &setup.server, &key, &session).await {
                Ok(saved) => reply(&saved, "session", Some(session.user_id), true),
                Err(r) => r,
            };
        }
        Ok(Checked::Project) => {}
    }
    match &setup.session {
        Some(stored) => {
            let same_project = match state.composio_api.session_user(&key, &stored.session_id).await {
                Ok(user) => user.is_some(),
                Err(e) => return key_failed(e, &key),
            };
            if same_project {
                let next = with_key(setup.server.clone(), composio::PROJECT_KEY_HEADER, &key);
                return match state.db.update_mcp_server(&next).await {
                    Ok(Some(saved)) => {
                        forget_apps(&state, &space);
                        reply(&saved, "session", Some(stored.user_id.clone()), false)
                    }
                    _ => error(StatusCode::INTERNAL_SERVER_ERROR, "could not save the key"),
                };
            }
            let session = match state.composio_api.create_session(&key, &stored.user_id).await {
                Ok(s) => s,
                Err(e) => return key_failed(e, &key),
            };
            match move_to_session(&state, &space, &setup.server, &key, &session).await {
                Ok(saved) => reply(&saved, "session", Some(session.user_id), true),
                Err(r) => r,
            }
        }
        None => {
            if let Err(e) = state.composio_api.check_key(&key).await {
                return key_failed(e, &key);
            }
            let next = with_key(setup.server.clone(), composio::PROJECT_KEY_HEADER, &key);
            match state.db.update_mcp_server(&next).await {
                Ok(Some(saved)) => {
                    forget_apps(&state, &space);
                    reply(&saved, "url", setup.user_id(), false)
                }
                _ => error(StatusCode::INTERNAL_SERVER_ERROR, "could not save the key"),
            }
        }
    }
}

/// The space's Composio with its key, or the reply that explains what's missing and how to fix
/// it. Nothing here calls Composio.
#[allow(clippy::result_large_err)]
async fn composio_with_key(state: &AppState, space: &SpaceId) -> Result<(ComposioSetup, String), Response> {
    let Some(setup) = composio_setup(state, space).await else {
        return Err(error_with(
            StatusCode::CONFLICT,
            "Add Composio first: paste your free Composio API key in Apps & accounts. kwant never contacts Composio before that.",
            "add_composio",
        ));
    };
    let Some(key) = setup.key.clone() else {
        return Err(error_with(StatusCode::CONFLICT, "Add your Composio API key to finish setting up Composio.", "replace_key"));
    };
    Ok((setup, key))
}

/// The Composio user to connect apps for, or the reply that offers the switch.
#[allow(clippy::result_large_err)]
fn user_or_switch(setup: &ComposioSetup) -> Result<String, Response> {
    setup.user_id().ok_or_else(|| {
        error_with(
            StatusCode::CONFLICT,
            "This Composio setup doesn't say which Composio user your apps belong to. Switch to the one-key setup and kwant picks one.",
            "switch",
        )
    })
}

fn composio_failed(e: composio::ComposioError) -> Response {
    let status = if e.is_key() { StatusCode::CONFLICT } else { StatusCode::BAD_GATEWAY };
    error_with(status, &e.message(), e.action())
}

/// Composio's popular apps as rows, each with `connected` from `connected` (false when unknown).
fn popular_rows<'a>(apps: impl Iterator<Item = &'a composio::PopularApp>, connected: Option<&std::collections::BTreeSet<String>>) -> Vec<Value> {
    apps.map(|a| {
        let mut row = serde_json::to_value(a).unwrap_or(Value::Null);
        row["connected"] = json!(connected.is_some_and(|c| c.contains(&a.slug)));
        row
    })
    .collect()
}

/// Before Composio is added: Composio's popular apps from kwant itself, no Composio call.
fn popular_before_setup(q: Option<&str>) -> Response {
    let apps: Vec<&composio::PopularApp> = match q {
        Some(q) => composio::search_popular(q, SEARCH_POPULAR_LIMIT),
        None => composio::popular_apps().iter().collect(),
    };
    let count = apps.len();
    Json(json!({
        "apps": popular_rows(apps.into_iter(), None),
        "count": count,
        "connected_checked": false,
        "mode": Value::Null,
        "can_connect": false,
        "source": "popular",
        "warning": COMPOSIO_WARNING,
    }))
    .into_response()
}

/// At most this many popular apps answer a search; their status is one Composio call.
const SEARCH_POPULAR_LIMIT: usize = 20;

type SearchSeen = HashMap<(String, u64, String), (std::time::Instant, std::collections::BTreeSet<String>)>;

fn connect_searches() -> &'static Mutex<SearchSeen> {
    static SEEN: OnceLock<Mutex<SearchSeen>> = OnceLock::new();
    SEEN.get_or_init(Default::default)
}

/// Which of `slugs` are connected with Composio Connect: the one-minute tile cache when fresh,
/// else one `list` call for just these apps, itself kept a minute. `None` when it can't be told.
async fn connect_search_status(state: &AppState, space: &SpaceId, key: &str, slugs: &[&str]) -> Option<std::collections::BTreeSet<String>> {
    let ck = cache_key(state, space);
    let fp = fingerprint(key, "");
    if let Some(s) = apps_seen().lock().unwrap_or_else(|e| e.into_inner()).get(&ck) {
        if s.fingerprint == fp && s.at.elapsed() < APPS_TTL && s.problem.is_none() {
            return Some(s.apps.clone());
        }
    }
    if slugs.is_empty() {
        return Some(Default::default());
    }
    let lk = (ck.clone(), fp, slugs.join(","));
    {
        let mut seen = connect_searches().lock().unwrap_or_else(|e| e.into_inner());
        seen.retain(|_, (at, _)| at.elapsed() < APPS_TTL);
        if seen.len() > 1000 {
            seen.clear();
        }
        if let Some((_, apps)) = seen.get(&lk) {
            return Some(apps.clone());
        }
    }
    match state.composio_api.connect_active(key, slugs).await {
        Ok(mut apps) => {
            apps.extend(apps_through_kwant(&ck, fp).into_iter().filter(|a| slugs.contains(&a.as_str())));
            connect_searches().lock().unwrap_or_else(|e| e.into_inner()).insert(lk, (std::time::Instant::now(), apps.clone()));
            Some(apps)
        }
        Err(e) => {
            tracing::warn!(error = %e.message(), "connections: could not check Composio Connect apps");
            None
        }
    }
}

/// With a Composio Connect key: Composio's popular apps (Composio Connect has no catalogue
/// call), each with `connected`.
async fn connect_popular(state: &AppState, space: &SpaceId, setup: &ComposioSetup, key: &str, q: Option<&str>) -> Response {
    let (apps, connected) = match q {
        None => {
            let seen = connected_apps_cached(state, space, setup).await;
            (composio::popular_apps().iter().collect::<Vec<_>>(), seen.filter(|s| s.problem.is_none()).map(|s| s.apps))
        }
        Some(q) => {
            let apps = composio::search_popular(q, SEARCH_POPULAR_LIMIT);
            let slugs: Vec<&str> = apps.iter().map(|a| a.slug.as_str()).collect();
            let connected = connect_search_status(state, space, key, &slugs).await;
            (apps, connected)
        }
    };
    let count = apps.len();
    Json(json!({
        "apps": popular_rows(apps.into_iter(), connected.as_ref()),
        "count": count,
        "connected_checked": connected.is_some(),
        "mode": "connect",
        "can_connect": true,
        "source": "popular",
        "warning": COMPOSIO_WARNING,
    }))
    .into_response()
}

#[derive(Debug, Deserialize)]
pub struct AppSearch {
    /// Without `q` the whole catalogue comes back, for matching in the browser.
    #[serde(default)]
    pub q: Option<String>,
}

// The whole Composio catalogue, per Composio client and key, kept a day: the app list barely
// changes, and Find a tool matches every keystroke against it in the browser. A day-old list is
// still served at once while a fresh one loads in the background.

const LIST_TTL: std::time::Duration = std::time::Duration::from_secs(24 * 60 * 60);

#[derive(Debug, Clone, Serialize)]
struct ListedApp {
    slug: String,
    name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    logo: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tools_count: Option<u64>,
}

#[derive(Debug, Clone)]
struct AppList {
    at: std::time::Instant,
    fetched_at: DateTime<Utc>,
    apps: Arc<Vec<ListedApp>>,
    refreshing: bool,
}

fn app_lists() -> &'static Mutex<HashMap<String, AppList>> {
    static LISTS: OnceLock<Mutex<HashMap<String, AppList>>> = OnceLock::new();
    LISTS.get_or_init(Default::default)
}

fn list_key(state: &AppState, key: &str) -> String {
    format!("{:p}:{:x}", Arc::as_ptr(&state.composio_api), fingerprint(key, ""))
}

fn store_list(lk: &str, apps: Vec<composio::ComposioApp>) -> AppList {
    learn_names(&apps);
    let listed = apps
        .into_iter()
        .map(|a| ListedApp { slug: a.slug, name: a.name, logo: a.logo, tools_count: a.tools_count })
        .collect();
    let list = AppList { at: std::time::Instant::now(), fetched_at: Utc::now(), apps: Arc::new(listed), refreshing: false };
    let mut lists = app_lists().lock().unwrap_or_else(|e| e.into_inner());
    if lists.len() > 64 {
        lists.retain(|_, l| l.at.elapsed() < LIST_TTL);
    }
    lists.insert(lk.to_string(), list.clone());
    list
}

/// The catalogue for `key`: cached, else one paged fetch. A list older than a day is served as it
/// is while one background fetch replaces it.
async fn whole_list(state: &AppState, key: &str) -> Result<AppList, composio::ComposioError> {
    let lk = list_key(state, key);
    let cached = {
        let mut lists = app_lists().lock().unwrap_or_else(|e| e.into_inner());
        match lists.get_mut(&lk) {
            Some(l) => {
                let stale = l.at.elapsed() >= LIST_TTL && !l.refreshing;
                if stale {
                    l.refreshing = true;
                }
                Some((l.clone(), stale))
            }
            None => None,
        }
    };
    match cached {
        Some((list, stale)) => {
            if stale {
                let (api, key, lk) = (state.composio_api.clone(), key.to_string(), lk.clone());
                tokio::spawn(async move {
                    match api.all_apps(&key).await {
                        Ok(apps) => {
                            store_list(&lk, apps);
                        }
                        Err(e) => {
                            tracing::warn!(error = %e.message(), "connections: could not refresh the Composio app list");
                            if let Some(l) = app_lists().lock().unwrap_or_else(|e| e.into_inner()).get_mut(&lk) {
                                l.refreshing = false;
                            }
                        }
                    }
                });
            }
            Ok(list)
        }
        None => Ok(store_list(&lk, state.composio_api.all_apps(key).await?)),
    }
}

fn if_none_match(headers: &HeaderMap, etag: &str) -> bool {
    headers
        .get(axum::http::header::IF_NONE_MATCH)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| v.split(',').any(|t| t.trim().trim_start_matches("W/") == etag))
}

/// `GET /api/connections/composio/apps?space=` without `q`: every app in Composio's catalogue,
/// compact, with `connected` for the space's user (that from the one-minute apps cache, so at
/// most one account call). Sends an `ETag`; a matching `If-None-Match` gets `304`.
async fn list_composio_apps(state: &AppState, headers: &HeaderMap, space: &SpaceId, setup: &ComposioSetup, key: &str) -> Response {
    let started = std::time::Instant::now();
    let list = match whole_list(state, key).await {
        Ok(l) => l,
        Err(e) => return composio_failed(e),
    };
    let user = setup.user_id();
    let seen = connected_apps_cached(state, space, setup).await;
    let connected = seen.as_ref().filter(|s| s.problem.is_none()).map(|s| s.apps.clone());
    let etag = {
        use std::hash::{Hash, Hasher};
        let mut h = std::collections::hash_map::DefaultHasher::new();
        (fingerprint(key, ""), list.fetched_at.timestamp_millis(), list.apps.len(), &connected, setup.mode(), &user).hash(&mut h);
        format!("\"{:016x}\"", h.finish())
    };
    let cache_headers = [
        (axum::http::header::ETAG, etag.clone()),
        (axum::http::header::CACHE_CONTROL, "private, no-cache".to_string()),
    ];
    if if_none_match(headers, &etag) {
        return (StatusCode::NOT_MODIFIED, cache_headers).into_response();
    }
    let apps: Vec<Value> = list
        .apps
        .iter()
        .map(|a| {
            let mut row = serde_json::to_value(a).unwrap_or(Value::Null);
            row["connected"] = json!(connected.as_ref().is_some_and(|c| c.contains(&a.slug)));
            row
        })
        .collect();
    tracing::debug!(apps = apps.len(), ms = started.elapsed().as_millis() as u64, "connections: Composio app list");
    (
        cache_headers,
        Json(json!({
            "apps": apps,
            "count": list.apps.len(),
            "fetched_at": list.fetched_at,
            "connected_checked": connected.is_some(),
            "mode": setup.mode(),
            "can_connect": user.is_some(),
            "source": "all",
            "warning": COMPOSIO_WARNING,
        })),
    )
        .into_response()
}

/// `GET /api/connections/composio/apps?space=&q=`: apps in Composio's catalogue matching `q`
/// (without `q`, the whole catalogue: see `list_composio_apps`),
/// searched with the person's own Composio key, each with `connected` for the space's Composio
/// user (one account call per search). `source` says where the apps come from: `"all"` is
/// Composio's catalogue (a project key); `"popular"` is Composio's top apps bundled with kwant,
/// served before Composio is added (no Composio call, `mode: null`) and with a Composio Connect
/// key. 409 when Composio is added without a key.
pub async fn search_composio_apps(
    State(state): State<AppState>,
    headers: HeaderMap,
    Scope(space): Scope,
    axum::extract::Query(query): axum::extract::Query<AppSearch>,
) -> Response {
    if let Err(r) = refuse_agent_tokens(&state, &headers, REFUSAL).await {
        return r;
    }
    let q = query.q.as_deref().map(str::trim);
    if q.is_some_and(|q| q.is_empty() || q.chars().count() > composio::MAX_QUERY_CHARS) {
        return error(StatusCode::BAD_REQUEST, "type an app name, up to 60 characters");
    }
    if composio_setup(&state, &space).await.is_none() {
        return popular_before_setup(q);
    }
    let (setup, key) = match composio_with_key(&state, &space).await {
        Ok(found) => found,
        Err(r) => return r,
    };
    if setup.connect {
        return connect_popular(&state, &space, &setup, &key, q).await;
    }
    let Some(q) = q else {
        return list_composio_apps(&state, &headers, &space, &setup, &key).await;
    };
    let apps = match state.composio_api.search(&key, q).await {
        Ok(apps) => apps,
        Err(e) => return composio_failed(e),
    };
    learn_names(&apps);
    let user = setup.user_id();
    // One call for every app on the page; if it fails the search still answers, unchecked.
    let connected = match &user {
        Some(u) if !apps.is_empty() => {
            let result = state.composio_api.connected_apps(&key, u).await;
            remember_apps(&cache_key(&state, &space), fingerprint(&key, u), &result);
            result.ok()
        }
        _ => None,
    };
    let rows: Vec<Value> = apps
        .iter()
        .map(|a| {
            let mut row = serde_json::to_value(a).unwrap_or(Value::Null);
            row["connected"] = json!(connected.as_ref().is_some_and(|c| c.contains(&a.slug)));
            row
        })
        .collect();
    Json(json!({
        "apps": rows,
        "connected_checked": connected.is_some(),
        "mode": setup.mode(),
        "can_connect": user.is_some(),
        "source": "all",
        "warning": COMPOSIO_WARNING,
    }))
    .into_response()
}

/// `POST /api/connections/composio/apps/:slug/connect?space=`: starts signing in to one app on
/// Composio for the space's Composio user (the session's in the one-key setup, else the URL's
/// `user_id`), and returns the address the person opens. The sign-in happens on Composio's site.
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
    let (setup, key) = match composio_with_key(&state, &space).await {
        Ok(found) => found,
        Err(r) => return r,
    };
    let link = match &setup.session {
        _ if setup.connect => state.composio_api.connect_add_link(&key, &slug).await,
        Some(s) => state.composio_api.session_link(&key, &s.session_id, &slug).await,
        None => {
            let user_id = match user_or_switch(&setup) {
                Ok(u) => u,
                Err(r) => return r,
            };
            state.composio_api.connect_link(&key, &user_id, &slug).await
        }
    };
    match link {
        Ok(redirect_url) => Json(json!({"redirect_url": redirect_url, "warning": COMPOSIO_WARNING})).into_response(),
        Err(e) => composio_failed(e),
    }
}

/// `GET /api/connections/composio/apps/:slug/status?space=`: whether the space's Composio user has
/// finished signing in to the app on Composio: `{"connected": bool}`. The connection card and Find
/// a tool poll it after Connect. 409 when Composio can't be used for this.
pub async fn composio_app_status(
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
    let (setup, key) = match composio_with_key(&state, &space).await {
        Ok(found) => found,
        Err(r) => return r,
    };
    if setup.connect {
        return match state.composio_api.connect_active(&key, &[slug.as_str()]).await {
            Ok(active) => {
                let connected = active.contains(&slug);
                note_app(&cache_key(&state, &space), fingerprint(&key, ""), &slug, connected);
                Json(json!({"connected": connected})).into_response()
            }
            Err(e) => composio_failed(e),
        };
    }
    let user_id = match user_or_switch(&setup) {
        Ok(u) => u,
        Err(r) => return r,
    };
    match state.composio_api.is_connected(&key, &user_id, &slug).await {
        Ok(connected) => {
            note_app(&cache_key(&state, &space), fingerprint(&key, &user_id), &slug, connected);
            Json(json!({"connected": connected})).into_response()
        }
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
    fn only_mcp_servers_and_composio_slugs_can_be_marked_production() {
        assert!(production_target("composio:github", &[]).is_ok());
        assert!(production_target("composio:google_sheets", &[]).is_ok());
        assert!(production_target("composio:", &[]).is_err());
        assert!(production_target("composio:GitHub", &[]).is_err());
        assert!(production_target("composio:../x", &[]).is_err());
        assert!(production_target(&format!("composio:{}", "a".repeat(65)), &[]).is_err());
        assert!(production_target("mcp:srv_gone", &[]).is_err(), "a server not in this space");
        assert!(production_target("account:p1", &[]).is_err());
        assert!(production_target("telegram", &[]).is_err());
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
