//! Downloads a bot's own page triggers, and file choosers it opens, have nobody to show a card
//! to and no Hand back/Try again button: `browser_stream.rs`'s chooser and download handling is
//! built for the person's own live view. This module gives every bot session its own, dedicated
//! DevTools connection (never the one `send_cdp_command` uses for ordinary actions) that stays
//! connected in the background, lands finished downloads in the bot's own `downloads/` folder,
//! and answers a bot's own file picker from a file in that folder or a chat attachment.
//!
//! Known, accepted gaps (see `autonomy/bot-browser-files/impl-files.md`): whether
//! `Browser.setDownloadBehavior` / `Page.setInterceptFileChooserDialog` are scoped to this one
//! DevTools connection or to the whole target was not verified against real Chrome (the earlier
//! harness run was stopped rather than risk the shared server's memory); this follows the
//! `Browser.*` methods' own CDP domain (browser-level, not page-level) and is documented as an
//! assumption. Because of that assumption, every session sets `Browser.setDownloadBehavior` to
//! the *same* shared staging folder (outside every bot's workspace) and claims a finished
//! download only once its `downloadWillBegin.frameId` shows up in this page's own frame tree —
//! regardless of whether the setting turns out to be per-connection or browser-wide, only the
//! center whose page the download actually started on ever moves the file into its own
//! `downloads/`. Reconnecting to follow a popup polls `get_ws_url()` every few seconds rather than
//! reacting the instant a popup opens, so capture on a brand new popup can lag by that long.
//!
//! Remote Chrome (`KWNT_BROWSER_CDP_URL`) has no filesystem in common with this server, so a
//! staging path means nothing there: `local` sessions get `allowAndName` at the shared staging
//! path; every other session gets `deny` with `eventsEnabled: true`, and a download attempt is
//! reported back as failed rather than tracked.

use super::browser::{connect_cdp, BrowserController, BrowserError, WsStream};
use super::safe_fs;
use futures::{SinkExt, StreamExt};
use serde_json::{json, Value};
use std::collections::HashSet;
use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex as StdMutex};
use std::time::Duration;
use tokio::sync::{mpsc, oneshot};
use tokio_tungstenite::tungstenite::Message;

/// A download is cancelled once it passes this: nobody asked for anything this large, and the
/// bot's folder isn't a place to quietly fill a disk.
const MAX_DOWNLOAD_BYTES: u64 = 1_000_000_000;
/// This browser has no filesystem in common with kwnt: the most this connection will chunk in
/// for one upload, mirroring `browser_stream.rs`'s own remote-chooser cap.
const REMOTE_UPLOAD_CAP_BYTES: u64 = 100 * 1024 * 1024;
const UPLOAD_CHUNK_BYTES: usize = 1024 * 1024;
const FOLLOW_EVERY: Duration = Duration::from_secs(2);
const RETRY_WAIT: Duration = Duration::from_secs(3);
const JOB_WAIT: Duration = Duration::from_secs(20);
/// A staged download left in the shared incoming folder this long without being landed (no
/// center's frame tree ever claimed it, or a crash interrupted the move) is swept as an orphan.
const INCOMING_ORPHAN_AGE: Duration = Duration::from_secs(3600);
/// Remote Chrome has no filesystem in common with this server, so a download can't be kept.
const REMOTE_DOWNLOAD_REFUSAL: &str = "Downloads need the browser on this computer for now.";

#[derive(Clone)]
enum DlStatus {
    InProgress,
    Done(String),
    Cancelled,
    Failed(String),
}

#[derive(Clone)]
struct DownloadEntry {
    guid: String,
    name: String,
    total: Option<u64>,
    received: u64,
    status: DlStatus,
    reported_terminal: bool,
}

#[derive(Clone)]
struct ChooserInfo {
    backend_node_id: i64,
    frame_id: String,
    accept: String,
    multiple: bool,
}

#[derive(Default)]
struct Shared {
    download: Option<DownloadEntry>,
    chooser: Option<ChooserInfo>,
    /// Frame ids belonging to this page, seeded from `Page.getFrameTree` at setup and kept
    /// current from `Page.frameAttached`/`Page.frameNavigated`: the set a `downloadWillBegin`
    /// event's `frameId` is checked against before this center claims the download as its own.
    frames: HashSet<String>,
}

