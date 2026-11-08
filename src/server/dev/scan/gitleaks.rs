//! Gitleaks, pinned: one release, one checksum per shipped platform, fetched on first use into
//! `<data_dir>/tools`, checked before every run, and run with no network over the PR's added lines.
//!
//! The added lines of each file are written at their new-side line numbers (everything else
//! blank) into a private temp tree, so a removed secret can't hit and gitleaks' line numbers are
//! the PR's. Gitleaks reads no config or ignore file from the PR: `.gitleaks*` names are renamed,
//! `--ignore-gitleaks-allow` drops inline allow comments, the ignore path is an empty folder and
//! the environment is cleared. It runs with `--redact`, and only the rule id, file and lines of
//! its report are read.

use super::super::facts::FileDiff;
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::Duration;

/// The gitleaks release kwant runs. Upgrading means new checksums below, from the release's
/// `checksums.txt` and a local check of each archive's `gitleaks` binary.
pub const VERSION: &str = "8.30.1";
const RELEASES: &str = "https://github.com/gitleaks/gitleaks/releases/download";

/// One shipped platform's pinned build.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Pin {
    /// `std::env::consts::OS`.
    pub os: &'static str,
    /// `std::env::consts::ARCH`.
    pub arch: &'static str,
    pub archive: &'static str,
    /// sha256 of the release archive.
    pub archive_sha256: &'static str,
    /// sha256 of the `gitleaks` binary inside it.
    pub binary_sha256: &'static str,
}

pub const PINS: [Pin; 4] = [
    Pin {
        os: "macos",
        arch: "aarch64",
        archive: "gitleaks_8.30.1_darwin_arm64.tar.gz",
        archive_sha256: "b40ab0ae55c505963e365f271a8d3846efbc170aa17f2607f13df610a9aeb6a5",
        binary_sha256: "ba52fb1bfabbcde42f032afad3d6e0b19dff8ed105229a16e7caa338bbc0e84f",
    },
    Pin {
        os: "macos",
        arch: "x86_64",
        archive: "gitleaks_8.30.1_darwin_x64.tar.gz",
        archive_sha256: "dfe101a4db2255fc85120ac7f3d25e4342c3c20cf749f2c20a18081af1952709",
        binary_sha256: "cee01fea7173f1b779dff188e1c26ecbcb4027d394acc573b23aaf0be260e291",
    },
    Pin {
        os: "linux",
        arch: "aarch64",
        archive: "gitleaks_8.30.1_linux_arm64.tar.gz",
        archive_sha256: "e4a487ee7ccd7d3a7f7ec08657610aa3606637dab924210b3aee62570fb4b080",
        binary_sha256: "00e91bbe655bd7c47753e8cfe61cb76ea1a5d7e7702fe161ee40102b46b3823b",
    },
    Pin {
        os: "linux",
        arch: "x86_64",
        archive: "gitleaks_8.30.1_linux_x64.tar.gz",
        archive_sha256: "551f6fc83ea457d62a0d98237cbad105af8d557003051f41f3e7ca7b3f2470eb",
        binary_sha256: "88f91962aa2f93ac6ab281d553b9e125f5197bbbce38f9f2437f7299c32e5509",
    },
];

/// How long one gitleaks run may take before it is killed and the tile reads `not_run`.
pub const RUN_TIMEOUT: Duration = Duration::from_secs(60);
/// How long the first-use download may take.
const FETCH_TIMEOUT: Duration = Duration::from_secs(120);
const MAX_ARCHIVE: usize = 32 << 20;
const MAX_UNPACKED: u64 = 128 << 20;
/// Most bytes written for one scan; a bigger diff reads `not_run`.
const MAX_TREE: usize = 64 << 20;
const MAX_REPORT: u64 = 16 << 20;

/// Where the binary comes from and what it must hash to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Source {
    pub url: String,
    pub archive: &'static str,
    pub archive_sha256: String,
    pub binary_sha256: String,
    /// How long the first-use download may take.
    pub fetch_timeout: Duration,
}

