use super::{routes::get_or_create_browser_controller, AppState};
use crate::sandbox::browser::{BrowserController, BrowserError};
use axum::{extract::{Path, State, WebSocketUpgrade, ws::{Message, WebSocket}}, http::{HeaderMap, StatusCode}, response::Response};
use futures::{SinkExt, StreamExt};
use serde::Deserialize;
use serde_json::{json, Value};
use std::time::Duration;
use tokio::{sync::OwnedRwLockWriteGuard, time::{timeout, Instant}};
use tokio_tungstenite::{connect_async, tungstenite::Message as CdpMessage};

#[derive(Debug, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum BrowserInput {
    Click { x: f64, y: f64 },
    Type { text: String },
    Key { key: String, code: Option<String> },
    Scroll { x: f64, y: f64, delta_x: f64, delta_y: f64 },
    Navigate { url: String },
}

impl BrowserController {
    /// Input only: the independent screencast delivers visual changes.
    pub async fn dispatch_input(&self, input: BrowserInput) -> Result<(), BrowserError> {
        let point = |x: f64, y: f64| -> Result<(), BrowserError> {
            if x.is_finite() && y.is_finite() && x >= 0.0 && y >= 0.0 && x < 1280.0 && y < 800.0 { Ok(()) }
            else { Err(BrowserError::Protocol("Pointer outside viewport".into())) }
        };
        match input {
            BrowserInput::Click { x, y } => {
                point(x, y)?;
                for event in ["mousePressed", "mouseReleased"] {
                    self.send_cdp_command("Input.dispatchMouseEvent", json!({"type":event,"x":x,"y":y,"button":"left","clickCount":1})).await?;
                }
            }
            BrowserInput::Type { text } => {
                if text.len() > 16384 { return Err(BrowserError::Protocol("Text too long".into())); }
                self.send_cdp_command("Input.insertText", json!({"text":text})).await?;
            }
            BrowserInput::Key { key, code } => {
                let keycode = match key.as_str() {
                    "Enter"=>13,"Backspace"=>8,"Tab"=>9,"Escape"=>27,"ArrowLeft"=>37,
                    "ArrowUp"=>38,"ArrowRight"=>39,"ArrowDown"=>40,"Delete"=>46,"Home"=>36,"End"=>35,
                    _ => return Err(BrowserError::Protocol("Unsupported key".into())),
                };
                for event in ["rawKeyDown", "keyUp"] {
                    self.send_cdp_command("Input.dispatchKeyEvent", json!({"type":event,"key":key,"code":code.as_deref().unwrap_or_default(),"windowsVirtualKeyCode":keycode})).await?;
                }
            }
            BrowserInput::Scroll { x, y, delta_x, delta_y } => {
                point(x,y)?;
                if !delta_x.is_finite() || !delta_y.is_finite() { return Err(BrowserError::Protocol("Invalid scroll".into())); }
                self.send_cdp_command("Input.dispatchMouseEvent", json!({"type":"mouseWheel","x":x,"y":y,"deltaX":delta_x,"deltaY":delta_y})).await?;
            }
            BrowserInput::Navigate { url } => {
                let parsed = reqwest::Url::parse(&url).map_err(|e| BrowserError::Protocol(e.to_string()))?;
                if !matches!(parsed.scheme(), "http"|"https") { return Err(BrowserError::Protocol("Use an HTTP or HTTPS URL".into())); }
                self.navigate_unlocked(&url).await?;
            }
        }
        Ok(())
    }
}

pub async fn upgrade(State(state): State<AppState>, Path(id): Path<String>, headers: HeaderMap, ws: WebSocketUpgrade) -> Result<Response, StatusCode> {
    // Prevent a third-party website from controlling an authenticated same-origin session.
    let origin = headers.get("origin").and_then(|v| v.to_str().ok()).and_then(|s| reqwest::Url::parse(s).ok()).ok_or(StatusCode::FORBIDDEN)?;
    let host = headers.get("host").and_then(|v| v.to_str().ok()).ok_or(StatusCode::FORBIDDEN)?;
    let expected = reqwest::Url::parse(&format!("{}://{}", origin.scheme(), host)).map_err(|_| StatusCode::FORBIDDEN)?;
    if !matches!(origin.scheme(), "http" | "https") || origin.origin() != expected.origin() { return Err(StatusCode::FORBIDDEN); }
    Ok(ws.max_message_size(32 * 1024).on_upgrade(move |socket| async move {
        let result = serve(socket, state, id).await;
        if let Err(error) = result { tracing::debug!("Browser stream ended: {}", error); }
    }))
}

