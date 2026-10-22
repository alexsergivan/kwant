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
    session::{Caller, GatewayNaming, McpToolSession},
    ApprovalOutcome, GatewayAccess, McpTurnContext, ToolApprovalRequest, ToolApprover,
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
use tokio_util::sync::CancellationToken;

/// How long `finish_turn` waits for cancelled in-flight calls to release the turn's servers.
const CLOSE_GRACE: Duration = Duration::from_secs(10);

/// How long a tool call waits for the user before its turn stops waiting (the card stays open).
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
    /// Cancelled when the turn ends: calls still waiting (for a card) give up.
    ended: CancellationToken,
}

impl PermissionSession {
    pub fn new(kwant_id: String, kwant_name: String, turn_id: String, ctx: McpTurnContext, cwd: Option<PathBuf>) -> Self {
        Self { kwant_id, kwant_name, turn_id, ctx, cwd, gateway: Default::default(), ended: CancellationToken::new() }
    }

    fn caller(&self) -> Caller {
        Caller { kwant_id: self.kwant_id.clone(), kwant_name: self.kwant_name.clone(), turn_id: self.turn_id.clone() }
    }

    /// Connects to the kwant's servers once per turn; later requests reuse the connections.
    /// `None` once the turn has ended, so a late request can't start servers again.
    async fn tool_session(&self) -> Option<Arc<McpToolSession>> {
        let mut slot = self.gateway.lock().await;
        if self.ended.is_cancelled() {
            return None;
        }
        if let Some(open) = slot.as_ref() {
            return Some(open.clone());
        }
        let cwd = self.cwd.as_deref().filter(|d| d.exists());
        let (session, problems) = McpToolSession::open(&self.ctx, cwd).await;
        for problem in problems {
            tracing::warn!(kwant = %self.kwant_name, %problem, "MCP gateway could not reach a server");
        }
        let session = Arc::new(session);
        *slot = Some(session.clone());
        Some(session)
    }

    /// Ends the turn: calls still waiting give up (their cards are withdrawn), then the
    /// gateway's connections close (stdio servers are stopped).
    async fn close(&self) {
        self.ended.cancel();
        let Some(mut open) = self.gateway.lock().await.take() else { return };
        // Cancelled calls drop their reference as they unwind
        let deadline = tokio::time::Instant::now() + CLOSE_GRACE;
        loop {
            match Arc::try_unwrap(open) {
                Ok(session) => return session.shutdown().await,
                Err(still_shared) if tokio::time::Instant::now() < deadline => {
                    open = still_shared;
                    tokio::time::sleep(Duration::from_millis(20)).await;
                }
                // Its servers stop (kill_on_drop) when the last call drops it
                Err(_) => return tracing::warn!(kwant = %self.kwant_name, "MCP gateway call outlived its turn"),
            }
        }
    }
}

/// The record of a call the turn ended under, while it waited for its card.
fn cancelled_record(id: &str, tool_name: &str, server: &str, tool: &str, arguments: &Value) -> Value {
    json!({
        "id": id,
        "name": tool_name,
        "server": server,
        "tool": tool,
        "arguments": arguments,
        "status": "cancelled",
    })
}

/// Asks the user through the permission broker: stores the ticket, emits `ApprovalRequired`
/// and waits for `/api/approvals/:id/resolve`, a timeout, or the turn being dropped.
pub struct BrokerApprover {
    pub db: Database,
    pub broker: PermissionBroker,
    pub events: broadcast::Sender<RuntimeEvent>,
    pub timeout: Duration,
    /// The conversation "always allow" answers are kept for (see [`turn_conversation`]).
    pub conversation: String,
    /// The chat a card left unanswered resumes in after a late Allow (see [`turn_chat`]);
    /// `None` resumes in the bot's direct chat.
    pub chat: Option<Value>,
}

/// Withdraws the ticket if the waiting turn is cancelled (its future dropped) mid-wait.
struct TicketGuard {
    id: Option<String>,
    broker: PermissionBroker,
    db: Database,
    events: broadcast::Sender<RuntimeEvent>,
}

impl Drop for TicketGuard {
    fn drop(&mut self) {
        let (Some(id), Ok(handle)) = (self.id.take(), tokio::runtime::Handle::try_current()) else {
            return;
        };
        let (broker, db, events) = (self.broker.clone(), self.db.clone(), self.events.clone());
        handle.spawn(async move {
            if broker.cancel(&id).await {
                let _ = db.update_approval_status(&id, "cancelled").await;
                let _ = events.send(RuntimeEvent::ApprovalWithdrawn { id });
            }
        });
    }
}