impl Source {
    /// This computer's pinned build, if it is a shipped platform.
    pub fn pinned() -> Option<Source> {
        Source::for_platform(std::env::consts::OS, std::env::consts::ARCH)
    }

    pub fn for_platform(os: &str, arch: &str) -> Option<Source> {
        let p = PINS.iter().find(|p| p.os == os && p.arch == arch)?;
        Some(Source {
            url: Source::url_for(RELEASES, p.archive),
            archive: p.archive,
            archive_sha256: p.archive_sha256.into(),
            binary_sha256: p.binary_sha256.into(),
            fetch_timeout: FETCH_TIMEOUT,
        })
    }

    pub fn url_for(base: &str, archive: &str) -> String {
        format!("{base}/v{VERSION}/{archive}")
    }
}

pub(crate) fn sha256_hex(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

/// The verified binary's path, fetching it first if needed. `Err` is the tile's reason.
pub async fn ensure(tools_dir: &Path, src: &Source) -> Result<PathBuf, String> {
    static LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());
    let _one_at_a_time = LOCK.lock().await;
    let key: String = src.binary_sha256.chars().take(12).collect();
    let dir = tools_dir.join(format!("gitleaks-{VERSION}-{key}"));
    let bin = dir.join("gitleaks");
    if let Ok(bytes) = tokio::fs::read(&bin).await {
        if sha256_hex(&bytes) == src.binary_sha256 {
            return Ok(bin);
        }
        // Changed on disk since it was fetched: fetch it again rather than run it
        tracing::warn!("dev scan: the cached gitleaks didn't match its checksum; fetching it again");
        let _ = tokio::fs::remove_file(&bin).await;
    }
    let archive = fetch(&src.url, src.fetch_timeout).await?;
    if sha256_hex(&archive) != src.archive_sha256 {
        return Err("the gitleaks download didn't match its pinned checksum, so it wasn't run".into());
    }
    let binary = tokio::task::spawn_blocking(move || unpack(&archive))
        .await
        .map_err(|_| "the gitleaks download couldn't be unpacked".to_string())??;
    if sha256_hex(&binary) != src.binary_sha256 {
        return Err("the gitleaks binary didn't match its pinned checksum, so it wasn't run".into());
    }
    install(&dir, &bin, &binary).await.map_err(|_| "gitleaks couldn't be saved in kwant's data folder".to_string())?;
    Ok(bin)
}

async fn fetch(url: &str, timeout: Duration) -> Result<Vec<u8>, String> {
    let policy = reqwest::redirect::Policy::custom(|a| {
        if a.previous().len() >= 5 || a.url().scheme() != "https" {
            a.stop()
        } else {
            a.follow()
        }
    });
    let http = reqwest::Client::builder()
        .redirect(policy)
        .connect_timeout(timeout.min(Duration::from_secs(15)))
        .timeout(timeout)
        .user_agent("kwant-dev-scan")
        .build()
        .map_err(|_| "the gitleaks download couldn't start".to_string())?;
    let failed = |e: reqwest::Error| {
        if e.is_timeout() {
            "the gitleaks download timed out".to_string()
        } else {
            "the gitleaks download failed: the release couldn't be reached".to_string()
        }
    };
    let mut res = http.get(url).send().await.map_err(failed)?;
    if res.status().as_u16() != 200 {
        return Err(format!("the gitleaks download failed (HTTP {})", res.status().as_u16()));
    }
    let mut out = Vec::new();
    while let Some(chunk) = res.chunk().await.map_err(failed)? {
        if out.len() + chunk.len() > MAX_ARCHIVE {
            return Err("the gitleaks download was larger than expected, so it wasn't used".into());
        }
        out.extend_from_slice(&chunk);
    }
    Ok(out)
}

