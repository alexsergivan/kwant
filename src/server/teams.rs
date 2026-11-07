//! Ready-made teams (next-077 spec T) and team discovery (spec D) over HTTP: the catalogue, the
//! free-name preview, the one-shot create, Suggest, and the Undo preview. The routes and their
//! errors are `docs/teams-api.md`. Every route but the catalogue is the person's: the
//! `person_only` list refuses a bot's or an agent's token first, and each handler checks again.

#![allow(clippy::result_large_err)]

use super::spaces::{check_workspace, space_param, Scope};
use super::AppState;
use crate::db::models::NewTeam;
use crate::db::spaces::SpaceId;
use crate::db::team_templates::{self as tt, Clash, ClashKind, CreateError, Creation, Plan, PlanBot, PlanRoutine, PlanSkill, Target};
use crate::server::dev::git::{self, GitError, Repo};
use crate::teams::discover::{self, Listing, Mode, Role, Row, Stops};
use crate::teams::templates::{self, CatalogueEntry, Template};
use crate::teams::names;
use axum::{
    body::Bytes,
    extract::{Path, State},
    http::{HeaderMap, StatusCode, Uri},
    response::{IntoResponse, Response},
    Json,
};
use chrono::{DateTime, Duration as ChronoDuration, Utc};
use serde_json::{json, Map, Value};
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

/// How long a suggestion can be used (Q-D6c, A17).
pub const SUGGESTION_MINUTES: i64 = 60;
/// Suggestions kept at once; the oldest goes first.
const MAX_SUGGESTIONS: usize = 256;
/// Suggests reading at once (each may walk a big folder or fetch a repo).
const MAX_READING: usize = 2;
/// `ls-tree` bytes kept (about [`discover::MAX_LISTED`] records).
const MAX_TREE_BYTES: usize = 16 << 20;
/// Longest `Idempotency-Key`.
const MAX_KEY_LEN: usize = 200;

// ---------------------------------------------------------------------------------------------
// Errors

fn fail(status: StatusCode, code: &str, message: impl Into<String>) -> Response {
    (status, Json(json!({ "error": message.into(), "code": code }))).into_response()
}

fn fail_at(status: StatusCode, code: &str, message: impl Into<String>, field: &str) -> Response {
    (status, Json(json!({ "error": message.into(), "code": code, "field": field }))).into_response()
}

fn internal() -> Response {
    fail(StatusCode::INTERNAL_SERVER_ERROR, "internal", "something went wrong on the server")
}

async fn person(state: &AppState, headers: &HeaderMap) -> Result<(), Response> {
    super::mcp_suggest::refuse_agent_tokens(state, headers, "only the person can do this").await
}

/// The `?space=` of a create: 404 for an unknown space, **423** for an archived one (api.md §5;
/// the usual `Scope` answers 409).
async fn create_space(state: &AppState, uri: &Uri) -> Result<SpaceId, Response> {
    let Some(id) = space_param(uri.query()) else { return Ok(SpaceId::default_space()) };
    let id = id.trim();
    if id.is_empty() {
        return Err(fail(StatusCode::NOT_FOUND, "unknown_space", "no such space"));
    }
    match state.db.get_space(&SpaceId::new(id)).await {
        Ok(Some(s)) if s.archived_at.is_some() => Err(fail(StatusCode::LOCKED, "space_archived", "this space is archived; restore it first")),
        Ok(Some(s)) => Ok(SpaceId::new(s.id)),
        Ok(None) => Err(fail(StatusCode::NOT_FOUND, "unknown_space", "no such space")),
        Err(_) => Err(internal()),
    }
}

// ---------------------------------------------------------------------------------------------
// §1 The catalogue

/// `GET /api/team-templates?space=`: the three templates, plus whether this space already has a
/// team made from each. Agents may read it (Q-T9).
pub async fn catalogue(State(state): State<AppState>, Scope(space): Scope) -> Response {
    let mut out = Vec::new();
    for t in templates::all() {
        let Ok(used) = state.db.template_used_in_space(&space, &t.id).await else { return internal() };
        out.push(CatalogueEntry { template: t.catalogue(), used_in_space: used });
    }
    Json(json!({ "templates": out })).into_response()
}

// ---------------------------------------------------------------------------------------------
// §2 The preview

/// `GET /api/team-templates/:id/preview?space=`: the free handle and names for this space (T9).
pub async fn preview(State(state): State<AppState>, headers: HeaderMap, Scope(space): Scope, Path(id): Path<String>) -> Response {
    if let Err(r) = person(&state, &headers).await {
        return r;
    }
    let Some(t) = templates::get(&id) else {
        return fail(StatusCode::NOT_FOUND, "unknown_template", "no such template");
    };
    match free_names(&state, &space, t).await {
        Ok((handle, names, clashes)) => Json(json!({
            "template_id": t.id,
            "handle": handle,
            "handle_wanted": t.handle,
            "names": names,
            "clashes": clashes,
        }))
        .into_response(),
        Err(_) => internal(),
    }
}

