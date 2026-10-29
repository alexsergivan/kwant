//! One browser service per installation; isolated CDP contexts per bot/team.
//! The private CDP endpoint is used only by the server, never by the web client.
use super::browser::{BrowserController, BrowserError};
use super::computer::{self, ComputerMode};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::{collections::HashMap, path::{Path, PathBuf}, time::Duration};
use tokio::{process::Command, sync::Mutex, time::{sleep, timeout}};

/// The VM browser's image. Bump it whenever `docker/browser.Dockerfile` or
/// `docker/browser-entrypoint.sh` change: kwant builds an image only when its tag is missing, and
/// replaces a browser container made from another tag, so the bump is what ships the change.
pub const BROWSER_IMAGE: &str = "kwnt-browser:v2";
const BROWSER_DOCKERFILE: &str = include_str!("../../docker/browser.Dockerfile");
const BROWSER_ENTRYPOINT: &str = include_str!("../../docker/browser-entrypoint.sh");

#[derive(Clone, Serialize, Deserialize)]
struct SessionTarget {
    context: String,
    target: String,
}

pub struct BrowserService {
    root: PathBuf,
    endpoint: Option<String>,
    /// Local Chrome or the vm browser; changing it drops the connection (see [`Self::set_mode`]).
    mode: std::sync::RwLock<ComputerMode>,
    /// Local mode only: whether the person asked to see the kwant Chrome as a window (to sign in
    /// or solve a CAPTCHA). Otherwise it runs without one and the app streams it. Not saved: a
    /// restart goes back to the in-app view.
    local_window: std::sync::atomic::AtomicBool,
    // Serialize startup and session creation, including simultaneous status/frame requests.
    state: Mutex<Option<ServiceState>>,
    /// Why the last [`Self::session`] failed, for the error the person sees.
    last_error: std::sync::Mutex<Option<String>>,
    /// When a bot last used the browser (see [`Self::note_use`]).
    last_use: std::sync::Mutex<Option<std::time::SystemTime>>,
}

struct ServiceState {
    browser: BrowserController,
    endpoint: String,
    sessions: HashMap<String, SessionTarget>,
}

impl BrowserService {
    pub fn new(root: PathBuf, endpoint: Option<String>) -> Self {
        let mode = computer::current_mode(None);
        Self {
            root, endpoint, mode: std::sync::RwLock::new(mode),
            local_window: std::sync::atomic::AtomicBool::new(false), state: Mutex::new(None),
            last_error: std::sync::Mutex::new(None), last_use: std::sync::Mutex::new(None),
        }
    }

    pub fn mode(&self) -> ComputerMode {
        *self.mode.read().unwrap_or_else(|e| e.into_inner())
    }

    /// Whether local Chrome is shown as a window instead of in the app.
    pub fn local_window(&self) -> bool {
        self.local_window.load(std::sync::atomic::Ordering::SeqCst)
    }

    fn local_profile(&self) -> PathBuf { self.root.join("local-chrome") }

