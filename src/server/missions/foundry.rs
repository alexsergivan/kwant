//! Mission Control Phase D3: the Skill Foundry. When a bot has finished the same class of work
//! well several times (a cluster: ≥ 3 verified tasks of one class, ≥ 70% verified, in 60 days),
//! kwant asks that bot, in one hidden turn, to draft a skill from those tasks with `skill_draft`.
//! The draft is an ordinary `pending` skill plus its inputs, checks, examples and the tools it
//! uses. Each draft is then replayed as a dry run on a scratch copy of up to 3 of its examples:
//! nothing outside the scratch folder runs, and the report goes on the draft card. The person
//! enables, edits or rejects it, and can roll a lineage back to an earlier version. The tools a
//! draft lists are shown, never granted. Off by default. See docs/mission-control.md §19.

use super::routes::{bad, conflict, internal, not_found};
use crate::db::foundry::{cluster_id, split_cluster, Cluster, FoundryRun, NewDraft, SkillMeta};
use crate::db::models::Kwant;
use crate::db::skills::{clean_skill_text, Skill, SkillError};
use crate::drivers::engines::Tier;
use crate::server::continuity::SelfScope;
use crate::server::mcp_runtime::RehearseLog;
use crate::server::routes::{scope_for, turn_once, TurnOpts};
use crate::server::spaces::Scope;
use crate::server::AppState;
use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use chrono::{DateTime, Duration, Utc};
use serde_json::{json, Value};
use std::path::{Path as FsPath, PathBuf};
use std::sync::LazyLock;
use tokio_util::sync::CancellationToken;

pub const SETTING: &str = "skill_foundry";
pub const MODES: [&str; 2] = ["off", "on"];
pub const MODEL_SETTING: &str = "skill_foundry_model";
pub const MODEL_MODES: [&str; 4] = ["fast", "balanced", "deep", "bot"];
pub const DEFAULT_MODEL: &str = "balanced";
pub const DRAFTS_SETTING: &str = "skill_foundry_daily_drafts";
pub const DEFAULT_DRAFTS: i64 = 2;
pub const DRAFTS_RANGE: (i64, i64) = (1, 20);
pub const REPLAYS_SETTING: &str = "skill_foundry_replays";
pub const DEFAULT_REPLAYS: i64 = 3;
/// 0 turns replays off: drafts are then shown without a report.
pub const REPLAYS_RANGE: (i64, i64) = (0, 5);
pub const REPLAY_TOKENS_SETTING: &str = "skill_foundry_replay_tokens";
pub const DEFAULT_REPLAY_TOKENS: i64 = 60_000;
pub const REPLAY_TOKENS_RANGE: (i64, i64) = (5_000, 500_000);

pub const TOOL: &str = "skill_draft";
pub const DRAFT_PURPOSE: &str = "foundry_draft";
pub const REPLAY_PURPOSE: &str = "foundry_replay";
pub const MAX_INPUTS: usize = 8;
pub const MAX_CHECKS: usize = 5;
pub const MIN_EXAMPLES: usize = 2;
pub const MAX_EXAMPLES: usize = 5;
pub const MAX_TOOLS: usize = 30;
const MAX_FIELD_CHARS: usize = 300;
/// Tasks a draft prompt shows: verified ones, and failed ones for contrast.
const PROMPT_VERIFIED: i64 = 5;
const PROMPT_FAILED: i64 = 2;
const DRAFT_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(20 * 60);
const REPLAY_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(15 * 60);
/// How often a replay's token cap is checked while its turn runs.
const CAP_POLL: std::time::Duration = std::time::Duration::from_secs(2);
const TICK: std::time::Duration = std::time::Duration::from_secs(60 * 60);
/// What the original task did, listed next to a replay's "would have…" lines.
const MAX_ORIGINAL_ACTIONS: usize = 20;

/// One Foundry turn at a time, server-wide.
static ONE: LazyLock<tokio::sync::Mutex<()>> = LazyLock::new(|| tokio::sync::Mutex::new(()));

pub async fn enabled(state: &AppState) -> bool {
    matches!(state.db.get_setting(SETTING).await.ok().flatten().as_deref().map(str::trim), Some("on"))
}

pub async fn model_mode(state: &AppState) -> &'static str {
    let saved = state.db.get_setting(MODEL_SETTING).await.ok().flatten();
    MODEL_MODES.into_iter().find(|m| Some(*m) == saved.as_deref().map(str::trim)).unwrap_or(DEFAULT_MODEL)
}

pub async fn daily_drafts(state: &AppState) -> i64 {
    super::int_setting(state, DRAFTS_SETTING, DRAFTS_RANGE, DEFAULT_DRAFTS).await
}

pub async fn replays(state: &AppState) -> i64 {
    super::int_setting(state, REPLAYS_SETTING, REPLAYS_RANGE, DEFAULT_REPLAYS).await
}

pub async fn replay_tokens(state: &AppState) -> i64 {
    super::int_setting(state, REPLAY_TOKENS_SETTING, REPLAY_TOKENS_RANGE, DEFAULT_REPLAY_TOKENS).await
}

pub fn tier_of(mode: &str) -> Option<Tier> {
    super::auditor::tier_of(mode)
}

pub fn settings_error(mode: Option<&str>, model: Option<&str>, drafts: Option<i64>, replays: Option<i64>, tokens: Option<i64>) -> Option<String> {
    if let Some(m) = mode.filter(|m| !MODES.contains(m)) {
        return Some(format!("skill_foundry must be off or on (got {m})"));
    }
    if let Some(m) = model.filter(|m| !MODEL_MODES.contains(m)) {
        return Some(format!("skill_foundry_model must be fast, balanced, deep or bot (got {m})"));
    }
    for (key, value, (lo, hi)) in [(DRAFTS_SETTING, drafts, DRAFTS_RANGE), (REPLAYS_SETTING, replays, REPLAYS_RANGE), (REPLAY_TOKENS_SETTING, tokens, REPLAY_TOKENS_RANGE)] {
        if let Some(n) = value.filter(|n| !(lo..=hi).contains(n)) {
            return Some(format!("{key} must be {lo}–{hi} (got {n})"));
        }
    }
    None
}

