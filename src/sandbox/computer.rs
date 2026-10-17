//! Where bots do computer work: the person's own computer ("local": their installed Chrome with a
//! kwant-only profile, and host shell commands that each need approval) or isolated containers
//! ("vm": the Docker browser and executor). Local is the default on macOS, vm everywhere else.
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::time::Duration;

/// Settings row holding the person's explicit choice.
pub const SETTING_KEY: &str = "computer_mode";
/// Overrides the platform default (not an explicit saved choice).
pub const MODE_ENV: &str = "KWNT_COMPUTER_MODE";
/// Path to the Chrome/Chromium binary local mode launches, instead of searching for one.
pub const CHROME_PATH_ENV: &str = "KWNT_CHROME_PATH";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ComputerMode {
    Local,
    Vm,
}

impl ComputerMode {
    pub fn parse(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "local" => Some(Self::Local),
            "vm" => Some(Self::Vm),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Local => "local",
            Self::Vm => "vm",
        }
    }
}

/// The mode used when the person has not chosen one: a valid env value, else local on macOS
/// (`os` is `std::env::consts::OS`) and vm elsewhere.
pub fn default_mode(os: &str, env: Option<&str>) -> ComputerMode {
    env.and_then(ComputerMode::parse)
        .unwrap_or(if os == "macos" {
            ComputerMode::Local
        } else {
            ComputerMode::Vm
        })
}

/// A valid saved choice wins over the env/platform default.
pub fn effective_mode(saved: Option<&str>, os: &str, env: Option<&str>) -> ComputerMode {
    saved
        .and_then(ComputerMode::parse)
        .unwrap_or_else(|| default_mode(os, env))
}

/// [`effective_mode`] for this process's OS and environment.
pub fn current_mode(saved: Option<&str>) -> ComputerMode {
    effective_mode(
        saved,
        std::env::consts::OS,
        std::env::var(MODE_ENV).ok().as_deref(),
    )
}

const MAC_CHROME_APPS: [&str; 3] = [
    "Google Chrome.app/Contents/MacOS/Google Chrome",
    "Chromium.app/Contents/MacOS/Chromium",
    "Google Chrome Canary.app/Contents/MacOS/Google Chrome Canary",
];
const CHROME_COMMANDS: [&str; 5] = [
    "google-chrome",
    "google-chrome-stable",
    "chromium",
    "chromium-browser",
    "chrome",
];

/// Finds the browser local mode launches: the override if it names a file, else the macOS app
/// bundles (in `/Applications` and `~/Applications`), else a Chrome/Chromium command on `path`.
pub fn find_chrome(
    override_path: Option<&str>,
    os: &str,
    home: Option<&Path>,
    path: Option<&str>,
) -> Option<PathBuf> {
    if let Some(p) = override_path.filter(|p| !p.trim().is_empty()) {
        return Path::new(p).is_file().then(|| PathBuf::from(p));
    }
    if os == "macos" {
        let roots = [
            Some(PathBuf::from("/Applications")),
            home.map(|h| h.join("Applications")),
        ];
        for root in roots.iter().flatten() {
            for app in MAC_CHROME_APPS {
                let candidate = root.join(app);
                if candidate.is_file() {
                    return Some(candidate);
                }
            }
        }
    }
    let path = path?;
    std::env::split_paths(path).find_map(|dir| {
        CHROME_COMMANDS
            .iter()
            .map(|name| dir.join(name))
            .find(|candidate| candidate.is_file())
    })
}

/// Whether a window can be shown: always on macOS and Windows, on Linux only with an X11 or
/// Wayland display.
pub fn has_display(os: &str, display: Option<&str>, wayland: Option<&str>) -> bool {
    match os {
        "linux" | "freebsd" | "openbsd" | "netbsd" => {
            display.is_some_and(|d| !d.is_empty()) || wayland.is_some_and(|d| !d.is_empty())
        }
        _ => true,
    }
}

/// Why local mode cannot run here, or `None` if it can.
pub fn local_unavailable_reason(chrome: Option<&Path>, display: bool) -> Option<String> {
    if chrome.is_none() {
        return Some(format!(
            "No Chrome or Chromium was found. Install Google Chrome, or set {CHROME_PATH_ENV} to its path."
        ));
    }
    if !display {
        return Some(
            "This computer has no screen to show a browser window on (no display).".into(),
        );
    }
    None
}

