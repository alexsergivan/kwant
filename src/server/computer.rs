//! Settings → Computer: whether bots use this computer (its Chrome and shell) or the isolated
//! vm containers, and the approval every local shell command needs.
use super::AppState;
use crate::broker::{ApprovalDecision, ApprovalRequest};
use crate::db::models::NewApproval;
use crate::drivers::events::RuntimeEvent;
use crate::sandbox::computer::{self, ComputerMode};
use axum::{extract::State, http::StatusCode, Json};
use serde::Deserialize;
use serde_json::{json, Value};
use std::path::Path;

/// The mode in force: the saved choice, else `KWNT_COMPUTER_MODE`, else the platform default.
pub async fn current_mode(state: &AppState) -> ComputerMode {
    let saved = state
        .db
        .get_setting(computer::SETTING_KEY)
        .await
        .ok()
        .flatten();
    computer::current_mode(saved.as_deref())
}

/// Points the browser service at the mode in force. On a change the old browser is
/// disconnected (never closed) and the next browser call opens a page in the new one.
pub async fn apply_mode(state: &AppState) -> ComputerMode {
    let mode = current_mode(state).await;
    if state.browser_service.mode() != mode {
        let mut controllers = state.browser_controllers.lock().await;
        if state.browser_service.set_mode(mode).await {
            for team_id in controllers.drain().map(|(id, _)| id) {
                let _ = state.event_sender.send(RuntimeEvent::SandboxStatusChanged {
                    team_id,
                    container_name: "browser-service".into(),
                    is_running: false,
                });
            }
        }
    }
    mode
}

async fn view(state: &AppState) -> Value {
    let mode = current_mode(state).await;
    let local = computer::local_reason();
    let vm = computer::vm_unavailable_reason(
        state.sandbox_manager.is_docker_available().await,
        state.browser_service.has_endpoint(),
    );
    let window = computer::window_reason();
    json!({
        "mode": mode,
        "available": {"local": local.is_none(), "vm": vm.is_none()},
        "reasons": {"local": local, "vm": vm},
        "window": {
            "visible": mode == ComputerMode::Local && state.browser_service.local_window(),
            "available": window.is_none(),
            "reason": window,
        },
    })
}

/// `GET /api/settings/computer`
pub async fn get_computer(State(state): State<AppState>) -> Json<Value> {
    Json(view(&state).await)
}

#[derive(Deserialize)]
pub struct ComputerPayload {
    pub mode: ComputerMode,
}

/// `PUT /api/settings/computer` with `{"mode":"local"|"vm"}`: saves the choice and switches at
/// once. 400 for an unknown mode or one this computer cannot run (with the reason).
pub async fn put_computer(
    State(state): State<AppState>,
    payload: Result<Json<ComputerPayload>, axum::extract::rejection::JsonRejection>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let bad = |m: String| (StatusCode::BAD_REQUEST, Json(json!({ "error": m })));
    let Json(ComputerPayload { mode }) =
        payload.map_err(|_| bad("mode must be \"local\" or \"vm\"".into()))?;
    let current = view(&state).await;
    if current["available"][mode.as_str()] != json!(true) {
        let reason = current["reasons"][mode.as_str()]
            .as_str()
            .unwrap_or("unavailable");
        return Err(bad(reason.to_string()));
    }
    state
        .db
        .set_setting(computer::SETTING_KEY, mode.as_str())
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({ "error": e.to_string() })),
            )
        })?;
    apply_mode(&state).await;
    Ok(Json(view(&state).await))
}

#[derive(Deserialize)]
pub struct WindowPayload {
    pub visible: bool,
}

/// `PUT /api/settings/computer/window` with `{"visible":bool}`: in local mode, shows the bots'
/// Chrome as a real window (for a sign-in or CAPTCHA) or puts it back in the app. Each bot's open
/// page is reopened in the other form. 400 outside local mode or with no display for a window;
/// 502 if Chrome could not be restarted (the previous form is kept).
pub async fn put_window(
    State(state): State<AppState>,
    payload: Result<Json<WindowPayload>, axum::extract::rejection::JsonRejection>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let fail = |status, m: String| (status, Json(json!({ "error": m })));
    let Json(WindowPayload { visible }) = payload
        .map_err(|_| fail(StatusCode::BAD_REQUEST, "visible must be true or false".into()))?;
    if apply_mode(&state).await != ComputerMode::Local {
        return Err(fail(
            StatusCode::BAD_REQUEST,
            "The browser runs in the VM, which has no window to open. Switch to Local computer first.".into(),
        ));
    }
    if visible {
        if let Some(reason) = computer::window_reason() {
            return Err(fail(StatusCode::BAD_REQUEST, reason));
        }
    }
    {
        // the restarted Chrome has new pages: every controller must reconnect
        let mut controllers = state.browser_controllers.lock().await;
        if let Err(error) = state.browser_service.set_local_window(visible).await {
            tracing::warn!("Could not restart local Chrome: {}", error);
            return Err(fail(
                StatusCode::BAD_GATEWAY,
                format!("Could not restart Chrome: {error}"),
            ));
        }
        for team_id in controllers.drain().map(|(id, _)| id) {
            let _ = state.event_sender.send(RuntimeEvent::SandboxStatusChanged {
                team_id,
                container_name: "browser-service".into(),
                is_running: false,
            });
        }
    }
    Ok(Json(view(&state).await))
}

/// Asks the person before a shell command runs on their own computer. Every command asks:
/// "always allow" is honoured for this one command only, never remembered.
pub async fn approve_local_shell(
    state: &AppState,
    target_id: &str,
    command: &str,
    cwd: &Path,
) -> bool {
    let details = json!({
        "source": "local_shell",
        "tool": "execute_shell",
        "arguments": {"command": command, "cwd": cwd.to_string_lossy()},
        "kwant_id": target_id,
    });
    let ticket = match state
        .db
        .save_approval(&NewApproval {
            turn_id: String::new(),
            kwant_id: target_id.to_string(),
            tool_name: "execute_shell".into(),
            parameters: details.clone(),
        })
        .await
    {
        Ok(a) => a.id,
        Err(e) => {
            tracing::warn!(error = %e, "could not store local shell approval; denying");
            return false;
        }
    };
    let rx = state
        .broker
        .request_approval(ApprovalRequest {
            id: ticket.clone(),
            kwant_id: target_id.to_string(),
            tool_name: "execute_shell".into(),
            details: details.clone(),
        })
        .await;
    let _ = state.event_sender.send(RuntimeEvent::ApprovalRequired {
        id: ticket.clone(),
        tool_name: "execute_shell".into(),
        details,
    });
    match tokio::time::timeout(super::mcp_runtime::APPROVAL_TIMEOUT, rx).await {
        Ok(Ok(ApprovalDecision::AllowOnce | ApprovalDecision::AlwaysAllowSession)) => true,
        Ok(_) => false,
        Err(_) => {
            if state.broker.cancel(&ticket).await {
                let _ = state.db.update_approval_status(&ticket, "expired").await;
                let _ = state
                    .event_sender
                    .send(RuntimeEvent::ApprovalWithdrawn { id: ticket });
            }
            false
        }
    }
}
