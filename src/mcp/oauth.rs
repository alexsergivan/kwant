//! OAuth sign-in for http MCP servers (MCP authorization spec 2025-06-18, plus Client ID
//! Metadata Documents from 2025-11-25): protected-resource and authorization-server discovery,
//! dynamic client registration (RFC 7591) or a client ID metadata document, the authorization
//! code flow with PKCE S256 and `resource` (RFC 8707), and token refresh.
//!
//! This module talks HTTP but not to the database; `server::mcp_oauth` stores what it returns.

use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use chrono::{DateTime, Utc};
use reqwest::Url;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};
use thiserror::Error;

/// How long a started sign-in may take before its `state` stops working.
pub const PENDING_TTL: Duration = Duration::from_secs(600);
/// Tokens expiring sooner than this are refreshed before use.
pub const REFRESH_MARGIN_SECS: i64 = 60;
const MAX_PENDING: usize = 64;
const HTTP_TIMEOUT: Duration = Duration::from_secs(15);
const CLIENT_NAME: &str = "kwnt";

/// The client ID metadata document for kwant on this computer (loopback callbacks), published
/// at kwant.si from `site/oauth/client.json`. [`static_client_document`] is its content.
pub const STATIC_CLIENT_ID: &str = "https://kwant.si/oauth/client.json";
/// Where a self-hosted kwant serves its own client ID metadata document.
pub const CLIENT_DOCUMENT_PATH: &str = "/api/mcp/oauth/client.json";
const CALLBACK_PATH: &str = "/api/mcp/oauth/callback";
const DOCUMENT_CLIENT_NAME: &str = "kwant";
const CLIENT_URI: &str = "https://kwant.si";
const LOGO_URI: &str = "https://kwant.si/apple-touch-icon.png";

#[derive(Error, Debug)]
pub enum OAuthError {
    #[error("{0}")]
    Discovery(String),
    #[error(
        "this server's sign-in needs an app registered in the server's own settings; \
         kwant can't register itself there"
    )]
    NoRegistration,
    #[error("this server's sign-in only works when kwant is opened on this computer (localhost) or over https")]
    CimdNeedsHttps,
    #[error("client registration failed: {0}")]
    Registration(String),
    #[error("token request failed: {0}")]
    Token(String),
    #[error("{0}")]
    Http(String),
}

impl OAuthError {
    /// A stable code for errors that trying again can't fix, so the app can say what to do
    /// instead of offering Retry.
    pub fn code(&self) -> Option<&'static str> {
        match self {
            OAuthError::NoRegistration => Some("no_client_registration"),
            OAuthError::CimdNeedsHttps => Some("cimd_needs_https"),
            _ => None,
        }
    }
}

fn http_err(e: reqwest::Error) -> OAuthError {
    OAuthError::Http(e.without_url().to_string())
}

/// Sign-in status for the API; never includes tokens.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct OAuthStatus {
    pub connected: bool,
    pub expires_at: Option<DateTime<Utc>>,
}

/// What the `WWW-Authenticate: Bearer …` challenge of a 401 tells us.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct BearerChallenge {
    pub resource_metadata: Option<String>,
    pub scope: Option<String>,
}

/// Parses the auth-params of a Bearer challenge (RFC 6750 §3 / RFC 9728 §5.1). Quoted and
/// token values are accepted; other schemes in the same header are skipped.
pub fn parse_www_authenticate(header: &str) -> BearerChallenge {
    let mut out = BearerChallenge::default();
    let mut rest = header.trim();
    let mut in_bearer = false;
    while !rest.is_empty() {
        rest = rest.trim_start_matches(|c: char| c == ',' || c.is_whitespace());
        // A name: either a scheme (followed by space or end) or a param (followed by '=')
        let name_end = rest.find(|c: char| c == '=' || c == ',' || c.is_whitespace()).unwrap_or(rest.len());
        let name = &rest[..name_end];
        if name.is_empty() {
            break;
        }
        let after = rest[name_end..].trim_start();
        if let Some(value_part) = after.strip_prefix('=') {
            let value_part = value_part.trim_start();
            let (value, remaining) = if let Some(q) = value_part.strip_prefix('"') {
                let mut value = String::new();
                let mut chars = q.char_indices();
                let mut end = q.len();
                while let Some((i, c)) = chars.next() {
                    match c {
                        '\\' => {
                            if let Some((_, n)) = chars.next() {
                                value.push(n);
                            }
                        }
                        '"' => {
                            end = i + 1;
                            break;
                        }
                        c => value.push(c),
                    }
                }
                (value, &q[end.min(q.len())..])
            } else {
                let end = value_part.find(',').unwrap_or(value_part.len());
                (value_part[..end].trim().to_string(), &value_part[end..])
            };
            if in_bearer {
                match name.to_ascii_lowercase().as_str() {
                    "resource_metadata" => out.resource_metadata = Some(value),
                    "scope" => out.scope = Some(value),
                    _ => {}
                }
            }
            rest = remaining;
        } else {
            in_bearer = name.eq_ignore_ascii_case("bearer");
            rest = after;
        }
    }
    out
}