fn midnight() -> DateTime<Utc> {
    Utc::now().date_naive().and_hms_opt(0, 0, 0).map(|d| d.and_utc()).unwrap_or_else(Utc::now)
}

fn clip(s: &str, n: usize) -> String {
    let s = s.trim();
    if s.chars().count() <= n {
        s.to_string()
    } else {
        format!("{}…", s.chars().take(n).collect::<String>())
    }
}

async fn skill_event(state: &AppState, skill_id: &str, kind: &str, actor: &str, detail: Value) {
    if let Err(e) = state.db.log_skill_event(skill_id, kind, actor, &detail).await {
        tracing::warn!(skill_id, kind, error = %e, "skill event not logged");
    }
}

fn changed(state: &AppState, skill: &Skill) {
    let _ = state.event_sender.send(crate::drivers::events::RuntimeEvent::SkillUpdated {
        skill: serde_json::to_value(skill).unwrap_or(Value::Null),
    });
}

// The miner

/// The hourly tick: while the Foundry is on, offers one draft for each cluster with no block, up
/// to the daily cap.
pub fn spawn(state: AppState, shutdown: CancellationToken) {
    tokio::spawn(async move {
        loop {
            tokio::select! {
                _ = shutdown.cancelled() => return,
                _ = tokio::time::sleep(TICK) => {}
            }
            if enabled(&state).await {
                tick(&state).await;
            }
        }
    });
}

/// One pass of the miner; the runs it started.
pub async fn tick(state: &AppState) -> Vec<FoundryRun> {
    let now = Utc::now();
    let mut started = Vec::new();
    for c in state.db.foundry_clusters(None, now).await.unwrap_or_default() {
        if !matches!(state.db.foundry_block(&c.kwant_id, &c.task_class, now).await, Ok(None)) {
            continue;
        }
        match begin_draft(state, &c, "kwant").await {
            Ok(run) => started.push(run),
            // The cap: no other cluster gets one today either
            Err(why) if why.contains("daily") => break,
            Err(why) => tracing::info!(cluster = %c.id(), why, "no Foundry draft"),
        }
    }
    started
}

/// Opens a draft run for `cluster` (`queued`) and drives it in the background. Refused when the
/// Foundry is off, the daily cap is reached, a draft for the class waits for review, or a draft
/// run for it is already live.
pub async fn begin_draft(state: &AppState, cluster: &Cluster, actor: &str) -> Result<FoundryRun, String> {
    let run = start_draft(state, cluster, actor).await?;
    let (s, r) = (state.clone(), run.clone());
    tokio::spawn(async move { drive_draft(&s, &r).await });
    Ok(run)
}

pub async fn start_draft(state: &AppState, cluster: &Cluster, actor: &str) -> Result<FoundryRun, String> {
    if !enabled(state).await {
        return Err("the Skill Foundry is off in Settings".into());
    }
    let cap = daily_drafts(state).await;
    if state.db.draft_runs_since(midnight()).await.map_err(|e| e.to_string())? >= cap {
        return Err(format!("the daily Foundry draft cap ({cap}) is reached"));
    }
    if matches!(state.db.foundry_block(&cluster.kwant_id, &cluster.task_class, Utc::now()).await, Ok(Some("has_draft"))) {
        return Err("a draft for this class is already waiting for review".into());
    }
    if let Ok(Some(_)) = state.db.live_foundry_run("draft", &cluster.kwant_id, &cluster.task_class, None).await {
        return Err("a draft for this class is already being written".into());
    }
    let run = state.db.create_foundry_run("draft", &cluster.space_id, &cluster.kwant_id, &cluster.task_class, None).await.map_err(|e| e.to_string())?;
    tracing::info!(run_id = %run.id, cluster = %cluster.id(), actor, "Foundry draft queued");
    Ok(run)
}

/// The draft turn: the owner bot, always asking, is asked to call `skill_draft` once. A turn that
/// ends without a draft ends the run `failed`; a draft queues its replay.
pub async fn drive_draft(state: &AppState, run: &FoundryRun) {
    let _unstick = Unstick(state.clone(), run.id.clone());
    let _one = ONE.lock().await;
    if !state.db.move_foundry_run(&run.id, "queued", "drafting", None).await.unwrap_or(false) {
        return;
    }
    let Ok(Some(owner)) = state.db.get_kwant(&run.kwant_id).await else {
        let _ = state.db.move_foundry_run(&run.id, "drafting", "failed", Some("the bot no longer exists")).await;
        return;
    };
    let prompt = draft_prompt(state, run).await;
    let job = state.turns.start_job([owner.id.clone()]);
    let started = Utc::now();
    let reply = match scope_for(state, &owner, None, None, &job, 0).await {
        Ok(scope) => {
            let mut opts = TurnOpts::always_asking("Skill Foundry");
            (opts.unsaved, opts.quiet_failure, opts.purpose) = (true, true, Some(DRAFT_PURPOSE.into()));
            opts.tier = tier_of(model_mode(state).await);
            match tokio::time::timeout(DRAFT_TIMEOUT, turn_once(&scope, &owner, prompt, opts)).await {
                Ok(r) => r,
                Err(_) => Err("the draft turn took longer than 20 minutes".into()),
            }
        }
        Err(e) => Err(e),
    };
    drop(job);
    let tokens = state.db.tokens_between(&owner.id, started - Duration::seconds(1), Utc::now() + Duration::seconds(1)).await.unwrap_or(0);
    let _ = state.db.add_foundry_tokens(&run.id, tokens).await;
    let skill_id = state.db.get_foundry_run(&run.id).await.ok().flatten().and_then(|r| r.skill_id);
    match skill_id {
        Some(skill_id) => {
            let _ = state.db.move_foundry_run(&run.id, "drafting", "done", None).await;
            drop(_one);
            if replays(state).await > 0 {
                if let Err(why) = begin_replay(state, &skill_id, "kwant").await {
                    tracing::info!(skill_id, why, "no replay for the Foundry draft");
                }
            }
        }
        None => {
            let why = match reply.err() {
                Some(e) => format!("the bot's turn ended without a draft: {}", clip(e.lines().next().unwrap_or(&e), 300)),
                None => "the bot's turn ended without a draft".into(),
            };
            let _ = state.db.move_foundry_run(&run.id, "drafting", "failed", Some(&why)).await;
        }
    }
}

