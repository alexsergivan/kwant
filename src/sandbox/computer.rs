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
/// What vm mode actually runs in containers. The CLI drivers (Claude, Codex, Grok) and their own
/// file and shell tools still run on the host; `cli_agents` joins this list once they move.
pub const VM_ISOLATES: [&str; 2] = ["browser", "kwnt_shell"];

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
/// unless the person asked to see one. A windowless Chrome sends `user_agent` (see
/// [`headed_user_agent`]) instead of its own, which tells every site it is "HeadlessChrome".
pub fn chrome_args(profile: &Path, headless: bool, user_agent: Option<&str>) -> Vec<String> {
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
        if let Some(agent) = user_agent {
            args.push(format!("--user-agent={agent}"));
        }
    }
    args.push("about:blank".into());
    args
}

/// The user agent Chrome sends with a window, for the Chrome whose `--version` printed
/// `version_output` ("Google Chrome 131.0.6778.86", "Chromium 154.0.8037.92 built on ..."). Chrome
/// fills it from a fixed template with the major version and a frozen platform, so this is the same
/// string the windowed Chrome on `os` sends. `None` if no version is found.
pub fn headed_user_agent(version_output: &str, os: &str) -> Option<String> {
    let version = version_output
        .split_whitespace()
        .find(|word| word.split('.').count() >= 2 && word.split('.').all(|n| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit())))?;
    let major = version.split('.').next()?;
    let platform = match os {
        "macos" => "Macintosh; Intel Mac OS X 10_15_7",
        "windows" => "Windows NT 10.0; Win64; x64",
        _ => "X11; Linux x86_64",
    };
    Some(format!("Mozilla/5.0 ({platform}) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/{major}.0.0.0 Safari/537.36"))
}

/// [`headed_user_agent`] for the Chrome at `chrome`, from its `--version`. Not on Windows, where
/// `chrome.exe --version` opens the browser instead of printing.
pub fn chrome_user_agent(chrome: &Path) -> Option<String> {
    if cfg!(windows) {
        return None;
    }
    let out = std::process::Command::new(chrome)
        .arg("--version")
        .stdin(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .output()
        .ok()?;
    headed_user_agent(&String::from_utf8_lossy(&out.stdout), std::env::consts::OS)
}

/// A normal Chrome user agent for kwant's plain HTTP requests (`web_fetch`): the local Chrome's,
/// or, without one, a recent Chrome on this platform. Worked out once per process.
pub fn browser_user_agent() -> &'static str {
    static AGENT: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    AGENT.get_or_init(|| {
        local_chrome_path()
            .and_then(|chrome| chrome_user_agent(&chrome))
            .or_else(|| headed_user_agent("Chrome 141.0.0.0", std::env::consts::OS))
            .unwrap_or_default()
    })
}

/// Whether a running Chrome is windowless, from its `/json/version` user agent. Only a fallback
/// where its command line can't be read: kwant's windowless Chrome no longer says so there.
pub fn is_headless_user_agent(user_agent: &str) -> bool {
    user_agent.contains("HeadlessChrome")
}

