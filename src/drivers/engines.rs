//! Which engines can actually run on this host, and the models each one offers: every CLI's
//! own sign-in check, every API key against its provider's free model list, and Ollama's pulled
//! models. Checks run in the background (at startup and on Re-check) and are cached; turns only
//! read the cache, and a turn the provider refuses marks its engine at once.

use crate::profiles::{classify::redact_detail, Engine, FailClass, ProviderFailure};
use chrono::{DateTime, Utc};
use serde::Serialize;
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::process::Stdio;
use std::sync::RwLock;
use std::time::Duration;

/// Longest one engine's check may take, all its commands and requests included.
pub const PROBE_TIMEOUT: Duration = Duration::from_secs(10);

/// What a turn gets when no engine at all can run.
pub const NO_ENGINE_MESSAGE: &str = "No engine is available — check Settings → Engines.";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum EngineId {
    Claude,
    Codex,
    Grok,
    AnthropicApi,
    OpenaiApi,
    XaiApi,
    Ollama,
    /// The space's Custom API accounts (OpenAI-compatible or Anthropic-compatible servers).
    CustomApi,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum EngineKind {
    Cli,
    Api,
    Local,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum EngineStatus {
    Ready,
    SignedOut,
    NotInstalled,
    Error,
    /// Not checked yet (startup).
    Checking,
    /// Turned off in Settings: never checked, routed to or offered to Laya.
    Disabled,
}

/// How much model a turn needs. Laya's effort picks it: low → fast, medium → balanced,
/// high → deep.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Tier {
    Fast,
    Balanced,
    Deep,
}

impl Tier {
    pub fn from_effort(effort: &str) -> Self {
        match effort {
            "low" => Tier::Fast,
            "high" => Tier::Deep,
            _ => Tier::Balanced,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Tier::Fast => "fast",
            Tier::Balanced => "balanced",
            Tier::Deep => "deep",
        }
    }
}

/// A model's tier from the words in its id: "claude-haiku-4-5" and "o4-mini" are fast,
/// "sonnet" and "gpt-6.1-sol" balanced, "opus", "fable" and "grok-4-heavy" deep. `None` when the
/// name doesn't say ("grok-4.6", "gpt-5.5").
pub fn tier_from_name(id: &str) -> Option<Tier> {
    let lower = id.to_lowercase();
    let words: Vec<&str> = lower.split(|c: char| !c.is_ascii_alphanumeric()).filter(|w| !w.is_empty()).collect();
    let has = |names: &[&str]| words.iter().any(|w| names.contains(w));
    if has(&["haiku", "mini", "nano", "luna", "fast", "flash", "lite"]) {
        Some(Tier::Fast)
    } else if has(&["opus", "fable", "astra", "heavy", "pro"]) {
        Some(Tier::Deep)
    } else if has(&["sonnet", "sol", "terra"]) {
        Some(Tier::Balanced)
    } else {
        None
    }
}

/// A codex model's tier from codex's own description ("Fast and affordable model for easier
/// tasks", "Frontier intelligence…", "…workhorse model"), else from its slug.
pub fn codex_tier(slug: &str, description: Option<&str>) -> Option<Tier> {
    let d = description.unwrap_or_default().to_lowercase();
    let says = |words: &[&str]| words.iter().any(|w| d.contains(w));
    if says(&["fast", "affordable", "efficient", "lighter", "easier"]) {
        Some(Tier::Fast)
    } else if says(&["frontier", "most demanding", "most capable"]) {
        Some(Tier::Deep)
    } else if says(&["workhorse", "balanced", "everyday"]) {
        Some(Tier::Balanced)
    } else {
        tier_from_name(slug)
    }
}

impl EngineId {
    /// Every engine, in the order the API lists them.
    pub const ALL: [EngineId; 8] = [
        EngineId::Claude,
        EngineId::Codex,
        EngineId::Grok,
        EngineId::AnthropicApi,
        EngineId::OpenaiApi,
        EngineId::XaiApi,
        EngineId::Ollama,
        EngineId::CustomApi,
    ];

    pub fn name(self) -> &'static str {
        match self {
            EngineId::Claude => "Claude Code CLI",
            EngineId::Codex => "OpenAI Codex CLI",
            EngineId::Grok => "Grok CLI",
            EngineId::AnthropicApi => "Anthropic API",
            EngineId::OpenaiApi => "OpenAI API",
            EngineId::XaiApi => "xAI API",
            EngineId::Ollama => "Ollama",
            EngineId::CustomApi => "Custom API",
        }
    }

    pub fn kind(self) -> EngineKind {
        match self {
            EngineId::Claude | EngineId::Codex | EngineId::Grok => EngineKind::Cli,
            EngineId::AnthropicApi | EngineId::OpenaiApi | EngineId::XaiApi | EngineId::CustomApi => {
                EngineKind::Api
            }
            EngineId::Ollama => EngineKind::Local,
        }
    }

    /// The CLI and the API that run a model family.
    pub fn as_str(self) -> &'static str {
        match self {
            EngineId::Claude => "claude",
            EngineId::Codex => "codex",
            EngineId::Grok => "grok",
            EngineId::AnthropicApi => "anthropic_api",
            EngineId::OpenaiApi => "openai_api",
            EngineId::XaiApi => "xai_api",
            EngineId::Ollama => "ollama",
            EngineId::CustomApi => "custom_api",
        }
    }

    pub fn parse(id: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|e| e.as_str() == id)
    }

    /// The model family it runs (`None` for Ollama).
    pub fn family(self) -> Option<Engine> {
        match self {
            EngineId::Claude | EngineId::AnthropicApi => Some(Engine::Claude),
            EngineId::Codex | EngineId::OpenaiApi => Some(Engine::Codex),
            EngineId::Grok | EngineId::XaiApi => Some(Engine::Grok),
            EngineId::CustomApi => Some(Engine::Custom),
            EngineId::Ollama => None,
        }
    }

    /// For Custom API both are [`EngineId::CustomApi`]: it has no CLI.
    pub fn of_family(engine: Engine) -> (EngineId, EngineId) {
        match engine {
            Engine::Claude => (EngineId::Claude, EngineId::AnthropicApi),
            Engine::Codex => (EngineId::Codex, EngineId::OpenaiApi),
            Engine::Grok => (EngineId::Grok, EngineId::XaiApi),
            Engine::Custom => (EngineId::CustomApi, EngineId::CustomApi),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct EngineModel {
    pub id: String,
    pub label: String,
    pub default: bool,
    pub tier: Option<Tier>,
}

impl EngineModel {
    fn new(id: impl Into<String>, label: impl Into<String>, default: bool, tier: Option<Tier>) -> Self {
        Self { id: id.into(), label: label.into(), default, tier }
    }
}

/// One engine as `GET /api/engines` shows it.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct EngineReport {
    pub id: EngineId,
    pub name: String,
    pub kind: EngineKind,
    /// Off in Settings ⇒ status is `disabled`.
    pub enabled: bool,
    pub status: EngineStatus,
    pub detail: Option<String>,
    pub version: Option<String>,
    pub models: Vec<EngineModel>,
    pub checked_at: Option<DateTime<Utc>>,
    /// On, and its own login (`status` ready) or one of its accounts can take a turn: set
    /// from the profiles by `profile_turn::with_accounts`.
    pub ready: bool,
    /// The account a kwant on this engine runs on; `None` when not ready.
    pub uses: Option<EngineAccount>,
}

/// An account (profile) an engine runs on, as `GET /api/engines` names it.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct EngineAccount {
    pub profile_id: String,
    pub label: String,
}