struct Job {
    method: String,
    params: Value,
    reply: oneshot::Sender<Result<Value, BrowserError>>,
}

/// A file ready to hand to a chooser: already resolved and validated by the caller (a workspace
/// path opened through [`safe_fs`], or a chat attachment) before it reaches this module, which
/// only ever deals in CDP mechanics.
pub struct UploadFile {
    pub name: String,
    pub mime: String,
    pub last_modified_ms: i64,
    pub source: UploadSource,
}

pub enum UploadSource {
    /// A path this Chrome can open directly (same host), already placed under its correct
    /// display name by the caller.
    Path(PathBuf),
    /// This browser has no filesystem in common with kwnt: the bytes, sent in over CDP.
    Bytes(Vec<u8>),
}

impl UploadFile {
    fn len(&self) -> u64 {
        match &self.source {
            UploadSource::Path(p) => std::fs::metadata(p).map(|m| m.len()).unwrap_or(0),
            UploadSource::Bytes(b) => b.len() as u64,
        }
    }
}

/// One dedicated connection per bot session, running in the background for as long as the
/// session's [`BrowserController`] lives.
#[derive(Clone)]
pub struct DownloadCenter {
    shared: Arc<StdMutex<Shared>>,
    jobs: mpsc::Sender<Job>,
}

impl DownloadCenter {
    /// `local`: whether this Chrome shares a filesystem with this server (see the module doc).
    /// `browser_context_id`: the session's own context, for a non-default context (VM / job
    /// pages); `None` for a local page, which shares the kwant profile's default context.
    /// `staging_root`: one folder shared by every session, outside any bot's workspace.
    ///
    /// Waits briefly for the first connect-and-setup round trip (`Page.enable`,
    /// `Browser.setDownloadBehavior`, ...) to land before returning, so that round trip is always
    /// part of whatever request just opened this session, never something a later, unrelated
    /// request races against and appears to have caused. A browser slow or unreachable past that
    /// wait just keeps retrying in the background, same as before this wait existed.
    pub async fn start(ctrl: BrowserController, workspace: PathBuf, local: bool, browser_context_id: Option<String>, staging_root: PathBuf) -> Self {
        let shared = Arc::new(StdMutex::new(Shared::default()));
        let (tx, rx) = mpsc::channel(8);
        let (ready_tx, ready_rx) = oneshot::channel();
        let center = Self { shared: shared.clone(), jobs: tx };
        tokio::spawn(run(ctrl, workspace, local, browser_context_id, staging_root, shared, rx, ready_tx));
        let _ = tokio::time::timeout(Duration::from_secs(2), ready_rx).await;
        center
    }

    /// `entry`'s current status as the JSON a bot sees, without touching `reported_terminal`.
    fn entry_value(entry: &DownloadEntry) -> Value {
        let pct = entry
            .total
            .filter(|t| *t > 0)
            .map(|t| (entry.received.saturating_mul(100) / t).min(100));
        match &entry.status {
            DlStatus::InProgress => json!({
                "status": "in_progress",
                "name": entry.name,
                "note": match pct {
                    Some(p) => format!("Download started: {} ({p}%)", entry.name),
                    None => format!("Download started: {}", entry.name),
                },
            }),
            DlStatus::Done(rel) => {
                let note = format!(
                    "Downloaded {} ({}) to {} in your folder.",
                    entry.name,
                    human_size(entry.received),
                    rel
                );
                json!({"status": "done", "name": entry.name, "path": rel, "note": note})
            }
            DlStatus::Cancelled => {
                let note = format!("The download of {} was stopped: it passed 1 GB.", entry.name);
                json!({"status": "cancelled", "name": entry.name, "note": note})
            }
            DlStatus::Failed(why) => {
                let note = format!("The download of {} failed: {why}", entry.name);
                json!({"status": "failed", "name": entry.name, "note": note})
            }
        }
    }

    /// Current download info, if any, without marking a terminal state consumed: safe to call
    /// any number of times, unlike `report` below. For a page read that's a side effect of
    /// something else (an action's own before/after snapshot, a navigation's result) rather than
    /// the bot's dedicated "did it finish" check, which must still see the terminal state once
    /// itself.
    pub fn peek(&self) -> Option<Value> {
        let shared = self.shared.lock().unwrap_or_else(|e| e.into_inner());
        Some(Self::entry_value(shared.download.as_ref()?))
    }

