//! MCP at turn time: the broker-backed approver, per-turn context setup, and the small MCP
//! endpoint the claude CLI calls (`--permission-prompt-tool`) so its MCP tool calls get the same
//! approval cards as API agents. Agent CLIs with no permission hook (codex) instead reach the
//! kwant's servers through kwnt's MCP gateway, which asks for approval before forwarding each call.

use super::AppState;
use crate::broker::{ApprovalDecision, ApprovalRequest, PermissionBroker};
use crate::db::models::{Kwant, McpServer, NewApproval};
use crate::db::Database;
use crate::drivers::events::RuntimeEvent;
use crate::mcp::{
    self,
    client::PROTOCOL_VERSION,
    session::{Caller, GatewayNaming, McpToolSession},
    ApprovalOutcome, Approvals, GatewayAccess, McpTurnContext, ToolApprovalRequest, ToolApprover,
};
use async_trait::async_trait;
use axum::{
    extract::{Path, State},
    http::{header::AUTHORIZATION, HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    Json,
};
use serde_json::{json, Value};
use std::path::{Path as FsPath, PathBuf};
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
    /// 21d: the turn's repeat guard for claude's own tools (Bash, Read…), fed by its hooks
    /// ([`permission_hook_endpoint`]). Separate from 18a's guard for kwnt's own tools; it goes
    /// when `finish_turn` drops the session.
    pub repeats: Arc<crate::mcp::repeat_guard::RepeatGuard>,
}

impl PermissionSession {
    pub fn new(kwant_id: String, kwant_name: String, turn_id: String, ctx: McpTurnContext, cwd: Option<PathBuf>) -> Self {
        Self {
            kwant_id,
            kwant_name,
            turn_id,
            ctx,
            cwd,
            gateway: Default::default(),
            ended: CancellationToken::new(),
            repeats: Default::default(),
        }
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
        let watch = super::tool_failures::TurnWatch::find(&self.turn_id);
        for problem in problems {
            tracing::warn!(kwant = %self.kwant_name, %problem, "MCP gateway could not reach a server");
            // The chat says so (0.9.5)
            if let Some(w) = &watch {
                w.server_problem(&problem).await;
            }
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
    /// kwnt started this turn on its own, nobody asked for it (the loose-ends owner, Watch,
    /// Verify, an audit…): "always allow" answers don't count and, under Ask or "Ask only when
    /// risky", every call that could change something asks, with the card saying why
    /// (`details.started_by`). Approvals off still runs everything (0.9.7): only
    /// [`BrokerApprover::strict_ask`] turns ask then. A call the person allowed on a card (late
    /// Allow) still runs once.
    pub always_ask: bool,
    /// Asks even when the bot's approvals are off: a turn that runs something the person hasn't
    /// reviewed (a Skill Foundry draft). Only with `always_ask`.
    pub strict_ask: bool,
    /// What kwnt started the turn for (its label, "Watch"), shown on an `always_ask` turn's card.
    pub started_by: Option<String>,
    /// Where task folders resolve, for the side-effect ledger (`turn_tool_calls`).
    pub data_dir: PathBuf,
}

fn held_cards() -> &'static std::sync::Mutex<std::collections::HashMap<String, Arc<tokio::sync::Notify>>> {
    static HELD: std::sync::OnceLock<std::sync::Mutex<std::collections::HashMap<String, Arc<tokio::sync::Notify>>>> =
        std::sync::OnceLock::new();
    HELD.get_or_init(Default::default)
}

/// Test-only (31k): the next card for `kwant_id` stops right after it is listed (its row is
/// `pending`, so /api/approvals, Needs you and the Inbox show it), before its bot finishes
/// setting it up, until the test calls `notify_one` on the returned gate. Lets a test answer a
/// card in that moment, the way a fast tap on a freshly listed card does on a busy machine.
#[doc(hidden)]
pub fn hold_next_card_after_listing(kwant_id: &str) -> Arc<tokio::sync::Notify> {
    let gate = Arc::new(tokio::sync::Notify::new());
    held_cards().lock().unwrap_or_else(|e| e.into_inner()).insert(kwant_id.to_string(), gate.clone());
    gate
}

/// Where a card held by [`hold_next_card_after_listing`] waits, once it is listed; every other
/// card goes straight on.
pub(crate) async fn card_listed(kwant_id: &str) {
    let gate = held_cards().lock().unwrap_or_else(|e| e.into_inner()).remove(kwant_id);
    if let Some(gate) = gate {
        gate.notified().await;
    }
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
                let _ = events.send(RuntimeEvent::approval_withdrawn(id, "cancelled", None));
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
        let noted = req.clone();
        let (outcome, decision, ticket) = self.classified(req).await;
        self.note(&noted, decision, ticket.as_deref()).await;
        if outcome.allowed() {
            grant_every_call_pass(&noted);
        }
        outcome
    }

    async fn note(&self, req: &ToolApprovalRequest, decision: &'static str, approval_id: Option<&str>) {
        // Truth Ledger: a turn that ran a tool reading outside text writes `outside` notes from now on
        if decision != "denied" && super::missions::truth_ledger::request_reads_outside(req) {
            super::missions::truth_ledger::taint(&req.turn_id);
        }
        super::missions::checkpoints::log_call(&self.db, &self.data_dir, req, decision, approval_id).await;
    }

    async fn tier_gate(&self, req: &ToolApprovalRequest) -> Option<String> {
        // A customer's turn: every call goes to the approver and asks, whatever else says it may run
        if every_call(&req.turn_id) {
            return Some(EVERY_CALL_REASON.to_string());
        }
        // Boxed: keeps this future (and every caller's) small
        Box::pin(super::missions::flight_school::tier_gate(&self.db, &self.data_dir, req)).await
    }

    async fn decide_granted(&self, req: ToolApprovalRequest, grant: &mcp::PrepushGrant) -> ApprovalOutcome {
        match Box::pin(self.granted(&req, grant)).await {
            Some(approval_id) => {
                if super::missions::truth_ledger::request_reads_outside(&req) {
                    super::missions::truth_ledger::taint(&req.turn_id);
                }
                let note = super::dev::prepush::grant_note(grant);
                super::missions::checkpoints::log_call_noted(&self.db, &self.data_dir, &req, "approved", approval_id.as_deref(), Some(&note)).await;
                ApprovalOutcome::Allowed
            }
            None => self.decide(req).await,
        }
    }
}

/// Turns where every tool call asks the person, whatever the bot's approvals say: a business
/// customer's Telegram turn (customer chats). Marked by [`prepare_turn_with_opts`], cleared by
/// [`finish_turn`].
static EVERY_CALL_TURNS: std::sync::LazyLock<std::sync::Mutex<std::collections::HashSet<String>>> =
    std::sync::LazyLock::new(|| std::sync::Mutex::new(std::collections::HashSet::new()));

/// What a card in an [`every_call`] turn says. Product copy is Mira's.
pub const EVERY_CALL_REASON: &str = "A customer's message started this turn, so every step asks you first.";

/// Whether every call in turn `turn_id` must ask the person (see [`EVERY_CALL_TURNS`]).
pub fn every_call(turn_id: &str) -> bool {
    EVERY_CALL_TURNS.lock().unwrap_or_else(|e| e.into_inner()).contains(turn_id)
}

fn mark_every_call(turn_id: &str, on: bool) {
    let mut m = EVERY_CALL_TURNS.lock().unwrap_or_else(|e| e.into_inner());
    if on {
        m.insert(turn_id.to_string());
    } else {
        m.remove(turn_id);
        EVERY_CALL_PASSES.lock().unwrap_or_else(|e| e.into_inner()).remove(turn_id);
    }
}

/// One-time passes for kwnt's own tools in an [`every_call`] turn: (tool, input) the approver
/// allowed, by turn. kwnt's tool endpoint runs a call in such a turn only on a matching pass, so
/// a CLI's own allow rules can't skip the card (see `continuity::self_endpoint`). Cleared with
/// the turn's mark.
static EVERY_CALL_PASSES: std::sync::LazyLock<std::sync::Mutex<EveryCallPasses>> = std::sync::LazyLock::new(Default::default);

/// Turn id → the (tool, input) pairs allowed and not yet run.
type EveryCallPasses = std::collections::HashMap<String, Vec<(String, Value)>>;

/// What kwnt's tool endpoint answers a call in an [`every_call`] turn that the approver never allowed.
pub const EVERY_CALL_REFUSED: &str = "This step needs the person's OK first.";

/// Records a pass for an allowed call to kwnt's own tools in an [`every_call`] turn.
fn grant_every_call_pass(req: &ToolApprovalRequest) {
    if req.server != mcp::SELF_SERVER_NAME || !every_call(&req.turn_id) {
        return;
    }
    let mut m = EVERY_CALL_PASSES.lock().unwrap_or_else(|e| e.into_inner());
    m.entry(req.turn_id.clone()).or_default().push((req.tool.clone(), req.arguments.clone()));
}

/// Whether kwnt's own tool `tool` may run with `args` in turn `turn_id`: always outside an
/// [`every_call`] turn; inside one only on a pass the approver granted, used up here.
pub fn take_every_call_pass(turn_id: &str, tool: &str, args: &Value) -> bool {
    if !every_call(turn_id) {
        return true;
    }
    let mut m = EVERY_CALL_PASSES.lock().unwrap_or_else(|e| e.into_inner());
    let Some(passes) = m.get_mut(turn_id) else {
        return false;
    };
    let Some(i) = passes.iter().position(|(t, a)| t == tool && a == args) else {
        return false;
    };
    passes.swap_remove(i);
    true
}

/// One of claude's plain read tools (Read, Glob, Grep).
fn plain_read(req: &ToolApprovalRequest) -> bool {
    req.builtin.is_some() && matches!(req.tool.as_str(), "Read" | "Glob" | "Grep")
}

/// A claude Bash call whose command only reads, in the bot's folder (see
/// [`mcp::shell_risk::reads_only`]).
fn shell_reads_only(req: &ToolApprovalRequest) -> bool {
    let Some(cwd) = req.builtin.as_ref().and_then(|b| b.cwd.as_deref()) else { return false };
    let Some(command) = req.arguments.get("command").and_then(Value::as_str) else { return false };
    if req.tool != "Bash" {
        return false;
    }
    let folder = std::path::Path::new(cwd).canonicalize().unwrap_or_else(|_| PathBuf::from(cwd));
    mcp::shell_risk::reads_only(command, &folder)
}

impl BrokerApprover {
    /// A push the person's Retry or Override covers (0.9.7): uses the grant up and records the call
    /// as approved by it (`Some(approval id)`), with no card. `None` when it must ask as usual: the
    /// payment floor, Flight School or the production floor wants a card (exactly as in
    /// [`Self::classified`]), it names kwnt's secrets, or the grant was used or expired.
    async fn granted(&self, req: &ToolApprovalRequest, grant: &mcp::PrepushGrant) -> Option<Option<String>> {
        if req.payment.is_some() || mcp::payment_floor::reason(req, None).is_some() || credit_floor(&self.db, req).await.is_some() {
            return None;
        }
        if home_floor(&self.db, req).await {
            return None;
        }
        if self.tier_gate(req).await.is_some() || super::dev::floor::for_call(&self.db, req).await.is_some() {
            return None;
        }
        let cwd = req.builtin.as_ref().and_then(|b| b.cwd.as_deref()).map(std::path::Path::new);
        if crate::sandbox::credentials::call_touches_kwnt_secrets(&req.arguments, cwd) {
            return None;
        }
        if !super::dev::prepush::use_grant(&self.db, &self.events, grant).await {
            return None;
        }
        tracing::info!(kwant = %req.kwant_id, tool = %req.tool_name, grant = %grant.id, card = %grant.pressed_card_id, "ran a push the person's {} approved", grant.kind);
        let mut details = match &req.builtin {
            Some(b) => json!({"source": "claude_builtin", "tool": req.tool, "arguments": req.arguments, "cwd": b.cwd, "outside_folder": b.outside_folder}),
            None => json!({"source": "mcp", "server": req.server, "tool": req.tool, "arguments": req.arguments}),
        };
        details["kwant_id"] = json!(req.kwant_id);
        details["kwant_name"] = json!(req.kwant_name);
        details["prepush_grant"] = grant_details(grant);
        let record = NewApproval { turn_id: req.turn_id.clone(), kwant_id: req.kwant_id.clone(), tool_name: req.tool_name.clone(), parameters: details };
        match self.db.save_granted(&record).await {
            Ok(id) => Some(Some(id)),
            Err(e) => {
                tracing::warn!(error = %e, "could not record a push the person's grant approved");
                Some(None)
            }
        }
    }

