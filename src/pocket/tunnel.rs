//! Join from anywhere (pa6, Pocket API v1.2): a free Cloudflare quick tunnel to the guest port, on
//! one tap from the person's card, for the pub quiz where everyone is on mobile data.
//!
//! - **cloudflared on demand.** One already on this computer (PATH or the usual install places)
//!   is used if it starts. Otherwise the pinned release ([`VERSION`], [`PINS`]) is downloaded from
//!   GitHub only (any other host is refused, redirects stay on GitHub's download hosts), capped in
//!   size, and its SHA-256 is checked before anything is unpacked, written, made executable or run.
//!   It lands in kwant's cache (`<cache>/kwant/cloudflared/<version>/`, beside the bots' browser)
//!   and must answer `--version` once. Any failure is the one plain line [`FAILED_LINE`].
//! - **Guest port only.** `cloudflared tunnel --no-autoupdate --url http://127.0.0.1:<guest port>`
//!   ([`tunnel_args`]); never the main port. The `https://<words>.trycloudflare.com` address is read
//!   from its output within [`URL_TIMEOUT`].
//! - **One per box.** Live pockets that asked share one tunnel ([`Tunnels`]); it stops when the
//!   last of them turns it off, ends, is deleted or expires, when pocket apps are switched off, and
//!   when kwant stops. Nothing is saved: a restart never brings it back (the address would change).
//! - **Mac keep-awake.** On macOS, while a tunnel or any open pocket runs, kwant holds
//!   `/usr/bin/caffeinate -i -w <kwnt pid>` ([`KeepAwake`]). Nothing elsewhere.
//!
//! No Cloudflare account, login or named tunnel. Long-lived kits (Secret Santa,
//! [`super::kits::Kit::stable_address_only`]) never get one.

use crate::server::dev::scan::gitleaks::{sha256_hex, tar_entry};
use crate::server::AppState;
use serde_json::{json, Value};
use std::collections::BTreeSet;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Mutex;
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncRead, BufReader};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

/// The cloudflared release kwant downloads. Upgrading means new pins below: the asset digests
/// GitHub lists for the release, and the binary's own SHA-256 from the release notes' table
/// (for the Mac `.tgz` files the notes list the binary inside, not the archive).
pub const VERSION: &str = "2026.11.1";
const RELEASES: &str = "https://github.com/cloudflare/cloudflared/releases/download";
/// The only hosts a download may start on or be redirected to.
const DOWNLOAD_HOSTS: &[&str] = &["github.com", "objects.githubusercontent.com", "release-assets.githubusercontent.com"];

/// What the person sees when the tunnel couldn't be set up or started.
pub const FAILED_LINE: &str = "Couldn't set up join from anywhere. Try again, or use Same Wi-Fi.";
/// What the person sees when a running tunnel stopped by itself.
pub const STOPPED_LINE: &str = "Join from anywhere stopped. Turn it on again for a new link.";
/// The honest limits, shown with a running tunnel.
pub const NOTE: &str = "A free Cloudflare link: a new address each time it's turned on, no uptime promise, meant for small groups.";
/// How long cloudflared has to print its address.
pub const URL_TIMEOUT: Duration = Duration::from_secs(30);
const FETCH_TIMEOUT: Duration = Duration::from_secs(180);
const VERSION_TIMEOUT: Duration = Duration::from_secs(15);
/// The biggest asset is the Windows `.exe` (about 55 MB).
const MAX_DOWNLOAD: usize = 96 << 20;
const MAX_UNPACKED: u64 = 128 << 20;

/// One platform's pinned build.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Pin {
    /// `std::env::consts::OS`.
    pub os: &'static str,
    /// `std::env::consts::ARCH`.
    pub arch: &'static str,
    pub asset: &'static str,
    /// SHA-256 of the release asset as downloaded.
    pub asset_sha256: &'static str,
    /// SHA-256 of the `cloudflared` binary (the same as the asset unless it is a `.tgz`).
    pub binary_sha256: &'static str,
}

impl Pin {
    pub fn archived(&self) -> bool {
        self.asset.ends_with(".tgz")
    }
}