    /// One line worth telling the bot about its most recent download, if anything changed: once
    /// it's reported as finished, cancelled or failed, it won't be reported again.
    pub fn report(&self) -> Option<Value> {
        let mut shared = self.shared.lock().unwrap_or_else(|e| e.into_inner());
        let entry = shared.download.as_mut()?;
        if entry.reported_terminal {
            return None;
        }
        let value = Self::entry_value(entry);
        if !matches!(entry.status, DlStatus::InProgress) {
            entry.reported_terminal = true;
        }
        Some(value)
    }

    /// The currently open file picker on this session, if any, worth telling the bot about.
    pub fn open_chooser(&self) -> Option<Value> {
        let shared = self.shared.lock().unwrap_or_else(|e| e.into_inner());
        let c = shared.chooser.as_ref()?;
        let accept = if c.accept.is_empty() { "any file".to_string() } else { c.accept.clone() };
        Some(json!({
            "accept": c.accept,
            "multiple": c.multiple,
            "note": format!(
                "A file picker opened (accepts {accept}). Use browser_upload with a file from your folder or a chat attachment."
            ),
        }))
    }

    async fn call(&self, method: &str, params: Value) -> Result<Value, BrowserError> {
        let (tx, rx) = oneshot::channel();
        self.jobs
            .send(Job { method: method.to_string(), params, reply: tx })
            .await
            .map_err(|_| BrowserError::Protocol("download/chooser connection isn't running".into()))?;
        rx.await
            .map_err(|_| BrowserError::Protocol("download/chooser connection dropped the request".into()))?
    }

    /// Answers the currently open chooser with `files`, exactly as the person's own chooser
    /// answer does in `browser_stream.rs`: `DOM.setFileInputFiles` when Chrome shares kwnt's
    /// filesystem, a chunked in-page injection otherwise. Refuses if no chooser is open.
    pub async fn answer_chooser(&self, local: bool, files: Vec<UploadFile>) -> Result<Vec<String>, String> {
        if files.is_empty() {
            return Err("No file was given to answer the picker with.".into());
        }
        let chooser = { self.shared.lock().unwrap_or_else(|e| e.into_inner()).chooser.clone() };
        let Some(chooser) = chooser else {
            return Err("No file picker is open on the page right now.".into());
        };
        let files: Vec<UploadFile> = if chooser.multiple { files } else { files.into_iter().take(1).collect() };
        if !local {
            let total: u64 = files.iter().map(UploadFile::len).sum();
            if total > REMOTE_UPLOAD_CAP_BYTES {
                return Err(
                    "That's more than this browser can take over a remote connection: 100 MB in all. Pick a smaller file.".into(),
                );
            }
        }
        let names: Vec<String> = files.iter().map(|f| f.name.clone()).collect();
        let result = if local {
            self.set_files_local(&chooser, &files).await
        } else {
            self.inject_files_remote(&chooser, &files).await
        };
        result.map_err(|_| "The page moved on before the file could be set.".to_string())?;
        self.shared.lock().unwrap_or_else(|e| e.into_inner()).chooser = None;
        Ok(names)
    }

    async fn set_files_local(&self, chooser: &ChooserInfo, files: &[UploadFile]) -> Result<(), BrowserError> {
        let mut paths = Vec::with_capacity(files.len());
        for f in files {
            match &f.source {
                UploadSource::Path(p) => paths.push(p.to_string_lossy().to_string()),
                UploadSource::Bytes(_) => {
                    return Err(BrowserError::Protocol("local upload needs a path, not bytes".into()))
                }
            }
        }
        self.call("DOM.setFileInputFiles", json!({"files": paths, "backendNodeId": chooser.backend_node_id}))
            .await
            .map(|_| ())
    }