    /// [`ToolApprover::decide`], with the ledger's decision class and the card's id.
    async fn classified(&self, req: ToolApprovalRequest) -> (ApprovalOutcome, &'static str, Option<String>) {
        // Approvals the person switched off: run, and keep a record of what ran. A setting that
        // can't be read asks, as before.
        let mode = self.db.kwant_approvals(&req.kwant_id).await.ok();
        // The payment floor (D8): a call that pays always gets a card, whatever the approvals,
        // "Ask only when risky" or "always allow" say. Allowing it covers this one call.
        let payment = match req.payment.clone().or_else(|| mcp::payment_floor::reason(&req, None)) {
            Some(why) => Some(why),
            None => credit_floor(&self.db, &req).await,
        };
        // Flight School (C3): the bot's tier for this task can add a card, never remove one. It
        // skips approvals off and smart Allow; a session "always allow" still counts (D9).
        let tier = match payment {
            Some(_) => None,
            None => self.tier_gate(&req).await,
        };
        // The production floor (plan §3.5): in a Dev mode space, a step that can reach production
        // (a deploy, a push to the watched branch, a migration, a marked connection) always asks,
        // and "always allow" covers just that call.
        let floor = super::dev::floor::for_call(&self.db, &req).await;
        // 26f: on a Home Assistant server, a lock, a door, a garage door, a gate or an alarm
        // always asks, in every mode, and "always allow" covers just that call
        let home = home_floor(&self.db, &req).await;
        let forced = payment.is_some() || tier.is_some() || floor.is_some() || home;
        // The two audit calls only record the audit in kwnt (risks the person closes), so they
        // follow approvals off even in an always-asking audit turn; its other tools still ask
        let audit_call = req.server == mcp::SELF_SERVER_NAME
            && (super::missions::auditor::TOOL_NAMES.contains(&req.tool.as_str()) || req.tool == super::missions::verifier::TOOL);
        // Approvals off means off in a turn kwant started on its own too (0.9.7): only a strict one
        // (it runs something the person hasn't reviewed) still asks.
        let strict = self.always_ask && self.strict_ask;
        let off = !forced && (!strict || audit_call) && mode.as_ref().is_some_and(|m| m.is_off());
        // "Ask only when risky": what the caller made of this call; nothing known asks. A turn that
        // always asks (kwant started it on its own) still lets claude's plain read tools through,
        // and a shell command that only reads (`git show`, `grep`) unless the turn is strict.
        let smart = mode
            .as_ref()
            .is_some_and(|m| m.is_smart())
            .then(|| req.smart.clone())
            .flatten()
            .filter(|_| !self.always_ask || plain_read(&req) || (!strict && shell_reads_only(&req)))
            .filter(|_| !forced);
        let mut details = match &req.builtin {
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
        if let Some(verdict) = &smart {
            details["smart_reason"] = json!(verdict.reason());
        }
        if let Some(why) = &payment {
            details["payment_reason"] = json!(why);
            // 26h: the card says whose credits it uses instead of "a payment"
            if let Some(app) = mcp::payment_floor::CreditApp::of_reason(why) {
                details["credits"] = json!(app.label());
            }
        }
        if let Some(why) = &tier {
            details["tier_reason"] = json!(why);
        }
        if let Some((_, card)) = &floor {
            details["prod_floor"] = card.clone();
        }
        // Its own field: `always_ask` means a turn kwnt started on its own (Ada, Nov 16)
        if home {
            details["home_always_ask"] = json!(true);
            details["home_reason"] = json!(mcp::home_floor::REASON);
        }
        // Telegram keeps these cards link-only (Q-G6), each with its own line: a turn that read
        // outside text (an injected request is likeliest there), and a turn kwant started on its
        // own rather than from the person's message.
        if super::missions::truth_ledger::is_tainted(&req.turn_id) {
            details["tainted"] = json!(true);
        }
        if every_call(&req.turn_id) {
            details["customer_chat"] = json!(true);
        }
        if self.always_ask {
            details["always_ask"] = json!(true);
            // The card picks its sentence from this, not from the label (Ada, #569 round 2)
            if self.strict_ask {
                details["strict_ask"] = json!(true);
            }
            if let Some(label) = self.started_by.as_deref().map(str::trim).filter(|l| !l.is_empty()) {
                details["started_by"] = json!(label);
            }
        }
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
            return (ApprovalOutcome::Denied, "denied", None);
        }
        // 27b: the person already said no to a call like this in this chat. Their no wins over
        // "not risky", "always allow" and the floors, until they write here again; a late Allow
        // on an older card is still the person saying yes
        let families = mcp::declined::families(&req);
        if let Some(before) = self.broker.declined.check(&req.kwant_id, &self.conversation, &families) {
            if self.broker.take_grant(&req.kwant_id, &self.conversation, &req.tool_name, &req.arguments).await {
                return (ApprovalOutcome::Allowed, "approved", None);
            }
            tracing::info!(kwant = %req.kwant_id, tool = %req.tool_name, declined_before = %before, "refused without a card: the person already said no to this");
            let mut record = record;
            record.parameters["declined_before"] = json!(before);
            if let Err(e) = self.db.save_refused(&record).await {
                tracing::warn!(error = %e, "could not record a refused call");
            }
            return (ApprovalOutcome::DeclinedBefore, "denied", None);
        }
        if let Some(mcp::SmartVerdict::Allow(why)) = &smart {
            tracing::info!(kwant = %req.kwant_id, tool = %req.tool_name, reason = %why, "ran without a card: not risky");
            if let Err(e) = self.db.save_auto_allowed(&record).await {
                tracing::warn!(error = %e, "could not record a call that ran without a card");
            }
            return (ApprovalOutcome::Allowed, "auto_allowed", None);
        }
        if payment.is_some() {
            tracing::info!(kwant = %req.kwant_id, tool = %req.tool_name, reason = ?payment, "a payment: asking whatever the approvals say");
        } else if let Some((f, _)) = &floor {
            tracing::info!(kwant = %req.kwant_id, tool = %req.tool_name, kind = ?f.kind, "can reach production: asking whatever the approvals say");
        } else if home {
            tracing::info!(kwant = %req.kwant_id, tool = %req.tool_name, "a lock, a door or an alarm: asking whatever the approvals say");
        } else if !off && !self.always_ask && self.broker.is_allowed_for_session(&req.kwant_id, &self.conversation, &req.tool_name).await {
            return (ApprovalOutcome::Allowed, "allowed", None);
        }
        if off {
            if let Err(e) = self.db.save_auto_allowed(&record).await {
                tracing::warn!(error = %e, "could not record a call that ran with approvals off");
            }
            return (ApprovalOutcome::Allowed, "auto_allowed", None);
        }
        // The person allowed this exact call on a card its earlier turn stopped waiting for
        if self.broker.take_grant(&req.kwant_id, &self.conversation, &req.tool_name, &req.arguments).await {
            tracing::info!(kwant = %req.kwant_id, tool = %req.tool_name, "ran a call the person allowed after its turn ended");
            return (ApprovalOutcome::Allowed, "approved", None);
        }
        // 31k: the waiter first, then the row. A card is listed (`pending` in /api/approvals,
        // Needs you, the Inbox) only once an answer to it reaches this call; before, a Deny in
        // between was told "This card expired" and the call kept waiting
        let ticket = uuid::Uuid::new_v4().to_string();
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
        // Its chat is known from the start, stored with the card: the board links to it, and a
        // card that outlives a restart still resumes there (see `keep_cards_after_restart`)
        let chat = self.chat.clone().unwrap_or_else(|| json!({"direct_kwant_id": req.kwant_id}));
        if let Err(e) = self.db.save_approval_with_id(&ticket, &record, Some(&chat)).await {
            guard.id = None;
            self.broker.cancel(&ticket).await;
            tracing::warn!(error = %e, "could not store MCP approval ticket; denying");
            return (ApprovalOutcome::Denied, "denied", None);
        }
        card_listed(&req.kwant_id).await;
        tokio::time::sleep(std::time::Duration::from_millis(200)).await; // RED PROOF ONLY, not committed
        let _ = self.events.send(RuntimeEvent::ApprovalRequired {
            id: ticket.clone(),
            tool_name: req.tool_name.clone(),
            details,
        });
        super::activity::approval_waiting(&req.turn_id, &ticket, &req.tool_name);
        // 26b: a routine run's turn waits on this card: its run says so until the wait ends
        let chat_key = ["thread_id", "channel_id", "direct_kwant_id"]
            .iter()
            .find_map(|k| chat[*k].as_str())
            .unwrap_or(&req.kwant_id)
            .to_string();
        super::scheduler::approval_wait(&self.events, &req.kwant_id, &chat_key, Some(&ticket));

        let decision = tokio::time::timeout(self.timeout, rx).await;
        guard.id = None;
        super::scheduler::approval_wait(&self.events, &req.kwant_id, &chat_key, None);
        // 29o: a Deny's reason goes to the call listening for it (`hear_deny_reason`). 29s: taken
        // before the card's step settles, so a heard reason is that step's detail when it ends
        let heard_reason = match &decision {
            Ok(Ok(ApprovalDecision::Deny)) => self
                .broker
                .take_deny_reason(&ticket)
                .filter(|why| mcp::tell_deny_reason(why.clone())),
            _ => None,
        };
        if let Some(settled) = card_settles(&decision) {
            super::activity::approval_settled_with(&req.turn_id, &ticket, settled, heard_reason.as_deref());
        }
        let outcome = match decision {
            Ok(Ok(ApprovalDecision::AllowOnce)) => ApprovalOutcome::Allowed,
            // A payment's "always allow" covers this one call: nothing is remembered
            Ok(Ok(ApprovalDecision::AlwaysAllowSession)) if payment.is_some() => ApprovalOutcome::Allowed,
            // So does a production step's
            Ok(Ok(ApprovalDecision::AlwaysAllowSession)) if floor.is_some() => ApprovalOutcome::Allowed,
            // And a lock's, a door's or an alarm's (26f)
            Ok(Ok(ApprovalDecision::AlwaysAllowSession)) if home => ApprovalOutcome::Allowed,
            Ok(Ok(ApprovalDecision::AlwaysAllowSession)) => {
                self.broker.allow_for_session(&req.kwant_id, &self.conversation, &req.tool_name).await;
                ApprovalOutcome::Allowed
            }
            // 27b: only the person's own no is remembered, not a dropped card. A card that was
            // withdrawn or whose channel dropped (28k) runs nothing either, but it wasn't the
            // person's no, so its step settled `stopped` above
            // 29o: a Deny with the person's reason goes back to the call, and its family isn't
            // remembered: they said what to do instead, and the adjusted call gets its own card
            Ok(Ok(ApprovalDecision::Deny)) => {
                // A reason nobody is listening for (no caller scoped by `hear_deny_reason`) means
                // the bot hears the plain DENIED_TEXT, so the no is remembered as a plain one
                if heard_reason.is_none() {
                    self.broker.declined.record(&req.kwant_id, &self.conversation, &families, &ticket);
                }
                ApprovalOutcome::Denied
            }
            // The card was withdrawn or its channel dropped (28k): nothing runs, but it wasn't
            // the person's no, so its step settled `stopped` above
            Ok(Err(_)) => ApprovalOutcome::Denied,
            // Nobody answered: the card stays up as `waiting`, and a late Allow resumes the work
            // (see `resume_late_approval`)
            Err(_) => {
                let outcome = if !self.broker.cancel(&ticket).await {
                    ApprovalOutcome::Denied
                } else {
                    match self.db.park_approval(&ticket, &chat).await {
                        Ok(true) => ApprovalOutcome::Unanswered,
                        _ => {
                            let _ = self.db.update_approval_status(&ticket, "expired").await;
                            let _ = self.events.send(RuntimeEvent::approval_withdrawn(
                                ticket.clone(),
                                "expired",
                                None,
                            ));
                            ApprovalOutcome::Denied
                        }
                    }
                };
                // The card stays up, but this turn stops waiting for it: `unanswered` (26b) when
                // it was parked for a late answer, else `stopped`
                let settled = if outcome == ApprovalOutcome::Unanswered {
                    "unanswered"
                } else {
                    "stopped"
                };
                super::activity::approval_settled(&req.turn_id, &ticket, settled);
                outcome
            }
        };
        let class = if outcome.allowed() { "approved" } else { "denied" };
        (outcome, class, Some(ticket))
    }
}

/// How an approval card's step settles once its wait ends with `decision`: "ok" on an Allow,
/// "declined" only on the person's Deny, and "stopped" when the card was withdrawn or its
/// channel dropped with no answer (28k: nobody pressed Deny, so it isn't the person's no). `None`
/// when the wait ran out: the timeout path settles it (`unanswered` or `stopped`).
fn card_settles<E, T>(decision: &Result<Result<ApprovalDecision, E>, T>) -> Option<&'static str> {
    match decision {
        Ok(Ok(ApprovalDecision::AllowOnce | ApprovalDecision::AlwaysAllowSession)) => Some("ok"),
        Ok(Ok(ApprovalDecision::Deny)) => Some("declined"),
        Ok(Err(_)) => Some("stopped"),
        Err(_) => None,
    }
}

/// 26h: why a call to a renamed credit app (FLUX, Picsart) asks: the bot's stored server with
/// this name is at the app's address ([`mcp::payment_floor::credits_reason`]). Calls kwnt runs
/// itself already carry the address in `req.payment`; this covers the ones an agent CLI asks
/// about by name only. `None` for kwnt's own and claude's built-in tools.
pub async fn credit_floor(db: &Database, req: &ToolApprovalRequest) -> Option<String> {
    if req.builtin.is_some() || req.server.is_empty() || req.server == mcp::SELF_SERVER_NAME {
        return None;
    }
    let servers = match db.list_active_mcp_servers_for_kwant(&req.kwant_id).await {
        Ok(servers) => servers,
        Err(e) => {
            tracing::warn!(error = %e, "could not read the bot's MCP servers for the credit check");
            return None;
        }
    };
    let server = servers.into_iter().find(|s| s.name == req.server)?;
    mcp::payment_floor::credits_reason(&server.name, server.url.as_deref(), &req.tool)
}

