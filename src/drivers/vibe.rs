//! Isolation for Mistral Vibe CLI runs (35b). Vibe 3.1.4 reads settings, MCP servers, skills,
//! plugins, trusted folders and history from its own home (`$VIBE_HOME`, default `~/.vibe`),
//! phones home (telemetry, update checks, remote experiments that can change its prompt and
//! tools), and its bash tool passes Vibe's whole env, API key included, to every command.
//!
//! So every kwant thread gets a private `VIBE_HOME` (0700) holding only kwnt's `config.toml`,
//! with every network feature off. Vibe's env is cleared down to [`KEPT_ENV`]; the model's shell
//! goes through a wrapper that keeps only PATH, HOME, LANG and TERM. Each tool's permission is
//! set for the turn (see [`tool_permissions`]), and an unlisted tool needs an approval a
//! headless run can't give, so it is refused. kwnt's gateway is the only MCP server.
//!
//! Vibe never gets `--trust`, `--auto-approve`/`--yolo` or `--resume`, and always gets
//! `--time-limit`.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde_json::{json, Value};

/// The Vibe release this driver was checked against (the Docker image installs it).
pub const PINNED_VERSION: &str = "3.1.4";

/// Vibe reads its Mistral key from this variable, from `$VIBE_HOME/.env` or from the system
/// keyring. Vibe needs it even for other providers.
pub const KEY_ENV: &str = "MISTRAL_API_KEY";

/// Longest a Vibe turn runs: `--time-limit`, after which Vibe stops itself (exit 3, `deadline`).
pub const TIME_LIMIT_SECS: u64 = 60 * 60;

const DIR_PREFIX: &str = "kwnt-vibe-";

/// A thread's home is removed after this long unused; no turn lasts this long.
const STALE_AFTER: Duration = Duration::from_secs(24 * 60 * 60);

/// Variables of kwnt's env Vibe still gets. Everything else (provider keys, tokens, kwnt's own
/// settings) is cleared. `LC_*` is kept too (see [`kept_env`]).
pub const KEPT_ENV: [&str; 23] = [
    "PATH",
    "LANG",
    "TERM",
    "TZ",
    "USER",
    "LOGNAME",
    "SHELL",
    "SSL_CERT_FILE",
    "SSL_CERT_DIR",
    "HTTPS_PROXY",
    "https_proxy",
    "HTTP_PROXY",
    "http_proxy",
    "NO_PROXY",
    "no_proxy",
    "ALL_PROXY",
    "all_proxy",
    // Windows can't start a process without these
    "SYSTEMROOT",
    "SystemRoot",
    "WINDIR",
    "COMSPEC",
    "PATHEXT",
    "PROGRAMDATA",
];

/// Whether kwnt's env variable `name` is passed on to Vibe.
pub fn kept_env(name: &str) -> bool {
    KEPT_ENV.contains(&name) || name.starts_with("LC_")
}

/// Every built-in tool name Vibe 3.1.4's config knows. Each gets a permission on every launch,
/// so none runs on Vibe's own default.
pub const CORE_TOOLS: [&str; 25] = [
    "ask_user_question",
    "bash",
    "bash_log_file",
    "bash_output",
    "bash_sessions",
    "bash_stdin",
    "edit",
    "git_bash",
    "git_bash_log_file",
    "git_bash_output",
    "git_bash_sessions",
    "git_bash_stdin",
    "grep",
    "powershell",
    "powershell_log_file",
    "powershell_output",
    "powershell_sessions",
    "powershell_stdin",
    "read_file",
    "skill",
    "task",
    "todo",
    "web_fetch",
    "web_search",
    "write_file",
];

/// The file tools a turn with a working folder may use without a card, kept inside the folder
/// (`outside_workdir = "deny"`). Like grok, shell commands and web fetches stay refused: Vibe
/// can't ask kwnt first, and a fetch could reach kwnt's own API on this machine.
pub const FOLDER_TOOLS: [&str; 4] = ["read_file", "grep", "write_file", "edit"];

/// Tools a no-tools turn may still see run: Vibe 3.1.4 always offers these (its TypeScript
/// sandbox and tool search, `wait`), and they reach nothing but other tools, which are refused.
pub const SANDBOX_TOOLS: [&str; 3] = ["run_typescript", "search_tool_functions", "wait"];

/// A name a gateway tool may have in Vibe's config: what [`crate::mcp::session::vibe_tool_names`]
/// makes, which is never longer than this.
fn valid_tool_name(name: &str) -> bool {
    !name.is_empty() && name.len() <= 200 && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// A model name Vibe gets on its command line and in `VIBE_MODELS`. It starts with a letter or
/// digit, so it can never be read as a flag (`--trust`, `-y`) or a hidden/relative name.
pub fn valid_model(model: &str) -> bool {
    model.starts_with(|c: char| c.is_ascii_alphanumeric())
        && model.len() <= 64
        && model != "default"
        && model.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
}

/// Model aliases Vibe 3.1.4 defines itself; a turn on one passes only `--model`.
const BUILTIN_MODELS: [&str; 2] = ["mistral-large-4-0", "mistral-medium-3.5"];

/// A Mistral key as Vibe would send it: printable, no spaces, of a sane length. Anything else
/// is not passed on (Vibe then reports it has no key).
pub fn valid_key(key: &str) -> bool {
    (8..=512).contains(&key.len()) && key.chars().all(|c| c.is_ascii_graphic())
}

/// The user's real Vibe home: `$VIBE_HOME`, else `~/.vibe`. Only read, never written.
pub fn user_vibe_home() -> Option<PathBuf> {
    match std::env::var_os("VIBE_HOME") {
        Some(dir) if !dir.is_empty() => Some(PathBuf::from(dir)),
        _ => dirs::home_dir().map(|h| h.join(".vibe")),
    }
}

/// `MISTRAL_API_KEY` in a dotenv file (`vibe --setup` writes `~/.vibe/.env`): `KEY=value`,
/// optionally `export`ed and quoted.
pub fn key_from_dotenv(text: &str) -> Option<String> {
    text.lines().find_map(|line| {
        let line = line.trim();
        let line = line.strip_prefix("export ").unwrap_or(line).trim_start();
        let (name, value) = line.split_once('=')?;
        if name.trim() != KEY_ENV {
            return None;
        }
        let value = value.trim();
        let value = ['"', '\'']
            .iter()
            .find_map(|q| value.strip_prefix(*q).and_then(|v| v.strip_suffix(*q)))
            .unwrap_or(value);
        Some(value.to_string()).filter(|v| valid_key(v))
    })
}

/// Where a Vibe key was found, for the engine check's detail line.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeySource {
    /// `$VIBE_HOME/.env` or `~/.vibe/.env` (what `vibe --setup` writes).
    VibeEnvFile,
    /// The macOS Keychain entry `vibe --setup` writes.
    Keychain,
    /// `MISTRAL_API_KEY` in kwnt's env.
    Env,
}

impl KeySource {
    pub fn label(self) -> &'static str {
        match self {
            KeySource::VibeEnvFile => "Vibe's .env",
            KeySource::Keychain => "macOS Keychain",
            KeySource::Env => "MISTRAL_API_KEY",
        }
    }
}

/// The Mistral key Vibe would use on this computer: its own `.env`, then (on a Mac) the Keychain
/// entry `vibe --setup` writes, then kwnt's env. `None`: Vibe isn't set up.
pub async fn find_key(vibe_home: Option<&Path>) -> Option<(String, KeySource)> {
    if let Some(key) = vibe_home
        .and_then(|h| std::fs::read_to_string(h.join(".env")).ok())
        .and_then(|t| key_from_dotenv(&t))
    {
        return Some((key, KeySource::VibeEnvFile));
    }
    #[cfg(target_os = "macos")]
    if let Some(key) = keychain_key().await {
        return Some((key, KeySource::Keychain));
    }
    std::env::var(KEY_ENV)
        .ok()
        .map(|k| k.trim().to_string())
        .filter(|k| valid_key(k))
        .map(|k| (k, KeySource::Env))
}

