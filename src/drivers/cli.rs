use super::{events::RuntimeEvent, Driver, DriverError, TurnInput, TurnOutput};
use async_trait::async_trait;
use std::path::Path;
use std::process::Stdio;
use std::time::{Duration, Instant};
use tokio::io::AsyncReadExt;
use tokio::process::Command;
use tokio::sync::mpsc;

/// Kills a CLI's whole process group if its turn is dropped (cancelled, or its space archived)
/// before the CLI exits. `kill_on_drop` only reaches the CLI itself; the tools, shells and MCP
/// servers it started would live on. Declare it after the child, so it drops first, while the
/// child is still unreaped and its group id can't be reused. Disarmed once the CLI has exited,
/// so anything a finished turn deliberately left running is kept.
pub(crate) struct ProcessGroup(Option<u32>);

impl ProcessGroup {
    pub(crate) fn of(child: &tokio::process::Child) -> Self {
        Self(child.id())
    }

    pub(crate) fn disarm(&mut self) {
        self.0 = None;
    }
}

impl Drop for ProcessGroup {
    fn drop(&mut self) {
        #[cfg(unix)]
        if let Some(pid) = self.0.and_then(|p| i32::try_from(p).ok()).filter(|p| *p > 0) {
            // SAFETY: kill(2) with a negative pid signals that process group; no memory involved
            unsafe {
                libc::kill(-pid, libc::SIGKILL);
            }
        }
    }
}

/// Starts the CLI as the leader of its own process group, for [`ProcessGroup`].
fn own_process_group(cmd: &mut Command) {
    #[cfg(unix)]
    cmd.process_group(0);
    #[cfg(not(unix))]
    let _ = cmd;
}

static CLI_BINARY_CACHE: std::sync::OnceLock<std::sync::Mutex<std::collections::HashMap<String, Option<String>>>> =
    std::sync::OnceLock::new();

/// Cached lookup: resolving binaries spawns `which`, and this runs on every chat turn.
pub fn find_cli_binary(name: &str) -> Option<String> {
    let cache = CLI_BINARY_CACHE.get_or_init(Default::default);
    if let Some(hit) = cache.lock().ok().and_then(|c| c.get(name).cloned()) {
        if hit.as_deref().is_none_or(|p| Path::new(p).exists()) {
            return hit;
        }
    }
    let found = find_cli_binary_uncached(name);
    if let Ok(mut c) = cache.lock() {
        c.insert(name.to_string(), found.clone());
    }
    found
}

fn find_cli_binary_uncached(name: &str) -> Option<String> {
    // 1. Try standard which command
    if let Ok(output) = std::process::Command::new("which").arg(name).output() {
        if output.status.success() {
            let path = String::from_utf8_lossy(&output.stdout).trim().to_string();
            if !path.is_empty() && Path::new(&path).exists() {
                return Some(path);
            }
        }
    }

    // 2. Check ~/.local/bin and /opt/homebrew/bin
    if let Ok(home) = std::env::var("HOME") {
        let local_path = format!("{}/.local/bin/{}", home, name);
        if Path::new(&local_path).exists() {
            return Some(local_path);
        }

        // For grok / nvm node binaries
        if name == "grok" {
            let nvm_pattern = format!("{}/.nvm/versions/node", home);
            if let Ok(entries) = std::fs::read_dir(&nvm_pattern) {
                for entry in entries.flatten() {
                    let candidate = entry.path().join("bin").join("grok");
                    if candidate.exists() {
                        return Some(candidate.to_string_lossy().to_string());
                    }
                }
            }
        }
    }

    let brew_path = format!("/opt/homebrew/bin/{}", name);
    if Path::new(&brew_path).exists() {
        return Some(brew_path);
    }

    let usr_path = format!("/usr/local/bin/{}", name);
    if Path::new(&usr_path).exists() {
        return Some(usr_path);
    }

    None
}

fn toml_string(value: &str) -> String {
    serde_json::Value::String(value.to_string()).to_string()
}

/// The `mcp_servers` override for `codex exec`: kwnt's gateway (when the kwant has servers)
/// and `enabled=false` for every server codex would load from the user's own config. A `-c`
/// table is merged into codex's config, not swapped in, so the user's servers must be switched
/// off by name; the names go inside the value because codex's `-c` key path can't quote. The
/// gateway asks the user before every call, so codex's own approval is turned off for it; its
/// token is read from an env variable so it never appears in argv. A user server named like the
/// gateway is switched off like any other when there is no gateway; with one, the turn must be
/// refused first (see `codex_reserved_name_conflict`), since the two entries would merge.
pub fn codex_mcp_servers(ctx: Option<&crate::mcp::McpTurnContext>, user_servers: &[String]) -> String {
    let mut entries = Vec::new();
    let gateway = ctx.and_then(|c| c.gateway.as_ref().map(|g| (c, g)));
    let has_gateway = gateway.is_some();
    if let Some((ctx, gateway)) = gateway {
        let slowest = ctx.servers.iter().map(|s| s.timeout_secs.max(0) as u64).max().unwrap_or(0);
        // The gateway connects to every server on codex's first tools/list
        let startup_secs = (slowest + 15).max(30);
        // A call may wait for the user's approval; codex must not give up first
        let tool_secs = (crate::mcp::APPROVAL_TIMEOUT_SECS + 30).max(slowest);
        entries.push(format!(
            "{}={{url={},bearer_token_env_var={},default_tools_approval_mode=\"approve\",startup_timeout_sec={},tool_timeout_sec={}}}",
            crate::mcp::RESERVED_SERVER_NAME,
            toml_string(&gateway.url),
            toml_string(crate::mcp::GATEWAY_TOKEN_ENV),
            startup_secs,
            tool_secs,
        ));
    }
    for name in user_servers {
        if !(has_gateway && name == crate::mcp::RESERVED_SERVER_NAME) {
            entries.push(format!("{}={{enabled=false}}", toml_string(name)));
        }
    }
    format!("mcp_servers={{{}}}", entries.join(","))
}