/// Where to look for protected-resource metadata (RFC 9728 §3): the path-specific location,
/// then the origin root.
pub fn protected_resource_metadata_urls(server_url: &str) -> Vec<String> {
    let Ok(url) = Url::parse(server_url) else { return vec![] };
    let origin = url.origin().ascii_serialization();
    let path = url.path().trim_end_matches('/');
    let mut urls = Vec::new();
    if !path.is_empty() {
        urls.push(format!("{origin}/.well-known/oauth-protected-resource{path}"));
    }
    urls.push(format!("{origin}/.well-known/oauth-protected-resource"));
    urls
}

/// Where to look for authorization-server metadata: RFC 8414 first, then OpenID Connect
/// discovery in both its path-inserted and path-appended forms.
pub fn auth_server_metadata_urls(issuer: &str) -> Vec<String> {
    let Ok(url) = Url::parse(issuer) else { return vec![] };
    let origin = url.origin().ascii_serialization();
    let path = url.path().trim_end_matches('/');
    let mut urls = vec![format!("{origin}/.well-known/oauth-authorization-server{path}")];
    if path.is_empty() {
        urls.push(format!("{origin}/.well-known/openid-configuration"));
    } else {
        urls.push(format!("{origin}/.well-known/openid-configuration{path}"));
        urls.push(format!("{origin}{path}/.well-known/openid-configuration"));
    }
    urls
}

/// The canonical resource identifier for a server URL (RFC 8707): no fragment, no trailing '/'
/// on a bare origin.
pub fn canonical_resource(server_url: &str) -> String {
    match Url::parse(server_url) {
        Ok(mut url) => {
            url.set_fragment(None);
            let s = url.to_string();
            if url.path() == "/" && url.query().is_none() {
                s.trim_end_matches('/').to_string()
            } else {
                s
            }
        }
        Err(_) => server_url.to_string(),
    }
}

/// Endpoints must be https; plain http is only accepted on loopback (local servers and tests).
pub fn acceptable_endpoint(endpoint: &str) -> bool {
    let Ok(url) = Url::parse(endpoint) else { return false };
    match url.scheme() {
        "https" => url.host().is_some(),
        "http" => match url.host() {
            Some(url::Host::Domain(d)) => d.eq_ignore_ascii_case("localhost"),
            Some(url::Host::Ipv4(ip)) => ip.is_loopback(),
            Some(url::Host::Ipv6(ip)) => ip.is_loopback(),
            None => false,
        },
        _ => false,
    }
}

fn random_token() -> String {
    // Two v4 UUIDs: 244 random bits from the OS generator, 64 unreserved characters
    format!("{}{}", uuid::Uuid::new_v4().simple(), uuid::Uuid::new_v4().simple())
}

/// A PKCE pair: the verifier stays here, the S256 challenge goes in the authorize URL.
#[derive(Clone)]
pub struct Pkce {
    pub verifier: String,
    pub challenge: String,
}

impl Pkce {
    pub fn new() -> Self {
        Self::from_verifier(random_token())
    }

    pub fn from_verifier(verifier: String) -> Self {
        let challenge = URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()));
        Self { verifier, challenge }
    }
}

impl Default for Pkce {
    fn default() -> Self {
        Self::new()
    }
}

/// What the token endpoint needs to finish a sign-in, kept between start and callback.
#[derive(Clone)]
pub struct PendingAuth {
    pub server_id: String,
    pub verifier: String,
    pub redirect_uri: String,
    pub token_endpoint: String,
    pub client_id: String,
    pub client_secret: Option<String>,
    pub auth_method: String,
    pub resource: String,
    created: Instant,
}

/// Started sign-ins keyed by their random `state`. Entries expire after [`PENDING_TTL`] and are
/// removed when used, so a `state` works once.
#[derive(Default)]
pub struct PendingStore {
    entries: Mutex<HashMap<String, PendingAuth>>,
}

impl PendingStore {
    pub fn insert(&self, auth: PendingAuth) -> String {
        self.insert_at(auth, Instant::now())
    }

    fn insert_at(&self, mut auth: PendingAuth, now: Instant) -> String {
        auth.created = now;
        let state = random_token();
        let mut entries = self.entries.lock().unwrap();
        entries.retain(|_, a| now.duration_since(a.created) < PENDING_TTL);
        // One open sign-in per server; a new Connect replaces the old one
        entries.retain(|_, a| a.server_id != auth.server_id);
        if entries.len() >= MAX_PENDING {
            if let Some(oldest) = entries.iter().min_by_key(|(_, a)| a.created).map(|(k, _)| k.clone()) {
                entries.remove(&oldest);
            }
        }
        entries.insert(state.clone(), auth);
        state
    }

    /// Removes and returns the sign-in for `state` if it exists and has not expired.
    pub fn take(&self, state: &str) -> Option<PendingAuth> {
        self.take_at(state, Instant::now())
    }

    fn take_at(&self, state: &str, now: Instant) -> Option<PendingAuth> {
        let auth = self.entries.lock().unwrap().remove(state)?;
        (now.duration_since(auth.created) < PENDING_TTL).then_some(auth)
    }
}

/// Whether a token with this expiry should be refreshed before use at `now`.
pub fn needs_refresh(expires_at: Option<DateTime<Utc>>, now: DateTime<Utc>) -> bool {
    expires_at.is_some_and(|t| (t - now).num_seconds() < REFRESH_MARGIN_SECS)
}