async fn free_names(state: &AppState, space: &SpaceId, t: &Template) -> Result<(String, Map<String, Value>, Vec<Value>), crate::db::DbError> {
    let handle = state.db.free_team_handle(space, &t.handle).await?;
    let mut clashes = Vec::new();
    if handle != t.handle {
        clashes.push(json!({ "field": "handle", "wanted": t.handle, "suggestion": handle }));
    }
    let mut names = Map::new();
    let mut taken = HashSet::new();
    for b in &t.bots {
        let free = state.db.free_bot_name(space, &b.name, &taken).await?;
        taken.insert(free.to_lowercase());
        if free != b.name {
            clashes.push(json!({ "field": format!("names.{}", b.slot()), "wanted": b.name, "suggestion": free }));
        }
        names.insert(b.slot().to_string(), json!(free));
    }
    Ok((handle, names, clashes))
}

// ---------------------------------------------------------------------------------------------
// §3 The create

/// `POST /api/teams/from-template?space=`: the whole team in one transaction, then one logged
/// action for one Undo (T3, T4). From a template (3a) or a suggestion (3b).
pub async fn from_template(State(state): State<AppState>, headers: HeaderMap, uri: Uri, body: Bytes) -> Response {
    if let Err(r) = person(&state, &headers).await {
        return r;
    }
    let space = match create_space(&state, &uri).await {
        Ok(s) => s,
        Err(r) => return r,
    };
    let key = match idempotency_key(&headers) {
        Ok(k) => k,
        Err(r) => return r,
    };
    // A repeated key answers what the first request made, before anything else is checked.
    if let Some(k) = &key {
        match state.db.team_creation_by_key(&space, k).await {
            Ok(Some(c)) => return created_response(&state, &c).await,
            Ok(None) => {}
            Err(_) => return internal(),
        }
    }
    let Ok(Value::Object(req)) = serde_json::from_slice::<Value>(&body) else {
        return fail(StatusCode::BAD_REQUEST, "bad_request", "the body must be a JSON object");
    };
    for k in req.keys() {
        if !["template_id", "handle", "names", "suggestion_id", "edits"].contains(&k.as_str()) {
            return fail_at(StatusCode::BAD_REQUEST, "unknown_field", format!("unknown field {k}"), k);
        }
    }
    let template_id = match req.get("template_id") {
        None | Some(Value::Null) => None,
        Some(Value::String(s)) => Some(s.clone()),
        Some(_) => return fail_at(StatusCode::BAD_REQUEST, "bad_request", "template_id must be a string", "template_id"),
    };
    let suggestion_id = match req.get("suggestion_id") {
        None | Some(Value::Null) => None,
        Some(Value::String(s)) => Some(s.clone()),
        Some(_) => return fail_at(StatusCode::BAD_REQUEST, "bad_request", "suggestion_id must be a string", "suggestion_id"),
    };
    let tz = state.db.get_setting(super::bot_actions::TIMEZONE_SETTING).await.ok().flatten().filter(|t| !t.trim().is_empty()).unwrap_or_else(|| "UTC".into());

    let (plan, checked_out) = match (template_id, suggestion_id) {
        (None, None) => return fail(StatusCode::BAD_REQUEST, "bad_request", "send template_id or suggestion_id"),
        (Some(tid), None) => {
            if req.contains_key("edits") {
                return fail_at(StatusCode::BAD_REQUEST, "unknown_field", "edits only goes with suggestion_id", "edits");
            }
            let Some(t) = templates::get(&tid) else {
                return fail(StatusCode::NOT_FOUND, "unknown_template", "no such template");
            };
            match template_plan(t, &req, key.clone(), tz) {
                Ok(p) => (p, None),
                Err(r) => return r,
            }
        }
        (tid, Some(sid)) => {
            // T1: the Coding template with a project posts the suggestion; no other template does
            if tid.as_deref().is_some_and(|t| t != "coding") {
                return fail(StatusCode::BAD_REQUEST, "bad_request", "only the Coding template goes with a suggestion");
            }
            for k in ["handle", "names"] {
                if req.contains_key(k) {
                    return fail_at(StatusCode::BAD_REQUEST, "unknown_field", format!("{k} goes in edits for a suggestion"), k);
                }
            }
            let Some(s) = store_peek(&sid, &space) else {
                return fail(StatusCode::GONE, "suggestion_expired", "this suggestion has expired; read the project again");
            };
            let plan = match suggestion_plan(&state, &space, &s, req.get("edits"), tid.is_some(), key.clone(), tz).await {
                Ok(p) => p,
                Err(r) => return r,
            };
            // Used up by the create; put back if it fails (Q-D6c)
            let Some(s) = store_take(&sid, &space) else {
                return fail(StatusCode::GONE, "suggestion_expired", "this suggestion has expired; read the project again");
            };
            (plan, Some(s))
        }
    };

    // The team's folder never overlaps another space's (as `POST /api/teams`)
    if let Target::New { handle, workspace_path, repo_url, .. } = &plan.target {
        let probe = NewTeam {
            name: handle.clone(),
            description: None,
            icon: None,
            lead_kwant_id: String::new(),
            member_kwants: Vec::new(),
            workspace_path: workspace_path.clone(),
            repo_url: repo_url.clone(),
        };
        if let Err(r) = check_workspace(&state, &space, None, &probe).await {
            if let Some(s) = checked_out {
                store_put(s);
            }
            return r;
        }
    }

    let handle_field = if checked_out.is_some() { "edits.handle" } else { "handle" };
    let result = state.db.create_team_from_plan(&space, &plan, None).await;
    if result.is_err() {
        if let Some(s) = checked_out {
            store_put(s);
        }
    }
    match result {
        Ok(c) => {
            super::bot_actions::announce(&state, &c.action_id).await;
            super::dev::repo::reconcile_soon(&state, &space);
            tracing::info!(team_id = %c.team_id, action_id = %c.action_id, template = ?c.template_id, "the person made a ready-made team");
            created_response(&state, &c).await
        }
        Err(CreateError::Clash(clashes)) => clash_response(&clashes, handle_field),
        Err(CreateError::BadExisting { field }) => fail_at(StatusCode::BAD_REQUEST, "bad_existing", "that bot isn't in this space any more", &field),
        Err(CreateError::TeamNotFound) => fail(StatusCode::NOT_FOUND, "team_not_found", "the team is gone"),
        Err(CreateError::Db(e)) => {
            // Two requests with one key at once: the second's insert fails, and the first answers
            if let Some(k) = &key {
                if let Ok(Some(c)) = state.db.team_creation_by_key(&space, k).await {
                    return created_response(&state, &c).await;
                }
            }
            tracing::warn!(error = %e, "making a ready-made team failed; nothing was made");
            fail(StatusCode::INTERNAL_SERVER_ERROR, "create_failed", "the team couldn't be made, and nothing was made; try again")
        }
    }
}

