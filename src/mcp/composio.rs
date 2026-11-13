//! Composio's app catalogue (API v3.1): search its toolkits and start a sign-in for one, with the
//! person's own Composio API key. kwnt only calls this once the person has added Composio as a
//! connector, and only from person-only routes (`server::connections`).
//!
//! Sources: <https://docs.composio.dev/reference/api-reference/toolkits/getToolkits>,
//! <https://docs.composio.dev/reference/api-reference/tool-router/postToolRouterSession> and
//! <https://docs.composio.dev/reference/api-reference/tool-router/postToolRouterSessionBySessionIdLink> and
//! <https://docs.composio.dev/reference/api-reference/connected-accounts/getConnectedAccounts>.
//!
//! A Composio Connect key (`ck_…`, a consumer key) works on none of those: it works only on
//! Composio Connect, an MCP server (<https://docs.composio.dev/docs/composio-connect>). kwnt
//! checks such a key, starts sign-ins and reads which apps are connected through that server's
//! own `COMPOSIO_MANAGE_CONNECTIONS` tool, with kwnt's MCP client.

use super::client::{McpClient, McpError};
use crate::db::models::McpServer;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::BTreeSet;
use std::sync::OnceLock;
use std::time::Duration;

pub const DEFAULT_API_URL: &str = "https://backend.composio.dev";
/// Points kwnt at another Composio API address (tests).
pub const API_URL_ENV: &str = "KWNT_COMPOSIO_API_URL";
const TIMEOUT: Duration = Duration::from_secs(10);
/// Largest Composio answer kwnt reads.
const MAX_BODY: usize = 2 * 1024 * 1024;
pub const MAX_QUERY_CHARS: usize = 60;
/// How many apps one search returns.
const PAGE: usize = 20;
/// The whole-catalogue listing: pages of this many apps, at most this many pages.
const LIST_PAGE: usize = 500;
const MAX_LIST_PAGES: usize = 20;

/// Composio Connect: the MCP server a consumer key (`ck_…`) works on, sent as
/// [`CONNECT_KEY_HEADER`]. Stateless streamable HTTP that answers as an event stream.
pub const CONNECT_MCP_URL: &str = "https://connect.composio.dev/mcp";
/// Points kwnt's own Composio Connect calls at another address (tests).
pub const CONNECT_URL_ENV: &str = "KWNT_COMPOSIO_CONNECT_URL";
/// The header a project key (`ak_…`) goes in.
pub const PROJECT_KEY_HEADER: &str = "x-api-key";
/// The header a Composio Connect key (`ck_…`) goes in.
pub const CONNECT_KEY_HEADER: &str = "x-consumer-api-key";
/// The Composio Connect tool kwnt uses for sign-ins and status: `add` and `list`, never anything
/// that runs an app's tools.
pub const MANAGE_TOOL: &str = "COMPOSIO_MANAGE_CONNECTIONS";
/// Composio Connect calls get this long (the MCP handshake and one call).
const CONNECT_TIMEOUT_SECS: i64 = 15;
/// Most apps one `list` call asks about.
pub const MAX_STATUS_SLUGS: usize = 120;
/// The Composio Connect tool kwnt asks whether an app exists, by the name the person typed.
/// (`COMPOSIO_MANAGE_CONNECTIONS` `list` can't tell: it answers "initiated" for a made-up app too;
/// checked 2026-11-13.)
pub const SEARCH_TOOL: &str = "COMPOSIO_SEARCH_TOOLS";
/// Composio's public app pages: `<this>/<slug>` answers 200 for an app Composio has and 404 for
/// one it hasn't (checked 2026-11-13: elevenlabs 200, google_search_console 200, notarealapp123
/// 404). kwnt asks with `HEAD`, no key and nothing about the person: only the app's slug.
pub const TOOLKITS_PAGE_URL: &str = "https://composio.dev/toolkits";
/// How long one app page check may take.
const PAGE_CHECK_TIMEOUT: Duration = Duration::from_secs(3);
/// Most app names one search asks Composio about.
pub const MAX_CANDIDATES: usize = 3;

/// Which kind of Composio key, from its prefix.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyKind {
    /// `ak_…`: a project API key, for Composio's REST API (`x-api-key`).
    Project,
    /// `ck_…`: a Composio Connect key, for its MCP server only (`x-consumer-api-key`).
    Connect,
}

/// The kind a key's prefix says it is; `None` for any other prefix (kwnt tries both).
pub fn key_kind(key: &str) -> Option<KeyKind> {
    if key.starts_with("ak_") {
        Some(KeyKind::Project)
    } else if key.starts_with("ck_") {
        Some(KeyKind::Connect)
    } else {
        None
    }
}

/// One of Composio's most popular apps, bundled with kwnt (`composio_popular.json`, made by
/// `scripts/composio_popular_apps.py` from the public composio.dev/toolkits page at release).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PopularApp {
    pub slug: String,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub logo: Option<String>,
    pub rank: u32,
}

/// Composio's popular apps, most popular first: shown before Composio is added and with a
/// Composio Connect key, which can't list the catalogue.
pub fn popular_apps() -> &'static [PopularApp] {
    static APPS: OnceLock<Vec<PopularApp>> = OnceLock::new();
    APPS.get_or_init(|| {
        let mut apps: Vec<PopularApp> = serde_json::from_str(include_str!("composio_popular.json"))
            .expect("src/mcp/composio_popular.json is valid");
        apps.retain(|a| valid_slug(&a.slug) && a.logo.as_deref().is_none_or(|l| l.starts_with("https://")));
        apps.sort_by_key(|a| a.rank);
        apps
    })
}

/// Whether `app`'s name, one of its name's words, or its slug starts with `query` (as typed,
/// trimmed, any case): the person is likely still typing that app's name.
pub fn popular_starts_with(app: &PopularApp, query: &str) -> bool {
    let q = query.trim().to_lowercase();
    let name = app.name.to_lowercase();
    name.starts_with(&q) || app.slug.starts_with(&q) || name.split(' ').any(|w| w.starts_with(&q))
}