/// Ends a run `failed` if its driver stops while it is still live (a panic, a dropped task).
struct Unstick(AppState, String);

impl Drop for Unstick {
    fn drop(&mut self) {
        let (state, id) = (self.0.clone(), std::mem::take(&mut self.1));
        if let Ok(rt) = tokio::runtime::Handle::try_current() {
            rt.spawn(async move { end_if_live(&state, &id, "the run stopped before it finished").await });
        }
    }
}

pub async fn end_if_live(state: &AppState, id: &str, reason: &str) -> bool {
    let Ok(Some(r)) = state.db.get_foundry_run(id).await else { return false };
    if !r.live() {
        return false;
    }
    let ended = state.db.move_foundry_run(id, &r.status, "failed", Some(reason)).await.unwrap_or(false);
    if ended {
        tracing::warn!(run_id = id, reason, "Foundry run ended");
    }
    ended
}

/// At startup, after the database marked runs a restart cut off `interrupted`: queued ones run
/// now, or fail if the Foundry was turned off.
pub async fn resume_after_restart(state: &AppState) {
    let on = enabled(state).await;
    for r in state.db.foundry_runs_in("queued").await.unwrap_or_default() {
        if !on {
            let _ = state.db.move_foundry_run(&r.id, "queued", "failed", Some("the Skill Foundry was turned off")).await;
            continue;
        }
        let s = state.clone();
        tokio::spawn(async move {
            match r.kind.as_str() {
                "draft" => drive_draft(&s, &r).await,
                _ => drive_replay(&s, &r).await,
            }
        });
    }
}

/// What a task looked like, for a prompt: title, acceptance, checks, evidence, what it did.
async fn task_brief(state: &AppState, task_id: &str) -> Option<String> {
    let task = state.db.get_task(task_id).await.ok().flatten()?;
    let checks: Vec<String> = state
        .db
        .list_checks(task_id)
        .await
        .unwrap_or_default()
        .iter()
        .map(|c| format!("{} {} ({})", c.kind, super::autofix::redact_value(&c.spec_value()), c.status))
        .collect();
    let evidence: Vec<String> = state
        .db
        .list_evidence(task_id)
        .await
        .unwrap_or_default()
        .iter()
        .take(5)
        .map(|e| format!("{} \"{}\" {}", e.kind, clip(&e.label, 120), clip(e.excerpt.as_deref().unwrap_or(""), 200)))
        .collect();
    let did = original_actions(state, task_id).await;
    Some(format!(
        "- task {id} \"{title}\"\n  acceptance: {acc}\n  checks: {ch}\n  evidence: {ev}\n  what it did: {did}",
        id = task.id,
        title = clip(&task.title, 200),
        acc = if task.acceptance.trim().is_empty() { "(none)".into() } else { clip(&task.acceptance, 600) },
        ch = if checks.is_empty() { "(none)".into() } else { checks.join("; ") },
        ev = if evidence.is_empty() { "(none)".into() } else { evidence.join("; ") },
        did = if did.is_empty() { "(no tool calls recorded)".into() } else { did.iter().take(10).cloned().collect::<Vec<_>>().join("; ") },
    ))
}

/// What the task's turns did that the side-effect ledger recorded, oldest first.
async fn original_actions(state: &AppState, task_id: &str) -> Vec<String> {
    state
        .db
        .tool_calls_of(task_id)
        .await
        .unwrap_or_default()
        .into_iter()
        .filter(|c| c.effect != "read")
        .take(MAX_ORIGINAL_ACTIONS)
        .map(|c| format!("{} ({})", clip(&c.summary, 200), c.decision))
        .collect()
}

async fn draft_prompt(state: &AppState, run: &FoundryRun) -> String {
    let now = Utc::now();
    let verified = state.db.cluster_tasks(&run.space_id, &run.kwant_id, &run.task_class, "verified", now, PROMPT_VERIFIED).await.unwrap_or_default();
    let failed = state.db.cluster_tasks(&run.space_id, &run.kwant_id, &run.task_class, "failed", now, PROMPT_FAILED).await.unwrap_or_default();
    let mut ok = Vec::new();
    for t in &verified {
        ok.extend(task_brief(state, t).await);
    }
    let mut bad = Vec::new();
    for t in &failed {
        bad.extend(task_brief(state, t).await);
    }
    format!(
        "[Skill Foundry] You finished several \"{class}\" tasks that an outside check verified. Write the \
         procedure you followed as a skill another run of you can reuse.\nVerified tasks:\n{ok}\nFailed tasks \
         of the same class, for contrast:\n{bad}\n\
         Call {tool} once with run_id {run}: a name (lowercase-with-dashes), a one-line description, a body \
         with the steps (what to check first, what to do, how to know it's done), the inputs a task must give \
         for it to apply, up to {mc} checks (the same kinds as a task's checks) that judge the result, 2–{mx} of \
         the verified task ids above as examples, and the tools the steps use (names you already have). \
         Listing a tool grants nothing. Change nothing else now. If the tasks don't share a procedure, \
         don't call {tool}; say why instead.",
        class = run.task_class,
        ok = if ok.is_empty() { "- none found".into() } else { ok.join("\n") },
        bad = if bad.is_empty() { "- none".into() } else { bad.join("\n") },
        tool = TOOL,
        run = run.id,
        mc = MAX_CHECKS,
        mx = MAX_EXAMPLES,
    )
}

// The tool

type ToolResult = (String, bool);

fn fail(msg: impl Into<String>) -> ToolResult {
    (msg.into(), true)
}

fn ok(v: Value) -> ToolResult {
    (serde_json::to_string_pretty(&v).unwrap_or_default(), false)
}

