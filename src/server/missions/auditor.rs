//! Mission Control Phase D2: the Swarm Auditor. A high-impact or low-confidence task that reaches
//! review gets one challenge from a teammate (with evidence) and one rebuttal from its owner; the
//! risks left unresolved are listed on the task until the person closes them. An audit never
//! verifies or fails a task and never counts agreement as proof: only outcome checks verify (§4).
//! Off by default. See docs/mission-control.md §18.

use super::routes::{bad, conflict, internal, not_found};
use crate::db::auditor::{audit_json, Answer, Audit, NewRisk};
use crate::db::missions::{OutcomeCheck, Task};
use crate::db::models::Kwant;
use crate::db::spaces::SpaceId;
use crate::drivers::engines::Tier;
use crate::server::continuity::SelfScope;
use crate::server::routes::{scope_for, turn_once, TurnOpts};
use crate::server::spaces::Scope;
use crate::server::AppState;
use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use chrono::{Duration, Utc};
use serde::Deserialize;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::LazyLock;

pub const SETTING: &str = "auditor";
pub const MODES: [&str; 2] = ["off", "on"];
pub const CAP_SETTING: &str = "auditor_daily_cap";
pub const DEFAULT_CAP: i64 = 5;
pub const CAP_RANGE: (i64, i64) = (1, 50);
/// The model tier audit turns run on: the engine's `fast` model by default, or `bot` for the
/// model the bot would use anyway.
pub const MODEL_SETTING: &str = "auditor_model";
pub const MODEL_MODES: [&str; 4] = ["fast", "balanced", "deep", "bot"];
pub const DEFAULT_MODEL: &str = "fast";
pub const MAX_RISKS: usize = 5;
const MAX_RISK_CHARS: usize = 500;
const MAX_EVIDENCE_CHARS: usize = 1000;
pub const PURPOSE: &str = "audit";
/// Longest an audit turn may take (an approval card it asked for included).
const TURN_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(20 * 60);
/// Write tools a bot in a live audit of a task can't use on that task.
pub const HELD_TOOLS: [&str; 5] = ["task_claim", "task_update", "task_attach_evidence", "task_request_check", "task_review"];

/// One audit runs at a time, server-wide; the others wait their turn.
static ONE: LazyLock<tokio::sync::Mutex<()>> = LazyLock::new(|| tokio::sync::Mutex::new(()));

pub async fn enabled(state: &AppState) -> bool {
    matches!(state.db.get_setting(SETTING).await.ok().flatten().as_deref().map(str::trim), Some("on"))
}

pub async fn daily_cap(state: &AppState) -> i64 {
    super::int_setting(state, CAP_SETTING, CAP_RANGE, DEFAULT_CAP).await
}

pub async fn model_mode(state: &AppState) -> &'static str {
    let saved = state.db.get_setting(MODEL_SETTING).await.ok().flatten();
    MODEL_MODES.into_iter().find(|m| Some(*m) == saved.as_deref().map(str::trim)).unwrap_or(DEFAULT_MODEL)
}

/// The tier an audit turn runs on for `mode`; `None` (`bot`) leaves it to the usual routing.
pub fn tier_of(mode: &str) -> Option<Tier> {
    match mode {
        "bot" => None,
        "balanced" => Some(Tier::Balanced),
        "deep" => Some(Tier::Deep),
        _ => Some(Tier::Fast),
    }
}

pub fn settings_error(mode: Option<&str>, cap: Option<i64>, model: Option<&str>) -> Option<String> {
    if let Some(m) = mode.filter(|m| !MODES.contains(m)) {
        return Some(format!("auditor must be off or on (got {m})"));
    }
    if let Some(n) = cap.filter(|n| !(CAP_RANGE.0..=CAP_RANGE.1).contains(n)) {
        return Some(format!("auditor_daily_cap must be {}–{} (got {n})", CAP_RANGE.0, CAP_RANGE.1));
    }
    if let Some(m) = model.filter(|m| !MODEL_MODES.contains(m)) {
        return Some(format!("auditor_model must be fast, balanced, deep or bot (got {m})"));
    }
    None
}

/// Why `task`, just in review, gets an audit: `high_impact` (priority high or urgent) or
/// `low_confidence` (no check that counts and isn't the owner's will judge it). `None` otherwise,
/// and always for a task without an owner.
pub fn trigger_of(task: &Task, checks: &[OutcomeCheck]) -> Option<&'static str> {
    task.owner_kwant_id.as_ref()?;
    if task.priority >= 2 {
        return Some("high_impact");
    }
    (super::checks_summary(task, checks)["status"] == "not_checked").then_some("low_confidence")
}

