//! The recorder on the Take over stage (api.md §1, §5). Only the stream socket that holds Take
//! over on a stage is linked here, so a recording can only start while the person controls the
//! browser, and the bot's own navigation waits behind that same lease. The socket installs the
//! recorder in kwnt's isolated world ([`super::capture::WORLD`]) when a recording starts and takes
//! it off when it stops; this side turns what arrives into steps and frames for the person.
//!
//! Every stop first takes the field edit still pending in the page (typed but not yet committed
//! by leaving the field, Enter or a click) as its `type` step, masked by the same rules
//! ([`Act::Flush`]); a page that can't be reached is skipped. The routes, the sweep and another
//! window ask the linked socket for that with [`Cmd::Stop`] and wait a moment ([`stop_live`]).
//!
//! A stage runs one bot's recording at a time. Hand back (the person releasing Take over) stops it
//! with `handed_back`; Take over ending without that (the tab closed, the socket gone, the computer
//! slept, another window taking over) stops it with `closed`; a restart stops it (`restart`, see
//! [`crate::db::Database::stop_leftover_recordings`]). Every one keeps the steps, and the first
//! reason wins.

use super::{capture, emit, now, scrub, step_json};
use crate::db::show_once::{Added, NewStep, Recording, ShowOnceError, StepRow};
use crate::server::AppState;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::{Arc, LazyLock, Mutex};
use std::time::Duration;
use tokio::sync::{mpsc, oneshot};

/// How long a stop waits for the stage to take the pending edit first.
pub const STOP_WAIT: Duration = Duration::from_secs(3);
/// Most pending edits taken from a page at a stop.
pub const MAX_FLUSHED: usize = 20;

/// Fired once a [`Cmd::Stop`] is done (or dropped unfired, which counts the same).
#[derive(Debug, Clone, Default)]
pub struct Done(Arc<Mutex<Option<oneshot::Sender<()>>>>);

impl Done {
    pub fn new() -> (Done, oneshot::Receiver<()>) {
        let (tx, rx) = oneshot::channel();
        (Done(Arc::new(Mutex::new(Some(tx)))), rx)
    }

    pub fn fire(&self) {
        if let Some(tx) = self.0.lock().unwrap_or_else(|e| e.into_inner()).take() {
            let _ = tx.send(());
        }
    }
}

impl PartialEq for Done {
    fn eq(&self, _: &Done) -> bool {
        true
    }
}

/// What the routes ask the linked socket to do.
#[derive(Debug, Clone, PartialEq)]
pub enum Cmd {
    /// Start (or Keep recording): install the recorder, open `start_url` as the person's own
    /// navigation, and record the page it's on.
    Begin { recording_id: String, start_url: Option<String> },
    /// Stop this recording here: take the pending edit, then stop with `reason` (first reason
    /// wins) and fire `done`.
    Stop { recording_id: String, reason: String, done: Done },
    /// The recording stopped elsewhere (Discard, or a stop the stage didn't answer in time): take
    /// the recorder off.
    End { recording_id: String, reason: String },
    /// A frame for the person on this socket (`skill.recording.state` after a note, a delete...).
    Frame(Value),
}

/// What the socket must do in the browser or send to the person.
#[derive(Debug, Clone, PartialEq)]
pub enum Act {
    /// A frame for the controlling socket.
    Send(Value),
    /// Add the recorder's binding and script (and run it on the page that's open now), then read
    /// the page's address into [`StageRecorder::on_navigated`].
    Install,
    /// Take the script off future pages. The page that's open keeps a recorder nobody listens
    /// to: what it sends is dropped here, never stored.
    Uninstall,
    /// Open this address as the person's own navigation. If it can't be opened, read the page
    /// that's open into [`StageRecorder::on_start_failed`].
    Navigate(String),
    /// Take the field edits still pending in the page (the recorder's flush, in its world) into
    /// [`StageRecorder::on_flushed`]; nothing when the page can't be reached.
    Flush,
    /// Then stop with this reason ([`StageRecorder::stop_now`]) and fire `done`.
    Stop { reason: String, done: Done },
}

struct Link {
    socket: u64,
    tx: mpsc::UnboundedSender<Cmd>,
}

static LINKS: LazyLock<Mutex<HashMap<String, Link>>> = LazyLock::new(Default::default);

fn links() -> std::sync::MutexGuard<'static, HashMap<String, Link>> {
    LINKS.lock().unwrap_or_else(|e| e.into_inner())
}

/// Whether a stream socket holds Take over on `stage` right now.
pub fn taken_over(stage: &str) -> bool {
    links().get(stage).is_some_and(|l| !l.tx.is_closed())
}

