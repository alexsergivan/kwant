//! LLM profiles: several accounts per engine (say a work and a personal Claude), how a turn
//! launches with one, and which one a turn moves to when an account runs out of quota.

pub mod classify;
pub mod login;
pub mod status;

use crate::db::models::{LlmProfile, LlmProfileState, ProfileKey};
use chrono::{DateTime, Utc};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

pub use classify::{classify, FailClass, ProviderFailure};
pub use status::{ProfileManager, ProfileStatus};

/// Most accounts one turn may try, across every engine of the space (Claude, Claude API key,
/// Codex, Grok, Custom API, plus a second account on a couple of them).
pub const MAX_ATTEMPTS: usize = 6;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Engine {
    Claude,
    Codex,
    Grok,
    /// A Custom API account: any OpenAI-compatible or Anthropic-compatible server (Agent
    /// Maestro, OpenRouter, LM Studio, …). Only ever runs on the space's own accounts.
    Custom,
}

impl Engine {
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "claude" => Some(Engine::Claude),
            "codex" => Some(Engine::Codex),
            "grok" => Some(Engine::Grok),
            "custom" => Some(Engine::Custom),
            _ => None,
        }
    }

    /// The engine a routed model id runs on, matching how drivers pick a CLI or API.
    /// Models of other providers (Ollama) have no profiles.
    pub fn from_model(model: &str) -> Option<Self> {
        // "custom:claude-sonnet-4.6" runs on a Custom API account, not on Claude's
        if model == "custom" || model.starts_with("custom:") || model.starts_with("custom/") {
            Some(Engine::Custom)
        } else if model.contains("claude") {
            Some(Engine::Claude)
        } else if model.contains("grok") {
            Some(Engine::Grok)
        } else if model.contains("codex") || model.contains("gpt") || model.contains("openai") {
            Some(Engine::Codex)
        } else {
            None
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Engine::Claude => "claude",
            Engine::Codex => "codex",
            Engine::Grok => "grok",
            Engine::Custom => "custom",
        }
    }

    /// Env variable that points the CLI at a config dir holding its login.
    pub fn home_env(self) -> &'static str {
        match self {
            Engine::Claude => "CLAUDE_CONFIG_DIR",
            Engine::Codex => "CODEX_HOME",
            Engine::Grok => "GROK_HOME",
            // No CLI; never set (see `system_home`)
            Engine::Custom => "KWNT_CUSTOM_API_HOME",
        }
    }

    /// Credentials in kwnt's env that would override a profile's login, removed for its CLI.
    pub fn credential_env(self) -> &'static [&'static str] {
        match self {
            Engine::Claude => &[
                "ANTHROPIC_API_KEY",
                "ANTHROPIC_AUTH_TOKEN",
                "CLAUDE_CODE_OAUTH_TOKEN",
                "CLAUDE_CODE_OAUTH_REFRESH_TOKEN",
                // A provider or proxy set for this machine (e.g. Bedrock) would run the
                // profile's turns there instead of on its own login
                "CLAUDE_CODE_USE_BEDROCK",
                "CLAUDE_CODE_USE_VERTEX",
                "CLAUDE_CODE_USE_FOUNDRY",
                "ANTHROPIC_BASE_URL",
                "CLAUDE_CODE_SKIP_BEDROCK_AUTH",
                "CLAUDE_CODE_SKIP_VERTEX_AUTH",
                "CLAUDE_CODE_SKIP_FOUNDRY_AUTH",
                "CLAUDE_CODE_SKIP_MANTLE_AUTH",
                "CLAUDE_CODE_SKIP_ANTHROPIC_AWS_AUTH",
                "CLAUDE_CODE_SKIP_ANTHROPIC_GOOGLE_CLOUD_AUTH",
                "AWS_BEARER_TOKEN_BEDROCK",
                "ANTHROPIC_GOOGLE_CLOUD_BASE_URL",
                // Model names set for that provider (e.g. a Bedrock id for "haiku") would be
                // sent to the profile's own API, which doesn't know them
                "ANTHROPIC_MODEL",
                "ANTHROPIC_SMALL_FAST_MODEL",
                "CLAUDE_CODE_SUBAGENT_MODEL",
                "CLAUDE_CODE_SUBAGENT_MODEL_FORCE",
                "CLAUDE_CODE_WORKFLOW_SUBAGENT_MODEL",
                "CLAUDE_CODE_AUTO_MODE_MODEL",
                "CLAUDE_CODE_BG_CLASSIFIER_MODEL",
            ],
            Engine::Codex => &["OPENAI_API_KEY", "CODEX_API_KEY"],
            Engine::Grok => &["XAI_API_KEY", "GROK_AUTH_PATH"],
            Engine::Custom => &[],
        }
    }

    /// Families of variables removed like [`Self::credential_env`]: every variable in kwnt's env
    /// starting with one of these. Covers the provider settings and per-model overrides
    /// (`ANTHROPIC_DEFAULT_HAIKU_MODEL`, its `_NAME`, …) without listing each one.
    pub fn credential_env_prefixes(self) -> &'static [&'static str] {
        match self {
            Engine::Claude => &[
                "ANTHROPIC_DEFAULT_",
                "ANTHROPIC_SMALL_FAST_MODEL_",
                "ANTHROPIC_CUSTOM_MODEL_OPTION",
                "ANTHROPIC_BEDROCK_",
                "ANTHROPIC_VERTEX_",
                "ANTHROPIC_FOUNDRY_",
                "ANTHROPIC_AWS_",
                "VERTEX_REGION_",
            ],
            Engine::Codex | Engine::Grok | Engine::Custom => &[],
        }
    }

    /// The Settings key the system profile uses, as before profiles.
    pub fn key_setting(self) -> &'static str {
        match self {
            Engine::Claude => "anthropic_api_key",
            Engine::Codex => "openai_api_key",
            Engine::Grok => "xai_api_key",
            // Never stored: a Custom API key lives with its account only
            Engine::Custom => "custom_api_key",
        }
    }

    /// Model id handed to the API driver, which picks the provider endpoint from it.
    pub fn api_model(self) -> &'static str {
        match self {
            Engine::Claude => "claude",
            Engine::Codex => "codex",
            Engine::Grok => "grok",
            Engine::Custom => "custom",
        }
    }

    /// The CLI's own config dir when kwnt doesn't set one: what the system profile uses.
    pub fn system_home(self) -> Option<PathBuf> {
        if self == Engine::Custom {
            return None;
        }
        let from_env = std::env::var_os(self.home_env())
            .filter(|v| !v.is_empty())
            .map(PathBuf::from);
        from_env.or_else(|| {
            dirs::home_dir().map(|h| {
                h.join(match self {
                    Engine::Claude => ".claude",
                    Engine::Codex => ".codex",
                    Engine::Grok => ".grok",
                    Engine::Custom => unreachable!("Custom API has no CLI home"),
                })
            })
        })
    }
}

