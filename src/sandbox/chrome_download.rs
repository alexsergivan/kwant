//! The bots' private browser without Node (0.7.6, O5): when a bot first needs the local browser
//! and this computer has no Chrome or Chromium, kwnt downloads Google's Chrome for Testing build
//! into its own cache folder (`~/Library/Caches/kwant/chrome` on a Mac, `~/.cache/kwant/chrome`
//! on Linux) and uses that. No admin rights, no Homebrew, no Node.
//!
//! It is silent unless it fails. Then [`FAILED_LINE`] is what the person sees, with "Try again"
//! ([`ChromeSetup::retry`]).
//!
//! Chrome for Testing publishes no checksums or signatures, so the download is taken only over
//! HTTPS from Google's own storage (`storage.googleapis.com/chrome-for-testing-public/`, as listed
//! in Google's JSON), checked for a complete body and the expected browser inside, and has to
//! start (`--version`) before it is used.
//!
//! There's no build for Linux on ARM, and a Linux that blocks the sandbox of a browser in the
//! home folder (Ubuntu 23.10+) couldn't start one, so neither downloads: they keep the existing
//! "install Chromium" advice.
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use serde::Serialize;

/// Google's list of the current Chrome for Testing builds and their downloads.
pub const CFT_JSON: &str =
    "https://googlechromelabs.github.io/chrome-for-testing/last-known-good-versions-with-downloads.json";
/// Where Google serves the builds; a download anywhere else is refused.
pub const CFT_STORAGE: &str = "https://storage.googleapis.com/chrome-for-testing-public/";
/// What the person sees when the browser couldn't be set up (design.md §8).
pub const FAILED_LINE: &str = "The bots' browser couldn't be set up. Bots can still answer; web pages won't open.";
/// After a failure, the next bot that needs the browser tries again only after this long (or at
/// once on "Try again"), so a broken network doesn't restart a large download on every call.
pub const RETRY_AFTER: Duration = Duration::from_secs(10 * 60);
/// A build is about 150–200 MB; anything past this is not a browser.
pub const MAX_BYTES: u64 = 1 << 30;

/// The Chrome for Testing platform for this OS and CPU, or `None` where Google has no build
/// (Linux on ARM) or kwnt doesn't download one (Windows).
pub fn cft_platform(os: &str, arch: &str) -> Option<&'static str> {
    match (os, arch) {
        ("macos", "aarch64") => Some("mac-arm64"),
        ("macos", "x86_64") => Some("mac-x64"),
        ("linux", "x86_64") => Some("linux64"),
        _ => None,
    }
}

/// The browser inside a build's zip (and inside its install folder).
pub fn binary_in(platform: &str) -> PathBuf {
    let app = "Google Chrome for Testing.app/Contents/MacOS/Google Chrome for Testing";
    match platform {
        "mac-arm64" | "mac-x64" => PathBuf::from(format!("chrome-{platform}")).join(app),
        _ => PathBuf::from(format!("chrome-{platform}")).join("chrome"),
    }
}

/// kwnt's own browser cache under `home`: `Library/Caches/kwant/chrome` on a Mac,
/// `.cache/kwant/chrome` elsewhere.
pub fn cache_root(os: &str, home: &Path) -> PathBuf {
    if os == "macos" {
        home.join("Library/Caches/kwant/chrome")
    } else {
        home.join(".cache/kwant/chrome")
    }
}

/// The newest browser kwnt downloaded under `home`, if any. Builds sit in
/// `<cache_root>/<version>-<platform>/`.
pub fn downloaded_chrome(os: &str, home: &Path) -> Option<PathBuf> {
    newest_in(&cache_root(os, home))
}

fn newest_in(root: &Path) -> Option<PathBuf> {
    let mut builds: Vec<(Vec<u32>, PathBuf)> = std::fs::read_dir(root)
        .ok()?
        .flatten()
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().to_string();
            let (version, platform) = name.split_once('-')?;
            let key: Vec<u32> = version.split('.').map(|n| n.parse().ok()).collect::<Option<_>>()?;
            let bin = e.path().join(binary_in(platform));
            bin.is_file().then_some((key, bin))
        })
        .collect();
    builds.sort_by(|a, b| b.0.cmp(&a.0));
    builds.into_iter().next().map(|(_, bin)| bin)
}

