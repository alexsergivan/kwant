//! Updates. The Mac shell (apps/mac) links this crate, runs the server in its own process and
//! registers an [`Updater`] on the shared [`UpdateHub`] that installs (`can_install: true`); the
//! server's `/api/app/update` routes read and drive the hub. Anywhere else (`kwnt serve`, Docker, a
//! VPS, the Windows beta) the server registers [`release_check`] instead: it reads the same
//! `latest.json` the Mac app does, on start and every 6 hours, and says how this install updates
//! ([`How`]) with `can_install: false`. `KWNT_UPDATE_CHECK=off` turns that off (`supported: false`).
//!
//! The hub owns the state machine: idle → checking → available → downloading → ready, with error
//! from checking or downloading. The updater only does the work (fetch, verify, install, relaunch)
//! and reports back through [`UpdateHub::found`], [`UpdateHub::progress`], [`UpdateHub::ready`] and
//! [`UpdateHub::failed`].

use serde::Serialize;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;
use tokio::sync::watch;

/// The release feed both the Mac app and the server read: the newest non-prerelease release's
/// `latest.json` (`{version, notes, pub_date, ...}`), written by scripts/mac_latest_json.py.
pub const LATEST_JSON: &str =
    "https://github.com/alexsergivan/kwant/releases/latest/download/latest.json";
pub const RELEASES: &str = "https://github.com/alexsergivan/kwant/releases";
/// How often the server looks again on its own; "Check again" looks at once.
pub const RECHECK_EVERY: Duration = Duration::from_secs(6 * 60 * 60);
/// A check that takes longer than this says it couldn't check, never "latest".
pub const CHECK_TIMEOUT: Duration = Duration::from_secs(8);

/// A version's release page: "What's new" and "Download".
pub fn release_page(version: &str) -> String {
    format!("{RELEASES}/tag/mac-v{version}")
}

/// What the updater is doing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum State {
    Idle,
    Checking,
    Available,
    Downloading,
    /// Installed; the app is relaunching on the new version.
    Ready,
    Error,
}

/// A newer release than the running app.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Available {
    pub version: String,
    /// That version's CHANGELOG section (markdown).
    pub notes: String,
    /// RFC 3339 publish date, when the release says.
    pub date: Option<String>,
    /// The release page, for "What's new" and "Download manually".
    pub url: String,
}

/// How an install the app can't update by itself gets the new version: shown under "Update".
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct How {
    /// `docker`, `binary` (a prebuilt `kwnt` on Linux), `windows` (the beta installer) or `source`
    /// (built from a clone).
    pub kind: &'static str,
    /// The lines to run, one per line; `None` when it's a download (Windows) or a build.
    pub command: Option<String>,
}

/// `GET /api/app/update`.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Status {
    /// Whether this server knows about updates: the app's own updater, or the release check.
    pub supported: bool,
    /// Whether "Update" installs here (the Mac app). Otherwise `how` says what to run.
    pub can_install: bool,
    /// The running app's version (the kwnt crate's version off-Mac).
    pub current: String,
    pub state: State,
    pub available: Option<Available>,
    /// Download progress 0–1, while downloading and the size is known.
    pub progress: Option<f64>,
    pub error: Option<String>,
    /// When the last check finished, found something or not (RFC 3339). `None` before the first.
    pub checked_at: Option<String>,
    /// With `can_install: false`: how this install updates.
    pub how: Option<How>,
}

/// The Mac shell's side: does the work, reports back through the hub. Both calls return at once.
pub trait Updater: Send + Sync + 'static {
    /// Asks the release feed for a newer version; ends in [`UpdateHub::found`] or
    /// [`UpdateHub::failed`].
    fn check(&self);
    /// Downloads `version`, verifies its signature, swaps the app and relaunches; reports
    /// [`UpdateHub::progress`], then [`UpdateHub::ready`] or [`UpdateHub::failed`]. On failure the
    /// running app is untouched.
    fn install(&self, version: &str);
}

/// Why a check or install can't start now.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Refused {
    /// No updater at all.
    Unsupported,
    /// Install where only the release check runs (not the Mac app).
    CannotInstall,
    /// Already checking, downloading or relaunching.
    Busy,
    /// Install without a newer version found.
    NothingToInstall,
}

