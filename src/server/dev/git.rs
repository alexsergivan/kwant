//! Git itself (any-git.md G1, G2): Dev mode reads a watched repo's history with git, from a bare
//! mirror kwant keeps per repo under `<data>/dev/<space>/` or straight from a team folder's own
//! `.git` (never its working tree). Any host git can fetch from works: GitHub, GitLab, Gitea, a
//! plain git server, a folder.
//!
//! Every call is the same safe run ([`run`]):
//! - git runs as kwant's own user, so the person's SSH keys and credential helpers work;
//! - a connection's token goes in as an `http.<origin>/.extraHeader` through
//!   `GIT_CONFIG_COUNT`/`GIT_CONFIG_KEY_n`/`GIT_CONFIG_VALUE_n`: never in argv, on disk or in a log;
//! - no prompt (`GIT_TERMINAL_PROMPT=0`, no askpass, its own session so ssh can't open a tty),
//!   no hooks, no fsmonitor, no pager, no external diff or textconv, no signature checks;
//! - only the https, http, ssh and git protocols (file only for a team folder), and no redirect
//!   while a token is set;
//! - `nice 10`, a timeout that kills its whole process group, and capped output;
//! - stderr is only sorted into a [`GitError`], never logged or returned, so a URL in it goes
//!   nowhere.

use super::facts::FileDiff;
use super::source::{Commit, Person};
use chrono::{DateTime, Utc};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};
use tokio::io::{AsyncRead, AsyncReadExt};
use zeroize::Zeroizing;

/// The oldest git kwant runs: `GIT_CONFIG_COUNT` came in 2.31.
pub const MIN_VERSION: (u32, u32) = (2, 31);
/// What the desk says to do when the server has no usable git.
pub const INSTALL_HINT: &str = "install git 2.31 or newer on the server";
/// How long one fetch may take (a first fetch of a big repo is the slow one).
pub const FETCH_TIMEOUT: Duration = Duration::from_secs(300);
/// How long one ls-remote may take.
pub const REMOTE_TIMEOUT: Duration = Duration::from_secs(30);
/// How long one local read may take.
pub const READ_TIMEOUT: Duration = Duration::from_secs(20);
/// Most stdout one call returns; a bigger answer is [`GitError::TooBig`].
pub const MAX_OUT: usize = 16 << 20;
/// Most stderr kept to sort an error by.
const MAX_ERR: usize = 8 << 10;
/// Biggest file [`Repo::file_at`] reads.
pub const MAX_FILE: usize = 1 << 20;
/// Most commits one [`Repo::commits_since`] lists, as GitHub's list.
pub const MAX_COMMITS: usize = crate::zombie::github::MAX_COMMITS;
/// Most files one diff lists, as GitHub's PR files.
pub const MAX_FILES: usize = crate::zombie::github::MAX_PR_FILES;

/// Why git didn't answer. Each says the same fixed words whatever git printed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GitError {
    /// No git on the server, or one older than [`MIN_VERSION`].
    NoGit,
    /// The host refused: no key, no credentials, a bad token, an unknown host key.
    Auth,
    /// No such repo, branch or commit.
    NotFound,
    /// The host couldn't be reached.
    Unreachable,
    Timeout,
    TooBig,
    /// kwant refused the input before running git (a URL or ref it won't pass on).
    Refused,
    Failed,
}

impl std::fmt::Display for GitError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            GitError::NoGit => "git isn't installed on the server: install git 2.31 or newer on the server",
            GitError::Auth => "git was refused: check the server's SSH key or git credentials for this repo",
            GitError::NotFound => "git could not find this repo or branch",
            GitError::Unreachable => "git could not reach the repo's host",
            GitError::Timeout => "git took too long to answer",
            GitError::TooBig => "git's answer was too big to read",
            GitError::Refused => "kwant won't pass that repo address or ref to git",
            GitError::Failed => "git could not read this repo",
        })
    }
}

impl std::error::Error for GitError {}

/// Sorts git's stderr. The text itself goes no further.
fn classify(stderr: &[u8]) -> GitError {
    let e = String::from_utf8_lossy(stderr).to_lowercase();
    let any = |words: &[&str]| words.iter().any(|w| e.contains(w));
    if any(&[
        "permission denied",
        "authentication failed",
        "could not read username",
        "could not read password",
        "terminal prompts disabled",
        "host key verification failed",
        "the requested url returned error: 401",
        "the requested url returned error: 403",
        "invalid credentials",
        "access denied",
    ]) {
        GitError::Auth
    } else if any(&["not found", "does not appear to be a git repository", "couldn't find remote ref", "not a valid", "unknown revision", "bad revision", "invalid object name", "not a git repository", "returned error: 404"]) {
        GitError::NotFound
    } else if any(&["could not resolve host", "connection refused", "connection timed out", "network is unreachable", "failed to connect", "could not connect", "ssl", "tls", "no route to host", "connection reset"]) {
        GitError::Unreachable
    } else {
        GitError::Failed
    }
}

/// An `Authorization` header for one origin, given to git through the environment. Its value is
/// never printed (`Debug` hides it) and is wiped when dropped.
pub struct Auth {
    key: String,
    value: Zeroizing<String>,
}

impl std::fmt::Debug for Auth {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Auth").field("key", &self.key).field("value", &"<hidden>").finish()
    }
}

impl Auth {
    /// `Authorization: <authorization>` for requests to `url`'s origin. `None` unless `url` is
    /// https (or http to this machine, for tests) and the value is one header line.
    pub fn header(url: &str, authorization: Zeroizing<String>) -> Option<Self> {
        let u = url::Url::parse(url).ok()?;
        let host = u.host_str()?;
        let ok = u.scheme() == "https" || (u.scheme() == "http" && crate::zombie::imap::is_loopback(host));
        if !ok || authorization.is_empty() || authorization.chars().any(|c| c.is_control() || !(c.is_ascii_graphic() || c == ' ')) {
            return None;
        }
        let origin = u.origin().ascii_serialization();
        Some(Auth { key: format!("http.{origin}/.extraheader"), value: Zeroizing::new(format!("Authorization: {}", authorization.as_str())) })
    }
}

/// Environment variables git keeps from kwant's own: the person's ssh and config choices. Every
/// other `GIT_*` is dropped (a `GIT_DIR`, `GIT_CONFIG_PARAMETERS` or `GIT_ASKPASS` from kwant's
/// environment must not steer these calls).
const KEEP_ENV: &[&str] = &[
    "GIT_SSH",
    "GIT_SSH_COMMAND",
    "GIT_SSH_VARIANT",
    "GIT_CONFIG_GLOBAL",
    "GIT_CONFIG_SYSTEM",
    "GIT_CONFIG_NOSYSTEM",
    "GIT_EXEC_PATH",
    "GIT_SSL_CAINFO",
    "GIT_SSL_CAPATH",
];

/// Settings every call gets, whatever the repo's own config says.
const SAFE_CONFIG: &[&str] = &[
    "core.fsmonitor=false",
    "core.hooksPath=/dev/null",
    "core.askPass=",
    "core.quotePath=false",
    "core.sshCommand=",
    "log.showSignature=false",
    "gc.autoDetach=false",
    "maintenance.autoDetach=false",
    "fetch.recurseSubmodules=false",
    "submodule.recurse=false",
    "protocol.allow=never",
    "protocol.https.allow=always",
    "protocol.http.allow=always",
    "protocol.ssh.allow=always",
    "protocol.git.allow=always",
];

#[derive(Clone, Copy)]
struct Opts<'a> {
    timeout: Duration,
    max_out: usize,
    auth: Option<&'a Auth>,
    /// A team folder: the file protocol is allowed (git reads its own objects).
    file: bool,
}

impl Opts<'static> {
    fn read() -> Self {
        Opts { timeout: READ_TIMEOUT, max_out: MAX_OUT, auth: None, file: false }
    }
}