    async fn inject_files_remote(&self, chooser: &ChooserInfo, files: &[UploadFile]) -> Result<(), BrowserError> {
        use base64::Engine;
        let world = self
            .call("Page.createIsolatedWorld", json!({"frameId": chooser.frame_id, "worldName": "kwnt_bot_upload"}))
            .await?;
        let context = world["executionContextId"]
            .as_i64()
            .ok_or_else(|| BrowserError::Protocol("no execution context for the upload".into()))?;
        let resolved = self
            .call("DOM.resolveNode", json!({"backendNodeId": chooser.backend_node_id, "executionContextId": context}))
            .await?;
        let object = resolved["object"]["objectId"]
            .as_str()
            .map(str::to_string)
            .ok_or_else(|| BrowserError::Protocol("couldn't resolve the file input".into()))?;
        let outcome: Result<(), BrowserError> = async {
            for f in files {
                let bytes = match &f.source {
                    UploadSource::Bytes(b) => b,
                    UploadSource::Path(_) => {
                        return Err(BrowserError::Protocol("remote upload needs bytes, not a path".into()))
                    }
                };
                let began = self
                    .call(
                        "Runtime.callFunctionOn",
                        json!({
                            "objectId": object,
                            "functionDeclaration": crate::server::browser_input::UPLOAD_BEGIN_FN,
                            "arguments": [{"value": {"name": f.name, "mime": f.mime, "lastModified": f.last_modified_ms}}],
                            "returnByValue": true,
                        }),
                    )
                    .await?;
                if began["result"]["value"] != Value::Bool(true) {
                    return Err(BrowserError::Protocol("the page refused the upload".into()));
                }
                for chunk in bytes.chunks(UPLOAD_CHUNK_BYTES) {
                    let b64 = base64::engine::general_purpose::STANDARD.encode(chunk);
                    let r = self
                        .call(
                            "Runtime.callFunctionOn",
                            json!({
                                "objectId": object,
                                "functionDeclaration": crate::server::browser_input::UPLOAD_CHUNK_FN,
                                "arguments": [{"value": b64}],
                                "returnByValue": true,
                            }),
                        )
                        .await?;
                    if r["result"]["value"] != Value::Bool(true) {
                        return Err(BrowserError::Protocol("the page refused an upload chunk".into()));
                    }
                }
                let finished = self
                    .call(
                        "Runtime.callFunctionOn",
                        json!({
                            "objectId": object,
                            "functionDeclaration": crate::server::browser_input::UPLOAD_FINISH_FN,
                            "returnByValue": true,
                        }),
                    )
                    .await?;
                if finished["result"]["value"] != Value::Bool(true) {
                    return Err(BrowserError::Protocol("the page refused to finish the upload".into()));
                }
            }
            Ok(())
        }
        .await;
        let _ = self.call("Runtime.releaseObject", json!({"objectId": object})).await;
        outcome
    }
}

async fn run(
    ctrl: BrowserController,
    workspace: PathBuf,
    local: bool,
    browser_context_id: Option<String>,
    staging_root: PathBuf,
    shared: Arc<StdMutex<Shared>>,
    mut jobs: mpsc::Receiver<Job>,
    ready: oneshot::Sender<()>,
) {
    let mut ready = Some(ready);
    let _ = tokio::fs::create_dir_all(&staging_root).await;
    loop {
        sweep_incoming(&staging_root).await;
        let ws_url = match ctrl.get_ws_url().await {
            Ok(u) => u,
            Err(_) => {
                tokio::time::sleep(RETRY_WAIT).await;
                continue;
            }
        };
        let mut ws = match connect_cdp(&ws_url).await {
            Ok(w) => w,
            Err(_) => {
                tokio::time::sleep(RETRY_WAIT).await;
                continue;
            }
        };
        let msg_id = AtomicU64::new(1);
        if setup(&mut ws, &msg_id, &shared, &staging_root, local, browser_context_id.as_deref()).await.is_err() {
            tokio::time::sleep(RETRY_WAIT).await;
            continue;
        }
        if let Some(tx) = ready.take() {
            let _ = tx.send(());
        }
        let mut follow = tokio::time::interval(FOLLOW_EVERY);
        follow.tick().await;
        loop {
            tokio::select! {
                biased;
                job = jobs.recv() => {
                    let Some(job) = job else { return };
                    let result = job_call(&mut ws, &msg_id, &shared, &workspace, local, &job.method, job.params).await;
                    let failed = result.is_err();
                    let _ = job.reply.send(result);
                    if failed { break; }
                }
                msg = ws.next() => {
                    if on_message(msg, &mut ws, &msg_id, &shared, &workspace, local).await.is_err() { break; }
                }
                _ = follow.tick() => {
                    if let Ok(fresh) = ctrl.get_ws_url().await {
                        if fresh != ws_url { break; }
                    }
                }
            }
        }
    }
}