impl EngineReport {
    fn checking(id: EngineId) -> Self {
        Self::from_probe(id, Probe::new(EngineStatus::Checking, None), None)
    }

    fn from_probe(id: EngineId, p: Probe, checked_at: Option<DateTime<Utc>>) -> Self {
        Self {
            id,
            name: id.name().into(),
            kind: id.kind(),
            enabled: true,
            status: p.status,
            detail: p.detail,
            version: p.version,
            models: if p.status == EngineStatus::Ready { p.models } else { Vec::new() },
            checked_at,
            ready: false,
            uses: None,
        }
    }

    /// The model a turn uses when none is picked.
    pub fn default_model(&self) -> Option<&EngineModel> {
        self.models.iter().find(|m| m.default).or(self.models.first())
    }
}

/// The result of one check, before it gets a time stamp.
#[derive(Debug, Clone, PartialEq)]
pub struct Probe {
    pub status: EngineStatus,
    pub detail: Option<String>,
    pub version: Option<String>,
    pub models: Vec<EngineModel>,
}

impl Probe {
    fn new(status: EngineStatus, detail: Option<String>) -> Self {
        Self { status, detail, version: None, models: Vec::new() }
    }
    fn ready(detail: impl Into<String>, models: Vec<EngineModel>) -> Self {
        Self { models, ..Self::new(EngineStatus::Ready, Some(detail.into())) }
    }
    fn signed_out(detail: impl Into<String>) -> Self {
        Self::new(EngineStatus::SignedOut, Some(detail.into()))
    }
    fn not_installed(detail: impl Into<String>) -> Self {
        Self::new(EngineStatus::NotInstalled, Some(detail.into()))
    }
    fn error(detail: impl Into<String>) -> Self {
        Self::new(EngineStatus::Error, Some(detail.into()))
    }
    fn with_version(mut self, version: Option<String>) -> Self {
        self.version = version;
        self
    }
}

/// Everything the checks need, read once from Settings and the environment.
#[derive(Debug, Clone, Default)]
pub struct ProbeEnv {
    pub claude_bin: Option<String>,
    /// Set only when kwnt points Claude Code at a config dir of its own (tests, isolated
    /// sign-in); otherwise the CLI uses its env's `CLAUDE_CONFIG_DIR` or `~/.claude`.
    pub claude_config_dir: Option<std::path::PathBuf>,
    pub codex_bin: Option<String>,
    pub grok_bin: Option<String>,
    pub anthropic_key: Option<String>,
    pub openai_key: Option<String>,
    pub xai_key: Option<String>,
    /// Ollama server root (no `/v1`).
    pub ollama_root: String,
    pub ollama_installed: bool,
    pub anthropic_base: String,
    pub openai_base: String,
    pub xai_base: String,
}

impl ProbeEnv {
    /// This host: CLIs on PATH, keys from Settings or else the env (as turns use them).
    pub async fn from_settings(db: &crate::db::Database) -> Self {
        async fn key(db: &crate::db::Database, setting: &str, var: &str) -> Option<String> {
            db.get_setting(setting)
                .await
                .ok()
                .flatten()
                .or_else(|| std::env::var(var).ok())
                .filter(|k| !k.trim().is_empty())
        }
        let ollama_url = db.get_setting("ollama_url").await.ok().flatten();
        Self {
            claude_bin: super::find_cli_binary("claude"),
            claude_config_dir: None,
            codex_bin: super::find_cli_binary("codex"),
            grok_bin: super::find_cli_binary("grok"),
            anthropic_key: key(db, "anthropic_api_key", "ANTHROPIC_API_KEY").await,
            openai_key: key(db, "openai_api_key", "OPENAI_API_KEY").await,
            xai_key: key(db, "xai_api_key", "XAI_API_KEY").await,
            ollama_root: super::ollama_root(ollama_url.as_deref()),
            ollama_installed: super::find_cli_binary("ollama").is_some(),
            anthropic_base: "https://api.anthropic.com".into(),
            openai_base: "https://api.openai.com".into(),
            xai_base: "https://api.x.ai".into(),
        }
    }
}

/// Checks one engine, never taking longer than [`PROBE_TIMEOUT`].
pub async fn probe(id: EngineId, env: &ProbeEnv) -> Probe {
    probe_within(id, env, PROBE_TIMEOUT).await
}

pub async fn probe_within(id: EngineId, env: &ProbeEnv, limit: Duration) -> Probe {
    let check = async {
        match id {
            EngineId::Claude => probe_claude(env.claude_bin.as_deref(), env.claude_config_dir.as_deref()).await,
            EngineId::Codex => probe_codex(env.codex_bin.as_deref()).await,
            EngineId::Grok => probe_grok(env.grok_bin.as_deref()).await,
            EngineId::AnthropicApi | EngineId::OpenaiApi | EngineId::XaiApi => probe_api(id, env).await,
            EngineId::Ollama => probe_ollama(env).await,
            // Nothing on the host to check: each account is checked on its own (see
            // `drivers::custom`), and a space's report is ready through its accounts
            EngineId::CustomApi => Probe::new(EngineStatus::NotInstalled, Some(CUSTOM_API_DETAIL.into())),
        }
    };
    // Dropping the check kills its commands (kill_on_drop) and aborts its requests.
    tokio::time::timeout(limit, check)
        .await
        .unwrap_or_else(|_| Probe::error(format!("Check timed out after {}s", limit.as_secs())))
}

struct CmdOut {
    success: bool,
    stdout: String,
    stderr: String,
}

async fn run(bin: &str, args: &[&str]) -> Result<CmdOut, String> {
    run_in(bin, args, None).await
}

/// [`run`], with Claude Code pointed at `claude_home` when one is given.
async fn run_in(bin: &str, args: &[&str], claude_home: Option<&std::path::Path>) -> Result<CmdOut, String> {
    let mut cmd = match claude_home {
        Some(home) => crate::profiles::CliEnv::for_home(crate::profiles::Engine::Claude, home).command(bin),
        None => tokio::process::Command::new(bin),
    };
    let out = cmd
        .args(args)
        .env("NO_COLOR", "1")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .output()
        .await
        .map_err(|e| format!("Couldn't run {bin}: {e}"))?;
    Ok(CmdOut {
        success: out.status.success(),
        stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
    })
}