fn idempotency_key(headers: &HeaderMap) -> Result<Option<String>, Response> {
    let Some(v) = headers.get("idempotency-key") else { return Ok(None) };
    let bad = || fail(StatusCode::BAD_REQUEST, "bad_request", "Idempotency-Key must be 1 to 200 visible characters");
    let s = v.to_str().map_err(|_| bad())?.trim();
    if s.is_empty() || s.len() > MAX_KEY_LEN || !s.bytes().all(|b| b.is_ascii_graphic()) {
        return Err(bad());
    }
    Ok(Some(s.to_string()))
}

/// The 201 body (api.md §3), the same for a first create and a repeated key.
async fn created_response(state: &AppState, c: &Creation) -> Response {
    let team = match state.db.get_team(&c.team_id).await {
        Ok(t) => serde_json::to_value(t).unwrap_or(Value::Null),
        Err(_) => return internal(),
    };
    let bots: Vec<Value> = c
        .created
        .bots
        .iter()
        .map(|b| json!({ "slot": b.slot, "id": b.id, "name": b.name, "preset": b.preset, "lead": b.lead, "existing": b.existing }))
        .collect();
    (
        StatusCode::CREATED,
        Json(json!({
            "team": team,
            "channel_id": c.created.channel_id,
            "action_id": c.action_id,
            "template_id": c.template_id,
            "bots": bots,
            "skills": c.created.skills,
            "routine": c.created.routine,
            "undo_until": c.undo_until(),
        })),
    )
        .into_response()
}

fn scope_list(items: &[String]) -> Vec<String> {
    items
        .iter()
        .take(templates::MAX_SCOPE_ITEMS)
        .map(|s| s.chars().take(templates::MAX_SCOPE_CHARS).collect())
        .collect()
}

/// 3a: a template with the person's handle and names; every other text is the template's.
#[allow(clippy::result_large_err)]
fn template_plan(t: &Template, req: &Map<String, Value>, key: Option<String>, tz: String) -> Result<Plan, Response> {
    let handle = match req.get("handle") {
        None | Some(Value::Null) => t.handle.clone(),
        Some(Value::String(h)) if names::valid_handle(h) => h.clone(),
        Some(_) => return Err(fail_at(StatusCode::BAD_REQUEST, "bad_handle", "a team name is lower-case letters, digits and -", "handle")),
    };
    let mut chosen: HashMap<String, String> = HashMap::new();
    match req.get("names") {
        None | Some(Value::Null) => {}
        Some(Value::Object(m)) => {
            for (slot, v) in m {
                let field = format!("names.{slot}");
                if t.bot(slot).is_none() {
                    return Err(fail_at(StatusCode::BAD_REQUEST, "unknown_field", format!("{slot} isn't a bot of this template"), &field));
                }
                match v {
                    Value::String(n) if names::valid_name(n) => {
                        chosen.insert(slot.clone(), n.clone());
                    }
                    _ => return Err(fail_at(StatusCode::BAD_REQUEST, "bad_name", "a bot name is 1 to 40 characters, no spaces or @", &field)),
                }
            }
        }
        Some(_) => return Err(fail_at(StatusCode::BAD_REQUEST, "bad_request", "names must be an object", "names")),
    }
    let bots = t
        .bots
        .iter()
        .map(|b| PlanBot {
            slot: b.slot().to_string(),
            field: format!("names.{}", b.slot()),
            preset: b.preset.clone(),
            lead: b.lead,
            name: chosen.get(b.slot()).cloned().unwrap_or_else(|| b.name.clone()),
            title: b.title.clone(),
            avatar: b.avatar_sprite.clone(),
            persona: b.persona.clone(),
            soul: b.soul.clone(),
            covers: scope_list(&b.good_for),
            avoids: scope_list(&b.not_for),
            existing: None,
        })
        .collect();
    Ok(Plan {
        template_id: Some(t.id.clone()),
        suggestion_id: None,
        idempotency_key: key,
        target: Target::New {
            handle,
            icon: t.icon.clone(),
            description: t.card_line.clone(),
            instructions: t.instructions.clone(),
            workspace_path: None,
            repo_url: None,
        },
        bots,
        skills: template_skills(t, |slot| Some(slot.to_string())),
        routine: t.routine.as_ref().map(|r| PlanRoutine { owner_slot: r.bot.clone(), name: r.name.clone(), instructions: r.prompt.clone(), cron: r.cron.clone() }),
        tz,
    })
}