/// How a turn runs with a profile.
#[derive(Debug, Clone)]
pub enum Launch {
    /// Exactly as before profiles: Settings key, then env key, then the CLI's own login.
    System,
    /// The API driver with this profile's stored key.
    ApiKey(ProfileKey),
    /// The engine's CLI with its config dir pointed at this home.
    CliHome(PathBuf),
}

/// Env changes that run a CLI on a profile's home: its home variable set and every credential
/// variable that would override the home's login removed.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct CliEnv {
    pub set: Vec<(String, std::ffi::OsString)>,
    pub remove: Vec<String>,
    /// Arguments that go before the CLI's subcommand (see [`CliEnv::command`]).
    pub args: Vec<String>,
}

/// Values Claude Code takes as "off" for the provider and model settings a project's
/// `.claude/settings*.json` could set. Passed with `--settings`, which outranks project and
/// user settings, so a profile runs on its own login even in a folder set up for Bedrock.
/// Empty means unset to Claude Code: its own model names and first-party API.
const CLAUDE_SETTINGS_OVERRIDE: &[(&str, &str)] = &[
    ("CLAUDE_CODE_USE_BEDROCK", "0"),
    ("CLAUDE_CODE_USE_VERTEX", "0"),
    ("CLAUDE_CODE_USE_FOUNDRY", "0"),
    ("ANTHROPIC_BASE_URL", ""),
    ("ANTHROPIC_MODEL", ""),
    ("ANTHROPIC_DEFAULT_HAIKU_MODEL", ""),
    ("ANTHROPIC_DEFAULT_SONNET_MODEL", ""),
    ("ANTHROPIC_DEFAULT_OPUS_MODEL", ""),
    ("ANTHROPIC_DEFAULT_FABLE_MODEL", ""),
    ("ANTHROPIC_SMALL_FAST_MODEL", ""),
    ("CLAUDE_CODE_SUBAGENT_MODEL", ""),
];

impl CliEnv {
    pub fn for_home(engine: Engine, home: &Path) -> Self {
        Self::for_home_in(engine, home, std::env::vars_os().map(|(k, _)| k))
    }

    /// [`Self::for_home`] against the given env variable names instead of kwnt's own.
    pub fn for_home_in(
        engine: Engine,
        home: &Path,
        env_keys: impl IntoIterator<Item = std::ffi::OsString>,
    ) -> Self {
        let mut remove: Vec<String> = engine
            .credential_env()
            .iter()
            .map(|s| s.to_string())
            .collect();
        let prefixes = engine.credential_env_prefixes();
        for key in env_keys {
            let Some(key) = key.to_str() else { continue };
            if prefixes.iter().any(|p| key.starts_with(p)) && !remove.iter().any(|r| r == key) {
                remove.push(key.to_string());
            }
        }
        let args = match engine {
            Engine::Claude => {
                let env: serde_json::Map<String, serde_json::Value> = CLAUDE_SETTINGS_OVERRIDE
                    .iter()
                    .map(|(k, v)| (k.to_string(), serde_json::Value::from(*v)))
                    .collect();
                vec!["--settings".into(), serde_json::json!({ "env": env }).to_string()]
            }
            Engine::Codex | Engine::Grok | Engine::Custom => Vec::new(),
        };
        Self {
            set: vec![(engine.home_env().to_string(), home.as_os_str().to_owned())],
            remove,
            args,
        }
    }