/// Sends `cmd` to the socket holding Take over on `stage`; false when there is none.
pub fn send(stage: &str, cmd: Cmd) -> bool {
    links().get(stage).is_some_and(|l| l.tx.send(cmd).is_ok())
}

/// Asks the socket holding Take over on `stage` to stop `recording_id` with `reason`, taking the
/// pending edit first, and waits up to [`STOP_WAIT`]. False when no socket is linked there; the
/// caller stops it in the database either way (a no-op when the stage did).
pub async fn stop_live(stage: &str, recording_id: &str, reason: &str) -> bool {
    let tx = match links().get(stage) {
        Some(link) => link.tx.clone(),
        None => return false,
    };
    stop_via(&tx, recording_id, reason).await
}

async fn stop_via(tx: &mpsc::UnboundedSender<Cmd>, recording_id: &str, reason: &str) -> bool {
    let (done, wait) = Done::new();
    if tx.send(Cmd::Stop { recording_id: recording_id.into(), reason: reason.into(), done }).is_err() {
        return false;
    }
    let _ = tokio::time::timeout(STOP_WAIT, wait).await;
    true
}

/// Tells whichever stage runs `recording_id` that it ended (Discard, a late sweep).
pub fn recording_ended(recording_id: &str, reason: &str) {
    for link in links().values() {
        let _ = link.tx.send(Cmd::End { recording_id: recording_id.into(), reason: reason.into() });
    }
}

/// One stream socket's side of the recorder.
pub struct StageRecorder {
    state: AppState,
    stage: String,
    socket: u64,
    rx: Option<mpsc::UnboundedReceiver<Cmd>>,
    /// The recording live on this socket.
    live: Option<String>,
    /// The page last recorded, so a reload or a same-page event isn't a new page step.
    last_page: Option<(String, String)>,
    /// Start opened a `start_url` that hasn't loaded yet: the page the browser was on before
    /// isn't a step, the start page is the first one.
    start_pending: bool,
}

impl StageRecorder {
    pub fn new(state: AppState, stage: String, socket: u64) -> Self {
        StageRecorder { state, stage, socket, rx: None, live: None, last_page: None, start_pending: false }
    }

    /// Whether a recording is live on this socket.
    pub fn is_live(&self) -> bool {
        self.live.is_some()
    }

    /// This socket took Take over: it's the stage's recorder from now on. A recording still live
    /// on the stage isn't this socket's. That's defensive only: a second window can't take over
    /// while the first holds control (the lease), so this runs only after the first window's
    /// connection dropped, before its socket noticed. It stops with `closed`, as the drop would,
    /// its steps kept for review, and that window takes its recorder off. The first reason wins.
    pub async fn claim(&mut self) {
        let (tx, rx) = mpsc::unbounded_channel();
        let old = links().insert(self.stage.clone(), Link { socket: self.socket, tx });
        self.rx = Some(rx);
        let Ok(Some(rec)) = self.state.db.live_recording_on(&self.stage).await else { return };
        if self.live.as_deref() == Some(rec.id.as_str()) {
            return;
        }
        let old = old.filter(|l| l.socket != self.socket);
        // the other window takes the pending edit and stops it, if it still can
        if let Some(old) = old.as_ref() {
            stop_via(&old.tx, &rec.id, "closed").await;
        }
        if let Ok(true) = self.state.db.stop_recording(&rec.id, "closed", now()).await {
            let stopped = self.state.db.get_recording(&rec.id).await.ok().flatten().unwrap_or(rec.clone());
            emit(&self.state, &stopped).await;
        }
        if let Some(old) = old {
            let stopped = self.state.db.get_recording(&rec.id).await.ok().flatten().unwrap_or(rec);
            let reason = stopped.stop_reason.clone().unwrap_or_else(|| "closed".into());
            let _ = old.tx.send(Cmd::End { recording_id: stopped.id.clone(), reason });
        }
    }

    fn unlink(&mut self) {
        let mut links = links();
        if links.get(&self.stage).is_some_and(|l| l.socket == self.socket) {
            links.remove(&self.stage);
        }
        self.rx = None;
    }

    /// The next command from the routes; never resolves while the socket isn't linked. `None`
    /// once, when another window took the stage over (the link is gone); pending after that.
    pub async fn next_cmd(&mut self) -> Option<Cmd> {
        match self.rx.as_mut() {
            Some(rx) => {
                let cmd = rx.recv().await;
                if cmd.is_none() {
                    self.rx = None;
                }
                cmd
            }
            None => std::future::pending().await,
        }
    }