/// Bots that may not challenge `task`: its owner and the owner's own lead loop (the mission's lead,
/// the lead that handed it off).
pub async fn excluded(state: &AppState, task: &Task) -> Vec<String> {
    let mut out: Vec<String> = task.owner_kwant_id.iter().cloned().collect();
    if let Some(m) = &task.mission_id {
        if let Ok(Some(m)) = state.db.get_mission(m).await {
            out.push(m.lead_kwant_id);
        }
    }
    if let ("delegation", Some(d)) = (task.source_kind.as_str(), task.source_id.as_deref()) {
        if let Ok(Some(d)) = state.db.get_delegation(d).await {
            out.push(d.lead_kwant_id);
        }
    }
    out
}

/// The challenger of `task`: a bot of its space outside [`excluded`], with the most verified
/// tasks of its class, then the fewest challenges in the last 7 days, then by name and id.
pub async fn pick_challenger(state: &AppState, task: &Task) -> Option<Kwant> {
    let not = excluded(state, task).await;
    let bots = state.db.list_kwants(&SpaceId::new(&task.space_id)).await.ok()?;
    let class = state.db.task_class(&task.id).await.unwrap_or_else(|_| crate::db::flight_school::DEFAULT_CLASS.into());
    let verified = state.db.verified_in_class(&task.space_id, &class).await.unwrap_or_default();
    let recent = state.db.challenges_since(Utc::now() - Duration::days(7)).await.unwrap_or_default();
    let count = |list: &[(String, i64)], id: &str| list.iter().find(|(k, _)| k == id).map_or(0, |c| c.1);
    let mut cands: Vec<Kwant> = bots.into_iter().filter(|k| !not.contains(&k.id)).collect();
    cands.sort_by(|a, b| {
        count(&verified, &b.id)
            .cmp(&count(&verified, &a.id))
            .then(count(&recent, &a.id).cmp(&count(&recent, &b.id)))
            .then(a.name.to_lowercase().cmp(&b.name.to_lowercase()))
            .then(a.id.cmp(&b.id))
    });
    cands.into_iter().next()
}

async fn event(state: &AppState, task_id: &str, kind: &str, actor: &str, detail: Value) {
    if let Err(e) = state.db.log_task_event(task_id, kind, actor, &detail).await {
        tracing::warn!(task_id, kind, error = %e, "audit event not logged");
    }
}

/// After a move to review: audits the task in the background if the auditor is on and it
/// qualifies. Never blocks or fails the move.
pub fn on_review(state: &AppState, task: &Task) {
    let (state, task) = (state.clone(), task.clone());
    tokio::spawn(async move {
        if !enabled(&state).await {
            return;
        }
        let checks = state.db.list_checks(&task.id).await.unwrap_or_default();
        if let Some(trigger) = trigger_of(&task, &checks) {
            if let Err(why) = begin(&state, &task, trigger).await {
                tracing::info!(task_id = %task.id, why, "no audit");
            }
        }
    });
}

/// [`start`], then runs the audit in the background ([`drive`]).
pub async fn begin(state: &AppState, task: &Task, trigger: &str) -> Result<Audit, String> {
    let audit = start(state, task, trigger).await?;
    let (s, a) = (state.clone(), audit.clone());
    tokio::spawn(async move { drive(&s, &a).await });
    Ok(audit)
}

/// Opens an audit of `task`'s current attempt (`queued`). Refused (and, for the cap or a missing
/// challenger, logged as `audit_skipped`) when it can't start.
pub async fn start(state: &AppState, task: &Task, trigger: &str) -> Result<Audit, String> {
    let Some(owner) = task.owner_kwant_id.clone() else { return Err("the task has no owner".into()) };
    if matches!(state.db.audit_of_attempt(&task.id, task.attempts).await, Ok(Some(_))) {
        return Err("this attempt of the task was already audited".into());
    }
    let midnight = Utc::now().date_naive().and_hms_opt(0, 0, 0).map(|d| d.and_utc()).unwrap_or_else(Utc::now);
    let cap = daily_cap(state).await;
    if state.db.audits_since(midnight).await.unwrap_or(0) >= cap {
        event(state, &task.id, "audit_skipped", "kwant", json!({"reason": "daily_cap", "cap": cap})).await;
        return Err(format!("the daily audit cap ({cap}) is reached"));
    }
    let Some(challenger) = pick_challenger(state, task).await else {
        event(state, &task.id, "audit_skipped", "kwant", json!({"reason": "no_challenger"})).await;
        return Err("no teammate outside the owner's lead loop can challenge it".into());
    };
    let audit = match state.db.create_audit(&task.id, &task.space_id, task.attempts, trigger, &challenger.id, &owner).await {
        Ok(Some(a)) => a,
        Ok(None) => return Err("this attempt of the task was already audited".into()),
        Err(e) => return Err(e.to_string()),
    };
    let model = model_mode(state).await;
    event(state, &task.id, "audit_started", "kwant", json!({"audit_id": audit.id, "trigger": trigger, "challenger": challenger.id, "model": model})).await;
    Ok(audit)
}