/// Adds `Authorization: Bearer <token>` unless the user configured their own Authorization
/// header, which always wins. Returns whether the token was added.
pub fn apply_bearer(headers: &mut std::collections::BTreeMap<String, String>, access_token: &str) -> bool {
    if headers.keys().any(|k| k.eq_ignore_ascii_case("authorization")) {
        return false;
    }
    headers.insert("Authorization".into(), format!("Bearer {access_token}"));
    true
}

pub fn has_user_authorization(headers: &std::collections::BTreeMap<String, String>) -> bool {
    headers.keys().any(|k| k.eq_ignore_ascii_case("authorization"))
}

/// The parts of authorization-server metadata kwnt uses.
#[derive(Debug, Clone, Deserialize, PartialEq)]
pub struct AuthServerMetadata {
    #[serde(default)]
    pub issuer: Option<String>,
    pub authorization_endpoint: String,
    pub token_endpoint: String,
    #[serde(default)]
    pub registration_endpoint: Option<String>,
    #[serde(default)]
    pub code_challenge_methods_supported: Option<Vec<String>>,
    #[serde(default)]
    pub token_endpoint_auth_methods_supported: Option<Vec<String>>,
    /// Client ID Metadata Documents (MCP authorization 2025-11-25): the client_id may be an
    /// https URL of a JSON document describing the client.
    #[serde(default, deserialize_with = "lenient_bool")]
    pub client_id_metadata_document_supported: Option<bool>,
}

/// A flag that is not a JSON boolean counts as absent, so one odd field never hides the rest of
/// the metadata.
fn lenient_bool<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Option<bool>, D::Error> {
    Ok(Value::deserialize(d)?.as_bool())
}

impl AuthServerMetadata {
    pub fn supports_client_documents(&self) -> bool {
        self.client_id_metadata_document_supported == Some(true)
    }
}

/// Everything discovery found for one MCP server.
#[derive(Debug, Clone)]
pub struct Discovery {
    /// Identifies the authorization server; registrations are stored per issuer.
    pub issuer: String,
    pub metadata: AuthServerMetadata,
    pub resource: String,
    pub scope: Option<String>,
}

/// A registered client.
#[derive(Clone)]
pub struct ClientRegistration {
    pub client_id: String,
    pub client_secret: Option<String>,
    pub auth_method: String,
}

/// A token endpoint's answer.
#[derive(Clone)]
pub struct TokenSet {
    pub access_token: String,
    pub refresh_token: Option<String>,
    pub expires_at: Option<DateTime<Utc>>,
}

pub fn http_client() -> reqwest::Client {
    reqwest::Client::builder()
        .timeout(HTTP_TIMEOUT)
        .redirect(reqwest::redirect::Policy::limited(3))
        .build()
        .unwrap_or_default()
}

async fn get_json(http: &reqwest::Client, url: &str) -> Result<Option<Value>, OAuthError> {
    let resp = http
        .get(url)
        .header("Accept", "application/json")
        .header("MCP-Protocol-Version", super::client::PROTOCOL_VERSION)
        .send()
        .await
        .map_err(http_err)?;
    if !resp.status().is_success() {
        return Ok(None);
    }
    Ok(resp.json::<Value>().await.ok())
}

/// Sends an unauthenticated `initialize` to read the 401 challenge, if the server sends one.
pub async fn probe_challenge(http: &reqwest::Client, server_url: &str) -> Option<BearerChallenge> {
    let body = json!({"jsonrpc": "2.0", "id": 0, "method": "initialize", "params": {
        "protocolVersion": super::client::PROTOCOL_VERSION, "capabilities": {},
        "clientInfo": {"name": CLIENT_NAME, "version": env!("CARGO_PKG_VERSION")}}});
    let resp = http
        .post(server_url)
        .header("Accept", "application/json, text/event-stream")
        .json(&body)
        .send()
        .await
        .ok()?;
    if resp.status() != reqwest::StatusCode::UNAUTHORIZED {
        return None;
    }
    let header = resp.headers().get(reqwest::header::WWW_AUTHENTICATE)?.to_str().ok()?;
    Some(parse_www_authenticate(header))
}