/// Runs `git [--git-dir <dir>] <args>` as described in the module docs. Its stdout on success.
async fn run(bin: &str, git_dir: Option<&Path>, args: &[&str], o: Opts<'_>) -> Result<Vec<u8>, GitError> {
    let mut cmd = tokio::process::Command::new(bin);
    if let Some(d) = git_dir {
        cmd.arg("--git-dir").arg(d);
    }
    cmd.arg("--no-pager");
    for c in SAFE_CONFIG {
        cmd.arg("-c").arg(c);
    }
    if o.file {
        cmd.args(["-c", "protocol.file.allow=always"]);
    }
    if o.auth.is_some() {
        // The token goes to this origin only: no redirect elsewhere, no helper asked or told
        cmd.args(["-c", "http.followRedirects=false", "-c", "credential.helper="]);
    } else {
        cmd.args(["-c", "http.followRedirects=initial"]);
    }
    cmd.args(args);
    for (k, _) in std::env::vars_os() {
        if let Some(k) = k.to_str() {
            if k.starts_with("GIT_") && !KEEP_ENV.contains(&k) {
                cmd.env_remove(k);
            }
        }
    }
    cmd.env_remove("SSH_ASKPASS");
    cmd.env("GIT_TERMINAL_PROMPT", "0")
        .env("GIT_OPTIONAL_LOCKS", "0")
        .env("GIT_LITERAL_PATHSPECS", "1")
        .env("GIT_NO_REPLACE_OBJECTS", "1")
        .env("GCM_INTERACTIVE", "never")
        .env("SSH_ASKPASS_REQUIRE", "never")
        .env("LC_ALL", "C");
    match o.auth {
        Some(a) => {
            cmd.env("GIT_CONFIG_COUNT", "1").env("GIT_CONFIG_KEY_0", &a.key).env("GIT_CONFIG_VALUE_0", a.value.as_str());
        }
        None => {
            cmd.env("GIT_CONFIG_COUNT", "0");
        }
    }
    cmd.current_dir(git_dir.filter(|d| d.is_dir()).unwrap_or(Path::new("/")));
    cmd.stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped()).kill_on_drop(true);
    #[cfg(unix)]
    // SAFETY: setsid(2) and nice(2) are async-signal-safe and touch no memory of this process
    unsafe {
        cmd.pre_exec(|| {
            // Its own session: no controlling terminal for ssh to prompt on, and one process
            // group the timeout can kill whole
            libc::setsid();
            libc::nice(10);
            Ok(())
        });
    }
    let mut child = cmd.spawn().map_err(|e| if e.kind() == std::io::ErrorKind::NotFound { GitError::NoGit } else { GitError::Failed })?;
    let mut group = crate::drivers::cli::ProcessGroup::of(&child);
    let (so, se) = (child.stdout.take(), child.stderr.take());
    let work = async {
        let (out, err) = tokio::try_join!(read_capped(so, o.max_out), async { Ok(drain(se, MAX_ERR).await) })?;
        let status = child.wait().await.map_err(|_| GitError::Failed)?;
        Ok::<_, GitError>((out, err, status))
    };
    let (out, err, status) = tokio::time::timeout(o.timeout, work).await.map_err(|_| GitError::Timeout)??;
    group.disarm();
    if status.success() {
        Ok(out)
    } else {
        Err(classify(&err))
    }
}

/// Reads all of `r`, or [`GitError::TooBig`] as soon as it passes `cap`.
async fn read_capped<R: AsyncRead + Unpin>(r: Option<R>, cap: usize) -> Result<Vec<u8>, GitError> {
    let Some(mut r) = r else { return Ok(Vec::new()) };
    let mut out = Vec::new();
    let mut buf = vec![0u8; 64 << 10];
    loop {
        let n = r.read(&mut buf).await.map_err(|_| GitError::Failed)?;
        if n == 0 {
            return Ok(out);
        }
        if out.len() + n > cap {
            return Err(GitError::TooBig);
        }
        out.extend_from_slice(&buf[..n]);
    }
}

/// Reads all of `r` (so git never blocks on a full pipe), keeping the first `keep` bytes.
async fn drain<R: AsyncRead + Unpin>(r: Option<R>, keep: usize) -> Vec<u8> {
    let Some(mut r) = r else { return Vec::new() };
    let mut out = Vec::new();
    let mut buf = [0u8; 4096];
    while let Ok(n) = r.read(&mut buf).await {
        if n == 0 {
            break;
        }
        let room = keep.saturating_sub(out.len()).min(n);
        out.extend_from_slice(&buf[..room]);
    }
    out
}

/// `(major, minor)` of `git --version`.
async fn version(bin: &str) -> Option<(u32, u32)> {
    let out = run(bin, None, &["--version"], Opts::read()).await.ok()?;
    parse_version(&String::from_utf8_lossy(&out))
}

fn parse_version(s: &str) -> Option<(u32, u32)> {
    let v = s.trim().strip_prefix("git version ")?;
    let mut it = v.split(|c: char| !c.is_ascii_digit());
    Some((it.next()?.parse().ok()?, it.next()?.parse().ok()?))
}

/// Whether the server has a git kwant can use ([`MIN_VERSION`] or newer). A yes is kept; a no is
/// asked again after a minute, so installing git needs no restart.
pub async fn available() -> bool {
    static SEEN: OnceLock<Mutex<Option<(bool, Instant)>>> = OnceLock::new();
    let seen = SEEN.get_or_init(Default::default);
    if let Some((ok, at)) = *seen.lock().unwrap() {
        if ok || at.elapsed() < Duration::from_secs(60) {
            return ok;
        }
    }
    let ok = version("git").await.is_some_and(|v| v >= MIN_VERSION);
    *seen.lock().unwrap() = Some((ok, Instant::now()));
    ok
}

// ---------------------------------------------------------------------------------------------
// Input checks: nothing reaches git's argv unchecked.