/// Popular apps matching `query` (name or slug, any case): those starting with it first, then
/// those containing it, each in popularity order; at most `limit`.
pub fn search_popular(query: &str, limit: usize) -> Vec<&'static PopularApp> {
    let q = query.trim().to_lowercase();
    let (mut first, mut then) = (Vec::new(), Vec::new());
    for a in popular_apps() {
        if popular_starts_with(a, &q) {
            first.push(a);
        } else if a.name.to_lowercase().contains(&q) || a.slug.contains(&q) {
            then.push(a);
        }
    }
    first.extend(then);
    first.truncate(limit);
    first
}

/// The slugs an app the person typed by name could have, most likely first, at most
/// [`MAX_CANDIDATES`]: "ElevenLabs" and "elevenlabs" → `elevenlabs`; "Eleven Labs" →
/// `elevenlabs`, `eleven_labs`, `eleven-labs`. None for a name with letters outside ASCII (no
/// Composio slug has them) or under two characters.
pub fn slug_candidates(query: &str) -> Vec<String> {
    if query.chars().any(|c| c.is_alphanumeric() && !c.is_ascii()) {
        return vec![];
    }
    let words: Vec<String> = query
        .split(|c: char| !c.is_ascii_alphanumeric())
        .filter(|w| !w.is_empty())
        .map(str::to_ascii_lowercase)
        .collect();
    let compact = words.concat();
    if compact.len() < 2 {
        return vec![];
    }
    let mut out = vec![compact];
    if words.len() > 1 {
        out.push(words.join("_"));
        out.push(words.join("-"));
    }
    out.retain(|s| valid_slug(s));
    out.dedup();
    out.truncate(MAX_CANDIDATES);
    out
}

/// An app Composio Connect's search named that is one of the slugs kwnt asked about.
#[derive(Debug, Clone, PartialEq)]
pub struct FoundApp {
    pub slug: String,
    pub description: Option<String>,
    /// Whether the person has a working account for it, when the answer says.
    pub connected: Option<bool>,
}

/// The apps among `candidates` that a `COMPOSIO_SEARCH_TOOLS` answer names, in `candidates`
/// order. Only exact slugs count: the search always answers with something ("notarealapp123"
/// gets Google and a design tool), so a looser match would offer apps the person didn't type.
/// Nothing else in the answer (accounts, the person's details, tool schemas) is kept.
pub fn found_toolkits(text: &str, candidates: &[String]) -> Result<Vec<FoundApp>, ComposioError> {
    let body: Value = serde_json::from_str(text.trim())
        .map_err(|_| ComposioError::Upstream(200, "Composio Connect answered something kwant can't read".into()))?;
    if body["successful"] == json!(false) || body["success"] == json!(false) {
        let why = body["error"].as_str().or_else(|| body["data"]["message"].as_str()).unwrap_or_default();
        return Err(ComposioError::Upstream(200, clip(why, 200)));
    }
    let data = if body["data"].is_object() { &body["data"] } else { &body };
    let mut named: BTreeSet<String> = BTreeSet::new();
    for r in data["results"].as_array().into_iter().flatten() {
        for t in r["toolkits"].as_array().into_iter().flatten() {
            if let Some(t) = t.as_str() {
                named.insert(t.to_ascii_lowercase());
            }
        }
    }
    let statuses: Vec<&Value> = data["toolkit_connection_statuses"].as_array().into_iter().flatten().collect();
    for s in &statuses {
        if let Some(t) = s["toolkit"].as_str() {
            named.insert(t.to_ascii_lowercase());
        }
    }
    Ok(candidates
        .iter()
        .filter(|c| named.contains(c.as_str()))
        .map(|c| {
            let status = statuses.iter().find(|s| s["toolkit"].as_str().is_some_and(|t| t.eq_ignore_ascii_case(c)));
            FoundApp {
                slug: c.clone(),
                description: status
                    .and_then(|s| s["description"].as_str())
                    .map(|d| clip(d.trim(), 240))
                    .filter(|d| !d.is_empty()),
                connected: status.and_then(|s| s["has_active_connection"].as_bool()),
            }
        })
        .collect())
}

/// The `results` of a `COMPOSIO_MANAGE_CONNECTIONS` answer (`{"successful", "data": {"results":
/// {<slug>: …}}}`), or why there are none.
pub fn manage_results(text: &str) -> Result<Value, ComposioError> {
    let body: Value = serde_json::from_str(text.trim())
        .map_err(|_| ComposioError::Upstream(200, "Composio Connect answered something kwant can't read".into()))?;
    if body["successful"] == json!(false) {
        let why = body["error"].as_str().or_else(|| body["data"]["message"].as_str()).unwrap_or_default();
        return Err(ComposioError::Upstream(200, clip(why, 200)));
    }
    let results = body["data"]["results"].clone();
    let results = if results.is_object() { results } else { body["results"].clone() };
    if results.is_object() {
        Ok(results)
    } else {
        Err(ComposioError::Upstream(200, "Composio Connect's answer has no results".into()))
    }
}

/// Whether one app's `list` result has a working account: its `status` is `active`, or one of
/// its accounts' is.
pub fn result_active(entry: &Value) -> bool {
    let active = |v: &Value| v.as_str().is_some_and(|s| s.eq_ignore_ascii_case("active"));
    active(&entry["status"])
        || entry["accounts"].as_array().is_some_and(|a| a.iter().any(|acc| active(&acc["status"])))
}

/// The HTTP status in an MCP client error ("HTTP 401 Unauthorized"), if it carries one.
fn http_status(message: &str) -> Option<u16> {
    let at = message.find("HTTP ")?;
    message[at + 5..].get(..3)?.parse().ok()
}

