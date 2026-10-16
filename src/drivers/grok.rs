//! Isolation for grok CLI runs. grok loads MCP servers, plugins and hooks from its own home
//! (`$GROK_HOME`, default `~/.grok`) and imports Claude's and Cursor's servers and hooks, so
//! headless runs would start the user's own servers with no approval card. Every run gets a
//! fresh private `GROK_HOME` holding only the user's model settings; the login is read in place
//! through `GROK_AUTH_PATH`. The directory is removed when the run ends, including on cancel.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::Duration;

/// Env switches that stop grok importing Claude/Cursor servers and hooks, and managed servers.
pub const ISOLATION_ENV: [(&str, &str); 6] = [
    ("GROK_CLAUDE_MCPS_ENABLED", "0"),
    ("GROK_CLAUDE_HOOKS_ENABLED", "0"),
    ("GROK_CURSOR_MCPS_ENABLED", "0"),
    ("GROK_CURSOR_HOOKS_ENABLED", "0"),
    ("GROK_MANAGED_MCPS_ENABLED", "0"),
    ("GROK_DISABLE_AUTOUPDATER", "1"),
];

/// Inherited env grok would read config from instead of the temp home.
pub const REMOVED_ENV: [&str; 1] = ["GROK_CONFIG"];

/// Tables copied from the user's `config.toml`: the default model and model definitions.
const KEPT_TABLES: [&str; 2] = ["models", "model"];

const DIR_PREFIX: &str = "kwnt-grok-";

/// Left behind only if kwnt itself was killed mid-run; no turn lasts this long.
const STALE_AFTER: Duration = Duration::from_secs(24 * 60 * 60);

/// The user's real grok home: `$GROK_HOME`, else `~/.grok`.
pub fn user_grok_home() -> Option<PathBuf> {
    match std::env::var_os("GROK_HOME") {
        Some(dir) if !dir.is_empty() => Some(PathBuf::from(dir)),
        _ => dirs::home_dir().map(|h| h.join(".grok")),
    }
}

/// The parts of the user's `config.toml` a kwnt run may use. Servers, plugins, hooks,
/// permissions and everything else are dropped. Unreadable config gives an empty table.
pub fn model_settings(user_config: &str) -> toml::Table {
    let parsed: toml::Table = match user_config.parse() {
        Ok(t) => t,
        Err(_) => {
            tracing::warn!(
                "could not parse the user's grok config.toml; using grok's default model settings"
            );
            return toml::Table::new();
        }
    };
    parsed
        .into_iter()
        .filter(|(k, _)| KEPT_TABLES.contains(&k.as_str()))
        .collect()
}

/// A private, per-run `GROK_HOME`. Dropping it removes the directory.
#[derive(Debug)]
pub struct GrokHome {
    pub path: PathBuf,
}

impl Drop for GrokHome {
    fn drop(&mut self) {
        if let Err(e) = std::fs::remove_dir_all(&self.path) {
            if e.kind() != std::io::ErrorKind::NotFound {
                tracing::warn!(error = %e, dir = %self.path.display(), "could not remove grok temp home");
            }
        }
    }
}

fn write_private(path: &Path, contents: &[u8]) -> std::io::Result<()> {
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(path)?;
    file.write_all(contents)?;
    file.sync_all()
}

impl GrokHome {
    /// Creates `<base>/kwnt-grok-<uuid>` (0700) with `config` as its `config.toml` (0600).
    pub fn create(base: &Path, config: &toml::Table) -> std::io::Result<Self> {
        sweep_stale(base);
        let path = base.join(format!("{DIR_PREFIX}{}", uuid::Uuid::new_v4().simple()));
        let mut builder = std::fs::DirBuilder::new();
        #[cfg(unix)]
        {
            use std::os::unix::fs::DirBuilderExt;
            builder.mode(0o700);
        }
        builder.create(&path)?;
        let home = Self { path };
        let text = toml::to_string(config)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
        write_private(&home.path.join("config.toml"), text.as_bytes())?;
        Ok(home)
    }