/// Where the setup stands, for the UI. The line shows only for `failed`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SetupState {
    /// Nothing has needed it yet (or Chrome is installed).
    Idle,
    /// Downloading or checking a build.
    Downloading,
    /// A build kwnt downloaded is in place.
    Ready,
    /// The last attempt failed: show [`FAILED_LINE`] and "Try again".
    Failed,
    /// No build for this computer (Linux on ARM, Windows) or its sandbox is blocked: the
    /// existing "install Chrome" advice applies.
    Unavailable,
}

/// [`ChromeSetup::view`]: what `GET /api/settings/computer` shows as `browser_setup`.
#[derive(Debug, Clone, Serialize)]
pub struct SetupView {
    pub state: SetupState,
    /// [`FAILED_LINE`] when `failed`.
    pub error: Option<String>,
    /// Why it failed, in a few words (no paths or tokens), for a details line or logs.
    pub detail: Option<String>,
    /// The Chrome for Testing version in use or being fetched.
    pub version: Option<String>,
}

struct Status {
    state: SetupState,
    detail: Option<String>,
    version: Option<String>,
    failed_at: Option<Instant>,
}

/// The private browser download for one cache folder. One download runs at a time; every bot
/// that needs the browser meanwhile waits for it.
pub struct ChromeSetup {
    /// Google's JSON (or a test server's).
    json_url: String,
    /// Downloads must start with this.
    storage: String,
    root: PathBuf,
    platform: Option<&'static str>,
    status: Mutex<Status>,
    run: tokio::sync::Mutex<()>,
    max_bytes: u64,
}

impl ChromeSetup {
    pub fn new(root: PathBuf, platform: Option<&'static str>) -> Self {
        Self {
            json_url: CFT_JSON.into(),
            storage: CFT_STORAGE.into(),
            root,
            platform,
            status: Mutex::new(Status { state: SetupState::Idle, detail: None, version: None, failed_at: None }),
            run: tokio::sync::Mutex::new(()),
            max_bytes: MAX_BYTES,
        }
    }

    /// For this computer: `$HOME`'s cache folder and this OS and CPU. Linux whose sandbox is
    /// blocked, and Docker, get no download.
    pub fn system() -> Self {
        let home = std::env::var_os("HOME").map(PathBuf::from).unwrap_or_default();
        let os = std::env::consts::OS;
        let platform = cft_platform(os, std::env::consts::ARCH).filter(|_| {
            !super::computer::chrome_sandbox_blocked()
                && crate::doctor::Platform::detect() != crate::doctor::Platform::Docker
                && !home.as_os_str().is_empty()
        });
        let setup = Self::new(cache_root(os, &home), platform);
        #[cfg(feature = "test-seams")]
        if let Ok(url) = std::env::var("KWNT_CHROME_FOR_TESTING_URL") {
            // A fake list (live QA): its downloads must sit in the list's own folder
            let storage = url.rfind('/').map(|i| url[..=i].to_string()).unwrap_or_else(|| url.clone());
            return setup.source(&url, &storage);
        }
        setup
    }

    /// Takes the build list from `json_url` and accepts downloads under `storage` (tests).
    pub fn source(mut self, json_url: &str, storage: &str) -> Self {
        self.json_url = json_url.into();
        self.storage = storage.into();
        self
    }

    pub fn max_bytes(mut self, max: u64) -> Self {
        self.max_bytes = max;
        self
    }

    /// Whether this computer can get a browser this way.
    pub fn available(&self) -> bool {
        self.platform.is_some()
    }

    /// The browser downloaded earlier, if it's in place.
    pub fn installed(&self) -> Option<PathBuf> {
        self.platform?;
        newest_in(&self.root)
    }