#[cfg(target_os = "macos")]
async fn keychain_key() -> Option<String> {
    let out = tokio::process::Command::new("/usr/bin/security")
        .args(["find-generic-password", "-s", "ai.mistral.vibe", "-a", KEY_ENV, "-w"])
        .stdin(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .kill_on_drop(true)
        .output()
        .await
        .ok()?;
    let key = String::from_utf8_lossy(&out.stdout).trim().to_string();
    (out.status.success() && valid_key(&key)).then_some(key)
}

/// kwnt's `config.toml` for Vibe: every network feature off (telemetry, update checks and
/// auto-update, notifications, connectors, OpenTelemetry, PR monitoring, remote experiments,
/// the model catalog, registry skills), no background processes or subagents, no session log
/// or generated titles, no skills, file tools held to the working folder, and every tool not
/// set for the turn left to an approval Vibe can't get headless.
pub fn config() -> toml::Table {
    let mut config = toml::Table::new();
    for key in [
        "enable_telemetry",
        "enable_update_checks",
        "enable_auto_update",
        "enable_notifications",
        "enable_connectors",
        "enable_otel",
        "enable_pr_monitoring",
        "enable_background_processes",
        "enable_subagents",
        "experimental_enable_registry_skills",
        "experimental_enable_model_catalog",
    ] {
        config.insert(key.into(), false.into());
    }
    config.insert("api_retry_max_elapsed_time".into(), 20.into());
    config.insert("outside_workdir".into(), "deny".into());
    config.insert("default_approval_policy".into(), "ask".into());
    // A name no skill has: Vibe's own skills (and any in ~/.agents) stay off
    config.insert("enabled_skills".into(), toml::Value::Array(vec!["kwnt-none".into()]));
    let mut logging = toml::Table::new();
    logging.insert("enabled".into(), false.into());
    logging.insert("generate_titles".into(), false.into());
    config.insert("session_logging".into(), logging.into());
    let mut experiments = toml::Table::new();
    experiments.insert("enable".into(), false.into());
    config.insert("experiments".into(), experiments.into());
    config
}

/// The shell wrapper Vibe's bash tool runs commands through: a clean env with only PATH, HOME,
/// LANG and TERM, so a command never sees the key, the gateway token or kwnt's env.
pub const SCRUB_SHELL: &str = "#!/bin/sh\n\
# kwnt: the model's shell commands get no API key, token or other inherited env\n\
exec /usr/bin/env -i PATH=\"$PATH\" HOME=\"$HOME\" LANG=\"${LANG:-C.UTF-8}\" TERM=dumb /bin/bash \"$@\"\n";

/// What the turn lets Vibe's own tools do.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolMode {
    /// A no-tools turn: every tool refused.
    None,
    /// No working folder: every tool refused (file tools would work in kwnt's own folder).
    NoFolder,
    /// The bot's working folder: [`FOLDER_TOOLS`] inside it, everything else refused.
    Folder,
}

/// `VIBE_TOOLS`: a permission for every core tool and each gateway tool (`kwnt_<name>`).
/// Gateway tools run without Vibe asking, because kwnt's gateway shows its approval card for
/// each call. `shell` is the scrub wrapper's path (unix).
pub fn tool_permissions(mode: ToolMode, shell: Option<&Path>, gateway_tools: &[String]) -> Value {
    let mut tools = serde_json::Map::new();
    for tool in CORE_TOOLS {
        let allowed = mode == ToolMode::Folder && FOLDER_TOOLS.contains(&tool);
        let mut entry = json!({ "permission": if allowed { "always" } else { "never" } });
        if FOLDER_TOOLS.contains(&tool) {
            entry["outside_workdir"] = "deny".into();
        }
        if tool == "bash" {
            if let Some(shell) = shell {
                entry["shell"] = shell.display().to_string().into();
            }
        }
        tools.insert(tool.into(), entry);
    }
    if mode != ToolMode::None {
        for name in gateway_tools.iter().filter(|n| valid_tool_name(n)) {
            tools.insert(
                format!("{}_{name}", crate::mcp::RESERVED_SERVER_NAME),
                json!({ "permission": "always" }),
            );
        }
    }
    Value::Object(tools)
}

/// `VIBE_ENABLED_TOOLS`: only the gateway's tools among the tools Vibe loads itself (its todo
/// list and scratchpad stay off). A name no tool has when there are none.
pub fn enabled_tools(gateway_tools: &[String]) -> Value {
    let names: Vec<String> = gateway_tools
        .iter()
        .filter(|n| valid_tool_name(n))
        .map(|n| format!("{}_{n}", crate::mcp::RESERVED_SERVER_NAME))
        .collect();
    if names.is_empty() {
        json!(["kwnt-none"])
    } else {
        json!(names)
    }
}

/// `VIBE_MCP_SERVERS`: kwnt's gateway as the only MCP server, its bearer token read from
/// [`crate::mcp::GATEWAY_TOKEN_ENV`] (never written to disk). `slowest` is the longest timeout
/// among the kwant's servers. Empty without a gateway, so no user server starts.
pub fn mcp_servers(gateway_url: Option<&str>, slowest: u64) -> Value {
    let Some(url) = gateway_url else {
        return json!([]);
    };
    json!([{
        "name": crate::mcp::RESERVED_SERVER_NAME,
        "transport": "streamable-http",
        "url": url,
        "auth": { "type": "static", "api_key_env": crate::mcp::GATEWAY_TOKEN_ENV },
        // The gateway connects to every server on the first tools/list
        "startup_timeout_sec": (slowest + 15).max(30),
        // A call may wait for the user's approval; Vibe must not give up first
        "tool_timeout_sec": (crate::mcp::APPROVAL_TIMEOUT_SECS + 30).max(slowest),
    }])
}

/// `VIBE_MODELS` for a model Vibe doesn't define itself, on Mistral's API. `None` for Vibe's own
/// aliases and for no model.
pub fn models(model: Option<&str>) -> Option<Value> {
    let model = model.filter(|m| valid_model(m) && !BUILTIN_MODELS.contains(m))?;
    Some(json!([{ "name": model, "provider": "mistral", "alias": model }]))
}

/// The line a turn gets when its model can't be handed to Vibe ([`valid_model`]), so kwnt never
/// runs Vibe's default model in its place. `None` for a model Vibe can take.
pub fn bad_model_notice(model: &str) -> Option<String> {
    (!valid_model(model)).then(|| {
        "Mistral Vibe can't run this bot's pinned model: Vibe takes a Mistral model id of letters, digits, \
         '.', '_' and '-', such as mistral-medium-3.5, mistral-large-4-0, mistral-small-latest or \
         ministral-8b-latest. Pin one of those, or no model, and send the message again."
            .to_string()
    })
}

/// Vibe's command line for one turn, after the binary.
pub fn args(turn: &TurnDir, model: Option<&str>) -> Vec<String> {
    let mut args = vec![
        "--prompt-file".to_string(),
        turn.prompt().display().to_string(),
        "--system-prompt-file".into(),
        turn.system().display().to_string(),
        "--output".into(),
        "streaming".into(),
        "--output-dir".into(),
        turn.out().display().to_string(),
        "--time-limit".into(),
        TIME_LIMIT_SECS.to_string(),
        // Anything not allowed by the turn's permissions is asked for, and so refused
        "--approval-policy".into(),
        "ask".into(),
    ];
    if let Some(model) = model.filter(|m| valid_model(m)) {
        args.extend(["--model".into(), model.to_string()]);
    }
    args
}