pub fn tool() -> Value {
    json!({
        "name": TOOL,
        "description": "Only when kwant asked you to draft a skill (Skill Foundry): store the procedure you followed in the listed tasks as a skill draft, for the person to review. Call it once. Listing tools grants nothing.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "run_id": {"type": "string"},
                "name": {"type": "string", "description": "lowercase-with-dashes, like release-notes"},
                "description": {"type": "string", "description": "One line: when to use it."},
                "body": {"type": "string", "description": "The steps."},
                "inputs": {"type": "array", "maxItems": MAX_INPUTS, "items": {"type": "object", "properties": {
                    "name": {"type": "string"}, "description": {"type": "string"}, "example": {"type": "string"}}, "required": ["name", "description"]}},
                "checks": {"type": "array", "maxItems": MAX_CHECKS, "items": {"type": "object", "properties": {
                    "kind": {"type": "string", "enum": crate::db::missions::KWANT_CHECK_KINDS},
                    "spec": {"type": "object", "description": "As a task's check: command {command}; http {url}; file_exists {path}; file_contains {path, text}; file_sha256 {path, sha256}."}},
                    "required": ["kind", "spec"]}},
                "examples": {"type": "array", "minItems": MIN_EXAMPLES, "maxItems": MAX_EXAMPLES, "items": {"type": "string"}, "description": "Ids of the verified tasks above."},
                "tools": {"type": "array", "maxItems": MAX_TOOLS, "items": {"type": "string"}, "description": "Tools the steps use, by the names you have them under."},
            },
            "required": ["run_id", "name", "description", "body", "examples"],
        },
    })
}

fn field(v: &Value, what: &str, required: bool) -> Result<Option<String>, String> {
    let s = v.as_str().map(str::trim).unwrap_or("");
    if s.is_empty() {
        return if required { Err(format!("{what} is empty")) } else { Ok(None) };
    }
    if s.chars().count() > MAX_FIELD_CHARS {
        return Err(format!("{what} is longer than {MAX_FIELD_CHARS} characters"));
    }
    Ok(Some(s.to_string()))
}

/// Checks a draft's inputs, checks and tools. `held` is every tool name the bot has now.
pub fn validate_parts(inputs: &Value, checks: &Value, tools: &Value, held: &[String]) -> Result<(Value, Value, Vec<String>), String> {
    let inputs = match inputs {
        Value::Null => Vec::new(),
        Value::Array(a) => a.clone(),
        _ => return Err("inputs must be a list".into()),
    };
    if inputs.len() > MAX_INPUTS {
        return Err(format!("at most {MAX_INPUTS} inputs"));
    }
    let mut clean_inputs = Vec::new();
    for (i, x) in inputs.iter().enumerate() {
        let n = i + 1;
        let name = field(&x["name"], &format!("input {n} name"), true)?;
        let description = field(&x["description"], &format!("input {n} description"), true)?;
        let example = field(&x["example"], &format!("input {n} example"), false)?;
        clean_inputs.push(json!({"name": name, "description": description, "example": example}));
    }
    let checks = match checks {
        Value::Null => Vec::new(),
        Value::Array(a) => a.clone(),
        _ => return Err("checks must be a list".into()),
    };
    if checks.len() > MAX_CHECKS {
        return Err(format!("at most {MAX_CHECKS} checks"));
    }
    let mut clean_checks = Vec::new();
    for (i, c) in checks.iter().enumerate() {
        let n = i + 1;
        let kind = c["kind"].as_str().unwrap_or("");
        if !crate::db::missions::KWANT_CHECK_KINDS.contains(&kind) {
            return Err(format!("check {n}: kind must be one of {}", crate::db::missions::KWANT_CHECK_KINDS.join(", ")));
        }
        if !c["spec"].is_object() {
            return Err(format!("check {n}: spec must be an object"));
        }
        super::validate_check(kind, &c["spec"], false).map_err(|e| format!("check {n}: {e}"))?;
        clean_checks.push(json!({"kind": kind, "spec": c["spec"]}));
    }
    let tools = match tools {
        Value::Null => Vec::new(),
        Value::Array(a) => a.clone(),
        _ => return Err("tools must be a list".into()),
    };
    if tools.len() > MAX_TOOLS {
        return Err(format!("at most {MAX_TOOLS} tools"));
    }
    let mut clean_tools: Vec<String> = Vec::new();
    for t in &tools {
        let Some(name) = t.as_str().map(str::trim).filter(|s| !s.is_empty()) else {
            return Err("each tool must be a name".into());
        };
        if !crate::mcp::builtin_tools::known(name) && !held.iter().any(|h| h == name) {
            return Err(format!("you don't have a tool called {name}; list only tools you have now (a skill can't ask for new ones)"));
        }
        if !clean_tools.iter().any(|c| c == name) {
            clean_tools.push(name.to_string());
        }
    }
    Ok((Value::Array(clean_inputs), Value::Array(clean_checks), clean_tools))
}

/// Every tool name `scope`'s bot holds now, past built-ins and server tools: kwant's own tools,
/// bare and as `mcp__<kwant server>__…`; and the bot's active servers.
async fn held_tools(state: &AppState, scope: &SelfScope) -> (Vec<String>, Vec<String>) {
    let mut held = Vec::new();
    for t in crate::server::continuity::self_tool_names(state, scope).await {
        held.push(format!("mcp__{}__{t}", crate::mcp::SELF_SERVER_NAME));
        held.push(t);
    }
    let servers: Vec<String> = state.db.list_active_mcp_servers_for_kwant(&scope.kwant_id).await.unwrap_or_default().into_iter().map(|s| s.name).collect();
    (held, servers)
}

/// `name` is a tool of one of `servers` (`mcp__server__tool`): kwant can't list a server's tools
/// without starting it, so the server is what's checked.
fn on_server(name: &str, servers: &[String]) -> bool {
    crate::mcp::split_tool_name(name).is_some_and(|(server, tool)| !tool.is_empty() && servers.iter().any(|s| s == server))
}