/// A template's starter skills, each given to the plan slot `owner` maps its bot to.
fn template_skills(t: &Template, owner: impl Fn(&str) -> Option<String>) -> Vec<PlanSkill> {
    t.skills
        .iter()
        .filter_map(|s| {
            Some(PlanSkill { owner_slot: owner(&s.owner)?, name: s.name.clone(), description: s.description.clone(), body: s.body.clone() })
        })
        .collect()
}

/// One row's state after the person's edits.
struct Chosen<'a> {
    row: &'a Row,
    on: bool,
    name: Option<String>,
    preset: Option<Role>,
    existing: Option<String>,
}

/// 3b: a suggestion with the person's edits. Every persona is built here from the preset and the
/// server's own facts; the edits only pick rows, names, presets and existing bots (Q-D6a).
#[allow(clippy::too_many_arguments)]
async fn suggestion_plan(
    state: &AppState,
    space: &SpaceId,
    s: &Suggestion,
    edits: Option<&Value>,
    coding: bool,
    key: Option<String>,
    tz: String,
) -> Result<Plan, Response> {
    let bad = |code: &str, msg: &str, field: &str| fail_at(StatusCode::BAD_REQUEST, code, msg, field);
    let mut rows: Vec<Chosen> = s.bots.iter().map(|r| Chosen { row: r, on: true, name: None, preset: None, existing: None }).collect();
    rows.extend(s.also.iter().map(|r| Chosen { row: r, on: false, name: None, preset: None, existing: None }));
    let mut handle = s.handle.clone();

    let edits = match edits {
        None | Some(Value::Null) => Map::new(),
        Some(Value::Object(m)) => m.clone(),
        Some(_) => return Err(bad("bad_request", "edits must be an object", "edits")),
    };
    for (k, v) in &edits {
        match k.as_str() {
            "handle" if s.team_id.is_none() => match v {
                Value::String(h) if names::valid_handle(h) => handle = h.clone(),
                _ => return Err(bad("bad_handle", "a team name is lower-case letters, digits and -", "edits.handle")),
            },
            "rows" => {}
            _ => return Err(bad("unknown_field", &format!("unknown field {k}"), &format!("edits.{k}"))),
        }
    }
    let live: Vec<crate::db::models::Kwant> = state.db.list_kwants(space).await.map_err(|_| internal())?;
    match edits.get("rows") {
        None | Some(Value::Null) => {}
        Some(Value::Object(m)) => {
            for (rk, rv) in m {
                let at = format!("edits.rows.{rk}");
                let Some(c) = rows.iter_mut().find(|c| &c.row.key == rk) else {
                    return Err(bad("bad_row", "no such row in this suggestion", &at));
                };
                let Value::Object(fields) = rv else {
                    return Err(bad("bad_row", "a row's edits must be an object", &at));
                };
                for (f, fv) in fields {
                    let at = format!("{at}.{f}");
                    match (f.as_str(), fv) {
                        ("on", Value::Bool(on)) => {
                            if c.row.lead && !on {
                                return Err(bad("lead_required", "the team needs its lead", &at));
                            }
                            c.on = *on;
                        }
                        ("name", Value::String(n)) if names::valid_name(n) => c.name = Some(n.clone()),
                        ("name", _) => return Err(bad("bad_name", "a bot name is 1 to 40 characters, no spaces or @", &at)),
                        ("preset", _) if c.row.lead => return Err(bad("bad_row", "the lead keeps its preset", &at)),
                        ("preset", Value::String(p)) if Role::from_preset(p).is_some() => c.preset = Role::from_preset(p),
                        ("preset", _) => return Err(bad("bad_preset", "not one of the team presets", &at)),
                        ("use_existing", Value::Null) => c.existing = None,
                        ("use_existing", Value::String(id)) if live.iter().any(|k| &k.id == id) => c.existing = Some(id.clone()),
                        ("use_existing", _) => return Err(bad("bad_existing", "not a bot in this space", &at)),
                        ("on", _) => return Err(bad("bad_row", "on must be true or false", &at)),
                        _ => return Err(bad("unknown_field", &format!("unknown field {f}"), &at)),
                    }
                }
            }
        }
        Some(_) => return Err(bad("bad_request", "rows must be an object", "edits.rows")),
    }

    let on: Vec<&Chosen> = rows.iter().filter(|c| c.on).collect();
    if on.len() > discover::MAX_BOTS {
        return Err(bad("too_many_bots", "6 bots at most", "edits.rows"));
    }
    let least = if s.team_id.is_some() { 1 } else { discover::MIN_BOTS };
    if on.len() < least {
        return Err(bad("too_few_bots", &format!("pick at least {least}"), "edits.rows"));
    }
    let mut seen = HashSet::new();
    for c in &on {
        if let Some(id) = &c.existing {
            if !seen.insert(id.clone()) {
                return Err(bad("bad_existing", "one bot can fill one row", &format!("edits.rows.{}.use_existing", c.row.key)));
            }
        }
    }

    let coding_t = templates::get("coding").expect("the Coding template exists");
    let mut bots = Vec::new();
    for c in &on {
        let r = c.row;
        let role = c.preset.or_else(|| Role::from_preset(&r.preset)).unwrap_or(Role::Backend);
        let p = role.preset();
        let moved = c.preset.is_some_and(|p| p.preset_id() != r.preset);
        // A fallback row is the Coding template's bot, with its own persona and scope (D7)
        let tbot = (s.fallback && !moved).then(|| if r.lead { Some(coding_t.lead()) } else { coding_t.bot(&r.name) }).flatten();
        let (title, avatar) = if moved { (p.title.clone(), p.avatar_sprite.clone()) } else { (r.title.clone(), r.avatar_sprite.clone()) };
        let default_name = if moved { p.name.clone() } else { r.name.clone() };
        let (persona, soul, covers, avoids) = match tbot {
            Some(b) => (b.persona.clone(), b.soul.clone(), scope_list(&b.good_for), scope_list(&b.not_for)),
            None => (discover::persona(p, r.lead, s.stack_line.as_deref(), &r.owns), p.soul.clone(), Vec::new(), Vec::new()),
        };
        bots.push(PlanBot {
            slot: r.key.clone(),
            field: format!("edits.rows.{}.name", r.key),
            preset: p.id.clone(),
            lead: r.lead,
            name: c.name.clone().unwrap_or(default_name),
            title,
            avatar,
            persona,
            soul,
            covers,
            avoids,
            existing: c.existing.clone(),
        });
    }

    // T1: posted for the Coding template, the team gets its starter skills, each on the row that
    // took that bot's role (the lead's on the lead)
    let skills = if coding {
        template_skills(coding_t, |owner| {
            let tb = coding_t.bot(owner)?;
            bots.iter()
                .find(|b| if tb.lead { b.lead } else { !b.lead && b.preset == tb.preset })
                .filter(|b| b.existing.is_none())
                .map(|b| b.slot.clone())
        })
    } else {
        Vec::new()
    };

    let target = match &s.team_id {
        Some(team_id) => Target::Existing { team_id: team_id.clone() },
        None => Target::New {
            handle,
            icon: coding_t.icon.clone(),
            description: coding_t.card_line.clone(),
            instructions: discover::team_instructions(s.stack_line.as_deref()),
            workspace_path: s.workspace_path.clone(),
            repo_url: s.repo_url.clone(),
        },
    };
    Ok(Plan {
        template_id: coding.then(|| "coding".to_string()),
        suggestion_id: Some(s.id.clone()),
        idempotency_key: key,
        target,
        bots,
        skills,
        routine: None,
        tz,
    })
}