/// A kwant thread's private Vibe home. A turn with no thread gets one of its own, removed when
/// it ends; a thread's home stays for its next turn and is swept after [`STALE_AFTER`] unused.
#[derive(Debug)]
pub struct VibeHome {
    pub path: PathBuf,
    /// Removed on drop: the home of a turn with no thread.
    temporary: bool,
}

impl Drop for VibeHome {
    fn drop(&mut self) {
        if self.temporary {
            remove_dir(&self.path);
        }
    }
}

fn remove_dir(path: &Path) {
    if let Err(e) = std::fs::remove_dir_all(path) {
        if e.kind() != std::io::ErrorKind::NotFound {
            tracing::warn!(error = %e, dir = %path.display(), "could not remove a vibe temp folder");
        }
    }
}

fn private_dir(path: &Path) -> std::io::Result<()> {
    #[cfg_attr(not(unix), allow(unused_mut))]
    let mut builder = std::fs::DirBuilder::new();
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        builder.mode(0o700);
    }
    match builder.create(path) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
            // Never a link someone planted in the temp folder, or a file
            let meta = std::fs::symlink_metadata(path)?;
            if !meta.is_dir() {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::AlreadyExists,
                    format!("{} is not a folder", path.display()),
                ));
            }
            #[cfg(unix)]
            {
                use std::os::unix::fs::{MetadataExt, PermissionsExt};
                // SAFETY: getuid has no preconditions
                let uid = unsafe { libc::getuid() };
                if meta.uid() != uid {
                    return Err(std::io::Error::new(
                        std::io::ErrorKind::PermissionDenied,
                        format!("{} belongs to another user", path.display()),
                    ));
                }
                std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700))?;
            }
            Ok(())
        }
        Err(e) => Err(e),
    }
}

/// Writes `contents` to `path` (0600 on unix, `mode` for the shell wrapper) through a fresh
/// temp file renamed over it, so a turn running at the same time never reads half a file.
fn write_atomic(path: &Path, contents: &[u8], #[allow(unused)] mode: u32) -> std::io::Result<()> {
    let tmp = path.with_extension(format!("tmp-{}", uuid::Uuid::new_v4().simple()));
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(mode);
    }
    let written = (|| {
        let mut file = options.open(&tmp)?;
        file.write_all(contents)?;
        file.sync_all()?;
        std::fs::rename(&tmp, path)
    })();
    if written.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    written
}

/// The home folder name of a kwant's thread: a hash, so ids never become paths.
fn home_name(kwant_id: &str, thread_id: &str) -> String {
    use sha2::{Digest, Sha256};
    let digest = Sha256::digest(format!("{kwant_id}\0{thread_id}").as_bytes());
    let hex: String = digest.iter().take(8).map(|b| format!("{b:02x}")).collect();
    format!("{DIR_PREFIX}{hex}")
}

impl VibeHome {
    /// `<base>/kwnt-vibe-<hash>` for the kwant's thread (0700), with kwnt's [`config`] written
    /// fresh, which also marks the home as used. An empty `thread_id` gets a new home that is
    /// removed with this value.
    pub fn for_thread(base: &Path, kwant_id: &str, thread_id: &str) -> std::io::Result<Self> {
        let (name, temporary) = if thread_id.is_empty() {
            (format!("{DIR_PREFIX}{}", uuid::Uuid::new_v4().simple()), true)
        } else {
            (home_name(kwant_id, thread_id), false)
        };
        let path = base.join(name);
        sweep_stale(base, &path);
        private_dir(&path)?;
        let home = Self { path, temporary };
        private_dir(&home.home_dir())?;
        let text = toml::to_string(&config())
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
        write_atomic(&home.path.join("config.toml"), text.as_bytes(), 0o600)?;
        Ok(home)
    }

    /// `HOME` for Vibe and its commands, so `~/.agents` and `~/.vibe` are never the user's.
    pub fn home_dir(&self) -> PathBuf {
        self.path.join("home")
    }

    /// Writes the [`SCRUB_SHELL`] wrapper (0700) and returns its path.
    pub fn scrub_shell(&self) -> std::io::Result<PathBuf> {
        let path = self.path.join("kwnt-shell.sh");
        write_atomic(&path, SCRUB_SHELL.as_bytes(), 0o700)?;
        Ok(path)
    }

    /// A folder for one turn's prompt files, temp files and Vibe's `export.json`, removed when
    /// the turn ends.
    pub fn turn(&self) -> std::io::Result<TurnDir> {
        let path = self.path.join(format!("turn-{}", uuid::Uuid::new_v4().simple()));
        private_dir(&path)?;
        let turn = TurnDir { path };
        private_dir(&turn.out())?;
        private_dir(&turn.tmp())?;
        Ok(turn)
    }
}

/// One turn's files inside a [`VibeHome`]. Dropping it removes them.
#[derive(Debug)]
pub struct TurnDir {
    pub path: PathBuf,
}

impl Drop for TurnDir {
    fn drop(&mut self) {
        remove_dir(&self.path);
    }
}

impl TurnDir {
    pub fn prompt(&self) -> PathBuf {
        self.path.join("prompt.md")
    }
    pub fn system(&self) -> PathBuf {
        self.path.join("system.md")
    }
    pub fn out(&self) -> PathBuf {
        self.path.join("out")
    }
    /// `TMPDIR` for Vibe: its own temp folders go here, not in the shared temp folder.
    pub fn tmp(&self) -> PathBuf {
        self.path.join("tmp")
    }

    pub fn write_prompts(&self, prompt: &str, system: &str) -> std::io::Result<()> {
        write_atomic(&self.prompt(), prompt.as_bytes(), 0o600)?;
        write_atomic(&self.system(), system.as_bytes(), 0o600)
    }

    /// Vibe's `export.json`, written on every exit; `None` when it didn't get that far.
    pub fn export(&self) -> Option<Export> {
        let text = std::fs::read_to_string(self.out().join("export.json")).ok()?;
        Export::parse(&text)
    }
}

/// Removes thread homes nobody used for [`STALE_AFTER`], and those of turns with no thread left
/// by a kwnt that was killed. `keep` is the home about to be used.
fn sweep_stale(base: &Path, keep: &Path) {
    let Ok(entries) = std::fs::read_dir(base) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path == keep {
            continue;
        }
        let stale = entry.file_name().to_str().is_some_and(|n| n.starts_with(DIR_PREFIX))
            && std::fs::symlink_metadata(&path)
                .ok()
                .filter(|m| m.is_dir())
                .and_then(|m| m.modified().ok())
                .and_then(|t| t.elapsed().ok())
                .is_some_and(|age| age > STALE_AFTER);
        if stale {
            let _ = std::fs::remove_dir_all(&path);
        }
    }
}

/// What Vibe's `export.json` says about a run.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Export {
    /// `finished`, `config_error`, `infrastructure_failure`, `deadline`, `terminated`, …
    pub outcome: Option<String>,
    pub stop_reason: Option<String>,
    pub error_message: Option<String>,
    /// `invalid_api_key`, `rate_limit`, `model_billing_denied`, `usage_limit`, `backend_error`, …
    pub error_code: Option<String>,
    pub usage: Option<super::TurnUsage>,
    /// The model the run used (`config.active_model`).
    pub model: Option<String>,
}

