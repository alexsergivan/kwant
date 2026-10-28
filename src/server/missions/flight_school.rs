//! Mission Control Phase C3: Flight School. A bot's tier for a class of tasks (`observe`,
//! `sandbox`, `act`) is a ceiling on top of its approval mode: it can add a card, never remove one.
//! [`tier_gate`] is the one function every permission path asks (`BrokerApprover::classified`,
//! claude's built-in Allow, kwant's own tools, and the API drivers' shell); it reads the policy on
//! every call, so a revoke holds from the next call on. Off (the default) gates nothing.
//! Contract: docs/mission-control.md §14.

use super::routes::{bad, conflict, internal, not_found};
use super::{bot_in_space, task_folder_in, transition, Actor};
use crate::db::flight_school::{rank, valid_class, ANY_CLASS, DEFAULT_CLASS, TIERS};
use crate::db::missions::{Task, TaskChange};
use crate::db::Database;
use crate::mcp::ToolApprovalRequest;
use crate::server::spaces::Scope;
use crate::server::AppState;
use axum::{
    extract::{Path, Query, State},
    response::{IntoResponse, Response},
    Json,
};
use serde::Deserialize;
use serde_json::{json, Value};

pub const SETTING: &str = "flight_school";
pub const NEW_BOT_SETTING: &str = "flight_school_new_bot_tier";
pub const DEFAULT_NEW_BOT_TIER: &str = "sandbox";
/// A task's blocked reason when its bot's engine runs tools kwant can't see (D7).
pub const ENGINE_CANT_SANDBOX: &str = "This engine can't be held to its tier: switch the bot to Claude or grant Act";

pub async fn enabled(db: &Database) -> bool {
    matches!(db.get_setting(SETTING).await.ok().flatten().as_deref().map(str::trim), Some("true" | "1" | "on"))
}

pub async fn new_bot_tier(db: &Database) -> &'static str {
    let v = db.get_setting(NEW_BOT_SETTING).await.ok().flatten();
    TIERS.iter().copied().find(|t| Some(*t) == v.as_deref().map(str::trim)).unwrap_or(DEFAULT_NEW_BOT_TIER)
}

pub fn settings_error(new_bot_tier: Option<&str>) -> Option<String> {
    new_bot_tier.filter(|t| !TIERS.contains(t)).map(|t| format!("flight_school_new_bot_tier must be observe, sandbox or act (got {t})"))
}

/// The bot's tier for `class`: its row for the class, else its `'*'` row, else `act` (D4: bots
/// that were there before Flight School keep today's freedom). A policy that can't be read is
/// `observe`: the gate fails closed.
pub async fn tier_of(db: &Database, kwant_id: &str, class: &str) -> (&'static str, &'static str) {
    match db.live_policy(kwant_id, class).await {
        Ok(Some(p)) => (TIERS[rank(&p.tier)], if p.task_class == ANY_CLASS { "any_class" } else { "class" }),
        Ok(None) => ("act", "default"),
        Err(e) => {
            tracing::warn!(error = %e, kwant = %kwant_id, "could not read a Flight School tier; treating it as observe");
            ("observe", "unreadable")
        }
    }
}

/// kwant's own tools that change nothing outside the chat or the board: free at every tier. The
/// rest of the self tools are `external`, as the side-effect ledger counts them.
fn self_tool_is_free(tool: &str) -> bool {
    tool.starts_with("memory_")
        || tool.starts_with("fact_")
        || tool.starts_with("task_")
        || matches!(tool, "mission_plan" | "tier_status" | "ask_user")
        || crate::server::continuity::READ_ONLY_SELF_TOOLS.contains(&tool)
}

/// The call's effect for the gate: the ledger's [`super::checkpoints::effect`], with kwant's free
/// self tools as `none`.
pub fn gate_effect(req: &ToolApprovalRequest, task_folder: Option<&std::path::Path>) -> &'static str {
    if req.server == crate::mcp::SELF_SERVER_NAME && self_tool_is_free(&req.tool) {
        return "none";
    }
    super::checkpoints::effect(req, task_folder)
}

