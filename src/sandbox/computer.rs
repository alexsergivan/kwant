//! Where bots do computer work: the person's own computer ("local": their installed Chrome with a
//! kwant-only profile, run without a window and streamed into the app unless the person asks for
//! a window, and host shell commands that each need approval) or isolated containers
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

/// Why local mode cannot run here, or `None` if it can. No display is needed: local Chrome runs
/// without a window and the app shows it.
pub fn local_unavailable_reason(chrome: Option<&Path>) -> Option<String> {
    chrome.is_none().then(|| format!(
        "No Chrome or Chromium was found. Install Google Chrome, or set {CHROME_PATH_ENV} to its path."
    ))
}

/// Why local Chrome cannot be opened as a window here, or `None` if it can.
pub fn window_unavailable_reason(display: bool) -> Option<String> {
    (!display).then(|| "This computer has no screen to show a browser window on (no display).".into())
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
    local_unavailable_reason(local_chrome_path().as_deref())
}

/// [`window_unavailable_reason`] for this process's environment.
pub fn window_reason() -> Option<String> {
    window_unavailable_reason(has_display(
        std::env::consts::OS,
        std::env::var("DISPLAY").ok().as_deref(),
        std::env::var("WAYLAND_DISPLAY").ok().as_deref(),
    ))
}

/// Reads the debugging port Chrome writes to `DevToolsActivePort` in its profile folder.
pub fn read_devtools_port(profile: &Path) -> Option<u16> {
    let text = std::fs::read_to_string(profile.join("DevToolsActivePort")).ok()?;
    text.lines().next()?.trim().parse().ok().filter(|&p| p > 0)
}

/// The viewport of windowless local Chrome, matching the vm browser and the in-app viewer.
pub const HEADLESS_WINDOW_SIZE: &str = "1280,800";

/// Chrome's command line for local mode: its own profile, never the person's everyday one, and a
/// debugging port Chrome picks itself (it listens on 127.0.0.1). Without a window (`headless`)
/// unless the person asked to see one.
pub fn chrome_args(profile: &Path, headless: bool) -> Vec<String> {
    let mut args = vec![
        format!("--user-data-dir={}", profile.display()),
        "--remote-debugging-port=0".into(),
        "--remote-debugging-address=127.0.0.1".into(),
        "--no-first-run".into(),
        "--no-default-browser-check".into(),
    ];
    if headless {
        args.push("--headless=new".into());
        args.push(format!("--window-size={HEADLESS_WINDOW_SIZE}"));
    }
    args.push("about:blank".into());
    args
}

/// Whether a running Chrome is windowless, from its `/json/version` user agent.
pub fn is_headless_user_agent(user_agent: &str) -> bool {
    user_agent.contains("HeadlessChrome")
}

fn client() -> reqwest::Client {
    reqwest::Client::builder()
        .timeout(Duration::from_secs(2))
        .build()
        .unwrap_or_default()
}

/// The user agent of the Chrome answering at `endpoint`, or `None` if nothing answers.
async fn user_agent(endpoint: &str) -> Option<String> {
    let resp = client().get(format!("{endpoint}/json/version")).send().await.ok()?;
    if !resp.status().is_success() {
        return None;
    }
    let version: serde_json::Value = resp.json().await.ok()?;
    Some(version["User-Agent"].as_str().unwrap_or_default().to_string())
}

/// The kwant Chrome running on `profile`: its CDP endpoint and whether it is windowless.
pub async fn running_local_chrome(profile: &Path) -> Option<(String, bool)> {
    let port = read_devtools_port(profile)?;
    let endpoint = format!("http://127.0.0.1:{port}");
    let agent = user_agent(&endpoint).await?;
    Some((endpoint, is_headless_user_agent(&agent)))
}

/// Waits until the kwant Chrome on `profile` stops answering (it was asked to close).
pub async fn wait_local_chrome_gone(profile: &Path) -> Result<(), String> {
    for _ in 0..60 {
        if running_local_chrome(profile).await.is_none() {
            // Chrome drops its profile lock just after the port closes
            tokio::time::sleep(Duration::from_millis(300)).await;
            return Ok(());
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
    Err("Chrome did not close within 15 seconds.".into())
}

/// Launches the kwant Chrome on `profile` (detached, so it outlives a switch or a kwant restart)
/// and returns its CDP endpoint. The caller makes sure none is running on this profile.
pub async fn launch_local_chrome(profile: &Path, headless: bool) -> Result<String, String> {
    let chrome = local_chrome_path().ok_or_else(|| local_reason().unwrap_or_default())?;
    if !headless {
        if let Some(reason) = window_reason() {
            return Err(reason);
        }
    }
    std::fs::create_dir_all(profile).map_err(|e| e.to_string())?;
    // a stale port file from a closed Chrome would point at nothing
    let _ = std::fs::remove_file(profile.join("DevToolsActivePort"));
    let mut cmd = std::process::Command::new(&chrome);
    cmd.args(chrome_args(profile, headless))
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
        if let Some((endpoint, _)) = running_local_chrome(profile).await {
            return Ok(endpoint);
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

        // local Chrome runs without a window, so only a window needs a display
        let chrome = Path::new("/usr/bin/chromium");
        assert_eq!(local_unavailable_reason(Some(chrome)), None);
        assert!(local_unavailable_reason(None)
            .unwrap()
            .contains(CHROME_PATH_ENV));
        assert_eq!(window_unavailable_reason(true), None);
        assert!(window_unavailable_reason(false).unwrap().contains("display"));

        assert_eq!(vm_unavailable_reason(true, false), None);
        assert_eq!(vm_unavailable_reason(false, true), None);
        assert!(vm_unavailable_reason(false, false)
            .unwrap()
            .contains("Docker"));
    }

    #[test]
    fn chrome_uses_its_own_profile_and_a_loopback_port() {
        for headless in [true, false] {
            let args = chrome_args(Path::new("/data/local-chrome"), headless);
            assert!(args.contains(&"--user-data-dir=/data/local-chrome".to_string()));
            assert!(args.contains(&"--remote-debugging-port=0".to_string()));
            assert!(args.contains(&"--remote-debugging-address=127.0.0.1".to_string()));
        }
    }

    #[test]
    fn chrome_is_windowless_unless_a_window_is_asked_for() {
        let args = chrome_args(Path::new("/p"), true);
        assert!(args.contains(&"--headless=new".to_string()));
        assert!(args.contains(&"--window-size=1280,800".to_string()));
        let args = chrome_args(Path::new("/p"), false);
        assert!(!args.iter().any(|a| a.starts_with("--headless")));
        assert!(!args.iter().any(|a| a.starts_with("--window-size")));
        assert!(is_headless_user_agent("Mozilla/5.0 (X11; Linux x86_64) HeadlessChrome/131.0 Safari/537.36"));
        assert!(!is_headless_user_agent("Mozilla/5.0 (Macintosh) Chrome/131.0 Safari/537.36"));
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
