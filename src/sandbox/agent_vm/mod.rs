//! Agents in the VM, slice 1: Claude turns run in a per-bot container with no network and no
//! real credential, when the person turns on "Run Claude inside the VM" (Linux only, off by
//! default). Design: `docs/agents-in-vm.md`.
//!
//! A turn is prepared by [`AgentVm::prepare_turn`], which fails closed: with the setting on, any
//! missing piece (Docker, the image, the broker, the container, a credential) is a clear error
//! for the turn and never a run on the host with the real credential. The prepared
//! [`IsolatedTurn`] turns the CLI command kwnt would have run on the host into a `docker exec` in
//! the bot's container, with only allow-listed env and a stand-in credential; the broker
//! ([`broker`]) adds the real one on the way out.

pub mod broker;
pub mod folder_rule;
pub mod notice;

use broker::{Broker, BrokerConfig, Credential, Secret, TurnLease, TurnSpec};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex as StdMutex};
use std::time::{Duration, Instant};
use tokio::process::Command;
use tokio::sync::{OwnedSemaphorePermit, Semaphore};

pub use crate::config::AgentsInVmConfig;

/// The setting (kwnt's settings table) for "Run Claude inside the VM": kept in both modes, only
/// the person changes it, and outside vm mode it has no effect (D1).
pub const SETTING_KEY: &str = "computer_isolate_claude";
/// The Claude Code version baked into the agent image (the host's at the time of slice 1).
pub const CLAUDE_VERSION: &str = "2.1.316";
/// Where the CLI in the container reaches the broker (socat in the container).
pub const IN_CONTAINER_ORIGIN: &str = "http://127.0.0.1:8080";
const CONTAINER_HOME: &str = "/home/agent";
const CONTAINER_RUN: &str = "/run/kwnt";
const IDLE_REMOVE_AFTER: Duration = Duration::from_secs(30 * 60);
/// How long a turn's processes get to exit after its TERM before the container goes (R-9).
const STOP_GRACE: Duration = Duration::from_secs(1);
/// Stand-in credentials the CLI in the container holds; the broker drops them.
pub const DUMMY_API_KEY: &str = "sk-ant-api03-kwnt-broker-stand-in-not-a-key-AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA";
pub const DUMMY_OAUTH_TOKEN: &str = "sk-ant-oat01-kwnt-broker-stand-in-not-a-token-AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA";

pub fn image_tag() -> String {
    format!("kwnt-agent:v1-claude-{CLAUDE_VERSION}")
}

/// Why a Claude turn can't run isolated. `code` is stable (UI and docs); `message` is for the
/// person.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Refusal {
    pub code: &'static str,
    pub message: String,
    /// For `unsafe_folder`: why, for the chat meta's `unsafe_kind` ([`notice::IsolationFailed`]).
    pub unsafe_kind: Option<folder_rule::UnsafeKind>,
}

/// Refusal codes. Where Mira's chat notices have a reason (`design/ui.md`), the code is that
/// reason; the rest get her "unknown code" notice, which names the code.
pub mod codes {
    pub const UNSUPPORTED_OS: &str = "unsupported_os";
    pub const DOCKER_UNAVAILABLE: &str = "docker_unavailable";
    /// The agent image is being set up (notice 2): the reply fails rather than waits.
    pub const PREPARING: &str = "preparing";
    /// The image setup failed (notice 2b).
    pub const DOWNLOAD_FAILED: &str = "download_failed";
    /// kwnt's broker socket couldn't start (notice 2c).
    pub const RELAY_DOWN: &str = "relay_down";
    pub const CONTAINER_FAILED: &str = "container_failed";
    pub const NO_CREDENTIAL: &str = "no_credential";
    pub const SIGN_IN_EXPIRED: &str = "sign_in_expired";
    /// kwnt runs as root: the container would run as uid 0 (test plan S-16, #14).
    pub const HOST_IS_ROOT: &str = "host_is_root";
    /// The folder's name can't be passed to Docker safely (newline, not UTF-8).
    pub const FOLDER_NAME_UNSUPPORTED: &str = "folder_name_unsupported";
    /// kwnt couldn't write down that the folder is used by an isolated bot (D4), so it doesn't
    /// start the container.
    pub const FOLDER_RECORD_FAILED: &str = "folder_record_failed";
    /// D5: the folder is, holds or sits in a place that must not be shared ([`super::folder_rule`]).
    pub const UNSAFE_FOLDER: &str = "unsafe_folder";
    /// D4b: a host Codex or Grok turn in a folder an isolated bot used.
    pub const FOLDER_USED_BY_ISOLATED_BOT: &str = "folder_used_by_isolated_bot";

    /// Every code above: each has its own words in [`super::notice`] (ui-check-446 fix 4).
    pub const ALL: &[&str] = &[
        UNSUPPORTED_OS,
        DOCKER_UNAVAILABLE,
        PREPARING,
        DOWNLOAD_FAILED,
        RELAY_DOWN,
        CONTAINER_FAILED,
        NO_CREDENTIAL,
        SIGN_IN_EXPIRED,
        HOST_IS_ROOT,
        FOLDER_NAME_UNSUPPORTED,
        FOLDER_RECORD_FAILED,
        UNSAFE_FOLDER,
        FOLDER_USED_BY_ISOLATED_BOT,
    ];
}

impl Refusal {
    pub fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self { code, message: message.into(), unsafe_kind: None }
    }

}

/// The OS gate: slice 1 is Linux only (Mac shows the toggle disabled).
pub fn supported_os() -> Result<(), Refusal> {
    if cfg!(target_os = "linux") {
        Ok(())
    } else if cfg!(target_os = "macos") {
        Err(Refusal::new(codes::UNSUPPORTED_OS, "Coming to Mac later"))
    } else {
        Err(Refusal::new(codes::UNSUPPORTED_OS, "Only on Linux for now"))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum ImageState {
    Unknown,
    Building,
    Ready,
    Failed(String),
}

#[derive(Debug, Default)]
struct ContainerUse {
    active: usize,
    last_used: Option<Instant>,
}

/// Can the switch be turned on right now (`isolate_claude_toggle`, design/ui.md).
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct Toggle {
    pub available: bool,
    /// `mac_later` | `docker_unavailable` | `not_supported_yet` (Windows; added to Mira's list)
    pub reason: Option<&'static str>,
}

/// The `download_failed` detail after a failed image build (Mira's E line, ui-check-446 fix 7).
/// The chip reads "Claude · Stopped: the setup failed (…)", so it starts in lower case.
pub const SETUP_FAILED: &str = "the setup failed (kwant’s log has the details)";

/// Download progress while `state` is `preparing`.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct Progress {
    pub done_bytes: u64,
    pub total_bytes: u64,
}

/// One engine's chip in the computer view (`agents.claude|codex|grok`, design/ui.md).
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct AgentState {
    /// `off` | `isolated` | `not_isolated` | `preparing` | `unavailable`
    pub state: &'static str,
    /// `docker_unavailable` | `download_needed` | `download_failed` | `relay_down` |
    /// `sign_in_expired` | `mac_later` | `local_mode` | `not_supported_yet`
    pub reason: Option<&'static str>,
    pub detail: Option<String>,
    pub progress: Option<Progress>,
}

impl AgentState {
    fn new(state: &'static str, reason: Option<&'static str>) -> Self {
        Self { state, reason, detail: None, progress: None }
    }
    fn detail(mut self, d: impl Into<String>) -> Self {
        self.detail = Some(d.into());
        self
    }
}

/// What [`AgentVm::view`] needs from the rest of kwnt.
#[derive(Debug, Clone, Copy)]
pub struct ViewInputs {
    /// The person's switch.
    pub setting_on: bool,
    /// The computer is set to "this computer's VM" (D1: isolation applies only then).
    pub vm_mode: bool,
    pub docker: bool,
    /// Some Claude account here signs in with the CLI (API-key and Custom accounts run inside
    /// kwnt, so the switch does nothing for them).
    pub cli_account: bool,
}

/// The image as the view sees it.
#[derive(Debug, Clone, PartialEq, Eq)]
enum ImageView {
    Ready,
    Missing,
    Building,
    Failed(String),
}

/// What the person reads when they try to turn the switch on and it isn't available: the PUT
/// 400's `error`, which the Settings error line prints as it is (ui-check-446 fix 5). The code
/// goes in `code`.
pub fn toggle_message(reason: &str) -> &'static str {
    match reason {
        "mac_later" => "Coming to Mac later. For now, Claude runs on this computer in both modes.",
        "not_supported_yet" => "Only on Linux for now. Claude runs on this computer in both modes.",
        codes::DOCKER_UNAVAILABLE => "Start Docker to turn this on.",
        _ => "Run Claude inside the VM can't be turned on here right now.",
    }
}