/// The `gitleaks` file out of the release's `.tar.gz`.
pub(crate) fn unpack(archive: &[u8]) -> Result<Vec<u8>, String> {
    let mut tar = Vec::new();
    flate2::read::GzDecoder::new(archive)
        .take(MAX_UNPACKED)
        .read_to_end(&mut tar)
        .map_err(|_| "the gitleaks download couldn't be unpacked".to_string())?;
    tar_entry(&tar, "gitleaks").ok_or_else(|| "the gitleaks download had no gitleaks binary in it".to_string())
}

/// A regular file named `want` from a ustar archive.
pub(crate) fn tar_entry(tar: &[u8], want: &str) -> Option<Vec<u8>> {
    let field = |b: &[u8]| -> String {
        let end = b.iter().position(|&c| c == 0).unwrap_or(b.len());
        String::from_utf8_lossy(&b[..end]).into_owned()
    };
    let mut off = 0usize;
    while off + 512 <= tar.len() {
        let h = &tar[off..off + 512];
        if h.iter().all(|&b| b == 0) {
            return None;
        }
        let name = field(&h[0..100]);
        let prefix = if &h[257..262] == b"ustar" { field(&h[345..500]) } else { String::new() };
        let full = if prefix.is_empty() { name } else { format!("{prefix}/{name}") };
        let size_txt = field(&h[124..136]);
        let size = usize::from_str_radix(size_txt.trim(), 8).ok()?;
        let data = off + 512;
        let end = data.checked_add(size)?;
        if end > tar.len() {
            return None;
        }
        let regular = h[156] == b'0' || h[156] == 0;
        if regular && full.trim_start_matches("./") == want {
            return Some(tar[data..end].to_vec());
        }
        off = data + size.div_ceil(512) * 512;
    }
    None
}

async fn install(dir: &Path, bin: &Path, bytes: &[u8]) -> std::io::Result<()> {
    tokio::fs::create_dir_all(dir).await?;
    let tmp = dir.join(format!("gitleaks.part-{}", std::process::id()));
    tokio::fs::write(&tmp, bytes).await?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        tokio::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o755)).await?;
    }
    tokio::fs::rename(&tmp, bin).await
}

// ---------------------------------------------------------------------------------------------
// What gets scanned

/// A file's added lines at their new-side line numbers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Target {
    pub path: String,
    pub lines: Vec<(u64, String)>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Targets {
    pub files: Vec<Target>,
    /// Files with added text that the scanner had no text for.
    pub unread: Vec<String>,
}

/// Lock files: hashes and versions only, and often too big for GitHub to send a diff.
const LOCKFILES: &[&str] = &[
    "Cargo.lock",
    "package-lock.json",
    "npm-shrinkwrap.json",
    "yarn.lock",
    "pnpm-lock.yaml",
    "bun.lockb",
    "bun.lock",
    "poetry.lock",
    "Pipfile.lock",
    "uv.lock",
    "pdm.lock",
    "go.sum",
    "composer.lock",
    "Gemfile.lock",
    "flake.lock",
];

const BINARY_EXT: &[&str] = &[
    "png", "jpg", "jpeg", "gif", "webp", "ico", "icns", "bmp", "tiff", "avif", "heic", "pdf", "zip", "gz", "tgz", "xz", "bz2", "zst",
    "7z", "jar", "woff", "woff2", "ttf", "otf", "eot", "mp3", "mp4", "m4a", "mov", "wav", "ogg", "webm", "flac", "wasm", "so", "dylib",
    "dll", "exe", "a", "o", "rlib", "bin", "dmg", "pkg", "deb", "rpm", "sqlite", "db",
];

fn base_name(path: &str) -> &str {
    path.rsplit('/').next().unwrap_or(path)
}

/// A file GitHub sent no patch for, whose added text has to be read some other way.
pub fn needs_text(f: &FileDiff) -> bool {
    if f.status == "removed" || f.additions == 0 {
        return false;
    }
    let name = base_name(&f.path);
    if LOCKFILES.contains(&name) {
        return false;
    }
    let ext = name.rsplit_once('.').map(|(_, e)| e.to_ascii_lowercase()).unwrap_or_default();
    !BINARY_EXT.contains(&ext.as_str())
}