    pub fn view(&self) -> SetupView {
        let s = self.status.lock().unwrap();
        let state = match (&s.state, self.platform) {
            (_, None) => SetupState::Unavailable,
            (SetupState::Idle, _) if self.installed().is_some() => SetupState::Ready,
            (state, _) => state.clone(),
        };
        SetupView {
            error: (state == SetupState::Failed).then(|| FAILED_LINE.to_string()),
            detail: if state == SetupState::Failed { s.detail.clone() } else { None },
            version: s.version.clone(),
            state,
        }
    }

    fn set(&self, state: SetupState, detail: Option<String>) {
        let mut s = self.status.lock().unwrap();
        s.failed_at = (state == SetupState::Failed).then(Instant::now);
        s.state = state;
        s.detail = detail;
    }

    /// The private browser, downloading it first if it isn't there. Several callers share one
    /// download. After a failure, callers get the failure at once until "Try again" or
    /// [`RETRY_AFTER`].
    pub async fn ensure(&self) -> Result<PathBuf, String> {
        let Some(platform) = self.platform else {
            return Err("There's no private browser download for this computer.".into());
        };
        if let Some(bin) = self.installed() {
            return Ok(bin);
        }
        let _one = self.run.lock().await;
        if let Some(bin) = self.installed() {
            self.set(SetupState::Ready, None);
            return Ok(bin);
        }
        {
            let s = self.status.lock().unwrap();
            if s.state == SetupState::Failed && s.failed_at.is_some_and(|t| t.elapsed() < RETRY_AFTER) {
                return Err(failed_message(s.detail.as_deref()));
            }
        }
        self.set(SetupState::Downloading, None);
        match self.download(platform).await {
            Ok(bin) => {
                self.set(SetupState::Ready, None);
                Ok(bin)
            }
            Err(why) => {
                tracing::warn!(reason = %why, "the bots' private browser couldn't be set up");
                self.set(SetupState::Failed, Some(why.clone()));
                Err(failed_message(Some(&why)))
            }
        }
    }

    /// "Try again": forgets the last failure and starts the download in the background.
    pub fn retry(self: &std::sync::Arc<Self>) {
        {
            let mut s = self.status.lock().unwrap();
            if s.state == SetupState::Failed {
                s.failed_at = None;
            }
        }
        if self.platform.is_none() || self.installed().is_some() {
            return;
        }
        let me = self.clone();
        tokio::spawn(async move {
            let _ = me.ensure().await;
        });
    }

    async fn download(&self, platform: &'static str) -> Result<PathBuf, String> {
        use futures::StreamExt;
        use tokio::io::AsyncWriteExt;

        let client = reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(30))
            .user_agent(concat!("kwnt/", env!("CARGO_PKG_VERSION")))
            .build()
            .map_err(|e| e.to_string())?;