impl Refused {
    pub fn message(&self) -> &'static str {
        match self {
            Refused::Unsupported => "this kwant doesn't check for updates (KWNT_UPDATE_CHECK=off)",
            Refused::CannotInstall => {
                "this kwant updates with a command, not from the app: see How to update"
            }
            Refused::Busy => "an update check or install is already running",
            Refused::NothingToInstall => "no newer version is available",
        }
    }
}

pub struct UpdateHub {
    updater: Mutex<Option<Arc<dyn Updater>>>,
    status: watch::Sender<Status>,
    /// Set by the desktop app before it starts the server: its own updater registers shortly, so
    /// the server doesn't start the release check.
    desktop_app: AtomicBool,
}

impl Default for UpdateHub {
    fn default() -> Self {
        Self::new()
    }
}

impl UpdateHub {
    pub fn new() -> Self {
        let (status, _) = watch::channel(Status {
            supported: false,
            can_install: false,
            current: env!("CARGO_PKG_VERSION").to_string(),
            state: State::Idle,
            available: None,
            progress: None,
            error: None,
            checked_at: None,
            how: None,
        });
        Self {
            updater: Mutex::new(None),
            status,
            desktop_app: AtomicBool::new(false),
        }
    }

    /// The hub of this process: the Mac shell registers on it, the server reads it.
    pub fn shared() -> Arc<UpdateHub> {
        static SHARED: OnceLock<Arc<UpdateHub>> = OnceLock::new();
        SHARED.get_or_init(|| Arc::new(UpdateHub::new())).clone()
    }

    /// Makes updates supported and installable, running `current` (the app's version, not the
    /// crate's). Replaces a release check registered before it.
    pub fn register(&self, current: &str, updater: Arc<dyn Updater>) {
        *self.updater.lock().unwrap() = Some(updater);
        self.status.send_modify(|s| {
            s.supported = true;
            s.can_install = true;
            s.how = None;
            s.current = current.to_string();
        });
    }

    /// The desktop app runs this server and registers its own updater: no release check here.
    pub fn mark_desktop_app(&self) {
        self.desktop_app.store(true, Ordering::Relaxed);
    }

    /// Whether the app's own updater runs (or is about to): the release check stays out.
    pub fn app_updates(&self) -> bool {
        self.desktop_app.load(Ordering::Relaxed) || self.status.borrow().can_install
    }

    /// Off the app: a check that only reports, with `how` for the Update button. Does nothing
    /// when the app's own updater is (or will be) registered.
    pub fn register_check(&self, updater: Arc<dyn Updater>, how: How) -> bool {
        if self.app_updates() {
            return false;
        }
        *self.updater.lock().unwrap() = Some(updater);
        self.status.send_modify(|s| {
            s.supported = true;
            s.can_install = false;
            s.how = Some(how);
        });
        true
    }

    pub fn status(&self) -> Status {
        self.status.borrow().clone()
    }

    /// Follows every change, for the "new version" notification.
    pub fn subscribe(&self) -> watch::Receiver<Status> {
        self.status.subscribe()
    }

    fn updater(&self) -> Option<Arc<dyn Updater>> {
        self.updater.lock().unwrap().clone()
    }

    /// Starts a check, unless one is running or an install is under way.
    pub fn check(&self) -> Result<Status, Refused> {
        let updater = self.updater().ok_or(Refused::Unsupported)?;
        let mut refused = None;
        self.status.send_if_modified(|s| match s.state {
            State::Checking | State::Downloading | State::Ready => {
                refused = Some(Refused::Busy);
                false
            }
            _ => {
                s.state = State::Checking;
                s.error = None;
                true
            }
        });
        if let Some(r) = refused {
            return Err(r);
        }
        updater.check();
        Ok(self.status())
    }

    /// Starts installing the version found, also after a failed attempt.
    pub fn install(&self) -> Result<Status, Refused> {
        let updater = self.updater().ok_or(Refused::Unsupported)?;
        if !self.status.borrow().can_install {
            return Err(Refused::CannotInstall);
        }
        let mut result = Err(Refused::NothingToInstall);
        self.status.send_if_modified(|s| match (s.state, &s.available) {
            (State::Checking | State::Downloading | State::Ready, _) => {
                result = Err(Refused::Busy);
                false
            }
            (_, Some(a)) => {
                result = Ok(a.version.clone());
                s.state = State::Downloading;
                s.progress = Some(0.0);
                s.error = None;
                true
            }
            (_, None) => false,
        });
        let version = result?;
        updater.install(&version);
        Ok(self.status())
    }