/// Finds the authorization server for `server_url`: protected-resource metadata (from the
/// challenge or the well-known locations), then the authorization server's own metadata.
/// Servers without protected-resource metadata are treated as their own authorization server
/// (the 2025-03-26 behaviour).
pub async fn discover(
    http: &reqwest::Client,
    server_url: &str,
    challenge: Option<&BearerChallenge>,
) -> Result<Discovery, OAuthError> {
    let mut resource_urls: Vec<String> =
        challenge.and_then(|c| c.resource_metadata.clone()).into_iter().collect();
    resource_urls.extend(protected_resource_metadata_urls(server_url));

    let mut resource_meta = None;
    for url in resource_urls.iter().filter(|u| acceptable_endpoint(u)) {
        if let Some(meta) = get_json(http, url).await? {
            if meta["authorization_servers"].as_array().is_some_and(|a| !a.is_empty()) {
                resource_meta = Some(meta);
                break;
            }
        }
    }

    let origin = Url::parse(server_url)
        .map(|u| u.origin().ascii_serialization())
        .map_err(|_| OAuthError::Discovery("server url is not valid".into()))?;
    let issuer = resource_meta
        .as_ref()
        .and_then(|m| m["authorization_servers"][0].as_str())
        .map(str::to_string)
        .unwrap_or_else(|| origin.clone());
    if !acceptable_endpoint(&issuer) {
        return Err(OAuthError::Discovery("the authorization server must use https".into()));
    }
    let resource = resource_meta
        .as_ref()
        .and_then(|m| m["resource"].as_str())
        .map(str::to_string)
        .unwrap_or_else(|| canonical_resource(server_url));
    let scope = challenge.and_then(|c| c.scope.clone()).or_else(|| {
        let scopes: Vec<&str> = resource_meta
            .as_ref()?
            .get("scopes_supported")?
            .as_array()?
            .iter()
            .filter_map(Value::as_str)
            .collect();
        (!scopes.is_empty()).then(|| scopes.join(" "))
    });

    let mut metadata = None;
    for url in auth_server_metadata_urls(&issuer) {
        if let Some(meta) = get_json(http, &url).await? {
            if let Ok(parsed) = serde_json::from_value::<AuthServerMetadata>(meta) {
                metadata = Some(parsed);
                break;
            }
        }
    }
    let metadata = match metadata {
        Some(m) => m,
        None if resource_meta.is_none() => AuthServerMetadata {
            // 2025-03-26 default endpoints, relative to the server origin
            issuer: None,
            authorization_endpoint: format!("{origin}/authorize"),
            token_endpoint: format!("{origin}/token"),
            registration_endpoint: Some(format!("{origin}/register")),
            code_challenge_methods_supported: None,
            token_endpoint_auth_methods_supported: None,
            client_id_metadata_document_supported: None,
        },
        None => return Err(OAuthError::Discovery("could not find the server's sign-in metadata".into())),
    };
    for endpoint in [Some(&metadata.authorization_endpoint), Some(&metadata.token_endpoint), metadata.registration_endpoint.as_ref()]
        .into_iter()
        .flatten()
    {
        if !acceptable_endpoint(endpoint) {
            return Err(OAuthError::Discovery("the server's sign-in endpoints must use https".into()));
        }
    }
    if let Some(methods) = &metadata.code_challenge_methods_supported {
        if !methods.iter().any(|m| m == "S256") {
            return Err(OAuthError::Discovery("the server's sign-in does not support PKCE (S256)".into()));
        }
    }
    Ok(Discovery { issuer, metadata, resource, scope })
}

/// Registers kwnt as a public client when the server allows it, otherwise as a confidential one.
pub async fn register_client(
    http: &reqwest::Client,
    meta: &AuthServerMetadata,
    redirect_uri: &str,
) -> Result<ClientRegistration, OAuthError> {
    let endpoint = meta.registration_endpoint.as_deref().ok_or(OAuthError::NoRegistration)?;
    let supported = meta.token_endpoint_auth_methods_supported.as_deref().unwrap_or(&[]);
    let requested = if supported.is_empty() || supported.iter().any(|m| m == "none") {
        "none"
    } else if supported.iter().any(|m| m == "client_secret_post") {
        "client_secret_post"
    } else {
        "client_secret_basic"
    };
    let body = json!({
        "client_name": CLIENT_NAME,
        "redirect_uris": [redirect_uri],
        "grant_types": ["authorization_code", "refresh_token"],
        "response_types": ["code"],
        "token_endpoint_auth_method": requested,
    });
    let resp = http.post(endpoint).json(&body).send().await.map_err(http_err)?;
    let status = resp.status();
    let reply: Value = resp.json().await.unwrap_or(Value::Null);
    if !status.is_success() {
        return Err(OAuthError::Registration(oauth_error_text(&reply, status)));
    }
    let client_id = reply["client_id"]
        .as_str()
        .filter(|s| !s.is_empty())
        .ok_or_else(|| OAuthError::Registration("no client_id in the response".into()))?
        .to_string();
    let client_secret = reply["client_secret"].as_str().filter(|s| !s.is_empty()).map(str::to_string);
    let auth_method = reply["token_endpoint_auth_method"]
        .as_str()
        .map(str::to_string)
        .unwrap_or_else(|| if client_secret.is_some() { "client_secret_post".into() } else { "none".into() });
    Ok(ClientRegistration { client_id, client_secret, auth_method })
}

fn is_loopback_url(url: &Url) -> bool {
    match url.host() {
        Some(url::Host::Domain(d)) => d.eq_ignore_ascii_case("localhost"),
        Some(url::Host::Ipv4(ip)) => ip.is_loopback(),
        Some(url::Host::Ipv6(ip)) => ip.is_loopback(),
        None => false,
    }
}

fn client_document(client_id: &str, application_type: &str, redirect_uris: Vec<String>) -> Value {
    json!({
        "client_id": client_id,
        "client_name": DOCUMENT_CLIENT_NAME,
        "client_uri": CLIENT_URI,
        "logo_uri": LOGO_URI,
        "application_type": application_type,
        "redirect_uris": redirect_uris,
        "grant_types": ["authorization_code", "refresh_token"],
        "response_types": ["code"],
        "token_endpoint_auth_method": "none",
    })
}

/// The document at [`STATIC_CLIENT_ID`]: kwant on this computer. Loopback callbacks without a
/// port (servers that follow RFC 8252 §7.3 accept any port) and on the default port (servers
/// that match exactly).
pub fn static_client_document() -> Value {
    let port = crate::config::DEFAULT_PORT;
    let mut uris = Vec::new();
    for host in ["127.0.0.1", "localhost", "[::1]"] {
        uris.push(format!("http://{host}{CALLBACK_PATH}"));
        uris.push(format!("http://{host}:{port}{CALLBACK_PATH}"));
    }
    client_document(STATIC_CLIENT_ID, "native", uris)
}

