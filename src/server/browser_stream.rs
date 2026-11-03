//! The live view of a bot's browser: a CDP screencast out, the person's input in.
//!
//! Protocol v1 (`click`, `type`, `key` without `type`, `scroll`, `navigate`, `pointer_*`) keeps
//! working for the phone overlay. Protocol v2 adds full mouse events, wheel and key events with
//! modifiers, `resize`, select interception, JS dialogs, `copy` and cursor updates. Input goes
//! out on the stream's own DevTools socket, in order, so moves are fire-and-forget and never
//! wait behind a bot's command.
use super::browser_input::{self as bi, KeyEvent, KeyKind, MouseButton, RateLimit};
use super::{routes::get_or_create_browser_controller, AppState};
use crate::sandbox::browser::{BrowserController, BrowserError, HeldPointer, FOCUSED_EDITABLE_JS, MOUSE_BUTTON_BITS};
use crate::sandbox::browser_history::HistoryDirection;
use axum::{extract::{Path, State, WebSocketUpgrade, ws::{Message, WebSocket}}, http::{HeaderMap, StatusCode}, response::Response};
use futures::{SinkExt, StreamExt};
use serde::Deserialize;
use serde_json::{json, Value};
use std::{collections::HashMap, sync::{Arc, atomic::{AtomicBool, AtomicU64, Ordering}}, time::Duration};
use tokio::{sync::{mpsc, oneshot, OwnedRwLockWriteGuard}, task::JoinHandle, time::{timeout, Instant}};
use tokio_tungstenite::tungstenite::Message as CdpMessage;

