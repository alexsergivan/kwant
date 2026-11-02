//! `/api/goals`. Reads are open like the rest of the app API; every change is person-only
//! (`auth.rs`), so a bot can propose a goal (`goal_propose`) but never approve, edit or run one.

use super::{build_goal, emit, emit_goal, event_json, goal_json, valid_budget, valid_mode, valid_title, validate_checks, CheckInput, GoalInput};
use super::{ranged, CONFIRM_RANGE, FIXES_RANGE, INTERVAL_RANGE, PRIORITY_RANGE};
use crate::db::goals::{Created, Edit, Goal};
use crate::db::missions::now_ms;
use crate::server::spaces::{space_param, Scope};
use crate::server::AppState;
use axum::{
    extract::{Path, RawQuery, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    Json,
};
use serde::Deserialize;
use serde_json::{json, Value};

/// Excerpt characters per check on list rows; a goal's own page shows the stored excerpt.
const LIST_EXCERPT: usize = 200;
const PAGE_EXCERPT: usize = 2000;
const PAGE_EVENTS: i64 = 50;

fn err(status: StatusCode, msg: impl Into<String>) -> Response {
    (status, Json(json!({ "error": msg.into() }))).into_response()
}

fn bad(msg: impl Into<String>) -> Response {
    err(StatusCode::BAD_REQUEST, msg)
}

fn conflict(msg: impl Into<String>) -> Response {
    err(StatusCode::CONFLICT, msg)
}

fn not_found() -> Response {
    err(StatusCode::NOT_FOUND, "no such goal")
}

fn internal(e: impl std::fmt::Display) -> Response {
    tracing::warn!(error = %e, "goals");
    err(StatusCode::INTERNAL_SERVER_ERROR, "something went wrong")
}

#[allow(clippy::result_large_err)]
async fn load(state: &AppState, query: Option<&str>, id: &str) -> Result<Goal, Response> {
    match state.db.get_goal(id).await {
        Ok(Some(g)) if space_param(query).is_none_or(|s| s.trim() == g.space_id) => Ok(g),
        Ok(_) => Err(not_found()),
        Err(e) => Err(internal(e)),
    }
}

pub async fn list_goals(State(state): State<AppState>, Scope(space): Scope) -> Response {
    let goals = match state.db.list_goals(space.as_str()).await {
        Ok(g) => g,
        Err(e) => return internal(e),
    };
    let mut out = Vec::new();
    for g in &goals {
        out.push(goal_json(&state, g, LIST_EXCERPT).await);
    }
    Json(json!({ "goals": out })).into_response()
}

pub async fn page(state: &AppState, g: &Goal) -> Value {
    let events = state.db.goal_events(&g.id, PAGE_EVENTS).await.unwrap_or_default();
    let fixes = state.db.goal_fix_links(&g.id).await.unwrap_or_default();
    let spent: i64 = fixes.iter().filter_map(|f| f.spent_tokens).sum::<i64>() + state.db.goal_spent_tokens(&g.id).await.unwrap_or(0);
    json!({
        "goal": goal_json(state, g, PAGE_EXCERPT).await,
        "events": events.iter().map(event_json).collect::<Vec<_>>(),
        "fix_tasks": fixes.iter().map(|f| json!({ "task_id": f.task_id, "state": f.state, "spent_tokens": f.spent_tokens, "created_at": f.created_at })).collect::<Vec<_>>(),
        "missions": crate::server::goals_remediation::missions_of(state, g).await,
        "spent_tokens": spent,
    })
}

pub async fn get_goal(State(state): State<AppState>, RawQuery(q): RawQuery, Path(id): Path<String>) -> Response {
    match load(&state, q.as_deref(), &id).await {
        Ok(g) => Json(page(&state, &g).await).into_response(),
        Err(r) => r,
    }
}

/// The person's goal: active at once, its first check due now.
pub async fn create_goal(State(state): State<AppState>, Scope(space): Scope, headers: HeaderMap, Json(p): Json<GoalInput>) -> Response {
    if let Some(task_id) = p.from_task.as_deref().map(str::trim) {
        if task_id.is_empty() {
            return bad("from_task needs a task id");
        }
        return crate::server::goals_remediation::from_task_route(&state, &space, &headers, task_id).await;
    }
    let route = format!("POST /api/goals?space={}", space.as_str());
    crate::server::missions::graph::idempotent(&state, &headers, route, || async {
        let new = build_goal(&state, &space, &p, "person", false).await.map_err(bad)?;
        match state.db.create_goal(&new, now_ms()).await {
            Ok(Created::Goal(g)) => {
                emit_goal(&state, &g);
                Ok(page(&state, &g).await)
            }
            Ok(Created::TooManyDrafts) => Err(internal("a person's goal counted as a draft")),
            Err(e) => Err(internal(e)),
        }
    })
    .await
}

#[derive(Debug, Deserialize)]
pub struct PatchGoal {
    pub title: Option<String>,
    pub description: Option<String>,
    pub mode: Option<String>,
    pub interval_minutes: Option<i64>,
    pub confirm_failures: Option<i64>,
    pub budget_tokens: Option<i64>,
    pub max_fixes_per_day: Option<i64>,
    pub priority: Option<i64>,
    pub checks: Option<Vec<CheckInput>>,
    /// Pause (true) or resume (false). Resuming checks at once.
    pub paused: Option<bool>,
}

pub async fn patch_goal(State(state): State<AppState>, RawQuery(q): RawQuery, Path(id): Path<String>, Json(p): Json<PatchGoal>) -> Response {
    let g = match load(&state, q.as_deref(), &id).await {
        Ok(g) => g,
        Err(r) => return r,
    };
    let checked = (|| -> Result<_, String> {
        let title = p.title.as_deref().map(valid_title).transpose()?;
        if p.description.as_ref().is_some_and(|d| d.chars().count() > 2000) {
            return Err("description is at most 2000 characters".into());
        }
        let mode = p.mode.as_deref().map(|m| valid_mode(Some(m), "suggest")).transpose()?;
        let interval = p.interval_minutes.map(|n| ranged(Some(n), INTERVAL_RANGE, 0, "interval_minutes")).transpose()?;
        let confirm = p.confirm_failures.map(|n| ranged(Some(n), CONFIRM_RANGE, 0, "confirm_failures")).transpose()?;
        let fixes = p.max_fixes_per_day.map(|n| ranged(Some(n), FIXES_RANGE, 0, "max_fixes_per_day")).transpose()?;
        let priority = p.priority.map(|n| ranged(Some(n), PRIORITY_RANGE, 0, "priority")).transpose()?;
        let budget = p.budget_tokens.map(|b| valid_budget(Some(b))).transpose()?;
        let checks = match &p.checks {
            Some(c) => Some(validate_checks(c, interval.unwrap_or(g.interval_minutes), !g.kwant_id.is_empty() || !g.folder.is_empty())?),
            None => None,
        };
        Ok((title, mode, interval, confirm, fixes, priority, budget, checks))
    })();
    let (title, mode, interval, confirm, fixes, priority, budget, checks) = match checked {
        Ok(v) => v,
        Err(e) => return bad(e),
    };
    let now = now_ms();
    let description = p.description.clone();
    let paused = p.paused;
    let result = state
        .db
        .edit_goal(&g.id, &g.space_id, "person", now, move |g| {
            let mut events = Vec::new();
            let mut changed = Vec::new();
            let schedule_moved = interval.is_some_and(|i| i != g.interval_minutes) || checks.is_some();
            macro_rules! set {
                ($field:ident, $v:expr) => {
                    if let Some(v) = $v {
                        if g.$field != v {
                            changed.push(stringify!($field));
                            g.$field = v;
                        }
                    }
                };
            }
            set!(title, title);
            set!(description, description.map(|d| d.trim().to_string()));
            set!(mode, mode);
            set!(interval_minutes, interval);
            set!(confirm_failures, confirm);
            set!(max_fixes_per_day, fixes);
            set!(priority, priority);
            set!(budget_tokens, budget);
            if checks.is_some() {
                changed.push("checks");
            }
            match paused {
                Some(true) if g.status == "draft" => return Err("approve the draft before pausing it".into()),
                Some(false) if g.status == "draft" => return Err("approve the draft to start checking it".into()),
                Some(true) if g.status != "paused" => {
                    g.status = "paused".into();
                    g.status_reason = String::new();
                    g.paused_at = Some(now);
                    g.next_check_at = None;
                    events.push(("paused", json!({})));
                }
                Some(false) if g.status == "paused" => {
                    // Back to a fresh start: no streak from before the pause carries over
                    g.status = "checking".into();
                    g.status_reason = String::new();
                    g.paused_at = None;
                    g.next_check_at = Some(now);
                    g.fail_streak = 0;
                    g.error_runs = 0;
                    events.push(("resumed", json!({})));
                }
                _ => {}
            }
            if schedule_moved && !matches!(g.status.as_str(), "draft" | "paused") {
                // New checks or a new interval: check now rather than on the old schedule
                g.next_check_at = Some(now);
            }
            if !changed.is_empty() {
                events.push(("edited", json!({ "fields": changed })));
            }
            Ok(Edit { checks, events })
        })
        .await;
    match result {
        Ok(Some(Ok(g))) => {
            emit_goal(&state, &g);
            Json(page(&state, &g).await).into_response()
        }
        Ok(Some(Err(why))) => conflict(why),
        Ok(None) => not_found(),
        Err(e) => internal(e),
    }
}

pub async fn approve_goal(State(state): State<AppState>, RawQuery(q): RawQuery, Path(id): Path<String>) -> Response {
    let g = match load(&state, q.as_deref(), &id).await {
        Ok(g) => g,
        Err(r) => return r,
    };
    match state.db.approve_goal(&g.id, &g.space_id, "person", now_ms()).await {
        Ok(Some(g)) => {
            emit_goal(&state, &g);
            Json(page(&state, &g).await).into_response()
        }
        Ok(None) => conflict(format!("the goal is {}, not a draft", g.status)),
        Err(e) => internal(e),
    }
}

/// Check now: runs the goal's checks at once, in the background, unless it is a draft, paused or
/// already being checked. 202 with the goal; its result comes over SSE.
pub async fn check_goal(State(state): State<AppState>, RawQuery(q): RawQuery, Path(id): Path<String>) -> Response {
    let g = match load(&state, q.as_deref(), &id).await {
        Ok(g) => g,
        Err(r) => return r,
    };
    if matches!(g.status.as_str(), "draft" | "paused") {
        return conflict(format!("the goal is {}", g.status));
    }
    match state.db.lease_goal(&g.id, now_ms(), false).await {
        Ok(Some(leased)) => {
            let body = page(&state, &leased).await;
            let st = state.clone();
            tokio::spawn(async move {
                super::runner::run_goal(&st, leased).await;
            });
            (StatusCode::ACCEPTED, Json(body)).into_response()
        }
        Ok(None) => conflict("the goal is being checked right now"),
        Err(e) => internal(e),
    }
}

pub async fn delete_goal(State(state): State<AppState>, RawQuery(q): RawQuery, Path(id): Path<String>) -> Response {
    let g = match load(&state, q.as_deref(), &id).await {
        Ok(g) => g,
        Err(r) => return r,
    };
    match state.db.delete_goal(&g.id, &g.space_id).await {
        Ok(true) => {
            emit(&state, &g.id, "deleted", &g.space_id);
            Json(json!({ "deleted": g.id })).into_response()
        }
        Ok(false) => not_found(),
        Err(e) => internal(e),
    }
}
