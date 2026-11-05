//! Managed Laya sidecar: installs `laya-serve` into the kwnt data dir on first run,
//! starts it next to the server, restarts it if it dies and reports its status.
//!
//! Two engines (the `laya_engine` setting, [`LayaEngine`]):
//! - **standard**: `laya-serve` 0.4.8 on the configured port, in `<data>/laya/venv`.
//! - **vega** (the default): laya 0.4.24's vegaml engine, which only Python can turn on, through
//!   kwant's own small server (`vega_serve.py`, embedded) in `<data>/laya/venv-vega`, on the next port.
//!
//! The standard engine always starts first, so routing works within a minute. The vega lane then
//! installs and starts vega in the background, and routing moves to it only once its `/health`
//! answers; until then 0.4.8 keeps serving. When vega can't run here (no Python 3.10+, under 8 GB
//! of memory, a failed install, no answer in time, two crashes) routing stays on 0.4.8, the status
//! says why, and vega is tried again only after a kwant update or a change of the setting.

use super::LayaRouter;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;
use tokio::process::{Child, Command};
use tokio::sync::watch;
use tokio_util::sync::CancellationToken;

/// Pinned so every install routes the same way; bump deliberately.
pub const LAYA_PACKAGE: &str = "laya[serve]==0.4.8";
/// The vega engine's laya: vegaml ships with laya since 0.4.23. Its own venv, so 0.4.8 is untouched.
pub const VEGA_PACKAGE: &str = "laya[serve]==0.4.24";
/// `<data>/laya/<VEGA_VENV>`.
pub const VEGA_VENV: &str = "venv-vega";
/// Setting key: `"standard"` or `"vega"` (the default).
pub const ENGINE_SETTING: &str = "laya_engine";
/// Below this much memory the vega engine (a 0.8B reader, 2–4 GB resident) isn't tried.
pub const VEGA_MIN_MEMORY_GB: f64 = 8.0;
/// Torch threads the vega engine may use, so it never takes the whole machine.
pub const VEGA_TORCH_THREADS: u32 = 4;
/// Crashes in one kwant run after which vega gives way to the standard engine.
pub const VEGA_MAX_CRASHES: u32 = 2;
/// The server kwant runs for the vega engine; written to `<data>/laya/vega_serve.py` on each start.
pub const VEGA_SERVE_PY: &str = include_str!("vega_serve.py");
/// Why vega fell back, for which kwant version: `<data>/laya/vega-fallback.json`.
pub const FALLBACK_FILE: &str = "vega-fallback.json";
/// Inside the vega venv once the install finished; holds [`VEGA_PACKAGE`].
const INSTALLED_FILE: &str = ".kwnt-installed";
const VERSION: &str = env!("CARGO_PKG_VERSION");

/// Which Laya engine routes (the `laya_engine` setting).
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum LayaEngine {
    /// laya-serve 0.4.8.
    Standard,
    /// laya 0.4.24 with vegaml, falling back to the standard engine when it can't run.
    #[default]
    Vega,
}

impl LayaEngine {
    /// The saved setting: anything but `"standard"` (including nothing saved) is vega.
    pub fn from_setting(value: Option<&str>) -> Self {
        match value.map(|v| v.trim().to_ascii_lowercase()).as_deref() {
            Some("standard") => Self::Standard,
            _ => Self::Vega,
        }
    }

    /// A value the settings API accepts.
    pub fn parse(value: &str) -> Option<Self> {
        match value.trim() {
            "standard" => Some(Self::Standard),
            "vega" => Some(Self::Vega),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Standard => "standard",
            Self::Vega => "vega",
        }
    }
}

/// Where the vega engine is, for the status.
#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum VegaState {
    /// Not wanted (the setting is `standard`, or Laya isn't run by kwant).
    #[default]
    Off,
    /// Waiting for the standard engine to start first.
    Waiting,
    Installing,
    Starting,
    /// Answering routing questions.
    Active,
    /// Can't run here; the standard engine routes. `detail` says why.
    Fallback,
}

#[derive(Debug, Clone, Serialize, Default)]
pub struct VegaStatus {
    pub state: VegaState,
    pub detail: String,
    /// Size of the downloaded vega model, once it ran.
    pub model_mb: Option<u64>,
    /// Where that model is cached.
    pub model_cache: Option<String>,
    /// Crashes in this kwant run.
    pub crashes: u32,
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum LayaState {
    /// Not checked yet.
    Unknown,
    /// Reachable, but run by someone else (Docker, systemd, a dev shell).
    External,
    Installing,
    Starting,
    Online,
    Offline,
    Failed,
}

#[derive(Debug, Clone, Serialize)]
pub struct LayaStatus {
    pub state: LayaState,
    pub detail: String,
    pub url: String,
    pub managed: bool,
    /// The engine answering routing questions now.
    pub engine: LayaEngine,
    pub vega: VegaStatus,
}

impl LayaStatus {
    pub fn new(url: &str) -> Self {
        Self {
            state: LayaState::Unknown,
            detail: String::new(),
            url: url.to_string(),
            managed: false,
            engine: LayaEngine::Standard,
            vega: VegaStatus::default(),
        }
    }

    pub fn is_up(&self) -> bool {
        matches!(self.state, LayaState::Online | LayaState::External)
    }
}

#[derive(Debug, Clone, Default)]
pub struct SidecarOptions {
    pub data_dir: PathBuf,
    pub managed: bool,
    /// Comma list of Laya checkpoints to preload (`english`, `multilingual`, `typed-decisions`).
    pub models: String,
    /// The vega engine's port; `None` = the standard engine's port + 1 ([`vega_port`]).
    pub vega_port: Option<u16>,
    /// Run this instead of installing laya-serve (like `KWNT_LAYA_SERVE_BIN`).
    pub standard_bin: Option<PathBuf>,
    /// Where the vega engine's Python comes from.
    pub vega_python: VegaPython,
    /// Install `venv-vega` with these commands instead of [`crate::setup::vega_commands`] (tests).
    pub vega_install: Option<Vec<crate::setup::Cmd>>,
    /// Serve this script instead of [`VEGA_SERVE_PY`] (tests).
    pub vega_script: Option<String>,
    /// This machine's memory in GB; `None` = detect it.
    pub memory_gb: Option<f64>,
    pub timings: SidecarTimings,
}

/// How long the supervisor waits for things. Tests shorten them.
#[derive(Debug, Clone)]
pub struct SidecarTimings {
    /// Health checks while an engine starts.
    pub tick: Duration,
    /// Health checks once it answers.
    pub online_tick: Duration,
    /// How long vega waits for the standard engine to come up before going ahead anyway.
    pub settle: Duration,
    pub vega_install: Duration,
    /// From starting vega to its `/health` answering, the first model download included.
    pub vega_health: Duration,
    /// An active vega that doesn't answer `/health` this long counts as a crash.
    pub vega_unhealthy: Duration,
    /// Pause before starting vega again after its first crash.
    pub vega_restart: Duration,
}

impl Default for SidecarTimings {
    fn default() -> Self {
        Self {
            tick: Duration::from_secs(2),
            online_tick: Duration::from_secs(15),
            settle: Duration::from_secs(600),
            vega_install: Duration::from_secs(40 * 60),
            vega_health: Duration::from_secs(15 * 60),
            vega_unhealthy: Duration::from_secs(60),
            vega_restart: Duration::from_secs(10),
        }
    }
}

/// Which engine answers: the standard lane runs laya-serve only while this is `Standard`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Serving {
    Standard,
    Vega,
}