impl Export {
    pub fn parse(text: &str) -> Option<Self> {
        let v: Value = serde_json::from_str(text).ok()?;
        let s = |v: &Value| v.as_str().map(str::trim).filter(|s| !s.is_empty()).map(str::to_string);
        let usage = v["usage"].as_object().map(|u| {
            let n = |k: &str| u.get(k).and_then(Value::as_u64).unwrap_or(0);
            let cached = n("cached_input_tokens");
            super::TurnUsage {
                fresh_input: n("input_tokens").saturating_sub(cached),
                cache_read: cached,
                cache_write: 0,
                output: n("output_tokens"),
                cost_usd: v["cost_usd"].as_f64().filter(|c| c.is_finite() && *c >= 0.0),
            }
        });
        Some(Self {
            outcome: s(&v["outcome"]),
            stop_reason: s(&v["stop_reason"]),
            error_message: s(&v["error"]["message"]),
            error_code: s(&v["error"]["code"]),
            usage: usage.filter(|u| u.fresh_input + u.cache_read + u.output > 0 || u.cost_usd.is_some_and(|c| c > 0.0)),
            model: s(&v["config"]["active_model"]).filter(|m| valid_model(m)),
        })
    }
}

/// Vibe's error codes for a quota or billing refusal: hold the account until the quota's reset.
const BILLING_CODES: [&str; 2] = ["model_billing_denied", "usage_limit"];

/// Vibe's error code for an HTTP 429.
const RATE_LIMIT_CODE: &str = "rate_limit";

/// How long a 429 without a Retry-After holds the account: a rate limit clears in about a
/// minute, so a short hold, never the hour a spent quota gets.
pub const RATE_LIMIT_HOLD_SECS: i64 = 5 * 60;

/// The longest Retry-After honoured; a longer one is held this long, then checked again.
const MAX_RETRY_AFTER_SECS: u64 = 24 * 60 * 60;

/// Seconds from a Retry-After in Vibe's error text (`retry-after: 30`, `"retry_after": 30`,
/// `retry after 30s`). Only seconds; anything else is `None`.
pub fn retry_after_secs(text: &str) -> Option<u64> {
    let lower = text.to_lowercase();
    ["retry-after", "retry_after", "retry after"].iter().find_map(|name| {
        let rest = &lower[lower.find(name)? + name.len()..];
        let rest = rest.trim_start_matches([' ', ':', '=', '"', '\'']);
        let digits: String = rest.chars().take_while(char::is_ascii_digit).take(9).collect();
        digits.parse::<u64>().ok().map(|s| s.min(MAX_RETRY_AFTER_SECS))
    })
}

/// A refusal that should move the turn to another account, as quota: a 402 or Vibe's billing
/// codes (or a 429 whose body says the quota is spent) hold the account until the quota's
/// reset, an hour when it doesn't say; any other 429 (an unknown body counts as a rate limit)
/// holds it for its Retry-After, else [`RATE_LIMIT_HOLD_SECS`]. A bad or missing key is auth.
/// Only for a run that did nothing (no reply, no tool call); the caller checks that.
pub fn failover(exit_code: Option<i32>, export: Option<&Export>, stderr: &str) -> Option<crate::profiles::ProviderFailure> {
    use crate::profiles::{classify, FailClass, ProviderFailure};
    let message = export.and_then(|e| e.error_message.clone()).unwrap_or_default();
    let code = export.and_then(|e| e.error_code.as_deref()).unwrap_or_default();
    let text = format!("{message}\n{stderr}");
    let lower = text.to_lowercase();
    let now = chrono::Utc::now();
    let detail = classify::redact_detail(text.trim(), 300);
    let billing = BILLING_CODES.contains(&code) || (exit_code == Some(2) && lower.contains("402"));
    let rate = code == RATE_LIMIT_CODE
        || (exit_code == Some(2) && (lower.contains("429") || lower.contains("too many requests")));
    let spent = || matches!(classify::classify(None, &text, now), Some((FailClass::Quota, _)));
    if billing || (rate && spent()) {
        let retry_at = classify::classify(Some(402), &text, now).and_then(|(_, at)| at);
        return Some(ProviderFailure { class: FailClass::Quota, retry_at, detail });
    }
    if rate {
        let hold = retry_after_secs(&text).map_or(RATE_LIMIT_HOLD_SECS, |s| s.max(1) as i64);
        let retry_at = Some(now + chrono::Duration::seconds(hold));
        return Some(ProviderFailure { class: FailClass::Quota, retry_at, detail });
    }
    let auth = code == "invalid_api_key"
        || lower.contains("missing mistral_api_key")
        || (exit_code == Some(1) && lower.contains("invalid api key"));
    if auth {
        return Some(ProviderFailure { class: FailClass::Auth, retry_at: None, detail });
    }
    let (class, retry_at) = classify::classify(None, &text, now)?;
    class.fails_over().then_some(ProviderFailure { class, retry_at, detail })
}

/// Longest refused-tool label quoted in a notice.
const REFUSED_LABEL_CHARS: usize = 120;

/// Longest piece of Vibe's output quoted in a notice.
const NOTICE_TAIL_CHARS: usize = 600;

/// What a Vibe run that produced no reply left behind, for [`empty_reply_notice`].
#[derive(Debug, Default)]
pub struct EmptyRun<'a> {
    pub stderr: &'a str,
    pub export: Option<&'a Export>,
    /// Tools Vibe refused (`tool_denied`), by name.
    pub refused: &'a [String],
    /// `None` when Vibe was killed by a signal or its status is unknown.
    pub exit_code: Option<i32>,
}

/// The message shown instead of an empty reply: why Vibe gave none, with quoted output redacted
/// (`secrets`: the key, the gateway token and the kwant's server secrets) and shortened.
pub fn empty_reply_notice(run: &EmptyRun<'_>, secrets: &[String]) -> String {
    let quote = |text: &str| {
        let text = crate::mcp::redact(text.trim(), secrets);
        let start = text.char_indices().rev().nth(NOTICE_TAIL_CHARS - 1).map_or(0, |(i, _)| i);
        if start > 0 {
            format!("…{}", &text[start..])
        } else {
            text
        }
    };
    if !run.refused.is_empty() {
        let label = |tool: &str| {
            let tool = crate::mcp::redact(tool, secrets);
            match tool.char_indices().nth(REFUSED_LABEL_CHARS) {
                Some((cut, _)) => format!("“{}…”", &tool[..cut]),
                None => format!("“{tool}”"),
            }
        };
        return format!(
            "Vibe stopped without a reply: it asked to use {}, which kwant doesn't allow in this chat. \
             Vibe can't run shell commands or fetch web pages in kwant. When a chat allows tools, it can read \
             and edit files in the bot's working folder and use the bot's assigned tools (each one asks first \
             with an approval card).",
            run.refused.iter().map(|t| label(t)).collect::<Vec<_>>().join(", ")
        );
    }
    let export = run.export;
    let code = export.and_then(|e| e.error_code.as_deref());
    let message = export.and_then(|e| e.error_message.as_deref());
    let lower = format!("{}\n{}", message.unwrap_or_default(), run.stderr).to_lowercase();
    if code == Some("invalid_api_key") || lower.contains("missing mistral_api_key") {
        return "Mistral Vibe isn't set up on this computer: it has no working Mistral API key. Run `vibe --setup`, then try again.".into();
    }
    match export.and_then(|e| e.outcome.as_deref()) {
        Some("deadline") => {
            return format!("Vibe ran out of time ({} minutes) before it replied.", TIME_LIMIT_SECS / 60)
        }
        Some("terminated") | Some("aborted") => return "Vibe's turn was stopped before it replied.".into(),
        _ => {}
    }
    if let Some(message) = message {
        return format!("Vibe stopped with an error: {}", quote(message));
    }
    let ended = match run.exit_code {
        Some(0) => "Vibe finished without a reply.".to_string(),
        Some(3) => "Vibe's turn was stopped before it replied.".to_string(),
        Some(code) => format!("Vibe exited with code {code} without a reply."),
        None => "Vibe was stopped before it replied.".to_string(),
    };
    match run.stderr.trim() {
        "" => ended,
        tail => format!("{ended} Its last output:\n{}", quote(tail)),
    }
}