/// 27e: the names (in kwant) of the turn's `servers` that are Home Assistant's: by what each
/// said on kwnt's last connection to it in this run, else by what its last test stored. A server
/// kwnt never heard from counts when its address is Home Assistant's MCP path (28m: on the Claude
/// CLI the CLI connects, not kwnt, so a new server is often one kwnt never heard from), see
/// [`mcp::home_floor::counts_as_home_assistant`]. The delivery check only adds a continuation.
pub async fn home_server_names(db: &Database, servers: &[McpServer]) -> Vec<String> {
    let mut names = Vec::new();
    for server in servers.iter().filter(|s| s.name != mcp::SELF_SERVER_NAME && s.name != mcp::RESERVED_SERVER_NAME) {
        let heard = match mcp::home_floor::seen(&server.id) {
            Some(seen) => Some(seen.reported),
            // A read that failed is a server kwnt can't place: its address decides (the safe side)
            None => db.mcp_reported_name(&server.id).await.ok().flatten(),
        };
        let heard = heard.as_ref().map(Option::as_deref);
        if mcp::home_floor::counts_as_home_assistant(heard, &server.transport, server.url.as_deref(), &server.args) {
            names.push(server.name.clone());
        }
    }
    names
}

/// 26f: whether `req` acts on a lock, a door, a garage door, a gate or an alarm on a Home
/// Assistant server ([`mcp::home_floor`]), so it always asks. What the server is comes from what
/// it said on kwnt's last connection to it in this run, else from what its last test stored; one
/// learned in this run is stored too. A server kwnt never heard from counts as Home Assistant's
/// (the safe side), and only for a call whose arguments name one of those things.
pub async fn home_floor(db: &Database, req: &ToolApprovalRequest) -> bool {
    if req.builtin.is_some() || req.server.is_empty() || req.server == mcp::SELF_SERVER_NAME {
        return false;
    }
    // Cheap first: most calls name no lock, door or alarm
    if !mcp::home_floor::sensitive_args(&req.arguments) {
        return false;
    }
    let server = match db.list_active_mcp_servers_for_kwant(&req.kwant_id).await {
        Ok(servers) => servers.into_iter().find(|s| s.name == req.server),
        // Can't tell which server it is: ask
        Err(e) => {
            tracing::warn!(error = %e, "could not read the bot's MCP servers; a lock or a door asks");
            return true;
        }
    };
    let Some(server) = server else { return false };
    let seen = mcp::home_floor::seen(&server.id);
    let stored = db.mcp_reported_name(&server.id).await.unwrap_or_else(|e| {
        tracing::warn!(error = %e, "could not read what the MCP server said it is");
        None
    });
    if let Some(s) = &seen {
        if stored.as_ref() != Some(&s.reported) {
            if let Err(e) = db.save_mcp_reported_name(&server.id, s.reported.as_deref()).await {
                tracing::warn!(error = %e, "could not store what the MCP server said it is");
            }
        }
    }
    let kind = match (&seen, &stored) {
        (Some(s), _) => mcp::home_floor::ServerKind::of(s.reported.as_deref()),
        (None, Some(name)) => mcp::home_floor::ServerKind::of(name.as_deref()),
        (None, None) => mcp::home_floor::ServerKind::Unknown,
    };
    let read_only = seen.and_then(|s| s.read_only).map(|tools| tools.contains(&req.tool));
    mcp::home_floor::decide(kind, &req.tool, &req.arguments, read_only)
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
        .map(|t| conversation_of(t.thread_id, t.context_id))
        .unwrap_or_else(|| format!("turn:{turn_id}"))
}

/// The conversation of a turn in `thread_id`, else in `context_id` (its channel or direct chat):
/// what "always allow" and a declined call (27b) are kept for.
pub(super) fn conversation_of(thread_id: Option<String>, context_id: String) -> String {
    thread_id.unwrap_or(context_id)
}

/// 27b: the person wrote to `kwant_id` in the conversation of `thread_id`, else `context_id`, so
/// what they declined there may be asked again.
pub(super) fn person_wrote(broker: &PermissionBroker, kwant_id: &str, thread_id: Option<String>, context_id: String) {
    broker.declined.clear(kwant_id, &conversation_of(thread_id, context_id));
}

/// The chat a tracked turn runs in, as `{"thread_id"}`, `{"channel_id"}` or
/// `{"direct_kwant_id"}`; `None` for a turn kwnt isn't tracking.
pub(super) async fn turn_chat(state: &AppState, turn_id: &str) -> Option<Value> {
    let turn = state.turns.active_turns().into_iter().find(|t| t.turn_id == turn_id)?;
    Some(match turn.thread_id {
        Some(thread) => json!({"thread_id": thread}),
        None if state.db.get_channel(&turn.context_id).await.ok().flatten().is_some() => {
            json!({"channel_id": turn.context_id})
        }
        None => json!({"direct_kwant_id": turn.context_id}),
    })
}

/// At startup, before any turn runs: the cards the last run of kwant was still waiting on are
/// kept as `waiting`, like a card whose turn stopped waiting, so answering one never fails. Allow
/// starts a turn in the card's chat that makes the call again; a payment's Allow still covers
/// only that one call (D8). Must run before [`super::routes::interrupt_running_turns`], which
/// clears the turns a card's chat may be looked up in.
pub async fn keep_cards_after_restart(state: &AppState) -> u64 {
    match state.db.park_orphaned_approvals().await {
        Ok(0) => 0,
        Ok(n) => {
            tracing::info!(count = n, "kept approval cards from before the restart answerable");
            n
        }
        Err(e) => {
            tracing::warn!(error = %e, "could not keep approval cards from before the restart");
            0
        }
    }
}

