//! The recorder on the Take over stage (api.md §1, §5). Only the stream socket that holds Take
//! over on a stage is linked here, so a recording can only start while the person controls the
//! browser, and the bot's own navigation waits behind that same lease. The socket installs the
//! recorder in kwnt's isolated world ([`super::capture::WORLD`]) when a recording starts and takes
//! it off when it stops; this side turns what arrives into steps and frames for the person.
//!
//! A stage runs one bot's recording at a time. Hand back stops it (`handed_back`), the socket
//! going away stops it (`closed`), and a restart stops it (`restart`, see
//! [`crate::db::Database::stop_leftover_recordings`]); every one keeps the steps.

use super::{capture, emit, now, scrub, step_json};
use crate::db::show_once::{Added, NewStep, Recording, ShowOnceError, StepRow};
use crate::server::AppState;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::{LazyLock, Mutex};
use tokio::sync::mpsc;

/// What the routes ask the linked socket to do.
#[derive(Debug, Clone, PartialEq)]
pub enum Cmd {
    /// Start (or Keep recording): install the recorder, open `start_url` as the person's own
    /// navigation, and record the page it's on.
    Begin { recording_id: String, start_url: Option<String> },
    /// The recording stopped elsewhere (Stop, Discard, the sweep): take the recorder off.
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
    /// Open this address as the person's own navigation.
    Navigate(String),
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

/// Tells whichever stage runs `recording_id` that it ended (the sweep's time limit, Discard).
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
}

impl StageRecorder {
    pub fn new(state: AppState, stage: String, socket: u64) -> Self {
        StageRecorder { state, stage, socket, rx: None, live: None, last_page: None }
    }

    /// Whether a recording is live on this socket.
    pub fn is_live(&self) -> bool {
        self.live.is_some()
    }

    /// This socket took Take over: it's the stage's recorder from now on.
    pub fn claim(&mut self) {
        let (tx, rx) = mpsc::unbounded_channel();
        links().insert(self.stage.clone(), Link { socket: self.socket, tx });
        self.rx = Some(rx);
    }

    fn unlink(&mut self) {
        let mut links = links();
        if links.get(&self.stage).is_some_and(|l| l.socket == self.socket) {
            links.remove(&self.stage);
        }
        self.rx = None;
    }

    /// The next command from the routes; never resolves while the socket isn't linked.
    pub async fn next_cmd(&mut self) -> Option<Cmd> {
        match self.rx.as_mut() {
            Some(rx) => rx.recv().await,
            None => std::future::pending().await,
        }
    }

    pub async fn on_cmd(&mut self, cmd: Cmd) -> Vec<Act> {
        match cmd {
            Cmd::Begin { recording_id, start_url } => {
                let open = self.state.db.get_recording(&recording_id).await.ok().flatten();
                let Some(rec) = open.filter(|r| r.state == "recording" && r.stage == self.stage) else { return vec![] };
                self.live = Some(rec.id.clone());
                self.last_page = None;
                let mut acts = vec![Act::Install, Act::Send(state_frame(&rec))];
                if let Some(url) = start_url {
                    acts.push(Act::Navigate(url));
                }
                acts
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

    /// A top-level page loaded (or the recorder was just installed on one): a `page` step, unless
    /// it's the page already recorded.
    pub async fn on_navigated(&mut self, url: &str) -> Vec<Act> {
        let Some(id) = self.live.clone() else { return vec![] };
        let Some(page) = scrub::scrub(url) else { return vec![] };
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
            return self.stop(&id, "limit_time").await;
        }
        if rec.near_sent == 0 && rec.near_at(t) && self.state.db.mark_near_sent(&id).await.unwrap_or(false) {
            return self.near_frame(&id, t).await;
        }
        vec![]
    }

    async fn stop(&mut self, id: &str, reason: &str) -> Vec<Act> {
        let _ = self.state.db.stop_recording(id, reason, now()).await;
        match self.state.db.get_recording(id).await {
            Ok(Some(rec)) => self.stopped(&rec, reason).await,
            _ => {
                self.live = None;
                vec![Act::Uninstall]
            }
        }
    }

    /// Hand back: the recording stops (`handed_back`) and the socket is no longer the stage's.
    pub async fn handed_back(&mut self) -> Vec<Act> {
        self.unlink();
        match self.live.clone() {
            Some(id) => self.stop(&id, "handed_back").await,
            None => vec![],
        }
    }

    /// The socket went away: a live recording stops (`closed`), the steps kept for review.
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