    /// Shows the local Chrome as a real window (`visible`) or runs it without one, streamed into
    /// the app. One profile cannot be open twice, so the running Chrome is closed and started
    /// again in the other form; each bot's open page is reopened there, and sign-ins saved in the
    /// profile carry over. On failure the previous form is kept.
    pub async fn set_local_window(&self, visible: bool) -> Result<(), BrowserError> {
        use std::sync::atomic::Ordering::SeqCst;
        let mut lock = self.state.lock().await;
        let previous = self.local_window.swap(visible, SeqCst);
        if self.mode() != ComputerMode::Local { return Ok(()); }
        // the page each bot had open, to reopen in the new Chrome
        let mut urls: HashMap<String, String> = HashMap::new();
        if let Some((endpoint, _)) = computer::running_local_chrome(&self.local_profile()).await {
            let browser = BrowserController::at_endpoint(&endpoint, None, true);
            if let Ok(targets) = browser.send_cdp_command("Target.getTargets", json!({})).await {
                for info in targets["targetInfos"].as_array().into_iter().flatten() {
                    if let (Some(id), Some(url)) = (info["targetId"].as_str(), info["url"].as_str()) {
                        urls.insert(id.to_string(), url.to_string());
                    }
                }
            }
        }
        *lock = None;
        let mut state = match self.connect().await {
            Ok(state) => state,
            Err(error) => { self.local_window.store(previous, SeqCst); return Err(error); }
        };
        let live = state.browser.send_cdp_command("Target.getTargets", json!({})).await?;
        let live: Vec<(String, bool)> = live["targetInfos"].as_array().into_iter().flatten()
            .filter_map(|t| Some((t["targetId"].as_str()?.to_string(), t["type"] == "page" && t["url"] == "about:blank")))
            .collect();
        // Reopen only the pages that showed a site. A blank page would be one more empty window
        // per bot that ever used the browser; its bot gets a new one when it next asks.
        let mut dropped = Vec::new();
        for (bot, session) in state.sessions.iter_mut() {
            if live.iter().any(|(id, _)| *id == session.target) { continue; }
            let Some(url) = urls.get(&session.target).filter(|u| u.starts_with("http")) else {
                dropped.push(bot.clone());
                continue;
            };
            match state.browser.send_cdp_command("Target.createTarget", json!({"url": url, "newWindow": true})).await {
                Ok(target) => if let Some(id) = target["targetId"].as_str() { session.target = id.to_string(); },
                Err(_) => dropped.push(bot.clone()),
            }
        }
        for bot in dropped { state.sessions.remove(&bot); }
        // Chrome's own blank start window would sit on top of the bots' pages in the window
        if visible && !state.sessions.is_empty() {
            for (id, _) in live.iter().filter(|(id, blank)| *blank && !state.sessions.values().any(|s| s.target == *id)) {
                let _ = state.browser.send_cdp_command("Target.closeTarget", json!({"targetId": id})).await;
            }
        }
        self.save_sessions(&state.sessions).await?;
        *lock = Some(state);
        Ok(())
    }

    /// The local Chrome's endpoint, in the form asked for (window or not), starting it if needed.
    async fn local_endpoint(&self) -> Result<String, BrowserError> {
        let profile = self.local_profile();
        let headless = !self.local_window();
        // at most one kwant Chrome per profile: reuse it, or wait for the one starting
        let (endpoint, running_headless) = computer::ensure_local_chrome(&profile, headless).await
            .map_err(BrowserError::Http)?;
        // a windowless Chrome started by an older kwant tells sites it is headless. Only that one,
        // which left no launch record: one kwant started itself is never restarted for it, so a
        // Chrome that ignores the user agent can't be restarted on every call
        let stale = running_headless && headless && computer::launched_headless(&profile).is_none()
            && computer::says_headless(&endpoint).await
            && computer::local_chrome_user_agent().await.is_some();
        if running_headless == headless && !stale { return Ok(endpoint); }
        // the other form (or that older one) is running on this profile: close it first
        let _ = BrowserController::at_endpoint(&endpoint, None, true)
            .send_cdp_command("Browser.close", json!({})).await;
        computer::wait_local_chrome_gone(&profile).await.map_err(BrowserError::Http)?;
        let (endpoint, _) = computer::ensure_local_chrome(&profile, headless).await.map_err(BrowserError::Http)?;
        Ok(endpoint)
    }

    async fn save_sessions(&self, sessions: &HashMap<String, SessionTarget>) -> Result<(), BrowserError> {
        tokio::fs::create_dir_all(&self.root).await.map_err(|e| BrowserError::Http(e.to_string()))?;
        let file = self.sessions_file();
        tokio::fs::write(file.with_extension("tmp"), serde_json::to_vec(sessions).unwrap()).await
            .map_err(|e| BrowserError::Http(e.to_string()))?;
        tokio::fs::rename(file.with_extension("tmp"), file).await.map_err(|e| BrowserError::Http(e.to_string()))
    }