/// What `tier` says about a call of `effect`: `None` leaves it to today's path, `Some(reason)`
/// forces a card. `checkpoint` is whether this run has a usable `before_run` checkpoint.
pub fn decide(tier: &str, effect: &str, bash: bool, checkpoint: bool) -> Option<&'static str> {
    match (tier, effect) {
        ("act", _) | (_, "none") => None,
        ("sandbox", "workspace") if checkpoint => None,
        ("sandbox", "workspace") => Some("Sandbox: this run has no checkpoint to undo file changes, so they ask"),
        ("sandbox", _) if bash => Some("Sandbox: commands always ask"),
        ("sandbox", _) => Some("Sandbox: only files in the task folder change without asking"),
        _ => Some("Observe: every change asks"),
    }
}

/// The one Flight School gate (§3.3). The payment floor comes first and no tier skips it; then,
/// only with `flight_school` on and only for a turn that works a task, the bot's live tier for the
/// task's class against the call's effect. `Some(reason)`: the call gets a card whatever the
/// approval mode, smart Allow or `always_ask` say (a session "always allow" still counts, D9).
pub async fn tier_gate(db: &Database, data_dir: &std::path::Path, req: &ToolApprovalRequest) -> Option<String> {
    if let Some(why) = req.payment.clone().or_else(|| crate::mcp::payment_floor::reason(req, None)) {
        return Some(why);
    }
    if !enabled(db).await {
        return None;
    }
    let task = task_of(db, &req.kwant_id, &req.turn_id).await?;
    let class = db.task_class(&task.id).await.unwrap_or_else(|_| DEFAULT_CLASS.into());
    let (tier, _) = tier_of(db, &req.kwant_id, &class).await;
    if tier == "act" {
        return None;
    }
    let folder = task_folder_in(db, data_dir, &task).await;
    let effect = gate_effect(req, folder.as_deref());
    let checkpoint = effect == "workspace" && db.run_has_checkpoint(&task.id).await.unwrap_or(false);
    decide(tier, effect, req.builtin.is_some() && req.tool_name == "Bash", checkpoint).map(str::to_string)
}

/// The gate for a shell command an API driver runs (`execute_shell`): always `external`.
pub async fn shell_gate(db: &Database, kwant_id: &str) -> Option<&'static str> {
    if !enabled(db).await {
        return None;
    }
    let task = task_of(db, kwant_id, "").await?;
    let class = db.task_class(&task.id).await.unwrap_or_else(|_| DEFAULT_CLASS.into());
    let (tier, _) = tier_of(db, kwant_id, &class).await;
    decide(tier, "external", true, false)
}

async fn task_of(db: &Database, kwant_id: &str, turn_id: &str) -> Option<Task> {
    let id = db.task_of_turn(kwant_id, turn_id).await.ok().flatten()?;
    db.get_task(&id).await.ok().flatten()
}

/// Whether kwant sees every tool call of a bot on this engine (§3.5): Claude (permission prompt)
/// and Grok (only MCP tools) do. Codex runs its shell and file tools in its own sandbox, and a
/// custom engine outside kwant's gateway, so neither can be held below `act`.
pub fn engine_can_be_gated(model_or_family: &str) -> bool {
    let m = model_or_family.trim().to_ascii_lowercase();
    !matches!(m.as_str(), "codex" | "custom")
        && !matches!(crate::profiles::Engine::from_model(&m), Some(crate::profiles::Engine::Codex | crate::profiles::Engine::Custom))
}

