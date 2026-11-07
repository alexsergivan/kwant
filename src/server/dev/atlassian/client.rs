//! The HTTP side: one [`AtlassianClient`] per connected site, holding the token(s) in memory only.
//!
//! - **Cloud, basic** (`email:token`, a classic API token): `https://<site>/rest/api/3/…`, Confluence
//!   at `https://<site>/wiki/…`.
//! - **Cloud, scoped** (a scoped API token, J3): the same `email:token` basic auth, but only through
//!   the gateway: `https://api.atlassian.com/ex/jira/<cloudId>/rest/api/3/…` and
//!   `…/ex/confluence/<cloudId>/wiki/…`. The cloud id comes from `https://<site>/_edge/tenant_info`.
//! - **Data Center** (a personal access token): `Authorization: Bearer <PAT>`, `…/rest/api/2/…`;
//!   Confluence DC is its own server with its own PAT (`…/rest/api/…`).
//!
//! Every request goes through [`forge::client`] (pinned DNS that refuses private addresses, no
//! proxy, no redirects, an 8 MB body cap). The `Authorization` header is marked sensitive and never
//! logged; `Debug` prints no token; errors carry no URL or header. Writes are never retried.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use reqwest::header::{HeaderMap, HeaderValue, AUTHORIZATION};
use reqwest::{Method, StatusCode};
use serde_json::Value;
use zeroize::Zeroizing;

use crate::server::dev::forge;
use crate::server::dev::git::Resolver;
use crate::server::dev::tickets::source::SourceError;

/// How long one request may take.
pub const TIMEOUT: Duration = Duration::from_secs(20);
/// The longest back-off a 429 or 503 can ask for.
pub const MAX_RETRY_AFTER: u64 = 3600;
/// Atlassian's API gateway for scoped tokens.
pub const GATEWAY: &str = "https://api.atlassian.com";

/// Jira Cloud or Data Center (Server counts as DC: same API).
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    Cloud,
    Dc,
}

impl Kind {
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "cloud" => Some(Self::Cloud),
            "dc" => Some(Self::Dc),
            _ => None,
        }
    }
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Cloud => "cloud",
            Self::Dc => "dc",
        }
    }
}

/// How the token is sent.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Auth {
    /// Cloud classic API token: basic auth at the site.
    Basic,
    /// Cloud scoped API token: basic auth at the gateway.
    Scoped,
    /// Data Center personal access token: bearer.
    Pat,
}

impl Auth {
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "basic" => Some(Self::Basic),
            "scoped" => Some(Self::Scoped),
            "pat" => Some(Self::Pat),
            _ => None,
        }
    }
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Basic => "basic",
            Self::Scoped => "scoped",
            Self::Pat => "pat",
        }
    }
}

/// Where a site is and how to reach it; no secret.
#[derive(Debug, Clone, PartialEq)]
pub struct Site {
    pub kind: Kind,
    /// The Jira base the person typed, normalised: scheme, host, port and any context path, no
    /// trailing slash (`https://jira.example.com`, `https://example.com/jira`).
    pub jira_url: String,
    /// DC only: Confluence's own base. Cloud uses `<jira_url>/wiki`.
    pub confluence_url: Option<String>,
    /// Cloud only: the Atlassian account email the token belongs to.
    pub email: Option<String>,
    pub auth: Auth,
    /// Cloud scoped only.
    pub cloud_id: Option<String>,
    /// The projects tickets are picked up from; `issue` refuses any other.
    pub projects: Vec<String>,
    /// The Confluence spaces searched; empty = Confluence off.
    pub spaces: Vec<String>,
}

/// The token(s), zeroed on drop.
pub struct Secrets {
    pub jira: Zeroizing<String>,
    /// DC Confluence's own PAT; Cloud uses the Jira token.
    pub confluence: Option<Zeroizing<String>>,
}

/// Normalises a person-typed address: adds `https://` when there's no scheme, drops any query,
/// fragment and trailing slash, refuses user info. Err is a sentence for the person.
pub fn normalize_base(raw: &str) -> Result<String, String> {
    let raw = raw.trim();
    if raw.is_empty() {
        return Err("type your Jira address".into());
    }
    let with_scheme = if raw.contains("://") { raw.to_string() } else { format!("https://{raw}") };
    let mut u = url::Url::parse(&with_scheme).map_err(|_| "that isn't a web address".to_string())?;
    if !matches!(u.scheme(), "https" | "http") || u.host_str().is_none() {
        return Err("that isn't a web address".into());
    }
    if !u.username().is_empty() || u.password().is_some() {
        return Err("leave the name and password out of the address".into());
    }
    u.set_query(None);
    u.set_fragment(None);
    // Cloud links are often pasted with a path (`/jira/software/…`, `/browse/X-1`, `/wiki/…`).
    if u.host_str().is_some_and(is_cloud_host) {
        u.set_path("");
    }
    let mut s = u.to_string();
    while s.ends_with('/') {
        s.pop();
    }
    Ok(s)
}

