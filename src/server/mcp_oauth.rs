//! OAuth sign-in for http MCP servers: the start/callback/disconnect endpoints, and the token
//! handling every connection goes through (refresh before expiry, inject `Authorization`).
//! Contract: docs/mcp-api.md.

use super::AppState;
use crate::db::models::{McpOAuthClient, McpOAuthToken, McpServer};
use crate::db::DbError;
use crate::mcp::oauth::{self, ClientAuth, OAuthError, OAuthStatus, PendingStore};
use axum::{
    extract::{Path, Query, State},
    http::{header, HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    Json,
};
use chrono::Utc;
use serde::Deserialize;
use serde_json::{json, Value};

/// Process-wide sign-in state: open sign-ins and the lock that serializes refreshes, so two
/// turns never spend the same (rotating) refresh token.
pub struct OAuthState {
    pub pending: PendingStore,
    pub http: reqwest::Client,
    refresh_lock: tokio::sync::Mutex<()>,
}

impl Default for OAuthState {
    fn default() -> Self {
        Self { pending: PendingStore::default(), http: oauth::http_client(), refresh_lock: tokio::sync::Mutex::new(()) }
    }
}

/// How a server's `Authorization` header was decided for one connection.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TokenUse {
    /// stdio, or http without a stored token.
    None,
    /// The user configured their own Authorization header; kwnt's token is not used.
    UserHeader,
    /// kwnt added `Authorization: Bearer <stored token>`.
    Injected,
}

type ApiError = (StatusCode, Json<Value>);

fn err(status: StatusCode, message: impl Into<String>) -> ApiError {
    (status, Json(json!({ "error": message.into() })))
}

fn db_err(e: DbError) -> ApiError {
    tracing::warn!(error = %e, "MCP OAuth database error");
    err(StatusCode::INTERNAL_SERVER_ERROR, "database error")
}

async fn refresh_stored(state: &AppState, server_id: &str, force: bool) -> Result<Option<McpOAuthToken>, DbError> {
    let _guard = state.mcp_oauth.refresh_lock.lock().await;
    // Re-read under the lock: another turn may have refreshed already
    let Some(token) = state.db.get_mcp_oauth_token(server_id).await? else {
        return Ok(None);
    };
    let now = Utc::now();
    if !force && !oauth::needs_refresh(token.expires_at, now) {
        return Ok(Some(token));
    }
    let expired = token.expires_at.is_some_and(|t| t <= now);
    let Some(refresh_token) = token.refresh_token.clone() else {
        if force || expired {
            state.db.delete_mcp_oauth_token(server_id).await?;
            return Ok(None);
        }
        return Ok(Some(token));
    };
    let client = ClientAuth {
        client_id: &token.client_id,
        client_secret: token.client_secret.as_deref(),
        auth_method: &token.auth_method,
    };
    match oauth::refresh(&state.mcp_oauth.http, &token.token_endpoint, &client, &refresh_token, &token.resource).await {
        Ok(set) => {
            let updated = McpOAuthToken {
                access_token: set.access_token,
                refresh_token: set.refresh_token,
                expires_at: set.expires_at,
                ..token
            };
            state.db.save_mcp_oauth_token(&updated).await?;
            Ok(Some(updated))
        }
        // The authorization server said no: the grant is gone, so the user has to sign in again
        Err(e @ OAuthError::Token(_)) => {
            tracing::info!(server_id, error = %e, "MCP OAuth refresh rejected; sign-in needed");
            state.db.delete_mcp_oauth_token(server_id).await?;
            Ok(None)
        }
        // Network trouble: keep the grant, and use the current token while it is still valid
        Err(e) => {
            tracing::warn!(server_id, error = %e, "MCP OAuth refresh failed");
            Ok((!expired && !force).then_some(token))
        }
    }
}

/// Adds the stored access token to an http server's headers, refreshing it first if it
/// expires within a minute. A user-set Authorization header wins.
pub async fn authorize_server(state: &AppState, server: &mut McpServer) -> TokenUse {
    if server.transport != "http" {
        return TokenUse::None;
    }
    if oauth::has_user_authorization(&server.headers) {
        return TokenUse::UserHeader;
    }
    match refresh_stored(state, &server.id, false).await {
        Ok(Some(token)) if oauth::apply_bearer(&mut server.headers, &token.access_token) => TokenUse::Injected,
        Ok(_) => TokenUse::None,
        Err(e) => {
            tracing::warn!(error = %e, "could not load MCP OAuth token");
            TokenUse::None
        }
    }
}

/// [`authorize_server`] for every server of a turn.
pub async fn authorize_servers(state: &AppState, servers: &mut [McpServer]) {
    for server in servers {
        authorize_server(state, server).await;
    }
}

