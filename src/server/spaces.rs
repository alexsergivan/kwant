//! Spaces through the API (S2): `GET/POST /api/spaces`, `GET/PATCH /api/spaces/:id`, and the
//! [`Scope`] extractor every collection endpoint takes its space from (`?space=<id>`).
//!
//! A missing `?space=` means the Default space. An unknown id is a 404 and an archived one a
//! 409: a request never falls back to Default. Item endpoints (`/api/kwants/:id`, …) take
//! their space from the row; [`row_guard`] refuses one whose explicit `?space=` names another
//! space (404) and any change to a row of an archived space (409).
#![allow(clippy::result_large_err)]

use super::AppState;
use crate::db::spaces::{NewSpace, Space, SpaceId, SpaceUpdate};
use crate::db::DbError;
use axum::{
    async_trait,
    extract::{FromRequestParts, Path, Query, State},
    http::{request::Parts, StatusCode},
    response::{IntoResponse, Response},
    Json,
};
use serde::{Deserialize, Deserializer};
use serde_json::{json, Value};
use std::path::PathBuf;

const MAX_NAME_CHARS: usize = 60;
const MAX_ICON_CHARS: usize = 16;

fn error(status: StatusCode, message: impl Into<String>) -> Response {
    (status, Json(json!({ "error": message.into() }))).into_response()
}

fn internal() -> Response {
    StatusCode::INTERNAL_SERVER_ERROR.into_response()
}

/// The space a collection request works in, from `?space=<id>`; Default when there is none.
#[derive(Debug, Clone)]
pub struct Scope(pub SpaceId);

#[async_trait]
impl FromRequestParts<AppState> for Scope {
    type Rejection = Response;

    async fn from_request_parts(parts: &mut Parts, state: &AppState) -> Result<Self, Response> {
        match space_param(parts.uri.query()) {
            None => Ok(Scope(SpaceId::default_space())),
            Some(id) => active_space(state, &id).await.map(Scope),
        }
    }
}

/// The `space` value of a raw query string. Parsed on its own so handlers' own query structs
/// don't have to know about it.
pub fn space_param(query: Option<&str>) -> Option<String> {
    url::form_urlencoded::parse(query.unwrap_or("").as_bytes())
        .find(|(k, _)| k == "space")
        .map(|(_, v)| v.into_owned())
}

/// The space `id` names, if it exists and isn't archived: 404 for an unknown (or empty) id, 409
/// for an archived one.
pub async fn active_space(state: &AppState, id: &str) -> Result<SpaceId, Response> {
    let id = id.trim();
    if id.is_empty() {
        return Err(error(StatusCode::NOT_FOUND, "no such space"));
    }
    match state.db.get_space(&SpaceId::new(id)).await {
        Ok(Some(s)) if s.archived_at.is_some() => Err(archived()),
        Ok(Some(s)) => Ok(SpaceId::new(s.id)),
        Ok(None) => Err(error(StatusCode::NOT_FOUND, "no such space")),
        Err(_) => Err(internal()),
    }
}

/// 409 when `space` is archived: nothing new starts there until it is restored.
pub async fn ensure_active(state: &AppState, space: &SpaceId) -> Result<(), Response> {
    if space.is_default() {
        return Ok(());
    }
    match state.db.get_space(space).await {
        Ok(Some(s)) if s.archived_at.is_some() => Err(archived()),
        Ok(_) => Ok(()),
        Err(_) => Err(internal()),
    }
}

/// The kinds of row an item route names, by the path segments before its id.
const ITEM_ROUTES: &[(&[&str], RowKind)] = &[
    (&["kwants"], RowKind::Kwant),
    (&["channels"], RowKind::Channel),
    (&["teams"], RowKind::Team),
    (&["sandbox"], RowKind::Team),
    (&["routines"], RowKind::Routine),
    (&["skills"], RowKind::Skill),
    (&["mcp", "servers"], RowKind::McpServer),
    (&["profiles"], RowKind::Profile),
    (&["questions"], RowKind::Question),
    (&["threads"], RowKind::Thread),
    (&["delegations"], RowKind::Delegation),
    (&["approvals"], RowKind::Approval),
];

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum RowKind {
    Kwant,
    Channel,
    Team,
    Routine,
    Skill,
    McpServer,
    Profile,
    Question,
    Thread,
    Delegation,
    Approval,
}

/// The kind and id of the row an `/api/...` path names, if it is an item route.
pub fn item_route(path: &str) -> Option<(RowKind, String)> {
    let segs: Vec<&str> = path.strip_prefix("/api/")?.split('/').collect();
    ITEM_ROUTES.iter().find_map(|(prefix, kind)| {
        let id = segs.get(prefix.len()).filter(|id| !id.is_empty())?;
        (segs[..prefix.len()] == **prefix)
            .then(|| (*kind, percent_encoding::percent_decode_str(id).decode_utf8_lossy().into_owned()))
    })
}

