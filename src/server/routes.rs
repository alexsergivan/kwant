use super::discussion;
use super::AppState;
use crate::broker::ApprovalDecision;
use crate::db::models::*;
use crate::drivers::{find_cli_binary, RoutingCandidate, RuntimeEvent, TurnInput};
use crate::router::{parse_mentions_with_lookup, TagMention, TeamDecision};
use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::IntoResponse,
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

pub async fn list_kwants(
    State(state): State<AppState>,
) -> Result<Json<Vec<KwantView>>, StatusCode> {
    state
        .db
        .kwant_views()
        .await
        .map(Json)
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)
}

/// Trims the name and rejects names that can't be @mentioned or clash with another kwant.
async fn validate_kwant_name(state: &AppState, name: &mut String, own_id: Option<&str>) -> Result<(), StatusCode> {
    *name = name.trim().to_string();
    if name.is_empty() || name.contains(char::is_whitespace) || name.contains('@') {
        return Err(StatusCode::BAD_REQUEST);
    }
    let kwants = state.db.list_kwants().await.map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    if kwants.iter().any(|k| Some(k.id.as_str()) != own_id && k.name.eq_ignore_ascii_case(name)) {
        return Err(StatusCode::CONFLICT);
    }
    Ok(())
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

/// The pinned profile must exist and be for the engine of the kwant's model.
async fn check_binding(
    state: &AppState,
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
    match crate::profiles::Engine::from_model(preferred_model) {
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
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)
}

async fn kwant_view_json(state: &AppState, id: &str) -> Result<Json<KwantView>, StatusCode> {
    match state.db.kwant_view(id).await {
        Ok(Some(v)) => Ok(Json(v)),
        Ok(None) => Err(StatusCode::NOT_FOUND),
        Err(_) => Err(StatusCode::INTERNAL_SERVER_ERROR),
    }
}

pub async fn create_kwant(
    State(state): State<AppState>,
    Json(mut payload): Json<CreateKwantRequest>,
) -> Result<Json<KwantView>, StatusCode> {
    validate_kwant_name(&state, &mut payload.base.name, None).await?;
    check_binding(&state, &payload.binding, &payload.base.preferred_model).await?;
    let kwant = state
        .db
        .create_kwant(&payload.base)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    save_binding(&state, &kwant.id, &payload.binding).await?;
    kwant_view_json(&state, &kwant.id).await
}

pub async fn update_kwant(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(mut payload): Json<UpdateKwantRequest>,
) -> Result<Json<KwantView>, StatusCode> {
    validate_kwant_name(&state, &mut payload.base.name, Some(&id)).await?;
    check_binding(&state, &payload.binding, &payload.base.preferred_model).await?;
    match state.db.update_kwant(&id, &payload.base).await {
        Ok(Some(_)) => {}
        Ok(None) => return Err(StatusCode::NOT_FOUND),
        Err(_) => return Err(StatusCode::INTERNAL_SERVER_ERROR),
    }
    save_binding(&state, &id, &payload.binding).await?;
    kwant_view_json(&state, &id).await
}

pub async fn list_channels(State(state): State<AppState>) -> Result<Json<Vec<Channel>>, StatusCode> {
    state
        .db
        .list_channels()
        .await
        .map(Json)
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)
}

pub async fn create_channel(
    State(state): State<AppState>,
    Json(payload): Json<NewChannel>,
) -> Result<Json<Channel>, StatusCode> {
    state
        .db
        .create_channel(&payload)
        .await
        .map(Json)
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)
}

pub async fn list_messages(
    State(state): State<AppState>,
    Path(channel_or_kwant_id): Path<String>,
) -> Result<Json<Vec<Message>>, StatusCode> {
    state
        .db
        .list_messages(&channel_or_kwant_id, 100)
        .await
        .map(Json)
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)
}

pub async fn list_teams(State(state): State<AppState>) -> Result<Json<Vec<Team>>, StatusCode> {
    state
        .db
        .list_teams()
        .await
        .map(Json)
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)
}

pub async fn create_team(
    State(state): State<AppState>,
    Json(payload): Json<NewTeam>,
) -> Result<Json<Team>, StatusCode> {
    state
        .db
        .create_team(&payload)
        .await
        .map(Json)
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)
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

pub async fn update_team(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(payload): Json<UpdateTeamPayload>,
) -> Result<Json<Team>, StatusCode> {
    if let Some(text) = &payload.instructions {
        if super::continuity::check_instructions(text).is_err() {
            return Err(StatusCode::BAD_REQUEST);
        }
    }
    match state.db.update_team(&id, &payload.team).await {
        Ok(Some(mut team)) => {
            if let Some(text) = &payload.instructions {
                state.db.set_team_instructions(&id, text).await.map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
                team.instructions = Some(text.trim().to_string()).filter(|t| !t.is_empty());
            }
            Ok(Json(team))
        }
        Ok(None) => Err(StatusCode::NOT_FOUND),
        Err(_) => Err(StatusCode::INTERNAL_SERVER_ERROR),
    }
}

pub async fn delete_team(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    match state.db.delete_team(&id).await {
        Ok(true) => Ok(Json(json!({ "status": "deleted" }))),
        Ok(false) => Err(StatusCode::NOT_FOUND),
        Err(_) => Err(StatusCode::INTERNAL_SERVER_ERROR),
    }
}

#[derive(Debug, Deserialize)]
pub struct CancelPayload {
    pub session_id: Option<String>,
    pub channel_id: Option<String>,
    pub team_id: Option<String>,
    pub kwant_id: Option<String>,
}

