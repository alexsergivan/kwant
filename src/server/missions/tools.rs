//! The task-board self tools: `mission_plan`, `task_list`, `task_claim`, `task_update`,
//! `task_attach_evidence`, `task_request_check`, `task_review`. Offered only while `mission_board`
//! is on. None of them can set `verified`: only a passing check that isn't the owner's does.

use super::checks::{hex, inside, run_task_checks};
use super::{evaluate, task_folder, transition, validate_check, Actor, PlanBy, PlanPlace, PlanTask};
use crate::db::missions::{now_ms, NewCheck, NewEvidence, Task, TaskChange};
use crate::db::spaces::SpaceId;
use crate::server::continuity::SelfScope;
use crate::server::AppState;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

type ToolResult = (String, bool);

fn fail(msg: impl Into<String>) -> ToolResult {
    (msg.into(), true)
}

fn ok(v: Value) -> ToolResult {
    (serde_json::to_string_pretty(&v).unwrap_or_default(), false)
}

const NAMES: [&str; 8] = ["mission_plan", "task_list", "task_claim", "task_update", "task_attach_evidence", "task_request_check", "task_review", "workflow_propose"];

pub fn handles(name: &str) -> bool {
    NAMES.contains(&name)
}

fn check_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "kind": {"type": "string", "enum": ["command", "http", "file_exists", "file_contains", "file_sha256"]},
            "spec": {"type": "object", "description": "command: {command, timeout_secs?}; http: {url, expect_status?, contains?}; file_exists: {path}; file_contains: {path, text}; file_sha256: {path, sha256}. Paths are relative to the mission's folder."},
            "required": {"type": "boolean"},
        },
        "required": ["kind", "spec"],
    })
}

