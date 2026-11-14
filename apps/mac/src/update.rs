//! In-app updates. The app asks GitHub's latest Mac release (`latest.json`, see `plugins.updater`
//! in tauri.conf.json) on start and every hour, by the wall clock (a Mac asleep overnight checks
//! when it wakes). The state lives on kwant's
//! [`UpdateHub`](kwnt::app_update::UpdateHub), so the web app shows it through `/api/app/update`
//! and the person starts the install there or from the menu-bar icon.
//!
//! Installing downloads the signed `kwant.app.tar.gz`, checks its signature against the public key
//! built into this app, swaps the app bundle, stops the server (so nothing is half-written) and
//! relaunches. Any failure before the swap leaves this app running as it was.

use kwnt::app_update::{Available, State, Status, UpdateHub, Updater};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime};
use tauri::{AppHandle, Runtime};
use tauri_plugin_updater::{Update, UpdaterExt};

const RELEASES: &str = "https://github.com/alexsergivan/kwant/releases";
/// After start, so the check doesn't compete with kwant starting up.
const FIRST_CHECK_AFTER: Duration = Duration::from_secs(15);
const CHECK_EVERY: Duration = Duration::from_secs(60 * 60);
/// How often the loop looks at the clock. A thread's sleep stops while the Mac sleeps, so a
/// plain `sleep(CHECK_EVERY)` could put the next check off by days; this keeps it close to due.
const LOOK_EVERY: Duration = Duration::from_secs(5 * 60);
/// Long enough for the web app to show "Restarting…" before the window goes away.
const RELAUNCH_AFTER: Duration = Duration::from_millis(1500);

/// The GitHub release page of `version`: "What's new" and "Download manually".
pub fn release_page(version: &str) -> String {
    format!("{RELEASES}/tag/mac-v{version}")
}

/// The menu-bar item for an update: its text and whether it can be clicked. `None`: no item.
pub fn menu_item(status: &Status) -> Option<(String, bool)> {
    let version = &status.available.as_ref()?.version;
    match status.state {
        State::Available | State::Error => Some((format!("Update to {version}…"), true)),
        State::Downloading => Some(match status.progress {
            Some(p) if p > 0.0 => (format!("Downloading kwant bots {version}… {}%", (p * 100.0).round() as u32), false),
            _ => (format!("Downloading kwant bots {version}…"), false),
        }),
        State::Ready => Some(("Restarting kwant bots…".to_string(), false)),
        State::Idle | State::Checking => None,
    }
}

/// Registers the updater on kwant's hub and starts the checks. Only when this app runs the
/// server: an attached kwant serves its own `/api/app/update`. The registration is done when
/// this returns (the checks run later, on their own thread), so a window that asks after it
/// hears `supported: true`. Call it once: `crate::start_updates` makes sure of that.
pub fn start<R: Runtime>(app: &AppHandle<R>) {
    let hub = UpdateHub::shared();
    register(app, hub.clone());
    std::thread::Builder::new()
        .name("kwant-update-check".into())
        .spawn(move || {
            std::thread::sleep(FIRST_CHECK_AFTER);
            let mut last: Option<SystemTime> = None;
            loop {
                if check_due(last, SystemTime::now()) {
                    // Busy (downloading, relaunching) means there's nothing to check
                    let _ = hub.check();
                    last = Some(SystemTime::now());
                }
                std::thread::sleep(LOOK_EVERY);
            }
        })
        .ok();
}

/// Puts this app's updater on `hub`: from now on it answers `supported: true` and can install.
fn register<R: Runtime>(app: &AppHandle<R>, hub: Arc<UpdateHub>) {
    let current = app.package_info().version.to_string();
    let updater = Arc::new(MacUpdater {
        app: app.clone(),
        hub: hub.clone(),
        found: Arc::new(Mutex::new(None)),
    });
    hub.register(&current, updater);
    tracing::info!(current, "in-app updates on");
}

/// Due on the first look, then once `CHECK_EVERY` has passed on the wall clock (or the clock
/// went back).
fn check_due(last: Option<SystemTime>, now: SystemTime) -> bool {
    match last {
        None => true,
        Some(last) => now
            .duration_since(last)
            .map_or(true, |gone| gone >= CHECK_EVERY),
    }
}

struct MacUpdater<R: Runtime> {
    app: AppHandle<R>,
    hub: Arc<UpdateHub>,
    /// The last check's release, which install downloads.
    found: Arc<Mutex<Option<Update>>>,
}

