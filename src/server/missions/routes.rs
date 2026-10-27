//! `/api/missions`, `/api/tasks` and `/api/checks`. Reads are open like the rest of the app API;
//! every change is person-only (`auth.rs`), so a bot token can't verify, approve or cancel.

use super::{
    checks, checks_json, emit_mission, emit_task, evaluate, mission_json, task_json, transition, validate_check, Actor, MoveError,
    PlanBy, PlanCheck, PlanPlace, PlanTask,
};
use crate::db::missions::{NewCheck, NewTask, Task, TaskChange, TaskEvent, MISSION_STATUSES, SOURCE_KINDS, TASK_STATES};
use crate::db::spaces::SpaceId;
use crate::server::spaces::{space_param, Scope};
use crate::server::AppState;
use axum::{
    extract::{Path, Query, RawQuery, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use serde::Deserialize;
use serde_json::{json, Value};

fn err(status: StatusCode, msg: impl Into<String>) -> Response {
    (status, Json(json!({"error": msg.into()}))).into_response()
}

fn bad(msg: impl Into<String>) -> Response {
    err(StatusCode::BAD_REQUEST, msg)
}

fn conflict(msg: impl Into<String>) -> Response {
    err(StatusCode::CONFLICT, msg)
}

fn not_found(what: &str) -> Response {
    err(StatusCode::NOT_FOUND, format!("no such {what}"))
}

fn internal(e: impl std::fmt::Display) -> Response {
    tracing::warn!(error = %e, "mission control");
    err(StatusCode::INTERNAL_SERVER_ERROR, "something went wrong")
}

#[allow(clippy::result_large_err)]
fn moved(r: Result<Task, MoveError>) -> Result<Task, Response> {
    r.map_err(|e| match e {
        MoveError::Missing => not_found("task"),
        MoveError::Db(crate::db::DbError::Sqlx(_)) | MoveError::NotAllowed(_) | MoveError::Stale => conflict(e.to_string()),
        MoveError::Db(e) => internal(e),
    })
}

/// The task `id`, refused (404) when an explicit `?space=` names another space.
#[allow(clippy::result_large_err)]
async fn load_task(state: &AppState, query: Option<&str>, id: &str) -> Result<Task, Response> {
    match state.db.get_task(id).await {
        Ok(Some(t)) if space_param(query).is_none_or(|s| s.trim() == t.space_id) => Ok(t),
        Ok(_) => Err(not_found("task")),
        Err(e) => Err(internal(e)),
    }
}

#[allow(clippy::result_large_err)]
async fn load_mission(state: &AppState, query: Option<&str>, id: &str) -> Result<crate::db::missions::Mission, Response> {
    match state.db.get_mission(id).await {
        Ok(Some(m)) if space_param(query).is_none_or(|s| s.trim() == m.space_id) => Ok(m),
        Ok(_) => Err(not_found("mission")),
        Err(e) => Err(internal(e)),
    }
}

async fn task_out(state: &AppState, t: &Task) -> Response {
    Json(task_json(state, t).await).into_response()
}

async fn mission_out(state: &AppState, id: &str) -> Response {
    match state.db.get_mission(id).await {
        Ok(Some(m)) => Json(mission_json(state, &m).await).into_response(),
        Ok(None) => not_found("mission"),
        Err(e) => internal(e),
    }
}

async fn event_json(state: &AppState, e: &TaskEvent) -> Value {
    let mut v = serde_json::to_value(e).unwrap_or(Value::Null);
    if let Some(obj) = v.as_object_mut() {
        obj.insert("detail".into(), e.detail_value());
        obj.insert("actor_name".into(), json!(super::name_of(state, Some(&e.actor)).await));
    }
    v
}

// Missions

#[derive(Deserialize)]
pub struct MissionQuery {
    status: Option<String>,
}

pub async fn list_missions(State(state): State<AppState>, Scope(space): Scope, Query(q): Query<MissionQuery>) -> Response {
    if q.status.as_deref().is_some_and(|s| !MISSION_STATUSES.contains(&s)) {
        return bad("unknown status");
    }
    match state.db.list_missions(&space, q.status.as_deref()).await {
        Ok(ms) => {
            let mut out = Vec::new();
            for m in &ms {
                out.push(mission_json(&state, m).await);
            }
            Json(json!({"missions": out})).into_response()
        }
        Err(e) => internal(e),
    }
}

#[derive(Deserialize)]
pub struct CreateMission {
    title: String,
    #[serde(default)]
    goal: String,
    lead_kwant_id: Option<String>,
    budget_tokens: Option<i64>,
    #[serde(default)]
    tasks: Vec<PlanTask>,
    channel_id: Option<String>,
    thread_id: Option<String>,
}

pub async fn create_mission(State(state): State<AppState>, Scope(space): Scope, Json(p): Json<CreateMission>) -> Response {
    let bots = match state.db.list_kwants(&space).await {
        Ok(b) => b,
        Err(e) => return internal(e),
    };
    let lead = match &p.lead_kwant_id {
        Some(id) => match super::bot_in_space(&state, &space, id).await {
            Some(k) => k,
            None => return bad("lead_kwant_id isn't a bot in this space"),
        },
        None => match bots.first() {
            Some(k) => k.clone(),
            None => return bad("this space has no bots to lead a mission"),
        },
    };
    let ws = crate::tools::bot_workspace(&crate::db::spaces::space_data_dir(&state.config.data_dir, &space), &lead.id);
    let place = PlanPlace {
        direct_kwant_id: if p.channel_id.is_none() && p.thread_id.is_none() { Some(lead.id.clone()) } else { None },
        channel_id: p.channel_id.clone(),
        thread_id: p.thread_id.clone(),
        workspace_path: ws.to_string_lossy().into_owned(),
    };
    let by = PlanBy { created_by: "person", active: true };
    match super::create_plan(&state, &space, &lead, &p.title, &p.goal, p.budget_tokens, &p.tasks, by, place).await {
        Ok((m, _)) => {
            super::mission_chat_line(&state, &m, format!("Mission \"{}\" started.", m.title)).await;
            mission_out(&state, &m.id).await
        }
        Err(e) => bad(e),
    }
}

pub async fn get_mission(State(state): State<AppState>, RawQuery(q): RawQuery, Path(id): Path<String>) -> Response {
    let m = match load_mission(&state, q.as_deref(), &id).await {
        Ok(m) => m,
        Err(r) => return r,
    };
    let space = SpaceId::new(&m.space_id);
    let tasks = state.db.list_tasks(&space, None, Some(&m.id), None, 1000).await.unwrap_or_default();
    let mut out = Vec::new();
    for t in &tasks {
        out.push(task_json(&state, t).await);
    }
    Json(json!({"mission": mission_json(&state, &m).await, "tasks": out})).into_response()
}

#[derive(Deserialize)]
pub struct PatchMission {
    title: Option<String>,
    goal: Option<String>,
    budget_tokens: Option<i64>,
}

pub async fn patch_mission(State(state): State<AppState>, RawQuery(q): RawQuery, Path(id): Path<String>, Json(p): Json<PatchMission>) -> Response {
    let m = match load_mission(&state, q.as_deref(), &id).await {
        Ok(m) => m,
        Err(r) => return r,
    };
    if matches!(m.status.as_str(), "done" | "cancelled") {
        return conflict(format!("the mission is {}", m.status));
    }
    let title = p.title.as_deref().map(str::trim);
    if title.is_some_and(|t| t.is_empty() || t.chars().count() > 200) {
        return bad("title must be 1–200 characters");
    }
    let (lo, hi) = super::BUDGET_RANGE;
    if p.budget_tokens.is_some_and(|b| !(lo..=hi).contains(&b)) {
        return bad(format!("budget_tokens must be {lo}–{hi}"));
    }
    if let Err(e) = state.db.edit_mission(&id, title, p.goal.as_deref().map(str::trim), p.budget_tokens).await {
        return internal(e);
    }
    emit_mission(&state, &id).await;
    mission_out(&state, &id).await
}

pub async fn approve_mission(State(state): State<AppState>, RawQuery(q): RawQuery, Path(id): Path<String>) -> Response {
    let m = match load_mission(&state, q.as_deref(), &id).await {
        Ok(m) => m,
        Err(r) => return r,
    };
    match state.db.set_mission_status(&id, "draft", "active", Some("person")).await {
        Ok(true) => {}
        Ok(false) => return conflict(format!("the mission is {}, not a draft", m.status)),
        Err(e) => return internal(e),
    }
    super::promote(&state, &id).await;
    if let Ok(Some(m)) = state.db.get_mission(&id).await {
        super::mission_chat_line(&state, &m, format!("Approved: mission \"{}\" is running.", m.title)).await;
    }
    emit_mission(&state, &id).await;
    mission_out(&state, &id).await
}

pub async fn cancel_mission(State(state): State<AppState>, RawQuery(q): RawQuery, Path(id): Path<String>) -> Response {
    let m = match load_mission(&state, q.as_deref(), &id).await {
        Ok(m) => m,
        Err(r) => return r,
    };
    let ok = match m.status.as_str() {
        "draft" | "active" => state.db.set_mission_status(&id, &m.status, "cancelled", Some("person")).await,
        s => return conflict(format!("the mission is already {s}")),
    };
    match ok {
        Ok(true) => {}
        Ok(false) => return conflict("the mission changed meanwhile"),
        Err(e) => return internal(e),
    }
    let space = SpaceId::new(&m.space_id);
    for t in state.db.list_tasks(&space, None, Some(&id), None, 1000).await.unwrap_or_default() {
        if !matches!(t.state.as_str(), "verified" | "cancelled") {
            let change = TaskChange { reason: Some("the mission was cancelled".into()), ..Default::default() };
            let _ = transition(&state, &t.id, &t.state, "cancelled", Actor::Person, "person", change).await;
        }
    }
    emit_mission(&state, &id).await;
    mission_out(&state, &id).await
}

#[derive(Deserialize, Default)]
pub struct RunMission {
    max_tasks: Option<usize>,
}

/// Test hook and "Run now": starts up to `max_tasks` runner turns now, even with the board off.
pub async fn run_mission(State(state): State<AppState>, RawQuery(q): RawQuery, Path(id): Path<String>, body: Option<Json<RunMission>>) -> Response {
    let m = match load_mission(&state, q.as_deref(), &id).await {
        Ok(m) => m,
        Err(r) => return r,
    };
    let max = body.and_then(|b| b.0.max_tasks).unwrap_or(1);
    if !(1..=5).contains(&max) {
        return bad("max_tasks must be 1–5");
    }
    if m.status != "active" {
        return conflict(format!("the mission is {}, not active", m.status));
    }
    let (started, reason) = super::runner::run_mission(&state, &m, max).await;
    let started: Vec<Value> = started.into_iter().map(|(t, k)| json!({"task_id": t, "kwant_id": k})).collect();
    let mut v = json!({"started": started});
    if let Some(r) = reason {
        v["reason"] = json!(r);
    }
    Json(v).into_response()
}

pub async fn insights(State(state): State<AppState>, Scope(space): Scope) -> Response {
    let since = chrono::Utc::now() - chrono::Duration::days(7);
    let events = match state.db.space_events_since(&space, since).await {
        Ok(e) => e,
        Err(e) => return internal(e),
    };
    let mut verified = std::collections::HashSet::new();
    let mut false_success = 0;
    let mut waiting_result: std::collections::HashSet<String> = Default::default();
    let mut interventions = 0;
    for e in &events {
        let person = e.actor == "person";
        match e.kind.as_str() {
            "state" | "lease_expired" | "budget" => {
                if e.to_state.as_deref() == Some("verified") {
                    verified.insert(e.task_id.clone());
                }
                if e.to_state.as_deref() == Some("review") && !matches!(e.actor.as_str(), "kwant" | "person") {
                    waiting_result.insert(e.task_id.clone());
                }
                if person {
                    interventions += 1;
                }
            }
            "check_result" if waiting_result.contains(&e.task_id) => {
                let status = e.detail_value()["status"].as_str().unwrap_or("").to_string();
                if !person && matches!(status.as_str(), "failed" | "error") {
                    false_success += 1;
                    waiting_result.remove(&e.task_id);
                } else if status == "passed" {
                    waiting_result.remove(&e.task_id);
                }
                if person {
                    interventions += 1;
                }
            }
            "check_result" if person => interventions += 1,
            "check_added" if person && e.detail_value()["approval"] == json!(true) => interventions += 1,
            _ => {}
        }
    }
    let all = state.db.list_tasks(&space, None, None, None, 1000).await.unwrap_or_default();
    let mut needs_you = all.iter().filter(|t| t.state == "needs_you").count();
    for t in all.iter().filter(|t| t.state == "review") {
        if !super::check_approvals(&state, t).await.is_empty() {
            needs_you += 1;
        }
    }
    let blocked = all.iter().filter(|t| t.state == "blocked").count();
    let tokens: i64 = all.iter().filter(|t| t.updated_at >= since).map(|t| t.spent_tokens).sum();
    let still_blocked: Vec<&str> = all.iter().filter(|t| t.state == "blocked").map(|t| t.id.as_str()).collect();
    let blocked_minutes = blocked_minutes(&events, &still_blocked, since, chrono::Utc::now());
    let per = if verified.is_empty() { Value::Null } else { json!(tokens / verified.len() as i64) };
    Json(json!({
        "verified_7d": verified.len(),
        "false_success_7d": false_success,
        "needs_you_now": needs_you,
        "blocked_now": blocked,
        "blocked_minutes_7d": blocked_minutes,
        "interventions_7d": interventions,
        "tokens_7d": tokens,
        "tokens_per_verified_7d": per,
    }))
    .into_response()
}

/// Minutes tasks spent in `blocked` between `since` and `now`, from the state events in that
/// window. A task whose first event in the window leaves `blocked`, or that is still blocked with
/// no event in the window, counts from `since`.
pub fn blocked_minutes(
    events: &[TaskEvent],
    still_blocked: &[&str],
    since: chrono::DateTime<chrono::Utc>,
    now: chrono::DateTime<chrono::Utc>,
) -> i64 {
    let mut open: std::collections::HashMap<&str, chrono::DateTime<chrono::Utc>> = Default::default();
    let mut seen: std::collections::HashSet<&str> = Default::default();
    let mut total = chrono::Duration::zero();
    for e in events.iter().filter(|e| e.to_state.is_some()) {
        let id = e.task_id.as_str();
        if seen.insert(id) && e.from_state.as_deref() == Some("blocked") {
            open.insert(id, since);
        }
        if let Some(start) = open.remove(id) {
            total += e.created_at - start;
        }
        if e.to_state.as_deref() == Some("blocked") {
            open.insert(id, e.created_at);
        }
    }
    for id in still_blocked {
        if !seen.contains(id) {
            open.insert(id, since);
        }
    }
    for start in open.values() {
        total += now - *start;
    }
    total.num_minutes().max(0)
}

// Tasks

#[derive(Deserialize)]
pub struct TaskQuery {
    state: Option<String>,
    mission_id: Option<String>,
    source_kind: Option<String>,
    limit: Option<i64>,
}

pub async fn list_tasks(State(state): State<AppState>, Scope(space): Scope, Query(q): Query<TaskQuery>) -> Response {
    if q.state.as_deref().is_some_and(|s| !TASK_STATES.contains(&s)) {
        return bad("unknown state");
    }
    if q.source_kind.as_deref().is_some_and(|s| !SOURCE_KINDS.contains(&s)) {
        return bad("unknown source_kind");
    }
    let limit = q.limit.unwrap_or(500).clamp(1, 1000);
    match state.db.list_tasks(&space, q.state.as_deref(), q.mission_id.as_deref(), q.source_kind.as_deref(), limit).await {
        Ok(ts) => {
            let mut out = Vec::new();
            for t in &ts {
                out.push(task_json(&state, t).await);
            }
            Json(json!({"tasks": out})).into_response()
        }
        Err(e) => internal(e),
    }
}

#[derive(Deserialize)]
pub struct CreateTask {
    title: String,
    #[serde(default)]
    acceptance: String,
    mission_id: Option<String>,
    owner_kwant_id: Option<String>,
    priority: Option<i64>,
    #[serde(default)]
    depends_on: Vec<String>,
    budget_tokens: Option<i64>,
    #[serde(default)]
    checks: Vec<PlanCheck>,
    source_kind: Option<String>,
    source_id: Option<String>,
}

pub async fn create_task(State(state): State<AppState>, Scope(space): Scope, Json(p): Json<CreateTask>) -> Response {
    let title = p.title.trim();
    if title.is_empty() || title.chars().count() > 200 {
        return bad("title must be 1–200 characters");
    }
    let source = p.source_kind.as_deref().unwrap_or("person");
    if !matches!(source, "person" | "zombie") {
        return bad("source_kind must be person or zombie");
    }
    if source == "zombie" && p.source_id.as_deref().is_none_or(|s| s.trim().is_empty()) {
        return bad("a zombie task needs source_id");
    }
    if source == "zombie" {
        if let Ok(Some(t)) = state.db.task_by_source("zombie", p.source_id.as_deref().unwrap_or("")).await {
            if !matches!(t.state.as_str(), "verified" | "cancelled" | "failed") {
                return conflict("that loose end already has a task");
            }
        }
    }
    if p.priority.is_some_and(|x| !(0..=3).contains(&x)) {
        return bad("priority must be 0–3");
    }
    let mission = match &p.mission_id {
        Some(id) => match state.db.get_mission(id).await {
            Ok(Some(m)) if m.space_id == space.as_str() => {
                if matches!(m.status.as_str(), "done" | "cancelled") {
                    return conflict(format!("the mission is {}", m.status));
                }
                Some(m)
            }
            Ok(_) => return bad("mission_id isn't a mission in this space"),
            Err(e) => return internal(e),
        },
        None => None,
    };
    let owner = match &p.owner_kwant_id {
        Some(o) => match super::bot_in_space(&state, &space, o).await {
            Some(k) => Some(k),
            None => return bad("owner_kwant_id isn't a bot in this space"),
        },
        None => None,
    };
    let budget = p.budget_tokens.unwrap_or(super::DEFAULT_TASK_BUDGET);
    if !(1..=super::BUDGET_RANGE.1).contains(&budget) {
        return bad(format!("budget_tokens must be 1–{}", super::BUDGET_RANGE.1));
    }
    let mut approvals = Vec::new();
    for c in &p.checks {
        match validate_check(&c.kind, &c.spec, true) {
            Ok(a) => approvals.push(a),
            Err(e) => return bad(e),
        }
    }
    for d in &p.depends_on {
        match state.db.get_task(d).await {
            Ok(Some(t)) if t.space_id == space.as_str() && t.mission_id == p.mission_id => {}
            Ok(_) => return bad(format!("depends_on: {d} isn't a task of the same mission in this space")),
            Err(e) => return internal(e),
        }
    }
    // Ready at once unless it waits on something; a draft mission's tasks wait for approval
    let ready = p.depends_on.is_empty() && mission.as_ref().is_none_or(|m| m.status == "active");
    let (channel_id, direct_kwant_id, thread_id) = match (&mission, &owner) {
        (Some(m), _) => (m.channel_id.clone(), m.direct_kwant_id.clone(), m.thread_id.clone()),
        (None, Some(k)) => (None, Some(k.id.clone()), None),
        (None, None) => (None, None, None),
    };
    let new = NewTask {
        mission_id: p.mission_id.clone(),
        title: title.into(),
        acceptance: p.acceptance.trim().into(),
        state: "backlog".into(),
        priority: p.priority.unwrap_or(1),
        owner_kwant_id: None,
        suggested_kwant_id: owner.map(|k| k.id),
        budget_tokens: budget,
        max_attempts: super::DEFAULT_MAX_ATTEMPTS,
        source_kind: source.into(),
        source_id: p.source_id.clone(),
        channel_id,
        direct_kwant_id,
        thread_id,
        created_by: "person".into(),
    };
    let task = match state.db.create_task(&space, &new).await {
        Ok(t) => t,
        Err(e) => return internal(e),
    };
    for (c, approved) in p.checks.iter().zip(approvals) {
        let nc = NewCheck {
            task_id: task.id.clone(),
            kind: c.kind.clone(),
            spec: c.spec.clone(),
            required: c.required.unwrap_or(true),
            approved,
            requested_by: "person".into(),
            approved_by: Some("person".into()),
        };
        if let Err(e) = state.db.add_check(&nc).await {
            return internal(e);
        }
    }
    for d in &p.depends_on {
        if let Err(e) = state.db.add_dependency(&task.id, d).await {
            return bad(e.to_string());
        }
    }
    let task = if ready {
        let change = TaskChange { reason: Some("created by the person".into()), ..Default::default() };
        match transition(&state, &task.id, "backlog", "ready", Actor::Kwant, "kwant", change).await {
            Ok(t) => t,
            Err(_) => task,
        }
    } else {
        if let Some(m) = &mission {
            super::promote(&state, &m.id).await;
        }
        state.db.get_task(&task.id).await.ok().flatten().unwrap_or(task)
    };
    emit_task(&state, &task).await;
    if let Some(m) = &task.mission_id {
        emit_mission(&state, m).await;
    }
    (StatusCode::OK, task_out(&state, &task).await).into_response()
}

pub async fn get_task(State(state): State<AppState>, RawQuery(q): RawQuery, Path(id): Path<String>) -> Response {
    let t = match load_task(&state, q.as_deref(), &id).await {
        Ok(t) => t,
        Err(r) => return r,
    };
    let checks = state.db.list_checks(&id).await.unwrap_or_default();
    let evidence = state.db.list_evidence(&id).await.unwrap_or_default();
    let events = state.db.task_events(&id, 500).await.unwrap_or_default();
    let mut ev = Vec::new();
    for e in &events {
        ev.push(event_json(&state, e).await);
    }
    Json(json!({
        "task": task_json(&state, &t).await,
        "checks": checks_json(&state, &t, &checks).await,
        "evidence": evidence,
        "events": ev,
    }))
    .into_response()
}

#[derive(Deserialize)]
pub struct PatchTask {
    title: Option<String>,
    acceptance: Option<String>,
    priority: Option<i64>,
    owner_kwant_id: Option<String>,
    budget_tokens: Option<i64>,
}

pub async fn patch_task(State(state): State<AppState>, RawQuery(q): RawQuery, Path(id): Path<String>, Json(p): Json<PatchTask>) -> Response {
    let t = match load_task(&state, q.as_deref(), &id).await {
        Ok(t) => t,
        Err(r) => return r,
    };
    if matches!(t.state.as_str(), "verified" | "cancelled") {
        return conflict(format!("the task is {}", t.state));
    }
    let title = p.title.as_deref().map(str::trim);
    if title.is_some_and(|x| x.is_empty() || x.chars().count() > 200) {
        return bad("title must be 1–200 characters");
    }
    if p.priority.is_some_and(|x| !(0..=3).contains(&x)) {
        return bad("priority must be 0–3");
    }
    if p.budget_tokens.is_some_and(|b| !(1..=super::BUDGET_RANGE.1).contains(&b)) {
        return bad(format!("budget_tokens must be 1–{}", super::BUDGET_RANGE.1));
    }
    let owner = match &p.owner_kwant_id {
        Some(o) => match super::bot_in_space(&state, &SpaceId::new(&t.space_id), o).await {
            Some(k) => Some(k.id),
            None => return bad("owner_kwant_id isn't a bot in this space"),
        },
        None => None,
    };
    if let Err(e) = state.db.edit_task(&id, title, p.acceptance.as_deref().map(str::trim), p.priority, owner.as_deref(), p.budget_tokens, "person").await {
        return internal(e);
    }
    match state.db.get_task(&id).await {
        Ok(Some(t)) => {
            emit_task(&state, &t).await;
            task_out(&state, &t).await
        }
        _ => not_found("task"),
    }
}

#[derive(Deserialize, Default)]
pub struct Note {
    note: Option<String>,
    reason: Option<String>,
}

fn body(b: Option<Json<Note>>) -> Note {
    b.map(|b| b.0).unwrap_or_default()
}

/// The person's Verify: runs the task's pending kwant checks, then records a passing `person` check.
/// Verified only if every required check then passes.
pub async fn verify_task(State(state): State<AppState>, RawQuery(q): RawQuery, Path(id): Path<String>, b: Option<Json<Note>>) -> Response {
    let t = match load_task(&state, q.as_deref(), &id).await {
        Ok(t) => t,
        Err(r) => return r,
    };
    if t.state != "review" {
        return conflict(format!("only a task in review can be verified; this one is {}", t.state));
    }
    let note = body(b).note.unwrap_or_default();
    let c = NewCheck {
        task_id: id.clone(),
        kind: "person".into(),
        spec: json!({"note": note.chars().take(2000).collect::<String>()}),
        required: true,
        approved: true,
        requested_by: "person".into(),
        approved_by: Some("person".into()),
    };
    let check = match state.db.add_check(&c).await {
        Ok(c) => c,
        Err(e) => return internal(e),
    };
    if let Err(e) = state.db.finish_check(&check.id, "passed", Some(&note), "person", None).await {
        return internal(e);
    }
    // Runs the approved kwant checks that haven't passed yet; evaluates
    let pending: Vec<_> = state.db.list_checks(&id).await.unwrap_or_default().into_iter().filter(|c| c.approved && c.status == "pending").collect();
    for c in &pending {
        checks::run_check(&state, &t, c).await;
    }
    let after = evaluate(&state, &id).await;
    match after {
        Some(t) if t.state == "verified" => task_out(&state, &t).await,
        Some(t) => {
            emit_task(&state, &t).await;
            let summary = super::checks_summary(&t, &state.db.list_checks(&id).await.unwrap_or_default());
            conflict(format!("not verified: its checks are {} (the task is {})", summary["status"].as_str().unwrap_or("?"), t.state))
        }
        None => not_found("task"),
    }
}

pub async fn reject_task(State(state): State<AppState>, RawQuery(q): RawQuery, Path(id): Path<String>, b: Option<Json<Note>>) -> Response {
    if let Err(r) = load_task(&state, q.as_deref(), &id).await {
        return r;
    }
    let b = body(b);
    let Some(reason) = b.reason.or(b.note).map(|s| s.trim().to_string()).filter(|s| !s.is_empty()) else {
        return bad("reason is required");
    };
    let change = TaskChange { reason: Some(reason), attempt: true, ..Default::default() };
    match moved(transition(&state, &id, "review", "ready", Actor::Person, "person", change).await) {
        Ok(t) => task_out(&state, &t).await,
        Err(r) => r,
    }
}

pub async fn block_task(State(state): State<AppState>, RawQuery(q): RawQuery, Path(id): Path<String>, b: Option<Json<Note>>) -> Response {
    let t = match load_task(&state, q.as_deref(), &id).await {
        Ok(t) => t,
        Err(r) => return r,
    };
    let b = body(b);
    let Some(reason) = b.reason.or(b.note).map(|s| s.trim().to_string()).filter(|s| !s.is_empty()) else {
        return bad("reason is required");
    };
    let change = TaskChange { reason: Some(reason), ..Default::default() };
    match moved(transition(&state, &id, &t.state, "blocked", Actor::Person, "person", change).await) {
        Ok(t) => task_out(&state, &t).await,
        Err(r) => r,
    }
}

pub async fn unblock_task(State(state): State<AppState>, RawQuery(q): RawQuery, Path(id): Path<String>) -> Response {
    let t = match load_task(&state, q.as_deref(), &id).await {
        Ok(t) => t,
        Err(r) => return r,
    };
    if t.state != "blocked" {
        return conflict(format!("the task is {}, not blocked", t.state));
    }
    let change = TaskChange::default();
    let t = match moved(transition(&state, &id, "blocked", "backlog", Actor::Person, "person", change).await) {
        Ok(t) => t,
        Err(r) => return r,
    };
    super::promote_one(&state, &t).await;
    match state.db.get_task(&id).await {
        Ok(Some(t)) => task_out(&state, &t).await,
        _ => not_found("task"),
    }
}

pub async fn retry_task(State(state): State<AppState>, RawQuery(q): RawQuery, Path(id): Path<String>) -> Response {
    if let Err(r) = load_task(&state, q.as_deref(), &id).await {
        return r;
    }
    // A retry gets the same number of attempts again
    let change = TaskChange { reason: Some("retried by the person".into()), detail: Some(json!({"reset_attempts": true})), ..Default::default() };
    match moved(transition(&state, &id, "failed", "ready", Actor::Person, "person", change).await) {
        Ok(t) => {
            let _ = state.db.reset_attempts(&id).await;
            let t = state.db.get_task(&id).await.ok().flatten().unwrap_or(t);
            emit_task(&state, &t).await;
            task_out(&state, &t).await
        }
        Err(r) => r,
    }
}

pub async fn cancel_task(State(state): State<AppState>, RawQuery(q): RawQuery, Path(id): Path<String>) -> Response {
    let t = match load_task(&state, q.as_deref(), &id).await {
        Ok(t) => t,
        Err(r) => return r,
    };
    let change = TaskChange { reason: Some("cancelled by the person".into()), ..Default::default() };
    match moved(transition(&state, &id, &t.state, "cancelled", Actor::Person, "person", change).await) {
        Ok(t) => task_out(&state, &t).await,
        Err(r) => r,
    }
}

#[derive(Deserialize)]
pub struct AddCheck {
    kind: String,
    #[serde(default)]
    spec: Value,
    required: Option<bool>,
}

pub async fn add_task_check(State(state): State<AppState>, RawQuery(q): RawQuery, Path(id): Path<String>, Json(p): Json<AddCheck>) -> Response {
    let t = match load_task(&state, q.as_deref(), &id).await {
        Ok(t) => t,
        Err(r) => return r,
    };
    if matches!(t.state.as_str(), "verified" | "cancelled") {
        return conflict(format!("the task is {}", t.state));
    }
    if let Err(e) = validate_check(&p.kind, &p.spec, true) {
        return bad(e);
    }
    let c = NewCheck { task_id: id.clone(), kind: p.kind, spec: p.spec, required: p.required.unwrap_or(true), approved: true, requested_by: "person".into(), approved_by: Some("person".into()) };
    let check = match state.db.add_check(&c).await {
        Ok(c) => c,
        Err(e) => return internal(e),
    };
    let check = if t.state == "review" { checks::run_check(&state, &t, &check).await } else { check };
    if let Some(t) = evaluate(&state, &id).await {
        emit_task(&state, &t).await;
    }
    Json(super::check_json(&t, &check, super::name_of(&state, check.checked_by.as_deref()).await)).into_response()
}

pub async fn run_task_checks(State(state): State<AppState>, RawQuery(q): RawQuery, Path(id): Path<String>) -> Response {
    if let Err(r) = load_task(&state, q.as_deref(), &id).await {
        return r;
    }
    let (checks, task) = checks::run_task_checks(&state, &id, None).await;
    let Some(task) = task else { return not_found("task") };
    Json(json!({"checks": checks_json(&state, &task, &checks).await, "task": task_json(&state, &task).await})).into_response()
}

pub async fn approve_check(State(state): State<AppState>, RawQuery(q): RawQuery, Path(id): Path<String>) -> Response {
    let check = match state.db.get_check(&id).await {
        Ok(Some(c)) => c,
        Ok(None) => return not_found("check"),
        Err(e) => return internal(e),
    };
    let t = match load_task(&state, q.as_deref(), &check.task_id).await {
        Ok(t) => t,
        Err(r) => return r,
    };
    match state.db.approve_check(&id).await {
        Ok(true) => {}
        Ok(false) => return conflict(format!("the check is {}, not waiting for approval", check.status)),
        Err(e) => return internal(e),
    }
    let _ = state.db.log_task_event(&t.id, "check_added", "person", &json!({"check_id": id, "approved": true, "approval": true})).await;
    let check = state.db.get_check(&id).await.ok().flatten().unwrap_or(check);
    let check = if t.state == "review" { checks::run_check(&state, &t, &check).await } else { check };
    if let Some(t) = evaluate(&state, &t.id).await {
        emit_task(&state, &t).await;
    }
    Json(super::check_json(&t, &check, super::name_of(&state, check.checked_by.as_deref()).await)).into_response()
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{Duration, TimeZone, Utc};

    fn ev(task: &str, from: &str, to: &str, at: chrono::DateTime<Utc>) -> TaskEvent {
        TaskEvent {
            id: 0,
            task_id: task.into(),
            mission_id: None,
            kind: "state".into(),
            actor: "person".into(),
            from_state: Some(from.into()),
            to_state: Some(to.into()),
            detail: String::new(),
            created_at: at,
        }
    }

    #[test]
    fn blocked_minutes_sums_time_in_blocked_within_the_window() {
        let since = Utc.with_ymd_and_hms(2026, 10, 20, 0, 0, 0).unwrap();
        let now = since + Duration::days(7);
        let events = vec![
            // a: blocked for 30 minutes inside the window
            ev("a", "ready", "blocked", since + Duration::hours(1)),
            ev("a", "blocked", "ready", since + Duration::hours(1) + Duration::minutes(30)),
            // b: blocked before the window, unblocked 2 hours in
            ev("b", "blocked", "ready", since + Duration::hours(2)),
            // c: blocked 10 minutes before now and still blocked
            ev("c", "ready", "blocked", now - Duration::minutes(10)),
        ];
        // d: blocked the whole window with no event in it
        assert_eq!(blocked_minutes(&events, &["c", "d"], since, now), 30 + 120 + 10 + 7 * 24 * 60);
        assert_eq!(blocked_minutes(&[], &[], since, now), 0);
    }
}