async fn serve(mut socket: WebSocket, state: AppState, id: String) -> Result<(), BrowserError> {
    let ctrl = match get_or_create_browser_controller(&state, &id).await {
        Some(ctrl) => ctrl,
        None => {
            let _ = socket.send(Message::Text(json!({"type":"error","error":"Browser service unavailable. Check Docker or KWNT_BROWSER_CDP_URL."}).to_string())).await;
            return Ok(());
        }
    };
    let (mut cdp, _) = connect_async(ctrl.get_ws_url().await?).await.map_err(|e| BrowserError::Ws(e.to_string()))?;
    let mut command_id = 1u64;
    for method in ["Page.enable", "Page.startScreencast"] {
        cdp.send(CdpMessage::Text(json!({"id":command_id,"method":method,"params": if method == "Page.startScreencast" {json!({"format":"jpeg","quality":65,"maxWidth":1280,"maxHeight":800,"everyNthFrame":1})} else {json!({})}}).to_string())).await.map_err(|e| BrowserError::Ws(e.to_string()))?;
        command_id += 1;
    }
    let mut lease: Option<OwnedRwLockWriteGuard<()>> = None;
    let mut last_client = Instant::now();
    let mut heartbeat = tokio::time::interval(Duration::from_secs(10));
    let mut pending_frame: Option<i64> = None;
    let mut frame_sent = Instant::now();
    let mut next_frame = 0u64;
    let mut pending_token = 0u64;
    let mut location_tick = tokio::time::interval(Duration::from_secs(2));
    loop {
        tokio::select! {
            _ = heartbeat.tick() => {
                if last_client.elapsed() > Duration::from_secs(35) { break; }
                if pending_frame.is_some() && frame_sent.elapsed() > Duration::from_secs(15) { break; }
                send(&mut socket, json!({"type":"ping"})).await?;
            }
            _ = location_tick.tick() => {
                let _ = ctrl.refresh_location().await;
                send(&mut socket, json!({"type":"location","url":ctrl.current_url.lock().await.clone(),"title":ctrl.current_title.lock().await.clone()})).await?;
            }
            message = cdp.next() => {
                let text = match message {
                    Some(Ok(CdpMessage::Text(text))) => text,
                    Some(Ok(CdpMessage::Ping(_))) | Some(Ok(CdpMessage::Pong(_))) => continue,
                    _ => break,
                };
                let event: Value = serde_json::from_str(&text).unwrap_or(Value::Null);
                if let Some(error) = event.get("error") {
                    send(&mut socket, json!({"type":"error","error":format!("Browser stream failed: {error}")})).await?;
                    break;
                }
                if event["method"] == "Page.screencastFrame" {
                    let session = event["params"]["sessionId"].as_i64().unwrap_or_default();
                    if pending_frame.is_some() {
                        cdp.send(CdpMessage::Text(json!({"id":command_id,"method":"Page.screencastFrameAck","params":{"sessionId":session}}).to_string())).await.map_err(|e| BrowserError::Ws(e.to_string()))?;
                        command_id += 1;
                        continue;
                    }
                    next_frame += 1;
                    pending_token = next_frame;
                    pending_frame = Some(session);
                    frame_sent = Instant::now();
                    send(&mut socket, json!({"type":"frame","id":next_frame,"image":event["params"]["data"],"width":1280,"height":800})).await?;
                }
            }
            message = socket.next() => {
                let text = match message {
                    Some(Ok(Message::Text(text))) => text,
                    Some(Ok(Message::Ping(_))) | Some(Ok(Message::Pong(_))) => { last_client = Instant::now(); continue; },
                    _ => break,
                };
                last_client = Instant::now();
                let value: Value = match serde_json::from_str(&text) { Ok(v) => v, Err(_) => break };
                match value["action"].as_str().unwrap_or("") {
                    "pong" => {},
                    "frame_ack" => {
                        if value["id"].as_u64() == Some(pending_token) {
                            if let Some(session) = pending_frame.take() {
                                cdp.send(CdpMessage::Text(json!({"id":command_id,"method":"Page.screencastFrameAck","params":{"sessionId":session}}).to_string())).await.map_err(|e| BrowserError::Ws(e.to_string()))?;
                                command_id += 1;
                            }
                        }
                    }
                    "take_control" => {
                        if lease.is_none() {
                            match ctrl.take_control().await {
                                Ok(guard) => lease = Some(guard),
                                Err(error) => { send(&mut socket, json!({"type":"error","error":error.to_string()})).await?; continue; }
                            }
                        }
                        send(&mut socket, json!({"type":"control","active":true})).await?;
                    }
                    "release_control" => {
                        lease = None;
                        send(&mut socket, json!({"type":"control","active":false})).await?;
                    }
                    _ => {
                        let result = if lease.is_none() { Err(BrowserError::HumanControl) } else {
                            match serde_json::from_value::<BrowserInput>(value.clone()) {
                                Ok(input) => ctrl.dispatch_input(input).await,
                                Err(_) => Err(BrowserError::Protocol("Invalid browser input".into())),
                            }
                        };
                        send(&mut socket, match result {
                            Ok(()) => json!({"type":"ack","id":value["id"]}),
                            Err(error) => json!({"type":"error","id":value["id"],"error":error.to_string()}),
                        }).await?;
                    }
                }
            }
        }
    }
    // Dropping the lease releases bot actions even on disconnect or timeout.
    drop(lease);
    let _ = cdp.close(None).await;
    Ok(())
}

async fn send(socket: &mut WebSocket, value: Value) -> Result<(), BrowserError> {
    timeout(Duration::from_secs(5), socket.send(Message::Text(value.to_string()))).await
        .map_err(|_| BrowserError::Timeout)?.map_err(|e| BrowserError::Ws(e.to_string()))
}