pub async fn call(state: &AppState, scope: &SelfScope, args: &Value) -> ToolResult {
    if !enabled(state).await {
        return fail("The Skill Foundry is off in Settings.");
    }
    let run = match args["run_id"].as_str() {
        Some(id) => state.db.get_foundry_run(id).await.ok().flatten(),
        None => None,
    };
    let Some(run) = run.filter(|r| r.kind == "draft" && r.kwant_id == scope.kwant_id) else {
        return fail("No such Skill Foundry draft run for you.");
    };
    if run.status != "drafting" || run.skill_id.is_some() {
        return fail("This draft run isn't waiting for a draft (call skill_draft once).");
    }
    let text = match clean_skill_text(args["name"].as_str().unwrap_or(""), args["description"].as_str().unwrap_or(""), args["body"].as_str().unwrap_or("")) {
        Ok(t) => t,
        Err(e) => return fail(format!("{e}")),
    };
    let examples: Vec<String> = match args["examples"].as_array() {
        Some(a) => a.iter().filter_map(|v| v.as_str().map(|s| s.trim().to_string())).collect(),
        None => return fail("examples must be a list of task ids."),
    };
    if !(MIN_EXAMPLES..=MAX_EXAMPLES).contains(&examples.len()) {
        return fail(format!("Give {MIN_EXAMPLES}–{MAX_EXAMPLES} example task ids."));
    }
    let cluster = state.db.cluster_tasks(&run.space_id, &run.kwant_id, &run.task_class, "verified", Utc::now(), 1000).await.unwrap_or_default();
    let mut seen = Vec::new();
    for e in &examples {
        if !cluster.contains(e) {
            return fail(format!("Example {e} isn't one of your verified \"{}\" tasks.", run.task_class));
        }
        if seen.contains(e) {
            return fail(format!("Example {e} is listed twice."));
        }
        seen.push(e.clone());
    }
    let (held, servers) = held_tools(state, scope).await;
    let tools_arg = match &args["tools"] {
        Value::Array(a) => Value::Array(a.iter().filter(|t| !t.as_str().is_some_and(|n| on_server(n, &servers))).cloned().collect()),
        other => other.clone(),
    };
    let (inputs, checks, mut tools) = match validate_parts(&args["inputs"], &args["checks"], &tools_arg, &held) {
        Ok(p) => p,
        Err(e) => return fail(format!("{e}.")),
    };
    // Server tools (checked by server above) keep their place after the others
    if let Some(a) = args["tools"].as_array() {
        for n in a.iter().filter_map(Value::as_str).map(str::trim).filter(|n| on_server(n, &servers)) {
            if !tools.iter().any(|t| t == n) {
                tools.push(n.to_string());
            }
        }
    }
    let draft = NewDraft { kwant_id: run.kwant_id.clone(), task_class: run.task_class.clone(), text, inputs, checks, examples: examples.clone(), tools };
    match state.db.insert_foundry_draft(&run.space_id, &draft, &run.id).await {
        Ok((skill, meta)) => {
            let _ = state.db.set_foundry_run_skill(&run.id, &skill.id).await;
            for t in &examples {
                if let Err(e) = state.db.log_task_event(t, "skill_drafted_from", &scope.kwant_id, &json!({"skill_id": skill.id, "version": meta.version})).await {
                    tracing::warn!(task_id = %t, error = %e, "skill_drafted_from not logged");
                }
            }
            changed(state, &skill);
            ok(json!({
                "skill_id": skill.id,
                "version": meta.version,
                "next": "Stored as a draft for the person to review. A dry-run replay on its examples runs after your turn; you don't need to do anything else.",
            }))
        }
        Err(SkillError::TooManyPending) => fail("You have too many skills waiting for review; the person needs to review some first."),
        Err(e) => fail(format!("Could not store the draft: {e}")),
    }
}

// Replay

/// Opens a replay run of draft `skill_id` (`queued`) and drives it in the background.
pub async fn begin_replay(state: &AppState, skill_id: &str, actor: &str) -> Result<FoundryRun, String> {
    if !enabled(state).await {
        return Err("the Skill Foundry is off in Settings".into());
    }
    let (skill, meta) = match (state.db.get_skill(skill_id).await, state.db.skill_meta(skill_id).await) {
        (Ok(Some(s)), Ok(Some(m))) => (s, m),
        _ => return Err("only a Skill Foundry draft can be replayed".into()),
    };
    if skill.status != "pending" {
        return Err(format!("only a draft waiting for review is replayed (this one is {})", skill.status));
    }
    if replays(state).await == 0 {
        return Err("replays are off (skill_foundry_replays = 0)".into());
    }
    if let Ok(Some(_)) = state.db.live_foundry_run("replay", &meta.kwant_id, &meta.task_class, Some(skill_id)).await {
        return Err("a replay of this draft is already running".into());
    }
    let run = state.db.create_foundry_run("replay", &meta.space_id, &meta.kwant_id, &meta.task_class, Some(skill_id)).await.map_err(|e| e.to_string())?;
    tracing::info!(run_id = %run.id, skill_id, actor, "Foundry replay queued");
    let (s, r) = (state.clone(), run.clone());
    tokio::spawn(async move { drive_replay(&s, &r).await });
    Ok(run)
}

/// The scratch folder of a replay's example `n`, under the data dir.
pub fn scratch_dir(state: &AppState, run_id: &str, n: usize) -> PathBuf {
    state.config.data_dir.join("foundry").join(run_id).join(n.to_string())
}

fn private_dir(dir: &FsPath) -> std::io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    if let Some(parent) = dir.parent() {
        std::fs::create_dir_all(parent)?;
        std::fs::set_permissions(parent, std::fs::Permissions::from_mode(0o700))?;
    }
    std::fs::create_dir_all(dir)?;
    std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700))
}

