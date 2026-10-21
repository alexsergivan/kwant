//! One browser service per installation; isolated CDP contexts per bot/team.
//! The private CDP endpoint is used only by the server, never by the web client.
use super::browser::{BrowserController, BrowserError};
use super::computer::{self, ComputerMode};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::{collections::HashMap, path::{Path, PathBuf}, time::Duration};
use tokio::{process::Command, sync::Mutex, time::{sleep, timeout}};

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
            last_error: std::sync::Mutex::new(None),
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
        if running_headless == headless { return Ok(endpoint); }
        // the other form is running on this profile: close it first
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
        // A deterministic installation name avoids collisions between local Kwnt instances.
        use std::hash::{Hash, Hasher};
        let mut hash = std::collections::hash_map::DefaultHasher::new();
        self.root.hash(&mut hash);
        let name = format!("kwnt-browser-{:016x}", hash.finish());
        if !matches!(Self::docker(&["inspect", "--format", "{{.State.Running}}", &name], 5).await.as_deref(), Ok("true")) {
            if Self::docker(&["inspect", "--format", "{{.Id}}", &name], 5).await.is_ok() {
                Self::docker(&["start", &name], 30).await?;
            } else {
                if Self::docker(&["image", "inspect", "kwnt-browser:v1"], 5).await.is_err() {
                    let build = self.root.join("browser-image");
                    tokio::fs::create_dir_all(&build).await.map_err(|e| BrowserError::Http(e.to_string()))?;
                    tokio::fs::write(build.join("Dockerfile"), include_str!("../../docker/browser.Dockerfile")).await
                        .map_err(|e| BrowserError::Http(e.to_string()))?;
                    tokio::fs::write(build.join("browser-entrypoint.sh"), include_str!("../../docker/browser-entrypoint.sh")).await
                        .map_err(|e| BrowserError::Http(e.to_string()))?;
                    Self::docker(&["build", "-t", "kwnt-browser:v1", &build.to_string_lossy()], 600).await?;
                }
                Self::docker(&["run", "-d", "--name", &name, "--restart", "unless-stopped",
                    "--init", "--cap-drop=ALL", "--security-opt=no-new-privileges", "--memory=1536m",
                    "--cpus=2", "--pids-limit=512", "--read-only", "--tmpfs", "/tmp:rw,nosuid,size=512m",
                    "-p", "127.0.0.1::9222", "kwnt-browser:v1"], 30).await?;
            }
        }
        let binding = Self::docker(&["port", &name, "9222/tcp"], 5).await?;
        let port = binding.lines().next().and_then(|line| line.rsplit(':').next())
            .and_then(|p| p.parse::<u16>().ok()).ok_or_else(|| BrowserError::Protocol("Browser port missing".into()))?;
        Ok(format!("http://127.0.0.1:{port}"))
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
        // A real window keeps its own size; a windowless browser gets the viewer's fixed viewport.
        if !local || !self.local_window() {
            ctrl.send_cdp_command("Emulation.setDeviceMetricsOverride", json!({
                "width": 1280, "height": 800, "deviceScaleFactor": 1, "mobile": false
            })).await?;
        }
        ctrl.refresh_location().await?;
        Ok(ctrl)
    }

    pub async fn reset(&self, id: &str) -> Result<(), BrowserError> {
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