/// `incoming`: the shared staging folder (outside any workspace) every session points
/// `Browser.setDownloadBehavior` at, so the global setting can't single out one bot's folder.
/// `local`: allow downloads there; otherwise deny them but keep hearing about attempts. Also
/// seeds `shared.frames` from this page's own frame tree, for `handle_event` to claim a download
/// only once it sees a `frameId` that's actually this page's.
async fn setup(
    ws: &mut WsStream,
    msg_id: &AtomicU64,
    shared: &Arc<StdMutex<Shared>>,
    incoming: &Path,
    local: bool,
    browser_context_id: Option<&str>,
) -> Result<(), BrowserError> {
    call_raw(ws, msg_id, "Page.enable", json!({})).await?;
    let behavior = download_behavior_params(local, incoming, browser_context_id);
    call_raw(ws, msg_id, "Browser.setDownloadBehavior", behavior).await?;
    call_raw(ws, msg_id, "Page.setInterceptFileChooserDialog", json!({"enabled": true})).await?;
    if let Ok(tree) = call_raw(ws, msg_id, "Page.getFrameTree", json!({})).await {
        let mut frames = HashSet::new();
        collect_frame_ids(&tree["frameTree"], &mut frames);
        shared.lock().unwrap_or_else(|e| e.into_inner()).frames = frames;
    }
    Ok(())
}

/// Builds `Browser.setDownloadBehavior`'s params for the Fix #4 mode switch: a `local` session
/// (one with a filesystem in common with this server) gets `allowAndName` at the shared staging
/// path; every other session (remote Chrome, with none) gets `deny`, kept to `eventsEnabled` so a
/// download attempt is still reported rather than silently vanishing.
fn download_behavior_params(local: bool, incoming: &Path, browser_context_id: Option<&str>) -> Value {
    let mut behavior = json!({
        "behavior": if local { "allowAndName" } else { "deny" },
        "eventsEnabled": true,
    });
    if local {
        behavior["downloadPath"] = json!(incoming.to_string_lossy());
    }
    if let Some(ctx) = browser_context_id {
        behavior["browserContextId"] = json!(ctx);
    }
    behavior
}

/// Recurses through a `Page.getFrameTree` result (`{frame: {id}, childFrames: [...]}`), collecting
/// every frame id belonging to this page.
fn collect_frame_ids(node: &Value, out: &mut HashSet<String>) {
    if let Some(id) = node["frame"]["id"].as_str() {
        out.insert(id.to_string());
    }
    for child in node["childFrames"].as_array().into_iter().flatten() {
        collect_frame_ids(child, out);
    }
}

/// Sends one request and waits only for its own reply, dropping anything else read in the
/// meantime: used for the short setup/follow-up calls this module issues on its own, never for a
/// job a caller is waiting on (see [`job_call`], which dispatches as it goes instead).
async fn call_raw(ws: &mut WsStream, msg_id: &AtomicU64, method: &str, params: Value) -> Result<Value, BrowserError> {
    let id = msg_id.fetch_add(1, Ordering::SeqCst);
    let request = json!({"id": id, "method": method, "params": params});
    ws.send(Message::Text(request.to_string())).await.map_err(|e| BrowserError::Ws(e.to_string()))?;
    tokio::time::timeout(JOB_WAIT, async {
        loop {
            match ws.next().await {
                Some(Ok(Message::Text(txt))) => {
                    let Ok(parsed) = serde_json::from_str::<Value>(&txt) else { continue };
                    if parsed.get("id").and_then(Value::as_u64) != Some(id) {
                        continue;
                    }
                    if let Some(err) = parsed.get("error") {
                        return Err(BrowserError::Protocol(err.to_string()));
                    }
                    return Ok(parsed.get("result").cloned().unwrap_or(Value::Null));
                }
                Some(Ok(Message::Close(_))) | None => {
                    return Err(BrowserError::Protocol("connection closed before a reply".into()))
                }
                Some(Err(e)) => return Err(BrowserError::Ws(e.to_string())),
                _ => continue,
            }
        }
    })
    .await
    .map_err(|_| BrowserError::Timeout)?
}