/// Why vm mode cannot run here, or `None` if it can. A configured browser service URL is enough
/// for the browser even without Docker.
pub fn vm_unavailable_reason(docker: bool, browser_url_configured: bool) -> Option<String> {
    (!docker && !browser_url_configured).then(|| {
        "Docker is not installed or not running. Install and start Docker (Docker Desktop on a Mac) to use the VM."
            .into()
    })
}

/// The local Chrome this process would launch, from the real environment.
pub fn local_chrome_path() -> Option<PathBuf> {
    let home = std::env::var_os("HOME").map(PathBuf::from);
    find_chrome(
        std::env::var(CHROME_PATH_ENV).ok().as_deref(),
        std::env::consts::OS,
        home.as_deref(),
        std::env::var("PATH").ok().as_deref(),
    )
}

/// [`local_unavailable_reason`] for this process's environment.
pub fn local_reason() -> Option<String> {
    let os = std::env::consts::OS;
    let display = has_display(
        os,
        std::env::var("DISPLAY").ok().as_deref(),
        std::env::var("WAYLAND_DISPLAY").ok().as_deref(),
    );
    local_unavailable_reason(local_chrome_path().as_deref(), display)
}

/// Reads the debugging port Chrome writes to `DevToolsActivePort` in its profile folder.
pub fn read_devtools_port(profile: &Path) -> Option<u16> {
    let text = std::fs::read_to_string(profile.join("DevToolsActivePort")).ok()?;
    text.lines().next()?.trim().parse().ok().filter(|&p| p > 0)
}

/// Chrome's command line for local mode: a headed window with its own profile, never the
/// person's everyday one, and a debugging port Chrome picks itself (it listens on 127.0.0.1).
pub fn chrome_args(profile: &Path) -> Vec<String> {
    vec![
        format!("--user-data-dir={}", profile.display()),
        "--remote-debugging-port=0".into(),
        "--remote-debugging-address=127.0.0.1".into(),
        "--no-first-run".into(),
        "--no-default-browser-check".into(),
        "about:blank".into(),
    ]
}

async fn answers(endpoint: &str) -> bool {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(2))
        .build()
        .unwrap_or_default();
    client
        .get(format!("{endpoint}/json/version"))
        .send()
        .await
        .is_ok_and(|r| r.status().is_success())
}

