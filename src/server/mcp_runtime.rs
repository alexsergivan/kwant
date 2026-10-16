//! MCP at turn time: the broker-backed approver, per-turn context setup, and the small MCP
//! endpoint the claude CLI calls (`--permission-prompt-tool`) so its MCP tool calls get the same
//! approval cards as API agents. Agent CLIs with no permission hook (codex) instead reach the
//! kwant's servers through kwnt's MCP gateway, which asks for approval before forwarding each call.

use super::AppState;
use crate::broker::{ApprovalDecision, ApprovalRequest, PermissionBroker};
use crate::db::models::{Kwant, NewApproval};
use crate::db::Database;
use crate::drivers::events::RuntimeEvent;
use crate::mcp::{
    self,
    client::PROTOCOL_VERSION,
    session::{Caller, McpToolSession},
    GatewayAccess, McpTurnContext, ToolApprovalRequest, ToolApprover,
};
use async_trait::async_trait;
use axum::{
    extract::{Path, State},
    http::{header::AUTHORIZATION, HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    Json,
};
use serde_json::{json, Value};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{broadcast, Mutex};

/// How long a tool call waits for the user before it is denied.
pub const APPROVAL_TIMEOUT: Duration = Duration::from_secs(mcp::APPROVAL_TIMEOUT_SECS);
/// The tool claude calls for permission decisions: `mcp__kwnt__approve`.
pub use crate::mcp::PERMISSION_TOOL;

/// An agent CLI turn whose permission prompts (claude) or gateway calls (codex) kwnt answers.
#[derive(Clone)]
pub struct PermissionSession {
    pub kwant_id: String,
    pub kwant_name: String,
    pub turn_id: String,
    pub ctx: McpTurnContext,
    /// Working directory for the kwant's stdio servers when the gateway starts them.
    pub cwd: Option<PathBuf>,
    /// The gateway's connections to the kwant's servers, opened on first use.
    gateway: Arc<Mutex<Option<Arc<McpToolSession>>>>,
}

impl PermissionSession {
    pub fn new(kwant_id: String, kwant_name: String, turn_id: String, ctx: McpTurnContext, cwd: Option<PathBuf>) -> Self {
        Self { kwant_id, kwant_name, turn_id, ctx, cwd, gateway: Default::default() }
    }

    fn caller(&self) -> Caller {
        Caller { kwant_id: self.kwant_id.clone(), kwant_name: self.kwant_name.clone(), turn_id: self.turn_id.clone() }
    }

    /// Connects to the kwant's servers once per turn; later requests reuse the connections.
    async fn tool_session(&self) -> Arc<McpToolSession> {
        let mut slot = self.gateway.lock().await;
        if let Some(open) = slot.as_ref() {
            return open.clone();
        }
        let cwd = self.cwd.as_deref().filter(|d| d.exists());
        let (session, problems) = McpToolSession::open(&self.ctx, cwd).await;
        for problem in problems {
            tracing::warn!(kwant = %self.kwant_name, %problem, "MCP gateway could not reach a server");
        }
        let session = Arc::new(session);
        *slot = Some(session.clone());
        session
    }

    /// Closes the gateway's connections (stdio servers are stopped).
    async fn close(&self) {
        let Some(open) = self.gateway.lock().await.take() else { return };
        // A call still in flight keeps its reference; its servers stop when that call drops it
        if let Ok(session) = Arc::try_unwrap(open) {
            session.shutdown().await;
        }
    }
}

/// Asks the user through the permission broker: stores the ticket, emits `ApprovalRequired`
/// and waits for `/api/approvals/:id/resolve`, a timeout, or the turn being dropped.
pub struct BrokerApprover {
    pub db: Database,
    pub broker: PermissionBroker,
    pub events: broadcast::Sender<RuntimeEvent>,
    pub timeout: Duration,
}

/// Withdraws the ticket if the waiting turn is cancelled (its future dropped) mid-wait.
struct TicketGuard {
    id: Option<String>,
    broker: PermissionBroker,
    db: Database,
}

impl Drop for TicketGuard {
    fn drop(&mut self) {
        let (Some(id), Ok(handle)) = (self.id.take(), tokio::runtime::Handle::try_current()) else {
            return;
        };
        let (broker, db) = (self.broker.clone(), self.db.clone());
        handle.spawn(async move {
            if broker.cancel(&id).await {
                let _ = db.update_approval_status(&id, "cancelled").await;
            }
        });
    }
}

#[async_trait]
impl ToolApprover for BrokerApprover {
    async fn approve(&self, req: ToolApprovalRequest) -> bool {
        if self.broker.is_allowed_for_session(&req.kwant_id, &req.tool_name).await {
            return true;
        }
        let details = json!({
            "source": "mcp",
            "server": req.server,
            "tool": req.tool,
            "arguments": req.arguments,
            "kwant_id": req.kwant_id,
            "kwant_name": req.kwant_name,
        });
        let ticket = match self
            .db
            .save_approval(&NewApproval {
                turn_id: req.turn_id.clone(),
                kwant_id: req.kwant_id.clone(),
                tool_name: req.tool_name.clone(),
                parameters: details.clone(),
            })
            .await
        {
            Ok(a) => a.id,
            Err(e) => {
                tracing::warn!(error = %e, "could not store MCP approval ticket; denying");
                return false;
            }
        };
        let rx = self
            .broker
            .request_approval(ApprovalRequest {
                id: ticket.clone(),
                kwant_id: req.kwant_id.clone(),
                tool_name: req.tool_name.clone(),
                details: details.clone(),
            })
            .await;
        let mut guard = TicketGuard { id: Some(ticket.clone()), broker: self.broker.clone(), db: self.db.clone() };
        let _ = self.events.send(RuntimeEvent::ApprovalRequired {
            id: ticket.clone(),
            tool_name: req.tool_name.clone(),
            details,
        });

        let decision = tokio::time::timeout(self.timeout, rx).await;
        guard.id = None;
        match decision {
            Ok(Ok(ApprovalDecision::AllowOnce)) => true,
            Ok(Ok(ApprovalDecision::AlwaysAllowSession)) => {
                self.broker.allow_for_session(&req.kwant_id, &req.tool_name).await;
                true
            }
            Ok(Ok(ApprovalDecision::Deny)) | Ok(Err(_)) => false,
            Err(_) => {
                self.broker.cancel(&ticket).await;
                let _ = self.db.update_approval_status(&ticket, "expired").await;
                false
            }
        }
    }
}

fn max_tool_iterations(setting: Option<String>) -> usize {
    setting
        .or_else(|| std::env::var("KWNT_MCP_MAX_TOOL_ITERATIONS").ok())
        .and_then(|v| v.trim().parse::<usize>().ok())
        .unwrap_or(mcp::DEFAULT_MAX_TOOL_ITERATIONS)
        .clamp(1, 50)
}

/// Address agent CLIs on this host use to reach kwnt.
fn local_base_url(state: &AppState) -> String {
    let host = match state.config.host.as_str() {
        "0.0.0.0" | "::" | "[::]" | "" | "localhost" => "127.0.0.1".to_string(),
        h if h.contains(':') && !h.starts_with('[') => format!("[{h}]"),
        h => h.to_string(),
    };
    format!("http://{host}:{}", state.config.port)
}

/// Builds the turn's MCP context when the kwant has enabled servers, and registers the
/// session claude's permission prompts and codex's gateway calls reach. Returns `None` (no
/// behaviour change) otherwise. `cwd` is where the gateway starts stdio servers. Pair with
/// [`finish_turn`].
pub async fn prepare_turn(state: &AppState, kwant: &Kwant, cwd: Option<PathBuf>) -> Option<(McpTurnContext, String)> {
    let mut servers = match state.db.list_active_mcp_servers_for_kwant(&kwant.id).await {
        Ok(s) if !s.is_empty() => s,
        Ok(_) => return None,
        Err(e) => {
            tracing::warn!(error = %e, "could not load MCP servers for kwant");
            return None;
        }
    };
    // Signed-in http servers get their (refreshed) access token as a secret header
    super::mcp_oauth::authorize_servers(state, &mut servers).await;
    let token = uuid::Uuid::new_v4().simple().to_string();
    let setting = state.db.get_setting("mcp_max_tool_iterations").await.ok().flatten();
    let ctx = McpTurnContext {
        servers,
        approver: Arc::new(BrokerApprover {
            db: state.db.clone(),
            broker: state.broker.clone(),
            events: state.event_sender.clone(),
            timeout: APPROVAL_TIMEOUT,
        }),
        config_dir: state.config.data_dir.join("mcp"),
        permission_url: Some(format!("{}/api/mcp/permission/{token}", local_base_url(state))),
        gateway: Some(GatewayAccess { url: format!("{}/api/mcp/gateway", local_base_url(state)), token: token.clone() }),
        max_tool_iterations: max_tool_iterations(setting),
        records: Default::default(),
    };
    let session = PermissionSession::new(kwant.id.clone(), kwant.name.clone(), uuid::Uuid::new_v4().to_string(), ctx.clone(), cwd);
    state.mcp_sessions.lock().await.insert(token.clone(), session);
    Some((ctx, token))
}

pub async fn finish_turn(state: &AppState, token: &str) {
    let session = state.mcp_sessions.lock().await.remove(token);
    if let Some(session) = session {
        session.close().await;
    }
}

fn rpc_result(id: Value, result: Value) -> Response {
    Json(json!({"jsonrpc": "2.0", "id": id, "result": result})).into_response()
}

fn rpc_error(id: Value, code: i64, message: &str) -> Response {
    Json(json!({"jsonrpc": "2.0", "id": id, "error": {"code": code, "message": message}})).into_response()
}

/// Answers one claude permission prompt. Only MCP tools from the kwant's own servers can be
/// approved here; anything else is denied, as it would be in `-p` mode without this handler.
async fn decide(session: &PermissionSession, args: &Value) -> Value {
    let tool_name = args["tool_name"].as_str().unwrap_or("");
    let input = args.get("input").cloned().unwrap_or_else(|| json!({}));
    let tool_use_id = args["tool_use_id"].as_str().unwrap_or("").to_string();

    let target = mcp::split_tool_name(tool_name)
        .filter(|(server, _)| session.ctx.servers.iter().any(|s| s.name == *server));
    let Some((server, tool)) = target else {
        return json!({"behavior": "deny", "message": format!("{tool_name} is not available to this kwant.")});
    };
    let allowed = session
        .ctx
        .approver
        .approve(ToolApprovalRequest {
            kwant_id: session.kwant_id.clone(),
            kwant_name: session.kwant_name.clone(),
            turn_id: session.turn_id.clone(),
            tool_name: tool_name.to_string(),
            server: server.to_string(),
            tool: tool.to_string(),
            arguments: input.clone(),
        })
        .await;
    session.ctx.record(json!({
        "id": tool_use_id,
        "name": tool_name,
        "server": server,
        "tool": tool,
        "arguments": input,
        "status": if allowed { "allowed" } else { "denied" },
    }));
    if allowed {
        json!({"behavior": "allow", "updatedInput": input})
    } else {
        json!({"behavior": "deny", "message": "The user denied this tool call."})
    }
}

/// `POST /api/mcp/permission/:token`: a minimal streamable-HTTP MCP server (JSON responses).
pub async fn permission_endpoint(
    State(state): State<AppState>,
    Path(token): Path<String>,
    Json(msg): Json<Value>,
) -> Response {
    let session = state.mcp_sessions.lock().await.get(&token).cloned();
    let Some(session) = session else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let Some(id) = msg.get("id").cloned() else {
        return StatusCode::ACCEPTED.into_response(); // notification
    };
    match msg["method"].as_str().unwrap_or("") {
        "initialize" => rpc_result(
            id,
            json!({
                "protocolVersion": msg["params"]["protocolVersion"].as_str().unwrap_or(PROTOCOL_VERSION),
                "capabilities": {"tools": {}},
                "serverInfo": {"name": mcp::RESERVED_SERVER_NAME, "version": env!("CARGO_PKG_VERSION")},
            }),
        ),
        "ping" => rpc_result(id, json!({})),
        "tools/list" => rpc_result(
            id,
            json!({"tools": [{
                "name": PERMISSION_TOOL,
                "description": "kwnt's permission prompt handler. Not for direct use.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "tool_name": {"type": "string"},
                        "input": {"type": "object"},
                        "tool_use_id": {"type": "string"},
                    },
                    "required": ["tool_name", "input"],
                },
            }]}),
        ),
        "tools/call" if msg["params"]["name"] == PERMISSION_TOOL => {
            let decision = decide(&session, &msg["params"]["arguments"]).await;
            rpc_result(id, json!({"content": [{"type": "text", "text": decision.to_string()}]}))
        }
        "tools/call" => rpc_error(id, -32602, "unknown tool"),
        _ => rpc_error(id, -32601, "method not found"),
    }
}

