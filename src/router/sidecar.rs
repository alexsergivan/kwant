//! Managed Laya sidecar: installs `laya-serve` into the kwnt data dir on first run,
//! starts it next to the server, restarts it if it dies and reports its status.

use super::LayaRouter;
use serde::Serialize;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;
use tokio::process::{Child, Command};
use tokio_util::sync::CancellationToken;

/// Pinned so every install routes the same way; bump deliberately.
pub const LAYA_PACKAGE: &str = "laya[serve]==0.4.8";

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
}

impl LayaStatus {
    pub fn new(url: &str) -> Self {
        Self { state: LayaState::Unknown, detail: String::new(), url: url.to_string(), managed: false }
    }

    pub fn is_up(&self) -> bool {
        matches!(self.state, LayaState::Online | LayaState::External)
    }
}

#[derive(Debug, Clone)]
pub struct SidecarOptions {
    pub data_dir: PathBuf,
    pub managed: bool,
    /// Comma list of Laya checkpoints to preload (`english`, `multilingual`, `typed-decisions`).
    pub models: String,
}

/// Starts the supervisor in the background. Cancel the token to stop the sidecar.
pub fn spawn(router: LayaRouter, opts: SidecarOptions, shutdown: CancellationToken) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move { supervise(router, opts, shutdown).await })
}

async fn supervise(router: LayaRouter, opts: SidecarOptions, shutdown: CancellationToken) {
    if router.is_reachable().await {
        router.set_status(LayaState::External, "Using the Laya service already running", false);
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
    let mut backoff = Duration::from_secs(5);

    while !shutdown.is_cancelled() {
        let bin = match ensure_installed(&router, &dir).await {
            Ok(bin) => bin,
            Err(e) => {
                let fix = crate::doctor::laya_fix(crate::doctor::Platform::detect(), true);
                router.set_status(LayaState::Failed, &format!("Install failed: {e}. {fix}"), true);
                if wait_or_cancel(&shutdown, Duration::from_secs(300)).await {
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
        let mut child = match start(&bin, &dir, port, &opts.models) {
            Ok(child) => child,
            Err(e) => {
                router.set_status(LayaState::Failed, &format!("Could not start laya-serve: {e}"), true);
                if wait_or_cancel(&shutdown, backoff).await {
                    return;
                }
                continue;
            }
        };

        let started = std::time::Instant::now();
        let exit = run_until_exit(&router, &mut child, &shutdown).await;
        if shutdown.is_cancelled() {
            let _ = child.kill().await;
            return;
        }

        let reason = last_error_line_from(&log_path, log_offset)
            .unwrap_or_else(|| format!("laya-serve exited ({})", exit.map(|s| s.to_string()).unwrap_or_default()));
        tracing::warn!(%reason, "Laya sidecar stopped; restarting in {:?}", backoff);
        router.set_status(LayaState::Failed, &reason, true);

        if started.elapsed() > Duration::from_secs(600) {
            backoff = Duration::from_secs(5);
        }
        if wait_or_cancel(&shutdown, backoff).await {
            return;
        }
        backoff = (backoff * 2).min(Duration::from_secs(300));
    }
}

/// Waits for the child to exit, flipping the status to online once `/health` answers.
async fn run_until_exit(
    router: &LayaRouter,
    child: &mut Child,
    shutdown: &CancellationToken,
) -> Option<std::process::ExitStatus> {
    let mut tick = tokio::time::interval(Duration::from_secs(2));
    let mut online = false;
    loop {
        tokio::select! {
            _ = shutdown.cancelled() => return None,
            status = child.wait() => return status.ok(),
            _ = tick.tick() => {
                let up = router.is_reachable().await;
                if up && !online {
                    online = true;
                    tick = tokio::time::interval(Duration::from_secs(15));
                    tracing::info!(url = %router.base_url, "Laya sidecar online");
                    router.warm_up().await;
                    router.set_status(LayaState::Online, "Model-based routing is active", true);
                } else if !up && online {
                    router.set_status(LayaState::Starting, "Laya stopped answering; waiting for it", true);
                    online = false;
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
async fn ensure_installed(router: &LayaRouter, dir: &Path) -> Result<PathBuf, String> {
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
    let log = dir.join("install.log");
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

    let offset = std::fs::metadata(&log).map(|m| m.len()).unwrap_or(0);
    for cmd in cmds {
        let out = log_file(&log)?;
        let err = out.try_clone().map_err(|e| e.to_string())?;
        let status = Command::new(&cmd.argv[0])
            .args(&cmd.argv[1..])
            .envs(cmd.env.iter().map(|(k, v)| (k, v)))
            .stdin(Stdio::null())
            .stdout(Stdio::from(out))
            .stderr(Stdio::from(err))
            .status()
            .await
            .map_err(|e| format!("{}: {e}", cmd.argv[0]))?;
        if !status.success() {
            let _ = std::fs::remove_dir_all(&venv);
            let reason = last_error_line_from(&log, offset).unwrap_or_else(|| format!("`{}` failed", cmd.display()));
            return Err(reason);
        }
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