        // Which build: the Stable channel's download for this platform
        let list: serde_json::Value = async {
            let r = client
                .get(&self.json_url)
                .timeout(Duration::from_secs(60))
                .send()
                .await
                .map_err(|_| "couldn't reach Google's Chrome for Testing list (offline?)".to_string())?;
            if !r.status().is_success() {
                return Err(format!("Google's Chrome for Testing list answered {}", r.status()));
            }
            r.json().await.map_err(|_| "Google's Chrome for Testing list couldn't be read".to_string())
        }
        .await?;
        let stable = &list["channels"]["Stable"];
        let version = stable["version"]
            .as_str()
            .filter(|v| !v.is_empty() && v.len() < 32 && v.split('.').all(|n| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit())))
            .ok_or("Google's Chrome for Testing list has no Stable version")?
            .to_string();
        let url = stable["downloads"]["chrome"]
            .as_array()
            .and_then(|all| all.iter().find(|d| d["platform"] == platform))
            .and_then(|d| d["url"].as_str())
            .ok_or_else(|| format!("Chrome for Testing {version} has no build for {platform}"))?
            .to_string();
        if !url.starts_with(&self.storage) || url.contains("..") {
            return Err(format!("Chrome for Testing {version} points somewhere other than Google's storage, so it wasn't downloaded"));
        }
        self.status.lock().unwrap().version = Some(version.clone());
        tracing::info!(version = %version, platform, "downloading the bots' private browser (Chrome for Testing)");

        std::fs::create_dir_all(&self.root).map_err(|_| "couldn't make kwant's browser cache folder".to_string())?;
        let zip = self.root.join(format!(".download.{}.zip", std::process::id()));
        let staging = self.root.join(format!(".staging.{version}-{platform}.{}", std::process::id()));
        struct Cleanup(PathBuf, PathBuf);
        impl Drop for Cleanup {
            fn drop(&mut self) {
                let _ = std::fs::remove_file(&self.0);
                let _ = std::fs::remove_dir_all(&self.1);
            }
        }
        let _cleanup = Cleanup(zip.clone(), staging.clone());

        let resp = client
            .get(&url)
            .send()
            .await
            .map_err(|_| "couldn't reach Google's storage to download Chrome for Testing (offline?)".to_string())?;
        if !resp.status().is_success() {
            return Err(format!("the Chrome for Testing download answered {}", resp.status()));
        }
        let total = resp.content_length().filter(|n| *n > 0);
        if total.is_some_and(|n| n > self.max_bytes) {
            return Err(format!("the Chrome for Testing download is larger than {} MB", self.max_bytes >> 20));
        }
        let mut file = tokio::fs::File::create(&zip).await.map_err(|_| "couldn't write to kwant's browser cache folder".to_string())?;
        let (mut got, mut next_note) = (0u64, 0u64);
        let mut body = resp.bytes_stream();
        loop {
            let chunk = match tokio::time::timeout(Duration::from_secs(120), body.next()).await {
                Err(_) => return Err("the Chrome for Testing download stalled".into()),
                Ok(None) => break,
                Ok(Some(c)) => c.map_err(|_| "the Chrome for Testing download broke off".to_string())?,
            };
            got += chunk.len() as u64;
            if got > self.max_bytes {
                return Err(format!("the Chrome for Testing download is larger than {} MB", self.max_bytes >> 20));
            }
            file.write_all(&chunk).await.map_err(|_| "couldn't write to kwant's browser cache folder (disk full?)".to_string())?;
            if got >= next_note {
                tracing::info!(mb = got >> 20, of_mb = total.map(|t| t >> 20), "downloading the bots' private browser");
                next_note = got + (25 << 20);
            }
        }
        file.flush().await.map_err(|e| e.to_string())?;
        drop(file);
        if total.is_some_and(|t| t != got) {
            return Err("the Chrome for Testing download ended early".into());
        }

        // Unpack beside earlier builds; only a complete, starting build is moved into place
        let inner = binary_in(platform);
        let (zip_path, staging_dir, want) = (zip.clone(), staging.clone(), inner.clone());
        tokio::task::spawn_blocking(move || -> Result<(), String> {
            let file = std::fs::File::open(&zip_path).map_err(|_| "couldn't read the download".to_string())?;
            let mut archive = zip::ZipArchive::new(file).map_err(|_| "the Chrome for Testing download isn't a readable zip".to_string())?;
            let wanted = want.to_string_lossy().replace('\\', "/");
            if archive.index_for_name(&wanted).is_none() {
                return Err("the Chrome for Testing download has no browser inside".into());
            }
            let _ = std::fs::remove_dir_all(&staging_dir);
            std::fs::create_dir_all(&staging_dir).map_err(|_| "couldn't make kwant's browser cache folder".to_string())?;
            archive.extract(&staging_dir).map_err(|_| "the Chrome for Testing download couldn't be unpacked".to_string())
        })
        .await
        .map_err(|_| "unpacking the browser stopped".to_string())??;
        let bin = staging.join(&inner);
        if !bin.is_file() {
            return Err("the Chrome for Testing download has no browser inside".into());
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o755));
        }
        let said = starts(&bin).await?;
        tracing::info!(version = %said, "the bots' private browser starts");

        let dest = self.root.join(format!("{version}-{platform}"));
        if dest.exists() {
            std::fs::remove_dir_all(&dest).map_err(|_| "couldn't replace an older private browser".to_string())?;
        }
        std::fs::rename(&staging, &dest).map_err(|_| "couldn't move the browser into place".to_string())?;
        Ok(dest.join(inner))
    }
}