fn toggle(docker: bool) -> Toggle {
    if cfg!(target_os = "macos") {
        Toggle { available: false, reason: Some("mac_later") }
    } else if supported_os().is_err() {
        Toggle { available: false, reason: Some("not_supported_yet") }
    } else if !docker {
        Toggle { available: false, reason: Some(codes::DOCKER_UNAVAILABLE) }
    } else {
        Toggle { available: true, reason: None }
    }
}

/// The Claude chip. D1: outside vm mode the switch does nothing (`not_isolated`, `local_mode`).
fn claude_state(inp: ViewInputs, image: impl FnOnce() -> ImageView) -> AgentState {
    if !inp.setting_on {
        return AgentState::new("off", None);
    }
    if cfg!(target_os = "macos") {
        return AgentState::new("off", Some("mac_later"));
    }
    if supported_os().is_err() {
        return AgentState::new("off", Some("not_supported_yet"));
    }
    if !inp.vm_mode {
        return AgentState::new("not_isolated", Some("local_mode"));
    }
    if !inp.cli_account {
        return AgentState::new("not_isolated", None)
            .detail("Bots using an API key run inside kwant; their tools already use the VM.");
    }
    if !inp.docker {
        return AgentState::new("unavailable", Some(codes::DOCKER_UNAVAILABLE));
    }
    match image() {
        ImageView::Ready => AgentState::new("isolated", None),
        ImageView::Building => AgentState::new("preparing", None),
        ImageView::Missing => AgentState::new("unavailable", Some("download_needed")),
        ImageView::Failed(why) => AgentState::new("unavailable", Some(codes::DOWNLOAD_FAILED)).detail(why),
    }
}

/// Codex and Grok always run on this computer in 0.7.7.
fn other_state() -> AgentState {
    AgentState::new("not_isolated", Some("not_supported_yet"))
}

pub struct AgentVm {
    /// `<data>/agent-vm`
    root: PathBuf,
    data_dir: PathBuf,
    broker: Arc<Broker>,
    image: StdMutex<ImageState>,
    image_build: tokio::sync::Mutex<()>,
    containers: StdMutex<HashMap<String, ContainerUse>>,
    container_start: tokio::sync::Mutex<()>,
    /// Set by tests to a prebuilt image.
    image_override: Option<String>,
    limits: AgentsInVmConfig,
    /// At most `limits.max_turns` isolated turns at once; a turn holds its permit until it ends.
    turns: Arc<Semaphore>,
    /// Turns waiting for a permit now, for the position a new one reports ([`TurnWait::Queued`]).
    waiting: std::sync::atomic::AtomicUsize,
    /// Where D4's folder record goes; `None` only in tests that don't start a container.
    db: Option<crate::db::Database>,
}

impl std::fmt::Debug for AgentVm {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AgentVm").field("root", &self.root).finish_non_exhaustive()
    }
}

/// What a turn reports while it waits for a free spot in the VM ([`AgentVm::prepare_turn_reporting`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TurnWait {
    /// Every spot is taken: `slots` turns run, and this one is `position` in line (1 is next).
    Queued { position: usize, slots: usize },
    /// It got its spot (sent only after `Queued`).
    Started,
}

/// One turn's inputs to [`AgentVm::prepare_turn`].
#[derive(Debug, Clone)]
pub struct TurnRequest {
    pub bot_id: String,
    /// The bot's folder on this computer; `None` gives the bot a private one.
    pub folder: Option<PathBuf>,
    pub credential: Credential,
    /// Folder holding the credential (never mountable).
    pub credential_home: Option<PathBuf>,
    /// `http://127.0.0.1:<port>`, kwnt's own origin, for its MCP endpoints.
    pub kwnt_origin: Option<String>,
}

impl AgentVm {
    /// The server's: always with its database, so every isolated turn's folder is recorded before
    /// it runs (D4); a turn whose record can't be written is refused.
    pub fn new(data_dir: &Path, limits: AgentsInVmConfig, db: crate::db::Database) -> Arc<Self> {
        let broker_cfg = BrokerConfig::new(data_dir.join("agent-vm").join("broker.log"));
        Self::with_parts(data_dir, broker_cfg, None, limits, Some(db))
    }

    /// For tests: a broker with other upstreams, and a prebuilt image. Without a database, folders
    /// aren't recorded (only tests build it so).
    pub fn with_parts(
        data_dir: &Path,
        cfg: BrokerConfig,
        image: Option<String>,
        limits: AgentsInVmConfig,
        db: Option<crate::db::Database>,
    ) -> Arc<Self> {
        let root = data_dir.join("agent-vm");
        Arc::new(Self {
            root,
            data_dir: data_dir.to_path_buf(),
            broker: Broker::new(cfg),
            image: StdMutex::new(ImageState::Unknown),
            image_build: tokio::sync::Mutex::new(()),
            containers: StdMutex::new(HashMap::new()),
            container_start: tokio::sync::Mutex::new(()),
            image_override: image,
            turns: Arc::new(Semaphore::new(limits.max_turns.max(1))),
            waiting: Default::default(),
            limits,
            db,
        })
    }

    pub fn limits(&self) -> &AgentsInVmConfig {
        &self.limits
    }

    /// Isolated turns that can start right now without waiting.
    pub fn free_turn_slots(&self) -> usize {
        self.turns.available_permits()
    }

    pub fn broker(&self) -> &Arc<Broker> {
        &self.broker
    }

    fn image(&self) -> String {
        self.image_override.clone().unwrap_or_else(image_tag)
    }

    /// The label every container of this kwnt carries (two kwnts on one host keep apart).
    fn root_label(&self) -> String {
        broker::sha8(&self.root.to_string_lossy())
    }

    fn bot_dir(&self, bot_id: &str) -> PathBuf {
        self.root.join(&sha_hex(bot_id)[..12])
    }

    fn container_name(bot_id: &str, workdir: &Path) -> String {
        format!("kwnt-agent-{}-{}", &sha_hex(bot_id)[..8], &sha_hex(&workdir.to_string_lossy())[..8])
    }

    /// `isolate_claude_toggle` and `agents` for the computer view. Asks Docker whether the image
    /// is there only when it isn't known yet.
    pub async fn view(&self, inp: ViewInputs) -> (Toggle, std::collections::BTreeMap<&'static str, AgentState>) {
        let known = self.image.lock().unwrap().clone();
        let image = match known {
            ImageState::Ready => ImageView::Ready,
            ImageState::Building => ImageView::Building,
            ImageState::Failed(why) => ImageView::Failed(why),
            ImageState::Unknown if inp.setting_on && inp.vm_mode && inp.docker && inp.cli_account => {
                if super::manager::image_exists(&self.image()).await {
                    let mut st = self.image.lock().unwrap();
                    if *st == ImageState::Unknown {
                        *st = ImageState::Ready;
                    }
                    ImageView::Ready
                } else {
                    ImageView::Missing
                }
            }
            ImageState::Unknown => ImageView::Missing,
        };
        let agents = [("claude", claude_state(inp, || image)), ("codex", other_state()), ("grok", other_state())];
        (toggle(inp.docker), agents.into_iter().collect())
    }

    /// Turning the switch on starts the download in the background (notice 2 covers a reply
    /// sent before it ends). A no-op when the image is ready or already being built.
    pub fn start_setup(self: &Arc<Self>) {
        if supported_os().is_err() {
            return;
        }
        let vm = self.clone();
        // `image_or_start_setup` may ask Docker (blocking, a few ms)
        tokio::task::spawn_blocking(move || {
            let _ = vm.image_or_start_setup();
        });
    }