/// The names kwnt's gateway offers Vibe this turn (`tools/list` on its Vibe naming), so each can
/// be allowed by name: Vibe has no wildcard permissions.
pub async fn gateway_tool_names(url: &str, token: &str, timeout: Duration) -> Result<Vec<String>, String> {
    let client = reqwest::Client::builder()
        .timeout(timeout)
        .no_proxy()
        .build()
        .map_err(|e| e.to_string())?;
    let reply = client
        .post(url)
        .bearer_auth(token)
        .header(reqwest::header::ACCEPT, "application/json, text/event-stream")
        .json(&json!({ "jsonrpc": "2.0", "id": 1, "method": "tools/list", "params": {} }))
        .send()
        .await
        .map_err(|e| format!("kwant's tool service didn't answer ({})", e.without_url()))?;
    if !reply.status().is_success() {
        return Err(format!("kwant's tool service answered {}", reply.status()));
    }
    let body: Value = reply.json().await.map_err(|_| "kwant's tool list couldn't be read".to_string())?;
    let names: Vec<String> = body["result"]["tools"]
        .as_array()
        .ok_or("kwant's tool service sent no tool list")?
        .iter()
        .filter_map(|t| t["name"].as_str())
        .filter(|n| valid_tool_name(n))
        .map(str::to_string)
        .collect();
    Ok(names)
}

/// One event of Vibe's `--output streaming` that kwnt acts on.
#[derive(Debug, Clone, PartialEq)]
pub enum StreamEvent {
    /// New reply text.
    Text(String),
    /// New thinking text.
    Thought(String),
    /// Vibe ran a tool. Vibe reports a tool once it settles, so its start and end come together.
    ToolStart { id: String, name: String, args: Value },
    /// A tool Vibe started ended; `error` says why when it didn't succeed.
    ToolEnd { id: String, ok: bool, error: Option<String> },
    /// Vibe refused a tool (`tool_denied`): a headless run can't approve it.
    Refused(String),
}

/// Splits Vibe's NDJSON output into events. Messages are keyed by `id`, so a message Vibe prints
/// again as it grows only adds its new text. Lines that aren't JSON are skipped.
#[derive(Debug, Default)]
pub struct StreamParser {
    pending: String,
    /// Text already passed on, by message id.
    said: std::collections::HashMap<String, String>,
    /// Tools started and not yet ended, by `toolCallId`.
    running: std::collections::HashSet<String>,
    /// Tools seen at all.
    seen: std::collections::HashSet<String>,
}

/// The tool's name in an effect: `file_system.bash` gives `bash`'s full title as Vibe shows it.
fn effect_name(v: &Value) -> String {
    [&v["detail"]["toolName"], &v["title"], &v["detail"]["kind"]]
        .into_iter()
        .find_map(|n| n.as_str().map(str::trim).filter(|n| !n.is_empty()))
        .unwrap_or("tool")
        .to_string()
}

/// The bare tool name of a Vibe effect title (`file_system.bash` → `bash`).
pub fn bare_tool_name(name: &str) -> &str {
    name.rsplit('.').next().unwrap_or(name)
}

impl StreamParser {
    pub fn push(&mut self, chunk: &str) -> Vec<StreamEvent> {
        self.pending.push_str(chunk);
        let mut events = Vec::new();
        while let Some(end) = self.pending.find('\n') {
            let line: String = self.pending.drain(..=end).collect();
            events.extend(self.parse_line(&line));
        }
        events
    }

    /// Whatever is left after the last newline.
    pub fn finish(&mut self) -> Vec<StreamEvent> {
        let rest = std::mem::take(&mut self.pending);
        self.parse_line(&rest)
    }

    fn parse_line(&mut self, line: &str) -> Vec<StreamEvent> {
        let Ok(v) = serde_json::from_str::<Value>(line.trim()) else {
            return Vec::new();
        };
        match v["type"].as_str() {
            Some("message") if v["role"].as_str() == Some("assistant") => {
                self.grown(&v, "message").map(StreamEvent::Text).into_iter().collect()
            }
            Some("reasoning") => self.grown(&v, "reasoning").map(StreamEvent::Thought).into_iter().collect(),
            Some("effect") => self.effect(&v),
            _ => Vec::new(),
        }
    }

    /// The text a message or reasoning line adds to what was passed on for its id.
    fn grown(&mut self, v: &Value, kind: &str) -> Option<String> {
        let text: String = match &v["content"] {
            Value::Array(parts) => parts
                .iter()
                .filter(|p| p["type"].as_str().is_none_or(|t| t == "text" || t == "reasoning" || t == "thinking"))
                .filter_map(|p| p["text"].as_str())
                .collect(),
            Value::String(s) => s.clone(),
            _ => v["text"].as_str().unwrap_or_default().to_string(),
        };
        let id = match v["id"].as_str().filter(|i| !i.is_empty()) {
            Some(id) => format!("{kind}:{id}"),
            None => format!("{kind}:#{}", self.said.len()),
        };
        let said = self.said.entry(id).or_default();
        let new = text.strip_prefix(said.as_str())?.to_string();
        if new.is_empty() {
            return None;
        }
        *said = text;
        Some(new)
    }