    /// `bin` run on the profile's home: env applied and [`Self::args`] in place, ready for the
    /// subcommand's own arguments.
    pub fn command(&self, bin: impl AsRef<std::ffi::OsStr>) -> tokio::process::Command {
        let mut cmd = tokio::process::Command::new(bin);
        self.apply(&mut cmd);
        cmd.args(&self.args);
        cmd
    }

    pub fn apply(&self, cmd: &mut tokio::process::Command) {
        for key in &self.remove {
            cmd.env_remove(key);
        }
        for (key, value) in &self.set {
            cmd.env(key, value);
        }
    }
}

/// Where kwnt keeps the home of a profile it manages.
pub fn managed_home(data_dir: &Path, profile_id: &str) -> PathBuf {
    data_dir.join("profiles").join(profile_id)
}

/// Creates a managed home, readable by this user only.
pub fn create_private_dir(path: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(path)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700))?;
    }
    Ok(())
}

/// Checks a home dir the user picked for a `cli_home` profile and returns its canonical path:
/// an existing, absolute, real (not symlinked) directory owned by kwnt's user, and not `/`,
/// `$HOME`, the engine's own default home or another profile's home.
pub fn validate_user_home(
    raw: &str,
    engine: Engine,
    other_homes: &[String],
) -> Result<PathBuf, String> {
    let path = Path::new(raw);
    if !path.is_absolute() {
        return Err("home_dir must be an absolute path".into());
    }
    let meta =
        std::fs::symlink_metadata(path).map_err(|_| "home_dir does not exist".to_string())?;
    if meta.file_type().is_symlink() {
        return Err("home_dir must not be a symlink".into());
    }
    if !meta.is_dir() {
        return Err("home_dir must be a directory".into());
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        let own_uid = dirs::home_dir()
            .and_then(|h| std::fs::metadata(h).ok())
            .map(|m| m.uid());
        if own_uid.is_some_and(|uid| uid != meta.uid()) {
            return Err("home_dir must belong to the user kwnt runs as".into());
        }
    }
    let canonical = path
        .canonicalize()
        .map_err(|_| "home_dir is not readable".to_string())?;
    let canon = |p: PathBuf| p.canonicalize().unwrap_or(p);
    if canonical == Path::new("/")
        || dirs::home_dir().map(canon).as_deref() == Some(canonical.as_path())
    {
        return Err("home_dir is too broad; pick a dedicated directory".into());
    }
    if engine.system_home().map(canon).as_deref() == Some(canonical.as_path()) {
        return Err(
            "home_dir is this engine's default login, which the system profile already uses".into(),
        );
    }
    if other_homes
        .iter()
        .any(|h| canon(PathBuf::from(h)) == canonical)
    {
        return Err("another profile already uses this home_dir".into());
    }
    Ok(canonical)
}

/// Facts about a profile that decide whether a turn can use it right now.
#[derive(Debug, Default)]
pub struct Availability {
    pub states: HashMap<String, LlmProfileState>,
    pub with_key: HashSet<String>,
    /// Profiles whose last login check said signed out.
    pub signed_out: HashSet<String>,
    /// Profiles whose last login check said signed in.
    pub signed_in: HashSet<String>,
}

/// How sure an account check must be that a profile is signed in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SignInBar {
    /// Not known to be signed out: what turn routing uses, so a profile isn't skipped before
    /// its first sign-in check.
    NotSignedOut,
    /// Signed in on its last check (an API-key profile: has its key): what Engines calls ready.
    SignedIn,
}

impl Availability {
    /// Whether `p` passes `bar`: also never out of quota, and never an API-key profile without
    /// its key.
    pub fn admits(&self, p: &LlmProfile, now: DateTime<Utc>, bar: SignInBar) -> bool {
        self.usable(p, now)
            && (bar == SignInBar::NotSignedOut
                || p.kind == "api_key"
                || self.signed_in.contains(&p.id))
    }

    /// Whether `p` could take a turn now: not out of usage, not an API key without its key,
    /// not known to be signed out.
    pub fn usable(&self, p: &LlmProfile, now: DateTime<Utc>) -> bool {
        let limited = self
            .states
            .get(&p.id)
            .and_then(|s| s.limited_until)
            .is_some_and(|t| t > now);
        let keyless = p.kind == "api_key" && !self.with_key.contains(&p.id);
        !limited && !keyless && !self.signed_out.contains(&p.id)
    }
}

