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
            // A Glob pattern names a place too (`/home/x/**`, `../../**`)
            _ if tool == "Glob" && !glob_stays_inside(input, folder) => false,
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

/// Whether a Glob's `pattern` stays under its `path` (or the folder) inside `folder`: not
/// absolute, not `~`, and with no `..` anywhere.
fn glob_stays_inside(input: &Value, folder: Option<&Path>) -> bool {
    let Some(folder) = folder else { return false };
    let pattern = input.get("pattern").and_then(Value::as_str).map(str::trim).unwrap_or("");
    let p = Path::new(pattern);
    if pattern.starts_with('~') || p.is_absolute() || p.components().any(|c| c == Component::ParentDir) {
        return false;
    }
    let base = target_path(input).unwrap_or_else(|| ".".into());
    is_inside(folder, &absolute(folder, &base).join(glob_root(p)).to_string_lossy())
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
        let Some(folder) = folder else { return SmartVerdict::Ask("the bot has no working folder this turn".into()) };
        return match read_risk(tool, input, folder) {
            Some(why) => SmartVerdict::Ask(why),
            None => SmartVerdict::Allow("reads outside the bot's folder, away from passwords and keys".into()),
        };
    }
    if WRITE_TOOLS.contains(&tool) {
        return SmartVerdict::Ask("it writes outside the bot's folder".into());
    }
    SmartVerdict::Ask(format!("kwnt doesn't know what `{tool}` does"))
}

/// Why a read outside the folder needs a card, or `None` when it stays away from passwords,
/// keys and the system's live process data. Relative paths count from the bot's `folder`.
fn read_risk(tool: &str, input: &Value, folder: &Path) -> Option<String> {
    let text = |k: &str| input.get(k).and_then(Value::as_str).map(str::trim).filter(|p| !p.is_empty());
    let mut places: Vec<(String, PathBuf)> = Vec::new();
    if tool == "Glob" {
        // Glob searches `pattern` under `path` (the folder when missing); an absolute pattern
        // ignores `path`
        let base = text("path").unwrap_or(".");
        let pattern = text("pattern").unwrap_or("");
        for p in [base, pattern] {
            if p.starts_with('~') {
                return Some("kwnt can't tell where a `~` path points".into());
            }
        }
        let joined = if Path::new(pattern).is_absolute() { PathBuf::from(pattern) } else { absolute(folder, base).join(pattern) };
        places.push((pattern.to_string(), joined));
    } else {
        for k in ["file_path", "notebook_path", "path"] {
            if let Some(p) = text(k) {
                if p.starts_with('~') {
                    return Some("kwnt can't tell where a `~` path points".into());
                }
                places.push((p.to_string(), absolute(folder, p)));
            }
        }
        if places.is_empty() {
            return Some("kwnt can't tell where this read looks".into());
        }
    }
    let searches = matches!(tool, "Glob" | "Grep");
    for (written, path) in places {
        let root = if tool == "Glob" { glob_root(&path) } else { path.clone() };
        for p in [Some(lexical(&path)), resolve(&root)].into_iter().flatten() {
            if let Some(why) = denied_place(&p) {
                return Some(why.replace("{}", &written));
            }
        }
        if searches {
            if let Some(why) = broad_search(&root) {
                return Some(why);
            }
        }
    }
    None
}