/// Starts the supervisor in the background. Cancel the token to stop the sidecar.
pub fn spawn(router: LayaRouter, opts: SidecarOptions, shutdown: CancellationToken) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move { supervise(router, opts, shutdown).await })
}

async fn supervise(router: LayaRouter, opts: SidecarOptions, shutdown: CancellationToken) {
    if router.is_reachable_at(&router.base_url).await {
        router.set_status(LayaState::External, "Using the Laya service already running", false);
        set_vega(&router, VegaState::Off, "Laya is run outside kwant, so kwant doesn't start the vega engine");
        router.warm_up().await;
        return monitor_external(router, shutdown).await;
    }

    let port = match local_port(&router.base_url) {
        Some(port) if opts.managed => port,
        _ => {
            let platform = crate::doctor::Platform::detect();
            let why = if opts.managed {
                let fix = match platform {
                    crate::doctor::Platform::Docker => crate::doctor::laya_fix(platform, false),
                    _ => "Start laya-serve at that address, or unset LAYA_URL so kwnt runs Laya here",
                };
                format!("Laya URL is not on this machine, so the app cannot start it. {fix}")
            } else {
                format!("Laya is not answering and the app is not managing it; using keyword routing. {}", crate::doctor::laya_fix(platform, false))
            };
            router.set_status(LayaState::Offline, &why, false);
            return monitor_external(router, shutdown).await;
        }
    };

    let dir = opts.data_dir.join("laya");
    let _ = std::fs::create_dir_all(&dir);
    let (serving_tx, serving_rx) = watch::channel(Serving::Standard);
    let (settled_tx, settled_rx) = watch::channel(false);
    tokio::join!(
        standard_lane(router.clone(), &opts, &dir, port, serving_rx, settled_tx, &shutdown),
        vega_lane(router.clone(), &opts, &dir, vega_port(&opts, port), serving_tx, settled_rx, &shutdown),
    );
}

/// The vega engine's port: [`SidecarOptions::vega_port`], else the standard engine's port + 1.
pub fn vega_port(opts: &SidecarOptions, standard_port: u16) -> u16 {
    opts.vega_port.unwrap_or_else(|| standard_port.saturating_add(1))
}

/// Runs laya-serve 0.4.8 whenever the vega engine isn't the one answering: first, so routing works
/// within a minute, and again after vega falls back or crashes. `settled` turns true once 0.4.8
/// answered (or couldn't start), which is when the vega lane may begin.
async fn standard_lane(
    router: LayaRouter,
    opts: &SidecarOptions,
    dir: &Path,
    port: u16,
    mut serving: watch::Receiver<Serving>,
    settled: watch::Sender<bool>,
    shutdown: &CancellationToken,
) {
    let mut backoff = Duration::from_secs(5);
    while !shutdown.is_cancelled() {
        if *serving.borrow_and_update() == Serving::Vega {
            tokio::select! {
                _ = shutdown.cancelled() => return,
                changed = serving.changed() => if changed.is_err() { return },
            }
            continue;
        }

        let bin = match ensure_installed(&router, dir, opts).await {
            Ok(bin) => bin,
            Err(e) => {
                let fix = crate::doctor::laya_fix(crate::doctor::Platform::detect(), true);
                router.set_status(LayaState::Failed, &format!("Install failed: {e}. {fix}"), true);
                settled.send_replace(true);
                if wait_or_cancel(shutdown, Duration::from_secs(300)).await {
                    return;
                }
                continue;
            }
        };

        router.set_status(
            LayaState::Starting,
            "Loading the routing model (the first start downloads it, ~420 MB)",
            true,
        );
        let log_path = dir.join("laya.log");
        let log_offset = std::fs::metadata(&log_path).map(|m| m.len()).unwrap_or(0);
        let mut child = match start(&bin, dir, port, &opts.models) {
            Ok(child) => child,
            Err(e) => {
                router.set_status(LayaState::Failed, &format!("Could not start laya-serve: {e}"), true);
                settled.send_replace(true);
                if wait_or_cancel(shutdown, backoff).await {
                    return;
                }
                continue;
            }
        };

        let started = std::time::Instant::now();
        let exit = run_until_exit(&router, &mut child, shutdown, &mut serving, &settled, &opts.timings).await;
        let status = match exit {
            StandardExit::Cancelled => {
                let _ = child.kill().await;
                return;
            }
            StandardExit::Handover => {
                let _ = child.kill().await;
                tracing::info!("vega engine answers now; laya-serve 0.4.8 stopped");
                backoff = Duration::from_secs(5);
                continue;
            }
            StandardExit::Exited(status) => status,
        };

        let reason = last_error_line_from(&log_path, log_offset)
            .unwrap_or_else(|| format!("laya-serve exited ({})", status.map(|s| s.to_string()).unwrap_or_default()));
        tracing::warn!(%reason, "Laya sidecar stopped; restarting in {:?}", backoff);
        if *serving.borrow() == Serving::Standard {
            router.set_status(LayaState::Failed, &reason, true);
        }
        settled.send_replace(true);

        if started.elapsed() > Duration::from_secs(600) {
            backoff = Duration::from_secs(5);
        }
        if wait_or_cancel(shutdown, backoff).await {
            return;
        }
        backoff = (backoff * 2).min(Duration::from_secs(300));
    }
}

enum StandardExit {
    Cancelled,
    /// The vega engine took over routing; stop 0.4.8.
    Handover,
    Exited(Option<std::process::ExitStatus>),
}

/// Waits for laya-serve to exit, flipping the status to online once its `/health` answers. Returns
/// early when kwant stops or when the vega engine takes over.
async fn run_until_exit(
    router: &LayaRouter,
    child: &mut Child,
    shutdown: &CancellationToken,
    serving: &mut watch::Receiver<Serving>,
    settled: &watch::Sender<bool>,
    timings: &SidecarTimings,
) -> StandardExit {
    let mut tick = tokio::time::interval(timings.tick);
    let mut online = false;
    loop {
        tokio::select! {
            _ = shutdown.cancelled() => return StandardExit::Cancelled,
            status = child.wait() => return StandardExit::Exited(status.ok()),
            changed = serving.changed() => {
                if changed.is_err() {
                    return StandardExit::Cancelled;
                }
                if *serving.borrow_and_update() == Serving::Vega {
                    return StandardExit::Handover;
                }
            }
            _ = tick.tick() => {
                let up = router.is_reachable_at(&router.base_url).await;
                if up && !online {
                    online = true;
                    tick = tokio::time::interval(timings.online_tick);
                    tracing::info!(url = %router.base_url, "Laya sidecar online");
                    router.warm_up_at(&router.base_url).await;
                    if *serving.borrow() == Serving::Standard {
                        router.update_status(|s| {
                            s.state = LayaState::Online;
                            s.managed = true;
                            s.engine = LayaEngine::Standard;
                            s.detail = online_detail(s);
                        });
                    }
                    settled.send_replace(true);
                } else if !up && online {
                    if *serving.borrow() == Serving::Standard {
                        router.set_status(LayaState::Starting, "Laya stopped answering; waiting for it", true);
                    }
                    online = false;
                }
            }
        }
    }
}

/// The status line while a model answers: which engine, and why vega isn't the one when it fell back.
fn online_detail(s: &LayaStatus) -> String {
    match (s.engine, s.vega.state) {
        (LayaEngine::Vega, _) => "Model-based routing is active (vega engine)".to_string(),
        (_, VegaState::Fallback) => format!("Model-based routing is active (standard engine; {})", s.vega.detail),
        (_, VegaState::Off) => "Model-based routing is active".to_string(),
        _ => "Model-based routing is active (standard engine while vega gets ready)".to_string(),
    }
}

