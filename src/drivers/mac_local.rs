//! Hardening for Claude turns in Mac "local" mode (0.8.0 F, item 079-mac-local-hardening;
//! agents-in-vm design.md Q6). Without the VM, Claude runs on the Mac itself, so a bot talked into
//! it by a web page, a file or another bot could `cat` a sign-in or plant a hook in
//! `~/.claude/settings.json`. This adds one `--settings` layer that:
//!
//! - denies Claude's file tools (Read, Grep, Glob, Edit, Write) the sign-in folders and kwnt's
//!   secrets (`Read(…)` and `Edit(…)` deny rules), and denies them changes to what kwnt loads or
//!   runs at its next start (`config.toml`, `laya/`, `bin/`: `Edit(…)` only). **Without
//!   `KWNT_MAC_SEATBELT` that is all**: the rules cover Claude's file tools only, and a shell
//!   command the person approves can still read these places and Keychain items;
//! - with `KWNT_MAC_SEATBELT=1`, turns on Claude's own Seatbelt sandbox for Bash with the same
//!   paths in `sandbox.filesystem.denyRead` and `denyWrite`, so `cat ~/.claude/.credentials.json`
//!   or a script reading the Keychain *files* fails at the OS level. Keychain items reached
//!   through macOS's security service (`security find-generic-password`) are not proven safe: on
//!   CI's Mac runner a sandboxed lookup couldn't read the probe item, but that runner's keychain
//!   isn't a person's login or iCloud keychain, so it's untested on a real Mac
//!   (`.github/workflows/mac-local.yml`);
//! - turns off Claude's auto memory (`autoMemoryEnabled: false`): it writes inside the denied
//!   `~/.claude` (or the profile's home), so every try would fail or ask;
//! - keeps kwnt in charge of every command: `autoAllowBashIfSandboxed: false` (a sandboxed
//!   command still asks kwnt, as today) and `allowUnsandboxedCommands: false` (the bot can't opt
//!   a command out of the sandbox).
//!
//! What it does **not** do (design.md Q6): the Claude process itself still reads its own sign-in
//! and the Keychain, as it must to run. This protects against a tricked bot's *tools*, not
//! against the CLI. The VM ("Run Claude inside the VM") is the protection for that.
//!
//! Writes stay as broad as today (`allowWrite: ["~"]`, plus Claude's own denyWrite list for shell
//! start-up files and `~/.claude`).
//!
//! **The Seatbelt half is opt-in for now** (`KWNT_MAC_SEATBELT=1`); the deny rules for the file
//! tools are always on. With the sandbox on, Bash's network goes through Claude's proxy, which asks
//! before each new domain (kwnt shows that as an approval card; `localhost` and `127.0.0.1`, kwnt's
//! own API, are allowed), and Unix sockets other than `$SSH_AUTH_SOCK` are blocked (Docker). Writes
//! outside the home folder fail. None
//! of that can be tried without a real Mac (CI has no signed-in Claude), so it stays off until it
//! has been tried on one.

use serde_json::{json, Map, Value};
use std::path::{Path, PathBuf};
use std::sync::RwLock;

/// Turns the Seatbelt sandbox on when `1`, `on`, `true` or `yes`. The deny rules apply either way.
pub const SEATBELT_ENV: &str = "KWNT_MAC_SEATBELT";

/// The running kwnt's data folder (`config.data_dir`: `--data-dir`, config.toml's `data_dir`,
/// `KWNT_DATA_DIR` or `~/.kwnt`), set by [`crate::serve::run`]. Replaceable, not a `OnceLock`:
/// the Mac app can start the server again in the same process.
static DATA_DIR: RwLock<Option<PathBuf>> = RwLock::new(None);

/// Records the data folder the server runs on, so every rule names the folder actually in use.
pub fn set_data_dir(dir: &Path) {
    let dir = std::path::absolute(dir).unwrap_or_else(|_| dir.to_path_buf());
    if let Ok(mut d) = DATA_DIR.write() {
        *d = Some(dir);
    }
}

/// The data folder for the rules: the running server's ([`set_data_dir`]) first; without a server
/// (the CI probe, a test) `KWNT_DATA_DIR`, else `~/.kwnt`, as [`crate::config::Config::load`] does.
fn data_dir_for(running: Option<PathBuf>, env: Option<std::ffi::OsString>) -> PathBuf {
    running
        .or_else(|| env.filter(|d| !d.is_empty()).map(PathBuf::from))
        .unwrap_or_else(crate::config::default_data_dir)
}