// ---------------------------------------------------------------------------------------------
// §4 Suggest

/// One answered Suggest, kept in memory for an hour (nothing on disk).
#[derive(Debug, Clone)]
pub struct Suggestion {
    pub id: String,
    pub space: String,
    pub team_id: Option<String>,
    pub workspace_path: Option<String>,
    pub repo_url: Option<String>,
    pub handle: String,
    pub stack_line: Option<String>,
    pub bots: Vec<Row>,
    pub also: Vec<Row>,
    pub fallback: bool,
    pub expires_at: DateTime<Utc>,
}

fn store() -> &'static Mutex<HashMap<String, Suggestion>> {
    static STORE: OnceLock<Mutex<HashMap<String, Suggestion>>> = OnceLock::new();
    STORE.get_or_init(|| Mutex::new(HashMap::new()))
}

fn store_put(s: Suggestion) {
    let mut m = store().lock().unwrap_or_else(|e| e.into_inner());
    let now = tt::now();
    m.retain(|_, v| v.expires_at > now);
    while m.len() >= MAX_SUGGESTIONS {
        let Some(oldest) = m.values().min_by_key(|v| v.expires_at).map(|v| v.id.clone()) else { break };
        m.remove(&oldest);
    }
    m.insert(s.id.clone(), s);
}

/// The live suggestion `id` of `space`, left in place.
fn store_peek(id: &str, space: &SpaceId) -> Option<Suggestion> {
    let m = store().lock().unwrap_or_else(|e| e.into_inner());
    m.get(id).filter(|s| s.space == space.as_str() && s.expires_at > tt::now()).cloned()
}

/// The live suggestion `id` of `space`, taken out: only one create can use it.
fn store_take(id: &str, space: &SpaceId) -> Option<Suggestion> {
    let mut m = store().lock().unwrap_or_else(|e| e.into_inner());
    let ok = m.get(id).is_some_and(|s| s.space == space.as_str() && s.expires_at > tt::now());
    if ok {
        m.remove(id)
    } else {
        None
    }
}

fn reading() -> &'static tokio::sync::Semaphore {
    static SEM: OnceLock<tokio::sync::Semaphore> = OnceLock::new();
    SEM.get_or_init(|| tokio::sync::Semaphore::new(MAX_READING))
}

fn new_suggestion_id() -> String {
    use ring::rand::SecureRandom;
    let mut b = [0u8; 16];
    if ring::rand::SystemRandom::new().fill(&mut b).is_err() {
        b = *uuid::Uuid::new_v4().as_bytes();
    }
    let hex: String = b.iter().map(|x| format!("{x:02x}")).collect();
    format!("sug_{hex}")
}

/// What Suggest reads.
enum Source {
    Folder(std::path::PathBuf),
    Remote(String),
}

/// What reading a project gave: the listing, the manifests read, and what stopped it early.
struct Read {
    listing: Listing,
    contents: HashMap<String, Vec<u8>>,
    stops: Stops,
}