/// The version number in `--version` output: "2.1.296 (Claude Code)", "codex-cli 0.162.1",
/// "grok 1.0.62 (1474f9cb)".
pub fn parse_version(text: &str) -> Option<String> {
    let line = text.lines().map(str::trim).find(|l| !l.is_empty())?;
    line.split_whitespace()
        .map(|w| w.trim_start_matches('v'))
        .find(|w| w.chars().next().is_some_and(|c| c.is_ascii_digit()) && w.contains('.'))
        .map(|w| w.chars().take(40).collect())
}

async fn version_of(bin: &str) -> Option<String> {
    run(bin, &["--version"]).await.ok().and_then(|o| parse_version(&o.stdout))
}

/// First non-empty line of a failed command, redacted, for the reason shown.
fn failure_line(out: &CmdOut) -> String {
    let text = format!("{}\n{}", out.stderr, out.stdout);
    let line = text
        .lines()
        .map(|l| crate::profiles::login::strip_ansi(l).trim().to_string())
        .find(|l| !l.is_empty())
        .unwrap_or_else(|| "no output".into());
    redact_detail(&line, 120)
}

/// Claude Code has no command that lists models; these aliases always point at the latest.
pub fn claude_cli_models() -> Vec<EngineModel> {
    [
        ("opus", "Claude Opus (latest)", true),
        ("sonnet", "Claude Sonnet (latest)", false),
        ("haiku", "Claude Haiku (latest)", false),
        ("fable", "Claude Fable (latest)", false),
    ]
    .into_iter()
    .map(|(id, label, default)| EngineModel::new(id, label, default, tier_from_name(id)))
    .collect()
}

async fn probe_claude(bin: Option<&str>, home: Option<&std::path::Path>) -> Probe {
    let Some(bin) = bin else {
        return Probe::not_installed("Claude Code isn't installed");
    };
    let version = version_of(bin).await;
    // `auth status` asks Claude Code itself, so a login kept in the macOS Keychain counts.
    let out = match run_in(bin, &["auth", "status", "--json"], home).await {
        Ok(o) => o,
        Err(e) => return Probe::error(e).with_version(version),
    };
    let parsed: Option<Value> = serde_json::from_str(out.stdout.trim()).ok();
    let probe = match parsed.as_ref().and_then(|v| v["loggedIn"].as_bool()) {
        Some(true) => {
            let v = parsed.as_ref().unwrap();
            let how = ["authMethod", "apiProvider"]
                .iter()
                .find_map(|k| v[*k].as_str().filter(|s| !s.is_empty()))
                .map(|s| format!("Signed in ({})", redact_detail(s, 40)))
                .unwrap_or_else(|| "Signed in".into());
            Probe::ready(how, claude_cli_models())
        }
        Some(false) => Probe::signed_out("Not signed in: run `claude auth login`"),
        None => Probe::error(format!("Couldn't read the sign-in status: {}", failure_line(&out))),
    };
    probe.with_version(version)
}

/// Models in `codex debug models` JSON that codex lists, best first.
pub fn parse_codex_models(json: &str) -> Vec<EngineModel> {
    let Ok(v) = serde_json::from_str::<Value>(json.trim()) else {
        return Vec::new();
    };
    let mut listed: Vec<(i64, String, String, Option<Tier>)> = v["models"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|m| m["visibility"].as_str().is_none_or(|s| s == "list"))
        .filter_map(|m| {
            let id = m["slug"].as_str()?.to_string();
            let label = m["display_name"].as_str().unwrap_or(&id).to_string();
            let tier = codex_tier(&id, m["description"].as_str());
            Some((m["priority"].as_i64().unwrap_or(i64::MAX), id, label, tier))
        })
        .collect();
    listed.sort_by(|a, b| (a.0, &a.1).cmp(&(b.0, &b.1)));
    listed
        .into_iter()
        .enumerate()
        .map(|(i, (_, id, label, tier))| EngineModel::new(id, label, i == 0, tier))
        .collect()
}

/// Used when a CLI's model list can't be read: run it without `--model`.
fn cli_default_model(engine: &str) -> Vec<EngineModel> {
    vec![EngineModel::new("default", format!("{engine} default"), true, None)]
}

async fn probe_codex(bin: Option<&str>) -> Probe {
    let Some(bin) = bin else {
        return Probe::not_installed("Codex isn't installed");
    };
    let version = version_of(bin).await;
    let out = match run(bin, &["login", "status"]).await {
        Ok(o) => o,
        Err(e) => return Probe::error(e).with_version(version),
    };
    let status = crate::profiles::status::codex_status(out.success, &out.stdout, &out.stderr);
    let probe = if status.signed_in == Some(true) {
        let models = match run(bin, &["debug", "models"]).await {
            Ok(o) if o.success => parse_codex_models(&o.stdout),
            _ => Vec::new(),
        };
        let models = if models.is_empty() { cli_default_model("Codex") } else { models };
        Probe::ready(status.account_label.unwrap_or_else(|| "Signed in".into()), models)
    } else if !out.success && !format!("{}{}", out.stdout, out.stderr).to_lowercase().contains("not logged in") {
        Probe::error(format!("Couldn't read the sign-in status: {}", failure_line(&out)))
    } else {
        Probe::signed_out("Not signed in: run `codex login`")
    };
    probe.with_version(version)
}

/// `grok models` output: whether it's signed in, and the models it lists.
pub fn parse_grok_models(text: &str) -> (bool, Vec<EngineModel>) {
    let lower = text.to_lowercase();
    let signed_out = ["not authenticated", "not logged in", "not signed in", "please log in", "grok login"]
        .iter()
        .any(|p| lower.contains(p));
    let default_line = text
        .lines()
        .find_map(|l| l.trim().strip_prefix("Default model:").map(|m| m.trim().to_string()));
    let mut models: Vec<EngineModel> = text
        .lines()
        .skip_while(|l| !l.trim().starts_with("Available models"))
        .skip(1)
        .filter_map(|l| {
            let l = l.trim();
            let rest = l.strip_prefix('*').or_else(|| l.strip_prefix('-'))?.trim();
            let id = rest.split_whitespace().next()?.to_string();
            let default = rest.contains("(default)") || default_line.as_deref() == Some(id.as_str());
            let tier = tier_from_name(&id);
            Some(EngineModel::new(id.clone(), id, default, tier))
        })
        .collect();
    if !models.is_empty() && !models.iter().any(|m| m.default) {
        models[0].default = true;
    }
    (!signed_out, models)
}

async fn probe_grok(bin: Option<&str>) -> Probe {
    let Some(bin) = bin else {
        return Probe::not_installed("Grok CLI isn't installed");
    };
    let version = version_of(bin).await;
    // grok has no status command; `grok models` says whether it's authenticated, for free.
    let out = match run(bin, &["models"]).await {
        Ok(o) => o,
        Err(e) => return Probe::error(e).with_version(version),
    };
    let (signed_in, models) = parse_grok_models(&format!("{}\n{}", out.stdout, out.stderr));
    let probe = if !signed_in {
        Probe::signed_out("Not signed in: run `grok login`")
    } else if !out.success {
        Probe::error(format!("Couldn't check: {}", failure_line(&out)))
    } else {
        let models = if models.is_empty() { cli_default_model("Grok") } else { models };
        Probe::ready("Signed in", models)
    };
    probe.with_version(version)
}