/// Runs an audit: the challenge turn, then the rebuttal turn if there are risks. Never moves the
/// task. Each stage that ends without its tool call ends the audit with a reason.
pub async fn drive(state: &AppState, audit: &Audit) {
    let _unstick = Unstick(state.clone(), audit.id.clone());
    let _one = ONE.lock().await;
    let task = match state.db.get_task(&audit.task_id).await {
        Ok(Some(t)) if matches!(t.state.as_str(), "review" | "verified") => t,
        _ => {
            let _ = state.db.move_audit(&audit.id, "queued", "skipped", Some("the task moved on before its audit")).await;
            event(state, &audit.task_id, "audit_skipped", "kwant", json!({"audit_id": audit.id, "reason": "task_moved"})).await;
            return;
        }
    };
    if !state.db.move_audit(&audit.id, "queued", "challenge", None).await.unwrap_or(false) {
        return;
    }
    let (Ok(Some(challenger)), Ok(Some(owner))) = (state.db.get_kwant(&audit.challenger_kwant_id).await, state.db.get_kwant(&audit.owner_kwant_id).await) else {
        let _ = state.db.move_audit(&audit.id, "challenge", "failed", Some("a bot of the audit no longer exists")).await;
        return finish(state, audit).await;
    };
    let p = challenge_prompt(state, audit, &task, &owner).await;
    let err = run_stage(state, audit, &challenger, "challenge", p).await;
    if state.db.move_audit(&audit.id, "challenge", "failed", Some(&ended_without("challenge", err.as_deref()))).await.unwrap_or(false) {
        return finish(state, audit).await;
    }
    if matches!(state.db.get_audit(&audit.id).await, Ok(Some(a)) if a.status == "rebuttal") {
        let p = rebuttal_prompt(state, audit, &task, &challenger).await;
        let err = run_stage(state, audit, &owner, "rebuttal", p).await;
        // No answer: the audit ends with every risk still open
        if matches!(state.db.get_audit(&audit.id).await, Ok(Some(a)) if a.status == "rebuttal") && state.db.record_rebuttal(&audit.id, &[]).await.ok().flatten().is_some() {
            let _ = state.db.move_audit(&audit.id, "done", "done", Some(&ended_without("rebuttal", err.as_deref()))).await;
        }
    }
    finish(state, audit).await;
}

/// Ends the audit failed if its driver stops while it is still live (a panic, a dropped task),
/// so an audit never stays running; a normal end leaves a final status and this does nothing.
struct Unstick(AppState, String);

impl Drop for Unstick {
    fn drop(&mut self) {
        let (state, id) = (self.0.clone(), std::mem::take(&mut self.1));
        if let Ok(rt) = tokio::runtime::Handle::try_current() {
            rt.spawn(async move { end_if_live(&state, &id, "the audit stopped before it finished").await });
        }
    }
}

/// Ends audit `id` as `failed` with `reason` when it is still queued or in a stage, and logs its
/// end on the task. False when it had already ended.
pub async fn end_if_live(state: &AppState, id: &str, reason: &str) -> bool {
    let Ok(Some(a)) = state.db.get_audit(id).await else { return false };
    if !matches!(a.status.as_str(), "queued" | "challenge" | "rebuttal") {
        return false;
    }
    if !state.db.move_audit(id, &a.status, "failed", Some(reason)).await.unwrap_or(false) {
        return false;
    }
    tracing::warn!(audit_id = id, task_id = %a.task_id, reason, "audit ended");
    finish(state, &a).await;
    true
}

/// At startup, after the database marked the audits a restart cut off mid-turn `interrupted`:
/// the ones still `queued` (no turn had started) run now, or are skipped if the auditor was
/// turned off.
pub async fn resume_after_restart(state: &AppState) {
    let queued = state.db.audits_in("queued").await.unwrap_or_default();
    let on = enabled(state).await;
    for a in queued {
        if on {
            let s = state.clone();
            tokio::spawn(async move { drive(&s, &a).await });
        } else if state.db.move_audit(&a.id, "queued", "skipped", Some("the auditor was turned off")).await.unwrap_or(false) {
            event(state, &a.task_id, "audit_skipped", "kwant", json!({"audit_id": a.id, "reason": "auditor_off"})).await;
        }
    }
}