    /// Whether a browser service URL (`KWNT_BROWSER_CDP_URL`) is configured for vm mode.
    pub fn has_endpoint(&self) -> bool { self.endpoint.is_some() }

    /// Switches where the browser runs. The current connection is dropped; the vm container and
    /// a local Chrome window keep running, while a windowless local Chrome is closed, since
    /// nothing would show it. Returns whether the mode changed.
    pub async fn set_mode(&self, mode: ComputerMode) -> bool {
        let mut lock = self.state.lock().await;
        let previous = {
            let mut current = self.mode.write().unwrap_or_else(|e| e.into_inner());
            if *current == mode { return false; }
            std::mem::replace(&mut *current, mode)
        };
        *lock = None;
        if previous == ComputerMode::Local {
            if let Some((endpoint, true)) = computer::running_local_chrome(&self.local_profile()).await {
                let _ = BrowserController::at_endpoint(&endpoint, None, true)
                    .send_cdp_command("Browser.close", json!({})).await;
            }
        }
        true
    }

    /// Each mode keeps its own saved pages, since they live in different browsers.
    fn sessions_file(&self) -> PathBuf {
        self.root.join(match self.mode() {
            ComputerMode::Vm => "browser-sessions.json",
            ComputerMode::Local => "browser-sessions-local.json",
        })
    }

    async fn docker(args: &[&str], seconds: u64) -> Result<String, BrowserError> {
        let out = timeout(Duration::from_secs(seconds), Command::new("docker")
            .args(args).kill_on_drop(true).output()).await
            .map_err(|_| BrowserError::Timeout)?
            .map_err(|e| BrowserError::Http(e.to_string()))?;
        if !out.status.success() {
            return Err(BrowserError::Protocol(String::from_utf8_lossy(&out.stderr).trim().to_string()));
        }
        Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
    }

    async fn managed_endpoint(&self) -> Result<String, BrowserError> {
        let name = managed_container_name(&self.root);
        let found = Self::docker(&["inspect", "--format",
            &format!("{{{{.Config.Image}}}}|{{{{index .Config.Labels \"{LABEL_BROWSER}\"}}}}"), &name], 5).await.ok();
        // A container made from an older image keeps that image's browser, and one made before
        // the labels (by an older kwant) restarts on boot and can't be reaped: make a new one.
        // Its profile is on tmpfs, so nothing is lost; cookies are kept in the data folder.
        let current = found.as_deref() == Some(&format!("{BROWSER_IMAGE}|1"));
        if found.is_some() && !current {
            Self::docker(&["rm", "-f", &name], 30).await?;
        }
        self.note_use();
        if current {
            if !matches!(Self::docker(&["inspect", "--format", "{{.State.Running}}", &name], 5).await.as_deref(), Ok("true")) {
                Self::docker(&["start", &name], 30).await?;
            }
        } else {
            if Self::docker(&["image", "inspect", BROWSER_IMAGE], 5).await.is_err() {
                let build = self.root.join("browser-image");
                tokio::fs::create_dir_all(&build).await.map_err(|e| BrowserError::Http(e.to_string()))?;
                tokio::fs::write(build.join("Dockerfile"), BROWSER_DOCKERFILE).await
                    .map_err(|e| BrowserError::Http(e.to_string()))?;
                tokio::fs::write(build.join("browser-entrypoint.sh"), BROWSER_ENTRYPOINT).await
                    .map_err(|e| BrowserError::Http(e.to_string()))?;
                Self::docker(&["build", "-t", BROWSER_IMAGE, &build.to_string_lossy()], 600).await?;
            }
            let args = managed_run_args(&name, &self.root);
            let args: Vec<&str> = args.iter().map(String::as_str).collect();
            Self::docker(&args, 30).await?;
        }
        let binding = Self::docker(&["port", &name, "9222/tcp"], 5).await?;
        let port = binding.lines().next().and_then(|line| line.rsplit(':').next())
            .and_then(|p| p.parse::<u16>().ok()).ok_or_else(|| BrowserError::Protocol("Browser port missing".into()))?;
        Ok(format!("http://127.0.0.1:{port}"))
    }