/// `engine`'s enabled profiles in the order failover prefers them: `bound` (if it is one of
/// them) or else the default first, then the rest by priority.
pub fn preference_order<'a>(
    profiles: &'a [LlmProfile],
    engine: Engine,
    bound: Option<&str>,
) -> Vec<&'a LlmProfile> {
    let mut mine: Vec<&LlmProfile> = profiles
        .iter()
        .filter(|p| p.engine == engine.as_str() && p.enabled)
        .collect();
    mine.sort_by(|a, b| (a.priority, a.created_at, &a.id).cmp(&(b.priority, b.created_at, &b.id)));
    let first = bound
        .and_then(|id| mine.iter().position(|p| p.id == id))
        .or_else(|| mine.iter().position(|p| p.is_default));
    if let Some(i) = first {
        let p = mine.remove(i);
        mine.insert(0, p);
    }
    mine
}

/// Profiles a turn on `engine` tries, in order. First the kwant's own profile (`bound`, if it is
/// an enabled profile of this engine) or else the engine default; then, with failover on, the
/// engine's other enabled profiles by priority that aren't out of quota, keyless or signed out.
/// The first one is skipped too when unusable, unless failover is off or nothing else is usable.
/// Empty when the engine has no enabled profile: the turn then runs as before profiles.
pub fn failover_order(
    profiles: &[LlmProfile],
    engine: Engine,
    bound: Option<&str>,
    failover: bool,
    avail: &Availability,
    now: DateTime<Utc>,
) -> Vec<LlmProfile> {
    let mine = preference_order(profiles, engine, bound);
    let Some(&first) = mine.first() else {
        return Vec::new();
    };
    if !failover {
        return vec![first.clone()];
    }
    let rest: Vec<LlmProfile> = mine[1..]
        .iter()
        .filter(|p| avail.usable(p, now))
        .map(|p| (*p).clone())
        .collect();
    let mut order = Vec::with_capacity(MAX_ATTEMPTS);
    if avail.usable(first, now) || rest.is_empty() {
        order.push(first.clone());
    }
    order.extend(rest);
    order.truncate(MAX_ATTEMPTS);
    order
}

/// What a turn on one engine may try across the whole space, and why it started where it did.
#[derive(Debug, Default, Clone, PartialEq)]
pub struct GlobalPlan {
    /// Accounts to try in order; `None` is the host's own login or Settings key when the bot's
    /// engine has no account of its own.
    pub attempts: Vec<Option<LlmProfile>>,
    /// The bot's own preferred account when it was left out because it is known to be out of
    /// usage, so the turn can say once why it runs elsewhere.
    pub skipped: Option<LlmProfile>,
    /// When the skipped account's limit resets (`None`: it is skipped for another reason).
    pub skipped_until: Option<DateTime<Utc>>,
    /// Set when every account that could take a turn is known to be out of usage: the earliest
    /// reset. The turn then parks instead of trying any of them.
    pub all_out_until: Option<DateTime<Utc>>,
}

/// What the caller knows that the profile list doesn't, for [`global_order`].
pub struct OrderRules<'a> {
    /// Whether the host's own login or key may serve a bot whose engine has no account of its own.
    pub host_head: bool,
    /// Whether an account's engine may run it at all (engine switched off, host login unusable).
    pub eligible: &'a dyn Fn(&LlmProfile) -> bool,
}