/// The message refusing a gateway turn when the user's codex config has its own server named
/// like kwnt's gateway: `-c` would merge the two entries rather than replace the user's.
pub fn codex_reserved_name_conflict(ctx: Option<&crate::mcp::McpTurnContext>, user_servers: &[String]) -> Option<String> {
    let reserved = crate::mcp::RESERVED_SERVER_NAME;
    (ctx.is_some_and(|c| c.gateway.is_some()) && user_servers.iter().any(|n| n == reserved)).then(|| {
        format!(
            "Your codex config has an MCP server named `{reserved}`, which kwnt reserves for its own tools, so it didn't run codex. Rename that server to use MCP tools in kwnt."
        )
    })
}

/// How long `codex mcp list` may take before the turn is refused.
const CODEX_MCP_LIST_TIMEOUT: Duration = Duration::from_secs(15);

/// `codex exec` overrides limiting the model's shell commands to codex's "core" env
/// (PATH, HOME, USER, SHELL...). codex's default passes its whole env, provider key included.
/// grok's headless mode refuses anything it would ask about. Like claude's policy (see
/// `mcp::builtin_tools`): fetching pages anywhere, and editing files inside the bot's working
/// folder, run without asking. Shell commands stay refused: grok has no approval hook for kwnt.
pub fn grok_builtin_rules(folder: Option<&Path>) -> Vec<String> {
    let mut rules = vec!["--allow".to_string(), "WebFetch".to_string()];
    // grok matches paths lexically, so the folder's real path keeps a symlinked folder working
    if let Some(dir) = folder.and_then(|d| d.canonicalize().ok()) {
        let dir = dir.display().to_string().replace(')', "\\)");
        for tool in ["Edit", "Write"] {
            rules.push("--allow".into());
            rules.push(format!("{tool}({dir}/**)"));
        }
    }
    rules
}

/// claude's own product features that would work outside kwnt, where the person can't see or
/// undo them: its scheduler (cron, wake-ups, the `schedule`/`loop` skills, cloud routines), push
/// notifications, and messaging other claude sessions. kwnt has its own routines and teammates.
pub const CLAUDE_DISALLOWED_TOOLS: &[&str] = &[
    "CronCreate",
    "CronDelete",
    "CronList",
    "ScheduleWakeup",
    "RemoteTrigger",
    "PushNotification",
    "ListAgents",
    "SendMessage",
    "Skill(schedule)",
    "Skill(loop)",
];

/// grok's own scheduler, for the same reason. Its subagents are already off (`--no-subagents`).
pub const GROK_DISALLOWED_TOOLS: &[&str] = &["SchedulerCreate", "SchedulerDelete", "SchedulerList"];

/// codex features kept off for the same reason: its local automations and its cross-session
/// memories. `-c features.<name>=false` rather than `--disable`, which refuses names a later codex
/// drops and would stop every turn.
pub const CODEX_DISABLED_FEATURES: &[&str] = &["in_app_local_automation", "memories"];

/// One `--disallowedTools` value: claude reads a comma-separated list.
pub fn claude_disallowed_args() -> [String; 2] {
    ["--disallowedTools".into(), CLAUDE_DISALLOWED_TOOLS.join(",")]
}

pub fn grok_disallowed_args() -> [String; 2] {
    ["--disallowed-tools".into(), GROK_DISALLOWED_TOOLS.join(",")]
}

pub fn codex_feature_args() -> Vec<String> {
    CODEX_DISABLED_FEATURES.iter().flat_map(|f| ["-c".to_string(), format!("features.{f}=false")]).collect()
}

pub const CODEX_SHELL_POLICY: [&str; 4] = [
    "-c",
    "shell_environment_policy.inherit=\"core\"",
    "-c",
    "shell_environment_policy.ignore_default_excludes=false",
];

/// Names of the MCP servers codex would load for this turn, as codex itself resolves them:
/// `$CODEX_HOME`/`~/.codex` config, its active profile, a trusted project's `.codex/config.toml`
/// for `cwd`, and system config. The listing holds server env values, so it is never logged.
pub async fn codex_user_mcp_servers(
    bin: &str,
    cwd: Option<&Path>,
    profile_env: Option<&crate::profiles::CliEnv>,
) -> Result<Vec<String>, String> {
    let mut cmd = Command::new(bin);
    cmd.args(["mcp", "list", "--json"]);
    if let Some(env) = profile_env {
        env.apply(&mut cmd);
    }
    if let Some(dir) = cwd.filter(|d| d.exists()) {
        cmd.current_dir(dir);
    }
    cmd.stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::null()).kill_on_drop(true);
    let output = match tokio::time::timeout(CODEX_MCP_LIST_TIMEOUT, cmd.output()).await {
        Ok(Ok(o)) => o,
        Ok(Err(e)) => return Err(format!("could not run `codex mcp list`: {e}")),
        Err(_) => return Err("`codex mcp list` timed out".into()),
    };
    if !output.status.success() {
        return Err(format!("`codex mcp list` failed ({})", output.status));
    }
    let listed: Vec<serde_json::Value> =
        serde_json::from_slice(&output.stdout).map_err(|_| "`codex mcp list --json` returned unexpected output".to_string())?;
    listed
        .iter()
        .map(|s| s["name"].as_str().map(str::to_string).ok_or_else(|| "`codex mcp list --json` entry has no name".to_string()))
        .collect()
}