/// One denied place: an absolute Mac path, a folder (`dir`) or a file, and whether reading it is
/// denied too (`read`) or only changing it (kwnt's own config and the programs it starts).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Denied {
    pub path: String,
    pub dir: bool,
    pub read: bool,
}

/// The places a bot's tools never read or write, for one turn.
#[derive(Debug, Clone, PartialEq)]
pub struct Places {
    pub home: PathBuf,
    pub data_dir: PathBuf,
    /// The profile's own sign-in home (`CLAUDE_CONFIG_DIR`), when the turn runs on one.
    pub profile_home: Option<PathBuf>,
}

impl Places {
    /// This computer's: `$HOME`, the running kwnt's data folder (see [`data_dir_for`]) and the
    /// profile's home. `None` without a home folder.
    pub fn here(profile_home: Option<PathBuf>) -> Option<Self> {
        let home = dirs::home_dir()?;
        let running = DATA_DIR.read().ok().and_then(|d| d.clone());
        let data_dir = data_dir_for(running, std::env::var_os("KWNT_DATA_DIR"));
        Some(Self { home, data_dir, profile_home })
    }

    /// The places to deny, absolute, without repeats. Mac paths, joined with `/` as strings:
    /// `Path::join` would use `\` when this is compiled for Windows (CI's rust (windows) runs
    /// these tests), and the rules are only ever read on a Mac. Where a place (or the folder it
    /// is in) is reached through a link, its resolved spelling is listed too: Seatbelt matches
    /// real paths, so `/var/…` alone wouldn't cover `/private/var/…`.
    pub fn denied(&self) -> Vec<Denied> {
        let h = mac_path(&self.home);
        let d = mac_path(&self.data_dir);
        let secret = |path: String, dir: bool| Denied { path, dir, read: true };
        let no_edit = |path: String, dir: bool| Denied { path, dir, read: false };
        let mut list: Vec<Denied> = vec![
            secret(mac_join(&h, ".claude"), true),
            // Claude's primaryApiKey and MCP servers' env and headers
            secret(mac_join(&h, ".claude.json"), false),
            secret(mac_join(&h, ".codex"), true),
            secret(mac_join(&h, ".grok"), true),
            // The login and other keychains' files. Reaching the Keychain through its service
            // (`security find-generic-password`) is Seatbelt's mach-lookup rule, not this one
            secret(mac_join(&h, "Library/Keychains"), true),
            secret(mac_join(&d, "profiles"), true),
            secret(mac_join(&d, crate::sandbox::credentials::VAULT_DIR), true),
            secret(mac_join(&d, crate::sandbox::credentials::KEY_FILE), false),
            secret(mac_join(&d, "api-token"), false),
            secret(mac_join(&d, "kwnt.db"), false),
            secret(mac_join(&d, "kwnt.db-wal"), false),
            secret(mac_join(&d, "kwnt.db-shm"), false),
            // Copies of the database, with its API keys and OAuth tokens (db/spaces.rs)
            secret(mac_join(&d, "backups"), true),
            // Each turn's claude-<uuid>.json, with MCP headers and env (mcp/cli_config.rs)
            secret(mac_join(&d, "mcp"), true),
            // run/mcp-cwd/<turn token> (server/mcp_runtime.rs)
            secret(mac_join(&d, "run"), true),
            // What kwnt loads or runs at its next start: readable, never changed by a bot
            no_edit(mac_join(&d, "config.toml"), false),
            no_edit(mac_join(&d, "laya"), true),
            no_edit(mac_join(&d, "bin"), true),
        ];
        // The CLIs' own homes when moved with their variables (absolute Mac paths only)
        for var in ["CLAUDE_CONFIG_DIR", "CODEX_HOME", "GROK_HOME"] {
            if let Some(p) = std::env::var_os(var).filter(|v| !v.is_empty()) {
                let p = mac_path(Path::new(&p));
                if p.starts_with('/') {
                    list.push(secret(p, true));
                }
            }
        }
        if let Some(p) = &self.profile_home {
            list.push(secret(mac_path(p), true));
        }
        // Only a Mac reads these rules; elsewhere a resolved path wouldn't be a Mac path
        if cfg!(unix) {
            let resolved: Vec<Denied> = list
                .iter()
                .filter_map(|x| resolved(&x.path).map(|path| Denied { path, ..x.clone() }))
                .collect();
            list.extend(resolved);
        }
        let mut out: Vec<Denied> = Vec::new();
        for x in list {
            match out.iter_mut().find(|y| y.path == x.path) {
                Some(y) => y.read |= x.read,
                None => out.push(x),
            }
        }
        out
    }
}

