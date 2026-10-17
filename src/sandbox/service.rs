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
    // Serialize startup and session creation, including simultaneous status/frame requests.
    state: Mutex<Option<ServiceState>>,
}

struct ServiceState {
    browser: BrowserController,
    endpoint: String,
    sessions: HashMap<String, SessionTarget>,
}

impl BrowserService {
    pub fn new(root: PathBuf, endpoint: Option<String>) -> Self {
        let mode = computer::current_mode(None);
        Self { root, endpoint, mode: std::sync::RwLock::new(mode), state: Mutex::new(None) }
    }

    pub fn mode(&self) -> ComputerMode {
        *self.mode.read().unwrap_or_else(|e| e.into_inner())
    }

    /// Whether a browser service URL (`KWNT_BROWSER_CDP_URL`) is configured for vm mode.
    pub fn has_endpoint(&self) -> bool { self.endpoint.is_some() }

    /// Switches where the browser runs. The current connection is dropped, not the browser:
    /// the local Chrome window and the vm container both keep running. Returns whether the
    /// mode changed.
    pub async fn set_mode(&self, mode: ComputerMode) -> bool {
        let mut lock = self.state.lock().await;
        let mut current = self.mode.write().unwrap_or_else(|e| e.into_inner());
        if *current == mode { return false; }
        *current = mode;
        *lock = None;
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
            _ if local => computer::ensure_local_chrome(&self.root.join("local-chrome")).await
                .map_err(BrowserError::Http)?,
            Some(endpoint) => endpoint.trim_end_matches('/').to_string(),
            None => self.managed_endpoint().await?,
        };
        let browser = BrowserController::at_endpoint(&endpoint, None, None, true);
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

    pub async fn session(&self, id: &str, workspace: &Path) -> Result<BrowserController, BrowserError> {
        let mut lock = self.state.lock().await;
        if lock.is_none() { *lock = Some(self.connect().await?); }
        let mut targets = lock.as_ref().unwrap().browser.send_cdp_command("Target.getTargets", json!({})).await;
        if targets.is_err() && self.mode() == ComputerMode::Local {
            // the person closed the Chrome window: open it again in this same call
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
            // is kept; each bot still gets its own tab. (`context` stays empty.)
            let target = state.browser.send_cdp_command("Target.createTarget", json!({"url": "about:blank"})).await?;
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
        tokio::fs::create_dir_all(&self.root).await.map_err(|e| BrowserError::Http(e.to_string()))?;
        let file = self.sessions_file();
        tokio::fs::write(file.with_extension("tmp"), serde_json::to_vec(&state.sessions).unwrap()).await
            .map_err(|e| BrowserError::Http(e.to_string()))?;
        tokio::fs::rename(file.with_extension("tmp"), file).await.map_err(|e| BrowserError::Http(e.to_string()))?;
        let ctrl = BrowserController::at_endpoint(&state.endpoint, Some(session.target), Some(workspace.to_path_buf()), false);
        // A real window keeps its own size; only the headless vm browser gets a fixed viewport.
        if !local {
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