/// A late answer to a card whose turn stopped waiting (`waiting`). Deny just closes it. Allow
/// lets that one call run once without a new card and starts a turn in the card's chat telling
/// the bot to make it again; "always allow" also remembers the tool for that conversation.
/// Approvals, secrets and the space are checked again when the call is made. Returns the JSON
/// reply, or `None` when the card isn't waiting (anymore).
pub async fn resume_late_approval(state: &AppState, ticket_id: &str, decision: ApprovalDecision, status: &str, via: Option<&str>) -> Option<Value> {
    let (approval, chat) = state.db.claim_waiting_approval(ticket_id, status).await.ok().flatten()?;
    // Recorded before it's announced, so a reload right after the event reads the same `answered_via`
    if let Some(via) = via {
        let _ = state.db.set_answered_via("approvals", ticket_id, via).await;
    }
    let _ = state.event_sender.send(RuntimeEvent::approval_withdrawn(ticket_id, status, via));
    if decision == ApprovalDecision::Deny {
        return Some(json!({"ticket_id": ticket_id, "resolved": true, "status": status, "resumed": false}));
    }
    let not_resumed = |why: &str| Some(json!({"ticket_id": ticket_id, "resolved": true, "status": status, "resumed": false, "reason": why}));
    // An audit turn is hidden and its stage is over: nothing runs, and no turn starts in the
    // bot's chat (its outcome is on the task's audit)
    if super::missions::auditor::is_audit_card(&chat, &approval.tool_name) {
        tracing::info!(ticket_id, kwant = %approval.kwant_id, tool = %approval.tool_name, "closed a late answer to an audit card");
        return not_resumed(super::missions::auditor::LATE_CARD);
    }
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
    // A payment's "always allow" covers the one call (the payment floor, D8). So does a production step's (plan §3.5)
    if decision == ApprovalDecision::AlwaysAllowSession
        && details.get("payment_reason").is_none()
        && details.get("prod_floor").is_none()
        && details.get("home_always_ask").is_none()
    {
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
    // The same agent and effort as the turn that asked, not what Laya makes of this prompt
    let mut opts = super::routes::TurnOpts::routed_as(chat["agent"].as_str(), chat["effort"].as_str());
    // A customer's turn goes on as one: the person allowed this call, not the ones after it
    if chat["purpose"].as_str() == Some(super::telegram::customers::PURPOSE) {
        super::telegram::customers::customer_opts(&mut opts);
    }
    tokio::spawn(async move {
        let _job = job;
        let _ = super::routes::try_kwant_turn(&scope, &target, prompt, opts).await;
        // The grant was for this turn: unused, it must not let the same call through later
        if broker.drop_grant(&target.id, &conversation, &tool_name, &arguments).await {
            tracing::info!(kwant = %target.id, tool = %tool_name, "dropped a late-Allow grant the resumed turn didn't use");
        }
    });
    Some(json!({"ticket_id": ticket_id, "resolved": true, "status": status, "resumed": "new_turn"}))
}

/// Cards nobody answered within [`crate::db::approval_modes::WAITING_APPROVAL_HOURS`] expire
/// (0.9.5): each once, with a notice in the card's chat and a Needs-you item whose Retry
/// (`POST /api/approvals/:id/retry`) asks the bot to make the call again, so the person gets a
/// fresh card. Audit cards just close (their stage is over). Returns how many expired.
pub async fn expire_unanswered(state: &AppState) -> usize {
    let Ok(expired) = state.db.expire_waiting_approvals().await else {
        return 0;
    };
    let n = expired.len();
    for (approval, chat) in expired {
        let _ = state.event_sender.send(RuntimeEvent::approval_withdrawn(&approval.id, "expired", None));
        if super::missions::auditor::is_audit_card(&chat, &approval.tool_name) {
            continue;
        }
        let Ok(Some(kwant)) = state.db.get_kwant(&approval.kwant_id).await else {
            continue;
        };
        let place = expired_place(&chat, &kwant.id);
        let text = expired_text(&kwant.name, &approval.tool_name, crate::db::approval_modes::WAITING_APPROVAL_HOURS);
        let msg = crate::db::models::NewMessage {
            channel_id: place.channel_id.clone(),
            direct_kwant_id: place.direct_kwant_id.clone(),
            sender_role: "system".into(),
            sender_id: "kwnt".into(),
            text: text.clone(),
            reasoning_content: None,
            tool_calls: None,
            tokens_used: 0,
        };
        let meta = json!({
            "kind": "approval_expired",
            "approval_id": approval.id,
            "kwant_id": kwant.id,
            "kwant_name": kwant.name,
            "tool_name": approval.tool_name,
        });
        let extra = crate::db::models::MessageExtra { thread_id: place.thread_id.clone(), meta: Some(meta) };
        let saved = state.db.insert_message_with(&msg, &extra).await.ok();
        if let Some(t) = &place.thread_id {
            let _ = state.db.touch_thread(t).await;
        }
        if let Some(m) = &saved {
            let _ = state.event_sender.send(RuntimeEvent::MessageAdded { message: serde_json::to_value(m).unwrap_or_default() });
        }
        super::stopped::ask(state, &place, saved.as_ref().map(|m| m.id.as_str()), &text, super::stopped::Resume::Approval { approval_id: approval.id.clone() }).await;
        tracing::info!(ticket_id = %approval.id, kwant = %kwant.id, tool = %approval.tool_name, "an approval card expired unanswered: told the person");
    }
    if n > 0 {
        super::missions::listener::refresh_needs_you(state).await;
    }
    n
}

/// Where an expired card's notice goes: its chat as recorded on the card, else the bot's DM.
fn expired_place(chat: &Value, kwant_id: &str) -> super::stopped::Place {
    let thread_id = chat["thread_id"].as_str().map(str::to_string);
    let channel_id = chat["channel_id"].as_str().map(str::to_string);
    let direct = chat["direct_kwant_id"].as_str().map(str::to_string);
    let direct_kwant_id = match (&thread_id, &channel_id, direct) {
        (None, None, None) => Some(kwant_id.to_string()),
        (_, _, d) => d,
    };
    super::stopped::Place { kwant_id: kwant_id.to_string(), channel_id, direct_kwant_id, thread_id }
}

/// The chat notice (and Needs-you line) for a card that expired unanswered.
pub fn expired_text(kwant_name: &str, tool_name: &str, hours: i64) -> String {
    format!("{kwant_name} stopped waiting for your OK to use {tool_name} (no answer in {hours} h), so that step didn't run. Retry asks you again.")
}

/// Retry on a card that expired unanswered: starts a turn in its chat telling the bot to make the
/// call again. Nothing is granted: the call asks for a fresh card. `None` when the card isn't
/// expired (never, or Retry already ran).
pub async fn retry_expired(state: &AppState, ticket_id: &str) -> Option<Value> {
    let (approval, chat) = state.db.claim_expired_approval(ticket_id).await.ok().flatten()?;
    let rows = state.db.close_stopped_for_approval(ticket_id).await.unwrap_or_default();
    super::waiting::closed(state, &rows, super::waiting::REASON_RESUMED);
    let not_resumed = |why: &str| Some(json!({"ticket_id": ticket_id, "status": "retried", "resumed": false, "reason": why}));
    let Ok(Some(kwant)) = state.db.get_kwant(&approval.kwant_id).await else {
        return not_resumed("the bot no longer exists");
    };
    if super::spaces::ensure_active(state, &crate::db::spaces::SpaceId::new(&kwant.space_id)).await.is_err() {
        return not_resumed("the bot's space is archived");
    }
    let details: Value = serde_json::from_str(&approval.parameters).unwrap_or(Value::Null);
    let arguments = details.get("arguments").cloned().unwrap_or_else(|| json!({}));
    let place = expired_place(&chat, &kwant.id);
    let thread_id = place.thread_id.clone();
    // A DM's scope is the bot's own chat (no channel, no thread)
    let channel_id = place.channel_id.clone();
    let conversation = thread_id.clone().or_else(|| channel_id.clone()).unwrap_or_else(|| kwant.id.clone());
    let depth = match &thread_id {
        Some(t) => state.db.delegation_for_thread(t).await.ok().flatten().map(|d| d.depth).unwrap_or(0),
        None => 0,
    };
    let job = state.turns.start_job([conversation.clone(), ticket_id.to_string()]);
    let scope = match super::routes::scope_for(state, &kwant, channel_id, thread_id, &job, depth).await {
        Ok(s) => s,
        Err(why) => return not_resumed(&why),
    };
    let prompt = expired_retry_prompt(&approval.tool_name, &arguments);
    let target = kwant.clone();
    let mut opts = super::routes::TurnOpts::routed_as(chat["agent"].as_str(), chat["effort"].as_str());
    // A customer's turn goes on as one: the person allowed this call, not the ones after it
    if chat["purpose"].as_str() == Some(super::telegram::customers::PURPOSE) {
        super::telegram::customers::customer_opts(&mut opts);
    }
    tokio::spawn(async move {
        let _job = job;
        let _ = super::routes::try_kwant_turn(&scope, &target, prompt, opts).await;
    });
    Some(json!({"ticket_id": ticket_id, "status": "retried", "resumed": "new_turn"}))
}

/// The turn Retry starts on an expired card.
pub fn expired_retry_prompt(tool_name: &str, arguments: &Value) -> String {
    let args = serde_json::to_string_pretty(arguments).unwrap_or_default();
    let args: String = args.chars().take(2000).collect();
    format!(
        "[The person pressed Retry on a tool call that waited too long for their OK]\nTool: {tool_name}\nArguments: {args}\n\n\
Nobody answered the approval card in time, so this call never ran. Make the same call again now; \
the person gets a fresh card to answer. Then continue the task from where you stopped."
    )
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
pub(crate) fn local_base_url(state: &AppState) -> String {
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
    prepare_turn_with_opts(state, kwant, turn_id, cwd, self_scope, TurnApprovalOpts::default()).await
}

/// How a turn's cards behave, beyond the bot's approval setting.
#[derive(Debug, Default, Clone)]
pub struct TurnApprovalOpts {
    /// The turn's (agent, effort): a card it leaves unanswered resumes on the same ones after a
    /// late Allow.
    pub routed: Option<(String, String)>,
    /// kwnt started the turn on its own (see [`BrokerApprover::always_ask`]).
    pub always_ask: bool,
    /// Asks even with approvals off (see [`BrokerApprover::strict_ask`]).
    pub strict_ask: bool,
    /// The turn's label, for its cards (see [`BrokerApprover::started_by`]).
    pub started_by: Option<String>,
    /// A Skill Foundry replay: nothing that could change anything outside the scratch folder
    /// runs, and every such call is logged here as what the bot would have done (see
    /// [`RehearseApprover`]).
    pub rehearse: Option<RehearseLog>,
    /// The turn's purpose (`TurnOpts::purpose`), kept on its cards as `purpose`: a late answer to
    /// a card of a hidden audit turn is closed, never resumed (see [`resume_late_approval`]).
    pub purpose: Option<String>,
    /// Every tool call asks the person, approvals off, "Ask only when risky", "always allow" and
    /// the bot's own free tools included: a business customer's turn (see [`every_call`]).
    pub every_call: bool,
}

/// The "would have…" lines of a replay turn.
pub type RehearseLog = Arc<std::sync::Mutex<Vec<Value>>>;

/// A Skill Foundry replay's approver: no cards and no grants. kwant's own read-only tools and
/// in-folder file tools run (the permission hook allows those on its own); every other call is
/// refused as [`ApprovalOutcome::Rehearsed`] and logged.
pub struct RehearseApprover {
    pub log: RehearseLog,
}

#[async_trait]
impl ToolApprover for RehearseApprover {
    async fn approve(&self, req: ToolApprovalRequest) -> bool {
        self.decide(req).await.allowed()
    }

    async fn decide(&self, req: ToolApprovalRequest) -> ApprovalOutcome {
        if req.server == mcp::SELF_SERVER_NAME && super::continuity::READ_ONLY_SELF_TOOLS.contains(&req.tool.as_str()) {
            return ApprovalOutcome::Allowed;
        }
        let line = json!({
            "tool": req.tool_name,
            "would_have": super::missions::checkpoints::summary(&req.tool_name, &req.arguments),
            "outside_folder": req.builtin.as_ref().is_some_and(|b| b.outside_folder),
        });
        if let Ok(mut log) = self.log.lock() {
            // A runaway turn can't grow it without bound
            if log.len() < 200 {
                log.push(line);
            }
        }
        ApprovalOutcome::Rehearsed
    }

    /// Subagents are refused too: their own calls aren't held to the scratch folder.
    async fn tier_gate(&self, req: &ToolApprovalRequest) -> Option<String> {
        mcp::builtin_tools::SUBAGENT_TOOLS.contains(&req.tool_name.as_str()).then(|| "a replay starts no subagents".into())
    }
}

/// Puts the pre-push card (B2) in front of `inner` for MCP calls that write to a GitHub branch
/// (Ada's review of #456, Later 6a); everything else is `inner`'s.
pub struct PrepushApprover {
    pub inner: Arc<dyn ToolApprover>,
    pub state: AppState,
}

#[async_trait]
impl ToolApprover for PrepushApprover {
    async fn approve(&self, req: ToolApprovalRequest) -> bool {
        self.inner.approve(req).await
    }

    async fn decide(&self, req: ToolApprovalRequest) -> ApprovalOutcome {
        self.inner.decide(req).await
    }

    async fn note(&self, req: &ToolApprovalRequest, decision: &'static str, approval_id: Option<&str>) {
        self.inner.note(req, decision, approval_id).await
    }

    async fn tier_gate(&self, req: &ToolApprovalRequest) -> Option<String> {
        self.inner.tier_gate(req).await
    }

    async fn prepush_gate(&self, req: &ToolApprovalRequest) -> Result<mcp::PrepushPass, String> {
        Box::pin(super::dev::prepush::check_mcp(&self.state, &req.kwant_id, &req.server, &req.tool, &req.arguments))
            .await
            .map(|p| mcp::PrepushPass { overrides: p.overrides, grant: p.grant })
    }

    async fn prepush_use(&self, overrides: &[String]) -> Option<String> {
        super::dev::prepush::use_up(&self.state, &super::dev::prepush::Pass { overrides: overrides.to_vec(), ..Default::default() }).await
    }

    async fn decide_granted(&self, req: ToolApprovalRequest, grant: &mcp::PrepushGrant) -> ApprovalOutcome {
        self.inner.decide_granted(req, grant).await
    }
}

/// What an approval record keeps about the grant that approved its call (0.9.7).
pub fn grant_details(grant: &mcp::PrepushGrant) -> Value {
    json!({
        "id": grant.id,
        "kind": grant.kind,
        "card_id": grant.pressed_card_id,
        "push_card_id": grant.card_id,
        "note": super::dev::prepush::grant_note(grant),
    })
}

/// [`prepare_turn_with_self`] with [`TurnApprovalOpts`].
pub async fn prepare_turn_with_opts(
    state: &AppState,
    kwant: &Kwant,
    turn_id: Option<String>,
    cwd: Option<PathBuf>,
    mut self_scope: Option<super::continuity::SelfScope>,
    opts: TurnApprovalOpts,
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
    // B1: a customer's message starts this turn, so none of the bot's other servers (mail,
    // calendar...) is even started: the agent CLI would run them under its own allow rules
    if opts.every_call {
        servers.clear();
    }
    let token = uuid::Uuid::new_v4().simple().to_string();
    isolate_stdio_servers(state, &mut servers, cwd.as_deref(), &token).await;
    if servers.is_empty() && self_scope.is_none() {
        return None;
    }
    // Signed-in http servers get their (refreshed) access token as a secret header
    super::mcp_oauth::authorize_servers(state, &mut servers).await;
    // the payment check reads the page this turn's browser tools use (its job page in a chat)
    let browser = self_scope.as_ref().and_then(super::browser_pages::scope_page).map(|t| (state.clone(), t));
    let self_tools = self_scope.is_some() && !servers.iter().any(|s| s.name.eq_ignore_ascii_case(mcp::SELF_SERVER_NAME));
    if self_tools {
        servers.push(super::continuity::self_server(&local_base_url(state), &token));
    }
    let setting = state.db.get_setting("mcp_max_tool_iterations").await.ok().flatten();
    let turn_id = turn_id.unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
    // The memory tools look the turn's taint up by this id
    if let Some(scope) = self_scope.as_mut() {
        scope.turn_id.get_or_insert_with(|| turn_id.clone());
    }
    if opts.every_call {
        mark_every_call(&turn_id, true);
    }
    // A turn that always asks reads outside text from the start (contract §2.4)
    if opts.always_ask {
        super::missions::truth_ledger::taint(&turn_id);
    }
    let mut chat = turn_chat(state, &turn_id).await;
    if let Some((agent, effort)) = &opts.routed {
        let chat = chat.get_or_insert_with(|| json!({"direct_kwant_id": kwant.id}));
        chat["agent"] = json!(agent);
        chat["effort"] = json!(effort);
    }
    if let Some(purpose) = &opts.purpose {
        chat.get_or_insert_with(|| json!({"direct_kwant_id": kwant.id}))["purpose"] = json!(purpose);
    }
    let broker_approver: Arc<dyn ToolApprover> = Arc::new(BrokerApprover {
        db: state.db.clone(),
        broker: state.broker.clone(),
        events: state.event_sender.clone(),
        timeout: Duration::from_millis(
            state
                .approval_wait_ms
                .load(std::sync::atomic::Ordering::Relaxed),
        ),
        conversation: turn_conversation(state, &turn_id),
        chat,
        always_ask: opts.always_ask,
        strict_ask: opts.strict_ask,
        started_by: opts.started_by.clone(),
        data_dir: state.config.data_dir.clone(),
    });
    let approver: Arc<dyn ToolApprover> = if let Some(log) = opts.rehearse.clone() {
        Arc::new(RehearseApprover { log })
    } else if opts.purpose.as_deref() == Some(super::dev::tickets::BRIEF_PURPOSE) {
        // A Dev mode brief only reads, whatever the ticket says (J7)
        Arc::new(super::dev::tickets::tools::BriefApprover { inner: broker_approver })
    } else if self_tools {
        Arc::new(super::continuity::SelfToolsApprover { inner: broker_approver, always_ask: opts.always_ask, browser })
    } else {
        broker_approver
    };
    // A dry run writes nothing, so it needs no pre-push card
    let approver: Arc<dyn ToolApprover> =
        if opts.rehearse.is_some() { approver } else { Arc::new(PrepushApprover { inner: approver, state: state.clone() }) };
    // 26h: files a tool returns are saved in the turn's chat (not in a dry run, which writes nothing)
    let media = match (&self_scope, &opts.rehearse) {
        (Some(scope), None) if scope.origin.is_some() => Some(Arc::new(super::attachments::ChatMedia {
            state: state.clone(),
            scope: scope.clone(),
        }) as Arc<dyn mcp::media::MediaSink>),
        _ => None,
    };
    let ctx = McpTurnContext {
        servers,
        approver,
        config_dir: state.config.data_dir.join("mcp"),
        permission_url: Some(format!("{}/api/mcp/permission/{token}", local_base_url(state))),
        gateway: Some(GatewayAccess { url: format!("{}/api/mcp/gateway", local_base_url(state)), token: token.clone() }),
        max_tool_iterations: max_tool_iterations(setting),
        approvals: {
            let mode = state.db.kwant_approvals(&kwant.id).await.map(|m| Approvals::from_mode(&m.effective)).unwrap_or_default();
            if opts.every_call || (opts.always_ask && opts.strict_ask) || opts.purpose.as_deref() == Some(super::dev::tickets::BRIEF_PURPOSE) {
                // A strict turn asks whatever the setting says, and a Dev mode brief only reads: never tell either to just go ahead
                Approvals::Ask
            } else if opts.always_ask && mode != Approvals::Off {
                // A turn kwant started on its own asks under "Ask only when risky" too
                Approvals::Ask
            } else {
                mode
            }
        },
        records: Default::default(),
        kwnt_only: opts.every_call,
        media,
    };
    let session = PermissionSession::new(kwant.id.clone(), kwant.name.clone(), turn_id, ctx.clone(), cwd);
    state.mcp_sessions.lock().await.insert(token.clone(), session);
    if let (true, Some(mut scope)) = (self_tools, self_scope) {
        // 18a: one repeat guard per turn token, fresh even when the caller's scope was cloned before
        scope.repeats = Default::default();
        state.self_tools.lock().await.insert(token.clone(), scope);
    }
    Some((ctx, token))
}

pub async fn finish_turn(state: &AppState, token: &str) {
    state.self_tools.lock().await.remove(token);
    let session = state.mcp_sessions.lock().await.remove(token);
    if let Some(session) = session {
        mark_every_call(&session.turn_id, false);
        super::missions::truth_ledger::end_turn(&session.turn_id);
        session.close().await;
    }
    remove_mcp_cwd(state, token).await;
}

/// The empty folder stdio servers start from for the turn (or test) `token`, when its folder is
/// one an isolated bot used ([`isolate_stdio_servers`]).
pub fn mcp_cwd(data_dir: &FsPath, token: &str) -> PathBuf {
    data_dir.join("run").join("mcp-cwd").join(token)
}

/// D4 (C8): when `cwd` is equal to, inside or holding a folder an isolated bot used (or, with no
/// folder, kwnt's own working folder sits in one; see [`Database::isolated_folder_near`]), a stdio
/// server could run what the bot left there, on this computer. Then:
/// - a server that names a file in that folder (`./serve.js`, an absolute path into it) isn't
///   started ([`mcp::runs_folder_file`]);
/// - every other stdio server starts from a fresh, empty, kwnt-owned folder [`mcp_cwd`]
///   ([`mcp::start_in`]), so `python3 -m x` or `npx x` can't pick up a planted file. The caller
///   removes it ([`finish_turn`], [`remove_mcp_cwd`]).
///
/// When that folder can't be made, no stdio server starts. Returns the folder when one was made.
pub(crate) async fn isolate_stdio_servers(state: &AppState, servers: &mut Vec<McpServer>, cwd: Option<&FsPath>, token: &str) -> Option<PathBuf> {
    if !servers.iter().any(|s| s.transport == "stdio") {
        return None;
    }
    let near = state.db.isolated_folder_near(cwd).await?;
    let here = cwd.filter(|d| d.exists()).map(FsPath::to_path_buf).or_else(|| std::env::current_dir().ok());
    servers.retain(|s| {
        let runs = mcp::runs_folder_file(s, &near) || here.as_deref().is_some_and(|h| mcp::runs_folder_file(s, h));
        if runs {
            tracing::warn!(server = %s.name, "not starting an MCP server that runs a file from a folder an isolated bot used");
        }
        !runs
    });
    if !servers.iter().any(|s| s.transport == "stdio") {
        return None;
    }
    let dir = mcp_cwd(&state.config.data_dir, token);
    let private = || {
        #[cfg_attr(not(unix), allow(unused_mut))]
        let mut b = std::fs::DirBuilder::new();
        #[cfg(unix)]
        std::os::unix::fs::DirBuilderExt::mode(&mut b, 0o700);
        b
    };
    let parent = dir.parent().unwrap_or(&dir);
    // The turn's own folder isn't made recursively: it must be new, never one already there
    let made = private().recursive(true).create(parent).and_then(|()| private().create(&dir));
    if let Err(e) = made {
        tracing::warn!(error = %e, "couldn't make an empty folder for MCP servers; not starting the stdio ones");
        servers.retain(|s| s.transport != "stdio");
        return None;
    }
    for s in servers.iter_mut() {
        mcp::start_in(s, &dir);
    }
    Some(dir)
}

/// Removes [`mcp_cwd`] for `token`, if it was made.
pub(crate) async fn remove_mcp_cwd(state: &AppState, token: &str) {
    let dir = mcp_cwd(&state.config.data_dir, token);
    if let Err(e) = tokio::fs::remove_dir_all(&dir).await {
        if e.kind() != std::io::ErrorKind::NotFound {
            tracing::warn!(error = %e, dir = %dir.display(), "couldn't remove an MCP server folder");
        }
    }
}

fn rpc_result(id: Value, result: Value) -> Response {
    Json(json!({"jsonrpc": "2.0", "id": id, "result": result})).into_response()
}

fn rpc_error(id: Value, code: i64, message: &str) -> Response {
    Json(json!({"jsonrpc": "2.0", "id": id, "error": {"code": code, "message": message}})).into_response()
}

/// A request for the side-effect ledger only (the call never reaches the approver).
fn noted_request(session: &PermissionSession, tool_name: &str, server: &str, tool: &str, input: &Value, builtin: Option<mcp::BuiltinCall>) -> ToolApprovalRequest {
    ToolApprovalRequest {
        kwant_id: session.kwant_id.clone(),
        kwant_name: session.kwant_name.clone(),
        turn_id: session.turn_id.clone(),
        tool_name: tool_name.to_string(),
        server: server.to_string(),
        tool: tool.to_string(),
        arguments: input.clone(),
        builtin,
        smart: None,
        payment: None,
    }
}

/// Answers one claude permission prompt. MCP tools from the kwant's own servers get their
/// approval card; claude's own tools follow [`mcp::builtin_tools`]; tools from servers the
/// kwant doesn't have are denied.
async fn decide(session: &PermissionSession, args: &Value, smart: bool, grant: Option<&mcp::PrepushGrant>) -> Value {
    let tool_name = args["tool_name"].as_str().unwrap_or("");
    let input = args.get("input").cloned().unwrap_or_else(|| json!({}));
    let tool_use_id = args["tool_use_id"].as_str().unwrap_or("").to_string();

    let mut verdict = None;
    let (server, tool, builtin) = match mcp::split_tool_name(tool_name) {
        Some((server, tool)) if session.ctx.servers.iter().any(|s| s.name == server) => {
            // Read-only tools run under "Ask only when risky": ask the server through the
            // gateway's connections (kwnt's own tools never need a card anyway)
            if smart && server != mcp::SELF_SERVER_NAME {
                let read_only = match session.tool_session().await {
                    Some(tools) => tools.read_only(tool_name),
                    None => None,
                };
                verdict = Some(match read_only {
                    Some(read_only) => mcp::session::read_only_verdict(read_only),
                    None => mcp::SmartVerdict::Ask("kwnt couldn't check whether this MCP tool is read-only".into()),
                });
            }
            // 26f: a call that names a lock, a door or an alarm needs to know what the server is;
            // kwnt connects to it once in this turn to hear it say so
            if server != mcp::SELF_SERVER_NAME
                && mcp::home_floor::sensitive_args(&input)
                && session.ctx.servers.iter().any(|s| s.name == server && mcp::home_floor::seen(&s.id).is_none())
            {
                let _ = session.tool_session().await;
            }
            (server, tool, None)
        }
        _ if tool_name.is_empty() || tool_name.starts_with("mcp__") => {
            if !tool_name.is_empty() {
                let req = noted_request(session, tool_name, "", tool_name, &input, None);
                session.ctx.approver.note(&req, "denied", None).await;
            }
            return json!({"behavior": "deny", "message": format!("{tool_name} is not available to this bot.")});
        }
        _ => {
            let folder = mcp::builtin_tools::canonical_folder(session.cwd.as_deref());
            // A WebFetch check may resolve a host name: keep that off the async workers
            let (decision, smart_verdict) = {
                let (tool, input, folder) = (tool_name.to_string(), input.clone(), folder.clone());
                tokio::task::spawn_blocking(move || {
                    let decision = mcp::builtin_tools::decide(&tool, &input, folder.as_deref());
                    let ask = matches!(decision, mcp::builtin_tools::BuiltinDecision::Ask { .. });
                    let verdict = (smart && ask).then(|| mcp::builtin_tools::smart_verdict(&tool, &input, folder.as_deref()));
                    (decision, verdict)
                })
                .await
                .unwrap_or((mcp::builtin_tools::BuiltinDecision::Ask { outside_folder: false }, None))
            };
            match decision {
                mcp::builtin_tools::BuiltinDecision::Allow => {
                    let cwd = folder.or_else(|| session.cwd.clone()).map(|f| f.display().to_string());
                    let call = mcp::BuiltinCall { cwd, outside_folder: false };
                    let req = noted_request(session, tool_name, "", tool_name, &input, Some(call.clone()));
                    // Flight School: a tier below act sends even an in-folder write to the approver
                    if session.ctx.approver.tier_gate(&req).await.is_some() {
                        ("", tool_name, Some(call))
                    } else {
                        // No card and no approval record, but the task's ledger still gets it
                        session.ctx.approver.note(&req, "allowed", None).await;
                        return json!({"behavior": "allow", "updatedInput": input});
                    }
                }
                mcp::builtin_tools::BuiltinDecision::Ask { outside_folder } => {
                    if smart {
                        verdict = Some(smart_verdict.unwrap_or_else(|| mcp::SmartVerdict::Ask("kwnt couldn't finish checking this call".into())));
                    }
                    let cwd = folder.or_else(|| session.cwd.clone()).map(|f| f.display().to_string());
                    ("", tool_name, Some(mcp::BuiltinCall { cwd, outside_folder }))
                }
            }
        }
    };
    let req = ToolApprovalRequest {
        kwant_id: session.kwant_id.clone(),
        kwant_name: session.kwant_name.clone(),
        turn_id: session.turn_id.clone(),
        tool_name: tool_name.to_string(),
        server: server.to_string(),
        tool: tool.to_string(),
        arguments: input.clone(),
        builtin,
        smart: verdict,
        payment: None,
    };
    // A push the person's Retry or Override covers skips the ordinary card (0.9.7)
    // 29o: and a Deny may come with the person's reason
    let approval = mcp::hear_deny_reason(async {
        match grant {
            Some(g) => session.ctx.approver.decide_granted(req, g).await,
            None => session.ctx.approver.decide(req).await,
        }
    });
    // Dropping the approval withdraws its card when the turn ends first
    let (outcome, reason) = tokio::select! {
        heard = approval => heard,
        _ = session.ended.cancelled() => {
            session.ctx.record(cancelled_record(&tool_use_id, tool_name, server, tool, &input));
            return json!({"behavior": "deny", "message": "The turn ended before the user answered."});
        }
    };
    // 26b: the turn's first unanswered (or otherwise failed) call carries the one reminder; a
    // plain denial doesn't
    let remind = !outcome.allowed()
        && mcp::reminds(outcome.record_status())
        && !session.ctx.had_miss();
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
    } else if remind {
        json!({"behavior": "deny", "message": mcp::with_no_false_claims(&outcome.refusal(reason.as_deref()))})
    } else {
        json!({"behavior": "deny", "message": outcome.refusal(reason.as_deref())})
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
            let smart = state.db.kwant_approvals(&session.kwant_id).await.is_ok_and(|m| m.is_smart());
            let args = &msg["params"]["arguments"];
            // B2: a bot's own push in a Dev mode space waits for its pre-push card, before any
            // allow rule (each Bash call starts in the bot's folder)
            let tool_name = args["tool_name"].as_str().unwrap_or("");
            let prepush = match (tool_name, args["input"]["command"].as_str(), mcp::builtin_tools::canonical_folder(session.cwd.as_deref())) {
                ("Bash", Some(command), Some(folder)) => super::dev::prepush::check_command(&state, &session.kwant_id, command, &folder).await,
                // claude calls the bot's MCP servers itself: a write to a GitHub branch asks here (6a)
                _ => match mcp::split_tool_name(tool_name).filter(|(server, _)| session.ctx.servers.iter().any(|s| s.name == *server)) {
                    Some((server, tool)) => Box::pin(super::dev::prepush::check_mcp(&state, &session.kwant_id, server, tool, &args["input"])).await,
                    None => Ok(Default::default()),
                },
            };
            // 28k: the gate's refusal is its own text to the bot, and "Blocked before push" on the step.
            // 28q: keyed to this call's own tool-use id (the id claude puts on the step), so a
            // neighbouring call of the same tool that failed on its own keeps its own reason
            let call = args["tool_use_id"].as_str();
            let held = || super::activity::refused(&session.turn_id, tool_name, call, &args["input"], mcp::PREPUSH_BLOCKED_STEP_DETAIL);
            let decision = match prepush {
                Err(why) => {
                    held();
                    json!({"behavior": "deny", "message": why})
                }
                Ok(pass) => {
                    let mut decision = decide(&session, args, smart, pass.grant.as_ref()).await;
                    if decision["behavior"] == "allow" {
                        // The override goes with the push that runs, never one the person declined
                        if let Some(why) = super::dev::prepush::use_up(&state, &pass).await {
                            held();
                            decision = json!({"behavior": "deny", "message": why});
                        } else if let (Some(pinned), Some(input)) = (pass.pinned, decision.get_mut("updatedInput").and_then(Value::as_object_mut)) {
                            // What goes is the commit kwant checked, even if a parallel call commits
                            input.insert("command".into(), Value::String(pinned));
                        }
                    }
                    decision
                }
            };
            rpc_result(id, json!({"content": [{"type": "text", "text": decision.to_string()}]}))
        }
        "tools/call" => rpc_error(id, -32602, "unknown tool"),
        _ => rpc_error(id, -32601, "method not found"),
    }
}