/// A failed Composio Connect call, in [`ComposioError`] terms: 401 or 403 is the key.
fn connect_error(e: McpError) -> ComposioError {
    let message = match e {
        McpError::Unauthorized { .. } => return ComposioError::KeyRefused,
        McpError::Rpc { message, .. } | McpError::Protocol(message) => {
            return ComposioError::Upstream(200, clip(&message, 200));
        }
        McpError::Startup { message, .. } | McpError::Transport(message) | McpError::Spawn(message) => message,
        McpError::Timeout(..) | McpError::Closed => return ComposioError::Unreachable,
    };
    match http_status(&message) {
        Some(401 | 403) => ComposioError::KeyRefused,
        Some(status) => ComposioError::Upstream(status, String::new()),
        None => ComposioError::Unreachable,
    }
}

/// One app (Composio calls it a toolkit), as kwnt shows it.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct ComposioApp {
    pub slug: String,
    pub name: String,
    pub description: String,
    /// An https logo address, or none.
    pub logo: Option<String>,
    pub tools_count: Option<u64>,
    /// The app works without signing in to it.
    pub no_auth: bool,
}

/// Where a person gets a Composio key. Composio documents no deep link for a Composio Connect key
/// ("Copy your consumer API key from the AI Clients setup in the Composio dashboard"; checked
/// 2026-11-12, signed out the dashboard sends a visitor to Composio's sign-in), so the dashboard.
pub const KEY_PAGE: &str = "https://dashboard.composio.dev";

/// Why a Composio call failed, in words for the person.
#[derive(Debug, Clone, PartialEq)]
pub enum ComposioError {
    /// Composio refused the API key (401).
    KeyRefused,
    /// The key is valid but not allowed to do this (403: a scoped project key).
    KeyNoAccess,
    /// Composio answered with an error; carries a short message.
    Upstream(u16, String),
    Unreachable,
}

impl ComposioError {
    pub fn message(&self) -> String {
        match self {
            Self::KeyRefused => "Composio didn't accept this key. Copy your key again from your Composio dashboard.".into(),
            Self::KeyNoAccess => "This Composio key isn't allowed to do this. Copy a full-access API key from your \
                Composio dashboard, under Project settings › API keys."
                .into(),
            Self::Upstream(status, _) if *status >= 500 => "Composio couldn't be reached; try again in a minute.".into(),
            Self::Upstream(status, m) if m.is_empty() => format!("Composio answered HTTP {status}; try again later."),
            Self::Upstream(status, m) => format!("Composio answered HTTP {status}: {m}"),
            Self::Unreachable => "Composio couldn't be reached; try again in a minute.".into(),
        }
    }

    /// The one thing the person can do about it: `replace_key` or `try_again`.
    pub fn action(&self) -> &'static str {
        match self {
            Self::KeyRefused | Self::KeyNoAccess => "replace_key",
            _ => "try_again",
        }
    }

    /// The key is the problem (refused or not allowed).
    pub fn is_key(&self) -> bool {
        matches!(self, Self::KeyRefused | Self::KeyNoAccess)
    }
}

/// A Tool Router session: one Composio user's MCP server with Composio's search, connect and
/// execute tools, over every app in the catalogue.
#[derive(Debug, Clone, PartialEq)]
pub struct Session {
    pub session_id: String,
    /// The session's MCP address, always https on composio.dev.
    pub mcp_url: String,
    pub user_id: String,
    /// The tools a bot gets (`COMPOSIO_SEARCH_TOOLS`, `COMPOSIO_MANAGE_CONNECTIONS`…).
    pub tools: Vec<String>,
}

/// The Composio user kwant picks for a space when nothing older names one.
pub fn space_user_id(space: &str) -> String {
    format!("kwant-{space}")
}

/// Whether `url` is an https address on composio.dev, the only place a Composio key may go.
pub fn is_composio_https(url: &str) -> bool {
    url::Url::parse(url).ok().is_some_and(|u| {
        u.scheme() == "https" && u.host_str().is_some_and(|h| h == "composio.dev" || h.ends_with(".composio.dev"))
    })
}

fn valid_session_id(id: &str) -> bool {
    !id.is_empty() && id.len() <= 128 && id.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
}

/// A session from a `POST /api/v3.1/tool_router/session` answer. The MCP address must be https on
/// composio.dev: kwant sends the key there with every bot call.
pub fn parse_session(body: &Value, user_id: &str) -> Result<Session, ComposioError> {
    let session_id = body["session_id"]
        .as_str()
        .filter(|s| valid_session_id(s))
        .ok_or_else(|| ComposioError::Upstream(201, "no session_id in the answer".into()))?;
    let mcp_url = body["mcp"]["url"]
        .as_str()
        .filter(|u| u.len() <= 2048 && is_composio_https(u))
        .ok_or_else(|| ComposioError::Upstream(201, "no https composio.dev MCP address in the answer".into()))?;
    let tools = body["tool_router_tools"]
        .as_array()
        .map(|t| t.iter().filter_map(|n| n.as_str()).filter(|n| n.len() <= 128).take(64).map(str::to_string).collect())
        .unwrap_or_default();
    Ok(Session { session_id: session_id.into(), mcp_url: mcp_url.into(), user_id: user_id.into(), tools })
}

/// The apps (toolkit slugs) with a working account of `user_id` in a `GET connected_accounts`
/// answer, checked the same way as [`has_active_account`].
pub fn active_apps(body: &Value, user_id: &str) -> std::collections::BTreeSet<String> {
    body["items"]
        .as_array()
        .map(|items| {
            items
                .iter()
                .filter(|a| {
                    a["status"] == "ACTIVE"
                        && a["is_disabled"] != json!(true)
                        && a["user_id"].as_str().is_none_or(|u| u == user_id)
                })
                .filter_map(|a| a["toolkit"]["slug"].as_str().filter(|s| valid_slug(s)).map(str::to_string))
                .collect()
        })
        .unwrap_or_default()
}

pub struct ComposioApi {
    base_url: String,
    /// Composio Connect's MCP address, for `ck_` keys.
    connect_url: String,
    /// Composio's public app pages, for the keyless check.
    toolkits_url: String,
    client: reqwest::Client,
}

/// Whether `slug` looks like a Composio toolkit slug (`gmail`, `google_drive`, `x-twitter`).
pub fn valid_slug(slug: &str) -> bool {
    !slug.is_empty()
        && slug.len() <= 64
        && slug
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' || c == '-')
}

