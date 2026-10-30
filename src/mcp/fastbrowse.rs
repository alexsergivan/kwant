//! fastbrowse (github.com/agent-labs-dev/fastbrowse) as an optional MCP server. It is a browser
//! agent with its own model key, so kwnt holds it to a few rules whatever its config says:
//!
//! - it runs on this computer's Chrome (`--local`), never the Browser Use cloud browser;
//! - it never goes through irreversible actions itself: `--allow-authorize` is refused and a
//!   call's `authorize` argument is dropped, so a run stops at `needs_confirmation`, and kwnt
//!   turns that stop into an approval card ([`confirmation`]);
//! - a task that could pay gets kwnt's payment card before it runs (`payment_floor`);
//! - every engine reaches it through kwnt's gateway, so the rules above hold for claude too.

use crate::db::models::McpServer;
use serde_json::{json, Value};
use std::path::{Path, PathBuf};

/// The model keys fastbrowse reads; one of them is enough.
pub const MODEL_KEYS: [&str; 2] = ["OPENROUTER_API_KEY", "AI_GATEWAY_API_KEY"];

/// Flags kwnt never lets a fastbrowse server start with, and why.
const REFUSED_FLAGS: [(&str, &str); 3] = [
    ("--allow-authorize", "kwnt never lets fastbrowse go through irreversible actions itself: it stops and kwnt asks you"),
    ("--cloud-profile", "fastbrowse runs on this computer's Chrome in kwnt, never a Browser Use cloud browser"),
    ("--transport", "kwnt starts fastbrowse over stdio"),
];

/// Variables that would send runs to the Browser Use cloud browser.
const REFUSED_ENV: [&str; 1] = ["BROWSER_USE_API_KEY"];

/// Where uv's installer, Homebrew and cargo put commands that a Mac app or a service's PATH
/// may not include.
const EXTRA_BIN_DIRS: [&str; 4] = ["~/.local/bin", "~/.cargo/bin", "/opt/homebrew/bin", "/usr/local/bin"];

/// The names fastbrowse looks for Chrome under (`find_chrome` in fastbrowse-cli).
const CHROME_NAMES: [&str; 6] = [
    "google-chrome-stable",
    "google-chrome",
    "chromium",
    "chromium-browser",
    "chrome",
    "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome",
];

/// A stdio server that runs fastbrowse's MCP server: `fastbrowse-mcp …`, `uvx --from fastbrowse
/// fastbrowse-mcp …`, `npx … fastbrowse-mcp`. Detected from the command, not the name, so a
/// renamed server keeps the rules.
pub fn is_fastbrowse(server: &McpServer) -> bool {
    server.transport == "stdio" && is_fastbrowse_command(server.command.as_deref(), &server.args)
}

pub fn is_fastbrowse_command(command: Option<&str>, args: &[String]) -> bool {
    command
        .into_iter()
        .chain(args.iter().map(String::as_str))
        .any(|part| {
            let base = Path::new(part).file_name().and_then(|n| n.to_str()).unwrap_or(part);
            let base = base.split('@').next().unwrap_or(base);
            base == "fastbrowse-mcp" || base == "fastbrowse"
        })
}

/// Why kwnt won't run this fastbrowse config, or `None` when it keeps to the rules.
pub fn config_problem(args: &[String], env_keys: &[&str]) -> Option<String> {
    for arg in args {
        let flag = arg.split('=').next().unwrap_or(arg);
        if let Some((_, why)) = REFUSED_FLAGS.iter().find(|(f, _)| *f == flag) {
            return Some(format!("fastbrowse can't run with {flag}: {why}."));
        }
        if flag == "--authorize" {
            return Some("fastbrowse can't run with --authorize: kwnt never lets it go through irreversible actions itself.".into());
        }
    }
    if let Some(key) = env_keys.iter().find(|k| REFUSED_ENV.contains(k)) {
        return Some(format!(
            "fastbrowse can't have {key}: in kwnt it runs on this computer's Chrome, never a Browser Use cloud browser."
        ));
    }
    if !args.iter().any(|a| a == "--local") {
        return Some("fastbrowse needs --local, so it runs on this computer's Chrome and never a cloud browser.".into());
    }
    None
}