/// The document a self-hosted kwant serves at `{origin}`[`CLIENT_DOCUMENT_PATH`]: its own
/// callback only. `None` unless `origin` is a public https origin.
pub fn self_hosted_client_document(origin: &str) -> Option<Value> {
    let url = Url::parse(origin).ok()?;
    if url.scheme() != "https" || is_loopback_url(&url) || url.host().is_none() || url.path() != "/" {
        return None;
    }
    let origin = url.origin().ascii_serialization();
    Some(client_document(&format!("{origin}{CLIENT_DOCUMENT_PATH}"), "web", vec![callback_url(&origin)]))
}

/// The client_id kwant uses with a server that supports client ID metadata documents, for a
/// sign-in started on `origin` (already checked by [`validate_origin`]).
pub fn client_document_id(origin: &str) -> Result<String, OAuthError> {
    let url = Url::parse(origin).map_err(|_| OAuthError::CimdNeedsHttps)?;
    if is_loopback_url(&url) {
        return Ok(STATIC_CLIENT_ID.to_string());
    }
    self_hosted_client_document(origin)
        .and_then(|d| d["client_id"].as_str().map(str::to_string))
        .ok_or(OAuthError::CimdNeedsHttps)
}

/// Picks how kwant identifies itself to a new authorization server: dynamic registration when
/// the server offers it (falling back to a client ID metadata document if registration is
/// refused), otherwise a client ID metadata document. See autonomy/mcp-oauth-cimd/impl.md.
pub async fn obtain_client(
    http: &reqwest::Client,
    meta: &AuthServerMetadata,
    origin: &str,
    redirect_uri: &str,
) -> Result<ClientRegistration, OAuthError> {
    let documents = meta.supports_client_documents();
    if meta.registration_endpoint.is_some() {
        match register_client(http, meta, redirect_uri).await {
            Err(OAuthError::Registration(e)) if documents => {
                if let Ok(client_id) = client_document_id(origin) {
                    tracing::info!(error = %e, "MCP OAuth registration refused; using kwant's client ID metadata document");
                    return Ok(document_client(client_id));
                }
                return Err(OAuthError::Registration(e));
            }
            other => return other,
        }
    }
    if documents {
        return client_document_id(origin).map(document_client);
    }
    Err(OAuthError::NoRegistration)
}

fn document_client(client_id: String) -> ClientRegistration {
    ClientRegistration { client_id, client_secret: None, auth_method: "none".into() }
}

/// The URL the user's browser opens to sign in.
pub fn authorize_url(
    discovery: &Discovery,
    client_id: &str,
    redirect_uri: &str,
    state: &str,
    pkce: &Pkce,
) -> Result<String, OAuthError> {
    let mut url = Url::parse(&discovery.metadata.authorization_endpoint)
        .map_err(|_| OAuthError::Discovery("invalid authorization endpoint".into()))?;
    {
        let mut q = url.query_pairs_mut();
        q.append_pair("response_type", "code")
            .append_pair("client_id", client_id)
            .append_pair("redirect_uri", redirect_uri)
            .append_pair("code_challenge", &pkce.challenge)
            .append_pair("code_challenge_method", "S256")
            .append_pair("state", state)
            .append_pair("resource", &discovery.resource);
        if let Some(scope) = &discovery.scope {
            q.append_pair("scope", scope);
        }
    }
    Ok(url.into())
}

fn oauth_error_text(body: &Value, status: reqwest::StatusCode) -> String {
    let code = body["error"].as_str().unwrap_or("");
    let description = body["error_description"].as_str().unwrap_or("");
    let text = match (code.is_empty(), description.is_empty()) {
        (false, false) => format!("{code}: {description}"),
        (false, true) => code.to_string(),
        _ => format!("HTTP {status}"),
    };
    super::truncate_text(&text, 300)
}

/// Client credentials for a token request.
pub struct ClientAuth<'a> {
    pub client_id: &'a str,
    pub client_secret: Option<&'a str>,
    pub auth_method: &'a str,
}

async fn token_request(
    http: &reqwest::Client,
    token_endpoint: &str,
    client: &ClientAuth<'_>,
    mut form: Vec<(&str, String)>,
    previous_refresh: Option<&str>,
) -> Result<TokenSet, OAuthError> {
    let mut req = http.post(token_endpoint).header("Accept", "application/json");
    match (client.auth_method, client.client_secret) {
        ("client_secret_basic", Some(secret)) => req = req.basic_auth(client.client_id, Some(secret)),
        (_, Some(secret)) => {
            form.push(("client_id", client.client_id.to_string()));
            form.push(("client_secret", secret.to_string()));
        }
        (_, None) => form.push(("client_id", client.client_id.to_string())),
    }
    let resp = req.form(&form).send().await.map_err(http_err)?;
    let status = resp.status();
    let body: Value = resp.json().await.unwrap_or(Value::Null);
    if !status.is_success() {
        return Err(OAuthError::Token(oauth_error_text(&body, status)));
    }
    let access_token = body["access_token"]
        .as_str()
        .filter(|s| !s.is_empty())
        .ok_or_else(|| OAuthError::Token("no access_token in the response".into()))?
        .to_string();
    let expires_at = body["expires_in"]
        .as_i64()
        .or_else(|| body["expires_in"].as_str().and_then(|s| s.parse().ok()))
        .filter(|s| *s > 0)
        .map(|s| Utc::now() + chrono::Duration::seconds(s));
    // A refresh response may omit refresh_token; the old one stays valid then
    let refresh_token = body["refresh_token"]
        .as_str()
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .or_else(|| previous_refresh.map(str::to_string));
    Ok(TokenSet { access_token, refresh_token, expires_at })
}