/// 26h D3: the largest hook payload kwnt reads. A claude PostToolUse for an MCP tool carries the
/// pictures it returned as base64 (a 20 MB image is ~27 MB); past this the hook gets a 413, which
/// its `curl -f` turns into no output, and claude keeps its own output (nothing is saved).
pub const HOOK_BODY_LIMIT: usize = 64 * 1024 * 1024;

/// `POST /api/mcp/permission/:token/hook`: claude's PreToolUse, PostToolUse and
/// PostToolUseFailure hooks for this turn's built-in tools ([`mcp::cli_hooks`]). Answers a
/// PreToolUse deny when the same call already failed 3 times the same way, and nothing otherwise
/// (204). An unknown token is a 404, which the hook's `curl -f` turns into no output: the call runs.
/// 26h D3: a PostToolUse for one of the bot's MCP tools saves the files in its result in the
/// turn's chat ([`mcp::media::finish_claude`]) and answers the text the model reads instead.
pub async fn permission_hook_endpoint(State(state): State<AppState>, Path(token): Path<String>, Json(msg): Json<Value>) -> Response {
    let session = state.mcp_sessions.lock().await.get(&token).cloned();
    let Some(session) = session else {
        return StatusCode::NOT_FOUND.into_response();
    };
    if let Some((server, tool)) = mcp::cli_hooks::mcp_call(&msg) {
        let Some(cfg) = session.ctx.servers.iter().find(|s| s.name == server && mcp::cli_hooks::saves_media_for(server)) else {
            return StatusCode::NO_CONTENT.into_response();
        };
        let dir = mcp::cli_hooks::tool_results_dir(&msg);
        let media = session.ctx.media.as_deref();
        return match mcp::media::finish_claude(&msg["tool_response"], server, cfg.url.as_deref(), tool, media, dir.as_deref()).await {
            Some(text) => {
                tracing::info!(kwant = %session.kwant_id, turn = %session.turn_id, %server, %tool, "saved a claude MCP tool's files in the chat");
                Json(mcp::cli_hooks::replace_output(&text)).into_response()
            }
            None => StatusCode::NO_CONTENT.into_response(),
        };
    }
    match mcp::cli_hooks::on_event(&session.repeats, &msg) {
        Some(out) => {
            tracing::info!(kwant = %session.kwant_id, turn = %session.turn_id, "repeat guard: a claude tool call was denied");
            Json(out).into_response()
        }
        None => StatusCode::NO_CONTENT.into_response(),
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

/// The engine's own id for a gateway `tools/call` (28q): Claude Code sends its tool-use id, which
/// its activity step carries, as `_meta["claudecode/toolUseId"]`. `None` when the engine sends none.
fn engine_call_id(msg: &Value) -> Option<&str> {
    msg["params"]["_meta"]["claudecode/toolUseId"].as_str().map(str::trim).filter(|id| !id.is_empty())
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
            let step_id = engine_call_id(&msg);
            let Some(tools) = session.tool_session().await else {
                return StatusCode::UNAUTHORIZED.into_response();
            };
            // An unknown grok name stays as given, and the call fails as an unknown tool
            let tool_name = tools
                .canonical_name(naming, name)
                .unwrap_or_else(|| name.to_string());
            // 29g: grok sends no id; its step carries the grok name, which only this map can read
            let called_as = (naming == GatewayNaming::Grok).then_some(name);
            // Dropping the call withdraws its card (or stops the forwarded call) when the turn ends
            let (caller, raw_arguments) = (session.caller(), arguments.to_string());
            let outcome = tokio::select! {
                outcome = tools.call_step(&caller, &call_id, step_id, called_as, &tool_name, &raw_arguments) => outcome,
                _ = session.ended.cancelled() => {
                    let (server, tool) = mcp::split_tool_name(&tool_name).unwrap_or(("", name));
                    session.ctx.record(cancelled_record(&call_id, &tool_name, server, tool, &arguments));
                    drop(tools);
                    return rpc_result(id, json!({"content": [{"type": "text", "text": "The turn ended before the call finished."}], "isError": true}));
                }
            };
            // The chat's failure line (0.9.5): codex and grok calls are reported here, by their real server
            // A call the repeat guard (15c) didn't send isn't new news about the connection
            let sent = outcome.record.get("repeat_stopped").is_none();
            if let Some(w) = super::tool_failures::TurnWatch::find(&session.turn_id).filter(|_| sent) {
                match outcome.status {
                    "ok" => w.call_done(&tool_name, &arguments, None).await,
                    "error" => w.call_done(&tool_name, &arguments, Some(&outcome.text)).await,
                    _ => {}
                }
            }
            rpc_result(
                id,
                json!({"content": [{"type": "text", "text": outcome.model_text()}], "isError": outcome.status != "ok"}),
            )
        }
        _ => rpc_error(id, -32601, "method not found"),
    }
}

