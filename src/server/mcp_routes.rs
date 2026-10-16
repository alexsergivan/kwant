//! MCP server registry and per-kwant assignment endpoints. Contract: docs/mcp-api.md.

use super::{mcp_oauth::{self, TokenUse}, AppState};
use crate::db::{models::McpServer, DbError};
use crate::mcp::{self, McpClient, McpError, McpServerInput, McpServerView};
use axum::{
    extract::{Path, State},
    http::StatusCode,
    Json,
};
use serde::Deserialize;
use serde_json::{json, Value};
use std::time::Instant;

type ApiError = (StatusCode, Json<Value>);
type ApiResult<T> = Result<Json<T>, ApiError>;

fn err(status: StatusCode, message: impl Into<String>) -> ApiError {
    (status, Json(json!({ "error": message.into() })))
}

fn db_err(e: DbError) -> ApiError {
    if let DbError::Sqlx(sqlx::Error::Database(ref d)) = e {
        if d.is_unique_violation() {
            return err(StatusCode::CONFLICT, "an MCP server with that name already exists");
        }
    }
    tracing::warn!(error = %e, "MCP registry database error");
    err(StatusCode::INTERNAL_SERVER_ERROR, "database error")
}

/// The API view of `server`, with its sign-in status.
async fn view(state: &AppState, server: &McpServer) -> McpServerView {
    let mut view = McpServerView::from(server);
    view.oauth = mcp_oauth::statuses(state).await.remove(&server.id).unwrap_or_default();
    view
}

pub async fn list_servers(State(state): State<AppState>) -> ApiResult<Vec<McpServerView>> {
    let servers = state.db.list_mcp_servers().await.map_err(db_err)?;
    let mut statuses = mcp_oauth::statuses(&state).await;
    Ok(Json(
        servers
            .iter()
            .map(|s| McpServerView { oauth: statuses.remove(&s.id).unwrap_or_default(), ..McpServerView::from(s) })
            .collect(),
    ))
}

pub async fn get_server(State(state): State<AppState>, Path(id): Path<String>) -> ApiResult<McpServerView> {
    match state.db.get_mcp_server(&id).await.map_err(db_err)? {
        Some(s) => Ok(Json(view(&state, &s).await)),
        None => Err(err(StatusCode::NOT_FOUND, "MCP server not found")),
    }
}

pub async fn create_server(State(state): State<AppState>, Json(input): Json<McpServerInput>) -> ApiResult<McpServerView> {
    let server = mcp::build_server(input, None).map_err(|m| err(StatusCode::BAD_REQUEST, m))?;
    let created = state.db.create_mcp_server(&server).await.map_err(db_err)?;
    Ok(Json(McpServerView::from(&created)))
}

pub async fn update_server(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(input): Json<McpServerInput>,
) -> ApiResult<McpServerView> {
    let existing = state
        .db
        .get_mcp_server(&id)
        .await
        .map_err(db_err)?
        .ok_or_else(|| err(StatusCode::NOT_FOUND, "MCP server not found"))?;
    let server = mcp::build_server(input, Some(&existing)).map_err(|m| err(StatusCode::BAD_REQUEST, m))?;
    let updated = state.db.update_mcp_server(&server).await.map_err(db_err)?;
    if updated.is_some() && (server.url != existing.url || server.transport != existing.transport) {
        // Tokens and registrations belong to the old address
        state.db.clear_mcp_oauth(&id).await.map_err(db_err)?;
    }
    match updated {
        Some(s) => Ok(Json(view(&state, &s).await)),
        None => Err(err(StatusCode::NOT_FOUND, "MCP server not found")),
    }
}

pub async fn delete_server(State(state): State<AppState>, Path(id): Path<String>) -> ApiResult<Value> {
    if state.db.delete_mcp_server(&id).await.map_err(db_err)? {
        Ok(Json(json!({ "status": "deleted" })))
    } else {
        Err(err(StatusCode::NOT_FOUND, "MCP server not found"))
    }
}