pub fn tools() -> Vec<Value> {
    vec![
        json!({
            "name": "mission_plan",
            "description": "Propose a mission: a goal split into tasks with acceptance criteria and outcome checks. It is a draft until the person approves it on the board; nothing runs before that. Each task is verified only by its checks passing (or a teammate's review / the person), never by its owner saying it is done. Use it for multi-step work worth tracking, not for a quick answer.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "title": {"type": "string"},
                    "goal": {"type": "string", "description": "The outcome, in a sentence or two."},
                    "budget_tokens": {"type": "integer", "description": "Token budget for the whole mission (optional; leave out or 0 for no limit)."},
                    "tasks": {
                        "type": "array",
                        "maxItems": super::MAX_PLAN_TASKS,
                        "items": {
                            "type": "object",
                            "properties": {
                                "key": {"type": "string", "description": "Short id other tasks use in depends_on."},
                                "title": {"type": "string"},
                                "acceptance": {"type": "string", "description": "What must be true when it is done."},
                                "owner": {"type": "string", "description": "Bot name or id in your space (optional)."},
                                "priority": {"type": "integer", "minimum": 0, "maximum": 3},
                                "depends_on": {"type": "array", "items": {"type": "string"}},
                                "budget_tokens": {"type": "integer"},
                                "checks": {"type": "array", "items": check_schema()},
                                "task_class": {"type": "string", "description": "Kind of work, as a short slug (seo-audit, release…); sets how much freedom its bot gets. Leave it out for tasks you'd run yourself."},
                            },
                            "required": ["title", "acceptance"],
                        },
                    },
                },
                "required": ["title", "goal", "tasks"],
            },
        }),
        json!({
            "name": "task_list",
            "description": "List tasks on your space's board.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "mission_id": {"type": "string"},
                    "state": {"type": "string", "enum": crate::db::missions::TASK_STATES},
                    "mine": {"type": "boolean", "description": "Only tasks you own."},
                },
            },
        }),
        json!({
            "name": "task_claim",
            "description": "Take a ready task (ready → working, with a lease). Refused if someone holds it, a dependency isn't verified, its mission isn't active or can't afford it, or you already hold another task.",
            "inputSchema": {"type": "object", "properties": {"task_id": {"type": "string"}}, "required": ["task_id"]},
        }),
        json!({
            "name": "task_update",
            "description": "Update a task you hold. status \"review\" = done: kwant runs its checks now and verifies it only if they pass. \"ready\" = let go of it. \"blocked\" needs a reason. A note alone renews your lease. You can never mark a task verified.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "task_id": {"type": "string"},
                    "status": {"type": "string", "enum": ["review", "ready", "blocked"]},
                    "note": {"type": "string"},
                    "reason": {"type": "string"},
                },
                "required": ["task_id"],
            },
        }),
        json!({
            "name": "task_attach_evidence",
            "description": "Attach evidence to a task: a file in the mission's folder (kwant hashes it itself), a url, a note or a screenshot path. Evidence alone never verifies a task.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "task_id": {"type": "string"},
                    "kind": {"type": "string", "enum": ["file", "url", "note", "screenshot"]},
                    "label": {"type": "string"},
                    "path": {"type": "string"},
                    "url": {"type": "string"},
                    "text": {"type": "string"},
                },
                "required": ["task_id", "kind", "label"],
            },
        }),
        json!({
            "name": "task_request_check",
            "description": "Run a task's approved checks (or one by check_id), or add a new check (kind + spec) and run it if it may run. A command check you add, or an http check of a private address, waits for the person's approval.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "task_id": {"type": "string"},
                    "check_id": {"type": "string"},
                    "kind": {"type": "string", "enum": ["command", "http", "file_exists", "file_contains", "file_sha256"]},
                    "spec": {"type": "object"},
                    "required": {"type": "boolean"},
                },
                "required": ["task_id"],
            },
        }),
        json!({
            "name": "task_review",
            "description": "Review a teammate's task in review. Not for your own tasks. A pass must cite at least one evidence id attached to that task.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "task_id": {"type": "string"},
                    "verdict": {"type": "string", "enum": ["pass", "fail"]},
                    "evidence_ids": {"type": "array", "items": {"type": "string"}},
                    "note": {"type": "string"},
                },
                "required": ["task_id", "verdict", "note"],
            },
        }),
        json!({
            "name": "workflow_propose",
            "description": "Propose a lasting fix after a failure: a routine's instructions or schedule, a check's spec, a task's acceptance or max attempts. It is only a proposal: the person approves or rejects it, and nothing changes until they approve.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "target": {"type": "string", "enum": crate::db::autofix::TARGETS},
                    "routine_id": {"type": "string", "description": "For routine_instructions and routine_schedule."},
                    "task_id": {"type": "string", "description": "For check_spec, task_acceptance and max_attempts."},
                    "after": {"description": "The new value: instructions or acceptance (string), a schedule (object), {check_id, spec?, required?} for a check, or a number 1–10 for max_attempts."},
                    "reason": {"type": "string", "description": "Why, in a sentence: what failed and how this fixes it."},
                },
                "required": ["target", "after", "reason"],
            },
        }),
    ]
}

pub async fn call(state: &AppState, scope: &SelfScope, name: &str, args: &Value) -> ToolResult {
    if !super::enabled(state).await {
        return fail("The task board is off in Settings.");
    }
    let space = match state.db.space_of_kwant(&scope.kwant_id).await {
        Ok(Some(s)) => s,
        _ => return fail("Could not find your space."),
    };
    if name == "mission_plan" {
        return plan(state, scope, &space, args).await;
    }
    if name == "task_list" {
        return list(state, scope, &space, args).await;
    }
    if name == "workflow_propose" {
        if let Some(t) = args["task_id"].as_str() {
            if !matches!(state.db.get_task(t).await, Ok(Some(t)) if t.space_id == space.as_str()) {
                return fail("No such task in your space.");
            }
        }
        if let Some(r) = args["routine_id"].as_str() {
            if !matches!(state.db.get_routine(r).await, Ok(Some(r)) if r.space_id == space.as_str()) {
                return fail("No such routine in your space.");
            }
        }
        return match super::autofix::propose(state, &scope.kwant_id, args).await {
            Ok(v) => ok(v),
            Err(e) => fail(e),
        };
    }
    let Some(task_id) = args["task_id"].as_str().map(str::trim).filter(|s| !s.is_empty()) else {
        return fail("task_id is required.");
    };
    let task = match state.db.get_task(task_id).await {
        Ok(Some(t)) if t.space_id == space.as_str() => t,
        _ => return fail("No such task in your space."),
    };
    match name {
        "task_claim" => claim(state, scope, &task).await,
        "task_update" => update(state, scope, &task, args).await,
        "task_attach_evidence" => evidence(state, scope, &task, args).await,
        "task_request_check" => request_check(state, scope, &task, args).await,
        "task_review" => review(state, scope, &task, args).await,
        _ => fail(format!("Unknown tool {name}.")),
    }
}