/// The viewport the screencast starts at when the controller has none saved.
const DEFAULT_VIEW: (u32, u32) = (1280, 800);
/// How long a discrete action may take before its ack becomes an error. Long, because a click
/// that opens `alert()` only completes once the person answers the dialog.
const ACTION_WAIT: Duration = Duration::from_secs(30);
/// Page lookups (select under the pointer, cursor, focus) give up fast: they block while a
/// dialog is open.
const PROBE_WAIT: Duration = Duration::from_secs(2);
const SELECT_GROUP: &str = "kwnt-select";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MouseKind {
    Move,
    Down,
    Up,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum BrowserInput {
    Click { x: f64, y: f64 },
    Type { text: String },
    /// v1 when `type` is missing: a named key pressed and released. v2 with `type` down or up.
    Key {
        key: String,
        code: Option<String>,
        #[serde(rename = "type")]
        kind: Option<KeyKind>,
        text: Option<String>,
        #[serde(default)]
        modifiers: u8,
        location: Option<u8>,
        #[serde(default)]
        auto_repeat: bool,
    },
    Scroll { x: f64, y: f64, delta_x: f64, delta_y: f64 },
    Navigate { url: String },
    /// Left button down, for drags such as slider CAPTCHAs. Held until `pointer_up`, control
    /// release or disconnect.
    PointerDown { x: f64, y: f64 },
    PointerMove { x: f64, y: f64 },
    PointerUp { x: f64, y: f64 },
    /// v2: any button, click count and modifiers. Moves get no ack.
    Mouse {
        #[serde(rename = "type")]
        kind: MouseKind,
        x: f64,
        y: f64,
        #[serde(default)]
        button: MouseButton,
        buttons: Option<u8>,
        click_count: Option<u32>,
        #[serde(default)]
        modifiers: u8,
    },
    /// v2 wheel with modifiers (Ctrl+wheel zooms a map, Shift+wheel scrolls sideways). No ack.
    Wheel {
        x: f64,
        y: f64,
        #[serde(default)]
        delta_x: f64,
        #[serde(default)]
        delta_y: f64,
        #[serde(default)]
        modifiers: u8,
    },
    /// The person's viewer size, applied while they control and put back afterwards.
    Resize { width: f64, height: f64, dpr: Option<f64> },
    /// Picks options in the select the last `select` message described.
    SelectOption { index: Option<i64>, indexes: Option<Vec<i64>> },
    /// Answers the open JS dialog.
    Dialog { accept: bool, prompt_text: Option<String> },
    /// The page's selected text, sent back as `clipboard`.
    Copy,
    /// Back, forward or reload in the tab's own history, acked like `navigate`.
    History { direction: HistoryDirection },
}

impl BrowserInput {
    pub fn parse(value: &Value) -> Result<Self, BrowserError> {
        let input = serde_json::from_value::<BrowserInput>(value.clone()).map_err(|_| BrowserError::Protocol("Invalid browser input".into()))?;
        let modifiers = match &input {
            BrowserInput::Key { modifiers, .. } | BrowserInput::Mouse { modifiers, .. } | BrowserInput::Wheel { modifiers, .. } => *modifiers,
            _ => 0,
        };
        if modifiers > 15 {
            return Err(BrowserError::Protocol("Invalid modifiers".into()));
        }
        Ok(input)
    }

    /// Streamed input: no ack, and dropped rather than queued while a dialog is open.
    pub fn is_stream(&self) -> bool {
        matches!(self, BrowserInput::Mouse { kind: MouseKind::Move, .. } | BrowserInput::Wheel { .. })
    }

    /// Lets go of something already held (a button or key up), so it still goes through while a
    /// dialog is open and nothing stays pressed.
    fn is_release(&self) -> bool {
        matches!(self, BrowserInput::Mouse { kind: MouseKind::Up, .. } | BrowserInput::PointerUp { .. } | BrowserInput::Key { kind: Some(KeyKind::Up), .. })
    }
}

impl BrowserController {
    /// Where a live-view button is held down, if one is.
    pub fn pointer_held(&self) -> Option<(f64, f64)> {
        self.pointer_held_slot().lock().unwrap_or_else(|e| e.into_inner()).map(|p| (p.x, p.y))
    }

    /// Lets go of every held live-view button where it was last seen, so the bot never gets a
    /// page mid-drag.
    pub async fn release_pointer(&self) {
        let held = self.pointer_held_slot().lock().unwrap_or_else(|e| e.into_inner()).take();
        for params in held.map(release_events).unwrap_or_default() {
            let _ = self.send_cdp_command("Input.dispatchMouseEvent", params).await;
        }
    }

    /// Sends one v1 action over the controller's own connection, outside a live view. Tests and
    /// callers without a stream use it; the live view itself goes through its own socket.
    pub async fn dispatch_input(&self, input: BrowserInput) -> Result<(), BrowserError> {
        let (width, height) = self.fixed_viewport().unwrap_or((1280, 800));
        let point = |x: f64, y: f64| bi::in_viewport(x, y, width as f64, height as f64).map_err(protocol);
        let mouse = |params: Value| self.send_cdp_command("Input.dispatchMouseEvent", params);
        match input {
            BrowserInput::Click { x, y } => {
                let (x, y) = point(x, y)?;
                for event in ["mousePressed", "mouseReleased"] {
                    mouse(json!({"type":event,"x":x,"y":y,"button":"left","buttons": if event == "mousePressed" {1} else {0},"clickCount":1})).await?;
                }
            }
            BrowserInput::Type { text } => {
                if text.len() > 16384 { return Err(protocol("Text too long")); }
                self.send_cdp_command("Input.insertText", json!({"text":text})).await?;
            }
            BrowserInput::Scroll { x, y, delta_x, delta_y } => {
                let (x, y) = point(x, y)?;
                if !delta_x.is_finite() || !delta_y.is_finite() { return Err(protocol("Invalid scroll")); }
                mouse(json!({"type":"mouseWheel","x":x,"y":y,"deltaX":delta_x,"deltaY":delta_y})).await?;
            }
            BrowserInput::PointerDown { x, y } => {
                let (x, y) = point(x, y)?;
                self.release_pointer().await;
                mouse(json!({"type":"mousePressed","x":x,"y":y,"button":"left","buttons":1,"clickCount":1})).await?;
                *self.pointer_held_slot().lock().unwrap_or_else(|e| e.into_inner()) = Some(HeldPointer { x, y, buttons: 1 });
            }
            BrowserInput::PointerMove { x, y } => {
                let (x, y) = point(x, y)?;
                let buttons = self.pointer_held_slot().lock().unwrap_or_else(|e| e.into_inner()).map(|h| h.buttons).unwrap_or(0);
                mouse(json!({"type":"mouseMoved","x":x,"y":y,"button":bi::moving_button(buttons),"buttons":buttons})).await?;
                if buttons != 0 {
                    *self.pointer_held_slot().lock().unwrap_or_else(|e| e.into_inner()) = Some(HeldPointer { x, y, buttons });
                }
            }
            BrowserInput::PointerUp { x, y } => {
                let (x, y) = point(x, y)?;
                let held = self.pointer_held_slot().lock().unwrap_or_else(|e| e.into_inner()).take();
                if held.is_some_and(|h| h.buttons & 1 != 0) {
                    mouse(json!({"type":"mouseReleased","x":x,"y":y,"button":"left","buttons":0,"clickCount":1})).await?;
                }
            }
            BrowserInput::Navigate { url } => {
                let parsed = reqwest::Url::parse(&url).map_err(|e| BrowserError::Protocol(e.to_string()))?;
                if !matches!(parsed.scheme(), "http"|"https") { return Err(protocol("Use an HTTP or HTTPS URL")); }
                self.navigate_unlocked(&url).await?;
            }
            BrowserInput::History { direction } => self.navigate_history(direction).await?,
            _ => return Err(protocol("This action needs the live view")),
        }
        Ok(())
    }
}

/// The `mouseReleased` events that let go of every held button, one per button in bit order.
fn release_events(held: HeldPointer) -> Vec<Value> {
    let mut buttons = held.buttons;
    MOUSE_BUTTON_BITS.iter().filter(|(bit, _)| held.buttons & bit != 0).map(|(bit, name)| {
        buttons &= !bit;
        json!({"type":"mouseReleased","x":held.x,"y":held.y,"button":name,"buttons":buttons,"clickCount":1})
    }).collect()
}

type Reply = oneshot::Receiver<Result<Value, BrowserError>>;
type Waiters = Arc<std::sync::Mutex<HashMap<u64, oneshot::Sender<Result<Value, BrowserError>>>>>;

/// A DevTools socket shared by the stream's tasks: one writer keeps commands in order, one reader
/// routes replies to whoever waits and events to the stream.
#[derive(Clone)]
struct Cdp {
    out: mpsc::UnboundedSender<String>,
    pending: Waiters,
    next: Arc<AtomicU64>,
}

impl Cdp {
    fn open(ws: crate::sandbox::browser::WsStream) -> (Self, mpsc::Receiver<Value>, [JoinHandle<()>; 2]) {
        let (mut sink, mut stream) = ws.split();
        let (out, mut out_rx) = mpsc::unbounded_channel::<String>();
        let (event_tx, events) = mpsc::channel::<Value>(64);
        let cdp = Cdp { out, pending: Arc::default(), next: Arc::new(AtomicU64::new(1)) };
        let writer = tokio::spawn(async move {
            while let Some(text) = out_rx.recv().await {
                if sink.send(CdpMessage::Text(text)).await.is_err() { break; }
            }
            let _ = sink.close().await;
        });
        let pending = cdp.pending.clone();
        let reader = tokio::spawn(async move {
            while let Some(message) = stream.next().await {
                let text = match message {
                    Ok(CdpMessage::Text(text)) => text,
                    Ok(CdpMessage::Ping(_)) | Ok(CdpMessage::Pong(_)) => continue,
                    _ => break,
                };
                let value: Value = serde_json::from_str(&text).unwrap_or(Value::Null);
                if let Some(id) = value.get("id").and_then(Value::as_u64) {
                    let waiter = pending.lock().unwrap_or_else(|e| e.into_inner()).remove(&id);
                    if let Some(waiter) = waiter {
                        let _ = waiter.send(match value.get("error") {
                            Some(error) => Err(BrowserError::Protocol(error["message"].as_str().map(str::to_string).unwrap_or_else(|| error.to_string()))),
                            None => Ok(value.get("result").cloned().unwrap_or(Value::Null)),
                        });
                    }
                } else if value.get("method").is_some() && event_tx.send(value).await.is_err() {
                    break;
                }
            }
            // waiters see a closed channel; the stream sees its events end
            pending.lock().unwrap_or_else(|e| e.into_inner()).clear();
        });
        (cdp, events, [writer, reader])
    }

    fn send(&self, id: u64, method: &str, params: Value) {
        let _ = self.out.send(json!({"id":id,"method":method,"params":params}).to_string());
    }

    /// Sends without waiting; the reply is dropped.
    fn fire(&self, method: &str, params: Value) {
        self.send(self.next.fetch_add(1, Ordering::SeqCst), method, params);
    }

    /// Sends now; the reply arrives on the returned channel.
    fn request(&self, method: &str, params: Value) -> Reply {
        let id = self.next.fetch_add(1, Ordering::SeqCst);
        let (tx, rx) = oneshot::channel();
        self.pending.lock().unwrap_or_else(|e| e.into_inner()).insert(id, tx);
        self.send(id, method, params);
        rx
    }

    async fn call(&self, method: &str, params: Value, wait: Duration) -> Result<Value, BrowserError> {
        settle(self.request(method, params), wait).await
    }
}

async fn settle(reply: Reply, wait: Duration) -> Result<Value, BrowserError> {
    match timeout(wait, reply).await {
        Ok(Ok(result)) => result,
        Ok(Err(_)) => Err(BrowserError::Ws("Browser connection closed".into())),
        Err(_) => Err(BrowserError::Timeout),
    }
}

/// State the stream and its worker share.
struct Shared {
    /// The page viewport in CSS px, from frame metadata or the last resize.
    view: std::sync::Mutex<(f64, f64)>,
    /// A `resize` is applied and must be put back on hand back.
    resized: AtomicBool,
    cursor: std::sync::Mutex<String>,
    cursor_busy: AtomicBool,
    cursor_at: std::sync::Mutex<Option<Instant>>,
}

impl Shared {
    fn point(&self, x: f64, y: f64) -> Result<(f64, f64), BrowserError> {
        let (w, h) = *self.view.lock().unwrap_or_else(|e| e.into_inner());
        bi::in_viewport(x, y, w, h).map_err(|e| BrowserError::Protocol(e.into()))
    }

    fn set_view(&self, width: f64, height: f64) {
        *self.view.lock().unwrap_or_else(|e| e.into_inner()) = (width, height);
    }

    fn view(&self) -> (f64, f64) {
        *self.view.lock().unwrap_or_else(|e| e.into_inner())
    }
}

enum Job {
    Input { input: BrowserInput, id: Value },
    HandBack(Option<oneshot::Sender<()>>),
}

/// Runs input in the order it came, on the stream's socket.
struct Worker {
    ctrl: BrowserController,
    cdp: Cdp,
    shared: Arc<Shared>,
    client: mpsc::UnboundedSender<Value>,
    /// Left presses that opened our select list instead of reaching the page: their up is
    /// swallowed too.
    swallowed: u8,
    /// The select the last `select` message described, as a remote object.
    open_select: Option<String>,
}

fn ack(id: &Value, extra: Value) -> Value {
    let mut ack = json!({"type":"ack","id":id});
    if let (Some(ack), Value::Object(extra)) = (ack.as_object_mut(), extra) {
        ack.extend(extra);
    }
    ack
}

fn error(id: &Value, error: impl std::fmt::Display) -> Value {
    json!({"type":"error","id":id,"error":error.to_string()})
}

fn protocol(message: &str) -> BrowserError {
    BrowserError::Protocol(message.into())
}

impl Worker {
    fn held(&self) -> Option<HeldPointer> {
        *self.ctrl.pointer_held_slot().lock().unwrap_or_else(|e| e.into_inner())
    }

    fn set_held(&self, held: Option<HeldPointer>) {
        *self.ctrl.pointer_held_slot().lock().unwrap_or_else(|e| e.into_inner()) = held.filter(|h| h.buttons != 0);
    }

    fn mouse(&self, params: Value) -> Reply {
        self.cdp.request("Input.dispatchMouseEvent", params)
    }

    /// Acks once every reply is in, with whether a text field has focus when `editable`.
    fn finish(&self, id: Value, replies: Vec<Reply>, editable: bool) {
        let (client, cdp) = (self.client.clone(), self.cdp.clone());
        tokio::spawn(async move {
            for reply in replies {
                if let Err(e) = settle(reply, ACTION_WAIT).await {
                    let _ = client.send(error(&id, e));
                    return;
                }
            }
            let extra = if editable { json!({"editable": focused_editable(&cdp).await}) } else { json!({}) };
            let _ = client.send(ack(&id, extra));
        });
    }

    async fn run(mut self, mut jobs: mpsc::UnboundedReceiver<Job>) {
        while let Some(job) = jobs.recv().await {
            match job {
                Job::Input { input, id } => {
                    if let Err(e) = self.handle(input, &id).await {
                        let _ = self.client.send(error(&id, e));
                    }
                }
                Job::HandBack(done) => {
                    self.hand_back().await;
                    if let Some(done) = done { let _ = done.send(()); }
                }
            }
        }
    }

    async fn handle(&mut self, input: BrowserInput, id: &Value) -> Result<(), BrowserError> {
        match input {
            BrowserInput::Click { x, y } => {
                let (x, y) = self.shared.point(x, y)?;
                if self.offer_select(x, y).await {
                    let _ = self.client.send(ack(id, json!({"editable": false})));
                    return Ok(());
                }
                let replies = ["mousePressed", "mouseReleased"].into_iter()
                    .map(|kind| self.mouse(json!({"type":kind,"x":x,"y":y,"button":"left","buttons": if kind == "mousePressed" {1} else {0},"clickCount":1})))
                    .collect();
                self.finish(id.clone(), replies, true);
            }
            BrowserInput::Type { text } => {
                if text.len() > 16384 { return Err(protocol("Text too long")); }
                let reply = self.cdp.request("Input.insertText", json!({"text":text}));
                self.finish(id.clone(), vec![reply], false);
            }
            BrowserInput::Key { key, code, kind: None, .. } => {
                let keycode = match key.as_str() {
                    "Enter"=>13,"Backspace"=>8,"Tab"=>9,"Escape"=>27,"ArrowLeft"=>37,
                    "ArrowUp"=>38,"ArrowRight"=>39,"ArrowDown"=>40,"Delete"=>46,"Home"=>36,"End"=>35,
                    _ => return Err(protocol("Unsupported key")),
                };
                let replies = ["rawKeyDown", "keyUp"].into_iter().map(|kind| self.cdp.request("Input.dispatchKeyEvent",
                    json!({"type":kind,"key":key,"code":code.as_deref().unwrap_or_default(),"windowsVirtualKeyCode":keycode}))).collect();
                self.finish(id.clone(), replies, false);
            }
            BrowserInput::Key { key, code, kind: Some(kind), text, modifiers, location, auto_repeat } => {
                let event = KeyEvent { kind, key: &key, code: code.as_deref().unwrap_or_default(), text: text.as_deref(), modifiers, location, auto_repeat };
                let params = bi::key_params(&event).map_err(protocol)?;
                let reply = self.cdp.request("Input.dispatchKeyEvent", params);
                self.finish(id.clone(), vec![reply], false);
            }
            BrowserInput::Scroll { x, y, delta_x, delta_y } => {
                let (x, y) = self.shared.point(x, y)?;
                if !delta_x.is_finite() || !delta_y.is_finite() { return Err(protocol("Invalid scroll")); }
                let reply = self.mouse(json!({"type":"mouseWheel","x":x,"y":y,"deltaX":delta_x,"deltaY":delta_y}));
                self.finish(id.clone(), vec![reply], false);
            }
            BrowserInput::Wheel { x, y, delta_x, delta_y, modifiers } => {
                let (x, y) = self.shared.point(x, y)?;
                if !delta_x.is_finite() || !delta_y.is_finite() || delta_x.abs() > 100_000.0 || delta_y.abs() > 100_000.0 {
                    return Err(protocol("Invalid scroll"));
                }
                let buttons = self.held().map(|h| h.buttons).unwrap_or(0);
                self.cdp.fire("Input.dispatchMouseEvent", json!({"type":"mouseWheel","x":x,"y":y,"deltaX":delta_x,"deltaY":delta_y,"modifiers":modifiers,"buttons":buttons}));
            }
            BrowserInput::Navigate { url } => {
                let parsed = reqwest::Url::parse(&url).map_err(|e| BrowserError::Protocol(e.to_string()))?;
                if !matches!(parsed.scheme(), "http"|"https") { return Err(protocol("Use an HTTP or HTTPS URL")); }
                self.ctrl.navigate_unlocked(&url).await?;
                let _ = self.client.send(ack(id, json!({})));
            }
            BrowserInput::History { direction } => {
                self.close_select();
                self.ctrl.navigate_history(direction).await?;
                let _ = self.client.send(ack(id, json!({})));
            }
            BrowserInput::PointerDown { x, y } => {
                let (x, y) = self.shared.point(x, y)?;
                // A second press without a release (lost pointerup) lets go of the first one
                self.release_held().await;
                // a phone tap-and-drag on a select shows kwant's list; nothing is held, so the
                // pointer_up that follows does nothing
                if self.offer_select(x, y).await {
                    let _ = self.client.send(ack(id, json!({})));
                    return Ok(());
                }
                let reply = self.mouse(json!({"type":"mousePressed","x":x,"y":y,"button":"left","buttons":1,"clickCount":1}));
                self.set_held(Some(HeldPointer { x, y, buttons: 1 }));
                self.finish(id.clone(), vec![reply], false);
            }
            BrowserInput::PointerMove { x, y } => {
                let (x, y) = self.shared.point(x, y)?;
                let held = self.held();
                let buttons = held.map(|h| h.buttons).unwrap_or(0);
                let reply = self.mouse(json!({"type":"mouseMoved","x":x,"y":y,"button":bi::moving_button(buttons),"buttons":buttons}));
                if held.is_some() { self.set_held(Some(HeldPointer { x, y, buttons })); }
                self.finish(id.clone(), vec![reply], false);
            }
            BrowserInput::PointerUp { x, y } => {
                let (x, y) = self.shared.point(x, y)?;
                let mut replies = vec![];
                if let Some(held) = self.held().filter(|h| h.buttons & 1 != 0) {
                    let buttons = held.buttons & !1;
                    self.set_held(Some(HeldPointer { x, y, buttons }));
                    replies.push(self.mouse(json!({"type":"mouseReleased","x":x,"y":y,"button":"left","buttons":buttons,"clickCount":1})));
                }
                self.finish(id.clone(), replies, true);
            }
            BrowserInput::Mouse { kind, x, y, button, buttons: _, click_count, modifiers } => {
                let (x, y) = self.shared.point(x, y)?;
                let held = self.held().map(|h| h.buttons).unwrap_or(0);
                let clicks = click_count.unwrap_or(1).clamp(1, 3);
                match kind {
                    MouseKind::Move => {
                        self.cdp.fire("Input.dispatchMouseEvent", json!({"type":"mouseMoved","x":x,"y":y,"button":bi::moving_button(held),"buttons":held,"modifiers":modifiers}));
                        if held != 0 { self.set_held(Some(HeldPointer { x, y, buttons: held })); }
                        self.check_cursor(x, y);
                    }
                    MouseKind::Down => {
                        let bit = button.bit();
                        if bit == 0 { return Err(protocol("Invalid mouse button")); }
                        if button == MouseButton::Left && clicks == 1 && modifiers == 0 && self.offer_select(x, y).await {
                            self.swallowed |= bit;
                            let _ = self.client.send(ack(id, json!({})));
                            return Ok(());
                        }
                        let mut replies = vec![];
                        if held & bit != 0 {
                            // a lost up: let go first so the page sees a clean press
                            replies.push(self.mouse(json!({"type":"mouseReleased","x":x,"y":y,"button":button.name(),"buttons":held & !bit,"clickCount":1,"modifiers":modifiers})));
                        }
                        let buttons = held | bit;
                        replies.push(self.mouse(json!({"type":"mousePressed","x":x,"y":y,"button":button.name(),"buttons":buttons,"clickCount":clicks,"modifiers":modifiers})));
                        self.set_held(Some(HeldPointer { x, y, buttons }));
                        self.finish(id.clone(), replies, false);
                    }
                    MouseKind::Up => {
                        let bit = button.bit();
                        if bit == 0 { return Err(protocol("Invalid mouse button")); }
                        if self.swallowed & bit != 0 {
                            self.swallowed &= !bit;
                            let _ = self.client.send(ack(id, json!({})));
                            return Ok(());
                        }
                        let mut replies = vec![];
                        if held & bit != 0 {
                            let buttons = held & !bit;
                            self.set_held(Some(HeldPointer { x, y, buttons }));
                            replies.push(self.mouse(json!({"type":"mouseReleased","x":x,"y":y,"button":button.name(),"buttons":buttons,"clickCount":clicks,"modifiers":modifiers})));
                        }
                        self.finish(id.clone(), replies, button == MouseButton::Left);
                    }
                }
            }
            BrowserInput::Resize { width, height, dpr } => {
                let Some(_) = self.ctrl.fixed_viewport() else {
                    // the page follows a real window: its size is the window's
                    let (w, h) = self.shared.view();
                    let _ = self.client.send(ack(id, json!({"ignored": true, "viewport": {"width": w, "height": h}})));
                    return Ok(());
                };
                let (w, h, dpr) = bi::clamp_resize(width, height, dpr).map_err(protocol)?;
                // on the controller's session, which owns the saved 1280x800 override
                self.ctrl.send_cdp_command("Emulation.setDeviceMetricsOverride",
                    json!({"width": w, "height": h, "deviceScaleFactor": dpr, "mobile": false})).await?;
                self.shared.resized.store(true, Ordering::SeqCst);
                self.shared.set_view(w as f64, h as f64);
                restart_screencast(&self.cdp, (w as f64 * dpr).round() as u32, (h as f64 * dpr).round() as u32).await?;
                let _ = self.client.send(ack(id, json!({"viewport": {"width": w, "height": h, "dpr": dpr}})));
            }
            BrowserInput::SelectOption { index, indexes } => {
                let Some(object) = self.open_select.clone() else { return Err(protocol("No select is open")) };
                let indexes = indexes.or(index.map(|i| vec![i])).unwrap_or_default();
                if indexes.is_empty() || indexes.len() > 500 { return Err(protocol("Choose an option")); }
                let result = self.cdp.call("Runtime.callFunctionOn", json!({
                    "objectId": object, "functionDeclaration": bi::SELECT_SET_FN,
                    "arguments": [{"value": indexes}], "returnByValue": true,
                }), PROBE_WAIT).await?;
                if result["result"]["value"] != true { return Err(protocol("That option can't be picked")); }
                self.close_select();
                let _ = self.client.send(ack(id, json!({})));
            }
            BrowserInput::Copy => {
                let result = self.cdp.call("Runtime.evaluate", json!({"expression": bi::COPY_JS, "returnByValue": true}), PROBE_WAIT).await?;
                let text: String = result["result"]["value"].as_str().unwrap_or_default().chars().take(bi::COPY_MAX).collect();
                let _ = self.client.send(json!({"type":"clipboard","id":id,"text":text}));
                let _ = self.client.send(ack(id, json!({})));
            }
            // answered by the stream itself, never queued behind the input a dialog blocks
            BrowserInput::Dialog { .. } => return Err(protocol("No dialog is open")),
        }
        Ok(())
    }

    /// If a select is under the point, describes it to the person instead of pressing on it:
    /// a headless browser draws its option list nowhere the screencast can see.
    async fn offer_select(&mut self, x: f64, y: f64) -> bool {
        // a press anywhere closes the list we described; free its object before probing again
        self.close_select();
        let Ok(found) = self.cdp.call("Runtime.evaluate", json!({
            "expression": bi::select_probe_js(x, y), "returnByValue": false, "objectGroup": SELECT_GROUP,
        }), PROBE_WAIT).await else { return false };
        let Some(object) = found["result"]["objectId"].as_str().map(str::to_string) else { return false };
        let Ok(info) = self.cdp.call("Runtime.callFunctionOn", json!({
            "objectId": object, "functionDeclaration": bi::SELECT_INFO_FN, "returnByValue": true,
        }), PROBE_WAIT).await else { return false };
        let info = &info["result"]["value"];
        if !info["options"].is_array() { return false; }
        self.open_select = Some(object);
        let _ = self.client.send(json!({"type":"select","options":info["options"],"multiple":info["multiple"],"rect":info["rect"]}));
        true
    }

    fn close_select(&mut self) {
        if self.open_select.take().is_some() {
            self.cdp.fire("Runtime.releaseObjectGroup", json!({"objectGroup": SELECT_GROUP}));
        }
    }

    /// After a move, looks up the cursor at most every 125 ms and tells the person when it
    /// changes.
    fn check_cursor(&self, x: f64, y: f64) {
        {
            let mut at = self.shared.cursor_at.lock().unwrap_or_else(|e| e.into_inner());
            if at.is_some_and(|t| t.elapsed() < bi::CURSOR_EVERY) { return; }
            if self.shared.cursor_busy.swap(true, Ordering::SeqCst) { return; }
            *at = Some(Instant::now());
        }
        let (cdp, shared, client) = (self.cdp.clone(), self.shared.clone(), self.client.clone());
        tokio::spawn(async move {
            if let Ok(result) = cdp.call("Runtime.evaluate", json!({"expression": bi::cursor_js(x, y), "returnByValue": true}), PROBE_WAIT).await {
                let cursor = bi::sanitize_cursor(result["result"]["value"].as_str().unwrap_or("auto"));
                let changed = {
                    let mut last = shared.cursor.lock().unwrap_or_else(|e| e.into_inner());
                    let changed = *last != cursor;
                    *last = cursor.clone();
                    changed
                };
                if changed { let _ = client.send(json!({"type":"cursor","cursor":cursor})); }
            }
            shared.cursor_busy.store(false, Ordering::SeqCst);
        });
    }

    async fn release_held(&self) {
        let held = self.ctrl.pointer_held_slot().lock().unwrap_or_else(|e| e.into_inner()).take();
        release_on(&self.cdp, held).await;
    }

    /// The person handed back: nothing stays pressed and the bot gets its own viewport back.
    async fn hand_back(&mut self) {
        self.release_held().await;
        self.swallowed = 0;
        self.close_select();
        restore_viewport(&self.ctrl, &self.cdp, &self.shared).await;
    }
}

async fn release_on(cdp: &Cdp, held: Option<HeldPointer>) {
    for params in held.map(release_events).unwrap_or_default() {
        let _ = cdp.call("Input.dispatchMouseEvent", params, PROBE_WAIT).await;
    }
}

async fn restore_viewport(ctrl: &BrowserController, cdp: &Cdp, shared: &Shared) {
    if !shared.resized.swap(false, Ordering::SeqCst) { return; }
    let (w, h) = ctrl.fixed_viewport().unwrap_or(DEFAULT_VIEW);
    let _ = ctrl.send_cdp_command("Emulation.setDeviceMetricsOverride", json!({"width": w, "height": h, "deviceScaleFactor": 1, "mobile": false})).await;
    shared.set_view(w as f64, h as f64);
    let _ = restart_screencast(cdp, w, h).await;
}

async fn restart_screencast(cdp: &Cdp, max_width: u32, max_height: u32) -> Result<(), BrowserError> {
    cdp.call("Page.stopScreencast", json!({}), PROBE_WAIT).await?;
    cdp.call("Page.startScreencast", screencast_params(max_width, max_height), PROBE_WAIT).await?;
    Ok(())
}

fn screencast_params(max_width: u32, max_height: u32) -> Value {
    json!({"format":"jpeg","quality":65,"maxWidth":max_width,"maxHeight":max_height,"everyNthFrame":1})
}

async fn focused_editable(cdp: &Cdp) -> bool {
    cdp.call("Runtime.evaluate", json!({"expression": FOCUSED_EDITABLE_JS, "returnByValue": true}), PROBE_WAIT).await
        .ok().and_then(|r| r["result"]["value"].as_bool()).unwrap_or(false)
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
    let ws = crate::sandbox::browser::connect_cdp(&ctrl.get_ws_url().await?).await?;
    let (cdp, mut events, tasks) = Cdp::open(ws);
    let (w, h) = ctrl.fixed_viewport().unwrap_or(DEFAULT_VIEW);
    let shared = Arc::new(Shared {
        view: std::sync::Mutex::new((w as f64, h as f64)),
        resized: AtomicBool::new(false),
        cursor: std::sync::Mutex::new("auto".into()),
        cursor_busy: AtomicBool::new(false),
        cursor_at: std::sync::Mutex::new(None),
    });
    let (client_tx, mut client_rx) = mpsc::unbounded_channel::<Value>();
    let (jobs, jobs_rx) = mpsc::unbounded_channel::<Job>();
    let worker = Worker { ctrl: ctrl.clone(), cdp: cdp.clone(), shared: shared.clone(), client: client_tx.clone(), swallowed: 0, open_select: None };
    let worker = tokio::spawn(worker.run(jobs_rx));
    let result = stream(&mut socket, &ctrl, &cdp, &mut events, &shared, &client_tx, &mut client_rx, &jobs).await;
    // Let go of anything held and put the viewport back, then drop the lease so bot actions
    // resume even on disconnect or timeout.
    worker.abort();
    let held = ctrl.pointer_held_slot().lock().unwrap_or_else(|e| e.into_inner()).take();
    release_on(&cdp, held).await;
    restore_viewport(&ctrl, &cdp, &shared).await;
    drop(result.1);
    for task in tasks { task.abort(); }
    result.0
}

#[allow(clippy::too_many_arguments)]
async fn stream(
    socket: &mut WebSocket,
    ctrl: &BrowserController,
    cdp: &Cdp,
    events: &mut mpsc::Receiver<Value>,
    shared: &Arc<Shared>,
    client_tx: &mpsc::UnboundedSender<Value>,
    client_rx: &mut mpsc::UnboundedReceiver<Value>,
    jobs: &mpsc::UnboundedSender<Job>,
) -> (Result<(), BrowserError>, Option<OwnedRwLockWriteGuard<()>>) {
    let mut lease: Option<OwnedRwLockWriteGuard<()>> = None;
    let result = async {
        cdp.call("Page.enable", json!({}), Duration::from_secs(10)).await?;
        let (w, h) = shared.view();
        cdp.call("Page.startScreencast", screencast_params(w as u32, h as u32), Duration::from_secs(10)).await?;
        let mut last_client = Instant::now();
        let mut heartbeat = tokio::time::interval(Duration::from_secs(10));
        let mut pending_frame: Option<i64> = None;
        let mut frame_sent = Instant::now();
        let mut next_frame = 0u64;
        let mut pending_token = 0u64;
        let mut location_tick = tokio::time::interval(Duration::from_secs(2));
        let location_busy = Arc::new(AtomicBool::new(false));
        let mut moves = RateLimit::new(bi::MOVES_PER_SECOND, 10.0, std::time::Instant::now());
        let mut dialog_open = false;
        loop {
            tokio::select! {
                _ = heartbeat.tick() => {
                    if last_client.elapsed() > Duration::from_secs(35) { break; }
                    if pending_frame.is_some() && frame_sent.elapsed() > Duration::from_secs(15) { break; }
                    send(socket, json!({"type":"ping"})).await?;
                }
                _ = location_tick.tick() => {
                    // the person handed back a resized page without releasing (a human step ended)
                    if lease.is_none() && !ctrl.is_held_for_person() && shared.resized.load(Ordering::SeqCst) {
                        let _ = jobs.send(Job::HandBack(None));
                    }
                    // read on the stream's socket, off the loop: it blocks while a dialog is open
                    if !location_busy.swap(true, Ordering::SeqCst) {
                        let (cdp, ctrl, client, busy) = (cdp.clone(), ctrl.clone(), client_tx.clone(), location_busy.clone());
                        tokio::spawn(async move {
                            if let Ok(result) = cdp.call("Runtime.evaluate", json!({"expression": "({url: location.href, title: document.title})", "returnByValue": true}), PROBE_WAIT).await {
                                if let Some(url) = result["result"]["value"]["url"].as_str() { *ctrl.current_url.lock().await = Some(url.to_string()); }
                                if let Some(title) = result["result"]["value"]["title"].as_str() { *ctrl.current_title.lock().await = Some(title.to_string()); }
                            }
                            let _ = client.send(json!({"type":"location","url":ctrl.current_url.lock().await.clone(),"title":ctrl.current_title.lock().await.clone()}));
                            busy.store(false, Ordering::SeqCst);
                        });
                    }
                }
                Some(value) = client_rx.recv() => send(socket, value).await?,
                event = events.recv() => {
                    let Some(event) = event else { break };
                    let params = &event["params"];
                    match event["method"].as_str().unwrap_or_default() {
                        "Page.screencastFrame" => {
                            let session = params["sessionId"].as_i64().unwrap_or_default();
                            let (dw, dh) = (params["metadata"]["deviceWidth"].as_f64().unwrap_or(0.0), params["metadata"]["deviceHeight"].as_f64().unwrap_or(0.0));
                            if dw >= 1.0 && dh >= 1.0 { shared.set_view(dw, dh); }
                            if pending_frame.is_some() {
                                cdp.fire("Page.screencastFrameAck", json!({"sessionId":session}));
                                continue;
                            }
                            next_frame += 1;
                            pending_token = next_frame;
                            pending_frame = Some(session);
                            frame_sent = Instant::now();
                            let (fw, fh) = shared.view();
                            send(socket, json!({"type":"frame","id":next_frame,"image":params["data"],"width":fw,"height":fh})).await?;
                        }
                        "Page.javascriptDialogOpening" => {
                            dialog_open = true;
                            send(socket, json!({"type":"dialog","kind":params["type"],"message":params["message"].as_str().unwrap_or_default().chars().take(4000).collect::<String>(),
                                "default_prompt":params["defaultPrompt"].as_str().unwrap_or_default().chars().take(4000).collect::<String>(),
                                "url":params["url"].as_str().unwrap_or_default().chars().take(300).collect::<String>()})).await?;
                        }
                        "Page.javascriptDialogClosed" => {
                            dialog_open = false;
                            send(socket, json!({"type":"dialog_closed","accepted":params["result"]})).await?;
                        }
                        _ => {}
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
                    let id = value.get("id").cloned().unwrap_or(Value::Null);
                    match value["action"].as_str().unwrap_or("") {
                        "pong" => {},
                        "frame_ack" => {
                            if value["id"].as_u64() == Some(pending_token) {
                                if let Some(session) = pending_frame.take() {
                                    cdp.fire("Page.screencastFrameAck", json!({"sessionId":session}));
                                }
                            }
                        }
                        "take_control" => {
                            // A pending human step already holds the browser for the person
                            if lease.is_none() && !ctrl.is_held_for_person() {
                                match ctrl.take_control().await {
                                    Ok(guard) => lease = Some(guard),
                                    Err(error) => { send(socket, json!({"type":"error","error":error.to_string()})).await?; continue; }
                                }
                            }
                            let (vw, vh) = shared.view();
                            send(socket, json!({"type":"control","active":true,"viewport":{"width":vw,"height":vh}})).await?;
                        }
                        "release_control" => {
                            let (done, wait) = oneshot::channel();
                            if jobs.send(Job::HandBack(Some(done))).is_ok() {
                                let _ = timeout(Duration::from_secs(8), wait).await;
                            }
                            lease = None;
                            let (vw, vh) = shared.view();
                            send(socket, json!({"type":"control","active":false,"viewport":{"width":vw,"height":vh}})).await?;
                        }
                        _ => {
                            if lease.is_none() && !ctrl.is_held_for_person() {
                                send(socket, error(&id, BrowserError::HumanControl)).await?;
                                continue;
                            }
                            let input = match BrowserInput::parse(&value) {
                                Ok(input) => input,
                                Err(e) => { send(socket, error(&id, e)).await?; continue; }
                            };
                            // The person's own input: never held to a bot's site rules
                            ctrl.site_guard.set_actor(crate::sandbox::site_guard::Actor::Person);
                            if let BrowserInput::Dialog { accept, prompt_text } = input {
                                if !dialog_open { send(socket, error(&id, protocol("No dialog is open"))).await?; continue; }
                                let mut params = json!({"accept": accept});
                                if let Some(text) = prompt_text { params["promptText"] = json!(text.chars().take(16384).collect::<String>()); }
                                let (reply, client) = (cdp.request("Page.handleJavaScriptDialog", params), client_tx.clone());
                                tokio::spawn(async move {
                                    let _ = client.send(match settle(reply, PROBE_WAIT).await { Ok(_) => ack(&id, json!({})), Err(e) => error(&id, e) });
                                });
                                continue;
                            }
                            if input.is_stream() {
                                // moves and wheel never queue: extra moves and anything under a dialog are dropped
                                if dialog_open { continue; }
                                if matches!(input, BrowserInput::Mouse { .. }) && !moves.allow(std::time::Instant::now()) { continue; }
                            } else if dialog_open && !input.is_release() {
                                send(socket, error(&id, protocol("The page is showing a dialog. Answer it first."))).await?;
                                continue;
                            }
                            let _ = jobs.send(Job::Input { input, id });
                        }
                    }
                }
            }
        }
        Ok(())
    }.await;
    (result, lease)
}

async fn send(socket: &mut WebSocket, value: Value) -> Result<(), BrowserError> {
    timeout(Duration::from_secs(5), socket.send(Message::Text(value.to_string()))).await
        .map_err(|_| BrowserError::Timeout)?.map_err(|e| BrowserError::Ws(e.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn v1_actions_still_parse() {
        for value in [
            json!({"action":"click","x":1,"y":2}),
            json!({"action":"type","text":"hi"}),
            json!({"action":"key","key":"Enter","code":"Enter"}),
            json!({"action":"scroll","x":1,"y":2,"delta_x":0,"delta_y":100}),
            json!({"action":"navigate","url":"https://example.com"}),
            json!({"action":"pointer_down","x":1,"y":2}),
            json!({"action":"pointer_move","x":1,"y":2}),
            json!({"action":"pointer_up","x":1,"y":2}),
        ] {
            let input = BrowserInput::parse(&value).unwrap_or_else(|_| panic!("{value}"));
            assert!(!input.is_stream(), "v1 input is always acked: {value}");
        }
        assert!(matches!(BrowserInput::parse(&json!({"action":"key","key":"Tab"})).unwrap(), BrowserInput::Key { kind: None, .. }));
    }

    #[test]
    fn v2_actions_parse_with_defaults() {
        match BrowserInput::parse(&json!({"action":"mouse","type":"down","x":5,"y":6,"button":"right","click_count":2,"modifiers":8})).unwrap() {
            BrowserInput::Mouse { kind, button, click_count, modifiers, .. } => {
                assert_eq!((kind, button, click_count, modifiers), (MouseKind::Down, MouseButton::Right, Some(2), 8));
            }
            other => panic!("{other:?}"),
        }
        let moved = BrowserInput::parse(&json!({"action":"mouse","type":"move","x":5,"y":6})).unwrap();
        assert!(moved.is_stream());
        assert!(matches!(moved, BrowserInput::Mouse { button: MouseButton::None, modifiers: 0, click_count: None, .. }));
        let wheel = BrowserInput::parse(&json!({"action":"wheel","x":1,"y":1,"delta_y":120,"modifiers":2})).unwrap();
        assert!(wheel.is_stream());
        assert!(matches!(wheel, BrowserInput::Wheel { delta_x, delta_y, modifiers: 2, .. } if delta_x == 0.0 && delta_y == 120.0));
        match BrowserInput::parse(&json!({"action":"key","type":"down","key":"a","code":"KeyA","modifiers":2})).unwrap() {
            BrowserInput::Key { kind, modifiers, auto_repeat, .. } => assert_eq!((kind, modifiers, auto_repeat), (Some(KeyKind::Down), 2, false)),
            other => panic!("{other:?}"),
        }
        assert!(BrowserInput::parse(&json!({"action":"key","type":"up","key":"a","code":"KeyA"})).unwrap().is_release());
        assert!(matches!(BrowserInput::parse(&json!({"action":"resize","width":900,"height":700})).unwrap(), BrowserInput::Resize { dpr: None, .. }));
        assert!(matches!(BrowserInput::parse(&json!({"action":"select_option","index":2})).unwrap(), BrowserInput::SelectOption { index: Some(2), indexes: None }));
        assert!(matches!(BrowserInput::parse(&json!({"action":"dialog","accept":true,"prompt_text":"x"})).unwrap(), BrowserInput::Dialog { accept: true, .. }));
        assert!(matches!(BrowserInput::parse(&json!({"action":"copy","id":3})).unwrap(), BrowserInput::Copy));
    }

    #[test]
    fn history_parses_each_direction_and_refuses_others() {
        for (wire, direction) in [("back", HistoryDirection::Back), ("forward", HistoryDirection::Forward), ("reload", HistoryDirection::Reload)] {
            match BrowserInput::parse(&json!({"action":"history","direction":wire,"id":3})).unwrap() {
                BrowserInput::History { direction: got } => assert_eq!(got, direction, "{wire}"),
                other => panic!("{other:?}"),
            }
        }
        for bad in [json!({"action":"history","direction":"sideways"}), json!({"action":"history"})] {
            assert_eq!(BrowserInput::parse(&bad).unwrap_err().to_string(), BrowserError::Protocol("Invalid browser input".into()).to_string(), "{bad}");
        }
    }

    #[test]
    fn bad_messages_are_refused() {
        for value in [
            json!({"action":"mouse","type":"hover","x":1,"y":1}),
            json!({"action":"mouse","type":"down","x":1,"y":1,"button":"thumb"}),
            json!({"action":"mouse","type":"down","x":1,"y":1,"modifiers":16}),
            json!({"action":"wheel","x":1,"y":1,"modifiers":300}),
            json!({"action":"key","type":"sideways","key":"a","code":"KeyA"}),
            json!({"action":"dialog"}),
            json!({"action":"resize","width":"wide","height":700}),
            json!({"action":"teleport"}),
        ] {
            assert!(BrowserInput::parse(&value).is_err(), "{value}");
        }
    }

    #[test]
    fn letting_go_releases_every_held_button_in_order() {
        let events = release_events(HeldPointer { x: 3.0, y: 4.0, buttons: 1 | 2 | 4 });
        let shape: Vec<_> = events.iter().map(|e| (e["button"].as_str().unwrap().to_string(), e["buttons"].as_u64().unwrap())).collect();
        assert_eq!(shape, vec![("left".into(), 6), ("right".into(), 4), ("middle".into(), 0)]);
        assert!(events.iter().all(|e| e["type"] == "mouseReleased" && e["x"] == 3.0 && e["y"] == 4.0));
        assert!(release_events(HeldPointer { x: 0.0, y: 0.0, buttons: 0 }).is_empty());
    }
}