    fn effect(&mut self, v: &Value) -> Vec<StreamEvent> {
        let id = match ["toolCallId", "id"].iter().find_map(|k| v[*k].as_str().filter(|i| !i.is_empty())) {
            Some(id) => id.to_string(),
            None => format!("vibe-{}", self.seen.len() + 1),
        };
        let name = effect_name(v);
        let mut events = Vec::new();
        if self.seen.insert(id.clone()) {
            let args = v["detail"].get("input").cloned().unwrap_or(Value::Null);
            self.running.insert(id.clone());
            events.push(StreamEvent::ToolStart { id: id.clone(), name: name.clone(), args });
        }
        let state = &v["state"];
        let status = state["status"].as_str().unwrap_or_default();
        if matches!(status, "completed" | "failed" | "cancelled") && self.running.remove(&id) {
            let ok = status == "completed";
            let error = (!ok).then(|| {
                [&state["error"]["message"], &state["reason"], &state["outputText"]]
                    .into_iter()
                    .find_map(|r| r.as_str().map(str::trim).filter(|r| !r.is_empty()))
                    .unwrap_or(status)
                    .to_string()
            });
            if state["error"]["code"].as_str() == Some("tool_denied") {
                events.push(StreamEvent::Refused(name));
            }
            events.push(StreamEvent::ToolEnd { id, ok, error });
        }
        events
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line(v: Value) -> String {
        format!("{v}\n")
    }

    #[test]
    fn config_turns_every_network_feature_off_and_asks_for_everything_else() {
        let config = config();
        for key in [
            "enable_telemetry",
            "enable_update_checks",
            "enable_auto_update",
            "enable_notifications",
            "enable_connectors",
            "enable_otel",
            "enable_pr_monitoring",
            "enable_background_processes",
            "enable_subagents",
            "experimental_enable_registry_skills",
            "experimental_enable_model_catalog",
        ] {
            assert_eq!(config[key].as_bool(), Some(false), "{key}");
        }
        assert_eq!(config["experiments"]["enable"].as_bool(), Some(false));
        assert_eq!(config["session_logging"]["enabled"].as_bool(), Some(false));
        assert_eq!(config["session_logging"]["generate_titles"].as_bool(), Some(false));
        assert_eq!(config["default_approval_policy"].as_str(), Some("ask"));
        assert_eq!(config["outside_workdir"].as_str(), Some("deny"));
        assert_eq!(config["enabled_skills"].as_array().unwrap(), &vec![toml::Value::from("kwnt-none")]);
        // Nothing that would load the user's servers, or trust and approve everything
        for key in ["mcp_servers", "enable_auto_approve", "bypass_tool_permissions", "providers"] {
            assert!(config.get(key).is_none(), "{key}");
        }
    }

    #[test]
    fn args_never_trust_approve_or_resume_and_always_set_a_time_limit() {
        let base = tempfile::tempdir().unwrap();
        let home = VibeHome::for_thread(base.path(), "k1", "t1").unwrap();
        let turn = home.turn().unwrap();
        let args = args(&turn, Some("mistral-medium-latest"));
        for bad in ["--trust", "--auto-approve", "--yolo", "--resume", "--continue", "-p"] {
            assert!(!args.iter().any(|a| a == bad), "{bad}: {args:?}");
        }
        let pair = |k: &str| args.iter().position(|a| a == k).map(|i| args[i + 1].as_str());
        assert_eq!(pair("--time-limit"), Some("3600"));
        assert_eq!(pair("--output"), Some("streaming"));
        assert_eq!(pair("--approval-policy"), Some("ask"));
        assert_eq!(pair("--model"), Some("mistral-medium-latest"));
        assert_eq!(pair("--prompt-file"), Some(turn.prompt().to_str().unwrap()));
        // A model that isn't a plain name, and "default", aren't passed
        for model in ["default", "--trust", "a b", "x;rm", ""] {
            assert!(!super::args(&turn, Some(model)).iter().any(|a| a == "--model"), "{model}");
        }
    }

    #[test]
    fn tool_permissions_refuse_everything_but_folder_files_and_the_gateway() {
        let gateway = vec!["github__create_issue".to_string(), "bad name".to_string()];
        let shell = Path::new("/h/kwnt-shell.sh");
        let folder = tool_permissions(ToolMode::Folder, Some(shell), &gateway);
        for tool in CORE_TOOLS {
            let want = if FOLDER_TOOLS.contains(&tool) { "always" } else { "never" };
            assert_eq!(folder[tool]["permission"].as_str(), Some(want), "{tool}");
        }
        for tool in FOLDER_TOOLS {
            assert_eq!(folder[tool]["outside_workdir"].as_str(), Some("deny"), "{tool}");
        }
        assert_eq!(folder["bash"]["permission"].as_str(), Some("never"));
        assert_eq!(folder["bash"]["shell"].as_str(), Some("/h/kwnt-shell.sh"));
        assert_eq!(folder["web_fetch"]["permission"].as_str(), Some("never"));
        assert_eq!(folder["kwnt_github__create_issue"]["permission"].as_str(), Some("always"));
        assert!(folder.get("kwnt_bad name").is_none());

        for mode in [ToolMode::None, ToolMode::NoFolder] {
            let tools = tool_permissions(mode, None, &gateway);
            for tool in CORE_TOOLS {
                assert_eq!(tools[tool]["permission"].as_str(), Some("never"), "{mode:?} {tool}");
            }
        }
        assert!(tool_permissions(ToolMode::None, None, &gateway).get("kwnt_github__create_issue").is_none());
        assert_eq!(enabled_tools(&gateway), json!(["kwnt_github__create_issue"]));
        assert_eq!(enabled_tools(&[]), json!(["kwnt-none"]));
    }

    #[test]
    fn the_gateway_is_the_only_server_and_its_token_stays_in_env() {
        let servers = mcp_servers(Some("http://127.0.0.1:7/api/mcp/gateway/vibe"), 400);
        let list = servers.as_array().unwrap();
        assert_eq!(list.len(), 1);
        let kwnt = &list[0];
        assert_eq!(kwnt["name"], "kwnt");
        assert_eq!(kwnt["transport"], "streamable-http");
        assert_eq!(kwnt["auth"]["type"], "static");
        assert_eq!(kwnt["auth"]["api_key_env"], "KWNT_MCP_GATEWAY_TOKEN");
        assert_eq!(kwnt["startup_timeout_sec"], 415);
        assert_eq!(kwnt["tool_timeout_sec"], 400);
        let short = mcp_servers(Some("http://x"), 0);
        assert_eq!(short[0]["startup_timeout_sec"], 30);
        assert_eq!(short[0]["tool_timeout_sec"], 330);
        assert_eq!(mcp_servers(None, 0), json!([]));
    }

    #[test]
    fn a_model_name_is_never_a_flag() {
        for ok in ["mistral-medium-latest", "mistral-medium-3.5", "devstral_2", "1b"] {
            assert!(valid_model(ok), "{ok}");
        }
        let long = "m".repeat(65);
        for bad in ["--trust", "-y", "--model", "-", ".hidden", "_x", "", "default", "a b", "x;rm", "a\"b", long.as_str()] {
            assert!(!valid_model(bad), "{bad}");
        }
    }

    #[test]
    fn models_are_defined_only_when_vibe_lacks_them() {
        // Any Mistral id Vibe's list lacks is defined, so a pin like ministral-3b runs as is
        assert_eq!(
            models(Some("ministral-3b-latest")),
            Some(json!([{"name": "ministral-3b-latest", "provider": "mistral", "alias": "ministral-3b-latest"}]))
        );
        assert_eq!(
            models(Some("mistral-medium-latest")),
            Some(json!([{"name": "mistral-medium-latest", "provider": "mistral", "alias": "mistral-medium-latest"}]))
        );
        assert_eq!(models(Some("mistral-medium-3.5")), None);
        assert_eq!(models(Some("default")), None);
        assert_eq!(models(Some("a\"b")), None);
        assert_eq!(models(None), None);
    }

    #[test]
    fn a_thread_keeps_its_private_home_and_a_turn_cleans_up() {
        let base = tempfile::tempdir().unwrap();
        let home = VibeHome::for_thread(base.path(), "k1", "t1").unwrap();
        let again = VibeHome::for_thread(base.path(), "k1", "t1").unwrap();
        assert_eq!(home.path, again.path);
        assert_ne!(home.path, VibeHome::for_thread(base.path(), "k1", "t2").unwrap().path);
        assert_ne!(home.path, VibeHome::for_thread(base.path(), "k2", "t1").unwrap().path);
        let name = home.path.file_name().unwrap().to_str().unwrap().to_owned();
        assert!(name.starts_with("kwnt-vibe-") && !name.contains("t1"), "{name}");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = |p: &Path| std::fs::metadata(p).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode(&home.path), 0o700);
            assert_eq!(mode(&home.path.join("config.toml")), 0o600);
            assert_eq!(mode(&home.scrub_shell().unwrap()), 0o700);
        }
        let text = std::fs::read_to_string(home.path.join("config.toml")).unwrap();
        assert!(text.contains("enable_telemetry = false"), "{text}");

        let turn = home.turn().unwrap();
        turn.write_prompts("hi", "sys").unwrap();
        let dir = turn.path.clone();
        assert!(dir.join("prompt.md").is_file());
        drop(turn);
        assert!(!dir.exists());
        drop((home, again));
        // A thread's home stays for its next turn
        assert!(base.path().join(name).is_dir());

