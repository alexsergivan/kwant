//! Whether each profile is signed in, checked with the engine CLI's own status command and
//! cached so the profile list stays fast and turns never wait on a check.

use super::login::LoginManager;
use super::{managed_home, Engine};
use crate::db::models::LlmProfile;
use serde::Serialize;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// How long a sign-in check is reused.
pub const STATUS_TTL: Duration = Duration::from_secs(60);
/// Longest a single check may take.
const PROBE_TIMEOUT: Duration = Duration::from_secs(8);

#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct ProfileStatus {
    /// `None` when it couldn't be told (CLI not installed, check timed out).
    pub signed_in: Option<bool>,
    /// The account the CLI reports (email or sign-in method), or "API key".
    pub account_label: Option<String>,
}

/// Profile homes, sign-in checks and logins.
pub struct ProfileManager {
    pub data_dir: PathBuf,
    pub logins: LoginManager,
    cache: Mutex<HashMap<String, (Instant, ProfileStatus)>>,
}

impl ProfileManager {
    pub fn new(data_dir: PathBuf) -> Self {
        Self {
            data_dir,
            logins: LoginManager::default(),
            cache: Mutex::new(HashMap::new()),
        }
    }

    /// The config dir a `cli_home` profile runs with.
    pub fn home_of(&self, p: &LlmProfile) -> Option<PathBuf> {
        (p.kind == "cli_home").then(|| {
            p.home_dir
                .as_ref()
                .map(PathBuf::from)
                .unwrap_or_else(|| managed_home(&self.data_dir, &p.id))
        })
    }

    pub fn set_status(&self, id: &str, status: ProfileStatus) {
        self.cache
            .lock()
            .unwrap()
            .insert(id.to_string(), (Instant::now(), status));
    }

    pub fn invalidate(&self, id: &str) {
        self.cache.lock().unwrap().remove(id);
    }

    /// Profiles whose last check said signed out, for failover to skip.
    pub fn signed_out(&self) -> HashSet<String> {
        self.cache
            .lock()
            .unwrap()
            .iter()
            .filter(|(_, (_, s))| s.signed_in == Some(false))
            .map(|(id, _)| id.clone())
            .collect()
    }

    /// Profiles whose last check said signed in, for Engines to call ready.
    pub fn signed_in(&self) -> HashSet<String> {
        self.cache
            .lock()
            .unwrap()
            .iter()
            .filter(|(_, (_, s))| s.signed_in == Some(true))
            .map(|(id, _)| id.clone())
            .collect()
    }

    /// Sign-in status of every profile; checks run concurrently, only for stale entries.
    /// `with_key`: profiles (and system profiles whose engine has a Settings or env key) that
    /// count as signed in through an API key.
    pub async fn statuses(
        &self,
        profiles: &[LlmProfile],
        with_key: &HashSet<String>,
    ) -> HashMap<String, ProfileStatus> {
        let mut out = HashMap::new();
        let mut checks = Vec::new();
        {
            let cache = self.cache.lock().unwrap();
            for p in profiles {
                if p.kind == "api_key" || (p.kind == "system" && with_key.contains(&p.id)) {
                    let has = with_key.contains(&p.id);
                    out.insert(
                        p.id.clone(),
                        ProfileStatus {
                            signed_in: Some(has),
                            account_label: has.then(|| "API key".to_string()),
                        },
                    );
                    continue;
                }
                match cache.get(&p.id) {
                    Some((at, s)) if at.elapsed() < STATUS_TTL => {
                        out.insert(p.id.clone(), s.clone());
                    }
                    _ => {
                        if let Some(engine) = Engine::parse(&p.engine) {
                            checks.push((p.id.clone(), engine, self.home_of(p)));
                        }
                    }
                }
            }
        }
        let results =
            futures::future::join_all(checks.into_iter().map(|(id, engine, home)| async move {
                let status = probe(engine, home.as_deref()).await;
                (id, status)
            }))
            .await;
        for (id, status) in results {
            self.set_status(&id, status.clone());
            out.insert(id, status);
        }
        out
    }
}

/// Asks the engine's CLI whether it's signed in, with `home` as its config dir (`None`: the
/// CLI's own default, i.e. the system profile).
pub async fn probe(engine: Engine, home: Option<&Path>) -> ProfileStatus {
    if engine == Engine::Grok {
        // grok has no status command; a login is its auth file
        let file = match home {
            Some(h) => Some(h.join("auth.json")),
            None => std::env::var_os("GROK_AUTH_PATH")
                .filter(|p| !p.is_empty())
                .map(PathBuf::from)
                .or_else(|| engine.system_home().map(|h| h.join("auth.json"))),
        };
        let signed_in = file.is_some_and(|f| f.is_file());
        return ProfileStatus {
            signed_in: Some(signed_in),
            account_label: signed_in.then(|| "grok login".into()),
        };
    }
    let Some(bin) = crate::drivers::find_cli_binary(engine.as_str()) else {
        return ProfileStatus {
            signed_in: None,
            account_label: Some(format!("{} CLI not installed", engine.as_str())),
        };
    };
    let mut cmd = tokio::process::Command::new(bin);
    match engine {
        Engine::Claude => cmd.args(["auth", "status", "--json"]),
        _ => cmd.args(["login", "status"]),
    };
    if let Some(home) = home {
        super::CliEnv::for_home(engine, home).apply(&mut cmd);
    }
    cmd.env("NO_COLOR", "1")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    let Ok(Ok(out)) = tokio::time::timeout(PROBE_TIMEOUT, cmd.output()).await else {
        return ProfileStatus::default();
    };
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    match engine {
        Engine::Claude => claude_status(&stdout),
        _ => codex_status(out.status.success(), &stdout, &stderr),
    }
}