pub async fn cancel_chat(
    State(state): State<AppState>,
    body: axum::body::Bytes,
) -> Result<Json<serde_json::Value>, StatusCode> {
    let mut tokens = state.active_cancellations.lock().await;
    let maybe_session_id = if !body.is_empty() {
        serde_json::from_slice::<CancelPayload>(&body)
            .ok()
            .and_then(|p| p.session_id.or(p.channel_id).or(p.team_id).or(p.kwant_id))
    } else {
        None
    };

    if let Some(session_id) = maybe_session_id {
        if let Some(token) = tokens.remove(&session_id) {
            token.cancel();
        }
    } else {
        for (_, token) in tokens.drain() {
            token.cancel();
        }
    }

    Ok(Json(json!({ "status": "cancelled" })))
}

#[derive(Debug, Deserialize)]
pub struct SendChatPayload {
    pub channel_id: Option<String>,
    pub kwant_id: Option<String>,
    pub team_id: Option<String>,
    pub prompt: String,
    pub sender_id: String,
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

/// Truncates to at most `max` bytes without splitting a UTF-8 character.
fn truncate_chars(text: &str, max: usize) -> &str {
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
struct TurnScope {
    state: AppState,
    channel_id: Option<String>,
    kwant_id: Option<String>,
    team_keys: Option<(String, String)>,
    team_context: Option<crate::drivers::TeamContext>,
    workspace_dir: Option<std::path::PathBuf>,
    /// Kwants in this chat that a speaker may hand the turn to with an @mention.
    teammates: Vec<Kwant>,
    cancel_token: CancellationToken,
}

/// Max follow-up turns kwants may trigger by @mentioning each other for one user message.
const MAX_HANDOFFS: usize = 4;

#[allow(clippy::too_many_arguments)]
fn execute_single_turn(
    state: AppState,
    target_kwant: Kwant,
    payload: SendChatPayload,
    cancel_token: CancellationToken,
    session_id: String,
    team_keys: Option<(String, String)>,
    kwant_id: Option<String>,
    team_context: Option<crate::drivers::TeamContext>,
    workspace_dir: Option<std::path::PathBuf>,
    teammates: Vec<Kwant>,
) {
    let active_cancellations = state.active_cancellations.clone();
    let scope = TurnScope {
        channel_id: payload.channel_id.clone(),
        kwant_id: kwant_id.or_else(|| payload.kwant_id.clone()),
        team_keys,
        team_context,
        workspace_dir,
        teammates,
        cancel_token,
        state,
    };
    let prompt = payload.prompt.clone();

    tokio::spawn(async move {
        if let Some(reply) = run_kwant_turn(&scope, &target_kwant, prompt.clone(), TurnOpts::default()).await {
            run_handoffs(&scope, std::slice::from_ref(&target_kwant.id), &[(target_kwant.clone(), reply)], &prompt).await;
        }

        let mut tokens = active_cancellations.lock().await;
        tokens.remove(&session_id);
        if let Some(cid) = &scope.channel_id {
            tokens.remove(cid);
        }
        if let Some(kid) = &scope.kwant_id {
            tokens.remove(kid);
        }
        if let Some((team_id, team_name)) = &scope.team_keys {
            tokens.remove(team_id);
            tokens.remove(team_name);
        }
    });
}

/// Runs a routine's turn as if the user had sent `prompt` to `kwant` in `channel_id` (or in
/// their direct chat): same team context, workspace, approvals and profile failover, and the
/// chat's stop button cancels it. The prompt message must already be saved. Returns the id of
/// the kwant's reply, or `Err` with why there is none.
pub(super) async fn run_routine_turn(
    state: &AppState,
    kwant: &Kwant,
    channel_id: Option<String>,
    prompt: String,
) -> Result<Option<String>, String> {
    let all_kwants = state.db.list_kwants().await.map_err(|_| "could not load agents".to_string())?;
    let channel = match &channel_id {
        Some(id) => Some(
            state.db.get_channel(id).await.ok().flatten().ok_or_else(|| "the routine's channel no longer exists".to_string())?,
        ),
        None => None,
    };
    let team = match channel.as_ref().and_then(|c| c.name.strip_prefix("team-")) {
        Some(name) => state.db.list_teams().await.unwrap_or_default().into_iter().find(|t| t.name.eq_ignore_ascii_case(name)),
        None => None,
    };
    let member_ids: Vec<String> = match (&team, &channel) {
        (Some(t), _) => serde_json::from_str(&t.member_kwants).unwrap_or_default(),
        (None, Some(c)) if c.name == "general" => all_kwants.iter().map(|k| k.id.clone()).collect(),
        (None, Some(c)) => serde_json::from_str(&c.member_kwants).unwrap_or_default(),
        (None, None) => Vec::new(),
    };
    let teammates: Vec<Kwant> = member_ids.iter().filter_map(|id| all_kwants.iter().find(|k| &k.id == id).cloned()).collect();
    let workspace_dir = match &team {
        Some(t) => Some(
            crate::tools::resolve_or_prepare_workspace(&state.config.data_dir, &t.name, t.workspace_path.as_deref(), t.repo_url.as_deref())
                .await,
        ),
        None => None,
    };
    let cancel_token = CancellationToken::new();
    let keys: Vec<String> = match &channel_id {
        Some(c) => vec![c.clone()],
        None => vec![kwant.id.clone()],
    };
    {
        let mut tokens = state.active_cancellations.lock().await;
        for key in &keys {
            tokens.insert(key.clone(), cancel_token.clone());
        }
    }
    let scope = TurnScope {
        state: state.clone(),
        kwant_id: if channel_id.is_none() { Some(kwant.id.clone()) } else { None },
        channel_id,
        team_keys: team.as_ref().map(|t| (t.id.clone(), t.name.clone())),
        team_context: team.as_ref().map(|t| crate::drivers::TeamContext {
            id: t.id.clone(),
            name: t.name.clone(),
            description: t.description.clone(),
            workspace_path: t.workspace_path.clone(),
            repo_url: t.repo_url.clone(),
        }),
        workspace_dir,
        teammates,
        cancel_token: cancel_token.clone(),
    };
    let reply = try_kwant_turn(&scope, kwant, prompt.clone(), TurnOpts::default()).await;
    let outcome = match reply {
        Ok(text) => {
            let context_id = keys[0].clone();
            let reply_id = state
                .db
                .list_messages(&context_id, 10)
                .await
                .unwrap_or_default()
                .into_iter()
                .rev()
                .find(|m| m.sender_role == "kwant" && m.sender_id == kwant.name)
                .map(|m| m.id);
            run_handoffs(&scope, std::slice::from_ref(&kwant.id), &[(kwant.clone(), text)], &prompt).await;
            Ok(reply_id)
        }
        Err(_) if cancel_token.is_cancelled() => Err("stopped by the user".into()),
        Err(why) => Err(format!("the agent's turn failed: {why}")),
    };
    let mut tokens = state.active_cancellations.lock().await;
    for key in &keys {
        tokens.remove(key);
    }
    outcome
}

/// Lets teammates that kwants @mentioned in `replies` answer in turn, so kwants can delegate to each other.
/// `already_spoken` kwants are not handed the turn again.
async fn run_handoffs(scope: &TurnScope, already_spoken: &[String], replies: &[(Kwant, String)], user_prompt: &str) {
    let mut spoken: Vec<String> = already_spoken.to_vec();
    let mut queue: std::collections::VecDeque<(Kwant, String, String)> = std::collections::VecDeque::new();
    let enqueue = |queue: &mut std::collections::VecDeque<(Kwant, String, String)>,
                   spoken: &[String],
                   from: &Kwant,
                   text: &str| {
        let names: Vec<&str> = scope.teammates.iter().map(|k| k.name.as_str()).collect();
        for i in crate::router::handoff_targets(text, &names) {
            let k = &scope.teammates[i];
            if k.id != from.id && !spoken.contains(&k.id) && !queue.iter().any(|(q, _, _)| q.id == k.id) {
                queue.push_back((k.clone(), from.name.clone(), text.to_string()));
            }
        }
    };
    for (from, text) in replies {
        enqueue(&mut queue, &spoken, from, text);
    }
    if queue.is_empty() {
        return;
    }

    let events = scope.state.event_sender.clone();
    let context_id = scope
        .channel_id
        .clone()
        .or_else(|| scope.kwant_id.clone())
        .unwrap_or_else(|| replies[0].0.id.clone());
    let total = |queue_len: usize, done: usize| (1 + done + queue_len).min(1 + MAX_HANDOFFS);
    let _ = events.send(RuntimeEvent::DiscussionStarted {
        team_id: context_id,
        team_name: scope.team_keys.as_ref().map(|(_, n)| n.clone()).unwrap_or_default(),
        total_turns: total(queue.len(), 0),
        speakers: queue.iter().map(|(k, _, _)| k.id.clone()).collect(),
        plan: queue.iter().map(|(k, from, _)| format!("{}: asked by {}", k.name, from)).collect(),
        kind: "handoff".into(),
    });

    let mut done = 0;
    while let Some((kwant, from, from_text)) = queue.pop_front() {
        if done >= MAX_HANDOFFS || scope.cancel_token.is_cancelled() {
            break;
        }
        let prompt = format!(
            "{from} handed this conversation to you by mentioning you.\n\nThe user's original request:\n{user}\n\n{from} wrote:\n{text}\n\n\
            Reply to the user now with your own part of this, in character. Don't repeat what {from} already said.",
            from = from,
            user = user_prompt,
            text = truncate_chars(&from_text, 4000),
        );
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
            Some(reply) => enqueue(&mut queue, &spoken, &kwant, &reply),
            None if scope.cancel_token.is_cancelled() => break,
            None => {}
        }
    }

    let _ = events.send(RuntimeEvent::DiscussionCompleted { total_turns: done + 1 });
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
        };
        run_kwant_turn(scope, &lead, prompt, opts).await;
    }

    let _ = events.send(RuntimeEvent::DiscussionCompleted { total_turns: turn });
}