/// `POST /api/teams/suggest?space=`: reads one folder, repo or team's project and suggests a
/// roster (D2–D8). No model call; nothing is made and nothing stays on disk.
pub async fn suggest(State(state): State<AppState>, headers: HeaderMap, Scope(space): Scope, body: Bytes) -> Response {
    if let Err(r) = person(&state, &headers).await {
        return r;
    }
    let Ok(Value::Object(req)) = serde_json::from_slice::<Value>(&body) else {
        return fail(StatusCode::BAD_REQUEST, "bad_request", "the body must be a JSON object");
    };
    for k in req.keys() {
        if !["workspace_path", "repo_url", "team_id"].contains(&k.as_str()) {
            return fail_at(StatusCode::BAD_REQUEST, "unknown_field", format!("unknown field {k}"), k);
        }
    }
    let given: Vec<(&str, &str)> =
        req.iter().filter_map(|(k, v)| match v { Value::Null => None, Value::String(s) => Some((k.as_str(), s.as_str())), _ => Some((k.as_str(), "")) }).collect();
    let [(kind, value)] = given.as_slice() else {
        return fail(StatusCode::BAD_REQUEST, "bad_request", "send exactly one of workspace_path, repo_url or team_id");
    };

    let mut team_id = None;
    let mut mode = Mode::New;
    let mut members: Vec<String> = Vec::new();
    let (raw, handle) = match *kind {
        "team_id" => {
            let team = match state.db.get_team(value).await {
                Ok(Some(t)) if t.space_id == space.as_str() => t,
                Ok(_) => return fail(StatusCode::NOT_FOUND, "team_not_found", "no such team in this space"),
                Err(_) => return internal(),
            };
            let src = team.workspace_path.clone().filter(|p| !p.trim().is_empty()).or(team.repo_url.clone().filter(|u| !u.trim().is_empty()));
            let Some(src) = src else {
                return fail(StatusCode::CONFLICT, "team_has_no_project", "this team has no folder or repo to read");
            };
            members = serde_json::from_str(&team.member_kwants).unwrap_or_default();
            if !team.lead_kwant_id.is_empty() && !members.contains(&team.lead_kwant_id) {
                members.insert(0, team.lead_kwant_id.clone());
            }
            let mut who = Vec::new();
            for id in &members {
                if let Ok(Some(k)) = state.db.get_kwant(id).await {
                    who.push((k.name, k.title));
                }
            }
            mode = Mode::AddTo { have: discover::roles_of(&who) };
            team_id = Some(team.id.clone());
            (src, team.name.clone())
        }
        _ => (value.to_string(), discover::default_handle(value)),
    };
    let from_folder = if *kind == "team_id" { raw.starts_with('/') } else { *kind == "workspace_path" };
    let source = if from_folder {
        match check_folder(&raw) {
            Some(p) => Source::Folder(p),
            None => return fail_at(StatusCode::BAD_REQUEST, "bad_path", "that isn't an absolute path to a folder kwant can read", "workspace_path"),
        }
    } else {
        match git::remote_url(&raw) {
            Some(u) => Source::Remote(u),
            None => return fail_at(StatusCode::BAD_REQUEST, "bad_repo_url", "Dev mode can't use this repo address", "repo_url"),
        }
    };

    let started = Instant::now();
    let Ok(_permit) = reading().acquire().await else { return internal() };
    let deadline = started + discover::budget();
    let read = match &source {
        Source::Folder(p) => read_folder(&state, p.clone(), deadline).await,
        Source::Remote(u) => read_remote(&state, &space, u, deadline).await,
    };
    let read = match read {
        Ok(r) => r,
        Err(r) => return r,
    };
    let ms = started.elapsed().as_millis() as u64;
    let found = discover::analyze(&read.listing, &read.contents, read.stops);
    let mut roster = discover::roster(&found, &mode);

    // D6: rows a bot of this space already fills (not one of the team's own)
    let live: Vec<(String, String, String)> = match state.db.list_kwants(&space).await {
        Ok(list) => list.into_iter().filter(|k| !members.contains(&k.id)).map(|k| (k.id, k.name, k.title)).collect(),
        Err(_) => return internal(),
    };
    {
        let mut rows: Vec<&mut Row> = roster.bots.iter_mut().chain(roster.also_possible.iter_mut()).collect();
        discover::match_existing(&mut rows, &live);
    }

    let (workspace_path, repo_url) = match (&source, team_id.is_some()) {
        (_, true) => (None, None),
        (Source::Folder(p), false) => (Some(p.to_string_lossy().into_owned()), None),
        (Source::Remote(u), false) => (None, Some(u.clone())),
    };
    let s = Suggestion {
        id: new_suggestion_id(),
        space: space.as_str().to_string(),
        team_id: team_id.clone(),
        workspace_path,
        repo_url,
        handle,
        stack_line: found.stack_line.clone(),
        bots: roster.bots.clone(),
        also: roster.also_possible.clone(),
        fallback: roster.fallback,
        expires_at: tt::now() + ChronoDuration::minutes(SUGGESTION_MINUTES),
    };
    let body = json!({
        "suggestion_id": s.id,
        "expires_at": s.expires_at,
        "team_id": team_id,
        "handle": s.handle,
        "facts": if roster.fallback { Vec::new() } else { found.facts.clone() },
        "stack_line": if roster.fallback { None } else { found.stack_line.clone() },
        "bots": roster.bots,
        "also_possible": roster.also_possible,
        "read": { "files": found.files, "manifests": found.manifests, "ms": ms },
        "partial": found.limits.first(),
        "limits": found.limits,
        "fallback": roster.fallback,
    });
    store_put(s);
    Json(body).into_response()
}