fn ended_without(stage: &str, err: Option<&str>) -> String {
    let who = if stage == "challenge" { "the challenger's turn ended without a challenge" } else { "the owner's turn ended without a rebuttal" };
    match err {
        Some(e) => format!("{who}: {}", e.lines().next().unwrap_or(e).chars().take(300).collect::<String>()),
        None => who.into(),
    }
}

async fn finish(state: &AppState, audit: &Audit) {
    drop_retries(&audit.id);
    let Ok(Some(a)) = state.db.get_audit(&audit.id).await else { return };
    let risks = state.db.risks_of_audit(&a.id).await.unwrap_or_default();
    let unresolved = risks.iter().filter(|r| r.unresolved()).count();
    event(
        state,
        &a.task_id,
        "audit_done",
        "kwant",
        json!({"audit_id": a.id, "status": a.status, "reason": a.reason, "risks": risks.len(), "unresolved": unresolved,
               "tokens": a.challenge_tokens + a.rebuttal_tokens}),
    )
    .await;
    super::emit_task_id(state, &a.task_id).await;
}

/// One hidden turn of `bot` in its direct chat, every change asking the person. Its tokens are
/// added to the stage. The turn's error, if any.
async fn run_stage(state: &AppState, audit: &Audit, bot: &Kwant, stage: &str, prompt: String) -> Option<String> {
    let started = Utc::now();
    let job = state.turns.start_job([bot.id.clone()]);
    let reply = match scope_for(state, bot, None, None, &job, 0).await {
        Ok(scope) => {
            let mut opts = TurnOpts::always_asking("Audit");
            (opts.unsaved, opts.quiet_failure, opts.purpose) = (true, true, Some(PURPOSE.into()));
            opts.tier = tier_of(model_mode(state).await);
            match tokio::time::timeout(TURN_TIMEOUT, turn_once(&scope, bot, prompt, opts)).await {
                Ok(r) => r,
                Err(_) => Err("the audit turn took longer than 20 minutes".into()),
            }
        }
        Err(e) => Err(e),
    };
    let tokens = state.db.tokens_between(&bot.id, started - Duration::seconds(1), Utc::now() + Duration::seconds(1)).await.unwrap_or(0);
    let _ = state.db.add_audit_tokens(&audit.id, stage, tokens).await;
    reply.err()
}

fn clip(s: &str, n: usize) -> String {
    let s = s.trim();
    if s.chars().count() <= n {
        s.to_string()
    } else {
        format!("{}…", s.chars().take(n).collect::<String>())
    }
}

async fn challenge_prompt(state: &AppState, audit: &Audit, task: &Task, owner: &Kwant) -> String {
    let evidence = state.db.list_evidence(&task.id).await.unwrap_or_default();
    let ev: Vec<String> = evidence
        .iter()
        .map(|e| {
            let at = e.uri.as_deref().or(e.path.as_deref()).unwrap_or("");
            format!("- [{}] {} \"{}\" {} {}", e.id, e.kind, e.label, at, clip(e.excerpt.as_deref().unwrap_or(""), 300))
        })
        .collect();
    let checks = state.db.list_checks(&task.id).await.unwrap_or_default();
    let ch: Vec<String> = checks
        .iter()
        .map(|c| format!("- {} {}: {}", c.kind, super::autofix::redact_value(&c.spec_value()), c.status))
        .collect();
    format!(
        "[Swarm Auditor] You are auditing task {tid} \"{title}\", done by {owner}. You are not its reviewer: \
         you can't verify or fail it, and nothing in this audit counts as proof either way; only its checks \
         or the person decide.\nAcceptance: {acc}\nEvidence attached:\n{ev}\nChecks:\n{ch}\n\
         Look for concrete reasons it may not meet its acceptance: wrong, missing, unsafe or unverifiable. \
         Read what you need, but change nothing. Then call audit_challenge once with audit_id {aid} and at \
         most {max} risks. Each risk needs `evidence`: what you looked at and what you saw (put ids from the \
         evidence list above in evidence_ids when they apply; the task's own id is not an evidence id). If you find no real risk, call it with an empty risks \
         list. Don't invent risks to have something to say.",
        tid = task.id,
        title = task.title,
        owner = owner.name,
        acc = if task.acceptance.is_empty() { "(none given)" } else { &task.acceptance },
        ev = if ev.is_empty() { "- none".into() } else { ev.join("\n") },
        ch = if ch.is_empty() { "- none: nothing outside will check it".into() } else { ch.join("\n") },
        aid = audit.id,
        max = MAX_RISKS,
    )
}