pub(crate) fn claude_status(stdout: &str) -> ProfileStatus {
    let Ok(v) = serde_json::from_str::<serde_json::Value>(stdout.trim()) else {
        return ProfileStatus::default();
    };
    let signed_in = v["loggedIn"].as_bool();
    // An auth method like "third_party" isn't an account: name the provider it runs on instead
    let label = ["email", "emailAddress", "account"]
        .iter()
        .find_map(|k| v[*k].as_str().filter(|s| !s.is_empty()))
        .map(|s| s.chars().take(80).collect())
        .or_else(|| (v["authMethod"] == "third_party").then(|| third_party_name(v["apiProvider"].as_str())));
    ProfileStatus {
        signed_in,
        account_label: if signed_in == Some(true) { label } else { None },
    }
}

/// What Claude Code runs on when it's set up for another provider instead of a Claude login.
fn third_party_name(provider: Option<&str>) -> String {
    match provider {
        Some("bedrock") => "Amazon Bedrock",
        Some("vertex") => "Google Vertex AI",
        Some("foundry") => "Microsoft Foundry",
        _ => "a third-party provider",
    }
    .into()
}

pub(crate) fn codex_status(success: bool, stdout: &str, stderr: &str) -> ProfileStatus {
    let text = format!("{stdout}\n{stderr}");
    let line = text
        .lines()
        .map(|l| super::login::strip_ansi(l).trim().to_string())
        .find(|l| !l.is_empty())
        .unwrap_or_default();
    let signed_in = success && !line.to_lowercase().contains("not logged in");
    ProfileStatus {
        signed_in: Some(signed_in),
        account_label: signed_in
            .then(|| super::classify::redact_detail(&line, 60))
            .filter(|s| !s.is_empty()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_cli_status_output() {
        let s =
            claude_status(r#"{"loggedIn":true,"authMethod":"claude.ai","email":"me@example.com"}"#);
        assert_eq!(
            s,
            ProfileStatus {
                signed_in: Some(true),
                account_label: Some("me@example.com".into())
            }
        );
        let s = claude_status(r#"{"loggedIn":false,"authMethod":"none"}"#);
        assert_eq!(
            s,
            ProfileStatus {
                signed_in: Some(false),
                account_label: None
            }
        );
        assert_eq!(claude_status("garbage"), ProfileStatus::default());
        // Set up for Bedrock (as this machine's env can do): the provider, never "third_party"
        let s = claude_status(r#"{"loggedIn":true,"authMethod":"third_party","apiProvider":"bedrock"}"#);
        assert_eq!(s.account_label.as_deref(), Some("Amazon Bedrock"));
        let s = claude_status(r#"{"loggedIn":true,"authMethod":"claude.ai"}"#);
        assert_eq!(s.account_label, None);
        let s = codex_status(true, "Logged in using ChatGPT\n", "");
        assert_eq!(s.signed_in, Some(true));
        assert_eq!(s.account_label.as_deref(), Some("Logged in using ChatGPT"));
        assert_eq!(
            codex_status(false, "", "Not logged in\n").signed_in,
            Some(false)
        );
    }

    #[tokio::test]
    async fn grok_status_is_its_auth_file() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(
            probe(Engine::Grok, Some(dir.path())).await.signed_in,
            Some(false)
        );
        std::fs::write(dir.path().join("auth.json"), "{}").unwrap();
        assert_eq!(
            probe(Engine::Grok, Some(dir.path())).await.signed_in,
            Some(true)
        );
    }

    #[test]
    fn signed_out_cache() {
        let m = ProfileManager::new(PathBuf::from("/tmp/x"));
        m.set_status(
            "a",
            ProfileStatus {
                signed_in: Some(false),
                account_label: None,
            },
        );
        m.set_status(
            "b",
            ProfileStatus {
                signed_in: Some(true),
                account_label: None,
            },
        );
        m.set_status("c", ProfileStatus::default());
        assert_eq!(m.signed_out(), HashSet::from(["a".to_string()]));
        m.invalidate("a");
        assert!(m.signed_out().is_empty());
    }
}
