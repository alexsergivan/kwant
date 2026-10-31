//! Mission Control Phase C3: Flight School. A bot's tier for a class of tasks (`observe`,
//! `sandbox`, `act`) is a ceiling on top of its approval mode: it can add a card, never remove one.
//! [`tier_gate`] is the one function every permission path asks (`BrokerApprover::classified`,
//! claude's built-in Allow, kwant's own tools, and the API drivers' shell); it reads the policy on
//! every call, so a revoke holds from the next call on. Off (the default) gates nothing.
//! Contract: docs/mission-control.md §14.

use super::routes::{bad, conflict, internal, not_found};
use super::{bot_in_space, task_folder_in, transition, Actor};
use crate::db::flight_school::{rank, valid_class, Decided, ScoreTask, TierProposal, ANY_CLASS, DEFAULT_CLASS, TIERS};
use crate::db::missions::{Task, TaskChange};
use crate::db::Database;
use crate::mcp::ToolApprovalRequest;
use crate::server::spaces::Scope;
use crate::server::AppState;
use axum::{
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode},
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
        || matches!(tool, "mission_plan" | "tier_status" | "tier_propose" | "ask_user")
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
        ("sandbox", _) if bash => Some("Sandbox: commands that can change things ask"),
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
        "proposals": state.db.tier_proposals_of(kwant_id, 20).await.unwrap_or_default().iter().map(proposal_json).collect::<Vec<_>>(),
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
    refresh_stale(&state, &bot.id, None).await;
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
    refresh_stale(state, kwant_id, None).await;
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

// C4: scorecards and proposals. A scorecard is computed from rows kwant wrote itself, never
// stored as truth; a proposal snapshots the one the bot was eligible with. Contract §3.4.

/// Days a scorecard looks back.
pub const WINDOW_DAYS: i64 = 30;
/// Days after the person declines a bot's step up before it may ask for the same step again.
pub const DECLINE_COOLDOWN_DAYS: i64 = 7;

/// The tier one step up, `None` at `act`.
pub fn next_tier(tier: &str) -> Option<&'static str> {
    match tier {
        "observe" => Some("sandbox"),
        "sandbox" => Some("act"),
        _ => None,
    }
}

/// What a move up to `to` asks of the scorecard (§9 D5): verified tasks, the last-n window that
/// must have no denial, and the first-try rate (`act` only, which also wants no false success).
struct Rule {
    min_verified: usize,
    last_n: usize,
    min_first_try: Option<f64>,
    no_false_success: bool,
}

fn rule(to: &str) -> Rule {
    if to == "act" {
        Rule { min_verified: 10, last_n: 10, min_first_try: Some(0.7), no_false_success: true }
    } else {
        Rule { min_verified: 5, last_n: 5, min_first_try: None, no_false_success: false }
    }
}

fn rate(k: usize, n: usize) -> Value {
    if n == 0 {
        Value::Null
    } else {
        json!(((k as f64 / n as f64) * 1000.0).round() / 1000.0)
    }
}

fn denied(t: &ScoreTask) -> bool {
    t.denied > 0 || t.denied_failures > 0
}

/// Verified on the first attempt, with no failure on the way.
fn first_try(t: &ScoreTask) -> bool {
    t.state == "verified" && t.attempts <= 1 && t.failures == 0
}