/// `p`'s spelling with links resolved, when it differs: the nearest part of it that exists is
/// resolved and the rest appended (a denied file need not exist yet). `None` when nothing
/// resolves differently.
fn resolved(p: &str) -> Option<String> {
    let mut base = Path::new(p);
    let mut rest: Vec<String> = Vec::new();
    let real = loop {
        if let Ok(r) = std::fs::canonicalize(base) {
            break mac_path(&r);
        }
        rest.push(base.file_name()?.to_str()?.to_string());
        base = base.parent()?;
    };
    let full = rest.iter().rev().fold(real, |acc, c| mac_join(&acc, c));
    (full.starts_with('/') && full != p).then_some(full)
}

/// A path as a Mac path string, without a trailing `/` (but `/` itself stays `/`).
fn mac_path(p: &Path) -> String {
    let s = p.to_string_lossy();
    let t = s.trim_end_matches('/');
    if t.is_empty() && s.starts_with('/') {
        "/".into()
    } else {
        t.into()
    }
}

/// `base/rel` with `/`, whatever OS this is compiled for.
fn mac_join(base: &str, rel: &str) -> String {
    format!("{}/{rel}", base.trim_end_matches('/'))
}

/// Whether a turn gets [`settings`]: Claude, on a Mac, on this computer (not in the VM).
pub fn applies(is_mac: bool, tool_kind: &str, isolated: bool) -> bool {
    is_mac && tool_kind == "claude" && !isolated
}

/// Whether the Seatbelt half is on, given `KWNT_MAC_SEATBELT`'s value (opt-in, see the module doc).
pub fn seatbelt_on(env: Option<&str>) -> bool {
    matches!(env.map(|v| v.trim().to_ascii_lowercase()).as_deref(), Some("1" | "on" | "true" | "yes"))
}

/// A permission-rule path: `//abs/path` is absolute in Claude's rule syntax (a single `/` would be
/// relative to the settings file).
fn rule_path(p: &str, dir: bool) -> String {
    let s = p.trim_end_matches('/');
    if dir {
        format!("/{s}/**")
    } else {
        format!("/{s}")
    }
}

/// The `--settings` object for one turn. `ssh_auth_sock`: the agent socket Bash may still reach
/// (git over ssh) when the sandbox is on.
pub fn settings(places: &Places, seatbelt: bool, ssh_auth_sock: Option<&str>) -> Value {
    let denied = places.denied();
    let mut deny = Vec::new();
    for x in denied.iter().filter(|x| x.read) {
        deny.push(Value::from(format!("Read({})", rule_path(&x.path, x.dir))));
    }
    for x in &denied {
        deny.push(Value::from(format!("Edit({})", rule_path(&x.path, x.dir))));
    }
    let mut out = Map::new();
    out.insert("permissions".into(), json!({ "deny": deny }));
    // Auto memory writes under <claude home>/projects/*/memory, inside the denied ~/.claude (or
    // the profile's home): every try would fail or ask
    out.insert("autoMemoryEnabled".into(), Value::Bool(false));
    if seatbelt {
        let read: Vec<Value> = denied.iter().filter(|x| x.read).map(|x| Value::from(x.path.as_str())).collect();
        let write: Vec<Value> = denied.iter().map(|x| Value::from(x.path.as_str())).collect();
        out.insert(
            "sandbox".into(),
            json!({
                "enabled": true,
                // A Mac always has Seatbelt; if it can't start, Claude warns and Bash runs as
                // today rather than every turn failing
                "failIfUnavailable": false,
                "autoAllowBashIfSandboxed": false,
                "allowUnsandboxedCommands": false,
                "filesystem": {
                    "denyRead": read,
                    "denyWrite": write,
                    "allowWrite": ["~"],
                },
                "network": {
                    // kwnt's own API, which the bot's tools block tells it to call with curl
                    "allowedDomains": ["localhost", "127.0.0.1"],
                    "allowUnixSockets": ssh_auth_sock.filter(|s| s.starts_with('/')).into_iter().collect::<Vec<_>>(),
                },
            }),
        );
    }
    Value::Object(out)
}