/// Returns the CDP endpoint of the kwant Chrome window, reusing it while it runs and launching
/// it (detached, so it outlives a switch or a kwant restart) when it does not.
pub async fn ensure_local_chrome(profile: &Path) -> Result<String, String> {
    if let Some(port) = read_devtools_port(profile) {
        let endpoint = format!("http://127.0.0.1:{port}");
        if answers(&endpoint).await {
            return Ok(endpoint);
        }
    }
    let chrome = local_chrome_path().ok_or_else(|| local_reason().unwrap_or_default())?;
    std::fs::create_dir_all(profile).map_err(|e| e.to_string())?;
    // a stale port file from a closed window would point at nothing
    let _ = std::fs::remove_file(profile.join("DevToolsActivePort"));
    let mut cmd = std::process::Command::new(&chrome);
    cmd.args(chrome_args(profile))
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    #[cfg(unix)]
    {
        // its own process group, so Ctrl+C on kwnt does not close the person's window
        use std::os::unix::process::CommandExt;
        cmd.process_group(0);
    }
    let mut child = cmd
        .spawn()
        .map_err(|e| format!("Could not start {}: {e}", chrome.display()))?;
    // reap it when it exits; never kill it
    std::thread::spawn(move || {
        let _ = child.wait();
    });
    for _ in 0..80 {
        if let Some(port) = read_devtools_port(profile) {
            let endpoint = format!("http://127.0.0.1:{port}");
            if answers(&endpoint).await {
                return Ok(endpoint);
            }
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
    Err("Chrome started but did not open its debugging port within 20 seconds.".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_is_local_on_macos_and_vm_elsewhere() {
        assert_eq!(default_mode("macos", None), ComputerMode::Local);
        assert_eq!(default_mode("linux", None), ComputerMode::Vm);
        assert_eq!(default_mode("windows", None), ComputerMode::Vm);
    }

    #[test]
    fn env_overrides_default_and_saved_overrides_env() {
        assert_eq!(default_mode("linux", Some("local")), ComputerMode::Local);
        assert_eq!(default_mode("macos", Some(" VM ")), ComputerMode::Vm);
        // a bad env value is ignored
        assert_eq!(default_mode("macos", Some("docker")), ComputerMode::Local);
        assert_eq!(
            effective_mode(Some("vm"), "macos", Some("local")),
            ComputerMode::Vm
        );
        assert_eq!(
            effective_mode(Some("local"), "linux", Some("vm")),
            ComputerMode::Local
        );
        assert_eq!(
            effective_mode(None, "linux", Some("local")),
            ComputerMode::Local
        );
        // a bad saved value falls back to the default
        assert_eq!(
            effective_mode(Some("??"), "macos", None),
            ComputerMode::Local
        );
    }

    #[test]
    fn mode_round_trips_as_lowercase_json() {
        assert_eq!(
            serde_json::to_string(&ComputerMode::Local).unwrap(),
            "\"local\""
        );
        assert_eq!(
            serde_json::from_str::<ComputerMode>("\"vm\"").unwrap(),
            ComputerMode::Vm
        );
        assert!(serde_json::from_str::<ComputerMode>("\"VM\"").is_err());
        assert_eq!(ComputerMode::parse("Local"), Some(ComputerMode::Local));
        assert_eq!(ComputerMode::parse(""), None);
    }

    #[test]
    fn chrome_is_found_by_override_or_on_path() {
        let dir = tempfile::tempdir().unwrap();
        let bin = dir.path().join("chromium");
        std::fs::write(&bin, "").unwrap();
        let path = dir.path().to_string_lossy().to_string();
        assert_eq!(
            find_chrome(None, "linux", None, Some(&path)),
            Some(bin.clone())
        );
        let custom = dir.path().join("my-chrome");
        std::fs::write(&custom, "").unwrap();
        assert_eq!(
            find_chrome(Some(custom.to_str().unwrap()), "linux", None, Some(&path)),
            Some(custom)
        );
        // an override naming nothing finds nothing, rather than silently using another browser
        assert_eq!(
            find_chrome(Some("/nope/chrome"), "linux", None, Some(&path)),
            None
        );
        let empty = tempfile::tempdir().unwrap();
        assert_eq!(
            find_chrome(None, "linux", None, Some(&empty.path().to_string_lossy())),
            None
        );
        assert_eq!(find_chrome(None, "linux", None, None), None);
    }

    #[test]
    fn chrome_is_found_in_mac_user_applications() {
        let home = tempfile::tempdir().unwrap();
        let app = home.path().join("Applications/Chromium.app/Contents/MacOS");
        std::fs::create_dir_all(&app).unwrap();
        std::fs::write(app.join("Chromium"), "").unwrap();
        let found = find_chrome(None, "macos", Some(home.path()), None);
        // /Applications on the test machine could hold a real Chrome first
        assert!(found.is_some());
        assert_eq!(find_chrome(None, "linux", Some(home.path()), None), None);
    }

    #[test]
    fn availability_reasons() {
        assert!(has_display("macos", None, None));
        assert!(!has_display("linux", None, None));
        assert!(!has_display("linux", Some(""), None));
        assert!(has_display("linux", Some(":0"), None));
        assert!(has_display("linux", None, Some("wayland-0")));

        let chrome = Path::new("/usr/bin/chromium");
        assert_eq!(local_unavailable_reason(Some(chrome), true), None);
        assert!(local_unavailable_reason(None, true)
            .unwrap()
            .contains(CHROME_PATH_ENV));
        assert!(local_unavailable_reason(Some(chrome), false)
            .unwrap()
            .contains("display"));

        assert_eq!(vm_unavailable_reason(true, false), None);
        assert_eq!(vm_unavailable_reason(false, true), None);
        assert!(vm_unavailable_reason(false, false)
            .unwrap()
            .contains("Docker"));
    }

    #[test]
    fn chrome_uses_its_own_profile_and_a_loopback_port() {
        let args = chrome_args(Path::new("/data/local-chrome"));
        assert!(args.contains(&"--user-data-dir=/data/local-chrome".to_string()));
        assert!(args.contains(&"--remote-debugging-port=0".to_string()));
        assert!(args.contains(&"--remote-debugging-address=127.0.0.1".to_string()));
    }

    #[test]
    fn devtools_port_file() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(read_devtools_port(dir.path()), None);
        std::fs::write(
            dir.path().join("DevToolsActivePort"),
            "41234\n/devtools/browser/x\n",
        )
        .unwrap();
        assert_eq!(read_devtools_port(dir.path()), Some(41234));
        std::fs::write(dir.path().join("DevToolsActivePort"), "0\n").unwrap();
        assert_eq!(read_devtools_port(dir.path()), None);
    }
}