    /// Records that a bot used the browser, for the idle reaper (see [`reap_containers`]). The
    /// time is kept in a file in the data folder so another kwant's reaper sees it too; it is
    /// written at most once a minute.
    pub fn note_use(&self) {
        let now = std::time::SystemTime::now();
        let mut last = self.last_use.lock().unwrap_or_else(|e| e.into_inner());
        if last.is_some_and(|t| now.duration_since(t).is_ok_and(|d| d < Duration::from_secs(60))) { return; }
        *last = Some(now);
        let _ = std::fs::create_dir_all(&self.root);
        let _ = std::fs::write(self.root.join(LAST_USE_FILE), b"");
    }

    /// Stops this kwant's browser container, if it runs one. Called when kwant shuts down; the
    /// next browser request starts it again. Its pages close; cookies are kept in the data folder.
    pub async fn shutdown(&self) {
        let mut lock = self.state.lock().await;
        *lock = None;
        if self.endpoint.is_some() { return; }
        let name = managed_container_name(&self.root);
        if matches!(Self::docker(&["inspect", "--format", "{{.State.Running}}", &name], 5).await.as_deref(), Ok("true")) {
            match Self::docker(&["stop", "-t", "5", &name], 20).await {
                Ok(_) => tracing::info!("Stopped the browser container {name}"),
                Err(e) => tracing::warn!("Could not stop the browser container {name}: {e}"),
            }
        }
    }

    /// Removes kwant browser containers whose data folder is gone and stops ones nobody has used
    /// for `idle` (see [`reap_action`]). This kwant's own container is stopped under the state
    /// lock, so no request is using it; the next one starts it again. Returns (removed, stopped).
    pub async fn reap_containers(&self, idle: Duration) -> (usize, usize) {
        let Ok(list) = Self::docker(&["ps", "-a", "--filter", &format!("label={LABEL_BROWSER}=1"),
            "--format", &format!("{{{{.Names}}}}\t{{{{.State}}}}\t{{{{.Label \"{LABEL_ROOT}\"}}}}")], 10).await
        else { return (0, 0) };
        let own = managed_container_name(&self.root);
        let (mut removed, mut stopped) = (0, 0);
        for line in list.lines() {
            let mut parts = line.splitn(3, '\t');
            let (Some(name), Some(state), Some(root)) = (parts.next(), parts.next(), parts.next()) else { continue };
            let root = Path::new(root);
            // a relative or missing label can't be checked from here: never treat it as gone
            let root_gone = root.is_absolute() && !root.is_dir();
            let idle_for = std::fs::metadata(root.join(LAST_USE_FILE)).and_then(|m| m.modified()).ok()
                .and_then(|t| std::time::SystemTime::now().duration_since(t).ok());
            let action = reap_action(state == "running", root_gone, idle_for, idle);
            match action {
                ReapAction::Keep => {}
                ReapAction::Remove => {
                    if name == own { *self.state.lock().await = None; }
                    if Self::docker(&["rm", "-f", name], 30).await.is_ok() { removed += 1; }
                }
                ReapAction::Stop => {
                    // another request may be using our own container: wait for it, then re-check
                    let guard = if name == own { Some(self.state.lock().await) } else { None };
                    if name == own {
                        let fresh = self.last_use.lock().unwrap_or_else(|e| e.into_inner())
                            .and_then(|t| std::time::SystemTime::now().duration_since(t).ok());
                        if fresh.is_some_and(|d| d < idle) { continue; }
                    }
                    if let Some(mut guard) = guard { *guard = None; }
                    if Self::docker(&["stop", "-t", "5", name], 20).await.is_ok() { stopped += 1; }
                }
            }
        }
        (removed, stopped)
    }