/// Whether a Chrome command line (arguments joined by spaces) runs it without a window.
pub fn is_headless_command_line(line: &str) -> bool {
    line.split(' ').any(|arg| arg == "--headless" || arg.starts_with("--headless="))
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

/// The kwant Chrome running on `profile`: its CDP endpoint and whether it is windowless. That comes
/// from the `--headless` flag on its command line, since its user agent reads like a windowed one.
pub async fn running_local_chrome(profile: &Path) -> Option<(String, bool)> {
    let port = read_devtools_port(profile)?;
    let endpoint = format!("http://127.0.0.1:{port}");
    let agent = user_agent(&endpoint).await?;
    let headless = match profile_owner(profile).and_then(command_line) {
        Some(line) => is_headless_command_line(&line),
        None => is_headless_user_agent(&agent),
    };
    Some((endpoint, headless))
}

/// Whether the Chrome at `endpoint` still tells sites it is "HeadlessChrome": one started before
/// kwant set its user agent, which kwant restarts so sites see a normal Chrome.
pub async fn says_headless(endpoint: &str) -> bool {
    user_agent(endpoint).await.is_some_and(|agent| is_headless_user_agent(&agent))
}

/// The live Chrome holding `profile`, from the `SingletonLock` link Chrome keeps in it
/// ("<host>-<pid>"). `None` when no Chrome holds it, or on Windows, which has no such link.
///
/// A Chrome that crashed leaves the link behind, and its pid can later belong to an unrelated
/// program. So the pid only counts while its command line is a Chrome on this profile; trusting
/// the pid alone waited on that program forever and never started Chrome again.
pub fn profile_owner(profile: &Path) -> Option<u32> {
    let pid = lock_pid(profile)?;
    (pid_alive(pid) && command_line(pid).is_some_and(|line| is_chrome_on(&line, profile))).then_some(pid)
}

fn lock_pid(profile: &Path) -> Option<u32> {
    let target = std::fs::read_link(profile.join("SingletonLock")).ok()?;
    target.to_string_lossy().rsplit('-').next()?.parse().ok()
}

/// Whether a process command line (arguments joined by spaces) is a Chrome or Chromium started
/// with `--user-data-dir=<profile>`.
fn is_chrome_on(line: &str, profile: &Path) -> bool {
    let flag = format!("--user-data-dir={}", profile.display());
    let Some(at) = line.find(&flag) else { return false };
    let rest = &line[at + flag.len()..];
    if !(rest.is_empty() || rest.starts_with(' ')) {
        return false; // another profile whose path starts with this one
    }
    // the profile path itself may say "chrome", so look for it in the rest of the line
    format!("{}{rest}", &line[..at]).to_lowercase().contains("chrom")
}

#[cfg(target_os = "linux")]
fn command_line(pid: u32) -> Option<String> {
    let raw = std::fs::read(format!("/proc/{pid}/cmdline")).ok()?;
    let args: Vec<_> = raw.split(|b| *b == 0).filter(|a| !a.is_empty()).map(String::from_utf8_lossy).collect();
    (!args.is_empty()).then(|| args.join(" "))
}

#[cfg(all(unix, not(target_os = "linux")))]
fn command_line(pid: u32) -> Option<String> {
    let out = std::process::Command::new("ps").args(["-ww", "-o", "command=", "-p", &pid.to_string()]).output().ok()?;
    let line = String::from_utf8_lossy(&out.stdout).trim().to_string();
    (out.status.success() && !line.is_empty()).then_some(line)
}

#[cfg(not(unix))]
fn command_line(_pid: u32) -> Option<String> {
    None
}

#[cfg(unix)]
fn pid_alive(pid: u32) -> bool {
    let Ok(pid) = i32::try_from(pid) else { return false };
    if pid <= 0 {
        return false;
    }
    // signal 0 only checks; EPERM means it exists under another user
    let found = unsafe { libc::kill(pid, 0) } == 0;
    found || std::io::Error::last_os_error().raw_os_error() == Some(libc::EPERM)
}

#[cfg(not(unix))]
fn pid_alive(_pid: u32) -> bool {
    false
}

/// Waits until the kwant Chrome on `profile` stops answering and its process lets go of the
/// profile (it was asked to close).
pub async fn wait_local_chrome_gone(profile: &Path) -> Result<(), String> {
    for _ in 0..60 {
        if running_local_chrome(profile).await.is_none() && profile_owner(profile).is_none() {
            // Chrome drops its profile lock just after the port closes
            tokio::time::sleep(Duration::from_millis(300)).await;
            return Ok(());
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
    Err("Chrome did not close within 15 seconds.".into())
}

/// How long [`ensure_chrome_with`] waits, and how often it looks.
#[derive(Clone, Copy)]
pub struct LaunchTiming {
    /// For a Chrome that holds the profile but does not answer yet (starting, or busy).
    pub owner: Duration,
    /// For the Chrome just started to open its debugging port.
    pub start: Duration,
    pub step: Duration,
}

impl Default for LaunchTiming {
    fn default() -> Self {
        Self { owner: Duration::from_secs(15), start: Duration::from_secs(20), step: Duration::from_millis(250) }
    }
}

type LaunchLocks = std::sync::Mutex<std::collections::HashMap<PathBuf, std::sync::Arc<tokio::sync::Mutex<()>>>>;

/// One launch at a time per profile in this process. Every caller (the Bot browser panel, its
/// stream, a bot's turn) goes through here, so they wait for the one Chrome instead of each
/// starting their own.
fn launch_lock(profile: &Path) -> std::sync::Arc<tokio::sync::Mutex<()>> {
    static LOCKS: std::sync::LazyLock<LaunchLocks> = std::sync::LazyLock::new(Default::default);
    LOCKS.lock().unwrap_or_else(|e| e.into_inner()).entry(profile.to_path_buf()).or_default().clone()
}

/// The Chrome found holding `profile` that did not answer within the owner wait, and when.
/// `set`: `Some(Some(pid))` records it, `Some(None)` clears it, `None` only reads.
fn unresponsive(profile: &Path, set: Option<Option<u32>>) -> Option<(u32, std::time::Instant)> {
    static STUCK: std::sync::LazyLock<std::sync::Mutex<std::collections::HashMap<PathBuf, (u32, std::time::Instant)>>> =
        std::sync::LazyLock::new(Default::default);
    let mut stuck = STUCK.lock().unwrap_or_else(|e| e.into_inner());
    match set {
        Some(Some(pid)) => { stuck.insert(profile.to_path_buf(), (pid, std::time::Instant::now())); }
        Some(None) => { stuck.remove(profile); }
        None => {}
    }
    stuck.get(profile).copied()
}

/// The same across kwnt processes sharing a data folder: an advisory lock on a file next to the
/// profile, released when dropped (or when the process dies).
struct FileLock {
    _file: std::fs::File,
}

impl FileLock {
    async fn acquire(profile: &Path) -> Result<Self, String> {
        let path = profile.with_extension("launch.lock");
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
        }
        let file = std::fs::OpenOptions::new().create(true).truncate(false).write(true).open(&path)
            .map_err(|e| format!("Could not open {}: {e}", path.display()))?;
        #[cfg(unix)]
        {
            use std::os::unix::io::AsRawFd;
            for _ in 0..240 {
                if unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } == 0 {
                    return Ok(Self { _file: file });
                }
                tokio::time::sleep(Duration::from_millis(250)).await;
            }
            Err("Another kwnt is still starting Chrome on this profile.".into())
        }
        #[cfg(not(unix))]
        Ok(Self { _file: file })
    }
}

/// The kwant Chrome on `profile`, started if none is: its CDP endpoint and whether it is
/// windowless. A running Chrome is reused whatever its form (the caller restarts it if it wants the
/// other one); `headless` only applies to a start.
pub async fn ensure_local_chrome(profile: &Path, headless: bool) -> Result<(String, bool), String> {
    ensure_chrome_with(
        profile,
        || running_local_chrome(profile),
        || launch_local_chrome(profile, headless),
        LaunchTiming::default(),
    )
    .await
}

/// [`ensure_local_chrome`] with the check and the start passed in, so tests can count starts.
///
/// Never starts a second Chrome on a profile a live Chrome holds: on macOS that second start hands
/// its arguments to the running one, which opens one more blank window each time. The stale
/// `DevToolsActivePort` is removed only once no process holds the profile, since removing a live
/// Chrome's port file left every later call unable to find it, so each one started another.
pub async fn ensure_chrome_with<'a, P, PF, S>(
    profile: &Path,
    probe: P,
    spawn: S,
    timing: LaunchTiming,
) -> Result<(String, bool), String>
where
    P: Fn() -> PF,
    PF: std::future::Future<Output = Option<(String, bool)>> + 'a,
    S: FnOnce() -> Result<(), String>,
{
    let lock = launch_lock(profile);
    let _in_process = lock.lock().await;
    let _across = FileLock::acquire(profile).await?;
    if let Some(found) = probe().await {
        unresponsive(profile, Some(None));
        return Ok(found);
    }
    if let Some(pid) = profile_owner(profile) {
        // callers queued behind one that already waited the full time for this Chrome look once
        // and give up, instead of each waiting again
        let known_stuck = unresponsive(profile, None).is_some_and(|(p, at)| p == pid && at.elapsed() < timing.owner);
        let deadline = tokio::time::Instant::now() + if known_stuck { Duration::ZERO } else { timing.owner };
        loop {
            if let Some(found) = probe().await {
                unresponsive(profile, Some(None));
                return Ok(found);
            }
            if profile_owner(profile).is_none() {
                break; // it closed: start a fresh one below
            }
            if tokio::time::Instant::now() >= deadline {
                if !known_stuck {
                    unresponsive(profile, Some(Some(pid)));
                }
                return Err(format!(
                    "kwant's Chrome (process {pid}) is open but not answering, so kwant did not start another. \
                     Quit that Chrome (Cmd+Q on a Mac) and try again."
                ));
            }
            tokio::time::sleep(timing.step).await;
        }
    }
    // nothing holds the profile, so a port file or lock left behind points at nothing (a lock
    // naming another program, or an old hostname, would stop the new Chrome from starting)
    let _ = std::fs::remove_file(profile.join("DevToolsActivePort"));
    if lock_pid(profile).is_some() {
        for name in ["SingletonLock", "SingletonSocket", "SingletonCookie"] {
            let _ = std::fs::remove_file(profile.join(name));
        }
    }
    spawn()?;
    let deadline = tokio::time::Instant::now() + timing.start;
    loop {
        if let Some(found) = probe().await {
            return Ok(found);
        }
        if tokio::time::Instant::now() >= deadline {
            return Err("Chrome started but did not open its debugging port in time.".into());
        }
        tokio::time::sleep(timing.step).await;
    }
}

