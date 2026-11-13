//! The live view of a bot's browser: a CDP screencast out, the person's input in.
//!
//! Protocol v1 (`click`, `type`, `key` without `type`, `scroll`, `navigate`, `pointer_*`) keeps
//! working for the phone overlay. Protocol v2 adds full mouse events, wheel and key events with
//! modifiers, `resize`, select interception, JS dialogs, `copy` and cursor updates. Input goes
//! out on the stream's own DevTools socket, in order, so moves are fire-and-forget and never
//! wait behind a bot's command.
use super::browser_input::{self as bi, KeyEvent, KeyKind, MouseButton, RateLimit};
use super::point;
use super::{routes::PageRefusal, AppState};
use super::show_once::{capture as recorder, stage::{Act, StageRecorder}};
use crate::db::attachments::Attachment;
use crate::sandbox::app_links::{self, AppLinkWatch, Verdict};
use crate::sandbox::browser::{web_address, BrowserController, BrowserError, HeldPointer, PersonLease, ViewClaim, ViewRelease, FOCUSED_EDITABLE_JS, MOUSE_BUTTON_BITS};
use crate::sandbox::browser_history::HistoryDirection;
use axum::{extract::{Path, State, WebSocketUpgrade, ws::{Message, WebSocket}}, http::{HeaderMap, StatusCode}, response::Response};
use futures::{SinkExt, StreamExt};
use serde::Deserialize;
use serde_json::{json, Value};
use std::{collections::HashMap, sync::{Arc, atomic::{AtomicBool, AtomicU64, Ordering}}, time::Duration};
use tokio::{sync::{mpsc, oneshot}, task::JoinHandle, time::{timeout, Instant}};
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
/// A frame the client has not acked for this long is let go, so a lost ack costs one stale
/// picture instead of the stream (and the person's control).
const FRAME_STALL: Duration = Duration::from_secs(4);
/// Chrome somewhere else has no shared filesystem with kwnt, so a file chooser answer there goes
/// in as base64 chunks instead of a real path; the compose browser's own container caps at
/// mem_limit 1536m, so the files sent that way are capped well under it.
const REMOTE_UPLOAD_CAP_BYTES: u64 = 100 * 1024 * 1024;
/// Per `Runtime.callFunctionOn` call, before base64 encoding.
const UPLOAD_CHUNK_BYTES: usize = 1_000_000;
/// How long a local-delivery temp copy (named after the attachment, not its content hash) stays
/// after `DOM.setFileInputFiles`, since the page may only read it later (e.g. only once the person
/// clicks "Post"). An opportunistic sweep of older orphans runs on every new chooser as a backstop.
const CHOOSER_TMP_TTL: Duration = Duration::from_secs(600);
const CHOOSER_TMP_ORPHAN_AGE: Duration = Duration::from_secs(3600);

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
                // told apart as a bot's navigate tells them: a path, another app, then the scheme
                let address = web_address(&url)?;
                let parsed = reqwest::Url::parse(&address).map_err(|e| BrowserError::Protocol(e.to_string()))?;
                if !matches!(parsed.scheme(), "http"|"https") { return Err(protocol("Use an HTTP or HTTPS URL")); }
                self.navigate_unlocked(&address).await?;
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
/// routes replies to whoever waits and events to the stream. [`Cdp::attach`] moves every clone to
/// another page's socket (a popup the page opened, and back).
#[derive(Clone)]
struct Cdp {
    out: Arc<std::sync::Mutex<mpsc::UnboundedSender<String>>>,
    pending: Waiters,
    next: Arc<AtomicU64>,
    tasks: Arc<std::sync::Mutex<CdpTasks>>,
}

/// The reader of the socket in use, and the writers of every socket not yet closed.
#[derive(Default)]
struct CdpTasks {
    reader: Option<JoinHandle<()>>,
    writers: Vec<JoinHandle<()>>,
}

impl Cdp {
    fn open(ws: crate::sandbox::browser::WsStream) -> (Self, mpsc::Receiver<Value>) {
        let cdp = Cdp { out: Arc::new(std::sync::Mutex::new(mpsc::unbounded_channel().0)), pending: Arc::default(), next: Arc::new(AtomicU64::new(1)), tasks: Arc::default() };
        let events = cdp.attach(ws);
        (cdp, events)
    }