/// Folders and files that hold passwords, keys or signed-in sessions: reading them always asks.
/// Compared without regard to case.
const CREDENTIAL_NAMES: &[&str] = &[
    ".ssh", ".aws", ".azure", ".gnupg", ".kube", ".minikube", ".docker", ".netrc", ".claude", ".claude.json", ".codex",
    ".git-credentials", ".gitcookies", ".npmrc", ".yarnrc", ".yarnrc.yml", ".pypirc", ".pgpass", ".my.cnf", ".password-store",
    ".mozilla", ".pki", ".vault-token", ".terraform.d", ".terraformrc", ".boto", ".s3cfg", ".oci", ".gem", ".m2", ".gradle",
    ".composer", ".fly", ".vercel", ".netlify", ".op", ".bash_history", ".zsh_history", ".python_history", ".node_repl_history",
    ".psql_history", ".mysql_history", ".lesshst", ".viminfo", "keychains", "keyrings", "id_rsa", "id_dsa", "id_ecdsa",
    "id_ed25519", "credentials", "credentials.json", "credentials.toml", "secrets", "secrets.json", "secrets.yaml",
    "secrets.yml", "auth.json", "hosts.yml", "token", "tokens", ".kwnt", ".openmausbot",
];
const CREDENTIAL_FOLDERS: &[&str] = &[
    ".config/gh", ".config/hub", ".config/gcloud", ".config/google-chrome", ".config/chromium", ".config/bravesoftware",
    ".config/microsoft-edge", ".config/vivaldi", ".config/opera", ".config/1password", ".config/op", ".config/bitwarden",
    ".config/bitwarden cli", ".config/github-copilot", ".config/configstore", ".config/rclone", ".config/doctl",
    ".config/heroku", ".config/netlify", ".config/vercel", ".config/.wrangler", ".config/helm", ".config/sops",
    ".config/age", ".config/keybase", ".config/code/user/globalstorage", ".local/share/keyrings", ".cargo/credentials",
    ".cargo/credentials.toml", "library/application support/google", "library/application support/firefox",
    "library/application support/bravesoftware", "library/application support/microsoft edge",
    "library/application support/chromium", "library/application support/arc", "library/application support/1password",
    "library/application support/bitwarden", "library/application support/code/user/globalstorage", "library/cookies",
    "library/safari", "library/containers/com.apple.safari", "library/group containers",
];
/// File endings of keys, certificates and password vaults.
const CREDENTIAL_ENDINGS: &[&str] = &[".pem", ".key", ".p12", ".pfx", ".jks", ".keystore", ".kdbx", ".ppk", ".asc", ".gpg"];
/// Live process and device data: `/proc/self/environ` holds the gateway token.
const SYSTEM_PLACES: &[&str] = &["/proc", "/sys", "/dev", "/run", "/var/run", "/private/var/run"];

/// Whether `path` (absolute) is live system data or names a credential, and why (`{}` stands
/// for the path as the bot wrote it).
fn denied_place(path: &Path) -> Option<&'static str> {
    let p = path.to_string_lossy().to_lowercase();
    if SYSTEM_PLACES.iter().any(|s| p == *s || p.starts_with(&format!("{s}/"))) {
        return Some("{} is live system data and can hold secrets");
    }
    credential_text(&p).then_some("{} can hold passwords or keys")
}

/// Whether a lowercase path names a credential folder or file anywhere along it.
fn credential_text(p: &str) -> bool {
    let parts: Vec<&str> = p.split('/').filter(|c| !c.is_empty()).collect();
    parts.iter().any(|c| {
        CREDENTIAL_NAMES.contains(c) || c.starts_with(".env") || CREDENTIAL_ENDINGS.iter().any(|e| c.ends_with(e))
    }) || CREDENTIAL_FOLDERS.iter().any(|f| {
        let tail = format!("/{f}");
        p.contains(&format!("{tail}/")) || p.ends_with(&tail) || p.starts_with(&format!("{f}/")) || p == *f
    })
}

/// How many entries kwnt looks through before calling a search too big to check.
const SEARCH_CHECK_LIMIT: usize = 5000;