/// Exchanges an authorization code (with its PKCE verifier) for tokens.
pub async fn exchange_code(http: &reqwest::Client, pending: &PendingAuth, code: &str) -> Result<TokenSet, OAuthError> {
    let client = ClientAuth {
        client_id: &pending.client_id,
        client_secret: pending.client_secret.as_deref(),
        auth_method: &pending.auth_method,
    };
    let form = vec![
        ("grant_type", "authorization_code".to_string()),
        ("code", code.to_string()),
        ("redirect_uri", pending.redirect_uri.clone()),
        ("code_verifier", pending.verifier.clone()),
        ("resource", pending.resource.clone()),
    ];
    token_request(http, &pending.token_endpoint, &client, form, None).await
}

pub async fn refresh(
    http: &reqwest::Client,
    token_endpoint: &str,
    client: &ClientAuth<'_>,
    refresh_token: &str,
    resource: &str,
) -> Result<TokenSet, OAuthError> {
    let form = vec![
        ("grant_type", "refresh_token".to_string()),
        ("refresh_token", refresh_token.to_string()),
        ("resource", resource.to_string()),
    ];
    token_request(http, token_endpoint, client, form, Some(refresh_token)).await
}

/// Builds a [`PendingAuth`]; `created` is set when it is stored.
pub fn pending_auth(
    server_id: &str,
    pkce: &Pkce,
    redirect_uri: &str,
    discovery: &Discovery,
    client: &ClientRegistration,
) -> PendingAuth {
    PendingAuth {
        server_id: server_id.to_string(),
        verifier: pkce.verifier.clone(),
        redirect_uri: redirect_uri.to_string(),
        token_endpoint: discovery.metadata.token_endpoint.clone(),
        client_id: client.client_id.clone(),
        client_secret: client.client_secret.clone(),
        auth_method: client.auth_method.clone(),
        resource: discovery.resource.clone(),
        created: Instant::now(),
    }
}

/// Validates the browser-supplied origin for the callback: http(s), no path, query or
/// credentials, and the same host the request came in on (directly or via a proxy).
pub fn validate_origin(origin: &str, hosts: &[&str]) -> Result<String, String> {
    let url = Url::parse(origin.trim()).map_err(|_| "origin is not a valid URL".to_string())?;
    if !matches!(url.scheme(), "http" | "https") {
        return Err("origin must be http or https".into());
    }
    if !url.username().is_empty() || url.password().is_some() || url.query().is_some() || url.fragment().is_some() {
        return Err("origin must not contain credentials, a query or a fragment".into());
    }
    if url.path() != "/" && !url.path().is_empty() {
        return Err("origin must not contain a path".into());
    }
    let serialized = url.origin().ascii_serialization();
    let authority = serialized.split_once("://").map(|(_, a)| a.to_ascii_lowercase()).unwrap_or_default();
    // Host headers may carry the default port explicitly; Url drops it
    let default_port = if url.scheme() == "https" { ":443" } else { ":80" };
    let matches = hosts.iter().any(|h| {
        let h = h.trim().to_ascii_lowercase();
        h == authority || h.strip_suffix(default_port) == Some(authority.as_str())
    });
    if !matches {
        return Err("origin does not match the host this request was sent to".into());
    }
    Ok(serialized)
}