    /// Sends from now on to `ws`, and returns its events. The old socket gets what was already
    /// sent to it, then closes; replies still owed on it are dropped.
    fn attach(&self, ws: crate::sandbox::browser::WsStream) -> mpsc::Receiver<Value> {
        let (mut sink, mut stream) = ws.split();
        let (out, mut out_rx) = mpsc::unbounded_channel::<String>();
        let (event_tx, events) = mpsc::channel::<Value>(64);
        // what the old socket still owes is never read now: its waiters fail at once
        self.pending.lock().unwrap_or_else(|e| e.into_inner()).clear();
        *self.out.lock().unwrap_or_else(|e| e.into_inner()) = out;
        let writer = tokio::spawn(async move {
            while let Some(text) = out_rx.recv().await {
                if sink.send(CdpMessage::Text(text)).await.is_err() { break; }
            }
            let _ = sink.close().await;
        });
        let pending = self.pending.clone();
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
                } else if value.get("method").is_some() && wanted(&value) && event_tx.send(value).await.is_err() {
                    break;
                }
            }
            // waiters see a closed channel; the stream sees its events end
            pending.lock().unwrap_or_else(|e| e.into_inner()).clear();
        });
        let mut tasks = self.tasks.lock().unwrap_or_else(|e| e.into_inner());
        // the old writer ends by itself once what was sent to it is out; its reader goes now
        if let Some(old) = tasks.reader.replace(reader) { old.abort(); }
        tasks.writers.retain(|task| !task.is_finished());
        tasks.writers.push(writer);
        events
    }

    fn close(&self) {
        let mut tasks = self.tasks.lock().unwrap_or_else(|e| e.into_inner());
        for task in tasks.reader.take().into_iter().chain(tasks.writers.drain(..)) { task.abort(); }
    }

    fn send(&self, id: u64, method: &str, params: Value) {
        let _ = self.out.lock().unwrap_or_else(|e| e.into_inner()).send(json!({"id":id,"method":method,"params":params}).to_string());
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

/// Whether the stream acts on this DevTools event. Runtime and Log are on only for the app-link
/// guard, and turning them on makes Chrome replay everything the page has logged: a page that
/// logged a few hundred lines would fill the event queue before the stream drains it, and every
/// reply behind them (the screencast's start) would wait until it timed out. So the rest of
/// those two domains is dropped as it arrives, and a noisy page can't stall the reader.
fn wanted(event: &Value) -> bool {
    let method = event["method"].as_str().unwrap_or_default();
    match method {
        "Runtime.bindingCalled" => event["params"]["name"] == app_links::BINDING || event["params"]["name"] == recorder::BINDING,
        "Log.entryAdded" => event["params"]["entry"]["text"].as_str().is_some_and(|text| text.starts_with(app_links::REFUSED_LOG) || app_links::blocked_frame(text)),
        _ => !method.starts_with("Runtime.") && !method.starts_with("Log."),
    }
}

/// A reply that never came because the page's socket closed with the command out.
const PAGE_CLOSED: &str = "Browser connection closed";

fn page_closed(e: &BrowserError) -> bool {
    matches!(e, BrowserError::Ws(m) if m == PAGE_CLOSED)
}

async fn settle(reply: Reply, wait: Duration) -> Result<Value, BrowserError> {
    match timeout(wait, reply).await {
        Ok(Ok(result)) => result,
        Ok(Err(_)) => Err(BrowserError::Ws(PAGE_CLOSED.into())),
        Err(_) => Err(BrowserError::Timeout),
    }
}

/// The one frame out with the client, and the newest picture Chrome drew while it was out.
/// Chrome sends a frame only when the page changes, so a picture dropped while the client
/// was busy would never come again on a page that then stays still.
#[derive(Default)]
struct FrameGate {
    next_id: u64,
    /// The id the client must ack, when a frame is out.
    out: Option<u64>,
    /// Chrome's screencast session to ack with it; a resent picture was acked on arrival.
    session: Option<i64>,
    sent_at: Option<Instant>,
    /// The newest frame message that arrived while one was out (already acked to Chrome).
    missed: Option<Value>,
}

impl FrameGate {
    /// A frame from Chrome: the message to send the client now, if none is out, and the session
    /// to ack at once, if the picture waits (Chrome keeps drawing while the client catches up).
    fn arrive(&mut self, session: i64, image: &Value, width: f64, height: f64, now: Instant) -> (Option<Value>, Option<i64>) {
        if self.out.is_some() {
            self.missed = Some(json!({"type":"frame","image":image,"width":width,"height":height}));
            return (None, Some(session));
        }
        (Some(self.send(Some(session), json!({"type":"frame","image":image,"width":width,"height":height}), now)), None)
    }

    fn send(&mut self, session: Option<i64>, mut frame: Value, now: Instant) -> Value {
        self.next_id += 1;
        frame["id"] = json!(self.next_id);
        self.out = Some(self.next_id);
        self.session = session;
        self.sent_at = Some(now);
        frame
    }

    /// The client acked `id`: the session to ack to Chrome, and the missed picture to send next.
    fn ack(&mut self, id: Option<u64>, now: Instant) -> (Option<i64>, Option<Value>) {
        if id.is_none() || id != self.out { return (None, None); }
        self.release(now)
    }

    /// A frame out for longer than `FRAME_STALL` is let go the same way, so a lost ack costs
    /// one stale picture for a few seconds instead of the stream, and never the newest picture.
    fn stall(&mut self, now: Instant) -> (Option<i64>, Option<Value>) {
        let stalled = self.out.is_some() && self.sent_at.is_some_and(|t| now.saturating_duration_since(t) > FRAME_STALL);
        if !stalled { return (None, None); }
        self.release(now)
    }

    fn release(&mut self, now: Instant) -> (Option<i64>, Option<Value>) {
        let session = self.session.take();
        self.out = None;
        let next = self.missed.take().map(|frame| self.send(None, frame, now));
        (session, next)
    }
}

/// State the stream and its worker share.
struct Shared {
    /// This live view among the open ones, for its claim on the page size.
    socket: u64,
    /// The page viewport in CSS px, from frame metadata or the last resize.
    view: std::sync::Mutex<(f64, f64)>,
    /// A `resize` is applied and must be put back on hand back.
    resized: AtomicBool,
    cursor: std::sync::Mutex<String>,
    /// The newest point a move asked to look the cursor up at.
    cursor_moves: Latest<(f64, f64)>,
    cursor_at: std::sync::Mutex<Option<Instant>>,
    /// Asks to read the page's address and title again.
    location: Latest<()>,
    /// The Show it once recorder is installed on this connection (a recording is live): a pick in
    /// kwnt's drawn option list is handed to it ([`recorder::record_pick`]).
    recording: AtomicBool,
    /// The size of the newest Point picture and when it was taken: while Chrome takes a clipped
    /// screenshot it can send one live-view frame at the clip's size, which is not the page's.
    shot: std::sync::Mutex<Option<(f64, f64, Instant)>>,
}

/// How long after a Point picture a frame at exactly its size is taken for that picture's.
const SHOT_FRAME_WINDOW: Duration = Duration::from_secs(3);

/// A frame of `dw` x `dh` CSS px at `now` is the one Chrome drew for the picture `shot`
/// (within a pixel, as Chrome rounds the clip), not a new page size.
fn shot_frame(shot: Option<(f64, f64, Instant)>, dw: f64, dh: f64, now: Instant) -> bool {
    shot.is_some_and(|(w, h, at)| {
        now.saturating_duration_since(at) <= SHOT_FRAME_WINDOW && (dw - w).abs() < 1.0 && (dh - h).abs() < 1.0
    })
}

/// Keeps only the newest value offered and makes sure exactly one runner drains it, so a value
/// offered while the runner is busy is never lost (the trailing probe) and never doubled.
struct Latest<T> {
    slot: std::sync::Mutex<Option<T>>,
    busy: AtomicBool,
}

impl<T> Latest<T> {
    fn new() -> Self {
        Self { slot: std::sync::Mutex::new(None), busy: AtomicBool::new(false) }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Option<T>> {
        self.slot.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Stores `value`; true when the caller must start the runner.
    fn offer(&self, value: T) -> bool {
        *self.lock() = Some(value);
        !self.busy.swap(true, Ordering::SeqCst)
    }

    /// The runner's next value, or `None` when it should stop.
    fn next(&self) -> Option<T> {
        loop {
            if let Some(value) = self.lock().take() { return Some(value); }
            self.busy.store(false, Ordering::SeqCst);
            // an offer that landed after the take: keep going unless its own runner started
            if self.lock().is_none() || self.busy.swap(true, Ordering::SeqCst) { return None; }
        }
    }
}

/// How long to wait before the next cursor lookup, so lookups stay `CURSOR_EVERY` apart.
fn cursor_wait(last: Option<Instant>, now: Instant) -> Duration {
    last.map(|t| bi::CURSOR_EVERY.saturating_sub(now.saturating_duration_since(t))).unwrap_or_default()
}

/// Whether a page event can change the address or title the person sees.
fn moves_location(method: &str, params: &Value) -> bool {
    match method {
        // the top document only; a subframe navigating changes nothing in the address bar
        "Page.frameNavigated" => params["frame"]["parentId"].is_null(),
        // pushState, replaceState and #fragment changes
        "Page.navigatedWithinDocument" => true,
        // the new page's title is parsed by now
        "Page.domContentEventFired" => true,
        _ => false,
    }
}

/// The address a top-level page navigation lands on. A new document has no title yet, so the
/// person sees the new address with an empty title at once instead of the last page's title until
/// the new one is parsed.
fn navigated_to(method: &str, params: &Value) -> Option<String> {
    if method != "Page.frameNavigated" || !params["frame"]["parentId"].is_null() { return None; }
    let url = params["frame"]["url"].as_str()?;
    Some(format!("{url}{}", params["frame"]["urlFragment"].as_str().unwrap_or_default()))
}

/// What a live view hears when Take over is refused: `code` is `held_by_person` while another of
/// the person's views has control (retrying can't win until it hands back) and `bot_busy` while
/// a bot is mid-step (a retry soon can). `error` is the text, as before.
fn takeover_refused(error: &BrowserError) -> Value {
    let code = match error {
        BrowserError::HeldByPerson => "held_by_person",
        BrowserError::HumanControl => "bot_busy",
        _ => "failed",
    };
    json!({"type":"error","error":error.to_string(),"code":code})
}

/// The error for input from a live view that has not taken control.
fn not_in_control(someone_has_control: bool) -> BrowserError {
    if someone_has_control { BrowserError::HumanControl } else { BrowserError::NotInControl }
}

/// Reads the page's address and title on the stream's socket, off the caller, and sends them
/// to the person. Requests while a read is running are folded into one more read. A read that
/// fails (the page is between documents) sends nothing: the next page event or tick reads again,
/// and resending what kwant last knew would put the previous page's title back on screen.
fn refresh_location(cdp: &Cdp, ctrl: &BrowserController, shared: &Arc<Shared>, client: &mpsc::UnboundedSender<Value>) {
    if !shared.location.offer(()) { return; }
    let (cdp, ctrl, shared, client) = (cdp.clone(), ctrl.clone(), shared.clone(), client.clone());
    tokio::spawn(async move {
        while shared.location.next().is_some() {
            // blocks while a dialog is open, so never on the stream loop
            let Some(location) = read_location(&cdp, &ctrl).await else { continue };
            let _ = client.send(location);
        }
    });
}

/// The `location` message for the page `cdp` is on (its address and title, also kept on the
/// controller); `None` when the page doesn't answer within `PROBE_WAIT` (a dialog is open).
async fn read_location(cdp: &Cdp, ctrl: &BrowserController) -> Option<Value> {
    let result = cdp.call("Runtime.evaluate", json!({"expression": "({url: location.href, title: document.title})", "returnByValue": true}), PROBE_WAIT).await.ok()?;
    if let Some(url) = result["result"]["value"]["url"].as_str() { *ctrl.current_url.lock().await = Some(url.to_string()); }
    if let Some(title) = result["result"]["value"]["title"].as_str() { *ctrl.current_title.lock().await = Some(title.to_string()); }
    Some(json!({"type":"location","url":ctrl.current_url.lock().await.clone(),"title":ctrl.current_title.lock().await.clone()}))
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

    fn shooting(&self, width: f64, height: f64) {
        *self.shot.lock().unwrap_or_else(|e| e.into_inner()) = Some((width, height, Instant::now()));
    }

    fn is_shot_frame(&self, dw: f64, dh: f64) -> bool {
        shot_frame(*self.shot.lock().unwrap_or_else(|e| e.into_inner()), dw, dh, Instant::now())
    }
}

/// G3 Point at it on this live view (see [`super::point`]): read-only, so it needs no control.
/// It runs off the stream loop, on the stream's socket, in its own isolated world.
struct Pointer {
    state: AppState,
    cdp: Cdp,
    client: mpsc::UnboundedSender<Value>,
    /// The isolated world's context in the current document, once made.
    world: std::sync::Mutex<Option<i64>>,
    /// The newest hover: an older one still waiting is dropped unanswered.
    hover: Latest<(Value, f64, f64)>,
}

impl Pointer {
    fn new(state: AppState, cdp: Cdp, client: mpsc::UnboundedSender<Value>) -> Arc<Self> {
        Arc::new(Self {
            state,
            cdp,
            client,
            world: std::sync::Mutex::new(None),
            hover: Latest::new(),
        })
    }

    /// A new top document: its world is made again on the next point.
    fn forget_world(&self) {
        *self.world.lock().unwrap_or_else(|e| e.into_inner()) = None;
    }

    fn reply(&self, id: &Value, result: Result<Value, point::PointError>) {
        let _ = self.client.send(match result {
            Ok(extra) => ack(id, extra),
            Err(e) => json!({"type":"error","id":id,"error":e.message(),"code":e.code()}),
        });
    }

    /// Takes one point action from the person's view; the answer goes to the client later.
    fn handle(self: &Arc<Self>, value: &Value, id: Value, shared: &Arc<Shared>) {
        let bad = |this: &Arc<Self>| this.reply(&id, Err(point::PointError::BadRequest));
        match value["action"].as_str().unwrap_or_default() {
            "point_at" => {
                let (Some(x), Some(y)) = (value["x"].as_f64(), value["y"].as_f64()) else {
                    return bad(self);
                };
                if !x.is_finite() || !y.is_finite() {
                    return bad(self);
                }
                // outside the view: nothing there
                let Ok((x, y)) = shared.point(x, y) else {
                    return self.reply(&id, Err(point::PointError::Nothing));
                };
                if !self.hover.offer((id, x, y)) {
                    return;
                }
                let this = self.clone();
                tokio::spawn(async move {
                    while let Some((id, x, y)) = this.hover.next() {
                        let result = this
                            .eval(&point::at_call(x, y))
                            .await
                            .and_then(|v| point::El::from_page(&v));
                        this.reply(&id, result.map(|el| json!({"el": el})));
                    }
                });
            }
            "point_neighbour" => {
                let Some(dir) = value["dir"].as_str().and_then(point::Dir::parse) else {
                    return bad(self);
                };
                let handle = value["handle"].as_str();
                if dir != point::Dir::Centre && !handle.is_some_and(point::valid_handle) {
                    return bad(self);
                }
                let call = point::neighbour_call(handle, dir);
                let this = self.clone();
                tokio::spawn(async move {
                    let result = this
                        .eval(&call)
                        .await
                        .and_then(|v| point::El::from_page(&v));
                    this.reply(&id, result.map(|el| json!({"el": el})));
                });
            }
            "point_pick" => {
                let Some(handle) = value["handle"]
                    .as_str()
                    .filter(|h| point::valid_handle(h))
                    .map(str::to_string)
                else {
                    return bad(self);
                };
                let Ok(n) = point::pick_number(&value["n"]) else {
                    return bad(self);
                };
                let (this, shared) = (self.clone(), shared.clone());
                tokio::spawn(async move {
                    let result = this.pick(&handle, n, &shared).await;
                    this.reply(&id, result.map(|pick| json!({"pick": pick})));
                });
            }
            _ => bad(self),
        }
    }

    /// Runs `call` in the point world of the top document, installing [`point::POINT_JS`] first
    /// when the document is new. A world that has gone with its document is made once more.
    async fn eval(&self, call: &str) -> Result<Value, point::PointError> {
        use point::PointError::Failed;
        for _ in 0..2 {
            // copied out: the lock is never held across an await
            let known = *self.world.lock().unwrap_or_else(|e| e.into_inner());
            let context = match known {
                Some(context) => context,
                None => {
                    let tree = self
                        .cdp
                        .call("Page.getFrameTree", json!({}), PROBE_WAIT)
                        .await
                        .map_err(|_| Failed)?;
                    let frame = tree["frameTree"]["frame"]["id"].as_str().ok_or(Failed)?;
                    let world = json!({"frameId": frame, "worldName": point::WORLD});
                    let world = self
                        .cdp
                        .call("Page.createIsolatedWorld", world, PROBE_WAIT)
                        .await
                        .map_err(|_| Failed)?;
                    let context = world["executionContextId"].as_i64().ok_or(Failed)?;
                    *self.world.lock().unwrap_or_else(|e| e.into_inner()) = Some(context);
                    context
                }
            };
            let expression = format!("globalThis.__kp ? {call} : {{err: 'install'}}");
            let run = |expression: String| {
                self.cdp.call(
                    "Runtime.evaluate",
                    json!({"expression": expression, "contextId": context, "returnByValue": true}),
                    PROBE_WAIT,
                )
            };
            let value = match run(expression.clone()).await {
                Ok(r) if r["result"]["value"]["err"] == "install" => {
                    let installed = run(point::POINT_JS.to_string()).await.map_err(|_| Failed)?;
                    if installed.get("exceptionDetails").is_some() {
                        return Err(Failed);
                    }
                    run(expression).await
                }
                other => other,
            };
            match value {
                Ok(r) if r.get("exceptionDetails").is_some() => return Err(Failed),
                Ok(r) => return Ok(r["result"]["value"].clone()),
                // a dialog or a hung page: trying again can't help
                Err(BrowserError::Timeout) => return Err(Failed),
                // the context went with its document: make the world again
                Err(_) => self.forget_world(),
            }
        }
        Err(Failed)
    }

    /// Reads the element, takes the picture of what of it is in view, and stores both as an
    /// unsent upload. The ack is what the composer chip shows.
    async fn pick(&self, handle: &str, n: u8, shared: &Shared) -> Result<Value, point::PointError> {
        use base64::Engine;
        let capture = point::Capture::from_page(&self.eval(&point::capture_call(handle)).await?)?;
        let png = match capture.clip {
            Some(c) => {
                let clip =
                    json!({"x": c.x, "y": c.y, "width": c.width, "height": c.height, "scale": 1});
                // the frame Chrome draws at this size meanwhile must not become the view's size
                shared.shooting(c.width, c.height);
                let shot = self
                    .cdp
                    .call(
                        "Page.captureScreenshot",
                        json!({"format": "png", "clip": clip, "fromSurface": true}),
                        Duration::from_secs(5),
                    )
                    .await;
                shot.ok().and_then(|r| {
                    base64::engine::general_purpose::STANDARD
                        .decode(r["data"].as_str()?)
                        .ok()
                })
            }
            None => None,
        };
        let frame = capture.el.frame.clone();
        let mut pick = point::Pick::new(capture, png.is_some(), chrono::Utc::now());
        let stored =
            match super::attachments::store_point(&self.state, &pick, n, png.as_deref()).await {
                Some(stored) => stored,
                // a picture that can't be kept: the pick still counts, with its code
                None if pick.picture => {
                    pick.picture = false;
                    super::attachments::store_point(&self.state, &pick, n, None)
                        .await
                        .ok_or(point::PointError::Failed)?
                }
                None => return Err(point::PointError::Failed),
            };
        Ok(json!({
            "attachment_id": stored.id,
            "picture": pick.picture,
            "url": pick.url,
            "host": pick.host,
            "path": pick.path,
            "label": pick.label,
            "selector": pick.selector,
            "box": pick.rect,
            "viewport": pick.viewport,
            "frame": frame,
            "captured_at": pick.captured_at.to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
            "thumb_url": pick.picture.then(|| format!("/api/attachments/{}/content", stored.id)),
        }))
    }
}

enum Job {
    Input { input: BrowserInput, id: Value },
    HandBack(Option<oneshot::Sender<()>>),
    /// The view closes or goes to the background but keeps control: only its size goes.
    Restore(Value),
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

/// A page's file picker, intercepted instead of letting Chrome draw its own natively. Lives in
/// `stream()`'s locals next to `dialog_open` — per-connection, latest wins, cleared on navigation
/// and Hand back, never a bot's concern (nothing is sent to the client during a bot's turn).
#[derive(Clone)]
struct OpenChooser {
    /// This connection's own counter, not a CDP id: lets the client tell which sheet a
    /// `file_chooser_done`/error belongs to even if it raced a newer chooser.
    id: u64,
    backend_node_id: i64,
    frame_id: String,
    multiple: bool,
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

/// A file chooser failure, with a machine-readable `code` (`too_big`, `gone`, `failed`) so the
/// sheet can show the right inline message. Its own type, never `error`: the chooser's `id`
/// comes from the server's own per-connection counter, not the client's action-correlation
/// counter `error`/`ack` are matched against, so reusing `error`'s envelope could misroute this
/// into an unrelated pending action or G3 Point wait by sheer numeric coincidence.
fn chooser_error(id: &Value, code: &str, message: impl std::fmt::Display) -> Value {
    json!({"type":"file_chooser_error","id":id,"code":code,"error":message.to_string()})
}

/// The one thing people ever see for a server-side file-chooser failure (a copy/CDP/lookup error):
/// never the raw OS or CDP error text, which goes to the server's own log instead.
fn file_chooser_failed() -> (&'static str, String) {
    ("failed", "Couldn’t send that file to the page.".to_string())
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
            let mut closed = false;
            for reply in replies {
                match settle(reply, ACTION_WAIT).await {
                    Ok(_) => {}
                    // the page closed with the input out: a popup the click itself closed
                    // ("Continue" on a sign-in). The view goes back to its opener, and a browser
                    // that went away ends the stream, so the person sees no error for it.
                    Err(e) if page_closed(&e) => { closed = true; break; }
                    Err(e) => {
                        let _ = client.send(error(&id, e));
                        return;
                    }
                }
            }
            let extra = if editable { json!({"editable": !closed && focused_editable(&cdp).await}) } else { json!({}) };
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
                Job::Restore(id) => {
                    restore_viewport(&self.ctrl, &self.cdp, &self.shared).await;
                    let (w, h) = self.shared.view();
                    let _ = self.client.send(ack(&id, json!({"viewport": {"width": w, "height": h}})));
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
                // told apart as a bot's navigate tells them: a path, another app, then the scheme
                let address = web_address(&url)?;
                let parsed = reqwest::Url::parse(&address).map_err(|e| BrowserError::Protocol(e.to_string()))?;
                if !matches!(parsed.scheme(), "http"|"https") { return Err(protocol("Use an HTTP or HTTPS URL")); }
                self.commit_recorder().await;
                self.ctrl.navigate_unlocked(&address).await?;
                refresh_location(&self.cdp, &self.ctrl, &self.shared, &self.client);
                let _ = self.client.send(ack(id, json!({})));
            }
            BrowserInput::History { direction } => {
                self.close_select();
                self.commit_recorder().await;
                self.ctrl.navigate_history(direction).await?;
                refresh_location(&self.cdp, &self.ctrl, &self.shared, &self.client);
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
                // held while it applies: of two views resizing at once, the newest claim is on the page
                let mut claims = self.ctrl.view_claims().lock().await;
                let claim = ViewClaim { socket: self.shared.socket, width: w, height: h, dpr };
                if claims.repeats_older(&claim) {
                    drop(claims);
                    let (w, h) = self.shared.view();
                    let _ = self.client.send(ack(id, json!({"superseded": true, "viewport": {"width": w, "height": h}})));
                    return Ok(());
                }
                // on the controller's session, which owns the saved 1280x800 override
                self.ctrl.send_cdp_command("Emulation.setDeviceMetricsOverride",
                    json!({"width": w, "height": h, "deviceScaleFactor": dpr, "mobile": false})).await?;
                claims.claim(claim);
                drop(claims);
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
                // Show it once: the page saw an untrusted change, which the recorder ignores; the
                // person's pick goes to the recorder in its own world as a `select` step
                if self.shared.recording.load(Ordering::SeqCst) {
                    let cdp = &self.cdp;
                    let call = |method: &'static str, params: Value| async move { cdp.call(method, params, PROBE_WAIT).await.ok() };
                    if !recorder::record_pick(call, &object, &indexes).await {
                        tracing::debug!("show it once: a pick in the drawn list wasn't recorded");
                    }
                }
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

    /// Show it once (S-F5): the person is leaving through kwnt's address bar or back/forward,
    /// which moves no focus in the page, so the recorder posts its pending field edits as steps
    /// first (masked as always). Nothing happens while no recording is on.
    async fn commit_recorder(&self) {
        if !self.shared.recording.load(Ordering::SeqCst) { return; }
        let cdp = &self.cdp;
        let call = |method: &'static str, params: Value| async move { cdp.call(method, params, PROBE_WAIT).await.ok() };
        if !recorder::commit_pending(call).await {
            tracing::debug!("show it once: pending edits weren't committed before the person's navigation");
        }
    }

    fn close_select(&mut self) {
        if self.open_select.take().is_some() {
            self.cdp.fire("Runtime.releaseObjectGroup", json!({"objectGroup": SELECT_GROUP}));
        }
    }

    /// After a move, looks up the cursor at most every 125 ms and tells the person when it
    /// changes. The first move is looked up at once; moves inside the window end in one
    /// trailing lookup at the newest point, so where the pointer comes to rest always counts.
    fn check_cursor(&self, x: f64, y: f64) {
        if !self.shared.cursor_moves.offer((x, y)) { return; }
        let (cdp, shared, client) = (self.cdp.clone(), self.shared.clone(), self.client.clone());
        tokio::spawn(async move {
            loop {
                // wait out the window first, so the lookup takes the newest point in it
                let wait = cursor_wait(*shared.cursor_at.lock().unwrap_or_else(|e| e.into_inner()), Instant::now());
                if !wait.is_zero() { tokio::time::sleep(wait).await; }
                let Some((x, y)) = shared.cursor_moves.next() else { break };
                *shared.cursor_at.lock().unwrap_or_else(|e| e.into_inner()) = Some(Instant::now());
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
            }
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

/// This view lets go of its page size. The page keeps a newer view's size, takes the newest
/// remaining one, or gets the bot's own viewport back when no other view has a size.
async fn restore_viewport(ctrl: &BrowserController, cdp: &Cdp, shared: &Shared) {
    if !shared.resized.swap(false, Ordering::SeqCst) { return; }
    let mut claims = ctrl.view_claims().lock().await;
    let (w, h, dpr) = match claims.release(shared.socket) {
        // a newer view's size is on the page; this view's frames follow it
        ViewRelease::Keep => return,
        ViewRelease::Reapply(next) => (next.width, next.height, next.dpr),
        ViewRelease::Restore => { let (w, h) = ctrl.fixed_viewport().unwrap_or(DEFAULT_VIEW); (w, h, 1.0) }
    };
    // the bot's own override is sent with a whole 1, as `set_viewport` sends it
    let scale = if dpr == 1.0 { json!(1) } else { json!(dpr) };
    let _ = ctrl.send_cdp_command("Emulation.setDeviceMetricsOverride", json!({"width": w, "height": h, "deviceScaleFactor": scale, "mobile": false})).await;
    drop(claims);
    shared.set_view(w as f64, h as f64);
    let _ = restart_screencast(cdp, (w as f64 * dpr).round() as u32, (h as f64 * dpr).round() as u32).await;
}

/// Numbers the live views, for their claims on the page size.
static NEXT_SOCKET: AtomicU64 = AtomicU64::new(1);

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

/// `?page=`: which of the team's pages to show (a key from `GET /api/sandbox/:id/pages`).
#[derive(Debug, Default, Deserialize)]
pub struct StreamQuery {
    #[serde(default)]
    pub page: Option<String>,
}

pub async fn upgrade(
    State(state): State<AppState>,
    Path(id): Path<String>,
    axum::extract::Query(query): axum::extract::Query<StreamQuery>,
    headers: HeaderMap,
    ws: WebSocketUpgrade,
) -> Result<Response, StatusCode> {
    // Prevent a third-party website from controlling an authenticated same-origin session.
    let origin = headers.get("origin").and_then(|v| v.to_str().ok()).and_then(|s| reqwest::Url::parse(s).ok()).ok_or(StatusCode::FORBIDDEN)?;
    let host = headers.get("host").and_then(|v| v.to_str().ok()).ok_or(StatusCode::FORBIDDEN)?;
    let expected = reqwest::Url::parse(&format!("{}://{}", origin.scheme(), host)).map_err(|_| StatusCode::FORBIDDEN)?;
    if !matches!(origin.scheme(), "http" | "https") || origin.origin() != expected.origin() { return Err(StatusCode::FORBIDDEN); }
    // The page shown: the one asked for (a page of this team only), else the page an open step
    // holds, else the page used last (0.9.3: each job has its own page)
    let id = match query.page.filter(|p| super::browser_pages::page_team(p) == id) {
        Some(page) => page,
        None => super::browser_pages::live_page(&state, &id).await,
    };
    Ok(ws.max_message_size(32 * 1024).on_upgrade(move |socket| async move {
        let result = serve(socket, state.clone(), id.clone()).await;
        if let Err(error) = result { tracing::debug!("Browser stream ended: {}", error); }
        // the page's idle time counts from when the person stopped watching it
        if state.browser_service.page_used_at(&id).is_some() { state.browser_service.touch_page(&id); }
    }))
}

/// The page the live view shows: its DevTools address, and the controller's news of the page in
/// front changing (a popup the page opened, or its close).
struct Page {
    ws_url: String,
    changes: tokio::sync::watch::Receiver<u64>,
}

/// Turns on what the live view needs on the page `cdp` is on: its events, focus, the app-link
/// guard and the screencast.
async fn watch_page(cdp: &Cdp, shared: &Shared) -> Result<(), BrowserError> {
    cdp.call("Page.enable", json!({}), Duration::from_secs(10)).await?;
    // Chrome would otherwise draw its own native file picker, which nobody watching the live
    // view can see (headless: closed at once; headful on a virtual display: off-screen).
    // Intercepting it turns it into a `Page.fileChooserOpened` event instead.
    cdp.call("Page.setInterceptFileChooserDialog", json!({"enabled": true}), Duration::from_secs(10)).await?;
    // A Chrome window behind other apps on a Mac doesn't have the OS's focus, so its page
    // can act unfocused and drop typed keys. While anyone watches or controls it, the page
    // behaves as focused (as Playwright does for every page). It ends with this connection.
    let _ = cdp.call("Emulation.setFocusEmulationEnabled", json!({"enabled": true}), PROBE_WAIT).await;
    guard_app_links(cdp).await;
    let (w, h) = shared.view();
    cdp.call("Page.startScreencast", screencast_params(w as u32, h as u32), Duration::from_secs(10)).await?;
    Ok(())
}

/// Moves the view to the page in front when that changed: a popup the page opened ("Continue
/// with Google"), or the opener once the popup closed. The new page's events, or `None` when
/// it is the page already shown or can't be reached.
async fn follow_page(ctrl: &BrowserController, cdp: &Cdp, page: &mut Page, shared: &Shared) -> Option<mpsc::Receiver<Value>> {
    let ws_url = ctrl.get_ws_url().await.ok()?;
    if ws_url == page.ws_url {
        return None;
    }
    let ws = crate::sandbox::browser::connect_cdp(&ws_url).await.ok()?;
    cdp.fire("Page.stopScreencast", json!({}));
    let events = cdp.attach(ws);
    tracing::info!("the live view follows the page in front");
    page.ws_url = ws_url;
    if let Err(e) = watch_page(cdp, shared).await {
        // a popup that closed at once: its socket ends and the view comes back from it
        tracing::debug!("the live view couldn't watch the page in front: {}", e);
    }
    Some(events)
}

/// The shown page's socket closed. When that page was a popup, its close reaches the controller
/// a moment later and the view goes back to the page that opened it; `None` when no other page
/// comes to the front within a few seconds (the browser itself went away).
async fn back_from_closed_page(ctrl: &BrowserController, cdp: &Cdp, page: &mut Page, shared: &Shared) -> Option<mpsc::Receiver<Value>> {
    let back = async {
        loop {
            if let Some(events) = follow_page(ctrl, cdp, page, shared).await {
                return events;
            }
            tokio::time::sleep(Duration::from_millis(200)).await;
        }
    };
    timeout(Duration::from_secs(4), back).await.ok()
}

async fn serve(mut socket: WebSocket, state: AppState, id: String) -> Result<(), BrowserError> {
    let ctrl = match super::routes::open_page(&state, &id, Duration::ZERO).await {
        Ok(ctrl) => ctrl,
        Err(refusal) => {
            // The view closes after this, and the app reconnects with backoff: a busy browser
            // says so and is tried again; the Docker advice is only for one that isn't there
            let error = match refusal {
                PageRefusal::Busy => crate::sandbox::service::BROWSER_RECONNECTING.to_string(),
                PageRefusal::PagesFull => super::browser_pages::pages_full_message(),
                PageRefusal::Unavailable => crate::sandbox::service::BROWSER_UNREACHABLE_VIEW.to_string(),
            };
            let _ = socket.send(Message::Text(json!({"type":"error","error":error}).to_string())).await;
            return Ok(());
        }
    };
    // a watched page is never put to rest; one put to rest before is woken for the view, which
    // talks to it on a connection of its own
    let _watching = ctrl.watch();
    let _woken = ctrl.wake().await;
    // subscribed first, so a popup opening while this connects is still followed
    let changes = ctrl.page_changes();
    let ws_url = ctrl.get_ws_url().await?;
    let ws = crate::sandbox::browser::connect_cdp(&ws_url).await?;
    let (cdp, mut events) = Cdp::open(ws);
    let mut page = Page { ws_url, changes };
    let (w, h) = ctrl.fixed_viewport().unwrap_or(DEFAULT_VIEW);
    let shared = Arc::new(Shared {
        socket: NEXT_SOCKET.fetch_add(1, Ordering::Relaxed),
        view: std::sync::Mutex::new((w as f64, h as f64)),
        resized: AtomicBool::new(false),
        cursor: std::sync::Mutex::new("auto".into()),
        cursor_moves: Latest::new(),
        cursor_at: std::sync::Mutex::new(None),
        location: Latest::new(),
        recording: AtomicBool::new(false),
        shot: std::sync::Mutex::new(None),
    });
    let (client_tx, mut client_rx) = mpsc::unbounded_channel::<Value>();
    let (jobs, jobs_rx) = mpsc::unbounded_channel::<Job>();
    let worker = Worker { ctrl: ctrl.clone(), cdp: cdp.clone(), shared: shared.clone(), client: client_tx.clone(), swallowed: 0, open_select: None };
    let worker = tokio::spawn(worker.run(jobs_rx));
    // Show it once records on the team's stage, whichever of its pages is shown
    let mut rec = StageRecorder::new(state.clone(), super::browser_pages::page_team(&id).to_string(), shared.socket);
    let pointer = Pointer::new(state.clone(), cdp.clone(), client_tx.clone());
    // a sign-in page that refuses a windowless browser: a real window on the desktop apps, the
    // ways that work on a server, nothing when Chrome already is a window
    let service = &state.browser_service;
    let local = service.mode() == crate::sandbox::computer::ComputerMode::Local;
    let window = if local && service.local_window() { None } else { Some(local && crate::sandbox::computer::window_reason().is_none()) };
    let mut signin = crate::sandbox::signin_gate::LiveNotice::new(window);
    let result = stream(&mut socket, &ctrl, &cdp, &mut events, &mut page, &shared, &client_tx, &mut client_rx, &jobs, &mut rec, &pointer, &mut signin, &state, local).await;
    // a recording live on this socket stops here (Show it once: `closed`), its steps kept, after
    // the edit still pending in the page, if the page can still be reached
    if rec.is_live() {
        let pending = flush_recorder(&cdp).await;
        let _ = rec.on_flushed(&pending).await;
    }
    rec.closed().await;
    // Let go of anything held and put the viewport back, then drop the lease so bot actions
    // resume even on disconnect or timeout.
    worker.abort();
    let held = ctrl.pointer_held_slot().lock().unwrap_or_else(|e| e.into_inner()).take();
    release_on(&cdp, held).await;
    restore_viewport(&ctrl, &cdp, &shared).await;
    drop(result.1);
    cdp.close();
    result.0
}

#[allow(clippy::too_many_arguments)]
async fn stream(
    socket: &mut WebSocket,
    ctrl: &BrowserController,
    cdp: &Cdp,
    events: &mut mpsc::Receiver<Value>,
    page: &mut Page,
    shared: &Arc<Shared>,
    client_tx: &mpsc::UnboundedSender<Value>,
    client_rx: &mut mpsc::UnboundedReceiver<Value>,
    jobs: &mpsc::UnboundedSender<Job>,
    rec: &mut StageRecorder,
    pointer: &Arc<Pointer>,
    signin: &mut crate::sandbox::signin_gate::LiveNotice,
    state: &AppState,
    local: bool,
) -> (Result<(), BrowserError>, Option<PersonLease>) {
    let mut lease: Option<PersonLease> = None;
    // the Show it once recorder's script on new documents, while a recording is live
    let mut recorder_script: Option<String> = None;
    let result = async {
        watch_page(cdp, shared).await?;
        // the view may open on a sign-in that was already refused
        let url = ctrl.current_url.lock().await.clone().unwrap_or_default();
        if let Some(notice) = signin.update(&url, ctrl.is_held_for_person()) { send(socket, notice).await?; }
        // a link to another app may have fired before the view opened, with nobody to close
        // Chrome's prompt: check the page still gets input, off the loop. Not while a button is
        // held, where a move would be part of someone's drag.
        let held = ctrl.pointer_held_slot().lock().unwrap_or_else(|e| e.into_inner()).is_some();
        let mut stuck_check = (!held).then(|| { let cdp = cdp.clone(); tokio::spawn(async move { input_is_stuck(&cdp).await }) });
        let mut last_client = Instant::now();
        let mut heartbeat = tokio::time::interval(Duration::from_secs(10));
        let mut frames = FrameGate::default();
        let mut location_tick = tokio::time::interval(Duration::from_secs(2));
        let mut moves = RateLimit::new(bi::MOVES_PER_SECOND, 10.0, std::time::Instant::now());
        let mut dialog_open = false;
        // the page's file picker, intercepted instead of Chrome drawing its own; latest wins
        let mut chooser: Option<OpenChooser> = None;
        let mut chooser_seq: u64 = 0;
        // the validate/inject work for an answer runs off this loop, in `tokio::spawn`; it reports
        // back a reply to forward plus, for a recoverable (not "gone") failure, the chooser to put
        // back so Try again can reuse the same id — one message, so the restore and the reply the
        // client reacts to can never be seen out of order
        let (chooser_reply_tx, mut chooser_reply_rx) = mpsc::unbounded_channel::<(Option<OpenChooser>, Value)>();
        let mut app_links = AppLinkWatch::default();
        // the events of the page the view moved to: a popup the page opened, or back from it
        let mut moved: Option<mpsc::Receiver<Value>> = None;
        loop {
            if let Some(next) = moved.take() {
                *events = next;
                frames = FrameGate::default();
                app_links = AppLinkWatch::default();
                dialog_open = false;
                chooser = None;
                pointer.forget_world();
                // Show it once records on the page in front too
                if rec.is_live() {
                    recorder_script = None;
                    recorder_acts(socket, ctrl, cdp, shared, rec, vec![Act::Install], &mut recorder_script).await?;
                }
                // the address bar moves before the new page's first picture is shown: a still
                // page (the opener once its popup closed) draws that picture once, and a client
                // must know whose it is. Its frames wait in `events` until the loop reads them.
                match read_location(cdp, ctrl).await {
                    Some(location) => {
                        // a "Continue with Google" popup in front
                        let url = location["url"].as_str().unwrap_or_default().to_string();
                        send(socket, location).await?;
                        if let Some(notice) = signin.update(&url, lease.is_some() || ctrl.is_held_for_person()) { send(socket, notice).await?; }
                    }
                    // a dialog open on the new page: the address follows when it answers
                    None => refresh_location(cdp, ctrl, shared, client_tx),
                }
            }
            let app_link_due = app_links.deadline();
            tokio::select! {
                // a popup the page opened came to the front, or closed
                _ = async { if page.changes.changed().await.is_err() { std::future::pending::<()>().await } } => {
                    moved = follow_page(ctrl, cdp, page, shared).await;
                }
                _ = async { match app_link_due { Some(at) => tokio::time::sleep_until(at).await, None => std::future::pending().await } } => {
                    if let Some(verdict) = app_links.due(Instant::now()) { act_on_app_link(cdp, client_tx, verdict); }
                }
                stuck = async { match stuck_check.as_mut() { Some(check) => check.await, None => std::future::pending().await } } => {
                    stuck_check = None;
                    if stuck.unwrap_or(false) {
                        tracing::info!("the page ignored input when the live view opened; stepping off it and back");
                        act_on_app_link(cdp, client_tx, app_links.stuck(Instant::now()));
                    }
                }
                _ = heartbeat.tick() => {
                    if last_client.elapsed() > Duration::from_secs(35) { break; }
                    send(socket, json!({"type":"ping"})).await?;
                }
                _ = location_tick.tick() => {
                    // the person handed back a resized page without releasing (a human step ended)
                    if lease.is_none() && !ctrl.is_held_for_person() && shared.resized.load(Ordering::SeqCst) {
                        let _ = jobs.send(Job::HandBack(None));
                    }
                    // a client that never acks the last frame (an identical picture fires no
                    // load) gets the next one anyway instead of a frozen view and a dropped stream,
                    // and the newest picture it missed meanwhile at once
                    let (session, next) = frames.stall(Instant::now());
                    if let Some(session) = session { cdp.fire("Page.screencastFrameAck", json!({"sessionId":session})); }
                    if let Some(next) = next { send(socket, next).await?; }
                    // a fallback for title changes no page event announces
                    refresh_location(cdp, ctrl, shared, client_tx);
                    // Show it once: the 30-minute limit and the 28:00 warning
                    let acts = rec.tick().await;
                    recorder_acts(socket, ctrl, cdp, shared, rec, acts, &mut recorder_script).await?;
                }
                cmd = rec.next_cmd() => {
                    if let Some(cmd) = cmd {
                        let acts = rec.on_cmd(cmd).await;
                        recorder_acts(socket, ctrl, cdp, shared, rec, acts, &mut recorder_script).await?;
                    }
                }
                Some(value) = client_rx.recv() => send(socket, value).await?,
                Some((restore, value)) = chooser_reply_rx.recv() => {
                    // a newer chooser (a fresh pick, a navigation) already took the slot: never
                    // clobber it with a stale one
                    if let Some(open) = restore { if chooser.is_none() { chooser = Some(open); } }
                    send(socket, value).await?;
                }
                event = events.recv() => {
                    let Some(event) = event else {
                        // the page's socket closed: a popup that closed, with its opener in front again
                        moved = back_from_closed_page(ctrl, cdp, page, shared).await;
                        if moved.is_none() { break }
                        continue;
                    };
                    let params = &event["params"];
                    let method = event["method"].as_str().unwrap_or_default();
                    // the person's own clicks, back/forward/reload and a bot's navigate all land
                    // here: the address follows at once instead of on the next tick
                    // a new document: the old title goes now, the new one follows on DOMContentLoaded
                    if let Some(url) = navigated_to(method, params) {
                        pointer.forget_world();
                        // its backendNodeId won't resolve to the new document's input
                        if let Some(open) = chooser.take() {
                            let _ = client_tx.send(chooser_error(&json!(open.id), "gone", "The page moved on."));
                        }
                        let _ = client_tx.send(json!({"type":"location","url":url,"title":""}));
                        if let Some(notice) = signin.update(&url, lease.is_some() || ctrl.is_held_for_person()) {
                            let _ = client_tx.send(notice);
                        }
                        if rec.is_live() {
                            let acts = rec.on_navigated(&url).await;
                            recorder_acts(socket, ctrl, cdp, shared, rec, acts, &mut recorder_script).await?;
                        }
                    }
                    if moves_location(method, params) { refresh_location(cdp, ctrl, shared, client_tx); }
                    match method {
                        // a link to another app the guard couldn't cancel (a frame's first address)
                        "Page.frameStartedNavigating" | "Page.frameRequestedNavigation" => {
                            app_links.started(params["url"].as_str().unwrap_or_default(), Instant::now());
                        }
                        "Log.entryAdded" => app_links.logged(params["entry"]["text"].as_str().unwrap_or_default()),
                        "Runtime.bindingCalled" if params["name"] == app_links::BINDING => {
                            let scheme = app_links::shown_scheme(params["payload"].as_str().unwrap_or_default());
                            if let Some(scheme) = scheme.filter(|_| app_links.may_notice(Instant::now())) {
                                send(socket, app_link_notice(&app_links::stopped_text(&scheme))).await?;
                            }
                        }
                        "Runtime.bindingCalled" if params["name"] == recorder::BINDING => {
                            let acts = rec.on_binding(params["payload"].as_str().unwrap_or_default()).await;
                            recorder_acts(socket, ctrl, cdp, shared, rec, acts, &mut recorder_script).await?;
                        }
                        "Page.screencastFrame" => {
                            let session = params["sessionId"].as_i64().unwrap_or_default();
                            let (dw, dh) = (params["metadata"]["deviceWidth"].as_f64().unwrap_or(0.0), params["metadata"]["deviceHeight"].as_f64().unwrap_or(0.0));
                            // a Point picture's frame: not the page's size and not worth showing
                            if shared.is_shot_frame(dw, dh) { cdp.fire("Page.screencastFrameAck", json!({"sessionId":session})); continue; }
                            if dw >= 1.0 && dh >= 1.0 { shared.set_view(dw, dh); }
                            let (fw, fh) = shared.view();
                            let (now, ack_now) = frames.arrive(session, &params["data"], fw, fh, Instant::now());
                            if let Some(session) = ack_now { cdp.fire("Page.screencastFrameAck", json!({"sessionId":session})); }
                            if let Some(frame) = now { send(socket, frame).await?; }
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
                        // during a bot's turn, nothing goes to the client: Part B covers bots
                        "Page.fileChooserOpened" if lease.is_some() || ctrl.is_held_for_person() => {
                            if let (Some(backend_node_id), Some(frame_id)) =
                                (params["backendNodeId"].as_i64(), params["frameId"].as_str())
                            {
                                let multiple = params["mode"].as_str() == Some("selectMultiple");
                                let accept = describe_accept(cdp, backend_node_id).await;
                                chooser_seq += 1;
                                let id = chooser_seq;
                                chooser = Some(OpenChooser { id, backend_node_id, frame_id: frame_id.to_string(), multiple });
                                send(socket, json!({"type":"file_chooser","id":id,"accept":accept,"multiple":multiple})).await?;
                            }
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
                            let (session, next) = frames.ack(value["id"].as_u64(), Instant::now());
                            if let Some(session) = session { cdp.fire("Page.screencastFrameAck", json!({"sessionId":session})); }
                            if let Some(next) = next { send(socket, next).await?; }
                        }
                        "take_control" => {
                            // A pending human step already holds the browser for the person
                            if lease.is_none() && !ctrl.is_held_for_person() {
                                match ctrl.take_control().await {
                                    Ok(guard) => {
                                        lease = Some(guard);
                                        // Show it once can start on this stage now
                                        rec.claim().await;
                                        // a button left down by an earlier view would turn the
                                        // person's first click into a drag
                                        let held = ctrl.pointer_held_slot().lock().unwrap_or_else(|e| e.into_inner()).take();
                                        release_on(cdp, held).await;
                                    }
                                    Err(error) => { send(socket, takeover_refused(&error)).await?; continue; }
                                }
                            }
                            let (vw, vh) = shared.view();
                            send(socket, json!({"type":"control","active":true,"viewport":{"width":vw,"height":vh}})).await?;
                            // taking control of a sign-in page: the person is about to sign in
                            let url = ctrl.current_url.lock().await.clone().unwrap_or_default();
                            if let Some(notice) = signin.update(&url, true) { send(socket, notice).await?; }
                        }
                        "release_control" => {
                            // Show it once: the edit still pending in the page is a step first
                            if rec.is_live() {
                                recorder_acts(socket, ctrl, cdp, shared, rec, vec![Act::Flush], &mut recorder_script).await?;
                            }
                            chooser = None;
                            let (done, wait) = oneshot::channel();
                            if jobs.send(Job::HandBack(Some(done))).is_ok() {
                                let _ = timeout(Duration::from_secs(8), wait).await;
                            }
                            lease = None;
                            // Show it once: Hand back stops a live recording (`handed_back`)
                            let acts = rec.handed_back().await;
                            recorder_acts(socket, ctrl, cdp, shared, rec, acts, &mut recorder_script).await?;
                            let (vw, vh) = shared.view();
                            send(socket, json!({"type":"control","active":false,"viewport":{"width":vw,"height":vh}})).await?;
                        }
                        // the view closes or goes to the background: only this view's own size
                        // goes, so it needs no control (a human step's view holds none)
                        "restore_viewport" => { let _ = jobs.send(Job::Restore(id)); }
                        // G3 Point at it: read-only, so watching is enough
                        "point_at" | "point_neighbour" | "point_pick" => pointer.handle(&value, id, shared),
                        "file_chooser" => {
                            if lease.is_none() && !ctrl.is_held_for_person() {
                                send(socket, chooser_error(&id, "failed", not_in_control(ctrl.control.try_read().is_err()))).await?;
                                continue;
                            }
                            let Some(seq) = value["id"].as_u64() else { send(socket, chooser_error(&id, "failed", protocol("Missing id"))).await?; continue; };
                            if chooser.as_ref().map(|c| c.id) != Some(seq) {
                                send(socket, chooser_error(&id, "gone", "The page moved on.")).await?;
                                continue;
                            }
                            let open = chooser.take().expect("checked above");
                            if value["cancel"] == true {
                                // the sheet already knows it cancelled; no reply needed
                                continue;
                            }
                            let ids: Vec<String> = value["attachment_ids"].as_array().cloned().unwrap_or_default()
                                .into_iter().filter_map(|v| v.as_str().map(str::to_string)).collect();
                            let (state, cdp, reply_tx) = (state.clone(), cdp.clone(), chooser_reply_tx.clone());
                            tokio::spawn(async move {
                                let (restore, reply) = match answer_file_chooser(&state, &cdp, local, &open, ids).await {
                                    Ok(names) => (None, json!({"type":"file_chooser_done","id":open.id,"names":names})),
                                    Err((code, message)) => {
                                        // "gone" means the picker itself is really gone (the page
                                        // navigated, or CDP lost the node); anything else is a
                                        // failure the same picker can still answer, so Try again
                                        // needs its id to still mean something
                                        let restore = (code != "gone").then(|| open.clone());
                                        (restore, chooser_error(&json!(open.id), code, message))
                                    }
                                };
                                let _ = reply_tx.send((restore, reply));
                            });
                        }
                        _ => {
                            if lease.is_none() && !ctrl.is_held_for_person() {
                                send(socket, error(&id, not_in_control(ctrl.control.try_read().is_err()))).await?;
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

/// Reads the `accept` attribute straight off the DOM node CDP already named in
/// `Page.fileChooserOpened`'s `backendNodeId`, with no `DOM.resolveNode`/`Runtime.callFunctionOn`
/// round trip just for this. `multiple` comes from the event's own `mode` instead (never raced).
async fn describe_accept(cdp: &Cdp, backend_node_id: i64) -> String {
    let Ok(described) = cdp.call("DOM.describeNode", json!({"backendNodeId": backend_node_id}), PROBE_WAIT).await else {
        return String::new();
    };
    accept_attr(described["node"]["attributes"].as_array().map(Vec::as_slice).unwrap_or_default())
}

/// `DOM.describeNode`'s `attributes` is a flat `[name, value, name, value, ...]` array; this picks
/// out `accept`'s value, or `""` when the input has none (any file type).
fn accept_attr(attributes: &[Value]) -> String {
    attributes
        .chunks(2)
        .find(|pair| pair.first().and_then(Value::as_str) == Some("accept"))
        .and_then(|pair| pair.get(1))
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string()
}

/// Validates the client's answer (known ids, belongs to this person's own unsent uploads, not
/// over the remote cap), then puts the files into the page. Names on success, or a coded error
/// for the sheet (`too_big`, `gone`, `failed` — matching the brief exactly).
async fn answer_file_chooser(
    state: &AppState,
    cdp: &Cdp,
    local: bool,
    open: &OpenChooser,
    ids: Vec<String>,
) -> Result<Vec<String>, (&'static str, String)> {
    if ids.is_empty() {
        return Err(("failed", "No file was chosen.".to_string()));
    }
    let ids: Vec<String> = if open.multiple { ids } else { ids.into_iter().take(1).collect() };
    let attachments = super::attachments::check_uploads(state, &ids).await.map_err(|e| {
        tracing::warn!("file chooser: check_uploads refused the answer: {}", e);
        file_chooser_failed()
    })?;
    if !local {
        let total: i64 = attachments.iter().map(|a| a.size).sum();
        if total < 0 || total as u64 > REMOTE_UPLOAD_CAP_BYTES {
            return Err(("too_big", "That’s more than this browser can take: 100 MB in all. Pick a smaller file.".to_string()));
        }
    }
    let names = attachments.iter().map(|a| a.name.clone()).collect();
    if local {
        set_file_input_files_local(state, cdp, open, &attachments).await?;
    } else {
        inject_files_remote(state, cdp, open, &attachments).await?;
    }
    Ok(names)
}

/// Local Chrome shares kwnt's filesystem, but `DOM.setFileInputFiles` names the file from the
/// path's own basename, not from any metadata, and the blob is named by its sha256 with no
/// extension. A copy named after the attachment's real name fixes that — the same workaround
/// Playwright's own buffer-based `setInputFiles` uses internally. The page may only read the
/// bytes later (e.g. only once the person clicks "Post"), so the copy can't be removed right
/// after this call returns: it's scheduled for removal after `CHOOSER_TMP_TTL`, with an
/// opportunistic sweep of older orphans on every new chooser as a backstop if kwnt restarts.
async fn set_file_input_files_local(
    state: &AppState,
    cdp: &Cdp,
    open: &OpenChooser,
    attachments: &[Attachment],
) -> Result<(), (&'static str, String)> {
    let root = super::attachments::root(state);
    let tmp_root = root.join("chooser-tmp");
    sweep_chooser_tmp(&tmp_root).await;
    let dir = tmp_root.join(uuid::Uuid::new_v4().to_string());
    tokio::fs::create_dir_all(&dir).await.map_err(|e| {
        tracing::warn!("file chooser: couldn't create {}: {}", dir.display(), e);
        file_chooser_failed()
    })?;
    let mut paths = Vec::with_capacity(attachments.len());
    for a in attachments {
        let Some(blob) = super::attachments::blob_path(&root, &a.sha256) else {
            tracing::warn!("file chooser: no blob for attachment {}", a.id);
            return Err(file_chooser_failed());
        };
        let dest = dir.join(super::attachments::clean_name(&a.name));
        tokio::fs::copy(&blob, &dest).await.map_err(|e| {
            tracing::warn!("file chooser: couldn't copy {} to {}: {}", blob.display(), dest.display(), e);
            file_chooser_failed()
        })?;
        paths.push(dest);
    }
    let result = cdp
        .call(
            "DOM.setFileInputFiles",
            json!({"files": paths.iter().map(|p| p.to_string_lossy().to_string()).collect::<Vec<_>>(), "backendNodeId": open.backend_node_id}),
            ACTION_WAIT,
        )
        .await;
    let cleanup_dir = dir.clone();
    tokio::spawn(async move {
        tokio::time::sleep(CHOOSER_TMP_TTL).await;
        let _ = tokio::fs::remove_dir_all(&cleanup_dir).await;
    });
    result.map(|_| ()).map_err(|_| ("gone", "The page moved on.".to_string()))
}

/// Deletes any `chooser-tmp` directory older than `CHOOSER_TMP_ORPHAN_AGE`, a backstop for the
/// scheduled cleanup in [`set_file_input_files_local`] if the process restarted before it ran.
async fn sweep_chooser_tmp(tmp_root: &std::path::Path) {
    let Ok(mut entries) = tokio::fs::read_dir(tmp_root).await else { return };
    let cutoff = std::time::SystemTime::now() - CHOOSER_TMP_ORPHAN_AGE;
    while let Ok(Some(entry)) = entries.next_entry().await {
        if entry.metadata().await.ok().and_then(|m| m.modified().ok()).is_some_and(|m| m < cutoff) {
            let _ = tokio::fs::remove_dir_all(entry.path()).await;
        }
    }
}

/// Chrome somewhere else (`KWNT_BROWSER_CDP_URL`) has no filesystem shared with kwnt, so the
/// bytes go in as base64 chunks instead, evaluated in the recorder's own isolated world (reusing
/// `Page.fileChooserOpened`'s own `frameId` — no extra `Page.getFrameTree` round trip, unlike
/// `record_pick`) so the upload's own finish call can tell the recorder directly (S-C13: the
/// label only, never the file's name). Kept as `Uint8Array`s in the page, not concatenated into a
/// JS number array, so a 100 MB file doesn't balloon past the compose browser's 1536m container.
async fn inject_files_remote(
    state: &AppState,
    cdp: &Cdp,
    open: &OpenChooser,
    attachments: &[Attachment],
) -> Result<(), (&'static str, String)> {
    use base64::Engine;
    let gone = || ("gone", "The page moved on.".to_string());
    let failed = file_chooser_failed;
    let world = cdp
        .call("Page.createIsolatedWorld", json!({"frameId": open.frame_id, "worldName": recorder::WORLD}), PROBE_WAIT)
        .await
        .map_err(|_| gone())?;
    let context = world["executionContextId"].as_i64().ok_or_else(gone)?;
    let resolved = cdp
        .call("DOM.resolveNode", json!({"backendNodeId": open.backend_node_id, "executionContextId": context}), PROBE_WAIT)
        .await
        .map_err(|_| gone())?;
    let object = resolved["object"]["objectId"].as_str().map(str::to_string).ok_or_else(gone)?;
    let root = super::attachments::root(state);
    let outcome = async {
        for a in attachments {
            let blob = super::attachments::blob_path(&root, &a.sha256).ok_or_else(failed)?;
            let bytes = tokio::fs::read(&blob).await.map_err(|_| failed())?;
            let began = cdp
                .call(
                    "Runtime.callFunctionOn",
                    json!({
                        "objectId": object, "functionDeclaration": bi::UPLOAD_BEGIN_FN,
                        "arguments": [{"value": {"name": a.name.clone(), "mime": a.mime.clone(), "lastModified": a.created_at.timestamp_millis()}}],
                        "returnByValue": true,
                    }),
                    ACTION_WAIT,
                )
                .await;
            if !began.is_ok_and(|r| r["result"]["value"] == true) { return Err(failed()); }
            for chunk in bytes.chunks(UPLOAD_CHUNK_BYTES) {
                let b64 = base64::engine::general_purpose::STANDARD.encode(chunk);
                let chunked = cdp
                    .call(
                        "Runtime.callFunctionOn",
                        json!({"objectId": object, "functionDeclaration": bi::UPLOAD_CHUNK_FN, "arguments": [{"value": b64}], "returnByValue": true}),
                        ACTION_WAIT,
                    )
                    .await;
                if !chunked.is_ok_and(|r| r["result"]["value"] == true) { return Err(failed()); }
            }
        }
        let finished = cdp
            .call(
                "Runtime.callFunctionOn",
                json!({"objectId": object, "functionDeclaration": bi::UPLOAD_FINISH_FN, "returnByValue": true}),
                ACTION_WAIT,
            )
            .await;
        if finished.is_ok_and(|r| r["result"]["value"] == true) { Ok(()) } else { Err(failed()) }
    }.await;
    let _ = cdp.call("Runtime.releaseObject", json!({"objectId": object}), PROBE_WAIT).await;
    outcome
}

/// Keeps links to another app from freezing the tab while the live view is open (see
/// [`app_links`]), and has the guard say when it stopped one. Runtime is enabled for that call
/// and Log for Chrome's own refusals.
async fn guard_app_links(cdp: &Cdp) {
    for (method, params) in [
        ("Runtime.addBinding", json!({"name": app_links::BINDING, "executionContextName": app_links::WORLD})),
        ("Page.addScriptToEvaluateOnNewDocument", json!({"source": app_links::guard_js(), "worldName": app_links::WORLD, "runImmediately": true})),
        ("Runtime.enable", json!({})),
        ("Log.enable", json!({})),
    ] {
        if let Err(e) = cdp.call(method, params, PROBE_WAIT).await { tracing::debug!(method, "app link guard: {}", e); }
    }
}

/// Does what the Show it once recorder asks of this socket (see [`super::show_once::stage`]): the
/// recorder goes into kwnt's isolated world only, where the page's scripts can't reach it.
async fn recorder_acts(
    socket: &mut WebSocket,
    ctrl: &BrowserController,
    cdp: &Cdp,
    shared: &Shared,
    rec: &mut StageRecorder,
    acts: Vec<Act>,
    script: &mut Option<String>,
) -> Result<(), BrowserError> {
    let mut queue: std::collections::VecDeque<Act> = acts.into();
    while let Some(act) = queue.pop_front() {
        match act {
            Act::Send(frame) => send(socket, frame).await?,
            Act::Install => {
                shared.recording.store(true, Ordering::SeqCst);
                if script.is_none() {
                    let binding = json!({"name": recorder::BINDING, "executionContextName": recorder::WORLD});
                    if let Err(e) = cdp.call("Runtime.addBinding", binding, PROBE_WAIT).await { tracing::debug!("recorder binding: {}", e); }
                    let source = json!({"source": recorder::recorder_js(), "worldName": recorder::WORLD, "runImmediately": true});
                    match cdp.call("Page.addScriptToEvaluateOnNewDocument", source, PROBE_WAIT).await {
                        Ok(added) => *script = added["identifier"].as_str().map(str::to_string),
                        Err(e) => tracing::warn!("show it once: the recorder wasn't installed: {}", e),
                    }
                }
                if let Some(url) = current_url(cdp).await { queue.extend(rec.on_installed(&url).await); }
            }
            Act::Uninstall => {
                shared.recording.store(false, Ordering::SeqCst);
                if let Some(identifier) = script.take() {
                    let _ = cdp.call("Page.removeScriptToEvaluateOnNewDocument", json!({"identifier": identifier}), PROBE_WAIT).await;
                }
            }
            Act::Navigate(url) => {
                // the person's own navigation: never held to a bot's site rules (W-T10)
                ctrl.site_guard.set_actor(crate::sandbox::site_guard::Actor::Person);
                let opened = match cdp.call("Page.navigate", json!({"url": url}), Duration::from_secs(10)).await {
                    Ok(r) => r["errorText"].as_str().is_none_or(str::is_empty),
                    Err(e) => { tracing::debug!("recorder start page: {}", e); false }
                };
                if !opened { queue.extend(rec.on_start_failed(current_url(cdp).await.as_deref()).await); }
            }
            Act::Flush => {
                let pending = flush_recorder(cdp).await;
                queue.extend(rec.on_flushed(&pending).await);
            }
            Act::Stop { reason, done } => {
                queue.extend(rec.stop_now(&reason).await);
                done.fire();
            }
        }
    }
    Ok(())
}

/// The Show it once recorder's pending field edits, taken from its isolated world (the same world
/// the recorder script was added to on this connection). Nothing when the page can't be reached.
async fn flush_recorder(cdp: &Cdp) -> Vec<String> {
    let Ok(tree) = cdp.call("Page.getFrameTree", json!({}), PROBE_WAIT).await else { return vec![] };
    let Some(frame) = tree["frameTree"]["frame"]["id"].as_str() else { return vec![] };
    let world = json!({"frameId": frame, "worldName": recorder::WORLD});
    let Ok(world) = cdp.call("Page.createIsolatedWorld", world, PROBE_WAIT).await else { return vec![] };
    let Some(context) = world["executionContextId"].as_i64() else { return vec![] };
    let eval = json!({"expression": recorder::FLUSH, "contextId": context, "returnByValue": true});
    let Ok(r) = cdp.call("Runtime.evaluate", eval, PROBE_WAIT).await else { return vec![] };
    r["result"]["value"].as_array()
        .map(|a| a.iter().filter_map(|v| v.as_str().map(str::to_string)).take(super::show_once::stage::MAX_FLUSHED).collect())
        .unwrap_or_default()
}

/// The address of the page open in the tab now.
async fn current_url(cdp: &Cdp) -> Option<String> {
    let history = cdp.call("Page.getNavigationHistory", json!({}), PROBE_WAIT).await.ok()?;
    let index = history["currentIndex"].as_u64()? as usize;
    history["entries"].get(index)?["url"].as_str().map(str::to_string)
}

/// Recovers the tab from Chrome's "Open <app>?" prompt off the loop and tells the person, or
/// only tells them when it has been recovered too often.
fn act_on_app_link(cdp: &Cdp, client: &mpsc::UnboundedSender<Value>, verdict: Verdict) {
    match verdict {
        Verdict::Recover(scheme) => {
            let (cdp, client) = (cdp.clone(), client.clone());
            tokio::spawn(async move {
                let text = match leave_app_prompt(&cdp).await {
                    Ok(()) => app_links::recovered_text(scheme.as_deref()),
                    Err(e) => { tracing::debug!("couldn't close an app prompt: {}", e); app_links::gave_up_text(scheme.as_deref()) }
                };
                let _ = client.send(app_link_notice(&text));
            });
        }
        Verdict::GiveUp(scheme) => { let _ = client.send(app_link_notice(&app_links::gave_up_text(scheme.as_deref()))); }
    }
}

/// Whether the page ignores input: scripts run, but a mouse move at a point outside any frame
/// never reaches it. That is what Chrome's "Open <app>?" prompt does to a tab. Anything that
/// can't be told for sure (no point to probe, the page changed or didn't answer) is `false`.
async fn input_is_stuck(cdp: &Cdp) -> bool {
    let eval = |expression: &str| cdp.call("Runtime.evaluate", json!({"expression": expression, "returnByValue": true}), PROBE_WAIT);
    let Ok(armed) = eval(app_links::PROBE_ARM_JS).await else { return false };
    let point = armed["result"]["value"].as_array().and_then(|p| Some((p.first()?.as_f64()?, p.get(1)?.as_f64()?)));
    let Some((x, y)) = point else { return false };
    // two moves, so one is a move even if the pointer already rests on the first point
    for dx in [0.0, 1.0] {
        let moved = json!({"type":"mouseMoved","x":x + dx,"y":y,"button":"none","buttons":0});
        if cdp.call("Input.dispatchMouseEvent", moved, PROBE_WAIT).await.is_err() { break; }
    }
    tokio::time::sleep(Duration::from_millis(100)).await;
    let reached = eval(app_links::PROBE_READ_JS).await.ok().and_then(|r| r["result"]["value"].as_i64());
    reached == Some(0)
}

/// Closes Chrome's "Open <app>?" prompt. Only a cross-site navigation does, so the tab steps to
/// about:blank and straight back to the entry it was on: the page loads again, as on a reload.
async fn leave_app_prompt(cdp: &Cdp) -> Result<(), BrowserError> {
    let current = |history: &Value| {
        let index = history["currentIndex"].as_u64()? as usize;
        let entry = history["entries"].get(index)?;
        Some((entry["id"].as_i64()?, entry["url"].as_str().unwrap_or_default().to_string()))
    };
    let history = cdp.call("Page.getNavigationHistory", json!({}), PROBE_WAIT).await?;
    let (entry, _) = current(&history).ok_or_else(|| protocol("No page to go back to"))?;
    cdp.call("Page.navigate", json!({"url": "about:blank"}), Duration::from_secs(10)).await?;
    // Chrome swaps the page's renderer on the way: until it's done, commands on this socket fail
    // with "Not attached to an active page"
    let mut last = Err(protocol("The page didn't come back"));
    for _ in 0..60 {
        tokio::time::sleep(Duration::from_millis(50)).await;
        let Ok(history) = cdp.call("Page.getNavigationHistory", json!({}), PROBE_WAIT).await else { continue };
        if !current(&history).is_some_and(|(id, url)| id != entry && url == "about:blank") { continue; }
        last = cdp.call("Page.navigateToHistoryEntry", json!({"entryId": entry}), Duration::from_secs(10)).await;
        if last.is_ok() { break; }
    }
    last.map(|_| ())
}

/// Tells the person about a link to another app. It has no id, so it shows as a banner.
fn app_link_notice(text: &str) -> Value {
    json!({"type":"error","notice":"app_link","error":text})
}

async fn send(socket: &mut WebSocket, value: Value) -> Result<(), BrowserError> {
    timeout(Duration::from_secs(5), socket.send(Message::Text(value.to_string()))).await
        .map_err(|_| BrowserError::Timeout)?.map_err(|e| BrowserError::Ws(e.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// G3: a clipped Point picture makes Chrome draw one frame at the clip's size; only that
    /// frame, soon after the picture, is kept from becoming the view's size.
    #[test]
    fn only_the_pictures_own_frame_is_not_the_view() {
        let at = Instant::now();
        let shot = Some((600.0, 40.4, at));
        assert!(shot_frame(shot, 600.0, 40.0, at + Duration::from_millis(50)));
        assert!(!shot_frame(shot, 1280.0, 800.0, at), "the page's own size");
        assert!(!shot_frame(shot, 600.0, 42.0, at), "another size");
        assert!(!shot_frame(shot, 600.0, 40.0, at + SHOT_FRAME_WINDOW + Duration::from_millis(1)), "too late");
        assert!(!shot_frame(None, 600.0, 40.0, at), "no picture");
    }

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
    fn latest_keeps_only_the_newest_value_for_one_runner() {
        let latest = Latest::new();
        assert!(latest.offer(1), "the first offer starts the runner");
        assert!(!latest.offer(2), "a busy runner is not started twice");
        assert!(!latest.offer(3));
        assert_eq!(latest.next(), Some(3), "moves inside the window collapse to the newest");
        // an offer while the runner works is its trailing value, not a second runner
        assert!(!latest.offer(4));
        assert_eq!(latest.next(), Some(4));
        assert_eq!(latest.next(), None, "nothing left: the runner stops");
        assert!(latest.offer(5), "after it stopped the next offer starts a new runner");
        assert_eq!(latest.next(), Some(5));
        assert_eq!(latest.next(), None);
    }

    #[test]
    fn latest_never_loses_an_offer_or_runs_twice_under_contention() {
        use std::sync::atomic::AtomicUsize;
        let latest = Arc::new(Latest::new());
        let (running, most, drained) = (Arc::new(AtomicUsize::new(0)), Arc::new(AtomicUsize::new(0)), Arc::new(AtomicUsize::new(0)));
        let mut runners = vec![];
        let offerers: Vec<_> = (0..4).map(|t| {
            let (latest, running, most, drained) = (latest.clone(), running.clone(), most.clone(), drained.clone());
            std::thread::spawn(move || {
                let mut started = vec![];
                for i in 0..2000 {
                    if latest.offer(t * 10_000 + i) {
                        let (latest, running, most, drained) = (latest.clone(), running.clone(), most.clone(), drained.clone());
                        started.push(std::thread::spawn(move || {
                            let now = running.fetch_add(1, Ordering::SeqCst) + 1;
                            most.fetch_max(now, Ordering::SeqCst);
                            while latest.next().is_some() { drained.fetch_add(1, Ordering::SeqCst); }
                            running.fetch_sub(1, Ordering::SeqCst);
                        }));
                    }
                }
                started
            })
        }).collect();
        for offerer in offerers { runners.extend(offerer.join().unwrap()); }
        for runner in runners { runner.join().unwrap(); }
        assert_eq!(most.load(Ordering::SeqCst), 1, "never two runners at once");
        assert!(drained.load(Ordering::SeqCst) >= 1);
        assert!(latest.lock().is_none(), "the last offer was taken by a runner");
        assert!(!latest.busy.load(Ordering::SeqCst), "and the runner stopped");
    }

    #[test]
    fn cursor_lookups_wait_out_the_window_after_the_last_one() {
        let t0 = Instant::now();
        assert_eq!(cursor_wait(None, t0), Duration::ZERO, "the first move is looked up at once");
        assert_eq!(cursor_wait(Some(t0), t0 + Duration::from_millis(40)), Duration::from_millis(85));
        assert_eq!(cursor_wait(Some(t0), t0 + bi::CURSOR_EVERY), Duration::ZERO);
        assert_eq!(cursor_wait(Some(t0), t0 + Duration::from_secs(5)), Duration::ZERO);
    }

    #[test]
    fn the_address_follows_top_level_navigation_and_title() {
        assert!(moves_location("Page.frameNavigated", &json!({"frame":{"id":"A","url":"https://b.test/"}})));
        assert!(!moves_location("Page.frameNavigated", &json!({"frame":{"id":"B","parentId":"A","url":"https://ads.test/"}})), "an iframe moving is not the page moving");
        assert!(moves_location("Page.navigatedWithinDocument", &json!({"frameId":"A","url":"https://b.test/#x"})));
        assert!(moves_location("Page.domContentEventFired", &json!({"timestamp":1.0})));
        for other in ["Page.screencastFrame", "Page.javascriptDialogOpening", "Page.loadEventFired", ""] {
            assert!(!moves_location(other, &json!({})), "{other}");
        }
    }

    #[test]
    fn a_new_top_level_document_clears_the_title_and_keeps_the_fragment() {
        assert_eq!(navigated_to("Page.frameNavigated", &json!({"frame":{"id":"A","url":"https://b.test/"}})).as_deref(), Some("https://b.test/"));
        assert_eq!(navigated_to("Page.frameNavigated", &json!({"frame":{"id":"A","url":"https://b.test/","urlFragment":"#top"}})).as_deref(), Some("https://b.test/#top"));
        assert_eq!(navigated_to("Page.frameNavigated", &json!({"frame":{"id":"B","parentId":"A","url":"https://ads.test/"}})), None, "an iframe keeps the page's title");
        assert_eq!(navigated_to("Page.navigatedWithinDocument", &json!({"frameId":"A","url":"https://b.test/#x"})), None, "same document, same title");
        assert_eq!(navigated_to("Page.domContentEventFired", &json!({"timestamp":1.0})), None);
    }

    /// Take over from a second window while the first has control is `held_by_person`, never
    /// the busy bot's error; a bot mid-step is `bot_busy`; once the first window hands back (its
    /// lease dropped) the second can take over.
    #[tokio::test]
    async fn takeover_refused_says_whether_the_person_or_a_bot_has_the_browser() {
        let ctrl = BrowserController::new(0);
        let first = ctrl.take_control().await.expect("a free browser");
        let refused = ctrl.take_control().await.expect_err("one window at a time");
        assert!(matches!(refused, BrowserError::HeldByPerson));
        let wire = takeover_refused(&refused);
        assert_eq!(wire["type"], "error");
        assert_eq!(wire["code"], "held_by_person");
        assert_eq!(wire["error"], "You're controlling this browser in another window. Press Hand back there to take over here.");
        drop(first);

        // A bot action in flight holds the read side
        let bot = ctrl.control.clone().try_read_owned().unwrap();
        let refused = ctrl.take_control().await.expect_err("the bot is mid-step");
        assert!(matches!(refused, BrowserError::HumanControl));
        assert_eq!(takeover_refused(&refused)["code"], "bot_busy");
        drop(bot);

        let second = ctrl.take_control().await.expect("free again");
        assert!(matches!(ctrl.take_control().await, Err(BrowserError::HeldByPerson)));
        drop(second);
        assert!(ctrl.take_control().await.is_ok(), "the count went down with the lease");
    }

    #[test]
    fn input_without_control_says_to_take_control_unless_someone_has_it() {
        assert_eq!(not_in_control(false).to_string(), "Take control of the browser first.");
        assert!(matches!(not_in_control(true), BrowserError::HumanControl));
    }

    #[test]
    fn a_picture_drawn_while_one_is_out_is_sent_next_on_ack_or_stall() {
        let t0 = Instant::now();
        let mut gate = FrameGate::default();
        let (first, ack_now) = gate.arrive(7, &json!("red"), 800.0, 600.0, t0);
        assert_eq!((first.unwrap()["id"].clone(), ack_now), (json!(1), None));
        // two more pictures while the first is out: Chrome gets its ack at once, the newest is kept
        assert_eq!(gate.arrive(8, &json!("green"), 800.0, 600.0, t0), (None, Some(8)));
        assert_eq!(gate.arrive(9, &json!("blue"), 800.0, 600.0, t0), (None, Some(9)));
        // a stale or unknown ack changes nothing
        assert_eq!(gate.ack(Some(5), t0), (None, None));
        assert_eq!(gate.ack(None, t0), (None, None));
        // the ack lets the first go and sends the newest picture with a new id
        let (session, next) = gate.ack(Some(1), t0);
        let next = next.expect("the missed picture");
        assert_eq!((session, next["id"].clone(), next["image"].clone(), next["width"].clone()), (Some(7), json!(2), json!("blue"), json!(800.0)));
        // its ack has nothing left to ack to Chrome or send
        assert_eq!(gate.ack(Some(2), t0), (None, None));

        // the same after a lost ack: nothing before the stall, the missed picture after it
        let (out, _) = gate.arrive(10, &json!("red"), 800.0, 600.0, t0);
        assert_eq!(out.unwrap()["id"], 3);
        assert_eq!(gate.arrive(11, &json!("blue"), 800.0, 600.0, t0), (None, Some(11)));
        assert_eq!(gate.stall(t0 + FRAME_STALL), (None, None));
        let (session, next) = gate.stall(t0 + FRAME_STALL + Duration::from_millis(1));
        let next = next.expect("the missed picture after the stall");
        assert_eq!((session, next["id"].clone(), next["image"].clone()), (Some(10), json!(4), json!("blue")));
        // a late ack of the stalled frame is ignored; a resend that is never acked is let go too
        assert_eq!(gate.ack(Some(3), t0), (None, None));
        assert_eq!(gate.stall(t0 + FRAME_STALL * 3), (None, None));
        let (out, _) = gate.arrive(12, &json!("still"), 800.0, 600.0, t0 + FRAME_STALL * 3);
        assert_eq!(out.unwrap()["id"], 5, "a new picture goes out once the resend is let go");
        // a frame out with nothing missed: the stall only lets it go
        assert_eq!(gate.stall(t0 + FRAME_STALL * 5), (Some(12), None));
    }

    #[test]
    fn letting_go_releases_every_held_button_in_order() {
        let events = release_events(HeldPointer { x: 3.0, y: 4.0, buttons: 1 | 2 | 4 });
        let shape: Vec<_> = events.iter().map(|e| (e["button"].as_str().unwrap().to_string(), e["buttons"].as_u64().unwrap())).collect();
        assert_eq!(shape, vec![("left".into(), 6), ("right".into(), 4), ("middle".into(), 0)]);
        assert!(events.iter().all(|e| e["type"] == "mouseReleased" && e["x"] == 3.0 && e["y"] == 4.0));
        assert!(release_events(HeldPointer { x: 0.0, y: 0.0, buttons: 0 }).is_empty());
    }

    /// Its own `type`, never `error`/`ack`: those are matched against the client's own
    /// `sequence.current` id-space, which the chooser's server-assigned id can collide with.
    #[test]
    fn a_file_chooser_failure_never_looks_like_a_generic_error() {
        let wire = chooser_error(&json!(7), "too_big", "That's bigger than kwnt can send here (100 MB).");
        assert_eq!(wire["type"], "file_chooser_error");
        assert_eq!(wire["id"], 7);
        assert_eq!(wire["code"], "too_big");
        assert_eq!(wire["error"], "That's bigger than kwnt can send here (100 MB).");
        assert_ne!(wire["type"], "error");
        assert_ne!(wire["type"], "ack");
    }

    #[test]
    fn accept_reads_the_flat_attribute_pairs_or_falls_back_to_any_file() {
        assert_eq!(accept_attr(&[json!("id"), json!("f1"), json!("accept"), json!("video/*")]), "video/*");
        assert_eq!(accept_attr(&[json!("accept"), json!("image/*"), json!("multiple"), json!("")]), "image/*");
        assert_eq!(accept_attr(&[json!("id"), json!("f1")]), "", "no accept attribute: any file type");
        assert_eq!(accept_attr(&[]), "");
        // a dangling name with no paired value doesn't panic
        assert_eq!(accept_attr(&[json!("accept")]), "");
    }
}