pub fn build_cli_prompt(input: &TurnInput) -> String {
    if input.history.is_empty() {
        return input.prompt.clone();
    }

    let mut parts = Vec::new();
    parts.push("### Conversation History:".to_string());
    let recent = if input.history.len() > 10 {
        &input.history[input.history.len() - 10..]
    } else {
        &input.history[..]
    };

    for msg in recent {
        let role = match msg.sender_role.as_str() {
            "user" => "User",
            // Name the speaker so a kwant can tell its own earlier replies from its teammates'
            "kwant" if !msg.sender_id.is_empty() && msg.sender_id != input.kwant.name => &msg.sender_id,
            "kwant" => "Assistant",
            "system" => "System",
            _ => &msg.sender_role,
        };
        parts.push(format!("{}: {}", role, msg.text));
    }

    parts.push(format!("\n### Current User Request:\n{}", input.prompt));
    parts.join("\n")
}

#[derive(Clone, Debug, Default)]
pub struct CliDriver {
    pub cli_binary: Option<String>,
    pub tool_kind: String, // "claude", "codex", "grok"
    /// The user's grok home to take model settings and login from; `None` means `$GROK_HOME`
    /// or `~/.grok`. Only read, never written.
    pub grok_home: Option<std::path::PathBuf>,
    /// Set when the turn runs on a `cli_home` profile: points the CLI at the profile's home and
    /// removes credential variables that would override its login.
    pub profile_env: Option<crate::profiles::CliEnv>,
    /// The account the turn runs on, named in the turn's reasoning line.
    pub account: Option<String>,
}

impl CliDriver {
    pub fn new(cli_binary: Option<String>, tool_kind: Option<String>) -> Self {
        let kind = tool_kind.unwrap_or_else(|| "claude".into());
        let bin = cli_binary.or_else(|| find_cli_binary(&kind));
        Self {
            cli_binary: bin,
            tool_kind: kind,
            grok_home: None,
            profile_env: None,
            account: None,
        }
    }

    /// Runs the CLI on a profile's own home (its login and settings) instead of the user's.
    /// grok still runs in a private temp home, taking settings and login from this one only.
    pub fn with_profile_home(
        mut self,
        engine: crate::profiles::Engine,
        home: std::path::PathBuf,
    ) -> Self {
        self.profile_env = Some(crate::profiles::CliEnv::for_home(engine, &home));
        if engine == crate::profiles::Engine::Grok {
            self.grok_home = Some(home);
        }
        self
    }

    pub fn with_account(mut self, name: impl Into<String>) -> Self {
        self.account = Some(name.into());
        self
    }

    /// The turn's reasoning line: CLI, account and effort, never the binary's path.
    fn executed_via(&self, effort: &str) -> String {
        let account = self.account.as_deref().unwrap_or("This machine");
        format!("Executed via {} CLI · {} [Effort: {}]", self.tool_kind, account, effort)
    }

    pub fn with_grok_home(mut self, dir: std::path::PathBuf) -> Self {
        self.grok_home = Some(dir);
        self
    }
}

/// Longest stderr tail kept from a grok run.
const GROK_STDERR_MAX: usize = 16 * 1024;

/// How a grok run gets the user's login.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum GrokAuth {
    /// `GROK_AUTH_PATH` points at the user's own auth file, which grok reads in place.
    InPlace,
    /// A 0600 copy inside the temp home, for a grok that ignores `GROK_AUTH_PATH`.
    Copied,
}

/// Everything a grok run needs, shared by the login fallback's retry.
struct GrokLaunch<'a> {
    bin_path: &'a str,
    input: &'a TurnInput,
    args: Vec<String>,
    /// The temp home's `config.toml`.
    config: toml::Table,
    /// The user's grok login file, if they have one.
    auth_file: Option<std::path::PathBuf>,
    /// This turn's gateway token, handed over in grok's env only.
    gateway_token: Option<String>,
    /// Credential variables a profile run removes.
    removed_env: Vec<String>,
}

/// What one grok process left behind.
struct GrokRun {
    /// The reply text from grok's `text` events.
    stdout: String,
    stderr: String,
    /// grok's `error` events.
    errors: Vec<String>,
    stop_reason: Option<String>,
    /// `None` if grok was killed by a signal.
    exit_code: Option<i32>,
    /// grok called at least one tool.
    tool_calls: bool,
}

/// Reads a pipe to the end, keeping at most the last `max` bytes.
async fn read_tail<R: tokio::io::AsyncRead + Unpin>(mut pipe: R, max: usize) -> String {
    let mut kept: Vec<u8> = Vec::new();
    let mut buffer = [0u8; 4096];
    while let Ok(n) = pipe.read(&mut buffer).await {
        if n == 0 {
            break;
        }
        kept.extend_from_slice(&buffer[..n]);
        if kept.len() > max {
            kept.drain(..kept.len() - max);
        }
    }
    String::from_utf8_lossy(&kept).into_owned()
}

