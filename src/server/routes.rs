use super::discussion;
use super::spaces::{check_workspace, Scope};
use super::AppState;
use crate::broker::ApprovalDecision;
use crate::db::models::*;
use crate::db::spaces::SpaceId;
use crate::db::DbError;
use crate::drivers::{find_cli_binary, RoutingCandidate, RuntimeEvent, TurnInput};
use crate::router::{parse_mentions_with_lookup, TagMention, TeamDecision};
use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use std::path::PathBuf;
use serde::{Deserialize, Serialize};
use serde_json::json;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

#[derive(Serialize)]
pub struct HealthResponse {
    pub status: &'static str,
    pub app: &'static str,
    pub version: &'static str,
    pub laya_url: String,
    pub laya: crate::router::sidecar::LayaStatus,
}

pub async fn health(State(state): State<AppState>) -> impl IntoResponse {
    Json(HealthResponse {
        status: "ok",
        app: "kwnt",
        version: env!("CARGO_PKG_VERSION"),
        laya_url: state.config.laya_url.clone(),
        laya: state.router.status(),
    })
}

pub async fn laya_status(State(state): State<AppState>) -> impl IntoResponse {
    Json(state.router.status())
}

/// A `{ "error": … }` body the UI can show as it stands.
fn api_error(status: StatusCode, message: impl Into<String>) -> Response {
    (status, Json(json!({ "error": message.into() }))).into_response()
}

/// [`db_status`] with a readable body. A unique-constraint error (two writes racing past the
/// name check) is the 409 `clash`.
fn db_error(e: &DbError, clash: &str) -> Response {
    match e {
        DbError::Sqlx(sqlx::Error::Database(d)) if d.is_unique_violation() => api_error(StatusCode::CONFLICT, clash),
        DbError::OtherSpace(m) | DbError::Conflict(m) => api_error(db_status(e), m.clone()),
        _ => {
            tracing::warn!(error = %e, "database write failed");
            api_error(StatusCode::INTERNAL_SERVER_ERROR, "database error")
        }
    }
}

/// 400 for a write that would mix spaces, 409 for a clash, 500 otherwise.
pub(crate) fn db_status(e: &DbError) -> StatusCode {
    match e {
        DbError::OtherSpace(_) => StatusCode::BAD_REQUEST,
        DbError::Conflict(_) => StatusCode::CONFLICT,
        _ => StatusCode::INTERNAL_SERVER_ERROR,
    }
}

/// The space's kwants, each with `runs_on`: the account its next turn would use.
pub async fn list_kwants(
    State(state): State<AppState>,
    Scope(space): Scope,
) -> Result<Json<Vec<serde_json::Value>>, StatusCode> {
    let views = state
        .db
        .kwant_views(&space)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let mut out = Vec::with_capacity(views.len());
    for v in views {
        let runs_on = super::profile_turn::runs_on(&state, &v.kwant).await;
        let mut item = serde_json::to_value(&v).map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
        item["runs_on"] = runs_on;
        let mode = state
            .db
            .kwant_approvals_in(&space, &v.kwant.id)
            .await
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
        item["approvals"] = json!(mode.bot);
        item["approvals_effective"] = json!(mode.effective);
        out.push(item);
    }
    Ok(Json(out))
}

/// Trims the name and rejects names that can't be @mentioned or clash with another kwant in the
/// space. Other spaces may use the same name.
#[allow(clippy::result_large_err)]
async fn validate_kwant_name(
    state: &AppState,
    space: &SpaceId,
    name: &mut String,
    own_id: Option<&str>,
) -> Result<(), Response> {
    *name = name.trim().to_string();
    if name.is_empty() {
        return Err(api_error(StatusCode::BAD_REQUEST, "a bot needs a name"));
    }
    if name.contains(char::is_whitespace) || name.contains('@') {
        return Err(api_error(StatusCode::BAD_REQUEST, "a bot name can't contain spaces or @"));
    }
    let kwants = state.db.list_kwants(space).await.map_err(|e| db_error(&e, ""))?;
    if kwants.iter().any(|k| Some(k.id.as_str()) != own_id && k.name.eq_ignore_ascii_case(name)) {
        return Err(api_error(StatusCode::CONFLICT, format!("a bot called {name} already exists in this space")));
    }
    Ok(())
}

/// Trims the name; 400 when it's empty and 409 when another team in the space has it, in any
/// case (a name-derived folder would clash on a Mac). Other spaces may use the same name.
#[allow(clippy::result_large_err)]
async fn check_team_name(
    state: &AppState,
    space: &SpaceId,
    name: &mut String,
    own_id: Option<&str>,
) -> Result<(), Response> {
    *name = name.trim().to_string();
    if name.is_empty() {
        return Err(api_error(StatusCode::BAD_REQUEST, "a team needs a name"));
    }
    let teams = state.db.list_teams(space).await.map_err(|e| db_error(&e, ""))?;
    if teams.iter().any(|t| Some(t.id.as_str()) != own_id && t.name.eq_ignore_ascii_case(name)) {
        return Err(api_error(StatusCode::CONFLICT, team_clash(name)));
    }
    Ok(())
}

fn team_clash(name: &str) -> String {
    format!("a team called {name} already exists in this space")
}

/// `Some(None)` for an explicit JSON `null`, so "unset" and "leave as is" differ.
fn deserialize_some<'de, D, T>(d: D) -> Result<Option<T>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: Deserialize<'de>,
{
    T::deserialize(d).map(Some)
}

/// Profile fields a kwant create or update may carry besides the kwant's own.
#[derive(Debug, Deserialize)]
pub struct ProfileBinding {
    /// A profile id, `null` for the engine default, absent to leave as is.
    #[serde(default, deserialize_with = "deserialize_some")]
    profile_id: Option<Option<String>>,
    profile_failover: Option<bool>,
}

#[derive(Debug, Deserialize)]
pub struct CreateKwantRequest {
    #[serde(flatten)]
    base: NewKwant,
    #[serde(flatten)]
    binding: ProfileBinding,
}

#[derive(Debug, Deserialize)]
pub struct UpdateKwantRequest {
    #[serde(flatten)]
    base: UpdateKwant,
    #[serde(flatten)]
    binding: ProfileBinding,
}

/// The pinned profile must exist, be in the kwant's space and be for the engine of its model.
async fn check_binding(
    state: &AppState,
    space: &SpaceId,
    binding: &ProfileBinding,
    preferred_model: &str,
) -> Result<(), StatusCode> {
    let Some(Some(id)) = &binding.profile_id else {
        return Ok(());
    };
    let profile = state
        .db
        .get_profile(id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::BAD_REQUEST)?;
    if profile.space_id != space.as_str() {
        return Err(StatusCode::BAD_REQUEST);
    }
    let family = crate::router::parse_pin(preferred_model).family;
    match family.as_deref().and_then(crate::profiles::Engine::parse) {
        Some(engine) if engine.as_str() != profile.engine => Err(StatusCode::BAD_REQUEST),
        _ => Ok(()),
    }
}

async fn save_binding(
    state: &AppState,
    kwant_id: &str,
    binding: &ProfileBinding,
) -> Result<(), StatusCode> {
    if binding.profile_id.is_none() && binding.profile_failover.is_none() {
        return Ok(());
    }
    let profile_id = binding.profile_id.as_ref().map(|p| p.as_deref());
    state
        .db
        .set_kwant_binding(kwant_id, profile_id, binding.profile_failover)
        .await
        .map_err(|e| db_status(&e))
}

async fn kwant_view_json(state: &AppState, id: &str) -> Result<Json<KwantView>, StatusCode> {
    match state.db.kwant_view(id).await {
        Ok(Some(v)) => Ok(Json(v)),
        Ok(None) => Err(StatusCode::NOT_FOUND),
        Err(_) => Err(StatusCode::INTERNAL_SERVER_ERROR),
    }
}

#[allow(clippy::result_large_err)]
pub async fn create_kwant(
    State(state): State<AppState>,
    Scope(space): Scope,
    Json(mut payload): Json<CreateKwantRequest>,
) -> Result<Json<KwantView>, Response> {
    if let Some(msg) = crate::import::check_persona_caps(&payload.base.persona_prompt, &payload.base.soul) {
        return Err(api_error(StatusCode::BAD_REQUEST, &msg));
    }
    validate_kwant_name(&state, &space, &mut payload.base.name, None).await?;
    check_binding(&state, &space, &payload.binding, &payload.base.preferred_model)
        .await
        .map_err(IntoResponse::into_response)?;
    let clash = format!("a bot called {} already exists in this space", payload.base.name);
    let kwant = state.db.create_kwant(&space, &payload.base).await.map_err(|e| db_error(&e, &clash))?;
    save_binding(&state, &kwant.id, &payload.binding).await.map_err(IntoResponse::into_response)?;
    ensure_general(&state, &space, &kwant.id).await;
    kwant_view_json(&state, &kwant.id).await.map_err(IntoResponse::into_response)
}

/// A new space starts empty, and a channel needs a lead, so its #general (routed by Laya)
/// appears with its first bot. Default's comes from the seed.
async fn ensure_general(state: &AppState, space: &SpaceId, lead: &str) {
    if space.is_default() || !matches!(state.db.general_channel(space).await, Ok(None)) {
        return;
    }
    let general = NewChannel {
        name: "general".into(),
        description: Some(crate::db::GENERAL_DESCRIPTION.into()),
        lead_kwant_id: lead.to_string(),
        member_kwants: vec![lead.to_string()],
        routing_mode: "auto".into(),
    };
    if let Err(e) = state.db.create_channel(space, &general).await {
        tracing::warn!(error = %e, space = %space, "could not create the space's #general");
    }
}

#[allow(clippy::result_large_err)]
pub async fn update_kwant(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(mut payload): Json<UpdateKwantRequest>,
) -> Result<Json<KwantView>, Response> {
    if let Some(msg) = crate::import::check_persona_caps(&payload.base.persona_prompt, &payload.base.soul) {
        return Err(api_error(StatusCode::BAD_REQUEST, &msg));
    }
    let space = state
        .db
        .space_of_kwant(&id)
        .await
        .map_err(|e| db_error(&e, ""))?
        .ok_or_else(|| api_error(StatusCode::NOT_FOUND, "bot not found"))?;
    validate_kwant_name(&state, &space, &mut payload.base.name, Some(&id)).await?;
    check_binding(&state, &space, &payload.binding, &payload.base.preferred_model)
        .await
        .map_err(IntoResponse::into_response)?;
    let clash = format!("a bot called {} already exists in this space", payload.base.name);
    match state.db.update_kwant(&id, &payload.base).await {
        Ok(Some(_)) => {}
        Ok(None) => return Err(api_error(StatusCode::NOT_FOUND, "bot not found")),
        Err(e) => return Err(db_error(&e, &clash)),
    }
    save_binding(&state, &id, &payload.binding).await.map_err(IntoResponse::into_response)?;
    kwant_view_json(&state, &id).await.map_err(IntoResponse::into_response)
}

/// `GET /api/kwants/:id/usage`: what deleting the bot would remove, for the confirm dialog.
pub async fn kwant_usage(State(state): State<AppState>, Path(id): Path<String>) -> Response {
    match state.db.get_kwant(&id).await {
        Ok(Some(_)) => {}
        Ok(None) => return api_error(StatusCode::NOT_FOUND, "bot not found"),
        Err(e) => return db_error(&e, ""),
    }
    match state.db.kwant_usage(&id).await {
        Ok(mut v) => {
            v["running"] = json!(kwant_is_busy(&state, &id).await);
            Json(v).into_response()
        }
        Err(e) => db_error(&e, ""),
    }
}

/// A turn of the bot is running or waiting, or delegated work for or from it is.
async fn kwant_is_busy(state: &AppState, id: &str) -> bool {
    if state.turns.active_turns().iter().any(|t| t.kwant_id == id) {
        return true;
    }
    state.db.kwant_has_live_delegations(id).await.unwrap_or(true)
}

/// `DELETE /api/kwants/:id`: the person removes a bot with its conversations, memories,
/// skills, routines and memberships. 409 `{error: "running"}` while it works, and
/// `{error: "last_bot"}` for a space's only bot. Its working folder is kept on disk.
pub async fn delete_kwant(State(state): State<AppState>, headers: axum::http::HeaderMap, Path(id): Path<String>) -> Response {
    if let Err(r) = super::mcp_suggest::refuse_agent_tokens(&state, &headers, "only the person deletes a bot").await {
        return r;
    }
    let bot = match state.db.get_kwant(&id).await {
        Ok(Some(b)) => b,
        Ok(None) => return api_error(StatusCode::NOT_FOUND, "bot not found"),
        Err(e) => return db_error(&e, ""),
    };
    let conflict = |error: &str, message: String| (StatusCode::CONFLICT, Json(json!({"error": error, "message": message}))).into_response();
    if kwant_is_busy(&state, &id).await {
        return conflict("running", format!("{} is working right now. Stop it or wait for it to finish, then delete it.", bot.name));
    }
    let space = match state.db.space_of_kwant(&id).await {
        Ok(Some(s)) => s,
        Ok(None) => return api_error(StatusCode::NOT_FOUND, "bot not found"),
        Err(e) => return db_error(&e, ""),
    };
    let others: Vec<_> = state.db.list_kwants(&space).await.unwrap_or_default().into_iter().filter(|k| k.id != id).collect();
    let Some(fallback) = others.first() else {
        return conflict("last_bot", format!("{} is the only bot in this space; a space needs at least one.", bot.name));
    };
    let usage = state.db.kwant_usage(&id).await.unwrap_or(serde_json::Value::Null);
    match state.db.delete_kwant_and_history(&id, &fallback.id).await {
        Ok(true) => {}
        Ok(false) => return api_error(StatusCode::NOT_FOUND, "bot not found"),
        Err(e) => return db_error(&e, ""),
    }
    state.broker.forget_kwant(&id).await;
    super::passwords::forget_deleted_owners(&state, &[&id]).await;
    state.scheduler_wake.notify_one();
    tracing::info!(kwant_id = %id, name = %bot.name, "the person deleted a bot");
    let folder = crate::db::spaces::space_data_dir(&state.config.data_dir, &space).join("bot-workspaces").join(&id);
    Json(json!({
        "status": "deleted",
        "deleted": id,
        "name": bot.name,
        "removed": usage,
        "kept_folder": folder.exists().then(|| folder.display().to_string()),
    }))
    .into_response()
}

pub async fn list_channels(State(state): State<AppState>, Scope(space): Scope) -> Result<Json<Vec<Channel>>, StatusCode> {
    state
        .db
        .list_channels(&space)
        .await
        .map(Json)
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)
}

#[allow(clippy::result_large_err)]
pub async fn create_channel(
    State(state): State<AppState>,
    Scope(space): Scope,
    Json(mut payload): Json<NewChannel>,
) -> Result<Json<Channel>, Response> {
    payload.name = payload.name.trim().to_string();
    if payload.name.is_empty() {
        return Err(api_error(StatusCode::BAD_REQUEST, "a channel needs a name"));
    }
    if !crate::db::models::ROUTING_MODES.contains(&payload.routing_mode.as_str()) {
        return Err(api_error(StatusCode::BAD_REQUEST, "routing_mode must be \"auto\" or \"lead\""));
    }
    if payload.lead_kwant_id.is_empty() {
        match payload.member_kwants.first() {
            Some(first) => payload.lead_kwant_id = first.clone(),
            None => return Err(api_error(StatusCode::BAD_REQUEST, "a channel needs at least one bot")),
        }
    }
    // The lead is always one of the bots Laya picks from
    payload.member_kwants = crate::db::models::members_with_lead(&payload.lead_kwant_id, &payload.member_kwants);
    let clash = format!("a channel called #{} already exists in this space", payload.name);
    let channels = state.db.list_channels(&space).await.map_err(|e| db_error(&e, ""))?;
    if channels.iter().any(|c| c.name.eq_ignore_ascii_case(&payload.name)) {
        return Err(api_error(StatusCode::CONFLICT, clash));
    }
    state
        .db
        .create_channel(&space, &payload)
        .await
        .map(Json)
        .map_err(|e| db_error(&e, &clash))
}

#[derive(Debug, Deserialize)]
pub struct MessagesQuery {
    /// A window centred on this message (e.g. a search hit older than the latest page).
    pub around: Option<String>,
    /// The page just before this message.
    pub before: Option<String>,
    /// Default 100, at most [`crate::db::history::MAX_HISTORY_PAGE`].
    pub limit: Option<i64>,
}

/// The latest 100 messages, or with `around`/`before` an older window. 404 when that
/// message isn't in this chat.
pub async fn list_messages(
    State(state): State<AppState>,
    Path(channel_or_kwant_id): Path<String>,
    Query(q): Query<MessagesQuery>,
) -> Result<Json<Vec<Message>>, (StatusCode, Json<serde_json::Value>)> {
    let err = |status: StatusCode, m: &str| (status, Json(json!({ "error": m })));
    let max = crate::db::history::MAX_HISTORY_PAGE;
    let limit = match q.limit {
        None => 100,
        Some(n) if (1..=max).contains(&n) => n,
        Some(_) => return Err(err(StatusCode::BAD_REQUEST, &format!("limit must be 1–{max}"))),
    };
    let db = &state.db;
    let rows = match (q.around.as_deref(), q.before.as_deref()) {
        (Some(_), Some(_)) => return Err(err(StatusCode::BAD_REQUEST, "use around or before, not both")),
        (Some(id), None) => db.list_messages_around(&channel_or_kwant_id, id, limit).await,
        (None, Some(id)) => db.list_messages_before(&channel_or_kwant_id, id, limit).await,
        (None, None) => db.list_messages(&channel_or_kwant_id, limit).await.map(Some),
    };
    match rows {
        Ok(Some(rows)) => Ok(Json(rows)),
        Ok(None) => Err(err(StatusCode::NOT_FOUND, "That message isn't in this chat.")),
        Err(_) => Err(err(StatusCode::INTERNAL_SERVER_ERROR, "Could not load messages.")),
    }
}

pub async fn list_teams(State(state): State<AppState>, Scope(space): Scope) -> Result<Json<Vec<Team>>, StatusCode> {
    state
        .db
        .list_teams(&space)
        .await
        .map(Json)
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)
}

#[allow(clippy::result_large_err)]
pub async fn create_team(
    State(state): State<AppState>,
    Scope(space): Scope,
    Json(mut payload): Json<NewTeam>,
) -> Result<Json<Team>, axum::response::Response> {
    check_team_name(&state, &space, &mut payload.name, None).await?;
    check_workspace(&state, &space, None, &payload).await?;
    state
        .db
        .create_team(&space, &payload)
        .await
        .map(Json)
        .map_err(|e| db_error(&e, &team_clash(&payload.name)))
}