/// The scorecard of `tasks` (a bot's settled tasks of one class, newest first) and whether it
/// may move from `tier` one step up, with what is missing. Pure, so the rules are unit-tested.
pub fn card_of(tasks: &[ScoreTask], tier: &str) -> (Value, Option<&'static str>, Vec<String>) {
    let n = |f: &dyn Fn(&ScoreTask) -> bool| tasks.iter().filter(|t| f(t)).count();
    let sum = |f: &dyn Fn(&ScoreTask) -> i64| tasks.iter().map(f).sum::<i64>();
    let verified = n(&|t| t.state == "verified");
    let first = n(&first_try);
    let false_success = n(&|t| t.false_success > 0);
    let next = next_tier(tier);
    let mut missing = Vec::new();
    let mut recent = Value::Null;
    if let Some(to) = next {
        let r = rule(to);
        let last: Vec<&ScoreTask> = tasks.iter().take(r.last_n).collect();
        let last_denied = last.iter().filter(|t| denied(t)).count();
        if verified < r.min_verified {
            missing.push(format!("{} more verified task(s) (has {verified}, needs {})", r.min_verified - verified, r.min_verified));
        }
        if last_denied > 0 {
            missing.push(format!("no denied call in the last {} tasks (has {last_denied})", r.last_n));
        }
        if let Some(min) = r.min_first_try {
            if verified == 0 || (first as f64) < min * verified as f64 {
                missing.push(format!("first-try rate of at least {}% (has {first} of {verified})", (min * 100.0) as i64));
            }
        }
        if r.no_false_success && false_success > 0 {
            missing.push(format!("no false success (has {false_success})"));
        }
        recent = json!({"n": last.len(), "of": r.last_n, "with_denial": last_denied});
    }
    let mut by_class = serde_json::Map::new();
    by_class.insert("denied".into(), json!(sum(&|t| t.denied_failures)));
    by_class.insert("check_failed".into(), json!(sum(&|t| t.check_failures)));
    let cards = sum(&|t| t.cards);
    let approved = sum(&|t| t.approved);
    let card = json!({
        "tasks": {
            "settled": tasks.len(),
            "verified": verified,
            "failed": n(&|t| t.state == "failed"),
            "cancelled": n(&|t| t.state == "cancelled"),
        },
        "first_try": {"k": first, "n": verified, "rate": rate(first, verified)},
        "verified_rate": {"k": verified, "n": tasks.len(), "rate": rate(verified, tasks.len())},
        "false_success": {"k": false_success, "n": verified, "rate": rate(false_success, verified)},
        "failures": {"total": sum(&|t| t.failures), "by_class": by_class},
        "cards": {
            "shown": cards,
            "approved": approved,
            "denied": sum(&|t| t.denied),
            "approved_rate": {"k": approved, "n": cards, "rate": rate(approved as usize, cards as usize)},
        },
        "calls": sum(&|t| t.calls),
        "external_calls": sum(&|t| t.external_calls),
        "interventions": sum(&|t| t.interventions),
        "tokens": {"spent": sum(&|t| t.spent_tokens), "recovery": sum(&|t| t.recovery_tokens)},
        "recent_denials": recent,
    });
    (card, next, missing)
}

/// `GET /api/capabilities/:kwant_id/scorecard?class=` and the snapshot a proposal keeps.
pub async fn scorecard(db: &Database, kwant_id: &str, class: &str) -> Result<Value, crate::db::DbError> {
    let since = chrono::Utc::now() - chrono::Duration::days(WINDOW_DAYS);
    let tasks = db.score_tasks(kwant_id, class, since).await?;
    let (tier, from) = tier_of(db, kwant_id, class).await;
    let (mut card, next, missing) = card_of(&tasks, tier);
    if let Some(o) = card.as_object_mut() {
        o.insert("kwant_id".into(), json!(kwant_id));
        o.insert("task_class".into(), json!(class));
        o.insert("window_days".into(), json!(WINDOW_DAYS));
        o.insert("since".into(), json!(since.to_rfc3339()));
        o.insert("tier".into(), json!(tier));
        o.insert("tier_from".into(), json!(from));
        o.insert("next_tier".into(), json!(next));
        o.insert("eligible".into(), json!(next.is_some() && missing.is_empty()));
        o.insert("missing".into(), json!(missing));
    }
    Ok(card)
}

/// A proposal as the API shows it: the snapshot as JSON and the Needs-you kind.
pub fn proposal_json(p: &TierProposal) -> Value {
    let mut v = serde_json::to_value(p).unwrap_or(Value::Null);
    v["scorecard"] = serde_json::from_str(&p.scorecard).unwrap_or(Value::Null);
    v["kind"] = json!("tier_proposal");
    v
}

fn changed(state: &AppState, p: &TierProposal) {
    let _ = state.event_sender.send(crate::drivers::events::RuntimeEvent::CapabilityChanged {
        capability: json!({
            "space_id": p.space_id,
            "kwant_id": p.kwant_id,
            "task_class": p.task_class,
            "proposal": proposal_json(p),
        }),
    });
}

/// Why an open proposal can't be granted as asked any more: the bot's tier moved, or its
/// scorecard fell below the step it asked for. `None`: still good.
async fn stale_reason(db: &Database, p: &TierProposal) -> Option<String> {
    let (tier, _) = tier_of(db, &p.kwant_id, &p.task_class).await;
    if tier != p.from_tier {
        return Some(format!("the tier changed to {tier} since the bot asked"));
    }
    let card = scorecard(db, &p.kwant_id, &p.task_class).await.ok()?;
    (card["eligible"] != json!(true)).then(|| {
        let missing: Vec<&str> = card["missing"].as_array().map(|m| m.iter().filter_map(Value::as_str).collect()).unwrap_or_default();
        format!("the scorecard no longer qualifies: {}", missing.join("; "))
    })
}