/// Merges `extra` into the `--settings` JSON already in `args` (a profile's env overrides), or
/// appends `--settings extra`. Claude reads one `--settings`, so they must be one object.
pub fn merge_settings_arg(args: &mut Vec<String>, extra: Value) {
    if let Some(i) = args.iter().position(|a| a == "--settings") {
        if let Some(Ok(Value::Object(mut have))) = args.get(i + 1).map(|v| serde_json::from_str::<Value>(v)) {
            if let Value::Object(add) = extra {
                for (k, v) in add {
                    have.insert(k, v);
                }
            }
            args[i + 1] = Value::Object(have).to_string();
            return;
        }
    }
    args.push("--settings".into());
    args.push(extra.to_string());
}

#[cfg(test)]
mod tests {
    use super::*;

    fn places() -> Places {
        Places { home: "/Users/o".into(), data_dir: "/Users/o/.kwnt".into(), profile_home: Some("/Users/o/.kwnt/profiles/p1".into()) }
    }

    fn strs(v: &Value) -> Vec<String> {
        v.as_array().unwrap().iter().map(|x| x.as_str().unwrap().to_string()).collect()
    }

    #[test]
    fn only_claude_on_a_mac_outside_the_vm() {
        assert!(applies(true, "claude", false));
        assert!(!applies(true, "claude", true), "the VM has its own walls");
        assert!(!applies(false, "claude", false), "Seatbelt is macOS only");
        for other in ["codex", "grok", "gemini"] {
            assert!(!applies(true, other, false), "{other}");
        }
    }

    #[test]
    fn the_sign_ins_and_kwnts_secrets_are_denied_to_tools_and_bash() {
        let s = settings(&places(), true, Some("/private/tmp/launchd-1/Listeners"));
        let deny = strs(&s["permissions"]["deny"]);
        for want in [
            "Read(//Users/o/.claude/**)",
            "Read(//Users/o/.codex/**)",
            "Read(//Users/o/.grok/**)",
            "Read(//Users/o/Library/Keychains/**)",
            "Read(//Users/o/.kwnt/profiles/**)",
            "Read(//Users/o/.kwnt/kwnt-vault/**)",
            "Read(//Users/o/.kwnt/kwnt-vault.key)",
            "Read(//Users/o/.kwnt/api-token)",
            "Read(//Users/o/.kwnt/kwnt.db)",
            "Read(//Users/o/.kwnt/kwnt.db-wal)",
            "Read(//Users/o/.kwnt/kwnt.db-shm)",
            "Read(//Users/o/.claude.json)",
            "Read(//Users/o/.kwnt/backups/**)",
            "Read(//Users/o/.kwnt/mcp/**)",
            "Read(//Users/o/.kwnt/run/**)",
            "Edit(//Users/o/.claude/**)",
            "Edit(//Users/o/.claude.json)",
            "Edit(//Users/o/.kwnt/api-token)",
            "Edit(//Users/o/.kwnt/backups/**)",
        ] {
            assert!(deny.contains(&want.to_string()), "{want} missing from {deny:?}");
        }
        // The profile's home sits inside profiles/ here, but is listed too (it may live elsewhere)
        assert!(deny.contains(&"Read(//Users/o/.kwnt/profiles/p1/**)".to_string()));
        let sb = &s["sandbox"];
        assert_eq!(sb["enabled"], true);
        assert_eq!(sb["autoAllowBashIfSandboxed"], false, "kwnt still decides every command");
        assert_eq!(sb["allowUnsandboxedCommands"], false, "a bot can't take a command out of the sandbox");
        assert_eq!(sb["failIfUnavailable"], false);
        let read = strs(&sb["filesystem"]["denyRead"]);
        for want in [
            "/Users/o/.claude",
            "/Users/o/.claude.json",
            "/Users/o/.codex",
            "/Users/o/.grok",
            "/Users/o/Library/Keychains",
            "/Users/o/.kwnt/kwnt-vault.key",
            "/Users/o/.kwnt/kwnt.db-shm",
            "/Users/o/.kwnt/backups",
            "/Users/o/.kwnt/mcp",
            "/Users/o/.kwnt/run",
        ] {
            assert!(read.contains(&want.to_string()), "{want} missing from {read:?}");
        }
        // Everything unreadable is unwritable too
        let write = strs(&sb["filesystem"]["denyWrite"]);
        for p in &read {
            assert!(write.contains(p), "{p} readable-denied but writable: {write:?}");
        }
        assert_eq!(sb["filesystem"]["allowWrite"], json!(["~"]), "writes stay as broad as today");
        // The bot's own folder and kwnt's other files aren't touched
        assert!(!read.iter().any(|p| p == "/Users/o/.kwnt" || p == "/Users/o"), "{read:?}");
        assert_eq!(sb["network"]["allowedDomains"], json!(["localhost", "127.0.0.1"]), "kwnt's own API needs no card");
        assert_eq!(sb["network"]["allowUnixSockets"], json!(["/private/tmp/launchd-1/Listeners"]), "git over ssh keeps its agent");
        let s = settings(&places(), true, None);
        assert_eq!(s["sandbox"]["network"]["allowUnixSockets"], json!([]));
    }