async fn plan(state: &AppState, scope: &SelfScope, space: &SpaceId, args: &Value) -> ToolResult {
    let tasks: Vec<PlanTask> = match serde_json::from_value(args["tasks"].clone()) {
        Ok(t) => t,
        Err(e) => return fail(format!("tasks: {e}")),
    };
    if tasks.is_empty() {
        return fail("A mission needs at least one task.");
    }
    let Ok(Some(lead)) = state.db.get_kwant(&scope.kwant_id).await else { return fail("Could not find you.") };
    let origin = scope.origin.clone().unwrap_or_default();
    let workspace = scope.workspace.clone().unwrap_or_else(|| {
        crate::tools::bot_workspace(&crate::db::spaces::space_data_dir(&state.config.data_dir, space), &scope.kwant_id)
    });
    let place = PlanPlace {
        channel_id: origin.channel_id,
        direct_kwant_id: origin.direct_kwant_id,
        thread_id: origin.thread_id,
        workspace_path: workspace.to_string_lossy().into_owned(),
    };
    let budget = args["budget_tokens"].as_i64();
    let by = PlanBy { created_by: &scope.kwant_id, active: false };
    let title = args["title"].as_str().unwrap_or("");
    let goal = args["goal"].as_str().unwrap_or("");
    match super::create_plan(state, space, &lead, title, goal, budget, &tasks, by, place).await {
        Ok((m, rows)) => {
            super::mission_chat_line(
                state,
                &m,
                format!("Proposed mission \"{}\" with {} task{}. Approve it on the board to start.", m.title, rows.len(), if rows.len() == 1 { "" } else { "s" }),
            )
            .await;
            ok(json!({
                "mission_id": m.id,
                "status": m.status,
                "tasks": rows.iter().map(|t| json!({"id": t.id, "title": t.title, "state": t.state})).collect::<Vec<_>>(),
                "next": "The person approves the mission on the board; nothing runs until then.",
            }))
        }
        Err(e) => fail(e),
    }
}

fn brief(t: &Task) -> Value {
    json!({
        "id": t.id, "title": t.title, "state": t.state, "mission_id": t.mission_id, "priority": t.priority,
        "owner_kwant_id": t.owner_kwant_id, "suggested_kwant_id": t.suggested_kwant_id, "acceptance": t.acceptance,
        "budget_tokens": t.budget_tokens, "spent_tokens": t.spent_tokens, "attempts": t.attempts, "blocked_reason": t.blocked_reason,
    })
}

async fn list(state: &AppState, scope: &SelfScope, space: &SpaceId, args: &Value) -> ToolResult {
    let st = args["state"].as_str();
    let tasks = match state.db.list_tasks(space, st, args["mission_id"].as_str(), None, 200).await {
        Ok(t) => t,
        Err(e) => return fail(format!("Could not list tasks: {e}")),
    };
    let mine = args["mine"].as_bool().unwrap_or(false);
    let rows: Vec<Value> = tasks
        .iter()
        .filter(|t| !mine || t.owner_kwant_id.as_deref() == Some(scope.kwant_id.as_str()))
        .filter(|t| st.is_some() || t.state != "cancelled")
        .map(brief)
        .collect();
    ok(json!({"tasks": rows}))
}