/// Marks the bot's open proposals (one class, or all) stale where they no longer hold. Lazy:
/// called wherever proposals are read or decided. Leaves them alone while Flight School is off.
pub async fn refresh_stale(state: &AppState, kwant_id: &str, class: Option<&str>) {
    if !enabled(&state.db).await {
        return;
    }
    let open = state.db.tier_proposals_of(kwant_id, 200).await.unwrap_or_default();
    for p in open.iter().filter(|p| p.status == "proposed" && class.is_none_or(|c| c == p.task_class)) {
        if let Some(why) = stale_reason(&state.db, p).await {
            if let Ok(Decided::Done(p)) = state.db.decide_tier_proposal(&p.id, "stale", "kwant", Some(&why)).await {
                tracing::info!(kwant = %p.kwant_id, proposal = %p.id, why = %why, "tier proposal went stale");
                changed(state, &p);
            }
        }
    }
}

// The `tier_propose` bot tool

pub fn propose_tool() -> Value {
    json!({
        "name": "tier_propose",
        "description": "Ask the person to move you one Flight School tier up for a class of tasks (observe → sandbox → act). kwant checks your scorecard (last 30 days of your verified, failed and denied work in that class) and refuses when you don't qualify yet; only the person can grant it. Say why in `reason`.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "task_class": {"type": "string", "description": "The class of tasks, e.g. general or research."},
                "reason": {"type": "string", "description": "Why you should do more on your own here, in a sentence or two."},
            },
            "required": ["task_class", "reason"],
        },
    })
}

pub async fn call_propose(state: &AppState, scope: &crate::server::continuity::SelfScope, args: &Value) -> (String, bool) {
    if !enabled(&state.db).await {
        return ("Flight School is off, so there is no tier to ask for.".into(), true);
    }
    let class = args["task_class"].as_str().map(str::trim).unwrap_or("");
    if !valid_class(class) {
        return ("task_class must be 1–40 of a-z, 0-9 and -".into(), true);
    }
    let reason: String = args["reason"].as_str().map(str::trim).unwrap_or("").chars().take(500).collect();
    if reason.is_empty() {
        return ("Say why in reason.".into(), true);
    }
    let bot = match state.db.get_kwant(&scope.kwant_id).await {
        Ok(Some(b)) => b,
        _ => return ("Couldn't find you.".into(), true),
    };
    refresh_stale(state, &bot.id, Some(class)).await;
    let (tier, _) = tier_of(&state.db, &bot.id, class).await;
    let Some(to) = next_tier(tier) else { return (format!("You already have act for {class}."), true) };
    if let Ok(Some(open)) = state.db.open_tier_proposal(&bot.id, class).await {
        return (format!("You already asked for {} on {class} (proposal {}); it waits for the person.", open.to_tier, open.id), false);
    }
    if let Ok(Some(at)) = state.db.last_declined(&bot.id, class, to).await {
        if let Ok(at) = chrono::DateTime::parse_from_rfc3339(&at) {
            let until = at.with_timezone(&chrono::Utc) + chrono::Duration::days(DECLINE_COOLDOWN_DAYS);
            if until > chrono::Utc::now() {
                return (format!("The person declined {to} on {class} recently; you can ask again after {}.", until.format("%Y-%m-%d")), true);
            }
        }
    }
    let card = match scorecard(&state.db, &bot.id, class).await {
        Ok(c) => c,
        Err(e) => {
            tracing::warn!(error = %e, "could not compute a scorecard");
            return ("Couldn't read your scorecard; try again later.".into(), true);
        }
    };
    if card["eligible"] != json!(true) {
        let missing: Vec<&str> = card["missing"].as_array().map(|m| m.iter().filter_map(Value::as_str).collect()).unwrap_or_default();
        return (format!("Not yet: to ask for {to} on {class} you need {}.", missing.join("; ")), true);
    }
    match state.db.create_tier_proposal(&bot.space_id, &bot.id, class, tier, to, &card, Some(&reason)).await {
        Ok(Ok(p)) => {
            changed(state, &p);
            let verified = card["tasks"]["verified"].as_u64().unwrap_or(0);
            crate::server::continuity::notify(
                state,
                crate::server::continuity::Notification {
                    kind: "tier_proposed",
                    kwant_id: bot.id.clone(),
                    kwant_name: bot.name.clone(),
                    channel_id: None,
                    title: format!("{} asks for {to} on {class}", bot.name),
                    body: format!("{} asks to move from {tier} to {to} on {class} tasks, with {verified} verified in the last {WINDOW_DAYS} days.", bot.name),
                },
            )
            .await;
            tracing::info!(kwant = %bot.id, proposal = %p.id, class, to, "tier proposed");
            (format!("Asked the person for {to} on {class} (proposal {}). Until they grant it you stay at {tier}.", p.id), false)
        }
        Ok(Err(open)) => (format!("You already asked for {} on {class} (proposal {}); it waits for the person.", open.to_tier, open.id), false),
        Err(e) => {
            tracing::warn!(error = %e, "could not save a tier proposal");
            ("Couldn't save the proposal; try again later.".into(), true)
        }
    }
}