    /// Runs [`Self::reap_containers`] at startup and then every [`REAP_EVERY`] until `cancel`.
    /// Does nothing when kwant uses an outside browser service and runs no container of its own,
    /// or when Docker isn't there.
    pub fn spawn_reaper(self: std::sync::Arc<Self>, cancel: tokio_util::sync::CancellationToken) {
        if self.endpoint.is_some() { return; }
        let idle = idle_limit();
        tokio::spawn(async move {
            loop {
                let (removed, stopped) = self.reap_containers(idle).await;
                if removed + stopped > 0 {
                    tracing::info!("browser: removed {removed} and stopped {stopped} leftover browser containers");
                }
                tokio::select! {
                    _ = cancel.cancelled() => return,
                    _ = sleep(REAP_EVERY) => {}
                }
            }
        });
    }

    async fn connect(&self) -> Result<ServiceState, BrowserError> {
        let local = self.mode() == ComputerMode::Local;
        let endpoint = match &self.endpoint {
            _ if local => self.local_endpoint().await?,
            Some(endpoint) => endpoint.trim_end_matches('/').to_string(),
            None => self.managed_endpoint().await?,
        };
        let browser = BrowserController::at_endpoint(&endpoint, None, true);
        if self.endpoint.is_some() || local {
            browser.send_cdp_command("Browser.getVersion", json!({})).await?;
        } else { timeout(Duration::from_secs(30), async {
            loop {
                if browser.send_cdp_command("Browser.getVersion", json!({})).await.is_ok() { break; }
                sleep(Duration::from_millis(250)).await;
            }
        }).await.map_err(|_| BrowserError::Timeout)?; }
        let sessions = tokio::fs::read(self.sessions_file()).await.ok()
            .and_then(|bytes| serde_json::from_slice(&bytes).ok()).unwrap_or_default();
        Ok(ServiceState { browser, endpoint, sessions })
    }

    /// What to tell the person when the browser could not be reached. In local mode that is the
    /// reason kwant's Chrome did not start, not Docker, which local mode does not use.
    pub fn unavailable_reason(&self) -> String {
        if self.mode() != ComputerMode::Local {
            return "Sandbox browser is unavailable. Check that Docker is running, then restart the browser.".into();
        }
        let last = self.last_error.lock().unwrap_or_else(|e| e.into_inner()).clone();
        match last {
            Some(reason) => format!("The bot browser could not start kwant's Chrome on this computer: {reason}"),
            None => "The bot browser could not start kwant's Chrome on this computer. Restart the browser and try again.".into(),
        }
    }

    pub async fn session(&self, id: &str, workspace: &Path) -> Result<BrowserController, BrowserError> {
        let result = self.open_session(id, workspace).await;
        *self.last_error.lock().unwrap_or_else(|e| e.into_inner()) = result.as_ref().err().map(|e| match e {
            // the local start's own words, without the CDP prefix
            BrowserError::Http(reason) => reason.clone(),
            other => other.to_string(),
        });
        result
    }