/// `*.atlassian.net` (and the older `*.jira.com`): Jira Cloud by its address.
pub fn is_cloud_host(host: &str) -> bool {
    let h = host.trim_end_matches('.').to_ascii_lowercase();
    h.ends_with(".atlassian.net") || h.ends_with(".jira.com")
}

/// The scoped-token gateway: `KWANT_ATLASSIAN_GATEWAY_URL` (a test seam: the mock serves `/ex/…`),
/// else [`GATEWAY`].
/// Read on every client (not cached, so a test binary can point it at its own mock), without
/// the info line `dev::env_url` writes, since a client is built per poll.
pub fn gateway() -> String {
    std::env::var("KWANT_ATLASSIAN_GATEWAY_URL")
        .ok()
        .map(|raw| raw.trim().trim_end_matches('/').to_string())
        .filter(|raw| url::Url::parse(raw).is_ok_and(|u| matches!(u.scheme(), "http" | "https") && u.host().is_some()))
        .unwrap_or_else(|| GATEWAY.to_string())
}

fn basic(email: &str, token: &str) -> Result<HeaderValue, SourceError> {
    use base64::Engine;
    let raw = Zeroizing::new(format!("{email}:{token}"));
    let enc = Zeroizing::new(format!("Basic {}", base64::engine::general_purpose::STANDARD.encode(raw.as_bytes())));
    sensitive(&enc)
}

fn bearer(token: &str) -> Result<HeaderValue, SourceError> {
    sensitive(&Zeroizing::new(format!("Bearer {token}")))
}

fn sensitive(v: &str) -> Result<HeaderValue, SourceError> {
    let mut h = HeaderValue::from_str(v).map_err(|_| SourceError::Invalid("the token has characters a token can't have".into()))?;
    h.set_sensitive(true);
    Ok(h)
}

/// A connected site. Implements [`TicketSource`](crate::server::dev::tickets::source::TicketSource)
/// and [`TicketWriter`](crate::server::dev::tickets::source::TicketWriter) (in `jira.rs` and
/// `confluence.rs`).
pub struct AtlassianClient {
    pub(super) site: Site,
    http: reqwest::Client,
    jira_auth: HeaderValue,
    conf_auth: Option<HeaderValue>,
    gateway: String,
    /// Confluence Cloud space key → numeric id (v2 pages carry only the id).
    pub(super) space_ids: Mutex<Option<HashMap<String, String>>>,
    /// DC's "Epic Link" custom field id, looked up once (`None` inside: the site has none).
    pub(super) epic_field: tokio::sync::OnceCell<Option<String>>,
}

impl std::fmt::Debug for AtlassianClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AtlassianClient").field("site", &self.site).field("token", &"<redacted>").finish()
    }
}

/// Which product a request is for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Api {
    Jira,
    Confluence,
}

impl AtlassianClient {
    pub fn new(site: Site, secrets: &Secrets, resolver: Arc<dyn Resolver>) -> Result<Self, SourceError> {
        let jira_auth = match (site.auth, site.email.as_deref()) {
            (Auth::Pat, _) => bearer(&secrets.jira)?,
            (_, Some(email)) => basic(email, &secrets.jira)?,
            (_, None) => return Err(SourceError::Invalid("a Jira Cloud token needs its Atlassian email".into())),
        };
        let conf_auth = match (site.kind, &secrets.confluence) {
            (Kind::Cloud, _) => Some(jira_auth.clone()),
            (Kind::Dc, Some(t)) => Some(bearer(t)?),
            (Kind::Dc, None) => None,
        };
        let gateway = gateway();
        let loopback = forge::loopback_allowed(&site.jira_url)
            || site.confluence_url.as_deref().is_some_and(forge::loopback_allowed)
            || (site.auth == Auth::Scoped && forge::loopback_allowed(&gateway));
        Ok(Self {
            http: forge::client(resolver, loopback, TIMEOUT),
            site,
            jira_auth,
            conf_auth,
            gateway,
            space_ids: Mutex::new(None),
            epic_field: tokio::sync::OnceCell::new(),
        })
    }

    pub fn site(&self) -> &Site {
        &self.site
    }

    /// `…/rest/api/3` (Cloud), `…/ex/jira/<id>/rest/api/3` (scoped) or `…/rest/api/2` (DC).
    pub fn jira_api(&self) -> String {
        match (self.site.kind, self.site.auth) {
            (Kind::Dc, _) => format!("{}/rest/api/2", self.site.jira_url),
            (Kind::Cloud, Auth::Scoped) => format!("{}/ex/jira/{}/rest/api/3", self.gateway, self.site.cloud_id.as_deref().unwrap_or("-")),
            (Kind::Cloud, _) => format!("{}/rest/api/3", self.site.jira_url),
        }
    }