    /// A spot for one isolated turn, waiting in line (FIFO) when all `max_turns` are taken, and
    /// telling `report` that it waits and when it's through.
    async fn turn_permit<F, Fut>(&self, report: &mut F) -> Result<OwnedSemaphorePermit, Refusal>
    where
        F: FnMut(TurnWait) -> Fut,
        Fut: std::future::Future<Output = ()>,
    {
        use std::sync::atomic::Ordering;
        use tokio::sync::TryAcquireError;
        let closed = || Refusal::new(codes::CONTAINER_FAILED, "kwnt is shutting down");
        match self.turns.clone().try_acquire_owned() {
            Ok(p) => return Ok(p),
            Err(TryAcquireError::Closed) => return Err(closed()),
            Err(TryAcquireError::NoPermits) => {}
        }
        /// Leaves the line however the wait ends (a stopped reply drops the future).
        struct InLine<'a>(&'a std::sync::atomic::AtomicUsize);
        impl Drop for InLine<'_> {
            fn drop(&mut self) {
                self.0.fetch_sub(1, Ordering::SeqCst);
            }
        }
        let position = self.waiting.fetch_add(1, Ordering::SeqCst) + 1;
        let in_line = InLine(&self.waiting);
        report(TurnWait::Queued { position, slots: self.limits.max_turns.max(1) }).await;
        let permit = self.turns.clone().acquire_owned().await.map_err(|_| closed())?;
        drop(in_line);
        report(TurnWait::Started).await;
        Ok(permit)
    }

    pub fn image_ready(&self) -> bool {
        *self.image.lock().unwrap() == ImageState::Ready
    }

    /// Builds the agent image if it isn't there (single flight).
    pub async fn ensure_image(&self) -> Result<(), Refusal> {
        let _held = self.image_build.lock().await;
        let tag = self.image();
        if super::manager::image_exists(&tag).await {
            *self.image.lock().unwrap() = ImageState::Ready;
            return Ok(());
        }
        if self.image_override.is_some() {
            let why = format!("The test image {tag} isn't built");
            *self.image.lock().unwrap() = ImageState::Failed(why.clone());
            return Err(Refusal::new(codes::DOWNLOAD_FAILED, why));
        }
        *self.image.lock().unwrap() = ImageState::Building;
        let built = async {
            super::manager::ensure_executor_image().await?;
            super::manager::ensure_image(
                &tag,
                &[
                    ("Dockerfile", include_str!("../../../docker/agent.Dockerfile")),
                    ("agent-entry.sh", include_str!("../../../docker/agent-entry.sh")),
                ],
                &[("CLAUDE_VERSION", CLAUDE_VERSION)],
            )
            .await
        }
        .await;
        match built {
            Ok(()) => {
                *self.image.lock().unwrap() = ImageState::Ready;
                Ok(())
            }
            Err(e) => {
                tracing::warn!(error = %e, "agent image build failed");
                let why = SETUP_FAILED.to_string();
                *self.image.lock().unwrap() = ImageState::Failed(why.clone());
                Err(Refusal::new(codes::DOWNLOAD_FAILED, why))
            }
        }
    }

    /// Everything a Claude turn needs to run isolated, or why it can't. Fails closed.
    pub async fn prepare_turn(self: &Arc<Self>, req: TurnRequest) -> Result<IsolatedTurn, Refusal> {
        self.prepare_turn_reporting(req, |_| std::future::ready(())).await
    }

    /// [`Self::prepare_turn`], telling `report` when the turn has to wait for a free spot and
    /// when it gets one, so the waiting isn't taken for a stalled reply.
    pub async fn prepare_turn_reporting<F, Fut>(self: &Arc<Self>, req: TurnRequest, mut report: F) -> Result<IsolatedTurn, Refusal>
    where
        F: FnMut(TurnWait) -> Fut,
        Fut: std::future::Future<Output = ()>,
    {
        supported_os()?;
        if !super::manager::SandboxManager::new().is_docker_available().await {
            return Err(Refusal::new(codes::DOCKER_UNAVAILABLE, "Docker isn't running on this computer"));
        }
        refuse_root(host_ids())?;
        let bot_dir = self.bot_dir(&req.bot_id);
        let workdir = match &req.folder {
            Some(f) => {
                let f = f.canonicalize().map_err(|_| {
                    Refusal::new(codes::CONTAINER_FAILED, "The bot's folder doesn't exist on this computer")
                })?;
                check_folder(&f, &folder_rule::Places::for_host(&self.data_dir, req.credential_home.as_deref()))?;
                f
            }
            None => {
                let w = bot_dir.join("work");
                std::fs::create_dir_all(&w).map_err(|e| Refusal::new(codes::CONTAINER_FAILED, format!("Couldn't make the bot's folder: {e}")))?;
                // Recorded (D4) and mounted by its real path, like a chosen folder: kwnt's data
                // folder may be reached through a symlink
                w.canonicalize().map_err(|e| Refusal::new(codes::CONTAINER_FAILED, format!("Couldn't find the bot's folder: {e}")))?
            }
        };
        check_mountable(&workdir)?;
        check_credential(&req.credential)?;
        // A reply during the image setup fails with notice 2 rather than waiting for it
        self.image_or_start_setup()?;
        // At most `max_turns` at once: a queued turn starts no container until it has a permit
        let permit = self.turn_permit(&mut report).await?;
        // D4: written down before anything runs in the folder, so a turn that fails later counts
        self.record_folder(&workdir, &req.bot_id).await?;
        let run_dir = bot_dir.join("run");
        self.broker
            .listen(&req.bot_id, &run_dir.join("broker.sock"))
            .await
            .map_err(|e| Refusal::new(codes::RELAY_DOWN, format!("kwnt's broker couldn't start: {e}")))?;
        let home = bot_dir.join("home");
        let container = self.ensure_container(&req.bot_id, &workdir, &home, &run_dir).await?;
        let oauth = req.credential.is_oauth();
        let lease = self.broker.register(TurnSpec {
            bot: req.bot_id.clone(),
            credential: req.credential,
            kwnt_origin: req.kwnt_origin.clone(),
            cap: self.limits.request_limit,
        });
        Ok(IsolatedTurn {
            vm: self.clone(),
            lease,
            container,
            workdir,
            run_dir,
            oauth,
            kwnt_origin: req.kwnt_origin,
            mcp_file: None,
            _permit: Some(permit),
        })
    }

    /// Ok when the image is there; otherwise starts its setup in the background (once) and
    /// refuses this turn with `preparing`, or `download_failed` after a failed setup (the next
    /// turn tries again).
    fn image_or_start_setup(self: &Arc<Self>) -> Result<(), Refusal> {
        let state = self.image.lock().unwrap().clone();
        let failed = match state {
            ImageState::Ready => return Ok(()),
            ImageState::Building => return Err(Refusal::new(codes::PREPARING, "Claude's VM is still being set up")),
            // An image built earlier is ready straight away
            ImageState::Unknown if self.image_present_sync() => return Ok(()),
            ImageState::Unknown => false,
            ImageState::Failed(_) => true,
        };
        // Marked here, not in the task, so the status shows `preparing` at once. Two turns that
        // both get here start one build: `ensure_image` is single flight and checks first.
        *self.image.lock().unwrap() = ImageState::Building;
        let vm = self.clone();
        tokio::spawn(async move {
            let _ = vm.ensure_image().await;
        });
        Err(if failed {
            Refusal::new(codes::DOWNLOAD_FAILED, "The last setup of Claude's VM didn't finish; kwnt is trying again")
        } else {
            Refusal::new(codes::PREPARING, "Claude's VM is being set up (the first time takes a few minutes)")
        })
    }

    /// Whether the image is in Docker already (blocking `docker image inspect`, a few ms).
    fn image_present_sync(&self) -> bool {
        let ok = std::process::Command::new("docker")
            .args(["image", "inspect", "--format", "{{.Id}}", &self.image()])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .is_ok_and(|s| s.success());
        if ok {
            *self.image.lock().unwrap() = ImageState::Ready;
        }
        ok
    }

    async fn record_folder(&self, workdir: &Path, bot_id: &str) -> Result<(), Refusal> {
        // Only `with_parts` (tests) builds a VM without a database; the server's `new` can't
        let Some(db) = &self.db else { return Ok(()) };
        db.record_isolated_folder(workdir, bot_id).await.map_err(|e| {
            tracing::warn!(error = %e, "couldn't record the isolated folder");
            Refusal::new(codes::FOLDER_RECORD_FAILED, "kwnt couldn't note that this folder is used inside the VM")
        })
    }

    /// The bot's running container for `workdir`, started when there's none, counted as in use
    /// by one more turn. The count goes up under `container_start`, the lock [`Self::end_container`]
    /// takes too, so a container is never removed under a turn that just picked it.
    async fn ensure_container(&self, bot_id: &str, workdir: &Path, home: &Path, run_dir: &Path) -> Result<String, Refusal> {
        let _held = self.container_start.lock().await;
        let name = self.start_container(bot_id, workdir, home, run_dir).await?;
        self.containers.lock().unwrap().entry(name.clone()).or_default().active += 1;
        Ok(name)
    }

    async fn start_container(&self, bot_id: &str, workdir: &Path, home: &Path, run_dir: &Path) -> Result<String, Refusal> {
        let name = Self::container_name(bot_id, workdir);
        let fail = |why: String| Refusal::new(codes::CONTAINER_FAILED, why);
        let running = Command::new("docker")
            .args(["inspect", "-f", "{{.State.Running}}", &name])
            .kill_on_drop(true)
            .output()
            .await
            .is_ok_and(|o| String::from_utf8_lossy(&o.stdout).trim() == "true");
        if running {
            return Ok(name);
        }
        let _ = Command::new("docker").args(["rm", "-f", &name]).kill_on_drop(true).output().await;
        for d in [home, run_dir] {
            std::fs::create_dir_all(d).map_err(|e| fail(format!("Couldn't prepare the bot's computer folders: {e}")))?;
            set_private(d);
        }
        let mounts = Mounts::new(workdir, home, run_dir)?;
        let args = run_args(&name, &self.root_label(), &mounts, &self.image(), host_ids(), self.limits.memory_mb);
        let out = Command::new("docker")
            .args(&args)
            .kill_on_drop(true)
            .output()
            .await
            .map_err(|e| fail(format!("Docker didn't start: {e}")))?;
        if !out.status.success() {
            let err = String::from_utf8_lossy(&out.stderr);
            tracing::warn!(container = %name, error = %err.trim(), "agent container failed to start");
            return Err(fail("The bot's computer didn't start (see kwnt's log)".into()));
        }
        Ok(name)
    }

    /// One turn fewer in `container`; true when that was its last one.
    fn release(&self, container: &str) -> bool {
        let mut map = self.containers.lock().unwrap();
        let Some(u) = map.get_mut(container) else { return false };
        u.active = u.active.saturating_sub(1);
        u.last_used = Some(Instant::now());
        u.active == 0
    }

    /// R-9 (S-9): after a container's last turn, nothing the bot started may keep running until
    /// the next one: a `setsid` child leaves the turn's process group, so the group's TERM misses
    /// it. After [`STOP_GRACE`] the container is removed (`docker rm -f`: SIGKILL to every process
    /// in it, PID 1 included) unless a turn picked it up meanwhile; the next turn starts a fresh
    /// one. The folders are mounts, so nothing the bot keeps is lost, only `/tmp` (a tmpfs).
    async fn end_container(&self, container: &str) {
        tokio::time::sleep(STOP_GRACE).await;
        let _held = self.container_start.lock().await;
        {
            let mut map = self.containers.lock().unwrap();
            if map.get(container).is_some_and(|u| u.active > 0) {
                return;
            }
            map.remove(container);
        }
        match Command::new("docker").args(["rm", "-f", container]).kill_on_drop(true).output().await {
            Ok(o) if o.status.success() => {}
            Ok(o) => tracing::warn!(container, error = %String::from_utf8_lossy(&o.stderr).trim(), "couldn't remove an agent container after its last turn"),
            Err(e) => tracing::warn!(container, error = %e, "couldn't remove an agent container after its last turn"),
        }
    }

    /// Removes this kwnt's agent containers left over from an earlier run, then every 5 minutes
    /// the ones idle for half an hour.
    pub fn start_reaper(self: &Arc<Self>) {
        let vm = self.clone();
        tokio::spawn(async move {
            vm.remove_containers(|_| true).await;
            loop {
                tokio::time::sleep(Duration::from_secs(300)).await;
                let idle: Vec<String> = {
                    let mut map = vm.containers.lock().unwrap();
                    let names: Vec<String> = map
                        .iter()
                        .filter(|(_, u)| u.active == 0 && u.last_used.is_some_and(|t| t.elapsed() > IDLE_REMOVE_AFTER))
                        .map(|(n, _)| n.clone())
                        .collect();
                    for n in &names {
                        map.remove(n);
                    }
                    names
                };
                if !idle.is_empty() {
                    vm.remove_containers(|n| idle.iter().any(|i| i == n)).await;
                }
            }
        });
    }

    /// Removes this kwnt's agent containers whose name `pick` accepts.
    pub async fn remove_containers(&self, pick: impl Fn(&str) -> bool) {
        let Ok(out) = Command::new("docker")
            .args(["ps", "-a", "--filter", "label=kwnt.agent=1", "--filter"])
            .arg(format!("label=kwnt.root={}", self.root_label()))
            .args(["--format", "{{.Names}}"])
            .kill_on_drop(true)
            .output()
            .await
        else {
            return;
        };
        for name in String::from_utf8_lossy(&out.stdout).lines().map(str::trim).filter(|n| n.starts_with("kwnt-agent-") && pick(n)) {
            let _ = Command::new("docker").args(["rm", "-f", name]).kill_on_drop(true).output().await;
        }
    }
}