#[cfg(test)]
mod expired_tests {
    use super::*;

    #[test]
    fn an_expired_card_says_what_didnt_run_and_what_resumes_it() {
        assert_eq!(
            expired_text("Ben", "Bash", 24),
            "Ben stopped waiting for your OK to use Bash (no answer in 24 h), so that step didn't run. Retry asks you again."
        );
        let p = expired_retry_prompt("Bash", &json!({"command": "ls"}));
        assert!(p.starts_with("[The person pressed Retry on a tool call that waited too long for their OK]\nTool: Bash"), "{p}");
        assert!(p.contains("\"command\": \"ls\""), "{p}");
        assert!(p.contains("fresh card"), "Retry grants nothing: {p}");
    }

    #[test]
    fn an_expired_card_with_no_chat_tells_the_bots_own_chat() {
        let p = expired_place(&Value::Null, "k1");
        assert_eq!((p.direct_kwant_id.as_deref(), p.channel_id, p.thread_id), (Some("k1"), None, None));
        let p = expired_place(&json!({"thread_id": "t1"}), "k1");
        assert_eq!((p.direct_kwant_id, p.thread_id.as_deref()), (None, Some("t1")));
        let p = expired_place(&json!({"channel_id": "c1"}), "k1");
        assert_eq!((p.direct_kwant_id, p.channel_id.as_deref()), (None, Some("c1")));
    }
}

#[cfg(test)]
mod card_settles_tests {
    use super::*;

    /// 28k: only the person's Deny settles a card `declined`. A card that was withdrawn, or whose
    /// channel dropped, settles `stopped`: nobody pressed Deny. The bot's outcome is `Denied`
    /// either way (nothing runs), and it hears the same text.
    #[tokio::test]
    async fn a_withdrawn_card_settles_stopped_and_only_a_deny_settles_declined() {
        let broker = PermissionBroker::new();
        let ask = |id: &str| ApprovalRequest {
            id: id.into(),
            kwant_id: "k1".into(),
            tool_name: "Bash".into(),
            details: Value::Null,
        };
        let wait = |rx: tokio::sync::oneshot::Receiver<ApprovalDecision>| tokio::time::timeout(Duration::from_secs(5), rx);

        // Withdrawn: the broker drops the ticket with no decision
        let rx = broker.request_approval(ask("apr_withdrawn")).await;
        assert!(broker.cancel("apr_withdrawn").await);
        let withdrawn = wait(rx).await;
        assert!(matches!(withdrawn, Ok(Err(_))), "the waiter sees the channel close");
        assert_eq!(card_settles(&withdrawn), Some("stopped"));

        // The person pressed Deny
        let rx = broker.request_approval(ask("apr_deny")).await;
        assert!(broker.resolve("apr_deny", ApprovalDecision::Deny).await);
        assert_eq!(card_settles(&wait(rx).await), Some("declined"));

        // Allow, once or for the session
        for d in [ApprovalDecision::AllowOnce, ApprovalDecision::AlwaysAllowSession] {
            let rx = broker.request_approval(ask("apr_ok")).await;
            assert!(broker.resolve("apr_ok", d).await);
            assert_eq!(card_settles(&wait(rx).await), Some("ok"));
        }

        // The wait ran out: the timeout path settles it, not this
        let rx = broker.request_approval(ask("apr_slow")).await;
        let ran_out = tokio::time::timeout(Duration::from_millis(1), rx).await;
        assert!(ran_out.is_err());
        assert_eq!(card_settles(&ran_out), None);

        // What the bot hears doesn't change: a withdrawn card and a Deny are both `Denied`
        assert_eq!(ApprovalOutcome::Denied.refusal_text(), mcp::DENIED_TEXT);
        assert_eq!(ApprovalOutcome::Denied.record_status(), "denied");
    }
}

#[cfg(test)]
mod engine_call_id_tests {
    use super::*;

    /// 28q: the gateway keys a held push's step note to Claude Code's tool-use id, which it sends
    /// on `tools/call` as `_meta["claudecode/toolUseId"]`; an engine that sends none gives `None`.
    #[test]
    fn the_gateway_reads_the_engines_own_call_id() {
        let call = |meta: Value| json!({"jsonrpc": "2.0", "id": 3, "method": "tools/call", "params": {"name": "github__push", "arguments": {}, "_meta": meta}});
        assert_eq!(engine_call_id(&call(json!({"claudecode/toolUseId": "toolu_01AbC"}))), Some("toolu_01AbC"));
        assert_eq!(engine_call_id(&call(json!({"progressToken": 1}))), None);
        assert_eq!(engine_call_id(&call(json!({"claudecode/toolUseId": " "}))), None);
        assert_eq!(engine_call_id(&call(json!({"claudecode/toolUseId": 7}))), None);
        assert_eq!(engine_call_id(&json!({"method": "tools/call", "params": {"name": "github__push"}})), None);
    }
}

/// 27b: after the person declines a card, a call of the same family in that chat is refused
/// without a card until they write there again.
#[cfg(test)]
mod declined_tests {
    use super::*;
    use crate::config::Config;
    use crate::db::{models::NewKwant, spaces::SpaceId};
    use crate::router::LayaRouter;
    use crate::tools::ToolRunner;
    use axum::body::{to_bytes, Body};
    use tempfile::{tempdir, TempDir};
    use tower::ServiceExt;

    struct Env {
        state: AppState,
        bot: Kwant,
        general: String,
        _tmp: TempDir,
    }

    async fn setup() -> Env {
        let tmp = tempdir().unwrap();
        let db = Database::init(tmp.path()).await.unwrap();
        let config = Config { data_dir: tmp.path().to_path_buf(), ..Config::default() };
        let state = AppState::new(
            config.clone(),
            db,
            ToolRunner::new(tmp.path().join("workspace")),
            PermissionBroker::new(),
            LayaRouter::new(config.laya_url.clone()),
        );
        let space = SpaceId::default_space();
        // The space's one MCP server is a plain http one, so a turn is set up the same on every
        // OS: the seeded stdio servers can all be dropped by `isolate_stdio_servers` (they are on
        // Windows), and with no server left `prepare_turn_with_opts` returns None
        for s in state.db.list_mcp_servers(&space).await.unwrap() {
            state.db.delete_mcp_server(&s.id).await.unwrap();
        }
        state.db.create_mcp_server(&space, &http_server("tools", "http://127.0.0.1:9/mcp")).await.unwrap();
        let bot = state.db.list_kwants(&space).await.unwrap()[0].clone();
        let general = state.db.list_channels(&space).await.unwrap().into_iter().find(|c| c.name == "general").unwrap().id;
        Env { state, bot, general, _tmp: tmp }
    }

    fn http_server(name: &str, url: &str) -> McpServer {
        McpServer {
            id: format!("s-{name}"),
            name: name.into(),
            transport: "http".into(),
            command: None,
            args: vec![],
            url: Some(url.into()),
            env: Default::default(),
            headers: Default::default(),
            enabled: true,
            timeout_secs: 10,
            created_at: None,
        }
    }

    async fn other_bot(e: &Env) -> Kwant {
        let input = NewKwant {
            name: "Other".into(),
            title: "Tester".into(),
            persona_prompt: String::new(),
            soul: String::new(),
            avatar_sprite: "s".into(),
            driver_type: "cli".into(),
            preferred_model: "codex".into(),
        };
        e.state.db.create_kwant(&SpaceId::default_space(), &input).await.unwrap()
    }

    fn approver(e: &Env, conversation: &str, timeout: Duration) -> BrokerApprover {
        BrokerApprover {
            db: e.state.db.clone(),
            data_dir: e.state.config.data_dir.clone(),
            broker: e.state.broker.clone(),
            events: e.state.event_sender.clone(),
            timeout,
            conversation: conversation.to_string(),
            chat: None,
            always_ask: false,
            strict_ask: false,
            started_by: None,
        }
    }

    fn bash(bot: &Kwant, command: &str) -> ToolApprovalRequest {
        ToolApprovalRequest {
            kwant_id: bot.id.clone(),
            kwant_name: bot.name.clone(),
            turn_id: "t-27b".into(),
            tool_name: "Bash".into(),
            server: String::new(),
            tool: "Bash".into(),
            arguments: json!({"command": command}),
            builtin: Some(mcp::BuiltinCall { cwd: None, outside_folder: false }),
            smart: None,
            payment: None,
        }
    }

    /// How a card was answered in [`ask`].
    #[derive(Clone)]
    enum Answer {
        Decide(ApprovalDecision),
        /// Its channel goes away with no decision.
        Drop,
        /// Nobody answers before the approver stops waiting.
        Nothing,
        /// 29o: answered through the app's route, `{decision, reason}` as the card sends it.
        Route(&'static str, Option<&'static str>),
    }

    /// 29o: the person answers card `id` through the app's route with `{decision, reason}`:
    /// the route's reply.
    async fn answer_route(state: &AppState, id: &str, decision: &str, reason: Option<&str>) -> Value {
        let mut body = json!({"decision": decision});
        if let Some(r) = reason {
            body["reason"] = json!(r);
        }
        let payload = serde_json::from_value(body).expect("the card's body parses");
        let resp = super::super::routes::resolve_approval(State(state.clone()), Path(id.to_string()), Json(payload))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        serde_json::from_slice(&to_bytes(resp.into_body(), usize::MAX).await.unwrap()).unwrap()
    }