    /// Confluence's base: `<site>/wiki` (Cloud), `…/ex/confluence/<id>/wiki` (scoped) or the DC
    /// server's own address. `None`: no Confluence for this site.
    pub fn confluence_base(&self) -> Option<String> {
        match (self.site.kind, self.site.auth) {
            (Kind::Dc, _) => self.site.confluence_url.clone(),
            (Kind::Cloud, Auth::Scoped) => Some(format!("{}/ex/confluence/{}/wiki", self.gateway, self.site.cloud_id.as_deref().unwrap_or("-"))),
            (Kind::Cloud, _) => Some(format!("{}/wiki", self.site.jira_url)),
        }
    }

    /// The browse link people open (never the gateway).
    pub fn browse_url(&self, key: &str) -> String {
        format!("{}/browse/{key}", self.site.jira_url)
    }

    /// The person-facing Confluence base, for page links.
    pub fn confluence_web(&self) -> String {
        match self.site.kind {
            Kind::Dc => self.site.confluence_url.clone().unwrap_or_default(),
            Kind::Cloud => format!("{}/wiki", self.site.jira_url),
        }
    }

    /// One JSON request. `url` must come from [`jira_api`](Self::jira_api) or
    /// [`confluence_base`](Self::confluence_base). 204 and an empty body read as `Null`.
    pub async fn call(&self, api: Api, method: Method, url: &str, body: Option<&Value>) -> Result<Value, SourceError> {
        let auth = match api {
            Api::Jira => self.jira_auth.clone(),
            Api::Confluence => self.conf_auth.clone().ok_or(SourceError::NoSpaces)?,
        };
        send(&self.http, method, url, Some(auth), body).await
    }
}

/// Sends one request and maps the answer. Shared by the client and the site check (which has no
/// client yet). Never logs the URL's query, a header or a body.
pub async fn send(http: &reqwest::Client, method: Method, url: &str, auth: Option<HeaderValue>, body: Option<&Value>) -> Result<Value, SourceError> {
    let parsed = url::Url::parse(url).map_err(|_| SourceError::Invalid("that address can't be used".into()))?;
    let loopback = forge::loopback_allowed(url);
    forge::check_url(&parsed, loopback).map_err(|_| SourceError::Unavailable("kwant doesn't connect to this address".into()))?;
    let path = parsed.path().to_string();
    let mut req = http.request(method.clone(), parsed).header(reqwest::header::ACCEPT, "application/json");
    if let Some(a) = auth {
        req = req.header(AUTHORIZATION, a);
    }
    if let Some(b) = body {
        req = req.json(b);
    }
    let res = match req.send().await {
        Ok(r) => r,
        Err(e) if forge::refused_by_pin(&e) => return Err(SourceError::Unavailable("kwant doesn't connect to this address".into())),
        Err(e) if e.is_timeout() => return Err(SourceError::Unavailable("Jira didn't answer in time".into())),
        Err(_) => return Err(SourceError::Unavailable("Jira couldn't be reached".into())),
    };
    let status = res.status();
    tracing::debug!(%method, path = %path, status = status.as_u16(), "atlassian");
    let headers = res.headers().clone();
    if status.is_redirection() {
        // A login page or another host: never followed, and the token is never re-sent.
        return Err(SourceError::Unavailable("Jira sent us somewhere else (is this the right address?)".into()));
    }
    match status {
        StatusCode::UNAUTHORIZED => return Err(SourceError::Unauthorized),
        StatusCode::FORBIDDEN => return Err(SourceError::Forbidden),
        StatusCode::NOT_FOUND | StatusCode::GONE => return Err(SourceError::NotFound),
        StatusCode::TOO_MANY_REQUESTS => return Err(SourceError::RateLimited { retry_after: retry_after(&headers) }),
        StatusCode::SERVICE_UNAVAILABLE if headers.contains_key("retry-after") => {
            return Err(SourceError::RateLimited { retry_after: retry_after(&headers) })
        }
        s if s.is_server_error() => return Err(SourceError::Unavailable(format!("Jira answered {}", s.as_u16()))),
        _ => {}
    }
    let bytes = forge::body(res).await.ok_or_else(|| SourceError::Unavailable("Jira's answer was too big".into()))?;
    if status.is_client_error() {
        return Err(SourceError::Invalid(client_error(&bytes).unwrap_or_else(|| format!("Jira refused the request ({})", status.as_u16()))));
    }
    if bytes.iter().all(u8::is_ascii_whitespace) {
        return Ok(Value::Null);
    }
    serde_json::from_slice(&bytes).map_err(|_| SourceError::Invalid("Jira's answer wasn't JSON".into()))
}