async fn claim(state: &AppState, scope: &SelfScope, task: &Task) -> ToolResult {
    let lease = super::lease_ms(state).await;
    match state.db.claim_task(&task.id, &scope.kwant_id, scope.turn_id.as_deref(), None, lease, false, &scope.kwant_id).await {
        Ok(Ok(t)) => {
            super::emit_task(state, &t).await;
            super::checkpoints::before_run(state, &t).await;
            if let Some(m) = &t.mission_id {
                super::emit_mission(state, m).await;
            }
            let checks = state.db.list_checks(&t.id).await.unwrap_or_default();
            let mut out = json!({
                "task": brief(&t),
                "lease_minutes": lease / 60_000,
                "checks": checks.iter().map(|c| json!({"id": c.id, "kind": c.kind, "spec": super::autofix::redact_value(&c.spec_value()), "approved": c.approved})).collect::<Vec<_>>(),
                "next": "Do the work, attach evidence with task_attach_evidence, then task_update status=review.",
            });
            if let Some(note) = super::autofix::retry_note(state, &t).await {
                out["autofix"] = json!(note.trim());
            }
            ok(out)
        }
        Ok(Err(why)) => fail(format!("Not claimed: {why}.")),
        Err(e) => fail(format!("Could not claim: {e}")),
    }
}

/// The caller's live claim on `task`, if it holds one.
async fn holds(state: &AppState, scope: &SelfScope, task: &Task) -> bool {
    matches!(state.db.open_claim(&task.id).await, Ok(Some(c))
        if c.kwant_id == scope.kwant_id && !c.system && c.expires_at > now_ms())
        && task.owner_kwant_id.as_deref() == Some(scope.kwant_id.as_str())
        && matches!(task.state.as_str(), "working" | "needs_you")
}

async fn update(state: &AppState, scope: &SelfScope, task: &Task, args: &Value) -> ToolResult {
    // A card answered a moment ago may not have moved the task back from needs_you yet
    let settled = super::listener::settle_needs_you(state, &task.id).await;
    let task = settled.as_ref().unwrap_or(task);
    if !holds(state, scope, task).await {
        return fail("You don't hold this task (claim it first, or your lease ran out).");
    }
    let note = args["note"].as_str().map(str::trim).filter(|s| !s.is_empty());
    let reason = args["reason"].as_str().map(str::trim).filter(|s| !s.is_empty());
    if let Some(n) = note {
        let _ = state.db.log_task_event(&task.id, "note", &scope.kwant_id, &json!({"note": n.chars().take(2000).collect::<String>()})).await;
    }
    let status = args["status"].as_str();
    let change = |reason: Option<&str>| TaskChange { reason: reason.map(String::from), ..Default::default() };
    let moved = match status {
        None if note.is_some() => {
            let _ = state.db.renew_claim(&task.id, super::lease_ms(state).await).await;
            super::emit_task_id(state, &task.id).await;
            return ok(json!({"task": brief(task), "lease": "renewed"}));
        }
        None => return fail("Give a status or a note."),
        Some("verified") => return fail("You can't mark a task verified; its checks or a teammate's review do."),
        Some("review") => {
            if task.state != "working" {
                return fail("Answer the open question or step first; the task is waiting on the person.");
            }
            transition(state, &task.id, "working", "review", Actor::Owner, &scope.kwant_id, change(note)).await
        }
        Some("ready") => transition(state, &task.id, &task.state, "ready", Actor::Owner, &scope.kwant_id, change(reason.or(note))).await,
        Some("blocked") => match reason {
            Some(r) => transition(state, &task.id, &task.state, "blocked", Actor::Owner, &scope.kwant_id, change(Some(r))).await,
            None => return fail("blocked needs a reason."),
        },
        Some(other) => return fail(format!("status must be review, ready or blocked, not {other}.")),
    };
    let task = match moved {
        Ok(t) => t,
        Err(e) => return fail(format!("Not moved: {e}.")),
    };
    // Handed in: the turn so far pays now, before its checks can verify it
    super::spend_at_hand_in(state, &task.id, scope.turn_id.as_deref()).await;
    if task.state != "review" {
        return ok(json!({"task": brief(&task)}));
    }
    let (checks, after) = run_task_checks(state, &task.id, None).await;
    let after = after.unwrap_or(task);
    let summary = super::checks_summary(&after, &checks);
    let next = match after.state.as_str() {
        "verified" => "Verified by its checks.",
        "review" if summary["status"] == "not_checked" => "No checks: it stays in review (not checked) until a teammate reviews it or the person verifies it.",
        "review" => "Waiting on checks that need the person's approval.",
        "ready" => "A check failed; the task is back in ready. Fix it and claim it again.",
        "failed" if after.attempts < after.max_attempts => "A check failed; AutoFix decides whether to retry it (see the task's autofix). Don't claim it again yourself.",
        "failed" => "A check failed on the last attempt; the task failed.",
        _ => "",
    };
    ok(json!({
        "task": brief(&after),
        "checks": checks.iter().map(|c| json!({"id": c.id, "kind": c.kind, "status": c.status, "output": c.output_excerpt})).collect::<Vec<_>>(),
        "next": next,
    }))
}