/// The Composio user an MCP URL acts for: its `user_id` query value.
pub fn user_id_of(mcp_url: &str) -> Option<String> {
    let url = url::Url::parse(mcp_url).ok()?;
    let id = url
        .query_pairs()
        .find(|(k, _)| k == "user_id")
        .map(|(_, v)| v.trim().to_string())?;
    (!id.is_empty() && id.len() <= 256).then_some(id)
}

fn https_or_none(v: &Value) -> Option<String> {
    v.as_str()
        .filter(|s| s.starts_with("https://") && s.len() <= 2048)
        .map(str::to_string)
}

fn clip(text: &str, max: usize) -> String {
    match text.char_indices().nth(max) {
        Some((i, _)) => format!("{}…", &text[..i]),
        None => text.to_string(),
    }
}

/// Apps from a `GET /api/v3.1/toolkits` answer; entries without a usable slug are left out.
pub fn parse_apps(body: &Value) -> Vec<ComposioApp> {
    body["items"]
        .as_array()
        .map(|items| {
            items
                .iter()
                .filter_map(|t| {
                    let slug = t["slug"].as_str().filter(|s| valid_slug(s))?.to_string();
                    let name = t["name"]
                        .as_str()
                        .map(|n| clip(n, 80))
                        .unwrap_or_else(|| slug.clone());
                    Some(ComposioApp {
                        description: clip(
                            t["meta"]["description"].as_str().unwrap_or_default(),
                            240,
                        ),
                        logo: https_or_none(&t["meta"]["logo"]),
                        tools_count: t["meta"]["tools_count"].as_u64(),
                        no_auth: t["no_auth"] == json!(true),
                        slug,
                        name,
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

/// Whether a `GET /api/v3.1/connected_accounts` answer holds a working account of `user_id` for the
/// app `slug`. Composio filters by user, app and status already; this checks again, so a wider
/// answer never reads as connected. Composio is dropping `user_id` from the answer, so a missing
/// one is fine but a different one isn't.
pub fn has_active_account(body: &Value, user_id: &str, slug: &str) -> bool {
    body["items"].as_array().is_some_and(|items| {
        items.iter().any(|a| {
            a["status"] == "ACTIVE"
                && a["toolkit"]["slug"] == slug
                && a["is_disabled"] != json!(true)
                && a["user_id"].as_str().is_none_or(|u| u == user_id)
        })
    })
}

impl ComposioApi {
    /// The Composio API at `base_url`. Composio Connect is Composio's own for Composio's API,
    /// else `<base_url>/connect/mcp`, so a fake Composio serves both.
    pub fn new(base_url: &str) -> Self {
        let base_url = base_url.trim_end_matches('/').to_string();
        let ours = base_url == DEFAULT_API_URL;
        let connect_url = if ours { CONNECT_MCP_URL.to_string() } else { format!("{base_url}/connect/mcp") };
        let toolkits_url = if ours { TOOLKITS_PAGE_URL.to_string() } else { format!("{base_url}/toolkits") };
        Self {
            connect_url,
            toolkits_url,
            base_url,
            client: reqwest::Client::builder()
                .timeout(TIMEOUT)
                // the key goes in a header: never follow a redirect to another host with it
                .redirect(reqwest::redirect::Policy::none())
                .user_agent(concat!("kwnt/", env!("CARGO_PKG_VERSION")))
                .build()
                .unwrap_or_default(),
        }
    }

    /// [`API_URL_ENV`], else Composio's own API; [`CONNECT_URL_ENV`] for Composio Connect.
    pub fn from_env() -> Self {
        let url = std::env::var(API_URL_ENV)
            .ok()
            .filter(|u| u.starts_with("http"));
        let api = Self::new(url.as_deref().unwrap_or(DEFAULT_API_URL));
        match std::env::var(CONNECT_URL_ENV).ok().filter(|u| u.starts_with("http")) {
            Some(c) => api.with_connect_url(&c),
            None => api,
        }
    }

    /// The same API with Composio Connect at `url`.
    pub fn with_connect_url(mut self, url: &str) -> Self {
        self.connect_url = url.to_string();
        self
    }

    /// Composio Connect as an MCP server with `key`, for kwnt's own calls (never saved).
    fn connect_server(&self, key: &str) -> McpServer {
        McpServer {
            id: "composio-connect".into(),
            name: "composio".into(),
            transport: "http".into(),
            command: None,
            args: vec![],
            url: Some(self.connect_url.clone()),
            env: Default::default(),
            headers: std::collections::BTreeMap::from([(CONNECT_KEY_HEADER.to_string(), key.to_string())]),
            enabled: true,
            timeout_secs: CONNECT_TIMEOUT_SECS,
            created_at: None,
        }
    }

    /// Checks a Composio Connect key: the MCP handshake and `tools/list`. Returns the tools a bot
    /// gets; Composio Connect without [`MANAGE_TOOL`] isn't one kwnt can use.
    pub async fn check_connect_key(&self, key: &str) -> Result<Vec<String>, ComposioError> {
        let mut client = McpClient::connect(&self.connect_server(key), None).await.map_err(connect_error)?;
        let tools = client.list_tools().await.map_err(connect_error);
        client.shutdown().await;
        let tools: Vec<String> = tools?.into_iter().map(|t| t.name).filter(|n| n.len() <= 128).take(64).collect();
        if !tools.iter().any(|t| t == MANAGE_TOOL) {
            return Err(ComposioError::Upstream(200, format!("Composio Connect has no {MANAGE_TOOL} tool")));
        }
        Ok(tools)
    }

    /// One tool call on Composio Connect; the answer's text.
    async fn connect_call(&self, key: &str, tool: &str, arguments: Value) -> Result<String, ComposioError> {
        let mut client = McpClient::connect(&self.connect_server(key), None).await.map_err(connect_error)?;
        let result = client.call_tool(tool, arguments).await.map_err(connect_error);
        client.shutdown().await;
        let result = result?;
        if result.is_error {
            return Err(ComposioError::Upstream(200, clip(&result.text, 200)));
        }
        Ok(result.text)
    }

    /// One `COMPOSIO_MANAGE_CONNECTIONS` call on Composio Connect; its `results`.
    async fn manage(&self, key: &str, toolkits: Value) -> Result<Value, ComposioError> {
        manage_results(&self.connect_call(key, MANAGE_TOOL, json!({"toolkits": toolkits})).await?)
    }

    /// Which of `candidates` are apps Composio has, from one `COMPOSIO_SEARCH_TOOLS` call on
    /// Composio Connect with the name the person typed (no side effects: it finds tools, it runs
    /// none). See [`found_toolkits`].
    pub async fn connect_find_apps(&self, key: &str, query: &str, candidates: &[String]) -> Result<Vec<FoundApp>, ComposioError> {
        if candidates.is_empty() {
            return Ok(vec![]);
        }
        let args = json!({
            "queries": [{"use_case": clip(query.trim(), MAX_QUERY_CHARS)}],
            "session": {"generate_id": true},
        });
        found_toolkits(&self.connect_call(key, SEARCH_TOOL, args).await?, candidates)
    }

    /// Whether Composio has an app with this exact slug, from its public app page: `Some(true)`
    /// for 2xx, `Some(false)` for 404 or 410, `None` when it can't be told (a timeout, another
    /// status, a redirect). No key and no cookies go with it; it never follows a redirect.
    pub async fn toolkit_page_exists(&self, slug: &str) -> Option<bool> {
        if !valid_slug(slug) {
            return Some(false);
        }
        let url = format!("{}/{slug}", self.toolkits_url.trim_end_matches('/'));
        let res = self.client.head(url).timeout(PAGE_CHECK_TIMEOUT).send().await.ok()?;
        match res.status().as_u16() {
            200..=299 => Some(true),
            404 | 410 => Some(false),
            _ => None,
        }
    }

    /// Starts signing in to the app `slug` through Composio Connect (`add`): the address the
    /// person opens, only ever https on composio.dev.
    pub async fn connect_add_link(&self, key: &str, slug: &str) -> Result<String, ComposioError> {
        let results = self.manage(key, json!([{"name": slug, "action": "add"}])).await?;
        results[slug]["redirect_url"]
            .as_str()
            .filter(|u| u.len() <= 2048 && is_composio_https(u))
            .map(str::to_string)
            .ok_or_else(|| ComposioError::Upstream(200, "no https composio.dev sign-in link in the answer".into()))
    }

    /// Which of `slugs` have a working account on Composio Connect, in one `list` call (no side
    /// effects). At most [`MAX_STATUS_SLUGS`]; slugs that aren't slugs are left out.
    pub async fn connect_active(&self, key: &str, slugs: &[&str]) -> Result<BTreeSet<String>, ComposioError> {
        let asked: Vec<&str> = slugs.iter().copied().filter(|s| valid_slug(s)).take(MAX_STATUS_SLUGS).collect();
        if asked.is_empty() {
            return Ok(BTreeSet::new());
        }
        let toolkits: Vec<Value> = asked.iter().map(|s| json!({"name": s, "action": "list"})).collect();
        let results = self.manage(key, Value::Array(toolkits)).await?;
        Ok(asked.into_iter().filter(|s| result_active(&results[*s])).map(str::to_string).collect())
    }

    fn url(&self, segments: &[&str]) -> Result<reqwest::Url, ComposioError> {
        let mut url =
            reqwest::Url::parse(&self.base_url).map_err(|_| ComposioError::Unreachable)?;
        url.path_segments_mut()
            .map_err(|_| ComposioError::Unreachable)?
            .pop_if_empty()
            .extend(segments);
        Ok(url)
    }

    async fn send(
        &self,
        req: reqwest::RequestBuilder,
        api_key: &str,
    ) -> Result<Value, ComposioError> {
        let res = req
            .header("x-api-key", api_key)
            .send()
            .await
            .map_err(|_| ComposioError::Unreachable)?;
        let status = res.status();
        if res.content_length().is_some_and(|n| n as usize > MAX_BODY) {
            return Err(ComposioError::Upstream(
                status.as_u16(),
                "the answer was too large".into(),
            ));
        }
        let body = res.bytes().await.map_err(|_| ComposioError::Unreachable)?;
        if body.len() > MAX_BODY {
            return Err(ComposioError::Upstream(
                status.as_u16(),
                "the answer was too large".into(),
            ));
        }
        let value: Value = serde_json::from_slice(&body).unwrap_or(Value::Null);
        if status == reqwest::StatusCode::UNAUTHORIZED {
            return Err(ComposioError::KeyRefused);
        }
        if status == reqwest::StatusCode::FORBIDDEN {
            return Err(ComposioError::KeyNoAccess);
        }
        if !status.is_success() {
            let message = value["error"]["message"]
                .as_str()
                .or_else(|| value["message"].as_str())
                .unwrap_or_default();
            return Err(ComposioError::Upstream(status.as_u16(), clip(message, 200)));
        }
        Ok(value)
    }

    /// Apps whose name, slug or description match `query`.
    /// Composio's whole app catalogue, most used first, paged through with `cursor` (at most
    /// `MAX_LIST_PAGES` pages of `LIST_PAGE`). Duplicate or unusable slugs are left out.
    pub async fn all_apps(&self, api_key: &str) -> Result<Vec<ComposioApp>, ComposioError> {
        let mut apps: Vec<ComposioApp> = Vec::new();
        let mut seen = std::collections::HashSet::new();
        let mut cursor: Option<String> = None;
        for _ in 0..MAX_LIST_PAGES {
            let mut url = self.url(&["api", "v3.1", "toolkits"])?;
            {
                let mut q = url.query_pairs_mut();
                q.append_pair("sort_by", "usage").append_pair("limit", &LIST_PAGE.to_string());
                if let Some(c) = &cursor {
                    q.append_pair("cursor", c);
                }
            }
            let body = self.send(self.client.get(url), api_key).await?;
            for app in parse_apps(&body) {
                if seen.insert(app.slug.clone()) {
                    apps.push(app);
                }
            }
            cursor = body["next_cursor"].as_str().filter(|c| !c.is_empty() && c.len() <= 512).map(str::to_string);
            if cursor.is_none() {
                break;
            }
        }
        Ok(apps)
    }

    pub async fn search(
        &self,
        api_key: &str,
        query: &str,
    ) -> Result<Vec<ComposioApp>, ComposioError> {
        let mut url = self.url(&["api", "v3.1", "toolkits"])?;
        url.query_pairs_mut()
            .append_pair("search", query)
            .append_pair("sort_by", "usage")
            .append_pair("limit", &PAGE.to_string());
        let body = self.send(self.client.get(url), api_key).await?;
        Ok(parse_apps(&body))
    }

    /// Creates a Tool Router session for `user_id` over every app (no toolkit list), with
    /// connection management on, so a bot can find any app's tools and ask to connect one.
    /// This is also the key check: a refused key fails here and nothing is kept.
    pub async fn create_session(&self, api_key: &str, user_id: &str) -> Result<Session, ComposioError> {
        let body = self
            .send(
                self.client
                    .post(self.url(&["api", "v3.1", "tool_router", "session"])?)
                    .json(&json!({"user_id": user_id, "manage_connections": {"enable": true}})),
                api_key,
            )
            .await?;
        parse_session(&body, user_id)
    }

    /// The user of session `session_id` as this key's project sees it; `None` when the project
    /// has no such session (another project's key, or a deleted session).
    pub async fn session_user(&self, api_key: &str, session_id: &str) -> Result<Option<String>, ComposioError> {
        if !valid_session_id(session_id) {
            return Ok(None);
        }
        match self.send(self.client.get(self.url(&["api", "v3.1", "tool_router", "session", session_id])?), api_key).await {
            Ok(body) => Ok(body["config"]["user_id"].as_str().or_else(|| body["user_id"].as_str()).map(str::to_string)),
            Err(ComposioError::Upstream(400 | 404, _)) => Ok(None),
            Err(e) => Err(e),
        }
    }

    /// One cheap authenticated call: `Ok` when Composio takes the key.
    pub async fn check_key(&self, api_key: &str) -> Result<(), ComposioError> {
        let mut url = self.url(&["api", "v3.1", "connected_accounts"])?;
        url.query_pairs_mut().append_pair("limit", "1");
        self.send(self.client.get(url), api_key).await.map(|_| ())
    }

    /// A sign-in link for the app `slug` in an existing session.
    pub async fn session_link(&self, api_key: &str, session_id: &str, slug: &str) -> Result<String, ComposioError> {
        if !valid_session_id(session_id) {
            return Err(ComposioError::Upstream(0, "the saved session id isn't valid".into()));
        }
        let link = self
            .send(
                self.client
                    .post(self.url(&["api", "v3.1", "tool_router", "session", session_id, "link"])?)
                    .json(&json!({"toolkit": slug})),
                api_key,
            )
            .await?;
        https_or_none(&link["redirect_url"])
            .ok_or_else(|| ComposioError::Upstream(201, "no https redirect_url in the answer".into()))
    }

    /// Starts signing `user_id` in to the app `slug`: a session for the user, then a sign-in link
    /// for the app. Returns the https address the person opens. (Setups made from an MCP URL.)
    pub async fn connect_link(
        &self,
        api_key: &str,
        user_id: &str,
        slug: &str,
    ) -> Result<String, ComposioError> {
        let session = self
            .send(
                self.client
                    .post(self.url(&["api", "v3.1", "tool_router", "session"])?)
                    .json(&json!({"user_id": user_id})),
                api_key,
            )
            .await?;
        let session_id = session["session_id"]
            .as_str()
            .filter(|s| !s.is_empty() && s.len() <= 256)
            .ok_or_else(|| ComposioError::Upstream(200, "no session_id in the answer".into()))?;
        self.session_link(api_key, session_id, slug).await
    }

    /// Every app `user_id` has a working account for, in one call.
    pub async fn connected_apps(
        &self,
        api_key: &str,
        user_id: &str,
    ) -> Result<std::collections::BTreeSet<String>, ComposioError> {
        let mut url = self.url(&["api", "v3.1", "connected_accounts"])?;
        url.query_pairs_mut()
            .append_pair("user_ids", user_id)
            .append_pair("statuses", "ACTIVE")
            .append_pair("limit", "200");
        let body = self.send(self.client.get(url), api_key).await?;
        Ok(active_apps(&body, user_id))
    }

    /// Whether `user_id` has a working (ACTIVE) connection to the app `slug` on Composio.
    pub async fn is_connected(
        &self,
        api_key: &str,
        user_id: &str,
        slug: &str,
    ) -> Result<bool, ComposioError> {
        let mut url = self.url(&["api", "v3.1", "connected_accounts"])?;
        url.query_pairs_mut()
            .append_pair("user_ids", user_id)
            .append_pair("toolkit_slugs", slug)
            .append_pair("statuses", "ACTIVE")
            .append_pair("limit", &PAGE.to_string());
        let body = self.send(self.client.get(url), api_key).await?;
        Ok(has_active_account(&body, user_id, slug))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_typed_name_gives_at_most_three_slugs() {
        for q in ["ElevenLabs", "elevenlabs", " ELEVENLABS "] {
            assert_eq!(slug_candidates(q), vec!["elevenlabs"], "{q}");
        }
        assert_eq!(slug_candidates("Eleven Labs"), vec!["elevenlabs", "eleven_labs", "eleven-labs"]);
        assert_eq!(slug_candidates("Google  Search-Console"), vec!["googlesearchconsole", "google_search_console", "google-search-console"]);
        assert_eq!(slug_candidates("x.com!"), vec!["xcom", "x_com", "x-com"]);
        for none in ["", "a", "?!", "Café", "日本"] {
            assert!(slug_candidates(none).is_empty(), "{none}");
        }
        assert!(slug_candidates(&"word ".repeat(30)).iter().all(|s| valid_slug(s)), "too long ones are left out");
    }

    fn recorded(name: &str) -> String {
        std::fs::read_to_string(format!("{}/tests/fixtures/composio/{name}", env!("CARGO_MANIFEST_DIR"))).unwrap()
    }

    #[test]
    fn composio_search_finds_only_the_slugs_asked_about() {
        let el = |q: &str| slug_candidates(q);
        let found = found_toolkits(&recorded("search_tools_elevenlabs.json"), &el("ElevenLabs")).unwrap();
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].slug, "elevenlabs");
        assert_eq!(found[0].connected, Some(true));
        assert!(found[0].description.as_deref().unwrap().starts_with("Create natural AI voices"));
        assert!(!format!("{found:?}").contains("redacted"), "nothing about the person's accounts is kept: {found:?}");
        let found = found_toolkits(&recorded("search_tools_eleven_labs.json"), &el("Eleven Labs")).unwrap();
        assert_eq!(found.iter().map(|f| f.slug.as_str()).collect::<Vec<_>>(), vec!["elevenlabs"]);
        // A made-up name still gets apps back (Google, a design tool): none of them is the one typed.
        let text = recorded("search_tools_notarealapp123.json");
        assert!(text.contains("googlesuper"));
        assert_eq!(found_toolkits(&text, &el("notarealapp123")).unwrap(), vec![]);
        // A failed search is an error, not "no such app".
        assert!(found_toolkits(r#"{"successful": false, "error": "rate limited"}"#, &el("x y")).is_err());
        assert!(found_toolkits("not json", &el("x y")).is_err());
    }

    #[test]
    fn manage_list_cannot_tell_a_real_app_from_a_made_up_one() {
        // Why kwant asks COMPOSIO_SEARCH_TOOLS instead: heygen is real, notarealapp123 isn't, and
        // `list` answers both the same.
        let results = manage_results(&recorded("manage_list_real_and_made_up.json")).unwrap();
        assert!(result_active(&results["elevenlabs"]));
        assert_eq!(results["heygen"]["status"], results["notarealapp123"]["status"]);
    }

    #[test]
    fn slugs_are_plain_lowercase_words() {
        assert!(valid_slug("gmail") && valid_slug("google_drive") && valid_slug("x-twitter"));
        for bad in ["", "Gmail", "../x", "a/b", "a b", "a?b", &"a".repeat(65)] {
            assert!(!valid_slug(bad), "{bad}");
        }
    }

    #[test]
    fn user_id_comes_from_the_mcp_url() {
        assert_eq!(
            user_id_of("https://backend.composio.dev/v3/mcp/abc/mcp?user_id=u-1").as_deref(),
            Some("u-1")
        );
        assert_eq!(
            user_id_of("https://backend.composio.dev/v3/mcp/abc/mcp?user_id=me%40x.dev&a=1")
                .as_deref(),
            Some("me@x.dev")
        );
        assert_eq!(
            user_id_of("https://backend.composio.dev/v3/mcp/abc/mcp"),
            None
        );
        assert_eq!(
            user_id_of("https://backend.composio.dev/v3/mcp/abc/mcp?user_id="),
            None
        );
    }

    #[test]
    fn apps_keep_only_safe_fields() {
        let body = json!({"items": [
            {"slug": "gmail", "name": "Gmail", "no_auth": false,
             "meta": {"description": "Mail", "logo": "https://logos.composio.dev/gmail.png", "tools_count": 23}},
            {"slug": "hack", "name": "Hack", "meta": {"logo": "javascript:alert(1)"}},
            {"slug": "../etc", "name": "Bad"},
            {"name": "No slug"}
        ]});
        let apps = parse_apps(&body);
        assert_eq!(apps.len(), 2);
        assert_eq!(
            apps[0].logo.as_deref(),
            Some("https://logos.composio.dev/gmail.png")
        );
        assert_eq!(apps[0].tools_count, Some(23));
        assert_eq!(apps[1].logo, None, "only https logos");
        assert!(parse_apps(&Value::Null).is_empty());
    }

    #[test]
    fn a_session_must_point_its_mcp_server_at_composio_over_https() {
        let ok = json!({"session_id": "trs_1", "mcp": {"type": "http", "url": "https://backend.composio.dev/tool_router/trs_1/mcp"},
            "tool_router_tools": ["COMPOSIO_SEARCH_TOOLS", "COMPOSIO_MANAGE_CONNECTIONS"]});
        let s = parse_session(&ok, "kwant-spc_default").unwrap();
        assert_eq!(s.mcp_url, "https://backend.composio.dev/tool_router/trs_1/mcp");
        assert_eq!(s.tools, ["COMPOSIO_SEARCH_TOOLS", "COMPOSIO_MANAGE_CONNECTIONS"]);
        for url in ["http://backend.composio.dev/x", "https://evil.example/mcp", "https://composio.dev.evil.example/mcp"] {
            let mut bad = ok.clone();
            bad["mcp"]["url"] = json!(url);
            assert!(parse_session(&bad, "u").is_err(), "{url}");
        }
        let mut bad = ok.clone();
        bad["session_id"] = json!("../x");
        assert!(parse_session(&bad, "u").is_err());
    }

    #[test]
    fn active_apps_are_the_working_accounts_of_this_user() {
        let body = json!({"items": [
            {"status": "ACTIVE", "toolkit": {"slug": "gmail"}},
            {"status": "ACTIVE", "toolkit": {"slug": "slack"}, "user_id": "someone-else"},
            {"status": "INITIATED", "toolkit": {"slug": "notion"}},
            {"status": "ACTIVE", "toolkit": {"slug": "github"}, "is_disabled": true},
            {"status": "ACTIVE", "toolkit": {"slug": "../x"}}
        ]});
        assert_eq!(active_apps(&body, "u-1").into_iter().collect::<Vec<_>>(), ["gmail"]);
        assert_eq!(space_user_id("spc_default"), "kwant-spc_default");
    }

    #[test]
    fn errors_say_what_to_do() {
        assert_eq!(ComposioError::KeyRefused.message(), "Composio didn't accept this key. Copy your key again from your Composio dashboard.");
        assert!(!ComposioError::KeyRefused.message().contains("ak_"), "either kind of key works");
        assert_eq!(ComposioError::KeyRefused.action(), "replace_key");
        assert_eq!(ComposioError::KeyNoAccess.action(), "replace_key");
        assert_eq!(ComposioError::Unreachable.message(), "Composio couldn't be reached; try again in a minute.");
        assert_eq!(ComposioError::Upstream(500, "boom".into()).action(), "try_again");
        assert!(!ComposioError::KeyRefused.message().contains("MCP server"));
    }

    #[test]
    fn the_key_prefix_says_its_kind() {
        assert_eq!(key_kind("ak_x"), Some(KeyKind::Project));
        assert_eq!(key_kind("ck_x"), Some(KeyKind::Connect));
        assert_eq!(key_kind("uak_x"), None);
        assert_eq!(key_kind("x"), None);
    }

    #[test]
    fn the_bundled_popular_apps_are_composios_top_120() {
        let apps = popular_apps();
        assert_eq!(apps.len(), 120);
        assert_eq!((apps[0].slug.as_str(), apps[0].name.as_str()), ("gmail", "Gmail"));
        assert!(apps.windows(2).all(|w| w[0].rank < w[1].rank), "most popular first");
        assert!(apps.iter().all(|a| valid_slug(&a.slug) && a.logo.as_deref().is_some_and(|l| l.starts_with("https://"))));
        let slugs: BTreeSet<_> = apps.iter().map(|a| &a.slug).collect();
        assert_eq!(slugs.len(), 120, "no duplicates");
    }

    #[test]
    fn popular_search_puts_names_that_start_with_the_query_first() {
        let slugs = |q: &str, n| search_popular(q, n).into_iter().map(|a| a.slug.as_str()).collect::<Vec<_>>();
        assert_eq!(slugs("gma", 20), ["gmail", "figma"], "starts with, then contains");
        let google = slugs("google", 20);
        assert_eq!(google[0], "googlecalendar", "popularity order among prefix matches: {google:?}");
        assert!(google.len() > 3 && google.iter().all(|s| s.starts_with("google")), "{google:?}");
        assert_eq!(slugs("CAL", 3).first(), Some(&"googlecalendar"), "a later word counts, any case");
        assert_eq!(slugs("google", 2).len(), 2);
        assert!(slugs("zzzz-not-an-app", 20).is_empty());
    }

    #[test]
    fn manage_connections_answers_are_read_as_composio_sends_them() {
        // As Composio answered a `list` on 2026-11-12.
        let listed = r#"{"successful":true,"data":{"message":"…","results":{
            "gmail":{"toolkit":"gmail","status":"active","accounts":[{"id":"ca_1","status":"ACTIVE"}]},
            "slack":{"toolkit":"slack","status":"initiated","accounts":[]},
            "notion":{"toolkit":"notion","status":"initiated","accounts":[{"id":"ca_2","status":"active"}]}},
            "summary":{"total_toolkits":3}},"error":null,"log_id":"log_x"}"#;
        let r = manage_results(listed).unwrap();
        assert!(result_active(&r["gmail"]) && result_active(&r["notion"]) && !result_active(&r["slack"]));
        assert!(!result_active(&r["github"]), "an app not in the answer");
        assert!(manage_results(r#"{"results":{"gmail":{"status":"active"}}}"#).is_ok(), "results at the top");
        assert_eq!(
            manage_results(r#"{"successful":false,"error":"max accounts","data":{}}"#),
            Err(ComposioError::Upstream(200, "max accounts".into()))
        );
        assert!(manage_results("not json").is_err());
        assert!(manage_results(r#"{"successful":true,"data":{}}"#).is_err());
    }

    #[test]
    fn connect_failures_name_the_key_only_for_401_and_403() {
        let unauthorized = McpError::Unauthorized { www_authenticate: None };
        assert_eq!(connect_error(unauthorized), ComposioError::KeyRefused);
        let startup = |m: &str| McpError::Startup { message: m.into(), stderr_tail: String::new() };
        assert_eq!(connect_error(startup("transport error: HTTP 403 Forbidden")), ComposioError::KeyRefused);
        assert_eq!(connect_error(startup("transport error: HTTP 502 Bad Gateway")), ComposioError::Upstream(502, String::new()));
        assert_eq!(connect_error(startup("transport error: error sending request")), ComposioError::Unreachable);
        assert_eq!(connect_error(McpError::Timeout(15, "tools/call".into())), ComposioError::Unreachable);
        assert_eq!(connect_error(McpError::Rpc { code: -32000, message: "nope".into() }), ComposioError::Upstream(200, "nope".into()));
        assert_eq!(ComposioError::Upstream(502, String::new()).message(), "Composio couldn't be reached; try again in a minute.");
    }

    #[test]
    fn only_an_active_enabled_account_of_this_user_and_app_counts() {
        let account = |status: &str, slug: &str, extra: Value| {
            let mut a = json!({"id": "ca_1", "status": status, "toolkit": {"slug": slug}});
            for (k, v) in extra.as_object().unwrap() {
                a[k] = v.clone();
            }
            json!({"items": [a], "next_cursor": null})
        };
        assert!(has_active_account(&account("ACTIVE", "gmail", json!({})), "u-1", "gmail"));
        assert!(has_active_account(
            &account("ACTIVE", "gmail", json!({"user_id": "u-1", "is_disabled": false})),
            "u-1",
            "gmail"
        ));
        for (body, why) in [
            (account("INITIATED", "gmail", json!({})), "not finished"),
            (account("EXPIRED", "gmail", json!({})), "expired"),
            (account("ACTIVE", "slack", json!({})), "another app"),
            (account("ACTIVE", "gmail", json!({"user_id": "u-2"})), "another user"),
            (account("ACTIVE", "gmail", json!({"is_disabled": true})), "disabled"),
            (json!({"items": []}), "none"),
            (Value::Null, "no answer"),
        ] {
            assert!(!has_active_account(&body, "u-1", "gmail"), "{why}");
        }
    }
}