/// Jira's `errorMessages`/`errors` on a 400, cut short: they describe the query, never the token.
fn client_error(bytes: &[u8]) -> Option<String> {
    let v: Value = serde_json::from_slice(bytes).ok()?;
    let mut msgs: Vec<String> = v["errorMessages"].as_array().into_iter().flatten().filter_map(|m| m.as_str().map(str::to_string)).collect();
    if let Some(o) = v["errors"].as_object() {
        msgs.extend(o.values().filter_map(|m| m.as_str().map(str::to_string)));
    }
    if let Some(m) = v["message"].as_str() {
        msgs.push(m.to_string());
    }
    let joined = msgs.join(" ");
    (!joined.trim().is_empty()).then(|| crate::server::dev::tickets::source::cut(joined.trim(), 300).0)
}

/// `Retry-After` in seconds or as an HTTP date, else `X-RateLimit-Reset` (an ISO time on Cloud),
/// clamped to 1 s..1 h; a 429 without either waits a minute.
pub fn retry_after(headers: &HeaderMap) -> Option<Duration> {
    let h = |name: &str| headers.get(name).and_then(|v| v.to_str().ok()).map(str::trim);
    let now = chrono::Utc::now();
    let secs = h("retry-after")
        .and_then(|v| v.parse::<u64>().ok().or_else(|| chrono::DateTime::parse_from_rfc2822(v).ok().map(|d| (d.timestamp() - now.timestamp()).max(1) as u64)))
        .or_else(|| h("x-ratelimit-reset").and_then(|v| chrono::DateTime::parse_from_rfc3339(v).ok()).map(|d| (d.timestamp() - now.timestamp()).max(1) as u64))
        .unwrap_or(60);
    Some(Duration::from_secs(secs.clamp(1, MAX_RETRY_AFTER)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn addresses_are_normalised() {
        assert_eq!(normalize_base("yourcompany.atlassian.net").unwrap(), "https://yourcompany.atlassian.net");
        assert_eq!(normalize_base("https://yourcompany.atlassian.net/jira/software/projects/TRAIL/boards/1?x=1").unwrap(), "https://yourcompany.atlassian.net");
        assert_eq!(normalize_base("https://jira.example.com/jira/").unwrap(), "https://jira.example.com/jira");
        assert!(normalize_base("https://me:pw@jira.example.com").is_err());
        assert!(normalize_base("ftp://jira.example.com").is_err());
        assert!(normalize_base(" ").is_err());
        assert!(is_cloud_host("yourcompany.atlassian.net") && !is_cloud_host("jira.example.com") && !is_cloud_host("atlassian.net.example.com"));
    }

    #[test]
    fn retry_after_reads_seconds_dates_and_resets() {
        let mut h = HeaderMap::new();
        assert_eq!(retry_after(&h), Some(Duration::from_secs(60)));
        h.insert("retry-after", HeaderValue::from_static("7"));
        assert_eq!(retry_after(&h), Some(Duration::from_secs(7)));
        h.insert("retry-after", HeaderValue::from_static("999999"));
        assert_eq!(retry_after(&h), Some(Duration::from_secs(MAX_RETRY_AFTER)));
        let mut h = HeaderMap::new();
        let later = (chrono::Utc::now() + chrono::Duration::seconds(120)).to_rfc3339();
        h.insert("x-ratelimit-reset", HeaderValue::from_str(&later).unwrap());
        let d = retry_after(&h).unwrap().as_secs();
        assert!((100..=120).contains(&d), "{d}");
    }

    #[test]
    fn the_token_never_shows_in_debug() {
        let site = Site {
            kind: Kind::Cloud,
            jira_url: "https://yourcompany.atlassian.net".into(),
            confluence_url: None,
            email: Some("oleks@example.com".into()),
            auth: Auth::Basic,
            cloud_id: None,
            projects: vec!["TRAIL".into()],
            spaces: vec![],
        };
        let s = Secrets { jira: Zeroizing::new("tok-SECRET-123".into()), confluence: None };
        let c = AtlassianClient::new(site, &s, Arc::new(crate::server::dev::git::SystemResolver)).unwrap();
        assert!(!format!("{c:?}").contains("SECRET"));
        assert!(c.jira_auth.is_sensitive());
        assert!(!format!("{:?}", c.jira_auth).contains("dG9r"), "a sensitive header prints as Sensitive");
        assert_eq!(c.jira_api(), "https://yourcompany.atlassian.net/rest/api/3");
        assert_eq!(c.confluence_base().unwrap(), "https://yourcompany.atlassian.net/wiki");
    }
}