/// Runs a draft's checks on a replay's scratch folder: file checks run there; command and http
/// checks would reach outside it, so they are `not_replayable`, never passed.
pub async fn scratch_checks(checks: &[Value], scratch: &FsPath) -> Vec<Value> {
    let mut out = Vec::new();
    for c in checks {
        let kind = c["kind"].as_str().unwrap_or("");
        let spec = super::autofix::redact_value(&c["spec"]);
        if kind.starts_with("file_") {
            let (status, output) = super::checks::run_spec(kind, &c["spec"], Some(scratch), false).await;
            out.push(json!({"kind": kind, "spec": spec, "status": status, "output": output}));
        } else {
            out.push(json!({"kind": kind, "spec": spec, "status": "not_replayable", "output": "reaches outside the scratch folder; not run in a replay"}));
        }
    }
    out
}

/// One example's verdict: `passed` (every replayable check passed, and there was one),
/// `failed`, or `not_checked`.
pub fn example_verdict(checks: &[Value]) -> &'static str {
    let ran: Vec<&str> = checks.iter().filter_map(|c| c["status"].as_str()).filter(|s| *s != "not_replayable").collect();
    if ran.is_empty() {
        "not_checked"
    } else if ran.iter().all(|s| *s == "passed") {
        "passed"
    } else {
        "failed"
    }
}

/// Replays a draft on up to `skill_foundry_replays` of its examples, one dry-run turn each, then
/// stores the report on the draft. A failed replay never touches the draft itself.
pub async fn drive_replay(state: &AppState, run: &FoundryRun) {
    let _unstick = Unstick(state.clone(), run.id.clone());
    let _one = ONE.lock().await;
    if !state.db.move_foundry_run(&run.id, "queued", "replaying", None).await.unwrap_or(false) {
        return;
    }
    let skill_id = run.skill_id.clone().unwrap_or_default();
    let (Ok(Some(skill)), Ok(Some(meta)), Ok(Some(owner))) = (state.db.get_skill(&skill_id).await, state.db.skill_meta(&skill_id).await, state.db.get_kwant(&run.kwant_id).await) else {
        let _ = state.db.move_foundry_run(&run.id, "replaying", "failed", Some("the draft or its bot no longer exists")).await;
        return;
    };
    let n = replays(state).await.max(0) as usize;
    let cap = replay_tokens(state).await.max(0) as u64;
    let mut examples = Vec::new();
    let mut tokens_total = 0;
    for (i, task_id) in meta.examples().into_iter().take(n).enumerate() {
        let r = replay_one(state, run, &skill, &meta, &owner, &task_id, i, cap).await;
        tokens_total += r["tokens"].as_i64().unwrap_or(0);
        examples.push(r);
    }
    let count = |v: &str| examples.iter().filter(|e| e["verdict"] == v).count();
    let report = json!({
        "run_id": run.id,
        "at": Utc::now(),
        "examples": examples,
        "passed": count("passed"),
        "failed": count("failed"),
        "not_checked": count("not_checked"),
        "errors": count("error"),
        "tokens": tokens_total,
    });
    let _ = state.db.add_foundry_tokens(&run.id, tokens_total).await;
    let _ = state.db.set_skill_replay(&skill_id, &report).await;
    skill_event(state, &skill_id, "skill_replayed", "kwant", json!({"run_id": run.id, "passed": report["passed"], "failed": report["failed"], "not_checked": report["not_checked"], "errors": report["errors"], "tokens": tokens_total})).await;
    let all_errors = !report["examples"].as_array().is_some_and(|a| a.iter().any(|e| e["verdict"] != "error"));
    let (to, why) = if all_errors { ("failed", Some("no example could be replayed")) } else { ("done", None) };
    let _ = state.db.move_foundry_run(&run.id, "replaying", to, why).await;
    if let Ok(Some(s)) = state.db.get_skill(&skill_id).await {
        changed(state, &s);
    }
}

#[allow(clippy::too_many_arguments)]
async fn replay_one(state: &AppState, run: &FoundryRun, skill: &Skill, meta: &SkillMeta, owner: &Kwant, task_id: &str, i: usize, cap: u64) -> Value {
    let began = std::time::Instant::now();
    let Ok(Some(task)) = state.db.get_task(task_id).await else {
        return json!({"task_id": task_id, "verdict": "error", "error": "the task no longer exists", "tokens": 0});
    };
    let scratch = scratch_dir(state, &run.id, i);
    let mut out = json!({"task_id": task.id, "title": task.title, "original_actions": original_actions(state, task_id).await});
    let result = async {
        let dir = scratch.clone();
        tokio::task::spawn_blocking(move || private_dir(&dir)).await.map_err(|e| e.to_string())?.map_err(|e| format!("the scratch folder can't be made: {e}"))?;
        let restored = super::checkpoints::materialize_first(state, task_id, &scratch).await?;
        let log: RehearseLog = Default::default();
        let prompt = replay_prompt(skill, &task, restored);
        let job = state.turns.start_job([owner.id.clone()]);
        // The cap: cancel this job's turn once it reports more fresh tokens than allowed
        let over = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let watcher = {
            let (turns, token, kwant, over) = (state.turns.clone(), job.token.clone(), owner.id.clone(), over.clone());
            tokio::spawn(async move {
                loop {
                    tokio::time::sleep(CAP_POLL).await;
                    if turns.usage_of(&kwant, REPLAY_PURPOSE) > cap {
                        over.store(true, std::sync::atomic::Ordering::SeqCst);
                        token.cancel();
                        return;
                    }
                }
            })
        };
        let started = Utc::now();
        let reply = match scope_for(state, owner, None, None, &job, 0).await {
            Ok(mut scope) => {
                scope.workspace_dir = Some(scratch.clone());
                let mut opts = TurnOpts::always_asking("Skill Foundry replay");
                (opts.unsaved, opts.quiet_failure, opts.purpose) = (true, true, Some(REPLAY_PURPOSE.into()));
                opts.tier = tier_of(model_mode(state).await);
                opts.rehearse = Some(log.clone());
                match tokio::time::timeout(REPLAY_TIMEOUT, turn_once(&scope, owner, prompt, opts)).await {
                    Ok(r) => r,
                    Err(_) => Err("the replay took longer than 15 minutes".into()),
                }
            }
            Err(e) => Err(e),
        };
        watcher.abort();
        drop(job);
        let tokens = state.db.tokens_between(&owner.id, started - Duration::seconds(1), Utc::now() + Duration::seconds(1)).await.unwrap_or(0);
        let would_have: Vec<Value> = log.lock().map(|l| l.clone()).unwrap_or_default();
        let checks = scratch_checks(&meta.checks(), &scratch).await;
        let capped = over.load(std::sync::atomic::Ordering::SeqCst);
        Ok::<_, String>((restored, reply, tokens, would_have, checks, capped))
    }
    .await;
    // The scratch copy never outlives its replay
    let dir = scratch.clone();
    let _ = tokio::task::spawn_blocking(move || std::fs::remove_dir_all(dir)).await;
    if let Some(parent) = scratch.parent() {
        let _ = std::fs::remove_dir(parent);
    }
    match result {
        Ok((restored, reply, tokens, would_have, checks, capped)) => {
            let error = if capped {
                Some(format!("stopped at the replay token cap ({cap})"))
            } else {
                reply.err().map(|e| clip(e.lines().next().unwrap_or(&e), 300))
            };
            // A turn that never ran (no Claude, an engine error) is an error, not a failed check
            let verdict = match &error {
                Some(_) if tokens == 0 && would_have.is_empty() => "error",
                _ => example_verdict(&checks),
            };
            out["files_restored"] = json!(restored);
            out["would_have"] = json!(would_have);
            out["checks"] = json!(checks);
            out["verdict"] = json!(verdict);
            out["error"] = json!(error);
            out["tokens"] = json!(tokens);
        }
        Err(e) => {
            out["verdict"] = json!("error");
            out["error"] = json!(e);
            out["tokens"] = json!(0);
        }
    }
    out["seconds"] = json!(began.elapsed().as_secs());
    out
}