async fn evidence(state: &AppState, scope: &SelfScope, task: &Task, args: &Value) -> ToolResult {
    if matches!(task.state.as_str(), "verified" | "cancelled") {
        return fail("This task is closed.");
    }
    let kind = args["kind"].as_str().unwrap_or("");
    let label = args["label"].as_str().map(str::trim).filter(|s| !s.is_empty());
    let Some(label) = label else { return fail("label is required.") };
    let s = |k: &str| args[k].as_str().map(str::trim).filter(|v| !v.is_empty()).map(String::from);
    let mut e = NewEvidence {
        task_id: task.id.clone(),
        kind: kind.into(),
        label: label.chars().take(200).collect(),
        captured_by: scope.kwant_id.clone(),
        ..Default::default()
    };
    match kind {
        "file" | "screenshot" => {
            let Some(rel) = s("path") else { return fail("path is required for a file.") };
            let Some(folder) = task_folder(state, task).await else { return fail("This task has no folder.") };
            let path = match inside(&folder, &rel) {
                Ok(p) => p,
                Err(why) => return fail(format!("Not attached: {why}.")),
            };
            let bytes = match tokio::fs::read(&path).await {
                Ok(b) => b,
                Err(_) => return fail(format!("{rel} doesn't exist or can't be read.")),
            };
            e.sha256 = Some(hex(&Sha256::digest(&bytes)));
            e.bytes = i64::try_from(bytes.len()).ok();
            e.path = Some(rel);
        }
        "url" => {
            let Some(url) = s("url").filter(|u| u.starts_with("http://") || u.starts_with("https://")) else {
                return fail("url must start with http:// or https://.");
            };
            e.uri = Some(url);
        }
        "note" => {
            let Some(text) = s("text") else { return fail("text is required for a note.") };
            e.excerpt = Some(text.chars().take(super::MAX_EXCERPT).collect());
        }
        _ => return fail("kind must be file, url, note or screenshot."),
    }
    match state.db.add_evidence(&e).await {
        Ok(ev) => {
            super::emit_task_id(state, &task.id).await;
            ok(json!({"evidence_id": ev.id, "sha256": ev.sha256, "bytes": ev.bytes}))
        }
        Err(err) => fail(format!("Could not attach: {err}")),
    }
}