async fn space_of_row(state: &AppState, kind: RowKind, id: &str) -> Result<Option<SpaceId>, DbError> {
    let db = &state.db;
    match kind {
        RowKind::Kwant => db.space_of_kwant(id).await,
        RowKind::Channel => db.space_of_channel(id).await,
        RowKind::Team => db.space_of_team(id).await,
        RowKind::Routine => db.space_of_routine(id).await,
        RowKind::Skill => db.space_of_skill(id).await,
        RowKind::McpServer => db.space_of_mcp_server(id).await,
        RowKind::Profile => db.space_of_profile(id).await,
        RowKind::Question => db.space_of_question(id).await,
        RowKind::Thread => db.space_of_thread(id).await,
        RowKind::Delegation => db.space_of_delegation(id).await,
        RowKind::Approval => db.space_of_approval(id).await,
    }
}

/// Middleware for every item route (see [`ITEM_ROUTES`]). A request with an explicit `?space=`
/// whose row lives in another space gets a 404, so a screen still showing the old space after a
/// switch can't read or change the wrong one; without `?space=` the row decides, as before. Any
/// change (not GET/HEAD) to a row of an archived space is a 409. Unknown rows pass through to the
/// handler, which answers as it always did.
pub async fn row_guard(
    State(state): State<AppState>,
    req: axum::extract::Request,
    next: axum::middleware::Next,
) -> Response {
    let Some((kind, id)) = item_route(req.uri().path()) else {
        return next.run(req).await;
    };
    let row_space = match space_of_row(&state, kind, &id).await {
        Ok(Some(space)) => space,
        Ok(None) => return next.run(req).await,
        Err(_) => return internal(),
    };
    if let Some(want) = space_param(req.uri().query()) {
        if want.trim() != row_space.as_str() {
            return error(StatusCode::NOT_FOUND, "that isn't in this space");
        }
    }
    let read = matches!(*req.method(), axum::http::Method::GET | axum::http::Method::HEAD);
    if !read {
        if let Err(r) = ensure_active(&state, &row_space).await {
            return r;
        }
    }
    next.run(req).await
}

fn archived() -> Response {
    error(StatusCode::CONFLICT, "this space is archived; restore it first")
}

/// One space as the API shows it, with what the space switcher needs.
pub async fn view(state: &AppState, s: Space) -> Result<Value, DbError> {
    let id = SpaceId::new(&s.id);
    let (bots, paused) = state.db.space_counts(&id).await?;
    let ready = super::profile_turn::ready_accounts(state, &id).await;
    let unread = super::away::unread_total(state, &id).await?;
    let general = state.db.general_channel(&id).await?.map(|c| c.id);
    Ok(json!({
        "id": s.id,
        "name": s.name,
        "icon": s.icon,
        "color": s.color,
        "sort": s.sort,
        "is_default": id.is_default(),
        "archived_at": s.archived_at,
        "created_at": s.created_at,
        "general_channel_id": general,
        "counts": { "bots": bots, "ready_accounts": ready, "unread": unread },
        "paused_routines": paused,
    }))
}

#[derive(Debug, Deserialize)]
pub struct ListQuery {
    pub archived: Option<String>,
    /// Ignored here: listing spaces isn't scoped to one.
    #[allow(dead_code)]
    pub space: Option<String>,
}

/// `GET /api/spaces`: active spaces in their order; `?archived=1` lists the archived ones.
pub async fn list(State(state): State<AppState>, Query(q): Query<ListQuery>) -> Response {
    let archived = matches!(q.archived.as_deref(), Some("1" | "true"));
    let Ok(spaces) = state.db.list_spaces().await else {
        return internal();
    };
    let mut out = Vec::new();
    for s in spaces.into_iter().filter(|s| s.archived_at.is_some() == archived) {
        match view(&state, s).await {
            Ok(v) => out.push(v),
            Err(_) => return internal(),
        }
    }
    Json(out).into_response()
}

/// `GET /api/spaces/:id`, archived or not.
pub async fn get(State(state): State<AppState>, Path(id): Path<String>) -> Response {
    match state.db.get_space(&SpaceId::new(id)).await {
        Ok(Some(s)) => respond(&state, s, StatusCode::OK).await,
        Ok(None) => error(StatusCode::NOT_FOUND, "no such space"),
        Err(_) => internal(),
    }
}