/// The three bind mounts of a bot's container, as `--mount` values. Docker reads `--mount` as
/// CSV, so each field is quoted (a `,` or `"` in a folder name can't add an option); a name with
/// a newline or that isn't UTF-8 is refused instead.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Mounts {
    workdir: String,
    args: [String; 3],
}

impl Mounts {
    fn new(workdir: &Path, home: &Path, run_dir: &Path) -> Result<Self, Refusal> {
        let w = check_mountable(workdir)?;
        let h = check_mountable(home)?;
        let r = check_mountable(run_dir)?;
        Ok(Self {
            workdir: w.to_string(),
            args: [mount_arg(w, w, false), mount_arg(h, CONTAINER_HOME, false), mount_arg(r, CONTAINER_RUN, true)],
        })
    }
}

fn csv_field(f: &str) -> String {
    format!("\"{}\"", f.replace('"', "\"\""))
}

fn mount_arg(src: &str, dst: &str, readonly: bool) -> String {
    let mut v = format!("type=bind,{},{}", csv_field(&format!("src={src}")), csv_field(&format!("dst={dst}")));
    if readonly {
        v.push_str(",readonly");
    }
    v
}

/// The folder as a string Docker can take: absolute, UTF-8, no control characters.
fn check_mountable(p: &Path) -> Result<&str, Refusal> {
    let refuse = || Refusal::new(codes::FOLDER_NAME_UNSUPPORTED, "The folder's name has characters kwnt can't pass to the VM. Rename the folder, then Retry");
    let s = p.to_str().ok_or_else(refuse)?;
    if !p.is_absolute() || s.chars().any(char::is_control) {
        return Err(refuse());
    }
    Ok(s)
}

/// The container must not run as uid 0: kwnt passes its own ids so the person owns what the bot
/// makes, and as root that would make the bot root in its container.
fn refuse_root(ids: Option<(u32, u32)>) -> Result<(), Refusal> {
    match ids {
        Some((0, _)) => Err(Refusal::new(
            codes::HOST_IS_ROOT,
            "kwnt is running as root, so Claude can't run inside the VM as you. Run kwnt as a normal user",
        )),
        _ => Ok(()),
    }
}

/// The `docker run` arguments for a bot's container. The folder is mounted at its own absolute
/// path, so paths in the chat mean the same inside and out.
fn run_args(name: &str, root_label: &str, mounts: &Mounts, image: &str, ids: Option<(u32, u32)>, memory_mb: u32) -> Vec<String> {
    let mut a: Vec<String> = ["run", "-d", "--name", name].map(String::from).to_vec();
    a.extend(["--label".into(), "kwnt.agent=1".into(), "--label".into(), format!("kwnt.root={root_label}")]);
    a.extend(["--label".into(), format!("kwnt.workspace={}", broker::sha8(&mounts.workdir))]);
    a.extend(
        [
            "--network", "none", "--cap-drop", "ALL", "--security-opt", "no-new-privileges", "--read-only",
            "--tmpfs", "/tmp:rw,exec,nosuid,nodev,size=256m", "--cpus", "2", "--pids-limit", "512",
            "--restart", "no", "--hostname", "kwnt-agent",
        ]
        .map(String::from),
    );
    // No swap beyond the memory limit
    a.extend(["--memory".into(), format!("{memory_mb}m"), "--memory-swap".into(), format!("{memory_mb}m")]);
    // As the host's own user: files the bot makes belong to the person, no chown needed
    if let Some((uid, gid)) = ids {
        a.extend(["--user".into(), format!("{uid}:{gid}")]);
    }
    a.extend(["-e".into(), format!("HOME={CONTAINER_HOME}")]);
    for m in &mounts.args {
        a.extend(["--mount".into(), m.clone()]);
    }
    a.extend(["-w".into(), mounts.workdir.clone(), image.to_string()]);
    a
}