async fn request_check(state: &AppState, scope: &SelfScope, task: &Task, args: &Value) -> ToolResult {
    if matches!(task.state.as_str(), "verified" | "cancelled") {
        return fail("This task is closed.");
    }
    let mut only = args["check_id"].as_str().map(String::from);
    let mut added = None;
    if let Some(kind) = args["kind"].as_str() {
        let spec = args["spec"].clone();
        let approved = match validate_check(kind, &spec, false) {
            Ok(a) => a,
            Err(e) => return fail(e),
        };
        // The owner's own check would let it verify its own work: it counts only once the person
        // approves it, so it waits like any other check that needs approval
        let own = [&task.owner_kwant_id, &task.suggested_kwant_id].iter().any(|o| o.as_deref() == Some(scope.kwant_id.as_str()));
        let approved = approved && !own;
        let c = NewCheck {
            task_id: task.id.clone(),
            kind: kind.into(),
            spec,
            required: args["required"].as_bool().unwrap_or(true),
            approved,
            requested_by: scope.kwant_id.clone(),
            approved_by: None,
        };
        match state.db.add_check(&c).await {
            Ok(c) => {
                only = Some(c.id.clone());
                added = Some(c);
            }
            Err(e) => return fail(format!("Could not add the check: {e}")),
        }
    } else if let Some(id) = &only {
        if !matches!(state.db.get_check(id).await, Ok(Some(c)) if c.task_id == task.id) {
            return fail("No such check on this task.");
        }
    }
    let (checks, after) = run_task_checks(state, &task.id, only.as_deref()).await;
    let shown: Vec<Value> = checks
        .iter()
        .filter(|c| only.as_deref().is_none_or(|id| id == c.id))
        .map(|c| json!({"id": c.id, "kind": c.kind, "approved": c.approved, "status": c.status, "output": c.output_excerpt}))
        .collect();
    let mut v = json!({"checks": shown, "task_state": after.map(|t| t.state)});
    if added.is_some_and(|c| !c.approved) {
        v["note"] = json!("This check waits for the person's approval before it runs.");
    }
    ok(v)
}

async fn review(state: &AppState, scope: &SelfScope, task: &Task, args: &Value) -> ToolResult {
    if task.owner_kwant_id.as_deref() == Some(scope.kwant_id.as_str()) {
        return fail("You can't review your own task; a teammate or the person does.");
    }
    if task.state != "review" {
        return fail(format!("The task is {}, not in review.", task.state));
    }
    let verdict = args["verdict"].as_str().unwrap_or("");
    if !matches!(verdict, "pass" | "fail") {
        return fail("verdict must be pass or fail.");
    }
    let Some(note) = args["note"].as_str().map(str::trim).filter(|s| !s.is_empty()) else { return fail("note is required.") };
    let ids: Vec<String> = args["evidence_ids"].as_array().map(|a| a.iter().filter_map(|v| v.as_str().map(String::from)).collect()).unwrap_or_default();
    let on_task = state.db.list_evidence(&task.id).await.unwrap_or_default();
    if let Some(bad) = ids.iter().find(|id| !on_task.iter().any(|e| &e.id == *id)) {
        return fail(format!("Evidence {bad} isn't attached to this task."));
    }
    if verdict == "pass" && ids.is_empty() {
        return fail("A pass must cite at least one evidence id on this task.");
    }
    let c = NewCheck {
        task_id: task.id.clone(),
        kind: "peer_review".into(),
        spec: json!({"note": note.chars().take(2000).collect::<String>(), "evidence_ids": ids}),
        required: true,
        approved: true,
        requested_by: scope.kwant_id.clone(),
        approved_by: None,
    };
    let check = match state.db.add_check(&c).await {
        Ok(c) => c,
        Err(e) => return fail(format!("Could not record the review: {e}")),
    };
    let status = if verdict == "pass" { "passed" } else { "failed" };
    if let Err(e) = state.db.finish_check(&check.id, status, Some(note), &scope.kwant_id, None).await {
        return fail(format!("Could not record the review: {e}"));
    }
    let _ = state.db.log_task_event(&task.id, "review", &scope.kwant_id, &json!({"verdict": verdict, "check_id": check.id})).await;
    let after = evaluate(state, &task.id).await;
    if let Some(t) = &after {
        super::emit_task(state, t).await;
    }
    ok(json!({"task_state": after.map(|t| t.state), "check_id": check.id}))
}

