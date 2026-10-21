//! What a claude CLI bot may do with claude's own (built-in) tools. claude asks kwnt before each
//! tool it would otherwise prompt for; kwnt answers from this policy:
//! - looking things up (web search, fetching pages, reading and searching inside the bot's
//!   working folder, its own to-do list and sub-agents) runs without asking;
//! - writing files runs without asking inside the working folder, and needs the person's OK
//!   anywhere else;
//! - reading outside the working folder, shell commands and any tool kwnt doesn't know need the
//!   person's OK (an approval card, where "Always this session" is honoured).
//!
//! Under "Ask only when risky" ([`smart_verdict`]) shell commands the classifier in
//! [`super::shell_risk`] finds safe, and reads outside the folder that don't touch credentials,
//! run without a card too.

use super::SmartVerdict;
use serde_json::Value;
use std::path::{Component, Path, PathBuf};

/// How kwnt answers one built-in tool call.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BuiltinDecision {
    Allow,
    /// Show the person an approval card. `outside_folder` is set for file tools whose path is
    /// outside the bot's working folder (or when the bot has none).
    Ask { outside_folder: bool },
}

/// Tools that only look things up, and touch no files.
const LOOKUP_TOOLS: &[&str] = &["WebSearch", "WebFetch", "TodoWrite", "TodoRead", "Task", "Agent", "BashOutput", "ExitPlanMode", "ToolSearch"];
/// Tools that read files: free inside the working folder.
const READ_TOOLS: &[&str] = &["Read", "Glob", "Grep", "LS", "NotebookRead"];
/// Tools that write files: free inside the working folder.
const WRITE_TOOLS: &[&str] = &["Write", "Edit", "MultiEdit", "NotebookEdit"];

/// The policy for `tool` with `input`, for a bot working in `folder` (canonical, or `None`).
pub fn decide(tool: &str, input: &Value, folder: Option<&Path>) -> BuiltinDecision {
    if LOOKUP_TOOLS.contains(&tool) {
        return BuiltinDecision::Allow;
    }
    let reads = READ_TOOLS.contains(&tool);
    if reads || WRITE_TOOLS.contains(&tool) {
        let inside = match (folder, target_path(input)) {
            (Some(folder), Some(path)) => is_inside(folder, &path),
            // A read with no path searches the working folder itself
            (Some(_), None) => reads,
            (None, _) => false,
        };
        return if inside { BuiltinDecision::Allow } else { BuiltinDecision::Ask { outside_folder: true } };
    }
    // Bash and anything this version of kwnt doesn't know about
    BuiltinDecision::Ask { outside_folder: false }
}

/// What "Ask only when risky" makes of a call [`decide`] would ask about.
pub fn smart_verdict(tool: &str, input: &Value, folder: Option<&Path>) -> SmartVerdict {
    if tool == "Bash" {
        let Some(folder) = folder else { return SmartVerdict::Ask("the bot has no working folder this turn".into()) };
        return match input.get("command").and_then(Value::as_str) {
            Some(command) => super::shell_risk::classify(command, folder),
            None => SmartVerdict::Ask("kwnt can't read this command".into()),
        };
    }
    if READ_TOOLS.contains(&tool) {
        // A pattern can name a place too (`Glob` with `/home/x/**`)
        let keys: &[&str] = if tool == "Glob" { &["path", "pattern"] } else { &["file_path", "notebook_path", "path"] };
        let paths: Vec<&str> = keys
            .iter()
            .filter_map(|k| input.get(*k).and_then(Value::as_str))
            .map(str::trim)
            .filter(|p| !p.is_empty())
            .collect();
        let Some(target) = target_path(input) else {
            return SmartVerdict::Ask("kwnt can't tell where this read looks".into());
        };
        if target.starts_with('~') {
            return SmartVerdict::Ask("kwnt can't tell where a `~` path points".into());
        }
        return match paths.iter().find(|p| credential_path(p)) {
            Some(p) => SmartVerdict::Ask(format!("{p} can hold passwords or keys")),
            None => SmartVerdict::Allow("reads outside the bot's folder, away from passwords and keys".into()),
        };
    }
    if WRITE_TOOLS.contains(&tool) {
        return SmartVerdict::Ask("it writes outside the bot's folder".into());
    }
    SmartVerdict::Ask(format!("kwnt doesn't know what `{tool}` does"))
}