/// The engine guard (D7) before a runner turn: a task whose bot is below `act` and pinned to an
/// engine kwant can't gate is blocked with [`ENGINE_CANT_SANDBOX`] instead of running unguarded.
/// `true` when it was blocked.
pub async fn block_if_ungated(state: &AppState, task: &Task, bot: &crate::db::models::Kwant, model: Option<&str>) -> bool {
    if !enabled(&state.db).await {
        return false;
    }
    let pinned = crate::router::parse_pin(&bot.preferred_model).family;
    let Some(engine) = model.map(str::to_string).or(pinned) else { return false };
    if engine_can_be_gated(&engine) {
        return false;
    }
    let class = state.db.task_class(&task.id).await.unwrap_or_else(|_| DEFAULT_CLASS.into());
    if tier_of(&state.db, &bot.id, &class).await.0 == "act" {
        return false;
    }
    let Ok(Some(t)) = state.db.get_task(&task.id).await else { return false };
    let change = TaskChange { reason: Some(ENGINE_CANT_SANDBOX.into()), ..Default::default() };
    match transition(state, &t.id, &t.state, "blocked", Actor::Kwant, "kwant", change).await {
        Ok(_) => {
            tracing::info!(task_id = %t.id, kwant = %bot.id, engine = %engine, "Flight School: blocked a task its engine can't be held to");
            true
        }
        Err(_) => false,
    }
}

/// The same guard once a turn knows its engine: an `auto` bot routed to Codex or a custom engine
/// while it works a task below `act`. The task is blocked and the turn doesn't start.
pub async fn refuse_turn(state: &AppState, bot: &crate::db::models::Kwant, model: &str) -> Option<String> {
    if engine_can_be_gated(model) || !enabled(&state.db).await {
        return None;
    }
    let task = task_of(&state.db, &bot.id, "").await?;
    block_if_ungated(state, &task, bot, Some(model)).await.then(|| format!("{}: {ENGINE_CANT_SANDBOX}.", bot.name))
}

/// The task's Needs-you item when the engine guard blocked it.
pub fn needs_you(task: &Task) -> Option<Value> {
    (task.state == "blocked" && task.blocked_reason.as_deref() == Some(ENGINE_CANT_SANDBOX))
        .then(|| json!({"kind": "engine_cant_sandbox", "id": task.id, "text": ENGINE_CANT_SANDBOX}))
}

/// `owner_tier` on task JSON: the tier of the bot that runs (or would run) the task, with Flight
/// School on; `null` when it is off.
pub async fn owner_tier(state: &AppState, task: &Task) -> Value {
    if !enabled(&state.db).await {
        return Value::Null;
    }
    let Some(bot) = task.owner_kwant_id.as_deref().or(task.suggested_kwant_id.as_deref()) else { return Value::Null };
    let class = state.db.task_class(&task.id).await.unwrap_or_else(|_| DEFAULT_CLASS.into());
    json!(tier_of(&state.db, bot, &class).await.0)
}

/// A bot's tiers: its live rows and, per class its tasks have, the tier that applies and why.
pub async fn status_json(state: &AppState, kwant_id: &str) -> Value {
    let policies = state.db.policies_of(kwant_id).await.unwrap_or_default();
    let mut classes = state.db.classes_of(kwant_id).await.unwrap_or_default();
    for p in &policies {
        if p.task_class != ANY_CLASS && !classes.contains(&p.task_class) {
            classes.push(p.task_class.clone());
        }
    }
    if !classes.iter().any(|c| c == DEFAULT_CLASS) {
        classes.insert(0, DEFAULT_CLASS.into());
    }
    let mut effective = Vec::new();
    for c in &classes {
        let (tier, from) = tier_of(&state.db, kwant_id, c).await;
        effective.push(json!({"task_class": c, "tier": tier, "from": from}));
    }
    let any = policies.iter().find(|p| p.task_class == ANY_CLASS).map(|p| p.tier.clone());
    json!({
        "flight_school": enabled(&state.db).await,
        "kwant_id": kwant_id,
        "default_tier": any.unwrap_or_else(|| "act".into()),
        "policies": policies,
        "effective": effective,
        "events": state.db.capability_events(kwant_id, 50).await.unwrap_or_default(),
    })
}