/// The added lines of a unified diff with their new-side line numbers.
pub fn added_lines(patch: &str) -> Vec<(u64, String)> {
    let mut out = Vec::new();
    let mut n = 0u64;
    for line in patch.split('\n') {
        if let Some(start) = super::hunk_new_start(line) {
            n = start;
            continue;
        }
        match line.as_bytes().first() {
            Some(b'+') => {
                out.push((n, line[1..].trim_end_matches('\r').to_string()));
                n += 1;
            }
            Some(b'-') | Some(b'\\') => {}
            _ => n += 1,
        }
    }
    out
}

/// What gitleaks reads: every added line of every file, with the head text standing in for a
/// file GitHub sent no patch for.
pub fn targets(files: &[FileDiff], contents: &BTreeMap<String, String>) -> Targets {
    let mut t = Targets::default();
    for f in files.iter().filter(|f| f.status != "removed") {
        let lines = match (&f.patch, contents.get(&f.path)) {
            (Some(p), _) => added_lines(p),
            (None, Some(text)) if needs_text(f) => text.split('\n').enumerate().map(|(i, l)| (i as u64 + 1, l.to_string())).collect(),
            (None, _) if needs_text(f) => {
                t.unread.push(f.path.clone());
                continue;
            }
            (None, _) => continue,
        };
        if !lines.is_empty() {
            t.files.push(Target { path: f.path.clone(), lines });
        }
    }
    t
}

// ---------------------------------------------------------------------------------------------
// Running it

/// One gitleaks finding: the rule and where. Never the value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Hit {
    pub rule: String,
    pub file: String,
    pub start: u64,
    pub end: u64,
}

/// A temp folder removed when dropped.
struct TempDir(PathBuf);

