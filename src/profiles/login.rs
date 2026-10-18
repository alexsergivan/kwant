//! In-app sign-in for `cli_home` profiles: runs the engine CLI's own login with its config dir
//! pointed at the profile home and shows the user the sign-in link (and device code).
//! Claude prints a link and then waits for the code the user pastes back; Codex and Grok use a
//! device code and finish on their own once the user approves it in the browser.

use super::{CliEnv, Engine};
use serde::Serialize;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::ChildStdin;
use tokio::sync::Notify;

/// A login nobody finishes is killed after this long.
pub const LOGIN_TIMEOUT: Duration = Duration::from_secs(600);

/// Hosts a sign-in link may point at. Anything else ends the login: kwnt never shows the user
/// a link a compromised or unexpected binary printed.
pub const LOGIN_HOSTS: &[&str] = &[
    "claude.ai",
    "claude.com",
    "console.anthropic.com",
    "platform.claude.com",
    "auth.openai.com",
    "chatgpt.com",
    "accounts.x.ai",
    "auth.x.ai",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum LoginStatus {
    /// Started, no link yet.
    Starting,
    /// Link shown; waiting for the user (and, for Claude, the pasted code).
    Waiting,
    Succeeded,
    Failed,
    Expired,
    Cancelled,
}

impl LoginStatus {
    pub fn is_active(self) -> bool {
        matches!(self, LoginStatus::Starting | LoginStatus::Waiting)
    }
}

/// What the UI shows for a login. Holds no secrets: the link and device code are meant for the user.
#[derive(Debug, Clone, Serialize)]
pub struct LoginView {
    pub flow_id: String,
    pub profile_id: String,
    pub status: LoginStatus,
    pub url: Option<String>,
    pub user_code: Option<String>,
    /// Whether the user must paste a code back (Claude).
    pub needs_code: bool,
    /// The pasted code went to the CLI, which is now checking it.
    pub code_sent: bool,
    pub error: Option<String>,
    pub expires_at: chrono::DateTime<chrono::Utc>,
}

struct Flow {
    view: Mutex<LoginView>,
    home: PathBuf,
    stdin: tokio::sync::Mutex<Option<ChildStdin>>,
    cancel: Notify,
}

/// Login flows by id, at most one active per profile home.
#[derive(Clone)]
pub struct LoginManager {
    flows: Arc<Mutex<HashMap<String, Arc<Flow>>>>,
    /// Held while a login starts, so two starts for one home can't both spawn a CLI.
    starting: Arc<tokio::sync::Mutex<()>>,
    timeout: Duration,
}

impl Default for LoginManager {
    fn default() -> Self {
        Self::with_timeout(LOGIN_TIMEOUT)
    }
}

/// Called with the finished login (its `profile_id` and `flow_id`) when it succeeds.
pub type OnSuccess = Arc<dyn Fn(LoginView) + Send + Sync>;

/// Why [`LoginManager::start_exclusive`] didn't start a login.
#[derive(Debug)]
pub enum StartError {
    /// A login for this home is already running.
    Busy(Box<LoginView>),
    Failed(String),
}

impl LoginManager {
    /// A manager whose logins are killed after `timeout` instead of [`LOGIN_TIMEOUT`].
    pub fn with_timeout(timeout: Duration) -> Self {
        Self {
            flows: Default::default(),
            starting: Default::default(),
            timeout,
        }
    }

    /// Starts a login, or returns the active one for this home.
    pub async fn start(
        &self,
        engine: Engine,
        bin: &str,
        profile_id: &str,
        home: &Path,
        on_success: OnSuccess,
    ) -> Result<LoginView, String> {
        let env = CliEnv::for_home(engine, home);
        match self
            .start_exclusive(engine, bin, profile_id, home, &env, on_success)
            .await
        {
            Ok(view) => Ok(view),
            Err(StartError::Busy(view)) => Ok(*view),
            Err(StartError::Failed(e)) => Err(e),
        }
    }

    /// Starts a login with `env` applied to the CLI. `home` is the config dir it writes to: only
    /// one login per home runs at a time.
    pub async fn start_exclusive(
        &self,
        engine: Engine,
        bin: &str,
        profile_id: &str,
        home: &Path,
        env: &CliEnv,
        on_success: OnSuccess,
    ) -> Result<LoginView, StartError> {
        let _one_start = self.starting.lock().await;
        {
            let mut flows = self.flows.lock().unwrap();
            flows.retain(|_, f| f.view.lock().unwrap().status.is_active() || !expired_long_ago(f));
            if let Some(f) = flows
                .values()
                .find(|f| f.home == home && f.view.lock().unwrap().status.is_active())
            {
                return Err(StartError::Busy(Box::new(f.view.lock().unwrap().clone())));
            }
        }
        let mut cmd = env.command(bin);
        match engine {
            Engine::Claude => cmd.args(["auth", "login", "--claudeai"]),
            Engine::Codex => cmd.args(["login", "--device-auth"]),
            Engine::Grok => cmd.args(["login", "--device-auth"]),
            Engine::Custom => {
                return Err(StartError::Failed("Custom API accounts use an API key, not a sign-in".into()))
            }
        };
        if home.is_dir() {
            cmd.current_dir(home);
        }
        // CLIs print plain links without a terminal; keep them from opening a host browser
        cmd.env("BROWSER", "true")
            .env("NO_COLOR", "1")
            .env_remove("DISPLAY")
            .env_remove("WAYLAND_DISPLAY")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        let mut child = cmd.spawn().map_err(|e| {
            StartError::Failed(format!("could not start {} login: {e}", engine.as_str()))
        })?;
        let timeout = self.timeout;
        let flow_id = format!("login_{}", uuid::Uuid::new_v4().simple());
        let flow = Arc::new(Flow {
            view: Mutex::new(LoginView {
                flow_id: flow_id.clone(),
                profile_id: profile_id.to_string(),
                status: LoginStatus::Starting,
                url: None,
                user_code: None,
                needs_code: engine == Engine::Claude,
                code_sent: false,
                error: None,
                expires_at: chrono::Utc::now() + chrono::Duration::from_std(timeout).unwrap(),
            }),
            home: home.to_path_buf(),
            stdin: tokio::sync::Mutex::new(child.stdin.take()),
            cancel: Notify::new(),
        });
        self.flows
            .lock()
            .unwrap()
            .insert(flow_id.clone(), flow.clone());

        let (line_tx, mut line_rx) = tokio::sync::mpsc::channel::<String>(64);
        if let Some(out) = child.stdout.take() {
            tokio::spawn(read_lines(BufReader::new(out), line_tx.clone()));
        }
        if let Some(err) = child.stderr.take() {
            tokio::spawn(read_lines(BufReader::new(err), line_tx));
        }
        tokio::spawn(async move {
            let deadline = tokio::time::sleep(timeout);
            tokio::pin!(deadline);
            let mut lines_open = true;
            let end = loop {
                tokio::select! {
                    line = line_rx.recv(), if lines_open => match line {
                        Some(line) => {
                            if let Err(e) = flow.take_line(&line) {
                                break (LoginStatus::Failed, Some(e));
                            }
                        }
                        None => lines_open = false,
                    },
                    status = child.wait() => {
                        // Read what's left so a link printed right before exit isn't lost
                        while let Ok(line) = line_rx.try_recv() {
                            let _ = flow.take_line(&line);
                        }
                        break match status {
                            Ok(s) if s.success() => (LoginStatus::Succeeded, None),
                            Ok(s) => (LoginStatus::Failed, Some(format!("sign-in did not finish (exit {})", s.code().unwrap_or(-1)))),
                            Err(_) => (LoginStatus::Failed, Some("sign-in process failed".into())),
                        };
                    }
                    _ = &mut deadline => break (LoginStatus::Expired, Some("sign-in timed out".into())),
                    _ = flow.cancel.notified() => break (LoginStatus::Cancelled, None),
                }
            };
            let _ = child.kill().await;
            flow.stdin.lock().await.take();
            let finished = {
                let mut view = flow.view.lock().unwrap();
                view.status = end.0;
                view.error = end.1;
                view.clone()
            };
            if end.0 == LoginStatus::Succeeded {
                on_success(finished);
            }
        });
        // Give the CLI a moment to print its link so the first response usually has it
        for _ in 0..30 {
            match self.get(&flow_id) {
                Some(view) if view.url.is_none() && view.status.is_active() => {}
                Some(view) => return Ok(view),
                None => break,
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        self.get(&flow_id)
            .ok_or_else(|| StartError::Failed("sign-in vanished".to_string()))
    }

    pub fn get(&self, flow_id: &str) -> Option<LoginView> {
        self.flows
            .lock()
            .unwrap()
            .get(flow_id)
            .map(|f| f.view.lock().unwrap().clone())
    }

    /// Hands the code the user pasted to Claude's login. Only once per flow.
    pub async fn submit_code(&self, flow_id: &str, code: &str) -> Result<LoginView, String> {
        let flow = self
            .flows
            .lock()
            .unwrap()
            .get(flow_id)
            .cloned()
            .ok_or("no such sign-in")?;
        let code = code.trim();
        let valid = (8..=512).contains(&code.len())
            && code
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || "._#~-".contains(c));
        if !valid {
            return Err("that doesn't look like a sign-in code".into());
        }
        {
            let view = flow.view.lock().unwrap();
            if !view.needs_code || view.status != LoginStatus::Waiting {
                return Err("this sign-in is not waiting for a code".into());
            }
        }
        let mut stdin = flow.stdin.lock().await;
        let pipe = stdin.as_mut().ok_or("a code was already sent")?;
        // Marked first: a CLI that rejects the code may exit before the write returns
        flow.view.lock().unwrap().code_sent = true;
        pipe.write_all(format!("{code}\n").as_bytes())
            .await
            .map_err(|_| "sign-in process is gone")?;
        let _ = pipe.flush().await;
        stdin.take();
        drop(stdin);
        let view = flow.view.lock().unwrap().clone();
        Ok(view)
    }

    pub fn cancel(&self, flow_id: &str) -> bool {
        match self.flows.lock().unwrap().get(flow_id) {
            Some(f) if f.view.lock().unwrap().status.is_active() => {
                f.cancel.notify_one();
                true
            }
            _ => false,
        }
    }

    /// Cancels every active login of a profile, e.g. before it is deleted.
    pub fn cancel_profile(&self, profile_id: &str) {
        for f in self.flows.lock().unwrap().values() {
            let view = f.view.lock().unwrap();
            if view.profile_id == profile_id && view.status.is_active() {
                f.cancel.notify_one();
            }
        }
    }
}

fn expired_long_ago(f: &Flow) -> bool {
    f.view.lock().unwrap().expires_at + chrono::Duration::minutes(30) < chrono::Utc::now()
}

async fn read_lines<R: tokio::io::AsyncRead + Unpin>(
    reader: BufReader<R>,
    tx: tokio::sync::mpsc::Sender<String>,
) {
    let mut lines = reader.lines();
    while let Ok(Some(line)) = lines.next_line().await {
        if tx.send(line).await.is_err() {
            break;
        }
    }
}

impl Flow {
    /// Picks the sign-in link and device code out of a line of CLI output.
    fn take_line(&self, raw: &str) -> Result<(), String> {
        let line = strip_ansi(raw);
        let mut view = self.view.lock().unwrap();
        if view.url.is_none() {
            if let Some(url) = first_https_url(&line) {
                let url = check_login_url(&url)?;
                if view.user_code.is_none() {
                    view.user_code = user_code_in_query(&url);
                }
                view.url = Some(url);
                view.status = LoginStatus::Waiting;
                return Ok(());
            }
        }
        if view.user_code.is_none() && view.url.is_some() {
            view.user_code = device_code(&line);
        }
        Ok(())
    }
}

/// Removes terminal escape sequences (colors, cursor moves, OSC hyperlinks).
pub fn strip_ansi(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c != '\u{1b}' {
            if !c.is_control() || c == '\t' {
                out.push(c);
            }
            continue;
        }
        match chars.next() {
            Some('[') => {
                for c in chars.by_ref() {
                    if ('@'..='~').contains(&c) {
                        break;
                    }
                }
            }
            Some(']') => {
                // OSC: ends at BEL or ESC \
                while let Some(c) = chars.next() {
                    if c == '\u{7}' {
                        break;
                    }
                    if c == '\u{1b}' {
                        chars.next();
                        break;
                    }
                }
            }
            _ => {}
        }
    }
    out
}

fn first_https_url(line: &str) -> Option<String> {
    let start = line.find("https://")?;
    let url: String = line[start..]
        .chars()
        .take_while(|c| !c.is_whitespace() && !matches!(c, '"' | '\'' | '<' | '>' | ')' | ']'))
        .collect();
    Some(url.trim_end_matches(['.', ',']).to_string())
}

/// Accepts a sign-in link only on an allowed host, over https, with no credentials or odd port.
pub fn check_login_url(raw: &str) -> Result<String, String> {
    let url =
        url::Url::parse(raw).map_err(|_| "the sign-in link was not a valid URL".to_string())?;
    let host = url.host_str().unwrap_or_default().to_ascii_lowercase();
    let ok = url.scheme() == "https"
        && url.username().is_empty()
        && url.password().is_none()
        && url.port().is_none()
        && LOGIN_HOSTS.contains(&host.as_str());
    if !ok {
        return Err(format!(
            "refused a sign-in link to an unexpected host ({host})"
        ));
    }
    Ok(url.to_string())
}

fn user_code_in_query(url: &str) -> Option<String> {
    let url = url::Url::parse(url).ok()?;
    let code = url
        .query_pairs()
        .find(|(k, _)| k == "user_code")?
        .1
        .into_owned();
    is_device_code(&code).then_some(code)
}

/// A device code like `ABCD-EFGH` or `ABCD-12345` printed on its own.
fn device_code(line: &str) -> Option<String> {
    line.split(|c: char| c.is_whitespace() || matches!(c, ':' | '"' | '\''))
        .map(|w| w.trim_matches(|c: char| !c.is_ascii_alphanumeric() && c != '-'))
        .find(|w| is_device_code(w))
        .map(str::to_string)
}

fn is_device_code(w: &str) -> bool {
    let Some((a, b)) = w.split_once('-') else {
        return false;
    };
    let part = |s: &str, lens: std::ops::RangeInclusive<usize>| {
        lens.contains(&s.len())
            && s.chars()
                .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit())
    };
    part(a, 4..=4) && part(b, 4..=5)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn login_url_allowlist() {
        assert!(
            check_login_url("https://claude.com/cai/oauth/authorize?code=true&client_id=x").is_ok()
        );
        assert!(check_login_url("https://auth.openai.com/codex/device").is_ok());
        assert!(check_login_url("https://accounts.x.ai/oauth2/device?user_code=ABCD-EFGH").is_ok());
        for bad in [
            "http://claude.com/cai/oauth",
            "https://claude.com.evil.example/x",
            "https://evil.example/claude.com",
            "https://user:pw@claude.com/x",
            "https://claude.com:8443/x",
            "https://sub.claude.ai.attacker.io/",
            "not a url",
        ] {
            assert!(check_login_url(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn strips_ansi_and_finds_links_and_codes() {
        let line = "\u{1b}[94mhttps://auth.openai.com/codex/device\u{1b}[0m";
        assert_eq!(
            first_https_url(&strip_ansi(line)).unwrap(),
            "https://auth.openai.com/codex/device"
        );
        assert_eq!(
            device_code(&strip_ansi("   \u{1b}[94mABCD-12345\u{1b}[0m")).as_deref(),
            Some("ABCD-12345")
        );
        assert_eq!(
            user_code_in_query("https://accounts.x.ai/oauth2/device?user_code=WXYZ-1234")
                .as_deref(),
            Some("WXYZ-1234")
        );
        assert_eq!(device_code("Paste code here if prompted > "), None);
        assert_eq!(
            first_https_url("Browser didn't open? Use the url below: https://claude.com/cai/oauth/authorize?a=b.").unwrap(),
            "https://claude.com/cai/oauth/authorize?a=b"
        );
    }

    #[cfg(unix)]
    fn fake_cli(dir: &Path, script: &str) -> String {
        use std::os::unix::fs::PermissionsExt;
        let path = dir.join("fake-cli");
        std::fs::write(&path, format!("#!/bin/sh\n{script}\n")).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        path.to_string_lossy().into_owned()
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn claude_style_login_takes_pasted_code() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path().join("home");
        std::fs::create_dir(&home).unwrap();
        // Prints a link, reads the code, records it in the home and exits 0 only for the right one
        let bin = fake_cli(
            dir.path(),
            r#"echo "Visit: https://claude.com/cai/oauth/authorize?code=true"
printf 'Paste code here if prompted > '
read code
echo "$CLAUDE_CONFIG_DIR" > "$CLAUDE_CONFIG_DIR/seen-home"
[ "$code" = "abcDEF123#state" ]"#,
        );
        let mgr = LoginManager::default();
        let done = Arc::new(Mutex::new(None));
        let done2 = done.clone();
        let view = mgr
            .start(
                Engine::Claude,
                &bin,
                "prf_x",
                &home,
                Arc::new(move |v: LoginView| *done2.lock().unwrap() = Some(v.profile_id)),
            )
            .await
            .unwrap();
        assert_eq!(view.status, LoginStatus::Waiting);
        assert!(view.needs_code);
        assert!(view.url.unwrap().starts_with("https://claude.com/"));
        // The same home gets the same flow back
        let again = mgr
            .start(Engine::Claude, &bin, "prf_x", &home, Arc::new(|_| {}))
            .await
            .unwrap();
        assert_eq!(again.flow_id, view.flow_id);
        assert!(mgr.submit_code(&view.flow_id, "bad code!").await.is_err());
        mgr.submit_code(&view.flow_id, "abcDEF123#state")
            .await
            .unwrap();
        assert!(mgr
            .submit_code(&view.flow_id, "abcDEF123#state")
            .await
            .is_err());
        for _ in 0..50 {
            if !mgr.get(&view.flow_id).unwrap().status.is_active() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        assert_eq!(
            mgr.get(&view.flow_id).unwrap().status,
            LoginStatus::Succeeded
        );
        assert_eq!(done.lock().unwrap().as_deref(), Some("prf_x"));
        let seen = std::fs::read_to_string(home.join("seen-home")).unwrap();
        assert_eq!(seen.trim(), home.to_str().unwrap());
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn unexpected_link_kills_the_login() {
        let dir = tempfile::tempdir().unwrap();
        let bin = fake_cli(
            dir.path(),
            "echo 'Go to https://evil.example/device'\nsleep 30",
        );
        let mgr = LoginManager::default();
        let view = mgr
            .start(Engine::Codex, &bin, "prf_y", dir.path(), Arc::new(|_| {}))
            .await
            .unwrap();
        assert_eq!(view.status, LoginStatus::Failed);
        assert!(view.url.is_none());
        assert!(view.error.unwrap().contains("unexpected host"));
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn device_login_shows_code_and_can_be_cancelled() {
        let dir = tempfile::tempdir().unwrap();
        let bin = fake_cli(
            dir.path(),
            "echo 'Open https://auth.openai.com/codex/device' >&2\necho '  ABCD-12345' >&2\nsleep 30",
        );
        let mgr = LoginManager::default();
        let view = mgr
            .start(Engine::Codex, &bin, "prf_z", dir.path(), Arc::new(|_| {}))
            .await
            .unwrap();
        assert_eq!(view.status, LoginStatus::Waiting);
        assert!(!view.needs_code);
        for _ in 0..20 {
            if mgr.get(&view.flow_id).unwrap().user_code.is_some() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        assert_eq!(
            mgr.get(&view.flow_id).unwrap().user_code.as_deref(),
            Some("ABCD-12345")
        );
        assert!(mgr.cancel(&view.flow_id));
        for _ in 0..20 {
            if !mgr.get(&view.flow_id).unwrap().status.is_active() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        assert_eq!(
            mgr.get(&view.flow_id).unwrap().status,
            LoginStatus::Cancelled
        );
    }
}