        // A turn with no thread gets its own home, removed with it
        let lone = VibeHome::for_thread(base.path(), "k1", "").unwrap();
        let lone_path = lone.path.clone();
        assert!(lone_path.is_dir());
        drop(lone);
        assert!(!lone_path.exists());
    }

    #[cfg(unix)]
    #[test]
    fn a_planted_link_is_never_used_as_a_home() {
        let base = tempfile::tempdir().unwrap();
        let elsewhere = tempfile::tempdir().unwrap();
        std::os::unix::fs::symlink(elsewhere.path(), base.path().join(home_name("k1", "t1"))).unwrap();
        assert!(VibeHome::for_thread(base.path(), "k1", "t1").is_err());
        assert!(!elsewhere.path().join("config.toml").exists());
    }

    #[test]
    fn stale_homes_are_swept_and_the_one_in_use_is_kept() {
        let base = tempfile::tempdir().unwrap();
        let old = base.path().join("kwnt-vibe-old");
        let other = base.path().join("other-old");
        std::fs::create_dir(&old).unwrap();
        std::fs::create_dir(&other).unwrap();
        let long_ago = std::time::SystemTime::now() - Duration::from_secs(25 * 60 * 60);
        for dir in [&old, &other] {
            std::fs::File::open(dir).unwrap().set_modified(long_ago).unwrap();
        }
        let home = VibeHome::for_thread(base.path(), "k", "t").unwrap();
        assert!(!old.exists());
        assert!(other.exists(), "only kwnt's own folders are swept");
        assert!(home.path.exists());
    }

    #[cfg(unix)]
    #[test]
    fn the_scrub_shell_hides_the_key_and_token_from_commands() {
        let base = tempfile::tempdir().unwrap();
        let home = VibeHome::for_thread(base.path(), "k", "t").unwrap();
        let shell = home.scrub_shell().unwrap();
        let out = std::process::Command::new(&shell)
            .args(["-c", "env"])
            .env("MISTRAL_API_KEY", "sk-vibe-secret-123")
            .env(crate::mcp::GATEWAY_TOKEN_ENV, "gw-secret-456")
            .env("KWNT_SOME_SETTING", "x")
            .env("HOME", home.home_dir())
            .output()
            .unwrap();
        assert!(out.status.success());
        let env = String::from_utf8_lossy(&out.stdout);
        assert!(!env.contains("sk-vibe-secret-123"), "{env}");
        assert!(!env.contains("gw-secret-456"), "{env}");
        assert!(!env.contains("KWNT_SOME_SETTING"), "{env}");
        assert!(env.contains("PATH="), "{env}");
        assert!(env.contains("TERM=dumb"), "{env}");
    }

    #[test]
    fn only_harmless_env_is_kept() {
        for name in ["PATH", "LANG", "LC_ALL", "LC_CTYPE", "HTTPS_PROXY", "SYSTEMROOT"] {
            assert!(kept_env(name), "{name}");
        }
        for name in [
            "MISTRAL_API_KEY",
            "KWNT_MCP_GATEWAY_TOKEN",
            "OPENAI_API_KEY",
            "ANTHROPIC_API_KEY",
            "VIBE_HOME",
            "VIBE_TOOLS",
            "HOME",
            "GITHUB_TOKEN",
            "AWS_SECRET_ACCESS_KEY",
            "LD_PRELOAD",
            "PYTHONPATH",
        ] {
            assert!(!kept_env(name), "{name}");
        }
    }

    #[test]
    fn the_key_is_read_from_vibes_dotenv() {
        assert_eq!(key_from_dotenv("MISTRAL_API_KEY=abcdefgh123\n").as_deref(), Some("abcdefgh123"));
        assert_eq!(key_from_dotenv("# c\nexport MISTRAL_API_KEY=\"abcdefgh123\"").as_deref(), Some("abcdefgh123"));
        assert_eq!(key_from_dotenv("MISTRAL_API_KEY='abcdefgh123'").as_deref(), Some("abcdefgh123"));
        assert_eq!(key_from_dotenv("OTHER=abcdefgh123"), None);
        assert_eq!(key_from_dotenv("MISTRAL_API_KEY=short"), None);
        assert_eq!(key_from_dotenv("MISTRAL_API_KEY=has space inside"), None);
        assert!(valid_key("abcdefgh"));
        assert!(!valid_key("abc\ndefgh"));
    }

    #[test]
    fn export_gives_usage_cost_and_the_error() {
        let ok = Export::parse(r#"{"outcome":"finished","stop_reason":null,"error":null,"usage":{"input_tokens":111,"output_tokens":22,"cached_input_tokens":11,"total_tokens":133},"cost_usd":0.0021,"exit_code":0,"config":{"active_model":"mistral-medium-3.5"}}"#).unwrap();
        assert_eq!(ok.outcome.as_deref(), Some("finished"));
        assert_eq!(
            ok.usage,
            Some(crate::drivers::TurnUsage { fresh_input: 100, cache_read: 11, cache_write: 0, output: 22, cost_usd: Some(0.0021) })
        );
        assert_eq!(ok.model.as_deref(), Some("mistral-medium-3.5"));
        let failed = Export::parse(r#"{"outcome":"config_error","error":{"message":"Invalid API key","code":"invalid_api_key"},"usage":{"input_tokens":0,"output_tokens":0,"cached_input_tokens":0},"cost_usd":0.0}"#).unwrap();
        assert_eq!(failed.error_code.as_deref(), Some("invalid_api_key"));
        assert_eq!(failed.usage, None, "nothing used, nothing recorded");
        assert!(Export::parse("not json").is_none());
    }

    #[test]
    fn limits_and_bad_keys_move_the_turn_on() {
        use crate::profiles::FailClass;
        let export = |code: &str, message: &str| Export {
            error_code: Some(code.into()).filter(|c: &String| !c.is_empty()),
            error_message: Some(message.into()),
            ..Default::default()
        };
        for code in ["rate_limit", "model_billing_denied", "usage_limit"] {
            let f = failover(Some(2), Some(&export(code, "whatever")), "").unwrap();
            assert_eq!(f.class, FailClass::Quota, "{code}");
        }
        // An unknown body on a 429 or 402 still counts as a limit
        for message in ["Client error '429 Too Many Requests': ???", "HTTP 402: something new"] {
            let f = failover(Some(2), Some(&export("", message)), "").unwrap();
            assert_eq!(f.class, FailClass::Quota, "{message}");
        }
        let f = failover(Some(2), None, "Error: Client error '429 Too Many Requests' for url").unwrap();
        assert_eq!(f.class, FailClass::Quota);
        let f = failover(Some(1), Some(&export("invalid_api_key", "Invalid API key (from env var MISTRAL_API_KEY)")), "").unwrap();
        assert_eq!(f.class, FailClass::Auth);
        let f = failover(Some(1), Some(&export("", "Missing MISTRAL_API_KEY environment variable for mistral provider")), "").unwrap();
        assert_eq!(f.class, FailClass::Auth);
        // A server error, a deadline and a clean finish don't
        assert!(failover(Some(2), Some(&export("backend_error", "Server error '500 Internal Server Error'")), "").is_none());
        assert!(failover(Some(3), Some(&Export { outcome: Some("deadline".into()), ..Default::default() }), "").is_none());
        assert!(failover(Some(0), None, "").is_none());
    }

    /// Rigel's live check: a pin Vibe can't take must refuse the turn, never run Vibe's default.
    #[test]
    fn a_model_vibe_cant_take_is_refused_in_plain_words() {
        for good in ["ministral-3b-latest", "mistral-medium-3.5", "codestral-latest"] {
            assert_eq!(bad_model_notice(good), None, "{good}");
        }
        for bad in ["mistral/ministral-3b-latest", "a b", "--trust", "default", ""] {
            let n = bad_model_notice(bad).unwrap_or_else(|| panic!("{bad}"));
            assert!(n.starts_with("Mistral Vibe can't run this bot's pinned model"), "{n}");
            assert!(n.contains("mistral-medium-3.5") && n.contains("mistral-large-4-0") && n.contains("ministral-8b-latest"), "{n}");
        }
    }

    /// Rigel's live check: one 429 on a free key held the engine for an hour. A 429 is a rate
    /// limit: its Retry-After, else a short hold; only a spent quota or billing gets the hour.
    #[test]
    fn a_rate_limit_holds_the_account_briefly_and_a_spent_quota_for_long() {
        use crate::profiles::FailClass;
        let export = |code: &str, message: &str| Export {
            error_code: Some(code.into()).filter(|c: &String| !c.is_empty()),
            error_message: Some(message.into()),
            ..Default::default()
        };
        let held_for = |f: &crate::profiles::ProviderFailure| {
            assert_eq!(f.class, FailClass::Quota, "a limit still moves the turn on");
            (f.retry_at.expect("a limit always says until when") - chrono::Utc::now()).num_seconds()
        };
        let short = RATE_LIMIT_HOLD_SECS;
        let long = crate::profiles::classify::DEFAULT_QUOTA_COOLDOWN_MINS * 60;
        // A 429 without Retry-After: the short hold, from the export code or the text alone
        let f = failover(Some(2), Some(&export("rate_limit", "Requests rate limit exceeded")), "").unwrap();
        assert!((short - 5..=short).contains(&held_for(&f)), "{f:?}");
        let f = failover(Some(2), None, "Error: Client error '429 Too Many Requests' for url").unwrap();
        assert!((short - 5..=short).contains(&held_for(&f)), "{f:?}");
        // A 429 with Retry-After: exactly that long
        for text in ["429 Too Many Requests; retry-after: 42", "{\"retry_after\": 42}", "Retry-After 42"] {
            let f = failover(Some(2), Some(&export("rate_limit", text)), "").unwrap();
            assert!((37..=42).contains(&held_for(&f)), "{text}: {f:?}");
        }
        // A silly Retry-After is capped at a day, and 0 still holds a moment
        let f = failover(Some(2), Some(&export("rate_limit", "retry-after: 99999999")), "").unwrap();
        assert!(held_for(&f) <= 24 * 60 * 60, "{f:?}");
        let f = failover(Some(2), Some(&export("rate_limit", "retry-after: 0")), "").unwrap();
        assert!(f.retry_at.unwrap() > chrono::Utc::now() - chrono::Duration::seconds(1));
        // A 429 whose body says the quota is spent: the long hold
        let f = failover(Some(2), Some(&export("rate_limit", "You exceeded your current quota")), "").unwrap();
        assert!((long - 5..=long).contains(&held_for(&f)), "{f:?}");
        // A 402 or a billing code: the long hold, also with an unknown body
        for (code, message) in [("model_billing_denied", "You have exceeded your plan usage"), ("usage_limit", "whatever"), ("", "HTTP 402: something new")] {
            let f = failover(Some(2), Some(&export(code, message)), "").unwrap();
            assert!((long - 5..=long).contains(&held_for(&f)), "{code} {message}: {f:?}");
        }
        assert_eq!(retry_after_secs("no hint here"), None);
        assert_eq!(retry_after_secs("retry-after: soon"), None);
        assert_eq!(retry_after_secs("Retry-After: 7"), Some(7));
    }

    #[test]
    fn notices_say_why_vibe_gave_no_reply() {
        let secrets = vec!["sk-secret".to_string()];
        let refused = vec!["file_system.bash".to_string()];
        let n = empty_reply_notice(&EmptyRun { refused: &refused, exit_code: Some(0), ..Default::default() }, &secrets);
        assert!(n.contains("“file_system.bash”") && n.contains("doesn't allow"), "{n}");
        let missing = Export { error_message: Some("Missing MISTRAL_API_KEY environment variable".into()), ..Default::default() };
        let n = empty_reply_notice(&EmptyRun { export: Some(&missing), exit_code: Some(1), ..Default::default() }, &secrets);
        assert!(n.contains("vibe --setup"), "{n}");
        let deadline = Export { outcome: Some("deadline".into()), ..Default::default() };
        let n = empty_reply_notice(&EmptyRun { export: Some(&deadline), exit_code: Some(3), ..Default::default() }, &secrets);
        assert!(n.contains("ran out of time (60 minutes)"), "{n}");
        let boom = Export { error_message: Some("Server error with sk-secret inside".into()), ..Default::default() };
        let n = empty_reply_notice(&EmptyRun { export: Some(&boom), exit_code: Some(2), ..Default::default() }, &secrets);
        assert!(n.starts_with("Vibe stopped with an error:") && !n.contains("sk-secret"), "{n}");
        let n = empty_reply_notice(&EmptyRun { stderr: "trace sk-secret", exit_code: Some(5), ..Default::default() }, &secrets);
        assert!(n.contains("code 5") && !n.contains("sk-secret"), "{n}");
    }

    #[test]
    fn parser_reads_replies_thinking_and_settled_tools() {
        let mut p = StreamParser::default();
        let user = line(json!({"type":"message","id":"u1","role":"user","content":[{"type":"text","text":"hi"}]}));
        assert!(p.push(&user).is_empty(), "the user's own message isn't the reply");
        let think = line(json!({"type":"reasoning","id":"r1","content":[{"type":"text","text":"Let me look."}]}));
        assert_eq!(p.push(&think), vec![StreamEvent::Thought("Let me look.".into())]);
        let denied = line(json!({
            "type":"effect","title":"file_system.bash","toolCallId":"abc",
            "detail":{"toolName":"file_system.bash","kind":"shell","input":{"command":"ls"}},
            "state":{"status":"failed","error":{"message":"Tool execution denied by approval policy","code":"tool_denied"}}
        }));
        assert_eq!(
            p.push(&denied),
            vec![
                StreamEvent::ToolStart { id: "abc".into(), name: "file_system.bash".into(), args: json!({"command":"ls"}) },
                StreamEvent::Refused("file_system.bash".into()),
                StreamEvent::ToolEnd { id: "abc".into(), ok: false, error: Some("Tool execution denied by approval policy".into()) },
            ]
        );
        assert!(p.push(&denied).is_empty(), "a repeated effect isn't run again");
        // A message printed again as it grows adds only its new text; split across chunks
        let first = line(json!({"type":"message","id":"m1","role":"assistant","content":[{"type":"text","text":"The "}]}));
        let full = line(json!({"type":"message","id":"m1","role":"assistant","content":[{"type":"text","text":"The card."}],"outcome":{"type":"committed"}}));
        assert_eq!(p.push(&first), vec![StreamEvent::Text("The ".into())]);
        let (a, b) = full.split_at(20);
        assert!(p.push(a).is_empty());
        assert_eq!(p.push(b), vec![StreamEvent::Text("card.".into())]);
        assert!(p.push(&full).is_empty());
        assert!(p.push("not json\n{\"type\":\"callback\"}\n").is_empty());
        let ran = json!({"type":"effect","title":"mcp_kwnt.github__x","toolCallId":"t2","state":{"status":"completed"}}).to_string();
        p.push(&ran);
        assert_eq!(p.finish().len(), 2);
        assert_eq!(bare_tool_name("mcp_kwnt.github__x"), "github__x");
        assert_eq!(bare_tool_name("run_typescript"), "run_typescript");
    }
}