/// Sets the vega engine's own state; the main status line follows while 0.4.8 answers.
fn set_vega(router: &LayaRouter, state: VegaState, detail: &str) {
    router.update_status(|s| {
        s.vega.state = state;
        s.vega.detail = detail.to_string();
        if s.engine == LayaEngine::Standard && s.state == LayaState::Online {
            s.detail = online_detail(s);
        }
    });
}

/// Why vega fell back, and on which kwant version: vega isn't tried again on the next start, only
/// after a kwant update or a change of the setting.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct FallbackRecord {
    pub reason: String,
    pub version: String,
    /// Unix seconds.
    #[serde(default)]
    pub at: i64,
}

impl FallbackRecord {
    pub fn read(dir: &Path) -> Option<Self> {
        serde_json::from_slice(&std::fs::read(dir.join(FALLBACK_FILE)).ok()?).ok()
    }

    fn write(dir: &Path, reason: &str) {
        let rec = Self { reason: reason.to_string(), version: VERSION.to_string(), at: chrono::Utc::now().timestamp() };
        if let Ok(json) = serde_json::to_vec_pretty(&rec) {
            let _ = std::fs::write(dir.join(FALLBACK_FILE), json);
        }
    }

    fn clear(dir: &Path) {
        let _ = std::fs::remove_file(dir.join(FALLBACK_FILE));
    }

    /// Whether this fallback still holds for `version`: it does until kwant is updated.
    pub fn holds_for(&self, version: &str) -> bool {
        self.version == version
    }
}

/// How the vega engine gets its Python.
#[derive(Debug, Clone, Default)]
pub enum VegaPython {
    /// uv (which fetches a Python 3.12) else a Python 3.10+ on the PATH; installs `venv-vega`.
    #[default]
    Detect,
    /// Run `vega_serve.py` with this Python as it is, without installing anything.
    Use(PathBuf),
    /// Behave as if neither uv nor a Python 3.10+ were found (tests).
    Missing,
}

/// Why vega can't run on this machine before anything is installed, or `None` when it can try.
pub fn vega_preflight(memory_gb: Option<f64>, has_python: bool) -> Option<String> {
    if let Some(gb) = memory_gb.filter(|gb| *gb < VEGA_MIN_MEMORY_GB) {
        return Some(format!(
            "vega needs {VEGA_MIN_MEMORY_GB:.0} GB of memory and this computer has {gb:.1} GB"
        ));
    }
    if !has_python {
        return Some("vega needs Python 3.10 or newer (or uv to fetch one) and neither was found".to_string());
    }
    None
}

enum VegaRun {
    Cancelled,
    /// The setting changed while vega was getting ready or running.
    EngineChanged,
    Fallback(String),
}

/// Runs the vega engine when the setting asks for it: after 0.4.8 is up, checks the machine,
/// installs `venv-vega`, starts `vega_serve.py` and moves routing to it once its `/health` answers.
/// Any reason it can't run sends routing back to 0.4.8 and is remembered until a kwant update or
/// a change of the setting.
async fn vega_lane(
    router: LayaRouter,
    opts: &SidecarOptions,
    dir: &Path,
    port: u16,
    serving: watch::Sender<Serving>,
    mut settled: watch::Receiver<bool>,
    shutdown: &CancellationToken,
) {
    let mut engine_rx = router.engine_watch();
    let mut first = true;
    loop {
        if shutdown.is_cancelled() {
            return;
        }
        let engine = *engine_rx.borrow_and_update();
        if !first {
            // the setting changed: a fallback gets one more try
            FallbackRecord::clear(dir);
        }
        first = false;
        let outcome = match engine {
            LayaEngine::Standard => {
                set_vega(&router, VegaState::Off, "Off: the standard engine is chosen in Settings");
                None
            }
            LayaEngine::Vega => match FallbackRecord::read(dir) {
                Some(rec) if rec.holds_for(VERSION) => {
                    set_vega(&router, VegaState::Fallback, &rec.reason);
                    None
                }
                _ => {
                    FallbackRecord::clear(dir);
                    Some(run_vega(&router, opts, dir, port, &serving, &mut settled, &mut engine_rx, shutdown).await)
                }
            },
        };
        match outcome {
            Some(VegaRun::Cancelled) => return,
            Some(VegaRun::EngineChanged) => {
                hand_back(&router, &serving);
                continue;
            }
            Some(VegaRun::Fallback(reason)) => {
                tracing::warn!(%reason, "vega engine fell back to laya-serve 0.4.8");
                FallbackRecord::write(dir, &reason);
                hand_back(&router, &serving);
                set_vega(&router, VegaState::Fallback, &reason);
            }
            None => {}
        }
        // Wait for the setting to change (or kwant to stop)
        tokio::select! {
            _ = shutdown.cancelled() => return,
            changed = engine_rx.changed() => if changed.is_err() { return },
        }
    }
}

/// Routing back to 0.4.8 (which the standard lane starts again).
fn hand_back(router: &LayaRouter, serving: &watch::Sender<Serving>) {
    router.set_active(None);
    router.update_status(|s| {
        if s.engine == LayaEngine::Vega {
            s.engine = LayaEngine::Standard;
            s.state = LayaState::Starting;
            s.detail = "Starting the standard engine again".to_string();
        }
    });
    serving.send_replace(Serving::Standard);
}