/// After a 401 with kwnt's token: refresh once and return the server with the new token, or
/// drop the token (the server then needs sign-in) and return `None`.
pub async fn retry_after_unauthorized(state: &AppState, original: &McpServer) -> Option<McpServer> {
    match refresh_stored(state, &original.id, true).await {
        Ok(Some(token)) => {
            let mut server = original.clone();
            oauth::apply_bearer(&mut server.headers, &token.access_token);
            Some(server)
        }
        Ok(None) => None,
        Err(e) => {
            tracing::warn!(error = %e, "could not refresh MCP OAuth token");
            None
        }
    }
}

/// Sign-in status for every server, for list/get responses.
pub async fn statuses(state: &AppState) -> std::collections::HashMap<String, OAuthStatus> {
    state
        .db
        .list_mcp_oauth_expiries()
        .await
        .unwrap_or_default()
        .into_iter()
        .map(|(id, expires_at)| (id, OAuthStatus { connected: true, expires_at }))
        .collect()
}

#[derive(Debug, Deserialize)]
pub struct StartBody {
    pub origin: String,
}

/// Hosts the request was addressed to: `Host` and every `X-Forwarded-Host` value.
fn request_hosts(headers: &HeaderMap) -> Vec<String> {
    let mut hosts = Vec::new();
    for name in [header::HOST.as_str(), "x-forwarded-host"] {
        for value in headers.get_all(name) {
            if let Ok(v) = value.to_str() {
                hosts.extend(v.split(',').map(|h| h.trim().to_string()).filter(|h| !h.is_empty()));
            }
        }
    }
    hosts
}

/// `POST /api/mcp/servers/:id/oauth/start` `{origin}` → `{authorize_url}`.
pub async fn start(
    State(state): State<AppState>,
    Path(id): Path<String>,
    headers: HeaderMap,
    Json(body): Json<StartBody>,
) -> Result<Json<Value>, ApiError> {
    let server = state
        .db
        .get_mcp_server(&id)
        .await
        .map_err(db_err)?
        .ok_or_else(|| err(StatusCode::NOT_FOUND, "MCP server not found"))?;
    let Some(server_url) = server.url.clone().filter(|_| server.transport == "http") else {
        return Err(err(StatusCode::BAD_REQUEST, "sign-in only applies to http servers"));
    };
    let hosts = request_hosts(&headers);
    let host_refs: Vec<&str> = hosts.iter().map(String::as_str).collect();
    let origin = oauth::validate_origin(&body.origin, &host_refs).map_err(|m| err(StatusCode::BAD_REQUEST, m))?;
    let redirect_uri = oauth::callback_url(&origin);
    let http = &state.mcp_oauth.http;
    let upstream = |e: OAuthError| err(StatusCode::BAD_GATEWAY, e.to_string());

    let challenge = oauth::probe_challenge(http, &server_url).await;
    let discovery = oauth::discover(http, &server_url, challenge.as_ref()).await.map_err(upstream)?;
    let client = match state.db.get_mcp_oauth_client(&id, &redirect_uri, &discovery.issuer).await.map_err(db_err)? {
        Some(c) => oauth::ClientRegistration {
            client_id: c.client_id,
            client_secret: c.client_secret,
            auth_method: c.auth_method,
        },
        None => {
            let reg = oauth::register_client(http, &discovery.metadata, &redirect_uri).await.map_err(upstream)?;
            state
                .db
                .save_mcp_oauth_client(&McpOAuthClient {
                    server_id: id.clone(),
                    redirect_uri: redirect_uri.clone(),
                    issuer: discovery.issuer.clone(),
                    client_id: reg.client_id.clone(),
                    client_secret: reg.client_secret.clone(),
                    auth_method: reg.auth_method.clone(),
                })
                .await
                .map_err(db_err)?;
            reg
        }
    };
    let pkce = oauth::Pkce::new();
    let pending = oauth::pending_auth(&id, &pkce, &redirect_uri, &discovery, &client);
    let state_param = state.mcp_oauth.pending.insert(pending);
    let authorize_url = oauth::authorize_url(&discovery, &client.client_id, &redirect_uri, &state_param, &pkce).map_err(upstream)?;
    Ok(Json(json!({ "authorize_url": authorize_url })))
}

#[derive(Debug, Deserialize)]
pub struct CallbackQuery {
    pub code: Option<String>,
    pub state: Option<String>,
    pub error: Option<String>,
    pub error_description: Option<String>,
}

fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;").replace('\'', "&#39;")
}

/// JSON that is safe inside a `<script>` element.
fn script_json(v: &Value) -> String {
    v.to_string()
        .replace('<', "\\u003c")
        .replace('>', "\\u003e")
        .replace('&', "\\u0026")
        .replace('\u{2028}', "\\u2028")
        .replace('\u{2029}', "\\u2029")
}