/// Chat models in a provider's `GET /v1/models` reply, newest first, default marked.
pub fn parse_api_models(id: EngineId, body: &Value) -> Vec<EngineModel> {
    let chat = |m: &str| match id {
        EngineId::AnthropicApi => m.starts_with("claude"),
        EngineId::XaiApi => m.contains("grok") && !m.contains("image") && !m.contains("imagine"),
        _ => {
            let family = m.starts_with("gpt-")
                || m.starts_with("chatgpt")
                || (m.starts_with('o') && m[1..].starts_with(|c: char| c.is_ascii_digit()));
            let other = ["audio", "realtime", "tts", "transcribe", "image", "search", "embedding", "moderation", "instruct"];
            family && !other.iter().any(|o| m.contains(o))
        }
    };
    let mut models: Vec<(i64, String, String)> = body["data"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|m| {
            let mid = m["id"].as_str()?;
            chat(mid).then(|| {
                let label = m["display_name"].as_str().unwrap_or(mid).to_string();
                let created = m["created"]
                    .as_i64()
                    .or_else(|| m["created_at"].as_str().and_then(|s| DateTime::parse_from_rfc3339(s).ok()).map(|t| t.timestamp()))
                    .unwrap_or(0);
                (created, mid.to_string(), label)
            })
        })
        .collect();
    // Newest first; the provider's own order breaks ties (Anthropic already lists newest first).
    models.sort_by_key(|m| std::cmp::Reverse(m.0));
    let preferred = (id == EngineId::AnthropicApi)
        .then_some(super::api::DEFAULT_CLAUDE_MODEL)
        .filter(|d| models.iter().any(|m| m.1 == *d));
    models
        .into_iter()
        .enumerate()
        .map(|(i, (_, mid, label))| {
            let default = preferred.map_or(i == 0, |d| d == mid);
            let tier = tier_from_name(&mid);
            EngineModel::new(mid, label, default, tier)
        })
        .collect()
}

async fn probe_api(id: EngineId, env: &ProbeEnv) -> Probe {
    let (key, base) = match id {
        EngineId::AnthropicApi => (&env.anthropic_key, &env.anthropic_base),
        EngineId::OpenaiApi => (&env.openai_key, &env.openai_base),
        _ => (&env.xai_key, &env.xai_base),
    };
    let Some(key) = key else {
        return Probe::not_installed("No API key: add one in Settings");
    };
    let client = match reqwest::Client::builder().timeout(PROBE_TIMEOUT).build() {
        Ok(c) => c,
        Err(e) => return Probe::error(format!("Couldn't start the check: {e}")),
    };
    // Listing models is free on every provider and needs a valid key.
    let mut req = client.get(format!("{}/v1/models", base.trim_end_matches('/')));
    req = if id == EngineId::AnthropicApi {
        req.header("x-api-key", key).header("anthropic-version", "2023-06-01")
    } else {
        req.bearer_auth(key)
    };
    let resp = match req.send().await {
        Ok(r) => r,
        Err(e) => {
            let why = if e.is_timeout() { "timed out" } else { "network error" };
            return Probe::error(format!("Couldn't reach {} ({why})", id.name()));
        }
    };
    let status = resp.status().as_u16();
    let text = resp.text().await.unwrap_or_default();
    if (200..300).contains(&status) {
        let body: Value = serde_json::from_str(&text).unwrap_or(Value::Null);
        return Probe::ready("API key accepted", parse_api_models(id, &body));
    }
    api_refusal(status, &text)
}

/// A refused key check as a status and reason; the key itself never appears in it.
pub fn api_refusal(status: u16, body: &str) -> Probe {
    match crate::profiles::classify(Some(status), body, Utc::now()) {
        Some((FailClass::Auth, _)) => Probe::signed_out(format!("API key rejected ({status})")),
        Some((FailClass::Quota, _)) => Probe::error(format!("Quota exceeded ({status})")),
        _ => Probe::error(format!("The provider answered HTTP {status}")),
    }
}

/// Model names in Ollama's `/api/tags` reply; `OLLAMA_MODEL` (or else the first) is the default.
pub fn parse_ollama_models(body: &Value, preferred: Option<&str>) -> Vec<EngineModel> {
    let names: Vec<String> = body["models"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|m| m["name"].as_str().map(str::to_string))
        .collect();
    let preferred = preferred.filter(|p| names.iter().any(|n| n == p));
    names
        .iter()
        .enumerate()
        // Local models have no tiers: Laya's effort never swaps them
        .map(|(i, n)| EngineModel::new(n.clone(), n.clone(), preferred.map_or(i == 0, |p| p == n), None))
        .collect()
}

async fn probe_ollama(env: &ProbeEnv) -> Probe {
    let Ok(client) = reqwest::Client::builder().timeout(Duration::from_secs(3)).build() else {
        return Probe::error("Couldn't start the check");
    };
    let root = env.ollama_root.trim_end_matches('/');
    let tags = match client.get(format!("{root}/api/tags")).send().await {
        Ok(r) if r.status().is_success() => r.json::<Value>().await.unwrap_or(Value::Null),
        Ok(r) => return Probe::error(format!("Ollama answered HTTP {}", r.status().as_u16())),
        Err(_) if env.ollama_installed => {
            return Probe::error(format!("Ollama isn't running at {root}: run `ollama serve`"))
        }
        Err(_) => return Probe::not_installed(format!("Ollama isn't running at {root}")),
    };
    let version = match client.get(format!("{root}/api/version")).send().await {
        Ok(r) => r.json::<Value>().await.ok().and_then(|v| v["version"].as_str().map(str::to_string)),
        Err(_) => None,
    };
    let preferred = std::env::var("OLLAMA_MODEL").ok();
    let models = parse_ollama_models(&tags, preferred.as_deref().map(str::trim));
    let probe = if models.is_empty() {
        Probe::error("No models pulled: run `ollama pull llama3.2`")
    } else {
        Probe::ready(format!("{} model(s) pulled", models.len()), models)
    };
    probe.with_version(version)
}

/// A refusal seen during a turn, shown over the last check until it expires or the next check.
#[derive(Debug, Clone)]
struct Mark {
    status: EngineStatus,
    detail: String,
    at: DateTime<Utc>,
    until: Option<DateTime<Utc>>,
}

/// Cached engine checks, shared by the API and turn routing.
#[derive(Default)]
pub struct EngineRegistry {
    checked: RwLock<HashMap<EngineId, EngineReport>>,
    marks: RwLock<HashMap<EngineId, Mark>>,
    /// Turned off in Settings (`engines_disabled`).
    disabled: RwLock<HashSet<EngineId>>,
    running: tokio::sync::Mutex<()>,
}

/// Settings key holding the ids of engines turned off, as a JSON list.
pub const DISABLED_SETTING: &str = "engines_disabled";