#[allow(clippy::too_many_arguments)]
async fn run_vega(
    router: &LayaRouter,
    opts: &SidecarOptions,
    dir: &Path,
    port: u16,
    serving: &watch::Sender<Serving>,
    settled: &mut watch::Receiver<bool>,
    engine_rx: &mut watch::Receiver<LayaEngine>,
    shutdown: &CancellationToken,
) -> VegaRun {
    let t = &opts.timings;
    // 0.4.8 first, so routing works within a minute
    if !*settled.borrow() {
        set_vega(router, VegaState::Waiting, "Waiting for the standard engine to start first");
        tokio::select! {
            _ = shutdown.cancelled() => return VegaRun::Cancelled,
            changed = engine_rx.changed() => return if changed.is_err() { VegaRun::Cancelled } else { VegaRun::EngineChanged },
            _ = settled.wait_for(|s| *s) => {}
            _ = tokio::time::sleep(t.settle) => {}
        }
    }

    let memory = opts.memory_gb.or_else(memory_gb);
    let data_dir = dir.parent().unwrap_or(dir);
    let platform = crate::doctor::Platform::detect();
    let venv = dir.join(VEGA_VENV);
    let python = match &opts.vega_python {
        VegaPython::Use(py) => Some(py.clone()),
        _ => None,
    };
    let finder = match &opts.vega_python {
        VegaPython::Missing => None,
        VegaPython::Use(_) => Some((None, None)),
        // the given commands bring their own Python
        VegaPython::Detect if opts.vega_install.is_some() => Some((None, None)),
        VegaPython::Detect if vega_installed(&venv) => Some((None, None)),
        VegaPython::Detect => {
            let home = dirs::home_dir();
            let uv = crate::setup::find_uv(data_dir, home.as_deref());
            let py = if uv.is_none() { crate::setup::find_python(platform) } else { None };
            (uv.is_some() || py.is_some()).then_some((uv, py))
        }
    };
    if let Some(why) = vega_preflight(memory, finder.is_some()) {
        return VegaRun::Fallback(why);
    }

    let python = match python {
        Some(py) => py,
        None => {
            let venv_py = venv_bin(&venv, "python");
            if !vega_installed(&venv) {
                set_vega(router, VegaState::Installing, "Installing the vega engine in the background (one-time, about 900 MB)");
                let (uv, py) = finder.unwrap_or((None, None));
                let cmds = opts.vega_install.clone().unwrap_or_else(|| {
                    crate::setup::vega_commands(data_dir, uv.as_deref(), py.as_deref(), platform == crate::doctor::Platform::Linux)
                });
                let install_log = dir.join("install-vega.log");
                let install = run_install(&cmds, &install_log);
                let res = tokio::select! {
                    _ = shutdown.cancelled() => return VegaRun::Cancelled,
                    changed = engine_rx.changed() => return if changed.is_err() { VegaRun::Cancelled } else { VegaRun::EngineChanged },
                    res = tokio::time::timeout(t.vega_install, install) => res,
                };
                let failed = match res {
                    Ok(Ok(())) if venv_py.exists() => None,
                    Ok(Ok(())) => Some(format!("{} is missing after the install", venv_py.display())),
                    Ok(Err(e)) => Some(e),
                    Err(_) => Some(format!("it took longer than {}", human(t.vega_install))),
                };
                if let Some(e) = failed {
                    let _ = std::fs::remove_dir_all(&venv);
                    return VegaRun::Fallback(format!("the vega install failed: {e}"));
                }
                let _ = std::fs::write(venv.join(INSTALLED_FILE), VEGA_PACKAGE);
            }
            venv_py
        }
    };

    let script = dir.join("vega_serve.py");
    if let Err(e) = std::fs::write(&script, opts.vega_script.as_deref().unwrap_or(VEGA_SERVE_PY)) {
        return VegaRun::Fallback(format!("couldn't write {}: {e}", script.display()));
    }
    let url = format!("http://127.0.0.1:{port}");
    let log_path = dir.join("vega.log");
    let mut crashes = 0;
    loop {
        set_vega(router, VegaState::Starting, "Loading the vega model (the first start downloads it, about 1.7 GB)");
        let offset = std::fs::metadata(&log_path).map(|m| m.len()).unwrap_or(0);
        let mut child = match start_vega(&python, &script, dir, port) {
            Ok(child) => child,
            Err(e) => return VegaRun::Fallback(format!("vega couldn't start: {e}")),
        };
        let why = match watch_vega(router, &mut child, &url, &hf_dir(dir), serving, engine_rx, shutdown, t).await {
            VegaExit::Cancelled => {
                let _ = child.kill().await;
                return VegaRun::Cancelled;
            }
            VegaExit::EngineChanged => {
                let _ = child.kill().await;
                return VegaRun::EngineChanged;
            }
            VegaExit::NeverHealthy => {
                let _ = child.kill().await;
                return VegaRun::Fallback(format!("vega didn't answer within {} of starting", human(t.vega_health)));
            }
            VegaExit::Unresponsive => {
                let _ = child.kill().await;
                format!("it stopped answering for {}", human(t.vega_unhealthy))
            }
            VegaExit::Crashed(status) => {
                let _ = child.kill().await;
                last_error_line_from(&log_path, offset)
                    .unwrap_or_else(|| format!("it exited ({})", status.map(|s| s.to_string()).unwrap_or_default()))
            }
        };
        crashes += 1;
        router.update_status(|s| s.vega.crashes = crashes);
        hand_back(router, serving);
        tracing::warn!(reason = %why, crashes, "vega engine stopped");
        if crashes >= VEGA_MAX_CRASHES {
            return VegaRun::Fallback(format!("vega stopped {crashes} times; last: {why}"));
        }
        set_vega(router, VegaState::Starting, &format!("vega stopped ({why}); starting it again"));
        if wait_or_cancel(shutdown, t.vega_restart).await {
            return VegaRun::Cancelled;
        }
    }
}

enum VegaExit {
    Cancelled,
    EngineChanged,
    NeverHealthy,
    Crashed(Option<std::process::ExitStatus>),
    /// Active, but `/health` stopped answering.
    Unresponsive,
}

/// Waits for vega's `/health`, hands routing to it, then watches it until it stops.
#[allow(clippy::too_many_arguments)]
async fn watch_vega(
    router: &LayaRouter,
    child: &mut Child,
    url: &str,
    hf: &Path,
    serving: &watch::Sender<Serving>,
    engine_rx: &mut watch::Receiver<LayaEngine>,
    shutdown: &CancellationToken,
    t: &SidecarTimings,
) -> VegaExit {
    let started = tokio::time::Instant::now();
    let mut tick = tokio::time::interval(t.tick);
    let mut active = false;
    let mut last_ok = tokio::time::Instant::now();
    loop {
        tokio::select! {
            _ = shutdown.cancelled() => return VegaExit::Cancelled,
            changed = engine_rx.changed() => {
                if changed.is_err() {
                    return VegaExit::Cancelled;
                }
                if *engine_rx.borrow_and_update() != LayaEngine::Vega {
                    return VegaExit::EngineChanged;
                }
            }
            status = child.wait() => return VegaExit::Crashed(status.ok()),
            _ = tick.tick() => {
                let up = router.is_reachable_at(url).await;
                if up {
                    last_ok = tokio::time::Instant::now();
                }
                if up && !active {
                    active = true;
                    tick = tokio::time::interval(t.online_tick.min(t.vega_unhealthy / 2).max(t.tick));
                    router.set_active(Some(url.to_string()));
                    router.update_status(|s| {
                        s.state = LayaState::Online;
                        s.managed = true;
                        s.engine = LayaEngine::Vega;
                        s.vega.state = VegaState::Active;
                        s.vega.detail = "Answering routing questions".to_string();
                        s.detail = online_detail(s);
                    });
                    serving.send_replace(Serving::Vega);
                    tracing::info!(%url, "vega engine online; routing moved from laya-serve 0.4.8");
                    let hf = hf.to_path_buf();
                    let (mb, cache) = tokio::task::spawn_blocking(move || model_cache(&hf)).await.unwrap_or((None, None));
                    router.update_status(|s| {
                        s.vega.model_mb = mb;
                        s.vega.model_cache = cache;
                    });
                } else if !active && started.elapsed() > t.vega_health {
                    return VegaExit::NeverHealthy;
                } else if active && last_ok.elapsed() > t.vega_unhealthy {
                    return VegaExit::Unresponsive;
                }
            }
        }
    }
}

async fn monitor_external(router: LayaRouter, shutdown: CancellationToken) {
    loop {
        if wait_or_cancel(&shutdown, Duration::from_secs(30)).await {
            return;
        }
        let up = router.is_reachable().await;
        let current = router.status();
        if up && !current.is_up() {
            router.set_status(LayaState::External, "Using the Laya service already running", false);
            router.warm_up().await;
        } else if !up && current.is_up() {
            router.set_status(LayaState::Offline, "Laya stopped answering; using keyword routing", false);
        }
    }
}

/// Returns the `laya-serve` executable, installing it into `<data>/laya/venv` when missing.
async fn ensure_installed(router: &LayaRouter, dir: &Path, opts: &SidecarOptions) -> Result<PathBuf, String> {
    if let Some(bin) = &opts.standard_bin {
        return Ok(bin.clone());
    }
    if let Ok(bin) = std::env::var("KWNT_LAYA_SERVE_BIN") {
        if !bin.is_empty() {
            return Ok(PathBuf::from(bin));
        }
    }
    let venv = dir.join("venv");
    let bin = venv_bin(&venv, "laya-serve");
    if bin.exists() {
        return Ok(bin);
    }

    router.set_status(
        LayaState::Installing,
        "Installing the Laya router (one-time, downloads PyTorch)",
        true,
    );
    // The data dir is the parent of `<data>/laya`; uv, else a real Python, else uv installed there
    let data_dir = dir.parent().unwrap_or(dir);
    let home = dirs::home_dir();
    let platform = crate::doctor::Platform::detect();
    let cmds = crate::setup::laya_commands(
        data_dir,
        crate::setup::find_uv(data_dir, home.as_deref()).as_deref(),
        crate::setup::find_python(platform).as_deref(),
        platform == crate::doctor::Platform::Linux,
    );
    if let Err(e) = run_install(&cmds, &dir.join("install.log")).await {
        let _ = std::fs::remove_dir_all(&venv);
        return Err(e);
    }

    if bin.exists() {
        Ok(bin)
    } else {
        Err(format!("install finished but {} is missing", bin.display()))
    }
}