#[cfg(unix)]
fn host_ids() -> Option<(u32, u32)> {
    // SAFETY: getuid/getgid have no preconditions and can't fail
    Some(unsafe { (libc::getuid(), libc::getgid()) })
}

#[cfg(not(unix))]
fn host_ids() -> Option<(u32, u32)> {
    None
}

fn set_private(dir: &Path) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700));
    }
    #[cfg(not(unix))]
    let _ = dir;
}

fn sha_hex(s: &str) -> String {
    use sha2::{Digest, Sha256};
    Sha256::digest(s.as_bytes()).iter().map(|b| format!("{b:02x}")).collect()
}

/// A bot's folder is mounted into the container, so it must not hold the person's secrets
/// ([`folder_rule`]: D5, with kwnt's own work folders inside its data dir allowed). `folder` is
/// already canonical.
pub fn check_folder(folder: &Path, places: &folder_rule::Places) -> Result<(), Refusal> {
    let Some(kind) = folder_rule::classify(folder, places) else { return Ok(()) };
    let why = match kind {
        folder_rule::UnsafeKind::Home => "is your home folder",
        folder_rule::UnsafeKind::ContainsHome | folder_rule::UnsafeKind::Root => "holds your home folder",
        folder_rule::UnsafeKind::SignIns => "holds sign-ins",
        folder_rule::UnsafeKind::KwantData => "holds kwant's own data",
        folder_rule::UnsafeKind::Docker => "holds Docker's control socket",
        folder_rule::UnsafeKind::SharedFolder => "is the folder every team without its own folder shares",
    };
    let mut r = Refusal::new(
        codes::UNSAFE_FOLDER,
        format!("The bot's folder {} {why}, so kwant won't share it with the VM. Pick a project folder instead", folder.display()),
    );
    r.unsafe_kind = Some(kind);
    Err(r)
}

fn check_credential(c: &Credential) -> Result<(), Refusal> {
    match c {
        Credential::ApiKey(k) if k.expose().trim().is_empty() => Err(no_credential()),
        Credential::ApiKey(_) => Ok(()),
        Credential::OAuthFile { path, refresh } => {
            let doc: Value = std::fs::read(path)
                .ok()
                .and_then(|b| serde_json::from_slice(&b).ok())
                .ok_or_else(no_credential)?;
            let o = &doc["claudeAiOauth"];
            if o["accessToken"].as_str().is_none_or(str::is_empty) {
                return Err(no_credential());
            }
            let expired = o["expiresAt"].as_i64().is_some_and(|t| t <= now_ms());
            let refreshable = *refresh && o["refreshToken"].as_str().is_some_and(|s| !s.is_empty());
            if expired && !refreshable {
                return Err(Refusal::new(codes::SIGN_IN_EXPIRED, "The Claude sign-in has expired. Sign in to Claude again"));
            }
            Ok(())
        }
    }
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

fn no_credential() -> Refusal {
    Refusal::new(codes::NO_CREDENTIAL, "There's no Claude sign-in or API key on this computer for this bot")
}

/// The real Claude credential for a turn. `profile_home`: the turn's sign-in profile home (the
/// broker owns refreshing it); `None`: this computer's own login (never refreshed by kwnt).
pub fn resolve_credential(profile_home: Option<&Path>) -> Result<(Credential, Option<PathBuf>), Refusal> {
    resolve_credential_in(profile_home, std::env::var("ANTHROPIC_API_KEY").ok(), std::env::var_os("CLAUDE_CONFIG_DIR").map(PathBuf::from), dirs::home_dir())
}

pub fn resolve_credential_in(
    profile_home: Option<&Path>,
    env_key: Option<String>,
    config_dir: Option<PathBuf>,
    home: Option<PathBuf>,
) -> Result<(Credential, Option<PathBuf>), Refusal> {
    if let Some(h) = profile_home {
        let creds = h.join(".credentials.json");
        if creds.is_file() {
            return Ok((Credential::OAuthFile { path: creds, refresh: true }, Some(h.to_path_buf())));
        }
        if let Some(k) = primary_api_key(&h.join(".claude.json")) {
            return Ok((Credential::ApiKey(Secret::new(k)), Some(h.to_path_buf())));
        }
        return Err(no_credential());
    }
    if let Some(k) = env_key.filter(|k| !k.trim().is_empty()) {
        return Ok((Credential::ApiKey(Secret::new(k)), None));
    }
    let dir = config_dir.clone().or_else(|| home.as_ref().map(|h| h.join(".claude")));
    if let Some(d) = &dir {
        let creds = d.join(".credentials.json");
        if creds.is_file() {
            return Ok((Credential::OAuthFile { path: creds, refresh: false }, Some(d.clone())));
        }
    }
    let json = match (&config_dir, &home) {
        (Some(d), _) => Some(d.join(".claude.json")),
        (None, Some(h)) => Some(h.join(".claude.json")),
        _ => None,
    };
    if let Some(k) = json.as_deref().and_then(primary_api_key) {
        return Ok((Credential::ApiKey(Secret::new(k)), dir));
    }
    Err(no_credential())
}

fn primary_api_key(path: &Path) -> Option<String> {
    let doc: Value = serde_json::from_slice(&std::fs::read(path).ok()?).ok()?;
    doc["primaryApiKey"].as_str().filter(|k| !k.trim().is_empty()).map(String::from)
}

/// Environment the CLI keeps inside the container; everything else kwnt set is dropped.
const PASS_ENV: &[&str] = &["CLAUDE_BASH_MAINTAIN_PROJECT_WORKING_DIR", "MCP_TOOL_TIMEOUT", "MCP_TIMEOUT", "BROWSER", "DISPLAY"];

/// Provider and model settings that would point the CLI somewhere else (as for a profile run).
const SETTINGS_OVERRIDE: &[(&str, &str)] = &[
    ("CLAUDE_CODE_USE_BEDROCK", "0"),
    ("CLAUDE_CODE_USE_VERTEX", "0"),
    ("CLAUDE_CODE_USE_FOUNDRY", "0"),
    ("ANTHROPIC_MODEL", ""),
    ("ANTHROPIC_DEFAULT_HAIKU_MODEL", ""),
    ("ANTHROPIC_DEFAULT_SONNET_MODEL", ""),
    ("ANTHROPIC_DEFAULT_OPUS_MODEL", ""),
    ("ANTHROPIC_DEFAULT_FABLE_MODEL", ""),
    ("ANTHROPIC_SMALL_FAST_MODEL", ""),
    ("CLAUDE_CODE_SUBAGENT_MODEL", ""),
];

/// A Claude turn prepared to run in the bot's container. Dropping it ends the turn: its broker
/// key stops working, the CLI's process group in the container is stopped and its MCP config
/// removed.
pub struct IsolatedTurn {
    vm: Arc<AgentVm>,
    lease: TurnLease,
    container: String,
    workdir: PathBuf,
    run_dir: PathBuf,
    oauth: bool,
    kwnt_origin: Option<String>,
    mcp_file: Option<crate::mcp::cli_config::ConfigFile>,
    /// This turn's place among the isolated turns at once; freed when the turn drops.
    _permit: Option<OwnedSemaphorePermit>,
}

impl std::fmt::Debug for IsolatedTurn {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("IsolatedTurn").field("container", &self.container).field("lease", &self.lease).finish_non_exhaustive()
    }
}

impl IsolatedTurn {
    pub fn container(&self) -> &str {
        &self.container
    }

    pub fn workdir(&self) -> &Path {
        &self.workdir
    }