/// What this computer lacks to run `server`, said plainly, or `None`. `path` is the PATH kwnt
/// would start it with and `home` the home folder.
pub fn missing_runtime(server: &McpServer, path: Option<&str>, home: Option<&Path>) -> Option<String> {
    let mut missing = Vec::new();
    let program = server.command.as_deref().unwrap_or("");
    if resolve_program(program, path, home).is_none() {
        let what = match program_name(program) {
            "uvx" | "uv" => format!(
                "uv, which runs fastbrowse (`{program}` was not found). Install it with \
                 `curl -LsSf https://astral.sh/uv/install.sh | sh`, or `brew install uv` on a Mac"
            ),
            "npx" | "node" | "npm" => format!(
                "Node.js, which runs fastbrowse (`{program}` was not found). Install it from nodejs.org, \
                 or `brew install node` on a Mac"
            ),
            _ => format!("`{program}`, the command this server starts. Check the path, or install fastbrowse with `uv tool install fastbrowse`"),
        };
        missing.push(what);
    }
    let chrome = server.env.get("FASTBROWSE_CHROME").map(String::as_str).filter(|c| !c.is_empty());
    let found_chrome = match chrome {
        Some(name) => resolve_program(name, path, None).is_some(),
        None => CHROME_NAMES.iter().any(|n| resolve_program(n, path, None).is_some()),
    };
    if !found_chrome {
        missing.push(match chrome {
            Some(name) => format!("Chrome at `{name}` (FASTBROWSE_CHROME), which was not found"),
            None => "Google Chrome or Chromium. Install Chrome, or set FASTBROWSE_CHROME to the browser's path".into(),
        });
    }
    let has_key = MODEL_KEYS.iter().any(|k| server.env.get(*k).is_some_and(|v| !v.trim().is_empty()))
        || server.env.contains_key("FASTBROWSE_LLM_PROVIDER");
    if !has_key {
        missing.push(
            "a model key: fastbrowse uses its own model, so add OPENROUTER_API_KEY (or AI_GATEWAY_API_KEY) \
             to this server"
                .into(),
        );
    }
    (!missing.is_empty()).then(|| format!("fastbrowse needs {}.", missing.join("; and ")))
}

/// The server as kwnt starts it: a bare `uvx`/`npx`/`fastbrowse-mcp` that isn't on PATH but is
/// in a usual install folder runs from there.
pub fn resolved(server: &McpServer, path: Option<&str>, home: Option<&Path>) -> McpServer {
    let mut out = server.clone();
    if let Some(program) = server.command.as_deref().and_then(|c| resolve_program(c, path, home)) {
        out.command = Some(program.display().to_string());
    }
    out
}

/// Checks a fastbrowse server before kwnt starts it; the error is what the person reads.
pub fn check_before_start(server: &McpServer) -> Result<McpServer, String> {
    let keys: Vec<&str> = server.env.keys().map(String::as_str).collect();
    if let Some(problem) = config_problem(&server.args, &keys) {
        return Err(problem);
    }
    let path = std::env::var("PATH").ok();
    let home = home_dir();
    if let Some(missing) = missing_runtime(server, path.as_deref(), home.as_deref()) {
        return Err(missing);
    }
    Ok(resolved(server, path.as_deref(), home.as_deref()))
}

fn home_dir() -> Option<PathBuf> {
    std::env::var_os("HOME").filter(|h| !h.is_empty()).map(PathBuf::from)
}

fn program_name(program: &str) -> &str {
    Path::new(program).file_name().and_then(|n| n.to_str()).unwrap_or(program)
}

/// `which`, plus the usual install folders when `home` is given. A name with a folder part is
/// checked where it points.
fn resolve_program(program: &str, path: Option<&str>, home: Option<&Path>) -> Option<PathBuf> {
    if program.is_empty() {
        return None;
    }
    let as_path = Path::new(program);
    if as_path.components().count() > 1 {
        return as_path.is_file().then(|| as_path.to_path_buf());
    }
    let mut dirs: Vec<PathBuf> = path.map(|p| std::env::split_paths(p).collect()).unwrap_or_default();
    if let Some(home) = home {
        for dir in EXTRA_BIN_DIRS {
            dirs.push(match dir.strip_prefix("~/") {
                Some(rest) => home.join(rest),
                None => PathBuf::from(dir),
            });
        }
    }
    dirs.into_iter().map(|d| d.join(program)).find(|c| c.is_file())
}

/// A call's arguments as kwnt sends them: never `authorize`, whatever the model asked for.
pub fn guard_arguments(arguments: &Value) -> Value {
    let mut out = arguments.clone();
    if let Some(map) = out.as_object_mut() {
        map.remove("authorize");
    }
    out
}

/// fastbrowse stopped before an irreversible action: what the approval card shows, or `None`
/// for any other result.
pub fn confirmation(arguments: &Value, result_text: &str) -> Option<Value> {
    let result: Value = serde_json::from_str(result_text).ok()?;
    if result["status"].as_str()? != "needs_confirmation" {
        return None;
    }
    Some(json!({
        "task": arguments["task"],
        "start": arguments["start"],
        "stopped_at": result["final_url"],
        "fastbrowse_says": result["answer"],
        "steps": result["steps"],
        "dollars": result["dollars"],
    }))
}