#[async_trait]
impl ToolApprover for BrokerApprover {
    async fn approve(&self, req: ToolApprovalRequest) -> bool {
        self.decide(req).await.allowed()
    }

    async fn decide(&self, req: ToolApprovalRequest) -> ApprovalOutcome {
        // Approvals the person switched off: run, and keep a record of what ran. A setting that
        // can't be read asks, as before.
        let off = self.db.kwant_approvals(&req.kwant_id).await.is_ok_and(|m| m.is_off());
        let details = match &req.builtin {
            Some(b) => json!({
                "source": "claude_builtin",
                "tool": req.tool,
                "arguments": req.arguments,
                "cwd": b.cwd,
                "outside_folder": b.outside_folder,
                "kwant_id": req.kwant_id,
                "kwant_name": req.kwant_name,
            }),
            None => json!({
                "source": "mcp",
                "server": req.server,
                "tool": req.tool,
                "arguments": req.arguments,
                "kwant_id": req.kwant_id,
                "kwant_name": req.kwant_name,
            }),
        };
        let record = NewApproval {
            turn_id: req.turn_id.clone(),
            kwant_id: req.kwant_id.clone(),
            tool_name: req.tool_name.clone(),
            parameters: details.clone(),
        };
        // kwnt's own secrets (saved-password vault, its key, the API token) are never a bot's,
        // approvals on or off. Best effort: a same-user process can still find them another way.
        let cwd = req.builtin.as_ref().and_then(|b| b.cwd.as_deref()).map(std::path::Path::new);
        if crate::sandbox::credentials::call_touches_kwnt_secrets(&req.arguments, cwd) {
            tracing::warn!(kwant = %req.kwant_id, tool = %req.tool_name, "refused a tool call that names kwnt's secret files");
            if let Err(e) = self.db.save_refused(&record).await {
                tracing::warn!(error = %e, "could not record a refused call");
            }
            return ApprovalOutcome::Denied;
        }
        if !off && self.broker.is_allowed_for_session(&req.kwant_id, &self.conversation, &req.tool_name).await {
            return ApprovalOutcome::Allowed;
        }
        if off {
            if let Err(e) = self.db.save_auto_allowed(&record).await {
                tracing::warn!(error = %e, "could not record a call that ran with approvals off");
            }
            return ApprovalOutcome::Allowed;
        }
        // The person allowed this exact call on a card its earlier turn stopped waiting for
        if self.broker.take_grant(&req.kwant_id, &self.conversation, &req.tool_name, &req.arguments).await {
            tracing::info!(kwant = %req.kwant_id, tool = %req.tool_name, "ran a call the person allowed after its turn ended");
            return ApprovalOutcome::Allowed;
        }
        let ticket = match self
            .db
            .save_approval(&record)
            .await
        {
            Ok(a) => a.id,
            Err(e) => {
                tracing::warn!(error = %e, "could not store MCP approval ticket; denying");
                return ApprovalOutcome::Denied;
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
        let mut guard = TicketGuard {
            id: Some(ticket.clone()),
            broker: self.broker.clone(),
            db: self.db.clone(),
            events: self.events.clone(),
        };
        let _ = self.events.send(RuntimeEvent::ApprovalRequired {
            id: ticket.clone(),
            tool_name: req.tool_name.clone(),
            details,
        });

        let decision = tokio::time::timeout(self.timeout, rx).await;
        guard.id = None;
        match decision {
            Ok(Ok(ApprovalDecision::AllowOnce)) => ApprovalOutcome::Allowed,
            Ok(Ok(ApprovalDecision::AlwaysAllowSession)) => {
                self.broker.allow_for_session(&req.kwant_id, &self.conversation, &req.tool_name).await;
                ApprovalOutcome::Allowed
            }
            Ok(Ok(ApprovalDecision::Deny)) | Ok(Err(_)) => ApprovalOutcome::Denied,
            // Nobody answered: the card stays up as `waiting`, and a late Allow resumes the work
            // (see `resume_late_approval`)
            Err(_) => {
                if !self.broker.cancel(&ticket).await {
                    return ApprovalOutcome::Denied;
                }
                let chat = self.chat.clone().unwrap_or_else(|| json!({"direct_kwant_id": req.kwant_id}));
                match self.db.park_approval(&ticket, &chat).await {
                    Ok(true) => ApprovalOutcome::Unanswered,
                    _ => {
                        let _ = self.db.update_approval_status(&ticket, "expired").await;
                        let _ = self.events.send(RuntimeEvent::ApprovalWithdrawn { id: ticket });
                        ApprovalOutcome::Denied
                    }
                }
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

/// The conversation a turn runs in: its thread, else its channel or direct chat. A turn kwnt
/// isn't tracking (no turn id, or not started through the turn registry) is its own
/// conversation, so an "always allow" there lasts only that turn.
fn turn_conversation(state: &AppState, turn_id: &str) -> String {
    state
        .turns
        .active_turns()
        .into_iter()
        .find(|t| t.turn_id == turn_id)
        .map(|t| t.thread_id.unwrap_or(t.context_id))
        .unwrap_or_else(|| format!("turn:{turn_id}"))
}

/// The chat a tracked turn runs in, as `{"thread_id"}`, `{"channel_id"}` or
/// `{"direct_kwant_id"}`; `None` for a turn kwnt isn't tracking.
async fn turn_chat(state: &AppState, turn_id: &str) -> Option<Value> {
    let turn = state.turns.active_turns().into_iter().find(|t| t.turn_id == turn_id)?;
    Some(match turn.thread_id {
        Some(thread) => json!({"thread_id": thread}),
        None if state.db.get_channel(&turn.context_id).await.ok().flatten().is_some() => {
            json!({"channel_id": turn.context_id})
        }
        None => json!({"direct_kwant_id": turn.context_id}),
    })
}

/// A late answer to a card whose turn stopped waiting (`waiting`). Deny just closes it. Allow
/// lets that one call run once without a new card and starts a turn in the card's chat telling
/// the bot to make it again; "always allow" also remembers the tool for that conversation.
/// Approvals, secrets and the space are checked again when the call is made. Returns the JSON
/// reply, or `None` when the card isn't waiting (anymore).
pub async fn resume_late_approval(state: &AppState, ticket_id: &str, decision: ApprovalDecision, status: &str) -> Option<Value> {
    let (approval, chat) = state.db.claim_waiting_approval(ticket_id, status).await.ok().flatten()?;
    let _ = state.event_sender.send(RuntimeEvent::ApprovalWithdrawn { id: ticket_id.to_string() });
    if decision == ApprovalDecision::Deny {
        return Some(json!({"ticket_id": ticket_id, "resolved": true, "status": status, "resumed": false}));
    }
    let not_resumed = |why: &str| Some(json!({"ticket_id": ticket_id, "resolved": true, "status": status, "resumed": false, "reason": why}));
    let Ok(Some(kwant)) = state.db.get_kwant(&approval.kwant_id).await else {
        return not_resumed("the bot no longer exists");
    };
    if super::spaces::ensure_active(state, &crate::db::spaces::SpaceId::new(&kwant.space_id)).await.is_err() {
        return not_resumed("the bot's space is archived");
    }
    let details: Value = serde_json::from_str(&approval.parameters).unwrap_or(Value::Null);
    let arguments = details.get("arguments").cloned().unwrap_or_else(|| json!({}));
    let thread_id = chat["thread_id"].as_str().map(str::to_string);
    let channel_id = chat["channel_id"].as_str().map(str::to_string);
    let conversation = thread_id.clone().or_else(|| channel_id.clone()).or_else(|| chat["direct_kwant_id"].as_str().map(str::to_string)).unwrap_or_else(|| kwant.id.clone());
    state.broker.grant_once(&kwant.id, &conversation, &approval.tool_name, &arguments).await;
    if decision == ApprovalDecision::AlwaysAllowSession {
        state.broker.allow_for_session(&kwant.id, &conversation, &approval.tool_name).await;
    }
    let depth = match &thread_id {
        Some(t) => state.db.delegation_for_thread(t).await.ok().flatten().map(|d| d.depth).unwrap_or(0),
        None => 0,
    };
    let job = state.turns.start_job([conversation.clone(), ticket_id.to_string()]);
    let scope = match super::routes::scope_for(state, &kwant, channel_id, thread_id, &job, depth).await {
        Ok(s) => s,
        Err(why) => return not_resumed(&why),
    };
    let prompt = late_approval_prompt(&approval.tool_name, &arguments);
    let target = kwant.clone();
    let (broker, tool_name) = (state.broker.clone(), approval.tool_name.clone());
    tokio::spawn(async move {
        let _job = job;
        let _ = super::routes::try_kwant_turn(&scope, &target, prompt, Default::default()).await;
        // The grant was for this turn: unused, it must not let the same call through later
        if broker.drop_grant(&target.id, &conversation, &tool_name, &arguments).await {
            tracing::info!(kwant = %target.id, tool = %tool_name, "dropped a late-Allow grant the resumed turn didn't use");
        }
    });
    Some(json!({"ticket_id": ticket_id, "resolved": true, "status": status, "resumed": "new_turn"}))
}

/// The resumed turn's prompt after a late Allow.
pub fn late_approval_prompt(tool_name: &str, arguments: &Value) -> String {
    let args = serde_json::to_string_pretty(arguments).unwrap_or_default();
    let args: String = args.chars().take(2000).collect();
    format!(
        "[The person allowed a tool call you asked for earlier]\nTool: {tool_name}\nArguments: {args}\n\n\
Your earlier turn ended before they answered. Make this same call again now, with the same arguments \
(it runs once without a new card), then continue the task from where you stopped."
    )
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
    prepare_turn_with_self(state, kwant, None, cwd, None).await
}

/// [`prepare_turn`], plus the agents' own continuity tools (`search_history`, `memory_*`) when
/// `self_scope` is set. Those are served by kwnt itself and never need an approval card.
/// `turn_id` is the running turn's id, so its approvals can be traced back to it (e.g. to link
/// them to their chat); without one the session gets a fresh id.
pub async fn prepare_turn_with_self(
    state: &AppState,
    kwant: &Kwant,
    turn_id: Option<String>,
    cwd: Option<PathBuf>,
    self_scope: Option<super::continuity::SelfScope>,
) -> Option<(McpTurnContext, String)> {
    let mut servers = match state.db.list_active_mcp_servers_for_kwant(&kwant.id).await {
        Ok(s) => s,
        Err(e) => {
            tracing::warn!(error = %e, "could not load MCP servers for kwant");
            // The agent's own tools don't depend on the stored servers
            self_scope.as_ref()?;
            Vec::new()
        }
    };
    if servers.is_empty() && self_scope.is_none() {
        return None;
    }
    // Signed-in http servers get their (refreshed) access token as a secret header
    super::mcp_oauth::authorize_servers(state, &mut servers).await;
    let token = uuid::Uuid::new_v4().simple().to_string();
    let self_tools = self_scope.is_some() && !servers.iter().any(|s| s.name.eq_ignore_ascii_case(mcp::SELF_SERVER_NAME));
    if self_tools {
        servers.push(super::continuity::self_server(&local_base_url(state), &token));
    }
    let setting = state.db.get_setting("mcp_max_tool_iterations").await.ok().flatten();
    let turn_id = turn_id.unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
    let broker_approver: Arc<dyn ToolApprover> = Arc::new(BrokerApprover {
        db: state.db.clone(),
        broker: state.broker.clone(),
        events: state.event_sender.clone(),
        timeout: APPROVAL_TIMEOUT,
        conversation: turn_conversation(state, &turn_id),
        chat: turn_chat(state, &turn_id).await,
    });
    let ctx = McpTurnContext {
        servers,
        approver: if self_tools {
            Arc::new(super::continuity::SelfToolsApprover { inner: broker_approver })
        } else {
            broker_approver
        },
        config_dir: state.config.data_dir.join("mcp"),
        permission_url: Some(format!("{}/api/mcp/permission/{token}", local_base_url(state))),
        gateway: Some(GatewayAccess { url: format!("{}/api/mcp/gateway", local_base_url(state)), token: token.clone() }),
        max_tool_iterations: max_tool_iterations(setting),
        records: Default::default(),
    };
    let session = PermissionSession::new(kwant.id.clone(), kwant.name.clone(), turn_id, ctx.clone(), cwd);
    state.mcp_sessions.lock().await.insert(token.clone(), session);
    if let (true, Some(scope)) = (self_tools, self_scope) {
        state.self_tools.lock().await.insert(token.clone(), scope);
    }
    Some((ctx, token))
}

pub async fn finish_turn(state: &AppState, token: &str) {
    state.self_tools.lock().await.remove(token);
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

/// Answers one claude permission prompt. MCP tools from the kwant's own servers get their
/// approval card; claude's own tools follow [`mcp::builtin_tools`]; tools from servers the
/// kwant doesn't have are denied.
async fn decide(session: &PermissionSession, args: &Value) -> Value {
    let tool_name = args["tool_name"].as_str().unwrap_or("");
    let input = args.get("input").cloned().unwrap_or_else(|| json!({}));
    let tool_use_id = args["tool_use_id"].as_str().unwrap_or("").to_string();

    let (server, tool, builtin) = match mcp::split_tool_name(tool_name) {
        Some((server, tool)) if session.ctx.servers.iter().any(|s| s.name == server) => (server, tool, None),
        _ if tool_name.is_empty() || tool_name.starts_with("mcp__") => {
            return json!({"behavior": "deny", "message": format!("{tool_name} is not available to this bot.")});
        }
        _ => {
            let folder = mcp::builtin_tools::canonical_folder(session.cwd.as_deref());
            match mcp::builtin_tools::decide(tool_name, &input, folder.as_deref()) {
                mcp::builtin_tools::BuiltinDecision::Allow => {
                    return json!({"behavior": "allow", "updatedInput": input});
                }
                mcp::builtin_tools::BuiltinDecision::Ask { outside_folder } => {
                    let cwd = folder.or_else(|| session.cwd.clone()).map(|f| f.display().to_string());
                    ("", tool_name, Some(mcp::BuiltinCall { cwd, outside_folder }))
                }
            }
        }
    };
    let approval = session.ctx.approver.decide(ToolApprovalRequest {
        kwant_id: session.kwant_id.clone(),
        kwant_name: session.kwant_name.clone(),
        turn_id: session.turn_id.clone(),
        tool_name: tool_name.to_string(),
        server: server.to_string(),
        tool: tool.to_string(),
        arguments: input.clone(),
        builtin,
    });
    // Dropping the approval withdraws its card when the turn ends first
    let outcome = tokio::select! {
        outcome = approval => outcome,
        _ = session.ended.cancelled() => {
            session.ctx.record(cancelled_record(&tool_use_id, tool_name, server, tool, &input));
            return json!({"behavior": "deny", "message": "The turn ended before the user answered."});
        }
    };
    session.ctx.record(json!({
        "id": tool_use_id,
        "name": tool_name,
        "server": server,
        "tool": tool,
        "arguments": input,
        "status": outcome.record_status(),
    }));
    if outcome.allowed() {
        json!({"behavior": "allow", "updatedInput": input})
    } else {
        json!({"behavior": "deny", "message": outcome.refusal_text()})
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
    gateway_rpc(&state, &headers, msg, GatewayNaming::Plain).await
}

/// `POST /api/mcp/gateway/grok`: the same gateway, with tool names grok accepts (no `__`).
/// Calls are mapped back to the real tool, so approval cards and records show its usual name.
pub async fn gateway_grok_endpoint(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(msg): Json<Value>,
) -> Response {
    gateway_rpc(&state, &headers, msg, GatewayNaming::Grok).await
}

async fn gateway_rpc(
    state: &AppState,
    headers: &HeaderMap,
    msg: Value,
    naming: GatewayNaming,
) -> Response {
    let session = match bearer_token(headers) {
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
        "tools/list" => match session.tool_session().await {
            Some(tools) => rpc_result(id, json!({"tools": tools.gateway_tools(naming)})),
            None => StatusCode::UNAUTHORIZED.into_response(),
        },
        "tools/call" => {
            let Some(name) = msg["params"]["name"].as_str() else {
                return rpc_error(id, -32602, "missing tool name");
            };
            let arguments = msg["params"].get("arguments").cloned().unwrap_or_else(|| json!({}));
            let call_id = format!("gw_{}", uuid::Uuid::new_v4().simple());
            let Some(tools) = session.tool_session().await else {
                return StatusCode::UNAUTHORIZED.into_response();
            };
            // An unknown grok name stays as given, and the call fails as an unknown tool
            let tool_name = tools
                .canonical_name(naming, name)
                .unwrap_or_else(|| name.to_string());
            // Dropping the call withdraws its card (or stops the forwarded call) when the turn ends
            let (caller, raw_arguments) = (session.caller(), arguments.to_string());
            let outcome = tokio::select! {
                outcome = tools.call(&caller, &call_id, &tool_name, &raw_arguments) => outcome,
                _ = session.ended.cancelled() => {
                    let (server, tool) = mcp::split_tool_name(&tool_name).unwrap_or(("", name));
                    session.ctx.record(cancelled_record(&call_id, &tool_name, server, tool, &arguments));
                    drop(tools);
                    return rpc_result(id, json!({"content": [{"type": "text", "text": "The turn ended before the call finished."}], "isError": true}));
                }
            };
            rpc_result(
                id,
                json!({"content": [{"type": "text", "text": outcome.text}], "isError": outcome.status != "ok"}),
            )
        }
        _ => rpc_error(id, -32601, "method not found"),
    }
}