    /// The check's answer. A release that isn't newer, or is a prerelease, counts as none.
    pub fn found(&self, release: Option<Available>) {
        self.status.send_modify(|s| {
            let release = release.filter(|r| is_newer(&r.version, &s.current));
            s.state = if release.is_some() { State::Available } else { State::Idle };
            s.available = release;
            s.progress = None;
            s.error = None;
            s.checked_at = Some(now_rfc3339());
        });
    }

    /// Download progress, 0–1 (`None`: size unknown). Called per chunk; followers hear of it in
    /// steps of 1%.
    pub fn progress(&self, fraction: Option<f64>) {
        let next = fraction.map(|f| f.clamp(0.0, 1.0));
        self.status.send_if_modified(|s| {
            let moved = match (s.progress, next) {
                (Some(was), Some(now)) => (now - was).abs() >= 0.01 || (now == 1.0 && was < 1.0),
                (was, now) => was != now,
            };
            if s.state != State::Downloading || !moved {
                return false;
            }
            s.progress = next;
            true
        });
    }

    /// Installed; the app relaunches next.
    pub fn ready(&self) {
        self.status.send_modify(|s| {
            s.state = State::Ready;
            s.progress = Some(1.0);
            s.error = None;
        });
    }

    /// A check or install failed. A version already found stays, so the person can retry or
    /// download it by hand.
    pub fn failed(&self, error: &str) {
        let error: String = error.chars().take(300).collect();
        self.status.send_modify(|s| {
            if s.state == State::Checking {
                s.checked_at = Some(now_rfc3339());
            }
            s.state = State::Error;
            s.progress = None;
            s.error = Some(error);
        });
    }
}

fn now_rfc3339() -> String {
    chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
}

// ---------- off the app: the release check ----------

/// How this install updates, from where and how it runs.
pub fn how_here() -> How {
    let exe = std::env::current_exe().ok();
    let home = std::env::var_os("HOME").map(std::path::PathBuf::from);
    let unit = std::fs::read_to_string("/proc/self/cgroup")
        .ok()
        .and_then(|c| Unit::from_cgroup(&c));
    how_for(
        crate::doctor::Platform::detect(),
        std::env::consts::OS,
        std::env::consts::ARCH,
        exe.as_deref(),
        home.as_deref(),
        unit,
    )
}

/// The systemd unit kwnt runs as, when it is `kwnt.service`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Unit {
    /// `systemctl --user` (what install.sh sets up).
    User,
    System,
}

impl Unit {
    /// From `/proc/self/cgroup`: `…/user@1000.service/app.slice/kwnt.service` or `/system.slice/kwnt.service`.
    pub fn from_cgroup(cgroup: &str) -> Option<Unit> {
        cgroup.lines().find_map(|l| {
            let path = l.rsplit(':').next()?;
            if !path.split('/').any(|part| part == "kwnt.service") {
                return None;
            }
            Some(if path.split('/').any(|part| part.starts_with("user@")) {
                Unit::User
            } else {
                Unit::System
            })
        })
    }
}

/// The one line kwant's own page gives for installing on Linux; run again, it upgrades in place.
pub const INSTALL_SH: &str = "curl -fsSL https://kwant.si/install.sh | sh";

/// [`how_here`] from its parts: `home` is `$HOME`, `unit` the systemd unit kwnt runs as, if any.
pub fn how_for(
    platform: crate::doctor::Platform,
    os: &str,
    arch: &str,
    exe: Option<&std::path::Path>,
    home: Option<&std::path::Path>,
    unit: Option<Unit>,
) -> How {
    use crate::doctor::Platform;
    if platform == Platform::Docker {
        return How {
            kind: "docker",
            command: Some("docker compose pull kwnt && docker compose up -d".into()),
        };
    }
    if os == "windows" {
        return How {
            kind: "windows",
            command: None,
        };
    }
    let built_here = exe.is_none_or(|p| p.components().any(|c| c.as_os_str() == "target"));
    let arch = match arch {
        "x86_64" => Some("x86_64"),
        "aarch64" => Some("aarch64"),
        _ => None,
    };
    match (os, arch, exe.and_then(|p| p.parent())) {
        ("linux", Some(arch), Some(dir)) if !built_here => {
            // install.sh's own place: running it again downloads, checks and restarts (never sudo).
            if home.is_some_and(|h| dir == h.join(".local/bin")) {
                return How {
                    kind: "binary",
                    command: Some(INSTALL_SH.into()),
                };
            }
            let sudo = if home.is_some_and(|h| dir.starts_with(h)) {
                ""
            } else {
                "sudo "
            };
            let get = format!("curl -fsSL {RELEASES}/latest/download/kwnt-linux-{arch}.tar.gz | {sudo}tar -xz -C {} kwnt", dir.display());
            let restart = match unit {
                Some(Unit::User) => "systemctl --user restart kwnt",
                Some(Unit::System) => "sudo systemctl restart kwnt",
                None => "# then stop kwnt serve and start it again",
            };
            How {
                kind: "binary",
                command: Some(format!("{get}\n{restart}")),
            }
        }
        _ => How {
            kind: "source",
            command: Some("git pull && make build".into()),
        },
    }
}