pub const PINS: [Pin; 5] = [
    Pin {
        os: "macos",
        arch: "aarch64",
        asset: "cloudflared-darwin-arm64.tgz",
        asset_sha256: "02fe4bcf08f2c09699cff30658228a739a0c47fb75f3653d1b8772a27ade4462",
        binary_sha256: "1cd7041838e5b0943378763ca86ddc0af896d1727875f36b29f5cc35eb2c4df7",
    },
    Pin {
        os: "macos",
        arch: "x86_64",
        asset: "cloudflared-darwin-amd64.tgz",
        asset_sha256: "5e0403b188dfee71fbd9b995e7dc9910d61b661c52371935ab6475d41185632f",
        binary_sha256: "52686df40889be19803c368470bd46d5228737462f10a40380bedb6af8ed49a2",
    },
    Pin {
        os: "linux",
        arch: "x86_64",
        asset: "cloudflared-linux-amd64",
        asset_sha256: "97bb3b02d23a5980fc9c15a1a270e692da1923f103f920d5b16d4931d5ab55e7",
        binary_sha256: "97bb3b02d23a5980fc9c15a1a270e692da1923f103f920d5b16d4931d5ab55e7",
    },
    Pin {
        os: "linux",
        arch: "aarch64",
        asset: "cloudflared-linux-arm64",
        asset_sha256: "b155cec3e48e489cfb8e5c602769fb8c771083018d8881d68b4d18a791567c5a",
        binary_sha256: "b155cec3e48e489cfb8e5c602769fb8c771083018d8881d68b4d18a791567c5a",
    },
    Pin {
        os: "windows",
        arch: "x86_64",
        asset: "cloudflared-windows-amd64.exe",
        asset_sha256: "341deda3db5ec498a0482254d6c38c0b7cf01a30c37bb5dd95d2216d08955e35",
        binary_sha256: "341deda3db5ec498a0482254d6c38c0b7cf01a30c37bb5dd95d2216d08955e35",
    },
];

/// Where a download comes from and what it must hash to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Source {
    pub url: String,
    pub archived: bool,
    pub asset_sha256: String,
    pub binary_sha256: String,
}

impl Source {
    pub fn for_platform(os: &str, arch: &str) -> Option<Source> {
        let p = PINS.iter().find(|p| p.os == os && p.arch == arch)?;
        Some(Source { url: format!("{RELEASES}/{VERSION}/{}", p.asset), archived: p.archived(), asset_sha256: p.asset_sha256.into(), binary_sha256: p.binary_sha256.into() })
    }
}

/// A download may start only at cloudflared's own GitHub releases, over https.
pub fn allowed_url(url: &str) -> bool {
    let Ok(u) = reqwest::Url::parse(url) else { return false };
    u.scheme() == "https"
        && u.host_str() == Some("github.com")
        && u.port().is_none()
        && u.username().is_empty()
        && u.password().is_none()
        && u.query().is_none()
        && u.fragment().is_none()
        && u.path().starts_with("/cloudflare/cloudflared/releases/download/")
        && !url.contains("..")
}

fn allowed_hop(u: &reqwest::Url) -> bool {
    u.scheme() == "https" && u.username().is_empty() && u.password().is_none() && u.host_str().is_some_and(|h| DOWNLOAD_HOSTS.contains(&h))
}

/// The `cloudflared` binary out of a downloaded asset, after its checksum: the asset's own
/// SHA-256 first (nothing is unpacked before it matches), then the binary's.
pub fn verified_binary(asset: &[u8], src: &Source) -> Result<Vec<u8>, String> {
    if sha256_hex(asset) != src.asset_sha256 {
        return Err("the cloudflared download didn't match its pinned checksum".into());
    }
    let binary = if src.archived {
        let mut tar = Vec::new();
        flate2::read::GzDecoder::new(asset).take(MAX_UNPACKED).read_to_end(&mut tar).map_err(|_| "the cloudflared download couldn't be unpacked".to_string())?;
        tar_entry(&tar, "cloudflared").ok_or("the cloudflared download had no cloudflared inside")?
    } else {
        asset.to_vec()
    };
    if sha256_hex(&binary) != src.binary_sha256 {
        return Err("the cloudflared binary didn't match its pinned checksum".into());
    }
    Ok(binary)
}

fn binary_name() -> &'static str {
    if cfg!(windows) {
        "cloudflared.exe"
    } else {
        "cloudflared"
    }
}

/// kwant's cloudflared cache for `home`: beside the bots' browser cache.
pub fn cache_dir(os: &str, home: &Path) -> PathBuf {
    let browser = crate::sandbox::chrome_download::cache_root(os, home);
    browser.parent().map(Path::to_path_buf).unwrap_or(browser).join("cloudflared").join(VERSION)
}

/// Checks `asset` and only then writes the binary into `dir`, makes it executable and runs
/// `--version` once. A binary that doesn't start is removed again.
pub async fn install(dir: &Path, src: &Source, asset: &[u8]) -> Result<PathBuf, String> {
    let binary = verified_binary(asset, src)?;
    let bin = dir.join(binary_name());
    let write = async {
        tokio::fs::create_dir_all(dir).await?;
        let tmp = dir.join(format!(".cloudflared.part-{}", std::process::id()));
        tokio::fs::write(&tmp, &binary).await?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            tokio::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o755)).await?;
        }
        tokio::fs::rename(&tmp, &bin).await
    };
    write.await.map_err(|_| "cloudflared couldn't be saved in kwant's cache folder".to_string())?;
    if let Err(why) = version(&bin).await {
        let _ = tokio::fs::remove_file(&bin).await;
        return Err(why);
    }
    Ok(bin)
}