    /// 29o: what the bot hears when the person said no with `reason` (the brief's words).
    fn said_why(reason: &str) -> String {
        format!(
            "The user denied this tool call. They said why: \"{reason}\". Don't make this same call again. \
If what they said points to another way, you may try that; it will ask them again. Otherwise say in your reply what you couldn't do."
        )
    }

    const fn why(reason: &'static str) -> Answer {
        Answer::Route("deny", Some(reason))
    }

    /// `a.decide(req)`, answering the card it shows with `answer`: the outcome and the card's
    /// ticket, `None` when no card went out. Like both real callers (the MCP session and the
    /// permission prompt) it listens for a Deny's reason ([`mcp::hear_deny_reason`]).
    async fn ask(e: &Env, a: &BrokerApprover, req: ToolApprovalRequest, answer: Answer) -> (ApprovalOutcome, Option<String>) {
        let (outcome, card, _) = asked(e, a, req, answer, true).await;
        (outcome, card)
    }

    /// [`ask`] from a caller that doesn't listen for a Deny's reason: the bot would hear the
    /// plain `DENIED_TEXT`.
    async fn ask_unheard(e: &Env, a: &BrokerApprover, req: ToolApprovalRequest, answer: Answer) -> (ApprovalOutcome, Option<String>) {
        let (outcome, card, _) = asked(e, a, req, answer, false).await;
        (outcome, card)
    }

    /// [`ask`], also giving the reason the call heard (`None` when `listening` is false).
    async fn asked(
        e: &Env,
        a: &BrokerApprover,
        req: ToolApprovalRequest,
        answer: Answer,
        listening: bool,
    ) -> (ApprovalOutcome, Option<String>, Option<String>) {
        let mut events = e.state.event_sender.subscribe();
        let broker = e.state.broker.clone();
        let decide = async move {
            if listening {
                mcp::hear_deny_reason(a.decide(req)).await
            } else {
                (a.decide(req).await, None)
            }
        };
        tokio::pin!(decide);
        let mut card = None;
        let outcome = loop {
            tokio::select! {
                out = &mut decide => break out,
                ev = events.recv() => {
                    if let Ok(RuntimeEvent::ApprovalRequired { id, .. }) = ev {
                        assert!(card.is_none(), "two cards for one call");
                        match answer.clone() {
                            Answer::Decide(d) => assert!(broker.resolve(&id, d).await),
                            Answer::Drop => assert!(broker.cancel(&id).await),
                            Answer::Nothing => {}
                            Answer::Route(d, r) => {
                                answer_route(&e.state, &id, d, r).await;
                            }
                        }
                        card = Some(id);
                    }
                }
            }
        };
        while let Ok(ev) = events.try_recv() {
            if let RuntimeEvent::ApprovalRequired { id, .. } = ev {
                panic!("a card went out after the call was settled: {id}");
            }
        }
        let (outcome, heard) = outcome;
        (outcome, card, heard)
    }

    /// The bot's approvals rows: (status, details).
    async fn approvals_of(e: &Env, bot: &Kwant) -> Vec<(String, Value)> {
        let rows = e.state.db.list_approvals(&SpaceId::default_space(), None, Some(&bot.id)).await.unwrap();
        rows.into_iter().map(|r| (r.status, serde_json::from_str(&r.parameters).unwrap())).collect()
    }

    const DENY: Answer = Answer::Decide(ApprovalDecision::Deny);
    const ALLOW: Answer = Answer::Decide(ApprovalDecision::AllowOnce);

    #[tokio::test]
    async fn a_declined_call_s_family_is_refused_without_a_card() {
        let e = setup().await;
        let a = approver(&e, &e.general, Duration::from_secs(10));
        let (out, card) = ask(&e, &a, bash(&e.bot, "gh pr view 12"), DENY).await;
        assert_eq!(out, ApprovalOutcome::Denied);
        let declined = card.expect("the first call asks");
        let rows = approvals_of(&e, &e.bot).await.len();

        // The same family another way: refused at once, no card, and no new ticket to answer
        let (out, card) = ask(&e, &a, bash(&e.bot, "gh pr view --json title"), ALLOW).await;
        assert_eq!(out, ApprovalOutcome::DeclinedBefore);
        assert_eq!(card, None);
        let after = approvals_of(&e, &e.bot).await;
        assert_eq!(after.len(), rows + 1, "one closed record, not a card");
        let (status, details) = after.iter().find(|(_, d)| d["declined_before"].is_string()).expect("the refusal is on record");
        assert_eq!(status, "denied");
        assert_eq!(details["declined_before"], json!(declined));
        assert!(e.state.broker.pending_ids().await.is_empty(), "nothing waits for an answer");

        // Wrapped, piped or behind a cd: still the same family
        for again in ["cd repo && gh pr view 12 | head", "sudo gh -R o/r pr view 13", "bash -c 'gh pr view 14'"] {
            let (out, card) = ask(&e, &a, bash(&e.bot, again), ALLOW).await;
            assert_eq!((out, card), (ApprovalOutcome::DeclinedBefore, None), "{again}");
        }

        // Another family still asks
        let (out, card) = ask(&e, &a, bash(&e.bot, "gh issue list"), ALLOW).await;
        assert_eq!(out, ApprovalOutcome::Allowed);
        assert!(card.is_some(), "gh issue list asks");
    }

    #[tokio::test]
    async fn only_the_person_s_no_is_remembered() {
        let e = setup().await;

        // Nobody answered: the card is parked, and the next call asks again
        let a = approver(&e, &e.general, Duration::from_millis(200));
        let (out, card) = ask(&e, &a, bash(&e.bot, "gh pr view 1"), Answer::Nothing).await;
        assert_eq!((out, card.is_some()), (ApprovalOutcome::Unanswered, true));
        let (out, card) = ask(&e, &a, bash(&e.bot, "gh pr view 2"), Answer::Nothing).await;
        assert_eq!((out, card.is_some()), (ApprovalOutcome::Unanswered, true), "unanswered isn't a no");

        // The card's channel went away (its turn ended): not a no either
        let a = approver(&e, &e.general, Duration::from_secs(10));
        let (out, card) = ask(&e, &a, bash(&e.bot, "docker run x"), Answer::Drop).await;
        assert_eq!((out, card.is_some()), (ApprovalOutcome::Denied, true));
        let (out, card) = ask(&e, &a, bash(&e.bot, "docker run y"), ALLOW).await;
        assert_eq!((out, card.is_some()), (ApprovalOutcome::Allowed, true), "a dropped card isn't a no");

        // kwnt refusing a call that names its own secrets: no card, and nothing remembered
        let (out, card) = ask(&e, &a, bash(&e.bot, "gh api --input ~/.kwnt/api-token user"), ALLOW).await;
        assert_eq!((out, card), (ApprovalOutcome::Denied, None));
        let (out, card) = ask(&e, &a, bash(&e.bot, "gh api user"), ALLOW).await;
        assert_eq!((out, card.is_some()), (ApprovalOutcome::Allowed, true), "kwnt's refusal isn't the person's no");
    }

    #[tokio::test]
    async fn a_no_holds_in_its_own_chat_and_bot_until_the_person_writes_there() {
        let e = setup().await;
        let other = other_bot(&e).await;
        let here = approver(&e, &e.general, Duration::from_secs(10));
        let (out, _) = ask(&e, &here, bash(&e.bot, "npm publish"), DENY).await;
        assert_eq!(out, ApprovalOutcome::Denied);

        // Another conversation with the same bot asks
        let thread = approver(&e, "thread-1", Duration::from_secs(10));
        let (out, card) = ask(&e, &thread, bash(&e.bot, "npm publish --tag next"), DENY).await;
        assert_eq!((out, card.is_some()), (ApprovalOutcome::Denied, true));
        // Another bot in the same conversation asks
        let (out, card) = ask(&e, &here, bash(&other, "npm publish"), ALLOW).await;
        assert_eq!((out, card.is_some()), (ApprovalOutcome::Allowed, true));
        // This bot here doesn't
        let (out, card) = ask(&e, &here, bash(&e.bot, "npm publish --dry-run"), ALLOW).await;
        assert_eq!((out, card), (ApprovalOutcome::DeclinedBefore, None));

        // The person writes to another bot, or to this bot in the thread: still a no here
        person_wrote(&e.state.broker, &other.id, None, e.general.clone());
        person_wrote(&e.state.broker, &e.bot.id, Some("thread-1".into()), e.general.clone());
        let (out, card) = ask(&e, &here, bash(&e.bot, "npm publish"), ALLOW).await;
        assert_eq!((out, card), (ApprovalOutcome::DeclinedBefore, None));
        // The thread's no went with the person's message there
        let (out, card) = ask(&e, &thread, bash(&e.bot, "npm publish"), ALLOW).await;
        assert_eq!((out, card.is_some()), (ApprovalOutcome::Allowed, true));

        // The person writes to this bot here: it asks again
        person_wrote(&e.state.broker, &e.bot.id, None, e.general.clone());
        let (out, card) = ask(&e, &here, bash(&e.bot, "npm publish"), ALLOW).await;
        assert_eq!((out, card.is_some()), (ApprovalOutcome::Allowed, true));
    }

    #[tokio::test]
    async fn the_approver_s_conversation_is_the_one_a_message_clears() {
        let e = setup().await;
        for thread_id in [None, Some("thread-9".to_string())] {
            let turn_id = uuid::Uuid::new_v4().to_string();
            e.state.turns.set_turn(super::super::turns::ActiveTurn {
                turn_id: turn_id.clone(),
                kwant_id: e.bot.id.clone(),
                context_id: e.general.clone(),
                thread_id: thread_id.clone(),
                state: "running".into(),
                since: chrono::Utc::now(),
                purpose: None,
                audit: None,
            });
            assert_eq!(turn_conversation(&e.state, &turn_id), conversation_of(thread_id, e.general.clone()));
            e.state.turns.end_turn(&turn_id);
        }
    }

    /// claude's permission prompt for `tool {input}` in a live turn of the bot in #general, the
    /// card (if any) answered with `answer`: the permission tool's reply.
    async fn claude_asks(e: &Env, tool: &str, input: Value, answer: Answer) -> (Value, Option<String>) {
        let turn_id = uuid::Uuid::new_v4().to_string();
        let folder = tempdir().unwrap();
        // The turn is registered before its MCP context is built, as `turn_once` does: the
        // approver takes its conversation from the registry then, and an unregistered turn is a
        // conversation of its own (`turn:<id>`), so two asks wouldn't share the person's no
        e.state.turns.set_turn(super::super::turns::ActiveTurn {
            turn_id: turn_id.clone(),
            kwant_id: e.bot.id.clone(),
            context_id: e.general.clone(),
            thread_id: None,
            state: "running".into(),
            since: chrono::Utc::now(),
            purpose: None,
            audit: None,
        });
        let (_, token) = prepare_turn_with_opts(&e.state, &e.bot, Some(turn_id.clone()), Some(folder.path().to_path_buf()), None, Default::default())
            .await
            .unwrap();
        let app = super::super::create_router(e.state.clone());
        let body = json!({"jsonrpc": "2.0", "id": 1, "method": "tools/call", "params": {"name": "approve",
            "arguments": {"tool_name": tool, "input": input, "tool_use_id": "tu_1"}}});
        let request = axum::http::Request::builder()
            .method("POST")
            .uri(format!("/api/mcp/permission/{token}"))
            .header("content-type", "application/json")
            .body(Body::from(body.to_string()))
            .unwrap();
        let mut events = e.state.event_sender.subscribe();
        let prompt = app.oneshot(request);
        tokio::pin!(prompt);
        let mut card = None;
        let resp = loop {
            tokio::select! {
                resp = &mut prompt => break resp.unwrap(),
                ev = events.recv() => {
                    if let Ok(RuntimeEvent::ApprovalRequired { id, .. }) = ev {
                        match answer.clone() {
                            Answer::Decide(d) => assert!(e.state.broker.resolve(&id, d).await),
                            Answer::Route(d, r) => {
                                answer_route(&e.state, &id, d, r).await;
                            }
                            Answer::Drop | Answer::Nothing => {}
                        }
                        card = Some(id);
                    }
                }
            }
        };
        let bytes = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
        let v: Value = serde_json::from_slice(&bytes).unwrap();
        finish_turn(&e.state, &token).await;
        e.state.turns.end_turn(&turn_id);
        let text = v["result"]["content"][0]["text"].as_str().unwrap_or_else(|| panic!("{v}"));
        (serde_json::from_str(text).unwrap(), card)
    }

    #[tokio::test]
    async fn claude_is_told_it_was_already_declined() {
        let e = setup().await;
        let (reply, card) = claude_asks(&e, "Bash", json!({"command": "git push origin main"}), DENY).await;
        assert!(card.is_some());
        assert_eq!(reply["behavior"], "deny");
        let (reply, card) = claude_asks(&e, "Bash", json!({"command": "git push --force origin main"}), ALLOW).await;
        assert_eq!(card, None, "no card the second time");
        assert_eq!(reply, json!({"behavior": "deny", "message": mcp::DECLINED_BEFORE_TEXT}));
    }