/// Why a recursive search from `root` needs a card: it starts at `/` or a home folder (or above
/// one), or something under it is a credential, a symlink or too much to check.
fn broad_search(root: &Path) -> Option<String> {
    let real = resolve(root).unwrap_or_else(|| lexical(root));
    let shown = real.display();
    let homes = std::env::var_os("HOME").map(PathBuf::from).into_iter().chain(["/home", "/Users", "/root"].map(PathBuf::from));
    for home in homes {
        let home = home.canonicalize().unwrap_or(home);
        if home.starts_with(&real) {
            return Some(format!("it searches {shown}, which holds home folders and their keys"));
        }
    }
    if !real.is_dir() {
        return None;
    }
    let mut queue = vec![real.clone()];
    let mut seen = 0usize;
    while let Some(dir) = queue.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else { continue };
        for entry in entries.flatten() {
            seen += 1;
            if seen > SEARCH_CHECK_LIMIT {
                return Some(format!("{shown} is too big for kwnt to check for passwords and keys"));
            }
            let path = entry.path();
            if credential_text(&path.to_string_lossy().to_lowercase()) {
                return Some(format!("{} can hold passwords or keys", path.display()));
            }
            match entry.file_type() {
                Ok(t) if t.is_symlink() => return Some(format!("{} is a symlink kwnt can't follow safely", path.display())),
                Ok(t) if t.is_dir() => queue.push(path),
                _ => {}
            }
        }
    }
    None
}

/// The folder a glob pattern starts from: its path up to the first part with a wildcard.
fn glob_root(pattern: &Path) -> PathBuf {
    let mut root = PathBuf::new();
    for c in pattern.components() {
        if c.as_os_str().to_string_lossy().contains(['*', '?', '[', '{']) {
            break;
        }
        root.push(c);
    }
    root
}

/// `path` made absolute against `folder`.
fn absolute(folder: &Path, path: &str) -> PathBuf {
    let p = Path::new(path);
    if p.is_absolute() { p.to_path_buf() } else { folder.join(p) }
}