impl CliDriver {
    /// One grok run in a fresh private `GROK_HOME` (see [`super::grok`]). The temp home lives
    /// until this returns or the turn is aborted; either way dropping it removes the directory.
    async fn run_grok_once(
        launch: &GrokLaunch<'_>,
        auth: GrokAuth,
        event_tx: &mpsc::Sender<RuntimeEvent>,
    ) -> std::io::Result<GrokRun> {
        let home = super::grok::GrokHome::create(&std::env::temp_dir(), &launch.config)?;
        let mut cmd = Command::new(launch.bin_path);
        if let Some(dir) = launch.input.workspace_dir.as_ref().filter(|d| d.exists()) {
            cmd.current_dir(dir);
        }
        cmd.env("BROWSER", "none");
        cmd.env("DISPLAY", "");
        cmd.env("GROK_HOME", &home.path);
        for (key, value) in super::grok::ISOLATION_ENV {
            cmd.env(key, value);
        }
        for key in super::grok::REMOVED_ENV {
            cmd.env_remove(key);
        }
        for key in &launch.removed_env {
            cmd.env_remove(key);
        }
        match (launch.auth_file.as_deref(), auth) {
            (Some(file), GrokAuth::InPlace) => {
                cmd.env("GROK_AUTH_PATH", file);
            }
            (Some(file), GrokAuth::Copied) => {
                home.copy_auth(file)?;
                cmd.env_remove("GROK_AUTH_PATH");
            }
            (None, _) => {}
        }
        // Never written to disk: the config's header refers to this variable by name
        match &launch.gateway_token {
            Some(token) => cmd.env(crate::mcp::GATEWAY_TOKEN_ENV, token),
            None => cmd.env_remove(crate::mcp::GATEWAY_TOKEN_ENV),
        };
        cmd.args(&launch.args);
        cmd.stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        // A cancelled turn aborts this task; the CLI must not outlive it
        cmd.kill_on_drop(true);
        own_process_group(&mut cmd);

        let mut child = cmd.spawn()?;
        let mut group = ProcessGroup::of(&child);
        let stdout_pipe = child.stdout.take();
        let stderr_pipe = child.stderr.take();
        let read_stdout = async {
            let mut run = GrokRun {
                stdout: String::new(),
                stderr: String::new(),
                errors: Vec::new(),
                stop_reason: None,
                exit_code: None,
                tool_calls: false,
            };
            let mut parser = super::grok::StreamParser::default();
            let mut buffer = [0u8; 4096];
            // Bytes of a UTF-8 character split across reads
            let mut partial: Vec<u8> = Vec::new();
            if let Some(mut stdout) = stdout_pipe {
                loop {
                    let n = match stdout.read(&mut buffer).await {
                        Ok(0) | Err(_) => break,
                        Ok(n) => n,
                    };
                    partial.extend_from_slice(&buffer[..n]);
                    let valid = match std::str::from_utf8(&partial) {
                        Ok(_) => partial.len(),
                        Err(e) if e.error_len().is_none() => e.valid_up_to(),
                        Err(_) => partial.len(),
                    };
                    let chunk = String::from_utf8_lossy(&partial[..valid]).into_owned();
                    partial.drain(..valid);
                    for event in parser.push(&chunk) {
                        Self::apply_grok_event(event, &mut run, event_tx).await;
                    }
                }
            }
            let rest = String::from_utf8_lossy(&partial).into_owned();
            for event in parser.push(&rest).into_iter().chain(parser.finish()) {
                Self::apply_grok_event(event, &mut run, event_tx).await;
            }
            run
        };
        let read_stderr = async {
            match stderr_pipe {
                Some(stderr) => read_tail(stderr, GROK_STDERR_MAX).await,
                None => String::new(),
            }
        };
        let (mut run, stderr) = tokio::join!(read_stdout, read_stderr);
        run.exit_code = child.wait().await.ok().and_then(|s| s.code());
        group.disarm();
        drop(home);
        run.stderr = stderr;
        Ok(run)
    }

    async fn apply_grok_event(
        event: super::grok::StreamEvent,
        run: &mut GrokRun,
        event_tx: &mpsc::Sender<RuntimeEvent>,
    ) {
        match event {
            super::grok::StreamEvent::Text(text) => {
                let _ = event_tx.send(RuntimeEvent::TextDelta(text.clone())).await;
                run.stdout.push_str(&text);
            }
            super::grok::StreamEvent::Error(message) => run.errors.push(message),
            super::grok::StreamEvent::End { stop_reason } => run.stop_reason = stop_reason,
            super::grok::StreamEvent::ToolCall => run.tool_calls = true,
        }
    }

    async fn run_grok(
        &self,
        bin_path: &str,
        input: &TurnInput,
        mut args: Vec<String>,
        event_tx: &mpsc::Sender<RuntimeEvent>,
    ) -> std::io::Result<GrokRun> {
        let user_home = self.grok_home.clone().or_else(super::grok::user_grok_home);
        let mut config = user_home
            .as_ref()
            .and_then(|h| std::fs::read_to_string(h.join("config.toml")).ok())
            .map(|text| super::grok::model_settings(&text))
            .unwrap_or_default();
        super::grok::restrict_shell_env(&mut config);
        // A profile's login is its home's auth.json and nothing else
        let env_auth = std::env::var_os("GROK_AUTH_PATH")
            .filter(|p| !p.is_empty() && self.profile_env.is_none());
        let auth_file = env_auth
            .map(std::path::PathBuf::from)
            .or_else(|| user_home.map(|h| h.join("auth.json")))
            .filter(|p| p.is_file());
        let gateway = input
            .mcp
            .as_ref()
            .and_then(|c| c.gateway.as_ref().map(|g| (c, g)));
        if let Some((ctx, gateway)) = gateway {
            let slowest = ctx
                .servers
                .iter()
                .map(|s| s.timeout_secs.max(0) as u64)
                .max()
                .unwrap_or(0);
            super::grok::add_gateway(&mut config, &gateway.grok_url(), slowest);
            args.push("--allow".into());
            args.push(super::grok::gateway_allow_rule());
        }
        args.extend(grok_builtin_rules(input.workspace_dir.as_deref()));
        args.push("--output-format".into());
        args.push("streaming-json".into());
        let gateway_token = gateway.map(|(_, g)| g.token.clone());
        let launch = GrokLaunch {
            bin_path,
            input,
            args,
            config,
            auth_file,
            gateway_token,
            removed_env: self
                .profile_env
                .as_ref()
                .map(|e| e.remove.clone())
                .unwrap_or_default(),
        };
        let run = Self::run_grok_once(&launch, GrokAuth::InPlace, event_tx).await?;
        if launch.auth_file.is_some()
            && run.stdout.trim().is_empty()
            && super::grok::not_signed_in(&run.stderr)
        {
            // Nothing ran: grok stops before the model when it finds no login
            tracing::info!(
                "grok ignored GROK_AUTH_PATH; retrying with a private copy of its login"
            );
            return Self::run_grok_once(&launch, GrokAuth::Copied, event_tx).await;
        }
        Ok(run)
    }
}