/// Engine ids in a stored `engines_disabled` value; unknown ids and bad JSON are ignored.
pub fn parse_disabled(stored: Option<&str>) -> HashSet<EngineId> {
    stored
        .and_then(|s| serde_json::from_str::<Vec<String>>(s).ok())
        .unwrap_or_default()
        .iter()
        .filter_map(|id| EngineId::parse(id))
        .collect()
}

impl EngineRegistry {
    /// Every engine as it stands: last check, with any newer refusal from a turn on top.
    pub fn snapshot(&self) -> Vec<EngineReport> {
        EngineId::ALL.iter().map(|id| self.report(*id)).collect()
    }

    pub fn report(&self, id: EngineId) -> EngineReport {
        if !self.is_enabled(id) {
            let mut report = self.checked.read().unwrap().get(&id).cloned().unwrap_or_else(|| EngineReport::checking(id));
            report.enabled = false;
            report.status = EngineStatus::Disabled;
            report.detail = Some("Turned off in Settings".into());
            report.models.clear();
            return report;
        }
        let mut report = self
            .checked
            .read()
            .unwrap()
            .get(&id)
            .cloned()
            .unwrap_or_else(|| EngineReport::checking(id));
        let now = Utc::now();
        if let Some(mark) = self.marks.read().unwrap().get(&id) {
            if mark.until.is_none_or(|u| u > now) {
                report.status = mark.status;
                report.detail = Some(mark.detail.clone());
                report.models.clear();
                report.checked_at = Some(mark.at);
            }
        }
        report
    }

    pub fn is_enabled(&self, id: EngineId) -> bool {
        !self.disabled.read().unwrap().contains(&id)
    }

    /// Replaces the set of engines turned off (from Settings).
    pub fn set_disabled(&self, ids: HashSet<EngineId>) {
        *self.disabled.write().unwrap() = ids;
    }

    /// Turns one engine on or off; returns the ids now off, sorted, for saving.
    pub fn set_enabled(&self, id: EngineId, enabled: bool) -> Vec<&'static str> {
        let mut disabled = self.disabled.write().unwrap();
        if enabled {
            disabled.remove(&id);
        } else {
            disabled.insert(id);
        }
        let mut ids: Vec<&'static str> = disabled.iter().map(|e| e.as_str()).collect();
        ids.sort();
        ids
    }

    /// Checks one engine (unless it is off) and stores the result.
    pub async fn recheck_one(&self, id: EngineId, env: &ProbeEnv) {
        if self.is_enabled(id) {
            let probe = probe(id, env).await;
            self.store(vec![(id, probe)]);
        }
    }

    pub fn status(&self, id: EngineId) -> EngineStatus {
        self.report(id).status
    }

    /// Whether a turn may use the engine. Before the first check finishes, `present` (binary
    /// found, key set) decides, as before checks existed.
    pub fn usable(&self, id: EngineId, present: bool) -> bool {
        match self.status(id) {
            EngineStatus::Ready => true,
            EngineStatus::Checking => present,
            _ => false,
        }
    }

    /// Stores check results; a fresh check replaces any refusal a turn saw.
    pub fn store(&self, results: Vec<(EngineId, Probe)>) {
        let now = Utc::now();
        let mut checked = self.checked.write().unwrap();
        let mut marks = self.marks.write().unwrap();
        for (id, probe) in results {
            marks.remove(&id);
            checked.insert(id, EngineReport::from_probe(id, probe, Some(now)));
        }
    }

    /// Checks every engine at once and returns the result. A call while a check is already
    /// running waits for it and returns its result instead of checking twice.
    pub async fn recheck(&self, env: &ProbeEnv) -> Vec<EngineReport> {
        let _guard = match self.running.try_lock() {
            Ok(g) => g,
            Err(_) => {
                let _wait = self.running.lock().await;
                return self.snapshot();
            }
        };
        // Engines turned off are never checked
        let on: Vec<EngineId> = EngineId::ALL.into_iter().filter(|id| self.is_enabled(*id)).collect();
        let results = futures::future::join_all(
            on.iter().map(|id| async move { (*id, probe(*id, env).await) }),
        )
        .await;
        self.store(results);
        self.snapshot()
    }

    /// A turn on this engine was refused: auth marks it signed out until the next check, quota
    /// marks it unavailable until the quota resets. Transient failures change nothing.
    pub fn record_failure(&self, id: EngineId, failure: &ProviderFailure) {
        let now = Utc::now();
        let (status, detail, until) = match failure.class {
            FailClass::Auth => (
                EngineStatus::SignedOut,
                match id.kind() {
                    EngineKind::Api => "API key rejected during a turn".to_string(),
                    _ => "Signed out during a turn: sign in again, then Re-check".to_string(),
                },
                None,
            ),
            FailClass::Quota => {
                let until = failure.retry_at.unwrap_or_else(|| {
                    now + chrono::Duration::minutes(crate::profiles::classify::DEFAULT_QUOTA_COOLDOWN_MINS)
                });
                (EngineStatus::Error, format!("Quota exceeded until {} UTC", until.format("%H:%M")), Some(until))
            }
            FailClass::Transient => return,
        };
        tracing::info!(engine = ?id, status = ?status, "engine marked unavailable after a refused turn");
        self.marks.write().unwrap().insert(id, Mark { status, detail, at: now, until });
    }
}

/// What Engines says about Custom API, which has nothing on the host to check.
pub const CUSTOM_API_DETAIL: &str = "Runs on this space's Custom API accounts: add one under Space settings → Accounts";

/// A turn on Custom API with no account of the space that can take it.
pub const NO_CUSTOM_ACCOUNT: &str =
    "No Custom API account is on in this space. Add one under Space settings → Accounts.";

/// The model a turn runs on one engine.
#[derive(Debug, Clone, PartialEq)]
pub struct ModelChoice {
    pub id: String,
    pub label: String,
    pub tier: Option<Tier>,
}

impl ModelChoice {
    fn of(m: &EngineModel) -> Self {
        Self { id: m.id.clone(), label: m.label.clone(), tier: m.tier }
    }

    /// The id to hand the CLI or API; `None` means the engine's own default (no `--model`).
    pub fn run_id(&self) -> Option<&str> {
        (self.id != "default").then_some(self.id.as_str())
    }
}

/// Picks the model for a turn among `models` (best or newest first, as checks list them).
///
/// A pinned model runs as is when the engine lists it (or lists nothing); otherwise the model
/// of the same tier stands in, so "sonnet" picked for the CLI still runs Sonnet on the API, and
/// an old pin like "claude-opus-5-5" runs Opus on the CLI. Without a pin, the first model of
/// `tier` (the default when it is of that tier), else the default.
pub fn choose_model(models: &[EngineModel], pinned: Option<&str>, tier: Tier) -> Option<ModelChoice> {
    let default = models.iter().find(|m| m.default).or(models.first());
    let of_tier = |t: Tier| {
        default
            .filter(|d| d.tier == Some(t))
            .or_else(|| models.iter().find(|m| m.tier == Some(t)))
    };
    if let Some(pin) = pinned.map(str::trim).filter(|p| !p.is_empty()) {
        if models.is_empty() {
            return Some(ModelChoice { id: pin.to_string(), label: pin.to_string(), tier: tier_from_name(pin) });
        }
        if let Some(m) = models.iter().find(|m| m.id.eq_ignore_ascii_case(pin)) {
            return Some(ModelChoice::of(m));
        }
        return tier_from_name(pin).and_then(of_tier).or(default).map(ModelChoice::of);
    }
    of_tier(tier).or(default).map(ModelChoice::of)
}