/// `cloudflared --version`: its first line, if it answers like cloudflared.
async fn version(bin: &Path) -> Result<String, String> {
    let mut tries = 0;
    let out = loop {
        let run = tokio::process::Command::new(bin).arg("--version").stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::null()).kill_on_drop(true).output();
        match tokio::time::timeout(VERSION_TIMEOUT, run).await {
            Ok(Ok(out)) if out.status.success() => break out,
            // ETXTBSY: a process forked elsewhere still has the just-written file open
            Ok(Err(e)) if e.raw_os_error() == Some(26) && tries < 20 => {
                tries += 1;
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
            _ => return Err("cloudflared didn't start".into()),
        }
    };
    let line = String::from_utf8_lossy(&out.stdout).lines().next().unwrap_or("").trim().to_string();
    if line.starts_with("cloudflared version") {
        Ok(line)
    } else {
        Err("cloudflared didn't start".into())
    }
}

async fn fetch(url: &str) -> Result<Vec<u8>, String> {
    if !allowed_url(url) {
        return Err("the cloudflared download isn't on cloudflared's GitHub releases, so it wasn't fetched".into());
    }
    let policy = reqwest::redirect::Policy::custom(|a| if a.previous().len() >= 5 || !allowed_hop(a.url()) { a.stop() } else { a.follow() });
    let http = reqwest::Client::builder()
        .redirect(policy)
        .connect_timeout(Duration::from_secs(15))
        .timeout(FETCH_TIMEOUT)
        .user_agent(concat!("kwnt/", env!("CARGO_PKG_VERSION")))
        .build()
        .map_err(|_| "the cloudflared download couldn't start".to_string())?;
    let mut res = http.get(url).send().await.map_err(|_| "couldn't reach GitHub to download cloudflared (offline?)".to_string())?;
    if res.status().as_u16() != 200 {
        return Err(format!("the cloudflared download answered HTTP {}", res.status().as_u16()));
    }
    if res.content_length().is_some_and(|n| n > MAX_DOWNLOAD as u64) {
        return Err("the cloudflared download was larger than expected".into());
    }
    let mut out = Vec::new();
    while let Some(chunk) = res.chunk().await.map_err(|_| "the cloudflared download broke off".to_string())? {
        if out.len() + chunk.len() > MAX_DOWNLOAD {
            return Err("the cloudflared download was larger than expected".into());
        }
        out.extend_from_slice(&chunk);
    }
    Ok(out)
}

/// cloudflared already on this computer: PATH, then the usual install places.
fn installed_candidates() -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = std::env::var_os("PATH").map(|p| std::env::split_paths(&p).map(|d| d.join(binary_name())).collect()).unwrap_or_default();
    let usual: &[&str] = if cfg!(windows) {
        &[r"C:\Program Files (x86)\cloudflared\cloudflared.exe", r"C:\Program Files\cloudflared\cloudflared.exe"]
    } else {
        &["/opt/homebrew/bin/cloudflared", "/usr/local/bin/cloudflared", "/usr/bin/cloudflared"]
    };
    out.extend(usual.iter().map(PathBuf::from));
    out.retain(|p| p.is_file());
    out.dedup();
    out
}

/// The cloudflared to run: a fake in tests, one already installed, the pinned one kwant
/// downloaded earlier (its checksum checked again), or a fresh download.
async fn ensure(data_dir: &Path) -> Result<PathBuf, String> {
    #[cfg(feature = "test-seams")]
    {
        let fake = data_dir.join(".test-cloudflared");
        if fake.is_file() {
            return version(&fake).await.map(|_| fake);
        }
    }
    #[cfg(not(feature = "test-seams"))]
    let _ = data_dir;
    for bin in installed_candidates() {
        if version(&bin).await.is_ok() {
            return Ok(bin);
        }
    }
    let src = Source::for_platform(std::env::consts::OS, std::env::consts::ARCH).ok_or("there's no cloudflared build for this computer")?;
    let home = crate::sandbox::chrome_download::browser_home().ok_or("there's no home folder for kwant's cache")?;
    let dir = cache_dir(std::env::consts::OS, &home);
    let bin = dir.join(binary_name());
    if let Ok(bytes) = tokio::fs::read(&bin).await {
        let want = src.binary_sha256.clone();
        if tokio::task::spawn_blocking(move || sha256_hex(&bytes) == want).await.unwrap_or(false) {
            return Ok(bin);
        }
        tracing::warn!("pocket apps: the cached cloudflared didn't match its checksum; fetching it again");
        let _ = tokio::fs::remove_file(&bin).await;
    }
    tracing::info!(version = VERSION, "pocket apps: downloading cloudflared for join from anywhere");
    let asset = fetch(&src.url).await?;
    install(&dir, &src, &asset).await
}