/// A provider refusal (quota, auth) in a CLI's error output, if that's what stopped it.
fn refusal(text: &str) -> Option<crate::profiles::ProviderFailure> {
    let (class, retry_at) = crate::profiles::classify(None, text, chrono::Utc::now())?;
    class
        .fails_over()
        .then(|| crate::profiles::ProviderFailure {
            class,
            retry_at,
            detail: crate::profiles::classify::redact_detail(text, 300),
        })
}

/// The error lines of codex's stderr. Its stderr also echoes the prompt, which must not be
/// mistaken for an error message.
fn codex_error_lines(stderr: &str) -> String {
    stderr
        .lines()
        .filter(|l| {
            let t = l.trim_start();
            t.starts_with("ERROR")
                || t.starts_with("Error:")
                || t.contains("unexpected status")
                || t.contains("stream error")
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// Whether codex's stderr shows it ran a command, edited a file or called a tool.
fn codex_had_effects(stderr: &str) -> bool {
    stderr.lines().any(|l| {
        let t = l.trim();
        t == "exec"
            || t.starts_with("exec ")
            || t.contains("apply_patch")
            || t.starts_with("tool ")
            || t == "file update"
    })
}

/// Says why grok gave no reply, so the chat never shows an empty message.
async fn empty_grok_reply(
    input: &TurnInput,
    run: &GrokRun,
    event_tx: &mpsc::Sender<RuntimeEvent>,
) -> (String, usize) {
    let mut secrets = input
        .mcp
        .as_ref()
        .map(|c| c.secret_values())
        .unwrap_or_default();
    secrets.extend(
        input
            .mcp
            .as_ref()
            .and_then(|c| c.gateway.as_ref())
            .map(|g| g.token.clone()),
    );
    let notice = super::grok::empty_reply_notice(
        &super::grok::EmptyRun {
            stderr: &run.stderr,
            errors: &run.errors,
            stop_reason: run.stop_reason.as_deref(),
            exit_code: run.exit_code,
        },
        &secrets,
    );
    tracing::warn!(exit_code = ?run.exit_code, stop_reason = ?run.stop_reason, "grok gave no reply");
    let msg = format!("⚠️ [{}] {notice}", input.kwant.name);
    let _ = event_tx.send(RuntimeEvent::TextDelta(msg.clone())).await;
    (msg, 0)
}

/// What `claude -p --output-format json` reports.
#[derive(Debug, Default, serde::Deserialize)]
struct ClaudeResult {
    #[serde(default)]
    is_error: bool,
    #[serde(default)]
    num_turns: u64,
    #[serde(default)]
    result: String,
    #[serde(default)]
    usage: Option<serde_json::Value>,
}

impl ClaudeResult {
    /// The result line of claude's JSON output, if it printed one.
    fn parse(stdout: &str) -> Option<Self> {
        stdout
            .lines()
            .rev()
            .find_map(|l| {
                serde_json::from_str::<serde_json::Value>(l.trim())
                    .ok()
                    .filter(|v| v["type"] == "result")
            })
            .and_then(|v| serde_json::from_value(v).ok())
    }

    fn tokens(&self) -> Option<usize> {
        let u = self.usage.as_ref()?;
        let sum = [
            "input_tokens",
            "output_tokens",
            "cache_creation_input_tokens",
            "cache_read_input_tokens",
        ]
        .iter()
        .filter_map(|k| u[*k].as_u64())
        .sum::<u64>();
        (sum > 0).then_some(sum as usize)
    }
}

#[async_trait]
impl Driver for CliDriver {
    async fn stream_turn(
        &self,
        input: TurnInput,
        event_tx: mpsc::Sender<RuntimeEvent>,
    ) -> Result<TurnOutput, DriverError> {
        let start = Instant::now();
        let bin_path = match &self.cli_binary {
            Some(b) => b.clone(),
            None => {
                let msg = format!(
                    "⚠️ [{}] Local CLI for `{}` was not found on this system.",
                    input.kwant.name, self.tool_kind
                );
                let _ = event_tx.send(RuntimeEvent::TextDelta(msg.clone())).await;
                let _ = event_tx
                    .send(RuntimeEvent::TurnComplete {
                        tokens: 10,
                        duration_ms: 10,
                        kwant_id: None,
                        profile_id: None,
                    })
                    .await;
                return Ok(TurnOutput {
                    text: msg,
                    reasoning: None,
                    tokens_used: 10,
                    ..Default::default()
                });
            }
        };

        let mut cmd = Command::new(&bin_path);
        if let Some(ref dir) = input.workspace_dir {
            if dir.exists() {
                cmd.current_dir(dir);
            }
        }
        let mut codex_out_file: Option<String> = None;
        // Lives until the CLI exits; dropping it deletes the per-turn MCP config file.
        let mut _mcp_config: Option<crate::mcp::cli_config::ConfigFile> = None;

        let effort = input.effort_level.as_deref().unwrap_or("medium");

        // Kept out of the chat: the turn's "Running on" notice already names the engine and
        // account, and the same binary runs every account
        tracing::info!(
            cli = %self.tool_kind,
            model = input.model.as_deref().unwrap_or("default"),
            effort,
            binary = %bin_path,
            profile_home = self.profile_env.is_some(),
            "invoking local CLI"
        );

        let system_prompt = input.format_system_prompt();
        let effective_prompt = build_cli_prompt(&input);

        // Prevent CLI tools from spawning desktop browser windows on host
        cmd.env("BROWSER", "none");
        cmd.env("DISPLAY", "");

        match self.tool_kind.as_str() {
            "codex" => {
                let out_file = format!("/tmp/kwnt_codex_{}.txt", std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos());
                let codex_effort = match effort {
                    "high" => "high",
                    "low" => "low",
                    _ => "medium",
                };
                let full_prompt = format!("[System Context: {}]\n\n{}", system_prompt, effective_prompt);
                // Fail closed: without the names, the user's own servers would run with no
                // approval card (and despite a per-kwant opt-out). Same for a user server named
                // like the gateway, which can't be switched off while the gateway uses its name.
                let listed = codex_user_mcp_servers(&bin_path, input.workspace_dir.as_deref(), self.profile_env.as_ref()).await.map_err(|e| {
                    tracing::warn!(error = %e, "could not list codex's own MCP servers; not running codex");
                    format!(
                        "kwnt couldn't check which MCP servers your codex config would start ({e}), so it didn't run codex. Check that `codex mcp list` works."
                    )
                });
                let checked = listed.and_then(|names| match codex_reserved_name_conflict(input.mcp.as_ref(), &names) {
                    Some(reason) => Err(reason),
                    None => Ok(names),
                });
                let user_servers = match checked {
                    Ok(names) => names,
                    Err(reason) => {
                        let msg = format!("⚠️ [{}] {reason}", input.kwant.name);
                        let _ = event_tx.send(RuntimeEvent::TextDelta(msg.clone())).await;
                        let _ = event_tx
                            .send(RuntimeEvent::TurnComplete {
                                tokens: 0,
                                duration_ms: start.elapsed().as_millis() as u64,
                                kwant_id: None,
                                profile_id: None,
                            })
                            .await;
                        return Ok(TurnOutput {
                            text: msg,
                            reasoning: None,
                            tokens_used: 0,
                            ..Default::default()
                        });
                    }
                };
                cmd.arg("exec");
                if let Some(model) = &input.model {
                    cmd.arg("-m").arg(model);
                }
                cmd.arg("--ephemeral")
                    .arg("--skip-git-repo-check")
                    .arg("--color")
                    .arg("never")
                    .arg("-c")
                    .arg(format!("reasoning_effort=\"{}\"", codex_effort))
                    // Never the user's own codex servers: codex exec has no hook for kwnt's
                    // approval cards. The kwant's servers come only through kwnt's gateway.
                    .arg("-c")
                    .arg(codex_mcp_servers(input.mcp.as_ref(), &user_servers))
                    // The model's shell sees only codex's core env, never kwnt's keys or rc exports
                    .args(CODEX_SHELL_POLICY)
                    // kwnt's routines and memory, never codex's own
                    .args(codex_feature_args());
                // codex exec has no approval hook: its sandbox keeps file writes (and shell
                // commands' writes) inside the bot's working folder instead of read-only everywhere
                if input.workspace_dir.as_deref().is_some_and(|d| d.exists()) {
                    cmd.arg("--sandbox").arg("workspace-write");
                }
                if let Some(gateway) = input.mcp.as_ref().and_then(|c| c.gateway.as_ref()) {
                    cmd.env(crate::mcp::GATEWAY_TOKEN_ENV, &gateway.token);
                    // codex's default shell env policy passes the token on to the model's shell
                    // commands; blank it there (a `set` entry merges, keeping the user's policy)
                    cmd.arg("-c").arg(format!("shell_environment_policy.set.{}=\"\"", crate::mcp::GATEWAY_TOKEN_ENV));
                }
                cmd.arg("-o")
                    .arg(&out_file)
                    .arg(&full_prompt);
                codex_out_file = Some(out_file);
            }
            "grok" => {
                let grok_effort = match effort {
                    "high" => "high",
                    "low" => "low",
                    _ => "medium",
                };
                let mut args: Vec<String> = input
                    .model
                    .iter()
                    .flat_map(|m| ["-m".to_string(), m.clone()])
                    .collect();
                args.extend([
                    // Without an explicit effort grok uses the user's default (often xhigh), which is far too slow for chat
                    "--reasoning-effort".into(),
                    grok_effort.into(),
                    "--no-plan".into(),
                    "--no-subagents".into(),
                ]);
                args.extend(grok_disallowed_args());
                args.extend([
                    "--rules".into(),
                    system_prompt.clone(),
                    "-p".into(),
                    effective_prompt.clone(),
                ]);
                let (text, tokens) = match self.run_grok(&bin_path, &input, args, &event_tx).await {
                    Ok(run) if run.stdout.trim().is_empty() && !run.tool_calls => {
                        match refusal(&format!("{}\n{}", run.errors.join("\n"), run.stderr)) {
                            Some(failure) => return Ok(TurnOutput::refused(failure)),
                            None => empty_grok_reply(&input, &run, &event_tx).await,
                        }
                    }
                    Ok(run) if run.stdout.trim().is_empty() => {
                        empty_grok_reply(&input, &run, &event_tx).await
                    }
                    Ok(run) => {
                        let tokens = run.stdout.len() / 4;
                        (run.stdout, tokens)
                    }
                    Err(e) => {
                        tracing::warn!(error = %e, binary = %bin_path, "grok run failed");
                        let msg = format!(
                            "⚠️ [{}] Execution error for {} CLI: {}",
                            input.kwant.name, self.tool_kind, e
                        );
                        let _ = event_tx.send(RuntimeEvent::TextDelta(msg.clone())).await;
                        (msg, 15)
                    }
                };
                let _ = event_tx
                    .send(RuntimeEvent::TurnComplete {
                        tokens,
                        duration_ms: start.elapsed().as_millis() as u64,
                        kwant_id: None,
                        profile_id: None,
                    })
                    .await;
                return Ok(TurnOutput {
                    text,
                    reasoning: Some(self.executed_via(effort)),
                    tokens_used: tokens,
                    ..Default::default()
                });
            }
            _ => {
                // claude default
                let claude_effort = match effort {
                    "high" => "high",
                    "low" => "low",
                    _ => "medium",
                };
                if let Some(model) = &input.model {
                    cmd.arg("--model").arg(model);
                }
                cmd.arg("--effort").arg(claude_effort);
                // kwnt decides every tool call. claude's own `auto` mode (picked on a fresh
                // config) would approve Bash itself and never ask kwnt, so no card and no record.
                cmd.arg("--permission-mode").arg("default");
                // Skip starting the user's personal MCP servers on every chat turn
                cmd.arg("--strict-mcp-config");
                // kwnt's routines and teammates, never claude's own scheduler or sessions
                cmd.args(claude_disallowed_args());
                if let Some(ctx) = &input.mcp {
                    match crate::mcp::cli_config::write_claude_config(ctx) {
                        Ok(file) => {
                            cmd.arg("--mcp-config").arg(&file.path);
                            cmd.arg("--permission-prompt-tool").arg(format!(
                                "mcp__{}__{}",
                                crate::mcp::RESERVED_SERVER_NAME,
                                crate::mcp::PERMISSION_TOOL
                            ));
                            // A tool call may wait for the user's approval; claude must not give up first
                            let slowest = ctx.servers.iter().map(|s| s.timeout_secs.max(0) as u64).max().unwrap_or(0);
                            let tool_timeout_ms = (crate::mcp::APPROVAL_TIMEOUT_SECS + 30).max(slowest) * 1000;
                            cmd.env("MCP_TOOL_TIMEOUT", tool_timeout_ms.to_string());
                            cmd.env("MCP_TIMEOUT", "30000");
                            _mcp_config = Some(file);
                        }
                        Err(e) => {
                            tracing::warn!(error = %e, "could not write MCP config; running without MCP");
                            let _ = event_tx
                                .send(RuntimeEvent::ThinkingDelta("MCP tools are unavailable this turn (config file error).".into()))
                                .await;
                        }
                    }
                }
                // claude's own tools follow kwnt's policy (see mcp::builtin_tools); tell the bot
                let folder = input.workspace_dir.as_deref().filter(|d| d.exists());
                let tools_block = if input.mcp.is_some() {
                    crate::mcp::builtin_tools::prompt_block(folder)
                } else {
                    String::new()
                };
                cmd.arg("--append-system-prompt").arg(format!("{system_prompt}{tools_block}"));
                // One JSON result: tells a usage limit or sign-out apart from a reply
                cmd.arg("--output-format").arg("json");
                cmd.arg("-p").arg(&effective_prompt);
            }
        }
        if let Some(env) = &self.profile_env {
            env.apply(&mut cmd);
            // claude -p has no subcommand, so its overrides can follow the other options
            cmd.args(&env.args);
        }

        // codex's reply comes from its -o file; an unread pipe could fill up and stall the CLI
        cmd.stdin(Stdio::null());
        cmd.stdout(if codex_out_file.is_some() {
            Stdio::null()
        } else {
            Stdio::piped()
        });
        // codex reports a usage limit or sign-out only on stderr
        cmd.stderr(if codex_out_file.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        });
        // A cancelled turn aborts this task; the CLI must not outlive it
        cmd.kill_on_drop(true);
        own_process_group(&mut cmd);

        match cmd.spawn() {
            Ok(mut child) => {
                let mut group = ProcessGroup::of(&child);
                let stdout_pipe = child.stdout.take();
                let stderr_pipe = child.stderr.take();
                let read_stdout = async {
                    match stdout_pipe {
                        Some(stdout) => read_tail(stdout, 8 * 1024 * 1024).await,
                        None => String::new(),
                    }
                };
                let read_stderr = async {
                    match stderr_pipe {
                        Some(stderr) => read_tail(stderr, GROK_STDERR_MAX).await,
                        None => String::new(),
                    }
                };
                let (stdout, stderr) = tokio::join!(read_stdout, read_stderr);
                let exit = child.wait().await.ok();
                group.disarm();

                let mut tokens = None;
                let output_str = if let Some(ref out_file) = codex_out_file {
                    let content = tokio::fs::read_to_string(out_file)
                        .await
                        .unwrap_or_default();
                    let _ = tokio::fs::remove_file(out_file).await;
                    let failed = !exit.is_some_and(|s| s.success());
                    if content.trim().is_empty() && failed && !codex_had_effects(&stderr) {
                        if let Some(failure) = refusal(&codex_error_lines(&stderr)) {
                            return Ok(TurnOutput::refused(failure));
                        }
                    }
                    content
                } else {
                    match ClaudeResult::parse(&stdout) {
                        Some(result) => {
                            if result.is_error && result.num_turns <= 1 {
                                if let Some(failure) = refusal(&result.result) {
                                    return Ok(TurnOutput::refused(failure));
                                }
                            }
                            tokens = result.tokens();
                            result.result
                        }
                        // A failed run with no output is an error, not an empty reply
                        None if stdout.trim().is_empty() && !exit.is_some_and(|s| s.success()) => {
                            let status = exit.and_then(|s| s.code()).map_or("a signal".to_string(), |c| format!("status {c}"));
                            tracing::warn!(binary = %bin_path, %status, "CLI exited with no output");
                            return Err(DriverError::Execution(format!("{} CLI exited with {status} and no output", self.tool_kind)));
                        }
                        // Not JSON (an older claude): its plain text is the reply
                        None => stdout,
                    }
                };
                let _ = event_tx
                    .send(RuntimeEvent::TextDelta(output_str.clone()))
                    .await;

                let duration = start.elapsed().as_millis() as u64;
                let tokens = tokens.unwrap_or(output_str.len() / 4);

                let _ = event_tx
                    .send(RuntimeEvent::TurnComplete {
                        tokens,
                        duration_ms: duration,
                        kwant_id: None,
                        profile_id: None,
                    })
                    .await;

                Ok(TurnOutput {
                    text: output_str,
                    reasoning: Some(self.executed_via(effort)),
                    tokens_used: tokens,
                    ..Default::default()
                })
            }
            Err(e) => {
                tracing::warn!(error = %e, binary = %bin_path, "CLI binary execution failed");
                let msg = format!(
                    "⚠️ [{}] Execution error for {} CLI: {}",
                    input.kwant.name, self.tool_kind, e
                );
                let _ = event_tx.send(RuntimeEvent::TextDelta(msg.clone())).await;
                let _ = event_tx
                    .send(RuntimeEvent::TurnComplete {
                        tokens: 15,
                        duration_ms: start.elapsed().as_millis() as u64,
                        kwant_id: None,
                        profile_id: None,
                    })
                    .await;

                Ok(TurnOutput {
                    text: msg,
                    reasoning: None,
                    tokens_used: 15,
                    ..Default::default()
                })
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::models::{Kwant, Message};

    #[test]
    fn grok_may_fetch_pages_and_edit_only_inside_the_folder() {
        assert_eq!(grok_builtin_rules(None), ["--allow", "WebFetch"]);
        let tmp = tempfile::tempdir().unwrap();
        let real = tmp.path().canonicalize().unwrap().display().to_string();
        let rules = grok_builtin_rules(Some(tmp.path()));
        assert_eq!(rules, ["--allow", "WebFetch", "--allow", &format!("Edit({real}/**)"), "--allow", &format!("Write({real}/**)")]);
        assert!(!rules.iter().any(|r| r.starts_with("Bash")), "shell commands stay refused");
    }

    #[test]
    fn engine_schedulers_and_sessions_are_switched_off() {
        let [flag, list] = claude_disallowed_args();
        assert_eq!(flag, "--disallowedTools");
        let tools: Vec<&str> = list.split(',').collect();
        for tool in ["CronCreate", "CronDelete", "CronList", "ScheduleWakeup", "RemoteTrigger", "PushNotification", "Skill(schedule)", "Skill(loop)"] {
            assert!(tools.contains(&tool), "{tool} missing from {list}");
        }
        // kwnt's own tools and claude's lookups stay usable
        assert!(!tools.iter().any(|t| t.starts_with("mcp__") || ["WebSearch", "WebFetch", "Read"].contains(t)));

        assert_eq!(grok_disallowed_args(), ["--disallowed-tools", "SchedulerCreate,SchedulerDelete,SchedulerList"]);

        let codex = codex_feature_args();
        assert_eq!(codex, ["-c", "features.in_app_local_automation=false", "-c", "features.memories=false"]);
        for pair in codex.iter().skip(1).step_by(2) {
            let table: toml::Table = pair.parse().unwrap();
            assert_eq!(table["features"].as_table().unwrap().values().next(), Some(&toml::Value::Boolean(false)), "{pair}");
        }
    }

    #[test]
    fn codex_shell_policy_is_core_only() {
        let args = CODEX_SHELL_POLICY.join(" ");
        assert_eq!(
            args,
            "-c shell_environment_policy.inherit=\"core\" -c shell_environment_policy.ignore_default_excludes=false"
        );
        // Each override must parse as TOML, the way `codex -c key=value` reads it
        for pair in CODEX_SHELL_POLICY.iter().skip(1).step_by(2) {
            let table: toml::Table = pair.parse().unwrap();
            assert!(table.contains_key("shell_environment_policy"), "{pair}");
        }
    }

    #[test]
    fn test_build_cli_prompt_with_history() {
        let kwant = Kwant {
            space_id: crate::db::spaces::DEFAULT_SPACE_ID.into(),
            id: "k1".into(),
            name: "Architect".into(),
            title: "Lead".into(),
            persona_prompt: "p".into(),
            soul: String::new(),
            avatar_sprite: "s".into(),
            driver_type: "cli".into(),
            preferred_model: "grok".into(),
            temperature: 0.7,
            max_tokens: 1000,
            level: 1,
            turns_completed: 0,
            created_at: None,
        };
        let input = TurnInput {
            continuity: String::new(),
            browser_api_url: None,
            thread_id: "t1".into(),
            kwant,
            prompt: "open some playlist page".into(),
            history: vec![
                Message {
                    id: "m1".into(),
                    channel_id: Some("c1".into()),
                    direct_kwant_id: None,
                    sender_role: "user".into(),
                    sender_id: "Player".into(),
                    text: "open booktun.ing".into(),
                    reasoning_content: None,
                    tool_calls: None,
                    tokens_used: 10,
                    created_at: None,
                    thread_id: None,
                    meta: None,
                },
                Message {
                    id: "m2".into(),
                    channel_id: Some("c1".into()),
                    direct_kwant_id: None,
                    sender_role: "kwant".into(),
                    sender_id: "Architect".into(),
                    text: "I opened booktun.ing for you.".into(),
                    reasoning_content: None,
                    tool_calls: None,
                    tokens_used: 20,
                    created_at: None,
                    thread_id: None,
                    meta: None,
                },
            ],
            model_override: None,
            model: None,
            effort_level: None,
            team_context: None,
            workspace_dir: None,
            sandbox_id: None,
            teammates: vec![],
            mcp: None,
        };

        let result = build_cli_prompt(&input);
        assert!(result.contains("### Conversation History:"));
        assert!(result.contains("User: open booktun.ing"));
        assert!(result.contains("Assistant: I opened booktun.ing for you."));
        assert!(result.contains("### Current User Request:\nopen some playlist page"));
    }

    #[test]
    fn test_build_cli_prompt_without_history() {
        let kwant = Kwant {
            space_id: crate::db::spaces::DEFAULT_SPACE_ID.into(),
            id: "k1".into(),
            name: "Architect".into(),
            title: "Lead".into(),
            persona_prompt: "p".into(),
            soul: String::new(),
            avatar_sprite: "s".into(),
            driver_type: "cli".into(),
            preferred_model: "grok".into(),
            temperature: 0.7,
            max_tokens: 1000,
            level: 1,
            turns_completed: 0,
            created_at: None,
        };
        let input = TurnInput {
            continuity: String::new(),
            browser_api_url: None,
            thread_id: "t1".into(),
            kwant,
            prompt: "hello world".into(),
            history: vec![],
            model_override: None,
            model: None,
            effort_level: None,
            team_context: None,
            workspace_dir: None,
            sandbox_id: None,
            teammates: vec![],
            mcp: None,
        };

        let result = build_cli_prompt(&input);
        assert_eq!(result, "hello world");
    }
}
