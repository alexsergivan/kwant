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
//! - before git reaches a remote, its host is looked up and refused if any address is this machine
//!   or link-local ([`check_remote`]); over http(s) git is pinned to the addresses checked
//!   (`http.curloptResolve`), so DNS can't answer differently between the check and the connect;
//! - `nice 10`, a timeout that kills its whole process group, and capped output;
//! - a mirror over [`MIRROR_CAP`] on disk is deleted after the fetch that grew it, and fetched again
//!   at most once a day;
//! - stderr is only sorted into a [`GitError`], never logged or returned, so a URL in it goes
//!   nowhere.

use super::facts::FileDiff;
use super::source::{Commit, Person};
use chrono::{DateTime, Utc};
use std::collections::HashMap;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};
use tokio::io::{AsyncRead, AsyncReadExt};
use zeroize::Zeroizing;

/// The oldest git kwant runs: `GIT_CONFIG_COUNT` came in 2.31.
pub const MIN_VERSION: (u32, u32) = (2, 31);
/// What the desk says to do when the server has no usable git.
pub const INSTALL_HINT: &str = "Install git 2.31 or newer on the server.";
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
/// The most disk one repo's mirror may use (1 GB): one over it is deleted after the fetch.
pub const MIRROR_CAP: u64 = 1 << 30;
/// How long a mirror that was too big waits before it is fetched again (unless its repo is
/// removed and added again, which starts afresh).
pub const TOO_BIG_RETRY: Duration = Duration::from_secs(24 * 60 * 60);
/// How long looking a remote's host up may take.
const LOOKUP_TIMEOUT: Duration = Duration::from_secs(10);
/// The first git with `http.curloptResolve` (2.37), which pins a checked address.
const PIN_VERSION: (u32, u32) = (2, 37);

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
    /// The repo's mirror passed [`MIRROR_CAP`] and was deleted: it is fetched again after
    /// [`TOO_BIG_RETRY`], or when the repo is removed and added again.
    RepoTooBig,
    /// kwant refused the input before running git (a URL or ref it won't pass on).
    Refused,
    Failed,
}

impl std::fmt::Display for GitError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            // Wording from Mira's copy-073.md; the desk matches RepoTooBig by `poll_code`.
            GitError::NoGit => "Needs git 2.31 or newer on the server.",
            GitError::Auth => "The repo’s host said no. Check the server’s SSH key or git login for this repo.",
            GitError::NotFound => "Couldn’t find this repo or branch.",
            GitError::Unreachable => "Couldn’t reach the repo’s host.",
            GitError::Timeout => "The repo’s host took too long to answer.",
            GitError::TooBig => "This repo is too big for Dev mode to read.",
            GitError::RepoTooBig => "This repo is too big for Dev mode to read (over 1 GB).",
            GitError::Refused => "Dev mode can’t use this repo address or branch name.",
            GitError::Failed => "Couldn’t read this repo.",
        })
    }
}

impl std::error::Error for GitError {}

impl GitError {
    /// The `poll_code` for a row whose `poll_error` is `message`: `too_big` for either too-big
    /// error (a mirror over the cap, or one answer from git over [`MAX_OUT`]), so the desk never
    /// matches strings; `None` for the rest.
    pub fn poll_code(message: &str) -> Option<&'static str> {
        [GitError::RepoTooBig, GitError::TooBig].iter().any(|e| e.to_string() == message).then_some("too_big")
    }
}

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

/// The `core.sshCommand` every call gets (G2 review F1): plain `ssh` from `PATH`, or the admin's
/// `GIT_SSH` program, so a repo's own config can never choose the command. (An empty value would
/// make git try to run `''` and break every ssh remote; `GIT_SSH_COMMAND` still wins over this.)
fn ssh_command(git_ssh: Option<&std::ffi::OsStr>) -> String {
    match git_ssh.and_then(|s| s.to_str()).filter(|s| !s.is_empty()) {
        // Single-quoted for the shell git runs it with: a `'` inside becomes `'\''`
        Some(prog) => format!("core.sshCommand='{}'", prog.replace('\'', "'\\''")),
        None => "core.sshCommand=ssh".to_string(),
    }
}

#[derive(Clone, Copy)]
struct Opts<'a> {
    timeout: Duration,
    max_out: usize,
    auth: Option<&'a Auth>,
    /// A team folder: the file protocol is allowed (git reads its own objects).
    file: bool,
    /// `-c` settings from [`check_remote`] that pin git to the addresses it checked.
    pin: &'a [String],
    /// A fetch into a mirror: no file git writes may pass this many bytes (`RLIMIT_FSIZE`), and
    /// what it fetches is kept as one pack, so a fetch stops at the cap instead of after it (G2
    /// review F3). Passing it is [`GitError::RepoTooBig`]. The limit is per file: the pack, its
    /// `.idx` and its `.rev` can each reach it, so one fetch can briefly write about 3× the cap.
    fsize: Option<u64>,
}

impl Opts<'static> {
    fn read() -> Self {
        Opts { timeout: READ_TIMEOUT, max_out: MAX_OUT, auth: None, file: false, pin: &[], fsize: None }
    }
}

/// Runs `git [--git-dir <dir>] <args>` as described in the module docs. Its stdout on success.
async fn run(bin: &str, git_dir: Option<&Path>, args: &[&str], o: Opts<'_>) -> Result<Vec<u8>, GitError> {
    run_with(bin, git_dir, args, o, false).await.map(|(out, _)| out)
}

/// [`run`]; with `head`, stdout past `o.max_out` isn't an error: reading stops there (git then
/// stops at a closed pipe) and the first `o.max_out` bytes come back with `true`.
///
/// A remote that answers with a redirect is asked once more at the same address with `.git`
/// when it had none (N-4: GitLab sends `/group/repo` to `/group/repo.git`, and git is never let
/// follow a redirect itself). Any other redirect is [`GitError::Failed`], as before.
async fn run_with(bin: &str, git_dir: Option<&Path>, args: &[&str], o: Opts<'_>, head: bool) -> Result<(Vec<u8>, bool), GitError> {
    match run_once(bin, git_dir, args, o, head).await {
        Err(Ran::Moved) => {
            let Some((i, url)) = args.iter().position(|a| *a == "--").and_then(|i| Some((i + 1, with_dot_git(args.get(i + 1)?)?))) else {
                return Err(GitError::Failed);
            };
            let mut again = args.to_vec();
            again[i] = &url;
            run_once(bin, git_dir, &again, o, head).await.map_err(Ran::error)
        }
        r => r.map_err(Ran::error),
    }
}

/// How one run of git failed: a [`GitError`], or a redirect from the remote.
enum Ran {
    Git(GitError),
    Moved,
}

impl Ran {
    fn error(self) -> GitError {
        match self {
            Ran::Git(e) => e,
            Ran::Moved => GitError::Failed,
        }
    }
}

impl From<GitError> for Ran {
    fn from(e: GitError) -> Self {
        Ran::Git(e)
    }
}

/// Whether git stopped at an HTTP redirect it wasn't allowed to follow.
fn redirected(stderr: &[u8]) -> bool {
    let e = String::from_utf8_lossy(stderr).to_lowercase();
    ["301", "302", "303", "307", "308"].iter().any(|c| e.contains(&format!("the requested url returned error: {c}")))
}

/// `url` with `.git` added, for an http(s) [`fetch_url`] whose path doesn't end in it: the same
/// scheme, host and port, so the address check, its pin and a token's origin all still hold.
/// `None` for anything else (ssh, scp-like, a path that already ends in `.git`, no path).
fn with_dot_git(url: &str) -> Option<String> {
    let lower = url.to_ascii_lowercase();
    if !(lower.starts_with("https://") || lower.starts_with("http://")) || fetch_url(url).as_deref() != Some(url) {
        return None;
    }
    let base = url.trim_end_matches('/');
    if base.to_ascii_lowercase().ends_with(".git") {
        return None;
    }
    let next = format!("{base}.git");
    (fetch_url(&next).as_deref() == Some(next.as_str()) && remote_host(&next) == remote_host(url)).then_some(next)
}

/// The one way kwant starts git on this computer (agents in the VM, D4c; S-13). A folder a bot
/// wrote can hold a `.git/config` it planted, so every call gets, whatever that config says:
/// `GIT_CONFIG_NOSYSTEM=1`, `-c core.fsmonitor=false -c core.hooksPath=/dev/null -c
/// core.pager=cat`, no `GIT_*` from kwant's environment except the person's ssh and global-config
/// choices ([`KEEP_ENV`]), no prompt and no stdin. kwant only runs git's built-in commands, which
/// an alias can't replace, so no alias ever runs. A CI test checks that nothing else in `src/`
/// starts git.
pub fn host_git_command(bin: &str) -> tokio::process::Command {
    let mut cmd = tokio::process::Command::new(bin);
    cmd.args(["-c", "core.fsmonitor=false", "-c", "core.hooksPath=/dev/null", "-c", "core.pager=cat"]);
    for (k, _) in std::env::vars_os() {
        if let Some(k) = k.to_str() {
            if k.starts_with("GIT_") && !KEEP_ENV.contains(&k) {
                cmd.env_remove(k);
            }
        }
    }
    cmd.env("GIT_CONFIG_NOSYSTEM", "1").env("GIT_TERMINAL_PROMPT", "0").stdin(std::process::Stdio::null());
    cmd
}

async fn run_once(bin: &str, git_dir: Option<&Path>, args: &[&str], o: Opts<'_>, head: bool) -> Result<(Vec<u8>, bool), Ran> {
    let mut cmd = host_git_command(bin);
    if let Some(d) = git_dir {
        cmd.arg("--git-dir").arg(d);
    }
    cmd.arg("--no-pager");
    for c in SAFE_CONFIG {
        cmd.arg("-c").arg(c);
    }
    cmd.arg("-c").arg(ssh_command(std::env::var_os("GIT_SSH").as_deref()));
    if o.fsize.is_some() {
        // One pack file, not loose objects each under the limit on their own
        cmd.args(["-c", "fetch.unpackLimit=1", "-c", "transfer.unpackLimit=1"]);
    }
    if o.file {
        cmd.args(["-c", "protocol.file.allow=always"]);
    }
    // No redirect: it would take git to a host the address check never saw (and, with a token,
    // the token with it)
    cmd.args(["-c", "http.followRedirects=false"]);
    if o.auth.is_some() {
        // The token goes to this origin only: no helper asked or told
        cmd.args(["-c", "credential.helper="]);
    }
    for c in o.pin {
        cmd.arg("-c").arg(c);
    }
    cmd.args(args);
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
    let fsize = o.fsize;
    #[cfg(unix)]
    // SAFETY: setsid(2), nice(2), setrlimit(2) and signal(2) are async-signal-safe and touch no
    // memory of this process
    unsafe {
        cmd.pre_exec(move || {
            // Its own session: no controlling terminal for ssh to prompt on, and one process
            // group the timeout can kill whole
            libc::setsid();
            libc::nice(10);
            if let Some(cap) = fsize {
                // A write past the cap stops git (SIGXFSZ, inherited by index-pack)
                let lim = libc::rlimit { rlim_cur: cap as libc::rlim_t, rlim_max: cap as libc::rlim_t };
                if libc::setrlimit(libc::RLIMIT_FSIZE, &lim) != 0 {
                    return Err(std::io::Error::last_os_error());
                }
                libc::signal(libc::SIGXFSZ, libc::SIG_DFL);
                // ...without a core file
                let none = libc::rlimit { rlim_cur: 0, rlim_max: 0 };
                libc::setrlimit(libc::RLIMIT_CORE, &none);
            }
            Ok(())
        });
    }
    // Windows has no setsid, nice or RLIMIT_FSIZE: git runs below normal priority with no
    // console window, and a mirror's size is checked after each fetch (`Repo::cap`) instead of
    // stopping the write at the cap.
    #[cfg(windows)]
    cmd.creation_flags(CREATE_NO_WINDOW | BELOW_NORMAL_PRIORITY_CLASS);
    let mut child = cmd.spawn().map_err(|e| if e.kind() == std::io::ErrorKind::NotFound { GitError::NoGit } else { GitError::Failed })?;
    let mut group = crate::drivers::cli::ProcessGroup::of(&child);
    let (so, se) = (child.stdout.take(), child.stderr.take());
    let work = async {
        let (out, err) = tokio::try_join!(read_head(so, o.max_out, head), async { Ok(drain(se, MAX_ERR).await) })?;
        let status = child.wait().await.map_err(|_| GitError::Failed)?;
        Ok::<_, GitError>((out, err, status))
    };
    let ((out, cut), err, status) = tokio::time::timeout(o.timeout, work).await.map_err(|_| GitError::Timeout)??;
    group.disarm();
    if cut {
        Ok((out, true))
    } else if status.success() {
        Ok((out, false))
    } else if o.fsize.is_some() && hit_size_limit(&status, &err) {
        Err(GitError::RepoTooBig.into())
    } else if redirected(&err) {
        Err(Ran::Moved)
    } else {
        Err(classify(&err).into())
    }
}