// Proposal and scorecard routes. Reads are open; grant and decline are person-only (`auth.rs`)

#[derive(Deserialize)]
pub struct CardQuery {
    class: Option<String>,
}

/// `GET /api/capabilities/:kwant_id/scorecard?class=` (class defaults to `general`).
pub async fn get_scorecard(State(state): State<AppState>, Scope(space): Scope, Path(kwant_id): Path<String>, Query(q): Query<CardQuery>) -> Response {
    let class = q.class.as_deref().map(str::trim).filter(|c| !c.is_empty()).unwrap_or(DEFAULT_CLASS);
    if !valid_class(class) {
        return bad("class must be 1–40 of a-z, 0-9 and -");
    }
    let Some(bot) = bot_in_space(&state, &space, &kwant_id).await else { return not_found("bot") };
    match scorecard(&state.db, &bot.id, class).await {
        Ok(v) => Json(v).into_response(),
        Err(e) => internal(e),
    }
}

#[derive(Deserialize)]
pub struct ProposalQuery {
    status: Option<String>,
    kwant_id: Option<String>,
}

/// `GET /api/capabilities/proposals?status=&kwant_id=`: the space's proposals, newest first.
pub async fn list_proposals(State(state): State<AppState>, Scope(space): Scope, Query(q): Query<ProposalQuery>) -> Response {
    let status = q.status.as_deref().map(str::trim).filter(|s| !s.is_empty());
    if status.is_some_and(|s| !matches!(s, "proposed" | "granted" | "declined" | "stale")) {
        return bad("status must be proposed, granted, declined or stale");
    }
    let bot = match q.kwant_id.as_deref().map(str::trim).filter(|k| !k.is_empty()) {
        Some(k) => match bot_in_space(&state, &space, k).await {
            Some(b) => Some(b.id),
            None => return not_found("bot"),
        },
        None => None,
    };
    let open = state.db.tier_proposals(space.as_str(), bot.as_deref(), Some("proposed"), 200).await.unwrap_or_default();
    let mut bots: Vec<&str> = open.iter().map(|p| p.kwant_id.as_str()).collect();
    bots.sort_unstable();
    bots.dedup();
    for b in bots {
        refresh_stale(&state, b, None).await;
    }
    match state.db.tier_proposals(space.as_str(), bot.as_deref(), status, 200).await {
        Ok(list) => Json(json!({"proposals": list.iter().map(proposal_json).collect::<Vec<_>>()})).into_response(),
        Err(e) => internal(e),
    }
}

fn not_open(status: &str, why: Option<&str>) -> Response {
    let error = match (status, why) {
        ("stale", Some(w)) => format!("this proposal is stale: {w}"),
        _ => format!("this proposal is already {status}"),
    };
    (StatusCode::CONFLICT, Json(json!({"error": error, "status": status}))).into_response()
}

#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct DecideBody {
    note: Option<String>,
}

#[allow(clippy::result_large_err)]
async fn proposal_in(state: &AppState, space: &crate::db::spaces::SpaceId, id: &str) -> Result<TierProposal, Response> {
    match state.db.tier_proposal(id).await {
        Ok(Some(p)) if p.space_id == space.as_str() => Ok(p),
        Ok(_) => Err(not_found("proposal")),
        Err(e) => Err(internal(e)),
    }
}

