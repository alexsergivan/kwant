//! Hardening for Claude turns in Mac "local" mode (0.8.0 F, item 079-mac-local-hardening;
//! agents-in-vm design.md Q6). Without the VM, Claude runs on the Mac itself, so a bot talked into
//! it by a web page, a file or another bot could `cat` a sign-in or plant a hook in
//! `~/.claude/settings.json`. This adds one `--settings` layer that:
//!
//! - denies Claude's file tools (Read, Grep, Glob, Edit, Write) the sign-in folders and kwnt's
//!   secrets: `Read(…)` and `Edit(…)` deny rules;
//! - turns on Claude's own Seatbelt sandbox for Bash with the same paths in
//!   `sandbox.filesystem.denyRead` and `denyWrite`, so `cat ~/.claude/.credentials.json` or a
//!   script reading the Keychain *files* fails at the OS level (items reached through macOS's
//!   security service, as `security find-generic-password` does, are not covered: the Seatbelt
//!   profile lets that service through; `.github/workflows/mac-local.yml` reports which);
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

/// Turns the Seatbelt sandbox on when `1`, `on`, `true` or `yes`. The deny rules apply either way.
pub const SEATBELT_ENV: &str = "KWNT_MAC_SEATBELT";

/// The places a bot's tools never read or write, for one turn.
#[derive(Debug, Clone, PartialEq)]
pub struct Places {
    pub home: PathBuf,
    pub data_dir: PathBuf,
    /// The profile's own sign-in home (`CLAUDE_CONFIG_DIR`), when the turn runs on one.
    pub profile_home: Option<PathBuf>,
}

impl Places {
    /// This computer's: `$HOME`, kwnt's data folder (`KWNT_DATA_DIR`, else `~/.kwnt`) and the
    /// profile's home. `None` without a home folder.
    pub fn here(profile_home: Option<PathBuf>) -> Option<Self> {
        let home = dirs::home_dir()?;
        let data_dir = std::env::var_os("KWNT_DATA_DIR")
            .filter(|d| !d.is_empty())
            .map(PathBuf::from)
            .unwrap_or_else(crate::config::default_data_dir);
        Some(Self { home, data_dir, profile_home })
    }

    /// Folders (`true`) and files (`false`) to deny, absolute, without repeats.
    pub fn denied(&self) -> Vec<(PathBuf, bool)> {
        let h = &self.home;
        let d = &self.data_dir;
        let mut out: Vec<(PathBuf, bool)> = vec![
            (h.join(".claude"), true),
            (h.join(".codex"), true),
            (h.join(".grok"), true),
            // The login and other keychains' files. Reaching the Keychain through its service
            // (`security find-generic-password`) is Seatbelt's mach-lookup rule, not this one
            (h.join("Library/Keychains"), true),
            (d.join("profiles"), true),
            (d.join(crate::sandbox::credentials::VAULT_DIR), true),
            (d.join(crate::sandbox::credentials::KEY_FILE), false),
            (d.join("api-token"), false),
            (d.join("kwnt.db"), false),
            (d.join("kwnt.db-wal"), false),
        ];
        // The CLIs' own homes when moved with their variables
        for var in ["CLAUDE_CONFIG_DIR", "CODEX_HOME", "GROK_HOME"] {
            if let Some(p) = std::env::var_os(var).filter(|v| !v.is_empty()).map(PathBuf::from) {
                if p.is_absolute() {
                    out.push((p, true));
                }
            }
        }
        if let Some(p) = &self.profile_home {
            out.push((p.clone(), true));
        }
        let mut seen = std::collections::HashSet::new();
        out.retain(|(p, _)| seen.insert(p.clone()));
        out
    }
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
fn rule_path(p: &Path, dir: bool) -> String {
    let s = p.to_string_lossy();
    let s = s.trim_end_matches('/');
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
    for tool in ["Read", "Edit"] {
        for (p, dir) in &denied {
            deny.push(Value::from(format!("{tool}({})", rule_path(p, *dir))));
        }
    }
    let mut out = Map::new();
    out.insert("permissions".into(), json!({ "deny": deny }));
    if seatbelt {
        let paths: Vec<Value> = denied.iter().map(|(p, _)| Value::from(p.to_string_lossy().into_owned())).collect();
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
                    "denyRead": paths,
                    "denyWrite": paths,
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
            "Edit(//Users/o/.claude/**)",
            "Edit(//Users/o/.kwnt/api-token)",
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
        for want in ["/Users/o/.claude", "/Users/o/.codex", "/Users/o/.grok", "/Users/o/Library/Keychains", "/Users/o/.kwnt/kwnt-vault.key"] {
            assert!(read.contains(&want.to_string()), "{want} missing from {read:?}");
        }
        assert_eq!(sb["filesystem"]["denyWrite"], sb["filesystem"]["denyRead"]);
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
        assert_eq!(rule_path(Path::new("/Users/o/.claude/"), true), "//Users/o/.claude/**");
        assert_eq!(rule_path(Path::new("/Users/o/.kwnt/api-token"), false), "//Users/o/.kwnt/api-token");
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
