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
//! assumption. Reconnecting to follow a popup polls `get_ws_url()` every few seconds rather than
//! reacting the instant a popup opens, so capture on a brand new popup can lag by that long.

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
    pub fn start(ctrl: BrowserController, workspace: PathBuf) -> Self {
        let shared = Arc::new(StdMutex::new(Shared::default()));
        let (tx, rx) = mpsc::channel(8);
        let center = Self { shared: shared.clone(), jobs: tx };
        tokio::spawn(run(ctrl, workspace, shared, rx));
        center
    }

    /// One line worth telling the bot about its most recent download, if anything changed: once
    /// it's reported as finished, cancelled or failed, it won't be reported again.
    pub fn report(&self) -> Option<Value> {
        let mut shared = self.shared.lock().unwrap_or_else(|e| e.into_inner());
        let entry = shared.download.as_mut()?;
        if entry.reported_terminal {
            return None;
        }
        let pct = entry
            .total
            .filter(|t| *t > 0)
            .map(|t| (entry.received.saturating_mul(100) / t).min(100));
        let value = match &entry.status {
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
                entry.reported_terminal = true;
                json!({"status": "done", "name": entry.name, "path": rel, "note": note})
            }
            DlStatus::Cancelled => {
                let note = format!("The download of {} was stopped: it passed 1 GB.", entry.name);
                entry.reported_terminal = true;
                json!({"status": "cancelled", "name": entry.name, "note": note})
            }
            DlStatus::Failed(why) => {
                let note = format!("The download of {} failed: {why}", entry.name);
                entry.reported_terminal = true;
                json!({"status": "failed", "name": entry.name, "note": note})
            }
        };
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

async fn run(ctrl: BrowserController, workspace: PathBuf, shared: Arc<StdMutex<Shared>>, mut jobs: mpsc::Receiver<Job>) {
    let incoming = workspace.join("downloads").join(".incoming");
    let _ = tokio::fs::create_dir_all(&incoming).await;
    loop {
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
        if setup(&mut ws, &msg_id, &workspace).await.is_err() {
            tokio::time::sleep(RETRY_WAIT).await;
            continue;
        }
        let mut follow = tokio::time::interval(FOLLOW_EVERY);
        follow.tick().await;
        loop {
            tokio::select! {
                biased;
                job = jobs.recv() => {
                    let Some(job) = job else { return };
                    let result = job_call(&mut ws, &msg_id, &shared, &workspace, &job.method, job.params).await;
                    let failed = result.is_err();
                    let _ = job.reply.send(result);
                    if failed { break; }
                }
                msg = ws.next() => {
                    if on_message(msg, &mut ws, &msg_id, &shared, &workspace).await.is_err() { break; }
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

async fn setup(ws: &mut WsStream, msg_id: &AtomicU64, workspace: &Path) -> Result<(), BrowserError> {
    let incoming = workspace.join("downloads").join(".incoming");
    call_raw(ws, msg_id, "Page.enable", json!({})).await?;
    call_raw(
        ws,
        msg_id,
        "Browser.setDownloadBehavior",
        json!({"behavior": "allowAndName", "downloadPath": incoming.to_string_lossy(), "eventsEnabled": true}),
    )
    .await?;
    call_raw(ws, msg_id, "Page.setInterceptFileChooserDialog", json!({"enabled": true})).await?;
    Ok(())
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
        on_message(msg, ws, msg_id, shared, workspace).await?;
    }
}

async fn on_message(
    msg: Option<Result<Message, tokio_tungstenite::tungstenite::Error>>,
    ws: &mut WsStream,
    msg_id: &AtomicU64,
    shared: &Arc<StdMutex<Shared>>,
    workspace: &Path,
) -> Result<(), BrowserError> {
    match msg {
        Some(Ok(Message::Text(txt))) => {
            let Ok(v) = serde_json::from_str::<Value>(&txt) else { return Ok(()) };
            if let Some(method) = v.get("method").and_then(Value::as_str) {
                let params = v.get("params").cloned().unwrap_or(Value::Null);
                handle_event(ws, msg_id, method, params, shared, workspace).await;
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
) {
    match method {
        "Browser.downloadWillBegin" => {
            let Some(guid) = params["guid"].as_str().map(str::to_string) else { return };
            let name = sanitize_name(params["suggestedFilename"].as_str().unwrap_or("download"));
            let mut s = shared.lock().unwrap_or_else(|e| e.into_inner());
            s.download = Some(DownloadEntry {
                guid,
                name,
                total: None,
                received: 0,
                status: DlStatus::InProgress,
                reported_terminal: false,
            });
        }
        "Browser.downloadProgress" => {
            let Some(guid) = params["guid"].as_str().map(str::to_string) else { return };
            let state = params["state"].as_str().unwrap_or_default().to_string();
            let received = params["receivedBytes"].as_u64().unwrap_or(0);
            let total = params["totalBytes"].as_u64().filter(|t| *t > 0);
            let file_path = params["filePath"].as_str().map(str::to_string);
            let mut finish = None;
            let mut over_cap = false;
            {
                let mut s = shared.lock().unwrap_or_else(|e| e.into_inner());
                if !matches!(&s.download, Some(e) if e.guid == guid) {
                    s.download = Some(DownloadEntry {
                        guid: guid.clone(),
                        name: sanitize_name("download"),
                        total,
                        received,
                        status: DlStatus::InProgress,
                        reported_terminal: false,
                    });
                }
                let entry = s.download.as_mut().expect("just set above");
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
        "Page.frameNavigated" => {
            if params["frame"]["parentId"].is_null() {
                shared.lock().unwrap_or_else(|e| e.into_inner()).chooser = None;
            }
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
    let result = tokio::task::spawn_blocking(move || -> std::io::Result<String> {
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
        dir.replace_file(OsStr::new(&final_name), &format!(".landing-{guid}"), &mut src, 0o600)?;
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