/// A full commit id (sha-1 or sha-256), lowercase hex.
pub fn is_sha(s: &str) -> bool {
    (s.len() == 40 || s.len() == 64) && s.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

/// A ref name kwant passes to git: a valid branch name (as the routes check) under `refs/`.
fn ok_ref(r: &str) -> bool {
    r.strip_prefix("refs/").is_some_and(super::routes::valid_branch)
}

fn ok_branch(b: &str) -> bool {
    super::routes::valid_branch(b)
}

/// A user name kept in an ssh URL (`git@`, `gitlab@`): never a password or a token.
fn ok_user(u: &str) -> bool {
    !u.is_empty() && u.len() <= 32 && !u.starts_with('-') && u.chars().all(|c| c.is_ascii_alphanumeric() || "._-".contains(c))
}

fn ok_host(h: &str) -> bool {
    h.len() > 1 && h.len() <= 253 && !h.starts_with('-') && !h.starts_with('.') && h.chars().all(|c| c.is_ascii_alphanumeric() || "-.".contains(c))
}

fn ok_path(p: &str) -> bool {
    let p = p.trim_matches('/');
    !p.is_empty()
        && p.len() <= 400
        && p.split('/').all(|s| !s.is_empty() && s != "." && s != ".." && !s.starts_with('-') && s.chars().all(|c| c.is_ascii_alphanumeric() || "._-~".contains(c)))
}

/// [`fetch_url`] for a remote that a team's folder or repo link names, which anyone working in that
/// folder can write: also `None` for a host on this machine or link-local (`localhost`, `127.0.0.0/8`,
/// `::1`, `169.254.0.0/16`, `fe80::/10`, `0.0.0.0`), so a folder can't point kwant's fetches at the
/// server itself or a cloud metadata address. LAN hosts (a self-hosted Gitea on `192.168.x`) stay.
/// A remote kwant makes itself from a connection the person set up uses [`fetch_url`].
pub fn remote_url(raw: &str) -> Option<String> {
    fetch_url(raw).filter(|u| !on_this_machine(u))
}

fn on_this_machine(url: &str) -> bool {
    use std::net::{Ipv4Addr, Ipv6Addr};
    let host = match url.find("://") {
        Some(i) => {
            let rest = &url[i + 3..];
            let rest = &rest[..rest.find('/').unwrap_or(rest.len())];
            let rest = rest.rsplit_once('@').map_or(rest, |(_, h)| h);
            // `host:port` or `[v6]:port`
            match rest.strip_prefix('[') {
                Some(r) => format!("[{}]", &r[..r.find(']').unwrap_or(r.len())]),
                None => rest.split(':').next().unwrap_or("").to_string(),
            }
        }
        None => {
            let h = url.split_once(':').map_or(url, |(h, _)| h);
            h.rsplit_once('@').map_or(h, |(_, h)| h).to_string()
        }
    };
    let v4 = |ip: Ipv4Addr| ip.is_loopback() || ip.is_link_local() || ip.is_unspecified() || ip.is_broadcast();
    let v6 = |ip: Ipv6Addr| ip.is_loopback() || ip.is_unspecified() || (ip.segments()[0] & 0xffc0) == 0xfe80 || ip.to_ipv4_mapped().is_some_and(v4);
    match url::Host::parse(&host) {
        Ok(url::Host::Domain(d)) => {
            let d = d.trim_end_matches('.').to_ascii_lowercase();
            d == "localhost" || d.ends_with(".localhost")
        }
        Ok(url::Host::Ipv4(ip)) => v4(ip),
        Ok(url::Host::Ipv6(ip)) => v6(ip),
        Err(_) => true,
    }
}

/// What git fetches a remote from, made from a remote URL or repo link as written: its userinfo
/// stripped first (an ssh user name like `git` is put back; a password or token never is), its
/// query and fragment dropped. https, http, ssh, git and scp-like `user@host:path` only: a local
/// path, `file://` or `ext::` is `None`, as is a host, user or path segment that starts with `-`.
pub fn fetch_url(raw: &str) -> Option<String> {
    let raw = raw.trim();
    if raw.is_empty() || raw.len() > 500 || raw.chars().any(|c| c.is_whitespace() || c.is_control()) {
        return None;
    }
    // Never a `.` or `..` segment (the URL parser would fold it away)
    if raw.split(['/', ':']).any(|seg| seg == "." || seg == "..") {
        return None;
    }
    // The user name (never its password) before anything is stripped, to put back on ssh only:
    // the userinfo is everything up to the last `@` before any `?` or `#`, as strip_userinfo reads
    let user = {
        let rest = raw.find("://").map(|i| &raw[i + 3..]).unwrap_or(raw);
        let rest = &rest[..rest.find(['?', '#']).unwrap_or(rest.len())];
        rest.rfind('@').and_then(|at| rest[..at].split(':').next()).filter(|u| ok_user(u)).map(str::to_string)
    };
    let s = super::repo::strip_userinfo(raw);
    if let Some(i) = s.find("://") {
        let scheme = s[..i].to_ascii_lowercase();
        let scheme = scheme.strip_prefix("git+").unwrap_or(&scheme).to_string();
        if !matches!(scheme.as_str(), "https" | "http" | "ssh" | "git") {
            return None;
        }
        let u = url::Url::parse(&format!("{scheme}{}", &s[i..])).ok()?;
        let host = u.host_str()?.to_ascii_lowercase();
        if matches!(u.host(), Some(url::Host::Domain(d)) if !ok_host(d)) {
            return None;
        }
        if !ok_path(u.path()) {
            return None;
        }
        let port = u.port().map(|p| format!(":{p}")).unwrap_or_default();
        let user = match (&*scheme, user) {
            ("ssh", Some(u)) => format!("{u}@"),
            _ => String::new(),
        };
        Some(format!("{scheme}://{user}{host}{port}{}", u.path()))
    } else {
        // scp-like: host:path, with the user name put back
        let (host, path) = s.split_once(':')?;
        let host = host.to_ascii_lowercase();
        if !ok_host(&host) || !ok_path(path) {
            return None;
        }
        Some(match user {
            Some(u) => format!("{u}@{host}:{path}"),
            None => format!("{host}:{path}"),
        })
    }
}

/// The web page of a commit on `host` (`base/<sha>`), for the hosts whose pages kwant knows.
pub fn commit_base(host: &str, repo: &str) -> Option<String> {
    let h = host.to_ascii_lowercase();
    let tail = if h == "gitlab.com" || h.starts_with("gitlab.") {
        "-/commit"
    } else if h == "bitbucket.org" {
        "commits"
    } else if h == "codeberg.org" || h.starts_with("gitea.") || h.starts_with("forgejo.") {
        "commit"
    } else {
        return None;
    };
    (ok_host(&h) && ok_path(repo)).then(|| format!("https://{h}/{}/{tail}", repo.trim_matches('/')))
}

// ---------------------------------------------------------------------------------------------
// Repos

/// One lock per mirror: a poll and a removal never meet in the same directory.
fn locks() -> &'static Mutex<std::collections::HashMap<PathBuf, Arc<tokio::sync::Mutex<()>>>> {
    static L: OnceLock<Mutex<std::collections::HashMap<PathBuf, Arc<tokio::sync::Mutex<()>>>>> = OnceLock::new();
    L.get_or_init(Default::default)
}

async fn lock(dir: &Path) -> tokio::sync::OwnedMutexGuard<()> {
    let m = locks().lock().unwrap().entry(dir.to_path_buf()).or_default().clone();
    m.lock_owned().await
}

/// A file name made of `s`: letters, digits, `.`, `_` and `-`, at most 60, never starting with `.`.
fn safe_name(s: &str) -> String {
    let n: String = s.chars().map(|c| if c.is_ascii_alphanumeric() || "._-".contains(c) { c } else { '_' }).take(60).collect();
    let n = n.trim_start_matches('.').to_string();
    if n.is_empty() {
        "_".into()
    } else {
        n
    }
}

/// Where the mirror of watched repo `key` in `space` lives:
/// `<data>/dev/<space>/<key, made safe>-<8 hex of its sha-256>.git` (the hash keeps two keys that
/// read the same once made safe apart).
pub fn mirror_dir(data_dir: &Path, space: &str, key: &str) -> PathBuf {
    use sha2::Digest;
    let hash = sha2::Sha256::digest(key.as_bytes());
    let hex: String = hash.iter().take(4).map(|b| format!("{b:02x}")).collect();
    data_dir.join("dev").join(safe_name(space)).join(format!("{}-{hex}.git", safe_name(key)))
}

/// Deletes the mirror of a repo no longer watched (its cards stay).
pub async fn remove_mirror(data_dir: &Path, space: &str, key: &str) {
    let dir = mirror_dir(data_dir, space, key);
    let _held = lock(&dir).await;
    if dir.is_dir() {
        if let Err(e) = tokio::fs::remove_dir_all(&dir).await {
            tracing::warn!(space = %space, "dev: could not remove a repo mirror: {}", e.kind());
        }
    }
}

/// A git repo kwant reads: a bare mirror it keeps, or a team folder's `.git`.
pub struct Repo {
    dir: PathBuf,
    local: bool,
    _held: Option<tokio::sync::OwnedMutexGuard<()>>,
}

impl std::fmt::Debug for Repo {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Repo").field("local", &self.local).finish_non_exhaustive()
    }
}