/// An absolute path to a real folder (not a symlink), normalised; `None` otherwise.
fn check_folder(raw: &str) -> Option<std::path::PathBuf> {
    let p = std::path::Path::new(raw.trim());
    if !p.is_absolute() || raw.contains('\0') || p.components().any(|c| matches!(c, std::path::Component::ParentDir)) {
        return None;
    }
    let m = std::fs::symlink_metadata(p).ok()?;
    m.file_type().is_dir().then(|| p.to_path_buf())
}

fn read_failed() -> Response {
    fail(StatusCode::BAD_GATEWAY, "read_failed", "couldn't read this project")
}

fn left(deadline: Instant) -> Duration {
    deadline.saturating_duration_since(Instant::now()).max(Duration::from_millis(1))
}

/// A folder: its git's tracked files when it has a readable `.git`, else a walk of the folder.
async fn read_folder(state: &AppState, folder: std::path::PathBuf, deadline: Instant) -> Result<Read, Response> {
    if folder.join(".git").exists() && git::available().await {
        if let Ok(repo) = Repo::local(&folder, &state.config.data_dir).await {
            match read_git(&repo, deadline).await {
                Ok(Some(r)) => return Ok(r),
                Ok(None) => {} // no commits yet: walk the folder
                Err(GitError::Timeout) => {
                    return Ok(Read { listing: Listing { time: true, ..Default::default() }, contents: HashMap::new(), stops: Stops { time: true, ..Default::default() } })
                }
                Err(e) => tracing::debug!(error = %e, "suggest: the folder's git didn't answer; walking it"),
            }
        }
    }
    tokio::task::spawn_blocking(move || {
        let listing = discover::walk(&folder, deadline);
        let picks = discover::pick_reads(&listing.entries);
        let mut stops = Stops { bytes: picks.bytes, too_big: picks.too_big, ..Default::default() };
        let mut contents = HashMap::new();
        for i in picks.read {
            if Instant::now() >= deadline {
                stops.time = true;
                break;
            }
            let e = &listing.entries[i];
            match discover::read_file(&folder, &e.path, discover::MAX_FILE_BYTES) {
                Ok(b) => {
                    contents.insert(e.path.clone(), b);
                }
                Err(discover::ReadError::TooBig) => stops.too_big.push(e.path.clone()),
                Err(discover::ReadError::Unreadable) => {}
            }
        }
        Read { listing, contents, stops }
    })
    .await
    .map_err(|_| read_failed())
}

/// The tree at `HEAD` and the manifests' blobs; `None` for a repo without commits.
async fn read_git(repo: &Repo, deadline: Instant) -> Result<Option<Read>, GitError> {
    let head = match repo.head().await {
        Ok(h) => h,
        Err(GitError::NotFound) => return Ok(None),
        Err(e) => return Err(e),
    };
    read_rev(repo, &head, deadline).await.map(Some)
}

async fn read_rev(repo: &Repo, rev: &str, deadline: Instant) -> Result<Read, GitError> {
    let (out, cut) = repo.tree(rev, MAX_TREE_BYTES, left(deadline)).await?;
    let mut entries = discover::parse_tree(&out);
    let cut = cut || entries.len() > discover::MAX_LISTED;
    entries.truncate(discover::MAX_LISTED);
    let listing = Listing { entries, cut, time: false };
    let picks = discover::pick_reads(&listing.entries);
    let mut stops = Stops { bytes: picks.bytes, too_big: picks.too_big, ..Default::default() };
    let mut contents = HashMap::new();
    for i in picks.read {
        if Instant::now() >= deadline {
            stops.time = true;
            break;
        }
        let e = &listing.entries[i];
        let Some(id) = &e.blob else { continue };
        match repo.blob(id, discover::MAX_FILE_BYTES as usize, left(deadline)).await {
            Ok(b) => {
                contents.insert(e.path.clone(), b);
            }
            Err(GitError::TooBig) => stops.too_big.push(e.path.clone()),
            Err(GitError::Timeout) => {
                stops.time = true;
                break;
            }
            Err(_) => {}
        }
    }
    Ok(Read { listing, contents, stops })
}

/// A repo link: the tip of its default branch into a throwaway mirror, read, then deleted.
async fn read_remote(state: &AppState, space: &SpaceId, url: &str, deadline: Instant) -> Result<Read, Response> {
    if !git::available().await {
        return Err(fail(StatusCode::BAD_GATEWAY, "read_failed", GitError::NoGit.to_string()));
    }
    let host = super::dev::repo::parse_remote(url).map(|(h, _)| h);
    let forge = host.as_deref().map(|h| if h == "github.com" { Some("github") } else if h.contains("gitlab") { Some("gitlab") } else { None }).unwrap_or(None);
    let resolver: Arc<dyn git::Resolver> = Arc::new(git::SystemResolver);
    let client = match host.as_deref() {
        Some(h) => super::dev::forge::client_for_host(state, h, resolver).await,
        None => None,
    };
    let auth = client.as_ref().and_then(|c| c.git_auth(url));
    let key = format!("suggest-{}", &new_suggestion_id()[4..20]);
    let data_dir = state.config.data_dir.clone();
    let result = async {
        let repo = Repo::mirror(&data_dir, space.as_str(), &key).await?;
        let branch = repo.default_branch(url, auth.as_ref()).await?;
        let tip = repo.fetch_tip(url, &branch, auth.as_ref(), left(deadline)).await?;
        read_rev(&repo, &tip, deadline).await
    }
    .await;
    git::remove_mirror(&data_dir, space.as_str(), &key).await;
    result.map_err(|e| match e {
        GitError::Auth | GitError::NotFound => (
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(json!({ "error": "kwant can't read this repo; connect its host first if it's private", "code": "cant_read", "forge": forge })),
        )
            .into_response(),
        GitError::Refused => fail_at(StatusCode::BAD_REQUEST, "bad_repo_url", e.to_string(), "repo_url"),
        other => fail(StatusCode::BAD_GATEWAY, "read_failed", other.to_string()),
    })
}