// Routes: reads are open like the rest of the app API, writes are person-only (`auth.rs`)

#[derive(Deserialize)]
pub struct CapQuery {
    kwant_id: String,
}

/// `GET /api/capabilities?kwant_id=`.
pub async fn get_capabilities(State(state): State<AppState>, Scope(space): Scope, Query(q): Query<CapQuery>) -> Response {
    let Some(bot) = bot_in_space(&state, &space, &q.kwant_id).await else { return not_found("bot") };
    Json(status_json(&state, &bot.id).await).into_response()
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SetTier {
    kwant_id: String,
    task_class: String,
    tier: String,
    note: Option<String>,
}

fn class_ok(class: &str) -> bool {
    class == ANY_CLASS || valid_class(class)
}

/// `POST /api/capabilities` (person-only): sets a bot's tier for a class, or `'*'` for every class
/// without its own row. Setting the tier it already has changes nothing (safe to repeat).
pub async fn set_capability(State(state): State<AppState>, Scope(space): Scope, Json(p): Json<SetTier>) -> Response {
    if !class_ok(&p.task_class) {
        return bad("task_class must be * or 1–40 of a-z, 0-9 and -");
    }
    if !TIERS.contains(&p.tier.as_str()) {
        return bad("tier must be observe, sandbox or act");
    }
    let note = p.note.as_deref().map(str::trim).filter(|n| !n.is_empty()).map(|n| n.chars().take(500).collect::<String>());
    let Some(bot) = bot_in_space(&state, &space, &p.kwant_id).await else { return not_found("bot") };
    let own = state.db.policies_of(&bot.id).await.unwrap_or_default();
    if own.iter().any(|x| x.task_class == p.task_class && x.tier == p.tier) {
        return Json(json!({"unchanged": true, "capabilities": status_json(&state, &bot.id).await})).into_response();
    }
    match state.db.set_policy(&bot.id, &p.task_class, &p.tier, "granted", note.as_deref()).await {
        Ok(_) => Json(json!({"unchanged": false, "capabilities": status_json(&state, &bot.id).await})).into_response(),
        Err(crate::db::DbError::Sqlx(e)) if e.to_string().contains("UNIQUE") => conflict("another change to this tier landed first; reload"),
        Err(e) => internal(e),
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Revoke {
    task_class: String,
    to_tier: String,
    note: Option<String>,
}

/// `POST /api/capabilities/:kwant_id/revoke` (person-only): lowers a bot's tier for a class at
/// once. The next tool call of a running turn is gated by the new tier; a call already running
/// finishes. Revoking to the tier it already has (or a higher one) changes nothing.
pub async fn revoke_capability(State(state): State<AppState>, Scope(space): Scope, Path(kwant_id): Path<String>, Json(p): Json<Revoke>) -> Response {
    if !class_ok(&p.task_class) {
        return bad("task_class must be * or 1–40 of a-z, 0-9 and -");
    }
    if !matches!(p.to_tier.as_str(), "observe" | "sandbox") {
        return bad("to_tier must be observe or sandbox");
    }
    let Some(bot) = bot_in_space(&state, &space, &kwant_id).await else { return not_found("bot") };
    let class = if p.task_class == ANY_CLASS { DEFAULT_CLASS } else { p.task_class.as_str() };
    let now = if p.task_class == ANY_CLASS {
        state.db.policies_of(&bot.id).await.unwrap_or_default().into_iter().find(|x| x.task_class == ANY_CLASS).map_or("act", |x| TIERS[rank(&x.tier)])
    } else {
        tier_of(&state.db, &bot.id, class).await.0
    };
    if rank(&p.to_tier) >= rank(now) {
        return Json(json!({"unchanged": true, "capabilities": status_json(&state, &bot.id).await})).into_response();
    }
    let note = p.note.as_deref().map(str::trim).filter(|n| !n.is_empty()).map(|n| n.chars().take(500).collect::<String>());
    match state.db.set_policy(&bot.id, &p.task_class, &p.to_tier, "revoked", note.as_deref()).await {
        Ok(_) => Json(json!({"unchanged": false, "capabilities": status_json(&state, &bot.id).await})).into_response(),
        Err(crate::db::DbError::Sqlx(e)) if e.to_string().contains("UNIQUE") => conflict("another change to this tier landed first; reload"),
        Err(e) => internal(e),
    }
}

// The `tier_status` bot tool (read-only)

pub fn tool() -> Value {
    json!({
        "name": "tier_status",
        "description": "Your Flight School tier: how much you may do on your own per class of task (observe: every change asks; sandbox: only files in the task folder, with a checkpoint; act: your normal approvals). Only the person changes it.",
        "inputSchema": {
            "type": "object",
            "properties": {"task_class": {"type": "string", "description": "One class; leave out for all."}},
        },
    })
}

pub async fn call_tool(state: &AppState, kwant_id: &str, args: &Value) -> (String, bool) {
    let mut v = status_json(state, kwant_id).await;
    if let Some(class) = args["task_class"].as_str().map(str::trim).filter(|c| !c.is_empty()) {
        if !valid_class(class) {
            return ("task_class must be 1–40 of a-z, 0-9 and -".into(), true);
        }
        let (tier, from) = tier_of(&state.db, kwant_id, class).await;
        v["effective"] = json!([{"task_class": class, "tier": tier, "from": from}]);
    }
    if let Some(obj) = v.as_object_mut() {
        obj.remove("events");
    }
    (serde_json::to_string_pretty(&v).unwrap_or_default(), false)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_tier_table() {
        // (tier, effect, bash, checkpoint) → card?
        for (tier, effect, bash, cp, card) in [
            ("act", "workspace", false, false, false),
            ("act", "external", true, false, false),
            ("sandbox", "none", false, false, false),
            ("sandbox", "workspace", false, true, false),
            ("sandbox", "workspace", false, false, true),
            ("sandbox", "external", true, true, true),
            ("sandbox", "external", false, true, true),
            ("observe", "none", false, false, false),
            ("observe", "workspace", false, true, true),
            ("observe", "external", true, true, true),
        ] {
            assert_eq!(decide(tier, effect, bash, cp).is_some(), card, "{tier} {effect} bash={bash} cp={cp}");
        }
        assert_eq!(decide("sandbox", "external", true, true), Some("Sandbox: commands always ask"));
    }

    #[test]
    fn engines_kwant_can_hold() {
        for ok in ["claude", "claude-sonnet-4-6", "grok", "grok-4", "anthropic/claude-opus"] {
            assert!(engine_can_be_gated(ok), "{ok}");
        }
        for no in ["codex", "gpt-5-codex", "gpt-5", "custom", "custom:claude-sonnet-4.6"] {
            assert!(!engine_can_be_gated(no), "{no}");
        }
    }

    #[test]
    fn free_self_tools() {
        let req = |tool: &str| ToolApprovalRequest {
            kwant_id: "k".into(),
            kwant_name: "K".into(),
            turn_id: "t".into(),
            tool_name: format!("mcp__{}__{tool}", crate::mcp::SELF_SERVER_NAME),
            server: crate::mcp::SELF_SERVER_NAME.into(),
            tool: tool.into(),
            arguments: json!({}),
            builtin: None,
            smart: None,
            payment: None,
        };
        for free in ["memory_add", "task_update", "mission_plan", "tier_status", "ask_user", "search_history"] {
            assert_eq!(gate_effect(&req(free), None), "none", "{free}");
        }
        for ext in ["browser_act", "bot_update", "channel_update", "settings_update", "delegate"] {
            assert_eq!(gate_effect(&req(ext), None), "external", "{ext}");
        }
    }
}