pub(crate) fn start(bin: &Path, dir: &Path, port: u16, models: &str) -> Result<Child, String> {
    let log = log_file(&dir.join("laya.log"))?;
    let err = log.try_clone().map_err(|e| e.to_string())?;
    let default_model = models.split(',').map(str::trim).find(|m| !m.is_empty()).unwrap_or("english");

    let mut cmd = Command::new(bin);
    cmd.env("LAYA_HOST", "127.0.0.1")
        .env("LAYA_PORT", port.to_string())
        .env("LAYA_MODELS", models)
        .env("LAYA_DEFAULT_MODEL", default_model)
        .env("LAYA_LOG_LEVEL", "warning")
        .stdin(Stdio::null())
        .stdout(Stdio::from(log))
        .stderr(Stdio::from(err))
        .kill_on_drop(true);
    if std::env::var_os("HF_HOME").is_none() {
        cmd.env("HF_HOME", dir.join("hf"));
    }
    cmd.spawn().map_err(|e| e.to_string())
}

/// Port of a loopback URL (`http://127.0.0.1:8000`), or `None` for remote hosts.
pub fn local_port(url: &str) -> Option<u16> {
    let rest = url.split("://").nth(1).unwrap_or(url);
    let authority = rest.split('/').next()?;
    let (host, port) = match authority.rfind(':').filter(|&i| !authority[i..].contains(']')) {
        Some(i) => (&authority[..i], authority[i + 1..].parse().ok()?),
        None => (authority, if url.starts_with("https") { 443 } else { 80 }),
    };
    matches!(host, "127.0.0.1" | "localhost" | "[::1]" | "0.0.0.0").then_some(port)
}

/// Last line of a log that looks like an error, to show users why Laya is down.
pub fn last_error_line(log: &Path) -> Option<String> {
    last_error_line_from(log, 0)
}

/// Like [`last_error_line`], but only looks at bytes written after `offset`.
pub fn last_error_line_from(log: &Path, offset: u64) -> Option<String> {
    let bytes = std::fs::read(log).ok()?;
    let text = String::from_utf8_lossy(bytes.get(offset as usize..)?);
    let tail: Vec<&str> = text.lines().rev().take(200).collect();
    let line = tail
        .iter()
        .find(|l| {
            let l = l.trim_start();
            l.starts_with("ERROR") || l.contains("Error:") || l.contains("error:")
        })
        .or_else(|| tail.iter().find(|l| !l.trim().is_empty()))?;
    let line = line.trim();
    Some(if line.len() > 300 { format!("{}…", &line[..line.floor_char_boundary(300)]) } else { line.to_string() })
}

async fn wait_or_cancel(shutdown: &CancellationToken, wait: Duration) -> bool {
    tokio::select! {
        _ = shutdown.cancelled() => true,
        _ = tokio::time::sleep(wait) => false,
    }
}

fn log_file(path: &Path) -> Result<std::fs::File, String> {
    std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .map_err(|e| format!("{}: {e}", path.display()))
}

fn venv_bin(venv: &Path, name: &str) -> PathBuf {
    if cfg!(windows) {
        venv.join("Scripts").join(format!("{name}.exe"))
    } else {
        venv.join("bin").join(name)
    }
}

/// "40 minutes", "90 seconds".
fn human(d: Duration) -> String {
    match d.as_secs() {
        s if s >= 120 => format!("{} minutes", s / 60),
        1 => "1 second".to_string(),
        s => format!("{s} seconds"),
    }
}

/// Whether `venv-vega` finished installing [`VEGA_PACKAGE`] (an older pin installs again).
fn vega_installed(venv: &Path) -> bool {
    venv_bin(venv, "python").exists()
        && std::fs::read_to_string(venv.join(INSTALLED_FILE)).is_ok_and(|s| s.trim() == VEGA_PACKAGE)
}

/// Where Hugging Face models are cached for the sidecar: `HF_HOME`, else `<data>/laya/hf`.
pub fn hf_dir(dir: &Path) -> PathBuf {
    std::env::var_os("HF_HOME").filter(|v| !v.is_empty()).map(PathBuf::from).unwrap_or_else(|| dir.join("hf"))
}

/// The vega model's size in MB and where it is: the `models--*vega*` folders under `<hf>/hub`.
pub fn model_cache(hf: &Path) -> (Option<u64>, Option<String>) {
    let hub = hf.join("hub");
    let Ok(entries) = std::fs::read_dir(&hub) else { return (None, None) };
    let dirs: Vec<PathBuf> = entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.is_dir() && p.file_name().is_some_and(|n| {
            let n = n.to_string_lossy().to_ascii_lowercase();
            n.starts_with("models--") && n.contains("vega")
        }))
        .collect();
    if dirs.is_empty() {
        return (None, None);
    }
    let bytes: u64 = dirs.iter().map(|d| dir_bytes(d)).sum();
    let cache = if dirs.len() == 1 { dirs[0].display().to_string() } else { hub.display().to_string() };
    (Some(bytes.div_ceil(1024 * 1024)), Some(cache))
}

/// Bytes in regular files under `dir`; symlinks (Hugging Face's snapshots) aren't followed.
fn dir_bytes(dir: &Path) -> u64 {
    let Ok(entries) = std::fs::read_dir(dir) else { return 0 };
    entries
        .flatten()
        .map(|e| match e.file_type() {
            Ok(t) if t.is_dir() => dir_bytes(&e.path()),
            Ok(t) if t.is_file() => e.metadata().map(|m| m.len()).unwrap_or(0),
            _ => 0,
        })
        .sum()
}

/// This machine's memory in GB (the container's limit when lower), or `None` when unknown.
pub fn memory_gb() -> Option<f64> {
    if cfg!(target_os = "macos") {
        let out = std::process::Command::new("sysctl").args(["-n", "hw.memsize"]).output().ok()?;
        let bytes: f64 = String::from_utf8_lossy(&out.stdout).trim().parse().ok()?;
        return Some(bytes / GIB);
    }
    let total = parse_meminfo(&std::fs::read_to_string("/proc/meminfo").ok()?)?;
    let limit = std::fs::read_to_string("/sys/fs/cgroup/memory.max")
        .ok()
        .and_then(|s| s.trim().parse::<f64>().ok())
        .map(|b| b / GIB);
    Some(limit.map_or(total, |l| l.min(total)))
}

const GIB: f64 = 1024.0 * 1024.0 * 1024.0;

/// `MemTotal` from `/proc/meminfo`, in GB.
pub fn parse_meminfo(text: &str) -> Option<f64> {
    let line = text.lines().find(|l| l.starts_with("MemTotal:"))?;
    let kb: f64 = line.split_whitespace().nth(1)?.parse().ok()?;
    Some(kb * 1024.0 / GIB)
}