/// The command line: a quick tunnel to the guest port, and nothing else.
pub fn tunnel_args(guest_port: u16) -> Vec<String> {
    vec!["tunnel".into(), "--no-autoupdate".into(), "--url".into(), format!("http://127.0.0.1:{guest_port}")]
}

/// The quick tunnel's address in one line of cloudflared's output: `https://<words>.trycloudflare.com`.
/// `api.trycloudflare.com` (named in its errors) and the bare domain are not addresses.
pub fn quick_url(line: &str) -> Option<String> {
    const SUFFIX: &str = ".trycloudflare.com";
    let mut rest = line;
    while let Some(i) = rest.find("https://") {
        let after = &rest[i + "https://".len()..];
        let host: String = after.chars().take_while(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '.').collect();
        let next = after[host.len()..].chars().next();
        if let Some(label) = host.to_ascii_lowercase().strip_suffix(SUFFIX) {
            let ok = !label.is_empty()
                && label.len() <= 63
                && label.contains('-')
                && !label.starts_with('-')
                && !label.ends_with('-')
                && label.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-');
            if ok && next.is_none_or(|c| c.is_whitespace() || c == '|' || c == '/') {
                return Some(format!("https://{label}{SUFFIX}"));
            }
        }
        rest = after;
    }
    None
}

/// The first quick-tunnel address among `lines`, or `None` once they end or `timeout` passes.
pub async fn wait_for_url(lines: &mut mpsc::Receiver<String>, timeout: Duration) -> Option<String> {
    tokio::time::timeout(timeout, async {
        while let Some(line) = lines.recv().await {
            if let Some(url) = quick_url(&line) {
                return Some(url);
            }
        }
        None
    })
    .await
    .ok()
    .flatten()
}

/// Reads `from` line by line into `to` until it ends; keeps draining after nobody listens, so
/// cloudflared never blocks on a full pipe.
fn forward(from: impl AsyncRead + Unpin + Send + 'static, to: mpsc::Sender<String>) {
    tokio::spawn(async move {
        let mut lines = BufReader::new(from).lines();
        while let Ok(Some(line)) = lines.next_line().await {
            if !to.is_closed() {
                let _ = to.try_send(line);
            }
        }
    });
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TunnelState {
    Off,
    Starting,
    On,
    Failed,
}

impl TunnelState {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Off => "off",
            Self::Starting => "starting",
            Self::On => "on",
            Self::Failed => "failed",
        }
    }
}

struct Inner {
    state: TunnelState,
    url: Option<String>,
    /// The line for `failed`.
    failure: Option<&'static str>,
    /// Pockets using the tunnel (asked for it and still live).
    users: BTreeSet<String>,
    /// Pockets that asked when it last failed: they see `failed` until they ask again.
    failed_for: BTreeSet<String>,
    run: Option<Run>,
    /// Which start a background task belongs to; an older task changes nothing.
    round: u64,
}

struct Run {
    cancel: CancellationToken,
    /// cloudflared's pid, to stop it at once on unix.
    #[cfg_attr(not(unix), allow(dead_code))]
    pid: Option<u32>,
}

impl Default for Inner {
    fn default() -> Self {
        Self { state: TunnelState::Off, url: None, failure: None, users: BTreeSet::new(), failed_for: BTreeSet::new(), run: None, round: 0 }
    }
}

/// The box's one quick tunnel, shared by the live pockets that asked for it. In memory only.
#[derive(Default)]
pub struct Tunnels {
    inner: Mutex<Inner>,
}

impl Tunnels {
    fn lock(&self) -> std::sync::MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Starting or running.
    pub fn active(&self) -> bool {
        matches!(self.lock().state, TunnelState::Starting | TunnelState::On)
    }

    /// The running tunnel's address (`https://<words>.trycloudflare.com`) if `code` uses it.
    pub fn url_for(&self, code: &str) -> Option<String> {
        let g = self.lock();
        (g.state == TunnelState::On && g.users.contains(code)).then(|| g.url.clone()).flatten()
    }

    /// `{state, url, note}` as this pocket sees it.
    pub fn view(&self, code: &str) -> Value {
        let g = self.lock();
        let (state, url, note) = if g.users.contains(code) {
            match g.state {
                TunnelState::On => (TunnelState::On, g.url.clone(), Some(NOTE)),
                TunnelState::Starting => (TunnelState::Starting, None, Some(NOTE)),
                other => (other, None, g.failure),
            }
        } else if g.state == TunnelState::Failed && g.failed_for.contains(code) {
            (TunnelState::Failed, None, g.failure)
        } else {
            (TunnelState::Off, None, None)
        };
        json!({ "state": state.as_str(), "url": url, "note": note })
    }