    #[test]
    fn the_seatbelt_half_is_opt_in_and_the_rules_apply_either_way() {
        for on in ["1", "on", "TRUE", " yes "] {
            assert!(seatbelt_on(Some(on)), "{on}");
        }
        for off in [None, Some("0"), Some("off"), Some(""), Some("maybe")] {
            assert!(!seatbelt_on(off), "{off:?}");
        }
        let s = settings(&places(), false, None);
        assert!(s.get("sandbox").is_none());
        assert!(!strs(&s["permissions"]["deny"]).is_empty());
    }

    #[test]
    fn one_settings_object_with_a_profiles_overrides() {
        let mut args = vec!["--settings".to_string(), json!({ "env": { "ANTHROPIC_BASE_URL": "" } }).to_string()];
        merge_settings_arg(&mut args, settings(&places(), true, None));
        assert_eq!(args.iter().filter(|a| *a == "--settings").count(), 1, "{args:?}");
        let v: Value = serde_json::from_str(&args[1]).unwrap();
        assert_eq!(v["env"]["ANTHROPIC_BASE_URL"], "", "the profile's overrides stay");
        assert_eq!(v["sandbox"]["enabled"], true);
        // No profile: appended
        let mut args = vec!["-p".to_string(), "hi".to_string()];
        merge_settings_arg(&mut args, json!({ "permissions": {} }));
        assert_eq!(args[2], "--settings");
    }

    #[test]
    fn paths_become_absolute_rules() {
        assert_eq!(rule_path("/Users/o/.claude/", true), "//Users/o/.claude/**");
        assert_eq!(rule_path("/Users/o/.kwnt/api-token", false), "//Users/o/.kwnt/api-token");
    }

    #[test]
    fn the_rules_use_mac_slashes_whatever_os_builds_them() {
        assert_eq!(mac_join("/Users/o/", ".claude"), "/Users/o/.claude");
        assert_eq!(mac_join("/", "x"), "/x");
        assert_eq!(mac_path(Path::new("/Users/o/.kwnt/")), "/Users/o/.kwnt");
        assert_eq!(mac_path(Path::new("/")), "/");
        let s = settings(&places(), true, None);
        let all = strs(&s["permissions"]["deny"]).into_iter().chain(strs(&s["sandbox"]["filesystem"]["denyRead"]));
        for r in all {
            assert!(!r.contains('\\'), "{r}");
        }
    }

    #[test]
    fn kwnts_start_files_are_readable_but_never_changed() {
        let s = settings(&places(), true, None);
        let deny = strs(&s["permissions"]["deny"]);
        let read = strs(&s["sandbox"]["filesystem"]["denyRead"]);
        let write = strs(&s["sandbox"]["filesystem"]["denyWrite"]);
        for (path, rule) in [
            ("/Users/o/.kwnt/config.toml", "/Users/o/.kwnt/config.toml"),
            ("/Users/o/.kwnt/laya", "/Users/o/.kwnt/laya/**"),
            ("/Users/o/.kwnt/bin", "/Users/o/.kwnt/bin/**"),
        ] {
            assert!(deny.contains(&format!("Edit(/{rule})")), "Edit {rule} missing from {deny:?}");
            assert!(!deny.contains(&format!("Read(/{rule})")), "{rule} must stay readable: {deny:?}");
            assert!(write.contains(&path.to_string()), "{path} missing from denyWrite {write:?}");
            assert!(!read.contains(&path.to_string()), "{path} must stay readable in Bash: {read:?}");
        }
    }