/// The accounts a turn on `engine` tries: its own preferred account first (`bound`, else the
/// engine's default), then, with failover on, every other usable account of the space across
/// all engines in ONE order, by `priority` (ties: oldest first). Accounts
/// known to be out of usage are skipped up front, the bot's own too, unless nothing else is
/// usable. With failover off the bot stays pinned to its own account.
pub fn global_order(
    profiles: &[LlmProfile],
    engine: Engine,
    bound: Option<&str>,
    failover: bool,
    avail: &Availability,
    now: DateTime<Utc>,
    rules: &OrderRules,
) -> GlobalPlan {
    let OrderRules { host_head, eligible } = rules;
    let host_head = *host_head;
    let elig: Vec<LlmProfile> = profiles.iter().filter(|p| p.enabled && eligible(p)).cloned().collect();
    // The Accounts list order (`priority`) is the one truth: `is_default` does not jump the queue
    // here. A bot explicitly bound to an enabled account of its engine starts on that account;
    // every other bot starts at the top of its engine's accounts in the list.
    let mut mine: Vec<&LlmProfile> = elig.iter().filter(|p| p.engine == engine.as_str()).collect();
    mine.sort_by(|a, b| (a.priority, a.created_at, &a.id).cmp(&(b.priority, b.created_at, &b.id)));
    if let Some(i) = bound.and_then(|id| mine.iter().position(|p| p.id == id)) {
        let p = mine.remove(i);
        mine.insert(0, p);
    }
    let first = mine.first().map(|p| (*p).clone());
    if !failover {
        let attempts = match first {
            Some(p) => vec![Some(p)],
            None if host_head => vec![None],
            None => Vec::new(),
        };
        return GlobalPlan { attempts, ..Default::default() };
    }
    let mut rest: Vec<&LlmProfile> = elig
        .iter()
        .filter(|p| first.as_ref().is_none_or(|f| f.id != p.id) && avail.usable(p, now))
        .collect();
    rest.sort_by(|a, b| (a.priority, a.created_at, &a.id).cmp(&(b.priority, b.created_at, &b.id)));
    let mut plan = GlobalPlan::default();
    match first {
        Some(f) if avail.usable(&f, now) || rest.is_empty() => plan.attempts.push(Some(f)),
        Some(f) => {
            plan.skipped_until = avail.states.get(&f.id).and_then(|s| s.limited_until).filter(|t| *t > now);
            plan.skipped = Some(f);
        }
        None if host_head => plan.attempts.push(None),
        None => {}
    }
    plan.attempts.extend(rest.into_iter().map(|p| Some(p.clone())));
    plan.attempts.truncate(MAX_ATTEMPTS);
    // Every account that could work, out of usage: nothing to try until the earliest reset
    if plan.attempts.iter().all(Option::is_some) {
        let limited_until = |p: &LlmProfile| {
            avail.states.get(&p.id).and_then(|s| s.limited_until).filter(|t| *t > now)
        };
        let live: Vec<&LlmProfile> = elig
            .iter()
            .filter(|p| !avail.signed_out.contains(&p.id) && !(p.kind == "api_key" && !avail.with_key.contains(&p.id)))
            .collect();
        if !live.is_empty() && live.iter().all(|p| limited_until(p).is_some()) {
            plan.all_out_until = live.iter().filter_map(|p| limited_until(p)).min();
            plan.attempts.clear();
        }
    }
    plan
}

/// Validates a profile name: 1 to 40 printable characters, trimmed.
pub fn clean_name(raw: &str) -> Result<String, String> {
    let name = raw.trim();
    if name.is_empty() || name.chars().count() > 40 || name.chars().any(char::is_control) {
        return Err("name must be 1 to 40 printable characters".into());
    }
    Ok(name.to_string())
}