#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;
#[cfg(windows)]
const BELOW_NORMAL_PRIORITY_CLASS: u32 = 0x0000_4000;

/// Whether a size-limited git (or the index-pack it ran) stopped at `RLIMIT_FSIZE`: killed by
/// SIGXFSZ, or told "File too large" if that signal is ignored.
fn hit_size_limit(status: &std::process::ExitStatus, stderr: &[u8]) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::process::ExitStatusExt;
        if status.signal() == Some(libc::SIGXFSZ) {
            return true;
        }
    }
    let _ = status;
    let e = String::from_utf8_lossy(stderr).to_lowercase();
    #[cfg(unix)]
    if e.contains(&format!("died of signal {}", libc::SIGXFSZ)) {
        return true;
    }
    e.contains("file too large")
}

/// [`read_capped`]; with `head`, the first `cap` bytes and `true` once it passes `cap` (and `r`
/// is closed, so the writer stops).
async fn read_head<R: AsyncRead + Unpin>(r: Option<R>, cap: usize, head: bool) -> Result<(Vec<u8>, bool), GitError> {
    if !head {
        return read_capped(r, cap).await.map(|out| (out, false));
    }
    let Some(mut r) = r else { return Ok((Vec::new(), false)) };
    let mut out = Vec::new();
    let mut buf = vec![0u8; 64 << 10];
    loop {
        let n = r.read(&mut buf).await.map_err(|_| GitError::Failed)?;
        if n == 0 {
            return Ok((out, false));
        }
        if out.len() + n > cap {
            let room = cap - out.len();
            out.extend_from_slice(&buf[..room]);
            return Ok((out, true));
        }
        out.extend_from_slice(&buf[..n]);
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

/// The server's git version, if it has one kwant can use ([`MIN_VERSION`] or newer). A yes is kept;
/// a no is asked again after a minute, so installing git needs no restart.
async fn usable_version() -> Option<(u32, u32)> {
    // The last answer and when it was asked
    type Seen = Option<(Option<(u32, u32)>, Instant)>;
    static SEEN: OnceLock<Mutex<Seen>> = OnceLock::new();
    let seen = SEEN.get_or_init(Default::default);
    if let Some((v, at)) = *seen.lock().unwrap() {
        if v.is_some() || at.elapsed() < Duration::from_secs(60) {
            return v;
        }
    }
    let v = version("git").await.filter(|v| *v >= MIN_VERSION);
    *seen.lock().unwrap() = Some((v, Instant::now()));
    v
}

/// Whether the server has a git kwant can use ([`MIN_VERSION`] or newer).
pub async fn available() -> bool {
    usable_version().await.is_some()
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
    match url::Host::parse(&remote_host(url).0) {
        Ok(url::Host::Domain(d)) => {
            let d = d.trim_end_matches('.').to_ascii_lowercase();
            d == "localhost" || d.ends_with(".localhost")
        }
        Ok(url::Host::Ipv4(ip)) => blocked(IpAddr::V4(ip)),
        Ok(url::Host::Ipv6(ip)) => blocked(IpAddr::V6(ip)),
        Err(_) => true,
    }
}

/// The host of a [`fetch_url`] (`[v6]` keeps its brackets) and its port, if it names one.
fn remote_host(url: &str) -> (String, Option<u16>) {
    match url.find("://") {
        Some(i) => {
            let rest = &url[i + 3..];
            let rest = &rest[..rest.find('/').unwrap_or(rest.len())];
            let rest = rest.rsplit_once('@').map_or(rest, |(_, h)| h);
            // `host:port` or `[v6]:port`
            match rest.strip_prefix('[') {
                Some(r) => {
                    let end = r.find(']').unwrap_or(r.len());
                    (format!("[{}]", &r[..end]), r[end..].strip_prefix("]:").and_then(|p| p.parse().ok()))
                }
                None => {
                    let mut it = rest.splitn(2, ':');
                    (it.next().unwrap_or("").to_string(), it.next().and_then(|p| p.parse().ok()))
                }
            }
        }
        None => {
            let h = url.split_once(':').map_or(url, |(h, _)| h);
            (h.rsplit_once('@').map_or(h, |(_, h)| h).to_string(), None)
        }
    }
}

/// Cloud metadata services outside link-local (G2 review F6): Alibaba's `100.100.100.200` and AWS's
/// IPv6 `fd00:ec2::254`. Only these addresses, not their ranges: `100.64.0.0/10` is Tailscale.
const METADATA_V4: Ipv4Addr = Ipv4Addr::new(100, 100, 100, 200);
const METADATA_V6: Ipv6Addr = Ipv6Addr::new(0xfd00, 0xec2, 0, 0, 0, 0, 0, 0x254);

/// An address kwant never lets git connect to: this machine (loopback, unspecified, `0.0.0.0/8`),
/// link-local (`169.254.0.0/16`, where cloud metadata lives, and `fe80::/10`), broadcast, the cloud
/// metadata addresses outside link-local ([`METADATA_V4`], [`METADATA_V6`]), or any of these inside
/// an IPv6 address (IPv4-mapped, IPv4-compatible, NAT64 `64:ff9b::/96`). Private LAN ranges are
/// allowed: people run Gitea at home.
pub fn blocked(ip: IpAddr) -> bool {
    let v4 = |ip: Ipv4Addr| ip.is_loopback() || ip.is_link_local() || ip.is_unspecified() || ip.is_broadcast() || ip.octets()[0] == 0 || ip == METADATA_V4;
    match ip {
        IpAddr::V4(ip) => v4(ip),
        IpAddr::V6(ip) => {
            let s = ip.segments();
            let nat64 = s[..6] == [0x64, 0xff9b, 0, 0, 0, 0];
            let embedded = |ip: Ipv6Addr| {
                let o = ip.octets();
                Ipv4Addr::new(o[12], o[13], o[14], o[15])
            };
            ip.is_loopback()
                || ip.is_unspecified()
                || ip == METADATA_V6
                || (s[0] & 0xffc0) == 0xfe80
                || ip.to_ipv4_mapped().is_some_and(v4)
                || (s[..6] == [0; 6] && v4(embedded(ip)))
                || (nat64 && v4(embedded(ip)))
        }
    }
}

/// The most branches [`Repo::heads`] lists.
pub const MAX_HEADS: usize = 200;

/// Looks a remote's host up: the seam [`check_remote`] is tested through, with no network.
#[async_trait::async_trait]
pub trait Resolver: Send + Sync {
    async fn lookup(&self, host: &str, port: u16) -> Result<Vec<IpAddr>, GitError>;
}

/// The system's resolver, with a timeout.
pub struct SystemResolver;

#[async_trait::async_trait]
impl Resolver for SystemResolver {
    async fn lookup(&self, host: &str, port: u16) -> Result<Vec<IpAddr>, GitError> {
        let found = tokio::time::timeout(LOOKUP_TIMEOUT, tokio::net::lookup_host((host, port))).await.map_err(|_| GitError::Unreachable)?.map_err(|_| GitError::Unreachable)?;
        Ok(found.map(|a| a.ip()).collect())
    }
}

/// Checks a [`fetch_url`] right before git reaches it (any-git.md G2): its host is looked up, and
/// refused ([`GitError::Refused`]) if it is, or any address it has is, [`blocked`]. Over http(s),
/// with git [`PIN_VERSION`] or newer, the answer is the `-c http.curloptResolve=<host>:<port>:<addrs>`
/// that pins git to the addresses checked, so DNS can't change between the check and the connect;
/// over ssh and git://, or on an older git, the check stands alone.
pub async fn check_remote(url: &str, resolver: &dyn Resolver, git: Option<(u32, u32)>) -> Result<Vec<String>, GitError> {
    if fetch_url(url).as_deref() != Some(url) || on_this_machine(url) {
        return Err(GitError::Refused);
    }
    let (host, port) = remote_host(url);
    let scheme = url.find("://").map(|i| &url[..i]);
    let port = port.unwrap_or(match scheme {
        Some("https") => 443,
        Some("http") => 80,
        Some("git") => 9418,
        _ => 22,
    });
    let addrs = match url::Host::parse(&host).map_err(|_| GitError::Refused)? {
        // A literal address needs no lookup and no pin: on_this_machine checked it
        url::Host::Ipv4(_) | url::Host::Ipv6(_) => return Ok(Vec::new()),
        url::Host::Domain(d) => resolver.lookup(&d, port).await?,
    };
    if addrs.is_empty() {
        return Err(GitError::Unreachable);
    }
    if addrs.iter().any(|a| blocked(*a)) {
        return Err(GitError::Refused);
    }
    if !matches!(scheme, Some("https" | "http")) || git.is_none_or(|v| v < PIN_VERSION) {
        return Ok(Vec::new());
    }
    let list: Vec<String> = addrs.iter().map(|a| if a.is_ipv6() { format!("[{a}]") } else { a.to_string() }).collect();
    Ok(vec![format!("http.curloptResolve={host}:{port}:{}", list.join(","))])
}

/// [`check_remote`] with the system's resolver and this server's git.
async fn checked(url: &str) -> Result<Vec<String>, GitError> {
    check_remote(url, &SystemResolver, usable_version().await).await
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

/// The note left beside a mirror deleted for passing [`MIRROR_CAP`]: `<mirror>.too-big`. Its
/// age is when that was.
pub fn too_big_mark(mirror: &Path) -> PathBuf {
    let mut name = mirror.file_name().unwrap_or_default().to_os_string();
    name.push(".too-big");
    mirror.with_file_name(name)
}

/// What `dir` and everything under it take on disk (links not followed).
fn disk_use(dir: &Path) -> u64 {
    let mut total = 0;
    let mut todo = vec![dir.to_path_buf()];
    while let Some(d) = todo.pop() {
        let Ok(list) = std::fs::read_dir(&d) else { continue };
        for e in list.flatten() {
            let Ok(m) = e.path().symlink_metadata() else { continue };
            if m.is_dir() {
                todo.push(e.path());
            } else {
                total += m.len();
            }
        }
    }
    total
}

/// The biggest single file under `dir` (links not followed).
fn largest_file(dir: &Path) -> u64 {
    let mut most = 0;
    let mut todo = vec![dir.to_path_buf()];
    while let Some(d) = todo.pop() {
        let Ok(list) = std::fs::read_dir(&d) else { continue };
        for e in list.flatten() {
            let Ok(m) = e.path().symlink_metadata() else { continue };
            if m.is_dir() {
                todo.push(e.path());
            } else {
                most = most.max(m.len());
            }
        }
    }
    most
}

/// Deletes the mirror of a repo no longer watched (its cards stay), and any note that it was too
/// big, so adding the repo again fetches it afresh.
pub async fn remove_mirror(data_dir: &Path, space: &str, key: &str) {
    let dir = mirror_dir(data_dir, space, key);
    let _held = lock(&dir).await;
    let _ = tokio::fs::remove_file(too_big_mark(&dir)).await;
    if dir.is_dir() {
        if let Err(e) = tokio::fs::remove_dir_all(&dir).await {
            tracing::warn!(space = %space, "dev: could not remove a repo mirror: {}", e.kind());
        }
    }
}

/// The git dir of team folder `folder`, only if it is that folder's own (G2 review F2): the folder
/// is writable by anyone working in it, so its `.git` could otherwise point kwant at another
/// space's repo or at one of kwant's own mirrors, and its cards would show that repo. Refused
/// ([`GitError::Refused`]): everything [`own_git_dir`] refuses, and a git dir, or its main repo,
/// under `<data>/dev` (kwant's mirrors). No `.git` at all is [`GitError::NotFound`].
pub fn local_git_dir(folder: &Path, data_dir: &Path) -> Result<PathBuf, GitError> {
    let (git_dir, common) = own_git_dirs(folder)?;
    if let Ok(mirrors) = std::fs::canonicalize(data_dir.join("dev")) {
        if git_dir.starts_with(&mirrors) || common.starts_with(&mirrors) {
            return Err(GitError::Refused);
        }
    }
    Ok(git_dir)
}

/// The git dir of `folder` if it is that folder's own (G2 delta D1; team detection reads a
/// folder's remote and branch only through this). Refused ([`GitError::Refused`]):
/// - a `.git` that is a symlink;
/// - a `.git` file (a linked worktree) unless the git dir it names points back at this folder's
///   `.git` (its `gitdir` file, which `git worktree add` writes) and sits in its main repo's
///   `worktrees/`, so only that repo can have made it (a submodule checkout is refused);
/// - a `.git` directory with a `commondir` (it would borrow another repo's objects and refs);
/// - `objects/info/alternates`, and any symlink among the objects and refs git reads.
///
/// No `.git` at all is [`GitError::NotFound`].
pub fn own_git_dir(folder: &Path) -> Result<PathBuf, GitError> {
    own_git_dirs(folder).map(|(git_dir, _)| git_dir)
}

/// [`own_git_dir`] and the main repo's git dir (the same dir unless `folder` is a worktree).
fn own_git_dirs(folder: &Path) -> Result<(PathBuf, PathBuf), GitError> {
    let folder = std::fs::canonicalize(folder).map_err(|_| GitError::NotFound)?;
    let dot_git = folder.join(".git");
    let meta = std::fs::symlink_metadata(&dot_git).map_err(|_| GitError::NotFound)?;
    let canon = |p: &Path| std::fs::canonicalize(p).map_err(|_| GitError::Refused);
    let (git_dir, common) = if meta.file_type().is_symlink() {
        return Err(GitError::Refused);
    } else if meta.is_dir() {
        if dot_git.join("commondir").exists() {
            return Err(GitError::Refused);
        }
        (dot_git.clone(), dot_git.clone())
    } else if meta.is_file() {
        // `gitdir: <main>/.git/worktrees/<name>`, relative to the folder if not absolute
        let text = read_small(&dot_git)?;
        let target = text.trim_end_matches(['\n', '\r']).strip_prefix("gitdir: ").filter(|t| !t.is_empty()).ok_or(GitError::Refused)?;
        let git_dir = canon(&folder.join(target))?;
        // git writes `<git dir>/gitdir` = this worktree's `.git` file
        let back = read_small(&git_dir.join("gitdir"))?;
        let back = Path::new(back.trim_end_matches(['\n', '\r']));
        let back = if back.is_absolute() { back.to_path_buf() } else { git_dir.join(back) };
        if canon(&back)? != dot_git {
            return Err(GitError::Refused);
        }
        let common = read_small(&git_dir.join("commondir"))?;
        let common = Path::new(common.trim_end_matches(['\n', '\r']));
        let common = canon(&if common.is_absolute() { common.to_path_buf() } else { git_dir.join(common) })?;
        if git_dir.parent() != Some(common.join("worktrees").as_path()) {
            return Err(GitError::Refused);
        }
        (git_dir, common)
    } else {
        return Err(GitError::Refused);
    };
    for d in [&git_dir, &common] {
        if d.join("objects/info/alternates").symlink_metadata().is_ok() || has_symlink(d) {
            return Err(GitError::Refused);
        }
    }
    Ok((git_dir, common))
}


/// A small text file (a `.git` file, `gitdir`, `commondir`), refused if it is a symlink or big.
fn read_small(p: &Path) -> Result<String, GitError> {
    let m = std::fs::symlink_metadata(p).map_err(|_| GitError::Refused)?;
    if !m.is_file() || m.len() > 4096 {
        return Err(GitError::Refused);
    }
    std::fs::read_to_string(p).map_err(|_| GitError::Refused)
}

/// Whether any of the entries git reads its history from in `git_dir` (`HEAD`, `packed-refs`,
/// `objects/`, `refs/`) is, or holds, a symlink.
fn has_symlink(git_dir: &Path) -> bool {
    let mut todo = Vec::new();
    for name in ["HEAD", "packed-refs", "objects", "refs"] {
        let p = git_dir.join(name);
        match p.symlink_metadata() {
            Ok(m) if m.file_type().is_symlink() => return true,
            Ok(m) if m.is_dir() => todo.push(p),
            _ => {}
        }
    }
    while let Some(d) = todo.pop() {
        let Ok(list) = std::fs::read_dir(&d) else { continue };
        for e in list.flatten() {
            match e.file_type() {
                Ok(t) if t.is_symlink() => return true,
                Ok(t) if t.is_dir() => todo.push(e.path()),
                _ => {}
            }
        }
    }
    false
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

    /// A team folder's own git, read in place: `<folder>/.git` (a directory, or a worktree's file),
    /// once [`local_git_dir`] has checked it's really this folder's. Called on every poll, since
    /// anyone working in the folder can change it.
    pub async fn local(folder: &Path, data_dir: &Path) -> Result<Repo, GitError> {
        let (folder, data_dir) = (folder.to_path_buf(), data_dir.to_path_buf());
        let dir = tokio::task::spawn_blocking(move || local_git_dir(&folder, &data_dir)).await.map_err(|_| GitError::Failed)??;
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
        self.not_too_big().await?;
        let pin = checked(url).await?;
        let heads = format!("+refs/heads/{branch}:refs/heads/{branch}");
        let args = ["fetch", "--quiet", "--no-tags", "--no-write-fetch-head", "--no-recurse-submodules", "--prune", "--", url, &heads, "+refs/tags/*:refs/tags/*"];
        self.fetch_capped(&args, Opts { timeout: FETCH_TIMEOUT, max_out: MAX_OUT, auth, file: false, pin: &pin, fsize: None }, crate::server::dev::mirror_cap()).await
    }

    /// Runs one `git fetch` into this mirror with no file allowed past `cap` bytes, then
    /// [`Self::cap`]: a fetch stopped at the cap, or a mirror over it, ends as
    /// [`GitError::RepoTooBig`] with the mirror deleted and its note left.
    async fn fetch_capped(&self, args: &[&str], o: Opts<'_>, cap: u64) -> Result<(), GitError> {
        let mut done = run("git", Some(&self.dir), args, Opts { fsize: Some(cap), ..o }).await.map(|_| ());
        if done.is_err() && done != Err(GitError::RepoTooBig) {
            // git 2.43 says only "invalid index-pack output" when the limit stops index-pack: a
            // file left at the cap (within one write) is the sign
            let dir = self.dir.clone();
            if tokio::task::spawn_blocking(move || largest_file(&dir)).await.unwrap_or(0) >= cap.saturating_sub(64 << 10) {
                done = Err(GitError::RepoTooBig);
            }
        }
        self.cap(cap).await?;
        if done == Err(GitError::RepoTooBig) {
            // Stopped at the limit, though what it left (if anything) is under the cap
            self.too_big().await;
        }
        done
    }

    /// [`GitError::RepoTooBig`] while this mirror's note says it passed the cap less than
    /// [`TOO_BIG_RETRY`] ago; an older note is cleared, so this fetch tries again.
    async fn not_too_big(&self) -> Result<(), GitError> {
        let mark = too_big_mark(&self.dir);
        let Ok(m) = tokio::fs::metadata(&mark).await else { return Ok(()) };
        let age = m.modified().ok().and_then(|t| t.elapsed().ok()).unwrap_or_default();
        if age < TOO_BIG_RETRY {
            return Err(GitError::RepoTooBig);
        }
        let _ = tokio::fs::remove_file(&mark).await;
        Ok(())
    }

    /// After a fetch: a mirror over `cap` bytes on disk is deleted and a note left beside it, so
    /// it isn't fetched again for [`TOO_BIG_RETRY`] ([`GitError::RepoTooBig`]).
    async fn cap(&self, cap: u64) -> Result<(), GitError> {
        if self.local {
            return Ok(());
        }
        let dir = self.dir.clone();
        let used = tokio::task::spawn_blocking(move || disk_use(&dir)).await.unwrap_or(0);
        if used <= cap {
            return Ok(());
        }
        self.too_big().await;
        Err(GitError::RepoTooBig)
    }

    /// Deletes this mirror and leaves the note that it was too big.
    async fn too_big(&self) {
        if let Err(e) = tokio::fs::remove_dir_all(&self.dir).await {
            if e.kind() != std::io::ErrorKind::NotFound {
                tracing::warn!("dev: could not remove a repo mirror over the size cap: {}", e.kind());
            }
        }
        let _ = tokio::fs::write(too_big_mark(&self.dir), b"").await;
    }

    /// Fetches one ref of `url` (a PR head, `refs/pull/7/head`) into `to` (`refs/kwant/…`); the sha
    /// it now points at.
    pub async fn fetch_ref(&self, url: &str, from: &str, to: &str, auth: Option<&Auth>) -> Result<String, GitError> {
        if self.local || !ok_ref(from) || !to.starts_with("refs/kwant/") || !ok_ref(to) || fetch_url(url).as_deref() != Some(url) {
            return Err(GitError::Refused);
        }
        self.not_too_big().await?;
        let pin = checked(url).await?;
        let spec = format!("+{from}:{to}");
        let args = ["fetch", "--quiet", "--no-tags", "--no-write-fetch-head", "--no-recurse-submodules", "--", url, &spec];
        self.fetch_capped(&args, Opts { timeout: FETCH_TIMEOUT, max_out: MAX_OUT, auth, file: false, pin: &pin, fsize: None }, crate::server::dev::mirror_cap()).await?;
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
        self.default_branch_within(url, auth, REMOTE_TIMEOUT).await
    }

    /// [`Self::default_branch`], with the remote's `ls-remote` given at most `timeout` (Suggest
    /// gives it what is left of its 10 s, R-HANG).
    pub async fn default_branch_within(&self, url: &str, auth: Option<&Auth>, timeout: Duration) -> Result<String, GitError> {
        let branch = if self.local {
            self.text(&["symbolic-ref", "--quiet", "HEAD"]).await?
        } else {
            if fetch_url(url).as_deref() != Some(url) {
                return Err(GitError::Refused);
            }
            let pin = checked(url).await?;
            let out = run("git", Some(&self.dir), &["ls-remote", "--symref", "--", url, "HEAD"], Opts { timeout, max_out: 1 << 16, auth, file: false, pin: &pin, fsize: None }).await?;
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
        let pin = checked(url).await?;
        let out = run("git", Some(&self.dir), &["ls-remote", "--heads", "--", url, &name], Opts { timeout: REMOTE_TIMEOUT, max_out: 1 << 16, auth, file: false, pin: &pin, fsize: None }).await?;
        Ok(String::from_utf8_lossy(&out).lines().any(|l| l.split('\t').nth(1) == Some(name.as_str())))
    }

    /// The branches of `url` (or of a team folder) and their heads, at most [`MAX_HEADS`], sorted
    /// by name; names [`ok_branch`] refuses are left out. A mirror asks the remote (`ls-remote`,
    /// through the same pin as a fetch) without fetching anything.
    pub async fn heads(&self, url: &str, auth: Option<&Auth>) -> Result<Vec<(String, String)>, GitError> {
        let out = if self.local {
            self.text(&["for-each-ref", "--format=%(objectname)\t%(refname)", "refs/heads/"]).await?
        } else {
            if fetch_url(url).as_deref() != Some(url) {
                return Err(GitError::Refused);
            }
            let pin = checked(url).await?;
            let out = run("git", Some(&self.dir), &["ls-remote", "--heads", "--", url], Opts { timeout: REMOTE_TIMEOUT, max_out: 1 << 20, auth, file: false, pin: &pin, fsize: None }).await?;
            String::from_utf8_lossy(&out).into_owned()
        };
        let mut heads: Vec<(String, String)> = out
            .lines()
            .filter_map(|l| {
                let (sha, name) = l.split_once('\t')?;
                let name = name.trim().strip_prefix("refs/heads/")?;
                (is_sha(sha.trim()) && ok_branch(name)).then(|| (name.to_string(), sha.trim().to_string()))
            })
            .collect();
        heads.sort();
        heads.truncate(MAX_HEADS);
        Ok(heads)
    }

    /// The committer dates of those of `shas` this repo has, by sha, in one `git log --no-walk`.
    /// A sha it doesn't have (a mirror that never fetched it) is left out, as is one whose date
    /// doesn't read; a name that isn't a sha is refused.
    pub async fn commit_dates(&self, shas: &[&str]) -> Result<std::collections::HashMap<String, DateTime<Utc>>, GitError> {
        if shas.iter().any(|s| !is_sha(s)) {
            return Err(GitError::Refused);
        }
        let mut dates = std::collections::HashMap::new();
        for chunk in shas.chunks(MAX_HEADS) {
            let mut args = vec!["log", "--no-walk=unsorted", "--ignore-missing", "--format=%H %ct", "--end-of-options"];
            args.extend_from_slice(chunk);
            args.push("--");
            let out = self.text(&args).await?;
            for line in out.lines() {
                let Some((sha, at)) = line.trim().split_once(' ') else { continue };
                let Some(at) = at.parse::<i64>().ok().and_then(|t| DateTime::from_timestamp(t, 0)) else { continue };
                if is_sha(sha) {
                    dates.insert(sha.to_string(), at);
                }
            }
        }
        Ok(dates)
    }

    /// Whether `a` is `b` or an ancestor of it.
    pub async fn is_ancestor(&self, a: &str, b: &str) -> Result<bool, GitError> {
        if !(is_sha(a) || ok_ref(a)) || !(is_sha(b) || ok_ref(b)) {
            return Err(GitError::Refused);
        }
        match self.git(&["merge-base", "--is-ancestor", "--end-of-options", a, b]).await {
            Ok(_) => Ok(true),
            Err(GitError::Failed) => Ok(false),
            Err(e) => Err(e),
        }
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
        // Hunks: each file takes its own section, found by path (a type change prints two)
        if let Ok(p) = self.git(&with(&["-p", "--no-color", "--no-ext-diff", "--no-textconv", "--src-prefix=a/", "--dst-prefix=b/"])).await {
            match_patches(&mut files, split_patches(&String::from_utf8_lossy(&p)));
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

    /// The commit `HEAD` names (a team folder's checkout, a mirror's default branch). `NotFound`
    /// for a repo with no commits yet.
    pub async fn head(&self) -> Result<String, GitError> {
        let sha = match self.text(&["rev-parse", "--verify", "--quiet", "--end-of-options", "HEAD^{commit}"]).await {
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

    /// Every path tracked at `rev` (a full sha), as `ls-tree -r -l -z` records
    /// (`<mode> <type> <id> <size>\t<path>`, size `-` for a submodule), up to `max_out` bytes of
    /// them: `true` when there were more (D2's path limit). A record cut at the limit is the
    /// caller's to drop.
    pub async fn tree(&self, rev: &str, max_out: usize, timeout: Duration) -> Result<(Vec<u8>, bool), GitError> {
        if !is_sha(rev) {
            return Err(GitError::Refused);
        }
        let o = Opts { file: self.local, max_out, timeout, ..Opts::read() };
        run_with("git", Some(&self.dir), &["ls-tree", "-r", "-l", "-z", "--full-tree", "--end-of-options", rev], o, true).await
    }

    /// The bytes of blob `id` (a full sha from [`Self::tree`]), or [`GitError::TooBig`] past `max_out`.
    pub async fn blob(&self, id: &str, max_out: usize, timeout: Duration) -> Result<Vec<u8>, GitError> {
        if !is_sha(id) {
            return Err(GitError::Refused);
        }
        run("git", Some(&self.dir), &["cat-file", "blob", id], Opts { file: self.local, max_out, timeout, ..Opts::read() }).await
    }

    /// Fetches only the tip of `branch` from `url` (depth 1, no tags) into this mirror, within
    /// `timeout`, and returns its sha: what a team Suggest reads (D2, D7). The mirror cap applies.
    pub async fn fetch_tip(&self, url: &str, branch: &str, auth: Option<&Auth>, timeout: Duration) -> Result<String, GitError> {
        if self.local || !ok_branch(branch) || fetch_url(url).as_deref() != Some(url) {
            return Err(GitError::Refused);
        }
        let pin = checked(url).await?;
        let to = "refs/kwant/suggest";
        let spec = format!("+refs/heads/{branch}:{to}");
        let args = ["fetch", "--quiet", "--depth=1", "--no-tags", "--no-write-fetch-head", "--no-recurse-submodules", "--", url, &spec];
        let o = Opts { timeout, max_out: MAX_OUT, auth, file: false, pin: &pin, fsize: None };
        self.fetch_capped(&args, o, crate::server::dev::mirror_cap()).await?;
        self.resolve(to).await
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

/// One file's section of a `-p` diff: the paths on its `--- a/` and `+++ b/` lines (`None` for
/// `/dev/null`, or when the section has no hunks and so no such lines) and its hunks from the
/// first `@@` (GitHub's `patch`).
#[derive(Debug, PartialEq)]
struct Section {
    old: Option<String>,
    new: Option<String>,
    patch: Option<String>,
}

/// A `-p` diff split per section. `patch` is `None` for a binary file or one with no hunks (a
/// mode change, an empty file, a pure rename). A type change (symlink to file) prints two
/// sections for one file: a delete, then an add.
fn split_patches(text: &str) -> Vec<Section> {
    if text.trim().is_empty() {
        return Vec::new();
    }
    let Some(body) = text.strip_prefix("diff --git ") else { return Vec::new() };
    body.split("\ndiff --git ")
        .map(|sec| {
            let none = Section { old: None, new: None, patch: None };
            let Some(i) = sec.find("\n@@ ") else { return none };
            let (head, hunks) = (&sec[..i], &sec[i + 1..]);
            if head.contains("\nBinary files ") {
                return none;
            }
            let side = |mark: &str, prefix: &str| head.lines().find_map(|l| l.strip_prefix(mark)).and_then(|n| patch_path(n, prefix));
            Section { old: side("--- ", "a/"), new: side("+++ ", "b/"), patch: Some(hunks.trim_end_matches('\n').to_string()) }
        })
        .collect()
}

/// A path from a `---`/`+++` line, without its `a/` or `b/`: `None` for `/dev/null`. Git ends
/// the line with a tab when the name has a space, and C-quotes a name with control characters,
/// `"` or `\` (and bytes over 0x7f while `core.quotePath` is on).
fn patch_path(name: &str, prefix: &str) -> Option<String> {
    let name = name.strip_suffix('\t').unwrap_or(name);
    if name == "/dev/null" {
        return None;
    }
    let name = match name.strip_prefix('"').and_then(|n| n.strip_suffix('"')) {
        Some(q) => unquote_c(q)?,
        None => name.to_string(),
    };
    name.strip_prefix(prefix).map(str::to_string)
}

/// Git's C-style quoting undone: `\\`, `\"`, `\t`, `\n` and the like, and `\ooo` octal bytes.
fn unquote_c(q: &str) -> Option<String> {
    let b = q.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] != b'\\' {
            out.push(b[i]);
            i += 1;
            continue;
        }
        let c = *b.get(i + 1)?;
        i += 2;
        out.push(match c {
            b'a' => 7,
            b'b' => 8,
            b't' => b'\t',
            b'n' => b'\n',
            b'v' => 11,
            b'f' => 12,
            b'r' => b'\r',
            b'0'..=b'3' => {
                let d = b.get(i..i + 2)?;
                if !d.iter().all(|x| (b'0'..=b'7').contains(x)) {
                    return None;
                }
                i += 2;
                (c - b'0') * 64 + (d[0] - b'0') * 8 + (d[1] - b'0')
            }
            other => other,
        });
    }
    Some(String::from_utf8_lossy(&out).into_owned())
}

/// Gives each file the hunks of its own section, matched by path, never by position: the `+++ b/`
/// path for a modify, add, rename or copy, the `--- a/` path for a delete. A type change takes
/// its add section's hunks (the new side, where review comments go), else its delete section's.
/// A file with no section of its own, or whose section is binary or has no hunks, keeps `None`.
fn match_patches(files: &mut [FileDiff], sections: Vec<Section>) {
    let mut by_new: HashMap<String, String> = HashMap::new();
    let mut deleted: HashMap<String, String> = HashMap::new();
    for s in sections {
        let Some(patch) = s.patch else { continue };
        match (s.old, s.new) {
            (_, Some(n)) => {
                by_new.entry(n).or_insert(patch);
            }
            (Some(o), None) => {
                deleted.entry(o).or_insert(patch);
            }
            (None, None) => {}
        }
    }
    for f in files.iter_mut() {
        f.patch = match f.status.as_str() {
            "removed" => deleted.remove(&f.path),
            "changed" => by_new.remove(&f.path).or_else(|| deleted.remove(&f.path)),
            _ => by_new.remove(&f.path),
        };
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Command;

    /// Writes an executable script at `path` that is safe to run while other tests start
    /// processes (D-2). Writing it here would hold a write fd on it in this process, and a
    /// thread that forks at that moment hands the fd to its child until that child execs; running
    /// the script then fails with "text file busy" (ETXTBSY), which `run` reports as `Failed`.
    /// So the bytes go to a draft, and a separate `cp` makes the real file: the only fd ever open
    /// for writing on it lives and dies in that `cp`, which has exited before anything runs it.
    #[cfg(unix)]
    fn exe(path: &Path, text: &str) {
        let draft = path.with_extension("draft");
        std::fs::write(&draft, text).unwrap();
        let copied = Command::new("cp").arg(&draft).arg(path).status().unwrap();
        assert!(copied.success(), "cp {copied}");
        std::fs::remove_file(&draft).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
    }

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

    /// 0.7.7 fresh reviews: every head git has gets its committer date in one call; a head it
    /// doesn't have (a mirror that never fetched it) is left out, not an error.
    #[tokio::test]
    async fn commit_dates_read_what_git_has_and_leave_out_the_rest() {
        let tmp = tempfile::tempdir().unwrap();
        let (w, c) = upstream(tmp.path());
        let repo = Repo::local(&w, &tmp.path().join("data")).await.unwrap();
        let missing = "e".repeat(40);
        let dates = repo.commit_dates(&[c[0].as_str(), missing.as_str(), c[2].as_str()]).await.unwrap();
        assert_eq!(dates.len(), 2, "{dates:?}");
        for sha in [&c[0], &c[2]] {
            let want: i64 = sh(&w, &["log", "-1", "--format=%ct", sha]).parse().unwrap();
            assert_eq!(dates[sha.as_str()].timestamp(), want);
        }
        assert!(repo.commit_dates(&[]).await.unwrap().is_empty());
        // Only shas reach git
        assert_eq!(repo.commit_dates(&["main"]).await, Err(GitError::Refused));
        assert_eq!(repo.commit_dates(&["--all"]).await, Err(GitError::Refused));
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
        let repo = Repo::local(&w, &tmp.path().join("data")).await.unwrap();
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
        assert_eq!(Repo::local(&tmp.path().join("nothing"), tmp.path()).await.unwrap_err(), GitError::NotFound);
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
        assert!(GitError::NoGit.to_string().contains("git 2.31 or newer on the server") && INSTALL_HINT.contains("git 2.31 or newer on the server"));
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

    // Unix only: the fake git is a `#!/bin/sh` script, which Windows can't run as a program
    #[cfg(unix)]
    #[tokio::test]
    async fn a_slow_git_is_killed_with_its_children() {
        let tmp = tempfile::tempdir().unwrap();
        let fake = tmp.path().join("git");
        let pid_file = tmp.path().join("child.pid");
        exe(&fake, &format!("#!/bin/sh\nsleep 30 &\necho $! > {}\nwait\n", pid_file.display()));
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

    // Unix only: the fake git is a `#!/bin/sh` script, which Windows can't run as a program
    #[cfg(unix)]
    #[tokio::test]
    async fn the_token_reaches_git_by_environment_only() {
        let tmp = tempfile::tempdir().unwrap();
        let fake = tmp.path().join("git");
        let seen = tmp.path().join("seen");
        // Records its argv and the config git would read from the environment
        exe(
            &fake,
            &format!(
                "#!/bin/sh\nprintf '%s\\n' \"$@\" > {0}.argv\nenv | grep -E '^(GIT_|SSH_ASKPASS|GCM_)' | sort > {0}.env\n",
                seen.display()
            ),
        );
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
        assert!(env.contains("GIT_CONFIG_NOSYSTEM=1\n"), "D4c");
        assert!(argv.contains("core.pager=cat") && argv.contains("core.hooksPath=/dev/null") && argv.contains("core.fsmonitor=false"), "D4c");
        let ours = ["GIT_CONFIG_NOSYSTEM", "GIT_CONFIG_COUNT", "GIT_CONFIG_KEY_0", "GIT_CONFIG_VALUE_0", "GIT_TERMINAL_PROMPT", "GIT_OPTIONAL_LOCKS", "GIT_LITERAL_PATHSPECS", "GIT_NO_REPLACE_OBJECTS", "GCM_INTERACTIVE", "SSH_ASKPASS_REQUIRE"];
        for name in env.lines().filter_map(|l| l.split('=').next()) {
            assert!(ours.contains(&name) || KEEP_ENV.contains(&name), "only kwant's own or the person's ssh/config choices reach git: {name}");
        }

        assert!(Auth::header("http://github.com/o/n.git", Zeroizing::new("Basic x".into())).is_none(), "never over plain http");
        assert!(Auth::header("ssh://git@github.com/o/n.git", Zeroizing::new("Basic x".into())).is_none());
        assert!(Auth::header("https://github.com/o/n.git", Zeroizing::new("Basic x\r\nX-Evil: 1".into())).is_none(), "one header line");
        assert!(Auth::header("http://127.0.0.1:9/o/n.git", Zeroizing::new("Basic x".into())).is_some(), "a test server on this machine");
    }

    /// A resolver with fixed answers that remembers what it was asked (no network).
    struct Fake {
        answers: std::collections::HashMap<&'static str, Vec<IpAddr>>,
        asked: Mutex<Vec<(String, u16)>>,
    }

    #[async_trait::async_trait]
    impl Resolver for Fake {
        async fn lookup(&self, host: &str, port: u16) -> Result<Vec<IpAddr>, GitError> {
            self.asked.lock().unwrap().push((host.to_string(), port));
            match host {
                "down.example" => Err(GitError::Unreachable),
                _ => Ok(self.answers.get(host).cloned().unwrap_or_default()),
            }
        }
    }

    fn ip(s: &str) -> IpAddr {
        s.parse().unwrap()
    }

    #[tokio::test]
    async fn a_remote_is_looked_up_refused_on_this_machine_and_pinned_to_what_was_checked() {
        let fake = Fake {
            answers: [
                ("gitea.home.example", vec![ip("192.168.1.20")]),
                ("two.example", vec![ip("203.0.113.7"), ip("2001:db8::5")]),
                ("rebind.example", vec![ip("127.0.0.1")]),
                ("meta.example", vec![ip("169.254.169.254")]),
                ("v6local.example", vec![ip("fe80::1")]),
                ("v6loop.example", vec![ip("::1")]),
                ("mapped.example", vec![ip("::ffff:127.0.0.1")]),
                ("mapped-meta.example", vec![ip("::ffff:169.254.169.254")]),
                ("nat64.example", vec![ip("64:ff9b::7f00:1")]),
                ("zero.example", vec![ip("0.0.0.0")]),
                ("mixed.example", vec![ip("203.0.113.7"), ip("127.0.0.2")]),
            ]
            .into_iter()
            .collect(),
            asked: Mutex::new(Vec::new()),
        };
        let new = Some((2, 43));
        let pin = |s: &str| Ok(vec![format!("http.curloptResolve={s}")]);

        // A LAN host is allowed (people run Gitea at home) and pinned to the address checked
        assert_eq!(check_remote("https://gitea.home.example/o/n.git", &fake, new).await, pin("gitea.home.example:443:192.168.1.20"));
        assert_eq!(check_remote("http://gitea.home.example:3000/o/n.git", &fake, new).await, pin("gitea.home.example:3000:192.168.1.20"));
        assert_eq!(check_remote("https://two.example/o/n.git", &fake, new).await, pin("two.example:443:203.0.113.7,[2001:db8::5]"), "every address checked, v6 in brackets");
        // An older git (before 2.37) can't pin: the check stands alone
        assert_eq!(check_remote("https://gitea.home.example/o/n.git", &fake, Some((2, 36))).await, Ok(vec![]));
        // ssh and git:// are checked, never pinned
        assert_eq!(check_remote("ssh://git@gitea.home.example/o/n.git", &fake, new).await, Ok(vec![]));
        assert_eq!(check_remote("git@gitea.home.example:o/n.git", &fake, new).await, Ok(vec![]));
        assert_eq!(check_remote("git://gitea.home.example/o/n.git", &fake, new).await, Ok(vec![]));
        assert_eq!(
            fake.asked.lock().unwrap().clone(),
            [("gitea.home.example", 443), ("gitea.home.example", 3000), ("two.example", 443), ("gitea.home.example", 443), ("gitea.home.example", 22), ("gitea.home.example", 22), ("gitea.home.example", 9418)]
                .map(|(h, p)| (h.to_string(), p))
        );

        // A name that resolves to this machine or link-local is refused, over every protocol
        for host in ["rebind.example", "meta.example", "v6local.example", "v6loop.example", "mapped.example", "mapped-meta.example", "nat64.example", "zero.example", "mixed.example"] {
            for url in [format!("https://{host}/o/n.git"), format!("ssh://git@{host}/o/n.git"), format!("git@{host}:o/n.git"), format!("git://{host}/o/n.git")] {
                assert_eq!(check_remote(&url, &fake, new).await, Err(GitError::Refused), "{url}");
            }
        }
        // A literal address or localhost is decided without a lookup
        fake.asked.lock().unwrap().clear();
        for url in ["https://127.0.0.1/o/n.git", "https://[::1]/o/n.git", "https://169.254.169.254/o/n.git", "https://[::ffff:169.254.169.254]/o/n.git", "https://localhost/o/n.git", "ssh://git@app.localhost/o/n.git"] {
            assert_eq!(check_remote(url, &fake, new).await, Err(GitError::Refused), "{url}");
        }
        assert_eq!(check_remote("https://192.168.1.9:8443/o/n.git", &fake, new).await, Ok(vec![]), "a LAN address: nothing to pin");
        assert!(fake.asked.lock().unwrap().is_empty());
        // No answer is unreachable; an address kwant won't pass on is refused before any lookup
        assert_eq!(check_remote("https://nowhere.example/o/n.git", &fake, new).await, Err(GitError::Unreachable));
        assert_eq!(check_remote("https://down.example/o/n.git", &fake, new).await, Err(GitError::Unreachable));
        assert_eq!(check_remote("file:///etc/o.git", &fake, new).await, Err(GitError::Refused));
        assert_eq!(check_remote("https://u:p@gitea.home.example/o/n.git", &fake, new).await, Err(GitError::Refused), "only a stripped fetch URL");
    }

    #[test]
    fn blocked_addresses_are_this_machine_and_link_local_never_the_lan() {
        for a in ["127.0.0.1", "127.9.9.9", "0.0.0.0", "0.1.2.3", "169.254.169.254", "169.254.0.1", "255.255.255.255", "::1", "::", "fe80::1", "febf::1", "::ffff:127.0.0.1", "::ffff:169.254.169.254", "::127.0.0.1", "64:ff9b::a9fe:a9fe"] {
            assert!(blocked(ip(a)), "{a}");
        }
        for a in ["10.0.0.1", "172.16.0.1", "192.168.1.20", "100.64.0.1", "203.0.113.7", "fd00::1", "2001:db8::5", "::ffff:192.168.1.20", "fec0::1"] {
            assert!(!blocked(ip(a)), "{a}");
        }
    }

    /// The pin is what git connects to: a name that can't resolve (`.invalid`) reaches a listener
    /// on this machine only through `http.curloptResolve`. (The check never lets kwant pin to
    /// loopback; `run` is called directly to see real git honour the setting.)
    #[tokio::test]
    async fn real_git_connects_where_it_was_pinned() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let seen = std::thread::spawn(move || {
            use std::io::{Read, Write};
            let (mut c, _) = listener.accept().unwrap();
            c.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
            let mut buf = [0u8; 4096];
            let n = c.read(&mut buf).unwrap_or(0);
            let _ = c.write_all(b"HTTP/1.0 404 Not Found\r\nContent-Length: 0\r\n\r\n");
            String::from_utf8_lossy(&buf[..n]).to_string()
        });
        let tmp = tempfile::tempdir().unwrap();
        let pin = [format!("http.curloptResolve=pinned.invalid:{port}:127.0.0.1")];
        let url = format!("http://pinned.invalid:{port}/o/n.git");
        let r = run("git", Some(tmp.path()), &["ls-remote", "--", &url], Opts { timeout: REMOTE_TIMEOUT, pin: &pin, ..Opts::read() }).await;
        assert_eq!(r, Err(GitError::NotFound));
        let request = seen.join().unwrap();
        assert!(request.starts_with("GET /o/n.git/info/refs"), "{request}");
        assert!(request.to_ascii_lowercase().contains(&format!("host: pinned.invalid:{port}")), "{request}");
    }

    #[test]
    fn only_an_http_address_without_dot_git_gets_one() {
        let g = |u: &str| with_dot_git(u);
        assert_eq!(g("https://gitlab.com/gitlab-org/gitter/webapp").as_deref(), Some("https://gitlab.com/gitlab-org/gitter/webapp.git"));
        assert_eq!(g("https://gitlab.com/group/sub/app/").as_deref(), Some("https://gitlab.com/group/sub/app.git"));
        assert_eq!(g("https://git.example.org:8443/g/r").as_deref(), Some("https://git.example.org:8443/g/r.git"));
        assert_eq!(g("http://gitea.lan/o/n").as_deref(), Some("http://gitea.lan/o/n.git"));
        for u in ["https://gitlab.com/g/r.git", "https://gitlab.com/g/r.GIT", "https://gitlab.com", "https://gitlab.com/", "ssh://git@gitlab.com/g/r", "git@gitlab.com:g/r", "git://gitlab.com/g/r", "https://u:p@gitlab.com/g/r", "https://gitlab.com/g/r?x=1"] {
            assert_eq!(g(u), None, "{u}");
        }
        assert!(redirected(b"fatal: unable to access 'https://gitlab.com/g/r/': The requested URL returned error: 301\n"));
        assert!(!redirected(b"fatal: unable to access 'https://gitlab.com/g/r/': The requested URL returned error: 404\n"));
    }

    /// A listener on this machine that answers each request by its path (`answer` gets the
    /// request line) and keeps the request lines it saw, for `n` requests.
    fn http_host(n: usize, answer: fn(&str) -> String) -> (u16, std::thread::JoinHandle<Vec<String>>) {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        listener.set_nonblocking(false).unwrap();
        let seen = std::thread::spawn(move || {
            use std::io::{Read, Write};
            let mut lines = Vec::new();
            for _ in 0..n {
                let Ok((mut c, _)) = listener.accept() else { break };
                c.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
                let mut buf = [0u8; 8192];
                let k = c.read(&mut buf).unwrap_or(0);
                let req = String::from_utf8_lossy(&buf[..k]).to_string();
                let line = req.lines().next().unwrap_or_default().to_string();
                let _ = c.write_all(answer(&line).as_bytes());
                lines.push(line);
            }
            lines
        });
        (port, seen)
    }

    /// N-4: GitLab answers `/group/repo` with a redirect to `/group/repo.git`. git isn't let follow
    /// it; kwant asks the same host once more with `.git`, and a private repo there is `Auth`
    /// (so Suggest offers "Connect GitLab"), not a generic failure.
    #[tokio::test]
    async fn an_address_without_dot_git_is_asked_again_with_it() {
        const SHA: &str = "0123456789abcdef0123456789abcdef01234567";
        fn gitlab(line: &str) -> String {
            let path = line.split(' ').nth(1).unwrap_or_default();
            if let Some(rest) = path.strip_prefix("/group/app/") {
                format!("HTTP/1.0 301 Moved Permanently\r\nLocation: /group/app.git/{rest}\r\nContent-Length: 0\r\n\r\n")
            } else if path.starts_with("/group/app.git/info/refs") {
                let body = format!("{SHA}\trefs/heads/main\n");
                format!("HTTP/1.0 200 OK\r\nContent-Type: text/plain\r\nContent-Length: {}\r\n\r\n{body}", body.len())
            } else if let Some(rest) = path.strip_prefix("/private/app/") {
                format!("HTTP/1.0 301 Moved Permanently\r\nLocation: /private/app.git/{rest}\r\nContent-Length: 0\r\n\r\n")
            } else if path.starts_with("/private/app.git/") {
                "HTTP/1.0 401 Unauthorized\r\nWWW-Authenticate: Basic realm=\"x\"\r\nContent-Length: 0\r\n\r\n".into()
            } else {
                // Moves every time: kwant asks twice at most and never follows it
                "HTTP/1.0 302 Found\r\nLocation: http://elsewhere.invalid/\r\nContent-Length: 0\r\n\r\n".into()
            }
        }
        let tmp = tempfile::tempdir().unwrap();
        let o = Opts { timeout: REMOTE_TIMEOUT, ..Opts::read() };

        let (port, seen) = http_host(2, gitlab);
        let url = format!("http://127.0.0.1:{port}/group/app");
        let out = run("git", Some(tmp.path()), &["ls-remote", "--heads", "--", &url], o).await.unwrap();
        assert_eq!(String::from_utf8_lossy(&out), format!("{SHA}\trefs/heads/main\n"));
        let seen = seen.join().unwrap();
        assert!(seen[0].starts_with("GET /group/app/info/refs"), "{seen:?}");
        assert!(seen[1].starts_with("GET /group/app.git/info/refs"), "{seen:?}");

        let (port, seen) = http_host(2, gitlab);
        let url = format!("http://127.0.0.1:{port}/private/app");
        assert_eq!(run("git", Some(tmp.path()), &["ls-remote", "--", &url], o).await, Err(GitError::Auth));
        assert!(seen.join().unwrap()[1].starts_with("GET /private/app.git/info/refs"));

        let (port, seen) = http_host(3, gitlab);
        let url = format!("http://127.0.0.1:{port}/moved/app");
        assert_eq!(run("git", Some(tmp.path()), &["ls-remote", "--", &url], o).await, Err(GitError::Failed));
        // Close the listener's third wait: there must be no third request
        let _ = std::net::TcpStream::connect(("127.0.0.1", port));
        let seen = seen.join().unwrap();
        assert!(seen[0].starts_with("GET /moved/app/info/refs") && seen[1].starts_with("GET /moved/app.git/info/refs"), "{seen:?}");
        assert_eq!(seen[2], "", "asked twice, never followed: {seen:?}");
    }

    #[tokio::test]
    async fn a_fetch_checks_its_remote_first() {
        let tmp = tempfile::tempdir().unwrap();
        let repo = Repo::mirror(tmp.path(), "s1", "localhost/o/n").await.unwrap();
        for url in ["https://localhost/o/n.git", "https://127.0.0.1/o/n.git", "http://[::1]:8080/o/n.git"] {
            assert_eq!(repo.fetch(url, "main", None).await, Err(GitError::Refused), "{url}");
            assert_eq!(repo.fetch_ref(url, "refs/pull/1/head", "refs/kwant/pr-1", None).await, Err(GitError::Refused), "{url}");
            assert_eq!(repo.default_branch(url, None).await, Err(GitError::Refused), "{url}");
            assert_eq!(repo.has_branch(url, "main", None).await, Err(GitError::Refused), "{url}");
        }
    }

    #[tokio::test]
    async fn a_mirror_over_the_cap_is_deleted_and_fetched_again_after_a_day_or_when_added_again() {
        let tmp = tempfile::tempdir().unwrap();
        let (w, _) = upstream(tmp.path());
        let data = tmp.path().join("data");
        let key = "gitlab.example.com/group/big";
        let dir = mirror_dir(&data, "s1", key);
        let repo = Repo::mirror(&data, "s1", key).await.unwrap();
        run("git", Some(&repo.dir), &["fetch", "--quiet", "--", w.to_str().unwrap(), "+refs/heads/main:refs/heads/main"], Opts { file: true, ..Opts::read() }).await.unwrap();
        assert_eq!(repo.cap(MIRROR_CAP).await, Ok(()), "a small repo stays");
        assert!(dir.join("HEAD").is_file());
        // A tiny cap (the test hook): the mirror goes and a note says why
        assert_eq!(repo.cap(1000).await, Err(GitError::RepoTooBig));
        assert!(!dir.exists(), "the mirror is deleted");
        assert!(too_big_mark(&dir).is_file());
        assert_eq!(GitError::RepoTooBig.to_string(), "This repo is too big for Dev mode to read (over 1 GB).");
        drop(repo);

        // The next polls don't fetch it again: refused before git or a lookup runs
        let repo = Repo::mirror(&data, "s1", key).await.unwrap();
        assert_eq!(repo.fetch("https://gitlab.example.com/group/big.git", "main", None).await, Err(GitError::RepoTooBig));
        assert_eq!(repo.fetch_ref("https://gitlab.example.com/group/big.git", "refs/pull/1/head", "refs/kwant/pr-1", None).await, Err(GitError::RepoTooBig));
        // A day on, it is tried again
        let day_ago = std::time::SystemTime::now() - TOO_BIG_RETRY - Duration::from_secs(60);
        std::fs::File::options().write(true).open(too_big_mark(&dir)).unwrap().set_modified(day_ago).unwrap();
        assert_eq!(repo.not_too_big().await, Ok(()));
        assert!(!too_big_mark(&dir).exists(), "the old note is cleared");
        drop(repo);

        // Removing the repo (and adding it again) starts afresh
        std::fs::write(too_big_mark(&dir), b"").unwrap();
        remove_mirror(&data, "s1", key).await;
        assert!(!too_big_mark(&dir).exists() && !dir.exists());
        let repo = Repo::mirror(&data, "s1", key).await.unwrap();
        assert_eq!(repo.not_too_big().await, Ok(()));

        // A team folder is never capped: kwant didn't copy it
        let local = Repo::local(&w, &data).await.unwrap();
        assert_eq!(local.cap(1).await, Ok(()));
        assert!(w.join(".git/HEAD").is_file());
    }

    #[test]
    fn a_password_in_a_remote_is_never_kept() {
        // http(s) and git:// lose their userinfo whole; ssh keeps a user name only
        assert_eq!(fetch_url("https://u:p@host.example/x.git").as_deref(), Some("https://host.example/x.git"));
        assert_eq!(fetch_url("ssh://u:p@host.example/x.git").as_deref(), Some("ssh://u@host.example/x.git"));
        assert_eq!(fetch_url("http://u:p@host.example:8080/x.git").as_deref(), Some("http://host.example:8080/x.git"));
        assert_eq!(fetch_url("git://u:p@host.example/x.git").as_deref(), Some("git://host.example/x.git"));
        assert_eq!(fetch_url("https://u@host.example/x.git").as_deref(), Some("https://host.example/x.git"));
        assert_eq!(fetch_url("u:p@host.example:x.git").as_deref(), Some("u@host.example:x.git"));
        let secret = "s3cr3t-c4n4ry";
        for raw in [format!("https://u:{secret}@host.example/x.git"), format!("ssh://u:{secret}@host.example/x.git"), format!("u:{secret}@host.example:x.git")] {
            for got in [fetch_url(&raw), remote_url(&raw)] {
                let got = got.unwrap_or_default();
                assert!(!got.contains(secret) && !got.contains(":p@"), "{raw} -> {got}");
            }
        }
    }

    #[test]
    fn diff_output_parses_like_githubs_files() {
        let raw = b":100644 100644 aaa bbb M\0src/a.rs\0:000000 100644 000 ccc A\0new.txt\0:100644 100644 ddd eee R087\0old.rs\0new.rs\0";
        let files = parse_raw(raw);
        assert_eq!(files.iter().map(|f| (f.path.as_str(), f.status.as_str(), f.previous_path.as_deref())).collect::<Vec<_>>(), [("src/a.rs", "modified", None), ("new.txt", "added", None), ("new.rs", "renamed", Some("old.rs"))]);
        assert_eq!(parse_numstat(b"3\t1\tsrc/a.rs\0-\t-\tnew.txt\x002\t0\t\0old.rs\0new.rs\0"), [(3, 1), (0, 0), (2, 0)]);
        let p = "diff --git a/x b/x\nindex 1..2 100644\n--- a/x\n+++ b/x\n@@ -1 +1 @@\n-a\n+b\ndiff --git a/y b/y\nBinary files a/y and b/y differ\ndiff --git a/z b/z\nold mode 100644\nnew mode 100755\n";
        let sec = |old: Option<&str>, new: Option<&str>, patch: Option<&str>| Section { old: old.map(Into::into), new: new.map(Into::into), patch: patch.map(Into::into) };
        assert_eq!(split_patches(p), [sec(Some("x"), Some("x"), Some("@@ -1 +1 @@\n-a\n+b")), sec(None, None, None), sec(None, None, None)]);
        assert!(split_patches("").is_empty());
        // A name with a space ends in a tab; an odd one is C-quoted, prefix and all
        let p = "diff --git a/sp ace b/sp ace\n--- a/sp ace\t\n+++ b/sp ace\t\n@@ -1 +1 @@\n-a\n+b\ndiff --git \"a/q\\\"t\\303\\251\" \"b/q\\\"t\\303\\251\"\ndeleted file mode 100644\n--- \"a/q\\\"t\\303\\251\"\n+++ /dev/null\n@@ -1 +0,0 @@\n-a\n";
        assert_eq!(split_patches(p), [sec(Some("sp ace"), Some("sp ace"), Some("@@ -1 +1 @@\n-a\n+b")), sec(Some("q\"t\u{e9}"), None, Some("@@ -1 +0,0 @@\n-a"))]);
        assert_eq!(unquote_c("a\\tb\\\\c\\nd"), Some("a\tb\\c\nd".into()));
        assert_eq!(unquote_c("bad\\9"), Some("bad9".into()), "an unknown escape is the character itself");
        assert_eq!(unquote_c("cut\\30"), None);
        assert_eq!(unquote_c("cut\\"), None);
    }

    /// Rigel's F-1 on #447: patches were matched to files by position, and dropped when the counts
    /// differed. Now each file finds its own section by path, so a section too many or too few
    /// costs only the file it belongs to.
    #[test]
    fn patches_are_matched_to_files_by_path() {
        let file = |path: &str, status: &str| FileDiff { path: path.into(), status: status.into(), additions: 1, deletions: 1, patch: None, previous_path: None };
        let sec = |old: Option<&str>, new: Option<&str>, patch: Option<&str>| Section { old: old.map(Into::into), new: new.map(Into::into), patch: patch.map(Into::into) };
        let mut files = vec![file("gone.rs", "removed"), file("link", "changed"), file("new.rs", "renamed"), file("lost.rs", "modified"), file("mode.sh", "modified"), file("back", "changed")];
        match_patches(
            &mut files,
            vec![
                sec(Some("gone.rs"), None, Some("@@ gone")),
                // A type change: the delete, then the add; the add (the new side) wins
                sec(Some("link"), None, Some("@@ old link")),
                sec(None, Some("link"), Some("@@ new link")),
                sec(Some("old.rs"), Some("new.rs"), Some("@@ renamed")),
                // mode.sh: a mode change has no hunks
                sec(None, None, None),
                // A type change whose new side is binary: the old side's hunks
                sec(Some("back"), None, Some("@@ old back")),
                sec(None, None, None),
                // A section for no file in the list is ignored
                sec(Some("other.rs"), Some("other.rs"), Some("@@ other")),
            ],
        );
        let got: Vec<_> = files.iter().map(|f| (f.path.as_str(), f.patch.as_deref())).collect();
        assert_eq!(got, [("gone.rs", Some("@@ gone")), ("link", Some("@@ new link")), ("new.rs", Some("@@ renamed")), ("lost.rs", None), ("mode.sh", None), ("back", Some("@@ old back"))]);
        // A delete never takes an add's hunks, nor a modify a delete's
        let mut files = vec![file("a", "removed"), file("b", "modified")];
        match_patches(&mut files, vec![sec(None, Some("a"), Some("@@ a")), sec(Some("b"), None, Some("@@ b"))]);
        assert!(files.iter().all(|f| f.patch.is_none()), "{files:?}");
    }

    /// F-1 with real git: a symlink turned into a file prints two `-p` sections for one `T` file.
    /// The edited text file next to it keeps its patch, the type-changed file gets its new side,
    /// and the card has no unread reason. A rename next to a modify still matches too.
    #[cfg(unix)]
    #[tokio::test]
    async fn a_type_change_keeps_every_files_patch() {
        let tmp = tempfile::tempdir().unwrap();
        let w = tmp.path().join("work");
        std::fs::create_dir_all(&w).unwrap();
        sh(&w, &["init", "-q"]);
        write(&w, "a.txt", b"hi\n");
        std::os::unix::fs::symlink("a.txt", w.join("link")).unwrap();
        write(&w, "old name.rs", b"fn one() {}\nfn two() {}\nfn three() {}\nfn four() {}\n");
        write(&w, "src/m.rs", b"fn m() {}\n");
        let base = commit_all(&w, "Base");
        std::fs::remove_file(w.join("link")).unwrap();
        write(&w, "link", b"real file now\n");
        write(&w, "a.txt", b"hi\nmore\n");
        let head = commit_all(&w, "Tiny change");
        let repo = Repo::local(&w, &tmp.path().join("data")).await.unwrap();
        let files = repo.diff(Some(&base), &head).await.unwrap();
        let got: Vec<_> = files.iter().map(|f| (f.path.as_str(), f.status.as_str(), f.additions, f.deletions, f.patch.as_deref())).collect();
        assert_eq!(got, [("a.txt", "modified", 1, 0, Some("@@ -1 +1,2 @@\n hi\n+more")), ("link", "changed", 1, 1, Some("@@ -0,0 +1 @@\n+real file now"))]);
        assert!(super::super::risk::unread_files(&files).is_empty());
        let r = super::super::risk::assess(&super::super::risk::Inputs { files: &files, ..Default::default() });
        assert!(r.reasons.iter().all(|x| !x.starts_with("Couldn't read")), "{:?}", r.reasons);
        assert_eq!(r.level, super::super::risk::Level::Low, "{:?}", r.reasons);

        // A rename (with a space in its name) next to a modify, and a delete
        sh(&w, &["mv", "old name.rs", "new name.rs"]);
        write(&w, "new name.rs", b"fn one() {}\nfn two() {}\nfn three() {}\nfn four() {}\nfn five() {}\n");
        write(&w, "src/m.rs", b"fn m() { 1 }\n");
        std::fs::remove_file(w.join("a.txt")).unwrap();
        let next = commit_all(&w, "Rename and edit");
        let files = repo.diff(Some(&head), &next).await.unwrap();
        let by = |p: &str| files.iter().find(|f| f.path == p).unwrap_or_else(|| panic!("{p}: {files:?}")).clone();
        assert_eq!(files.len(), 3, "{files:?}");
        assert_eq!(by("a.txt").status, "removed");
        assert_eq!(by("a.txt").patch.as_deref(), Some("@@ -1,2 +0,0 @@\n-hi\n-more"));
        assert_eq!((by("new name.rs").status.as_str(), by("new name.rs").previous_path.as_deref()), ("renamed", Some("old name.rs")));
        assert_eq!(by("new name.rs").patch.as_deref(), Some("@@ -2,3 +2,4 @@ fn one() {}\n fn two() {}\n fn three() {}\n fn four() {}\n+fn five() {}"));
        assert_eq!(by("src/m.rs").patch.as_deref(), Some("@@ -1 +1 @@\n-fn m() {}\n+fn m() { 1 }"));
    }

    // -----------------------------------------------------------------------------------------
    // G2 review (Lin, at 6dd0243): one test per probe.


    /// F1 (probe P1): an ssh remote runs `ssh`, never an empty command, whatever the repo's own
    /// config says. A fake `ssh` first on PATH logs what git asked it for.
    // Unix only: the fake git is a `#!/bin/sh` script, which Windows can't run as a program
    #[cfg(unix)]
    #[tokio::test]
    async fn ssh_remotes_run_ssh_and_never_the_repos_own_command() {
        let tmp = tempfile::tempdir().unwrap();
        let bin = tmp.path().join("bin");
        std::fs::create_dir_all(&bin).unwrap();
        let log = tmp.path().join("ssh.log");
        exe(&bin.join("ssh"), &format!("#!/bin/sh\necho \"$@\" >> '{}'\nexit 1\n", log.display()));
        // git itself, with the fake ssh first on its PATH
        let wrap = tmp.path().join("git-with-fake-ssh");
        exe(&wrap, &format!("#!/bin/sh\nPATH='{}':\"$PATH\"\nexport PATH\nexec git \"$@\"\n", bin.display()));
        // A mirror whose config names another command: `-c` must beat it
        let data = tmp.path().join("data");
        let repo = Repo::mirror(&data, "s1", "gitea.example.org/o/n").await.unwrap();
        let canary = tmp.path().join("canary");
        sh(&repo.dir, &["config", "core.sshCommand", &format!("touch '{}'", canary.display())]);
        let w = wrap.to_str().unwrap();
        for url in ["ssh://git@gitea.example.org/o/n.git", "git@gitea.example.org:o/n.git"] {
            let r = run(w, Some(&repo.dir), &["ls-remote", "--", url], Opts { timeout: REMOTE_TIMEOUT, ..Opts::read() }).await;
            assert!(r.is_err(), "the fake ssh says no");
        }
        let seen = std::fs::read_to_string(&log).unwrap_or_default();
        assert!(seen.contains("git@gitea.example.org git-upload-pack '/o/n.git'"), "ssh:// ran ssh: {seen:?}");
        assert!(seen.contains("git@gitea.example.org git-upload-pack 'o/n.git'"), "scp-style ran ssh: {seen:?}");
        assert!(!canary.exists(), "the repo's own sshCommand never ran");
        // The admin's GIT_SSH program is kept, quoted for the shell
        assert_eq!(ssh_command(None), "core.sshCommand=ssh");
        assert_eq!(ssh_command(Some(std::ffi::OsStr::new(""))), "core.sshCommand=ssh");
        assert_eq!(ssh_command(Some(std::ffi::OsStr::new("/opt/my ssh/plink"))), "core.sshCommand='/opt/my ssh/plink'");
        assert_eq!(ssh_command(Some(std::ffi::OsStr::new("/x/it's"))), "core.sshCommand='/x/it'\\''s'");
    }

    /// A team folder at `root/<name>` with one commit of its own.
    fn team_folder(root: &Path, name: &str) -> PathBuf {
        let f = root.join(name);
        std::fs::create_dir_all(&f).unwrap();
        sh(&f, &["init", "-q"]);
        write(&f, "mine.txt", b"ours\n");
        commit_all(&f, "Our own work");
        f
    }

    /// F2 (probe P3, setup 1): a `.git` file naming another repo's git dir (another team's, or a
    /// kwant mirror fetched with another space's token) is refused, not read.
    #[tokio::test]
    async fn a_git_file_naming_another_repo_is_refused() {
        let tmp = tempfile::tempdir().unwrap();
        let (victim, _) = upstream(tmp.path());
        let data = tmp.path().join("data");
        let mirror = Repo::mirror(&data, "space-b", "github.com/b/private").await.unwrap();
        let mirror_dir = mirror.dir.clone();
        drop(mirror);
        let a = tmp.path().join("team-a");
        std::fs::create_dir_all(&a).unwrap();
        for target in [victim.join(".git"), mirror_dir] {
            std::fs::write(a.join(".git"), format!("gitdir: {}\n", target.display())).unwrap();
            assert_eq!(Repo::local(&a, &data).await.unwrap_err(), GitError::Refused, "{}", target.display());
        }
        // Relative works the same
        std::fs::write(a.join(".git"), "gitdir: ../work/.git\n").unwrap();
        assert_eq!(Repo::local(&a, &data).await.unwrap_err(), GitError::Refused);
    }

    /// F2 (probe P3, setup 2): a `.git` symlink is refused.
    // Unix only: making a symlink on Windows needs admin rights or Developer Mode
    #[cfg(unix)]
    #[tokio::test]
    async fn a_git_symlink_is_refused() {
        let tmp = tempfile::tempdir().unwrap();
        let (victim, _) = upstream(tmp.path());
        let b = tmp.path().join("team-b");
        std::fs::create_dir_all(&b).unwrap();
        std::os::unix::fs::symlink(victim.join(".git"), b.join(".git")).unwrap();
        assert_eq!(Repo::local(&b, &tmp.path().join("data")).await.unwrap_err(), GitError::Refused);
    }

    /// F2 (probe P3, setup 3): a repo borrowing another's objects through
    /// `objects/info/alternates` is refused.
    #[tokio::test]
    async fn alternates_are_refused() {
        let tmp = tempfile::tempdir().unwrap();
        let (victim, _) = upstream(tmp.path());
        let c = team_folder(tmp.path(), "team-c");
        let data = tmp.path().join("data");
        assert!(Repo::local(&c, &data).await.is_ok(), "its own repo reads");
        std::fs::write(c.join(".git/objects/info/alternates"), format!("{}\n", victim.join(".git/objects").display())).unwrap();
        assert_eq!(Repo::local(&c, &data).await.unwrap_err(), GitError::Refused);
    }

    /// F2: the same reach by other routes: a `.git` directory with a `commondir`, or with its
    /// objects or refs a symlink to another repo's.
    // Unix only: making a symlink on Windows needs admin rights or Developer Mode
    #[cfg(unix)]
    #[tokio::test]
    async fn a_git_dir_borrowing_another_repos_objects_or_refs_is_refused() {
        let tmp = tempfile::tempdir().unwrap();
        let (victim, _) = upstream(tmp.path());
        let data = tmp.path().join("data");
        let d = team_folder(tmp.path(), "team-d");
        std::fs::write(d.join(".git/commondir"), format!("{}\n", victim.join(".git").display())).unwrap();
        assert_eq!(Repo::local(&d, &data).await.unwrap_err(), GitError::Refused, "commondir");
        for (name, what) in [("team-e", "objects"), ("team-f", "refs"), ("team-g", "objects/pack")] {
            let f = team_folder(tmp.path(), name);
            std::fs::remove_dir_all(f.join(".git").join(what)).unwrap();
            std::os::unix::fs::symlink(victim.join(".git").join(what), f.join(".git").join(what)).unwrap();
            assert_eq!(Repo::local(&f, &data).await.unwrap_err(), GitError::Refused, "{what}");
        }
    }

    /// F2: a folder under kwant's own mirrors (`<data>/dev`) is never read as a team's.
    #[tokio::test]
    async fn kwants_own_mirrors_are_never_read_as_a_team_folder() {
        let tmp = tempfile::tempdir().unwrap();
        let data = tmp.path().join("data");
        let inside = team_folder(&data.join("dev/space-b"), "checkout");
        assert_eq!(Repo::local(&inside, &data).await.unwrap_err(), GitError::Refused);
    }

    /// F2: a real linked worktree (`git worktree add`) still reads; a hand-made one that names
    /// another repo as its main repo doesn't (only that repo can make a worktree of itself).
    #[tokio::test]
    async fn a_real_worktree_still_reads_and_a_forged_one_does_not() {
        let tmp = tempfile::tempdir().unwrap();
        let (main, c) = upstream(tmp.path());
        let data = tmp.path().join("data");
        let wt = tmp.path().join("wt");
        sh(&main, &["worktree", "add", "-q", "-b", "wt-branch", wt.to_str().unwrap()]);
        let repo = Repo::local(&wt, &data).await.expect("a real worktree reads");
        assert_eq!(repo.resolve("refs/heads/main").await.unwrap(), c[4]);

        // Forged: a git dir in the team's own folder that points back at it, with the victim as
        // its main repo
        let (victim, _) = upstream(&tmp.path().join("v"));
        let f = tmp.path().join("team-h");
        let gd = f.join("fake");
        std::fs::create_dir_all(&gd).unwrap();
        std::fs::write(gd.join("gitdir"), format!("{}\n", f.join(".git").display())).unwrap();
        std::fs::write(gd.join("commondir"), format!("{}\n", victim.join(".git").display())).unwrap();
        std::fs::write(gd.join("HEAD"), "ref: refs/heads/main\n").unwrap();
        std::fs::write(f.join(".git"), format!("gitdir: {}\n", gd.display())).unwrap();
        assert_eq!(Repo::local(&f, &data).await.unwrap_err(), GitError::Refused);
    }

    /// Bytes that don't compress, so a pack of them is as big as they are.
    fn noise(n: usize) -> Vec<u8> {
        let mut x: u64 = 0x9e37_79b9_7f4a_7c15;
        (0..n)
            .map(|_| {
                x ^= x << 13;
                x ^= x >> 7;
                x ^= x << 17;
                x as u8
            })
            .collect()
    }

    /// F3 (probe P7): a fetch stops at the cap, not after it, and ends as too big: the mirror
    /// deleted, its note left, no ref written. (A tiny cap stands in for 1 GB; the source is a
    /// local repo with a 3 MB blob, so there's no network.)
    #[tokio::test]
    async fn a_fetch_stops_at_the_cap_and_ends_as_too_big() {
        let tmp = tempfile::tempdir().unwrap();
        let src = team_folder(tmp.path(), "big");
        write(&src, "noise.bin", &noise(3 << 20));
        commit_all(&src, "Three megabytes");
        let data = tmp.path().join("data");
        let key = "gitea.example.org/o/big";
        let dir = mirror_dir(&data, "s1", key);
        let cap: u64 = 500_000;
        let from = src.to_str().unwrap();
        let args = ["fetch", "--quiet", "--no-tags", "--no-write-fetch-head", "--", from, "+refs/heads/main:refs/heads/main"];

        // The limit itself: git is stopped with no file past the cap and no ref
        let repo = Repo::mirror(&data, "s1", key).await.unwrap();
        let r = run("git", Some(&repo.dir), &args, Opts { timeout: FETCH_TIMEOUT, file: true, fsize: Some(cap), ..Opts::read() }).await;
        assert!(r.is_err(), "stopped");
        assert!(largest_file(&dir) <= cap, "a file passed the cap: {}", largest_file(&dir));
        assert!(largest_file(&dir) > cap / 2, "it was the limit that stopped it");
        assert_eq!(repo.resolve("refs/heads/main").await, Err(GitError::NotFound), "no ref written");

        // A mirror's fetch: too big, the mirror deleted and the note left
        assert_eq!(repo.fetch_capped(&args, Opts { timeout: FETCH_TIMEOUT, file: true, ..Opts::read() }, cap).await, Err(GitError::RepoTooBig));
        assert!(!dir.exists(), "the mirror is deleted");
        assert!(too_big_mark(&dir).is_file(), "the note is left");
        assert_eq!(GitError::poll_code(&GitError::RepoTooBig.to_string()), Some("too_big"));
        drop(repo);

        // Under the cap it fetches as before
        std::fs::remove_file(too_big_mark(&dir)).unwrap();
        let repo = Repo::mirror(&data, "s1", key).await.unwrap();
        assert_eq!(repo.fetch_capped(&args, Opts { timeout: FETCH_TIMEOUT, file: true, ..Opts::read() }, MIRROR_CAP).await, Ok(()));
        assert!(repo.resolve("refs/heads/main").await.is_ok());
    }

    /// F6: cloud metadata addresses outside link-local are refused (exactly, not their ranges).
    #[test]
    fn cloud_metadata_outside_link_local_is_refused() {
        for a in ["100.100.100.200", "fd00:ec2::254", "::ffff:100.100.100.200", "64:ff9b::6464:64c8"] {
            assert!(blocked(ip(a)), "{a}");
        }
        for a in ["100.100.100.201", "100.64.0.1", "100.127.255.254", "fd00:ec2::253", "fd00::254"] {
            assert!(!blocked(ip(a)), "{a}");
        }
    }

    /// Both too-big errors give the desk `poll_code: "too_big"`; no other error does.
    #[test]
    fn both_too_big_errors_have_the_too_big_poll_code() {
        assert_eq!(GitError::poll_code(&GitError::TooBig.to_string()), Some("too_big"));
        assert_eq!(GitError::poll_code(&GitError::RepoTooBig.to_string()), Some("too_big"));
        for e in [GitError::NoGit, GitError::Auth, GitError::NotFound, GitError::Unreachable, GitError::Timeout, GitError::Refused, GitError::Failed] {
            assert_eq!(GitError::poll_code(&e.to_string()), None, "{e:?}");
        }
        assert_eq!(GitError::poll_code("anything else"), None);
    }
}