pub async fn get_team(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<Team>, StatusCode> {
    match state.db.get_team(&id).await {
        Ok(Some(team)) => Ok(Json(team)),
        Ok(None) => Err(StatusCode::NOT_FOUND),
        Err(_) => Err(StatusCode::INTERNAL_SERVER_ERROR),
    }
}

/// `PUT /api/teams/:id` body: a team, plus its pinned instructions if given ("" clears).
#[derive(Debug, Deserialize)]
pub struct UpdateTeamPayload {
    #[serde(flatten)]
    pub team: NewTeam,
    pub instructions: Option<String>,
}

#[allow(clippy::result_large_err)]
pub async fn update_team(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(mut payload): Json<UpdateTeamPayload>,
) -> Result<Json<Team>, axum::response::Response> {
    let status = |s: StatusCode| s.into_response();
    if let Some(text) = &payload.instructions {
        if super::continuity::check_instructions(text).is_err() {
            return Err(status(StatusCode::BAD_REQUEST));
        }
    }
    let space = match state.db.space_of_team(&id).await {
        Ok(Some(space)) => space,
        Ok(None) => return Err(status(StatusCode::NOT_FOUND)),
        Err(_) => return Err(status(StatusCode::INTERNAL_SERVER_ERROR)),
    };
    check_team_name(&state, &space, &mut payload.team.name, Some(&id)).await?;
    check_workspace(&state, &space, Some(&id), &payload.team).await?;
    match state.db.update_team(&id, &payload.team).await {
        Ok(Some(mut team)) => {
            if let Some(text) = &payload.instructions {
                state
                    .db
                    .set_team_instructions(&id, text)
                    .await
                    .map_err(|_| status(StatusCode::INTERNAL_SERVER_ERROR))?;
                team.instructions = Some(text.trim().to_string()).filter(|t| !t.is_empty());
            }
            Ok(Json(team))
        }
        Ok(None) => Err(status(StatusCode::NOT_FOUND)),
        Err(e) => Err(db_error(&e, &team_clash(&payload.team.name))),
    }
}

pub async fn delete_team(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    match state.db.delete_team(&id).await {
        Ok(true) => {
            super::passwords::forget_deleted_owners(&state, &[&id]).await;
            Ok(Json(json!({ "status": "deleted" })))
        }
        Ok(false) => Err(StatusCode::NOT_FOUND),
        Err(_) => Err(StatusCode::INTERNAL_SERVER_ERROR),
    }
}

#[derive(Debug, Deserialize)]
pub struct CancelPayload {
    pub turn_id: Option<String>,
    pub session_id: Option<String>,
    pub thread_id: Option<String>,
    pub channel_id: Option<String>,
    pub team_id: Option<String>,
    pub kwant_id: Option<String>,
}

/// Stops the dispatches for one turn, message, thread, channel, team or direct chat (the first
/// given), or everything with no body. A direct chat's Stop leaves that agent's threads running.
pub async fn cancel_chat(
    State(state): State<AppState>,
    Scope(space): Scope,
    body: axum::body::Bytes,
) -> Result<Json<serde_json::Value>, StatusCode> {
    let key = if !body.is_empty() {
        serde_json::from_slice::<CancelPayload>(&body).ok().and_then(|p| {
            p.turn_id.or(p.session_id).or(p.thread_id).or(p.channel_id).or(p.team_id).or(p.kwant_id)
        })
    } else {
        None
    };
    // Without an id, "stop all" stops only the request's space (Default when no ?space=)
    let cancelled = match &key {
        Some(k) => state.turns.cancel(Some(k)),
        None => super::spaces::stop_turns(&state, &space).await,
    };
    if let Some(thread_id) = &key {
        super::delegation::cancel_thread_delegation(&state, thread_id).await;
    }
    Ok(Json(json!({ "status": "cancelled", "cancelled": cancelled })))
}

#[derive(Debug, Deserialize)]
pub struct SendChatPayload {
    pub channel_id: Option<String>,
    pub kwant_id: Option<String>,
    pub team_id: Option<String>,
    /// Send into this thread of its agent (then `channel_id`, `kwant_id` and `team_id` are ignored).
    #[serde(default)]
    pub thread_id: Option<String>,
    pub prompt: String,
    pub sender_id: String,
    /// Uploaded files (`POST /api/attachments`) sent with this message, at most 10.
    #[serde(default)]
    pub attachment_ids: Vec<String>,
}

async fn get_configured_key(db: &crate::db::Database, target_model: &str) -> Option<String> {
    if target_model.contains("claude") {
        db.get_setting("anthropic_api_key").await.ok().flatten()
    } else if target_model.contains("grok") {
        db.get_setting("xai_api_key").await.ok().flatten()
    } else if target_model.contains("openai") || target_model.contains("codex") || target_model.contains("gpt") {
        db.get_setting("openai_api_key").await.ok().flatten()
    } else {
        None
    }
}

async fn get_configured_endpoint(db: &crate::db::Database, target_model: &str) -> Option<String> {
    if target_model == "local_ollama" {
        db.get_setting("ollama_url").await.ok().flatten()
    } else {
        None
    }
}

/// Longest driver error kept in a saved `turn_failed` message, in bytes.
const MAX_SAVED_ERROR_BYTES: usize = 600;

/// The chat message saved when a turn fails. Other bots read it as history, so the turn's secrets,
/// credentials and home folders are masked and a long error is clipped.
fn turn_failed_text(name: &str, why: &str, activity: &super::activity::TurnActivity) -> String {
    let shown = scrub_urls(&activity.redact(why));
    let clipped = truncate_chars(&shown, MAX_SAVED_ERROR_BYTES);
    let ellipsis = if clipped.len() < shown.len() { "…" } else { "" };
    format!("⚠️ [{name}] Couldn't finish this reply: {clipped}{ellipsis}")
}

/// Truncates to at most `max` bytes without splitting a UTF-8 character.
pub(super) fn truncate_chars(text: &str, max: usize) -> &str {
    if text.len() <= max {
        return text;
    }
    let mut end = max;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    &text[..end]
}

fn routing_candidates(members: &[Kwant]) -> Vec<RoutingCandidate> {
    members
        .iter()
        .map(|k| RoutingCandidate {
            id: k.id.clone(),
            name: k.name.clone(),
            title: k.title.clone(),
        })
        .collect()
}

/// Everything a kwant turn needs besides the speaker and the prompt, shared by handoff turns.
pub(super) struct TurnScope {
    pub(super) state: AppState,
    pub(super) channel_id: Option<String>,
    pub(super) kwant_id: Option<String>,
    /// Set for turns in a thread; then `channel_id` and `kwant_id` are `None`.
    pub(super) thread_id: Option<String>,
    pub(super) team_keys: Option<(String, String)>,
    pub(super) team_context: Option<crate::drivers::TeamContext>,
    pub(super) workspace_dir: Option<std::path::PathBuf>,
    /// Kwants in this chat that a speaker may hand the turn to with an @mention.
    pub(super) teammates: Vec<Kwant>,
    pub(super) cancel_token: CancellationToken,
    /// The dispatch this turn belongs to (see [`super::turns`]); its turns are added as keys.
    pub(super) job_id: String,
    /// How many delegations deep this chat is: 0 for user chats, 1 in a delegation thread, …
    pub(super) delegation_depth: i64,
}

impl TurnScope {
    /// The chat's id: channel, thread, or the direct chat's kwant (falling back to `speaker`).
    pub(super) fn context_id(&self, speaker: &Kwant) -> String {
        self.channel_id
            .clone()
            .or_else(|| self.thread_id.clone())
            .or_else(|| self.kwant_id.clone())
            .unwrap_or_else(|| speaker.id.clone())
    }

    pub(super) fn message(&self, sender_role: &str, sender_id: &str, text: String) -> NewMessage {
        NewMessage {
            channel_id: self.channel_id.clone(),
            direct_kwant_id: self.kwant_id.clone(),
            sender_role: sender_role.into(),
            sender_id: sender_id.into(),
            text,
            reasoning_content: None,
            tool_calls: None,
            tokens_used: 0,
        }
    }

    fn extra(&self) -> MessageExtra {
        MessageExtra { thread_id: self.thread_id.clone(), meta: None }
    }

    /// Saves a message in this chat (moving a thread up its list).
    pub(super) async fn save(&self, msg: &NewMessage, meta: Option<serde_json::Value>) -> Option<Message> {
        let extra = MessageExtra { meta, ..self.extra() };
        let saved = self.state.db.insert_message_with(msg, &extra).await.ok();
        if let Some(t) = &self.thread_id {
            let _ = self.state.db.touch_thread(t).await;
        }
        saved
    }
}

/// Builds the scope for a turn in a channel, a thread or `kwant`'s direct chat (when both are
/// `None`): its team, workspace and members, like a user message there would get.
pub(super) async fn scope_for(
    state: &AppState,
    kwant: &Kwant,
    channel_id: Option<String>,
    thread_id: Option<String>,
    job: &super::turns::JobGuard,
    delegation_depth: i64,
) -> Result<TurnScope, String> {
    // Teammates, the team and its folder all come from the kwant's own space
    let space = SpaceId::new(&kwant.space_id);
    let all_kwants = state.db.list_kwants(&space).await.map_err(|_| "could not load the bots".to_string())?;
    let channel = match &channel_id {
        Some(id) => Some(
            state.db.get_channel(id).await.ok().flatten().ok_or_else(|| "the chat's channel no longer exists".to_string())?,
        ),
        None => None,
    };
    let thread = match &thread_id {
        Some(id) => Some(state.db.get_thread(id).await.ok().flatten().ok_or_else(|| "the thread no longer exists".to_string())?),
        None => None,
    };
    let teams = state.db.list_teams(&space).await.unwrap_or_default();
    let team = match (&thread, channel.as_ref().and_then(|c| c.name.strip_prefix("team-"))) {
        (Some(t), _) => t.team_id.as_ref().and_then(|id| teams.iter().find(|x| &x.id == id).cloned()),
        (None, Some(name)) => teams.iter().find(|t| t.name.eq_ignore_ascii_case(name)).cloned(),
        _ => None,
    };
    // Threads are one agent's own conversation: nobody to hand the turn to
    let member_ids: Vec<String> = match (&thread, &channel) {
        (None, Some(c)) => channel_member_ids(c, team.as_ref(), &all_kwants),
        _ => Vec::new(),
    };
    let teammates: Vec<Kwant> = member_ids.iter().filter_map(|id| all_kwants.iter().find(|k| &k.id == id).cloned()).collect();
    let workspace_dir = match &team {
        Some(t) => Some(
            crate::tools::resolve_or_prepare_workspace(
                &crate::db::spaces::space_data_dir(&state.config.data_dir, &space),
                &t.name,
                t.workspace_path.as_deref(),
                t.repo_url.as_deref(),
            )
            .await,
        ),
        None => None,
    };
    Ok(TurnScope {
        state: state.clone(),
        kwant_id: if channel_id.is_none() && thread_id.is_none() { Some(kwant.id.clone()) } else { None },
        channel_id,
        thread_id,
        team_keys: team.as_ref().map(|t| (t.id.clone(), t.name.clone())),
        team_context: team.as_ref().map(team_context),
        workspace_dir,
        teammates,
        cancel_token: job.token.clone(),
        job_id: job.id.clone(),
        delegation_depth,
    })
}

/// The bots of a channel: its team's members for a `team-` channel, every bot of the space in
/// "general", else the channel's own member list. `team` is the channel's team, if any.
pub(super) fn channel_member_ids(channel: &Channel, team: Option<&Team>, all_kwants: &[Kwant]) -> Vec<String> {
    match team {
        Some(t) => serde_json::from_str(&t.member_kwants).unwrap_or_default(),
        None if channel.name == "general" => all_kwants.iter().map(|k| k.id.clone()).collect(),
        None => serde_json::from_str(&channel.member_kwants).unwrap_or_default(),
    }
}

fn team_context(t: &Team) -> crate::drivers::TeamContext {
    crate::drivers::TeamContext {
        id: t.id.clone(),
        name: t.name.clone(),
        description: t.description.clone(),
        workspace_path: t.workspace_path.clone(),
        repo_url: t.repo_url.clone(),
    }
}

/// Max follow-up turns kwants may trigger by @mentioning each other for one user message.
const MAX_HANDOFFS: usize = 4;
/// Delegations a lead's @mentions may start for one user message, follow-up rounds included:
/// a lead resumed with results can hand on again, but never in an endless loop.
pub const MAX_DELEGATIONS_PER_REQUEST: usize = 16;
/// How often one teammate may be handed work by @mention for one user message, so two bots
/// naming each other stop after a few rounds instead of running to the total cap. A mention past
/// either cap leaves a Continue notice ([`super::budget::CAPPED_KIND`]); the chain token budget
/// still pauses a long chain on its own.
pub const MAX_MENTION_DELEGATIONS_PER_BOT: usize = 4;

#[allow(clippy::too_many_arguments)]
fn execute_single_turn(
    state: AppState,
    target_kwant: Kwant,
    payload: SendChatPayload,
    job: super::turns::JobGuard,
    team_keys: Option<(String, String)>,
    team_context: Option<crate::drivers::TeamContext>,
    workspace_dir: Option<std::path::PathBuf>,
    teammates: Vec<Kwant>,
    routing: Option<serde_json::Value>,
) {
    let scope = TurnScope {
        channel_id: payload.channel_id.clone(),
        kwant_id: payload.kwant_id.clone(),
        thread_id: None,
        team_keys,
        team_context,
        workspace_dir,
        teammates,
        cancel_token: job.token.clone(),
        job_id: job.id.clone(),
        delegation_depth: 0,
        state,
    };
    let prompt = payload.prompt.clone();
    // the lead got a team-building request: say so, so it proposes the bots instead of answering
    // as its own specialty (the saved message stays as the person wrote it)
    let turn_prompt = if routing.as_ref().is_some_and(|r| r["basis"] == "team") {
        format!("{prompt}\n\n{}", crate::router::team::TEAM_BUILDING_NOTE)
    } else {
        prompt.clone()
    };

    tokio::spawn(async move {
        // Ends the job (its cancel keys) when the turn and its handoffs are done
        let _job = job;
        let opts = TurnOpts { routing, ..TurnOpts::default() };
        if let Some(reply) = run_kwant_turn(&scope, &target_kwant, turn_prompt, opts).await {
            run_handoffs(&scope, std::slice::from_ref(&target_kwant.id), &[(target_kwant.clone(), reply)], &prompt).await;
            // Several bots asked the person something: one list of it all (when the chain is over)
            super::asks::consolidate(&scope).await;
        }
    });
}

/// Runs a routine's turn as if the user had sent `prompt` to `kwant` in `thread_id`, in
/// `channel_id` or in their direct chat: same team context, workspace, approvals and profile failover, and the
/// chat's stop button cancels it. The prompt message must already be saved. Returns the id of
/// the kwant's reply, or `Err` with why there is none.
pub(super) async fn run_routine_turn(
    state: &AppState,
    kwant: &Kwant,
    channel_id: Option<String>,
    thread_id: Option<String>,
    prompt: String,
) -> Result<Option<String>, String> {
    let context_key = thread_id.clone().or_else(|| channel_id.clone()).unwrap_or_else(|| kwant.id.clone());
    let job = state.turns.start_job([context_key.clone()]);
    let scope = scope_for(state, kwant, channel_id, thread_id, &job, 0).await?;
    let reply = try_kwant_turn(&scope, kwant, prompt.clone(), TurnOpts::default()).await;
    let reply = continue_once_if_capped(&scope, kwant, reply, false).await;
    match reply {
        Ok(text) => {
            let reply_id = state
                .db
                .list_messages(&context_key, 10)
                .await
                .unwrap_or_default()
                .into_iter()
                .rev()
                .find(|m| m.sender_role == "kwant" && m.sender_id == kwant.name)
                .map(|m| m.id);
            run_handoffs(&scope, std::slice::from_ref(&kwant.id), &[(kwant.clone(), text)], &prompt).await;
            Ok(reply_id)
        }
        Err(_) if job.token.is_cancelled() => Err("stopped by the user".into()),
        Err(why) => Err(format!("the bot's turn failed: {why}")),
    }
}

/// The prompt of the one automatic continuation a routine's or delegation's turn gets when it ran
/// out of model calls (see [`continue_once_if_capped`]).
pub const CONTINUE_PROMPT: &str = "[kwnt: your last turn stopped at the limit of model calls before you finished. \
This is your one automatic continuation: pick up where you left off, finish the task, and end with a complete answer.]";

/// For turns nobody is watching (routines, delegations): a turn that stopped at the model-call
/// cap gets one more turn to finish, never more. Returns both replies joined, so a delegation's
/// result has all of it. A turn that didn't hit the cap, or failed, is returned as it is.
/// `always_ask` carries [`TurnOpts::always_ask`] over to the continuation.
pub(super) async fn continue_once_if_capped(
    scope: &TurnScope,
    kwant: &Kwant,
    first: Result<String, String>,
    always_ask: bool,
) -> Result<String, String> {
    let Ok(text) = &first else {
        return first;
    };
    if !crate::drivers::api::hit_call_cap(text) || scope.cancel_token.is_cancelled() {
        return first;
    }
    tracing::info!(kwant = %kwant.id, "turn hit the model-call cap: one automatic continuation");
    let opts = TurnOpts {
        label: Some("Automatic continuation (once): the last turn ran out of model calls".into()),
        always_ask,
        ..Default::default()
    };
    match try_kwant_turn(scope, kwant, CONTINUE_PROMPT.to_string(), opts).await {
        Ok(more) => Ok(format!("{}\n\n{more}", text.trim_end())),
        Err(_) => first,
    }
}

/// Lets teammates that kwants @mentioned in `replies` answer in turn, so kwants can delegate to each other.
/// `already_spoken` kwants are not handed the turn again.
/// [`run_handoffs`] for a lead's follow-up after delegation results: boxed with a named type,
/// because handing on starts delegations whose end resumes the lead again (a cycle of futures).
pub(super) fn run_handoffs_boxed<'a>(
    scope: &'a TurnScope,
    already_spoken: &'a [String],
    replies: &'a [(Kwant, String)],
    user_prompt: &'a str,
) -> std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send + 'a>> {
    Box::pin(run_handoffs(scope, already_spoken, replies, user_prompt))
}

async fn run_handoffs(scope: &TurnScope, already_spoken: &[String], replies: &[(Kwant, String)], user_prompt: &str) {
    let named: Vec<(String, String)> = replies.iter().map(|(k, t)| (k.name.clone(), t.clone())).collect();
    if super::budget::pause_if_over(scope, already_spoken, &named, user_prompt).await {
        return;
    }
    if let [(from, text)] = replies {
        if is_team_channel_lead(scope, from).await {
            return delegate_mentions(scope, from, text, user_prompt, already_spoken, None).await;
        }
    }
    if scope.teammates.is_empty() {
        return;
    }
    // (teammate, asked by, asker's text, the teammate whose result it waits for)
    type Queued = (Kwant, String, String, Option<String>);
    let mut spoken: Vec<String> = already_spoken.to_vec();
    let mut queue: std::collections::VecDeque<Queued> = std::collections::VecDeque::new();
    let mut skipped: Vec<(String, String)> = Vec::new();
    let mut enqueue = |queue: &mut std::collections::VecDeque<Queued>, spoken: &[String], from: &Kwant, text: &str| {
        let names: Vec<&str> = scope.teammates.iter().map(|k| k.name.as_str()).collect();
        let found = crate::router::handoff_mentions(text, &names);
        let deps = crate::router::handoff_dependencies(text, &names);
        skipped.extend(found.unknown.into_iter().map(|n| (n, String::new())));
        skipped.extend(found.in_code_block.iter().map(|&i| (scope.teammates[i].name.clone(), IN_CODE_BLOCK.to_string())));
        for &i in &found.targets {
            let k = &scope.teammates[i];
            // Someone who already spoke isn't handed the turn back: no ping-pong
            if k.id != from.id && !spoken.contains(&k.id) && !queue.iter().any(|(q, _, _, _)| q.id == k.id) {
                // "@Writer, once @Analyst has the numbers…": the Writer goes after the Analyst
                let after = deps.iter().find(|&&(d, _)| d == i).map(|&(_, p)| scope.teammates[p].id.clone());
                queue.push_back((k.clone(), from.name.clone(), text.to_string(), after));
            }
        }
    };
    for (from, text) in replies {
        enqueue(&mut queue, &spoken, from, text);
    }
    if queue.is_empty() {
        drop(enqueue);
        return report_skipped(scope, skipped).await;
    }

    let events = scope.state.event_sender.clone();
    let context_id = scope.context_id(&replies[0].0);
    let since = last_user_message_at(scope, &context_id).await;
    // How each bot's turn in this chain went, for the teammates that wait on it
    let mut outcomes: std::collections::HashMap<String, ChainOutcome> = std::collections::HashMap::new();
    for (k, text) in replies {
        outcomes.insert(k.id.clone(), chain_outcome(scope, &context_id, k, Some(text), since).await);
    }
    order_dependents(&mut queue);
    let total = |queue_len: usize, done: usize| (1 + done + queue_len).min(1 + MAX_HANDOFFS);
    let names_of = |id: &str| scope.teammates.iter().find(|k| k.id == id).map(|k| k.name.clone()).unwrap_or_default();
    let _ = events.send(RuntimeEvent::DiscussionStarted {
        team_id: context_id.clone(),
        team_name: scope.team_keys.as_ref().map(|(_, n)| n.clone()).unwrap_or_default(),
        total_turns: total(queue.len(), 0),
        speakers: queue.iter().map(|(k, _, _, _)| k.id.clone()).collect(),
        plan: queue
            .iter()
            .map(|(k, from, _, after)| match after {
                Some(p) => format!("{}: asked by {}, after {}", k.name, from, names_of(p)),
                None => format!("{}: asked by {}", k.name, from),
            })
            .collect(),
        kind: "handoff".into(),
    });

    // Teammates that waited for one who asked the person or failed
    let mut not_started: Vec<(String, String)> = Vec::new();
    let mut done = 0;
    while let Some((kwant, from, from_text, after)) = queue.pop_front() {
        if done >= MAX_HANDOFFS || scope.cancel_token.is_cancelled() {
            queue.push_front((kwant, from, from_text, after));
            break;
        }
        // Over the chain's token budget: one notice, and the rest wait for the person's "continue"
        let waiting: Vec<(String, String)> =
            std::iter::once((from.clone(), from_text.clone())).chain(queue.iter().map(|(_, f, t, _)| (f.clone(), t.clone()))).collect();
        if super::budget::pause_if_over(scope, &spoken, &waiting, user_prompt).await {
            queue.clear();
            break;
        }
        let waited_for = match after.as_ref().map(|p| (names_of(p), outcomes.get(p))) {
            // Waits for a teammate whose turn hasn't run: it was never handed one, so don't wait
            Some((_, None)) | None => None,
            Some((name, Some(ChainOutcome::Done(result)))) => Some((name, result.clone())),
            Some((name, Some(outcome))) => {
                not_started.push((kwant.name.clone(), outcome.not_started(&name)));
                continue;
            }
        };
        let mut prompt = format!(
            "{from} handed this conversation to you by mentioning you.\n\nThe user's original request:\n{user}\n\n{text}\n\n\
            Reply to the user now with your own part of this, in character. Don't repeat what {from} already said.",
            from = from,
            user = user_prompt,
            text = crate::drivers::peer_message(&from, truncate_chars(&from_text, 4000)),
        );
        if let Some((name, result)) = &waited_for {
            prompt.push_str(&format!(
                "\n\n{from} asked you to wait for {name}'s result. {name} has finished:\n{}",
                crate::drivers::peer_message(name, truncate_chars(result, 4000)),
            ));
        }
        let _ = events.send(RuntimeEvent::DiscussionTurnStarted {
            speaker_id: kwant.id.clone(),
            speaker_name: kwant.name.clone(),
            turn_index: done + 2,
            total_turns: total(queue.len(), done + 1),
            prompt: prompt.clone(),
            asked_by: Some(from.clone()),
            phase: None,
        });
        done += 1;
        spoken.push(kwant.id.clone());
        match run_kwant_turn(scope, &kwant, prompt, TurnOpts::default()).await {
            Some(reply) => {
                outcomes.insert(kwant.id.clone(), chain_outcome(scope, &context_id, &kwant, Some(&reply), since).await);
                enqueue(&mut queue, &spoken, &kwant, &reply);
                order_dependents(&mut queue);
            }
            None if scope.cancel_token.is_cancelled() => break,
            None => {
                outcomes.insert(kwant.id.clone(), ChainOutcome::Failed);
            }
        }
    }
    drop(enqueue);
    skipped.extend(not_started);
    if !scope.cancel_token.is_cancelled() {
        skipped.extend(queue.into_iter().map(|(k, _, _, _)| (k.name, format!("only {MAX_HANDOFFS} teammates take a turn per message"))));
    }

    let _ = events.send(RuntimeEvent::DiscussionCompleted { total_turns: done + 1, context_id: Some(context_id.clone()) });
    report_skipped(scope, skipped).await;
}

/// How one bot's turn in a handoff chain ended, for a teammate that waits on its result.
#[derive(Debug, Clone, PartialEq)]
pub(super) enum ChainOutcome {
    /// Finished, with this reply.
    Done(String),
    /// Ended asking the person something (a question card, or a reply whose last line asks).
    Asked,
    /// Failed or was stopped.
    Failed,
}

impl ChainOutcome {
    /// Why a teammate waiting on `name` doesn't start.
    pub(super) fn not_started(&self, name: &str) -> String {
        match self {
            ChainOutcome::Asked => format!("waiting for {name}, who asked you something first. Answer {name} and the chain goes on"),
            _ => format!("waiting for {name}'s result, and {name} didn't finish"),
        }
    }
}

/// Whether `kwant`'s turn ended with `reply` asking the person: an open question card it raised in
/// this chat since `since`, or a last line that asks ([`crate::router::sticky::asks_the_person`]).
async fn chain_outcome(
    scope: &TurnScope,
    context_id: &str,
    kwant: &Kwant,
    reply: Option<&str>,
    since: Option<chrono::DateTime<chrono::Utc>>,
) -> ChainOutcome {
    let Some(reply) = reply else { return ChainOutcome::Failed };
    let open = scope.state.db.list_questions(Some("open"), Some(context_id), None).await.unwrap_or_default();
    let carded = open.iter().any(|q| q.kwant_id == kwant.id && since.is_none_or(|t| q.created_at >= t));
    if carded || crate::router::sticky::asks_the_person(reply) {
        ChainOutcome::Asked
    } else {
        ChainOutcome::Done(reply.to_string())
    }
}

/// When the person last wrote in this chat.
pub(super) async fn last_user_message_at(scope: &TurnScope, context_id: &str) -> Option<chrono::DateTime<chrono::Utc>> {
    let messages = scope.state.db.list_messages(context_id, 100).await.unwrap_or_default();
    messages.iter().rev().find(|m| m.sender_role == "user").and_then(|m| m.created_at)
}