/// Starts the kwant Chrome on `profile`, detached, so it outlives a switch or a kwant restart.
/// Only [`ensure_local_chrome`] calls this, once no Chrome holds the profile.
fn launch_local_chrome(profile: &Path, headless: bool) -> Result<(), String> {
    let chrome = local_chrome_path().ok_or_else(|| local_reason().unwrap_or_default())?;
    if !headless {
        if let Some(reason) = window_reason() {
            return Err(reason);
        }
    }
    std::fs::create_dir_all(profile).map_err(|e| e.to_string())?;
    let agent = if headless { chrome_user_agent(&chrome) } else { None };
    let mut cmd = std::process::Command::new(&chrome);
    cmd.args(chrome_args(profile, headless, agent.as_deref()))
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
    tracing::info!(pid = child.id(), headless, profile = %profile.display(), "started kwant's local Chrome");
    // reap it when it exits; never kill it
    std::thread::spawn(move || {
        let _ = child.wait();
    });
    Ok(())
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
            let args = chrome_args(Path::new("/data/local-chrome"), headless, None);
            assert!(args.contains(&"--user-data-dir=/data/local-chrome".to_string()));
            assert!(args.contains(&"--remote-debugging-port=0".to_string()));
            assert!(args.contains(&"--remote-debugging-address=127.0.0.1".to_string()));
        }
    }

    #[test]
    fn chrome_is_windowless_unless_a_window_is_asked_for() {
        let args = chrome_args(Path::new("/p"), true, None);
        assert!(args.contains(&"--headless=new".to_string()));
        assert!(args.contains(&"--window-size=1280,800".to_string()));
        let args = chrome_args(Path::new("/p"), false, None);
        assert!(!args.iter().any(|a| a.starts_with("--headless")));
        assert!(!args.iter().any(|a| a.starts_with("--window-size")));
        assert!(is_headless_user_agent("Mozilla/5.0 (X11; Linux x86_64) HeadlessChrome/131.0 Safari/537.36"));
        assert!(!is_headless_user_agent("Mozilla/5.0 (Macintosh) Chrome/131.0 Safari/537.36"));
    }

    #[test]
    fn windowless_chrome_sends_the_windowed_user_agent() {
        assert_eq!(
            headed_user_agent("Google Chrome 131.0.6778.86 \n", "macos").as_deref(),
            Some("Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/131.0.0.0 Safari/537.36")
        );
        assert_eq!(
            headed_user_agent("Chromium 154.0.8037.92 built on Ubuntu 24.04.5 LTS", "linux").as_deref(),
            Some("Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/154.0.0.0 Safari/537.36")
        );
        assert!(headed_user_agent("Google Chrome 140.0.1.2", "windows").unwrap().contains("(Windows NT 10.0; Win64; x64)"));
        // the "24.04.5" in the build line is not taken for Chrome's version
        assert!(headed_user_agent("Chromium 154.0.8037.92 built on Ubuntu 24.04.5", "linux").unwrap().contains("Chrome/154.0.0.0"));
        assert_eq!(headed_user_agent("", "linux"), None);
        assert_eq!(headed_user_agent("Chromium version unknown", "linux"), None);

        let agent = headed_user_agent("Chromium 154.0.8037.92", "linux").unwrap();
        let args = chrome_args(Path::new("/p"), true, Some(&agent));
        assert!(args.contains(&format!("--user-agent={agent}")));
        assert!(!args.iter().any(|a| a.contains("Headless") && a.starts_with("--user-agent")));
        // a window already sends the normal one
        assert!(!chrome_args(Path::new("/p"), false, Some(&agent)).iter().any(|a| a.starts_with("--user-agent")));
        assert!(!browser_user_agent().contains("Headless"));
        assert!(browser_user_agent().contains(" Chrome/"));
    }

    #[test]
    fn windowless_is_read_from_the_command_line() {
        let headless = chrome_args(Path::new("/p"), true, Some("Mozilla/5.0 (X11; Linux x86_64) Chrome/154.0.0.0"));
        assert!(is_headless_command_line(&format!("/usr/lib/chromium/chromium {}", headless.join(" "))));
        let window = chrome_args(Path::new("/p"), false, None);
        assert!(!is_headless_command_line(&format!("/usr/lib/chromium/chromium {}", window.join(" "))));
        assert!(is_headless_command_line("chrome --headless --user-data-dir=/p"));
        // a profile or user agent that merely mentions it is not the flag
        assert!(!is_headless_command_line("chrome --user-data-dir=/home/me/--headless-notes"));
        assert!(!is_headless_command_line("chrome --user-agent=Mozilla/5.0 HeadlessChrome/154 --headlessly"));
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

    fn fast() -> LaunchTiming {
        LaunchTiming {
            owner: Duration::from_millis(400),
            start: Duration::from_secs(5),
            step: Duration::from_millis(20),
        }
    }

    /// A pid that is not running: a child that already exited and was reaped.
    fn dead_pid() -> u32 {
        let mut child = std::process::Command::new("true").spawn().unwrap();
        let pid = child.id();
        child.wait().unwrap();
        pid
    }

    #[cfg(unix)]
    fn hold_profile(profile: &Path, pid: u32) {
        std::os::unix::fs::symlink(format!("host-{pid}"), profile.join("SingletonLock")).unwrap();
    }

    /// A live process, alive until dropped. `chrome_on`: it looks like a Chrome started on that
    /// profile ("chrome --user-data-dir=<profile>" on its command line); otherwise it is some
    /// unrelated program, like the `sleep` whose pid reused a crashed Chrome's.
    #[cfg(unix)]
    struct Process(std::process::Child);

    #[cfg(unix)]
    impl Process {
        fn start(chrome_on: Option<&Path>) -> Self {
            let mut cmd = std::process::Command::new("sh");
            cmd.args(["-c", "read _"]);
            if let Some(profile) = chrome_on {
                cmd.arg("chrome").arg(format!("--user-data-dir={}", profile.display()));
            }
            let child = cmd.stdin(std::process::Stdio::piped()).spawn().unwrap();
            // just after the spawn its command line can still read as empty, which made
            // `profile_owner` miss it on busy machines; wait until `sh` shows up there
            let started = std::time::Instant::now();
            while !command_line(child.id()).is_some_and(|line| line.contains("read _")) {
                assert!(started.elapsed() < Duration::from_secs(5), "test process did not start");
                std::thread::sleep(Duration::from_millis(5));
            }
            Self(child)
        }
        fn id(&self) -> u32 { self.0.id() }
    }

    #[cfg(unix)]
    impl Drop for Process {
        fn drop(&mut self) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }

    #[cfg(unix)]
    #[test]
    fn profile_owner_reads_chromes_singleton_lock() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(profile_owner(dir.path()), None);
        let chrome = Process::start(Some(dir.path()));
        hold_profile(dir.path(), chrome.id());
        assert_eq!(profile_owner(dir.path()), Some(chrome.id()));
        std::fs::remove_file(dir.path().join("SingletonLock")).unwrap();
        hold_profile(dir.path(), dead_pid());
        assert_eq!(profile_owner(dir.path()), None);
    }

    /// Bug A: Chrome crashed and its pid now belongs to an unrelated live program. The lock names
    /// that pid, but it does not hold the profile.
    #[cfg(unix)]
    #[test]
    fn a_live_pid_that_is_not_this_profiles_chrome_does_not_hold_it() {
        let dir = tempfile::tempdir().unwrap();
        let other = Process::start(None);
        hold_profile(dir.path(), other.id());
        assert_eq!(profile_owner(dir.path()), None);
        // nor does a Chrome on another profile, even one whose path starts with this one
        let sibling = dir.path().with_extension("other");
        let chrome = Process::start(Some(&sibling));
        std::fs::remove_file(dir.path().join("SingletonLock")).unwrap();
        hold_profile(dir.path(), chrome.id());
        assert_eq!(profile_owner(dir.path()), None);
    }

    #[test]
    fn chrome_command_lines() {
        let p = Path::new("/Users/o/Library/Application Support/kwant/local-chrome");
        let flag = format!("--user-data-dir={}", p.display());
        assert!(is_chrome_on(&format!("/Applications/Google Chrome.app/Contents/MacOS/Google Chrome {flag} --remote-debugging-port=0"), p));
        assert!(is_chrome_on(&format!("/usr/bin/chromium {flag}"), p));
        assert!(!is_chrome_on(&format!("/usr/bin/chromium {flag}2"), p));
        assert!(!is_chrome_on("sleep 600", p));
        // the profile is called local-chrome, which alone is not a Chrome
        assert!(!is_chrome_on(&format!("/usr/bin/python3 {flag}"), p));
    }

    /// Rigel's repro of Bug A: Chrome was killed, and SingletonLock names a live `sleep`. One
    /// fresh Chrome starts at once, and the stale lock is cleared so it can.
    #[cfg(unix)]
    #[tokio::test]
    async fn a_lock_naming_another_program_starts_one_chrome() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("DevToolsActivePort"), "1\n").unwrap();
        let other = Process::start(None);
        hold_profile(dir.path(), other.id());
        let up = std::sync::atomic::AtomicBool::new(false);
        let probe = || {
            let found = up.load(std::sync::atomic::Ordering::SeqCst);
            async move { found.then(|| ("http://127.0.0.1:9".to_string(), true)) }
        };
        let lock = dir.path().join("SingletonLock");
        let mut spawns = 0;
        let timing = LaunchTiming { owner: Duration::from_secs(30), ..fast() };
        let started = std::time::Instant::now();
        let result = ensure_chrome_with(dir.path(), probe, || {
            assert!(std::fs::symlink_metadata(&lock).is_err(), "the stale lock goes before the start");
            spawns += 1;
            up.store(true, std::sync::atomic::Ordering::SeqCst);
            Ok(())
        }, timing).await;
        assert!(result.is_ok(), "{result:?}");
        assert_eq!(spawns, 1);
        assert!(started.elapsed() < Duration::from_secs(5), "it did not wait on the other program");
    }

    /// The Bot browser button, its stream and a bot's turn all asking at once start one Chrome.
    #[tokio::test]
    async fn concurrent_launches_start_one_chrome() {
        use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering::SeqCst};
        use std::sync::Arc;
        let dir = tempfile::tempdir().unwrap();
        let profile = dir.path().join("local-chrome");
        let spawns = Arc::new(AtomicUsize::new(0));
        let up = Arc::new(AtomicBool::new(false));
        let mut calls = Vec::new();
        for _ in 0..10 {
            let (profile, spawns, up) = (profile.clone(), spawns.clone(), up.clone());
            calls.push(tokio::spawn(async move {
                let probe = || {
                    let up = up.clone();
                    async move { up.load(SeqCst).then(|| ("http://127.0.0.1:9".to_string(), true)) }
                };
                let spawn = || {
                    spawns.fetch_add(1, SeqCst);
                    let up = up.clone();
                    // Chrome takes a moment to open its port
                    tokio::spawn(async move {
                        tokio::time::sleep(Duration::from_millis(150)).await;
                        up.store(true, SeqCst);
                    });
                    Ok(())
                };
                ensure_chrome_with(&profile, probe, spawn, fast()).await
            }));
        }
        for call in calls {
            assert!(call.await.unwrap().is_ok());
        }
        assert_eq!(spawns.load(SeqCst), 1);
    }

    /// A live Chrome that holds the profile but does not answer (its port file stale or gone) is
    /// never started again, and its port file is left alone.
    #[cfg(unix)]
    #[tokio::test]
    async fn stale_port_file_with_live_chrome_does_not_start_another() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("DevToolsActivePort"), "1\n/devtools/browser/x\n").unwrap();
        let chrome = Process::start(Some(dir.path()));
        hold_profile(dir.path(), chrome.id());
        let mut spawned = false;
        let result = ensure_chrome_with(dir.path(), || async { None }, || { spawned = true; Ok(()) }, fast()).await;
        let error = result.unwrap_err();
        assert!(!spawned);
        assert!(error.contains("did not start another"), "{error}");
        assert!(dir.path().join("DevToolsActivePort").exists());
    }

    /// Clicks queued behind one that waited the full time for a stuck Chrome give up at once.
    #[cfg(unix)]
    #[tokio::test]
    async fn callers_behind_a_stuck_chrome_do_not_each_wait() {
        let dir = tempfile::tempdir().unwrap();
        let chrome = Process::start(Some(dir.path()));
        hold_profile(dir.path(), chrome.id());
        let timing = LaunchTiming { owner: Duration::from_secs(2), ..fast() };
        let first = std::time::Instant::now();
        assert!(ensure_chrome_with(dir.path(), || async { None }, || Ok(()), timing).await.is_err());
        assert!(first.elapsed() >= Duration::from_secs(2));
        let again = std::time::Instant::now();
        let error = ensure_chrome_with(dir.path(), || async { None }, || Ok(()), timing).await.unwrap_err();
        assert!(again.elapsed() < Duration::from_millis(500), "{:?}", again.elapsed());
        assert!(error.contains("did not start another"), "{error}");
        // once it answers it is used, and forgotten as stuck
        let found = ensure_chrome_with(dir.path(), || async { Some(("u".to_string(), true)) }, || Ok(()), timing).await;
        assert_eq!(found.unwrap().0, "u");
        assert!(unresponsive(dir.path(), None).is_none());
    }

    /// The same live Chrome that answers once it has started is reused, not started again.
    #[cfg(unix)]
    #[tokio::test]
    async fn starting_chrome_is_waited_for() {
        use std::sync::atomic::{AtomicUsize, Ordering::SeqCst};
        let dir = tempfile::tempdir().unwrap();
        let chrome = Process::start(Some(dir.path()));
        hold_profile(dir.path(), chrome.id());
        let probes = AtomicUsize::new(0);
        let probe = || {
            let n = probes.fetch_add(1, SeqCst);
            async move { (n >= 3).then(|| ("http://127.0.0.1:9".to_string(), false)) }
        };
        let mut spawned = false;
        let found = ensure_chrome_with(dir.path(), probe, || { spawned = true; Ok(()) }, fast()).await.unwrap();
        assert!(!spawned);
        assert_eq!(found, ("http://127.0.0.1:9".to_string(), false));
    }

    /// A port file left by a Chrome that is gone is removed and one fresh Chrome starts.
    #[cfg(unix)]
    #[tokio::test]
    async fn stale_port_file_with_dead_chrome_starts_one() {
        let dir = tempfile::tempdir().unwrap();
        let port_file = dir.path().join("DevToolsActivePort");
        std::fs::write(&port_file, "1\n").unwrap();
        hold_profile(dir.path(), dead_pid());
        let up = std::sync::atomic::AtomicBool::new(false);
        let probe = || {
            let found = up.load(std::sync::atomic::Ordering::SeqCst);
            async move { found.then(|| ("http://127.0.0.1:9".to_string(), true)) }
        };
        let mut spawns = 0;
        let result = ensure_chrome_with(dir.path(), probe, || {
            assert!(!port_file.exists(), "the stale port file goes before the start");
            spawns += 1;
            up.store(true, std::sync::atomic::Ordering::SeqCst);
            Ok(())
        }, fast()).await;
        assert!(result.is_ok());
        assert_eq!(spawns, 1);
    }
}