/// The latest release, from [`LATEST_JSON`]. `Err` with the person's words when it can't tell.
pub async fn fetch_latest(client: &reqwest::Client, url: &str) -> Result<Available, String> {
    let res = client
        .get(url)
        .timeout(CHECK_TIMEOUT)
        .send()
        .await
        .map_err(|e| {
            if e.is_timeout() {
                "GitHub didn't answer in time.".to_string()
            } else {
                "GitHub couldn't be reached.".to_string()
            }
        })?;
    if !res.status().is_success() {
        return Err(format!("GitHub answered {}.", res.status().as_u16()));
    }
    let v: serde_json::Value = res
        .json()
        .await
        .map_err(|_| "GitHub's answer wasn't readable.".to_string())?;
    parse_latest(&v).ok_or_else(|| "GitHub's answer had no version.".to_string())
}

/// `latest.json` → the release, or `None` when it has no `x.y.z` version.
pub fn parse_latest(v: &serde_json::Value) -> Option<Available> {
    let version = v["version"]
        .as_str()?
        .trim()
        .trim_start_matches('v')
        .to_string();
    release_parts(&version)?;
    Some(Available {
        notes: v["notes"].as_str().unwrap_or_default().to_string(),
        date: v["pub_date"].as_str().map(str::to_string),
        url: release_page(&version),
        version,
    })
}

/// After a failed look, the next one comes sooner.
pub const RETRY_AFTER_ERROR: Duration = Duration::from_secs(30 * 60);

/// Whether the release check should look again now.
pub fn check_due(s: &Status, now: chrono::DateTime<chrono::Utc>) -> bool {
    let Some(at) = s
        .checked_at
        .as_deref()
        .and_then(|t| chrono::DateTime::parse_from_rfc3339(t).ok())
    else {
        return true;
    };
    let every = if s.state == State::Error && s.available.is_none() {
        RETRY_AFTER_ERROR
    } else {
        RECHECK_EVERY
    };
    now.signed_duration_since(at).to_std().unwrap_or_default() >= every
}

/// The server's updater off the app: looks, never installs.
struct ReleaseCheck {
    hub: Arc<UpdateHub>,
    client: reqwest::Client,
    url: String,
}

impl Updater for ReleaseCheck {
    fn check(&self) {
        let (hub, client, url) = (self.hub.clone(), self.client.clone(), self.url.clone());
        let work = async move {
            match fetch_latest(&client, &url).await {
                Ok(release) => hub.found(Some(release)),
                Err(e) => {
                    tracing::info!(error = %e, "update check failed");
                    hub.failed(&format!("Couldn't check for updates: {e}"));
                }
            }
        };
        match tokio::runtime::Handle::try_current() {
            Ok(rt) => drop(rt.spawn(work)),
            Err(_) => self.hub.failed("Couldn't check for updates: no runtime."),
        }
    }

    fn install(&self, _version: &str) {
        self.hub.failed(Refused::CannotInstall.message());
    }
}