/// Folders and files that hold passwords, keys or signed-in sessions: reading them always asks.
const CREDENTIAL_NAMES: &[&str] = &[
    ".ssh", ".aws", ".gnupg", ".kube", ".docker", ".netrc", ".claude", ".claude.json", ".codex", ".git-credentials",
    ".npmrc", ".pypirc", ".password-store", ".mozilla", "Keychains",
];
const CREDENTIAL_FOLDERS: &[&str] = &[
    ".config/gh", ".config/gcloud", ".config/google-chrome", ".config/chromium", ".config/BraveSoftware",
    ".config/microsoft-edge", "Library/Application Support/Google", "Library/Application Support/Firefox",
    "Library/Application Support/BraveSoftware", "Library/Cookies",
];

/// Whether `path`, as written or once symlinks are followed, is under a credential folder or
/// names a `.env` file.
fn credential_path(path: &str) -> bool {
    let resolved = resolve(Path::new(path)).map(|p| p.display().to_string());
    [Some(path.to_string()), resolved].into_iter().flatten().any(|p| {
        let parts: Vec<&str> = p.split('/').filter(|c| !c.is_empty()).collect();
        parts.iter().any(|c| CREDENTIAL_NAMES.contains(c) || c.starts_with(".env"))
            || CREDENTIAL_FOLDERS.iter().any(|f| p.contains(&format!("/{f}")) || p.starts_with(f))
    })
}

/// The file or folder a file tool acts on.
fn target_path(input: &Value) -> Option<String> {
    ["file_path", "notebook_path", "path"]
        .iter()
        .find_map(|k| input.get(*k).and_then(Value::as_str))
        .map(str::trim)
        .filter(|p| !p.is_empty())
        .map(str::to_string)
}

/// The canonical working folder a bot's file tools are measured against.
pub fn canonical_folder(folder: Option<&Path>) -> Option<PathBuf> {
    folder.and_then(|f| f.canonicalize().ok())
}

/// Whether `path` (absolute, or relative to `folder`) is `folder` or inside it, once symlinks
/// are followed. A path that doesn't exist yet is judged by its nearest existing parent, and
/// never by a `..` in its unwritten part. A `~` path counts as outside: a tool may expand it.
pub fn is_inside(folder: &Path, path: &str) -> bool {
    if path.starts_with('~') {
        return false;
    }
    let p = Path::new(path);
    let joined = if p.is_absolute() { p.to_path_buf() } else { folder.join(p) };
    match resolve(&joined) {
        Some(real) => real.starts_with(folder),
        None => false,
    }
}

/// Follows symlinks through the existing part of `path` and appends the rest, refusing a `..`
/// (or anything but a plain name) in the part that doesn't exist yet.
fn resolve(path: &Path) -> Option<PathBuf> {
    let mut existing = path.to_path_buf();
    let mut rest: Vec<std::ffi::OsString> = Vec::new();
    loop {
        if let Ok(real) = existing.canonicalize() {
            let mut out = real;
            for part in rest.iter().rev() {
                out.push(part);
            }
            return Some(out);
        }
        // A dangling symlink would be followed on write: its target is unknown, so refuse
        if existing.symlink_metadata().is_ok() {
            return None;
        }
        let name = match existing.components().next_back()? {
            Component::Normal(name) => name.to_os_string(),
            _ => return None,
        };
        rest.push(name);
        existing = existing.parent()?.to_path_buf();
    }
}