/// The page the sign-in popup lands on: tells the opener the outcome and closes, or goes to
/// the app when there is no opener.
pub fn callback_page(server_id: Option<&str>, result: Result<(), String>) -> Response {
    let (ok, error) = match &result {
        Ok(()) => (true, None),
        Err(e) => (false, Some(crate::mcp::truncate_text(e, 300))),
    };
    let mut message = json!({ "type": "kwnt-mcp-oauth", "server_id": server_id, "ok": ok });
    if let Some(e) = &error {
        message["error"] = json!(e);
    }
    let text = match &error {
        None => "Signed in. You can close this window.".to_string(),
        Some(e) => format!("Sign-in failed: {e}"),
    };
    let page = format!(
        "<!doctype html><html><head><meta charset=\"utf-8\"><title>kwnt sign-in</title></head>\
         <body><p>{}</p><script>(function(){{var m={};\
         if(window.opener){{try{{window.opener.postMessage(m,location.origin)}}catch(e){{}}window.close()}}\
         else{{location.replace(\"/\")}}}})();</script></body></html>",
        html_escape(&text),
        script_json(&message)
    );
    (
        StatusCode::OK,
        [
            (header::CONTENT_TYPE, "text/html; charset=utf-8"),
            (header::CACHE_CONTROL, "no-store"),
            (header::REFERRER_POLICY, "no-referrer"),
            (header::CONTENT_SECURITY_POLICY, "default-src 'none'; script-src 'unsafe-inline'"),
        ],
        page,
    )
        .into_response()
}

/// `GET /api/mcp/oauth/callback?code&state` (or `?error&state`).
pub async fn callback(State(state): State<AppState>, Query(q): Query<CallbackQuery>) -> Response {
    let Some(pending) = q.state.as_deref().and_then(|s| state.mcp_oauth.pending.take(s)) else {
        return callback_page(None, Err("this sign-in link has expired or was already used; try Connect again".into()));
    };
    let server_id = pending.server_id.clone();
    if let Some(code) = q.error.as_deref() {
        let detail = q.error_description.as_deref().map(|d| format!("{code}: {d}")).unwrap_or_else(|| code.to_string());
        return callback_page(Some(&server_id), Err(detail));
    }
    let Some(code) = q.code.as_deref().filter(|c| !c.is_empty()) else {
        return callback_page(Some(&server_id), Err("the sign-in response had no code".into()));
    };
    match state.db.get_mcp_server(&server_id).await {
        Ok(Some(_)) => {}
        Ok(None) => return callback_page(Some(&server_id), Err("this server was deleted".into())),
        Err(e) => {
            tracing::warn!(error = %e, "MCP OAuth callback database error");
            return callback_page(Some(&server_id), Err("database error".into()));
        }
    }
    let set = match oauth::exchange_code(&state.mcp_oauth.http, &pending, code).await {
        Ok(set) => set,
        Err(e) => return callback_page(Some(&server_id), Err(e.to_string())),
    };
    let token = McpOAuthToken {
        server_id: server_id.clone(),
        access_token: set.access_token,
        refresh_token: set.refresh_token,
        expires_at: set.expires_at,
        token_endpoint: pending.token_endpoint,
        client_id: pending.client_id,
        client_secret: pending.client_secret,
        auth_method: pending.auth_method,
        resource: pending.resource,
    };
    if let Err(e) = state.db.save_mcp_oauth_token(&token).await {
        tracing::warn!(error = %e, "could not store MCP OAuth token");
        return callback_page(Some(&server_id), Err("could not store the sign-in".into()));
    }
    callback_page(Some(&server_id), Ok(()))
}

/// `DELETE /api/mcp/servers/:id/oauth`: forgets the tokens (the registration is kept).
pub async fn disconnect(State(state): State<AppState>, Path(id): Path<String>) -> Result<Json<Value>, ApiError> {
    if state.db.get_mcp_server(&id).await.map_err(db_err)?.is_none() {
        return Err(err(StatusCode::NOT_FOUND, "MCP server not found"));
    }
    state.db.delete_mcp_oauth_token(&id).await.map_err(db_err)?;
    Ok(Json(json!({ "status": "disconnected" })))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn callback_page_escapes_everything() {
        let resp = callback_page(Some("id\"</script>"), Err("<img src=x onerror=alert(1)>\u{2028}".into()));
        let headers = resp.headers().clone();
        assert_eq!(headers[header::CACHE_CONTROL], "no-store");
        let body = futures::executor::block_on(axum::body::to_bytes(resp.into_body(), 1 << 16)).unwrap();
        let body = String::from_utf8(body.to_vec()).unwrap();
        assert!(!body.contains("<img"), "{body}");
        assert!(!body.contains("</script>\""), "{body}");
        assert_eq!(body.matches("</script>").count(), 1, "{body}");
        assert!(body.contains("&lt;img"));
        assert!(body.contains("\\u003cimg"));
        assert!(body.contains("postMessage(m,location.origin)"));
    }

    #[test]
    fn collects_request_hosts() {
        let mut h = HeaderMap::new();
        h.insert(header::HOST, "127.0.0.1:7777".parse().unwrap());
        h.insert("x-forwarded-host", "k.dev, proxy.internal".parse().unwrap());
        assert_eq!(request_hosts(&h), vec!["127.0.0.1:7777", "k.dev", "proxy.internal"]);
    }
}