async fn rebuttal_prompt(state: &AppState, audit: &Audit, task: &Task, challenger: &Kwant) -> String {
    let risks = state.db.risks_of_audit(&audit.id).await.unwrap_or_default();
    let list: Vec<String> = risks.iter().map(|r| format!("- [{}] {} (evidence: {})", r.id, r.risk, clip(&r.evidence, 400))).collect();
    format!(
        "[Swarm Auditor] {who} challenged your task {tid} \"{title}\" with these risks:\n{list}\n\
         Call audit_rebut once with audit_id {aid} and an answer per risk: stance \"disputed\" when it \
         doesn't apply (give your evidence: what shows that), or \"accepted\" when it is a real risk. Change \
         nothing now: the person sees both sides and decides. Your answer doesn't verify the task.",
        who = challenger.name,
        tid = task.id,
        title = task.title,
        list = list.join("\n"),
        aid = audit.id,
    )
}

// Tools

type ToolResult = (String, bool);

fn fail(msg: impl Into<String>) -> ToolResult {
    (msg.into(), true)
}

fn ok(v: Value) -> ToolResult {
    (serde_json::to_string_pretty(&v).unwrap_or_default(), false)
}

pub const TOOL_NAMES: [&str; 2] = ["audit_challenge", "audit_rebut"];

pub fn tools() -> Vec<Value> {
    vec![
        json!({
            "name": "audit_challenge",
            "description": "Only when kwant asked you to audit a teammate's task: list the concrete risks you found, each with the evidence you looked at. Call it once; an empty list means you found none. It never verifies or fails the task.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "audit_id": {"type": "string"},
                    "risks": {
                        "type": "array",
                        "maxItems": MAX_RISKS,
                        "items": {
                            "type": "object",
                            "properties": {
                                "risk": {"type": "string", "description": "What may be wrong, in a sentence."},
                                "evidence": {"type": "string", "description": "What you looked at and what you saw."},
                                "evidence_ids": {"type": "array", "items": {"type": "string"}, "description": "Ids from the task's evidence list this rests on (not the task id). Leave it out when none apply."},
                            },
                            "required": ["risk", "evidence"],
                        },
                    },
                },
                "required": ["audit_id", "risks"],
            },
        }),
        json!({
            "name": "audit_rebut",
            "description": "Only when kwant asked you to answer an audit of your task: per risk, \"disputed\" (with your evidence) or \"accepted\". Call it once. The person decides; your answer never verifies the task.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "audit_id": {"type": "string"},
                    "answers": {
                        "type": "array",
                        "items": {
                            "type": "object",
                            "properties": {
                                "risk_id": {"type": "string"},
                                "stance": {"type": "string", "enum": ["disputed", "accepted"]},
                                "answer": {"type": "string"},
                                "evidence": {"type": "string", "description": "Needed when disputed: what shows the risk doesn't apply."},
                            },
                            "required": ["risk_id", "stance", "answer"],
                        },
                    },
                },
                "required": ["audit_id", "answers"],
            },
        }),
    ]
}

/// The audit of `task_id` still running, if any.
pub async fn live_audit(state: &AppState, task_id: &str) -> Option<Audit> {
    state.db.audits_of(task_id).await.ok()?.into_iter().find(|a| matches!(a.status.as_str(), "queued" | "challenge" | "rebuttal"))
}

/// Why `kwant_id` can't use `tool` on `task` now: it is in a live audit of that task. A queued
/// audit of a task that already left review (a check sent it back) holds nothing: it will be skipped.
pub async fn held(state: &AppState, kwant_id: &str, tool: &str, task: &Task) -> Option<String> {
    if !HELD_TOOLS.contains(&tool) {
        return None;
    }
    let a = live_audit(state, &task.id).await?;
    if a.status == "queued" && !matches!(task.state.as_str(), "review" | "verified") {
        return None;
    }
    (a.challenger_kwant_id == kwant_id || a.owner_kwant_id == kwant_id)
        .then(|| format!("{tool} isn't available on this task during its audit: an audit never verifies, fails or changes the task."))
}

fn text(v: &Value, max: usize) -> Result<String, String> {
    let s = v.as_str().map(str::trim).unwrap_or("");
    if s.is_empty() {
        return Err("is empty".into());
    }
    if s.chars().count() > max {
        return Err(format!("is longer than {max} characters"));
    }
    Ok(s.to_string())
}

