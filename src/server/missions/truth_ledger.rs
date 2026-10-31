//! Mission Control Phase C1: the Truth Ledger in shadow mode. Memory stays the write path and SQL
//! triggers mirror it into facts (`crate::db::truth_ledger`); this module holds the setting, the
//! per-turn taint bit (a turn that read outside text: its notes land `outside`/`proposed`) and
//! the routes: read the facts, and the person's accept/reject. The bot's memory block is the same
//! in every mode in C1. See docs/mission-control.md §21.

// Handler helpers return the ready-made error `Response`
#![allow(clippy::result_large_err)]

use super::routes::{bad, err, internal, not_found};
use crate::db::truth_ledger::{Decision, FactFilter};
use crate::server::spaces::{space_param, Scope};
use crate::server::AppState;
use axum::{
    extract::{Path, Query, RawQuery, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    Json,
};
use serde::Deserialize;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

pub const SETTING: &str = "truth_ledger";
/// `on` reads the same as `shadow` until C2 makes memory a view of the ledger.
pub const MODES: [&str; 3] = ["off", "shadow", "on"];
const STATUSES: [&str; 4] = ["proposed", "accepted", "superseded", "disputed"];
const SCOPES: [&str; 3] = ["bot", "team", "space"];

/// The saved mode; anything unreadable is `off`.
pub async fn mode(state: &AppState) -> &'static str {
    match state.db.get_setting(SETTING).await.ok().flatten().as_deref().map(str::trim) {
        Some("shadow") => "shadow",
        Some("on") => "on",
        _ => "off",
    }
}

pub fn settings_error(mode: Option<&str>) -> Option<String> {
    mode.filter(|m| !MODES.contains(m)).map(|m| format!("{SETTING} must be off, shadow or on (got {m})"))
}

// Taint: a turn that read outside text (contract §2.4)

/// Turns that read outside text, with when they were marked. Never cleared within a turn; the
/// turn's end removes it, and anything older than a day goes (a turn that never ended cleanly).
static TAINTED: Mutex<Option<HashMap<String, Instant>>> = Mutex::new(None);
const TAINT_TTL: Duration = Duration::from_secs(24 * 3600);

pub fn taint(turn_id: &str) {
    if turn_id.is_empty() {
        return;
    }
    let mut guard = TAINTED.lock().unwrap_or_else(|e| e.into_inner());
    let map = guard.get_or_insert_with(HashMap::new);
    if map.len() > 1000 {
        map.retain(|_, at| at.elapsed() < TAINT_TTL);
    }
    map.entry(turn_id.to_string()).or_insert_with(Instant::now);
}

pub fn is_tainted(turn_id: &str) -> bool {
    let guard = TAINTED.lock().unwrap_or_else(|e| e.into_inner());
    guard.as_ref().is_some_and(|m| m.contains_key(turn_id))
}

/// The turn ended: its id isn't used again.
pub fn end_turn(turn_id: &str) {
    let mut guard = TAINTED.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(map) = guard.as_mut() {
        map.remove(turn_id);
    }
}

/// Built-in tools (claude's, Codex's, the API driver's) that bring text from outside into the turn.
const OUTSIDE_TOOLS: &[&str] = &[
    "WebFetch",
    "WebSearch",
    "Bash",
    "Task",
    "Agent",
    "web_search",
    "web_fetch",
    "command_execution",
    "execute_shell",
];

/// kwant's own tools that read outside text: a page, an attachment, another server's tools.
const OUTSIDE_SELF_TOOLS: &[&str] = &["read_attachment", "mcp_search", crate::server::page_source::TOOL];

fn outside_self_tool(tool: &str) -> bool {
    tool.starts_with("browser_") || OUTSIDE_SELF_TOOLS.contains(&tool)
}

/// Whether calling tool `name` (as an engine reports it: `WebFetch`, `mcp__github__get_issue`,
/// `mcp__kwnt_tools__browser_open`, the API driver's bare `browser_open`) reads outside text.
/// Any MCP server but kwant's own counts.
pub fn reads_outside(name: &str) -> bool {
    if let Some(rest) = name.strip_prefix("mcp__") {
        let self_prefix = format!("{}__", crate::mcp::SELF_SERVER_NAME);
        return match rest.strip_prefix(&self_prefix) {
            Some(tool) => outside_self_tool(tool),
            None => true,
        };
    }
    OUTSIDE_TOOLS.contains(&name) || outside_self_tool(name)
}

/// [`reads_outside`] for an approval request (server and tool split).
pub fn request_reads_outside(req: &crate::mcp::ToolApprovalRequest) -> bool {
    if req.builtin.is_some() {
        return OUTSIDE_TOOLS.contains(&req.tool.as_str()) || OUTSIDE_TOOLS.contains(&req.tool_name.as_str());
    }
    if req.server == crate::mcp::SELF_SERVER_NAME {
        return outside_self_tool(&req.tool);
    }
    !req.server.is_empty() || reads_outside(&req.tool_name)
}