fn bearer_token(headers: &HeaderMap) -> Option<&str> {
    let value = headers.get(AUTHORIZATION)?.to_str().ok()?;
    let (scheme, token) = value.split_once(' ')?;
    scheme.eq_ignore_ascii_case("bearer").then(|| token.trim()).filter(|t| !t.is_empty())
}

/// `POST /api/mcp/gateway`: a streamable-HTTP MCP server (JSON responses) that offers the
/// kwant's servers' tools to an agent CLI. The turn is picked by its bearer token, so the token
/// stays out of URLs and argv. Every call is approved through the turn's approver (the
/// approval card) before it is forwarded, and recorded like any other MCP call.
pub async fn gateway_endpoint(State(state): State<AppState>, headers: HeaderMap, Json(msg): Json<Value>) -> Response {
    let session = match bearer_token(&headers) {
        Some(token) => state.mcp_sessions.lock().await.get(token).cloned(),
        None => None,
    };
    let Some(session) = session else {
        return StatusCode::UNAUTHORIZED.into_response();
    };
    let Some(id) = msg.get("id").cloned() else {
        return StatusCode::ACCEPTED.into_response(); // notification
    };
    match msg["method"].as_str().unwrap_or("") {
        "initialize" => rpc_result(
            id,
            json!({
                "protocolVersion": msg["params"]["protocolVersion"].as_str().unwrap_or(PROTOCOL_VERSION),
                "capabilities": {"tools": {}},
                "serverInfo": {"name": mcp::RESERVED_SERVER_NAME, "version": env!("CARGO_PKG_VERSION")},
            }),
        ),
        "ping" => rpc_result(id, json!({})),
        "tools/list" => rpc_result(id, json!({"tools": session.tool_session().await.gateway_tools()})),
        "tools/call" => {
            let Some(name) = msg["params"]["name"].as_str() else {
                return rpc_error(id, -32602, "missing tool name");
            };
            let arguments = msg["params"].get("arguments").cloned().unwrap_or_else(|| json!({}));
            let call_id = format!("gw_{}", uuid::Uuid::new_v4().simple());
            let tools = session.tool_session().await;
            let outcome = tools.call(&session.caller(), &call_id, &format!("mcp__{name}"), &arguments.to_string()).await;
            rpc_result(
                id,
                json!({"content": [{"type": "text", "text": outcome.text}], "isError": outcome.status != "ok"}),
            )
        }
        _ => rpc_error(id, -32601, "method not found"),
    }
}