    /// Fallback login: a 0600 copy of the user's `auth.json`, removed with the home.
    pub fn copy_auth(&self, auth: &Path) -> std::io::Result<()> {
        write_private(&self.path.join("auth.json"), &std::fs::read(auth)?)
    }
}

/// Removes temp homes left by a kwnt that was killed before it could clean up.
fn sweep_stale(base: &Path) {
    let Ok(entries) = std::fs::read_dir(base) else {
        return;
    };
    for entry in entries.flatten() {
        let stale = entry
            .file_name()
            .to_str()
            .is_some_and(|n| n.starts_with(DIR_PREFIX))
            && entry
                .metadata()
                .ok()
                .and_then(|m| m.modified().ok())
                .and_then(|t| t.elapsed().ok())
                .is_some_and(|age| age > STALE_AFTER);
        if stale {
            let _ = std::fs::remove_dir_all(entry.path());
        }
    }
}

/// grok's message when no login was found; the run did nothing, so it can be retried.
pub fn not_signed_in(stderr: &str) -> bool {
    stderr.contains("Not signed in")
}

#[cfg(test)]
mod tests {
    use super::*;

    const USER_CONFIG: &str = r#"
[models]
default = "mock"

[model.mock]
model = "mock-model"
base_url = "http://127.0.0.1:1/v1"
api_key = "user-key"

[mcp_servers.grokuser]
command = "/bin/evil"

[hooks]
x = 1

[plugins.p]
enabled = true

[permission]
allow = ["Bash(*)"]
"#;

    #[test]
    fn keeps_only_model_settings() {
        let kept = model_settings(USER_CONFIG);
        assert_eq!(kept.keys().collect::<Vec<_>>(), ["model", "models"]);
        assert_eq!(kept["models"]["default"].as_str(), Some("mock"));
        assert_eq!(
            kept["model"]["mock"]["base_url"].as_str(),
            Some("http://127.0.0.1:1/v1")
        );
        assert!(model_settings("not [toml").is_empty());
        assert!(model_settings("").is_empty());
    }

    #[test]
    fn temp_home_is_private_and_removed_on_drop() {
        let base = tempfile::tempdir().unwrap();
        let auth = base.path().join("real-auth.json");
        std::fs::write(&auth, "{}").unwrap();
        let home = GrokHome::create(base.path(), &model_settings(USER_CONFIG)).unwrap();
        let path = home.path.clone();
        assert!(path
            .file_name()
            .unwrap()
            .to_str()
            .unwrap()
            .starts_with(DIR_PREFIX));
        let written = std::fs::read_to_string(path.join("config.toml")).unwrap();
        assert!(
            written.contains("mock-model")
                && !written.contains("grokuser")
                && !written.contains("hooks"),
            "{written}"
        );
        home.copy_auth(&auth).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = |p: &Path| std::fs::metadata(p).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode(&path), 0o700);
            assert_eq!(mode(&path.join("config.toml")), 0o600);
            assert_eq!(mode(&path.join("auth.json")), 0o600);
        }
        drop(home);
        assert!(!path.exists());
        assert!(auth.exists(), "the user's own file is untouched");
    }

    #[test]
    fn sweeps_only_stale_kwnt_homes() {
        let base = tempfile::tempdir().unwrap();
        let old = base.path().join(format!("{DIR_PREFIX}old"));
        let other = base.path().join("someone-else");
        std::fs::create_dir(&old).unwrap();
        std::fs::create_dir(&other).unwrap();
        let past = std::time::SystemTime::now() - STALE_AFTER - Duration::from_secs(60);
        std::fs::File::open(&old)
            .unwrap()
            .set_modified(past)
            .unwrap();
        let fresh = GrokHome::create(base.path(), &toml::Table::new()).unwrap();
        assert!(!old.exists() && other.exists() && fresh.path.exists());
    }
}