/// Sends one request on behalf of a caller awaiting its own result, meanwhile dispatching any
/// unsolicited events (downloads, choosers) that arrive while waiting, instead of dropping them.
async fn job_call(
    ws: &mut WsStream,
    msg_id: &AtomicU64,
    shared: &Arc<StdMutex<Shared>>,
    workspace: &Path,
    local: bool,
    method: &str,
    params: Value,
) -> Result<Value, BrowserError> {
    let id = msg_id.fetch_add(1, Ordering::SeqCst);
    let request = json!({"id": id, "method": method, "params": params});
    ws.send(Message::Text(request.to_string())).await.map_err(|e| BrowserError::Ws(e.to_string()))?;
    loop {
        let msg = tokio::time::timeout(JOB_WAIT, ws.next()).await.map_err(|_| BrowserError::Timeout)?;
        if let Some(Ok(Message::Text(txt))) = &msg {
            if let Ok(v) = serde_json::from_str::<Value>(txt) {
                if v.get("id").and_then(Value::as_u64) == Some(id) {
                    if let Some(err) = v.get("error") {
                        return Err(BrowserError::Protocol(err.to_string()));
                    }
                    return Ok(v.get("result").cloned().unwrap_or(Value::Null));
                }
            }
        }
        on_message(msg, ws, msg_id, shared, workspace, local).await?;
    }
}

async fn on_message(
    msg: Option<Result<Message, tokio_tungstenite::tungstenite::Error>>,
    ws: &mut WsStream,
    msg_id: &AtomicU64,
    shared: &Arc<StdMutex<Shared>>,
    workspace: &Path,
    local: bool,
) -> Result<(), BrowserError> {
    match msg {
        Some(Ok(Message::Text(txt))) => {
            let Ok(v) = serde_json::from_str::<Value>(&txt) else { return Ok(()) };
            if let Some(method) = v.get("method").and_then(Value::as_str) {
                let params = v.get("params").cloned().unwrap_or(Value::Null);
                handle_event(ws, msg_id, method, params, shared, workspace, local).await;
            }
            Ok(())
        }
        Some(Ok(Message::Close(_))) | None => Err(BrowserError::Protocol("connection closed".into())),
        Some(Err(e)) => Err(BrowserError::Ws(e.to_string())),
        _ => Ok(()),
    }
}