async fn respond(state: &AppState, s: Space, status: StatusCode) -> Response {
    match view(state, s).await {
        Ok(v) => (status, Json(v)).into_response(),
        Err(_) => internal(),
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CreateBody {
    pub name: String,
    pub icon: Option<String>,
    pub color: Option<String>,
    /// This machine's switch in the new space, for every engine. Default on.
    pub this_machine_enabled: Option<bool>,
}

fn check_name(name: &str) -> Result<String, Response> {
    let name = name.trim();
    if name.is_empty() {
        return Err(error(StatusCode::BAD_REQUEST, "a space needs a name"));
    }
    if name.chars().count() > MAX_NAME_CHARS {
        return Err(error(StatusCode::BAD_REQUEST, format!("a space name is at most {MAX_NAME_CHARS} characters")));
    }
    Ok(name.to_string())
}

fn check_icon(icon: &str) -> Result<String, Response> {
    let icon = icon.trim();
    if icon.is_empty() || icon.chars().count() > MAX_ICON_CHARS {
        return Err(error(StatusCode::BAD_REQUEST, format!("an icon is 1 to {MAX_ICON_CHARS} characters")));
    }
    Ok(icon.to_string())
}

/// 409 when another space already has `name` (names are unique ignoring ASCII case).
async fn name_free(state: &AppState, name: &str, except: Option<&str>) -> Result<(), Response> {
    let Ok(spaces) = state.db.list_spaces().await else {
        return Err(internal());
    };
    if spaces.iter().any(|s| Some(s.id.as_str()) != except && s.name.eq_ignore_ascii_case(name)) {
        return Err(error(StatusCode::CONFLICT, "another space already has that name"));
    }
    Ok(())
}

fn unique_violation(e: &DbError) -> bool {
    matches!(e, DbError::Sqlx(sqlx::Error::Database(d)) if d.is_unique_violation())
}

/// `POST /api/spaces`: a new, empty space with its own This machine accounts and work folder.
/// Its #general appears with its first bot (a channel needs a lead).
pub async fn create(State(state): State<AppState>, Json(body): Json<CreateBody>) -> Response {
    let name = match check_name(&body.name) {
        Ok(n) => n,
        Err(r) => return r,
    };
    let icon = match body.icon.as_deref().map(check_icon).transpose() {
        Ok(i) => i.unwrap_or_else(|| "box".into()),
        Err(r) => return r,
    };
    if let Err(r) = name_free(&state, &name, None).await {
        return r;
    }
    let input = NewSpace {
        name,
        icon,
        color: body.color.map(|c| c.trim().to_string()).filter(|c| !c.is_empty()),
        this_machine_enabled: body.this_machine_enabled.unwrap_or(true),
    };
    let space = match state.db.create_space(&input).await {
        Ok(s) => s,
        Err(e) if unique_violation(&e) => return error(StatusCode::CONFLICT, "another space already has that name"),
        Err(_) => return internal(),
    };
    let dir = crate::db::spaces::space_data_dir(&state.config.data_dir, &SpaceId::new(&space.id));
    if let Err(e) = std::fs::create_dir_all(&dir) {
        // Team folders are also made on first use, so the space still works
        tracing::warn!(error = %e, space = %space.id, "could not create the space's work folder");
    }
    tracing::info!(space = %space.id, "space created");
    respond(&state, space, StatusCode::CREATED).await
}

/// `Some(None)` for an explicit null, so a PATCH can clear a field.
fn present<'de, D: Deserializer<'de>, T: Deserialize<'de>>(d: D) -> Result<Option<Option<T>>, D::Error> {
    Option::<T>::deserialize(d).map(Some)
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PatchBody {
    pub name: Option<String>,
    pub icon: Option<String>,
    #[serde(default, deserialize_with = "present")]
    pub color: Option<Option<String>>,
    pub sort: Option<i64>,
    /// `true` archives, `false` restores.
    pub archived: Option<bool>,
}

/// `PATCH /api/spaces/:id`: rename, restyle, reorder, archive or restore. Archiving pauses the
/// space's routines and stops its running turns; restoring brings back only the routines the
/// archive paused. The Default space can be renamed but not archived (409). No hard delete.
pub async fn update(State(state): State<AppState>, Path(id): Path<String>, Json(body): Json<PatchBody>) -> Response {
    let id = SpaceId::new(id);
    let current = match state.db.get_space(&id).await {
        Ok(Some(s)) => s,
        Ok(None) => return error(StatusCode::NOT_FOUND, "no such space"),
        Err(_) => return internal(),
    };
    let name = match body.name.as_deref().map(check_name).transpose() {
        Ok(n) => n,
        Err(r) => return r,
    };
    let icon = match body.icon.as_deref().map(check_icon).transpose() {
        Ok(i) => i,
        Err(r) => return r,
    };
    if body.archived == Some(true) && id.is_default() {
        return error(StatusCode::CONFLICT, "the Default space can't be archived");
    }
    if let Some(n) = &name {
        if let Err(r) = name_free(&state, n, Some(id.as_str())).await {
            return r;
        }
    }
    let change = SpaceUpdate {
        name,
        icon,
        color: body.color.map(|c| c.map(|c| c.trim().to_string()).filter(|c| !c.is_empty())),
        sort: body.sort,
    };
    match state.db.update_space(&id, &change).await {
        Ok(_) => {}
        Err(e) if unique_violation(&e) => return error(StatusCode::CONFLICT, "another space already has that name"),
        Err(_) => return internal(),
    }
    match (body.archived, current.archived_at.is_some()) {
        (Some(true), false) => {
            let paused = match state.db.archive_space(&id).await {
                Ok(n) => n,
                Err(e @ DbError::Conflict(_)) => return error(StatusCode::CONFLICT, e.to_string()),
                Err(_) => return internal(),
            };
            let stopped = stop_turns(&state, &id).await;
            state.scheduler_wake.notify_one();
            tracing::info!(space = %id, paused, stopped, "space archived");
        }
        (Some(false), true) => {
            let resumed = match state.db.restore_space(&id).await {
                Ok(n) => n,
                Err(_) => return internal(),
            };
            super::scheduler::reschedule_all(&state, chrono::Utc::now()).await;
            state.scheduler_wake.notify_one();
            tracing::info!(space = %id, resumed, "space restored");
        }
        _ => {}
    }
    get(State(state), Path(id.as_str().to_string())).await
}

/// Cancels every running turn of the space's bots, channels and teams. Returns how many jobs.
pub async fn stop_turns(state: &AppState, space: &SpaceId) -> usize {
    let mut keys: Vec<String> = Vec::new();
    keys.extend(state.db.list_kwants(space).await.unwrap_or_default().into_iter().map(|k| k.id));
    keys.extend(state.db.list_channels(space).await.unwrap_or_default().into_iter().map(|c| c.id));
    keys.extend(state.db.list_teams(space).await.unwrap_or_default().into_iter().map(|t| t.id));
    keys.iter().map(|k| state.turns.cancel(Some(k.as_str()))).sum()
}

/// Where a space's own files live, for the overlap check: Default's work folders in the data dir
/// (the data dir itself also holds every other space), another space's whole folder.
fn work_roots(data_dir: &std::path::Path, space: &SpaceId) -> Vec<PathBuf> {
    if space.is_default() {
        vec![data_dir.join("workspace"), data_dir.join("workspaces")]
    } else {
        vec![crate::db::spaces::space_data_dir(data_dir, space)]
    }
}

/// A team's folder as `resolve_or_prepare_workspace` would pick it, made absolute.
fn team_folder(data_dir: &std::path::Path, space: &SpaceId, name: &str, path: Option<&str>, repo: Option<&str>) -> PathBuf {
    let base = crate::db::spaces::space_data_dir(data_dir, space);
    let raw = match path.map(str::trim).filter(|p| !p.is_empty()) {
        Some(p) => match (p.strip_prefix("~/"), std::env::var("HOME")) {
            (Some(rest), Ok(home)) => PathBuf::from(home).join(rest),
            _ => PathBuf::from(p),
        },
        None if repo.is_some_and(|r| !r.trim().is_empty()) => base.join("workspaces").join(name),
        None => base.join("workspace"),
    };
    normalize(&raw)
}

/// Absolute, with `.` and `..` resolved and symlinks followed as far as the path exists.
pub fn normalize(path: &std::path::Path) -> PathBuf {
    use std::path::Component;
    let abs = match path.is_absolute() {
        true => path.to_path_buf(),
        false => std::env::current_dir().unwrap_or_default().join(path),
    };
    let mut clean = PathBuf::new();
    for c in abs.components() {
        match c {
            Component::CurDir => {}
            Component::ParentDir => {
                clean.pop();
            }
            other => clean.push(other),
        }
    }
    // Follow symlinks in the part that exists, then add back the rest
    let mut existing = clean.clone();
    let mut rest = Vec::new();
    while !existing.exists() {
        match (existing.file_name().map(|n| n.to_os_string()), existing.parent()) {
            (Some(name), Some(parent)) => {
                rest.push(name);
                existing = parent.to_path_buf();
            }
            _ => return clean,
        }
    }
    let mut out = std::fs::canonicalize(&existing).unwrap_or(existing);
    out.extend(rest.into_iter().rev());
    out
}

/// Whether one folder is, holds or sits in the other. Compared without case, since macOS's
/// default file system ignores it; on Linux that refuses a few folders that only differ in case.
fn overlaps(a: &std::path::Path, b: &std::path::Path) -> bool {
    let fold = |p: &std::path::Path| PathBuf::from(p.to_string_lossy().to_lowercase());
    let (a, b) = (fold(a), fold(b));
    a.starts_with(&b) || b.starts_with(&a)
}

/// Why `name` can't name a team: its folder (when it has no `workspace_path`) is made from the
/// name, so a separator, `..` or a leading dot could put it anywhere.
pub fn bad_team_name(name: &str) -> Option<&'static str> {
    let name = name.trim();
    if name.contains(['/', '\\', '\0']) {
        Some("a team name can't contain / or \\")
    } else if name.contains("..") || name.starts_with('.') {
        Some("a team name can't start with a dot or contain ..")
    } else {
        None
    }
}

/// 400 when a team's folder would share files with another space: the same folder as, or one
/// inside or around, another space's work folder or one of its teams'. The folder is the one the
/// team will really use, from its `workspace_path` or else its name and repo. A new or renamed
/// team also needs a name that is safe as a folder name.
pub async fn check_workspace(
    state: &AppState,
    space: &SpaceId,
    team_id: Option<&str>,
    team: &crate::db::models::NewTeam,
) -> Result<(), Response> {
    let renamed = match team_id {
        None => true,
        Some(id) => match state.db.get_team(id).await {
            Ok(Some(old)) => old.name != team.name,
            Ok(None) => true,
            Err(_) => return Err(internal()),
        },
    };
    if renamed {
        if let Some(why) = bad_team_name(&team.name) {
            return Err(error(StatusCode::BAD_REQUEST, why));
        }
    }
    let data_dir = normalize(&state.config.data_dir);
    let mine = team_folder(&data_dir, space, &team.name, team.workspace_path.as_deref(), team.repo_url.as_deref());
    let Ok(spaces) = state.db.list_spaces().await else {
        return Err(internal());
    };
    for other in spaces.iter().map(|s| SpaceId::new(&s.id)).filter(|s| s != space) {
        if work_roots(&data_dir, &other).iter().any(|root| overlaps(&mine, &normalize(root))) {
            return Err(collision(&other, &spaces, "its work folder"));
        }
        let Ok(teams) = state.db.list_teams(&other).await else {
            return Err(internal());
        };
        for t in teams.iter().filter(|t| Some(t.id.as_str()) != team_id) {
            let theirs = team_folder(&data_dir, &other, &t.name, t.workspace_path.as_deref(), t.repo_url.as_deref());
            if overlaps(&mine, &theirs) {
                return Err(collision(&other, &spaces, &format!("team {}", t.name)));
            }
        }
    }
    Ok(())
}

fn collision(other: &SpaceId, spaces: &[Space], what: &str) -> Response {
    let name = spaces.iter().find(|s| s.id == other.as_str()).map(|s| s.name.as_str()).unwrap_or("another space");
    error(
        StatusCode::BAD_REQUEST,
        format!("that folder overlaps {what} in the space \"{name}\"; spaces never share a folder"),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_space_parameter_is_read_from_the_raw_query() {
        assert_eq!(space_param(None), None);
        assert_eq!(space_param(Some("q=x")), None);
        assert_eq!(space_param(Some("q=x&space=spc_1")).as_deref(), Some("spc_1"));
        assert_eq!(space_param(Some("space=a%20b")).as_deref(), Some("a b"));
        assert_eq!(space_param(Some("space=")).as_deref(), Some(""));
    }

    #[test]
    fn folders_overlap_when_one_holds_the_other() {
        let tmp = tempfile::tempdir().unwrap();
        let a = normalize(&tmp.path().join("x/../a"));
        assert_eq!(a, normalize(&tmp.path().join("a")));
        assert!(overlaps(&a, &a.join("b")));
        assert!(overlaps(&a.join("b"), &a));
        assert!(!overlaps(&a, &normalize(&tmp.path().join("ab"))));
    }

    #[test]
    fn a_patch_tells_a_null_colour_from_a_missing_one() {
        let p: PatchBody = serde_json::from_str(r#"{"color":null}"#).unwrap();
        assert_eq!(p.color, Some(None));
        let p: PatchBody = serde_json::from_str(r#"{}"#).unwrap();
        assert_eq!(p.color, None);
        assert!(serde_json::from_str::<PatchBody>(r#"{"archive":true}"#).is_err());
    }
}