    /// 29o (a, b, c) on claude's permission prompt: the card's `{decision: "deny", reason}` reaches
    /// the bot as one trimmed line of at most 500 chars, and the same family asks again; the old
    /// body, an empty reason and a plain Deny still tell it `DENIED_TEXT` word for word, and only
    /// the plain Deny is remembered (27b).
    #[tokio::test]
    async fn claude_hears_why_the_person_said_no() {
        let e = setup().await;
        let denied = |message: String| json!({"behavior": "deny", "message": message});
        let push = |args: &str| json!({"command": format!("git push {args}")});
        let (reply, card) = claude_asks(&e, "Bash", push("--force origin main"), why("  push without\n--force \r\n")).await;
        assert!(card.is_some());
        assert_eq!(reply, denied(said_why("push without --force")));

        // Capped at 500 chars, on a char boundary
        let long: &'static str = "é".repeat(501).leak();
        let (reply, card) = claude_asks(&e, "Bash", push("origin main"), why(long)).await;
        assert!(card.is_some(), "a no with a reason isn't remembered: the same family asks again");
        assert_eq!(reply, denied(said_why(&"é".repeat(500))));

        // Nothing left of the reason, or the old body: a plain Deny, remembered as before (27b)
        let (reply, card) = claude_asks(&e, "Bash", push("origin dev"), why("  \n\t ")).await;
        assert!(card.is_some());
        assert_eq!(reply, denied(mcp::DENIED_TEXT.to_string()));
        let (reply, card) = claude_asks(&e, "Bash", json!({"command": "gh pr merge 7"}), Answer::Route("deny", None)).await;
        assert!(card.is_some());
        assert_eq!(reply, denied(mcp::DENIED_TEXT.to_string()));
        for command in ["git push origin main", "gh pr merge 8"] {
            let (reply, card) = claude_asks(&e, "Bash", json!({"command": command}), ALLOW).await;
            assert_eq!(card, None, "{command}");
            assert_eq!(reply, denied(mcp::DECLINED_BEFORE_TEXT.to_string()), "{command}");
        }

        // A reason on an Allow is ignored: the call runs
        let (reply, card) = claude_asks(&e, "Bash", json!({"command": "npm publish"}), Answer::Route("allow_once", Some("fine"))).await;
        assert!(card.is_some());
        assert_eq!(reply["behavior"], "allow");
        assert_eq!(e.state.broker.deny_reasons_held(), 0, "nothing left behind");
    }

    /// 29o (c) on the approver itself: after a no with a reason, a call of the same family gets
    /// its own card, so nothing runs without the person's yes.
    #[tokio::test]
    async fn a_no_with_a_reason_lets_the_adjusted_call_ask() {
        let e = setup().await;
        let a = approver(&e, &e.general, Duration::from_secs(10));
        let (out, card, heard) = asked(&e, &a, bash(&e.bot, "git push --force origin main"), why("push without --force"), true).await;
        assert_eq!((out, card.is_some()), (ApprovalOutcome::Denied, true));
        assert_eq!(heard.as_deref(), Some("push without --force"), "the waiting call hears the reason");
        let (out, card) = ask(&e, &a, bash(&e.bot, "git push origin main"), ALLOW).await;
        assert_eq!((out, card.is_some()), (ApprovalOutcome::Allowed, true), "the adjusted call asks, and the person allows it");
        let (out, card) = ask(&e, &a, bash(&e.bot, "git push origin main"), DENY).await;
        assert_eq!((out, card.is_some()), (ApprovalOutcome::Denied, true));
        let (out, card) = ask(&e, &a, bash(&e.bot, "git push origin dev"), ALLOW).await;
        assert_eq!((out, card), (ApprovalOutcome::DeclinedBefore, None), "a plain no still stays a no");
        assert_eq!(e.state.broker.deny_reasons_held(), 0);
    }

    /// 29o round 2: a reason no call listens for reaches nobody (the bot hears the plain
    /// `DENIED_TEXT`), so the no is remembered as a plain one: the same family isn't asked again.
    #[tokio::test]
    async fn a_reason_nobody_hears_stays_a_plain_no() {
        let e = setup().await;
        let a = approver(&e, &e.general, Duration::from_secs(10));
        let (out, card) = ask_unheard(&e, &a, bash(&e.bot, "git push --force origin main"), why("push without --force")).await;
        assert_eq!((out, card.is_some()), (ApprovalOutcome::Denied, true));
        let (out, card) = ask(&e, &a, bash(&e.bot, "git push origin main"), ALLOW).await;
        assert_eq!((out, card), (ApprovalOutcome::DeclinedBefore, None), "an unheard reason is a plain no");
        assert_eq!(e.state.broker.deny_reasons_held(), 0, "nothing left behind");
    }

    /// 29s: the reason a person gives with Deny shows in Details: once the waiting call hears it,
    /// the card's approval step ends `declined` with `You said why: “…”`, redacted (a token in the
    /// reason never shows). A reason nobody heard, a plain Deny and an Allow add nothing.
    #[tokio::test]
    async fn a_heard_deny_reason_is_the_cards_detail_in_details() {
        use crate::drivers::events::TurnTag;
        use crate::server::activity::TurnActivity;
        let e = setup().await;
        let a = approver(&e, &e.general, Duration::from_secs(10));
        let tag = TurnTag { turn_id: "t-29s".into(), kwant_id: e.bot.id.clone(), context_id: "c".into(), thread_id: None };
        let turn = TurnActivity::start(broadcast::channel(64).0, tag);
        let on_turn = |command: &str| ToolApprovalRequest { turn_id: "t-29s".into(), ..bash(&e.bot, command) };
        let token = "ghp_abcdefghijklmnopqrstuvwxyz0123";
        let reason: &'static str = Box::leak(format!("push without --force, my token {token} is fine").into_boxed_str());
        let (out, _, heard) = asked(&e, &a, on_turn("git push --force origin main"), why(reason), true).await;
        assert_eq!((out, heard.as_deref()), (ApprovalOutcome::Denied, Some(reason)), "the call hears it whole");
        let (out, _) = ask_unheard(&e, &a, on_turn("rm -rf build"), why("keep the build folder")).await;
        assert_eq!(out, ApprovalOutcome::Denied);
        let (out, _) = ask(&e, &a, on_turn("gh pr merge 7"), DENY).await;
        assert_eq!(out, ApprovalOutcome::Denied);
        let (out, _) = ask(&e, &a, on_turn("npm run build"), ALLOW).await;
        assert_eq!(out, ApprovalOutcome::Allowed);
        let saved = turn.finish().unwrap();
        let cards: Vec<(String, Option<String>)> = saved["steps"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|s| s["kind"] == "approval")
            .map(|s| (s["status"].as_str().unwrap().to_string(), s["detail"].as_str().map(str::to_string)))
            .collect();
        assert_eq!(
            cards,
            vec![
                (
                    "declined".to_string(),
                    Some("You said why: \u{201c}push without --force, my token [redacted] is fine\u{201d}".to_string())
                ),
                ("declined".to_string(), None),
                ("declined".to_string(), None),
                ("ok".to_string(), None),
            ],
            "{saved}"
        );
        assert!(!saved.to_string().contains(token), "{saved}");
        assert!(!saved.to_string().contains("keep the build folder"), "an unheard reason shows nothing new");
        assert_eq!(e.state.broker.deny_reasons_held(), 0, "nothing left behind");
    }

    /// 29o (d): a late Deny with a reason (its turn stopped waiting) answers as a plain late Deny
    /// does, and keeps nothing for a call that will never come.
    #[tokio::test]
    async fn a_late_deny_with_a_reason_answers_as_today() {
        let e = setup().await;
        let a = approver(&e, &e.general, Duration::from_millis(200));
        let mut bodies = vec![];
        for (n, reason) in [(1, None), (2, Some("use staging"))] {
            let (out, card) = ask(&e, &a, bash(&e.bot, &format!("gh pr merge {n}")), Answer::Nothing).await;
            assert_eq!(out, ApprovalOutcome::Unanswered);
            let card = card.unwrap();
            let mut body = answer_route(&e.state, &card, "deny", reason).await;
            assert_eq!(body["ticket_id"], json!(card));
            body.as_object_mut().unwrap().remove("ticket_id");
            bodies.push(body);
        }
        assert_eq!(bodies[0], bodies[1]);
        assert_eq!(bodies[1], json!({"resolved": true, "status": "denied", "resumed": false}));
        assert_eq!(e.state.broker.deny_reasons_held(), 0, "nothing kept for a call that won't come");
    }

    /// 29o (b) on kwnt's own tool calls (an API engine, codex, grok and claude through the
    /// gateway): the bot's tool result says why the person said no; a plain Deny still says
    /// `DENIED_TEXT` (`an_api_bot_s_tool_result_says_it_was_already_declined`).
    #[tokio::test]
    async fn an_api_bot_s_tool_result_says_why_the_person_said_no() {
        let e = setup().await;
        let (_, token) = prepare_turn_with_opts(&e.state, &e.bot, Some("t-host-why".into()), None, None, Default::default()).await.unwrap();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let app = super::super::create_router(e.state.clone());
        let serve = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let server = http_server("perm", &format!("http://{addr}/api/mcp/permission/{token}"));
        let ctx = McpTurnContext {
            servers: vec![server],
            approver: Arc::new(approver(&e, &e.general, Duration::from_secs(10))),
            config_dir: std::env::temp_dir(),
            permission_url: None,
            gateway: None,
            max_tool_iterations: 5,
            approvals: mcp::Approvals::Ask,
            records: Default::default(),
            kwnt_only: false,
            media: None,
        };
        let (session, problems) = McpToolSession::open(&ctx, None).await;
        assert!(problems.is_empty(), "{problems:?}");
        let caller = Caller { kwant_id: e.bot.id.clone(), kwant_name: e.bot.name.clone(), turn_id: "t-api-why".into() };

        let mut events = e.state.event_sender.subscribe();
        let mut cards = 0;
        for (n, reason) in [(1, "use the read-only tool\ninstead"), (2, "ask Ada first")] {
            let call_id = format!("c{n}");
            let call = session.call(&caller, &call_id, "mcp__perm__approve", r#"{"tool_name": "Bash", "input": {}}"#);
            tokio::pin!(call);
            let out = loop {
                tokio::select! {
                    out = &mut call => break out,
                    ev = events.recv() => {
                        if let Ok(RuntimeEvent::ApprovalRequired { id, .. }) = ev {
                            cards += 1;
                            answer_route(&e.state, &id, "deny", Some(reason)).await;
                        }
                    }
                }
            };
            let heard = said_why(&reason.replace('\n', " "));
            assert_eq!((out.status, out.text.as_str()), ("denied", heard.as_str()));
            assert_eq!(out.record["status"], "denied");
        }
        assert_eq!(cards, 2, "the second call asked: a no with a reason isn't remembered");
        assert_eq!(e.state.broker.deny_reasons_held(), 0);
        session.shutdown().await;
        finish_turn(&e.state, &token).await;
        serve.abort();
    }

    #[tokio::test]
    async fn an_api_bot_s_tool_result_says_it_was_already_declined() {
        let e = setup().await;
        // A real MCP server over http for the API engine's tool session: kwnt's own permission
        // endpoint for a live turn, whose one tool is `approve`
        let (_, token) = prepare_turn_with_opts(&e.state, &e.bot, Some("t-host".into()), None, None, Default::default()).await.unwrap();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let app = super::super::create_router(e.state.clone());
        let serve = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let server = http_server("perm", &format!("http://{addr}/api/mcp/permission/{token}"));
        let ctx = McpTurnContext {
            servers: vec![server],
            approver: Arc::new(approver(&e, &e.general, Duration::from_secs(10))),
            config_dir: std::env::temp_dir(),
            permission_url: None,
            gateway: None,
            max_tool_iterations: 5,
            approvals: mcp::Approvals::Ask,
            records: Default::default(),
            kwnt_only: false,
            media: None,
        };
        let (session, problems) = McpToolSession::open(&ctx, None).await;
        assert!(problems.is_empty(), "{problems:?}");
        let caller = Caller { kwant_id: e.bot.id.clone(), kwant_name: e.bot.name.clone(), turn_id: "t-api".into() };

        let mut events = e.state.event_sender.subscribe();
        let first = {
            let call = session.call(&caller, "c1", "mcp__perm__approve", r#"{"tool_name": "Bash", "input": {}}"#);
            tokio::pin!(call);
            loop {
                tokio::select! {
                    out = &mut call => break out,
                    ev = events.recv() => {
                        if let Ok(RuntimeEvent::ApprovalRequired { id, .. }) = ev {
                            assert!(e.state.broker.resolve(&id, ApprovalDecision::Deny).await);
                        }
                    }
                }
            }
        };
        assert_eq!((first.status, first.text.as_str()), ("denied", mcp::DENIED_TEXT));

        let second = session.call(&caller, "c2", "mcp__perm__approve", r#"{"tool_name": "Read", "input": {}}"#).await;
        assert_eq!((second.status, second.text.as_str()), ("denied", mcp::DECLINED_BEFORE_TEXT));
        assert_eq!(second.record["status"], "denied");
        while let Ok(ev) = events.try_recv() {
            assert!(!matches!(ev, RuntimeEvent::ApprovalRequired { .. }), "no card the second time");
        }
        session.shutdown().await;
        finish_turn(&e.state, &token).await;
        serve.abort();
    }
}