async fn handle_event(
    ws: &mut WsStream,
    msg_id: &AtomicU64,
    method: &str,
    params: Value,
    shared: &Arc<StdMutex<Shared>>,
    workspace: &Path,
    local: bool,
) {
    match method {
        "Browser.downloadWillBegin" => {
            let Some(guid) = params["guid"].as_str().map(str::to_string) else { return };
            let frame_id = params["frameId"].as_str().unwrap_or_default();
            let name = sanitize_name(params["suggestedFilename"].as_str().unwrap_or("download"));
            let mut s = shared.lock().unwrap_or_else(|e| e.into_inner());
            // Another bot's session may see this event too (see the module doc's accepted gap):
            // only claim it when the frame it started on is actually this page's own.
            if !s.frames.contains(frame_id) {
                return;
            }
            s.download = Some(DownloadEntry {
                guid,
                name,
                total: None,
                received: 0,
                status: if local {
                    DlStatus::InProgress
                } else {
                    DlStatus::Failed(REMOTE_DOWNLOAD_REFUSAL.into())
                },
                reported_terminal: false,
            });
        }
        "Browser.downloadProgress" => {
            // Remote mode already answered `downloadWillBegin` as failed and never allowed the
            // download to start: nothing here is this center's to track.
            if !local {
                return;
            }
            let Some(guid) = params["guid"].as_str().map(str::to_string) else { return };
            let state = params["state"].as_str().unwrap_or_default().to_string();
            let received = params["receivedBytes"].as_u64().unwrap_or(0);
            let total = params["totalBytes"].as_u64().filter(|t| *t > 0);
            let file_path = params["filePath"].as_str().map(str::to_string);
            let mut finish = None;
            let mut over_cap = false;
            {
                let mut s = shared.lock().unwrap_or_else(|e| e.into_inner());
                // `downloadProgress` carries no `frameId`, so unlike `downloadWillBegin` there's
                // nothing here to check frame ownership against. Rather than adopt an unrecognized
                // guid outright (which would hand another bot's progress to this center whenever
                // the setting turns out to be browser-wide), only ever update a guid this center
                // already claimed via a frame-checked `downloadWillBegin`; anything else is ignored.
                let Some(entry) = s.download.as_mut().filter(|e| e.guid == guid) else { return };
                if let Some(t) = total {
                    entry.total = Some(t);
                }
                entry.received = received;
                match state.as_str() {
                    "completed" => match file_path {
                        Some(path) => finish = Some((entry.name.clone(), path)),
                        None => entry.status = DlStatus::Failed("Chrome didn't say where the file landed.".into()),
                    },
                    "canceled" => entry.status = DlStatus::Cancelled,
                    _ if received > MAX_DOWNLOAD_BYTES => over_cap = true,
                    _ => {}
                }
            }
            if over_cap {
                let _ = call_raw(ws, msg_id, "Browser.cancelDownload", json!({"guid": guid})).await;
            }
            if let Some((name, path)) = finish {
                tokio::spawn(land_download(workspace.to_path_buf(), shared.clone(), guid, name, path));
            }
        }
        "Page.fileChooserOpened" => {
            let backend_node_id = params["backendNodeId"].as_i64().unwrap_or_default();
            let frame_id = params["frameId"].as_str().unwrap_or_default().to_string();
            let multiple = params["mode"].as_str() == Some("selectMultiple");
            let accept = describe_accept(ws, msg_id, backend_node_id).await;
            let mut s = shared.lock().unwrap_or_else(|e| e.into_inner());
            s.chooser = Some(ChooserInfo { backend_node_id, frame_id, accept, multiple });
        }
        "Page.frameAttached" => {
            if let Some(id) = params["frameId"].as_str() {
                shared.lock().unwrap_or_else(|e| e.into_inner()).frames.insert(id.to_string());
            }
        }
        "Page.frameNavigated" => {
            let Some(id) = params["frame"]["id"].as_str() else { return };
            let mut s = shared.lock().unwrap_or_else(|e| e.into_inner());
            if params["frame"]["parentId"].is_null() {
                s.chooser = None;
                // A top-level navigation leaves any earlier subframes behind; keep the set from
                // growing forever and start it over with just the new main frame.
                s.frames.clear();
            }
            s.frames.insert(id.to_string());
        }
        _ => {}
    }
}

async fn describe_accept(ws: &mut WsStream, msg_id: &AtomicU64, backend_node_id: i64) -> String {
    let Ok(described) = call_raw(ws, msg_id, "DOM.describeNode", json!({"backendNodeId": backend_node_id})).await
    else {
        return String::new();
    };
    described["node"]["attributes"]
        .as_array()
        .map(Vec::as_slice)
        .unwrap_or_default()
        .chunks(2)
        .find(|pair| pair.first().and_then(Value::as_str) == Some("accept"))
        .and_then(|pair| pair.get(1))
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string()
}

/// Moves a finished download out of staging and into the bot's own `downloads/` folder, picking
/// a collision-free name there (Chrome already made the staged name collision-free on its own
/// side via `allowAndName`, so this is the one place a name actually has to be chosen).
async fn land_download(workspace: PathBuf, shared: Arc<StdMutex<Shared>>, guid: String, name: String, staged_path: String) {
    let landing_guid = guid.clone();
    let result = tokio::task::spawn_blocking(move || -> std::io::Result<String> {
        std::fs::create_dir_all(&workspace)?;
        let root = safe_fs::Dir::open_strict(&workspace)?;
        let dir = root.make_dirs(Path::new("downloads"))?;
        let mut src = std::fs::File::open(&staged_path)?;
        let existing: HashSet<String> = dir
            .list()?
            .into_iter()
            .filter(|e| matches!(e.kind, safe_fs::Kind::File))
            .map(|e| e.name.to_string_lossy().to_string())
            .collect();
        let final_name = collision_safe_name(&existing, &name);
        dir.replace_file(OsStr::new(&final_name), &format!(".landing-{landing_guid}"), &mut src, 0o600)?;
        drop(src);
        let _ = std::fs::remove_file(&staged_path);
        Ok(final_name)
    })
    .await;
    let mut s = shared.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(entry) = &mut s.download {
        if entry.guid == guid {
            entry.status = match result {
                Ok(Ok(final_name)) => DlStatus::Done(format!("downloads/{final_name}")),
                Ok(Err(e)) => DlStatus::Failed(e.to_string()),
                Err(_) => DlStatus::Failed("something went wrong landing the download".into()),
            };
        }
    }
}