/// Moves every queued teammate that waits for another queued one behind it; the rest keep
/// their order. Dependencies have no cycles ([`crate::router::handoff_dependencies`]).
fn order_dependents<T>(queue: &mut std::collections::VecDeque<(Kwant, String, String, Option<T>)>)
where
    T: AsRef<str>,
{
    for _ in 0..queue.len() {
        let misplaced = queue.iter().enumerate().find_map(|(i, (_, _, _, after))| {
            let p = after.as_ref()?;
            let at = queue.iter().position(|(k, _, _, _)| k.id == p.as_ref())?;
            (at > i).then_some((i, at))
        });
        let Some((i, at)) = misplaced else { return };
        if let Some(item) = queue.remove(i) {
            queue.insert(at, item);
        }
    }
}

const IN_CODE_BLOCK: &str = "the mention was inside code. Write the name as plain text to ask them";
/// A follow-up that only credited a teammate who just reported ("As @SEO found, …").
const CREDIT_ONLY: &str = "the mention only credited their result. To hand them more work, @mention them with the task";

/// Says in the chat, and logs, the @mentions of one reply that handed nothing on, as ONE line
/// under it: "Not sent to Social, Writer: <why>", or "Not sent to Social: <why>; Writer: <why>"
/// when the reasons differ. The note is in the chat's history, so the speaker reads it on its next
/// turn. Reasons don't repeat the name. An empty reason means no teammate has that name; it's
/// worked out here.
async fn report_skipped(scope: &TurnScope, skipped: Vec<(String, String)>) {
    let mut said: Vec<String> = Vec::new();
    // (reason, names), in order of first mention
    let mut groups: Vec<(String, Vec<String>)> = Vec::new();
    for (name, why) in skipped {
        if said.iter().any(|n| n.eq_ignore_ascii_case(&name)) {
            continue;
        }
        said.push(name.clone());
        let why = if why.is_empty() { unknown_mention(scope, &name).await } else { why };
        tracing::info!(context = ?scope.channel_id.as_ref().or(scope.thread_id.as_ref()), mention = %name, "mention handed nothing on: {why}");
        match groups.iter_mut().find(|(w, _)| *w == why) {
            Some((_, names)) => names.push(name),
            None => groups.push((why, vec![name])),
        }
    }
    let Some(text) = refusal_line(&groups) else { return };
    let meta = serde_json::json!({"kind": "handoff_refused", "mention": said[0], "mentions": said});
    if let Some(saved) = scope.save(&scope.message("system", "kwnt", text), Some(meta)).await {
        let _ = scope
            .state
            .event_sender
            .send(RuntimeEvent::MessageAdded { message: serde_json::to_value(&saved).unwrap_or_default() });
    }
}

/// "Not sent to A, B: why" for refused mentions grouped by reason; `None` when there are none.
pub fn refusal_line(groups: &[(String, Vec<String>)]) -> Option<String> {
    let parts: Vec<String> = groups.iter().map(|(why, names)| format!("{}: {why}", names.join(", "))).collect();
    (!parts.is_empty()).then(|| format!("Not sent to {}", parts.join("; ")))
}

/// Why `@name` matched no one in this chat.
async fn unknown_mention(scope: &TurnScope, name: &str) -> String {
    let space = scope.teammates.first().map(|k| SpaceId::new(&k.space_id));
    let in_space = match &space {
        Some(space) => scope.state.db.list_kwants(space).await.unwrap_or_default().iter().any(|k| k.name.eq_ignore_ascii_case(name)),
        None => false,
    };
    if in_space {
        "not in this chat".to_string()
    } else {
        "no teammate has that name here".to_string()
    }
}

/// Whether `kwant` leads the team channel this turn is in (a channel turn, not a thread).
async fn is_team_channel_lead(scope: &TurnScope, kwant: &Kwant) -> bool {
    let (Some(cid), None, Some(_)) = (&scope.channel_id, &scope.thread_id, &scope.team_keys) else {
        return false;
    };
    match scope.state.db.get_channel(cid).await {
        Ok(Some(channel)) => channel.name.starts_with("team-") && channel.lead_kwant_id == kwant.id,
        _ => false,
    }
}

/// A team lead's @mentions in its team channel hand work on as delegations, not handoff turns:
/// each teammate gets the task in its own thread (Sent-to chip, queue, the same guards as the
/// delegate tool), and when the last one ends the lead is resumed once with every result to give
/// the user one answer; that follow-up may hand on again, also to `just_reported` teammates (whose
/// results it is about): a mention there is a correction or the next step, unless it only credits
/// them ([`crate::router::only_credited`]). One user message starts at most
/// [`MAX_DELEGATIONS_PER_REQUEST`], at most [`MAX_MENTION_DELEGATIONS_PER_BOT`] to any one
/// teammate, which is what ends a loop; mentions past a cap wait on a Continue notice
/// ([`super::budget::pause_capped`]). Every other mention that hands nothing on is said in the
/// channel. Teammates the lead already delegated to from this channel (with the tool) aren't sent
/// it twice. `resumed`: the person pressed Continue on such a notice: only these teammates are
/// handed the work, with a fresh allowance.
pub(super) async fn delegate_mentions(
    scope: &TurnScope,
    lead: &Kwant,
    text: &str,
    user_prompt: &str,
    just_reported: &[String],
    resumed: Option<&[String]>,
) {
    let Some(cid) = scope.channel_id.clone() else { return };
    let names: Vec<&str> = scope.teammates.iter().map(|k| k.name.as_str()).collect();
    let found = crate::router::handoff_mentions(text, &names);
    // "@Writer, once @Analyst has the numbers…": the Writer's task waits for the Analyst's result
    let after_of: std::collections::HashMap<String, String> = crate::router::handoff_dependencies(text, &names)
        .into_iter()
        .map(|(d, p)| (scope.teammates[d].id.clone(), scope.teammates[p].id.clone()))
        .collect();
    let mut skipped: Vec<(String, String)> = found.unknown.into_iter().map(|n| (n, String::new())).collect();
    skipped.extend(found.in_code_block.iter().map(|&i| (scope.teammates[i].name.clone(), IN_CODE_BLOCK.to_string())));
    if found.targets.iter().all(|&i| scope.teammates[i].id == lead.id) {
        return report_skipped(scope, skipped).await;
    }
    let mine: Vec<_> = scope
        .state
        .db
        .list_delegations(Some(&lead.id), None, 50)
        .await
        .unwrap_or_default()
        .into_iter()
        .filter(|d| d.origin_channel_id.as_deref() == Some(cid.as_str()))
        .collect();
    let open: Vec<&str> = mine.iter().filter(|d| d.is_open()).map(|d| d.specialist_kwant_id.as_str()).collect();
    // Delegations this user message already started, counting earlier follow-up rounds
    let messages = scope.state.db.list_messages(&cid, 100).await.unwrap_or_default();
    let since = messages.iter().rev().find(|m| m.sender_role == "user").or(messages.first()).and_then(|m| m.created_at);
    let this_request: Vec<_> = match resumed {
        Some(_) => Vec::new(),
        None => mine.iter().filter(|d| d.continue_lead && since.is_some_and(|t| d.created_at >= t)).collect(),
    };
    let mut room = MAX_DELEGATIONS_PER_REQUEST.saturating_sub(this_request.len());
    let asked = |id: &str| this_request.iter().filter(|d| d.specialist_kwant_id == id).count();

    let mut targets: Vec<&Kwant> = Vec::new();
    // Past a cap: they wait on a Continue notice instead of a dead end
    let mut capped: Vec<&Kwant> = Vec::new();
    for &i in &found.targets {
        let k = &scope.teammates[i];
        if k.id == lead.id || targets.iter().any(|t| t.id == k.id) || resumed.is_some_and(|ids| !ids.contains(&k.id)) {
            continue;
        }
        if just_reported.contains(&k.id) && crate::router::only_credited(text, &k.name) {
            skipped.push((k.name.clone(), CREDIT_ONLY.to_string()));
        } else if open.contains(&k.id.as_str()) {
            skipped.push((k.name.clone(), "still working on an earlier task from this chat".to_string()));
        } else if asked(&k.id) >= MAX_MENTION_DELEGATIONS_PER_BOT {
            capped.push(k);
        } else {
            targets.push(k);
        }
    }
    // Teammates waited for start first, so their delegation exists when the waiting one is created
    for _ in 0..targets.len() {
        let misplaced = targets.iter().enumerate().find_map(|(i, k)| {
            let at = targets.iter().position(|t| Some(&t.id) == after_of.get(&k.id))?;
            (at > i).then_some((i, at))
        });
        let Some((i, at)) = misplaced else { break };
        let k = targets.remove(i);
        targets.insert(at, k);
    }
    let mut started: std::collections::HashMap<String, String> = std::collections::HashMap::new();
    let user = truncate_chars(user_prompt.trim(), 3000);
    let task = format!(
        "The user asked in the team channel:\n{user}\n\n{lead} mentioned you:\n{text}",
        lead = lead.name,
        text = truncate_chars(text.trim(), 4000),
    );
    let title = truncate_chars(user.lines().find(|l| !l.trim().is_empty()).unwrap_or("Task").trim(), 80).to_string();
    let allowed: Vec<String> = scope.teammates.iter().map(|k| k.id.clone()).collect();
    for (n, specialist) in targets.into_iter().enumerate() {
        if n >= MAX_HANDOFFS {
            skipped.push((specialist.name.clone(), format!("a message hands work to at most {MAX_HANDOFFS} teammates")));
            continue;
        }
        if room == 0 {
            capped.push(specialist);
            continue;
        }
        // Waits for a teammate started here, or for one still on an earlier task from this chat
        let after = match after_of.get(&specialist.id) {
            Some(p) => {
                let earlier = || mine.iter().find(|d| d.is_open() && &d.specialist_kwant_id == p).map(|d| d.id.clone());
                match started.get(p).cloned().or_else(earlier) {
                    Some(id) => Some(id),
                    None if targets_mentioned(&found.targets, scope, p) => {
                        let name = scope.teammates.iter().find(|k| &k.id == p).map(|k| k.name.clone()).unwrap_or_default();
                        skipped.push((specialist.name.clone(), format!("it was to wait for {name}'s result, and {name} wasn't sent the task")));
                        continue;
                    }
                    None => None,
                }
            }
            None => None,
        };
        let req = super::delegation::StartRequest {
            lead: lead.clone(),
            specialist: specialist.clone(),
            task: task.clone(),
            title: Some(title.clone()),
            origin: super::delegation::Origin { channel_id: Some(cid.clone()), ..Default::default() },
            continue_lead: true,
            origin_depth: scope.delegation_depth,
            allowed: Some(allowed.clone()),
            after,
        };
        match super::delegation::start(&scope.state, req).await {
            Ok(d) => {
                started.insert(specialist.id.clone(), d.id);
                room -= 1;
            }
            Err((_, why)) => skipped.push((specialist.name.clone(), why.trim_end_matches('.').to_string())),
        }
    }
    report_skipped(scope, skipped).await;
    if !capped.is_empty() {
        super::budget::pause_capped(scope, lead, &capped, this_request.len() + started.len(), text, user_prompt).await;
    }
}

/// Whether the teammate with `id` is among the @mentioned `targets` (indices into the teammates).
fn targets_mentioned(targets: &[usize], scope: &TurnScope, id: &str) -> bool {
    targets.iter().any(|&i| scope.teammates[i].id == id)
}

/// Runs a moderated team discussion: the lead opens (or pauses to ask the user), teammates react to
/// each other briefly or pass, @mentioned teammates join in, and the lead closes with one summary.
async fn run_discussion(scope: &TurnScope, team: &Team, planned: Vec<Kwant>, user_prompt: &str) {
    use std::collections::{HashMap, VecDeque};

    let events = scope.state.event_sender.clone();
    let Some(lead) = planned.first().cloned() else {
        return;
    };
    let responders: Vec<String> = planned.iter().skip(1).map(|k| k.name.clone()).collect();
    let mut queue: VecDeque<(Kwant, Option<String>)> = planned.iter().cloned().map(|k| (k, None)).collect();

    let mut plan = vec![format!("{}: opens with a proposal", lead.name)];
    plan.extend(responders.iter().map(|n| format!("{}: reacts and adds what's missing", n)));
    plan.push(format!("{}: summary for you", lead.name));
    let _ = events.send(RuntimeEvent::DiscussionStarted {
        team_id: team.id.clone(),
        team_name: team.name.clone(),
        total_turns: queue.len() + 1,
        speakers: planned.iter().map(|k| k.id.clone()).collect(),
        plan,
        kind: "discussion".into(),
    });

    let names: Vec<&str> = scope.teammates.iter().map(|k| k.name.as_str()).collect();
    let mut spoke: HashMap<String, usize> = HashMap::new();
    let mut contributors: Vec<String> = Vec::new();
    let mut previous: Option<Kwant> = None;
    let mut turn = 0;
    let mut waiting_for_user = false;

    while let Some((kwant, asked_by)) = queue.pop_front() {
        if scope.cancel_token.is_cancelled() || turn >= discussion::MAX_TURNS {
            break;
        }
        if previous.as_ref().is_some_and(|p| p.id == kwant.id) {
            continue;
        }
        turn += 1;
        let opening = turn == 1;
        let prompt = if opening {
            discussion::opening_prompt(user_prompt, &team.name, &responders)
        } else {
            let prev = previous.as_ref().map(|p| p.name.as_str()).unwrap_or(&lead.name);
            discussion::reply_prompt(user_prompt, &team.name, prev, asked_by.as_deref())
        };
        let _ = events.send(RuntimeEvent::DiscussionTurnStarted {
            speaker_id: kwant.id.clone(),
            speaker_name: kwant.name.clone(),
            turn_index: turn,
            total_turns: turn + queue.len().min(discussion::MAX_TURNS - turn) + 1,
            prompt: prompt.clone(),
            asked_by: asked_by.clone(),
            phase: Some(if opening { "open" } else { "reply" }.into()),
        });
        let opts = TurnOpts {
            label: Some(discussion::turn_label(turn, &team.name)),
            // Only the opening turn may navigate the bot browser for a URL in the user's message
            nav_prompt: Some(if opening { user_prompt.to_string() } else { String::new() }),
            discussion: Some(opening.then(|| discussion::question_label(&kwant.name, &team.name))),
            routing: None,
            on_started: None,
            ..Default::default()
        };
        let Some(raw) = run_kwant_turn(scope, &kwant, prompt, opts).await else {
            if scope.cancel_token.is_cancelled() {
                break;
            }
            continue;
        };
        match discussion::parse_reply(&raw) {
            discussion::Reply::Pass => {
                let _ = events.send(RuntimeEvent::ThinkingDelta(format!("{} has nothing to add", kwant.name)));
            }
            discussion::Reply::AskUser(_) if opening => {
                waiting_for_user = true;
                break;
            }
            discussion::Reply::AskUser(text) | discussion::Reply::Speak(text) => {
                *spoke.entry(kwant.id.clone()).or_default() += 1;
                if !contributors.contains(&kwant.name) {
                    contributors.push(kwant.name.clone());
                }
                for i in crate::router::handoff_targets(&text, &names) {
                    let k = &scope.teammates[i];
                    let turns_taken = spoke.get(&k.id).copied().unwrap_or(0);
                    if k.id != kwant.id
                        && turns_taken < discussion::MAX_TURNS_PER_KWANT
                        && !queue.iter().any(|(q, _)| q.id == k.id)
                    {
                        queue.push_back((k.clone(), Some(kwant.name.clone())));
                    }
                }
                previous = Some(kwant.clone());
            }
        }
    }

    if !waiting_for_user && !scope.cancel_token.is_cancelled() && contributors.len() >= 2 {
        turn += 1;
        let prompt = discussion::summary_prompt(user_prompt, &contributors);
        let _ = events.send(RuntimeEvent::DiscussionTurnStarted {
            speaker_id: lead.id.clone(),
            speaker_name: lead.name.clone(),
            turn_index: turn,
            total_turns: turn,
            prompt: prompt.clone(),
            asked_by: None,
            phase: Some("summary".into()),
        });
        let opts = TurnOpts {
            label: Some(discussion::summary_label(&team.name, &contributors)),
            nav_prompt: Some(String::new()),
            discussion: None,
            routing: None,
            on_started: None,
            ..Default::default()
        };
        run_kwant_turn(scope, &lead, prompt, opts).await;
    }

    let _ = events.send(RuntimeEvent::DiscussionCompleted { total_turns: turn, context_id: Some(team.id.clone()) });
}

/// Per-turn tweaks for turns that kwnt drives itself (team discussions).
#[derive(Default)]
pub(super) struct TurnOpts {
    /// Saved as the message's `reasoning_content` instead of the driver's reasoning.
    pub(super) label: Option<String>,
    /// Text checked for a URL to open in the bot browser; defaults to the prompt.
    nav_prompt: Option<String>,
    /// Discussion reply handling: `[PASS]` replies aren't saved; `Some(label)` lets an `[ASK_USER]`
    /// reply be saved as a question for the user under that label.
    pub(super) discussion: Option<Option<String>>,
    /// How this reply's agent was picked, saved as the reply's `meta.routing`.
    routing: Option<serde_json::Value>,
    /// Awaited once the turn has its slot and is running, before the driver starts; a turn
    /// that is stopped while queued never calls it.
    pub(super) on_started: Option<futures::future::BoxFuture<'static, ()>>,
    /// The retry of a turn a usage limit stopped (see [`super::parked`]): it never parks again.
    pub(super) retried: bool,
    /// The delegation this turn works on; a parked retry restarts the delegation.
    pub(super) delegation_id: Option<String>,
    /// (agent, effort) to run on instead of asking Laya: a turn resumed after a late Allow
    /// keeps those of the turn that asked.
    routed: Option<(String, String)>,
    /// Every call that could change something asks the person, even with approvals off (an
    /// owner turn kwnt starts on its own; see [`super::loose_ends_owner`]).
    pub(super) always_ask: bool,
    /// Shared with the save step: holds a reply with open choices, and gets the saved message
    /// (see [`super::decide`]).
    pub(super) hook: Option<super::decide::HookSlot>,
    /// Merged into the saved reply's meta.
    pub(super) extra_meta: Option<serde_json::Value>,
    /// The reply isn't saved (a teammate's view for the decide-first round).
    pub(super) unsaved: bool,
    /// A failed turn saves no `turn_failed` note (the caller posts something else).
    pub(super) quiet_failure: bool,
    /// Shown as the turn's `purpose` in `GET /api/turns` (`turns[]`).
    pub(super) purpose: Option<String>,
}

impl TurnOpts {
    /// Options that run the turn on `agent` at `effort` (both needed), skipping Laya.
    pub(super) fn routed_as(agent: Option<&str>, effort: Option<&str>) -> Self {
        let routed = agent.zip(effort).map(|(a, e)| (a.to_string(), e.to_string()));
        TurnOpts { routed, ..Default::default() }
    }

    /// Options for a turn nobody asked for, saved under `label`: every call that could change
    /// something asks (see [`TurnOpts::always_ask`]).
    pub(super) fn always_asking(label: &str) -> Self {
        TurnOpts { label: Some(label.to_string()), always_ask: true, ..Default::default() }
    }
}

/// Runs one kwant turn: picks model/effort, syncs the bot browser, streams the reply and saves it.
/// Returns the raw reply text, or `None` if the turn was cancelled or failed.
async fn run_kwant_turn(
    scope: &TurnScope,
    target_kwant: &Kwant,
    prompt: String,
    opts: TurnOpts,
) -> Option<String> {
    try_kwant_turn(scope, target_kwant, prompt, opts).await.ok()
}

/// Longest failure reason kept for a routine run's history.
const MAX_FAILURE_CHARS: usize = 500;

/// [`run_kwant_turn`], but a failed turn says why: the driver's error or the provider's refusal.
/// The turn first waits for a slot under the per-agent and server-wide caps (see [`super::turns`]).
/// A reply that ends by asking the person to choose may be held and decided first (see
/// [`super::decide`]); then the decided reply's text comes back.
pub(super) async fn try_kwant_turn(
    scope: &TurnScope,
    target_kwant: &Kwant,
    prompt: String,
    mut opts: TurnOpts,
) -> Result<String, String> {
    let system = super::decide::system_turn(&opts);
    let Some(person) = super::decide::eligible(scope, target_kwant, system).await else {
        return turn_once(scope, target_kwant, prompt, opts).await;
    };
    let hook = std::sync::Arc::new(std::sync::Mutex::new(super::decide::Hook {
        hold_choices: true,
        teammates: scope.teammates.iter().map(|k| k.name.clone()).collect(),
        person_hard: super::decide::person_asked_hard(&person),
        ..Default::default()
    }));
    opts.hook = Some(hook.clone());
    let reply = turn_once(scope, target_kwant, prompt, opts).await;
    let held = hook.lock().ok().and_then(|mut h| h.held.take());
    match (reply, held) {
        (Ok(_), Some(held)) => Ok(super::decide::decide(scope, target_kwant, held).await),
        (reply, _) => reply,
    }
}

/// One turn, without the decide-first gate.
pub(super) async fn turn_once(
    scope: &TurnScope,
    target_kwant: &Kwant,
    prompt: String,
    opts: TurnOpts,
) -> Result<String, String> {
    let tag = crate::drivers::events::TurnTag {
        turn_id: uuid::Uuid::new_v4().to_string(),
        kwant_id: target_kwant.id.clone(),
        context_id: scope.context_id(target_kwant),
        thread_id: scope.thread_id.clone(),
    };
    let turns = &scope.state.turns;
    turns.add_keys(&scope.job_id, std::slice::from_ref(&tag.turn_id));
    // Sends turn_ended even if this future is dropped, queued or mid-turn
    let mut ended = TurnEndedGuard {
        sender: scope.state.event_sender.clone(),
        turns: turns.clone(),
        tag: tag.clone(),
        outcome: "cancelled",
        running_row: None,
    };
    let Some(_slot) = turns.acquire(&tag, &scope.cancel_token, &scope.state.event_sender).await else {
        return Err("stopped by the user".into());
    };
    turns.set_turn(super::turns::ActiveTurn {
        turn_id: tag.turn_id.clone(),
        kwant_id: tag.kwant_id.clone(),
        context_id: tag.context_id.clone(),
        thread_id: tag.thread_id.clone(),
        state: "running".into(),
        since: chrono::Utc::now(),
        purpose: opts.purpose.clone(),
    });
    // Kept until the turn ends, so a restart mid-reply is said in the chat (see interrupt_running_turns)
    let running = crate::db::continuity::RunningTurn {
        turn_id: tag.turn_id.clone(),
        kwant_id: target_kwant.id.clone(),
        kwant_name: target_kwant.name.clone(),
        channel_id: scope.channel_id.clone(),
        // Where scope.save puts the reply
        direct_kwant_id: scope.kwant_id.clone(),
        thread_id: scope.thread_id.clone(),
        started_at: chrono::Utc::now(),
    };
    if scope.state.db.add_running_turn(&running).await.is_ok() {
        ended.running_row = Some(scope.state.db.clone());
    }
    let _ = scope.state.event_sender.send(RuntimeEvent::TurnStarted {
        kwant_id: target_kwant.id.clone(),
        context_id: Some(tag.context_id.clone()),
        turn_id: Some(tag.turn_id.clone()),
        thread_id: tag.thread_id.clone(),
    });
    if scope.cancel_token.is_cancelled() {
        return Err("stopped by the user".into());
    }
    let mut opts = opts;
    if let Some(started) = opts.on_started.take() {
        started.await;
    }
    // A discussion round's reply isn't saved as is, so it isn't followed up; nor is a held one
    let follow_up = opts.discussion.is_none() && !opts.unsaved;
    let hook = opts.hook.clone();
    let reply = run_kwant_turn_inner(scope, target_kwant, prompt, opts, &tag).await;
    let follow_up = follow_up && !hook.is_some_and(|h| h.lock().is_ok_and(|h| h.held.is_some()));
    if scope.cancel_token.is_cancelled() {
        super::questions::cancel_for_turn(&scope.state, &tag.turn_id).await;
        super::human_steps::cancel_for_turn(&scope.state, &tag.turn_id).await;
    } else if let (Ok(text), true) = (&reply, follow_up) {
        super::waiting::turn_ended(scope, target_kwant, &tag.turn_id, text).await;
    }
    ended.outcome = match (&reply, scope.cancel_token.is_cancelled()) {
        (Ok(_), _) => "completed",
        (Err(_), true) => "cancelled",
        (Err(_), false) => "failed",
    };
    reply.map_err(|why| truncate_chars(&scrub_urls(why.trim()), MAX_FAILURE_CHARS).to_string())
}