    /// Whether the turn asked for more model calls than its cap (the CLI then got a 429).
    pub fn cap_reached(&self) -> bool {
        self.lease.cap_reached()
    }

    /// The turn's request cap.
    pub fn request_cap(&self) -> u32 {
        self.lease.cap()
    }

    /// Model requests the turn made so far.
    pub fn requests(&self) -> u32 {
        self.lease.requests()
    }

    fn base(&self) -> String {
        format!("{IN_CONTAINER_ORIGIN}/t/{}", self.lease.key())
    }

    pub fn anthropic_base_url(&self) -> String {
        format!("{}/anthropic", self.base())
    }

    fn pid_file(&self) -> String {
        format!("/tmp/kwnt-turn-{}.pid", &self.lease.key()[..12])
    }

    /// Writes this turn's `--mcp-config` where the container sees it and returns its path in
    /// the container, and the names of servers left out. Only kwnt's own endpoints come along
    /// (through the broker); the container has no network for anything else.
    pub fn write_mcp_config(&mut self, ctx: &crate::mcp::McpTurnContext) -> std::io::Result<(String, Vec<String>)> {
        let (doc, dropped) = self.mcp_document(ctx);
        let name = format!("mcp-{}.json", &self.lease.key()[..16]);
        let path = self.run_dir.join(&name);
        broker::write_private(&path, serde_json::to_string_pretty(&doc)?.as_bytes())?;
        self.mcp_file = Some(crate::mcp::cli_config::ConfigFile { path });
        Ok((format!("{CONTAINER_RUN}/{name}"), dropped))
    }

    fn mcp_document(&self, ctx: &crate::mcp::McpTurnContext) -> (Value, Vec<String>) {
        let mut doc = crate::mcp::cli_config::claude_config(ctx);
        let mut dropped = Vec::new();
        let origin = self.kwnt_origin.as_deref().map(|o| o.trim_end_matches('/').to_string());
        if let Some(servers) = doc["mcpServers"].as_object_mut() {
            let names: Vec<String> = servers.keys().cloned().collect();
            for name in names {
                let entry = &servers[&name];
                let local = (entry["type"] == "http")
                    .then(|| entry["url"].as_str())
                    .flatten()
                    .zip(origin.as_deref())
                    .and_then(|(url, o)| url.strip_prefix(o).filter(|rest| rest.starts_with("/api/")).map(String::from));
                let local = local.filter(|rest| {
                    let path = rest.split_once('?').map_or(rest.as_str(), |(p, _)| p);
                    self.lease.allow_kwnt_path(path)
                });
                match local {
                    Some(rest) => {
                        let (path, query) = rest.split_once('?').map_or((rest.as_str(), None), |(p, q)| (p, Some(q)));
                        let url = format!("{}/kwnt{path}{}", self.base(), query.map(|q| format!("?{q}")).unwrap_or_default());
                        servers[&name]["url"] = json!(url);
                    }
                    None => {
                        servers.remove(&name);
                        dropped.push(name);
                    }
                }
            }
        }
        (doc, dropped)
    }

    /// The `docker exec` that runs `cmd` (kwnt's claude command, built as for this computer) in
    /// the bot's container instead. Only [`PASS_ENV`] survives; the home, broker address and a
    /// stand-in credential are set; provider settings are pinned to the broker.
    pub fn wrap(&self, cmd: &Command) -> Command {
        let std = cmd.as_std();
        let mut out = Command::new("docker");
        out.args(["exec", "-w"]).arg(&self.workdir);
        let mut env: Vec<(String, String)> = std
            .get_envs()
            .filter_map(|(k, v)| {
                let k = k.to_str()?;
                PASS_ENV.contains(&k).then(|| (k.to_string(), v.map(|v| v.to_string_lossy().into_owned()).unwrap_or_default()))
            })
            .collect();
        let base_url = self.anthropic_base_url();
        env.extend([
            ("HOME".to_string(), CONTAINER_HOME.to_string()),
            ("CLAUDE_CONFIG_DIR".into(), format!("{CONTAINER_HOME}/.claude")),
            ("ANTHROPIC_BASE_URL".into(), base_url.clone()),
            ("CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC".into(), "1".into()),
            ("DISABLE_AUTOUPDATER".into(), "1".into()),
            ("DISABLE_TELEMETRY".into(), "1".into()),
            ("DISABLE_ERROR_REPORTING".into(), "1".into()),
        ]);
        if self.oauth {
            env.push(("CLAUDE_CODE_OAUTH_TOKEN".into(), DUMMY_OAUTH_TOKEN.into()));
        } else {
            env.push(("ANTHROPIC_API_KEY".into(), DUMMY_API_KEY.into()));
        }
        for (k, v) in &env {
            out.arg("-e").arg(format!("{k}={v}"));
        }
        out.arg(&self.container);
        // A process group of its own, its id written down, so the turn's end can stop it all
        out.args(["setsid", "-w", "sh", "-c"]);
        out.arg(format!("echo $$ > {}; exec \"$@\"", self.pid_file()));
        out.args(["sh", "claude"]);
        // Claude's own web tools only reach the broker's 400 (D3): start without them
        let args = notice::with_disallowed_tools(std.get_args().map(OsString::from).collect());
        out.args(&args);
        let mut settings: serde_json::Map<String, Value> = SETTINGS_OVERRIDE.iter().map(|(k, v)| (k.to_string(), json!(v))).collect();
        settings.insert("ANTHROPIC_BASE_URL".into(), json!(base_url));
        out.arg("--settings").arg(json!({ "env": settings }).to_string());
        out
    }
}