/// Connects once and lists the tools.
async fn try_list(server: &McpServer, cwd: &std::path::Path) -> Result<(Value, Vec<mcp::McpTool>), (McpError, String)> {
    match McpClient::connect(server, Some(cwd)).await {
        Ok(mut client) => {
            let tools = client.list_tools().await;
            let stderr_tail = client.stderr_tail();
            client.shutdown().await;
            tools.map(|t| (client.server_info.clone(), t)).map_err(|e| (e, stderr_tail))
        }
        Err(e) => {
            let stderr_tail = match e {
                McpError::Startup { ref stderr_tail, .. } => stderr_tail.clone(),
                _ => String::new(),
            };
            Err((e, stderr_tail))
        }
    }
}

/// Connects with the stored config (and sign-in), lists the tools and disconnects. Errors are
/// redacted. A 401 without a usable sign-in answers `auth_required: true`.
pub async fn test_server(State(state): State<AppState>, Path(id): Path<String>) -> ApiResult<Value> {
    let stored = state
        .db
        .get_mcp_server(&id)
        .await
        .map_err(db_err)?
        .ok_or_else(|| err(StatusCode::NOT_FOUND, "MCP server not found"))?;
    let start = Instant::now();
    let cwd = state.config.data_dir.join("workspace");

    let mut server = stored.clone();
    let token_use = mcp_oauth::authorize_server(&state, &mut server).await;
    let mut secrets = server.secret_values();
    let mut result = try_list(&server, &cwd).await;
    if token_use == TokenUse::Injected && matches!(result, Err((McpError::Unauthorized { .. }, _))) {
        // The token was rejected early (revoked, or the server's clock): refresh once
        if let Some(retry) = mcp_oauth::retry_after_unauthorized(&state, &stored).await {
            secrets.extend(retry.secret_values());
            result = try_list(&retry, &cwd).await;
        }
    }

    let mut outcome = match result {
        Ok((server_info, tools)) => json!({ "ok": true, "server_info": server_info, "tools": tools }),
        Err((McpError::Unauthorized { .. }, _)) if token_use != TokenUse::UserHeader => {
            if token_use == TokenUse::Injected {
                // Still rejected after a refresh: the sign-in is no longer valid
                let _ = state.db.delete_mcp_oauth_token(&id).await;
            }
            json!({ "ok": false, "auth_required": true, "error": "Sign-in required" })
        }
        Err((e, stderr_tail)) => json!({ "ok": false, "error": mcp::redact(&e.to_string(), &secrets),
                                         "stderr_tail": mcp::redact(&stderr_tail, &secrets) }),
    };
    outcome["duration_ms"] = json!(start.elapsed().as_millis() as u64);
    Ok(Json(outcome))
}

pub async fn get_kwant_servers(State(state): State<AppState>, Path(kwant_id): Path<String>) -> ApiResult<Value> {
    if state.db.get_kwant(&kwant_id).await.map_err(db_err)?.is_none() {
        return Err(err(StatusCode::NOT_FOUND, "kwant not found"));
    }
    let ids = state.db.get_kwant_mcp_server_ids(&kwant_id).await.map_err(db_err)?;
    Ok(Json(json!({ "server_ids": ids })))
}

#[derive(Debug, Deserialize)]
pub struct AssignServers {
    pub server_ids: Vec<String>,
}

pub async fn set_kwant_servers(
    State(state): State<AppState>,
    Path(kwant_id): Path<String>,
    Json(body): Json<AssignServers>,
) -> ApiResult<Value> {
    if state.db.get_kwant(&kwant_id).await.map_err(db_err)?.is_none() {
        return Err(err(StatusCode::NOT_FOUND, "kwant not found"));
    }
    let known: Vec<String> = state.db.list_mcp_servers().await.map_err(db_err)?.into_iter().map(|s| s.id).collect();
    if let Some(unknown) = body.server_ids.iter().find(|id| !known.contains(id)) {
        return Err(err(StatusCode::BAD_REQUEST, format!("unknown MCP server id '{unknown}'")));
    }
    state.db.set_kwant_mcp_servers(&kwant_id, &body.server_ids).await.map_err(db_err)?;
    let ids = state.db.get_kwant_mcp_server_ids(&kwant_id).await.map_err(db_err)?;
    Ok(Json(json!({ "server_ids": ids })))
}