    async fn open_session(&self, id: &str, _workspace: &Path) -> Result<BrowserController, BrowserError> {
        let mut lock = self.state.lock().await;
        if lock.is_none() { *lock = Some(self.connect().await?); }
        let mut targets = lock.as_ref().unwrap().browser.send_cdp_command("Target.getTargets", json!({})).await;
        if targets.is_err() && self.mode() == ComputerMode::Local {
            // Chrome quit, or the person closed its window once done there: start it again in
            // this same call, back in the app
            self.local_window.store(false, std::sync::atomic::Ordering::SeqCst);
            *lock = None;
            *lock = Some(self.connect().await?);
            targets = lock.as_ref().unwrap().browser.send_cdp_command("Target.getTargets", json!({})).await;
        }
        let state = lock.as_mut().unwrap();
        let targets = match targets {
            Ok(targets) => targets,
            Err(error) => { *lock = None; return Err(error); }
        };
        let existing = state.sessions.get(id).filter(|session| targets["targetInfos"].as_array()
            .is_some_and(|targets| targets.iter().any(|target| target["targetId"] == session.target)));
        let local = self.mode() == ComputerMode::Local;
        let fresh_context = existing.is_none() && !local;
        let session = if let Some(existing) = existing { existing.clone() } else if local {
            // Local pages share the kwant profile's default context, so a sign-in in the window
            // is kept. (`context` stays empty.) Each bot gets its own window, not a tab: Chrome
            // only paints the front tab of a window, so a background tab's screenshot never comes.
            let target = state.browser.send_cdp_command("Target.createTarget", json!({"url": "about:blank", "newWindow": true})).await?;
            SessionTarget { context: String::new(), target: target["targetId"].as_str()
                .ok_or_else(|| BrowserError::Protocol("Browser page missing".into()))?.to_string() }
        } else {
            // Do not dispose on detach: contexts survive a Kwnt server restart.
            let context = state.browser.send_cdp_command("Target.createBrowserContext", json!({"disposeOnDetach": false})).await?;
            let context = context["browserContextId"].as_str()
                .ok_or_else(|| BrowserError::Protocol("Browser context missing".into()))?.to_string();
            let target = state.browser.send_cdp_command("Target.createTarget", json!({
                "url": "about:blank", "browserContextId": context
            })).await;
            let target = match target {
                Ok(target) => target,
                Err(error) => {
                    let _ = state.browser.send_cdp_command("Target.disposeBrowserContext", json!({"browserContextId": context})).await;
                    return Err(error);
                }
            };
            SessionTarget { context, target: target["targetId"].as_str()
                .ok_or_else(|| BrowserError::Protocol("Browser page missing".into()))?.to_string() }
        };
        state.sessions.insert(id.to_string(), session.clone());
        self.save_sessions(&state.sessions).await?;
        let ctrl = BrowserController::at_endpoint(&state.endpoint, Some(session.target), false);
        if !local {
            // The VM browser's profile is on tmpfs, so its contexts' cookies go when the
            // container restarts: keep them per bot in the data folder (local mode's on-disk
            // profile already keeps them).
            ctrl.set_cookie_jar(Some(self.cookie_jar(id)));
            if fresh_context {
                let restored = ctrl.restore_cookies().await;
                if restored > 0 { tracing::debug!("Restored {} browser cookies for {}", restored, id); }
            }
        }
        // A real window keeps its own size; a windowless browser gets the viewer's fixed viewport.
        if !local || !self.local_window() {
            ctrl.set_viewport(1280, 800).await?;
        }
        ctrl.refresh_location().await?;
        Ok(ctrl)
    }

    /// Where a bot's VM-browser cookies are kept between container restarts.
    pub fn cookie_jar(&self, id: &str) -> PathBuf {
        let name: String = id.chars().map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == '_' { c } else { '_' }).take(120).collect();
        self.root.join("browser-cookies").join(format!("{name}.json"))
    }

    pub async fn reset(&self, id: &str) -> Result<(), BrowserError> {
        // A reset is a clean slate, including the saved sign-ins
        let _ = std::fs::remove_file(self.cookie_jar(id));
        let mut lock = self.state.lock().await;
        if lock.is_none() && self.sessions_file().exists() {
            *lock = Some(self.connect().await?);
        }
        if let Some(state) = lock.as_mut() {
            if let Some(session) = state.sessions.remove(id) {
                if session.context.is_empty() {
                    // a local tab in the shared profile: close just that tab
                    let _ = state.browser.send_cdp_command("Target.closeTarget", json!({"targetId": session.target})).await;
                } else {
                    state.browser.send_cdp_command("Target.disposeBrowserContext", json!({"browserContextId": session.context})).await?;
                }
            }
        }
        Ok(())
    }
}