/// The models a turn on `id` can pick from: the last check's, or for Claude Code (whose aliases
/// never change) its aliases even when the host's own login is out and an account runs it.
pub fn known_models(engines: &EngineRegistry, id: EngineId) -> Vec<EngineModel> {
    let models = engines.report(id).models;
    if models.is_empty() && id == EngineId::Claude && engines.is_enabled(id) {
        return claude_cli_models();
    }
    models
}

/// Which engine a turn on `target_model` runs on without a profile, and why not when none can.
#[derive(Debug, Clone, PartialEq)]
pub enum SystemRoute {
    Api { engine: EngineId, key: String },
    Cli { engine: EngineId, bin: String },
    Ollama,
}

fn is_ollama_model(model: &str) -> bool {
    model == "local_ollama" || model.starts_with("ollama/")
}

/// The provider key a turn would use: the Settings key, else the provider's env var.
pub fn key_for(api: EngineId, configured_key: Option<String>) -> Option<String> {
    let var = match api {
        EngineId::AnthropicApi => "ANTHROPIC_API_KEY",
        EngineId::XaiApi => "XAI_API_KEY",
        _ => "OPENAI_API_KEY",
    };
    configured_key.or_else(|| std::env::var(var).ok()).filter(|k| !k.trim().is_empty())
}

/// Picks the API (when its key works) or else the CLI (when signed in); never another family's
/// engine. `Err` says why nothing can run it.
pub fn system_route(
    engines: &EngineRegistry,
    target_model: &str,
    configured_key: Option<String>,
    find_bin: impl Fn(&str) -> Option<String>,
) -> Result<SystemRoute, String> {
    if is_ollama_model(target_model) {
        return if engines.usable(EngineId::Ollama, true) {
            Ok(SystemRoute::Ollama)
        } else {
            Err(unavailable(target_model, &[engines.report(EngineId::Ollama)]))
        };
    }
    let Some(family) = Engine::from_model(target_model) else {
        return Err(format!("No engine runs the model '{target_model}'. Pick another in the bot's settings."));
    };
    // Never the host's env keys or CLIs: only the space's own accounts run it
    if family == Engine::Custom {
        return Err(NO_CUSTOM_ACCOUNT.into());
    }
    let (cli, api) = EngineId::of_family(family);
    if let Some(key) = key_for(api, configured_key) {
        if engines.usable(api, true) {
            return Ok(SystemRoute::Api { engine: api, key });
        }
    }
    if let Some(bin) = find_bin(family.as_str()) {
        if engines.usable(cli, true) {
            return Ok(SystemRoute::Cli { engine: cli, bin });
        }
    }
    Err(unavailable(target_model, &[engines.report(cli), engines.report(api)]))
}