pub async fn call(state: &AppState, scope: &SelfScope, name: &str, args: &Value) -> ToolResult {
    if !enabled(state).await {
        return fail("The auditor is off in Settings.");
    }
    let Some(audit) = (match args["audit_id"].as_str() {
        Some(id) => state.db.get_audit(id).await.ok().flatten(),
        None => None,
    }) else {
        return fail("No such audit.");
    };
    let res = match name {
        "audit_challenge" => challenge(state, scope, &audit, args).await,
        "audit_rebut" => rebut(state, scope, &audit, args).await,
        _ => fail(format!("Unknown tool {name}.")),
    };
    if res.1 {
        tracing::info!(audit_id = %audit.id, kwant = %scope.kwant_id, tool = name, reason = %res.0, "audit call refused");
        // Refused input from the stage's own bot: its corrected call doesn't ask again
        if let Ok(Some(a)) = state.db.get_audit(&audit.id).await {
            if waits_on(&a, name, &scope.kwant_id) {
                arm_retry(&scope.kwant_id, &a.id, name);
            }
        }
    }
    res
}

/// Whether `audit` is waiting for `tool` from `kwant_id`.
fn waits_on(audit: &Audit, tool: &str, kwant_id: &str) -> bool {
    match tool {
        "audit_challenge" => audit.status == "challenge" && audit.challenger_kwant_id == kwant_id,
        "audit_rebut" => audit.status == "rebuttal" && audit.owner_kwant_id == kwant_id,
        _ => false,
    }
}

/// How many corrected calls a stage may make without a new card.
const RETRIES: u8 = 2;

#[derive(Default)]
struct Retry {
    armed: bool,
    left: u8,
}

/// One challenge, one card: the person allows the audit call, and if the tool then refuses its
/// input (an evidence id that isn't the task's, a risk too long), the bot's corrected call goes
/// through without a second card. At most [`RETRIES`] per stage; the tool checks the call again
/// as usual. Keyed by (bot, audit, tool); dropped when the audit ends.
static RETRY_PASSES: LazyLock<std::sync::Mutex<HashMap<RetryKey, Retry>>> = LazyLock::new(Default::default);

/// (bot, audit, tool).
type RetryKey = (String, String, String);

fn arm_retry(kwant_id: &str, audit_id: &str, tool: &str) {
    let mut passes = RETRY_PASSES.lock().unwrap_or_else(|e| e.into_inner());
    let r = passes.entry((kwant_id.into(), audit_id.into(), tool.into())).or_insert(Retry { armed: false, left: RETRIES });
    r.armed = r.left > 0;
}

/// Uses the retry pass for this call, if a refused call of the same audit stage left one.
pub fn take_retry(kwant_id: &str, tool: &str, args: &Value) -> bool {
    let Some(audit_id) = args["audit_id"].as_str() else { return false };
    if !TOOL_NAMES.contains(&tool) {
        return false;
    }
    let mut passes = RETRY_PASSES.lock().unwrap_or_else(|e| e.into_inner());
    match passes.get_mut(&(kwant_id.to_string(), audit_id.to_string(), tool.to_string())) {
        Some(r) if r.armed => {
            r.armed = false;
            r.left -= 1;
            true
        }
        _ => false,
    }
}

fn drop_retries(audit_id: &str) {
    RETRY_PASSES.lock().unwrap_or_else(|e| e.into_inner()).retain(|(_, a, _), _| a != audit_id);
}

