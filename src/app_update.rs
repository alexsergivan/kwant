//! In-app updates for the Mac app. The Mac shell (apps/mac) links this crate, runs the server in its
//! own process and registers an [`Updater`] on the shared [`UpdateHub`]; the server's
//! `/api/app/update` routes read and drive the hub. Anywhere else (`kwnt serve`, Docker, a VPS)
//! nothing registers, so the routes report `supported: false` and the web app shows no update UI.
//!
//! The hub owns the state machine: idle → checking → available → downloading → ready, with error
//! from checking or downloading. The updater only does the work (fetch, verify, install, relaunch)
//! and reports back through [`UpdateHub::found`], [`UpdateHub::progress`], [`UpdateHub::ready`] and
//! [`UpdateHub::failed`].

use serde::Serialize;
use std::sync::{Arc, Mutex, OnceLock};
use tokio::sync::watch;

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

/// `GET /api/app/update`.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Status {
    /// Whether this process is the Mac app and can update itself.
    pub supported: bool,
    /// The running app's version (the kwnt crate's version off-Mac).
    pub current: String,
    pub state: State,
    pub available: Option<Available>,
    /// Download progress 0–1, while downloading and the size is known.
    pub progress: Option<f64>,
    pub error: Option<String>,
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
    /// Not the Mac app.
    Unsupported,
    /// Already checking, downloading or relaunching.
    Busy,
    /// Install without a newer version found.
    NothingToInstall,
}

impl Refused {
    pub fn message(&self) -> &'static str {
        match self {
            Refused::Unsupported => "updates are only available in the kwant bots Mac app",
            Refused::Busy => "an update check or install is already running",
            Refused::NothingToInstall => "no newer version is available",
        }
    }
}

pub struct UpdateHub {
    updater: Mutex<Option<Arc<dyn Updater>>>,
    status: watch::Sender<Status>,
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
            current: env!("CARGO_PKG_VERSION").to_string(),
            state: State::Idle,
            available: None,
            progress: None,
            error: None,
        });
        Self { updater: Mutex::new(None), status }
    }

    /// The hub of this process: the Mac shell registers on it, the server reads it.
    pub fn shared() -> Arc<UpdateHub> {
        static SHARED: OnceLock<Arc<UpdateHub>> = OnceLock::new();
        SHARED.get_or_init(|| Arc::new(UpdateHub::new())).clone()
    }

    /// Makes updates supported, running `current` (the app's version, not the crate's).
    pub fn register(&self, current: &str, updater: Arc<dyn Updater>) {
        *self.updater.lock().unwrap() = Some(updater);
        self.status.send_modify(|s| {
            s.supported = true;
            s.current = current.to_string();
        });
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
            s.state = State::Error;
            s.progress = None;
            s.error = Some(error);
        });
    }
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
    }
}