    pub async fn on_cmd(&mut self, cmd: Cmd) -> Vec<Act> {
        match cmd {
            Cmd::Begin { recording_id, start_url } => {
                let open = self.state.db.get_recording(&recording_id).await.ok().flatten();
                let Some(rec) = open.filter(|r| r.state == "recording" && r.stage == self.stage) else { return vec![] };
                self.live = Some(rec.id.clone());
                // Keep recording: the page the recording last recorded, so going on from the same
                // page adds no second page step (a page changed while it was stopped still does)
                self.last_page = self.last_page_step(&rec.id).await;
                self.start_pending = start_url.is_some();
                let mut acts = vec![Act::Install, Act::Send(state_frame(&rec))];
                if let Some(url) = start_url {
                    acts.push(Act::Navigate(url));
                }
                acts
            }
            Cmd::Stop { recording_id, reason, done } => {
                if self.live.as_deref() != Some(recording_id.as_str()) {
                    done.fire();
                    return vec![];
                }
                vec![Act::Flush, Act::Stop { reason, done }]
            }
            Cmd::End { recording_id, reason } => {
                if self.live.as_deref() != Some(recording_id.as_str()) {
                    return vec![];
                }
                self.live = None;
                vec![Act::Uninstall, Act::Send(stopped_frame(&recording_id, &reason))]
            }
            Cmd::Frame(frame) => vec![Act::Send(frame)],
        }
    }

    /// The recorder was just installed on the page at `url`: that page is the first step, unless
    /// Start opens a `start_url` next (B1), whose own load is the first page step.
    pub async fn on_installed(&mut self, url: &str) -> Vec<Act> {
        if self.start_pending {
            return vec![];
        }
        self.on_navigated(url).await
    }

    /// The `start_url` couldn't be opened: the page the browser is on is the first step after all.
    pub async fn on_start_failed(&mut self, url: Option<&str>) -> Vec<Act> {
        self.start_pending = false;
        match url {
            Some(url) => self.on_navigated(url).await,
            None => vec![],
        }
    }

    /// The recording's last page step that isn't deleted, as `(origin, path)`.
    async fn last_page_step(&self, id: &str) -> Option<(String, String)> {
        let steps = self.state.db.list_steps(id, now()).await.ok()?;
        steps
            .into_iter()
            .rev()
            .filter(|s| s.deleted_at.is_none() && s.kind == "page")
            .find_map(|s| Some((s.origin?, s.path?)))
    }

    /// A top-level page loaded: a `page` step, unless it's the page already recorded.
    pub async fn on_navigated(&mut self, url: &str) -> Vec<Act> {
        let Some(id) = self.live.clone() else { return vec![] };
        let Some(page) = scrub::scrub(url) else { return vec![] };
        self.start_pending = false;
        let key = (page.origin.clone(), page.path.clone());
        if self.last_page.as_ref() == Some(&key) {
            return vec![];
        }
        self.last_page = Some(key);
        let step = NewStep { kind: "page".into(), origin: Some(page.origin), path: Some(page.path), ..Default::default() };
        self.add(&id, step).await
    }

    /// The recorder in the page called its binding.
    pub async fn on_binding(&mut self, payload: &str) -> Vec<Act> {
        let Some(id) = self.live.clone() else { return vec![] };
        let Some(captured) = capture::parse(payload) else { return vec![] };
        self.add(&id, captured.step).await
    }

    /// What the recorder's flush returned at a stop: each pending field edit, as the payload the
    /// binding would have carried, goes through the same parse (mask, address rules) and becomes a
    /// `type` step. Anything else is dropped, and so is an edit past the 200-step cap.
    pub async fn on_flushed(&mut self, payloads: &[String]) -> Vec<Act> {
        let Some(id) = self.live.clone() else { return vec![] };
        let mut acts = vec![];
        for payload in payloads.iter().take(MAX_FLUSHED) {
            let Some(captured) = capture::parse(payload).filter(|c| c.step.kind == "type") else { continue };
            let t = now();
            match self.state.db.add_final_step(&id, &captured.step, t).await {
                Ok(Some(step)) => {
                    let n = self.number_of(&id, &step, t).await;
                    acts.push(Act::Send(json!({"type":"skill.recording.step","recording_id":id,"step":step_json(&step, n, t)})));
                }
                Ok(None) => break,
                Err(ShowOnceError::WrongState(_)) | Err(ShowOnceError::NotFound) => {
                    self.live = None;
                    acts.push(Act::Uninstall);
                    break;
                }
                Err(e) => tracing::warn!("show it once: a pending edit wasn't saved at the stop: {e:?}"),
            }
        }
        acts
    }