async fn challenge(state: &AppState, scope: &SelfScope, audit: &Audit, args: &Value) -> ToolResult {
    if audit.challenger_kwant_id != scope.kwant_id {
        return fail("You aren't this audit's challenger.");
    }
    if audit.status != "challenge" {
        return fail(format!("This audit isn't waiting for a challenge (it is {}).", audit.status));
    }
    let Some(items) = args["risks"].as_array() else { return fail("risks must be a list (empty if you found none).") };
    if items.len() > MAX_RISKS {
        return fail(format!("At most {MAX_RISKS} risks: keep the ones that matter most."));
    }
    let on_task: Vec<String> = state.db.list_evidence(&audit.task_id).await.unwrap_or_default().into_iter().map(|e| e.id).collect();
    let mut risks = Vec::new();
    for (i, r) in items.iter().enumerate() {
        let n = i + 1;
        let risk = match text(&r["risk"], MAX_RISK_CHARS) {
            Ok(s) => s,
            Err(e) => return fail(format!("risk {n}: risk {e}.")),
        };
        let evidence = match text(&r["evidence"], MAX_EVIDENCE_CHARS) {
            Ok(s) => s,
            Err(e) => return fail(format!("risk {n}: evidence {e}; say what you looked at and what you saw.")),
        };
        // The task's own id names no evidence: dropped rather than refused
        let ids: Vec<String> = r["evidence_ids"]
            .as_array()
            .map(|a| a.iter().filter_map(|v| v.as_str().map(String::from)).filter(|id| *id != audit.task_id).collect())
            .unwrap_or_default();
        if let Some(bad) = ids.iter().find(|id| !on_task.contains(id)) {
            return fail(format!("risk {n}: evidence id {bad} isn't one of the task's evidence ids; leave it out or use an id from the evidence list."));
        }
        risks.push(NewRisk { risk, evidence, evidence_ids: ids });
    }
    match state.db.record_challenge(&audit.id, &risks).await {
        Ok(Some(rows)) => {
            event(state, &audit.task_id, "audit_challenge", &scope.kwant_id, json!({"audit_id": audit.id, "risks": rows.len()})).await;
            ok(json!({
                "audit_id": audit.id,
                "risks": rows.iter().map(|r| json!({"id": r.id, "risk": r.risk})).collect::<Vec<_>>(),
                "next": if rows.is_empty() { "Recorded: no risks raised. That isn't a pass; the task's checks still decide." } else { "Recorded. The owner answers once; the person sees both sides." },
            }))
        }
        Ok(None) => fail("This audit isn't waiting for a challenge any more."),
        Err(e) => fail(format!("Could not record the challenge: {e}")),
    }
}

async fn rebut(state: &AppState, scope: &SelfScope, audit: &Audit, args: &Value) -> ToolResult {
    if audit.owner_kwant_id != scope.kwant_id {
        return fail("Only the task's owner answers its audit.");
    }
    if audit.status != "rebuttal" {
        return fail(format!("This audit isn't waiting for a rebuttal (it is {}).", audit.status));
    }
    let risks = state.db.risks_of_audit(&audit.id).await.unwrap_or_default();
    let Some(items) = args["answers"].as_array() else { return fail("answers must be a list.") };
    let mut answers: Vec<Answer> = Vec::new();
    for (i, a) in items.iter().enumerate() {
        let n = i + 1;
        let Some(risk_id) = a["risk_id"].as_str().filter(|id| risks.iter().any(|r| r.id == *id)) else {
            return fail(format!("answer {n}: risk_id isn't one of this audit's risks."));
        };
        if answers.iter().any(|x| x.risk_id == risk_id) {
            return fail(format!("answer {n}: risk {risk_id} is answered twice."));
        }
        let stance = match a["stance"].as_str() {
            Some(s @ ("disputed" | "accepted")) => s.to_string(),
            _ => return fail(format!("answer {n}: stance must be disputed or accepted.")),
        };
        let answer = match text(&a["answer"], MAX_EVIDENCE_CHARS) {
            Ok(s) => s,
            Err(e) => return fail(format!("answer {n}: answer {e}.")),
        };
        let evidence = match (text(&a["evidence"], MAX_EVIDENCE_CHARS), stance.as_str()) {
            (Ok(s), _) => Some(s),
            (Err(_), "disputed") => return fail(format!("answer {n}: a disputed risk needs evidence: what shows it doesn't apply.")),
            (Err(_), _) => None,
        };
        answers.push(Answer { risk_id: risk_id.to_string(), stance, answer, evidence });
    }
    match state.db.record_rebuttal(&audit.id, &answers).await {
        Ok(Some(rows)) => {
            let disputed = rows.iter().filter(|r| r.status == "disputed").count();
            event(state, &audit.task_id, "audit_rebuttal", &scope.kwant_id, json!({"audit_id": audit.id, "answered": answers.len(), "disputed": disputed})).await;
            ok(json!({
                "audit_id": audit.id,
                "unresolved": rows.iter().filter(|r| r.unresolved()).count(),
                "next": "Recorded. Disputed and open risks stay listed on the task until the person closes them.",
            }))
        }
        Ok(None) => fail("This audit isn't waiting for a rebuttal any more."),
        Err(e) => fail(format!("Could not record the rebuttal: {e}")),
    }
}

// Routes

