use super::AppState;
use crate::broker::ApprovalDecision;
use crate::db::models::*;
use crate::drivers::{find_cli_binary, resolve_driver_for_model, RuntimeEvent, TurnInput};
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
}

pub async fn health(State(state): State<AppState>) -> impl IntoResponse {
    Json(HealthResponse {
        status: "ok",
        app: "kwnt",
        version: env!("CARGO_PKG_VERSION"),
        laya_url: state.config.laya_url.clone(),
    })
}

pub async fn list_kwants(State(state): State<AppState>) -> Result<Json<Vec<Kwant>>, StatusCode> {
    state
        .db
        .list_kwants()
        .await
        .map(Json)
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)
}

pub async fn create_kwant(
    State(state): State<AppState>,
    Json(payload): Json<NewKwant>,
) -> Result<Json<Kwant>, StatusCode> {
    state
        .db
        .create_kwant(&payload)
        .await
        .map(Json)
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)
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

pub async fn update_team(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(payload): Json<NewTeam>,
) -> Result<Json<Team>, StatusCode> {
    match state.db.update_team(&id, &payload).await {
        Ok(Some(team)) => Ok(Json(team)),
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
) {
    let event_broadcast = state.event_sender.clone();
    let db = state.db.clone();
    let router = state.router.clone();
    let active_cancellations = state.active_cancellations.clone();
    let channel_id = payload.channel_id.clone();
    let kwant_id = kwant_id.or_else(|| payload.kwant_id.clone());
    let prompt = payload.prompt.clone();

    tokio::spawn(async move {
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

        let driver = resolve_driver_for_model(&target_model, configured_key, endpoint);
        let turn_input = TurnInput {
            thread_id: context_id,
            kwant: target_kwant.clone(),
            prompt,
            history,
            model_override: Some(target_model),
            effort_level: Some(effort_level),
            team_context,
            workspace_dir,
        };

        let (driver_tx, mut driver_rx) = mpsc::channel(32);
        let driver_task = tokio::spawn(async move {
            driver.stream_turn(turn_input, driver_tx).await
        });

        let mut cancelled = false;
        loop {
            tokio::select! {
                _ = cancel_token.cancelled() => {
                    cancelled = true;
                    driver_task.abort();
                    break;
                }
                event_opt = driver_rx.recv() => {
                    match event_opt {
                        Some(event) => {
                            let _ = event_broadcast.send(event);
                        }
                        None => break,
                    }
                }
            }
        }

        if cancelled {
            let _ = event_broadcast.send(RuntimeEvent::ThinkingDelta("⏹️ Cancelled by user".into()));
        } else if let Ok(Ok(output)) = driver_task.await {
            let asst_msg = NewMessage {
                channel_id: channel_id.clone(),
                direct_kwant_id: kwant_id.clone(),
                sender_role: "kwant".into(),
                sender_id: target_kwant.name.clone(),
                text: output.text,
                reasoning_content: output.reasoning,
                tool_calls: None,
                tokens_used: output.tokens_used as i64,
            };
            let _ = db.insert_message(&asst_msg).await;
        }

        let mut tokens = active_cancellations.lock().await;
        tokens.remove(&session_id);
        if let Some(cid) = &channel_id {
            tokens.remove(cid);
        }
        if let Some(kid) = &kwant_id {
            tokens.remove(kid);
        }
        if let Some((team_id, team_name)) = &team_keys {
            tokens.remove(team_id);
            tokens.remove(team_name);
        }
    });
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
                let _ = state.event_sender.send(RuntimeEvent::ThinkingDelta(format!(
                    "🔮 Laya Team Router: Initiating {}-turn autonomous discussion with @{}",
                    turns, team.name
                )));

                let event_broadcast = state.event_sender.clone();
                let db = state.db.clone();
                let router = state.router.clone();
                let channel_id = payload.channel_id.clone();
                let kwant_id = payload.kwant_id.clone();
                let prompt = payload.prompt.clone();
                let active_cancellations = state.active_cancellations.clone();
                let all_kwants_clone = all_kwants.clone();
                let team_clone = team.clone();
                let speakers_clone = speakers.clone();
                let plan_clone = plan.clone();
                let cancel_token_clone = cancel_token.clone();
                let session_id_clone = session_id.clone();
                let workspace_dir_clone = workspace_dir.clone();

                tokio::spawn(async move {
                    let context_id = channel_id
                        .as_deref()
                        .or(kwant_id.as_deref())
                        .unwrap_or(&team_clone.id)
                        .to_string();

                    let _ = event_broadcast.send(RuntimeEvent::DiscussionStarted {
                        team_id: team_clone.id.clone(),
                        team_name: team_clone.name.clone(),
                        total_turns: turns,
                        speakers: speakers_clone.clone(),
                        plan: plan_clone.clone(),
                    });

                    let mut cumulative_transcript: Vec<(String, String)> = Vec::new();

                    for (turn_idx, speaker_id) in speakers_clone.iter().enumerate() {
                        if cancel_token_clone.is_cancelled() {
                            break;
                        }

                        let speaker = match all_kwants_clone.iter().find(|k| &k.id == speaker_id) {
                            Some(k) => k.clone(),
                            None => {
                                tracing::warn!("Speaker Kwant ID {} not found", speaker_id);
                                continue;
                            }
                        };

                        let speaker_prompt = if cumulative_transcript.is_empty() {
                            prompt.clone()
                        } else {
                            let turns_formatted = cumulative_transcript
                                .iter()
                                .enumerate()
                                .map(|(idx, (name, text))| format!("### Turn {} ({})\n{}", idx + 1, name, text))
                                .collect::<Vec<_>>()
                                .join("\n\n");
                            format!(
                                "Original user query: {}\n\nDiscussion so far:\n{}\n\nPlease provide your analysis, critique, or synthesis according to the discussion plan.",
                                prompt, turns_formatted
                            )
                        };

                        let _ = event_broadcast.send(RuntimeEvent::DiscussionTurnStarted {
                            speaker_id: speaker.id.clone(),
                            speaker_name: speaker.name.clone(),
                            turn_index: turn_idx + 1,
                            total_turns: turns,
                            prompt: speaker_prompt.clone(),
                        });

                        let available_models = crate::drivers::detect_available_models(&db).await;
                        let route = router
                            .route_prompt_with_models(&speaker_prompt, "chat", &available_models)
                            .await
                            .unwrap_or_else(|_| crate::router::RouteDecision {
                                selected_model: speaker.preferred_model.clone(),
                                effort_level: "medium".into(),
                                confidence: 0.8,
                                provider_endpoint: None,
                                is_fallback: true,
                                reasoning: "Fallback routing".into(),
                            });

                        let target_model = route.selected_model;
                        let effort_level = route.effort_level;

                        let configured_key = get_configured_key(&db, &target_model).await;
                        let endpoint = get_configured_endpoint(&db, &target_model).await;

                        let driver = resolve_driver_for_model(&target_model, configured_key, endpoint);
                        let turn_input = TurnInput {
                            thread_id: context_id.clone(),
                            kwant: speaker.clone(),
                            prompt: speaker_prompt,
                            history: Vec::new(),
                            model_override: Some(target_model.clone()),
                            effort_level: Some(effort_level),
                            team_context: Some(crate::drivers::TeamContext {
                                id: team_clone.id.clone(),
                                name: team_clone.name.clone(),
                                description: team_clone.description.clone(),
                                workspace_path: team_clone.workspace_path.clone(),
                                repo_url: team_clone.repo_url.clone(),
                            }),
                            workspace_dir: workspace_dir_clone.clone(),
                        };

                        let (driver_tx, mut driver_rx) = mpsc::channel(32);
                        let driver_task = tokio::spawn(async move {
                            driver.stream_turn(turn_input, driver_tx).await
                        });

                        let mut turn_cancelled = false;
                        loop {
                            tokio::select! {
                                _ = cancel_token_clone.cancelled() => {
                                    turn_cancelled = true;
                                    driver_task.abort();
                                    break;
                                }
                                event_opt = driver_rx.recv() => {
                                    match event_opt {
                                        Some(event) => {
                                            let _ = event_broadcast.send(event);
                                        }
                                        None => break,
                                    }
                                }
                            }
                        }

                        if turn_cancelled {
                            let _ = event_broadcast.send(RuntimeEvent::ThinkingDelta(
                                "⏹️ Discussion stopped by user".into(),
                            ));
                            break;
                        }

                        if let Ok(Ok(output)) = driver_task.await {
                            let asst_msg = NewMessage {
                                channel_id: channel_id.clone(),
                                direct_kwant_id: Some(speaker.id.clone()),
                                sender_role: "kwant".into(),
                                sender_id: speaker.name.clone(),
                                text: output.text.clone(),
                                reasoning_content: Some(format!(
                                    "Team Discussion • Turn {} of {} • @{}",
                                    turn_idx + 1,
                                    turns,
                                    team_clone.name
                                )),
                                tool_calls: None,
                                tokens_used: output.tokens_used as i64,
                            };
                            let _ = db.insert_message(&asst_msg).await;
                            cumulative_transcript.push((speaker.name.clone(), output.text));
                        }
                    }

                    let _ = event_broadcast.send(RuntimeEvent::DiscussionCompleted {
                        total_turns: turns,
                    });

                    let mut tokens = active_cancellations.lock().await;
                    tokens.remove(&session_id_clone);
                    if let Some(cid) = &channel_id {
                        tokens.remove(cid);
                    }
                    if let Some(kid) = &kwant_id {
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
                all_kwants
                    .iter()
                    .find(|k| k.id == channel.lead_kwant_id)
                    .cloned()
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

    let _ = state.db.update_approval_status(&ticket_id, status_str).await;
    let resolved = state.broker.resolve(&ticket_id, decision).await;

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
}

#[derive(Deserialize)]
pub struct SaveSettingsPayload {
    pub anthropic_api_key: Option<String>,
    pub xai_api_key: Option<String>,
    pub openai_api_key: Option<String>,
    pub ollama_url: Option<String>,
}

pub async fn get_settings(State(state): State<AppState>) -> Result<Json<SettingsResponse>, StatusCode> {
    let anthropic = state.db.get_setting("anthropic_api_key").await.unwrap_or(None).unwrap_or_default();
    let xai = state.db.get_setting("xai_api_key").await.unwrap_or(None).unwrap_or_default();
    let openai = state.db.get_setting("openai_api_key").await.unwrap_or(None).unwrap_or_default();
    let ollama = state.db.get_setting("ollama_url").await.unwrap_or(None).unwrap_or_else(|| "http://localhost:11434/v1".to_string());

    let claude_bin = find_cli_binary("claude");
    let grok_bin = find_cli_binary("grok");
    let codex_bin = find_cli_binary("codex");

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
    }))
}

pub async fn save_settings(
    State(state): State<AppState>,
    Json(payload): Json<SaveSettingsPayload>,
) -> Result<Json<serde_json::Value>, StatusCode> {
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
        if trimmed.starts_with("~/") {
            if let Some(ref home) = home_dir {
                home.join(&trimmed[2..])
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
    directories.sort_by(|a, b| a.to_lowercase().cmp(&b.to_lowercase()));

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

pub async fn get_sandbox_status(
    State(state): State<AppState>,
    Path(team_id): Path<String>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    let sandbox = state.sandbox_manager.get_sandbox(&team_id).await;
    match sandbox {
        Some(s) => Ok(Json(json!({
            "team_id": s.team_id,
            "container_name": s.container_name,
            "cdp_port": s.cdp_port,
            "workspace_path": s.workspace_path.to_string_lossy(),
            "is_running": s.is_running
        }))),
        None => Ok(Json(json!({
            "team_id": team_id,
            "container_name": "",
            "cdp_port": 0,
            "workspace_path": "",
            "is_running": false
        }))),
    }
}

pub async fn get_browser_screenshot(
    State(state): State<AppState>,
    Path(team_id): Path<String>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    let controller = {
        let lock = state.browser_controllers.lock().await;
        lock.get(&team_id).cloned()
    };

    let placeholder = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mNkYAAAAAYAAjCB0C8AAAAASUVORK5CYII=";
    let (b64, url) = if let Some(ctrl) = controller {
        let shot = match ctrl.capture_screenshot().await {
            Ok(img) => img,
            Err(_) => {
                let lock = ctrl.last_screenshot.lock().await;
                lock.clone().unwrap_or_else(|| placeholder.to_string())
            }
        };
        let curr_url = ctrl.current_url.lock().await.clone().unwrap_or_default();
        (shot, curr_url)
    } else {
        (placeholder.to_string(), String::new())
    };

    let timestamp = chrono::Utc::now().to_rfc3339();
    let _ = state.event_tx.send(RuntimeEvent::BrowserScreenshot {
        team_id: team_id.clone(),
        url: url.clone(),
        image_base64: b64.clone(),
        timestamp: timestamp.clone(),
    });

    Ok(Json(json!({
        "team_id": team_id,
        "url": url,
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
    let _ = state.sandbox_manager.stop_sandbox(&team_id).await;
    {
        let mut controllers = state.browser_controllers.lock().await;
        controllers.remove(&team_id);
    }

    let _ = state.event_tx.send(RuntimeEvent::SandboxStatusChanged {
        team_id: team_id.clone(),
        container_name: format!("kwnt-sandbox-{}", team_id),
        is_running: false,
    });

    Ok(Json(json!({
        "status": "restarted",
        "team_id": team_id,
        "message": "Sandbox stopped and cleaned"
    })))
}

pub async fn navigate_sandbox(
    State(state): State<AppState>,
    Path(team_id): Path<String>,
    Json(payload): Json<NavigatePayload>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    let controller = {
        let lock = state.browser_controllers.lock().await;
        lock.get(&team_id).cloned()
    };

    let (title, status_code) = if let Some(ctrl) = controller {
        match ctrl.navigate(&payload.url).await {
            Ok(res) => (res.title, res.status_code),
            Err(e) => {
                tracing::warn!("Navigation failed: {}, falling back", e);
                (payload.url.clone(), 200)
            }
        }
    } else {
        (payload.url.clone(), 200)
    };

    let _ = state.event_tx.send(RuntimeEvent::BrowserNavigated {
        team_id: team_id.clone(),
        url: payload.url.clone(),
        title: title.clone(),
        status_code,
    });

    Ok(Json(json!({
        "status": "navigated",
        "team_id": team_id,
        "url": payload.url,
        "title": title,
        "status_code": status_code
    })))
}

pub async fn execute_tool(
    state: &AppState,
    team_id: Option<&str>,
    tool_name: &str,
    args: &serde_json::Value,
) -> Result<serde_json::Value, crate::tools::ToolError> {
    let controller = if let Some(tid) = team_id {
        let lock = state.browser_controllers.lock().await;
        lock.get(tid).cloned()
    } else {
        None
    };

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
            let _ = state.event_tx.send(RuntimeEvent::BrowserNavigated {
                team_id: team_str,
                url,
                title,
                status_code,
            });
        }
        "browser_screenshot" => {
            let url = result["url"].as_str().unwrap_or_default().to_string();
            let image_base64 = result["image_base64"].as_str().unwrap_or_default().to_string();
            let timestamp = chrono::Utc::now().to_rfc3339();
            let _ = state.event_tx.send(RuntimeEvent::BrowserScreenshot {
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