impl Repo {
    /// The mirror of `key` in `space`, made (empty) if it isn't there yet. Held locked until dropped.
    pub async fn mirror(data_dir: &Path, space: &str, key: &str) -> Result<Repo, GitError> {
        let dir = mirror_dir(data_dir, space, key);
        let held = lock(&dir).await;
        if !dir.join("HEAD").is_file() {
            let parent = dir.parent().ok_or(GitError::Failed)?;
            tokio::fs::create_dir_all(parent).await.map_err(|_| GitError::Failed)?;
            let _ = tokio::fs::remove_dir_all(&dir).await;
            let d = dir.to_str().ok_or(GitError::Refused)?;
            if let Err(e) = run("git", None, &["init", "--bare", "--quiet", "--template=", "--", d], Opts::read()).await {
                let _ = tokio::fs::remove_dir_all(&dir).await;
                return Err(e);
            }
        }
        Ok(Repo { dir, local: false, _held: Some(held) })
    }

    /// A team folder's own git, read in place: `<folder>/.git` (a directory, or a worktree's file).
    pub fn local(folder: &Path) -> Result<Repo, GitError> {
        let dir = folder.join(".git");
        if !dir.exists() {
            return Err(GitError::NotFound);
        }
        Ok(Repo { dir, local: true, _held: None })
    }

    pub fn is_local(&self) -> bool {
        self.local
    }

    async fn git(&self, args: &[&str]) -> Result<Vec<u8>, GitError> {
        run("git", Some(&self.dir), args, Opts { file: self.local, ..Opts::read() }).await
    }

    async fn text(&self, args: &[&str]) -> Result<String, GitError> {
        Ok(String::from_utf8_lossy(&self.git(args).await?).into_owned())
    }

    /// Brings the mirror's `branch` and tags up to date from `url` (pruning deleted tags). A team
    /// folder isn't fetched: it's read as its team left it.
    pub async fn fetch(&self, url: &str, branch: &str, auth: Option<&Auth>) -> Result<(), GitError> {
        if self.local {
            return Ok(());
        }
        if !ok_branch(branch) || fetch_url(url).as_deref() != Some(url) {
            return Err(GitError::Refused);
        }
        let heads = format!("+refs/heads/{branch}:refs/heads/{branch}");
        let args = ["fetch", "--quiet", "--no-tags", "--no-write-fetch-head", "--no-recurse-submodules", "--prune", "--", url, &heads, "+refs/tags/*:refs/tags/*"];
        run("git", Some(&self.dir), &args, Opts { timeout: FETCH_TIMEOUT, max_out: MAX_OUT, auth, file: false }).await.map(|_| ())
    }

    /// Fetches one ref of `url` (a PR head, `refs/pull/7/head`) into `to` (`refs/kwant/…`); the sha
    /// it now points at.
    pub async fn fetch_ref(&self, url: &str, from: &str, to: &str, auth: Option<&Auth>) -> Result<String, GitError> {
        if self.local || !ok_ref(from) || !to.starts_with("refs/kwant/") || !ok_ref(to) || fetch_url(url).as_deref() != Some(url) {
            return Err(GitError::Refused);
        }
        let spec = format!("+{from}:{to}");
        let args = ["fetch", "--quiet", "--no-tags", "--no-write-fetch-head", "--no-recurse-submodules", "--", url, &spec];
        run("git", Some(&self.dir), &args, Opts { timeout: FETCH_TIMEOUT, max_out: MAX_OUT, auth, file: false }).await?;
        self.resolve(to).await
    }

    /// Deletes a ref [`Self::fetch_ref`] made.
    pub async fn drop_ref(&self, name: &str) {
        if !self.local && name.starts_with("refs/kwant/") && ok_ref(name) {
            let _ = self.git(&["update-ref", "-d", name]).await;
        }
    }

    /// The default branch of `url` (what its `HEAD` points at), or of a team folder.
    pub async fn default_branch(&self, url: &str, auth: Option<&Auth>) -> Result<String, GitError> {
        let branch = if self.local {
            self.text(&["symbolic-ref", "--quiet", "HEAD"]).await?
        } else {
            if fetch_url(url).as_deref() != Some(url) {
                return Err(GitError::Refused);
            }
            let out = run("git", Some(&self.dir), &["ls-remote", "--symref", "--", url, "HEAD"], Opts { timeout: REMOTE_TIMEOUT, max_out: 1 << 16, auth, file: false }).await?;
            let out = String::from_utf8_lossy(&out).into_owned();
            out.lines().find_map(|l| l.strip_prefix("ref: ").and_then(|r| r.strip_suffix("\tHEAD")).map(str::to_string)).ok_or(GitError::NotFound)?
        };
        branch.trim().strip_prefix("refs/heads/").filter(|b| ok_branch(b)).map(str::to_string).ok_or(GitError::NotFound)
    }

    /// Whether `url` (or a team folder) has `branch`.
    pub async fn has_branch(&self, url: &str, branch: &str, auth: Option<&Auth>) -> Result<bool, GitError> {
        if !ok_branch(branch) {
            return Ok(false);
        }
        let name = format!("refs/heads/{branch}");
        if self.local {
            return Ok(self.resolve(&name).await.is_ok());
        }
        if fetch_url(url).as_deref() != Some(url) {
            return Err(GitError::Refused);
        }
        let out = run("git", Some(&self.dir), &["ls-remote", "--heads", "--", url, &name], Opts { timeout: REMOTE_TIMEOUT, max_out: 1 << 16, auth, file: false }).await?;
        Ok(String::from_utf8_lossy(&out).lines().any(|l| l.split('\t').nth(1) == Some(name.as_str())))
    }

    /// The commit a ref or sha names.
    pub async fn resolve(&self, rev: &str) -> Result<String, GitError> {
        if !(is_sha(rev) || ok_ref(rev)) {
            return Err(GitError::Refused);
        }
        let spec = format!("{rev}^{{commit}}");
        // `--quiet` prints nothing for a missing ref: that's a not found
        let sha = match self.text(&["rev-parse", "--verify", "--quiet", "--end-of-options", &spec]).await {
            Ok(s) => s.trim().to_string(),
            Err(GitError::Failed) => return Err(GitError::NotFound),
            Err(e) => return Err(e),
        };
        if is_sha(&sha) {
            Ok(sha)
        } else {
            Err(GitError::NotFound)
        }
    }

    /// Up to [`MAX_COMMITS`] commits on `branch`'s first-parent line newer than `last_seen`, oldest
    /// first, and whether `last_seen` was found. With no `last_seen`, or one that's gone (a force
    /// push), the newest ones, as GitHub's list.
    pub async fn commits_since(&self, branch: &str, last_seen: Option<&str>) -> Result<(Vec<Commit>, bool), GitError> {
        if !ok_branch(branch) {
            return Err(GitError::Refused);
        }
        let head = format!("refs/heads/{branch}");
        self.resolve(&head).await?;
        let found = match last_seen.filter(|s| is_sha(s)) {
            Some(s) => self.git(&["merge-base", "--is-ancestor", s, &head]).await.is_ok(),
            None => false,
        };
        let max = format!("--max-count={MAX_COMMITS}");
        let not = last_seen.map(|s| format!("^{s}")).unwrap_or_default();
        let mut args = vec!["log", "-z", "--first-parent", &max, LOG_FORMAT, "--end-of-options", &head];
        if found {
            args.push(&not);
        }
        args.push("--");
        let out = self.git(&args).await?;
        let mut list = parse_log(&out);
        list.reverse();
        Ok((list, found))
    }

    /// One commit, with its files against its first parent (or every file, for a root commit).
    pub async fn commit(&self, sha: &str) -> Result<Commit, GitError> {
        if !is_sha(sha) {
            return Err(GitError::Refused);
        }
        let out = self.git(&["log", "-z", "-1", LOG_FORMAT, "--end-of-options", sha, "--"]).await?;
        let mut c = parse_log(&out).into_iter().next().ok_or(GitError::NotFound)?;
        c.files = self.diff(c.parents.first().map(String::as_str), sha).await?;
        Ok(c)
    }