/// `path` with `.` and `..` worked out on the text alone.
fn lexical(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for c in path.components() {
        match c {
            Component::ParentDir => {
                out.pop();
            }
            Component::CurDir => {}
            other => out.push(other),
        }
    }
    out
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

    fn smart(tool: &str, input: Value, ws: &Path) -> bool {
        smart_verdict(tool, &input, Some(ws)).is_allowed()
    }

    // Rigel's deny-list repros (smart-approvals gate, 2026-10-21)
    #[test]
    fn smart_reads_of_live_system_data_ask() {
        let (_tmp, ws) = folder();
        for path in ["/proc/self/environ", "/proc/1/environ", "/PROC/self/environ", "/proc/self/root/etc/passwd", "/sys/kernel/x", "/dev/stdin", "/run/secrets/x", "/var/run/docker.sock"] {
            assert!(!smart("Read", json!({"file_path": path}), &ws), "{path}");
        }
        assert!(!smart("Grep", json!({"pattern": "TOKEN", "path": "/proc/self"}), &ws));
        assert!(!smart("Glob", json!({"pattern": "/proc/*/environ"}), &ws));
        #[cfg(unix)]
        {
            // A symlink in the folder that points into /proc
            std::os::unix::fs::symlink("/proc/self", ws.join("me")).unwrap();
            assert_eq!(decide("Read", &json!({"file_path": "me/environ"}), Some(&ws)), BuiltinDecision::Ask { outside_folder: true });
            assert!(!smart("Read", json!({"file_path": "me/environ"}), &ws));
        }
    }

    #[test]
    fn smart_credential_names_match_without_regard_to_case_and_cover_more_places() {
        let (_tmp, ws) = folder();
        for path in [
            "/home/x/.SSH/id_rsa", "/srv/app/.ENV", "/srv/app/.Env.local", "/Users/x/Library/keychains/login.keychain-db",
            "/Users/x/Library/Application Support/google/Chrome/Default/Cookies", "/home/x/.Config/GH/hosts.yml",
            "/home/x/.azure/accessTokens.json", "/home/x/.pgpass", "/home/x/.vault-token", "/home/x/.kube/config",
            "/home/x/.cargo/credentials.toml", "/home/x/.config/Bitwarden CLI/data.json", "/home/x/.local/share/keyrings/login.keyring",
            "/home/x/.bash_history", "/etc/ssl/private/server.key", "/opt/vault.kdbx", "/home/x/.kwnt/secrets.json",
        ] {
            assert!(!smart("Read", json!({"file_path": path}), &ws), "{path}");
        }
        assert!(smart("Read", json!({"file_path": "/etc/hostname"}), &ws), "ordinary reads outside still run");
    }

    #[test]
    fn smart_relative_reads_count_from_the_bots_folder() {
        let (tmp, ws) = folder();
        std::fs::create_dir_all(tmp.path().join(".ssh")).unwrap();
        std::fs::write(tmp.path().join(".ssh/id_rsa"), "k").unwrap();
        std::fs::write(tmp.path().join("notes.txt"), "n").unwrap();
        assert!(!smart("Read", json!({"file_path": "../.ssh/id_rsa"}), &ws));
        assert!(!smart("Read", json!({"file_path": "src/../../.ssh/id_rsa"}), &ws));
        assert!(smart("Read", json!({"file_path": "../notes.txt"}), &ws));
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(tmp.path().join(".ssh"), ws.join("keys")).unwrap();
            assert!(!smart("Read", json!({"file_path": "keys/id_rsa"}), &ws), "a symlink to a key folder");
        }
    }

    #[test]
    fn smart_searches_from_root_home_or_above_credentials_ask() {
        let (tmp, ws) = folder();
        let home = std::env::var("HOME").unwrap_or_else(|_| "/root".into());
        for root in ["/", "/home", "/Users", home.as_str()] {
            assert!(!smart("Grep", json!({"pattern": "x", "path": root}), &ws), "Grep {root}");
            assert!(!smart("Glob", json!({"pattern": "**/*", "path": root}), &ws), "Glob {root}");
        }
        // A folder that holds a credential folder somewhere below
        let other = tmp.path().join("other");
        std::fs::create_dir_all(other.join("app/config")).unwrap();
        std::fs::write(other.join("app/readme.md"), "r").unwrap();
        let other_s = other.display().to_string();
        assert!(smart("Grep", json!({"pattern": "x", "path": other_s}), &ws), "a plain folder outside");
        std::fs::write(other.join("app/config/.env"), "SECRET=1").unwrap();
        assert!(!smart("Grep", json!({"pattern": "SECRET", "path": other_s}), &ws), "a .env deep below");
        assert!(!smart("Glob", json!({"pattern": format!("{other_s}/**/*")}), &ws));
    }

    #[test]
    fn globs_cannot_escape_through_their_pattern() {
        let (tmp, ws) = folder();
        // Under plain ask: an absolute or `..` pattern is outside, whatever `path` says
        for pattern in ["/home/**/.ssh/*", "../../**/*", "src/**/../../../*", "~/.aws/*"] {
            assert_eq!(decide("Glob", &json!({"pattern": pattern}), Some(&ws)), BuiltinDecision::Ask { outside_folder: true }, "{pattern}");
            assert_eq!(
                decide("Glob", &json!({"pattern": pattern, "path": ws.display().to_string()}), Some(&ws)),
                BuiltinDecision::Ask { outside_folder: true },
                "{pattern} with path"
            );
        }
        assert_eq!(decide("Glob", &json!({"pattern": "src/**/*.rs"}), Some(&ws)), BuiltinDecision::Allow);
        assert!(!smart("Glob", json!({"pattern": "/home/**/.ssh/*"}), &ws));
        let _ = tmp;
    }

    #[test]
    fn shell_and_unknown_tools_ask() {
        let (_tmp, ws) = folder();
        assert_eq!(decide("Bash", &json!({"command": "ls"}), Some(&ws)), BuiltinDecision::Ask { outside_folder: false });
        assert_eq!(decide("SomeFutureTool", &json!({}), Some(&ws)), BuiltinDecision::Ask { outside_folder: false });
    }
}
