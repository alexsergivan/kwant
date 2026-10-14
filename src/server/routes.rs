use super::AppState;
use crate::broker::ApprovalDecision;
use crate::db::models::*;
use crate::drivers::{find_cli_binary, resolve_driver_for_model, RuntimeEvent, TurnInput};
use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use serde::{Deserialize, Serialize};
use serde_json::json;
use tokio::sync::mpsc;

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

#[derive(Debug, Deserialize)]
pub struct SendChatPayload {
    pub channel_id: Option<String>,
    pub kwant_id: Option<String>,
    pub prompt: String,
    pub sender_id: String,
}

pub async fn send_chat(
    State(state): State<AppState>,
    Json(payload): Json<SendChatPayload>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    // 1. Identify Kwant to execute
    let kwants = state
        .db
        .list_kwants()
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    let target_kwant = if let Some(kid) = &payload.kwant_id {
        kwants.into_iter().find(|k| &k.id == kid)
    } else if let Some(cid) = &payload.channel_id {
        if let Ok(Some(channel)) = state.db.get_channel(cid).await {
            kwants.into_iter().find(|k| k.id == channel.lead_kwant_id)
        } else {
            kwants.into_iter().next()
        }
    } else {
        kwants.into_iter().next()
    }
    .ok_or(StatusCode::BAD_REQUEST)?;

    // 2. Insert user message to DB
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

    // 3. Automatically detect available models and route via Laya
    let available_models = crate::drivers::detect_available_models(&state.db).await;
    let route = state
        .router
        .route_prompt_with_models(&payload.prompt, "chat", &available_models)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    let target_model = route.selected_model;
    let effort_level = route.effort_level;

    // Broadcast Laya routing & effort decision to SSE
    let _ = state.event_sender.send(RuntimeEvent::ThinkingDelta(format!(
        "🔮 Laya Router: model='{}', effort='{}' ({:.0}% conf) — {}",
        target_model,
        effort_level,
        route.confidence * 100.0,
        route.reasoning
    )));

    // 4. Load recent conversation history
    let context_id = payload
        .channel_id
        .as_deref()
        .or(payload.kwant_id.as_deref())
        .unwrap_or(&target_kwant.id);
    let history = state
        .db
        .list_messages(context_id, 20)
        .await
        .unwrap_or_default();

    // 5. Build turn input and driver
    let configured_key = if target_model.contains("claude") {
        state.db.get_setting("anthropic_api_key").await.ok().flatten()
    } else if target_model.contains("grok") {
        state.db.get_setting("xai_api_key").await.ok().flatten()
    } else if target_model.contains("openai") || target_model.contains("codex") || target_model.contains("gpt") {
        state.db.get_setting("openai_api_key").await.ok().flatten()
    } else {
        None
    };

    let endpoint = if target_model == "local_ollama" {
        state.db.get_setting("ollama_url").await.ok().flatten()
    } else {
        None
    };

    let driver = resolve_driver_for_model(&target_model, configured_key, endpoint);
    let turn_input = TurnInput {
        thread_id: context_id.to_string(),
        kwant: target_kwant.clone(),
        prompt: payload.prompt.clone(),
        history,
        model_override: Some(target_model.clone()),
        effort_level: Some(effort_level),
    };

    // 6. Spawn driver execution in background, feeding events into the SSE event_sender
    let event_broadcast = state.event_sender.clone();
    let db = state.db.clone();
    let channel_id = payload.channel_id.clone();
    let kwant_id = payload.kwant_id.clone();
    let target_kwant_clone = target_kwant.clone();

    tokio::spawn(async move {
        let (driver_tx, mut driver_rx) = mpsc::channel(32);

        let driver_task = tokio::spawn(async move {
            driver.stream_turn(turn_input, driver_tx).await
        });

        // Forward events to SSE clients
        while let Some(event) = driver_rx.recv().await {
            let _ = event_broadcast.send(event);
        }

        if let Ok(Ok(output)) = driver_task.await {
            // Save assistant message to DB
            let asst_msg = NewMessage {
                channel_id,
                direct_kwant_id: kwant_id,
                sender_role: "kwant".into(),
                sender_id: target_kwant_clone.name,
                text: output.text,
                reasoning_content: output.reasoning,
                tool_calls: None,
                tokens_used: output.tokens_used as i64,
            };
            let _ = db.insert_message(&asst_msg).await;
        }
    });

    Ok(Json(json!({
        "status": "dispatched",
        "user_message_id": saved_user_msg.id,
        "kwant": target_kwant.name
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