    /// Adds `code` as a user; returns the start's generation when a new start is needed.
    fn want(&self, code: &str) -> Option<(u64, CancellationToken)> {
        let mut g = self.lock();
        g.users.insert(code.to_string());
        g.failed_for.remove(code);
        if matches!(g.state, TunnelState::Starting | TunnelState::On) {
            return None;
        }
        g.round += 1;
        g.state = TunnelState::Starting;
        g.url = None;
        g.failure = None;
        g.failed_for.clear();
        let cancel = CancellationToken::new();
        g.run = Some(Run { cancel: cancel.clone(), pid: None });
        Some((g.round, cancel))
    }

    /// `code` stops using the tunnel (turned off, ended, deleted, expired). The last user stops it.
    pub fn release(&self, code: &str) {
        let mut g = self.lock();
        g.failed_for.remove(code);
        if g.users.remove(code) && g.users.is_empty() {
            Self::stop_locked(&mut g);
        }
    }

    /// Stops the tunnel for everyone (pocket apps switched off, kwant stopping).
    pub fn stop_all(&self) {
        let mut g = self.lock();
        g.users.clear();
        g.failed_for.clear();
        Self::stop_locked(&mut g);
    }

    fn stop_locked(g: &mut Inner) {
        if let Some(run) = g.run.take() {
            run.cancel.cancel();
            // At once, too: kwant may be exiting before the task gets to run
            #[cfg(unix)]
            {
                if let Some(pid) = run.pid.and_then(|p| i32::try_from(p).ok()) {
                    // SAFETY: plain kill(2) on our own child, not yet reaped (the task reaps it)
                    unsafe {
                        libc::kill(pid, libc::SIGTERM);
                    }
                }
            }
        }
        g.round += 1;
        g.state = TunnelState::Off;
        g.url = None;
        g.failure = None;
    }

    fn started(&self, round: u64, pid: Option<u32>) -> bool {
        let mut g = self.lock();
        if g.round != round {
            return false;
        }
        match g.run.as_mut() {
            Some(run) => {
                run.pid = pid;
                true
            }
            None => false,
        }
    }

    fn on(&self, round: u64, url: String) {
        let mut g = self.lock();
        if g.round == round && g.state == TunnelState::Starting {
            g.state = TunnelState::On;
            g.url = Some(url);
        }
    }

    fn failed(&self, round: u64, line: &'static str) {
        let mut g = self.lock();
        if g.round != round {
            return;
        }
        g.run = None;
        g.state = TunnelState::Failed;
        g.url = None;
        g.failure = Some(line);
        g.failed_for = std::mem::take(&mut g.users);
    }
}

/// Why a pocket can't have a quick tunnel, as `(error code, line)`, or `None` when it can.
pub fn refusal(s: &super::Snapshot) -> Option<(&'static str, &'static str)> {
    if s.kit().is_none_or(|k| k.stable_address_only()) {
        return Some(("no_quick_tunnel", "This pocket runs for weeks, so it needs an address that stays the same. A quick link isn't offered for it."));
    }
    if s.row.state != crate::db::pocket::OPEN {
        return Some(("pocket_ended", "This pocket has ended."));
    }
    None
}

/// Turns the quick tunnel on for this pocket: joins the running one, or starts it in the
/// background (the card reads `join.tunnel.state` until it is `on` or `failed`).
pub fn turn_on(state: &AppState, code: &str, guest_port: u16) {
    let Some((round, cancel)) = state.pocket.tunnel.want(code) else { return };
    let state = state.clone();
    tokio::spawn(async move { run(state, round, guest_port, cancel).await });
}