impl Drop for IsolatedTurn {
    fn drop(&mut self) {
        let last = self.vm.release(&self.container);
        // Killing `docker exec` doesn't stop what it started in the container: TERM the turn's
        // process group, then, after the container's last turn, remove the container (R-9)
        let (vm, container) = (self.vm.clone(), self.container.clone());
        let script = format!("p=$(cat {f} 2>/dev/null) && kill -TERM -- -$p 2>/dev/null; rm -f {f}", f = self.pid_file());
        if let Ok(rt) = tokio::runtime::Handle::try_current() {
            rt.spawn(async move {
                let _ = Command::new("docker").args(["exec", &container, "sh", "-c", &script]).kill_on_drop(true).output().await;
                if last {
                    vm.end_container(&container).await;
                }
            });
        }
        // Without a runtime (kwnt is exiting) the reaper removes it at the next start
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_switch_refusals_read_as_words_not_codes() {
        assert_eq!(toggle_message("mac_later"), "Coming to Mac later. For now, Claude runs on this computer in both modes.");
        assert_eq!(toggle_message("not_supported_yet"), "Only on Linux for now. Claude runs on this computer in both modes.");
        assert_eq!(toggle_message("docker_unavailable"), "Start Docker to turn this on.");
        assert!(!toggle_message("brand_new").contains('_'));
        // The chip detail after a failed setup (fix 7)
        assert!(SETUP_FAILED.starts_with("the setup failed") && !SETUP_FAILED.contains("computer") && !SETUP_FAILED.contains("kwnt"));
    }

    #[test]
    fn the_chips_follow_miras_contract_and_d1() {
        let on = ViewInputs { setting_on: true, vm_mode: true, docker: true, cli_account: true };
        let st = |inp: ViewInputs, img: ImageView| {
            let a = claude_state(inp, || img);
            (a.state, a.reason)
        };
        if cfg!(target_os = "linux") {
            assert_eq!(st(on, ImageView::Ready), ("isolated", None));
            assert_eq!(st(on, ImageView::Building), ("preparing", None));
            assert_eq!(st(on, ImageView::Missing), ("unavailable", Some("download_needed")));
            let failed = claude_state(on, || ImageView::Failed("no network".into()));
            assert_eq!((failed.state, failed.reason, failed.detail.as_deref()), ("unavailable", Some("download_failed"), Some("no network")));
            // D1: outside vm mode the switch does nothing, and says so
            assert_eq!(st(ViewInputs { vm_mode: false, ..on }, ImageView::Ready), ("not_isolated", Some("local_mode")));
            assert_eq!(st(ViewInputs { docker: false, ..on }, ImageView::Ready), ("unavailable", Some("docker_unavailable")));
            assert_eq!(st(ViewInputs { cli_account: false, ..on }, ImageView::Ready).0, "not_isolated");
            assert_eq!(toggle(true), Toggle { available: true, reason: None });
            assert_eq!(toggle(false), Toggle { available: false, reason: Some("docker_unavailable") });
        }
        if cfg!(target_os = "macos") {
            assert_eq!(st(on, ImageView::Ready), ("off", Some("mac_later")));
            assert_eq!(toggle(true).reason, Some("mac_later"));
        }
        assert_eq!(st(ViewInputs { setting_on: false, ..on }, ImageView::Ready), ("off", None));
        assert_eq!((other_state().state, other_state().reason), ("not_isolated", Some("not_supported_yet")));
        let json = serde_json::to_value(claude_state(ViewInputs { setting_on: false, ..on }, || ImageView::Ready)).unwrap();
        assert_eq!(json, serde_json::json!({"state": "off", "reason": null, "detail": null, "progress": null}));
    }

    #[test]
    fn folders_holding_secrets_are_refused() {
        let places = folder_rule::Places {
            home: Some("/home/p".into()),
            data_dir: "/home/p/.kwnt".into(),
            sign_ins: vec!["/home/p/.kwnt/profiles".into(), "/srv/creds/claude".into()],
            docker: vec![],
        };
        for (bad, kind) in [("/", "root"), ("/home", "contains_home"), ("/home/p", "home"), ("/home/p/.kwnt", "kwant_data"), ("/srv", "sign_ins"), ("/srv/creds", "sign_ins"), ("/home/p/.kwnt/workspace", "shared_folder")] {
            let r = check_folder(Path::new(bad), &places).unwrap_err();
            assert_eq!(r.code, codes::UNSAFE_FOLDER, "{bad}");
            assert_eq!(r.unsafe_kind.map(folder_rule::UnsafeKind::as_str), Some(kind), "{bad}");
        }
        for ok in ["/home/p/projects/site", "/home/p/.kwnt/workspaces/b1", "/home/p/.kwnt/bot-workspaces/kw_1", "/srv/other"] {
            assert!(check_folder(Path::new(ok), &places).is_ok(), "{ok}");
        }
    }

    #[test]
    fn run_args_isolate_the_container() {
        let m = Mounts::new(Path::new("/w/proj"), Path::new("/d/h"), Path::new("/d/run")).unwrap();
        let a = run_args("kwnt-agent-a-b", "r", &m, "img", Some((1000, 1000)), 1024);
        let s = a.join(" ");
        for want in [
            "--network none", "--cap-drop ALL", "--security-opt no-new-privileges", "--read-only", "--user 1000:1000",
            "--memory 1024m", "--memory-swap 1024m", "-w /w/proj img", "--label kwnt.agent=1",
            r#"--mount type=bind,"src=/w/proj","dst=/w/proj""#,
            r#"--mount type=bind,"src=/d/h","dst=/home/agent""#,
            r#"--mount type=bind,"src=/d/run","dst=/run/kwnt",readonly"#,
        ] {
            assert!(s.contains(want), "{want} in {s}");
        }
        // Exactly three mounts, and no other way in
        assert_eq!(a.iter().filter(|x| *x == "--mount").count(), 3);
        for never in ["-v", "--volume", "--volumes-from", "--privileged", "--device", "--pid", "--ipc", "--userns", "--add-host"] {
            assert!(!a.iter().any(|x| x == never), "{never}");
        }
        assert!(!s.contains("docker.sock"));
    }

    #[test]
    fn mount_values_cant_add_options_and_odd_names_are_refused() {
        // A comma or quote stays inside the src field (Docker reads --mount as CSV)
        let m = Mounts::new(Path::new("/w/a,dst=/etc,readonly=false"), Path::new("/d/h"), Path::new("/d/run")).unwrap();
        assert_eq!(m.args[0], r#"type=bind,"src=/w/a,dst=/etc,readonly=false","dst=/w/a,dst=/etc,readonly=false""#);
        let q = Mounts::new(Path::new("/w/say \"hi\": x=1"), Path::new("/d/h"), Path::new("/d/run")).unwrap();
        assert_eq!(q.args[0], r#"type=bind,"src=/w/say ""hi"": x=1","dst=/w/say ""hi"": x=1""#);
        for bad in ["/w/new\nline", "relative/dir", "/w/tab\there"] {
            assert_eq!(Mounts::new(Path::new(bad), Path::new("/d/h"), Path::new("/d/run")).unwrap_err().code, codes::FOLDER_NAME_UNSUPPORTED, "{bad:?}");
        }
        #[cfg(unix)]
        {
            use std::os::unix::ffi::OsStrExt;
            let p = Path::new(std::ffi::OsStr::from_bytes(b"/w/\xff"));
            assert_eq!(Mounts::new(p, Path::new("/d/h"), Path::new("/d/run")).unwrap_err().code, codes::FOLDER_NAME_UNSUPPORTED);
        }
    }

    #[test]
    fn the_container_never_runs_as_root() {
        assert_eq!(refuse_root(Some((0, 0))).unwrap_err().code, codes::HOST_IS_ROOT);
        assert!(refuse_root(Some((1000, 1000))).is_ok());
    }

    #[tokio::test]
    async fn a_turn_that_waits_for_a_spot_says_so_and_when_it_gets_one() {
        let d = tempfile::tempdir().unwrap();
        let vm = AgentVm::with_parts(d.path(), BrokerConfig::new(d.path().join("broker.log")), None, AgentsInVmConfig { max_turns: 1, ..Default::default() }, None);
        // A free spot: no report at all
        let seen = Arc::new(StdMutex::new(Vec::new()));
        let log = |seen: &Arc<StdMutex<Vec<TurnWait>>>| {
            let seen = seen.clone();
            move |w| {
                seen.lock().unwrap().push(w);
                std::future::ready(())
            }
        };
        let first = vm.turn_permit(&mut log(&seen)).await.unwrap();
        assert!(seen.lock().unwrap().is_empty());
        // Taken: the next one is first in line, then the one after it second
        let (vm2, s2) = (vm.clone(), seen.clone());
        let second = tokio::spawn(async move { vm2.turn_permit(&mut log(&s2)).await.map(|_| ()) });
        tokio::time::sleep(Duration::from_millis(50)).await;
        assert_eq!(*seen.lock().unwrap(), vec![TurnWait::Queued { position: 1, slots: 1 }]);
        let third_seen = Arc::new(StdMutex::new(Vec::new()));
        let (vm3, s3) = (vm.clone(), third_seen.clone());
        let third = tokio::spawn(async move { vm3.turn_permit(&mut log(&s3)).await.map(|_| ()) });
        tokio::time::sleep(Duration::from_millis(50)).await;
        assert_eq!(*third_seen.lock().unwrap(), vec![TurnWait::Queued { position: 2, slots: 1 }]);
        // A stopped reply leaves the line
        third.abort();
        let _ = third.await;
        assert_eq!(vm.waiting.load(std::sync::atomic::Ordering::SeqCst), 1);
        drop(first);
        second.await.unwrap().unwrap();
        assert_eq!(*seen.lock().unwrap(), vec![TurnWait::Queued { position: 1, slots: 1 }, TurnWait::Started]);
        assert_eq!(vm.waiting.load(std::sync::atomic::Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn at_most_max_turns_hold_a_permit_and_every_drop_frees_one() {
        let d = tempfile::tempdir().unwrap();
        let vm = AgentVm::with_parts(d.path(), BrokerConfig::new(d.path().join("broker.log")), None, AgentsInVmConfig { max_turns: 2, ..Default::default() }, None);
        let p1 = vm.turns.clone().acquire_owned().await.unwrap();
        let p2 = vm.turns.clone().acquire_owned().await.unwrap();
        assert_eq!(vm.free_turn_slots(), 0);
        let vm2 = vm.clone();
        let third = tokio::spawn(async move { vm2.turns.clone().acquire_owned().await.unwrap() });
        tokio::time::sleep(Duration::from_millis(50)).await;
        assert!(!third.is_finished(), "a third turn waits");
        // A turn that ends (even by panic or cancel) drops its IsolatedTurn and with it the permit
        let turn = test_turn(&vm, Some(p1), d.path());
        drop(turn);
        let p3 = tokio::time::timeout(Duration::from_secs(2), third).await.unwrap().unwrap();
        drop((p2, p3));
        assert_eq!(vm.free_turn_slots(), 2);
    }

    fn test_turn(vm: &Arc<AgentVm>, permit: Option<OwnedSemaphorePermit>, dir: &Path) -> IsolatedTurn {
        let lease = vm.broker().register(TurnSpec {
            bot: "b".into(),
            credential: Credential::ApiKey(Secret::new("sk-real")),
            kwnt_origin: Some("http://127.0.0.1:8790".into()),
            cap: 5,
        });
        IsolatedTurn {
            vm: vm.clone(),
            lease,
            container: "kwnt-agent-x".into(),
            workdir: "/w/proj".into(),
            run_dir: dir.join("run"),
            oauth: false,
            kwnt_origin: Some("http://127.0.0.1:8790".into()),
            mcp_file: None,
            _permit: permit,
        }
    }

    #[test]
    fn the_request_limit_and_memory_come_from_server_config() {
        let d = tempfile::tempdir().unwrap();
        let limits = AgentsInVmConfig { request_limit: 7, memory_mb: 2048, max_turns: 3 };
        let vm = AgentVm::with_parts(d.path(), BrokerConfig::new(d.path().join("broker.log")), None, limits.clone(), None);
        assert_eq!(vm.limits(), &limits);
        assert_eq!(vm.free_turn_slots(), 3);
        assert_eq!(AgentsInVmConfig::default(), AgentsInVmConfig { request_limit: 200, memory_mb: 1024, max_turns: 2 });
    }

    #[test]
    fn credentials_resolve_profile_first_and_never_refresh_the_hosts() {
        let d = tempfile::tempdir().unwrap();
        let prof = d.path().join("prof");
        std::fs::create_dir_all(&prof).unwrap();
        std::fs::write(prof.join(".credentials.json"), r#"{"claudeAiOauth":{"accessToken":"a"}}"#).unwrap();
        let (c, home) = resolve_credential_in(Some(&prof), Some("env".into()), None, None).unwrap();
        assert!(matches!(c, Credential::OAuthFile { refresh: true, .. }));
        assert_eq!(home.as_deref(), Some(prof.as_path()));

        let (c, _) = resolve_credential_in(None, Some("sk-env".into()), None, Some(d.path().into())).unwrap();
        assert!(matches!(c, Credential::ApiKey(ref k) if k.expose() == "sk-env"));

        let h = d.path().join("home");
        std::fs::create_dir_all(h.join(".claude")).unwrap();
        std::fs::write(h.join(".claude/.credentials.json"), r#"{"claudeAiOauth":{"accessToken":"a"}}"#).unwrap();
        let (c, _) = resolve_credential_in(None, None, None, Some(h.clone())).unwrap();
        assert!(matches!(c, Credential::OAuthFile { refresh: false, .. }));

        std::fs::remove_file(h.join(".claude/.credentials.json")).unwrap();
        std::fs::write(h.join(".claude.json"), r#"{"primaryApiKey":"sk-console"}"#).unwrap();
        let (c, _) = resolve_credential_in(None, None, None, Some(h.clone())).unwrap();
        assert!(matches!(c, Credential::ApiKey(ref k) if k.expose() == "sk-console"));

        let empty = d.path().join("empty");
        std::fs::create_dir_all(&empty).unwrap();
        assert_eq!(resolve_credential_in(None, None, None, Some(empty.clone())).unwrap_err().code, codes::NO_CREDENTIAL);
        assert_eq!(resolve_credential_in(Some(&empty), Some("env".into()), None, None).unwrap_err().code, codes::NO_CREDENTIAL);
    }

    #[test]
    fn an_expired_host_sign_in_is_refused_but_a_profiles_is_refreshable() {
        let d = tempfile::tempdir().unwrap();
        let f = d.path().join(".credentials.json");
        std::fs::write(&f, r#"{"claudeAiOauth":{"accessToken":"a","refreshToken":"r","expiresAt":1}}"#).unwrap();
        assert_eq!(check_credential(&Credential::OAuthFile { path: f.clone(), refresh: false }).unwrap_err().code, codes::SIGN_IN_EXPIRED);
        assert!(check_credential(&Credential::OAuthFile { path: f, refresh: true }).is_ok());
    }

    #[test]
    fn the_os_gate_says_mac_later() {
        let r = supported_os();
        if cfg!(target_os = "linux") {
            assert!(r.is_ok());
        } else if cfg!(target_os = "macos") {
            assert_eq!(r.unwrap_err().message, "Coming to Mac later");
        }
    }

    #[tokio::test]
    async fn wrap_keeps_only_allowed_env_and_points_claude_at_the_broker() {
        let d = tempfile::tempdir().unwrap();
        let vm = AgentVm::with_parts(d.path(), BrokerConfig::new(d.path().join("broker.log")), None, AgentsInVmConfig::default(), None);
        let mut turn = test_turn(&vm, None, d.path());
        let key = turn.lease.key().to_string();
        let mut cmd = Command::new("/host/claude");
        cmd.env("CLAUDE_BASH_MAINTAIN_PROJECT_WORKING_DIR", "1")
            .env("ANTHROPIC_API_KEY", "sk-real")
            .env("CLAUDE_CONFIG_DIR", "/home/p/.claude")
            .env("AWS_SECRET_ACCESS_KEY", "x")
            .args(["--model", "haiku", "-p", "hi"]);
        let w = turn.wrap(&cmd);
        let args: Vec<String> = w.as_std().get_args().map(|a| a.to_string_lossy().into_owned()).collect();
        let all = args.join("\u{1}");
        assert_eq!(w.as_std().get_program(), "docker");
        assert!(!all.contains("sk-real") && !all.contains("AWS_SECRET") && !all.contains("/home/p/.claude"), "{args:?}");
        assert!(args.contains(&format!("ANTHROPIC_BASE_URL=http://127.0.0.1:8080/t/{key}/anthropic")));
        assert!(args.contains(&format!("ANTHROPIC_API_KEY={DUMMY_API_KEY}")));
        assert!(args.contains(&"CLAUDE_BASH_MAINTAIN_PROJECT_WORKING_DIR=1".to_string()));
        assert!(args.windows(2).any(|p| p == ["exec", "-w"]));
        let settings = args.last().unwrap();
        assert!(settings.contains(&format!("/t/{key}/anthropic")) && settings.contains("CLAUDE_CODE_USE_BEDROCK"));
        assert!(w.as_std().get_envs().next().is_none(), "docker itself gets no extra env");

        // MCP: kwnt's own endpoints go through the broker; outside servers are left out
        std::fs::create_dir_all(d.path().join("run")).unwrap();
        let server = |name: &str, url: &str| crate::db::models::McpServer {
            id: format!("id-{name}"),
            name: name.into(),
            transport: "http".into(),
            command: None,
            args: vec![],
            url: Some(url.into()),
            env: Default::default(),
            headers: Default::default(),
            enabled: true,
            timeout_secs: 30,
            created_at: None,
        };
        let ctx = crate::mcp::McpTurnContext {
            servers: vec![
                server("notion", "https://mcp.notion.com/mcp"),
                server("kwnt_tools", "http://127.0.0.1:8790/api/mcp/self"),
                server("kwnt_other", "http://127.0.0.1:8790/api/settings/computer"),
            ],
            approver: Arc::new(crate::mcp::FixedApprover(true)),
            config_dir: d.path().join("cfg"),
            permission_url: Some("http://127.0.0.1:8790/api/mcp/permission/perm456".into()),
            gateway: None,
            max_tool_iterations: 4,
            smart_approvals: false,
            records: Default::default(),
        };
        let (path, mut dropped) = turn.write_mcp_config(&ctx).unwrap();
        dropped.sort();
        assert!(path.starts_with("/run/kwnt/mcp-"));
        assert_eq!(dropped, vec!["kwnt_other".to_string(), "notion".to_string()]);
        let host_file = turn.mcp_file.as_ref().unwrap().path.clone();
        let doc: Value = serde_json::from_str(&std::fs::read_to_string(&host_file).unwrap()).unwrap();
        assert_eq!(doc["mcpServers"]["kwnt_tools"]["url"], format!("http://127.0.0.1:8080/t/{key}/kwnt/api/mcp/self"));
        assert!(doc["mcpServers"]["kwnt"]["url"].as_str().unwrap().ends_with("/kwnt/api/mcp/permission/perm456"));
        drop(turn);
        assert!(!host_file.exists());
    }
}