    /// The files changed from `base` to `head` (every file of `head` with no base), as GitHub
    /// lists a PR's: status, line counts, renames, and each text file's hunks. At most
    /// [`MAX_FILES`]; when the patches are too big to read whole, the list comes without them.
    pub async fn diff(&self, base: Option<&str>, head: &str) -> Result<Vec<FileDiff>, GitError> {
        if !is_sha(head) || base.is_some_and(|b| !is_sha(b)) {
            return Err(GitError::Refused);
        }
        let range: Vec<&str> = match base {
            Some(b) => vec![b, head],
            None => vec!["--root", head],
        };
        let with = |extra: &[&'static str]| {
            let mut a: Vec<&str> = vec!["diff-tree", "-r", "-M", "--no-commit-id"];
            a.extend_from_slice(extra);
            a.extend_from_slice(&range);
            a.push("--");
            a
        };
        let raw = self.git(&with(&["-z", "--raw"])).await?;
        let num = self.git(&with(&["-z", "--numstat"])).await?;
        let mut files = parse_raw(&raw);
        let counts = parse_numstat(&num);
        if counts.len() == files.len() {
            for (f, (a, d)) in files.iter_mut().zip(counts) {
                f.additions = a;
                f.deletions = d;
            }
        }
        // Hunks: one section per file, in the same order; anything else and none are kept
        if let Ok(p) = self.git(&with(&["-p", "--no-color", "--no-ext-diff", "--no-textconv", "--src-prefix=a/", "--dst-prefix=b/"])).await {
            let patches = split_patches(&String::from_utf8_lossy(&p));
            if patches.len() == files.len() {
                for (f, p) in files.iter_mut().zip(patches) {
                    f.patch = p;
                }
            }
        }
        files.truncate(MAX_FILES);
        Ok(files)
    }

    /// The commit both `a` and `b` grew from.
    pub async fn merge_base(&self, a: &str, b: &str) -> Result<String, GitError> {
        if !(is_sha(a) || ok_ref(a)) || !(is_sha(b) || ok_ref(b)) {
            return Err(GitError::Refused);
        }
        let out = self.text(&["merge-base", "--end-of-options", a, b]).await?;
        let sha = out.trim().to_string();
        if is_sha(&sha) {
            Ok(sha)
        } else {
            Err(GitError::NotFound)
        }
    }

    /// A text file at `sha`. `None` when it isn't a file there, is bigger than [`MAX_FILE`] or
    /// isn't UTF-8.
    pub async fn file_at(&self, path: &str, sha: &str) -> Result<Option<String>, GitError> {
        if !is_sha(sha) || path.is_empty() || path.len() > 4096 || path.starts_with('/') || path.contains('\0') {
            return Err(GitError::Refused);
        }
        let spec = format!("{sha}:{path}");
        match self.text(&["cat-file", "-t", &spec]).await {
            Ok(t) if t.trim() == "blob" => {}
            Ok(_) | Err(GitError::NotFound | GitError::Failed) => return Ok(None),
            Err(e) => return Err(e),
        }
        match run("git", Some(&self.dir), &["cat-file", "blob", &spec], Opts { max_out: MAX_FILE, file: self.local, ..Opts::read() }).await {
            Ok(bytes) => Ok(String::from_utf8(bytes).ok()),
            Err(GitError::TooBig | GitError::NotFound) => Ok(None),
            Err(e) => Err(e),
        }
    }

    /// The tag nearest `branch`'s tip (`git describe`), with its commit's date. `None`: no tag.
    pub async fn latest_tag(&self, branch: &str) -> Result<Option<(String, Option<DateTime<Utc>>)>, GitError> {
        if !ok_branch(branch) {
            return Err(GitError::Refused);
        }
        let head = format!("refs/heads/{branch}");
        let name = match self.text(&["describe", "--tags", "--abbrev=0", "--end-of-options", &head]).await {
            Ok(n) => n.trim().to_string(),
            Err(_) => return Ok(None),
        };
        let tag = format!("refs/tags/{name}");
        if !ok_ref(&tag) {
            return Ok(None);
        }
        let sha = self.resolve(&tag).await?;
        let at = self.text(&["log", "-1", "--format=%cI", "--end-of-options", &sha, "--"]).await.ok().and_then(|d| date(d.trim()));
        Ok(Some((name, at)))
    }

    /// How many commits on `branch` touched `path` between `since` and `until` (at most 2), by
    /// commit date as GitHub's `since`/`until`.
    pub async fn path_commits(&self, branch: &str, path: &str, since: DateTime<Utc>, until: DateTime<Utc>) -> Result<usize, GitError> {
        if !ok_branch(branch) || path.is_empty() || path.contains('\0') {
            return Err(GitError::Refused);
        }
        let head = format!("refs/heads/{branch}");
        let (s, u) = (format!("--since={}", since.to_rfc3339()), format!("--until={}", until.to_rfc3339()));
        let out = self.text(&["log", "--format=%H", "--max-count=2", &s, &u, "--end-of-options", &head, "--", path]).await?;
        Ok(out.lines().filter(|l| is_sha(l.trim())).count())
    }

    /// The names at the root of `branch` (files and folders).
    pub async fn root_names(&self, branch: &str) -> Result<Vec<String>, GitError> {
        if !ok_branch(branch) {
            return Err(GitError::Refused);
        }
        let head = format!("refs/heads/{branch}");
        let out = self.git(&["ls-tree", "-z", "--name-only", "--end-of-options", &head]).await?;
        Ok(out.split(|b| *b == 0).filter(|n| !n.is_empty()).map(|n| String::from_utf8_lossy(n).into_owned()).collect())
    }
}

/// One commit per NUL-ended record: sha, parents, author name, email, date, committer name,
/// email, date and the whole message, split by unit separators.
const LOG_FORMAT: &str = "--format=%H%x1f%P%x1f%an%x1f%ae%x1f%aI%x1f%cn%x1f%ce%x1f%cI%x1f%B";

fn date(s: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(s).ok().map(|d| d.with_timezone(&Utc))
}

fn parse_log(out: &[u8]) -> Vec<Commit> {
    let text = String::from_utf8_lossy(out);
    text.split('\0')
        .map(|r| r.trim_start_matches('\n'))
        .filter(|r| !r.is_empty())
        .filter_map(|r| {
            let f: Vec<&str> = r.splitn(9, '\x1f').collect();
            if f.len() < 9 || !is_sha(f[0]) {
                return None;
            }
            let person = |name: &str, email: &str, at: &str| Person { name: name.to_string(), email: Some(email.to_string()).filter(|e| !e.is_empty()), date: date(at) };
            Some(Commit {
                sha: f[0].to_string(),
                url: None,
                message: f[8].trim_end().to_string(),
                author: person(f[2], f[3], f[4]),
                committer: person(f[5], f[6], f[7]),
                login: None,
                parents: f[1].split_whitespace().filter(|p| is_sha(p)).map(str::to_string).collect(),
                files: Vec::new(),
            })
        })
        .collect()
}

/// `--raw -z`: `:mode mode sha sha STATUS\0path\0` (two paths for a rename or copy).
fn parse_raw(out: &[u8]) -> Vec<FileDiff> {
    let text = String::from_utf8_lossy(out);
    let mut it = text.split('\0');
    let mut files = Vec::new();
    while let Some(meta) = it.next() {
        let meta = meta.trim_start_matches('\n');
        if !meta.starts_with(':') {
            continue;
        }
        let code = meta.rsplit(' ').next().and_then(|s| s.chars().next()).unwrap_or('M');
        let (path, previous) = if matches!(code, 'R' | 'C') {
            let old = it.next().unwrap_or_default().to_string();
            (it.next().unwrap_or_default().to_string(), Some(old))
        } else {
            (it.next().unwrap_or_default().to_string(), None)
        };
        let status = match code {
            'A' => "added",
            'D' => "removed",
            'R' => "renamed",
            'C' => "copied",
            'T' => "changed",
            _ => "modified",
        };
        files.push(FileDiff { path, status: status.into(), additions: 0, deletions: 0, patch: None, previous_path: previous });
    }
    files
}