/// `GET /api/tasks/:id/risks`: the task's audits with their risks, and how many are unresolved.
pub async fn task_risks(State(state): State<AppState>, Path(id): Path<String>) -> Response {
    match state.db.get_task(&id).await {
        Ok(Some(_)) => {}
        Ok(None) => return not_found("task"),
        Err(e) => return internal(e),
    }
    let audits = match state.db.audits_of(&id).await {
        Ok(a) => a,
        Err(e) => return internal(e),
    };
    let mut out = Vec::new();
    let mut open = 0;
    for a in &audits {
        let risks = state.db.risks_of_audit(&a.id).await.unwrap_or_default();
        open += risks.iter().filter(|r| r.unresolved()).count();
        out.push(audit_json(a, &risks));
    }
    Json(json!({"audits": out, "open_risks": open})).into_response()
}

/// `GET /api/risks?space=`: unresolved risks across the space, newest first.
pub async fn space_risks(State(state): State<AppState>, Scope(space): Scope) -> Response {
    match state.db.unresolved_risks(space.as_str(), 200).await {
        Ok(rows) => {
            let list: Vec<Value> = rows
                .iter()
                .map(|r| {
                    let mut j = serde_json::to_value(r).unwrap_or_default();
                    j["evidence_ids"] = json!(r.evidence_ids());
                    j
                })
                .collect();
            Json(json!({"risks": list})).into_response()
        }
        Err(e) => internal(e),
    }
}

/// `POST /api/tasks/:id/audit` (person only): audit the task's current attempt now.
pub async fn start_audit(State(state): State<AppState>, Path(id): Path<String>) -> Response {
    if !enabled(&state).await {
        return conflict("The auditor is off in Settings.");
    }
    let task = match state.db.get_task(&id).await {
        Ok(Some(t)) => t,
        Ok(None) => return not_found("task"),
        Err(e) => return internal(e),
    };
    if !matches!(task.state.as_str(), "review" | "verified") {
        return conflict("Only a task in review or verified can be audited.");
    }
    match begin(&state, &task, "manual").await {
        Ok(a) => (StatusCode::ACCEPTED, Json(audit_json(&a, &[]))).into_response(),
        Err(why) => conflict(why),
    }
}

#[derive(Deserialize)]
pub struct ResolveBody {
    resolution: String,
    note: Option<String>,
}

/// `POST /api/tasks/:id/risks/:risk_id/resolve` (person only): close an unresolved risk.
pub async fn resolve_risk(State(state): State<AppState>, Path((id, risk_id)): Path<(String, String)>, Json(body): Json<ResolveBody>) -> Response {
    if !matches!(body.resolution.as_str(), "resolved" | "dismissed") {
        return bad("resolution must be resolved or dismissed");
    }
    let note = body.note.as_deref().map(str::trim).filter(|s| !s.is_empty()).map(|s| clip(s, MAX_EVIDENCE_CHARS));
    match state.db.get_risk(&risk_id).await {
        Ok(Some(r)) if r.task_id == id => {}
        Ok(_) => return not_found("risk"),
        Err(e) => return internal(e),
    }
    match state.db.resolve_risk(&risk_id, &body.resolution, note.as_deref()).await {
        Ok(true) => {
            event(&state, &id, "risk_resolved", "person", json!({"risk_id": risk_id, "resolution": body.resolution})).await;
            super::emit_task_id(&state, &id).await;
            match state.db.get_risk(&risk_id).await {
                Ok(Some(r)) => Json(json!({"risk": r, "unresolved": r.unresolved()})).into_response(),
                _ => internal("the risk vanished"),
            }
        }
        Ok(false) => conflict("This risk is already closed."),
        Err(e) => internal(e),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn settings_are_checked() {
        assert!(settings_error(Some("on"), Some(5), Some("fast")).is_none());
        assert!(settings_error(Some("suggest"), None, None).is_some());
        assert!(settings_error(None, Some(0), None).is_some());
        assert!(settings_error(None, Some(51), None).is_some());
        assert!(settings_error(None, None, Some("bot")).is_none());
        assert!(settings_error(None, None, Some("haiku")).is_some());
    }

    #[test]
    fn audit_turns_run_on_the_fast_tier_unless_told_otherwise() {
        assert_eq!(tier_of(DEFAULT_MODEL), Some(Tier::Fast));
        assert_eq!(tier_of("balanced"), Some(Tier::Balanced));
        assert_eq!(tier_of("deep"), Some(Tier::Deep));
        assert_eq!(tier_of("bot"), None);
    }

    #[test]
    fn a_stage_without_its_call_says_why() {
        assert_eq!(ended_without("challenge", None), "the challenger's turn ended without a challenge");
        assert!(ended_without("rebuttal", Some("boom\nmore")).ends_with(": boom"));
    }
}
