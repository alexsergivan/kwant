//! Which engines can actually run on this host, and the models each one offers: every CLI's
//! own sign-in check, every API key against its provider's free model list, and Ollama's pulled
//! models. Checks run in the background (at startup and on Re-check) and are cached; turns only
//! read the cache, and a turn the provider refuses marks its engine at once.

use crate::profiles::{classify::redact_detail, Engine, FailClass, ProviderFailure};
use chrono::{DateTime, Utc};
use serde::Serialize;
use serde_json::Value;
use std::collections::HashMap;
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
}

impl EngineId {
    /// Every engine, in the order the API lists them.
    pub const ALL: [EngineId; 7] = [
        EngineId::Claude,
        EngineId::Codex,
        EngineId::Grok,
        EngineId::AnthropicApi,
        EngineId::OpenaiApi,
        EngineId::XaiApi,
        EngineId::Ollama,
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
        }
    }

    pub fn kind(self) -> EngineKind {
        match self {
            EngineId::Claude | EngineId::Codex | EngineId::Grok => EngineKind::Cli,
            EngineId::AnthropicApi | EngineId::OpenaiApi | EngineId::XaiApi => EngineKind::Api,
            EngineId::Ollama => EngineKind::Local,
        }
    }

    /// The CLI and the API that run a model family.
    pub fn of_family(engine: Engine) -> (EngineId, EngineId) {
        match engine {
            Engine::Claude => (EngineId::Claude, EngineId::AnthropicApi),
            Engine::Codex => (EngineId::Codex, EngineId::OpenaiApi),
            Engine::Grok => (EngineId::Grok, EngineId::XaiApi),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct EngineModel {
    pub id: String,
    pub label: String,
    pub default: bool,
}

/// One engine as `GET /api/engines` shows it.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct EngineReport {
    pub id: EngineId,
    pub name: String,
    pub kind: EngineKind,
    pub status: EngineStatus,
    pub detail: Option<String>,
    pub version: Option<String>,
    pub models: Vec<EngineModel>,
    pub checked_at: Option<DateTime<Utc>>,
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
            status: p.status,
            detail: p.detail,
            version: p.version,
            models: if p.status == EngineStatus::Ready { p.models } else { Vec::new() },
            checked_at,
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
            EngineId::Claude => probe_claude(env.claude_bin.as_deref()).await,
            EngineId::Codex => probe_codex(env.codex_bin.as_deref()).await,
            EngineId::Grok => probe_grok(env.grok_bin.as_deref()).await,
            EngineId::AnthropicApi | EngineId::OpenaiApi | EngineId::XaiApi => probe_api(id, env).await,
            EngineId::Ollama => probe_ollama(env).await,
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
    let out = tokio::process::Command::new(bin)
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
    .map(|(id, label, default)| EngineModel { id: id.into(), label: label.into(), default })
    .collect()
}

async fn probe_claude(bin: Option<&str>) -> Probe {
    let Some(bin) = bin else {
        return Probe::not_installed("Claude Code isn't installed");
    };
    let version = version_of(bin).await;
    // `auth status` asks Claude Code itself, so a login kept in the macOS Keychain counts.
    let out = match run(bin, &["auth", "status", "--json"]).await {
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
    let mut listed: Vec<(i64, String, String)> = v["models"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|m| m["visibility"].as_str().is_none_or(|s| s == "list"))
        .filter_map(|m| {
            let id = m["slug"].as_str()?.to_string();
            let label = m["display_name"].as_str().unwrap_or(&id).to_string();
            Some((m["priority"].as_i64().unwrap_or(i64::MAX), id, label))
        })
        .collect();
    listed.sort();
    listed
        .into_iter()
        .enumerate()
        .map(|(i, (_, id, label))| EngineModel { id, label, default: i == 0 })
        .collect()
}

/// Used when a CLI's model list can't be read: run it without `--model`.
fn cli_default_model(engine: &str) -> Vec<EngineModel> {
    vec![EngineModel { id: "default".into(), label: format!("{engine} default"), default: true }]
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
            Some(EngineModel { label: id.clone(), id, default })
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
        .map(|(i, (_, mid, label))| EngineModel {
            default: preferred.map_or(i == 0, |d| d == mid),
            id: mid,
            label,
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
        .map(|(i, n)| EngineModel {
            id: n.clone(),
            label: n.clone(),
            default: preferred.map_or(i == 0, |p| p == n),
        })
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
    running: tokio::sync::Mutex<()>,
}

impl EngineRegistry {
    /// Every engine as it stands: last check, with any newer refusal from a turn on top.
    pub fn snapshot(&self) -> Vec<EngineReport> {
        EngineId::ALL.iter().map(|id| self.report(*id)).collect()
    }

    pub fn report(&self, id: EngineId) -> EngineReport {
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
        let results = futures::future::join_all(
            EngineId::ALL.iter().map(|id| async move { (*id, probe(*id, env).await) }),
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
}