/// Labels on every browser container kwant starts: what it is, and whose data folder it serves.
pub const LABEL_BROWSER: &str = "kwnt.browser";
pub const LABEL_ROOT: &str = "kwnt.root";
/// Touched in the data folder whenever a bot uses the browser; its age is the container's idle time.
pub const LAST_USE_FILE: &str = "browser-last-used";
/// A browser container nobody has used for this long is stopped (`KWNT_BROWSER_IDLE_MINUTES`).
pub const DEFAULT_IDLE_MINUTES: u64 = 30;
/// How often a running kwant looks for leftover browser containers.
pub const REAP_EVERY: Duration = Duration::from_secs(10 * 60);

/// The idle limit: `KWNT_BROWSER_IDLE_MINUTES` (at least 1), else [`DEFAULT_IDLE_MINUTES`].
pub fn idle_limit() -> Duration {
    let minutes = std::env::var("KWNT_BROWSER_IDLE_MINUTES").ok()
        .and_then(|v| v.trim().parse::<u64>().ok()).filter(|m| *m > 0).unwrap_or(DEFAULT_IDLE_MINUTES);
    Duration::from_secs(minutes * 60)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReapAction { Keep, Stop, Remove }

/// What the reaper does with one labelled browser container. Its data folder gone: remove it,
/// running or not, since no kwant can use it again. Running and unused for `limit`, or with no
/// record of use: stop it; kwant starts it again on the next browser request. Otherwise keep it.
pub fn reap_action(running: bool, root_gone: bool, idle_for: Option<Duration>, limit: Duration) -> ReapAction {
    if root_gone { return ReapAction::Remove; }
    if running && idle_for.is_none_or(|d| d >= limit) { return ReapAction::Stop; }
    ReapAction::Keep
}

/// The `docker run` arguments for the browser container of the kwant whose data folder is
/// `root`. It never restarts on its own (kwant starts it on demand), and it carries labels so a
/// later kwant can find it, stop it when idle and remove it once its data folder is gone.
pub fn managed_run_args(name: &str, root: &Path) -> Vec<String> {
    let root = std::fs::canonicalize(root).unwrap_or_else(|_| root.to_path_buf());
    [
        "run", "-d", "--name", name, "--restart", "no",
        "--label", &format!("{LABEL_BROWSER}=1"), "--label", &format!("{LABEL_ROOT}={}", root.display()),
        "--init", "--cap-drop=ALL", "--security-opt=no-new-privileges", "--memory=1536m",
        "--cpus=2", "--pids-limit=512", "--read-only", "--tmpfs", "/tmp:rw,nosuid,size=512m",
        "-p", "127.0.0.1::9222", BROWSER_IMAGE,
    ].iter().map(|a| a.to_string()).collect()
}

/// The VM browser container of the kwant whose data folder is `root`: one name per installation
/// avoids collisions between local kwant instances.
pub fn managed_container_name(root: &Path) -> String {
    use std::hash::{Hash, Hasher};
    let mut hash = std::collections::hash_map::DefaultHasher::new();
    root.hash(&mut hash);
    format!("kwnt-browser-{:016x}", hash.finish())
}

#[cfg(test)]
mod reaper_tests {
    use super::*;

    const LIMIT: Duration = Duration::from_secs(30 * 60);
    const MIN: Duration = Duration::from_secs(60);

    #[test]
    fn a_container_whose_data_folder_is_gone_is_removed_running_or_not() {
        for running in [true, false] {
            for idle in [None, Some(MIN), Some(LIMIT * 2)] {
                assert_eq!(reap_action(running, true, idle, LIMIT), ReapAction::Remove);
            }
        }
    }

    #[test]
    fn a_running_container_is_stopped_only_once_idle_past_the_limit() {
        assert_eq!(reap_action(true, false, Some(MIN), LIMIT), ReapAction::Keep);
        assert_eq!(reap_action(true, false, Some(LIMIT - MIN), LIMIT), ReapAction::Keep);
        assert_eq!(reap_action(true, false, Some(LIMIT), LIMIT), ReapAction::Stop);
        assert_eq!(reap_action(true, false, Some(LIMIT * 10), LIMIT), ReapAction::Stop);
        // never used since kwant started tracking it: nothing is waiting on it
        assert_eq!(reap_action(true, false, None, LIMIT), ReapAction::Stop);
    }

    #[test]
    fn a_stopped_container_with_its_folder_is_left_for_kwant_to_start_again() {
        for idle in [None, Some(MIN), Some(LIMIT * 10)] {
            assert_eq!(reap_action(false, false, idle, LIMIT), ReapAction::Keep);
        }
    }

    #[test]
    fn the_container_never_restarts_on_its_own_and_carries_its_labels() {
        let tmp = tempfile::tempdir().unwrap();
        let root = std::fs::canonicalize(tmp.path()).unwrap();
        let args = managed_run_args("kwnt-browser-x", tmp.path());
        let pairs: Vec<(&str, &str)> = args.windows(2).map(|w| (w[0].as_str(), w[1].as_str())).collect();
        assert!(pairs.contains(&("--restart", "no")), "{args:?}");
        assert!(!args.iter().any(|a| a.contains("unless-stopped") || a.contains("always")), "{args:?}");
        assert!(pairs.contains(&("--label", "kwnt.browser=1")), "{args:?}");
        let root_label = format!("kwnt.root={}", root.display());
        assert!(pairs.contains(&("--label", root_label.as_str())), "{args:?}");
        assert!(pairs.contains(&("--name", "kwnt-browser-x")), "{args:?}");
        assert_eq!(args.last().map(String::as_str), Some(BROWSER_IMAGE));
        // the published port stays loopback-only
        assert!(pairs.contains(&("-p", "127.0.0.1::9222")), "{args:?}");
    }

    #[test]
    fn a_relative_data_folder_is_labelled_absolute() {
        let args = managed_run_args("kwnt-browser-x", Path::new("."));
        let label = args.iter().find(|a| a.starts_with("kwnt.root=")).unwrap();
        assert!(Path::new(label.trim_start_matches("kwnt.root=")).is_absolute(), "{label}");
    }

    #[test]
    fn use_is_recorded_in_the_data_folder_at_most_once_a_minute() {
        let tmp = tempfile::tempdir().unwrap();
        let service = BrowserService::new(tmp.path().to_path_buf(), None);
        service.note_use();
        let file = tmp.path().join(LAST_USE_FILE);
        let first = std::fs::metadata(&file).unwrap().modified().unwrap();
        std::fs::remove_file(&file).unwrap();
        service.note_use();
        assert!(!file.exists(), "a second use within a minute writes nothing");
        assert!(first <= std::time::SystemTime::now());
    }
}

#[cfg(test)]
mod image_tests {
    use super::{BROWSER_DOCKERFILE, BROWSER_ENTRYPOINT, BROWSER_IMAGE};
    use sha2::{Digest, Sha256};

    /// The browser files `kwnt-browser:v2` was built from. A change to them without a new
    /// [`BROWSER_IMAGE`] would never reach a computer that already has the old tag.
    #[test]
    fn the_browser_image_tag_moves_with_its_files() {
        let mut files = Sha256::new();
        files.update(BROWSER_DOCKERFILE);
        files.update(BROWSER_ENTRYPOINT);
        let hash: String = files.finalize().iter().take(8).map(|b| format!("{b:02x}")).collect();
        assert_eq!(
            (BROWSER_IMAGE, hash.as_str()),
            ("kwnt-browser:v2", "52d86739629ce24f"),
            "docker/browser files changed: bump BROWSER_IMAGE, then record the new hash here"
        );
    }
}