/// Starts the release check on `hub` when the app's own updater isn't there: one look a few
/// seconds after start, then every [`RECHECK_EVERY`] (wall clock, so a sleeping laptop looks
/// again when it wakes). Stops if the app's updater takes over. `KWNT_UPDATE_CHECK=off` skips it.
pub fn start_release_check(hub: Arc<UpdateHub>) {
    if std::env::var("KWNT_UPDATE_CHECK")
        .is_ok_and(|v| matches!(v.trim(), "off" | "0" | "false" | "no"))
    {
        tracing::info!("update check off (KWNT_UPDATE_CHECK)");
        return;
    }
    let client = reqwest::Client::builder()
        .user_agent(concat!("kwnt/", env!("CARGO_PKG_VERSION")))
        .build()
        .unwrap_or_default();
    // A test feed (a local latest.json) for trying the states; GitHub's otherwise
    let url = std::env::var("KWNT_UPDATE_FEED")
        .ok()
        .filter(|u| {
            u.starts_with("https://")
                || u.starts_with("http://127.0.0.1")
                || u.starts_with("http://localhost")
        })
        .unwrap_or_else(|| LATEST_JSON.to_string());
    let check = Arc::new(ReleaseCheck {
        hub: hub.clone(),
        client,
        url,
    });
    if !hub.register_check(check, how_here()) {
        return;
    }
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_secs(5)).await;
        loop {
            if hub.status().can_install {
                return;
            }
            if check_due(&hub.status(), chrono::Utc::now()) {
                // Busy means a check is already running
                let _ = hub.check();
            }
            tokio::time::sleep(Duration::from_secs(5 * 60)).await;
        }
    });
}

/// `x.y.z` as numbers; `None` for anything else, including prereleases (`0.2.0-rc.1`).
fn release_parts(version: &str) -> Option<[u64; 3]> {
    let mut parts = version.trim().trim_start_matches('v').split('.');
    let mut out = [0u64; 3];
    for slot in &mut out {
        *slot = parts.next()?.parse().ok()?;
    }
    parts.next().is_none().then_some(out)
}

