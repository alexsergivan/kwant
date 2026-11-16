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
//! Where Chrome keeps the bytes depends on where it runs ([`DownloadHome`]):
//! - Chrome on this computer (`Local`) gets `allowAndName` at the shared staging path.
//! - kwant's own browser container (`Container`, vm mode with no `KWNT_BROWSER_CDP_URL`) gets
//!   `allowAndName` at [`CONTAINER_DOWNLOAD_DIR`] on the container's own tmpfs, so Chrome names
//!   the file by its download guid there. Once it completes, kwant streams it out with
//!   `docker exec <container> head -c` into the staging folder (`docker cp` can't read a tmpfs mount),
//!   lands it like a local one, and removes the container copy. The tmpfs is 512 MB and Chrome's
//!   own profile lives there too, so these downloads stop at [`CONTAINER_MAX_DOWNLOAD_BYTES`].
//! - A browser on another computer (`Elsewhere`, `KWNT_BROWSER_CDP_URL`) has no filesystem and
//!   no Docker in common with this server: it gets `deny` with `eventsEnabled: true`, and a
//!   download attempt is reported back as failed rather than tracked. kwant never fetches the
//!   URL itself instead (it could reach this server's own ports).

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
/// A browser on another computer has no filesystem in common with this server, so a download
/// can't be kept: say so in words the bot and the person can act on.
const REMOTE_DOWNLOAD_REFUSAL: &str = "This browser runs on another computer, so kwant can't keep its downloads. \
Copy the file's link instead, or switch this bot to Local computer.";
/// Where Chrome in kwant's browser container writes downloads: on the container's own tmpfs,
/// each file named by its download guid (`allowAndName`), never by anything the page chose.
pub const CONTAINER_DOWNLOAD_DIR: &str = "/tmp/kwnt-dl";
/// The container's `/tmp` is a 512 MB tmpfs that Chrome's own profile shares: a download is
/// stopped once it's past this, so it can't fill the browser's disk.
const CONTAINER_MAX_DOWNLOAD_BYTES: u64 = 400 * 1024 * 1024;
const CONTAINER_TOO_BIG: &str =
    "it's over 400 MB, the most this bot's browser can hold. Copy the file's link instead, or switch this bot to Local computer.";
/// The longest one `docker exec` may take: copying a finished download out of the container, or
/// a small chore (making the folder, removing a file).
const CONTAINER_COPY_WAIT: Duration = Duration::from_secs(300);
const CONTAINER_CHORE_WAIT: Duration = Duration::from_secs(15);
/// The image's own non-root user (the one Chrome runs as): every `docker exec` kwant runs in
/// the container uses it, so a future image that defaults to root can't quietly widen them.
const CONTAINER_USER: &str = "browser";

/// Where a session's Chrome keeps a download, and so how it reaches the bot's `downloads/`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DownloadHome {
    /// Chrome on this computer: it writes straight into the shared staging folder.
    Local,
    /// kwant's own browser container, by name: Chrome writes into [`CONTAINER_DOWNLOAD_DIR`] on
    /// the container's tmpfs, and kwant copies the finished file out with Docker.
    Container(String),
    /// A browser kwant was pointed at on another computer (`KWNT_BROWSER_CDP_URL`): no shared
    /// disk, no Docker, so downloads are refused.
    Elsewhere,
}

impl DownloadHome {
    /// The largest download this home can take.
    fn cap(&self) -> u64 {
        match self {
            Self::Container(_) => CONTAINER_MAX_DOWNLOAD_BYTES,
            _ => MAX_DOWNLOAD_BYTES,
        }
    }

    /// Why a download over [`Self::cap`] was stopped, in the bot's own words.
    fn too_big(&self) -> &'static str {
        match self {
            Self::Container(_) => CONTAINER_TOO_BIG,
            _ => "it passed 1 GB.",
        }
    }
}