    async fn add(&mut self, id: &str, step: NewStep) -> Vec<Act> {
        let t = now();
        match self.state.db.add_step(id, &step, t).await {
            Ok(Added::Step { step, near }) => {
                let n = self.number_of(id, &step, t).await;
                let mut acts = vec![Act::Send(json!({"type":"skill.recording.step","recording_id":id,"step":step_json(&step, n, t)}))];
                if near {
                    acts.extend(self.near_frame(id, t).await);
                }
                acts
            }
            Ok(Added::Stopped { recording, reason }) => self.stopped(&recording, &reason).await,
            Err(ShowOnceError::WrongState(_)) | Err(ShowOnceError::NotFound) => {
                // stopped or discarded by a route meanwhile
                self.live = None;
                vec![Act::Uninstall]
            }
            Err(e) => {
                tracing::warn!("show it once: a step wasn't saved: {e:?}");
                vec![]
            }
        }
    }

    async fn number_of(&self, id: &str, step: &StepRow, t: i64) -> usize {
        let steps = self.state.db.list_steps(id, t).await.unwrap_or_default();
        steps.iter().position(|s| s.id == step.id).map(|i| i + 1).unwrap_or(steps.len())
    }

    async fn near_frame(&self, id: &str, t: i64) -> Vec<Act> {
        let Ok(Some(rec)) = self.state.db.get_recording(id).await else { return vec![] };
        vec![Act::Send(json!({
            "type": "skill.recording.near", "recording_id": id,
            "steps_left": (crate::db::show_once::LIMIT_STEPS - rec.step_count).max(0),
            "secs_left": (crate::db::show_once::LIMIT_SECS - rec.elapsed_at(t)).max(0),
        }))]
    }

    async fn stopped(&mut self, rec: &Recording, reason: &str) -> Vec<Act> {
        self.live = None;
        emit(&self.state, rec).await;
        vec![Act::Uninstall, Act::Send(stopped_frame(&rec.id, reason))]
    }

    /// Every couple of seconds: the 30-minute limit and the 28:00 warning.
    pub async fn tick(&mut self) -> Vec<Act> {
        let Some(id) = self.live.clone() else { return vec![] };
        let t = now();
        let Ok(Some(rec)) = self.state.db.get_recording(&id).await else {
            self.live = None;
            return vec![Act::Uninstall];
        };
        if rec.state != "recording" {
            self.live = None;
            return vec![Act::Uninstall];
        }
        if rec.limit_reached(t) == Some("limit_time") {
            return vec![Act::Flush, Act::Stop { reason: "limit_time".into(), done: Done::default() }];
        }
        if rec.near_sent == 0 && rec.near_at(t) && self.state.db.mark_near_sent(&id).await.unwrap_or(false) {
            return self.near_frame(&id, t).await;
        }
        vec![]
    }

    /// Stops the live recording with `reason` (after [`Act::Flush`]); the first reason wins.
    pub async fn stop_now(&mut self, reason: &str) -> Vec<Act> {
        match self.live.clone() {
            Some(id) => self.stop(&id, reason).await,
            None => vec![],
        }
    }

    async fn stop(&mut self, id: &str, reason: &str) -> Vec<Act> {
        let _ = self.state.db.stop_recording(id, reason, now()).await;
        match self.state.db.get_recording(id).await {
            // the first reason wins: a Stop or Hand back that got there first is what the person sees
            Ok(Some(rec)) => {
                let first = rec.stop_reason.clone().unwrap_or_else(|| reason.to_string());
                self.stopped(&rec, &first).await
            }
            _ => {
                self.live = None;
                vec![Act::Uninstall]
            }
        }
    }

    /// Hand back: the recording stops (`handed_back`) and the socket is no longer the stage's. The
    /// socket takes the pending edit ([`Act::Flush`]) before calling this.
    pub async fn handed_back(&mut self) -> Vec<Act> {
        self.unlink();
        match self.live.clone() {
            Some(id) => self.stop(&id, "handed_back").await,
            None => vec![],
        }
    }

    /// The socket went away without Hand back (tab closed, connection lost, the computer slept): a
    /// live recording stops with `closed`, the steps kept for review. The socket takes the pending
    /// edit ([`Act::Flush`]) before calling this, if the page can still be reached. After Hand back nothing is
    /// live here any more, so its `handed_back` stands.
    pub async fn closed(&mut self) {
        self.unlink();
        if let Some(id) = self.live.clone() {
            let _ = self.stop(&id, "closed").await;
        }
    }
}

fn stopped_frame(id: &str, reason: &str) -> Value {
    json!({"type":"skill.recording.stopped","recording_id":id,"reason":reason})
}

/// `skill.recording.state`, sent on the stage's socket after a change there.
pub fn state_frame(rec: &Recording) -> Value {
    json!({"type":"skill.recording.state","recording":super::recording_json(rec, now(), None)})
}