fn replay_prompt(skill: &Skill, task: &crate::db::missions::Task, restored: usize) -> String {
    format!(
        "[Skill Foundry replay] This is a dry run of an earlier task, to test a drafted skill. Your folder is a \
         scratch copy{files}; nothing you do here is kept or sent. Calls that could change anything outside \
         this folder (send, publish, sign in, pay, run commands, other servers' tools) don't run: kwant records \
         them as what you would have done.\nTask \"{title}\"\nAcceptance: {acc}\n\nFollow this skill:\n\
         --- skill {name} ---\n{body}\n--- end of skill ---\n\nDo the task in this folder as far as you can, then \
         end your turn with a short summary of what you did and what you would have done outside it.",
        files = if restored > 0 { format!(" of its folder before it started ({restored} files)") } else { " (empty: the task had no snapshot)".into() },
        title = clip(&task.title, 300),
        acc = if task.acceptance.trim().is_empty() { "(none given)".into() } else { clip(&task.acceptance, 2000) },
        name = skill.name,
        body = skill.body,
    )
}

// Routes

fn skill_error(e: SkillError) -> Response {
    match e {
        SkillError::NotFound => not_found("skill"),
        SkillError::Conflict(why) => conflict(why),
        SkillError::Db(detail) => internal(detail),
        other => bad(other.to_string()),
    }
}

/// The candidate clusters of a space, each with why no draft is offered now (`block`).
async fn clusters_json(state: &AppState, space: &str) -> Vec<Value> {
    let now = Utc::now();
    let mut out = Vec::new();
    for c in state.db.foundry_clusters(Some(space), now).await.unwrap_or_default() {
        let block = state.db.foundry_block(&c.kwant_id, &c.task_class, now).await.ok().flatten();
        out.push(json!({"id": c.id(), "kwant_id": c.kwant_id, "task_class": c.task_class, "verified": c.verified, "total": c.total, "block": block}));
    }
    out
}

fn skill_json(skill: &Skill, meta: &SkillMeta) -> Value {
    let mut j = serde_json::to_value(skill).unwrap_or_default();
    j["foundry"] = meta.json();
    j
}

/// `GET /api/skills/foundry?space=` (person only): settings, candidate clusters, drafts and
/// versions with their replay reports, and recent runs.
pub async fn overview(State(state): State<AppState>, Scope(space): Scope) -> Response {
    let skills = match state.db.foundry_skills(space.as_str(), None).await {
        Ok(s) => s,
        Err(e) => return internal(e),
    };
    let runs = state.db.foundry_runs_of_space(space.as_str(), 20).await.unwrap_or_default();
    Json(json!({
        "enabled": enabled(&state).await,
        "settings": {
            "model": model_mode(&state).await,
            "daily_drafts": daily_drafts(&state).await,
            "replays": replays(&state).await,
            "replay_tokens": replay_tokens(&state).await,
        },
        "drafts_today": state.db.draft_runs_since(midnight()).await.unwrap_or(0),
        "clusters": clusters_json(&state, space.as_str()).await,
        "skills": skills.iter().map(|(s, m)| skill_json(s, m)).collect::<Vec<_>>(),
        "runs": runs,
    }))
    .into_response()
}

/// `POST /api/skills/foundry/:cluster/draft` (person only): draft a skill for the cluster now,
/// without waiting for the hourly tick. A bot's live skill for the class gets a new version.
pub async fn draft_now(State(state): State<AppState>, Scope(space): Scope, Path(cluster): Path<String>) -> Response {
    if !enabled(&state).await {
        return conflict("The Skill Foundry is off in Settings.");
    }
    let Some((kwant, class)) = split_cluster(&cluster) else { return bad("cluster must be <bot id>~<class>") };
    let found = state.db.foundry_clusters(Some(space.as_str()), Utc::now()).await.unwrap_or_default().into_iter().find(|c| c.kwant_id == kwant && c.task_class == class);
    let Some(c) = found else {
        return not_found("cluster (it needs 3 or more verified tasks of one class, 70% of them verified, in 60 days)");
    };
    match begin_draft(&state, &c, "person").await {
        Ok(run) => (StatusCode::ACCEPTED, Json(json!({"run": run, "cluster": cluster_id(&c.kwant_id, &c.task_class)}))).into_response(),
        Err(why) => conflict(why),
    }
}