// Routes (all person only, see `auth::person_only`)

#[derive(Deserialize, Default)]
pub struct ListQuery {
    scope: Option<String>,
    scope_id: Option<String>,
    status: Option<String>,
    q: Option<String>,
}

fn opt(v: &Option<String>) -> Option<&str> {
    v.as_deref().map(str::trim).filter(|s| !s.is_empty())
}

/// `GET /api/facts?space=&scope=&scope_id=&status=&q=`: the setting and the space's facts, newest
/// first (at most 500). With the ledger off: `{setting: "off", facts: []}`.
pub async fn list(State(state): State<AppState>, Scope(space): Scope, Query(q): Query<ListQuery>) -> Response {
    let setting = mode(&state).await;
    if setting == "off" {
        return Json(json!({ "setting": setting, "facts": [] })).into_response();
    }
    let filter = FactFilter { scope: opt(&q.scope), scope_id: opt(&q.scope_id), status: opt(&q.status), q: opt(&q.q) };
    if filter.scope.is_some_and(|s| !SCOPES.contains(&s)) {
        return bad("scope must be bot, team or space");
    }
    if filter.status.is_some_and(|s| !STATUSES.contains(&s)) {
        return bad("status must be proposed, accepted, superseded or disputed");
    }
    if filter.q.is_some_and(|t| t.chars().count() > 200) {
        return bad("q is at most 200 characters");
    }
    match state.db.list_facts(space.as_str(), &filter).await {
        Ok(facts) => Json(json!({ "setting": setting, "facts": facts })).into_response(),
        Err(e) => internal(e),
    }
}

async fn space_of(state: &AppState, query: Option<&str>) -> Result<String, Response> {
    match space_param(query) {
        None => Ok(crate::db::spaces::SpaceId::default_space().as_str().to_string()),
        Some(id) => crate::server::spaces::active_space(state, &id).await.map(|s| s.as_str().to_string()),
    }
}

/// `GET /api/facts/:id?space=`: the fact with its events (evidence, citations and disputes come
/// in C2 and are empty). 404 with the ledger off.
pub async fn get(State(state): State<AppState>, RawQuery(q): RawQuery, Path(id): Path<String>) -> Response {
    if mode(&state).await == "off" {
        return not_found("fact");
    }
    let space = match space_of(&state, q.as_deref()).await {
        Ok(s) => s,
        Err(r) => return r,
    };
    match fact_json(&state, &space, &id).await {
        Ok(Some(v)) => Json(v).into_response(),
        Ok(None) => not_found("fact"),
        Err(e) => internal(e),
    }
}

async fn fact_json(state: &AppState, space: &str, id: &str) -> Result<Option<Value>, crate::db::DbError> {
    let Some(fact) = state.db.get_fact(space, id).await? else {
        return Ok(None);
    };
    let events = state.db.fact_events(id).await?;
    Ok(Some(json!({
        "fact": fact,
        "events": events,
        "evidence": [],
        "citations": [],
        "disputes": [],
    })))
}

/// `POST /api/facts/:id/accept?space=` (person only; `Idempotency-Key` optional, the same key
/// replays): a `proposed` fact becomes `accepted`, decided by the person. 409 when it isn't
/// proposed (any more), so of racing clicks exactly one changes it; 409 with the ledger off.
pub async fn accept(state: State<AppState>, q: RawQuery, headers: HeaderMap, id: Path<String>) -> Response {
    decide(state, q, headers, id, true).await
}

/// `POST /api/facts/:id/reject?space=`: like accept; the fact is superseded with no successor.
/// In C1 the bot's note itself stays as it is.
pub async fn reject(state: State<AppState>, q: RawQuery, headers: HeaderMap, id: Path<String>) -> Response {
    decide(state, q, headers, id, false).await
}

async fn decide(State(state): State<AppState>, RawQuery(q): RawQuery, headers: HeaderMap, Path(id): Path<String>, accept: bool) -> Response {
    if mode(&state).await == "off" {
        return err(StatusCode::CONFLICT, "the Truth Ledger is off");
    }
    let space = match space_of(&state, q.as_deref()).await {
        Ok(s) => s,
        Err(r) => return r,
    };
    let route = format!("POST /api/facts/{id}/{}", if accept { "accept" } else { "reject" });
    super::graph::idempotent(&state, &headers, route, || async {
        match state.db.decide_fact(&space, &id, accept).await {
            Ok(Decision::Done) => match fact_json(&state, &space, &id).await {
                Ok(Some(v)) => Ok(v),
                Ok(None) => Err(not_found("fact")),
                Err(e) => Err(internal(e)),
            },
            Ok(Decision::NotProposed(status)) => Err((
                StatusCode::CONFLICT,
                Json(json!({ "error": format!("this fact is already {status}"), "status": status })),
            )
                .into_response()),
            Ok(Decision::NotFound) => Err(not_found("fact")),
            Err(e) => Err(internal(e)),
        }
    })
    .await
}