impl<R: Runtime> Updater for MacUpdater<R> {
    fn check(&self) {
        let (app, hub, found) = (self.app.clone(), self.hub.clone(), self.found.clone());
        tauri::async_runtime::spawn(async move {
            let result = match app.updater() {
                Ok(updater) => updater.check().await,
                Err(e) => Err(e),
            };
            match result {
                Ok(Some(update)) => {
                    let release = Available {
                        version: update.version.clone(),
                        notes: update.body.clone().unwrap_or_default(),
                        date: update.raw_json["pub_date"].as_str().map(str::to_string),
                        url: release_page(&update.version),
                    };
                    tracing::info!(version = %update.version, "kwant update available");
                    *found.lock().unwrap() = Some(update);
                    hub.found(Some(release));
                }
                Ok(None) => {
                    *found.lock().unwrap() = None;
                    hub.found(None);
                }
                Err(e) => {
                    tracing::warn!(error = %e, "update check failed");
                    hub.failed(&format!("Couldn't check for updates: {e}"));
                }
            }
        });
    }

    fn install(&self, version: &str) {
        let update = self.found.lock().unwrap().clone().filter(|u| u.version == version);
        let Some(update) = update else {
            self.hub.failed("That update is no longer on offer. Check for updates again.");
            return;
        };
        let (app, hub) = (self.app.clone(), self.hub.clone());
        tauri::async_runtime::spawn(async move {
            let mut received = 0u64;
            let progress_hub = hub.clone();
            // download() also checks the signature against the key built into this app
            let bytes = update
                .download(
                    |chunk, total| {
                        received += chunk as u64;
                        progress_hub.progress(total.filter(|t| *t > 0).map(|t| received as f64 / t as f64));
                    },
                    || {},
                )
                .await;
            let bytes = match bytes {
                Ok(b) => b,
                Err(e) => {
                    tracing::warn!(error = %e, version = %update.version, "update download failed");
                    return hub.failed(&format!("The download failed: {e}"));
                }
            };
            if let Err(e) = update.install(&bytes) {
                tracing::warn!(error = %e, version = %update.version, "update install failed");
                return hub.failed(&format!("Couldn't install the update: {e}"));
            }
            tracing::info!(version = %update.version, "update installed; relaunching");
            hub.ready();
            tokio::time::sleep(RELAUNCH_AFTER).await;
            crate::stop_server(&app);
            app.restart();
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn status(state: State, version: Option<&str>, progress: Option<f64>) -> Status {
        Status {
            supported: true,
            current: "0.1.1".into(),
            state,
            available: version.map(|v| Available {
                version: v.into(),
                notes: String::new(),
                date: None,
                url: release_page(v),
            }),
            progress,
            error: None,
            can_install: true,
            checked_at: None,
            how: None,
        }
    }

    #[test]
    fn checks_by_the_wall_clock() {
        let t = SystemTime::UNIX_EPOCH + Duration::from_secs(1_800_000_000);
        assert!(check_due(None, t));
        assert!(!check_due(Some(t), t + Duration::from_secs(59 * 60)));
        // a night asleep: due on the first look after waking
        assert!(check_due(Some(t), t + Duration::from_secs(9 * 60 * 60)));
        assert!(
            check_due(Some(t + Duration::from_secs(60)), t),
            "clock went back"
        );
    }

    /// The hub answers `supported: true` as soon as `register` returns, not after the first check:
    /// `ensure_local` relies on it to register before it hands a window the port.
    #[test]
    fn a_registered_updater_is_supported_at_once() {
        let app = tauri::test::mock_app();
        let hub = Arc::new(UpdateHub::new());
        hub.mark_desktop_app();
        assert!(!hub.status().supported, "a desktop app's hub is off until its updater registers");
        register(app.handle(), hub.clone());
        let s = hub.status();
        assert!(s.supported);
        assert!(s.can_install);
        assert_eq!(s.state, State::Idle, "no check has run yet");
        assert_eq!(s.current, app.package_info().version.to_string());
    }

    #[test]
    fn release_pages() {
        assert_eq!(release_page("0.1.2"), "https://github.com/alexsergivan/kwant/releases/tag/mac-v0.1.2");
    }

    #[test]
    fn menu_items() {
        assert_eq!(menu_item(&status(State::Idle, None, None)), None);
        assert_eq!(menu_item(&status(State::Checking, None, None)), None);
        assert_eq!(menu_item(&status(State::Error, None, None)), None, "a failed check offers nothing");
        assert_eq!(menu_item(&status(State::Available, Some("0.1.2"), None)), Some(("Update to 0.1.2…".into(), true)));
        assert_eq!(menu_item(&status(State::Error, Some("0.1.2"), None)), Some(("Update to 0.1.2…".into(), true)), "retry");
        assert_eq!(
            menu_item(&status(State::Downloading, Some("0.1.2"), Some(0.0))),
            Some(("Downloading kwant bots 0.1.2…".into(), false))
        );
        assert_eq!(
            menu_item(&status(State::Downloading, Some("0.1.2"), Some(0.426))),
            Some(("Downloading kwant bots 0.1.2… 43%".into(), false))
        );
        assert_eq!(menu_item(&status(State::Ready, Some("0.1.2"), Some(1.0))), Some(("Restarting kwant bots…".into(), false)));
    }
}