async fn run(state: AppState, round: u64, guest_port: u16, cancel: CancellationToken) {
    let tunnels = &state.pocket.tunnel;
    let bin = match ensure(&state.config.data_dir).await {
        Ok(bin) => bin,
        Err(why) => {
            tracing::warn!(reason = %why, "pocket apps: join from anywhere couldn't be set up");
            tunnels.failed(round, FAILED_LINE);
            return;
        }
    };
    if cancel.is_cancelled() {
        return;
    }
    let mut cmd = tokio::process::Command::new(&bin);
    cmd.args(tunnel_args(guest_port)).stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped()).kill_on_drop(true);
    // Settings from the environment could point it somewhere else
    for (k, _) in std::env::vars_os() {
        if k.to_string_lossy().starts_with("TUNNEL_") {
            cmd.env_remove(k);
        }
    }
    let mut child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) => {
            tracing::warn!("pocket apps: cloudflared didn't start: {e}");
            tunnels.failed(round, FAILED_LINE);
            return;
        }
    };
    if !tunnels.started(round, child.id()) {
        let _ = child.kill().await;
        return;
    }
    let (tx, mut rx) = mpsc::channel(256);
    if let Some(out) = child.stdout.take() {
        forward(out, tx.clone());
    }
    if let Some(err) = child.stderr.take() {
        forward(err, tx);
    }
    tokio::select! {
        _ = cancel.cancelled() => {
            let _ = child.kill().await;
            return;
        }
        url = wait_for_url(&mut rx, URL_TIMEOUT) => match url {
            Some(url) => {
                tracing::info!("pocket apps: join from anywhere is on");
                tunnels.on(round, url);
            }
            None => {
                tracing::warn!("pocket apps: cloudflared gave no quick tunnel address");
                tunnels.failed(round, FAILED_LINE);
                let _ = child.kill().await;
                return;
            }
        }
    }
    drop(rx);
    tokio::select! {
        _ = cancel.cancelled() => {
            let _ = child.kill().await;
            tracing::info!("pocket apps: join from anywhere stopped");
        }
        _ = child.wait() => {
            tracing::warn!("pocket apps: cloudflared stopped by itself");
            tunnels.failed(round, STOPPED_LINE);
        }
    }
}

// ---- Mac keep-awake ----

/// The command that keeps this computer awake while pockets are live: macOS only.
pub fn keep_awake_command(os: &str, pid: u32) -> Option<(&'static str, Vec<String>)> {
    (os == "macos").then(|| ("/usr/bin/caffeinate", vec!["-i".to_string(), "-w".to_string(), pid.to_string()]))
}

/// `caffeinate` held while wanted; `-w <kwnt pid>` lets it go by itself if kwant dies.
#[derive(Default)]
pub struct KeepAwake {
    child: Mutex<Option<tokio::process::Child>>,
}

impl KeepAwake {
    pub fn set(&self, want: bool) {
        let mut child = self.child.lock().unwrap_or_else(|e| e.into_inner());
        if !want {
            // kill_on_drop
            *child = None;
            return;
        }
        if child.as_mut().is_some_and(|c| matches!(c.try_wait(), Ok(None))) {
            return;
        }
        *child = None;
        let Some((awake_bin, args)) = keep_awake_command(std::env::consts::OS, std::process::id()) else { return };
        match tokio::process::Command::new(awake_bin).args(args).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).kill_on_drop(true).spawn() {
            Ok(c) => *child = Some(c),
            Err(e) => tracing::warn!("pocket apps: couldn't keep the Mac awake: {e}"),
        }
    }
}