async fn foundry_skill(state: &AppState, space: &str, id: &str) -> Result<(Skill, SkillMeta), Box<Response>> {
    match (state.db.get_skill(id).await, state.db.skill_meta(id).await) {
        (Ok(Some(s)), Ok(Some(m))) if m.space_id == space => Ok((s, m)),
        (Err(e), _) | (_, Err(e)) => Err(Box::new(internal(e))),
        _ => Err(Box::new(not_found("Skill Foundry skill"))),
    }
}

/// `POST /api/skills/:id/replay` (person only): replay a draft again (after an edit, say).
pub async fn replay(State(state): State<AppState>, Scope(space): Scope, Path(id): Path<String>) -> Response {
    if let Err(r) = foundry_skill(&state, space.as_str(), &id).await {
        return *r;
    }
    match begin_replay(&state, &id, "person").await {
        Ok(run) => (StatusCode::ACCEPTED, Json(json!({"run": run}))).into_response(),
        Err(why) => conflict(why),
    }
}

/// `POST /api/skills/:id/rollback` (person only): make `id`, an earlier version of its lineage,
/// the live one again. Works with the Foundry off: a rollback only ever goes back.
pub async fn rollback(State(state): State<AppState>, Scope(space): Scope, Path(id): Path<String>) -> Response {
    if let Err(r) = foundry_skill(&state, space.as_str(), &id).await {
        return *r;
    }
    let before = state.db.skill_meta(&id).await.ok().flatten().map(|m| m.lineage_id);
    match state.db.rollback_skill(&id, "person").await {
        Ok(skill) => {
            if let Some(lineage) = before {
                for (s, _) in state.db.lineage(&lineage).await.unwrap_or_default() {
                    changed(&state, &s);
                }
            }
            Json(skill).into_response()
        }
        Err(e) => skill_error(e),
    }
}

/// `GET /api/skills/:id/foundry`: a Foundry skill with its versions, history and runs.
pub async fn detail(State(state): State<AppState>, Scope(space): Scope, Path(id): Path<String>) -> Response {
    let (skill, meta) = match foundry_skill(&state, space.as_str(), &id).await {
        Ok(p) => p,
        Err(r) => return *r,
    };
    let versions = state.db.lineage(&meta.lineage_id).await.unwrap_or_default();
    let ids: Vec<String> = versions.iter().map(|(s, _)| s.id.clone()).collect();
    let events = state.db.skill_events(&ids).await.unwrap_or_default();
    let runs: Vec<FoundryRun> = state.db.foundry_runs_of_space(space.as_str(), 200).await.unwrap_or_default().into_iter().filter(|r| r.skill_id.as_ref().is_some_and(|s| ids.contains(s))).collect();
    Json(json!({
        "skill": skill_json(&skill, &meta),
        "versions": versions.iter().map(|(s, m)| json!({"id": s.id, "version": m.version, "status": s.status, "name": s.name, "created_at": s.created_at, "reviewed_at": s.reviewed_at})).collect::<Vec<_>>(),
        "events": events.iter().map(|e| json!({"skill_id": e.skill_id, "kind": e.kind, "actor": e.actor, "detail": serde_json::from_str::<Value>(&e.detail).unwrap_or_default(), "at": e.created_at})).collect::<Vec<_>>(),
        "runs": runs,
    }))
    .into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn settings_are_checked() {
        assert!(settings_error(Some("on"), Some("balanced"), Some(2), Some(3), Some(60_000)).is_none());
        assert!(settings_error(Some("auto"), None, None, None, None).is_some());
        assert!(settings_error(None, Some("opus"), None, None, None).is_some());
        assert!(settings_error(None, None, Some(0), None, None).is_some());
        assert!(settings_error(None, None, None, Some(6), None).is_some());
        assert!(settings_error(None, None, None, Some(0), None).is_none());
        assert!(settings_error(None, None, None, None, Some(1_000)).is_some());
    }

    #[test]
    fn verdicts_never_count_unreplayable_checks_as_passed() {
        let c = |s: &str| json!({"status": s});
        assert_eq!(example_verdict(&[c("not_replayable")]), "not_checked");
        assert_eq!(example_verdict(&[]), "not_checked");
        assert_eq!(example_verdict(&[c("passed"), c("not_replayable")]), "passed");
        assert_eq!(example_verdict(&[c("passed"), c("failed")]), "failed");
        assert_eq!(example_verdict(&[c("error")]), "failed");
    }

    #[test]
    fn drafts_list_only_tools_the_bot_holds() {
        let held = vec!["Read".to_string(), "task_list".to_string()];
        assert!(validate_parts(&Value::Null, &Value::Null, &json!(["Read", "task_list", "Read"]), &held).is_ok_and(|(_, _, t)| t == ["Read", "task_list"]));
        assert!(validate_parts(&Value::Null, &Value::Null, &json!(["NotATool"]), &held).unwrap_err().contains("don't have a tool called NotATool"));
        assert!(validate_parts(&Value::Null, &Value::Null, &json!([7]), &held).is_err());
    }

    #[test]
    fn draft_checks_pass_the_task_check_validator() {
        let none: Vec<String> = Vec::new();
        assert!(validate_parts(&Value::Null, &json!([{"kind": "file_exists", "spec": {"path": "NOTES.md"}}]), &Value::Null, &none).is_ok());
        assert!(validate_parts(&Value::Null, &json!([{"kind": "person", "spec": {}}]), &Value::Null, &none).is_err());
        assert!(validate_parts(&Value::Null, &json!([{"kind": "http", "spec": {"url": "ftp://x"}}]), &Value::Null, &none).is_err());
        let six = Value::Array(vec![json!({"kind": "file_exists", "spec": {"path": "a"}}); 6]);
        assert!(validate_parts(&Value::Null, &six, &Value::Null, &none).is_err());
        let nine = Value::Array(vec![json!({"name": "n", "description": "d"}); 9]);
        assert!(validate_parts(&nine, &Value::Null, &Value::Null, &none).is_err());
        assert!(validate_parts(&json!([{"name": "n"}]), &Value::Null, &Value::Null, &none).is_err());
    }
}