/// Deletes any file left in the shared incoming folder older than `INCOMING_ORPHAN_AGE`: a
/// backstop for a staged download no center's frame tree ever claimed, or one `land_download`
/// never got to finish moving before a restart.
async fn sweep_incoming(incoming: &Path) {
    let Ok(mut entries) = tokio::fs::read_dir(incoming).await else { return };
    let cutoff = std::time::SystemTime::now() - INCOMING_ORPHAN_AGE;
    while let Ok(Some(entry)) = entries.next_entry().await {
        if entry.metadata().await.ok().and_then(|m| m.modified().ok()).is_some_and(|m| m < cutoff) {
            let _ = tokio::fs::remove_file(entry.path()).await;
        }
    }
}

fn collision_safe_name(existing: &HashSet<String>, name: &str) -> String {
    if !existing.contains(name) {
        return name.to_string();
    }
    let (stem, ext) = match name.rsplit_once('.') {
        Some((s, e)) if !s.is_empty() => (s.to_string(), Some(e.to_string())),
        _ => (name.to_string(), None),
    };
    for n in 2..10_000u32 {
        let candidate = match &ext {
            Some(e) => format!("{stem} ({n}).{e}"),
            None => format!("{stem} ({n})"),
        };
        if !existing.contains(&candidate) {
            return candidate;
        }
    }
    format!("{name}.{}", uuid::Uuid::new_v4())
}

fn sanitize_name(raw: &str) -> String {
    let base = Path::new(raw).file_name().and_then(|n| n.to_str()).unwrap_or("download");
    let cleaned: String = base.chars().map(|c| if c.is_control() { '_' } else { c }).collect();
    let cleaned = cleaned.trim();
    if cleaned.is_empty() || cleaned == "." || cleaned == ".." {
        "download".to_string()
    } else {
        cleaned.chars().take(200).collect()
    }
}

fn human_size(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    if bytes < 1024 {
        return format!("{bytes} B");
    }
    let mut size = bytes as f64;
    let mut unit = 0;
    while size >= 1024.0 && unit < UNITS.len() - 1 {
        size /= 1024.0;
        unit += 1;
    }
    format!("{size:.1} {}", UNITS[unit])
}

#[cfg(test)]
mod tests {
    use super::*;

    // Fix #4: local sessions (a filesystem in common with this server) get downloads kept at the
    // shared staging path; every other session (remote Chrome) gets them refused up front, since
    // there's nowhere in common to land the bytes.
    #[test]
    fn local_sessions_allow_and_name_at_the_staging_path() {
        let incoming = Path::new("/tmp/kwnt-download-tmp");
        let params = download_behavior_params(true, incoming, None);
        assert_eq!(params["behavior"], "allowAndName");
        assert_eq!(params["eventsEnabled"], true);
        assert_eq!(params["downloadPath"], "/tmp/kwnt-download-tmp");
        assert!(params.get("browserContextId").is_none());
    }

    #[test]
    fn remote_sessions_deny_but_keep_hearing_about_attempts() {
        let incoming = Path::new("/tmp/kwnt-download-tmp");
        let params = download_behavior_params(false, incoming, None);
        assert_eq!(params["behavior"], "deny");
        assert_eq!(params["eventsEnabled"], true);
        assert!(params.get("downloadPath").is_none(), "a remote session has no filesystem in common with this server");
    }

    #[test]
    fn a_non_default_browser_context_is_passed_through_either_way() {
        let incoming = Path::new("/tmp/kwnt-download-tmp");
        let local = download_behavior_params(true, incoming, Some("ctx-123"));
        assert_eq!(local["browserContextId"], "ctx-123");
        let remote = download_behavior_params(false, incoming, Some("ctx-123"));
        assert_eq!(remote["browserContextId"], "ctx-123");
    }
}