/// Says in each chat which reply a previous run of kwnt cut off by stopping mid-turn. Runs once at
/// startup, before any new turn can start. The note's meta is
/// `{"kind":"turn_interrupted","interrupted":true,"kwant_id","kwant_name","turn_id","started_at"}`.
pub async fn interrupt_running_turns(state: &AppState) -> usize {
    let Ok(turns) = state.db.take_running_turns().await else {
        return 0;
    };
    for t in &turns {
        let msg = NewMessage {
            channel_id: t.channel_id.clone(),
            direct_kwant_id: t.direct_kwant_id.clone(),
            sender_role: "system".into(),
            sender_id: "kwnt".into(),
            text: format!("This reply from {} was interrupted by a restart. Send your message again to retry.", t.kwant_name),
            reasoning_content: None,
            tool_calls: None,
            tokens_used: 0,
        };
        let meta = json!({
            "kind": "turn_interrupted",
            "interrupted": true,
            "kwant_id": t.kwant_id,
            "kwant_name": t.kwant_name,
            "turn_id": t.turn_id,
            "started_at": t.started_at,
        });
        let _ = state.db.insert_message_with(&msg, &MessageExtra { thread_id: t.thread_id.clone(), meta: Some(meta) }).await;
        if let Some(tid) = &t.thread_id {
            let _ = state.db.touch_thread(tid).await;
        }
    }
    if !turns.is_empty() {
        tracing::info!(count = turns.len(), "marked chat replies cut short by a restart as interrupted");
    }
    turns.len()
}