/// Holds the Mac awake while a tunnel or any open pocket runs (the supervisor calls this).
pub async fn keep_awake(state: &AppState, on: bool) {
    if keep_awake_command(std::env::consts::OS, 0).is_none() {
        return;
    }
    let want = on && (state.pocket.tunnel.active() || !state.db.pocket_codes_open().await.unwrap_or_default().is_empty());
    state.pocket.awake.set(want);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_pin_is_a_github_release_asset_of_this_version() {
        for p in PINS {
            let src = Source::for_platform(p.os, p.arch).unwrap();
            assert!(allowed_url(&src.url), "{}", src.url);
            assert!(src.url.ends_with(&format!("/{VERSION}/{}", p.asset)), "{}", src.url);
            for sha in [p.asset_sha256, p.binary_sha256] {
                assert!(sha.len() == 64 && sha.chars().all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase()), "{sha}");
            }
            assert_eq!(p.archived(), p.os == "macos");
            assert_eq!(p.asset_sha256 == p.binary_sha256, !p.archived(), "{}", p.asset);
        }
        assert!(Source::for_platform("linux", "x86").is_none());
        assert!(Source::for_platform("freebsd", "x86_64").is_none());
    }

    #[test]
    fn only_cloudflareds_github_releases() {
        for bad in [
            "http://github.com/cloudflare/cloudflared/releases/download/2026.11.1/cloudflared-linux-amd64",
            "https://example.com/cloudflare/cloudflared/releases/download/2026.11.1/cloudflared-linux-amd64",
            "https://github.com.evil.net/cloudflare/cloudflared/releases/download/x/y",
            "https://github.com/someone/cloudflared/releases/download/2026.11.1/cloudflared-linux-amd64",
            "https://github.com/cloudflare/cloudflared/releases/download/../../x",
            "https://user@github.com/cloudflare/cloudflared/releases/download/2026.11.1/y",
            "https://github.com:8443/cloudflare/cloudflared/releases/download/2026.11.1/y",
            "https://github.com/cloudflare/cloudflared/releases/download/2026.11.1/y?x=1",
            "not a url",
        ] {
            assert!(!allowed_url(bad), "{bad}");
        }
        let hop = |u: &str| allowed_hop(&reqwest::Url::parse(u).unwrap());
        assert!(hop("https://release-assets.githubusercontent.com/github-production-release-asset/1"));
        assert!(hop("https://objects.githubusercontent.com/x"));
        assert!(!hop("http://objects.githubusercontent.com/x"));
        assert!(!hop("https://evil.example/x"));
    }

    fn src_for(asset: &[u8], binary: &[u8], archived: bool) -> Source {
        Source { url: format!("{RELEASES}/{VERSION}/x"), archived, asset_sha256: sha256_hex(asset), binary_sha256: sha256_hex(binary) }
    }

    /// A one-file ustar `.tar.gz`, like the Mac assets.
    fn tgz(name: &str, data: &[u8]) -> Vec<u8> {
        use std::io::Write;
        let mut h = [0u8; 512];
        h[..name.len()].copy_from_slice(name.as_bytes());
        h[100..107].copy_from_slice(b"0000755");
        let size = format!("{:011o}", data.len());
        h[124..135].copy_from_slice(size.as_bytes());
        h[156] = b'0';
        h[257..262].copy_from_slice(b"ustar");
        let mut tar = h.to_vec();
        tar.extend_from_slice(data);
        tar.resize(tar.len().div_ceil(512) * 512 + 1024, 0);
        let mut gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
        gz.write_all(&tar).unwrap();
        gz.finish().unwrap()
    }

    #[test]
    fn the_checksums_come_before_anything_is_unpacked() {
        let bin = b"#!/bin/sh\necho 'cloudflared version 2026.11.1'\n".to_vec();
        let asset = tgz("cloudflared", &bin);
        assert_eq!(verified_binary(&asset, &src_for(&asset, &bin, true)).unwrap(), bin);
        assert_eq!(verified_binary(&bin, &src_for(&bin, &bin, false)).unwrap(), bin);
        // A changed asset: refused on the asset's checksum
        let mut tampered = asset.clone();
        let last = tampered.len() - 1;
        tampered[last] ^= 1;
        assert!(verified_binary(&tampered, &src_for(&asset, &bin, true)).unwrap_err().contains("pinned checksum"));
        // The right archive with another binary inside: refused on the binary's checksum
        let other = tgz("cloudflared", b"something else");
        assert!(verified_binary(&other, &src_for(&other, &bin, true)).unwrap_err().contains("binary didn't match"));
        // No cloudflared inside
        let wrong_name = tgz("README", &bin);
        assert!(verified_binary(&wrong_name, &src_for(&wrong_name, &bin, true)).is_err());
    }

    #[tokio::test]
    async fn a_checksum_mismatch_writes_nothing() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("cloudflared").join(VERSION);
        let good = b"#!/bin/sh\necho 'cloudflared version 2026.11.1'\n".to_vec();
        let src = src_for(&good, &good, false);
        let err = install(&dir, &src, b"#!/bin/sh\necho evil\n").await.unwrap_err();
        assert!(err.contains("pinned checksum"), "{err}");
        assert!(!dir.exists(), "nothing written, let alone made executable");
    }

    #[tokio::test]
    async fn a_non_github_url_is_refused_before_any_request() {
        let err = fetch("https://example.com/cloudflared-linux-amd64").await.unwrap_err();
        assert!(err.contains("isn't on cloudflared's GitHub releases"), "{err}");
        let err = fetch("http://127.0.0.1:9/cloudflared").await.unwrap_err();
        assert!(err.contains("isn't on cloudflared's GitHub releases"), "{err}");
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn a_verified_binary_is_installed_executable_and_must_start() {
        use std::os::unix::fs::PermissionsExt;
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("cf");
        let good = b"#!/bin/sh\necho 'cloudflared version 2026.11.1 (built 2026-11-17-15:04 UTC)'\n".to_vec();
        let bin = install(&dir, &src_for(&good, &good, false), &good).await.unwrap();
        assert_eq!(bin, dir.join("cloudflared"));
        assert_eq!(std::fs::metadata(&bin).unwrap().permissions().mode() & 0o777, 0o755);
        // Matches its pin but doesn't answer like cloudflared: removed again
        let dud = b"#!/bin/sh\nexit 3\n".to_vec();
        let dir2 = tmp.path().join("dud");
        assert!(install(&dir2, &src_for(&dud, &dud, false), &dud).await.is_err());
        assert!(!dir2.join("cloudflared").exists());
    }

    #[test]
    fn the_command_targets_only_the_guest_port() {
        assert_eq!(tunnel_args(8788), ["tunnel", "--no-autoupdate", "--url", "http://127.0.0.1:8788"]);
        let args = tunnel_args(9001).join(" ");
        assert!(!args.contains("8787") && !args.contains("--config") && !args.contains("login"), "{args}");
    }

    #[test]
    fn the_address_is_read_from_real_looking_output() {
        let out = "\
2026-11-23T07:32:12Z INF Thank you for trying Cloudflare Tunnel. Doing so, without a Cloudflare account, is a quick way to experiment and try it out. However, be aware that these account-less Tunnels have no uptime guarantee, are subject to the Cloudflare Online Services Terms of Use (https://www.cloudflare.com/website-terms/), and Cloudflare reserves the right to investigate your use of Tunnels for violations of such terms. If you intend to use Tunnels in production you should use a pre-created named tunnel by following: https://developers.cloudflare.com/cloudflare-one/connections/connect-apps
2026-11-23T07:32:12Z INF Requesting new quick Tunnel on trycloudflare.com...
2026-11-23T07:32:18Z INF +--------------------------------------------------------------------------------------------+
2026-11-23T07:32:18Z INF |  Your quick Tunnel has been created! Visit it at (it may take some time to be reachable):  |
2026-11-23T07:32:18Z INF |  https://bowl-cited-than-amplifier.trycloudflare.com                                       |
2026-11-23T07:32:18Z INF +--------------------------------------------------------------------------------------------+";
        let found: Vec<String> = out.lines().filter_map(quick_url).collect();
        assert_eq!(found, ["https://bowl-cited-than-amplifier.trycloudflare.com"]);
        for not in [
            "2026-11-23T07:32:12Z ERR failed to request quick Tunnel: Post \"https://api.trycloudflare.com/tunnel\": dial tcp: no such host",
            "see https://trycloudflare.com for more",
            "https://x.trycloudflare.com.evil.net/",
            "https://bad_name-x.trycloudflare.com",
            "https://-a.trycloudflare.com",
            "http://plain-http.trycloudflare.com",
        ] {
            assert_eq!(quick_url(not), None, "{not}");
        }
        assert_eq!(quick_url("Visit https://Some-Words-Here.trycloudflare.com/").as_deref(), Some("https://some-words-here.trycloudflare.com"));
    }

    #[tokio::test]
    async fn it_gives_up_after_the_timeout_or_when_the_output_ends() {
        let (tx, mut rx) = mpsc::channel(4);
        tx.send("INF Requesting new quick Tunnel on trycloudflare.com...".into()).await.unwrap();
        let started = std::time::Instant::now();
        assert_eq!(wait_for_url(&mut rx, Duration::from_millis(80)).await, None);
        assert!(started.elapsed() >= Duration::from_millis(80));
        drop(tx);
        let (tx, mut rx) = mpsc::channel::<String>(4);
        drop(tx);
        assert_eq!(wait_for_url(&mut rx, Duration::from_secs(30)).await, None, "cloudflared exited: no wait");
        let (tx, mut rx) = mpsc::channel(4);
        tx.send("INF |  https://a-b.trycloudflare.com  |".into()).await.unwrap();
        assert_eq!(wait_for_url(&mut rx, Duration::from_secs(1)).await.as_deref(), Some("https://a-b.trycloudflare.com"));
    }

    #[test]
    fn one_tunnel_shared_until_the_last_user_goes() {
        let t = Tunnels::default();
        let (round, _) = t.want("aaaaaa").expect("the first user starts it");
        assert!(t.want("bbbbbb").is_none(), "the second shares it");
        assert_eq!(t.view("aaaaaa")["state"], "starting");
        t.on(round, "https://a-b.trycloudflare.com".into());
        assert_eq!(t.url_for("bbbbbb").as_deref(), Some("https://a-b.trycloudflare.com"));
        assert_eq!(t.view("cccccc")["state"], "off");
        assert_eq!(t.url_for("cccccc"), None);
        t.release("aaaaaa");
        assert!(t.active());
        assert_eq!(t.view("aaaaaa")["state"], "off");
        t.release("bbbbbb");
        assert!(!t.active());
        // A late answer from the stopped start changes nothing
        t.on(round, "https://late-one.trycloudflare.com".into());
        assert_eq!(t.view("bbbbbb")["state"], "off");
        // A failure shows to the pockets that asked, until they ask again
        let (round, _) = t.want("dddddd").unwrap();
        t.failed(round, FAILED_LINE);
        assert_eq!(t.view("dddddd"), json!({ "state": "failed", "url": null, "note": FAILED_LINE }));
        assert_eq!(t.view("eeeeee")["state"], "off");
        assert!(t.want("dddddd").is_some(), "asking again starts again");
        t.stop_all();
        assert_eq!(t.view("dddddd")["state"], "off");
    }

    #[test]
    fn only_a_mac_is_kept_awake() {
        assert_eq!(keep_awake_command("macos", 4242), Some(("/usr/bin/caffeinate", vec!["-i".to_string(), "-w".to_string(), "4242".to_string()])));
        assert_eq!(keep_awake_command("linux", 4242), None);
        assert_eq!(keep_awake_command("windows", 4242), None);
    }
}