/// Runs install commands in order, logging to `log`; the error is the log's last error line.
async fn run_install(cmds: &[crate::setup::Cmd], log: &Path) -> Result<(), String> {
    let offset = std::fs::metadata(log).map(|m| m.len()).unwrap_or(0);
    for cmd in cmds {
        let Some(program) = cmd.argv.first() else { continue };
        let out = log_file(log)?;
        let err = out.try_clone().map_err(|e| e.to_string())?;
        let status = Command::new(program)
            .args(&cmd.argv[1..])
            .envs(cmd.env.iter().map(|(k, v)| (k, v)))
            .stdin(Stdio::null())
            .stdout(Stdio::from(out))
            .stderr(Stdio::from(err))
            .kill_on_drop(true)
            .status()
            .await
            .map_err(|e| format!("{program}: {e}"))?;
        if !status.success() {
            return Err(last_error_line_from(log, offset).unwrap_or_else(|| format!("`{}` failed", cmd.display())));
        }
    }
    Ok(())
}

/// Starts `vega_serve.py` on loopback, with torch held to [`VEGA_TORCH_THREADS`] threads.
fn start_vega(python: &Path, script: &Path, dir: &Path, port: u16) -> Result<Child, String> {
    let log = log_file(&dir.join("vega.log"))?;
    let err = log.try_clone().map_err(|e| e.to_string())?;
    let threads = VEGA_TORCH_THREADS.to_string();
    let mut cmd = Command::new(python);
    cmd.arg("-u")
        .arg(script)
        .current_dir(dir)
        .env("LAYA_HOST", "127.0.0.1")
        .env("LAYA_PORT", port.to_string())
        .env("LAYA_LOG_LEVEL", "warning")
        .env("KWNT_VEGA_THREADS", &threads)
        .env("OMP_NUM_THREADS", &threads)
        .env("MKL_NUM_THREADS", &threads)
        .env("TOKENIZERS_PARALLELISM", "false")
        .env("HF_HOME", hf_dir(dir))
        .stdin(Stdio::null())
        .stdout(Stdio::from(log))
        .stderr(Stdio::from(err))
        .kill_on_drop(true);
    cmd.spawn().map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Instant;

    #[test]
    fn engine_defaults_to_vega_and_standard_turns_it_off() {
        assert_eq!(LayaEngine::default(), LayaEngine::Vega);
        assert_eq!(LayaEngine::from_setting(None), LayaEngine::Vega);
        assert_eq!(LayaEngine::from_setting(Some("vega")), LayaEngine::Vega);
        assert_eq!(LayaEngine::from_setting(Some(" Standard ")), LayaEngine::Standard);
        assert_eq!(LayaEngine::from_setting(Some("something-else")), LayaEngine::Vega);
        assert_eq!(LayaEngine::parse("standard"), Some(LayaEngine::Standard));
        assert_eq!(LayaEngine::parse("vega"), Some(LayaEngine::Vega));
        assert_eq!(LayaEngine::parse("vegaml"), None);
        assert_eq!(LayaEngine::parse(""), None);
        assert_eq!(serde_json::to_value(LayaEngine::Vega).unwrap(), "vega");
        assert_eq!(super::super::LayaRouter::new("http://127.0.0.1:1".into()).engine(), LayaEngine::Vega);
    }

    #[test]
    fn preflight_needs_8_gb_and_a_python() {
        assert_eq!(vega_preflight(Some(16.0), true), None);
        assert_eq!(vega_preflight(Some(8.0), true), None);
        // unknown memory doesn't block
        assert_eq!(vega_preflight(None, true), None);
        let low = vega_preflight(Some(7.6), true).unwrap();
        assert!(low.contains("8 GB") && low.contains("7.6 GB"), "{low}");
        let nopy = vega_preflight(Some(16.0), false).unwrap();
        assert!(nopy.contains("Python 3.10"), "{nopy}");
        let meminfo = "MemTotal:       16314276 kB\nMemFree:          123 kB\n";
        let gb = parse_meminfo(meminfo).unwrap();
        assert!((gb - 15.56).abs() < 0.01, "{gb}");
        assert_eq!(parse_meminfo("MemFree: 1 kB"), None);
    }

    #[test]
    fn a_fallback_holds_until_kwant_is_updated() {
        let dir = tempfile::tempdir().unwrap();
        assert!(FallbackRecord::read(dir.path()).is_none());
        FallbackRecord::write(dir.path(), "vega needs 8 GB of memory");
        let rec = FallbackRecord::read(dir.path()).unwrap();
        assert_eq!(rec.reason, "vega needs 8 GB of memory");
        assert!(rec.holds_for(VERSION));
        assert!(!rec.holds_for("0.0.1"));
        FallbackRecord::clear(dir.path());
        assert!(FallbackRecord::read(dir.path()).is_none());
        // a damaged file means no fallback (vega is tried)
        std::fs::write(dir.path().join(FALLBACK_FILE), "{not json").unwrap();
        assert!(FallbackRecord::read(dir.path()).is_none());
    }

    #[test]
    fn model_cache_counts_the_vega_model_without_following_links() {
        let hf = tempfile::tempdir().unwrap();
        assert_eq!(model_cache(hf.path()), (None, None));
        let model = hf.path().join("hub/models--nandakishorm--vega-U");
        std::fs::create_dir_all(model.join("blobs")).unwrap();
        std::fs::create_dir_all(model.join("snapshots/abc")).unwrap();
        std::fs::write(model.join("blobs/w"), vec![0u8; 3 * 1024 * 1024]).unwrap();
        #[cfg(unix)]
        std::os::unix::fs::symlink(model.join("blobs/w"), model.join("snapshots/abc/model.safetensors")).unwrap();
        // laya's own model isn't counted
        std::fs::create_dir_all(hf.path().join("hub/models--other--modernbert/blobs")).unwrap();
        std::fs::write(hf.path().join("hub/models--other--modernbert/blobs/w"), vec![0u8; 1024 * 1024]).unwrap();
        assert_eq!(model_cache(hf.path()), (Some(3), Some(model.display().to_string())));
    }

    // ---- the supervisor, with fake 0.4.8 and vega servers (needs python3) ----

    /// One at a time: they bind ports and start processes. `KWNT_SIDECAR_TEST_PORT` pins the pair.
    static SERIAL: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

    fn python3() -> bool {
        let ok = std::process::Command::new("python3")
            .args(["-c", "import http.server"])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .is_ok_and(|s| s.success());
        if !ok {
            eprintln!("skipped: no python3");
        }
        ok
    }

    fn ports() -> (u16, u16) {
        if let Some(base) = std::env::var("KWNT_SIDECAR_TEST_PORT").ok().and_then(|p| p.parse::<u16>().ok()) {
            return (base, base + 1);
        }
        let a = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let b = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        (a.local_addr().unwrap().port(), b.local_addr().unwrap().port())
    }

    const FAKE_SERVER: &str = r#"
import http.server, json
class H(http.server.BaseHTTPRequestHandler):
    def _send(self, body):
        b = json.dumps(body).encode()
        self.send_response(200)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(b)))
        self.end_headers()
        self.wfile.write(b)
    def do_GET(self):
        self._send({"status": "ok", "engine": ENGINE})
    def do_POST(self):
        self.rfile.read(int(self.headers.get("Content-Length") or 0))
        self._send({"model": ENGINE, "engine": ENGINE, "answers": {}, "usage": {"input_tokens": 0, "output_tokens": 0}})
    def log_message(self, *a):
        pass
