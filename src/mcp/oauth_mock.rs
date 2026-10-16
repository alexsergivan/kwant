//! A small OAuth-protected streamable-HTTP MCP server, run with the hidden
//! `kwnt mcp-oauth-mock-server` subcommand. It implements just enough of the MCP authorization
//! spec (protected-resource and authorization-server metadata, dynamic registration, PKCE S256,
//! `resource`, refresh with rotation) for tests and live checks of kwnt's sign-in without a
//! real provider.
//!
//! `/authorize` shows an "Allow" link instead of asking for credentials; its target is also sent
//! in the `X-Mock-Redirect` header so tests can follow it without a browser.

use axum::{
    extract::{Query, State},
    http::{HeaderMap, StatusCode},
    response::{Html, IntoResponse, Response},
    routing::{get, post},
    Form, Json, Router,
};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

#[derive(Debug, Clone)]
pub struct MockOptions {
    /// `expires_in` of issued access tokens. Below 60 makes kwnt refresh on every use.
    pub expires_in: i64,
}

impl Default for MockOptions {
    fn default() -> Self {
        Self { expires_in: 3600 }
    }
}

struct Grant {
    client_id: String,
    redirect_uri: String,
    challenge: String,
    resource: String,
}

#[derive(Default)]
struct Inner {
    clients: HashMap<String, Vec<String>>, // client_id -> redirect_uris
    codes: HashMap<String, Grant>,
    access: HashSet<String>,
    refresh: HashMap<String, String>, // refresh token -> client_id
}

pub struct MockState {
    pub base_url: String,
    pub options: MockOptions,
    counter: AtomicU64,
    pub registrations: AtomicU64,
    pub code_exchanges: AtomicU64,
    pub refreshes: AtomicU64,
    inner: Mutex<Inner>,
}

impl MockState {
    fn next(&self, prefix: &str) -> String {
        format!("{prefix}-{}-{}", self.counter.fetch_add(1, Ordering::SeqCst), uuid::Uuid::new_v4().simple())
    }

    /// Revokes every access token, as a provider would when the user removes the app.
    pub fn revoke_access_tokens(&self) {
        self.inner.lock().unwrap().access.clear();
    }

    /// Revokes every access and refresh token: the user removed the app at the provider.
    pub fn revoke_all(&self) {
        let mut inner = self.inner.lock().unwrap();
        inner.access.clear();
        inner.refresh.clear();
    }

    fn resource(&self) -> String {
        format!("{}/mcp", self.base_url)
    }
}

type Shared = Arc<MockState>;

fn oauth_error(code: &str, description: &str) -> Response {
    (StatusCode::BAD_REQUEST, Json(json!({"error": code, "error_description": description}))).into_response()
}

async fn resource_metadata(State(s): State<Shared>) -> Json<Value> {
    Json(json!({
        "resource": s.resource(),
        "authorization_servers": [s.base_url],
        "scopes_supported": ["mcp"],
        "bearer_methods_supported": ["header"],
    }))
}

async fn server_metadata(State(s): State<Shared>) -> Json<Value> {
    let b = &s.base_url;
    Json(json!({
        "issuer": b,
        "authorization_endpoint": format!("{b}/authorize"),
        "token_endpoint": format!("{b}/token"),
        "registration_endpoint": format!("{b}/register"),
        "response_types_supported": ["code"],
        "grant_types_supported": ["authorization_code", "refresh_token"],
        "code_challenge_methods_supported": ["S256"],
        "token_endpoint_auth_methods_supported": ["none"],
    }))
}

async fn register(State(s): State<Shared>, Json(body): Json<Value>) -> Response {
    let uris: Vec<String> = body["redirect_uris"]
        .as_array()
        .map(|a| a.iter().filter_map(|u| u.as_str().map(str::to_string)).collect())
        .unwrap_or_default();
    if uris.is_empty() {
        return oauth_error("invalid_redirect_uri", "redirect_uris is required");
    }
    s.registrations.fetch_add(1, Ordering::SeqCst);
    let client_id = s.next("client");
    s.inner.lock().unwrap().clients.insert(client_id.clone(), uris.clone());
    (StatusCode::CREATED, Json(json!({"client_id": client_id, "redirect_uris": uris, "token_endpoint_auth_method": "none"})))
        .into_response()
}

fn escape(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;").replace('\'', "&#39;")
}

async fn authorize(State(s): State<Shared>, Query(q): Query<HashMap<String, String>>) -> Response {
    let get = |k: &str| q.get(k).cloned().unwrap_or_default();
    let (client_id, redirect_uri, state) = (get("client_id"), get("redirect_uri"), get("state"));
    let known = s.inner.lock().unwrap().clients.get(&client_id).is_some_and(|u| u.contains(&redirect_uri));
    if !known {
        return (StatusCode::BAD_REQUEST, "unknown client or redirect_uri").into_response();
    }
    if get("response_type") != "code" || get("code_challenge_method") != "S256" || get("code_challenge").is_empty() {
        return (StatusCode::BAD_REQUEST, "PKCE S256 and response_type=code are required").into_response();
    }
    if get("resource") != s.resource() {
        return (StatusCode::BAD_REQUEST, "resource does not match this server").into_response();
    }
    let code = s.next("code");
    s.inner.lock().unwrap().codes.insert(
        code.clone(),
        Grant { client_id, redirect_uri: redirect_uri.clone(), challenge: get("code_challenge"), resource: get("resource") },
    );
    let mut target = url::Url::parse(&redirect_uri).expect("registered redirect_uri is a URL");
    target.query_pairs_mut().append_pair("code", &code).append_pair("state", &state);
    let mut denied = url::Url::parse(&redirect_uri).expect("registered redirect_uri is a URL");
    denied
        .query_pairs_mut()
        .append_pair("error", "access_denied")
        .append_pair("error_description", "The user said no")
        .append_pair("state", &state);
    let page = format!(
        "<!doctype html><title>Mock sign-in</title><h1>kwnt mock sign-in</h1>\
         <p><a id=\"allow\" href=\"{}\">Allow</a> · <a id=\"deny\" href=\"{}\">Deny</a></p>",
        escape(target.as_str()),
        escape(denied.as_str())
    );
    let mut resp = Html(page).into_response();
    if let Ok(v) = target.as_str().parse() {
        resp.headers_mut().insert("x-mock-redirect", v);
    }
    resp
}