    #[test]
    fn auto_memory_is_off_because_its_folder_is_denied() {
        for seatbelt in [false, true] {
            assert_eq!(settings(&places(), seatbelt, None)["autoMemoryEnabled"], json!(false), "seatbelt {seatbelt}");
        }
    }

    #[test]
    fn the_rules_name_the_running_servers_data_folder() {
        // A data folder from config.toml (or --data-dir), not from the environment
        let config: crate::config::Config = toml::from_str("data_dir = \"/Volumes/ext/kwnt\"").unwrap();
        let env = Some(std::ffi::OsString::from("/Users/o/.kwnt"));
        assert_eq!(data_dir_for(Some(config.data_dir.clone()), env.clone()), PathBuf::from("/Volumes/ext/kwnt"));
        // Without a running server: KWNT_DATA_DIR, else ~/.kwnt
        assert_eq!(data_dir_for(None, env), PathBuf::from("/Users/o/.kwnt"));
        assert_eq!(data_dir_for(None, Some("".into())), crate::config::default_data_dir());
        let p = Places { home: "/Users/o".into(), data_dir: config.data_dir, profile_home: None };
        let deny = strs(&settings(&p, false, None)["permissions"]["deny"]);
        for want in ["Read(//Volumes/ext/kwnt/api-token)", "Read(//Volumes/ext/kwnt/kwnt.db)", "Read(//Volumes/ext/kwnt/profiles/**)"] {
            assert!(deny.contains(&want.to_string()), "{want} missing from {deny:?}");
        }
        assert!(!deny.iter().any(|r| r.contains("/Users/o/.kwnt/")), "{deny:?}");
        // What serve::run records is what a turn's rules use
        let dir = tempfile::tempdir().unwrap();
        set_data_dir(dir.path());
        assert_eq!(Places::here(None).unwrap().data_dir, std::path::absolute(dir.path()).unwrap());
    }

    #[cfg(unix)]
    #[test]
    fn a_linked_folder_is_denied_by_its_resolved_spelling_too() {
        let t = tempfile::tempdir().unwrap();
        let real = t.path().join("real-data");
        std::fs::create_dir_all(real.join("profiles")).unwrap();
        std::fs::write(real.join("api-token"), "x").unwrap();
        let link = t.path().join("data-link");
        std::os::unix::fs::symlink(&real, &link).unwrap();
        let p = Places { home: t.path().join("home"), data_dir: link.clone(), profile_home: None };
        let denied = p.denied();
        let real = mac_path(&std::fs::canonicalize(&real).unwrap());
        let link = mac_path(&link);
        let find = |path: String| denied.iter().find(|x| x.path == path).cloned();
        for (name, dir, read) in [("api-token", false, true), ("profiles", true, true), ("kwnt.db-shm", false, true), ("config.toml", false, false)] {
            let want = Denied { path: format!("{real}/{name}"), dir, read };
            assert_eq!(find(want.path.clone()), Some(want), "resolved {name}");
            assert!(find(format!("{link}/{name}")).is_some(), "as written: {name}");
        }
        // Nothing listed twice; places not reached through a link aren't doubled
        let mut paths: Vec<&String> = denied.iter().map(|x| &x.path).collect();
        paths.sort();
        paths.dedup();
        assert_eq!(paths.len(), denied.len());
        assert_eq!(resolved("/"), None);
    }

    /// Not a check: `.github/workflows/mac-local.yml` runs it on the macOS runner to get the exact
    /// settings this computer's turn would get (Seatbelt on), then hands them to the real Claude
    /// and to sandbox-runtime's Seatbelt to see what's enforced there.
    #[test]
    #[ignore = "writes KWNT_MAC_SETTINGS_OUT for the mac-local CI probe"]
    fn write_settings_for_the_mac_ci_probe() {
        let out = std::env::var_os("KWNT_MAC_SETTINGS_OUT").expect("KWNT_MAC_SETTINGS_OUT");
        let places = Places::here(None).expect("a home folder");
        let sock = std::env::var("SSH_AUTH_SOCK").ok();
        let s = settings(&places, true, sock.as_deref());
        std::fs::write(out, serde_json::to_vec_pretty(&s).unwrap()).unwrap();
    }
}