impl TempDir {
    fn new() -> std::io::Result<TempDir> {
        use std::sync::atomic::{AtomicU64, Ordering};
        static N: AtomicU64 = AtomicU64::new(0);
        let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
        let p = std::env::temp_dir().join(format!("kwant-scan-{}-{nanos}-{}", std::process::id(), N.fetch_add(1, Ordering::Relaxed)));
        #[cfg_attr(not(unix), allow(unused_mut))]
        let mut b = std::fs::DirBuilder::new();
        #[cfg(unix)]
        {
            use std::os::unix::fs::DirBuilderExt;
            b.mode(0o700);
        }
        b.create(&p)?;
        Ok(TempDir(p))
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// `<i>/<path>` with `..`, `.` and empty parts dropped and `.gitleaks*` names renamed, so a PR
/// can't write outside the tree or hand gitleaks its own config or ignore list.
fn safe_rel(i: usize, path: &str) -> PathBuf {
    let mut p = PathBuf::from(i.to_string());
    for part in path.split(['/', '\\']) {
        if part.is_empty() || part == "." || part == ".." {
            continue;
        }
        let part: String = part.chars().filter(|c| *c != '\0').collect();
        if part.to_ascii_lowercase().starts_with(".gitleaks") {
            p.push(format!("_{part}"));
        } else {
            p.push(part);
        }
    }
    if p.components().count() == 1 {
        p.push("file");
    }
    p
}

/// Where file `i` goes in the tree: under `r/` at its own (made safe) path, so rules that look at
/// the path still apply, and under `n/` as `<i>.txt`, a name gitleaks' built-in path allowlist
/// (`node_modules/`, lockfiles, images, `gitleaks.toml`, …) never matches, so no added line goes
/// unscanned because of where it lives. Both copies map back to `files[i]`.
fn tree_paths(i: usize, path: &str) -> [PathBuf; 2] {
    [Path::new("r").join(safe_rel(i, path)), Path::new("n").join(format!("{i}.txt"))]
}

/// Writes the targets under `root` (see [`tree_paths`]).
fn write_tree(root: &Path, files: &[Target]) -> Result<(), String> {
    let mut total = 0usize;
    for (i, f) in files.iter().enumerate() {
        let paths = tree_paths(i, &f.path);
        let max = f.lines.iter().map(|(n, _)| *n).max().unwrap_or(0) as usize;
        let mut rows = vec![""; max];
        for (n, text) in &f.lines {
            if *n >= 1 {
                rows[*n as usize - 1] = text.as_str();
            }
        }
        let body = rows.join("\n") + "\n";
        total += body.len();
        if total > MAX_TREE {
            return Err("the diff is too large to scan (over 64 MB of added lines)".into());
        }
        for rel in paths {
            let path = root.join(rel);
            if let Some(dir) = path.parent() {
                std::fs::create_dir_all(dir).map_err(|_| "the scan's temp folder couldn't be written".to_string())?;
            }
            std::fs::write(&path, &body).map_err(|_| "the scan's temp folder couldn't be written".to_string())?;
        }
    }
    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Jail {
    /// Linux: a new user and network namespace with no interfaces but loopback.
    Unshare,
    /// macOS: a Seatbelt profile that denies all network.
    Seatbelt,
    /// Neither is available: the environment points every proxy at a closed port.
    EnvOnly,
}

const SEATBELT: &str = "(version 1)(allow default)(deny network*)";

async fn jail() -> Jail {
    static J: tokio::sync::OnceCell<Jail> = tokio::sync::OnceCell::const_new();
    *J.get_or_init(|| async {
        let probe = |mut c: tokio::process::Command| async move {
            c.stdin(std::process::Stdio::null()).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null()).kill_on_drop(true);
            matches!(tokio::time::timeout(Duration::from_secs(5), c.status()).await, Ok(Ok(s)) if s.success())
        };
        if cfg!(target_os = "linux") {
            let mut c = tokio::process::Command::new("unshare");
            c.args(["-rn", "true"]);
            if probe(c).await {
                return Jail::Unshare;
            }
        }
        if cfg!(target_os = "macos") {
            let mut c = tokio::process::Command::new("/usr/bin/sandbox-exec");
            c.args(["-p", SEATBELT, "/usr/bin/true"]);
            if probe(c).await {
                return Jail::Seatbelt;
            }
        }
        tracing::info!("dev scan: no network sandbox here; gitleaks runs with every proxy pointed at a closed port");
        Jail::EnvOnly
    })
    .await
}

/// Runs gitleaks over `files`. `Err` is the tile's reason; a timeout, a crash or an unreadable
/// report is never a pass.
pub async fn run(bin: &Path, files: &[Target], timeout: Duration) -> Result<Vec<Hit>, String> {
    let tmp = TempDir::new().map_err(|_| "the scan's temp folder couldn't be made".to_string())?;
    let tree = tmp.0.join("tree");
    let ignore = tmp.0.join("ignore");
    let report = tmp.0.join("report.json");
    let (tree2, files2, ignore2) = (tree.clone(), files.to_vec(), ignore.clone());
    tokio::task::spawn_blocking(move || {
        std::fs::create_dir_all(&ignore2).map_err(|_| "the scan's temp folder couldn't be written".to_string())?;
        write_tree(&tree2, &files2)
    })
    .await
    .map_err(|_| "the scan's temp folder couldn't be written".to_string())??;

    let bin_s = bin.to_string_lossy().into_owned();
    let mut args: Vec<String> = vec![
        "dir".into(),
        "tree".into(),
        "--report-format".into(),
        "json".into(),
        "--report-path".into(),
        report.to_string_lossy().into_owned(),
        "--redact".into(),
        "--no-banner".into(),
        "--no-color".into(),
        "--exit-code".into(),
        "0".into(),
        "--log-level".into(),
        "error".into(),
        "--ignore-gitleaks-allow".into(),
        "--gitleaks-ignore-path".into(),
        ignore.to_string_lossy().into_owned(),
    ];
    let mut cmd = match jail().await {
        Jail::Unshare => {
            let mut c = tokio::process::Command::new("unshare");
            args.splice(0..0, ["-rn".to_string(), bin_s]);
            c.args(&args);
            c
        }
        Jail::Seatbelt => {
            let mut c = tokio::process::Command::new("/usr/bin/sandbox-exec");
            args.splice(0..0, ["-p".to_string(), SEATBELT.to_string(), bin_s]);
            c.args(&args);
            c
        }
        Jail::EnvOnly => {
            let mut c = tokio::process::Command::new(bin);
            c.args(&args);
            c
        }
    };
    let closed = "http://127.0.0.1:9";
    cmd.current_dir(&tmp.0)
        .env_clear()
        .env("PATH", "/usr/bin:/bin")
        .env("HOME", &tmp.0)
        .env("TMPDIR", &tmp.0);
    for k in ["HTTP_PROXY", "HTTPS_PROXY", "ALL_PROXY", "http_proxy", "https_proxy", "all_proxy"] {
        cmd.env(k, closed);
    }
    cmd.env("NO_PROXY", "").env("no_proxy", "");
    cmd.stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .kill_on_drop(true);
    let status = match tokio::time::timeout(timeout, async { cmd.spawn()?.wait().await }).await {
        Err(_) => return Err(format!("gitleaks didn't finish within {} seconds", timeout.as_secs().max(1))),
        Ok(Err(_)) => return Err("gitleaks couldn't be started".into()),
        Ok(Ok(s)) => s,
    };
    if !status.success() {
        return Err("gitleaks stopped with an error".into());
    }
    let mut raw = Vec::new();
    std::fs::File::open(&report)
        .and_then(|f| f.take(MAX_REPORT).read_to_end(&mut raw))
        .map_err(|_| "gitleaks wrote no report".to_string())?;
    parse_report(&raw, files).ok_or_else(|| "gitleaks wrote a report kwant couldn't read".to_string())
}

/// The rule, file and lines of each finding; the original path comes back from the tree index.
fn parse_report(raw: &[u8], files: &[Target]) -> Option<Vec<Hit>> {
    #[derive(serde::Deserialize)]
    struct Row {
        #[serde(rename = "RuleID", default)]
        rule: String,
        #[serde(rename = "File", default)]
        file: String,
        #[serde(rename = "StartLine", default)]
        start: u64,
        #[serde(rename = "EndLine", default)]
        end: u64,
    }
    let rows: Vec<Row> = serde_json::from_slice(raw).ok()?;
    let mut hits: Vec<Hit> = rows
        .into_iter()
        .map(|r| {
            let rule: String = r.rule.chars().filter(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_').take(60).collect();
            let rel = r.file.replace('\\', "/");
            let rel = rel.find("tree/").map_or("", |i| &rel[i + 5..]);
            // `r/<i>/<path>` or `n/<i>.txt`: the index is the next part either way
            let idx = match rel.split_once('/') {
                Some(("r", rest)) => rest.split('/').next(),
                Some(("n", rest)) => rest.strip_suffix(".txt"),
                _ => None,
            }
            .and_then(|i| i.parse::<usize>().ok());
            let file = idx.and_then(|i| files.get(i)).map(|t| t.path.clone()).unwrap_or_default();
            Hit { rule: if rule.is_empty() { "unknown".into() } else { rule }, file, start: r.start, end: r.end.max(r.start) }
        })
        .collect();
    hits.sort_by(|a, b| (&a.file, a.start, &a.rule).cmp(&(&b.file, b.start, &b.rule)));
    hits.dedup_by(|a, b| a.file == b.file && a.start == b.start && a.rule == b.rule);
    Some(hits)
}

#[cfg(test)]
pub(crate) fn safe_rel_for_test(i: usize, path: &str) -> PathBuf {
    safe_rel(i, path)
}

#[cfg(test)]
pub(crate) fn tree_paths_for_test(i: usize, path: &str) -> [PathBuf; 2] {
    tree_paths(i, path)
}

#[cfg(test)]
pub(crate) fn parse_report_for_test(raw: &[u8], files: &[Target]) -> Option<Vec<Hit>> {
    parse_report(raw, files)
}