async fn token(State(s): State<Shared>, Form(f): Form<HashMap<String, String>>) -> Response {
    let get = |k: &str| f.get(k).cloned().unwrap_or_default();
    let mut inner = s.inner.lock().unwrap();
    let client_id = match get("grant_type").as_str() {
        "authorization_code" => {
            let Some(grant) = inner.codes.remove(&get("code")) else {
                return oauth_error("invalid_grant", "unknown or used code");
            };
            let challenge = URL_SAFE_NO_PAD.encode(Sha256::digest(get("code_verifier").as_bytes()));
            if challenge != grant.challenge {
                return oauth_error("invalid_grant", "PKCE verification failed");
            }
            if grant.client_id != get("client_id") || grant.redirect_uri != get("redirect_uri") || grant.resource != get("resource") {
                return oauth_error("invalid_grant", "client, redirect_uri or resource mismatch");
            }
            s.code_exchanges.fetch_add(1, Ordering::SeqCst);
            grant.client_id
        }
        "refresh_token" => {
            // Rotation: the old refresh token stops working
            let Some(client_id) = inner.refresh.remove(&get("refresh_token")) else {
                return oauth_error("invalid_grant", "unknown refresh token");
            };
            if client_id != get("client_id") || get("resource") != s.resource() {
                return oauth_error("invalid_grant", "client or resource mismatch");
            }
            s.refreshes.fetch_add(1, Ordering::SeqCst);
            client_id
        }
        _ => return oauth_error("unsupported_grant_type", "only authorization_code and refresh_token"),
    };
    let access = s.next("at");
    let refresh = s.next("rt");
    inner.access.insert(access.clone());
    inner.refresh.insert(refresh.clone(), client_id);
    Json(json!({"access_token": access, "token_type": "Bearer", "expires_in": s.options.expires_in, "refresh_token": refresh}))
        .into_response()
}

async fn mcp(State(s): State<Shared>, headers: HeaderMap, Json(msg): Json<Value>) -> Response {
    let token = headers
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .unwrap_or("");
    if !s.inner.lock().unwrap().access.contains(token) {
        let challenge = format!(
            "Bearer resource_metadata=\"{}/.well-known/oauth-protected-resource/mcp\", scope=\"mcp\"",
            s.base_url
        );
        return (StatusCode::UNAUTHORIZED, [("www-authenticate", challenge)], Json(json!({"error": "invalid_token"})))
            .into_response();
    }
    let Some(id) = msg.get("id").cloned() else {
        return StatusCode::ACCEPTED.into_response();
    };
    let result = match msg["method"].as_str().unwrap_or("") {
        "initialize" => json!({
            "protocolVersion": super::client::PROTOCOL_VERSION,
            "capabilities": {"tools": {}},
            "serverInfo": {"name": "kwnt-oauth-mock", "version": env!("CARGO_PKG_VERSION")},
        }),
        "tools/list" => json!({"tools": [{"name": "whoami", "description": "Says who you are signed in as.",
                                           "inputSchema": {"type": "object", "properties": {}}}]}),
        "tools/call" => json!({"content": [{"type": "text", "text": "signed in to the mock"}], "isError": false}),
        _ => return Json(json!({"jsonrpc": "2.0", "id": id, "error": {"code": -32601, "message": "method not found"}})).into_response(),
    };
    Json(json!({"jsonrpc": "2.0", "id": id, "result": result})).into_response()
}

pub fn router(base_url: &str, options: MockOptions) -> (Router, Shared) {
    let state = Arc::new(MockState {
        base_url: base_url.trim_end_matches('/').to_string(),
        options,
        counter: AtomicU64::new(1),
        registrations: AtomicU64::new(0),
        code_exchanges: AtomicU64::new(0),
        refreshes: AtomicU64::new(0),
        inner: Mutex::default(),
    });
    let app = Router::new()
        .route("/.well-known/oauth-protected-resource", get(resource_metadata))
        .route("/.well-known/oauth-protected-resource/mcp", get(resource_metadata))
        .route("/.well-known/oauth-authorization-server", get(server_metadata))
        .route("/register", post(register))
        .route("/authorize", get(authorize))
        .route("/token", post(token))
        .route("/mcp", post(mcp).delete(|| async { StatusCode::OK }))
        .with_state(state.clone());
    (app, state)
}

/// Serves on `listener` until the process ends. The base URL is the listener's address.
pub async fn serve(listener: tokio::net::TcpListener, options: MockOptions) -> std::io::Result<()> {
    let base = format!("http://{}", listener.local_addr()?);
    let (app, _) = router(&base, options);
    axum::serve(listener, app).await
}