/// `POST /api/capabilities/proposals/:id/grant` (person-only; `Idempotency-Key` optional). kwant
/// checks again that the bot is still at the tier it asked from and still qualifies; if not, the
/// proposal goes stale and this answers 409. Of racing grants exactly one writes the policy.
pub async fn grant_proposal(State(state): State<AppState>, Scope(space): Scope, headers: HeaderMap, Path(id): Path<String>) -> Response {
    let route = format!("capabilities/proposals/{id}/grant");
    super::graph::idempotent(&state, &headers, route, || async {
        let p = proposal_in(&state, &space, &id).await?;
        if p.status != "proposed" {
            return Err(not_open(&p.status, p.note.as_deref()));
        }
        if !enabled(&state.db).await {
            return Err(super::routes::conflict("Flight School is off; switch it on to grant tiers"));
        }
        if let Some(why) = stale_reason(&state.db, &p).await {
            return Err(match state.db.decide_tier_proposal(&p.id, "stale", "kwant", Some(&why)).await {
                Ok(Decided::Done(p)) => {
                    changed(&state, &p);
                    not_open("stale", Some(&why))
                }
                Ok(Decided::NotOpen(s)) => not_open(&s, None),
                Ok(Decided::Missing) => not_found("proposal"),
                Err(e) => internal(e),
            });
        }
        match state.db.decide_tier_proposal(&p.id, "granted", "person", None).await {
            Ok(Decided::Done(p)) => {
                tracing::info!(kwant = %p.kwant_id, proposal = %p.id, tier = %p.to_tier, "tier proposal granted");
                changed(&state, &p);
                Ok(json!({"proposal": proposal_json(&p), "capabilities": status_json(&state, &p.kwant_id).await}))
            }
            Ok(Decided::NotOpen(s)) => Err(not_open(&s, None)),
            Ok(Decided::Missing) => Err(not_found("proposal")),
            Err(crate::db::DbError::Sqlx(e)) if e.to_string().contains("UNIQUE") => Err(not_open("granted", None)),
            Err(e) => Err(internal(e)),
        }
    })
    .await
}

/// `POST /api/capabilities/proposals/:id/decline {note?}` (person-only; `Idempotency-Key`
/// optional). The bot may ask for the same step again after [`DECLINE_COOLDOWN_DAYS`].
pub async fn decline_proposal(
    State(state): State<AppState>,
    Scope(space): Scope,
    headers: HeaderMap,
    Path(id): Path<String>,
    body: Option<Json<DecideBody>>,
) -> Response {
    let note = body.and_then(|Json(b)| b.note).map(|n| n.trim().chars().take(500).collect::<String>()).filter(|n| !n.is_empty());
    let route = format!("capabilities/proposals/{id}/decline");
    super::graph::idempotent(&state, &headers, route, || async {
        let p = proposal_in(&state, &space, &id).await?;
        match state.db.decide_tier_proposal(&p.id, "declined", "person", note.as_deref()).await {
            Ok(Decided::Done(p)) => {
                changed(&state, &p);
                Ok(json!({"proposal": proposal_json(&p), "capabilities": status_json(&state, &p.kwant_id).await}))
            }
            Ok(Decided::NotOpen(s)) => Err(not_open(&s, None)),
            Ok(Decided::Missing) => Err(not_found("proposal")),
            Err(e) => Err(internal(e)),
        }
    })
    .await
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
        assert_eq!(decide("sandbox", "external", true, true), Some("Sandbox: commands that can change things ask"));
    }

    fn task(state: &str, attempts: i64) -> ScoreTask {
        ScoreTask {
            id: "t".into(),
            state: state.into(),
            attempts,
            finished_at: chrono::Utc::now(),
            failures: 0,
            denied_failures: 0,
            check_failures: 0,
            calls: 0,
            external_calls: 0,
            cards: 0,
            approved: 0,
            denied: 0,
            false_success: 0,
            interventions: 0,
            spent_tokens: 0,
            recovery_tokens: 0,
        }
    }

    #[test]
    fn eligibility_rules() {
        // sandbox → act: 10 verified, first try at 70% exactly qualifies, 60% doesn't
        let mut ten: Vec<ScoreTask> = (0..10).map(|i| task("verified", if i < 7 { 1 } else { 2 })).collect();
        assert_eq!(card_of(&ten, "sandbox").1, Some("act"));
        assert!(card_of(&ten, "sandbox").2.is_empty(), "{:?}", card_of(&ten, "sandbox").2);
        ten[0].attempts = 3;
        assert!(card_of(&ten, "sandbox").2.iter().any(|m| m.contains("first-try")));
        // A denial older than the last 10 doesn't count; a newer one does
        let mut eleven: Vec<ScoreTask> = (0..11).map(|_| task("verified", 1)).collect();
        eleven[10].denied = 1;
        assert!(card_of(&eleven, "sandbox").2.is_empty());
        eleven[9].denied_failures = 1;
        assert!(card_of(&eleven, "sandbox").2.iter().any(|m| m.contains("last 10")));
        // observe → sandbox: 5 verified, no first-try rule
        let five: Vec<ScoreTask> = (0..5).map(|_| task("verified", 3)).collect();
        assert!(card_of(&five, "observe").2.is_empty());
        assert!(!card_of(&five[..4], "observe").2.is_empty());
        // act: nothing above it
        assert_eq!(card_of(&five, "act").1, None);
        // No rate without its n
        assert_eq!(card_of(&[], "observe").0["first_try"], json!({"k": 0, "n": 0, "rate": null}));
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