/// Validates an API key: 8 to 512 characters, no whitespace or control characters.
pub fn clean_key(raw: &str) -> Result<ProfileKey, String> {
    let key = raw.trim();
    let len = key.chars().count();
    if !(8..=512).contains(&len) || key.chars().any(|c| c.is_whitespace() || c.is_control()) {
        return Err("api_key must be 8 to 512 characters with no spaces".into());
    }
    Ok(ProfileKey(key.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn profile(id: &str, engine: &str, kind: &str, priority: i64, default: bool) -> LlmProfile {
        LlmProfile {
            space_id: crate::db::spaces::DEFAULT_SPACE_ID.into(),
            id: id.into(),
            engine: engine.into(),
            name: id.into(),
            kind: kind.into(),
            home_dir: None,
            priority,
            enabled: true,
            is_default: default,
            created_at: None,
        }
    }

    fn now() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 10, 16, 12, 0, 0).unwrap()
    }

    fn ids(v: &[LlmProfile]) -> Vec<&str> {
        v.iter().map(|p| p.id.as_str()).collect()
    }

    #[test]
    fn engine_from_model() {
        assert_eq!(Engine::from_model("claude"), Some(Engine::Claude));
        assert_eq!(Engine::from_model("claude-opus-5-5"), Some(Engine::Claude));
        assert_eq!(Engine::from_model("grok-beta"), Some(Engine::Grok));
        assert_eq!(Engine::from_model("gpt-5"), Some(Engine::Codex));
        assert_eq!(Engine::from_model("codex"), Some(Engine::Codex));
        assert_eq!(Engine::from_model("local_ollama"), None);
    }

    #[test]
    fn launch_env_per_engine() {
        let home = Path::new("/data/profiles/prf_1");
        let env = CliEnv::for_home(Engine::Claude, home);
        assert_eq!(
            env.set,
            vec![("CLAUDE_CONFIG_DIR".to_string(), home.as_os_str().to_owned())]
        );
        assert!(env.remove.contains(&"ANTHROPIC_API_KEY".to_string()));
        assert!(env.remove.contains(&"CLAUDE_CODE_OAUTH_TOKEN".to_string()));
        for provider in ["CLAUDE_CODE_USE_BEDROCK", "CLAUDE_CODE_USE_VERTEX", "CLAUDE_CODE_USE_FOUNDRY", "ANTHROPIC_BASE_URL"] {
            assert!(env.remove.contains(&provider.to_string()), "{provider}");
        }
        // Alex's Bedrock shell: its model names must not reach a profile's own login
        let shell = [
            "ANTHROPIC_DEFAULT_HAIKU_MODEL",
            "ANTHROPIC_DEFAULT_SONNET_MODEL",
            "ANTHROPIC_DEFAULT_OPUS_MODEL",
            "ANTHROPIC_DEFAULT_HAIKU_MODEL_NAME",
            "ANTHROPIC_BEDROCK_BASE_URL",
            "ANTHROPIC_VERTEX_PROJECT_ID",
            "AWS_REGION",
            "PATH",
        ];
        let env = CliEnv::for_home_in(Engine::Claude, home, shell.iter().map(|k| k.into()));
        for k in &shell[..6] {
            assert!(env.remove.contains(&k.to_string()), "{k}");
        }
        for k in ["ANTHROPIC_MODEL", "ANTHROPIC_SMALL_FAST_MODEL", "CLAUDE_CODE_SUBAGENT_MODEL"] {
            assert!(env.remove.contains(&k.to_string()), "{k}");
        }
        // Ordinary tool env stays: a kwant's own AWS work still has its region
        assert!(!env.remove.iter().any(|k| k == "AWS_REGION" || k == "PATH"));
        // A project's .claude/settings.json can't switch it back on: --settings outranks it
        assert_eq!(env.args[0], "--settings");
        let over: serde_json::Value = serde_json::from_str(&env.args[1]).unwrap();
        assert_eq!(over["env"]["CLAUDE_CODE_USE_BEDROCK"], "0");
        assert_eq!(over["env"]["ANTHROPIC_DEFAULT_HAIKU_MODEL"], "");
        let env = CliEnv::for_home(Engine::Codex, home);
        assert_eq!(env.set[0].0, "CODEX_HOME");
        assert!(env.args.is_empty());
        assert!(env.remove.contains(&"OPENAI_API_KEY".to_string()));
        let env = CliEnv::for_home(Engine::Grok, home);
        assert_eq!(env.set[0].0, "GROK_HOME");
        assert!(env.remove.contains(&"XAI_API_KEY".to_string()));
        assert!(env.remove.contains(&"GROK_AUTH_PATH".to_string()));
    }

    #[test]
    fn order_starts_with_bound_then_default_then_priority() {
        let profiles = vec![
            profile("sys", "claude", "system", 0, true),
            profile("work", "claude", "cli_home", 10, false),
            profile("personal", "claude", "cli_home", 20, false),
            profile("g", "grok", "system", 0, true),
        ];
        let empty = Availability::default();
        assert_eq!(
            ids(&failover_order(
                &profiles,
                Engine::Claude,
                None,
                true,
                &empty,
                now()
            )),
            ["sys", "work", "personal"]
        );
        assert_eq!(
            ids(&failover_order(
                &profiles,
                Engine::Claude,
                Some("personal"),
                true,
                &empty,
                now()
            )),
            ["personal", "sys", "work"]
        );
        // A binding to another engine's profile is ignored
        assert_eq!(
            ids(&failover_order(
                &profiles,
                Engine::Claude,
                Some("g"),
                true,
                &empty,
                now()
            ))[0],
            "sys"
        );
        // Pinned: only the first
        assert_eq!(
            ids(&failover_order(
                &profiles,
                Engine::Claude,
                Some("work"),
                false,
                &empty,
                now()
            )),
            ["work"]
        );
        assert!(failover_order(&profiles, Engine::Codex, None, true, &empty, now()).is_empty());
    }

    #[test]
    fn order_skips_limited_keyless_signed_out_and_disabled() {
        let mut profiles = vec![
            profile("sys", "claude", "system", 0, true),
            profile("a", "claude", "cli_home", 1, false),
            profile("b", "claude", "api_key", 2, false),
            profile("c", "claude", "cli_home", 3, false),
            profile("d", "claude", "cli_home", 4, false),
        ];
        profiles[4].enabled = false;
        let states = HashMap::from([(
            "sys".to_string(),
            LlmProfileState {
                profile_id: "sys".into(),
                limited_until: Some(now() + chrono::Duration::hours(1)),
                last_error: Some("quota".into()),
            },
        )]);
        let avail = Availability {
            states,
            with_key: HashSet::new(),
            signed_out: HashSet::from(["a".to_string()]),
            ..Default::default()
        };
        // sys is limited so it's skipped even as the default; a is signed out; b has no key; d is disabled
        assert_eq!(
            ids(&failover_order(
                &profiles,
                Engine::Claude,
                None,
                true,
                &avail,
                now()
            )),
            ["c"]
        );
        // Pinned to a limited profile: still tried, the user asked for it
        assert_eq!(
            ids(&failover_order(
                &profiles,
                Engine::Claude,
                None,
                false,
                &avail,
                now()
            )),
            ["sys"]
        );
        // An expired limit doesn't count
        let later = now() + chrono::Duration::hours(2);
        assert_eq!(
            ids(&failover_order(
                &profiles,
                Engine::Claude,
                None,
                true,
                &avail,
                later
            )),
            ["sys", "c"]
        );
    }

    #[test]
    fn order_keeps_unusable_first_when_nothing_else_works() {
        let profiles = vec![profile("sys", "claude", "system", 0, true)];
        let states = HashMap::from([(
            "sys".to_string(),
            LlmProfileState {
                profile_id: "sys".into(),
                limited_until: Some(now() + chrono::Duration::hours(1)),
                last_error: None,
            },
        )]);
        let avail = Availability {
            states,
            ..Default::default()
        };
        assert_eq!(
            ids(&failover_order(
                &profiles,
                Engine::Claude,
                None,
                true,
                &avail,
                now()
            )),
            ["sys"]
        );
    }

    #[test]
    fn order_is_capped() {
        let profiles: Vec<_> = (0..9)
            .map(|i| profile(&format!("p{i}"), "codex", "cli_home", i, i == 0))
            .collect();
        assert_eq!(
            failover_order(
                &profiles,
                Engine::Codex,
                None,
                true,
                &Availability::default(),
                now()
            )
            .len(),
            MAX_ATTEMPTS
        );
    }

    fn limited(ids: &[(&str, i64)]) -> Availability {
        Availability {
            states: ids
                .iter()
                .map(|(id, hours)| {
                    (
                        id.to_string(),
                        LlmProfileState {
                            profile_id: id.to_string(),
                            limited_until: Some(now() + chrono::Duration::hours(*hours)),
                            last_error: Some("quota".into()),
                        },
                    )
                })
                .collect(),
            ..Default::default()
        }
    }

    fn all_on(_: &LlmProfile) -> bool {
        true
    }

    fn plan_ids(p: &GlobalPlan) -> Vec<String> {
        p.attempts.iter().map(|a| a.as_ref().map_or("host".into(), |p| p.id.clone())).collect()
    }

    /// Claude (default, priority 10), Codex (20), Custom/Maestro (5), Grok (30), one Claude key (40).
    fn space() -> Vec<LlmProfile> {
        vec![
            profile("claude", "claude", "cli_home", 10, true),
            profile("codex", "codex", "cli_home", 20, true),
            profile("maestro", "custom", "custom_api", 5, true),
            profile("grok", "grok", "cli_home", 30, true),
            profile("ckey", "claude", "api_key", 40, false),
        ]
    }

    fn global(engine: Engine, failover: bool, avail: &Availability) -> GlobalPlan {
        let mut avail_keys = Availability { with_key: HashSet::from(["ckey".to_string()]), ..Default::default() };
        avail_keys.states = avail.states.clone();
        avail_keys.signed_out = avail.signed_out.clone();
        global_order(&space(), engine, None, failover, &avail_keys, now(), &OrderRules { host_head: false, eligible: &all_on })
    }

    #[test]
    fn failover_crosses_engines_by_one_priority_order() {
        // Nothing limited: the bot's own account first, then everything else by priority
        let p = global(Engine::Claude, true, &Availability::default());
        assert_eq!(plan_ids(&p), ["claude", "maestro", "codex", "grok", "ckey"]);
        assert_eq!((p.skipped, p.all_out_until), (None, None));
        // A Codex bot starts on Codex, then Maestro (5) before Claude (10)
        assert_eq!(plan_ids(&global(Engine::Codex, true, &Availability::default())), ["codex", "maestro", "claude", "grok", "ckey"]);
    }

    #[test]
    fn the_list_order_decides_and_is_default_does_not_jump_the_queue() {
        let mut profiles = space();
        // A second Claude account sits above the default one in the Accounts list
        profiles.push(profile("claude2", "claude", "cli_home", 1, false));
        let plan = |bound: Option<&str>| {
            let p = global_order(&profiles, Engine::Claude, bound, true, &Availability::default(), now(), &OrderRules { host_head: false, eligible: &all_on });
            plan_ids(&p)
        };
        // Not bound: the top of the list for its engine, although `claude` is the default
        assert_eq!(plan(None), ["claude2", "maestro", "claude", "codex", "grok"]);
        // Bound to an account: that account first, then the list in order
        assert_eq!(plan(Some("claude")), ["claude", "claude2", "maestro", "codex", "grok"]);
        // A binding to an account of another engine, or a gone one, is not a binding
        assert_eq!(plan(Some("codex"))[0], "claude2");
        assert_eq!(plan(Some("gone"))[0], "claude2");
    }

    #[test]
    fn a_limited_account_is_skipped_up_front_and_named() {
        let p = global(Engine::Claude, true, &limited(&[("claude", 3)]));
        assert_eq!(plan_ids(&p), ["maestro", "codex", "grok", "ckey"]);
        assert_eq!(p.skipped_until, Some(now() + chrono::Duration::hours(3)));
        assert_eq!(p.skipped.map(|s| s.id), Some("claude".into()));
        // A limit that has already passed doesn't count: back on the preferred account
        let mut past = limited(&[("claude", 3)]);
        past.states.get_mut("claude").unwrap().limited_until = Some(now() - chrono::Duration::minutes(1));
        let p = global(Engine::Claude, true, &past);
        assert_eq!(plan_ids(&p)[0], "claude");
        assert!(p.skipped.is_none());
    }

    #[test]
    fn signed_out_keyless_and_ineligible_accounts_are_never_tried() {
        let mut avail = limited(&[]);
        avail.signed_out.insert("codex".into());
        let no_grok = |p: &LlmProfile| p.engine != "grok";
        let mut with_key = Availability { with_key: HashSet::new(), ..Default::default() };
        with_key.signed_out = avail.signed_out.clone();
        let p = global_order(&space(), Engine::Claude, None, true, &with_key, now(), &OrderRules { host_head: false, eligible: &no_grok });
        // codex signed out, grok switched off, the Claude key has no key
        assert_eq!(plan_ids(&p), ["claude", "maestro"]);
    }

    #[test]
    fn failover_off_pins_the_bot_to_its_own_account() {
        let p = global(Engine::Claude, false, &limited(&[("claude", 3)]));
        assert_eq!(plan_ids(&p), ["claude"]);
        assert!(p.skipped.is_none() && p.all_out_until.is_none());
    }

    #[test]
    fn everything_out_gives_the_earliest_reset_and_no_attempts() {
        let p = global(Engine::Claude, true, &limited(&[("claude", 3), ("codex", 5), ("maestro", 4), ("grok", 9), ("ckey", 6)]));
        assert!(p.attempts.is_empty());
        assert_eq!(p.all_out_until, Some(now() + chrono::Duration::hours(3)));
        // One account still usable: not all out
        let p = global(Engine::Claude, true, &limited(&[("claude", 3), ("codex", 5), ("maestro", 4), ("ckey", 6)]));
        assert_eq!(plan_ids(&p), ["grok"]);
        assert!(p.all_out_until.is_none());
    }

    #[test]
    fn a_bot_whose_engine_has_no_account_starts_on_the_host() {
        let profiles = vec![profile("codex", "codex", "cli_home", 20, true)];
        let rules = OrderRules { host_head: true, eligible: &all_on };
        let p = global_order(&profiles, Engine::Claude, None, true, &Availability::default(), now(), &rules);
        assert_eq!(plan_ids(&p), ["host", "codex"]);
        let rules = OrderRules { host_head: false, eligible: &all_on };
        let p = global_order(&profiles, Engine::Claude, None, true, &Availability::default(), now(), &rules);
        assert_eq!(plan_ids(&p), ["codex"]);
    }

    #[test]
    fn the_cap_covers_every_engine() {
        assert_eq!(global(Engine::Claude, true, &Availability::default()).attempts.len(), 5);
        const { assert!(MAX_ATTEMPTS >= 5) };
    }

    #[test]
    fn user_home_validation() {
        let dir = tempfile::tempdir().unwrap();
        let ok = dir.path().join("work-claude");
        std::fs::create_dir(&ok).unwrap();
        assert_eq!(
            validate_user_home(ok.to_str().unwrap(), Engine::Claude, &[]).unwrap(),
            ok.canonicalize().unwrap()
        );
        assert!(validate_user_home("relative/dir", Engine::Claude, &[]).is_err());
        assert!(validate_user_home(
            dir.path().join("missing").to_str().unwrap(),
            Engine::Claude,
            &[]
        )
        .is_err());
        let file = dir.path().join("file");
        std::fs::write(&file, "x").unwrap();
        assert!(validate_user_home(file.to_str().unwrap(), Engine::Claude, &[]).is_err());
        #[cfg(unix)]
        {
            let link = dir.path().join("link");
            std::os::unix::fs::symlink(&ok, &link).unwrap();
            assert!(validate_user_home(link.to_str().unwrap(), Engine::Claude, &[]).is_err());
        }
        assert!(validate_user_home("/", Engine::Claude, &[]).is_err());
        if let Some(home) = dirs::home_dir() {
            assert!(validate_user_home(home.to_str().unwrap(), Engine::Claude, &[]).is_err());
        }
        let taken = vec![ok.to_string_lossy().into_owned()];
        assert!(validate_user_home(ok.to_str().unwrap(), Engine::Claude, &taken).is_err());
    }

    #[test]
    fn name_and_key_validation() {
        assert_eq!(clean_name("  Work  ").unwrap(), "Work");
        assert!(clean_name("").is_err());
        assert!(clean_name(&"x".repeat(41)).is_err());
        assert!(clean_name("a\nb").is_err());
        assert!(clean_key("sk-ant-1234567890").is_ok());
        assert!(clean_key("short").is_err());
        assert!(clean_key("has space inside").is_err());
        assert!(format!("{:?}", clean_key("sk-ant-1234567890").unwrap()).contains("redacted"));
    }
}