/// `text` with every URL cut to scheme and host, so a failure reason that names a custom
/// endpoint can be stored without the credentials some endpoints carry in the userinfo,
/// path or query.
fn scrub_urls(text: &str) -> String {
    text.split(' ')
        .map(|word| {
            let Some(scheme_end) = word.find("://") else {
                return word.to_string();
            };
            let (scheme, rest) = word.split_at(scheme_end + 3);
            let host_end = rest.find(['/', '?', '#']).unwrap_or(rest.len());
            let (authority, tail) = rest.split_at(host_end);
            let host = authority.rsplit_once('@').map_or(authority, |(_, h)| h);
            if tail.is_empty() {
                format!("{scheme}{host}")
            } else {
                format!("{scheme}{host}/…")
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

struct TurnEndedGuard {
    sender: tokio::sync::broadcast::Sender<RuntimeEvent>,
    turns: super::turns::TurnRegistry,
    tag: crate::drivers::events::TurnTag,
    outcome: &'static str,
    /// Set once the turn is recorded as running; the record is removed when the turn ends.
    running_row: Option<crate::db::Database>,
}

impl Drop for TurnEndedGuard {
    fn drop(&mut self) {
        self.turns.end_turn(&self.tag.turn_id);
        if let (Some(db), Ok(rt)) = (self.running_row.take(), tokio::runtime::Handle::try_current()) {
            let turn_id = self.tag.turn_id.clone();
            rt.spawn(async move {
                let _ = db.remove_running_turn(&turn_id).await;
            });
        }
        let tag = std::mem::take(&mut self.tag);
        let _ = self.sender.send(RuntimeEvent::TurnEnded {
            kwant_id: tag.kwant_id,
            context_id: Some(tag.context_id),
            outcome: self.outcome.to_string(),
            turn_id: Some(tag.turn_id),
            thread_id: tag.thread_id,
        });
    }
}

/// Sends a turn's events tagged with the turn and its chat.
struct TurnEvents {
    sender: tokio::sync::broadcast::Sender<RuntimeEvent>,
    tag: crate::drivers::events::TurnTag,
    /// The turn's live steps, fed by the driver's events.
    activity: std::sync::Arc<super::activity::TurnActivity>,
}

impl TurnEvents {
    fn send(&self, event: RuntimeEvent) {
        let _ = self.sender.send(RuntimeEvent::Scoped { tag: self.tag.clone(), event: Box::new(event) });
    }
}

/// The engine and model a reply ran on, saved in its `meta.ran_on`.
struct RanOn {
    engine: crate::drivers::engines::EngineId,
    profile_id: Option<String>,
    /// The account's name, or "This machine" for the host's own CLI.
    account: Option<String>,
    choice: Option<crate::drivers::engines::ModelChoice>,
}

impl RanOn {
    /// `{engine, engine_name, family, model, model_label, tier, picked_by, effort, profile_id, account}`;
    /// `driver_model` is what the driver itself chose when nothing was picked for it.
    fn to_json(&self, driver_model: Option<&str>, picked_by: &str, effort: &str) -> serde_json::Value {
        let family = self.engine.family().map_or("ollama", |f| f.as_str());
        let (model, label, tier) = match (&self.choice, driver_model) {
            (Some(c), _) => (Some(c.id.clone()), Some(c.label.clone()), c.tier),
            (None, Some(m)) => (Some(m.to_string()), None, crate::drivers::engines::tier_from_name(m)),
            (None, None) => (None, None, None),
        };
        serde_json::json!({
            "engine": self.engine.as_str(),
            "engine_name": self.engine.name(),
            "family": family,
            "model": model,
            "model_label": label,
            "tier": tier,
            "picked_by": picked_by,
            "effort": effort,
            "profile_id": self.profile_id,
            "account": self.account,
        })
    }
}

/// Setting: seconds a streaming model may stay silent (outside tool runs) before its turn is stopped.
pub const STALL_SETTING: &str = "turn_stall_secs";
pub const DEFAULT_STALL_SECS: u64 = 120;

/// The stall limit from settings: missing or unreadable means the default, 0 turns it off.
async fn stall_limit(db: &crate::db::Database) -> Option<std::time::Duration> {
    let secs = db
        .get_setting(STALL_SETTING)
        .await
        .ok()
        .flatten()
        .and_then(|v| v.trim().parse::<u64>().ok())
        .unwrap_or(DEFAULT_STALL_SECS);
    (secs > 0).then(|| std::time::Duration::from_secs(secs))
}

async fn run_kwant_turn_inner(
    scope: &TurnScope,
    target_kwant: &Kwant,
    prompt: String,
    opts: TurnOpts,
    tag: &crate::drivers::events::TurnTag,
) -> Result<String, String> {
    let state = &scope.state;
    let event_broadcast = TurnEvents {
        sender: state.event_sender.clone(),
        tag: tag.clone(),
        activity: super::activity::TurnActivity::start(state.event_sender.clone(), tag.clone()),
    };
    let park_prompt = prompt.clone();
    let db = state.db.clone();
    let router = state.router.clone();
    let channel_id = scope.channel_id.clone();
    let kwant_id = scope.kwant_id.clone();
    let team_keys = &scope.team_keys;
    let cancel_token = &scope.cancel_token;
    // A bot outside a team still gets a folder of its own to write files in
    let workspace_dir = scope.workspace_dir.clone().or_else(|| {
        let space = SpaceId::new(&target_kwant.space_id);
        Some(crate::tools::bot_workspace(&crate::db::spaces::space_data_dir(&state.config.data_dir, &space), &target_kwant.id))
    });

    {
        let space = SpaceId::new(&target_kwant.space_id);
        let profile_engines = super::profile_turn::profile_engines(state, &space).await;
        let host_off = super::profile_turn::host_off(state, &space).await;
        let available_models =
            crate::drivers::detect_available_models(&db, &state.engines, &profile_engines, &host_off).await;
        // A bot pinned to Custom API runs on its account even when that account's last check
        // failed, so the reply says what is wrong with it (no account, server down, key refused)
        let pinned_custom = crate::router::parse_pin(&target_kwant.preferred_model).family.as_deref()
            == Some(crate::profiles::Engine::Custom.as_str());
        if available_models.is_empty() && !pinned_custom {
            // Say why when it's this space's own switch, so nobody hunts for a broken engine
            let (text, meta) = crate::drivers::engines::no_engine_notice(&target_kwant.name, !host_off.is_empty());
            event_broadcast.send(RuntimeEvent::TextDelta(text.clone()));
            event_broadcast.send(RuntimeEvent::TurnComplete {
                tokens: 0,
                duration_ms: 0,
                kwant_id: Some(target_kwant.id.clone()),
                profile_id: None,
            });
            let msg = NewMessage {
                reasoning_content: opts.label.clone(),
                ..scope.message("kwant", &target_kwant.name, text.clone())
            };
            scope.save(&msg, Some(meta)).await;
            return Err(text);
        }
        let route = match &opts.routed {
            Some((agent, effort)) => Ok(crate::router::RouteDecision {
                selected_model: agent.clone(),
                effort_level: effort.clone(),
                confidence: 1.0,
                provider_endpoint: None,
                is_fallback: false,
                tier: None,
                model: None,
                reasoning: "Same agent and effort as the turn that asked for this tool".into(),
            }),
            None => router.route_prompt_with_models(&prompt, "chat", &available_models).await,
        };
        let route = match route {
            Ok(r) => r,
            Err(_) => crate::router::RouteDecision {
                selected_model: target_kwant.preferred_model.clone(),
                effort_level: "medium".into(),
                confidence: 0.8,
                provider_endpoint: None,
                is_fallback: true,
                tier: None,
                model: None,
                reasoning: "Fallback routing".into(),
            },
        };
        let mut route = route;
        if opts.routed.is_none() {
            if let Some(capped) = crate::router::fallback::cap_effort_for_attachment(&prompt, &route.effort_level) {
                route.reasoning = format!("{} · effort capped at {capped}: a short message with an attachment", route.reasoning);
                route.effort_level = capped.into();
            }
        }
        let route = crate::router::apply_preferred_model(route, &target_kwant.preferred_model);

        let target_model = route.selected_model.clone();
        // Flight School (D7): a task below act never runs on an engine kwant can't gate
        if let Some(text) = Box::pin(super::missions::flight_school::refuse_turn(state, target_kwant, &target_model)).await {
            event_broadcast.send(RuntimeEvent::TextDelta(text.clone()));
            event_broadcast.send(RuntimeEvent::TurnComplete { tokens: 0, duration_ms: 0, kwant_id: Some(target_kwant.id.clone()), profile_id: None });
            return Err(text);
        }
        let effort_level = route.effort_level.clone();
        let tier = route.tier.unwrap_or(crate::drivers::engines::Tier::Balanced);
        let pinned_model = route.model.clone();
        let pinned_family = crate::router::parse_pin(&target_kwant.preferred_model).family.is_some();

        event_broadcast.send(RuntimeEvent::ThinkingDelta(format!(
            "🔮 Laya Router: model='{}', effort='{}' ({:.0}% conf) — {}",
            target_model,
            effort_level,
            route.confidence * 100.0,
            route.reasoning
        )));

        let context_id = tag.context_id.clone();

        let mut history = db.list_messages(&context_id, 20).await.unwrap_or_default();
        crate::drivers::drop_current_message(&mut history, &prompt);

        let configured_key = get_configured_key(&db, &target_model).await;
        let endpoint = get_configured_endpoint(&db, &target_model).await;

        let sandbox_target = team_keys
            .as_ref()
            .map(|(id, _)| id.clone())
            .or_else(|| channel_id.clone())
            .or_else(|| kwant_id.clone())
            .unwrap_or_else(|| target_kwant.id.clone());

        let active_ctrl = state.browser_controllers.lock().await.get(&sandbox_target).cloned();
        // The user may have browsed in the live viewer since the last turn; read the real location
        let active_browser_url = if let Some(ref ctrl) = active_ctrl {
            let _ = ctrl.sync_location().await;
            ctrl.current_url.lock().await.clone()
        } else {
            None
        };

        let mut final_prompt = prompt.clone();
        let nav_prompt = opts.nav_prompt.as_deref().unwrap_or(&prompt);
        let target_url_opt = detect_url_with_context(nav_prompt, active_browser_url.as_deref());

        if let Some(target_url) = target_url_opt {
            event_broadcast.send(RuntimeEvent::ThinkingDelta(format!(
                "🌐 Navigating bot browser to {}...",
                target_url
            )));
            if let Some(ctrl) = get_or_create_browser_controller(state, &sandbox_target).await {
                match ctrl.navigate(&target_url).await {
                Ok(nav) => {
                    let page_text = ctrl.extract_content().await.unwrap_or_default();
                    let preview = truncate_chars(&page_text, 3000);
                    event_broadcast.send(RuntimeEvent::BrowserNavigated {
                        team_id: sandbox_target.clone(),
                        url: nav.url.clone(),
                        title: nav.title.clone(),
                        status_code: nav.status_code,
                    });
                    if let Some(b64) = ctrl.last_screenshot.lock().await.clone() {
                        event_broadcast.send(RuntimeEvent::BrowserScreenshot {
                            team_id: sandbox_target.clone(),
                            url: nav.url.clone(),
                            image_base64: b64,
                            timestamp: chrono::Utc::now().to_rfc3339(),
                        });
                    }
                    let captcha = ctrl.probe_captcha().await.ok().flatten();
                    let wall = crate::sandbox::detect_bot_wall(&nav.url, &nav.title, &page_text).or(captcha.as_ref().map(|c| c.label()));
                    final_prompt = if let Some(kind) = wall {
                        format!(
                            "{}\n\n[Sandbox Browser State: Tried to open {} in the right-panel bot browser. {}]",
                            prompt, target_url, bot_wall_notice(kind, &nav.url)
                        )
                    } else {
                        let clickable = ctrl.list_clickables(30).await;
                        format!(
                            "{}\n\n[Sandbox Browser State: The right-panel bot browser is now on {}. Title: \"{}\".\nClickable (topmost first): {}\nPage Content Preview:\n{}\n\nUse this page content to answer the user's request; only describe what it actually shows. NEVER attempt to open a desktop browser window.]",
                            prompt, nav.url, nav.title, clickable.join(" | "), preview
                        )
                    };
                }
                Err(crate::sandbox::BrowserError::Navigation(reason)) => {
                    let shown_url = ctrl.current_url.lock().await.clone().unwrap_or_else(|| target_url.clone());
                    if let Some(b64) = ctrl.last_screenshot.lock().await.clone() {
                        event_broadcast.send(RuntimeEvent::BrowserScreenshot {
                            team_id: sandbox_target.clone(),
                            url: shown_url,
                            image_base64: b64,
                            timestamp: chrono::Utc::now().to_rfc3339(),
                        });
                    }
                    final_prompt = format!(
                        "{}\n\n[Sandbox Browser State: Tried to open {} in the right-panel bot browser, but the site could not be reached ({}). The browser itself works fine; do not retry or suggest restarting it. Briefly tell the user the address didn't resolve/load and, if it looks like a typo, suggest the likely intended address.]",
                        prompt, target_url, reason
                    );
                }
                Err(_) => {}
                }
            }
        } else if let Some(ref curr_url) = active_browser_url {
            if !curr_url.is_empty() && !curr_url.contains("about:blank") && !curr_url.contains("example.com") {
                let (title, page_text, clickable) = match active_ctrl {
                    Some(ref ctrl) => (
                        ctrl.current_title.lock().await.clone().unwrap_or_default(),
                        ctrl.extract_content().await.unwrap_or_default(),
                        ctrl.list_clickables(30).await,
                    ),
                    None => (String::new(), String::new(), Vec::new()),
                };
                final_prompt = format!(
                    "{}\n\n[Active Sandbox Browser State: The right-panel bot browser (shared live with the user, who may have clicked or typed in it) is currently viewing: {}. Title: \"{}\".\nClickable (topmost first, e.g. dialog buttons): {}\nPage Content Preview:\n{}\n\nMaintain context on this website. Do not suggest or switch to other services like YouTube unless explicitly requested.]",
                    prompt, curr_url, title, clickable.join(" | "), truncate_chars(&page_text, 3000)
                );
            }
        }

        // A routed model id like "claude-sonnet" still runs on Claude's accounts, never
        // straight on this machine's login
        let engine = crate::profiles::Engine::parse(&target_model)
            .or_else(|| crate::profiles::Engine::from_model(&target_model));
        let candidates = super::profile_turn::plan(state, engine, &target_kwant.id).await;
        // This machine turned off in the kwant's space: no host fallback, so nothing to try
        let host_off = match engine {
            Some(e) => super::profile_turn::host_off(state, &SpaceId::new(&target_kwant.space_id)).await.contains(&e),
            None => false,
        };
        let attempts: Vec<Option<LlmProfile>> = if candidates.is_empty() && host_off {
            Vec::new()
        } else if candidates.is_empty() {
            vec![None]
        } else {
            candidates.into_iter().map(Some).collect()
        };
        // The agents' own continuity tools, for engines that can call MCP tools
        let self_scope = (engine.is_some() && super::continuity::agent_tools_enabled(state).await).then(|| {
            super::continuity::SelfScope {
                kwant_id: target_kwant.id.clone(),
                kwant_name: target_kwant.name.clone(),
                origin: Some(super::continuity::TurnOrigin {
                    channel_id: channel_id.clone(),
                    direct_kwant_id: kwant_id.clone(),
                    thread_id: scope.thread_id.clone(),
                    team_id: scope.team_context.as_ref().map(|t| t.id.clone()),
                    member_ids: scope.teammates.iter().map(|k| k.id.clone()).collect(),
                    depth: scope.delegation_depth,
                }),
                cancel: Some(cancel_token.clone()),
                turn_id: Some(tag.turn_id.clone()),
                workspace: workspace_dir.clone(),
                sandbox_target: Some(sandbox_target.clone()),
            }
        });
        let card_opts = super::mcp_runtime::TurnApprovalOpts {
            routed: Some((target_model.clone(), effort_level.clone())),
            always_ask: opts.always_ask,
        };
        let mcp_turn = super::mcp_runtime::prepare_turn_with_opts(
            state,
            target_kwant,
            Some(tag.turn_id.clone()),
            workspace_dir.clone(),
            self_scope,
            card_opts,
        )
        .await;
        let mcp_ctx = mcp_turn.as_ref().map(|(ctx, _)| ctx.clone());
        if let Some(ctx) = &mcp_ctx {
            event_broadcast.activity.add_secrets(ctx.secret_values());
        }
        let with_tools = mcp_ctx
            .as_ref()
            .is_some_and(|c| c.servers.iter().any(|s| s.name == crate::mcp::SELF_SERVER_NAME));
        let continuity = super::continuity::prompt_block(
            state,
            target_kwant,
            scope.team_context.as_ref().map(|t| t.id.as_str()),
            channel_id.as_deref(),
            with_tools,
        )
        .await;
        let skills = super::skills::prompt_block(state, &target_kwant.id, &prompt, with_tools).await;
        let turn_input = TurnInput {
            browser_api_url: Some(format!(
                "http://127.0.0.1:{}/api/sandbox/{}",
                state.config.port, sandbox_target
            )),
            thread_id: context_id,
            kwant: target_kwant.clone(),
            prompt: final_prompt,
            history,
            model_override: Some(target_model.clone()),
            model: None,
            effort_level: Some(effort_level.clone()),
            team_context: scope.team_context.clone(),
            workspace_dir: workspace_dir.clone(),
            sandbox_id: Some(sandbox_target.clone()),
            teammates: scope
                .teammates
                .iter()
                .filter(|k| k.id != target_kwant.id)
                .map(|k| crate::drivers::Teammate { name: k.name.clone(), title: k.title.clone() })
                .collect(),
            mcp: mcp_ctx.clone(),
            continuity,
            skills,
            person: super::bot_actions::person_place(&state.db).await,
            store: Some(state.db.clone()),
        };

        let had_state = state.db.list_profile_states().await.unwrap_or_default();
        let stall_after = stall_limit(&state.db).await;
        let turn_start = std::time::Instant::now();
        // Notices about profile switches, kept with the saved reply
        let mut notices: Vec<String> = Vec::new();
        let mut cancelled = false;
        let mut driver_result = None;
        let mut driver_error: Option<RuntimeEvent> = None;
        let mut gave_up: Option<String> = None;
        // Saved with the gave-up reply, so a client can offer Retry without reading its wording
        let mut gave_up_meta: Option<serde_json::Value> = None;
        let mut ran_on: Option<RanOn> = None;
        if attempts.is_empty() {
            let text = format!(
                "⚠️ [{}] This machine is off in this space and no other {} account is on.",
                target_kwant.name,
                engine.map(|e| e.as_str()).unwrap_or("model")
            );
            event_broadcast.send(RuntimeEvent::TextDelta(text.clone()));
            event_broadcast.send(RuntimeEvent::TurnComplete {
                tokens: 0,
                duration_ms: turn_start.elapsed().as_millis() as u64,
                kwant_id: Some(target_kwant.id.clone()),
                profile_id: None,
            });
            gave_up = Some(text);
        }
        for (i, profile) in attempts.iter().enumerate() {
            let (driver, host_engine) = match super::profile_turn::driver_for(
                state,
                profile.as_ref(),
                &target_model,
                configured_key.clone(),
                endpoint.clone(),
            )
            .await
            {
                Ok(d) => d,
                // The host's own CLI or key can't run it: try the next profile, else say why
                Err(why) if i + 1 < attempts.len() => {
                    tracing::info!(kwant = %target_kwant.id, reason = %why, "skipping an engine that can't run");
                    continue;
                }
                Err(why) => {
                    let text = format!("⚠️ [{}] {why}", target_kwant.name);
                    event_broadcast.send(RuntimeEvent::TextDelta(text.clone()));
                    event_broadcast.send(RuntimeEvent::TurnComplete {
                        tokens: 0,
                        duration_ms: turn_start.elapsed().as_millis() as u64,
                        kwant_id: Some(target_kwant.id.clone()),
                        profile_id: profile.as_ref().map(|p| p.id.clone()),
                    });
                    gave_up = Some(text);
                    break;
                }
            };
            // The model for the engine this attempt runs on: the pinned one, else the effort's tier
            let runs_on = host_engine.or_else(|| profile.as_ref().and_then(super::profile_turn::profile_engine));
            let custom = profile.as_ref().filter(|p| p.kind == "custom_api");
            let choice = match custom {
                Some(p) => super::profile_turn::custom_model(state, p, pinned_model.as_deref(), tier).await,
                None => runs_on.and_then(|e| {
                    crate::drivers::engines::choose_model(
                        &crate::drivers::engines::known_models(&state.engines, e),
                        pinned_model.as_deref(),
                        tier,
                    )
                }),
            };
            // An API account whose models weren't listed here picks its own from the tier
            let unlisted_api = custom.is_none()
                && runs_on.is_some_and(|e| {
                    e.kind() == crate::drivers::engines::EngineKind::Api
                        && state.engines.report(e).models.is_empty()
                });
            let mut input = turn_input.clone();
            input.model = if unlisted_api { None } else { choice.as_ref().and_then(|c| c.run_id()).map(str::to_string) };
            if let Some(e) = runs_on {
                event_broadcast.send(RuntimeEvent::ThinkingDelta(format!(
                    "Running on {}{} · {}{}",
                    e.name(),
                    super::profile_turn::account_name(profile.as_ref(), e)
                        .map(|a| format!(" · {a}"))
                        .unwrap_or_default(),
                    input.model.as_deref().unwrap_or("default model"),
                    choice
                        .as_ref()
                        .and_then(|c| c.tier)
                        .map(|t| format!(" ({} tier)", t.as_str()))
                        .unwrap_or_default(),
                )));
            }
            if ran_on.is_none() {
                if let Some(note) = runs_on.and_then(|e| crate::drivers::activity::thinking_note(e.as_str())) {
                    event_broadcast.activity.note(note);
                }
            }
            ran_on = runs_on.map(|e| RanOn {
                engine: e,
                profile_id: profile.as_ref().map(|p| p.id.clone()),
                account: super::profile_turn::account_name(profile.as_ref(), e),
                choice: if unlisted_api { None } else { choice.clone() },
            });
            let stall_limit = if driver.streams_progress() { stall_after } else { None };
            let (driver_tx, mut driver_rx) = mpsc::channel(32);
            let driver_task =
                tokio::spawn(async move { driver.stream_turn(input, driver_tx).await });
            // Stall watchdog: silence while no tool runs, for longer than the limit, stops the turn
            let mut last_output = tokio::time::Instant::now();
            let mut tools_running = 0usize;
            let mut stalled = false;

            // The driver's TurnComplete is held back: it's re-sent with the kwant and profile,
            // or dropped when the turn moves to another profile.
            let mut held_complete = None;
            let mut side_effects = false;
            loop {
                let stall_at = stall_limit.filter(|_| tools_running == 0).map(|d| last_output + d);
                tokio::select! {
                    _ = cancel_token.cancelled() => {
                        cancelled = true;
                        driver_task.abort();
                        break;
                    }
                    _ = async {
                        match stall_at {
                            Some(at) => tokio::time::sleep_until(at).await,
                            None => std::future::pending().await,
                        }
                    } => {
                        stalled = true;
                        driver_task.abort();
                        break;
                    }
                    event_opt = driver_rx.recv() => {
                        last_output = tokio::time::Instant::now();
                        match &event_opt {
                            Some(RuntimeEvent::ToolCallPending { .. }) => tools_running += 1,
                            Some(RuntimeEvent::ToolCallCompleted { .. }) => tools_running = tools_running.saturating_sub(1),
                            _ => {}
                        }
                        match event_opt {
                            Some(RuntimeEvent::TurnComplete { tokens, duration_ms, .. }) => {
                                held_complete = Some((tokens, duration_ms));
                            }
                            // Only resets the stall clock (above)
                            Some(RuntimeEvent::Progress) => {}
                            // Lets a task handed in during the turn pay for it then (see `super::missions::spend_at_hand_in`)
                            Some(RuntimeEvent::UsageSoFar { fresh }) => state.turns.saw_usage(&tag.turn_id, fresh),
                            Some(event) => {
                                if let RuntimeEvent::ToolCallPending { name, args, .. } = &event {
                                    if name == super::questions::NATIVE_ASK_TOOL {
                                        let place = crate::db::questions::QuestionPlace {
                                            kwant_id: target_kwant.id.clone(),
                                            channel_id: scope.channel_id.clone(),
                                            direct_kwant_id: scope.kwant_id.clone(),
                                            thread_id: scope.thread_id.clone(),
                                            turn_id: Some(tag.turn_id.clone()),
                                        };
                                        super::questions::record_native(state, &place, &target_kwant.name, args).await;
                                    }
                                }
                                let visible = match &event {
                                    RuntimeEvent::TextDelta(t) => !t.trim().is_empty(),
                                    RuntimeEvent::ToolCallPending { .. } | RuntimeEvent::ApprovalRequired { .. } => true,
                                    _ => false,
                                };
                                side_effects |= visible;
                                event_broadcast.activity.observe(&event);
                                event_broadcast.send(event);
                            }
                            None => break,
                        }
                    }
                }
            }
            if cancelled {
                break;
            }
            if stalled {
                let secs = stall_limit.map_or(0, |d| d.as_secs());
                let text = format!(
                    "⚠️ [{}] The model sent nothing for {secs} s, so this reply was stopped. Send your message again to retry.",
                    target_kwant.name
                );
                tracing::warn!(kwant = %target_kwant.id, secs, "turn stopped by the stall watchdog");
                event_broadcast.send(RuntimeEvent::TextDelta(text.clone()));
                event_broadcast.send(RuntimeEvent::TurnComplete {
                    tokens: 0,
                    duration_ms: turn_start.elapsed().as_millis() as u64,
                    kwant_id: Some(target_kwant.id.clone()),
                    profile_id: profile.as_ref().map(|p| p.id.clone()),
                });
                gave_up = Some(text);
                gave_up_meta = Some(json!({
                    "kind": "turn_stalled",
                    "stalled": true,
                    "kwant_id": target_kwant.id,
                    "kwant_name": target_kwant.name,
                    "stall_secs": secs,
                }));
                break;
            }
            let result = driver_task.await;
            let refused = match &result {
                Ok(Ok(out)) => out.failure.clone(),
                _ => None,
            };
            if let Some(failure) = refused {
                if let Some(p) = profile {
                    super::profile_turn::record_failure(state, p, &failure).await;
                }
                if let Some(engine) = host_engine {
                    state.engines.record_failure(engine, &failure);
                }
                let next = attempts.get(i + 1).and_then(|n| n.as_ref());
                if let (Some(from), Some(to), false) = (profile, next, side_effects) {
                    let notice = super::profile_turn::switch_notice(from, to, &failure);
                    event_broadcast.send(RuntimeEvent::ProfileSwitched {
                        kwant_id: target_kwant.id.clone(),
                        from_profile_id: from.id.clone(),
                        from_name: from.name.clone(),
                        to_profile_id: to.id.clone(),
                        to_name: to.name.clone(),
                        class: failure.class.as_str().into(),
                        retry_at: failure.retry_at,
                    });
                    event_broadcast.send(RuntimeEvent::ThinkingDelta(notice.clone()));
                    tracing::info!(kwant = %target_kwant.id, from = %from.id, to = %to.id, class = failure.class.as_str(), "turn moved to another profile");
                    notices.push(notice);
                    continue;
                }
                let text = match profile {
                    Some(p) => {
                        super::profile_turn::give_up_notice(&target_kwant.name, p, &failure, i + 1)
                    }
                    None => format!(
                        "⚠️ [{}] The model provider refused the turn ({}). {}",
                        target_kwant.name,
                        failure.class.as_str(),
                        failure.detail
                    ),
                };
                let mut text = text;
                if failure.class == crate::profiles::classify::FailClass::Quota && !side_effects {
                    if let Some(line) = super::parked::park(scope, target_kwant, &park_prompt, opts.retried || opts.discussion.is_some(), opts.delegation_id.clone(), failure.retry_at).await {
                        text = format!("{text}\n\n{line}");
                    }
                }
                event_broadcast.send(RuntimeEvent::TextDelta(text.clone()));
                event_broadcast.send(RuntimeEvent::TurnComplete {
                    tokens: 0,
                    duration_ms: turn_start.elapsed().as_millis() as u64,
                    kwant_id: Some(target_kwant.id.clone()),
                    profile_id: profile.as_ref().map(|p| p.id.clone()),
                });
                gave_up = Some(text);
                break;
            }
            if let Some(p) = profile {
                if had_state.contains_key(&p.id) && matches!(result, Ok(Ok(_))) {
                    let _ = state.db.clear_profile_state(&p.id).await;
                }
            }
            // Sent once the failure is saved, so the chat it refreshes already shows it
            driver_error = match &result {
                Ok(Err(e)) => Some(RuntimeEvent::Error {
                    code: "driver_error".into(),
                    message: e.to_string(),
                    kwant_id: Some(target_kwant.id.clone()),
                }),
                Err(_) => Some(RuntimeEvent::Error {
                    code: "driver_crashed".into(),
                    message: "the model driver stopped unexpectedly".into(),
                    kwant_id: Some(target_kwant.id.clone()),
                }),
                Ok(Ok(_)) => None,
            };
            if let Some((tokens, duration_ms)) = held_complete {
                event_broadcast.send(RuntimeEvent::TurnComplete {
                    tokens,
                    duration_ms,
                    kwant_id: Some(target_kwant.id.clone()),
                    profile_id: profile.as_ref().map(|p| p.id.clone()),
                });
            }
            driver_result = Some(result);
            break;
        }
        if let Some((_, token)) = &mcp_turn {
            super::mcp_runtime::finish_turn(state, token).await;
        }
        let mcp_records = mcp_ctx.map(|c| c.take_records()).filter(|r| !r.is_empty());
        // Ends what still runs; kept on the reply so the steps show after a reload
        let activity = event_broadcast.activity.finish();

        let with_notices = |reasoning: Option<String>| -> Option<String> {
            if notices.is_empty() {
                return reasoning;
            }
            Some(match reasoning {
                Some(r) => format!("{}\n{r}", notices.join("\n")),
                None => notices.join("\n"),
            })
        };
        if cancelled {
            if let Some(e) = driver_error.take() {
                event_broadcast.send(e);
            }
            event_broadcast.send(RuntimeEvent::ThinkingDelta("⏹️ Cancelled by user".into()));
            return Err("stopped by the user".into());
        } else if let Some(text) = gave_up {
            if !opts.quiet_failure {
                let msg = NewMessage {
                    reasoning_content: with_notices(opts.label.clone()),
                    ..scope.message("kwant", &target_kwant.name, text.clone())
                };
                scope.save(&msg, super::activity::with_activity(gave_up_meta, activity)).await;
            }
            return Err(text);
        } else if let Some(Ok(Ok(output))) = driver_result {
            // This chat got through: a turn parked behind a usage limit would only repeat it
            let _ = db
                .drop_parked_for_chat(&target_kwant.id, scope.channel_id.as_deref(), scope.kwant_id.as_deref(), scope.thread_id.as_deref())
                .await;
            // Post-turn sync: If bot response outputs a URL, sync navigation so the user sees it in Sandbox VM
            if let Some(bot_url) = extract_url_from_text(&output.text) {
                if let Some(ctrl) = get_or_create_browser_controller(state, &sandbox_target).await {
                    let curr = ctrl.current_url.lock().await.clone().unwrap_or_default();
                    if curr != bot_url && !bot_url.contains("localhost") && !bot_url.contains("127.0.0.1") {
                        if let Ok(nav) = ctrl.navigate(&bot_url).await {
                            event_broadcast.send(RuntimeEvent::BrowserNavigated {
                                team_id: sandbox_target.clone(),
                                url: nav.url.clone(),
                                title: nav.title.clone(),
                                status_code: nav.status_code,
                            });
                            if let Some(b64) = ctrl.last_screenshot.lock().await.clone() {
                                event_broadcast.send(RuntimeEvent::BrowserScreenshot {
                                    team_id: sandbox_target.clone(),
                                    url: nav.url.clone(),
                                    image_base64: b64,
                                    timestamp: chrono::Utc::now().to_rfc3339(),
                                });
                            }
                        }
                    }
                }
            }

            // Counts toward the chain's token budget (see `super::budget`)
            let fresh = output.usage.map_or(output.tokens_used as u64, |u| u.fresh());
            state.turns.add_job_tokens(&scope.job_id, fresh);
            // …and toward the task the bot holds, if any (see `super::missions`)
            let turn_started_ms = chrono::Utc::now().timestamp_millis() - turn_start.elapsed().as_millis() as i64;
            // (less what a task the bot handed in mid-turn already paid for)
            let booked = state.turns.take_booked(&tag.turn_id);
            super::missions::spend(state, &target_kwant.id, &scope.context_id(target_kwant), Some(&tag.turn_id), turn_started_ms, fresh, booked).await;

            let mut reply_text = output.text.clone();
            let saved = match &opts.discussion {
                _ if opts.unsaved => None,
                None => Some((
                    output.text.clone(),
                    with_notices(opts.label.clone().or(output.reasoning)),
                )),
                Some(question_label) => match discussion::parse_reply(&output.text) {
                    discussion::Reply::Pass => None,
                    discussion::Reply::AskUser(q) => match question_label {
                        Some(label) => Some((q, with_notices(Some(label.clone())))),
                        None => Some((q, with_notices(opts.label.clone()))),
                    },
                    discussion::Reply::Speak(t) => Some((t, with_notices(opts.label.clone()))),
                },
            };
            if let Some((text, reasoning_content)) = saved {
                // A reply's own choices come off its text into meta.decisions
                let (text, decisions) = match opts.discussion {
                    None => super::decide::take_decisions(&text),
                    Some(_) => (text, Vec::new()),
                };
                if opts.discussion.is_none() {
                    reply_text = text.clone();
                }
                let asst_msg = NewMessage {
                    reasoning_content,
                    tool_calls: mcp_records.map(serde_json::Value::Array),
                    tokens_used: output.tokens_used as i64,
                    ..scope.message("kwant", &target_kwant.name, text)
                };
                let pick = match (pinned_model.is_some(), pinned_family) {
                    (true, _) => "pinned_model",
                    (false, true) => "pinned_engine",
                    (false, false) => "laya",
                };
                let mut meta = ran_on
                    .as_ref()
                    .map(|r| serde_json::json!({ "ran_on": r.to_json(output.model.as_deref(), pick, &effort_level) }));
                if let Some(usage) = &output.usage {
                    meta.get_or_insert_with(|| serde_json::json!({}))["usage"] = serde_json::json!(usage);
                }
                if let Some(routing) = &opts.routing {
                    meta.get_or_insert_with(|| serde_json::json!({}))["routing"] = routing.clone();
                }
                if let Some(serde_json::Value::Object(extra)) = &opts.extra_meta {
                    let m = meta.get_or_insert_with(|| serde_json::json!({}));
                    for (k, v) in extra {
                        m[k] = v.clone();
                    }
                }
                let meta = super::decide::with_decisions(meta, &decisions);
                let meta = super::activity::with_activity(meta, activity);
                let hold = opts.hook.as_ref().and_then(|h| {
                    let h = h.lock().ok()?;
                    let names: Vec<&str> = h.teammates.iter().map(String::as_str).collect();
                    if !h.hold_choices || super::decide::hands_on(&asst_msg.text, &names) {
                        return None;
                    }
                    let ctx = super::decide::GateCtx { person_hard: h.person_hard, decided: !decisions.is_empty() };
                    super::decide::gate_in(&asst_msg.text, ctx)
                });
                if let Some(gate) = hold {
                    if let Some(h) = &opts.hook {
                        if let Ok(mut h) = h.lock() {
                            let text = asst_msg.text.clone();
                            h.held = Some(super::decide::Held { turn_id: tag.turn_id.clone(), gate, text, msg: asst_msg, meta });
                        }
                    }
                } else {
                    let saved = scope.save(&asst_msg, meta).await;
                    if let (Some(m), false) = (&saved, decisions.is_empty()) {
                        super::decide::note_bot_decisions(state, target_kwant, m).await;
                    }
                    if let Some(h) = &opts.hook {
                        if let Ok(mut h) = h.lock() {
                            h.saved = saved;
                        }
                    }
                }
            }
            return Ok(reply_text);
        }
        let why = match driver_result {
            Some(Ok(Err(e))) => format!("the {target_model} driver failed: {e}"),
            Some(Err(_)) => format!("the {target_model} driver stopped unexpectedly"),
            _ => format!("the {target_model} turn ended without a reply"),
        };
        // Saved like a reply, so the failure is still there after a reload (the live error event isn't)
        if !opts.quiet_failure {
            let msg = NewMessage {
                reasoning_content: with_notices(opts.label.clone()),
                ..scope.message("kwant", &target_kwant.name, turn_failed_text(&target_kwant.name, &why, &event_broadcast.activity))
            };
            scope.save(&msg, super::activity::with_activity(Some(json!({"kind": "turn_failed"})), activity)).await;
        }
        if let Some(e) = driver_error {
            event_broadcast.send(e);
        }
        Err(why)
    }
}

/// The space a chat message belongs to: its channel's, else its direct kwant's, else its team's,
/// else Default.
async fn chat_space(state: &AppState, payload: &SendChatPayload) -> Result<SpaceId, DbError> {
    let db = &state.db;
    if let Some(id) = &payload.channel_id {
        if let Some(space) = db.space_of_channel(id).await? {
            return Ok(space);
        }
    }
    if let Some(id) = &payload.kwant_id {
        if let Some(space) = db.space_of_kwant(id).await? {
            return Ok(space);
        }
    }
    if let Some(id) = &payload.team_id {
        if let Some(space) = db.space_of_team(id).await? {
            return Ok(space);
        }
    }
    Ok(SpaceId::default_space())
}

pub async fn send_chat(
    State(state): State<AppState>,
    headers: axum::http::HeaderMap,
    axum::extract::RawQuery(query): axum::extract::RawQuery,
    Json(payload): Json<SendChatPayload>,
) -> axum::response::Response {
    use super::spaces::RowKind;
    // An explicit ?space= must match every row the body names (a thread overrides the others)
    let rows: Vec<(RowKind, Option<&str>)> = match payload.thread_id.as_deref() {
        Some(t) => vec![(RowKind::Thread, Some(t))],
        None => vec![
            (RowKind::Channel, payload.channel_id.as_deref()),
            (RowKind::Kwant, payload.kwant_id.as_deref()),
            (RowKind::Team, payload.team_id.as_deref()),
        ],
    };
    if let Err(r) = super::spaces::ids_guard(&state, query.as_deref(), &rows).await {
        return r;
    }
    let from_person = super::mcp_suggest::refuse_agent_tokens(&state, &headers, "").await.is_ok();
    match send_chat_inner(state, payload, from_person).await {
        Ok(json) => json.into_response(),
        Err(status) => status.into_response(),
    }
}

async fn send_chat_inner(
    state: AppState,
    mut payload: SendChatPayload,
    from_person: bool,
) -> Result<Json<serde_json::Value>, StatusCode> {
    let uploads = super::attachments::check_uploads(&state, &payload.attachment_ids).await?;
    if let Some(thread_id) = payload.thread_id.clone() {
        return send_to_thread(state, thread_id, payload, uploads).await;
    }
    // Everything this message can reach (mentions, teams, Laya's candidates) is in its space
    let space = chat_space(&state, &payload).await.map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    if super::spaces::ensure_active(&state, &space).await.is_err() {
        return Err(StatusCode::CONFLICT);
    }
    let all_kwants = state
        .db
        .list_kwants(&space)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    let all_teams = state
        .db
        .list_teams(&space)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    // Parse @mentions from prompt with team/kwant lookup
    let mentions = parse_mentions_with_lookup(&payload.prompt, |tag| {
        if all_teams.iter().any(|t| t.name.eq_ignore_ascii_case(tag)) {
            Some(true)
        } else if all_kwants.iter().any(|k| k.name.eq_ignore_ascii_case(tag)) {
            Some(false)
        } else {
            None
        }
    });

    let kwant_mention = mentions.iter().find_map(|m| {
        if let TagMention::Kwant(name) = m {
            Some(name.as_str())
        } else {
            None
        }
    });

    let team_mention = mentions.iter().find_map(|m| {
        if let TagMention::Team(name) = m {
            Some(name.as_str())
        } else {
            None
        }
    });

    // A reply with no @mention to a bot that just asked the person something goes back to that bot
    let mut awaiting: Option<(Kwant, Option<String>)> = None;
    // Set when that is the lead of a gathered list of questions (see `super::asks`)
    let mut gathered: Option<serde_json::Value> = None;
    let mut last_speaker: Option<String> = None;
    // The channel route worked out while deciding that, reused below instead of asking Laya twice
    let mut pre_route: Option<crate::router::team::ChannelRoute> = None;
    if let (None, None, None, None, Some(cid)) =
        (kwant_mention, team_mention, payload.kwant_id.as_ref(), payload.team_id.as_ref(), payload.channel_id.as_ref())
    {
        let recent = state.db.list_messages(cid, 30).await.unwrap_or_default();
        let open = state.db.list_questions(Some("open"), Some(cid), None).await.unwrap_or_default();
        let open: Vec<_> = open.into_iter().filter(|q| q.channel_id.as_deref() == Some(cid.as_str())).collect();
        last_speaker = recent.iter().rev().find(|m| m.sender_role == "kwant").map(|m| m.sender_id.clone());
        if let Some(a) = crate::router::sticky::awaiting_answer(&recent, &open, &all_kwants) {
            let mut sticks = true;
            // A plain "…?" reply gives way when the message clearly belongs to another bot
            if !a.card {
                if let Ok(Some(channel)) = state.db.get_channel(cid).await {
                    let members = channel_members(&channel, &all_kwants);
                    if channel.routing_mode == "auto" && members.len() > 1 {
                        let route = state
                            .router
                            .route_channel_query_after(&payload.prompt, &channel.name, &channel.lead_kwant_id, &members, last_speaker.as_deref())
                            .await;
                        sticks = !crate::router::sticky::yields_to_route(&a, &route);
                        if !sticks {
                            tracing::info!(asker = %a.kwant.id, kwant_id = %route.kwant_id, basis = %route.basis, "a plain question gave way to a clear match");
                        }
                        pre_route = Some(route);
                    }
                }
            }
            if sticks {
                // The answer to a list that gathered several bots' questions: its lead passes it on
                if a.question_id.is_none() {
                    let tail_from = recent.iter().rposition(|m| m.sender_role == "user").map_or(0, |i| i + 1);
                    gathered = super::asks::consolidated_in(&recent[tail_from..])
                        .filter(|(_, link)| link["kwant_id"].as_str() == Some(a.kwant.id.as_str()))
                        .map(|(_, link)| link);
                }
                awaiting = Some((a.kwant.clone(), a.question_id));
            }
        }
    }

    // Detect if a team is targeted
    let target_team = if let Some(tid) = &payload.team_id {
        all_teams
            .iter()
            .find(|t| &t.id == tid || t.name.eq_ignore_ascii_case(tid))
            .cloned()
    } else if let Some(tname) = team_mention {
        all_teams
            .iter()
            .find(|t| t.name.eq_ignore_ascii_case(tname))
            .cloned()
    } else if kwant_mention.is_none() && payload.kwant_id.is_none() && awaiting.is_none() {
        if let Some(cid) = &payload.channel_id {
            if let Ok(Some(channel)) = state.db.get_channel(cid).await {
                if channel.name.starts_with("team-") {
                    let tname = &channel.name[5..];
                    all_teams
                        .iter()
                        .find(|t| t.name.eq_ignore_ascii_case(tname))
                        .cloned()
                } else {
                    None
                }
            } else {
                None
            }
        } else {
            None
        }
    } else {
        None
    };

    // Also resolve active team context for single Kwants invoked inside team channels
    let active_team = if let Some(ref t) = target_team {
        Some(t.clone())
    } else if let Some(tid) = &payload.team_id {
        all_teams
            .iter()
            .find(|t| &t.id == tid || t.name.eq_ignore_ascii_case(tid))
            .cloned()
    } else if let Some(cid) = &payload.channel_id {
        if let Ok(Some(channel)) = state.db.get_channel(cid).await {
            if channel.name.starts_with("team-") {
                let tname = &channel.name[5..];
                all_teams
                    .iter()
                    .find(|t| t.name.eq_ignore_ascii_case(tname))
                    .cloned()
            } else {
                None
            }
        } else {
            None
        }
    } else {
        None
    };

    // Kwants in this chat that a speaker can hand the turn to by @mentioning them
    let chat_teammates: Vec<Kwant> = if let Some(ref t) = active_team {
        let ids: Vec<String> = serde_json::from_str(&t.member_kwants).unwrap_or_default();
        ids.iter().filter_map(|id| all_kwants.iter().find(|k| &k.id == id).cloned()).collect()
    } else if let Some(cid) = &payload.channel_id {
        match state.db.get_channel(cid).await {
            Ok(Some(c)) if c.name == "general" => all_kwants.clone(),
            Ok(Some(c)) => {
                let ids: Vec<String> = serde_json::from_str(&c.member_kwants).unwrap_or_default();
                ids.iter().filter_map(|id| all_kwants.iter().find(|k| &k.id == id).cloned()).collect()
            }
            _ => Vec::new(),
        }
    } else {
        Vec::new()
    };

    let active_team_context = active_team.as_ref().map(|t| crate::drivers::TeamContext {
        id: t.id.clone(),
        name: t.name.clone(),
        description: t.description.clone(),
        workspace_path: t.workspace_path.clone(),
        repo_url: t.repo_url.clone(),
    });

    let workspace_dir = if let Some(ref team) = active_team {
        Some(
            crate::tools::resolve_or_prepare_workspace(
                &crate::db::spaces::space_data_dir(&state.config.data_dir, &space),
                &team.name,
                team.workspace_path.as_deref(),
                team.repo_url.as_deref(),
            )
            .await,
        )
    } else {
        None
    };

    // Insert user message to DB
    let user_msg = NewMessage {
        channel_id: payload.channel_id.clone(),
        direct_kwant_id: payload.kwant_id.clone(),
        sender_role: "user".into(),
        sender_id: payload.sender_id.clone(),
        text: payload.prompt.clone(),
        reasoning_content: None,
        tool_calls: None,
        tokens_used: payload.prompt.len() as i64 / 4,
    };
    let user_extra = MessageExtra {
        thread_id: None,
        meta: (!uploads.is_empty()).then(|| json!({"attachments": super::attachments::cards(&uploads)})),
    };
    let saved_user_msg = state
        .db
        .insert_message_with(&user_msg, &user_extra)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    // The person wrote here: the bots waiting on them in this chat don't need to remind them
    super::waiting::person_wrote(&state, payload.channel_id.as_deref(), payload.kwant_id.as_deref(), None).await;
    let note = super::attachments::attach_to_message(
        &state,
        &uploads,
        &saved_user_msg.id,
        payload.channel_id.as_deref(),
        payload.kwant_id.as_deref(),
        None,
    )
    .await;
    payload.prompt.push_str(&note);

    // The person answered the lead's list: the cards close, specialists waiting in threads go
    // on, and the lead is told whom to pass the rest to
    if let Some(link) = &gathered {
        let note = super::asks::deliver_answer(&state, link, &payload.prompt, &all_kwants).await;
        payload.prompt.push_str(&note);
    }

    // "continue" after a chain paused at its token budget carries that chain on
    if let Some(cid) = payload.channel_id.as_deref() {
        if let Some(notice) = super::budget::resume_if_continue(&state, cid, &payload.prompt, from_person).await {
            return Ok(Json(json!({ "status": "continued", "user_message_id": saved_user_msg.id, "notice_id": notice })));
        }
    }

    // Typed instead of tapped: the answer closes the bot's open question, and a turn still waiting
    // on it carries on with it (no second turn)
    if let Some((asker, Some(qid))) = &awaiting {
        if super::questions::answer_by_message(&state, qid, &payload.prompt).await {
            return Ok(Json(json!({
                "status": "answered",
                "user_message_id": saved_user_msg.id,
                "kwant": asker.name,
                "question_id": qid,
            })));
        }
    }

    // Stop answers to this message, chat or team; other dispatches (threads, other chats) keep going
    let mut keys = vec![saved_user_msg.id.clone()];
    keys.extend(payload.channel_id.iter().chain(payload.kwant_id.iter()).cloned());
    if let Some(team) = target_team.as_ref().or(active_team.as_ref()) {
        keys.extend([team.id.clone(), team.name.clone()]);
    }
    let job = state.turns.start_job(keys);

    // Branch: Team routing vs Single Kwant routing
    if let Some(team) = target_team {
        let member_ids: Vec<String> = serde_json::from_str(&team.member_kwants).unwrap_or_default();
        let mut members: Vec<Kwant> = member_ids
            .iter()
            .filter_map(|mid| all_kwants.iter().find(|k| &k.id == mid).cloned())
            .collect();
        if members.is_empty() {
            members = all_kwants.clone();
        }

        let routing_context_id = payload.channel_id.clone().or_else(|| payload.kwant_id.clone());
        let _ = state.event_sender.send(RuntimeEvent::RoutingStarted {
            context_id: routing_context_id.clone(),
            candidates: routing_candidates(&members),
        });

        let team_decision = state
            .router
            .route_team_query(&payload.prompt, &team, &members)
            .await;

        match team_decision {
            TeamDecision::Direct { kwant_id, reason, basis, matched_terms } => {
                let target_kwant = members
                    .iter()
                    .find(|k| k.id == kwant_id)
                    .or_else(|| all_kwants.iter().find(|k| k.id == kwant_id))
                    .unwrap_or(&members[0])
                    .clone();

                // The team router doesn't say which path it took; Laya being down means keywords did
                let fallback = !state.router.status().is_up();
                let basis = if basis.is_empty() { "skills".to_string() } else { basis };
                let routing = routing_meta(&target_kwant, &reason, members.len(), fallback, &matched_terms, &basis);
                let _ = state.event_sender.send(RuntimeEvent::RoutingDecided {
                    context_id: routing_context_id,
                    kwant_id: target_kwant.id.clone(),
                    kwant_name: target_kwant.name.clone(),
                    kwant_title: target_kwant.title.clone(),
                    reason: reason.clone(),
                    mode: "direct".into(),
                    candidate_count: Some(members.len()),
                    fallback,
                    matched_terms,
                    basis: Some(basis),
                });

                let _ = state.event_sender.send(RuntimeEvent::ThinkingDelta(format!(
                    "🔮 Laya Team Router: Routed to {} ({}) — {}",
                    target_kwant.name, target_kwant.title, reason
                )));

                execute_single_turn(
                    state.clone(),
                    target_kwant.clone(),
                    payload,
                    job,
                    Some((team.id.clone(), team.name.clone())),
                    active_team_context,
                    workspace_dir.clone(),
                    members.clone(),
                    Some(routing),
                );

                Ok(Json(json!({
                    "status": "dispatched",
                    "user_message_id": saved_user_msg.id,
                    "kwant": target_kwant.name,
                    "team": team.name
                })))
            }
            TeamDecision::Discuss {
                turns,
                speakers,
                plan,
            } => {
                if let Some(lead) = speakers
                    .first()
                    .and_then(|sid| all_kwants.iter().find(|k| &k.id == sid))
                {
                    let _ = state.event_sender.send(RuntimeEvent::RoutingDecided {
                        context_id: routing_context_id,
                        kwant_id: lead.id.clone(),
                        kwant_name: lead.name.clone(),
                        kwant_title: lead.title.clone(),
                        reason: format!(
                            "This needs several perspectives — starting a {}-turn team discussion led by {}",
                            turns, lead.name
                        ),
                        mode: "discuss".into(),
                        candidate_count: Some(members.len()),
                        fallback: !state.router.status().is_up(),
                        matched_terms: Vec::new(),
                        basis: Some("discuss".into()),
                    });
                }

                let _ = state.event_sender.send(RuntimeEvent::ThinkingDelta(format!(
                    "🔮 Laya Team Router: Initiating {}-turn autonomous discussion with @{}",
                    turns, team.name
                )));

                let planned: Vec<Kwant> = speakers
                    .iter()
                    .filter_map(|sid| all_kwants.iter().find(|k| &k.id == sid).cloned())
                    .collect();
                let scope = TurnScope {
                    state: state.clone(),
                    channel_id: payload.channel_id.clone(),
                    kwant_id: payload.kwant_id.clone(),
                    thread_id: None,
                    team_keys: Some((team.id.clone(), team.name.clone())),
                    team_context: Some(team_context(&team)),
                    workspace_dir: workspace_dir.clone(),
                    teammates: members.clone(),
                    cancel_token: job.token.clone(),
                    job_id: job.id.clone(),
                    delegation_depth: 0,
                };
                let team_clone = team.clone();
                let prompt = payload.prompt.clone();

                tokio::spawn(async move {
                    let _job = job;
                    run_discussion(&scope, &team_clone, planned, &prompt).await;
                });

                Ok(Json(json!({
                    "status": "dispatched",
                    "user_message_id": saved_user_msg.id,
                    "kwant": team.name,
                    "team": team.name,
                    "discussion": {
                        "turns": turns,
                        "speakers": speakers,
                        "plan": plan
                    }
                })))
            }
        }
    } else {
        // Target single Kwant
        let mut routing = None;
        let target_kwant = if let Some(kname) = kwant_mention {
            all_kwants
                .iter()
                .find(|k| k.name.eq_ignore_ascii_case(kname))
                .cloned()
        } else if let Some(kid) = &payload.kwant_id {
            all_kwants
                .iter()
                .find(|k| &k.id == kid || k.name.eq_ignore_ascii_case(kid))
                .cloned()
        } else if let Some((asker, _)) = awaiting.clone() {
            let reason = format!("{} asked you a question here, so your reply goes back to {}", asker.name, asker.name);
            let candidates = chat_teammates.len().max(1);
            routing = Some(routing_meta(&asker, &reason, candidates, false, &[], "asked"));
            let _ = state.event_sender.send(RuntimeEvent::RoutingDecided {
                context_id: payload.channel_id.clone(),
                kwant_id: asker.id.clone(),
                kwant_name: asker.name.clone(),
                kwant_title: asker.title.clone(),
                reason,
                mode: "direct".into(),
                candidate_count: Some(candidates),
                fallback: false,
                matched_terms: Vec::new(),
                basis: Some("asked".into()),
            });
            Some(asker)
        } else if let Some(cid) = &payload.channel_id {
            if let Ok(Some(channel)) = state.db.get_channel(cid).await {
                let lead = all_kwants
                    .iter()
                    .find(|k| k.id == channel.lead_kwant_id)
                    .or_else(|| all_kwants.first())
                    .cloned();
                let members = channel_members(&channel, &all_kwants);

                if channel.routing_mode == "auto" && members.len() > 1 {
                    let context_id = payload.channel_id.clone();
                    let _ = state.event_sender.send(RuntimeEvent::RoutingStarted {
                        context_id: context_id.clone(),
                        candidates: routing_candidates(&members),
                    });
                    let route = match pre_route.take() {
                        Some(route) => route,
                        None => {
                            state
                                .router
                                .route_channel_query_after(
                                    &payload.prompt,
                                    &channel.name,
                                    &channel.lead_kwant_id,
                                    &members,
                                    last_speaker.as_deref(),
                                )
                                .await
                        }
                    };
                    let chosen = members.iter().find(|k| k.id == route.kwant_id).cloned().or(lead);
                    if let Some(ref k) = chosen {
                        routing = Some(routing_meta(k, &route.reason, members.len(), route.fallback, &route.matched_terms, &route.basis));
                        let _ = state.event_sender.send(RuntimeEvent::RoutingDecided {
                            context_id,
                            kwant_id: k.id.clone(),
                            kwant_name: k.name.clone(),
                            kwant_title: k.title.clone(),
                            reason: route.reason,
                            mode: "direct".into(),
                            candidate_count: Some(members.len()),
                            fallback: route.fallback,
                            matched_terms: route.matched_terms,
                            basis: Some(route.basis),
                        });
                    }
                    chosen
                } else {
                    lead
                }
            } else {
                all_kwants.first().cloned()
            }
        } else {
            all_kwants.first().cloned()
        }
        .ok_or(StatusCode::BAD_REQUEST)?;

        let team_keys = active_team.as_ref().map(|t| (t.id.clone(), t.name.clone()));
        execute_single_turn(
            state.clone(),
            target_kwant.clone(),
            payload,
            job,
            team_keys,
            active_team_context,
            workspace_dir,
            chat_teammates,
            routing,
        );

        Ok(Json(json!({
            "status": "dispatched",
            "user_message_id": saved_user_msg.id,
            "kwant": target_kwant.name
        })))
    }
}

/// The bots a channel message can be routed to. #general is the hub for every agent, including
/// ones created after it was seeded; channels made before the lead was always a member still
/// count the lead.
fn channel_members(channel: &crate::db::models::Channel, all_kwants: &[Kwant]) -> Vec<Kwant> {
    if channel.name == "general" {
        return all_kwants.to_vec();
    }
    let member_ids: Vec<String> = serde_json::from_str(&channel.member_kwants).unwrap_or_default();
    crate::db::models::members_with_lead(&channel.lead_kwant_id, &member_ids)
        .iter()
        .filter_map(|mid| all_kwants.iter().find(|k| &k.id == mid).cloned())
        .collect()
}

/// A reply's `meta.routing`: who Laya picked for a channel or team message and why.
fn routing_meta(kwant: &Kwant, reason: &str, candidates: usize, fallback: bool, terms: &[String], basis: &str) -> serde_json::Value {
    json!({
        "basis": basis,
        "kwant_id": kwant.id,
        "kwant_name": kwant.name,
        "reason": reason,
        "candidate_count": candidates,
        "fallback": fallback,
        "matched_terms": terms,
    })
}

/// `/api/chat/send` into a thread: the thread's agent answers there, with only that thread's history.
async fn send_to_thread(
    state: AppState,
    thread_id: String,
    payload: SendChatPayload,
    uploads: Vec<crate::db::attachments::Attachment>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    let thread = state.db.get_thread(&thread_id).await.map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?.ok_or(StatusCode::NOT_FOUND)?;
    if thread.archived {
        return Err(StatusCode::CONFLICT);
    }
    let kwant = state.db.get_kwant(&thread.kwant_id).await.map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?.ok_or(StatusCode::NOT_FOUND)?;
    if super::spaces::ensure_active(&state, &SpaceId::new(&kwant.space_id)).await.is_err() {
        return Err(StatusCode::CONFLICT);
    }
    let depth = match state.db.delegation_for_thread(&thread.id).await.ok().flatten() {
        Some(d) => d.depth,
        None => 0,
    };
    let user_msg = NewMessage {
        channel_id: None,
        direct_kwant_id: None,
        sender_role: "user".into(),
        sender_id: payload.sender_id.clone(),
        text: payload.prompt.clone(),
        reasoning_content: None,
        tool_calls: None,
        tokens_used: payload.prompt.len() as i64 / 4,
    };
    let meta = (!uploads.is_empty()).then(|| json!({"attachments": super::attachments::cards(&uploads)}));
    let extra = MessageExtra { thread_id: Some(thread.id.clone()), meta };
    let saved = state.db.insert_message_with(&user_msg, &extra).await.map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let _ = state.db.touch_thread(&thread.id).await;
    super::waiting::person_wrote(&state, None, None, Some(&thread.id)).await;
    let note = super::attachments::attach_to_message(&state, &uploads, &saved.id, None, None, Some(&thread.id)).await;

    let job = state.turns.start_job([saved.id.clone(), thread.id.clone()]);
    let scope = scope_for(&state, &kwant, None, Some(thread.id.clone()), &job, depth).await.map_err(|_| StatusCode::NOT_FOUND)?;
    let prompt = payload.prompt + &note;
    let target = kwant.clone();
    tokio::spawn(async move {
        let _job = job;
        run_kwant_turn(&scope, &target, prompt, TurnOpts::default()).await;
    });
    Ok(Json(json!({
        "status": "dispatched",
        "user_message_id": saved.id,
        "kwant": kwant.name,
        "thread_id": thread.id,
    })))
}

#[derive(Debug, Deserialize)]
pub struct ResolveApprovalPayload {
    pub decision: String, // "allow_once", "always_allow", "deny"
}

pub async fn resolve_approval(
    State(state): State<AppState>,
    Path(ticket_id): Path<String>,
    Json(payload): Json<ResolveApprovalPayload>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    let decision = match payload.decision.as_str() {
        "allow_once" => ApprovalDecision::AllowOnce,
        "always_allow" => ApprovalDecision::AlwaysAllowSession,
        _ => ApprovalDecision::Deny,
    };

    let status_str = match decision {
        ApprovalDecision::AllowOnce => "allowed_once",
        ApprovalDecision::AlwaysAllowSession => "always_allowed",
        ApprovalDecision::Deny => "denied",
    };

    // Only a ticket still waiting can be answered; a withdrawn one (turn ended, expired)
    // keeps its status and its tool never runs
    let resolved = state.broker.resolve(&ticket_id, decision.clone()).await;
    if !resolved {
        // A card whose turn stopped waiting: a late answer, and Allow resumes the work
        return match super::mcp_runtime::resume_late_approval(&state, &ticket_id, decision, status_str).await {
            Some(mut reply) => {
                reply["ticket_id"] = json!(ticket_id);
                reply["resolved"] = json!(true);
                reply["status"] = json!(status_str);
                Ok(Json(reply))
            }
            None => Err(StatusCode::CONFLICT),
        };
    }
    let _ = state.db.update_approval_status(&ticket_id, status_str).await;
    // The card is answered: a task that waited on it is working again (Mission Control)
    super::missions::listener::refresh_needs_you(&state).await;

    Ok(Json(json!({
        "ticket_id": ticket_id,
        "resolved": resolved,
        "status": status_str
    })))
}

#[derive(Serialize)]
pub struct SettingsResponse {
    pub anthropic_api_key: String,
    pub xai_api_key: String,
    pub openai_api_key: String,
    pub ollama_url: String,
    pub claude_cli_found: bool,
    pub claude_cli_path: Option<String>,
    pub grok_cli_found: bool,
    pub grok_cli_path: Option<String>,
    pub codex_cli_found: bool,
    pub codex_cli_path: Option<String>,
    /// "••••••••" when set: ntfy topics and webhook tokens are secrets.
    pub notify_webhook_url: String,
    pub notify_on: Vec<String>,
    /// Turns one agent runs at once (in different chats/threads); more wait in a queue.
    pub max_turns_per_agent: usize,
    /// Turns the whole server runs at once.
    pub max_turns_global: usize,
    /// Minutes before an unfinished delegation gets a "still working" note; 0 = never.
    pub delegation_follow_up_minutes: i64,
    /// Whether turns get the agents' own tools (history search, memory); see
    /// [`super::continuity::agent_tools_enabled`].
    pub agent_tools: bool,
    /// The person's IANA time zone, "" when not set; routines agents create default to it.
    pub timezone: String,
    /// The person's city as they typed it, "" when not set.
    pub location: String,
    /// How long a CAPTCHA step waits for the person before the bot moves on (1–60).
    pub captcha_timeout_minutes: u64,
    /// Hours before a bot waiting on the person reminds them once; 0 = never.
    pub waiting_follow_up_hours: i64,
    /// Fresh tokens one chain may use before hand-offs pause (see `super::budget`); 0 = off.
    pub chain_token_budget: u64,
    /// Bots decide reversible choices themselves (see `super::decide`).
    pub decide_first: bool,
    /// Bots work ahead in quiet chats.
    pub work_ahead: bool,
    /// Work-ahead runs per space per day (1–10).
    pub work_ahead_daily_cap: u64,
    /// Mission Control: the board, the bots' task tools and the mission runner (see `super::missions`).
    pub mission_board: bool,
    /// Minutes a task claim lives without activity (5–240).
    pub mission_lease_minutes: i64,
    /// A new mission's token budget when none is given; 0 = no limit (the default).
    pub mission_default_budget_tokens: i64,
    /// A new task's token budget when none is given; 0 = no limit (the default).
    pub mission_default_task_budget_tokens: i64,
    /// Time Machine: checkpoints before and after each task run (see `super::missions::checkpoints`).
    pub checkpoints: bool,
    /// Largest checkpoint, in MB (10–2000).
    pub checkpoint_max_mb: i64,
    /// Checkpoints kept per task (1–20), plus its first `before_run`.
    pub checkpoint_keep: i64,
    /// AutoFix: `off` (record failures only), `suggest` (the person starts each retry) or `on`.
    pub autofix: String,
    /// Share of a task's budget its AutoFix retries may spend (0–100).
    pub recovery_budget_pct: i64,
    /// Flight School: tiers are enforced for task turns (see `super::missions::flight_school`).
    pub flight_school: bool,
    /// The tier a bot hired while Flight School is on starts at, for every class.
    pub flight_school_new_bot_tier: String,
}

/// Unknown keys are refused, so a typo or a key from another version fails loudly instead of
/// being dropped.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SaveSettingsPayload {
    pub anthropic_api_key: Option<String>,
    pub xai_api_key: Option<String>,
    pub openai_api_key: Option<String>,
    pub ollama_url: Option<String>,
    /// https only; "" removes it, the mask keeps it.
    pub notify_webhook_url: Option<String>,
    pub notify_on: Option<Vec<String>>,
    pub max_turns_per_agent: Option<usize>,
    pub max_turns_global: Option<usize>,
    pub delegation_follow_up_minutes: Option<i64>,
    /// Stored as the `agent_tools` setting: "on" or "off".
    pub agent_tools: Option<bool>,
    /// An IANA name such as "Europe/Berlin"; "" clears it.
    pub timezone: Option<String>,
    /// Saved as `timezone` only while none is set: the app sends the browser's zone on load, and
    /// this never overwrites one the person or a bot chose. Ignored when `timezone` is also sent.
    pub timezone_default: Option<String>,
    /// The person's city, plain text ("Ljubljana"); "" clears it. Never looked up anywhere.
    pub location: Option<String>,
    pub captcha_timeout_minutes: Option<u64>,
    pub waiting_follow_up_hours: Option<i64>,
    pub chain_token_budget: Option<u64>,
    pub decide_first: Option<bool>,
    pub work_ahead: Option<bool>,
    pub work_ahead_daily_cap: Option<u64>,
    pub mission_board: Option<bool>,
    pub mission_lease_minutes: Option<i64>,
    pub mission_default_budget_tokens: Option<i64>,
    pub mission_default_task_budget_tokens: Option<i64>,
    pub checkpoints: Option<bool>,
    pub checkpoint_max_mb: Option<i64>,
    pub checkpoint_keep: Option<i64>,
    pub autofix: Option<String>,
    pub recovery_budget_pct: Option<i64>,
    pub flight_school: Option<bool>,
    pub flight_school_new_bot_tier: Option<String>,
}

pub async fn get_settings(State(state): State<AppState>) -> Result<Json<SettingsResponse>, StatusCode> {
    let anthropic = state.db.get_setting("anthropic_api_key").await.unwrap_or(None).unwrap_or_default();
    let xai = state.db.get_setting("xai_api_key").await.unwrap_or(None).unwrap_or_default();
    let openai = state.db.get_setting("openai_api_key").await.unwrap_or(None).unwrap_or_default();
    let ollama = state.db.get_setting("ollama_url").await.unwrap_or(None).unwrap_or_else(|| "http://localhost:11434/v1".to_string());

    let claude_bin = find_cli_binary("claude");
    let grok_bin = find_cli_binary("grok");
    let codex_bin = find_cli_binary("codex");
    let (notify_webhook_url, notify_on) = super::continuity::notify_settings_view(&state).await;

    Ok(Json(SettingsResponse {
        anthropic_api_key: if anthropic.is_empty() { String::new() } else { "••••••••".into() },
        xai_api_key: if xai.is_empty() { String::new() } else { "••••••••".into() },
        openai_api_key: if openai.is_empty() { String::new() } else { "••••••••".into() },
        ollama_url: ollama,
        claude_cli_found: claude_bin.is_some(),
        claude_cli_path: claude_bin,
        grok_cli_found: grok_bin.is_some(),
        grok_cli_path: grok_bin,
        codex_cli_found: codex_bin.is_some(),
        codex_cli_path: codex_bin,
        notify_webhook_url,
        notify_on,
        max_turns_per_agent: state.turns.limiter.limits().0,
        max_turns_global: state.turns.limiter.limits().1,
        delegation_follow_up_minutes: super::delegation::follow_up_minutes(&state).await,
        agent_tools: super::continuity::agent_tools_enabled(&state).await,
        timezone: state.db.get_setting(super::bot_actions::TIMEZONE_SETTING).await.ok().flatten().unwrap_or_default(),
        location: state.db.get_setting(super::bot_actions::LOCATION_SETTING).await.ok().flatten().unwrap_or_default(),
        captcha_timeout_minutes: state.db.get_setting(super::human_steps::CAPTCHA_TIMEOUT_SETTING).await.ok().flatten()
            .and_then(|v| v.parse::<u64>().ok())
            .filter(|m| (1..=super::human_steps::MAX_CAPTCHA_TIMEOUT_MINUTES).contains(m))
            .unwrap_or(super::human_steps::DEFAULT_CAPTCHA_TIMEOUT_MINUTES),
        waiting_follow_up_hours: super::waiting::follow_up_hours(&state).await,
        chain_token_budget: super::budget::limit(&state).await,
        decide_first: super::decide::enabled(&state).await,
        work_ahead: super::decide::work_ahead_enabled(&state).await,
        work_ahead_daily_cap: super::decide::work_ahead_cap(&state).await,
        mission_board: super::missions::enabled(&state).await,
        mission_lease_minutes: super::missions::lease_minutes(&state).await,
        mission_default_budget_tokens: super::missions::default_budget(&state).await,
        mission_default_task_budget_tokens: super::missions::default_task_budget(&state).await,
        checkpoints: super::missions::checkpoints::enabled(&state).await,
        checkpoint_max_mb: super::missions::checkpoints::max_mb(&state).await,
        checkpoint_keep: super::missions::checkpoints::keep(&state).await,
        autofix: super::missions::autofix::mode(&state).await.into(),
        recovery_budget_pct: super::missions::autofix::recovery_pct(&state).await,
        flight_school: super::missions::flight_school::enabled(&state.db).await,
        flight_school_new_bot_tier: super::missions::flight_school::new_bot_tier(&state.db).await.into(),
    }))
}

/// Applies the saved turn limits (at startup); missing or bad values keep the defaults.
pub async fn load_turn_limits(state: &AppState) {
    use super::turns::*;
    let read = |key: &'static str| async move { state.db.get_setting(key).await.ok().flatten().and_then(|v| v.parse::<usize>().ok()) };
    let per_agent = read(PER_AGENT_SETTING).await.unwrap_or(DEFAULT_PER_AGENT).clamp(1, MAX_PER_AGENT);
    let global = read(GLOBAL_SETTING).await.unwrap_or(DEFAULT_GLOBAL).clamp(1, MAX_GLOBAL);
    state.turns.limiter.set_limits(per_agent, global);
}

pub async fn save_settings(
    State(state): State<AppState>,
    payload: Result<Json<SaveSettingsPayload>, axum::extract::rejection::JsonRejection>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    use super::turns::{GLOBAL_SETTING, MAX_GLOBAL, MAX_PER_AGENT, PER_AGENT_SETTING};
    let bad = |m: String| Err((StatusCode::BAD_REQUEST, Json(json!({ "error": m }))));
    // 400 rather than axum's 422, like the other refusals here
    let Json(payload) = match payload {
        Ok(p) => p,
        Err(e) => return bad(e.body_text()),
    };
    // Checked first, so a bad value saves nothing
    if payload.max_turns_per_agent.is_some_and(|n| !(1..=MAX_PER_AGENT).contains(&n)) {
        return bad(format!("max_turns_per_agent must be 1–{MAX_PER_AGENT}"));
    }
    if payload.max_turns_global.is_some_and(|n| !(1..=MAX_GLOBAL).contains(&n)) {
        return bad(format!("max_turns_global must be 1–{MAX_GLOBAL}"));
    }
    let max_minutes = super::delegation::MAX_FOLLOW_UP_MINUTES;
    if payload.delegation_follow_up_minutes.is_some_and(|m| !(0..=max_minutes).contains(&m)) {
        return bad(format!("delegation_follow_up_minutes must be 0–{max_minutes}"));
    }
    let check_tz = |tz: &str| crate::routines::Schedule::Daily { time: "09:00".into(), tz: tz.to_string() }.validate();
    let max_captcha = super::human_steps::MAX_CAPTCHA_TIMEOUT_MINUTES;
    if payload.captcha_timeout_minutes.is_some_and(|m| !(1..=max_captcha).contains(&m)) {
        return bad(format!("captcha_timeout_minutes must be 1–{max_captcha}"));
    }
    let max_hours = super::waiting::MAX_HOURS;
    if payload.waiting_follow_up_hours.is_some_and(|h| !(0..=max_hours).contains(&h)) {
        return bad(format!("waiting_follow_up_hours must be 0–{max_hours}"));
    }
    if payload.work_ahead_daily_cap.is_some_and(|n| !(1..=super::decide::MAX_WORK_AHEAD_CAP).contains(&n)) {
        return bad(format!("work_ahead_daily_cap must be 1–{}", super::decide::MAX_WORK_AHEAD_CAP));
    }
    if let Some(e) = super::missions::settings_error(payload.mission_lease_minutes, payload.mission_default_budget_tokens, payload.mission_default_task_budget_tokens) {
        return bad(e);
    }
    if let Some(e) = super::missions::checkpoints::settings_error(payload.checkpoint_max_mb, payload.checkpoint_keep) {
        return bad(e);
    }
    if let Some(e) = super::missions::autofix::settings_error(payload.autofix.as_deref(), payload.recovery_budget_pct) {
        return bad(e);
    }
    if let Some(e) = super::missions::flight_school::settings_error(payload.flight_school_new_bot_tier.as_deref()) {
        return bad(e);
    }
    if payload.chain_token_budget.is_some_and(|n| n > super::budget::MAX) {
        return bad(format!("chain_token_budget must be 0–{}", super::budget::MAX));
    }
    let timezone = payload.timezone.as_deref().map(crate::routines::canonical_tz);
    if let Some(tz) = timezone.as_deref().filter(|t| !t.is_empty()) {
        if let Err(e) = check_tz(tz) {
            return bad(e);
        }
    }
    // A browser's zone this server doesn't know is skipped, not an error: nothing the person can fix
    let timezone_default = payload
        .timezone_default
        .as_deref()
        .map(crate::routines::canonical_tz)
        .filter(|tz| !tz.is_empty() && timezone.is_none() && check_tz(tz).is_ok());
    let location = match payload.location.as_deref() {
        Some(raw) if raw.chars().count() > crate::drivers::MAX_LOCATION_CHARS => {
            return bad(format!("location must be at most {} characters", crate::drivers::MAX_LOCATION_CHARS))
        }
        Some(raw) => Some(crate::drivers::clean_location(raw)),
        None => None,
    };
    if let Err(e) =
        super::continuity::save_notify_settings(&state, payload.notify_webhook_url, payload.notify_on).await
    {
        return Err((StatusCode::BAD_REQUEST, Json(json!({ "error": e }))));
    }
    // A new key or Ollama URL is checked right away, in the background
    let mut engines_changed = false;
    if let Some(key) = payload.anthropic_api_key {
        if !key.starts_with('•') && !key.is_empty() {
            let _ = state.db.set_setting("anthropic_api_key", &key).await;
            engines_changed = true;
        }
    }
    if let Some(key) = payload.xai_api_key {
        if !key.starts_with('•') && !key.is_empty() {
            let _ = state.db.set_setting("xai_api_key", &key).await;
            engines_changed = true;
        }
    }
    if let Some(key) = payload.openai_api_key {
        if !key.starts_with('•') && !key.is_empty() {
            let _ = state.db.set_setting("openai_api_key", &key).await;
            engines_changed = true;
        }
    }
    if let Some(url) = payload.ollama_url {
        if !url.is_empty() {
            let _ = state.db.set_setting("ollama_url", &url).await;
            engines_changed = true;
        }
    }
    if engines_changed {
        let state = state.clone();
        tokio::spawn(async move {
            super::engine_routes::recheck_all(&state, &SpaceId::default_space()).await;
        });
    }
    if let Some(minutes) = payload.captcha_timeout_minutes {
        let _ = state.db.set_setting(super::human_steps::CAPTCHA_TIMEOUT_SETTING, &minutes.to_string()).await;
    }
    if payload.max_turns_per_agent.is_some() || payload.max_turns_global.is_some() {
        let (per_agent, global) = state.turns.limiter.limits();
        let per_agent = payload.max_turns_per_agent.unwrap_or(per_agent);
        let global = payload.max_turns_global.unwrap_or(global);
        let _ = state.db.set_setting(PER_AGENT_SETTING, &per_agent.to_string()).await;
        let _ = state.db.set_setting(GLOBAL_SETTING, &global.to_string()).await;
        state.turns.limiter.set_limits(per_agent, global);
    }
    if let Some(n) = payload.chain_token_budget {
        let _ = state.db.set_setting(super::budget::SETTING, &n.to_string()).await;
    }
    if let Some(m) = payload.delegation_follow_up_minutes {
        let _ = state.db.set_setting(super::delegation::FOLLOW_UP_SETTING, &m.to_string()).await;
    }
    if let Some(h) = payload.waiting_follow_up_hours {
        let _ = state.db.set_setting(super::waiting::HOURS_SETTING, &h.to_string()).await;
        state.scheduler_wake.notify_one();
    }
    if let Some(on) = payload.decide_first {
        let _ = state.db.set_setting(super::decide::SETTING, if on { "true" } else { "false" }).await;
    }
    if let Some(on) = payload.work_ahead {
        let _ = state.db.set_setting(super::decide::WORK_AHEAD_SETTING, if on { "true" } else { "false" }).await;
    }
    if let Some(n) = payload.work_ahead_daily_cap {
        let _ = state.db.set_setting(super::decide::WORK_AHEAD_CAP_SETTING, &n.to_string()).await;
    }
    if let Some(on) = payload.mission_board {
        let _ = state.db.set_setting(super::missions::SETTING, if on { "true" } else { "false" }).await;
    }
    if let Some(m) = payload.mission_lease_minutes {
        let _ = state.db.set_setting(super::missions::LEASE_SETTING, &m.to_string()).await;
    }
    if let Some(n) = payload.mission_default_budget_tokens {
        let _ = state.db.set_setting(super::missions::BUDGET_SETTING, &n.to_string()).await;
    }
    if let Some(n) = payload.mission_default_task_budget_tokens {
        let _ = state.db.set_setting(super::missions::TASK_BUDGET_SETTING, &n.to_string()).await;
    }
    if let Some(on) = payload.checkpoints {
        let _ = state.db.set_setting(super::missions::checkpoints::SETTING, if on { "true" } else { "false" }).await;
    }
    if let Some(n) = payload.checkpoint_max_mb {
        let _ = state.db.set_setting(super::missions::checkpoints::MAX_MB_SETTING, &n.to_string()).await;
    }
    if let Some(n) = payload.checkpoint_keep {
        let _ = state.db.set_setting(super::missions::checkpoints::KEEP_SETTING, &n.to_string()).await;
    }
    if let Some(m) = &payload.autofix {
        let _ = state.db.set_setting(super::missions::autofix::SETTING, m).await;
    }
    if let Some(n) = payload.recovery_budget_pct {
        let _ = state.db.set_setting(super::missions::autofix::PCT_SETTING, &n.to_string()).await;
    }
    if let Some(on) = payload.flight_school {
        let _ = state.db.set_setting(super::missions::flight_school::SETTING, if on { "true" } else { "false" }).await;
    }
    if let Some(t) = &payload.flight_school_new_bot_tier {
        let _ = state.db.set_setting(super::missions::flight_school::NEW_BOT_SETTING, t).await;
    }
    if let Some(on) = payload.agent_tools {
        let _ = state.db.set_setting("agent_tools", if on { "on" } else { "off" }).await;
    }
    if let Some(tz) = timezone {
        let _ = state.db.set_setting(super::bot_actions::TIMEZONE_SETTING, &tz).await;
    }
    if let Some(tz) = timezone_default {
        let _ = state.db.set_setting_if_blank(super::bot_actions::TIMEZONE_SETTING, &tz).await;
    }
    if let Some(city) = location {
        let _ = state.db.set_setting(super::bot_actions::LOCATION_SETTING, &city).await;
    }

    Ok(Json(json!({ "status": "saved" })))
}

#[derive(Debug, Deserialize)]
pub struct DirectoriesQuery {
    pub path: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct QuickPath {
    pub name: String,
    pub path: String,
}

#[derive(Debug, Serialize)]
pub struct DirectoriesResponse {
    pub current_path: String,
    pub parent_path: Option<String>,
    pub directories: Vec<String>,
    pub quick_paths: Vec<QuickPath>,
}

pub async fn list_system_directories(
    Query(query): Query<DirectoriesQuery>,
) -> Result<Json<DirectoriesResponse>, StatusCode> {
    let home_dir = std::env::var("HOME").ok().map(PathBuf::from);
    let current_dir = std::env::current_dir().ok();

    let target_path = if let Some(p) = query.path.filter(|s| !s.trim().is_empty()) {
        let trimmed = p.trim();
        if let Some(rest) = trimmed.strip_prefix("~/") {
            if let Some(ref home) = home_dir {
                home.join(rest)
            } else {
                PathBuf::from(trimmed)
            }
        } else {
            PathBuf::from(trimmed)
        }
    } else if let Some(ref cur) = current_dir {
        cur.clone()
    } else if let Some(ref home) = home_dir {
        home.clone()
    } else {
        PathBuf::from("/")
    };

    let canonical_target = match target_path.canonicalize() {
        Ok(c) => c,
        Err(_) => target_path.clone(),
    };

    let mut directories = Vec::new();
    if let Ok(mut entries) = tokio::fs::read_dir(&canonical_target).await {
        while let Ok(Some(entry)) = entries.next_entry().await {
            if let Ok(file_type) = entry.file_type().await {
                if file_type.is_dir() {
                    let name = entry.file_name().to_string_lossy().to_string();
                    if !name.starts_with('.') {
                        directories.push(name);
                    }
                }
            }
        }
    }
    directories.sort_by_key(|a| a.to_lowercase());

    let parent_path = canonical_target.parent().map(|p| p.to_string_lossy().to_string());

    let mut quick_paths = Vec::new();
    if let Some(ref cur) = current_dir {
        quick_paths.push(QuickPath {
            name: "Current Project".into(),
            path: cur.to_string_lossy().to_string(),
        });
    }
    if let Some(ref home) = home_dir {
        quick_paths.push(QuickPath {
            name: "Home (~)".into(),
            path: home.to_string_lossy().to_string(),
        });
        let dev_path = home.join("dev");
        if dev_path.exists() {
            quick_paths.push(QuickPath {
                name: "~/dev".into(),
                path: dev_path.to_string_lossy().to_string(),
            });
        }
        let projects_path = home.join("projects");
        if projects_path.exists() {
            quick_paths.push(QuickPath {
                name: "~/projects".into(),
                path: projects_path.to_string_lossy().to_string(),
            });
        }
        let docs_path = home.join("Documents");
        if docs_path.exists() {
            quick_paths.push(QuickPath {
                name: "Documents".into(),
                path: docs_path.to_string_lossy().to_string(),
            });
        }
    }

    Ok(Json(DirectoriesResponse {
        current_path: canonical_target.to_string_lossy().to_string(),
        parent_path,
        directories,
        quick_paths,
    }))
}

#[derive(Debug, Serialize, Deserialize)]
pub struct SelectFolderResponse {
    pub path: Option<String>,
    pub canceled: bool,
}

pub async fn select_folder() -> Result<Json<SelectFolderResponse>, StatusCode> {
    if std::env::var("KWNT_TEST_MODE").is_ok() {
        return Ok(Json(SelectFolderResponse {
            path: None,
            canceled: true,
        }));
    }

    #[cfg(target_os = "macos")]
    {
        let res = tokio::process::Command::new("osascript")
            .arg("-e")
            .arg("POSIX path of (choose folder with prompt \"Select Team Workspace Folder:\")")
            .output()
            .await;

        match res {
            Ok(output) if output.status.success() => {
                let raw = String::from_utf8_lossy(&output.stdout);
                let path = raw.trim().trim_end_matches('/').to_string();
                if !path.is_empty() {
                    return Ok(Json(SelectFolderResponse {
                        path: Some(path),
                        canceled: false,
                    }));
                }
            }
            Ok(output) => {
                let err = String::from_utf8_lossy(&output.stderr);
                if err.contains("User canceled") || err.contains("-128") {
                    return Ok(Json(SelectFolderResponse {
                        path: None,
                        canceled: true,
                    }));
                }
                tracing::warn!("osascript folder picker output error: {}", err);
            }
            Err(e) => {
                tracing::warn!("Failed to invoke osascript: {}", e);
            }
        }
    }

    #[cfg(target_os = "linux")]
    {
        let res = tokio::process::Command::new("zenity")
            .arg("--file-selection")
            .arg("--directory")
            .arg("--title=Select Team Workspace Folder")
            .output()
            .await;

        if let Ok(output) = res {
            if output.status.success() {
                let raw = String::from_utf8_lossy(&output.stdout);
                let path = raw.trim().trim_end_matches('/').to_string();
                if !path.is_empty() {
                    return Ok(Json(SelectFolderResponse {
                        path: Some(path),
                        canceled: false,
                    }));
                }
            } else {
                return Ok(Json(SelectFolderResponse {
                    path: None,
                    canceled: true,
                }));
            }
        }
    }

    // Fallback if native dialog is not supported on platform or headless
    Ok(Json(SelectFolderResponse {
        path: None,
        canceled: false,
    }))
}

#[derive(Deserialize)]
pub struct NavigatePayload {
    pub url: String,
}

/// A sandbox target by team, channel or kwant id, or by team name in the Default space. Folders
/// are under the target's own space, so same-named teams in two spaces never share one.
pub async fn resolve_sandbox_target_info(state: &AppState, target_id: &str) -> Option<(String, PathBuf)> {
    let workspaces = |space: &str| {
        crate::db::spaces::space_data_dir(&state.config.data_dir, &SpaceId::new(space)).join("workspaces")
    };
    let team_folder = |team: Team| {
        let ws = team
            .workspace_path
            .as_deref()
            .filter(|s| !s.trim().is_empty())
            .map(PathBuf::from)
            .unwrap_or_else(|| workspaces(&team.space_id).join(&team.name));
        (team.name, ws)
    };
    if let Ok(Some(team)) = state.db.get_team(target_id).await {
        Some(team_folder(team))
    } else if let Ok(Some(channel)) = state.db.get_channel(target_id).await {
        let safe_cname = channel.name.to_lowercase().replace(|c: char| !c.is_alphanumeric(), "-");
        let ws = workspaces(&channel.space_id).join(format!("chan-{}", safe_cname));
        Some((format!("chan-{}", safe_cname), ws))
    } else if let Ok(Some(kwant)) = state.db.get_kwant(target_id).await {
        let safe_kname = kwant.name.to_lowercase().replace(|c: char| !c.is_alphanumeric(), "-");
        Some((format!("bot-{}", safe_kname), workspaces(&kwant.space_id).join(format!("bot-{}", safe_kname))))
    } else if let Ok(Some(team)) = state.db.get_team_by_name(&SpaceId::default_space(), target_id).await {
        Some(team_folder(team))
    } else {
        None
    }
}

const EXCLUDED_EXTENSIONS: &[&str] = &[
    "rs", "ts", "tsx", "js", "jsx", "json", "toml", "yaml", "yml", "md",
    "txt", "log", "lock", "png", "jpg", "jpeg", "gif", "svg", "css", "html",
    "sh", "py", "go", "c", "cpp", "h", "java", "kt", "rb", "php", "sql",
];

pub fn extract_url_from_text(text: &str) -> Option<String> {
    for word in text.split_whitespace() {
        let clean = word.trim_matches(|c: char| {
            !c.is_alphanumeric() && c != '.' && c != '/' && c != ':' && c != '-' && c != '_' && c != '?' && c != '=' && c != '&' && c != '#'
        });
        if clean.starts_with("http://") || clean.starts_with("https://") {
            return Some(clean.to_string());
        }
        if !clean.contains('@') && clean.contains('.') && !clean.starts_with('.') && !clean.ends_with('.') {
            let host_part = clean.split('/').next().unwrap_or("");
            let parts: Vec<&str> = host_part.split('.').collect();
            if parts.len() >= 2 {
                let tld = parts.last().unwrap_or(&"");
                let tld_lower = tld.to_lowercase();
                if tld.len() >= 2
                    && tld.len() <= 12
                    && tld.chars().all(|c| c.is_ascii_alphabetic())
                    && !EXCLUDED_EXTENSIONS.contains(&tld_lower.as_str())
                {
                    let valid_labels = parts.iter().all(|label| {
                        !label.is_empty()
                            && !label.starts_with('-')
                            && !label.ends_with('-')
                            && label.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
                    });
                    if valid_labels {
                        return Some(format!("https://{}", clean));
                    }
                }
            }
        }
    }
    None
}

pub fn detect_url_with_context(prompt: &str, current_url: Option<&str>) -> Option<String> {
    let lower = prompt.to_lowercase();
    let is_browser_request = lower.contains("browser")
        || lower.contains("open ")
        || lower.contains("navigate")
        || lower.contains("go to ")
        || lower.contains("browse ")
        || lower.contains("visit ")
        || lower.contains("search for ")
        || lower.contains("look up ")
        || lower.contains("http://")
        || lower.contains("https://");

    if let Some(direct_url) = extract_url_from_text(prompt) {
        if is_browser_request || prompt.split_whitespace().count() <= 3 {
            return Some(direct_url);
        }
    }

    if !is_browser_request {
        return None;
    }

    // Relative navigation if active current_url exists
    if let Some(curr) = current_url {
        let curr = curr.trim();
        if (curr.starts_with("http://") || curr.starts_with("https://")) && !curr.contains("about:blank") && !curr.contains("example.com") {
            let origin = if let Some(idx) = curr[8..].find('/') {
                &curr[..8 + idx]
            } else {
                curr
            };

            for word in prompt.split_whitespace() {
                let clean = word.trim_matches(|c: char| !c.is_alphanumeric() && c != '/');
                if clean.starts_with('/') && clean.len() > 1 {
                    return Some(format!("{}{}", origin, clean));
                }
            }

            if lower.contains("playlist") {
                return Some(format!("{}/playlists", origin));
            } else if lower.contains("explore") {
                return Some(format!("{}/explore", origin));
            } else if lower.contains("search") && !lower.contains("google") {
                return Some(format!("{}/search", origin));
            } else if lower.contains("login") || lower.contains("sign in") {
                return Some(format!("{}/login", origin));
            } else if lower.contains("settings") {
                return Some(format!("{}/settings", origin));
            } else if lower.contains("about") {
                return Some(format!("{}/about", origin));
            } else if lower.contains("home") {
                return Some(origin.to_string());
            }
        }
    }

    None
}

pub fn detect_url_in_prompt(prompt: &str) -> Option<String> {
    detect_url_with_context(prompt, None)
}

pub async fn get_or_create_browser_controller(
    state: &AppState,
    team_id: &str,
) -> Option<crate::sandbox::BrowserController> {
    // follow a mode switch made elsewhere (another kwnt process sharing the data dir)
    super::computer::apply_mode(state).await;
    // keeps the browser container from being stopped as idle while bots use it
    state.browser_service.note_use();
    let mut controllers = state.browser_controllers.lock().await;
    if let Some(ctrl) = controllers.get(team_id) {
        if !ctrl.is_managed_session() || ctrl.refresh_location().await.is_ok() { return Some(ctrl.clone()); }
        if ctrl.control.try_write().is_err() { return None; }
        controllers.remove(team_id);
    }
    let (_, workspace) = resolve_sandbox_target_info(state, team_id).await?;
    match state.browser_service.session(team_id, &workspace).await {
        Ok(ctrl) => {
            // Passwords belong to the team, channel or bot (by id), never to a folder
            if let Some((owner, _)) = super::passwords::password_owner(state, team_id).await {
                ctrl.attach_vault(state.credentials.clone(), &owner).await;
            }
            controllers.insert(team_id.to_string(), ctrl.clone());
            let _ = state.event_sender.send(RuntimeEvent::SandboxStatusChanged {
                team_id: team_id.to_string(), container_name: "browser-service".into(), is_running: true,
            });
            Some(ctrl)
        }
        Err(error) => { tracing::warn!("Browser service unavailable: {}", error); None }
    }
}

pub async fn get_sandbox_status(
    State(state): State<AppState>, Path(team_id): Path<String>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    let running = get_or_create_browser_controller(&state, &team_id).await.is_some();
    let execution = state.sandbox_manager.get_sandbox(&team_id).await;
    Ok(Json(json!({"team_id":team_id,"is_running":running,"container_name":"browser-service",
        "execution_running":execution.is_some_and(|s| s.is_running)})))
}

fn browser_unavailable(state: &AppState) -> (StatusCode, Json<serde_json::Value>) {
    (StatusCode::SERVICE_UNAVAILABLE, Json(json!({
        "error": state.browser_service.unavailable_reason(),
        "is_running": false
    })))
}

fn browser_request_failed(error: crate::sandbox::BrowserError) -> (StatusCode, Json<serde_json::Value>) {
    if matches!(error, crate::sandbox::BrowserError::HumanControl) { return (StatusCode::CONFLICT, Json(json!({"error": error.to_string()}))); }
    if let crate::sandbox::BrowserError::Navigation(reason) = &error {
        return (StatusCode::UNPROCESSABLE_ENTITY, Json(json!({
            "error": format!("The site could not be reached ({}). The browser is working; check the address.", reason)
        })));
    }
    tracing::warn!("Sandbox browser request failed: {}", error);
    (StatusCode::BAD_GATEWAY, Json(json!({
        "error": "The sandbox browser did not respond. Restart the browser and try again."
    })))
}

pub async fn get_browser_screenshot(
    State(state): State<AppState>,
    Path(team_id): Path<String>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    let ctrl = get_or_create_browser_controller(&state, &team_id).await
        .ok_or_else(|| browser_unavailable(&state))?;
    // The user may have navigated in the live viewer; keep url/title (and other viewers) in sync
    let previous_url = ctrl.current_url.lock().await.clone();
    if let Some((new_url, new_title)) = ctrl.sync_location().await {
        if previous_url.as_deref() != Some(new_url.as_str()) {
            let _ = state.event_sender.send(RuntimeEvent::BrowserNavigated {
                team_id: team_id.clone(),
                url: new_url,
                title: new_title,
                status_code: 200,
            });
        }
    }
    let b64 = ctrl.capture_screenshot().await.map_err(browser_request_failed)?;
    let url = ctrl.current_url.lock().await.clone().unwrap_or_default();
    let title = ctrl.current_title.lock().await.clone().unwrap_or_default();

    let timestamp = chrono::Utc::now().to_rfc3339();

    Ok(Json(json!({
        "team_id": team_id,
        "url": url,
        "title": title,
        "image_base64": b64,
        "screenshot": b64,
        "format": "jpeg",
        "timestamp": timestamp
    })))
}

pub async fn restart_sandbox(
    State(state): State<AppState>,
    Path(team_id): Path<String>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    {
        let mut controllers = state.browser_controllers.lock().await;
        let _guard = if let Some(ctrl) = controllers.get(&team_id) {
            Some(ctrl.control.clone().try_write_owned().map_err(|_| StatusCode::CONFLICT)?)
        } else { None };
        state.browser_service.reset(&team_id).await.map_err(|_| StatusCode::BAD_GATEWAY)?;
        controllers.remove(&team_id);
    }
    let running = get_or_create_browser_controller(&state, &team_id).await.is_some();
    let _ = state.event_sender.send(RuntimeEvent::SandboxStatusChanged {
        team_id: team_id.clone(), container_name: format!("browser-service-{}", team_id), is_running: running,
    });
    Ok(Json(json!({"status":if running {"restarted"} else {"stopped"},"team_id":team_id,"is_running":running,
        "error":if running {None} else {Some(state.browser_service.unavailable_reason())} })))
}

pub async fn navigate_sandbox(
    State(state): State<AppState>,
    Path(team_id): Path<String>,
    Json(payload): Json<NavigatePayload>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    let ctrl = get_or_create_browser_controller(&state, &team_id).await
        .ok_or_else(|| browser_unavailable(&state))?;
    let nav = ctrl.navigate(&payload.url).await.map_err(browser_request_failed)?;

    let _ = state.event_sender.send(RuntimeEvent::BrowserNavigated {
        team_id: team_id.clone(),
        url: nav.url.clone(),
        title: nav.title.clone(),
        status_code: nav.status_code,
    });

    if let Some(b64) = ctrl.last_screenshot.lock().await.clone() {
        let _ = state.event_sender.send(RuntimeEvent::BrowserScreenshot {
            team_id: team_id.clone(),
            url: nav.url.clone(),
            image_base64: b64,
            timestamp: chrono::Utc::now().to_rfc3339(),
        });
    }

    // Return the real page so agents can't mistake a redirect/CAPTCHA for success
    let mut page = sandbox_page_state(&ctrl).await;
    page["status"] = json!(if page["blocked"].as_bool() == Some(true) { "blocked" } else { "navigated" });
    page["team_id"] = json!(team_id);
    page["requested_url"] = json!(payload.url);
    page["status_code"] = json!(nav.status_code);
    Ok(Json(page))
}

#[derive(Deserialize)]
pub struct InteractPayload {
    pub action: String,
    pub x: Option<i64>,
    pub y: Option<i64>,
    pub text: Option<String>,
    pub key: Option<String>,
    pub code: Option<String>,
    pub delta_x: Option<f64>,
    pub delta_y: Option<f64>,
}

pub async fn interact_sandbox(
    State(state): State<AppState>,
    Path(team_id): Path<String>,
    Json(payload): Json<InteractPayload>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    let valid = match payload.action.as_str() {
        "click" | "scroll" => matches!((payload.x, payload.y), (Some(x), Some(y)) if x >= 0 && y >= 0),
        "type" => payload.text.is_some(),
        "key" => payload.key.as_ref().is_some_and(|key| !key.is_empty()),
        _ => false,
    };
    if !valid {
        return Err((StatusCode::BAD_REQUEST, Json(json!({
            "error": "Invalid browser action or missing input parameters."
        }))));
    }

    let ctrl = get_or_create_browser_controller(&state, &team_id).await.ok_or_else(|| browser_unavailable(&state))?;

    let _permit = ctrl.control.try_read().map_err(|_| (StatusCode::CONFLICT, Json(json!({"error":"Browser is controlled by a person."}))))?;
    let shot_res = match payload.action.as_str() {
        "click" => ctrl.interact_click(payload.x.unwrap(), payload.y.unwrap()).await,
        "type" => ctrl.interact_type(payload.text.as_deref().unwrap()).await,
        "key" => ctrl.interact_key(payload.key.as_deref().unwrap(), payload.code.as_deref()).await,
        "scroll" => ctrl.interact_scroll(
            payload.x.unwrap(), payload.y.unwrap(),
            payload.delta_x.unwrap_or(0.0), payload.delta_y.unwrap_or(0.0),
        ).await,
        _ => unreachable!(),
    };
    // Returning an old screenshot as success hides failed input from the user.
    let screenshot = shot_res.map_err(|error| {
        tracing::warn!(team_id = %team_id, "Browser interaction failed: {}", error);
        (StatusCode::BAD_GATEWAY, Json(json!({
            "error": "Could not control the sandbox browser. Refresh the view or restart the browser and try again."
        })))
    })?;
    let current_url = ctrl.current_url.lock().await.clone().unwrap_or_default();

    let _ = state.event_sender.send(RuntimeEvent::BrowserScreenshot {
        team_id: team_id.clone(),
        url: current_url.clone(),
        image_base64: screenshot.clone(),
        timestamp: chrono::Utc::now().to_rfc3339(),
    });

    Ok(Json(json!({
        "status": "ok",
        "action": payload.action,
        "team_id": team_id,
        "url": current_url,
        "screenshot": screenshot,
        "image_base64": screenshot
    })))
}

/// Compact, text-only page state for agents (no base64 image to wade through).
async fn sandbox_page_state(ctrl: &crate::sandbox::BrowserController) -> serde_json::Value {
    let (url, title) = match ctrl.sync_location().await {
        Some(loc) => loc,
        None => (
            ctrl.current_url.lock().await.clone().unwrap_or_default(),
            ctrl.current_title.lock().await.clone().unwrap_or_default(),
        ),
    };
    let text = ctrl.extract_content().await.unwrap_or_default();
    let clickable = ctrl.list_clickables(40).await;
    let mut page = json!({
        "url": url,
        "title": title,
        "text": truncate_chars(&text, 2000),
        "clickable": clickable,
        "blocked": false,
    });
    // A CAPTCHA widget on an otherwise normal page (TikTok's slider over the feed, a
    // reCAPTCHA on a form) counts too; its box tells the person's view where to look
    let captcha = ctrl.probe_captcha().await.ok().flatten();
    let wall = crate::sandbox::detect_bot_wall(&url, &title, &text).or(captcha.as_ref().map(|c| c.label()));
    if let Some(kind) = wall {
        page["blocked"] = json!(true);
        page["notice"] = json!(bot_wall_notice(kind, &url));
    }
    if let Some(captcha) = captcha {
        page["captcha"] = json!(captcha);
    }
    page
}

/// Instruction for the agent when a site answers with a CAPTCHA instead of content.
pub fn bot_wall_notice(kind: &str, url: &str) -> String {
    format!(
        "BLOCKED: the site answered with a {} ({}) instead of the requested content. \
         Do NOT describe results you have not seen. Tell the user plainly that the site blocked the automated browser, then either \
         retry once on another engine via the browser API and say you switched (web: https://www.bing.com/search?q=<query>, images: https://www.bing.com/images/search?q=<query>), \
         or offer that they solve the check themselves in the live browser panel.",
        kind, url
    )
}

async fn broadcast_sandbox_screenshot(state: &AppState, team_id: &str, ctrl: &crate::sandbox::BrowserController) {
    if let Some(b64) = ctrl.last_screenshot.lock().await.clone() {
        let url = ctrl.current_url.lock().await.clone().unwrap_or_default();
        let _ = state.event_sender.send(RuntimeEvent::BrowserScreenshot {
            team_id: team_id.to_string(),
            url,
            image_base64: b64,
            timestamp: chrono::Utc::now().to_rfc3339(),
        });
    }
}

pub async fn get_sandbox_page(
    State(state): State<AppState>,
    Path(team_id): Path<String>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    let ctrl = get_or_create_browser_controller(&state, &team_id)
        .await
        .ok_or_else(|| browser_unavailable(&state))?;
    Ok(Json(sandbox_page_state(&ctrl).await))
}

/// Agent-facing action endpoint: performs one action and returns the resulting page state,
/// so an agent can act and answer without extra round-trips.
pub async fn act_sandbox(
    State(state): State<AppState>,
    Path(team_id): Path<String>,
    Json(payload): Json<InteractPayload>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    let ctrl = get_or_create_browser_controller(&state, &team_id)
        .await
        .ok_or_else(|| browser_unavailable(&state))?;
    let _permit = ctrl.control.try_read().map_err(|_| browser_request_failed(crate::sandbox::BrowserError::HumanControl))?;
    let text = payload.text.clone().unwrap_or_default();
    let result: Result<String, String> = match payload.action.as_str() {
        "click_text" | "click" if !text.is_empty() => ctrl
            .click_text(&text)
            .await
            .map(|label| format!("clicked \"{}\"", label))
            .map_err(|e| e.to_string()),
        "type" => ctrl
            .interact_type(&text)
            .await
            .map(|_| format!("typed {} chars", text.chars().count()))
            .map_err(|e| e.to_string()),
        "key" => {
            let key = payload.key.clone().unwrap_or_else(|| "Enter".into());
            ctrl.interact_key(&key, payload.code.as_deref())
                .await
                .map(|_| format!("pressed {}", key))
                .map_err(|e| e.to_string())
        }
        "scroll" => ctrl
            .interact_scroll(640, 400, 0.0, payload.delta_y.unwrap_or(600.0))
            .await
            .map(|_| "scrolled".to_string())
            .map_err(|e| e.to_string()),
        other => Err(format!(
            "Unknown action \"{}\". Use click_text (with text), type (with text), key (with key) or scroll (with delta_y).",
            other
        )),
    };
    broadcast_sandbox_screenshot(&state, &team_id, &ctrl).await;
    let mut page = sandbox_page_state(&ctrl).await;
    match result {
        Ok(done) => page["result"] = json!(done),
        Err(err) => page["error"] = json!(err),
    }
    if let Some(url) = page["url"].as_str() {
        let _ = state.event_sender.send(RuntimeEvent::BrowserNavigated {
            team_id: team_id.clone(),
            url: url.to_string(),
            title: page["title"].as_str().unwrap_or("").to_string(),
            status_code: 200,
        });
    }
    Ok(Json(page))
}

pub async fn execute_tool(
    state: &AppState,
    team_id: Option<&str>,
    tool_name: &str,
    args: &serde_json::Value,
) -> Result<serde_json::Value, crate::tools::ToolError> {
    let controller = if tool_name.starts_with("browser_") {
        if let Some(tid) = team_id { get_or_create_browser_controller(state, tid).await } else { None }
    } else { None };

    if tool_name == "execute_shell"
        && super::computer::apply_mode(state).await == crate::sandbox::computer::ComputerMode::Local
    {
        // Local mode: the command runs on this computer, so the person approves each one first.
        let workspace = match team_id {
            Some(tid) => resolve_sandbox_target_info(state, tid).await.map(|(_, ws)| ws),
            None => None,
        }
        .unwrap_or_else(|| state.tools.workspace_dir.clone());
        tokio::fs::create_dir_all(&workspace).await?;
        let command = args["command"].as_str()
            .ok_or_else(|| crate::tools::ToolError::InvalidArgs("Missing 'command' argument".into()))?;
        if !super::computer::approve_local_shell(state, team_id.unwrap_or("default"), command, &workspace).await {
            return Err(crate::tools::ToolError::Execution(
                "The person did not approve this command, so it was not run.".into(),
            ));
        }
        return crate::tools::shell::execute_shell(&workspace, args).await;
    }
    if tool_name == "execute_shell" {
        // VM mode: commands only ever run inside the bot's or team's sandbox. With no known
        // target there is no sandbox to use, so refuse rather than fall back to this computer.
        let target = match team_id {
            Some(tid) => resolve_sandbox_target_info(state, tid).await.map(|t| (tid, t)),
            None => None,
        };
        let Some((tid, (name, workspace))) = target else {
            return Err(crate::tools::ToolError::Execution(format!(
                "No bot or team named '{}' was found, so the command was not run. In VM mode commands only run inside a bot's or team's sandbox.",
                team_id.unwrap_or("")
            )));
        };
        tokio::fs::create_dir_all(&workspace).await?;
        let sandbox = state.sandbox_manager.ensure_sandbox_running(tid, &name, &workspace).await
            .map_err(|e| crate::tools::ToolError::Execution(e.to_string()))?;
        let out = state.sandbox_manager.exec_in_sandbox(&sandbox.container_name,
            args["command"].as_str().unwrap_or(""), 30).await
            .map_err(|e| crate::tools::ToolError::Execution(e.to_string()))?;
        return Ok(json!({"command":out.command,"stdout":out.stdout,"stderr":out.stderr,"exit_code":out.exit_code,"success":out.success}));
    }

    let runner = match controller {
        Some(ctrl) => state.tools.clone().with_browser(ctrl),
        None => state.tools.clone(),
    };

    let result = runner.execute(tool_name, args).await?;

    let team_str = team_id.unwrap_or("default").to_string();
    match tool_name {
        "browser_navigate" => {
            let url = args["url"].as_str().unwrap_or_default().to_string();
            let title = result["title"].as_str().unwrap_or(&url).to_string();
            let status_code = result["status_code"].as_u64().unwrap_or(200) as u16;
            let _ = state.event_sender.send(RuntimeEvent::BrowserNavigated {
                team_id: team_str,
                url,
                title,
                status_code,
            });
        }
        "browser_screenshot" => {
            let url = result["url"]
                .as_str()
                .filter(|s| !s.is_empty())
                .unwrap_or("about:blank")
                .to_string();
            let image_base64 = result["image_base64"]
                .as_str()
                .filter(|s| !s.is_empty())
                .unwrap_or(crate::tools::sandbox::PLACEHOLDER_PNG)
                .to_string();
            let timestamp = result["timestamp"]
                .as_str()
                .map(String::from)
                .unwrap_or_else(|| chrono::Utc::now().to_rfc3339());
            let _ = state.event_sender.send(RuntimeEvent::BrowserScreenshot {
                team_id: team_str,
                url,
                image_base64,
                timestamp,
            });
        }
        _ => {}
    }

    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::{scrub_urls, turn_failed_text};

    #[test]
    fn a_saved_turn_failure_hides_secrets_and_home_folders() {
        let tag = crate::drivers::events::TurnTag {
            turn_id: "t-fail".into(),
            kwant_id: "k".into(),
            context_id: "c".into(),
            thread_id: None,
        };
        let activity = super::super::activity::TurnActivity::start(tokio::sync::broadcast::channel(4).0, tag);
        activity.add_secrets(vec!["hunter2-mcp-value".into()]);
        let why = format!(
            "the claude driver failed: 401 key sk-ant-api03-ABCDEFGHIJKLMNOPQRSTUVWX for /home/oleks/.kwnt/secrets.json \
            (env hunter2-mcp-value) {}",
            "x".repeat(2000)
        );
        let text = turn_failed_text("Lin", &why, &activity);
        assert!(text.starts_with("⚠️ [Lin] Couldn't finish this reply: the claude driver failed: 401 key"), "{text}");
        for leak in ["sk-ant-api03-ABCDEF", "/home/oleks", "hunter2-mcp-value"] {
            assert!(!text.contains(leak), "{leak} in {text}");
        }
        assert!(text.contains("~/.kwnt/secrets.json"), "{text}");
        assert!(text.ends_with('…') && text.len() < 700, "{} bytes", text.len());
        // A short error is kept whole
        assert!(!turn_failed_text("Lin", "the turn ended without a reply", &activity).ends_with('…'));
        // Whole user names, with spaces and on Windows
        let text = turn_failed_text("Lin", r"no config at /Users/Oleks Smith/.kwnt or C:\Users\Oleks Smith\.kwnt", &activity);
        assert!(!text.contains("Oleks") && !text.contains("Smith"), "{text}");
        assert!(text.ends_with(r"no config at ~/.kwnt or ~\.kwnt"), "{text}");
    }

    #[test]
    fn failure_reasons_drop_url_credentials() {
        assert_eq!(
            scrub_urls("HTTP API error: error sending request for url (https://u:p4ss@llm.example/v1/chat?key=sk-123&x=1)"),
            "HTTP API error: error sending request for url (https://llm.example/…"
        );
        assert_eq!(scrub_urls("see http://host:8080/a#frag now"), "see http://host:8080/… now");
        let keyed = scrub_urls("401 from https://llm.example/v1/key/sk-path-secret/chat");
        assert_eq!(keyed, "401 from https://llm.example/…");
        assert!(!keyed.contains("sk-path-secret"));
        assert_eq!(scrub_urls("down: https://llm.example"), "down: https://llm.example");
        assert_eq!(scrub_urls("not logged in; run `codex login`"), "not logged in; run `codex login`");
    }
}