/// What the bot reads after the person answers a confirmation card.
pub fn confirmation_note(allowed: bool) -> &'static str {
    if allowed {
        "\n\nkwnt: fastbrowse stopped before an irreversible step and never takes one itself. The person \
         approved this step: finish it yourself in kwnt's own browser (it asks again if the step pays)."
    } else {
        "\n\nkwnt: fastbrowse stopped before an irreversible step and the person did not approve it. Don't \
         take this step another way; tell the person where it stopped."
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    fn server(command: &str, args: &[&str], env: &[(&str, &str)]) -> McpServer {
        McpServer {
            id: "s".into(),
            name: "web".into(),
            transport: "stdio".into(),
            command: Some(command.into()),
            args: args.iter().map(|a| a.to_string()).collect(),
            url: None,
            env: env.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect::<BTreeMap<_, _>>(),
            headers: BTreeMap::new(),
            enabled: true,
            timeout_secs: 600,
            created_at: None,
        }
    }

    fn strings(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn fastbrowse_is_found_by_its_command_not_its_name() {
        assert!(is_fastbrowse(&server("uvx", &["--from", "fastbrowse", "fastbrowse-mcp", "--local"], &[])));
        assert!(is_fastbrowse(&server("/home/me/.local/bin/fastbrowse-mcp", &["--local"], &[])));
        assert!(is_fastbrowse(&server("npx", &["-y", "fastbrowse@1.30.0"], &[])));
        assert!(!is_fastbrowse(&server("uvx", &["mcp-server-git"], &[])));
        assert!(!is_fastbrowse(&server("npx", &["-y", "@playwright/mcp"], &[])));
    }

    #[test]
    fn authorize_and_cloud_settings_are_refused_and_local_is_required() {
        let ok = strings(&["--from", "fastbrowse", "fastbrowse-mcp", "--local"]);
        assert_eq!(config_problem(&ok, &["OPENROUTER_API_KEY"]), None);
        for bad in ["--allow-authorize", "--authorize", "--allow-authorize=true", "--cloud-profile", "--transport"] {
            let mut args = ok.clone();
            args.push(bad.into());
            assert!(config_problem(&args, &[]).is_some(), "{bad} should be refused");
        }
        assert!(config_problem(&ok, &["BROWSER_USE_API_KEY"]).unwrap().contains("cloud"));
        assert!(config_problem(&strings(&["fastbrowse-mcp"]), &[]).unwrap().contains("--local"));
    }

    #[test]
    fn a_call_never_carries_authorize() {
        let args = json!({"task": "Send the form", "authorize": true, "max_steps": 5});
        assert_eq!(guard_arguments(&args), json!({"task": "Send the form", "max_steps": 5}));
    }

    #[test]
    fn missing_uv_chrome_and_key_are_said_plainly() {
        let empty = tempfile::tempdir().unwrap();
        let path = empty.path().display().to_string();
        let s = server("uvx", &["--from", "fastbrowse", "fastbrowse-mcp", "--local"], &[]);
        let said = missing_runtime(&s, Some(&path), Some(empty.path())).unwrap();
        assert!(said.contains("uv, which runs fastbrowse"), "{said}");
        assert!(said.contains("astral.sh/uv/install.sh"), "{said}");
        assert!(said.contains("brew install uv"), "{said}");
        assert!(said.contains("Chrome"), "{said}");
        assert!(said.contains("OPENROUTER_API_KEY"), "{said}");

        let n = server("npx", &["-y", "fastbrowse"], &[]);
        assert!(missing_runtime(&n, Some(&path), None).unwrap().contains("Node.js"));
    }

    #[test]
    fn uv_in_its_install_folder_is_found_when_path_lacks_it() {
        let home = tempfile::tempdir().unwrap();
        let bin = home.path().join(".local/bin");
        std::fs::create_dir_all(&bin).unwrap();
        std::fs::write(bin.join("uvx"), "").unwrap();
        let chrome_dir = tempfile::tempdir().unwrap();
        std::fs::write(chrome_dir.path().join("chromium"), "").unwrap();
        let path = chrome_dir.path().display().to_string();
        let s = server("uvx", &["--from", "fastbrowse", "fastbrowse-mcp", "--local"], &[("OPENROUTER_API_KEY", "k")]);
        assert_eq!(missing_runtime(&s, Some(&path), Some(home.path())), None);
        assert_eq!(resolved(&s, Some(&path), Some(home.path())).command.unwrap(), bin.join("uvx").display().to_string());
    }

    #[test]
    fn only_a_needs_confirmation_result_is_a_confirmation() {
        let args = json!({"task": "Submit the contact form", "start": "https://example.com/"});
        let stop = json!({"status": "needs_confirmation", "final_url": "https://example.com/contact", "answer": null, "steps": 4, "dollars": 0.01});
        let card = confirmation(&args, &stop.to_string()).unwrap();
        assert_eq!(card["stopped_at"], "https://example.com/contact");
        assert_eq!(card["task"], "Submit the contact form");
        assert!(confirmation(&args, &json!({"status": "complete"}).to_string()).is_none());
        assert!(confirmation(&args, "not json").is_none());
    }
}