fn failed_message(detail: Option<&str>) -> String {
    match detail {
        Some(d) => format!("{FAILED_LINE} ({d})"),
        None => FAILED_LINE.into(),
    }
}

/// Runs `bin --version` (retrying a moment while another process still holds the new file open),
/// and gives its first line. A build that can't start (missing system libraries) fails here.
async fn starts(bin: &Path) -> Result<String, String> {
    let mut tries = 0;
    let out = loop {
        let run = tokio::process::Command::new(bin)
            .arg("--version")
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true)
            .output();
        match tokio::time::timeout(Duration::from_secs(60), run).await {
            Err(_) => return Err("the downloaded browser didn't answer --version".into()),
            // ETXTBSY: a process forked elsewhere still has the just-written file open
            Ok(Err(e)) if e.raw_os_error() == Some(26) && tries < 20 => {
                tries += 1;
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
            Ok(Err(_)) => return Err("the downloaded browser couldn't be started".into()),
            Ok(Ok(out)) => break out,
        }
    };
    let said = String::from_utf8_lossy(&out.stdout).lines().next().unwrap_or_default().trim().to_string();
    if !out.status.success() || !said.contains("Chrom") {
        return Err("the downloaded browser doesn't start here (missing system libraries?)".into());
    }
    Ok(said)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn platforms_with_a_build() {
        assert_eq!(cft_platform("macos", "aarch64"), Some("mac-arm64"));
        assert_eq!(cft_platform("macos", "x86_64"), Some("mac-x64"));
        assert_eq!(cft_platform("linux", "x86_64"), Some("linux64"));
        // Google has no Linux ARM build; Windows isn't downloaded
        assert_eq!(cft_platform("linux", "aarch64"), None);
        assert_eq!(cft_platform("windows", "x86_64"), None);
    }

    #[test]
    fn where_builds_go_and_which_one_is_used() {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path();
        assert_eq!(cache_root("macos", home), home.join("Library/Caches/kwant/chrome"));
        assert_eq!(cache_root("linux", home), home.join(".cache/kwant/chrome"));
        assert_eq!(
            binary_in("mac-arm64"),
            PathBuf::from("chrome-mac-arm64/Google Chrome for Testing.app/Contents/MacOS/Google Chrome for Testing")
        );
        assert_eq!(downloaded_chrome("linux", home), None);
        let root = cache_root("linux", home);
        for v in ["9.0.1.2", "131.0.6778.85", "131.0.6778.9"] {
            let bin = root.join(format!("{v}-linux64")).join(binary_in("linux64"));
            std::fs::create_dir_all(bin.parent().unwrap()).unwrap();
            std::fs::write(&bin, "").unwrap();
        }
        // a half-unpacked or foreign folder is ignored
        std::fs::create_dir_all(root.join(".staging.999.0.0.0-linux64.1/chrome-linux64")).unwrap();
        std::fs::create_dir_all(root.join("999.0.0.0-linux64")).unwrap();
        assert_eq!(
            downloaded_chrome("linux", home),
            Some(root.join("131.0.6778.85-linux64").join(binary_in("linux64")))
        );
    }

    #[test]
    fn no_build_means_unavailable_and_no_line() {
        let tmp = tempfile::tempdir().unwrap();
        let setup = ChromeSetup::new(tmp.path().to_path_buf(), None);
        let v = setup.view();
        assert_eq!(v.state, SetupState::Unavailable);
        assert!(v.error.is_none());
        assert!(!setup.available());
    }
}