/// Whether `candidate` is a release (not a prerelease) newer than `current`.
pub fn is_newer(candidate: &str, current: &str) -> bool {
    let Some(candidate) = release_parts(candidate) else {
        return false;
    };
    // A running prerelease (0.2.0-rc.1) is older than its release 0.2.0
    let current = current.trim().trim_start_matches('v');
    match release_parts(current) {
        Some(current) => candidate > current,
        None => release_parts(current.split('-').next().unwrap_or("")).is_some_and(|base| candidate >= base),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Default)]
    struct Fake {
        calls: Mutex<Vec<String>>,
    }

    impl Updater for Fake {
        fn check(&self) {
            self.calls.lock().unwrap().push("check".into());
        }
        fn install(&self, version: &str) {
            self.calls.lock().unwrap().push(format!("install {version}"));
        }
    }

    fn release(version: &str) -> Available {
        Available {
            version: version.into(),
            notes: "### Added\n- things".into(),
            date: Some("2026-10-27T10:00:00Z".into()),
            url: format!("https://github.com/alexsergivan/kwant/releases/tag/mac-v{version}"),
        }
    }

    fn hub() -> (UpdateHub, Arc<Fake>) {
        let hub = UpdateHub::new();
        let fake = Arc::new(Fake::default());
        hub.register("0.1.1", fake.clone());
        (hub, fake)
    }

    #[test]
    fn versions() {
        assert!(is_newer("0.1.2", "0.1.1"));
        assert!(is_newer("0.2.0", "0.1.9"));
        assert!(is_newer("1.0.0", "0.10.10"));
        assert!(is_newer("0.10.0", "0.9.0"), "numeric, not string, order");
        assert!(is_newer("v0.1.2", "0.1.1"));
        assert!(!is_newer("0.1.1", "0.1.1"));
        assert!(!is_newer("0.1.0", "0.1.1"));
        assert!(!is_newer("0.2.0-rc.1", "0.1.1"), "prereleases are never offered");
        assert!(!is_newer("0.2", "0.1.1"));
        assert!(!is_newer("0.2.0.1", "0.1.1"));
        assert!(!is_newer("latest", "0.1.1"));
        assert!(is_newer("0.2.0", "0.2.0-rc.1"), "a release replaces its own rc");
        assert!(!is_newer("0.1.9", "0.2.0-rc.1"));
    }

    #[test]
    fn unsupported_until_registered() {
        let hub = UpdateHub::new();
        let s = hub.status();
        assert!(!s.supported);
        assert_eq!(s.current, env!("CARGO_PKG_VERSION"));
        assert_eq!(s.state, State::Idle);
        assert_eq!(hub.check(), Err(Refused::Unsupported));
        assert_eq!(hub.install(), Err(Refused::Unsupported));
    }

    #[test]
    fn check_finds_a_newer_version_then_installs() {
        let (hub, fake) = hub();
        assert!(hub.status().supported);
        assert_eq!(hub.status().current, "0.1.1");
        assert_eq!(hub.install(), Err(Refused::NothingToInstall));

        assert_eq!(hub.check().unwrap().state, State::Checking);
        assert_eq!(hub.check(), Err(Refused::Busy), "one check at a time");
        hub.found(Some(release("0.1.2")));
        let s = hub.status();
        assert_eq!(s.state, State::Available);
        assert_eq!(s.available.as_ref().unwrap().version, "0.1.2");

        let s = hub.install().unwrap();
        assert_eq!(s.state, State::Downloading);
        assert_eq!(s.progress, Some(0.0));
        assert_eq!(hub.install(), Err(Refused::Busy));
        assert_eq!(hub.check(), Err(Refused::Busy), "no check while downloading");
        hub.progress(Some(0.5));
        assert_eq!(hub.status().progress, Some(0.5));
        let mut rx = hub.subscribe();
        rx.borrow_and_update();
        hub.progress(Some(0.503));
        assert!(!rx.has_changed().unwrap(), "under 1%: no wake-up");
        assert_eq!(hub.status().progress, Some(0.5));
        hub.progress(Some(7.0));
        assert_eq!(hub.status().progress, Some(1.0), "clamped");
        hub.ready();
        assert_eq!(hub.status().state, State::Ready);
        assert_eq!(hub.check(), Err(Refused::Busy), "relaunching");

        assert_eq!(*fake.calls.lock().unwrap(), vec!["check", "install 0.1.2"]);
    }

    #[test]
    fn older_same_or_prerelease_counts_as_none() {
        let (hub, _) = hub();
        for v in ["0.1.1", "0.1.0", "0.2.0-rc.1"] {
            hub.check().unwrap();
            hub.found(Some(release(v)));
            let s = hub.status();
            assert_eq!(s.state, State::Idle, "{v}");
            assert_eq!(s.available, None, "{v}");
        }
        hub.check().unwrap();
        hub.found(None);
        assert_eq!(hub.status().state, State::Idle);
    }

    #[test]
    fn failed_install_keeps_the_version_and_can_retry() {
        let (hub, fake) = hub();
        hub.check().unwrap();
        hub.found(Some(release("0.2.0")));
        hub.install().unwrap();
        hub.failed("signature didn't match");
        let s = hub.status();
        assert_eq!(s.state, State::Error);
        assert_eq!(s.error.as_deref(), Some("signature didn't match"));
        assert_eq!(s.available.as_ref().unwrap().version, "0.2.0", "manual download link stays");
        assert_eq!(s.progress, None);
        hub.progress(Some(0.3));
        assert_eq!(hub.status().progress, None, "progress only while downloading");

        assert_eq!(hub.install().unwrap().state, State::Downloading);
        assert_eq!(hub.status().error, None);
        assert_eq!(fake.calls.lock().unwrap().len(), 3);
    }

    #[test]
    fn failed_check_without_a_version() {
        let (hub, _) = hub();
        hub.check().unwrap();
        hub.failed(&"x".repeat(1000));
        let s = hub.status();
        assert_eq!(s.state, State::Error);
        assert_eq!(s.error.unwrap().len(), 300, "clipped");
        assert_eq!(hub.install(), Err(Refused::NothingToInstall));
        assert_eq!(hub.check().unwrap().state, State::Checking, "a new check clears the error");
        assert_eq!(hub.status().error, None);
    }

    struct Looks;
    impl Updater for Looks {
        fn check(&self) {}
        fn install(&self, _: &str) {}
    }

    fn how() -> How {
        How {
            kind: "docker",
            command: Some("docker compose pull kwnt && docker compose up -d".into()),
        }
    }

    #[test]
    fn release_check_reports_but_never_installs() {
        let hub = UpdateHub::new();
        assert!(hub.register_check(Arc::new(Looks), how()));
        let s = hub.status();
        assert!(s.supported);
        assert!(!s.can_install);
        assert_eq!(s.how.as_ref().unwrap().kind, "docker");
        assert_eq!(s.current, env!("CARGO_PKG_VERSION"));
        assert_eq!(s.checked_at, None);
        hub.check().unwrap();
        hub.found(Some(release("99.0.0")));
        let s = hub.status();
        assert_eq!(s.state, State::Available);
        assert!(s.checked_at.is_some(), "a finished check says when");
        assert_eq!(hub.install(), Err(Refused::CannotInstall));
        assert_eq!(
            hub.status().state,
            State::Available,
            "a refused install changes nothing"
        );
    }

    #[test]
    fn the_app_updater_wins_over_the_release_check() {
        // The app registers after the server started its check: it takes over
        let hub = UpdateHub::new();
        assert!(hub.register_check(Arc::new(Looks), how()));
        hub.register("0.1.1", Arc::new(Fake::default()));
        let s = hub.status();
        assert!(s.can_install);
        assert_eq!(s.how, None);
        // The app said it's coming: the server doesn't start a check
        let hub = UpdateHub::new();
        hub.mark_desktop_app();
        assert!(!hub.register_check(Arc::new(Looks), how()));
        assert!(!hub.status().supported);
        // Already registered: same
        let (hub, _) = hub_pair();
        assert!(!hub.register_check(Arc::new(Looks), how()));
        assert!(hub.status().can_install);
    }

    fn hub_pair() -> (UpdateHub, Arc<Fake>) {
        hub()
    }

    #[test]
    fn a_failed_check_never_says_latest() {
        let hub = UpdateHub::new();
        hub.register_check(Arc::new(Looks), how());
        hub.check().unwrap();
        hub.failed("Couldn't check for updates: GitHub couldn't be reached.");
        let s = hub.status();
        assert_eq!(s.state, State::Error);
        assert_eq!(s.available, None);
        assert!(s.checked_at.is_some());
        // a failed install doesn't move checked_at
        let (hub, _) = hub_pair();
        hub.check().unwrap();
        hub.found(Some(release("0.2.0")));
        let at = hub.status().checked_at;
        hub.install().unwrap();
        hub.failed("download failed");
        assert_eq!(hub.status().checked_at, at);
    }

    #[test]
    fn rechecks_every_6_hours_and_30_minutes_after_a_failure() {
        let hub = UpdateHub::new();
        hub.register_check(Arc::new(Looks), how());
        let now = chrono::Utc::now();
        assert!(check_due(&hub.status(), now), "never checked");
        hub.check().unwrap();
        hub.found(None);
        let s = hub.status();
        let at: chrono::DateTime<chrono::Utc> =
            chrono::DateTime::parse_from_rfc3339(s.checked_at.as_deref().unwrap())
                .unwrap()
                .into();
        assert!(!check_due(&s, at + chrono::Duration::hours(5)));
        assert!(check_due(&s, at + chrono::Duration::hours(6)));
        hub.check().unwrap();
        hub.failed("offline");
        let s = hub.status();
        let at: chrono::DateTime<chrono::Utc> =
            chrono::DateTime::parse_from_rfc3339(s.checked_at.as_deref().unwrap())
                .unwrap()
                .into();
        assert!(!check_due(&s, at + chrono::Duration::minutes(29)));
        assert!(check_due(&s, at + chrono::Duration::minutes(30)));
    }

    #[test]
    fn latest_json_parses() {
        let v = serde_json::json!({"version": "0.9.4", "notes": "kwant 0.9.4: things", "pub_date": "2026-11-12T20:49:18Z", "platforms": {}});
        let a = parse_latest(&v).unwrap();
        assert_eq!(a.version, "0.9.4");
        assert_eq!(a.notes, "kwant 0.9.4: things");
        assert_eq!(a.date.as_deref(), Some("2026-11-12T20:49:18Z"));
        assert_eq!(
            a.url,
            "https://github.com/alexsergivan/kwant/releases/tag/mac-v0.9.4"
        );
        assert_eq!(
            parse_latest(&serde_json::json!({"version": "v1.0.0"}))
                .unwrap()
                .version,
            "1.0.0"
        );
        assert_eq!(
            parse_latest(&serde_json::json!({"version": "latest"})),
            None
        );
        assert_eq!(parse_latest(&serde_json::json!({})), None);
    }

    #[tokio::test]
    async fn fetch_latest_says_why_it_couldnt() {
        let client = reqwest::Client::new();
        // nothing listens on port 9 here
        let e = fetch_latest(&client, "http://127.0.0.1:9/latest.json")
            .await
            .unwrap_err();
        assert_eq!(e, "GitHub couldn't be reached.");
    }

    #[test]
    fn how_each_install_updates() {
        use crate::doctor::Platform;
        use std::path::Path;
        let home = Some(Path::new("/home/me"));
        let docker = how_for(
            Platform::Docker,
            "linux",
            "x86_64",
            Some(Path::new("/usr/local/bin/kwnt")),
            home,
            None,
        );
        assert_eq!(docker.kind, "docker");
        assert_eq!(
            docker.command.as_deref(),
            Some("docker compose pull kwnt && docker compose up -d")
        );
        // install.sh's place: run it again (it upgrades in place, restarts its user unit, never sudo)
        let installed = how_for(
            Platform::Linux,
            "linux",
            "x86_64",
            Some(Path::new("/home/me/.local/bin/kwnt")),
            home,
            Some(Unit::User),
        );
        assert_eq!(
            installed,
            How {
                kind: "binary",
                command: Some(INSTALL_SH.into())
            }
        );
        let bin = how_for(
            Platform::Linux,
            "linux",
            "x86_64",
            Some(Path::new("/usr/local/bin/kwnt")),
            home,
            Some(Unit::System),
        );
        assert_eq!(bin.kind, "binary");
        assert_eq!(
            bin.command.as_deref(),
            Some("curl -fsSL https://github.com/alexsergivan/kwant/releases/latest/download/kwnt-linux-x86_64.tar.gz | sudo tar -xz -C /usr/local/bin kwnt\nsudo systemctl restart kwnt")
        );
        let arm = how_for(
            Platform::Linux,
            "linux",
            "aarch64",
            Some(Path::new("/opt/kwnt/kwnt")),
            home,
            None,
        );
        assert!(arm.command.as_deref().unwrap().contains(
            "kwnt-linux-aarch64.tar.gz | sudo tar -xz -C /opt/kwnt kwnt\n# then stop kwnt serve"
        ));
        // under the home folder: no sudo
        let mine = how_for(
            Platform::Linux,
            "linux",
            "aarch64",
            Some(Path::new("/home/me/apps/kwnt")),
            home,
            Some(Unit::User),
        );
        assert!(mine
            .command
            .as_deref()
            .unwrap()
            .contains("| tar -xz -C /home/me/apps kwnt\nsystemctl --user restart kwnt"));
        let src = how_for(
            Platform::Linux,
            "linux",
            "x86_64",
            Some(Path::new("/home/me/kwant/target/release/kwnt")),
            home,
            None,
        );
        assert_eq!(src.kind, "source");
        assert_eq!(
            how_for(
                Platform::Mac,
                "macos",
                "aarch64",
                Some(Path::new("/usr/local/bin/kwnt")),
                home,
                None
            )
            .kind,
            "source"
        );
        assert_eq!(
            how_for(Platform::Linux, "windows", "x86_64", None, home, None),
            How {
                kind: "windows",
                command: None
            }
        );
    }

    #[test]
    fn systemd_unit_from_cgroup() {
        assert_eq!(
            Unit::from_cgroup(
                "0::/user.slice/user-1000.slice/user@1000.service/app.slice/kwnt.service\n"
            ),
            Some(Unit::User)
        );
        assert_eq!(
            Unit::from_cgroup("0::/system.slice/kwnt.service\n"),
            Some(Unit::System)
        );
        // another service's child (a shell in a bot host) is not kwnt's unit
        assert_eq!(
            Unit::from_cgroup("0::/system.slice/openmausbot.service\n"),
            None
        );
        assert_eq!(
            Unit::from_cgroup("0::/user.slice/user-1000.slice/session-3.scope\n"),
            None
        );
        assert_eq!(Unit::from_cgroup(""), None);
    }

    #[test]
    fn status_json_shape() {
        let (hub, _) = hub();
        hub.check().unwrap();
        hub.found(Some(release("0.1.2")));
        let v = serde_json::to_value(hub.status()).unwrap();
        assert_eq!(v["supported"], true);
        assert_eq!(v["current"], "0.1.1");
        assert_eq!(v["state"], "available");
        assert_eq!(v["available"]["version"], "0.1.2");
        assert_eq!(v["available"]["date"], "2026-10-27T10:00:00Z");
        assert!(v["available"]["url"].as_str().unwrap().ends_with("/mac-v0.1.2"));
        assert!(v["progress"].is_null());
        assert!(v["error"].is_null());
        assert_eq!(v["can_install"], true);
        assert!(v["checked_at"].as_str().unwrap().ends_with('Z'));
        assert!(v["how"].is_null());
    }
}