pub fn callback_url(origin: &str) -> String {
    format!("{origin}{CALLBACK_PATH}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_bearer_challenges() {
        let c = parse_www_authenticate(
            r#"Bearer realm="mcp", resource_metadata="https://mcp.example.com/.well-known/oauth-protected-resource", scope="read write""#,
        );
        assert_eq!(c.resource_metadata.as_deref(), Some("https://mcp.example.com/.well-known/oauth-protected-resource"));
        assert_eq!(c.scope.as_deref(), Some("read write"));

        let c = parse_www_authenticate("Bearer error=invalid_token, resource_metadata=https://x.dev/prm");
        assert_eq!(c.resource_metadata.as_deref(), Some("https://x.dev/prm"));

        // Another scheme's params are ignored; escapes in quoted strings are handled
        let c = parse_www_authenticate(r#"Basic resource_metadata="https://evil.test", Bearer scope="a \"b\"""#);
        assert_eq!(c.resource_metadata, None);
        assert_eq!(c.scope.as_deref(), Some(r#"a "b""#));

        assert_eq!(parse_www_authenticate(""), BearerChallenge::default());
        assert_eq!(parse_www_authenticate("Bearer"), BearerChallenge::default());
        assert_eq!(parse_www_authenticate(r#"Bearer resource_metadata="unterminated"#).resource_metadata.as_deref(), Some("unterminated"));
    }

    #[test]
    fn builds_metadata_urls() {
        assert_eq!(
            protected_resource_metadata_urls("https://mcp.notion.com/mcp"),
            vec![
                "https://mcp.notion.com/.well-known/oauth-protected-resource/mcp",
                "https://mcp.notion.com/.well-known/oauth-protected-resource",
            ]
        );
        assert_eq!(protected_resource_metadata_urls("https://a.dev/"), vec!["https://a.dev/.well-known/oauth-protected-resource"]);
        assert_eq!(
            auth_server_metadata_urls("https://auth.dev"),
            vec!["https://auth.dev/.well-known/oauth-authorization-server", "https://auth.dev/.well-known/openid-configuration"]
        );
        assert_eq!(
            auth_server_metadata_urls("https://auth.dev/tenant1"),
            vec![
                "https://auth.dev/.well-known/oauth-authorization-server/tenant1",
                "https://auth.dev/.well-known/openid-configuration/tenant1",
                "https://auth.dev/tenant1/.well-known/openid-configuration",
            ]
        );
        assert_eq!(canonical_resource("https://mcp.notion.com/mcp#x"), "https://mcp.notion.com/mcp");
        assert_eq!(canonical_resource("https://mcp.dev/"), "https://mcp.dev");
    }

    #[test]
    fn only_https_or_loopback_endpoints() {
        assert!(acceptable_endpoint("https://auth.dev/token"));
        assert!(acceptable_endpoint("http://127.0.0.1:9/token"));
        assert!(acceptable_endpoint("http://localhost/token"));
        assert!(acceptable_endpoint("http://[::1]:8/token"));
        assert!(!acceptable_endpoint("http://auth.dev/token"));
        assert!(!acceptable_endpoint("http://10.0.0.1/token"));
        assert!(!acceptable_endpoint("javascript:alert(1)"));
        assert!(!acceptable_endpoint("file:///etc/passwd"));
    }

    #[test]
    fn pkce_s256_matches_rfc7636_example() {
        let p = Pkce::from_verifier("dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk".into());
        assert_eq!(p.challenge, "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM");
        let a = Pkce::new();
        let b = Pkce::new();
        assert_ne!(a.verifier, b.verifier);
        assert!((43..=128).contains(&a.verifier.len()));
        assert!(a.verifier.chars().all(|c| c.is_ascii_alphanumeric()));
    }

    fn pending(server: &str) -> PendingAuth {
        PendingAuth {
            server_id: server.into(),
            verifier: "v".into(),
            redirect_uri: "https://k.dev/api/mcp/oauth/callback".into(),
            token_endpoint: "https://a.dev/token".into(),
            client_id: "c".into(),
            client_secret: None,
            auth_method: "none".into(),
            resource: "https://m.dev/mcp".into(),
            created: Instant::now(),
        }
    }

    #[test]
    fn state_is_single_use_and_expires() {
        let store = PendingStore::default();
        let state = store.insert(pending("s1"));
        assert!(state.len() >= 43);
        assert!(store.take("not-a-state").is_none());
        assert_eq!(store.take(&state).unwrap().server_id, "s1");
        assert!(store.take(&state).is_none(), "a state works once");

        let now = Instant::now();
        let old = store.insert_at(pending("s2"), now);
        assert!(store.take_at(&old, now + PENDING_TTL + Duration::from_secs(1)).is_none(), "expired");

        // A new start for the same server replaces the earlier one
        let first = store.insert(pending("s3"));
        let second = store.insert(pending("s3"));
        assert!(store.take(&first).is_none());
        assert_eq!(store.take(&second).unwrap().server_id, "s3");
    }

    #[test]
    fn refresh_decision() {
        let now = Utc::now();
        assert!(!needs_refresh(None, now), "no expiry: use as is");
        assert!(!needs_refresh(Some(now + chrono::Duration::seconds(3600)), now));
        assert!(!needs_refresh(Some(now + chrono::Duration::seconds(61)), now));
        assert!(needs_refresh(Some(now + chrono::Duration::seconds(59)), now));
        assert!(needs_refresh(Some(now - chrono::Duration::seconds(5)), now));
    }

    #[test]
    fn user_authorization_header_wins() {
        let mut headers = std::collections::BTreeMap::new();
        assert!(apply_bearer(&mut headers, "tok"));
        assert_eq!(headers["Authorization"], "Bearer tok");

        let mut headers = std::collections::BTreeMap::from([("authorization".to_string(), "Bearer mine".to_string())]);
        assert!(!apply_bearer(&mut headers, "tok"));
        assert_eq!(headers.len(), 1);
        assert_eq!(headers["authorization"], "Bearer mine");
        assert!(has_user_authorization(&headers));
    }

    #[test]
    fn validates_callback_origin() {
        assert_eq!(validate_origin("https://app.example.com", &["app.example.com"]).unwrap(), "https://app.example.com");
        assert_eq!(validate_origin("http://127.0.0.1:7777", &["127.0.0.1:7777"]).unwrap(), "http://127.0.0.1:7777");
        assert_eq!(validate_origin("https://k.dev/", &["k.dev:443"]).unwrap(), "https://k.dev");
        // Proxy: Host is internal, X-Forwarded-Host is public
        assert!(validate_origin("https://k.dev", &["127.0.0.1:7777", "k.dev"]).is_ok());
        assert!(validate_origin("https://evil.test", &["k.dev"]).is_err());
        assert!(validate_origin("https://k.dev.evil.test", &["k.dev"]).is_err());
        assert!(validate_origin("https://k.dev/x", &["k.dev"]).is_err());
        assert!(validate_origin("https://u:p@k.dev", &["k.dev"]).is_err());
        assert!(validate_origin("javascript:alert(1)", &["k.dev"]).is_err());
        assert!(validate_origin("http://k.dev:8080", &["k.dev"]).is_err());
        assert_eq!(callback_url("https://k.dev"), "https://k.dev/api/mcp/oauth/callback");
    }

    /// ElevenLabs' hosted MCP (read Nov 13 2026): no registration endpoint, client documents yes.
    #[test]
    fn reads_client_document_support() {
        let meta: AuthServerMetadata = serde_json::from_value(json!({
            "issuer": "https://api.us.elevenlabs.io",
            "authorization_endpoint": "https://elevenlabs.io/app/oauth/authorize",
            "token_endpoint": "https://api.us.elevenlabs.io/v1/oauth/token",
            "code_challenge_methods_supported": ["S256"],
            "token_endpoint_auth_methods_supported": ["none", "client_secret_post", "private_key_jwt"],
            "client_id_metadata_document_supported": true
        }))
        .unwrap();
        assert!(meta.supports_client_documents());
        assert_eq!(meta.registration_endpoint, None);

        let base = json!({"authorization_endpoint": "https://a.dev/a", "token_endpoint": "https://a.dev/t"});
        let without: AuthServerMetadata = serde_json::from_value(base.clone()).unwrap();
        assert!(!without.supports_client_documents());
        // An odd value never hides the rest of the metadata
        for odd in [json!("true"), json!(1), Value::Null] {
            let mut m = base.clone();
            m["client_id_metadata_document_supported"] = odd;
            let parsed: AuthServerMetadata = serde_json::from_value(m).unwrap();
            assert!(!parsed.supports_client_documents());
            assert_eq!(parsed.token_endpoint, "https://a.dev/t");
        }
    }

    #[test]
    fn picks_the_client_document_for_the_origin() {
        for loopback in ["http://127.0.0.1:8799", "http://127.0.0.1:51234", "http://localhost:8799", "http://[::1]:8799"] {
            assert_eq!(client_document_id(loopback).unwrap(), STATIC_CLIENT_ID, "{loopback}");
        }
        assert_eq!(
            client_document_id("https://kwant.example.com").unwrap(),
            "https://kwant.example.com/api/mcp/oauth/client.json"
        );
        assert_eq!(
            client_document_id("https://kwant.example.com:8443").unwrap(),
            "https://kwant.example.com:8443/api/mcp/oauth/client.json"
        );
        // Plain http on a LAN or public name: no document can name it
        for no in ["http://192.168.1.5:8799", "http://kwant.example.com", "not a url"] {
            assert!(matches!(client_document_id(no), Err(OAuthError::CimdNeedsHttps)), "{no}");
        }
    }

    #[test]
    fn self_hosted_document_names_only_its_own_callback() {
        let doc = self_hosted_client_document("https://kwant.example.com").unwrap();
        assert_eq!(doc["client_id"], "https://kwant.example.com/api/mcp/oauth/client.json");
        assert_eq!(doc["redirect_uris"], json!(["https://kwant.example.com/api/mcp/oauth/callback"]));
        assert_eq!(doc["token_endpoint_auth_method"], "none");
        assert_eq!(doc["grant_types"], json!(["authorization_code", "refresh_token"]));
        assert_eq!(doc["response_types"], json!(["code"]));
        assert_eq!(doc["client_name"], "kwant");
        assert_eq!(doc["application_type"], "web");
        for no in ["http://kwant.example.com", "https://localhost:8443", "https://127.0.0.1", "https://k.dev/x", "nope"] {
            assert!(self_hosted_client_document(no).is_none(), "{no}");
        }
    }

    /// site/oauth/client.json is what kwant.si serves as STATIC_CLIENT_ID; it must say what the
    /// code expects (loopback callbacks, with and without the default port).
    #[test]
    fn static_document_matches_the_site_file() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("site/oauth/client.json");
        let file: Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        let doc = static_client_document();
        assert_eq!(file, doc, "update site/oauth/client.json to match static_client_document()");
        assert_eq!(doc["client_id"], STATIC_CLIENT_ID);
        assert_eq!(doc["application_type"], "native");
        let uris: Vec<&str> = doc["redirect_uris"].as_array().unwrap().iter().filter_map(Value::as_str).collect();
        assert!(uris.contains(&"http://127.0.0.1:8799/api/mcp/oauth/callback"));
        assert!(uris.contains(&"http://127.0.0.1/api/mcp/oauth/callback"));
        assert!(uris.iter().all(|u| {
            let url = Url::parse(u).unwrap();
            url.scheme() == "http" && is_loopback_url(&url) && url.path() == CALLBACK_PATH
        }));
    }

    #[test]
    fn dead_end_errors_have_codes() {
        assert_eq!(OAuthError::NoRegistration.code(), Some("no_client_registration"));
        assert_eq!(OAuthError::CimdNeedsHttps.code(), Some("cimd_needs_https"));
        assert_eq!(OAuthError::Registration("x".into()).code(), None);
        assert!(!OAuthError::NoRegistration.to_string().contains("automatic client registration"));
    }
}