/// The paragraph a claude CLI bot reads about its own tools; `smart` when its approvals ask only
/// for risky calls.
pub fn prompt_block(folder: Option<&Path>, smart: bool) -> String {
    let place = match folder {
        Some(f) => format!("Your working folder is {}. You can create, edit and delete files there without asking.", f.display()),
        None => "You have no working folder this turn, so every file write needs the person's OK.".to_string(),
    };
    let cards = if smart {
        "Shell commands (Bash) that stay in that folder run without asking. Anything risky shows the person an approval card first: \
         the network (except localhost), other users, git push, publishing, containers, mail, paths outside the folder, and \
         commands kwnt can't read for sure ($(…), backticks, bash -c). Prefer plain commands and relative paths. \
         Writing files outside the folder, and reading passwords or keys, also needs a card."
    } else {
        "Writing or reading files outside it, and every shell command (Bash), shows the person an approval card first."
    };
    format!(
        "\n\n[Your tools]\nYou can search the web (WebSearch) and read web pages (WebFetch) whenever it helps; cite the pages you used. \
         {place} {cards} \
         Ask for what you need and wait for their answer instead of giving up. If they deny it, say what you would have done."
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn folder() -> (tempfile::TempDir, PathBuf) {
        let tmp = tempfile::tempdir().unwrap();
        let ws = tmp.path().join("ws");
        std::fs::create_dir_all(ws.join("src")).unwrap();
        let canon = ws.canonicalize().unwrap();
        (tmp, canon)
    }

    #[test]
    fn lookups_run_without_asking() {
        for tool in ["WebSearch", "WebFetch", "TodoWrite", "Task"] {
            assert_eq!(decide(tool, &json!({"url": "https://x"}), None), BuiltinDecision::Allow, "{tool}");
        }
    }

    #[test]
    fn writes_inside_the_folder_run_and_outside_ask() {
        let (tmp, ws) = folder();
        let inside = ws.join("src/new.go").display().to_string();
        assert_eq!(decide("Write", &json!({"file_path": inside}), Some(&ws)), BuiltinDecision::Allow);
        assert_eq!(decide("Edit", &json!({"file_path": "src/a.go"}), Some(&ws)), BuiltinDecision::Allow);
        assert_eq!(decide("NotebookEdit", &json!({"notebook_path": "n.ipynb"}), Some(&ws)), BuiltinDecision::Allow);
        let outside = tmp.path().join("other.txt").display().to_string();
        for tool in ["Write", "Edit", "MultiEdit"] {
            assert_eq!(decide(tool, &json!({"file_path": outside}), Some(&ws)), BuiltinDecision::Ask { outside_folder: true }, "{tool}");
        }
        // No folder, or no path: ask
        assert_eq!(decide("Write", &json!({"file_path": inside}), None), BuiltinDecision::Ask { outside_folder: true });
        assert_eq!(decide("Write", &json!({}), Some(&ws)), BuiltinDecision::Ask { outside_folder: true });
    }

    #[test]
    fn dot_dot_and_symlinks_cannot_escape() {
        let (tmp, ws) = folder();
        for sneaky in ["../escape.txt", "src/../../escape.txt", "new/../../escape.txt", "/etc/passwd", "~/.ssh/id_rsa"] {
            assert!(!is_inside(&ws, sneaky), "{sneaky}");
        }
        // A not-yet-written path is judged by its existing parent
        assert!(is_inside(&ws, "src/deep/new/file.rs"));
        #[cfg(unix)]
        {
            let outside = tmp.path().join("outside");
            std::fs::create_dir_all(&outside).unwrap();
            std::os::unix::fs::symlink(&outside, ws.join("link")).unwrap();
            assert!(!is_inside(&ws, "link/file.txt"), "a symlink out of the folder is outside");
            std::os::unix::fs::symlink(tmp.path().join("nowhere"), ws.join("dangling")).unwrap();
            assert!(!is_inside(&ws, "dangling"), "a dangling symlink is refused");
            std::os::unix::fs::symlink(ws.join("src"), ws.join("inner")).unwrap();
            assert!(is_inside(&ws, "inner/ok.rs"), "a symlink within the folder stays inside");
        }
    }

    #[test]
    fn reads_are_free_inside_and_ask_outside() {
        let (tmp, ws) = folder();
        assert_eq!(decide("Read", &json!({"file_path": "src/x.rs"}), Some(&ws)), BuiltinDecision::Allow);
        assert_eq!(decide("Grep", &json!({"pattern": "fn"}), Some(&ws)), BuiltinDecision::Allow);
        assert_eq!(decide("Glob", &json!({"pattern": "**/*.rs", "path": ws.display().to_string()}), Some(&ws)), BuiltinDecision::Allow);
        let secret = tmp.path().join(".ssh/id_rsa").display().to_string();
        assert_eq!(decide("Read", &json!({"file_path": secret}), Some(&ws)), BuiltinDecision::Ask { outside_folder: true });
    }

    #[test]
    fn shell_and_unknown_tools_ask() {
        let (_tmp, ws) = folder();
        assert_eq!(decide("Bash", &json!({"command": "ls"}), Some(&ws)), BuiltinDecision::Ask { outside_folder: false });
        assert_eq!(decide("SomeFutureTool", &json!({}), Some(&ws)), BuiltinDecision::Ask { outside_folder: false });
    }
}