// ---------------------------------------------------------------------------------------------
// §6 The Undo preview

/// `GET /api/teams/:id/template`: what Undo of the team's create would remove, and whether it can.
pub async fn team_template(State(state): State<AppState>, headers: HeaderMap, Path(id): Path<String>) -> Response {
    if let Err(r) = person(&state, &headers).await {
        return r;
    }
    let c = match state.db.team_creation_for_team(&id).await {
        Ok(Some(c)) => c,
        Ok(None) => return fail(StatusCode::NOT_FOUND, "not_a_template_team", "this team wasn't made from a template or a suggestion"),
        Err(_) => return internal(),
    };
    match state.db.team_template_info(&c).await {
        Ok(info) => Json(info).into_response(),
        Err(_) => internal(),
    }
}

/// 409 for taken names or handles. `code`, `field` and `suggestion` describe the first clash (the
/// handle first) as api.md §5 always did; `clashes[]` lists every one, in the preview's shape.
fn clash_response(clashes: &[Clash], handle_field: &str) -> Response {
    let list: Vec<Value> = clashes
        .iter()
        .map(|c| {
            let field = if c.kind == ClashKind::Handle { handle_field.to_string() } else { c.field.clone() };
            json!({ "field": field, "wanted": c.wanted, "suggestion": c.suggestion })
        })
        .collect();
    let first = &clashes[0];
    let (code, error) = match first.kind {
        ClashKind::Handle => ("handle_taken", format!("that team name is taken; {} is free", first.suggestion)),
        ClashKind::Name => ("name_taken", format!("that name is taken; {} is free", first.suggestion)),
    };
    (
        StatusCode::CONFLICT,
        Json(json!({ "error": error, "code": code, "field": list[0]["field"], "suggestion": first.suggestion, "clashes": list })),
    )
        .into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn suggestion_ids_are_128_bit_and_distinct() {
        let a = new_suggestion_id();
        let b = new_suggestion_id();
        assert!(a.starts_with("sug_") && a.len() == 4 + 32, "{a}");
        assert_ne!(a, b);
    }

    #[test]
    fn folders_must_be_absolute_real_directories() {
        let dir = tempfile::tempdir().unwrap();
        assert!(check_folder(dir.path().to_str().unwrap()).is_some());
        assert!(check_folder("relative/path").is_none());
        assert!(check_folder(&format!("{}/../x", dir.path().display())).is_none());
        let file = dir.path().join("f");
        std::fs::write(&file, "x").unwrap();
        assert!(check_folder(file.to_str().unwrap()).is_none());
        #[cfg(unix)]
        {
            let link = dir.path().join("l");
            std::os::unix::fs::symlink(dir.path(), &link).unwrap();
            assert!(check_folder(link.to_str().unwrap()).is_none());
        }
    }

    fn sample(id: &str, space: &str, mins: i64) -> Suggestion {
        Suggestion {
            id: id.into(),
            space: space.into(),
            team_id: None,
            workspace_path: None,
            repo_url: None,
            handle: "shop".into(),
            stack_line: None,
            bots: Vec::new(),
            also: Vec::new(),
            fallback: false,
            expires_at: tt::now() + ChronoDuration::minutes(mins),
        }
    }

    #[test]
    fn a_suggestion_is_bound_to_its_space_and_used_once() {
        let s = SpaceId::new("spc_store_test");
        store_put(sample("sug_store_a", s.as_str(), 60));
        assert!(store_peek("sug_store_a", &SpaceId::new("spc_other")).is_none());
        assert!(store_take("sug_store_a", &SpaceId::new("spc_other")).is_none());
        assert!(store_peek("sug_store_a", &s).is_some());
        assert!(store_take("sug_store_a", &s).is_some());
        assert!(store_take("sug_store_a", &s).is_none());
        store_put(sample("sug_store_old", s.as_str(), -1));
        assert!(store_peek("sug_store_old", &s).is_none());
    }

    #[test]
    fn idempotency_keys_are_checked() {
        let mut h = HeaderMap::new();
        assert_eq!(idempotency_key(&h).unwrap(), None);
        h.insert("idempotency-key", "abc-123".parse().unwrap());
        assert_eq!(idempotency_key(&h).unwrap().as_deref(), Some("abc-123"));
        h.insert("idempotency-key", "a b".parse().unwrap());
        assert!(idempotency_key(&h).is_err());
        h.insert("idempotency-key", "x".repeat(201).parse().unwrap());
        assert!(idempotency_key(&h).is_err());
    }
}