http.server.HTTPServer((os.environ.get("LAYA_HOST", "127.0.0.1"), int(os.environ["LAYA_PORT"])), H).serve_forever()
"#;

    /// laya-serve 0.4.8's stand-in: an executable that answers at once.
    fn fake_standard(dir: &Path) -> PathBuf {
        let bin = dir.join("fake-laya-serve");
        std::fs::write(&bin, format!("#!/usr/bin/env python3\nimport os\nENGINE = 'standard'\n{FAKE_SERVER}")).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        bin
    }

    /// vega_serve.py's stand-in. `mode`: `ok` (answers once `gate` exists), `crash`, `crash-once`, `hang`.
    /// Writes the env it got to `<dir>/vega-env.json`.
    fn fake_vega(dir: &Path, mode: &str, gate: Option<&Path>) -> String {
        let gate = gate.map(|g| g.display().to_string()).unwrap_or_default();
        let envout = dir.join("vega-env.json").display().to_string();
        let once = dir.join("crashed-once").display().to_string();
        format!(
            r#"import json, os, sys, time
MODE, GATE, ONCE = {mode:?}, {gate:?}, {once:?}
with open({envout:?}, "w") as f:
    json.dump({{k: os.environ.get(k) for k in ("LAYA_HOST", "OMP_NUM_THREADS", "MKL_NUM_THREADS", "KWNT_VEGA_THREADS", "HF_HOME")}}, f)
if MODE == "crash" or (MODE == "crash-once" and not os.path.exists(ONCE)):
    open(ONCE, "w").close()
    print("ERROR: fake vega crashed", file=sys.stderr, flush=True)
    sys.exit(3)
if MODE == "hang":
    time.sleep(3600)
while GATE and not os.path.exists(GATE):
    time.sleep(0.05)
ENGINE = "vega"
{FAKE_SERVER}"#
        )
    }

    struct Rig {
        router: LayaRouter,
        cancel: CancellationToken,
        handle: tokio::task::JoinHandle<()>,
        laya: PathBuf,
        std_url: String,
        vega_url: String,
    }

    impl Rig {
        fn start(data: &Path, (std_port, vega_port): (u16, u16), engine: LayaEngine, tweak: impl FnOnce(&mut SidecarOptions)) -> Self {
            let std_url = format!("http://127.0.0.1:{std_port}");
            let router = LayaRouter::new(std_url.clone());
            router.set_engine(engine);
            let laya = data.join("laya");
            std::fs::create_dir_all(&laya).unwrap();
            let mut opts = SidecarOptions {
                data_dir: data.to_path_buf(),
                managed: true,
                models: "english".into(),
                vega_port: Some(vega_port),
                standard_bin: Some(fake_standard(data)),
                vega_python: VegaPython::Use("python3".into()),
                vega_script: Some(fake_vega(data, "ok", None)),
                memory_gb: Some(16.0),
                timings: SidecarTimings {
                    tick: Duration::from_millis(50),
                    online_tick: Duration::from_millis(100),
                    settle: Duration::from_secs(10),
                    vega_install: Duration::from_secs(10),
                    vega_health: Duration::from_secs(20),
                    vega_unhealthy: Duration::from_secs(1),
                    vega_restart: Duration::from_millis(50),
                },
                ..Default::default()
            };
            tweak(&mut opts);
            let cancel = CancellationToken::new();
            let handle = spawn(router.clone(), opts, cancel.clone());
            Self { router, cancel, handle, laya, std_url, vega_url: format!("http://127.0.0.1:{vega_port}") }
        }

        async fn until(&self, what: &str, f: impl Fn(&LayaStatus) -> bool) {
            let end = Instant::now() + Duration::from_secs(20);
            while !f(&self.router.status()) {
                assert!(Instant::now() < end, "timed out waiting for {what}: {:?}", self.router.status());
                tokio::time::sleep(Duration::from_millis(25)).await;
            }
        }

        async fn stop(self) {
            self.cancel.cancel();
            let _ = tokio::time::timeout(Duration::from_secs(5), self.handle).await;
        }
    }

    fn standard_online(s: &LayaStatus) -> bool {
        s.state == LayaState::Online && s.engine == LayaEngine::Standard
    }

    #[tokio::test]
    async fn standard_serves_until_vega_is_healthy_then_hands_over() {
        if !python3() {
            return;
        }
        let _one = SERIAL.lock().await;
        let data = tempfile::tempdir().unwrap();
        let gate = data.path().join("gate");
        let venv = data.path().join("laya").join(VEGA_VENV);
        // the install path, with a "venv" whose python is this machine's python3
        let install = crate::setup::Cmd {
            label: "fake install".into(),
            argv: vec!["sh".into(), "-c".into(), format!("mkdir -p {0}/bin && ln -s \"$(command -v python3)\" {0}/bin/python", venv.display())],
            env: vec![],
        };
        let hub = crate::router::sidecar::hf_dir(&data.path().join("laya")).join("hub/models--nandakishorm--vega-U/blobs");
        std::fs::create_dir_all(&hub).unwrap();
        std::fs::write(hub.join("w"), vec![0u8; 2 * 1024 * 1024]).unwrap();
        let script = fake_vega(data.path(), "ok", Some(&gate));
        let rig = Rig::start(data.path(), ports(), LayaEngine::default(), |o| {
            o.vega_python = VegaPython::Detect;
            o.vega_install = Some(vec![install]);
            o.vega_script = Some(script);
        });

        // 0.4.8 first, and it keeps answering while vega isn't healthy
        rig.until("0.4.8 online, vega starting", |s| standard_online(s) && s.vega.state == VegaState::Starting).await;
        tokio::time::sleep(Duration::from_millis(400)).await;
        let s = rig.router.status();
        assert!(standard_online(&s), "{s:?}");
        assert_eq!(rig.router.url(), rig.std_url);
        assert!(rig.router.is_reachable().await);
        assert!(s.detail.contains("while vega gets ready"), "{}", s.detail);
        assert_eq!(std::fs::read_to_string(venv.join(INSTALLED_FILE)).unwrap(), VEGA_PACKAGE);
        // started on loopback with torch held to 4 threads
        let env: serde_json::Value = serde_json::from_slice(&std::fs::read(data.path().join("vega-env.json")).unwrap()).unwrap();
        assert_eq!(env["LAYA_HOST"], "127.0.0.1");
        assert_eq!(env["OMP_NUM_THREADS"], "4");
        assert_eq!(env["MKL_NUM_THREADS"], "4");
        assert_eq!(env["KWNT_VEGA_THREADS"], "4");

        // healthy: routing moves to vega and 0.4.8 stops
        std::fs::write(&gate, "").unwrap();
        rig.until("vega active", |s| s.engine == LayaEngine::Vega && s.vega.state == VegaState::Active).await;
        assert_eq!(rig.router.url(), rig.vega_url);
        let s = rig.router.status();
        assert_eq!(s.state, LayaState::Online);
        assert_eq!(s.url, rig.vega_url);
        assert_eq!(s.detail, "Model-based routing is active (vega engine)");
        let end = Instant::now() + Duration::from_secs(10);
        while rig.router.is_reachable_at(&rig.std_url).await {
            assert!(Instant::now() < end, "0.4.8 still running after the handover");
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        rig.until("model size", |s| s.vega.model_mb.is_some()).await;
        let s = rig.router.status();
        assert_eq!(s.vega.model_mb, Some(2));
        assert!(s.vega.model_cache.as_deref().unwrap().ends_with("models--nandakishorm--vega-U"));
        rig.stop().await;
    }

    #[tokio::test]
    async fn low_memory_falls_back_once_and_only_an_update_retries() {
        if !python3() {
            return;
        }
        let _one = SERIAL.lock().await;
        let data = tempfile::tempdir().unwrap();
        let ports = ports();
        let rig = Rig::start(data.path(), ports, LayaEngine::Vega, |o| o.memory_gb = Some(4.0));
        rig.until("fallback", |s| s.vega.state == VegaState::Fallback && standard_online(s)).await;
        let s = rig.router.status();
        assert!(s.vega.detail.contains("8 GB") && s.vega.detail.contains("4.0 GB"), "{}", s.vega.detail);
        // the plain status line says why
        assert!(s.detail.starts_with("Model-based routing is active (standard engine; vega needs 8 GB"), "{}", s.detail);
        assert_eq!(rig.router.url(), rig.std_url);
        assert!(!data.path().join("vega-env.json").exists(), "vega must not start");
        let rec = FallbackRecord::read(&rig.laya).unwrap();
        assert_eq!(rec.version, VERSION);
        rig.stop().await;

        // the next start (now with enough memory) doesn't try again
        let rig = Rig::start(data.path(), ports, LayaEngine::Vega, |_| {});
        rig.until("0.4.8 online", standard_online).await;
        tokio::time::sleep(Duration::from_millis(500)).await;
        let s = rig.router.status();
        assert_eq!(s.vega.state, VegaState::Fallback, "{s:?}");
        assert!(!data.path().join("vega-env.json").exists(), "vega must not start again");
        rig.stop().await;

        // after a kwant update it does
        let mut rec = FallbackRecord::read(&data.path().join("laya")).unwrap();
        rec.version = "0.0.1".into();
        std::fs::write(data.path().join("laya").join(FALLBACK_FILE), serde_json::to_vec(&rec).unwrap()).unwrap();
        let rig = Rig::start(data.path(), ports, LayaEngine::Vega, |_| {});
        rig.until("vega active after the update", |s| s.vega.state == VegaState::Active).await;
        assert!(FallbackRecord::read(&rig.laya).is_none());
        rig.stop().await;
    }

    #[tokio::test]
    async fn no_python_falls_back() {
        if !python3() {
            return;
        }
        let _one = SERIAL.lock().await;
        let data = tempfile::tempdir().unwrap();
        let rig = Rig::start(data.path(), ports(), LayaEngine::Vega, |o| o.vega_python = VegaPython::Missing);
        rig.until("fallback", |s| s.vega.state == VegaState::Fallback && standard_online(s)).await;
        assert!(rig.router.status().vega.detail.contains("Python 3.10"));
        assert_eq!(rig.router.url(), rig.std_url);
        assert!(FallbackRecord::read(&rig.laya).is_some());
        rig.stop().await;
    }

    #[tokio::test]
    async fn a_failed_install_falls_back_and_removes_the_half_venv() {
        if !python3() {
            return;
        }
        let _one = SERIAL.lock().await;
        let data = tempfile::tempdir().unwrap();
        let venv = data.path().join("laya").join(VEGA_VENV);
        let install = crate::setup::Cmd {
            label: "fake install".into(),
            argv: vec![
                "sh".into(),
                "-c".into(),
                format!("mkdir -p {} && echo 'ERROR: No matching distribution found for laya' >&2 && exit 1", venv.display()),
            ],
            env: vec![],
        };
        let rig = Rig::start(data.path(), ports(), LayaEngine::Vega, |o| {
            o.vega_python = VegaPython::Detect;
            o.vega_install = Some(vec![install]);
        });
        rig.until("fallback", |s| s.vega.state == VegaState::Fallback && standard_online(s)).await;
        let why = rig.router.status().vega.detail;
        assert!(why.starts_with("the vega install failed: ERROR: No matching distribution"), "{why}");
        assert!(!venv.exists());
        assert!(rig.laya.join("install-vega.log").exists());
        assert_eq!(rig.router.url(), rig.std_url);
        rig.stop().await;
    }

    #[tokio::test]
    async fn vega_that_never_answers_falls_back_in_time() {
        if !python3() {
            return;
        }
        let _one = SERIAL.lock().await;
        let data = tempfile::tempdir().unwrap();
        let script = fake_vega(data.path(), "hang", None);
        let rig = Rig::start(data.path(), ports(), LayaEngine::Vega, |o| {
            o.vega_script = Some(script);
            o.timings.vega_health = Duration::from_secs(1);
        });
        rig.until("fallback", |s| s.vega.state == VegaState::Fallback).await;
        let s = rig.router.status();
        assert_eq!(s.vega.detail, "vega didn't answer within 1 second of starting");
        assert!(standard_online(&s) || s.state == LayaState::Starting, "{s:?}");
        assert_eq!(rig.router.url(), rig.std_url);
        assert!(!rig.router.is_reachable_at(&rig.vega_url).await);
        rig.stop().await;
    }

    #[tokio::test]
    async fn two_crashes_fall_back() {
        if !python3() {
            return;
        }
        let _one = SERIAL.lock().await;
        let data = tempfile::tempdir().unwrap();
        let script = fake_vega(data.path(), "crash", None);
        let rig = Rig::start(data.path(), ports(), LayaEngine::Vega, |o| o.vega_script = Some(script));
        rig.until("fallback", |s| s.vega.state == VegaState::Fallback).await;
        let s = rig.router.status();
        assert_eq!(s.vega.crashes, 2);
        assert_eq!(s.vega.detail, "vega stopped 2 times; last: ERROR: fake vega crashed");
        assert_eq!(rig.router.url(), rig.std_url);
        rig.until("0.4.8 online", standard_online).await;
        rig.stop().await;
    }

    #[tokio::test]
    async fn one_crash_restarts_vega() {
        if !python3() {
            return;
        }
        let _one = SERIAL.lock().await;
        let data = tempfile::tempdir().unwrap();
        let script = fake_vega(data.path(), "crash-once", None);
        let rig = Rig::start(data.path(), ports(), LayaEngine::Vega, |o| o.vega_script = Some(script));
        rig.until("vega active", |s| s.vega.state == VegaState::Active).await;
        assert_eq!(rig.router.status().vega.crashes, 1);
        assert_eq!(rig.router.url(), rig.vega_url);
        assert!(FallbackRecord::read(&rig.laya).is_none());
        rig.stop().await;
    }

    #[tokio::test]
    async fn the_setting_switches_engines_live_and_retries_a_fallback() {
        if !python3() {
            return;
        }
        let _one = SERIAL.lock().await;
        let data = tempfile::tempdir().unwrap();
        let ports = ports();
        std::fs::create_dir_all(data.path().join("laya")).unwrap();
        FallbackRecord::write(&data.path().join("laya"), "vega stopped 2 times; last: boom");
        let rig = Rig::start(data.path(), ports, LayaEngine::Standard, |_| {});
        rig.until("standard only", |s| standard_online(s) && s.vega.state == VegaState::Off).await;
        assert_eq!(rig.router.status().detail, "Model-based routing is active");
        tokio::time::sleep(Duration::from_millis(300)).await;
        assert!(!data.path().join("vega-env.json").exists(), "vega must not start on standard");

        // choosing vega again gives the earlier fallback one more try
        rig.router.set_engine(LayaEngine::Vega);
        rig.until("vega active", |s| s.vega.state == VegaState::Active && s.engine == LayaEngine::Vega).await;
        assert_eq!(rig.router.url(), rig.vega_url);
        assert!(FallbackRecord::read(&rig.laya).is_none());

        // and standard hands routing back to 0.4.8 and stops vega
        rig.router.set_engine(LayaEngine::Standard);
        rig.until("back on 0.4.8", |s| standard_online(s) && s.vega.state == VegaState::Off).await;
        assert_eq!(rig.router.url(), rig.std_url);
        let end = Instant::now() + Duration::from_secs(10);
        while rig.router.is_reachable_at(&rig.vega_url).await {
            assert!(Instant::now() < end, "vega still running on standard");
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        rig.stop().await;
    }
}