/// `--numstat -z`: `add\tdel\tpath\0`, or `add\tdel\t\0old\0new\0` for a rename; `-` for binary.
fn parse_numstat(out: &[u8]) -> Vec<(u64, u64)> {
    let text = String::from_utf8_lossy(out);
    let mut it = text.split('\0');
    let mut out = Vec::new();
    while let Some(tok) = it.next() {
        let tok = tok.trim_start_matches('\n');
        let mut f = tok.splitn(3, '\t');
        let (Some(a), Some(d), Some(p)) = (f.next(), f.next(), f.next()) else { continue };
        if p.is_empty() {
            it.next();
            it.next();
        }
        out.push((a.parse().unwrap_or(0), d.parse().unwrap_or(0)));
    }
    out
}

/// A `-p` diff split per file: each file's hunks from its first `@@` (GitHub's `patch`), `None`
/// for a binary file or one with no hunks (a mode change, an empty file).
fn split_patches(text: &str) -> Vec<Option<String>> {
    if text.trim().is_empty() {
        return Vec::new();
    }
    let Some(body) = text.strip_prefix("diff --git ") else { return Vec::new() };
    body.split("\ndiff --git ")
        .map(|sec| {
            let hunks = sec.find("\n@@ ").map(|i| &sec[i + 1..])?;
            if sec[..sec.len() - hunks.len()].contains("\nBinary files ") {
                return None;
            }
            Some(hunks.trim_end_matches('\n').to_string())
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Command;

    /// Plain git for setting a throwaway repo up (never the code under test).
    fn sh(dir: &Path, args: &[&str]) -> String {
        let out = Command::new("git")
            .arg("-C")
            .arg(dir)
            .args(["-c", "user.name=Demo Person", "-c", "user.email=demo@example.com", "-c", "init.defaultBranch=main", "-c", "commit.gpgsign=false", "-c", "tag.gpgsign=false"])
            .args(args)
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_AUTHOR_DATE", "2026-11-01T10:00:00Z")
            .env("GIT_COMMITTER_DATE", "2026-11-01T10:00:00Z")
            .output()
            .unwrap();
        assert!(out.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&out.stderr));
        String::from_utf8_lossy(&out.stdout).trim().to_string()
    }

    fn write(dir: &Path, path: &str, text: &[u8]) {
        let p = dir.join(path);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, text).unwrap();
    }

    fn commit_all(dir: &Path, msg: &str) -> String {
        sh(dir, &["add", "-A"]);
        sh(dir, &["commit", "-q", "--allow-empty", "-m", msg]);
        sh(dir, &["rev-parse", "HEAD"])
    }

    /// A work repo with: a root commit, a feature branch merged with a merge commit, a rename, a
    /// binary file and a tag.
    fn upstream(root: &Path) -> (PathBuf, Vec<String>) {
        let w = root.join("work");
        std::fs::create_dir_all(&w).unwrap();
        sh(&w, &["init", "-q"]);
        write(&w, "README.md", b"hello\n");
        write(&w, "fly.toml", b"app = 'demo'\n");
        let c0 = commit_all(&w, "Start");
        write(&w, "src/a.rs", b"fn a() {}\n");
        let c1 = commit_all(&w, "Add a\n\nWith a body.");
        sh(&w, &["tag", "v0.1.0"]);
        sh(&w, &["checkout", "-q", "-b", "feature"]);
        write(&w, "src/b.rs", b"fn b() {}\n");
        let f1 = commit_all(&w, "Add b on a branch");
        sh(&w, &["checkout", "-q", "main"]);
        sh(&w, &["merge", "-q", "--no-ff", "-m", "Merge feature", "feature"]);
        let m = sh(&w, &["rev-parse", "HEAD"]);
        sh(&w, &["mv", "src/a.rs", "src/renamed.rs"]);
        write(&w, "logo.bin", &[0, 159, 146, 150, 0, 1, 2]);
        write(&w, "README.md", b"hello\nworld\n");
        let c3 = commit_all(&w, "Rename a, add a logo");
        (w, vec![c0, c1, f1, m, c3])
    }

    #[tokio::test]
    async fn a_mirror_reads_first_parent_history_files_tags_and_text() {
        let tmp = tempfile::tempdir().unwrap();
        let (w, c) = upstream(tmp.path());
        let data = tmp.path().join("data");
        let url = w.to_str().unwrap();
        let repo = Repo::mirror(&data, "space 1", "local:web").await.unwrap();
        // A local path is no fetch URL: the mirror is filled the way a test may, by file protocol
        assert_eq!(repo.fetch(url, "main", None).await, Err(GitError::Refused));
        run("git", Some(&repo.dir), &["fetch", "--quiet", "--", url, "+refs/heads/main:refs/heads/main", "+refs/tags/*:refs/tags/*"], Opts { file: true, ..Opts::read() }).await.unwrap();

        let (list, found) = repo.commits_since("main", None).await.unwrap();
        assert!(!found);
        let shas: Vec<&str> = list.iter().map(|c| c.sha.as_str()).collect();
        assert_eq!(shas, [&c[0], &c[1], &c[3], &c[4]], "first parent only: the branch commit isn't listed");
        assert_eq!(list[1].message, "Add a\n\nWith a body.");
        assert_eq!((list[1].author.name.as_str(), list[1].author.email.as_deref()), ("Demo Person", Some("demo@example.com")));
        assert_eq!(list[1].committer.date.unwrap().to_rfc3339(), "2026-11-01T10:00:00+00:00");
        assert_eq!(list[2].parents, [c[1].clone(), c[2].clone()], "the merge's parents, first one first");

        let (list, found) = repo.commits_since("main", Some(&c[1])).await.unwrap();
        assert!(found);
        assert_eq!(list.iter().map(|c| c.sha.clone()).collect::<Vec<_>>(), [c[3].clone(), c[4].clone()]);
        let (list, found) = repo.commits_since("main", Some(&c[4])).await.unwrap();
        assert!(found && list.is_empty(), "nothing new");
        let gone = "0".repeat(40);
        let (list, found) = repo.commits_since("main", Some(&gone)).await.unwrap();
        assert!(!found && list.len() == 4, "a lost last-seen commit: the newest, as GitHub's list");
        assert_eq!(repo.commits_since("nope", None).await.unwrap_err(), GitError::NotFound);
        assert_eq!(repo.commits_since("-x", None).await.unwrap_err(), GitError::Refused);

        // The merge against its first parent is what the branch brought
        let merge = repo.commit(&c[3]).await.unwrap();
        assert_eq!(merge.files, [FileDiff { path: "src/b.rs".into(), status: "added".into(), additions: 1, deletions: 0, patch: Some("@@ -0,0 +1 @@\n+fn b() {}".into()), previous_path: None }]);
        let last = repo.commit(&c[4]).await.unwrap();
        let by = |p: &str| last.files.iter().find(|f| f.path == p).unwrap().clone();
        assert_eq!(by("src/renamed.rs").status, "renamed");
        assert_eq!(by("src/renamed.rs").previous_path.as_deref(), Some("src/a.rs"));
        assert_eq!((by("logo.bin").status.as_str(), by("logo.bin").patch.clone(), by("logo.bin").additions), ("added", None, 0), "binary: no patch");
        assert_eq!((by("README.md").additions, by("README.md").deletions), (1, 0));
        assert_eq!(by("README.md").patch.as_deref(), Some("@@ -1 +1,2 @@\n hello\n+world"));
        let root = repo.commit(&c[0]).await.unwrap();
        assert_eq!(root.files.iter().map(|f| f.path.as_str()).collect::<Vec<_>>(), ["README.md", "fly.toml"], "a root commit lists every file");
        assert!(root.files.iter().all(|f| f.status == "added"));
        // Three-dot: from where the branch grew
        let mb = repo.merge_base(&c[1], &c[2]).await.unwrap();
        assert_eq!(mb, c[1]);
        assert_eq!(repo.diff(Some(&c[0]), &c[4]).await.unwrap().len(), 4);

        assert_eq!(repo.file_at("README.md", &c[4]).await.unwrap().as_deref(), Some("hello\nworld\n"));
        assert_eq!(repo.file_at("logo.bin", &c[4]).await.unwrap(), None, "not UTF-8");
        assert_eq!(repo.file_at("src", &c[4]).await.unwrap(), None, "a folder");
        assert_eq!(repo.file_at("missing.txt", &c[4]).await.unwrap(), None);
        assert_eq!(repo.file_at("README.md", "main").await.unwrap_err(), GitError::Refused, "a sha only");

        let (tag, at) = repo.latest_tag("main").await.unwrap().unwrap();
        assert_eq!((tag.as_str(), at.unwrap().to_rfc3339().as_str()), ("v0.1.0", "2026-11-01T10:00:00+00:00"));
        let mut names = repo.root_names("main").await.unwrap();
        names.sort();
        assert_eq!(names, ["README.md", "fly.toml", "logo.bin", "src"]);
        assert_eq!(super::super::repo::deploy_hint(&names), Some("fly"));

        let day = chrono::Duration::days(1);
        let t = date("2026-11-01T10:00:00Z").unwrap();
        assert_eq!(repo.path_commits("main", "README.md", t - day, t + day).await.unwrap(), 2);
        assert_eq!(repo.path_commits("main", "README.md", t + day, t + day + day).await.unwrap(), 0);
        assert_eq!(repo.path_commits("main", "src/*", t - day, t + day).await.unwrap(), 0, "a path is literal, never a glob");
    }

    #[tokio::test]
    async fn a_team_folder_is_read_in_place_without_touching_its_work() {
        let tmp = tempfile::tempdir().unwrap();
        let (w, c) = upstream(tmp.path());
        write(&w, "README.md", b"uncommitted\n");
        let status_before = sh(&w, &["status", "--porcelain"]);
        let index_before = std::fs::read(w.join(".git/index")).unwrap();
        let repo = Repo::local(&w).unwrap();
        assert!(repo.is_local());
        assert_eq!(repo.default_branch("", None).await.unwrap(), "main");
        assert!(repo.has_branch("", "feature", None).await.unwrap());
        assert!(!repo.has_branch("", "nope", None).await.unwrap());
        repo.fetch("anything", "main", None).await.unwrap();
        let (list, _) = repo.commits_since("main", Some(&c[3])).await.unwrap();
        assert_eq!(list.iter().map(|c| c.sha.clone()).collect::<Vec<_>>(), [c[4].clone()]);
        assert_eq!(repo.file_at("README.md", &c[4]).await.unwrap().as_deref(), Some("hello\nworld\n"), "the commit's text, not the folder's");
        assert_eq!(std::fs::read_to_string(w.join("README.md")).unwrap(), "uncommitted\n");
        assert_eq!(sh(&w, &["status", "--porcelain"]), status_before);
        assert_eq!(std::fs::read(w.join(".git/index")).unwrap(), index_before, "the index wasn't refreshed");
        assert_eq!(Repo::local(&tmp.path().join("nothing")).unwrap_err(), GitError::NotFound);
    }

    #[tokio::test]
    async fn a_mirror_lives_under_data_dev_and_is_removed_whole() {
        let tmp = tempfile::tempdir().unwrap();
        let data = tmp.path();
        let a = mirror_dir(data, "s1", "gitlab.example.com/group/sub/ops");
        let b = mirror_dir(data, "s1", "gitlab.example.com/group_sub/ops");
        assert!(a.starts_with(data.join("dev").join("s1")));
        assert_ne!(a, b, "two keys that read the same once made safe stay apart");
        assert_eq!(mirror_dir(data, "../x", "../../etc").parent().unwrap().file_name().unwrap(), "_x", "no way out of the data folder");
        assert!(mirror_dir(data, "../x", "../../etc").starts_with(data.join("dev")));
        let repo = Repo::mirror(data, "s1", "gitlab.example.com/group/sub/ops").await.unwrap();
        assert!(a.join("HEAD").is_file());
        assert!(!a.join("hooks").exists(), "no template: no sample hooks");
        drop(repo);
        remove_mirror(data, "s1", "gitlab.example.com/group/sub/ops").await;
        assert!(!a.exists());
    }

    #[test]
    fn a_folders_remote_never_points_at_this_machine() {
        for raw in [
            "http://127.0.0.1:9/o/n.git",
            "https://127.1/o/n",
            "http://localhost:3000/o/n",
            "https://git.localhost/o/n",
            "ssh://dev@[::1]:2222/o/n",
            "git@127.0.0.1:o/n",
            "https://169.254.169.254/latest/meta-data",
            "http://[fe80::1]/o/n",
            "http://[::ffff:127.0.0.1]/o/n",
            "http://0.0.0.0/o/n",
            "http://2130706433/o/n",
            "http://LOCALHOST./o/n",
        ] {
            assert_eq!(remote_url(raw), None, "{raw}");
        }
        for raw in ["https://gitlab.example.com/group/ops.git", "http://192.168.1.20:3000/o/n.git", "git@10.0.0.5:o/n", "ssh://git@[2001:db8::1]/o/n"] {
            assert_eq!(remote_url(raw), fetch_url(raw), "{raw}");
            assert!(remote_url(raw).is_some(), "{raw}");
        }
    }

    #[test]
    fn fetch_urls_lose_their_userinfo_and_refuse_what_git_must_not_get() {
        let ok = [
            ("https://deploy:tok3n@gitlab.example.com/group/sub/ops.git", "https://gitlab.example.com/group/sub/ops.git"),
            ("https://x-access-token:a/b@c@github.com/o/n.git", "https://github.com/o/n.git"),
            ("https://gitlab.com/group/proj?x=1#y", "https://gitlab.com/group/proj"),
            ("git+https://host.example:8443/o/n", "https://host.example:8443/o/n"),
            ("ssh://git:secret@git.example.com:2222/o/n.git", "ssh://git@git.example.com:2222/o/n.git"),
            ("git@gitlab.com:group/proj.git", "git@gitlab.com:group/proj.git"),
            ("gitea.example.org:o/n", "gitea.example.org:o/n"),
            ("git://git.example.org/o/n", "git://git.example.org/o/n"),
            ("http://127.0.0.1:9/o/n.git", "http://127.0.0.1:9/o/n.git"),
            ("oauth2:tok@gitlab.com:group/proj", "oauth2@gitlab.com:group/proj"),
            ("ssh://dev@[::1]:2222/o/n", "ssh://dev@[::1]:2222/o/n"),
        ];
        for (raw, want) in ok {
            assert_eq!(fetch_url(raw).as_deref(), Some(want), "{raw}");
        }
        // A token as an ssh user name is never put back (made here so no token shape sits in the source)
        let tok = format!("{}{}", ["gh", "p_"].concat(), "a".repeat(40));
        assert_eq!(fetch_url(&format!("ssh://{tok}@host.example/o/n")).as_deref(), Some("ssh://host.example/o/n"));
        for raw in [
            "/srv/repos/app",
            "./app",
            "file:///srv/app",
            "ext::sh -c touch% /tmp/pwned",
            "fd::17",
            "https://-oProxyCommand=x/o/n",
            "ssh://-oProxyCommand=x/o/n",
            "-oProxyCommand=x:o/n",
            "git@host:-o/n",
            "https://host.example/o/../n",
            "https://host.example/",
            "https://host .example/o/n",
            "https://host.example/o\n/n",
            "ssh://host.example/o/./n",
            "",
        ] {
            assert_eq!(fetch_url(raw), None, "{raw}");
        }
    }

    #[test]
    fn commit_pages_are_known_for_some_hosts() {
        assert_eq!(commit_base("gitlab.com", "group/sub/p").as_deref(), Some("https://gitlab.com/group/sub/p/-/commit"));
        assert_eq!(commit_base("gitlab.example.com", "g/p").as_deref(), Some("https://gitlab.example.com/g/p/-/commit"));
        assert_eq!(commit_base("codeberg.org", "o/n").as_deref(), Some("https://codeberg.org/o/n/commit"));
        assert_eq!(commit_base("bitbucket.org", "o/n").as_deref(), Some("https://bitbucket.org/o/n/commits"));
        assert_eq!(commit_base("git.example.org", "o/n"), None);
    }

    #[test]
    fn errors_are_sorted_and_say_fixed_words() {
        assert_eq!(classify(b"fatal: could not read Username for 'https://gitlab.example.com': terminal prompts disabled"), GitError::Auth);
        assert_eq!(classify(b"git@host: Permission denied (publickey)."), GitError::Auth);
        assert_eq!(classify(b"remote: Repository not found.\nfatal: repository 'https://h/o/n.git/' not found"), GitError::NotFound);
        assert_eq!(classify(b"fatal: couldn't find remote ref refs/heads/nope"), GitError::NotFound);
        assert_eq!(classify(b"fatal: unable to access 'https://x/': Could not resolve host: x"), GitError::Unreachable);
        assert_eq!(classify(b"something else"), GitError::Failed);
        for e in [GitError::Auth, GitError::NotFound, GitError::Unreachable, GitError::Failed] {
            assert!(!e.to_string().contains("http"), "no URL in the words: {e}");
        }
        assert!(GitError::NoGit.to_string().contains(INSTALL_HINT));
        assert_eq!(parse_version("git version 2.43.0\n"), Some((2, 43)));
        assert_eq!(parse_version("git version 2.30.9.windows.1"), Some((2, 30)));
        assert!(parse_version("git version 2.30.9").unwrap() < MIN_VERSION);
    }

    #[tokio::test]
    async fn no_git_says_so() {
        assert_eq!(run("/nonexistent/kwant-no-git", None, &["--version"], Opts::read()).await, Err(GitError::NoGit));
        assert_eq!(version("/nonexistent/kwant-no-git").await, None);
        assert!(available().await, "the test machine has git {MIN_VERSION:?} or newer");
    }

    #[tokio::test]
    async fn a_slow_git_is_killed_with_its_children() {
        let tmp = tempfile::tempdir().unwrap();
        let fake = tmp.path().join("git");
        let pid_file = tmp.path().join("child.pid");
        std::fs::write(&fake, format!("#!/bin/sh\nsleep 30 &\necho $! > {}\nwait\n", pid_file.display())).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&fake, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        let t = Instant::now();
        let r = run(fake.to_str().unwrap(), None, &["x"], Opts { timeout: Duration::from_millis(500), ..Opts::read() }).await;
        assert_eq!(r, Err(GitError::Timeout));
        assert!(t.elapsed() < Duration::from_secs(5));
        tokio::time::sleep(Duration::from_millis(200)).await;
        let pid: i32 = std::fs::read_to_string(&pid_file).unwrap().trim().parse().unwrap();
        // SAFETY: signal 0 only asks whether the process exists
        let alive = unsafe { libc::kill(pid, 0) } == 0;
        assert!(!alive, "the grandchild was killed with the group");
    }

    #[tokio::test]
    async fn the_token_reaches_git_by_environment_only() {
        let tmp = tempfile::tempdir().unwrap();
        let fake = tmp.path().join("git");
        let seen = tmp.path().join("seen");
        // Records its argv and the config git would read from the environment
        std::fs::write(
            &fake,
            format!(
                "#!/bin/sh\nprintf '%s\\n' \"$@\" > {0}.argv\nenv | grep -E '^(GIT_|SSH_ASKPASS|GCM_)' | sort > {0}.env\n",
                seen.display()
            ),
        )
        .unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&fake, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        let token = format!("{}{}", ["gh", "s_"].concat(), "c4n4ry".repeat(6));
        let header = Zeroizing::new(format!("Basic {token}"));
        let auth = Auth::header("https://github.com/o/n.git", header).unwrap();
        assert!(!format!("{auth:?}").contains(&token), "Debug hides it");
        run(fake.to_str().unwrap(), Some(tmp.path()), &["fetch", "--", "https://github.com/o/n.git"], Opts { auth: Some(&auth), ..Opts::read() }).await.unwrap();
        let argv = std::fs::read_to_string(seen.with_extension("argv")).unwrap();
        let env = std::fs::read_to_string(seen.with_extension("env")).unwrap();
        assert!(!argv.contains(&token), "argv: {argv}");
        assert!(argv.contains("credential.helper=") && argv.contains("http.followRedirects=false"));
        assert!(argv.contains("protocol.allow=never") && !argv.contains("protocol.file.allow"));
        assert!(env.contains("GIT_CONFIG_COUNT=1\n"));
        assert!(env.contains("GIT_CONFIG_KEY_0=http.https://github.com/.extraheader\n"));
        assert!(env.contains(&format!("GIT_CONFIG_VALUE_0=Authorization: Basic {token}\n")));
        assert!(env.contains("GIT_TERMINAL_PROMPT=0\n"));
        let ours = ["GIT_CONFIG_COUNT", "GIT_CONFIG_KEY_0", "GIT_CONFIG_VALUE_0", "GIT_TERMINAL_PROMPT", "GIT_OPTIONAL_LOCKS", "GIT_LITERAL_PATHSPECS", "GIT_NO_REPLACE_OBJECTS", "GCM_INTERACTIVE", "SSH_ASKPASS_REQUIRE"];
        for name in env.lines().filter_map(|l| l.split('=').next()) {
            assert!(ours.contains(&name) || KEEP_ENV.contains(&name), "only kwant's own or the person's ssh/config choices reach git: {name}");
        }

        assert!(Auth::header("http://github.com/o/n.git", Zeroizing::new("Basic x".into())).is_none(), "never over plain http");
        assert!(Auth::header("ssh://git@github.com/o/n.git", Zeroizing::new("Basic x".into())).is_none());
        assert!(Auth::header("https://github.com/o/n.git", Zeroizing::new("Basic x\r\nX-Evil: 1".into())).is_none(), "one header line");
        assert!(Auth::header("http://127.0.0.1:9/o/n.git", Zeroizing::new("Basic x".into())).is_some(), "a test server on this machine");
    }

    #[test]
    fn diff_output_parses_like_githubs_files() {
        let raw = b":100644 100644 aaa bbb M\0src/a.rs\0:000000 100644 000 ccc A\0new.txt\0:100644 100644 ddd eee R087\0old.rs\0new.rs\0";
        let files = parse_raw(raw);
        assert_eq!(files.iter().map(|f| (f.path.as_str(), f.status.as_str(), f.previous_path.as_deref())).collect::<Vec<_>>(), [("src/a.rs", "modified", None), ("new.txt", "added", None), ("new.rs", "renamed", Some("old.rs"))]);
        assert_eq!(parse_numstat(b"3\t1\tsrc/a.rs\0-\t-\tnew.txt\x002\t0\t\0old.rs\0new.rs\0"), [(3, 1), (0, 0), (2, 0)]);
        let p = "diff --git a/x b/x\nindex 1..2 100644\n--- a/x\n+++ b/x\n@@ -1 +1 @@\n-a\n+b\ndiff --git a/y b/y\nBinary files a/y and b/y differ\ndiff --git a/z b/z\nold mode 100644\nnew mode 100755\n";
        assert_eq!(split_patches(p), [Some("@@ -1 +1 @@\n-a\n+b".to_string()), None, None]);
        assert!(split_patches("").is_empty());
    }
}