/// Per-turn tweaks for turns that kwnt drives itself (team discussions).
#[derive(Default)]
struct TurnOpts {
    /// Saved as the message's `reasoning_content` instead of the driver's reasoning.
    label: Option<String>,
    /// Text checked for a URL to open in the bot browser; defaults to the prompt.
    nav_prompt: Option<String>,
    /// Discussion reply handling: `[PASS]` replies aren't saved; `Some(label)` lets an `[ASK_USER]`
    /// reply be saved as a question for the user under that label.
    discussion: Option<Option<String>>,
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
async fn try_kwant_turn(
    scope: &TurnScope,
    target_kwant: &Kwant,
    prompt: String,
    opts: TurnOpts,
) -> Result<String, String> {
    let context_id = scope
        .channel_id
        .clone()
        .or_else(|| scope.kwant_id.clone())
        .unwrap_or_else(|| target_kwant.id.clone());
    let _ = scope.state.event_sender.send(RuntimeEvent::TurnStarted {
        kwant_id: target_kwant.id.clone(),
        context_id: Some(context_id.clone()),
    });
    // Sends turn_ended even if this future is dropped mid-turn
    let mut ended = TurnEndedGuard {
        sender: scope.state.event_sender.clone(),
        kwant_id: target_kwant.id.clone(),
        context_id,
        outcome: "cancelled",
    };
    let reply = run_kwant_turn_inner(scope, target_kwant, prompt, opts).await;
    ended.outcome = match (&reply, scope.cancel_token.is_cancelled()) {
        (Ok(_), _) => "completed",
        (Err(_), true) => "cancelled",
        (Err(_), false) => "failed",
    };
    reply.map_err(|why| truncate_chars(&scrub_urls(why.trim()), MAX_FAILURE_CHARS).to_string())
}

/// `text` with every URL's userinfo and query dropped, so a failure reason that names a
/// custom endpoint can be stored without the credentials some endpoints carry there.
fn scrub_urls(text: &str) -> String {
    text.split(' ')
        .map(|word| {
            let Some(scheme_end) = word.find("://") else {
                return word.to_string();
            };
            let (scheme, rest) = word.split_at(scheme_end + 3);
            let host_end = rest.find(['/', '?', '#']).unwrap_or(rest.len());
            let (authority, path) = rest.split_at(host_end);
            let host = authority.rsplit_once('@').map_or(authority, |(_, h)| h);
            let path = match path.find(['?', '#']) {
                Some(q) => format!("{}?…", &path[..q]),
                None => path.to_string(),
            };
            format!("{scheme}{host}{path}")
        })
        .collect::<Vec<_>>()
        .join(" ")
}

struct TurnEndedGuard {
    sender: tokio::sync::broadcast::Sender<RuntimeEvent>,
    kwant_id: String,
    context_id: String,
    outcome: &'static str,
}

impl Drop for TurnEndedGuard {
    fn drop(&mut self) {
        let _ = self.sender.send(RuntimeEvent::TurnEnded {
            kwant_id: std::mem::take(&mut self.kwant_id),
            context_id: Some(std::mem::take(&mut self.context_id)),
            outcome: self.outcome.to_string(),
        });
    }
}

async fn run_kwant_turn_inner(
    scope: &TurnScope,
    target_kwant: &Kwant,
    prompt: String,
    opts: TurnOpts,
) -> Result<String, String> {
    let state = &scope.state;
    let event_broadcast = state.event_sender.clone();
    let db = state.db.clone();
    let router = state.router.clone();
    let channel_id = scope.channel_id.clone();
    let kwant_id = scope.kwant_id.clone();
    let team_keys = &scope.team_keys;
    let cancel_token = &scope.cancel_token;

    {
        let available_models = crate::drivers::detect_available_models(&db).await;
        let route = match router.route_prompt_with_models(&prompt, "chat", &available_models).await {
            Ok(r) => r,
            Err(_) => crate::router::RouteDecision {
                selected_model: target_kwant.preferred_model.clone(),
                effort_level: "medium".into(),
                confidence: 0.8,
                provider_endpoint: None,
                is_fallback: true,
                reasoning: "Fallback routing".into(),
            },
        };
        let route = crate::router::apply_preferred_model(route, &target_kwant.preferred_model);

        let target_model = route.selected_model;
        let effort_level = route.effort_level;

        let _ = event_broadcast.send(RuntimeEvent::ThinkingDelta(format!(
            "🔮 Laya Router: model='{}', effort='{}' ({:.0}% conf) — {}",
            target_model,
            effort_level,
            route.confidence * 100.0,
            route.reasoning
        )));

        let context_id = channel_id
            .as_deref()
            .or(kwant_id.as_deref())
            .unwrap_or(&target_kwant.id)
            .to_string();

        let history = db.list_messages(&context_id, 20).await.unwrap_or_default();

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
            let _ = event_broadcast.send(RuntimeEvent::ThinkingDelta(format!(
                "🌐 Navigating bot browser to {}...",
                target_url
            )));
            if let Some(ctrl) = get_or_create_browser_controller(state, &sandbox_target).await {
                match ctrl.navigate(&target_url).await {
                Ok(nav) => {
                    let page_text = ctrl.extract_content().await.unwrap_or_default();
                    let preview = truncate_chars(&page_text, 3000);
                    let _ = event_broadcast.send(RuntimeEvent::BrowserNavigated {
                        team_id: sandbox_target.clone(),
                        url: nav.url.clone(),
                        title: nav.title.clone(),
                        status_code: nav.status_code,
                    });
                    if let Some(b64) = ctrl.last_screenshot.lock().await.clone() {
                        let _ = event_broadcast.send(RuntimeEvent::BrowserScreenshot {
                            team_id: sandbox_target.clone(),
                            url: nav.url.clone(),
                            image_base64: b64,
                            timestamp: chrono::Utc::now().to_rfc3339(),
                        });
                    }
                    final_prompt = if let Some(kind) = crate::sandbox::detect_bot_wall(&nav.url, &nav.title, &page_text) {
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
                        let _ = event_broadcast.send(RuntimeEvent::BrowserScreenshot {
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

        let engine = crate::profiles::Engine::from_model(&target_model);
        let candidates = super::profile_turn::plan(state, engine, &target_kwant.id).await;
        let attempts: Vec<Option<LlmProfile>> = if candidates.is_empty() {
            vec![None]
        } else {
            candidates.into_iter().map(Some).collect()
        };
        // The agents' own continuity tools, for engines that can call MCP tools
        let self_scope = (engine.is_some() && super::continuity::agent_tools_enabled(state).await).then(|| {
            super::continuity::SelfScope { kwant_id: target_kwant.id.clone(), kwant_name: target_kwant.name.clone() }
        });
        let mcp_turn = super::mcp_runtime::prepare_turn_with_self(
            state,
            target_kwant,
            scope.workspace_dir.clone(),
            self_scope,
        )
        .await;
        let mcp_ctx = mcp_turn.as_ref().map(|(ctx, _)| ctx.clone());
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
            effort_level: Some(effort_level),
            team_context: scope.team_context.clone(),
            workspace_dir: scope.workspace_dir.clone(),
            sandbox_id: Some(sandbox_target.clone()),
            teammates: scope
                .teammates
                .iter()
                .filter(|k| k.id != target_kwant.id)
                .map(|k| crate::drivers::Teammate { name: k.name.clone(), title: k.title.clone() })
                .collect(),
            mcp: mcp_ctx.clone(),
            continuity,
        };

        let had_state = state.db.list_profile_states().await.unwrap_or_default();
        let turn_start = std::time::Instant::now();
        // Notices about profile switches, kept with the saved reply
        let mut notices: Vec<String> = Vec::new();
        let mut cancelled = false;
        let mut driver_result = None;
        let mut gave_up: Option<String> = None;
        for (i, profile) in attempts.iter().enumerate() {
            let driver = super::profile_turn::driver_for(
                state,
                profile.as_ref(),
                &target_model,
                configured_key.clone(),
                endpoint.clone(),
            )
            .await;
            let input = turn_input.clone();
            let (driver_tx, mut driver_rx) = mpsc::channel(32);
            let driver_task =
                tokio::spawn(async move { driver.stream_turn(input, driver_tx).await });

            // The driver's TurnComplete is held back: it's re-sent with the kwant and profile,
            // or dropped when the turn moves to another profile.
            let mut held_complete = None;
            let mut side_effects = false;
            loop {
                tokio::select! {
                    _ = cancel_token.cancelled() => {
                        cancelled = true;
                        driver_task.abort();
                        break;
                    }
                    event_opt = driver_rx.recv() => {
                        match event_opt {
                            Some(RuntimeEvent::TurnComplete { tokens, duration_ms, .. }) => {
                                held_complete = Some((tokens, duration_ms));
                            }
                            Some(event) => {
                                let visible = match &event {
                                    RuntimeEvent::TextDelta(t) => !t.trim().is_empty(),
                                    RuntimeEvent::ToolCallPending { .. } | RuntimeEvent::ApprovalRequired { .. } => true,
                                    _ => false,
                                };
                                side_effects |= visible;
                                let _ = event_broadcast.send(event);
                            }
                            None => break,
                        }
                    }
                }
            }
            if cancelled {
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
                let next = attempts.get(i + 1).and_then(|n| n.as_ref());
                if let (Some(from), Some(to), false) = (profile, next, side_effects) {
                    let notice = super::profile_turn::switch_notice(from, to, &failure);
                    let _ = event_broadcast.send(RuntimeEvent::ProfileSwitched {
                        kwant_id: target_kwant.id.clone(),
                        from_profile_id: from.id.clone(),
                        from_name: from.name.clone(),
                        to_profile_id: to.id.clone(),
                        to_name: to.name.clone(),
                        class: failure.class.as_str().into(),
                        retry_at: failure.retry_at,
                    });
                    let _ = event_broadcast.send(RuntimeEvent::ThinkingDelta(notice.clone()));
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
                let _ = event_broadcast.send(RuntimeEvent::TextDelta(text.clone()));
                let _ = event_broadcast.send(RuntimeEvent::TurnComplete {
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
            match &result {
                Ok(Err(e)) => {
                    let _ = event_broadcast.send(RuntimeEvent::Error {
                        code: "driver_error".into(),
                        message: e.to_string(),
                        kwant_id: Some(target_kwant.id.clone()),
                    });
                }
                Err(_) => {
                    let _ = event_broadcast.send(RuntimeEvent::Error {
                        code: "driver_crashed".into(),
                        message: "the model driver stopped unexpectedly".into(),
                        kwant_id: Some(target_kwant.id.clone()),
                    });
                }
                Ok(Ok(_)) => {}
            }
            if let Some((tokens, duration_ms)) = held_complete {
                let _ = event_broadcast.send(RuntimeEvent::TurnComplete {
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
            let _ = event_broadcast.send(RuntimeEvent::ThinkingDelta("⏹️ Cancelled by user".into()));
            return Err("stopped by the user".into());
        } else if let Some(text) = gave_up {
            let msg = NewMessage {
                channel_id: channel_id.clone(),
                direct_kwant_id: kwant_id.clone(),
                sender_role: "kwant".into(),
                sender_id: target_kwant.name.clone(),
                text: text.clone(),
                reasoning_content: with_notices(opts.label.clone()),
                tool_calls: None,
                tokens_used: 0,
            };
            let _ = db.insert_message(&msg).await;
            return Err(text);
        } else if let Some(Ok(Ok(output))) = driver_result {
            // Post-turn sync: If bot response outputs a URL, sync navigation so the user sees it in Sandbox VM
            if let Some(bot_url) = extract_url_from_text(&output.text) {
                if let Some(ctrl) = get_or_create_browser_controller(state, &sandbox_target).await {
                    let curr = ctrl.current_url.lock().await.clone().unwrap_or_default();
                    if curr != bot_url && !bot_url.contains("localhost") && !bot_url.contains("127.0.0.1") {
                        if let Ok(nav) = ctrl.navigate(&bot_url).await {
                            let _ = event_broadcast.send(RuntimeEvent::BrowserNavigated {
                                team_id: sandbox_target.clone(),
                                url: nav.url.clone(),
                                title: nav.title.clone(),
                                status_code: nav.status_code,
                            });
                            if let Some(b64) = ctrl.last_screenshot.lock().await.clone() {
                                let _ = event_broadcast.send(RuntimeEvent::BrowserScreenshot {
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

            let saved = match &opts.discussion {
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
                let asst_msg = NewMessage {
                    channel_id: channel_id.clone(),
                    direct_kwant_id: kwant_id.clone(),
                    sender_role: "kwant".into(),
                    sender_id: target_kwant.name.clone(),
                    text,
                    reasoning_content,
                    tool_calls: mcp_records.map(serde_json::Value::Array),
                    tokens_used: output.tokens_used as i64,
                };
                let _ = db.insert_message(&asst_msg).await;
            }
            return Ok(output.text);
        }
        Err(match driver_result {
            Some(Ok(Err(e))) => format!("the {target_model} driver failed: {e}"),
            Some(Err(_)) => format!("the {target_model} driver stopped unexpectedly"),
            _ => format!("the {target_model} turn ended without a reply"),
        })
    }
}

pub async fn send_chat(
    State(state): State<AppState>,
    Json(payload): Json<SendChatPayload>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    let all_kwants = state
        .db
        .list_kwants()
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    let all_teams = state
        .db
        .list_teams()
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
    } else if kwant_mention.is_none() && payload.kwant_id.is_none() {
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
                &state.config.data_dir,
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
    let saved_user_msg = state
        .db
        .insert_message(&user_msg)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    let cancel_token = CancellationToken::new();
    let session_id = saved_user_msg.id.clone();
    {
        let mut tokens = state.active_cancellations.lock().await;
        tokens.insert(session_id.clone(), cancel_token.clone());
        if let Some(cid) = &payload.channel_id {
            tokens.insert(cid.clone(), cancel_token.clone());
        }
        if let Some(kid) = &payload.kwant_id {
            tokens.insert(kid.clone(), cancel_token.clone());
        }
        if let Some(ref team) = target_team {
            tokens.insert(team.id.clone(), cancel_token.clone());
            tokens.insert(team.name.clone(), cancel_token.clone());
        }
    }

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
            TeamDecision::Direct { kwant_id, reason } => {
                let target_kwant = members
                    .iter()
                    .find(|k| k.id == kwant_id)
                    .or_else(|| all_kwants.iter().find(|k| k.id == kwant_id))
                    .unwrap_or(&members[0])
                    .clone();

                let _ = state.event_sender.send(RuntimeEvent::RoutingDecided {
                    context_id: routing_context_id,
                    kwant_id: target_kwant.id.clone(),
                    kwant_name: target_kwant.name.clone(),
                    kwant_title: target_kwant.title.clone(),
                    reason: reason.clone(),
                    mode: "direct".into(),
                });

                let _ = state.event_sender.send(RuntimeEvent::ThinkingDelta(format!(
                    "🔮 Laya Team Router: Routed to {} ({}) — {}",
                    target_kwant.name, target_kwant.title, reason
                )));

                let kwant_id_cleanup = payload.kwant_id.clone();
                execute_single_turn(
                    state.clone(),
                    target_kwant.clone(),
                    payload,
                    cancel_token,
                    session_id,
                    Some((team.id.clone(), team.name.clone())),
                    kwant_id_cleanup,
                    active_team_context,
                    workspace_dir.clone(),
                    members.clone(),
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
                    team_keys: Some((team.id.clone(), team.name.clone())),
                    team_context: Some(crate::drivers::TeamContext {
                        id: team.id.clone(),
                        name: team.name.clone(),
                        description: team.description.clone(),
                        workspace_path: team.workspace_path.clone(),
                        repo_url: team.repo_url.clone(),
                    }),
                    workspace_dir: workspace_dir.clone(),
                    teammates: members.clone(),
                    cancel_token: cancel_token.clone(),
                };
                let team_clone = team.clone();
                let prompt = payload.prompt.clone();
                let session_id_clone = session_id.clone();
                let active_cancellations = state.active_cancellations.clone();

                tokio::spawn(async move {
                    run_discussion(&scope, &team_clone, planned, &prompt).await;

                    let mut tokens = active_cancellations.lock().await;
                    tokens.remove(&session_id_clone);
                    if let Some(cid) = &scope.channel_id {
                        tokens.remove(cid);
                    }
                    if let Some(kid) = &scope.kwant_id {
                        tokens.remove(kid);
                    }
                    tokens.remove(&team_clone.id);
                    tokens.remove(&team_clone.name);
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
        } else if let Some(cid) = &payload.channel_id {
            if let Ok(Some(channel)) = state.db.get_channel(cid).await {
                let lead = all_kwants
                    .iter()
                    .find(|k| k.id == channel.lead_kwant_id)
                    .or_else(|| all_kwants.first())
                    .cloned();
                // #general is the hub for every agent, including ones created after it was seeded
                let members: Vec<Kwant> = if channel.name == "general" {
                    all_kwants.clone()
                } else {
                    let member_ids: Vec<String> =
                        serde_json::from_str(&channel.member_kwants).unwrap_or_default();
                    member_ids
                        .iter()
                        .filter_map(|mid| all_kwants.iter().find(|k| &k.id == mid).cloned())
                        .collect()
                };

                if channel.routing_mode == "auto" && members.len() > 1 {
                    let context_id = payload.channel_id.clone();
                    let _ = state.event_sender.send(RuntimeEvent::RoutingStarted {
                        context_id: context_id.clone(),
                        candidates: routing_candidates(&members),
                    });
                    let (chosen_id, reason) = state
                        .router
                        .route_channel_query(&payload.prompt, &channel.name, &channel.lead_kwant_id, &members)
                        .await;
                    let chosen = members.iter().find(|k| k.id == chosen_id).cloned().or(lead);
                    if let Some(ref k) = chosen {
                        let _ = state.event_sender.send(RuntimeEvent::RoutingDecided {
                            context_id,
                            kwant_id: k.id.clone(),
                            kwant_name: k.name.clone(),
                            kwant_title: k.title.clone(),
                            reason,
                            mode: "direct".into(),
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
        let kwant_id_cleanup = payload.kwant_id.clone();
        execute_single_turn(
            state.clone(),
            target_kwant.clone(),
            payload,
            cancel_token,
            session_id,
            team_keys,
            kwant_id_cleanup,
            active_team_context,
            workspace_dir,
            chat_teammates,
        );

        Ok(Json(json!({
            "status": "dispatched",
            "user_message_id": saved_user_msg.id,
            "kwant": target_kwant.name
        })))
    }
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
    let resolved = state.broker.resolve(&ticket_id, decision).await;
    if !resolved {
        return Err(StatusCode::CONFLICT);
    }
    let _ = state.db.update_approval_status(&ticket_id, status_str).await;

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
}

#[derive(Deserialize)]
pub struct SaveSettingsPayload {
    pub anthropic_api_key: Option<String>,
    pub xai_api_key: Option<String>,
    pub openai_api_key: Option<String>,
    pub ollama_url: Option<String>,
    /// https only; "" removes it, the mask keeps it.
    pub notify_webhook_url: Option<String>,
    pub notify_on: Option<Vec<String>>,
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
    }))
}

pub async fn save_settings(
    State(state): State<AppState>,
    Json(payload): Json<SaveSettingsPayload>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    // Checked first, so a bad webhook URL saves nothing
    if let Err(e) =
        super::continuity::save_notify_settings(&state, payload.notify_webhook_url, payload.notify_on).await
    {
        return Err((StatusCode::BAD_REQUEST, Json(json!({ "error": e }))));
    }
    if let Some(key) = payload.anthropic_api_key {
        if !key.starts_with('•') && !key.is_empty() {
            let _ = state.db.set_setting("anthropic_api_key", &key).await;
        }
    }
    if let Some(key) = payload.xai_api_key {
        if !key.starts_with('•') && !key.is_empty() {
            let _ = state.db.set_setting("xai_api_key", &key).await;
        }
    }
    if let Some(key) = payload.openai_api_key {
        if !key.starts_with('•') && !key.is_empty() {
            let _ = state.db.set_setting("openai_api_key", &key).await;
        }
    }
    if let Some(url) = payload.ollama_url {
        if !url.is_empty() {
            let _ = state.db.set_setting("ollama_url", &url).await;
        }
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

pub async fn resolve_sandbox_target_info(state: &AppState, target_id: &str) -> Option<(String, PathBuf)> {
    if let Ok(Some(team)) = state.db.get_team(target_id).await {
        let ws = team
            .workspace_path
            .as_deref()
            .filter(|s| !s.trim().is_empty())
            .map(PathBuf::from)
            .unwrap_or_else(|| state.config.data_dir.join("workspaces").join(&team.name));
        Some((team.name, ws))
    } else if let Ok(Some(channel)) = state.db.get_channel(target_id).await {
        let safe_cname = channel.name.to_lowercase().replace(|c: char| !c.is_alphanumeric(), "-");
        let ws = state.config.data_dir.join("workspaces").join(format!("chan-{}", safe_cname));
        Some((format!("chan-{}", safe_cname), ws))
    } else if let Ok(Some(kwant)) = state.db.get_kwant(target_id).await {
        let safe_kname = kwant.name.to_lowercase().replace(|c: char| !c.is_alphanumeric(), "-");
        Some((format!("bot-{}", safe_kname), state.config.data_dir.join("workspaces").join(format!("bot-{}", safe_kname))))
    } else if let Ok(Some(team)) = state.db.get_team_by_name(target_id).await {
        let ws = team
            .workspace_path
            .as_deref()
            .filter(|s| !s.trim().is_empty())
            .map(PathBuf::from)
            .unwrap_or_else(|| state.config.data_dir.join("workspaces").join(&team.name));
        Some((team.name, ws))
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
    let mut controllers = state.browser_controllers.lock().await;
    if let Some(ctrl) = controllers.get(team_id) {
        if !ctrl.is_managed_session() || ctrl.refresh_location().await.is_ok() { return Some(ctrl.clone()); }
        if ctrl.control.try_write().is_err() { return None; }
        controllers.remove(team_id);
    }
    let (_, workspace) = resolve_sandbox_target_info(state, team_id).await?;
    match state.browser_service.session(team_id, &workspace).await {
        Ok(ctrl) => {
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

fn browser_unavailable() -> (StatusCode, Json<serde_json::Value>) {
    (StatusCode::SERVICE_UNAVAILABLE, Json(json!({
        "error": "Sandbox browser is unavailable. Check that Docker is running, then restart the browser.",
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
        .ok_or_else(browser_unavailable)?;
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
        "error":if running {None} else {Some("Browser service unavailable. Check Docker or KWNT_BROWSER_CDP_URL.")} })))
}

pub async fn navigate_sandbox(
    State(state): State<AppState>,
    Path(team_id): Path<String>,
    Json(payload): Json<NavigatePayload>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    let ctrl = get_or_create_browser_controller(&state, &team_id).await
        .ok_or_else(browser_unavailable)?;
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

    let ctrl = get_or_create_browser_controller(&state, &team_id).await.ok_or_else(|| {
        (StatusCode::SERVICE_UNAVAILABLE, Json(json!({
            "error": "Sandbox browser is unavailable. Check that Docker is running, then restart the browser."
        })))
    })?;

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
    if let Some(kind) = crate::sandbox::detect_bot_wall(&url, &title, &text) {
        page["blocked"] = json!(true);
        page["notice"] = json!(bot_wall_notice(kind, &url));
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
        .ok_or_else(browser_unavailable)?;
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
        .ok_or_else(browser_unavailable)?;
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

    if tool_name == "execute_shell" {
        if let Some(tid) = team_id {
            if let Some((name, workspace)) = resolve_sandbox_target_info(state, tid).await {
                tokio::fs::create_dir_all(&workspace).await?;
                let sandbox = state.sandbox_manager.ensure_sandbox_running(tid, &name, &workspace).await
                    .map_err(|e| crate::tools::ToolError::Execution(e.to_string()))?;
                let out = state.sandbox_manager.exec_in_sandbox(&sandbox.container_name,
                    args["command"].as_str().unwrap_or(""), 30).await
                    .map_err(|e| crate::tools::ToolError::Execution(e.to_string()))?;
                return Ok(json!({"command":out.command,"stdout":out.stdout,"stderr":out.stderr,"exit_code":out.exit_code,"success":out.success}));
            }
        }
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

#[derive(Deserialize)]
pub struct SavePasswordPayload {
    pub domain: Option<String>,
    pub origin: Option<String>,
    pub username: String,
    pub password: String,
}

pub async fn get_saved_passwords(
    State(state): State<AppState>,
    Path(team_id): Path<String>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    let controller = get_or_create_browser_controller(&state, &team_id).await;
    if let Some(ctrl) = controller {
        let list = ctrl.list_saved_credentials().await;
        Ok(Json(json!({ "passwords": list })))
    } else {
        // Fallback: check workspace directly
        if let Some((_, ws_dir)) = resolve_sandbox_target_info(&state, &team_id).await {
            let creds_file = ws_dir.join(".saved_passwords.json");
            if creds_file.exists() {
                if let Ok(content) = std::fs::read_to_string(&creds_file) {
                    if let Ok(creds) = serde_json::from_str::<serde_json::Value>(&content) {
                        return Ok(Json(json!({ "passwords": creds })));
                    }
                }
            }
        }
        Ok(Json(json!({ "passwords": [] })))
    }
}

pub async fn save_password(
    State(state): State<AppState>,
    Path(team_id): Path<String>,
    Json(payload): Json<SavePasswordPayload>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    let controller = get_or_create_browser_controller(&state, &team_id).await;
    let domain = payload.domain.unwrap_or_default();
    let origin = payload.origin.unwrap_or_default();

    if let Some(ctrl) = controller {
        let saved = ctrl.save_credential(&domain, &origin, &payload.username, &payload.password).await;
        Ok(Json(json!({ "status": "saved", "credential": saved })))
    } else if let Some((_, ws_dir)) = resolve_sandbox_target_info(&state, &team_id).await {
        let ctrl = crate::sandbox::BrowserController::with_workspace(19222, ws_dir);
        let saved = ctrl.save_credential(&domain, &origin, &payload.username, &payload.password).await;
        Ok(Json(json!({ "status": "saved", "credential": saved })))
    } else {
        Err(StatusCode::NOT_FOUND)
    }
}

pub async fn delete_password(
    State(state): State<AppState>,
    Path((team_id, cred_id)): Path<(String, String)>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    let controller = get_or_create_browser_controller(&state, &team_id).await;
    if let Some(ctrl) = controller {
        let removed = ctrl.delete_saved_credential(&cred_id).await;
        Ok(Json(json!({ "status": if removed { "deleted" } else { "not_found" } })))
    } else if let Some((_, ws_dir)) = resolve_sandbox_target_info(&state, &team_id).await {
        let ctrl = crate::sandbox::BrowserController::with_workspace(19222, ws_dir);
        let removed = ctrl.delete_saved_credential(&cred_id).await;
        Ok(Json(json!({ "status": if removed { "deleted" } else { "not_found" } })))
    } else {
        Err(StatusCode::NOT_FOUND)
    }
}


#[cfg(test)]
mod tests {
    use super::scrub_urls;

    #[test]
    fn failure_reasons_drop_url_credentials() {
        assert_eq!(
            scrub_urls("HTTP API error: error sending request for url (https://u:p4ss@llm.example/v1/chat?key=sk-123&x=1)"),
            "HTTP API error: error sending request for url (https://llm.example/v1/chat?…"
        );
        assert_eq!(scrub_urls("see http://host:8080/a#frag now"), "see http://host:8080/a?… now");
        assert_eq!(scrub_urls("not logged in; run `codex login`"), "not logged in; run `codex login`");
    }
}