/// "claude can't run right now (Claude Code CLI: Not signed in: run `claude auth login`; …)".
fn unavailable(target_model: &str, reports: &[EngineReport]) -> String {
    let why: Vec<String> = reports
        .iter()
        .map(|r| match (&r.detail, r.status) {
            (Some(d), _) => format!("{}: {d}", r.name),
            (None, EngineStatus::Checking) => format!("{}: not checked yet", r.name),
            (None, _) => format!("{}: not available", r.name),
        })
        .collect();
    format!(
        "'{target_model}' can't run right now ({}). Fix it in Settings → Engines, or pick another engine for this bot.",
        why.join("; ")
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versions_parse_from_each_cli() {
        assert_eq!(parse_version("2.1.296 (Claude Code)\n").as_deref(), Some("2.1.296"));
        assert_eq!(parse_version("codex-cli 0.162.1").as_deref(), Some("0.162.1"));
        assert_eq!(parse_version("grok 1.0.62 (1474f9cbdd91)").as_deref(), Some("1.0.62"));
        assert_eq!(parse_version("no version here"), None);
    }

    #[test]
    fn grok_models_output_parses_signed_in_and_out() {
        let out = "You are not authenticated.\n\nDefault model: grok-4.6\n\nAvailable models:\n  * grok-4.6 (default)\n  - grok-4.5\n";
        let (signed_in, models) = parse_grok_models(out);
        assert!(!signed_in);
        assert_eq!(models.iter().map(|m| m.id.as_str()).collect::<Vec<_>>(), ["grok-4.6", "grok-4.5"]);
        assert!(models[0].default && !models[1].default);
        let (signed_in, _) = parse_grok_models("Default model: grok-4.6\n\nAvailable models:\n  * grok-4.6 (default)\n");
        assert!(signed_in);
    }

    #[test]
    fn codex_models_keep_listed_ones_by_priority() {
        let json = r#"{"models":[
            {"slug":"b","display_name":"B","visibility":"list","priority":2},
            {"slug":"hidden","display_name":"H","visibility":"hide","priority":0},
            {"slug":"a","display_name":"A","visibility":"list","priority":1}]}"#;
        let models = parse_codex_models(json);
        assert_eq!(models.iter().map(|m| m.id.as_str()).collect::<Vec<_>>(), ["a", "b"]);
        assert!(models[0].default && !models[1].default);
        assert!(parse_codex_models("not json").is_empty());
    }

    #[test]
    fn api_model_lists_keep_chat_models_newest_first() {
        let openai = serde_json::json!({"data":[
            {"id":"gpt-5.5","created":10},{"id":"text-embedding-3-large","created":20},
            {"id":"gpt-6-sol","created":30},{"id":"gpt-4o-realtime-preview","created":40},{"id":"o4-mini","created":5}]});
        let m = parse_api_models(EngineId::OpenaiApi, &openai);
        assert_eq!(m.iter().map(|m| m.id.as_str()).collect::<Vec<_>>(), ["gpt-6-sol", "gpt-5.5", "o4-mini"]);
        assert!(m[0].default);

        let anthropic = serde_json::json!({"data":[
            {"id":"claude-fable-5-1","display_name":"Claude Fable 5.1","created_at":"2026-09-01T00:00:00Z"},
            {"id":"claude-opus-5-5","display_name":"Claude Opus 5.5","created_at":"2026-08-01T00:00:00Z"}]});
        let m = parse_api_models(EngineId::AnthropicApi, &anthropic);
        assert_eq!(m[0].label, "Claude Fable 5.1");
        assert!(m.iter().find(|x| x.id == super::super::api::DEFAULT_CLAUDE_MODEL).unwrap().default);
        assert_eq!(m.iter().filter(|x| x.default).count(), 1);

        let xai = serde_json::json!({"data":[{"id":"grok-4.6","created":2},{"id":"grok-imagine-image","created":3}]});
        assert_eq!(parse_api_models(EngineId::XaiApi, &xai).len(), 1);
    }

    #[test]
    fn api_refusals_map_to_status_without_the_key() {
        assert_eq!(api_refusal(401, "{\"error\":\"invalid x-api-key\"}").status, EngineStatus::SignedOut);
        let quota = api_refusal(429, "You exceeded your current quota");
        assert_eq!(quota.status, EngineStatus::Error);
        assert_eq!(quota.detail.as_deref(), Some("Quota exceeded (429)"));
        assert_eq!(api_refusal(500, "boom").detail.as_deref(), Some("The provider answered HTTP 500"));
    }

    #[test]
    fn ollama_default_is_the_configured_model() {
        let tags = serde_json::json!({"models":[{"name":"llama3.2"},{"name":"qwen3"}]});
        let m = parse_ollama_models(&tags, Some("qwen3"));
        assert!(!m[0].default && m[1].default);
        assert!(parse_ollama_models(&tags, Some("missing"))[0].default);
    }

    #[test]
    fn unchecked_engines_fall_back_to_presence_then_follow_checks() {
        let reg = EngineRegistry::default();
        assert_eq!(reg.status(EngineId::Claude), EngineStatus::Checking);
        assert!(reg.usable(EngineId::Claude, true));
        assert!(!reg.usable(EngineId::Claude, false));
        reg.store(vec![(EngineId::Claude, Probe::signed_out("Not signed in"))]);
        assert!(!reg.usable(EngineId::Claude, true));
        reg.store(vec![(EngineId::Claude, Probe::ready("Signed in", claude_cli_models()))]);
        assert!(reg.usable(EngineId::Claude, false));
        assert_eq!(reg.report(EngineId::Claude).default_model().unwrap().id, "opus");
    }

    #[test]
    fn refused_turns_mark_the_engine_until_the_next_check() {
        let reg = EngineRegistry::default();
        reg.store(vec![(EngineId::Codex, Probe::ready("Signed in", Vec::new()))]);
        let auth = ProviderFailure { class: FailClass::Auth, retry_at: None, detail: String::new() };
        reg.record_failure(EngineId::Codex, &auth);
        assert_eq!(reg.status(EngineId::Codex), EngineStatus::SignedOut);
        reg.store(vec![(EngineId::Codex, Probe::ready("Signed in", Vec::new()))]);
        assert_eq!(reg.status(EngineId::Codex), EngineStatus::Ready);

        // A quota mark lifts by itself once the reset time passes
        let past = ProviderFailure { class: FailClass::Quota, retry_at: Some(Utc::now() - chrono::Duration::minutes(1)), detail: String::new() };
        reg.record_failure(EngineId::Codex, &past);
        assert_eq!(reg.status(EngineId::Codex), EngineStatus::Ready);
        let future = ProviderFailure { class: FailClass::Quota, retry_at: Some(Utc::now() + chrono::Duration::hours(1)), detail: String::new() };
        reg.record_failure(EngineId::Codex, &future);
        let r = reg.report(EngineId::Codex);
        assert_eq!(r.status, EngineStatus::Error);
        assert!(r.detail.unwrap().starts_with("Quota exceeded until"));

        let transient = ProviderFailure { class: FailClass::Transient, retry_at: None, detail: String::new() };
        reg.record_failure(EngineId::Claude, &transient);
        assert_eq!(reg.status(EngineId::Claude), EngineStatus::Checking);
    }

    #[test]
    fn routing_skips_engines_that_are_not_ready() {
        let reg = EngineRegistry::default();
        let bin = |_: &str| Some("/usr/bin/fake".to_string());
        reg.store(vec![
            (EngineId::AnthropicApi, Probe::signed_out("API key rejected (401)")),
            (EngineId::Claude, Probe::ready("Signed in", claude_cli_models())),
        ]);
        // The rejected key is skipped for the signed-in CLI
        assert!(matches!(
            system_route(&reg, "claude", Some("sk-bad".into()), bin),
            Ok(SystemRoute::Cli { engine: EngineId::Claude, .. })
        ));
        reg.store(vec![(EngineId::Claude, Probe::signed_out("Not signed in: run `claude auth login`"))]);
        let err = system_route(&reg, "claude", Some("sk-bad".into()), bin).unwrap_err();
        assert!(err.contains("claude auth login") && err.contains("API key rejected"), "{err}");
        assert!(!err.contains("sk-bad"));
        // Never another family's engine, even when it is ready
        reg.store(vec![(EngineId::Codex, Probe::ready("Signed in", Vec::new()))]);
        assert!(system_route(&reg, "claude", None, bin).is_err());
        assert!(system_route(&reg, "mystery-model", None, bin).is_err());
        reg.store(vec![(EngineId::Ollama, Probe::not_installed("Ollama isn't running"))]);
        assert!(system_route(&reg, "local_ollama", None, bin).is_err());
    }

    fn tiers(models: &[EngineModel]) -> Vec<(&str, Option<Tier>)> {
        models.iter().map(|m| (m.id.as_str(), m.tier)).collect()
    }

    #[test]
    fn claude_cli_models_carry_their_tier() {
        assert_eq!(
            tiers(&claude_cli_models()),
            [("opus", Some(Tier::Deep)), ("sonnet", Some(Tier::Balanced)), ("haiku", Some(Tier::Fast)), ("fable", Some(Tier::Deep))]
        );
    }

    #[test]
    fn codex_tiers_come_from_the_description_then_the_slug() {
        // Shape of the real `codex debug models` output on this host
        let json = r#"{"models":[
            {"slug":"gpt-6.1-sol","display_name":"GPT-6.1 Sol","description":"Latest workhorse model","visibility":"list","priority":1},
            {"slug":"gpt-6-astra","display_name":"GPT-6 Astra","description":"Frontier intelligence for the most demanding work","visibility":"list","priority":2},
            {"slug":"gpt-6-luna","display_name":"GPT-6 Luna","description":"Fast and affordable model for easier tasks","visibility":"list","priority":3},
            {"slug":"gpt-5.6-terra","display_name":"GPT-5.6 Terra","description":"Older balanced model","visibility":"list","priority":4},
            {"slug":"gpt-5.5","display_name":"GPT-5.5","description":"Legacy coding model","visibility":"list","priority":5},
            {"slug":"gpt-7-mini","display_name":"GPT-7 mini","visibility":"list","priority":6}]}"#;
        assert_eq!(
            tiers(&parse_codex_models(json)),
            [
                ("gpt-6.1-sol", Some(Tier::Balanced)),
                ("gpt-6-astra", Some(Tier::Deep)),
                ("gpt-6-luna", Some(Tier::Fast)),
                ("gpt-5.6-terra", Some(Tier::Balanced)),
                ("gpt-5.5", None),
                ("gpt-7-mini", Some(Tier::Fast)),
            ]
        );
    }

    #[test]
    fn grok_api_and_ollama_tiers() {
        let (_, grok) = parse_grok_models("Default model: grok-4.6\n\nAvailable models:\n  * grok-4.6 (default)\n  - grok-4-heavy\n  - grok-4-fast\n");
        assert_eq!(tiers(&grok), [("grok-4.6", None), ("grok-4-heavy", Some(Tier::Deep)), ("grok-4-fast", Some(Tier::Fast))]);

        let anthropic = serde_json::json!({"data":[
            {"id":"claude-opus-5-5","created_at":"2026-08-03T00:00:00Z"},
            {"id":"claude-sonnet-5-5","created_at":"2026-08-02T00:00:00Z"},
            {"id":"claude-haiku-5-5","created_at":"2026-08-01T00:00:00Z"}]});
        let m = parse_api_models(EngineId::AnthropicApi, &anthropic);
        assert_eq!(tiers(&m), [
            ("claude-opus-5-5", Some(Tier::Deep)),
            ("claude-sonnet-5-5", Some(Tier::Balanced)),
            ("claude-haiku-5-5", Some(Tier::Fast)),
        ]);
        let openai = serde_json::json!({"data":[{"id":"gpt-6-luna","created":3},{"id":"o4-mini","created":2},{"id":"gpt-5.5","created":1}]});
        assert_eq!(tiers(&parse_api_models(EngineId::OpenaiApi, &openai)), [("gpt-6-luna", Some(Tier::Fast)), ("o4-mini", Some(Tier::Fast)), ("gpt-5.5", None)]);

        let ollama = serde_json::json!({"models":[{"name":"llama3.2:latest"},{"name":"qwen3-mini:8b"}]});
        assert!(parse_ollama_models(&ollama, None).iter().all(|m| m.tier.is_none()), "Ollama models have no tier");
    }

    #[test]
    fn effort_maps_to_tier() {
        assert_eq!(Tier::from_effort("low"), Tier::Fast);
        assert_eq!(Tier::from_effort("medium"), Tier::Balanced);
        assert_eq!(Tier::from_effort("high"), Tier::Deep);
        assert_eq!(Tier::from_effort("weird"), Tier::Balanced);
        assert_eq!(serde_json::to_value(Tier::Deep).unwrap(), "deep");
    }

    fn pick(models: &[EngineModel], pin: Option<&str>, tier: Tier) -> Option<String> {
        choose_model(models, pin, tier).map(|c| c.id)
    }

    #[test]
    fn choose_model_maps_tier_to_each_engines_model() {
        let claude = claude_cli_models();
        assert_eq!(pick(&claude, None, Tier::Fast).as_deref(), Some("haiku"));
        assert_eq!(pick(&claude, None, Tier::Balanced).as_deref(), Some("sonnet"));
        assert_eq!(pick(&claude, None, Tier::Deep).as_deref(), Some("opus"), "the default wins when it is of the tier");

        // No model of the tier: the engine default
        let grok = vec![EngineModel::new("grok-4.6", "grok-4.6", true, None), EngineModel::new("grok-4.5", "grok-4.5", false, None)];
        assert_eq!(pick(&grok, None, Tier::Fast).as_deref(), Some("grok-4.6"));
        assert_eq!(choose_model(&[], None, Tier::Fast), None);

        // A CLI whose list couldn't be read runs without --model
        let unknown = cli_default_model("Codex");
        assert_eq!(choose_model(&unknown, None, Tier::Deep).unwrap().run_id(), None);
    }

    #[test]
    fn pinned_models_run_as_is_or_as_the_same_tier() {
        let claude = claude_cli_models();
        assert_eq!(pick(&claude, Some("sonnet"), Tier::Fast).as_deref(), Some("sonnet"), "a pin beats Laya's tier");
        assert_eq!(pick(&claude, Some("HAIKU"), Tier::Deep).as_deref(), Some("haiku"));
        // Legacy full ids land on the CLI alias of the same tier
        assert_eq!(pick(&claude, Some("claude-opus-5-5"), Tier::Fast).as_deref(), Some("opus"));
        assert_eq!(pick(&claude, Some("claude-sonnet-5-5"), Tier::Fast).as_deref(), Some("sonnet"));
        // And a CLI alias pinned onto the API lands on the API's model of that tier
        let api = vec![
            EngineModel::new("claude-opus-5-5", "Opus", true, Some(Tier::Deep)),
            EngineModel::new("claude-sonnet-5-5", "Sonnet", false, Some(Tier::Balanced)),
        ];
        assert_eq!(pick(&api, Some("sonnet"), Tier::Fast).as_deref(), Some("claude-sonnet-5-5"));
        // Nothing listed: the pin runs as given
        assert_eq!(pick(&[], Some("gpt-5.5"), Tier::Fast).as_deref(), Some("gpt-5.5"));
        // Listed models but an unknown pin with no tier: the default
        assert_eq!(pick(&api, Some("mystery"), Tier::Fast).as_deref(), Some("claude-opus-5-5"));
    }

    #[test]
    fn disabled_setting_parses_leniently() {
        assert_eq!(parse_disabled(Some(r#"["claude","grok","nope"]"#)), HashSet::from([EngineId::Claude, EngineId::Grok]));
        assert!(parse_disabled(Some("not json")).is_empty());
        assert!(parse_disabled(None).is_empty());
    }

    #[test]
    fn disabled_engines_report_off_and_are_never_usable() {
        let reg = EngineRegistry::default();
        reg.store(vec![(EngineId::Claude, Probe::ready("Signed in", claude_cli_models()))]);
        assert!(reg.report(EngineId::Claude).enabled);
        assert_eq!(reg.set_enabled(EngineId::Grok, false), ["grok"]);
        assert_eq!(reg.set_enabled(EngineId::Claude, false), ["claude", "grok"]);
        let r = reg.report(EngineId::Claude);
        assert_eq!((r.status, r.enabled, r.models.len()), (EngineStatus::Disabled, false, 0));
        assert_eq!(serde_json::to_value(&r).unwrap()["status"], "disabled");
        assert!(!reg.usable(EngineId::Claude, true));
        assert!(!reg.usable(EngineId::Grok, true), "off even before its first check");
        assert!(known_models(&reg, EngineId::Claude).is_empty());
        let bin = |_: &str| Some("/bin/true".to_string());
        assert!(system_route(&reg, "claude", None, bin).is_err(), "routing never picks an engine that is off");

        assert_eq!(reg.set_enabled(EngineId::Claude, true), ["grok"]);
        let r = reg.report(EngineId::Claude);
        assert_eq!((r.status, r.enabled), (EngineStatus::Ready, true), "the last check comes back");
    }

    #[tokio::test]
    async fn recheck_skips_engines_that_are_off() {
        let reg = EngineRegistry::default();
        reg.set_disabled(EngineId::ALL.into_iter().collect());
        let reports = reg.recheck(&ProbeEnv::default()).await;
        assert!(reports.iter().all(|r| r.status == EngineStatus::Disabled && r.checked_at.is_none()));
        reg.recheck_one(EngineId::Claude, &ProbeEnv::default()).await;
        assert!(reg.report(EngineId::Claude).checked_at.is_none(), "recheck_one leaves an engine that is off alone");
    }
}
