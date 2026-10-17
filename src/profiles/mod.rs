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

/// Most profiles one turn may try.
pub const MAX_ATTEMPTS: usize = 3;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Engine {
    Claude,
    Codex,
    Grok,
}

impl Engine {
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "claude" => Some(Engine::Claude),
            "codex" => Some(Engine::Codex),
            "grok" => Some(Engine::Grok),
            _ => None,
        }
    }

    /// The engine a routed model id runs on, matching how drivers pick a CLI or API.
    /// Models of other providers (Ollama) have no profiles.
    pub fn from_model(model: &str) -> Option<Self> {
        if model.contains("claude") {
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
        }
    }

    /// Env variable that points the CLI at a config dir holding its login.
    pub fn home_env(self) -> &'static str {
        match self {
            Engine::Claude => "CLAUDE_CONFIG_DIR",
            Engine::Codex => "CODEX_HOME",
            Engine::Grok => "GROK_HOME",
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
            ],
            Engine::Codex => &["OPENAI_API_KEY", "CODEX_API_KEY"],
            Engine::Grok => &["XAI_API_KEY", "GROK_AUTH_PATH"],
        }
    }

    /// The Settings key the system profile uses, as before profiles.
    pub fn key_setting(self) -> &'static str {
        match self {
            Engine::Claude => "anthropic_api_key",
            Engine::Codex => "openai_api_key",
            Engine::Grok => "xai_api_key",
        }
    }

    /// Model id handed to the API driver, which picks the provider endpoint from it.
    pub fn api_model(self) -> &'static str {
        match self {
            Engine::Claude => "claude",
            Engine::Codex => "codex",
            Engine::Grok => "grok",
        }
    }

    /// The CLI's own config dir when kwnt doesn't set one: what the system profile uses.
    pub fn system_home(self) -> Option<PathBuf> {
        let from_env = std::env::var_os(self.home_env())
            .filter(|v| !v.is_empty())
            .map(PathBuf::from);
        from_env.or_else(|| {
            dirs::home_dir().map(|h| {
                h.join(match self {
                    Engine::Claude => ".claude",
                    Engine::Codex => ".codex",
                    Engine::Grok => ".grok",
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
}

impl CliEnv {
    pub fn for_home(engine: Engine, home: &Path) -> Self {
        Self {
            set: vec![(engine.home_env().to_string(), home.as_os_str().to_owned())],
            remove: engine
                .credential_env()
                .iter()
                .map(|s| s.to_string())
                .collect(),
        }
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

    fn usable(&self, p: &LlmProfile, now: DateTime<Utc>) -> bool {
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
        let env = CliEnv::for_home(Engine::Codex, home);
        assert_eq!(env.set[0].0, "CODEX_HOME");
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
        let profiles: Vec<_> = (0..6)
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