#[derive(Clone, Debug)]
enum DlStatus {
    InProgress,
    Done(String),
    /// Stopped before it finished, with why.
    Cancelled(String),
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
    /// kwant asked Chrome to cancel it for passing the cap, so the `canceled` that follows says
    /// so (and the cancel isn't sent again on every later progress event).
    too_big: Option<&'static str>,
    /// The container's file is being copied out: a newer download must leave it alone.
    copying: bool,
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
    /// `home`: where this Chrome keeps a download (see the module doc).
    /// `browser_context_id`: the session's own context, for a non-default context (VM / job
    /// pages); `None` for a local page, which shares the kwant profile's default context.
    /// `staging_root`: one folder shared by every session, outside any bot's workspace.
    ///
    /// Waits briefly for the first connect-and-setup round trip (`Page.enable`,
    /// `Browser.setDownloadBehavior`, ...) to land before returning, so that round trip is always
    /// part of whatever request just opened this session, never something a later, unrelated
    /// request races against and appears to have caused. A browser slow or unreachable past that
    /// wait just keeps retrying in the background, same as before this wait existed.
    pub async fn start(ctrl: BrowserController, workspace: PathBuf, home: DownloadHome, browser_context_id: Option<String>, staging_root: PathBuf) -> Self {
        let shared = Arc::new(StdMutex::new(Shared::default()));
        let (tx, rx) = mpsc::channel(8);
        let (ready_tx, ready_rx) = oneshot::channel();
        let center = Self { shared: shared.clone(), jobs: tx };
        let session = Session { workspace, home, browser_context_id, staging_root };
        tokio::spawn(run(ctrl, session, shared, rx, ready_tx));
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
            DlStatus::Cancelled(why) => {
                let note = format!("The download of {} was stopped: {why}", entry.name);
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

/// A session's own, unchanging settings for [`run`], grouped so the function stays under
/// clippy's argument-count lint.
struct Session {
    workspace: PathBuf,
    home: DownloadHome,
    browser_context_id: Option<String>,
    staging_root: PathBuf,
}

/// What `handle_event` needs to land a finished download: borrowed from [`Session`].
struct Landing<'a> {
    workspace: &'a Path,
    home: &'a DownloadHome,
    staging_root: &'a Path,
    browser_context_id: Option<&'a str>,
}

async fn run(
    ctrl: BrowserController,
    session: Session,
    shared: Arc<StdMutex<Shared>>,
    mut jobs: mpsc::Receiver<Job>,
    ready: oneshot::Sender<()>,
) {
    let Session { workspace, home, browser_context_id, staging_root } = session;
    let landing = Landing {
        workspace: &workspace,
        home: &home,
        staging_root: &staging_root,
        browser_context_id: browser_context_id.as_deref(),
    };
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
        if let DownloadHome::Container(name) = &home {
            prepare_container_dir(name).await;
        }
        if setup(&mut ws, &msg_id, &shared, &staging_root, &home, browser_context_id.as_deref()).await.is_err() {
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
                    let result = job_call(&mut ws, &msg_id, &shared, &landing, &job.method, job.params).await;
                    let failed = result.is_err();
                    let _ = job.reply.send(result);
                    if failed { break; }
                }
                msg = ws.next() => {
                    if on_message(msg, &mut ws, &msg_id, &shared, &landing).await.is_err() { break; }
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

/// `incoming`: the shared staging folder (outside any workspace) every local session points
/// `Browser.setDownloadBehavior` at, so the global setting can't single out one bot's folder.
/// `home`: see [`download_behavior_params`]. Also
/// seeds `shared.frames` from this page's own frame tree, for `handle_event` to claim a download
/// only once it sees a `frameId` that's actually this page's.
async fn setup(
    ws: &mut WsStream,
    msg_id: &AtomicU64,
    shared: &Arc<StdMutex<Shared>>,
    incoming: &Path,
    home: &DownloadHome,
    browser_context_id: Option<&str>,
) -> Result<(), BrowserError> {
    call_raw(ws, msg_id, "Page.enable", json!({})).await?;
    let behavior = download_behavior_params(home, incoming, browser_context_id);
    call_raw(ws, msg_id, "Browser.setDownloadBehavior", behavior).await?;
    call_raw(ws, msg_id, "Page.setInterceptFileChooserDialog", json!({"enabled": true})).await?;
    if let Ok(tree) = call_raw(ws, msg_id, "Page.getFrameTree", json!({})).await {
        let mut frames = HashSet::new();
        collect_frame_ids(&tree["frameTree"], &mut frames);
        shared.lock().unwrap_or_else(|e| e.into_inner()).frames = frames;
    }
    Ok(())
}

/// Builds `Browser.setDownloadBehavior`'s params for where this Chrome runs: on this computer it
/// gets `allowAndName` at the shared staging path; in kwant's browser container, `allowAndName`
/// at [`CONTAINER_DOWNLOAD_DIR`] on the container's own tmpfs (the staging path means nothing
/// inside it); on another computer, `deny`, kept to `eventsEnabled` so a download attempt is
/// still reported rather than silently vanishing.
fn download_behavior_params(home: &DownloadHome, incoming: &Path, browser_context_id: Option<&str>) -> Value {
    let download_path = match home {
        DownloadHome::Local => Some(incoming.to_string_lossy().to_string()),
        DownloadHome::Container(_) => Some(CONTAINER_DOWNLOAD_DIR.to_string()),
        DownloadHome::Elsewhere => None,
    };
    let mut behavior = json!({
        "behavior": if download_path.is_some() { "allowAndName" } else { "deny" },
        "eventsEnabled": true,
    });
    if let Some(path) = download_path {
        behavior["downloadPath"] = json!(path);
    }
    if let Some(ctx) = browser_context_id {
        behavior["browserContextId"] = json!(ctx);
    }
    behavior
}

/// `Browser.cancelDownload`'s params. Chrome looks the guid up in one browser context only, so a
/// session in its own context (vm mode) must name it: without it real Chrome answers "No download
/// item found for the given GUID" and the download runs on.
fn cancel_params(guid: &str, browser_context_id: Option<&str>) -> Value {
    let mut params = json!({"guid": guid});
    if let Some(ctx) = browser_context_id {
        params["browserContextId"] = json!(ctx);
    }
    params
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

/// Sends a request from inside event handling without waiting for its reply. Waiting there (as
/// [`call_raw`] does) drops every other message in the meantime: Chrome sends a cancelled
/// download's `canceled` progress event before its `cancelDownload` reply, so the download stayed
/// "in progress" for good, and a reply a [`job_call`] was waiting for could be lost the same way.
/// The reply has no `method`, so [`on_message`] passes over it; a refusal (the download already
/// finished) needs nothing either, since its `completed` event comes through the usual way.
async fn send_only(ws: &mut WsStream, msg_id: &AtomicU64, method: &str, params: Value) {
    let id = msg_id.fetch_add(1, Ordering::SeqCst);
    let request = json!({"id": id, "method": method, "params": params});
    if let Err(e) = ws.send(Message::Text(request.to_string())).await {
        tracing::warn!("browser downloads: couldn't send {method}: {e}");
    }
}

/// Sends one request on behalf of a caller awaiting its own result, meanwhile dispatching any
/// unsolicited events (downloads, choosers) that arrive while waiting, instead of dropping them.
async fn job_call(
    ws: &mut WsStream,
    msg_id: &AtomicU64,
    shared: &Arc<StdMutex<Shared>>,
    landing: &Landing<'_>,
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
        on_message(msg, ws, msg_id, shared, landing).await?;
    }
}

async fn on_message(
    msg: Option<Result<Message, tokio_tungstenite::tungstenite::Error>>,
    ws: &mut WsStream,
    msg_id: &AtomicU64,
    shared: &Arc<StdMutex<Shared>>,
    landing: &Landing<'_>,
) -> Result<(), BrowserError> {
    match msg {
        Some(Ok(Message::Text(txt))) => {
            let Ok(v) = serde_json::from_str::<Value>(&txt) else { return Ok(()) };
            if let Some(method) = v.get("method").and_then(Value::as_str) {
                let params = v.get("params").cloned().unwrap_or(Value::Null);
                handle_event(ws, msg_id, method, params, shared, landing).await;
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
    landing: &Landing<'_>,
) {
    let home = landing.home;
    match method {
        "Browser.downloadWillBegin" => {
            let Some(guid) = params["guid"].as_str().map(str::to_string) else { return };
            let frame_id = params["frameId"].as_str().unwrap_or_default();
            let name = sanitize_name(params["suggestedFilename"].as_str().unwrap_or("download"));
            let replaced = {
                let mut s = shared.lock().unwrap_or_else(|e| e.into_inner());
                // Another bot's session may see this event too (see the module doc's accepted gap):
                // only claim it when the frame it started on is actually this page's own.
                if !s.frames.contains(frame_id) {
                    return;
                }
                // This center tracks one download at a time. In the container an earlier one still
                // running would go on filling the shared tmpfs with nobody left to cap, land or remove
                // it, so it is stopped and removed as the new one takes its place.
                let replaced = match (home, s.download.as_ref()) {
                    (DownloadHome::Container(_), Some(prev))
                        if prev.guid != guid && matches!(prev.status, DlStatus::InProgress) && !prev.copying && plain_guid(&prev.guid) =>
                    {
                        Some(prev.guid.clone())
                    }
                    _ => None,
                };
                let status = match home {
                    DownloadHome::Elsewhere => DlStatus::Failed(REMOTE_DOWNLOAD_REFUSAL.into()),
                    // the guid names the file inside the container: never let it be a path
                    DownloadHome::Container(_) if !plain_guid(&guid) => {
                        DlStatus::Failed("the browser gave the download a name kwant can't use.".into())
                    }
                    _ => DlStatus::InProgress,
                };
                s.download = Some(DownloadEntry {
                    guid,
                    name,
                    total: None,
                    received: 0,
                    status,
                    reported_terminal: false,
                    too_big: None,
                    copying: false,
                });
                replaced
            };
            if let (DownloadHome::Container(container), Some(prev)) = (home, replaced) {
                send_only(ws, msg_id, "Browser.cancelDownload", cancel_params(&prev, landing.browser_context_id)).await;
                tokio::spawn(remove_from_container(container.clone(), prev));
            }
        }
        "Browser.downloadProgress" => {
            // A browser on another computer already answered `downloadWillBegin` as failed and
            // never allowed the download to start: nothing here is this center's to track.
            if *home == DownloadHome::Elsewhere {
                return;
            }
            let Some(guid) = params["guid"].as_str().map(str::to_string) else { return };
            let state = params["state"].as_str().unwrap_or_default().to_string();
            let received = params["receivedBytes"].as_u64().unwrap_or(0);
            let total = params["totalBytes"].as_u64().filter(|t| *t > 0);
            let file_path = params["filePath"].as_str().map(str::to_string);
            let cap = home.cap();
            let mut finish = None;
            let mut over_cap = false;
            let mut clear_container = false;
            {
                let mut s = shared.lock().unwrap_or_else(|e| e.into_inner());
                // `downloadProgress` carries no `frameId`, so unlike `downloadWillBegin` there's
                // nothing here to check frame ownership against. Rather than adopt an unrecognized
                // guid outright (which would hand another bot's progress to this center whenever
                // the setting turns out to be browser-wide), only ever update a guid this center
                // already claimed via a frame-checked `downloadWillBegin`; anything else is ignored.
                let Some(entry) = s.download.as_mut().filter(|e| e.guid == guid) else { return };
                if !matches!(entry.status, DlStatus::InProgress) {
                    // refused at the start (an unusable guid): stop it, and leave nothing behind
                    over_cap = state == "inProgress" && matches!(entry.status, DlStatus::Failed(_)) && entry.too_big.is_none();
                    if over_cap {
                        entry.too_big = Some("the browser gave the download a name kwant can't use.");
                    }
                    clear_container = matches!(state.as_str(), "completed" | "canceled");
                } else {
                    if let Some(t) = total {
                        entry.total = Some(t);
                    }
                    entry.received = received;
                    match state.as_str() {
                        "completed" => match (home, file_path) {
                            // it can finish between two progress events, past the cap: never copy that out
                            (DownloadHome::Container(_), _) if entry.too_big.is_some() || received.max(total.unwrap_or(0)) > cap => {
                                entry.status = DlStatus::Cancelled(entry.too_big.unwrap_or(home.too_big()).to_string());
                                clear_container = true;
                            }
                            // the container's file is the guid kwant checked, whatever path Chrome reports
                            (DownloadHome::Container(_), _) => {
                                entry.copying = true;
                                finish = Some((entry.name.clone(), String::new()));
                            }
                            (_, Some(path)) => finish = Some((entry.name.clone(), path)),
                            (_, None) => entry.status = DlStatus::Failed("Chrome didn't say where the file landed.".into()),
                        },
                        "canceled" => {
                            entry.status = DlStatus::Cancelled(entry.too_big.unwrap_or("the browser stopped it.").to_string());
                            clear_container = true;
                        }
                        // a size the server announced up front is enough to stop it before it fills anything
                        _ if entry.too_big.is_none() && (received > cap || total.is_some_and(|t| t > cap)) => {
                            entry.too_big = Some(home.too_big());
                            over_cap = true;
                        }
                        _ => {}
                    }
                }
            }
            if over_cap {
                send_only(ws, msg_id, "Browser.cancelDownload", cancel_params(&guid, landing.browser_context_id)).await;
            }
            match (home, finish) {
                (DownloadHome::Container(container), Some((name, _))) => {
                    tokio::spawn(land_from_container(
                        container.clone(),
                        landing.workspace.to_path_buf(),
                        landing.staging_root.to_path_buf(),
                        shared.clone(),
                        guid,
                        name,
                    ));
                }
                (_, Some((name, path))) => {
                    tokio::spawn(land_download(landing.workspace.to_path_buf(), shared.clone(), guid, name, path));
                }
                (DownloadHome::Container(container), None) if clear_container && plain_guid(&guid) => {
                    tokio::spawn(remove_from_container(container.clone(), guid));
                }
                _ => {}
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
    let result = tokio::task::spawn_blocking(move || land_staged(&workspace, &landing_guid, &name, Path::new(&staged_path)))
        .await
        .unwrap_or_else(|_| Err(std::io::Error::other("something went wrong landing the download")));
    finish_landing(&shared, &guid, result);
}

/// The blocking part of landing: `staged` (a file in the staging folder) becomes
/// `downloads/<name>` in `workspace`, through [`safe_fs`], and the staged copy is removed.
fn land_staged(workspace: &Path, guid: &str, name: &str, staged: &Path) -> std::io::Result<String> {
    std::fs::create_dir_all(workspace)?;
    let root = safe_fs::Dir::open_strict(workspace)?;
    let dir = root.make_dirs(Path::new("downloads"))?;
    let mut src = std::fs::File::open(staged)?;
    let existing: HashSet<String> = dir
        .list()?
        .into_iter()
        .filter(|e| matches!(e.kind, safe_fs::Kind::File))
        .map(|e| e.name.to_string_lossy().to_string())
        .collect();
    let final_name = collision_safe_name(&existing, name);
    dir.replace_file(OsStr::new(&final_name), &format!(".landing-{guid}"), &mut src, 0o600)?;
    drop(src);
    let _ = std::fs::remove_file(staged);
    Ok(final_name)
}

fn finish_landing(shared: &Arc<StdMutex<Shared>>, guid: &str, result: std::io::Result<String>) {
    let mut s = shared.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(entry) = &mut s.download {
        if entry.guid == guid {
            entry.status = match result {
                Ok(final_name) => DlStatus::Done(format!("downloads/{final_name}")),
                Err(e) => DlStatus::Failed(e.to_string()),
            };
        }
    }
}

/// Chrome names a download by a guid it makes up (a UUID); that guid is then a file name inside
/// the container and in staging, so anything else (a separator, `..`, an empty string) is refused.
fn plain_guid(guid: &str) -> bool {
    !guid.is_empty() && guid.len() <= 64 && guid.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
}

/// The download's file inside the container, and the partial file Chrome may keep beside it.
fn container_paths(guid: &str) -> [String; 2] {
    [format!("{CONTAINER_DOWNLOAD_DIR}/{guid}"), format!("{CONTAINER_DOWNLOAD_DIR}/{guid}.crdownload")]
}

/// One small `docker` chore (making the folder, removing a file) through the browser service's
/// own Docker helper, with a time limit.
async fn docker_chore(args: &[&str]) -> Result<(), String> {
    super::service::BrowserService::docker(args, CONTAINER_CHORE_WAIT.as_secs()).await.map(|_| ()).map_err(|e| e.to_string())
}

/// Makes sure the container has its download folder (its tmpfs starts empty after every
/// restart), and sweeps anything an interrupted landing left there over an hour ago. It runs
/// when a session connects or reconnects, so the sweep is a backstop, not a timer.
async fn prepare_container_dir(container: &str) {
    if let Err(e) = docker_chore(&["exec", "-u", CONTAINER_USER, container, "mkdir", "-p", CONTAINER_DOWNLOAD_DIR]).await {
        tracing::warn!("browser downloads: couldn't make {CONTAINER_DOWNLOAD_DIR} in {container}: {e}");
        return;
    }
    let _ = docker_chore(&["exec", "-u", CONTAINER_USER, container, "find", CONTAINER_DOWNLOAD_DIR, "-mindepth", "1", "-mmin", "+60", "-delete"]).await;
}

/// Removes a download's file (and any partial file) from the container's tmpfs.
async fn remove_from_container(container: String, guid: String) {
    let [file, partial] = container_paths(&guid);
    if let Err(e) = docker_chore(&["exec", "-u", CONTAINER_USER, &container, "rm", "-f", "--", &file, &partial]).await {
        tracing::warn!("browser downloads: couldn't remove {file} from {container}: {e}");
    }
}

/// Copies `guid`'s finished file out of the container into `staging/<guid>`, streaming it
/// straight to disk (`docker cp` can't read a tmpfs mount), and refuses one past the cap.
async fn copy_out_of_container(container: &str, staging: &Path, guid: &str) -> Result<PathBuf, String> {
    tokio::fs::create_dir_all(staging).await.map_err(|e| e.to_string())?;
    let staged = staging.join(guid);
    let _ = tokio::fs::remove_file(&staged).await;
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let file = options.open(&staged).map_err(|e| e.to_string())?;
    let [source, _] = container_paths(guid);
    // `head -c`, not `cat`: whatever sits at that path (a symlink to /dev/zero, say), at most one
    // byte past the cap ever reaches this disk, and the size check below then refuses it.
    let limit = (CONTAINER_MAX_DOWNLOAD_BYTES + 1).to_string();
    let mut command = tokio::process::Command::new("docker");
    command.args(["exec", "-u", CONTAINER_USER, container, "head", "-c", &limit, "--", &source]);
    let run = run_into_file(command, file);
    let outcome = match tokio::time::timeout(CONTAINER_COPY_WAIT, run).await {
        Ok(Ok(out)) if out.status.success() => match tokio::fs::metadata(&staged).await {
            Ok(meta) if meta.len() > CONTAINER_MAX_DOWNLOAD_BYTES => Err(CONTAINER_TOO_BIG.to_string()),
            Ok(_) => Ok(()),
            Err(e) => Err(e.to_string()),
        },
        Ok(Ok(out)) => Err(format!("couldn't copy it out of the browser: {}", String::from_utf8_lossy(&out.stderr).trim())),
        Ok(Err(e)) => Err(format!("couldn't copy it out of the browser: {e}")),
        Err(_) => Err("copying it out of the browser took too long.".into()),
    };
    match outcome {
        Ok(()) => Ok(staged),
        Err(why) => {
            let _ = tokio::fs::remove_file(&staged).await;
            Err(why)
        }
    }
}

/// Runs `command` with its stdout written straight into `file` and its stderr captured.
/// `Command::output()` can't be used here: tokio's replaces any stdout set beforehand with a
/// pipe, so the file stayed empty and the whole download sat in memory instead. The child is
/// killed if this future is dropped (the copy-out's time limit).
async fn run_into_file(mut command: tokio::process::Command, file: std::fs::File) -> std::io::Result<std::process::Output> {
    command
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::from(file))
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true)
        .spawn()?
        .wait_with_output()
        .await
}

/// A download finished inside kwant's browser container: copy it out into staging, land it in
/// the bot's `downloads/` the same way as a local one, and leave nothing behind in the container
/// or in staging whatever happens.
async fn land_from_container(
    container: String,
    workspace: PathBuf,
    staging: PathBuf,
    shared: Arc<StdMutex<Shared>>,
    guid: String,
    name: String,
) {
    let copied = copy_out_of_container(&container, &staging, &guid).await;
    remove_from_container(container, guid.clone()).await;
    let result = match copied {
        Ok(staged) => {
            let landing_guid = guid.clone();
            let landed = tokio::task::spawn_blocking(move || {
                let landed = land_staged(&workspace, &landing_guid, &name, &staged);
                if landed.is_err() {
                    let _ = std::fs::remove_file(&staged);
                }
                landed
            })
            .await;
            landed.unwrap_or_else(|_| Err(std::io::Error::other("something went wrong landing the download")))
        }
        Err(why) => Err(std::io::Error::other(why)),
    };
    finish_landing(&shared, &guid, result);
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

    // The copy-out streams the container file to disk: stdout goes into the staged file, not a
    // pipe (73f24f58 used `output()`, which landed an empty file in the real-Chromium run).
    #[tokio::test]
    async fn the_copy_out_writes_its_bytes_into_the_staged_file() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("source");
        let bytes: Vec<u8> = (0..300_000u32).map(|i| (i % 251) as u8).collect();
        std::fs::write(&source, &bytes).unwrap();
        let staged = dir.path().join("staged");
        let file = std::fs::File::create(&staged).unwrap();
        let mut command = tokio::process::Command::new("head");
        command.args(["-c", "300001", "--"]).arg(&source);
        let out = run_into_file(command, file).await.unwrap();
        assert!(out.status.success());
        assert!(out.stdout.is_empty(), "nothing is held in memory");
        assert_eq!(std::fs::read(&staged).unwrap(), bytes);

        // past the limit, `head -c` stops one byte over, which the size check then refuses
        let file = std::fs::File::create(&staged).unwrap();
        let mut command = tokio::process::Command::new("head");
        command.args(["-c", "1000", "--"]).arg(&source);
        run_into_file(command, file).await.unwrap();
        assert_eq!(std::fs::metadata(&staged).unwrap().len(), 1000);

        // a missing file is a failure with its reason on stderr
        let file = std::fs::File::create(&staged).unwrap();
        let mut command = tokio::process::Command::new("head");
        command.args(["-c", "10", "--"]).arg(dir.path().join("missing"));
        let out = run_into_file(command, file).await.unwrap();
        assert!(!out.status.success());
        assert!(!out.stderr.is_empty());
    }

    // Fix #4: local sessions (a filesystem in common with this server) get downloads kept at the
    // shared staging path; a browser on another computer gets them refused up front, since
    // there's nowhere in common to land the bytes.
    #[test]
    fn local_sessions_allow_and_name_at_the_staging_path() {
        let incoming = Path::new("/tmp/kwnt-download-tmp");
        let params = download_behavior_params(&DownloadHome::Local, incoming, None);
        assert_eq!(params["behavior"], "allowAndName");
        assert_eq!(params["eventsEnabled"], true);
        assert_eq!(params["downloadPath"], "/tmp/kwnt-download-tmp");
        assert!(params.get("browserContextId").is_none());
    }

    // 20g: kwant's own browser container keeps the download on its own tmpfs, never at a host
    // path (which means nothing inside the container).
    #[test]
    fn the_managed_container_allows_and_names_on_its_own_tmpfs() {
        let incoming = Path::new("/home/kwnt/attachments/download-tmp");
        let params = download_behavior_params(&DownloadHome::Container("kwnt-browser-x".into()), incoming, Some("ctx-9"));
        assert_eq!(params["behavior"], "allowAndName");
        assert_eq!(params["eventsEnabled"], true);
        assert_eq!(params["downloadPath"], CONTAINER_DOWNLOAD_DIR);
        assert_eq!(params["browserContextId"], "ctx-9");
        assert!(CONTAINER_DOWNLOAD_DIR.starts_with("/tmp/"), "only /tmp is writable in the read-only container");
    }

    #[test]
    fn a_browser_on_another_computer_denies_but_keeps_hearing_about_attempts() {
        let incoming = Path::new("/tmp/kwnt-download-tmp");
        let params = download_behavior_params(&DownloadHome::Elsewhere, incoming, None);
        assert_eq!(params["behavior"], "deny");
        assert_eq!(params["eventsEnabled"], true);
        assert!(params.get("downloadPath").is_none(), "a remote session has no filesystem in common with this server");
    }

    #[test]
    fn a_non_default_browser_context_is_passed_through_every_way() {
        let incoming = Path::new("/tmp/kwnt-download-tmp");
        for home in [DownloadHome::Local, DownloadHome::Container("c".into()), DownloadHome::Elsewhere] {
            assert_eq!(download_behavior_params(&home, incoming, Some("ctx-123"))["browserContextId"], "ctx-123", "{home:?}");
        }
    }

    #[test]
    fn a_cancel_names_the_sessions_own_browser_context() {
        assert_eq!(cancel_params("g", Some("ctx-1")), json!({"guid": "g", "browserContextId": "ctx-1"}));
        assert_eq!(cancel_params("g", None), json!({"guid": "g"}));
    }

    #[test]
    fn the_container_cap_leaves_room_on_its_512_mb_tmpfs() {
        let home = DownloadHome::Container("c".into());
        assert!(home.cap() <= 400 * 1024 * 1024);
        assert_eq!(DownloadHome::Local.cap(), MAX_DOWNLOAD_BYTES);
        assert!(home.too_big().contains("400 MB"), "{}", home.too_big());
        assert!(DownloadHome::Local.too_big().contains("1 GB"));
    }

    #[test]
    fn the_refusal_for_another_computer_says_what_to_do_instead() {
        assert!(!REMOTE_DOWNLOAD_REFUSAL.contains("for now"));
        assert!(REMOTE_DOWNLOAD_REFUSAL.contains("another computer"));
        assert!(REMOTE_DOWNLOAD_REFUSAL.contains("Copy the file's link"));
        assert!(REMOTE_DOWNLOAD_REFUSAL.contains("Local computer"));
    }

    // The guid becomes a file name inside the container and in staging: only Chrome's own
    // UUID shape is accepted, never a path.
    #[test]
    fn only_a_plain_guid_names_a_file() {
        assert!(plain_guid("0b9d3c2e-6a1f-4c7e-9a52-3f1e2d4c5b6a"));
        let long = "a".repeat(65);
        for bad in ["", "../etc/passwd", "a/b", "..", "x y", "a\\b", long.as_str()] {
            assert!(!plain_guid(bad), "{bad:?}");
        }
        let [file, partial] = container_paths("abc-1");
        assert_eq!(file, "/tmp/kwnt-dl/abc-1");
        assert_eq!(partial, "/tmp/kwnt-dl/abc-1.crdownload");
    }

    fn container_landing<'a>(home: &'a DownloadHome, dir: &'a Path) -> Landing<'a> {
        Landing { workspace: dir, home, staging_root: dir, browser_context_id: Some("ctx-1") }
    }

    fn claimed() -> Arc<StdMutex<Shared>> {
        let shared = Arc::new(StdMutex::new(Shared::default()));
        shared.lock().unwrap().frames.insert("F1".into());
        shared
    }

    /// Feeds one event through `handle_event` against a stand-in browser that answers every
    /// request with an empty result; returns the requests it was sent (a cancel, say).
    async fn feed(shared: &Arc<StdMutex<Shared>>, landing: &Landing<'_>, method: &str, params: Value) -> Vec<Value> {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let seen = Arc::new(StdMutex::new(Vec::new()));
        let log = seen.clone();
        let server = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let mut ws = tokio_tungstenite::accept_async(stream).await.unwrap();
            while let Some(Ok(Message::Text(txt))) = ws.next().await {
                let v: Value = serde_json::from_str(&txt).unwrap();
                log.lock().unwrap().push(v.clone());
                let _ = ws.send(Message::Text(json!({"id": v["id"], "result": {}}).to_string())).await;
            }
        });
        let mut ws = connect_cdp(&format!("ws://{addr}")).await.unwrap();
        handle_event(&mut ws, &AtomicU64::new(1), method, params, shared, landing).await;
        // a cancel is sent without waiting for its reply: let the stand-in read it before closing
        let _ = ws.close(None).await;
        let _ = tokio::time::timeout(Duration::from_secs(5), server).await;
        let sent = seen.lock().unwrap().clone();
        sent
    }

    // Chrome's own order (seen in the real-Chromium run): the `canceled` event comes before the
    // `cancelDownload` reply. Both reach a pending job's loop, the download ends as too big, and
    // the job still gets its own reply.
    #[tokio::test]
    async fn a_cancel_mid_job_keeps_chromes_canceled_event_and_the_jobs_reply() {
        let dir = tempfile::tempdir().unwrap();
        let home = DownloadHome::Container("kwnt-browser-unit-test-none".into());
        let shared = claimed();
        let landing = container_landing(&home, dir.path());
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let big = 450u64 * 1024 * 1024;
        let server = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let mut ws = tokio_tungstenite::accept_async(stream).await.unwrap();
            let mut job_id = Value::Null;
            while let Some(Ok(Message::Text(txt))) = ws.next().await {
                let v: Value = serde_json::from_str(&txt).unwrap();
                let event = |method: &str, params: Value| Message::Text(json!({"method": method, "params": params}).to_string());
                match v["method"].as_str() {
                    Some("Browser.cancelDownload") => {
                        ws.send(event("Browser.downloadProgress", json!({"guid": "g9", "state": "canceled", "receivedBytes": 4096, "totalBytes": big}))).await.unwrap();
                        ws.send(Message::Text(json!({"id": v["id"], "result": {}}).to_string())).await.unwrap();
                        ws.send(Message::Text(json!({"id": job_id, "result": {"done": true}}).to_string())).await.unwrap();
                    }
                    _ => {
                        job_id = v["id"].clone();
                        ws.send(event("Browser.downloadWillBegin", json!({"guid": "g9", "frameId": "F1", "suggestedFilename": "big.mp4"}))).await.unwrap();
                        ws.send(event("Browser.downloadProgress", json!({"guid": "g9", "state": "inProgress", "receivedBytes": 0, "totalBytes": big}))).await.unwrap();
                    }
                }
            }
        });
        let mut ws = connect_cdp(&format!("ws://{addr}")).await.unwrap();
        let result = job_call(&mut ws, &AtomicU64::new(1), &shared, &landing, "Runtime.evaluate", json!({})).await;
        assert_eq!(result.unwrap(), json!({"done": true}), "the job's own reply is not lost");
        let entry = shared.lock().unwrap().download.clone().unwrap();
        assert!(matches!(&entry.status, DlStatus::Cancelled(why) if why.contains("over 400 MB")), "{:?}", entry.status);
        server.abort();
    }

    #[tokio::test]
    async fn another_computers_browser_refuses_with_the_clearer_message() {
        let dir = tempfile::tempdir().unwrap();
        let home = DownloadHome::Elsewhere;
        let shared = claimed();
        let landing = container_landing(&home, dir.path());
        feed(&shared, &landing, "Browser.downloadWillBegin", json!({"guid": "g1", "frameId": "F1", "suggestedFilename": "clip.mp4"})).await;
        let center = DownloadCenter { shared: shared.clone(), jobs: mpsc::channel(1).0 };
        let report = center.report().unwrap();
        assert_eq!(report["status"], "failed");
        assert!(report["note"].as_str().unwrap().contains("runs on another computer"), "{report}");
    }

    #[tokio::test]
    async fn a_container_download_announced_past_the_cap_is_stopped_as_too_big() {
        let dir = tempfile::tempdir().unwrap();
        let home = DownloadHome::Container("kwnt-browser-unit-test-none".into());
        let shared = claimed();
        let landing = container_landing(&home, dir.path());
        feed(&shared, &landing, "Browser.downloadWillBegin", json!({"guid": "g2", "frameId": "F1", "suggestedFilename": "big.mp4"})).await;
        let sent = feed(&shared, &landing, "Browser.downloadProgress", json!({"guid": "g2", "state": "inProgress", "receivedBytes": 1024, "totalBytes": 450 * 1024 * 1024})).await;
        assert!(shared.lock().unwrap().download.as_ref().unwrap().too_big.is_some(), "cancel was asked for on the announced size");
        assert_eq!(sent.len(), 1, "{sent:?}");
        assert_eq!(sent[0]["method"], "Browser.cancelDownload");
        assert_eq!(sent[0]["params"], json!({"guid": "g2", "browserContextId": "ctx-1"}));
        // a later progress event doesn't ask again
        let sent = feed(&shared, &landing, "Browser.downloadProgress", json!({"guid": "g2", "state": "inProgress", "receivedBytes": 4096, "totalBytes": 450 * 1024 * 1024})).await;
        assert!(sent.is_empty(), "{sent:?}");
        feed(&shared, &landing, "Browser.downloadProgress", json!({"guid": "g2", "state": "canceled", "receivedBytes": 2048, "totalBytes": 450 * 1024 * 1024})).await;
        let center = DownloadCenter { shared: shared.clone(), jobs: mpsc::channel(1).0 };
        let report = center.report().unwrap();
        assert_eq!(report["status"], "cancelled");
        assert!(report["note"].as_str().unwrap().contains("over 400 MB"), "{report}");
    }

    #[tokio::test]
    async fn a_container_download_that_finishes_past_the_cap_is_never_copied_out() {
        let dir = tempfile::tempdir().unwrap();
        let home = DownloadHome::Container("kwnt-browser-unit-test-none".into());
        let shared = claimed();
        let landing = container_landing(&home, dir.path());
        feed(&shared, &landing, "Browser.downloadWillBegin", json!({"guid": "g5", "frameId": "F1", "suggestedFilename": "big.bin"})).await;
        // no size announced, and it finished before a progress event could pass the cap
        feed(&shared, &landing, "Browser.downloadProgress", json!({"guid": "g5", "state": "completed", "receivedBytes": 401 * 1024 * 1024})).await;
        let entry = shared.lock().unwrap().download.clone().unwrap();
        assert!(!entry.copying, "no copy-out was started");
        assert!(matches!(&entry.status, DlStatus::Cancelled(why) if why.contains("over 400 MB")), "{:?}", entry.status);
    }

    #[tokio::test]
    async fn a_container_download_at_the_cap_is_still_copied_out() {
        let dir = tempfile::tempdir().unwrap();
        let home = DownloadHome::Container("kwnt-browser-unit-test-none".into());
        let shared = claimed();
        let landing = container_landing(&home, dir.path());
        feed(&shared, &landing, "Browser.downloadWillBegin", json!({"guid": "g6", "frameId": "F1", "suggestedFilename": "ok.bin"})).await;
        feed(&shared, &landing, "Browser.downloadProgress", json!({"guid": "g6", "state": "completed", "receivedBytes": CONTAINER_MAX_DOWNLOAD_BYTES})).await;
        assert!(shared.lock().unwrap().download.as_ref().unwrap().copying, "the copy-out started");
    }

    #[tokio::test]
    async fn a_second_container_download_stops_the_first() {
        let dir = tempfile::tempdir().unwrap();
        let home = DownloadHome::Container("kwnt-browser-unit-test-none".into());
        let shared = claimed();
        let landing = container_landing(&home, dir.path());
        feed(&shared, &landing, "Browser.downloadWillBegin", json!({"guid": "g1", "frameId": "F1", "suggestedFilename": "a.bin"})).await;
        let sent = feed(&shared, &landing, "Browser.downloadWillBegin", json!({"guid": "g7", "frameId": "F1", "suggestedFilename": "b.bin"})).await;
        assert_eq!(sent.len(), 1, "{sent:?}");
        assert_eq!(sent[0]["method"], "Browser.cancelDownload");
        assert_eq!(sent[0]["params"], json!({"guid": "g1", "browserContextId": "ctx-1"}));
        assert_eq!(shared.lock().unwrap().download.as_ref().unwrap().guid, "g7");
    }

    #[tokio::test]
    async fn a_second_container_download_leaves_one_being_copied_out_alone() {
        let dir = tempfile::tempdir().unwrap();
        let home = DownloadHome::Container("kwnt-browser-unit-test-none".into());
        let shared = claimed();
        let landing = container_landing(&home, dir.path());
        feed(&shared, &landing, "Browser.downloadWillBegin", json!({"guid": "g8", "frameId": "F1", "suggestedFilename": "a.bin"})).await;
        shared.lock().unwrap().download.as_mut().unwrap().copying = true;
        let sent = feed(&shared, &landing, "Browser.downloadWillBegin", json!({"guid": "g9", "frameId": "F1", "suggestedFilename": "b.bin"})).await;
        assert!(sent.is_empty(), "{sent:?}");
    }

    #[tokio::test]
    async fn a_second_local_download_sends_no_cancel() {
        let dir = tempfile::tempdir().unwrap();
        let home = DownloadHome::Local;
        let shared = claimed();
        let landing = container_landing(&home, dir.path());
        feed(&shared, &landing, "Browser.downloadWillBegin", json!({"guid": "l1", "frameId": "F1", "suggestedFilename": "a.bin"})).await;
        let sent = feed(&shared, &landing, "Browser.downloadWillBegin", json!({"guid": "l2", "frameId": "F1", "suggestedFilename": "b.bin"})).await;
        assert!(sent.is_empty(), "{sent:?}");
    }

    #[tokio::test]
    async fn a_container_download_from_another_bots_frame_is_never_claimed() {
        let dir = tempfile::tempdir().unwrap();
        let home = DownloadHome::Container("kwnt-browser-unit-test-none".into());
        let shared = claimed();
        let landing = container_landing(&home, dir.path());
        feed(&shared, &landing, "Browser.downloadWillBegin", json!({"guid": "g3", "frameId": "OTHER", "suggestedFilename": "x.txt"})).await;
        feed(&shared, &landing, "Browser.downloadProgress", json!({"guid": "g3", "state": "completed", "receivedBytes": 3, "totalBytes": 3})).await;
        assert!(shared.lock().unwrap().download.is_none());
    }

    #[tokio::test]
    async fn a_container_download_with_an_unusable_guid_fails_and_is_cancelled() {
        let dir = tempfile::tempdir().unwrap();
        let home = DownloadHome::Container("kwnt-browser-unit-test-none".into());
        let shared = claimed();
        let landing = container_landing(&home, dir.path());
        feed(&shared, &landing, "Browser.downloadWillBegin", json!({"guid": "../x", "frameId": "F1", "suggestedFilename": "x.txt"})).await;
        let sent = feed(&shared, &landing, "Browser.downloadProgress", json!({"guid": "../x", "state": "inProgress", "receivedBytes": 3})).await;
        assert_eq!(sent.first().map(|v| v["method"].clone()), Some(json!("Browser.cancelDownload")));
        let s = shared.lock().unwrap();
        let entry = s.download.as_ref().unwrap();
        assert!(matches!(entry.status, DlStatus::Failed(_)));
        assert!(entry.too_big.is_some(), "the download was cancelled");
    }
}
