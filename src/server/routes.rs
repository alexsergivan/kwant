use super::AppState;
use crate::broker::ApprovalDecision;
use crate::db::models::*;
use crate::drivers::{find_cli_binary, resolve_driver_for_model, RuntimeEvent, TurnInput};
use crate::router::{parse_mentions_with_lookup, TagMention, TeamDecision};
use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
    Json,
};
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

pub async fn cancel_chat(
    State(state): State<AppState>,
    body: axum::body::Bytes,
) -> Result<Json<serde_json::Value>, StatusCode> {
    let mut tokens = state.active_cancellations.lock().await;
    let maybe_session_id = if !body.is_empty() {
        if let Ok(val) = serde_json::from_slice::<serde_json::Value>(&body) {
            val.get("session_id")
                .or_else(|| val.get("channel_id"))
                .or_else(|| val.get("team_id"))
                .and_then(|s| s.as_str())
                .map(|s| s.to_string())
        } else {
            None
        }
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
) {
    let event_broadcast = state.event_sender.clone();
    let db = state.db.clone();
    let router = state.router.clone();
    let active_cancellations = state.active_cancellations.clone();
    let channel_id = payload.channel_id.clone();
    let kwant_id = payload.kwant_id.clone();
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

                execute_single_turn(
                    state.clone(),
                    target_kwant.clone(),
                    payload,
                    cancel_token,
                    session_id,
                    Some((team.id.clone(), team.name.clone())),
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

                    let mut previous_turn_output: Option<(String, String)> = None;

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

                        let speaker_prompt = if let Some((prev_name, prev_text)) = &previous_turn_output {
                            format!(
                                "Original query: {}\n\nPrevious response from {}:\n{}\n\nPlease provide your analysis, critique, or next steps according to the discussion plan.",
                                prompt, prev_name, prev_text
                            )
                        } else {
                            prompt.clone()
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
                            previous_turn_output = Some((speaker.name.clone(), output.text));
                        }
                    }

                    if !cancel_token_clone.is_cancelled() {
                        let _ = event_broadcast.send(RuntimeEvent::DiscussionCompleted {
                            total_turns: turns,
                        });
                    }

                    let mut tokens = active_cancellations.lock().await;
                    tokens.remove(&session_id_clone);
                    if let Some(cid) = &channel_id {
                        tokens.remove(cid);
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

        execute_single_turn(
            state.clone(),
            target_kwant.clone(),
            payload,
            cancel_token,
            session_id,
            None,
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

