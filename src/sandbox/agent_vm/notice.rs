//! What the person and the model read when isolation stops something (Mira's
//! `design/ui.md`, "Copy for Ada's decisions 2").
//!
//! - [`IsolationFailed`] is the chat meta `turn_isolation_failed`, one shape for every reason, so
//!   the frontend has one renderer. `unsafe_folder` and `folder_used_by_isolated_bot` add the
//!   folder, where it came from and, for `unsafe_folder`, `unsafe_kind`.
//! - [`body`] holds the broker's refusal texts. The CLI shows them to Claude, not to the person,
//!   so they say what to do next.

use super::folder_rule::UnsafeKind;
use serde_json::{json, Map, Value};
use std::path::Path;

/// The Claude CLI's own tools an isolated turn starts without (`--disallowedTools`), so the model
/// goes straight to kwnt's browser tools instead of hitting the broker's 400 and retrying (D3).
pub const DISALLOWED_TOOLS: &str = "WebSearch,WebFetch";

/// `args` (kwnt's claude argv, without the program) with [`DISALLOWED_TOOLS`] added: merged into
/// the first `--disallowedTools` value (claude reads one comma-separated list), or as a new flag.
pub fn with_disallowed_tools(mut args: Vec<std::ffi::OsString>) -> Vec<std::ffi::OsString> {
    if let Some(i) = args.iter().position(|a| a == "--disallowedTools") {
        if let Some(v) = args.get_mut(i + 1) {
            let mut list: Vec<String> = v.to_string_lossy().split(',').map(str::trim).filter(|s| !s.is_empty()).map(String::from).collect();
            for t in DISALLOWED_TOOLS.split(',') {
                if !list.iter().any(|x| x == t) {
                    list.push(t.into());
                }
            }
            *v = list.join(",").into();
            return args;
        }
    }
    args.extend(["--disallowedTools".into(), DISALLOWED_TOOLS.into()]);
    args
}

pub mod reason {
    pub const UNSAFE_FOLDER: &str = "unsafe_folder";
    pub const FOLDER_USED_BY_ISOLATED_BOT: &str = "folder_used_by_isolated_bot";
}

/// The broker's response bodies the model reads.
pub mod body {
    /// 400: a server tool (`web_search_*`, `web_fetch_*`, `code_execution_*`, …) in the request.
    pub const SERVER_TOOL: &str = "Web search and fetch don't work while this bot runs inside kwant's VM. Use kwant's browser tools instead.";
    /// 400: `mcp_servers` in the request.
    pub const MCP_SERVERS: &str = "Remote MCP servers can't be added from inside kwant's VM. Use the tools kwant already gives this bot.";
    /// 400: a content block Anthropic would fetch itself (`source` by URL or Files API id). Not in
    /// Mira's table; same voice.
    pub const FETCH_BY_REFERENCE: &str = "Anthropic can't fetch links or files for this bot while it runs inside kwant's VM. Put the content in the message, or use kwant's browser tools.";
    /// 403: no turn is open for this container (D2).
    pub const NO_TURN: &str = "No reply is running for this bot, so kwant won't pass this request on.";

    /// 429: the turn reached its request cap.
    pub fn request_cap(limit: u32) -> String {
        format!("This reply reached its limit of {limit} requests to Claude. kwant paused it, and the person can continue it from the chat.")
    }
}

/// The safety line shared by notices 1–3 and the rows below: nothing ran outside the VM.
pub const SAFETY: &str = "While key protection is on, Claude never runs outside its sealed box, so nothing ran on this computer.";

/// The chat text saved with a `turn_isolation_failed` notice that stopped Claude before it
/// started (Mira's notices 1, 2, 2b, 2c, 4 and the rows of ui-check-446 fix 4). The card is drawn
/// from the meta; this is what a client without the card, and the chat history, show. Every
/// refusal code kwant sends has its own row ([`super::codes::ALL`]); the "Something went wrong"
/// row is only for a code a newer kwant adds. `unsafe_folder` with its folder is
/// [`unsafe_folder_text`].
pub fn start_text(bot: &str, reason: &str) -> String {
    let couldnt_start = format!("{bot} didn't reply: Claude's sealed box couldn't start.");
    let not_ready = format!("{bot} didn't reply: Claude's sealed box isn't ready yet.");
    let this_folder = format!("{bot} didn't reply: Claude can't take this folder into its sealed box.");
    match reason {
        "docker_unavailable" => format!("{couldnt_start} Docker isn't running on this computer. Start Docker, then Retry. {SAFETY}"),
        "preparing" => format!("{not_ready} The one-time download is still running. Retry when it's done. {SAFETY}"),
        "download_failed" | "download_needed" => {
            format!("{not_ready} The one-time download didn't finish. Try it again in Computer settings, then Retry. {SAFETY}")
        }
        "relay_down" => format!("{couldnt_start} kwant couldn't connect Claude to your sign-in. Restart kwant, then Retry. {SAFETY}"),
        "sign_in_expired" => format!("{bot} didn't reply: your Claude sign-in has expired. Sign in again in Settings › This space › Accounts, then Retry."),
        "no_credential" => format!(
            "{bot} didn't reply: there's no Claude sign-in for it on this computer. Sign in to Claude in Settings › This space › Accounts, then Retry."
        ),
        "host_is_root" => format!(
            "{couldnt_start} kwant is running as root, so Claude's sealed box can't run as you. Start kwant as your normal user, then Retry. {SAFETY}"
        ),
        "folder_name_unsupported" => format!(
            "{this_folder} The folder's name has characters kwant can't pass to Claude's sealed box, like a line break. Rename the folder, then Retry. Nothing ran."
        ),
        "folder_record_failed" => format!(
            "{couldnt_start} kwant couldn't note that this folder is used by Claude's sealed box, so it didn't start Claude. Retry; if it keeps happening, restart kwant. {SAFETY}"
        ),
        "container_failed" => format!("{couldnt_start} Docker couldn't start it. Retry; if it keeps happening, restart Docker. {SAFETY}"),
        "unsupported_os" => format!(
            "{couldnt_start} Isolation is only on Linux for now. Turn off key protection to run Claude on this computer. {SAFETY}"
        ),
        // Without its folder (Mira's "missing or unknown" row)
        reason::UNSAFE_FOLDER => format!(
            "{this_folder} This folder holds your home folder or sign-ins, so kwant won't share it with Claude's sealed box. Pick a project folder instead, then Retry. Nothing ran."
        ),
        code => format!(
            "{couldnt_start} Something went wrong ({code}). Retry, or turn off key protection to run Claude on this computer. {SAFETY}"
        ),
    }
}

/// The chat text saved with notice 6 (`unsafe_folder`), from Mira's two tables in `design/ui.md`:
/// one sentence for `kind`, one for where the folder came from. `home` shortens the folder.
pub fn unsafe_folder_text(bot: &str, kind: UnsafeKind, folder: &FolderInfo, home: Option<&Path>) -> String {
    let shown = tilde(&folder.folder, home);
    let team = folder.team.as_ref().map(|(_, name)| name.as_str());
    let first = format!("{bot} didn't reply: Claude can't take this folder into its sealed box.");
    let why = match kind {
        UnsafeKind::Home => format!("{shown} is your home folder. In its sealed box, Claude would see all your files and sign-ins."),
        UnsafeKind::ContainsHome | UnsafeKind::Root => {
            format!("{shown} holds your home folder. In its sealed box, Claude would see all your files and sign-ins.")
        }
        UnsafeKind::SignIns => format!(
            "{shown} holds sign-ins, like your SSH keys or Claude, Codex or Grok sign-in, so kwant won't share it with Claude's sealed box."
        ),
        UnsafeKind::KwantData => format!("{shown} holds kwant's own data, including your sign-ins, so kwant won't share it with Claude's sealed box."),
        UnsafeKind::Docker => format!("{shown} holds Docker's control socket, which would let Claude out of its sealed box."),
        // No path: kwant's own path means nothing to people
        UnsafeKind::SharedFolder => {
            let give = match team {
                Some(t) => format!("Give @{t} its own folder"),
                None => "Give this team its own folder".into(),
            };
            return format!(
                "{first} This is the folder teams share when they don't have their own. If Claude took it into its sealed box, Codex and Grok bots in every team would be kept out of it. {give}, then Retry. Nothing ran."
            );
        }
    };
    let pick = match (folder.from, team) {
        (FolderFrom::Team, Some(t)) => format!("Pick a project folder for @{t} instead"),
        (FolderFrom::Goal, _) => "Pick a project folder for this goal instead".into(),
        _ => "Pick a project folder instead".into(),
    };
    format!("{first} {why} {pick}, then Retry. Nothing ran.")
}

/// `folder` with `home` (the person's home folder) shortened to `~`.
pub fn tilde(folder: &str, home: Option<&Path>) -> String {
    let Some(home) = home.map(|h| h.to_string_lossy()).filter(|h| h.len() > 1) else { return folder.to_string() };
    let home = home.trim_end_matches('/');
    match folder.strip_prefix(home) {
        Some("") => "~".into(),
        Some(rest) if rest.starts_with('/') => format!("~{rest}"),
        _ => folder.to_string(),
    }
}

/// The text saved when an isolated reply stopped partway (notice 3).
pub fn mid_turn_text(bot: &str) -> String {
    format!(
        "{bot}'s reply stopped partway: Claude's sealed box lost its connection. What it changed before that is in the task folder, and Undo still works. Retry to start the reply again. {SAFETY}"
    )
}

/// The chat text saved with notice 7 (`folder_used_by_isolated_bot`). `engine`: `codex` or `grok`.
pub fn folder_used_text(bot: &str, engine: &str) -> String {
    let engine = match engine {
        "codex" => "Codex",
        "grok" => "Grok",
        other => other,
    };
    format!(
        "{bot} didn't reply: {engine} won't run in a folder Claude used in its sealed box. A Claude bot in its sealed box could have left settings there that {engine} would load on this computer, so kwant keeps {engine} out of it for now. Switch {bot} to Claude, or pick another folder, then Retry. Nothing ran."
    )
}

/// Where a turn's folder came from, which picks the second reason sentence and whether the
/// notice offers "Change folder".
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FolderFrom {
    Team,
    Space,
    Goal,
    Bot,
}

impl FolderFrom {
    pub fn as_str(self) -> &'static str {
        match self {
            FolderFrom::Team => "team",
            FolderFrom::Space => "space",
            FolderFrom::Goal => "goal",
            FolderFrom::Bot => "bot",
        }
    }
}

/// The folder a refused turn would have used.
#[derive(Debug, Clone)]
pub struct FolderInfo {
    /// Canonical path.
    pub folder: String,
    pub from: FolderFrom,
    /// Set when `from` is [`FolderFrom::Team`].
    pub team: Option<(String, String)>,
}

impl FolderInfo {
    pub fn new(folder: &Path, from: FolderFrom) -> Self {
        Self { folder: folder.to_string_lossy().into_owned(), from, team: None }
    }

    pub fn team(folder: &Path, team_id: &str, team_name: &str) -> Self {
        Self { folder: folder.to_string_lossy().into_owned(), from: FolderFrom::Team, team: Some((team_id.into(), team_name.into())) }
    }
}

/// The `turn_isolation_failed` chat meta.
#[derive(Debug, Clone)]
pub struct IsolationFailed {
    pub kwant_name: String,
    /// `claude` for every reason but `folder_used_by_isolated_bot` (`codex` or `grok`).
    pub engine: String,
    pub reason: String,
    /// `start` when it failed before the CLI ran.
    pub phase: &'static str,
    pub progress_pct: Option<u8>,
    /// The stable refusal code, when it isn't already the reason.
    pub code: Option<String>,
    pub folder: Option<FolderInfo>,
    pub unsafe_kind: Option<UnsafeKind>,
}

impl IsolationFailed {
    pub fn new(kwant_name: &str, engine: &str, reason: &str) -> Self {
        Self {
            kwant_name: kwant_name.into(),
            engine: engine.into(),
            reason: reason.into(),
            phase: "start",
            progress_pct: None,
            code: None,
            folder: None,
            unsafe_kind: None,
        }
    }

    /// `unsafe_folder`: Claude, refused before `docker run`.
    pub fn unsafe_folder(kwant_name: &str, folder: FolderInfo, kind: UnsafeKind) -> Self {
        Self { folder: Some(folder), unsafe_kind: Some(kind), ..Self::new(kwant_name, "claude", reason::UNSAFE_FOLDER) }
    }

    /// `folder_used_by_isolated_bot`: a host Codex or Grok turn in a folder an isolated bot used.
    pub fn folder_used(kwant_name: &str, engine: &str, folder: FolderInfo) -> Self {
        Self { folder: Some(folder), ..Self::new(kwant_name, engine, reason::FOLDER_USED_BY_ISOLATED_BOT) }
    }

    pub fn to_meta(&self) -> Value {
        let mut m = Map::new();
        m.insert("kind".into(), json!("turn_isolation_failed"));
        m.insert("kwant_name".into(), json!(self.kwant_name));
        m.insert("engine".into(), json!(self.engine));
        m.insert("reason".into(), json!(self.reason));
        m.insert("phase".into(), json!(self.phase));
        m.insert("progress_pct".into(), json!(self.progress_pct));
        m.insert("code".into(), json!(self.code));
        if let Some(f) = &self.folder {
            m.insert("folder".into(), json!(f.folder));
            m.insert("folder_from".into(), json!(f.from.as_str()));
            if let Some((id, name)) = &f.team {
                m.insert("team_id".into(), json!(id));
                m.insert("team_name".into(), json!(name));
            }
        }
        if let Some(k) = self.unsafe_kind {
            m.insert("unsafe_kind".into(), json!(k.as_str()));
        }
        Value::Object(m)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unsafe_folder_meta_matches_the_ui_contract() {
        let m = IsolationFailed::unsafe_folder("Ada", FolderInfo::team(Path::new("/home/oleks"), "t_1", "Checkout"), UnsafeKind::Home).to_meta();
        assert_eq!(
            m,
            json!({ "kind": "turn_isolation_failed", "kwant_name": "Ada", "engine": "claude", "reason": "unsafe_folder",
                    "phase": "start", "progress_pct": null, "code": null, "folder": "/home/oleks", "folder_from": "team",
                    "team_id": "t_1", "team_name": "Checkout", "unsafe_kind": "home" })
        );
    }

    #[test]
    fn folder_used_meta_has_no_unsafe_kind_and_no_team_outside_a_team() {
        let m = IsolationFailed::folder_used("Rex", "codex", FolderInfo::new(Path::new("/home/oleks/shop"), FolderFrom::Bot)).to_meta();
        assert_eq!(m["reason"], "folder_used_by_isolated_bot");
        assert_eq!(m["engine"], "codex");
        assert_eq!(m["folder_from"], "bot");
        for absent in ["unsafe_kind", "team_id", "team_name"] {
            assert!(m.get(absent).is_none(), "{absent}");
        }
    }

    #[test]
    fn saved_texts_use_miras_first_lines_and_never_claim_a_host_run() {
        assert!(start_text("Ada", "docker_unavailable").starts_with("Ada didn't reply: Claude's sealed box couldn't start. Docker isn't running"));
        assert!(start_text("Ada", "preparing").starts_with("Ada didn't reply: Claude's sealed box isn't ready yet."));
        assert!(start_text("Ada", "sign_in_expired").contains("Settings › This space › Accounts"));
        for r in ["docker_unavailable", "preparing", "download_failed", "relay_down", "container_failed", "host_is_root", "folder_record_failed", "unsupported_os"] {
            assert!(start_text("Ada", r).ends_with(SAFETY), "{r}");
        }
        // A code from a newer kwant still reads, naming the code
        assert!(start_text("Ada", "brand_new").contains("Something went wrong (brand_new)"));
        // Notice 3 has the safety line too (ui-check-446 fix 10)
        let mid = mid_turn_text("Ada");
        assert!(mid.starts_with("Ada's reply stopped partway: Claude's sealed box lost its connection."), "{mid}");
        assert!(mid.ends_with(SAFETY), "{mid}");
        let used = folder_used_text("Rex", "codex");
        assert!(used.starts_with("Rex didn't reply: Codex won't run in a folder Claude used in its sealed box."), "{used}");
    }

    #[test]
    fn every_refusal_code_has_its_own_words() {
        // ui-check-446 fix 4: no code kwant sends falls to "Something went wrong (code)"
        for &code in super::super::codes::ALL {
            if code == reason::FOLDER_USED_BY_ISOLATED_BOT {
                continue; // folder_used_text
            }
            let t = start_text("Ada", code);
            assert!(!t.contains("Something went wrong"), "{code}: {t}");
            assert!(t.starts_with("Ada didn't reply:"), "{code}: {t}");
            for never in ["kwnt", "in the computer", "shared with the computer"] {
                assert!(!t.contains(never), "{code}: {t}");
            }
        }
        let t = start_text("Ada", "no_credential");
        assert_eq!(
            t,
            "Ada didn't reply: there's no Claude sign-in for it on this computer. Sign in to Claude in Settings › This space › Accounts, then Retry."
        );
        assert!(start_text("Ada", "host_is_root").contains("kwant is running as root"));
        assert!(start_text("Ada", "folder_name_unsupported").contains("like a line break. Rename the folder, then Retry. Nothing ran."));
    }

    #[test]
    fn the_unsafe_folder_text_says_each_thing_once_with_miras_words() {
        let home = Path::new("/home/oleks");
        let bot = FolderInfo::new(Path::new("/home/oleks"), FolderFrom::Bot);
        let t = unsafe_folder_text("Ada", UnsafeKind::Home, &bot, Some(home));
        assert_eq!(
            t,
            "Ada didn't reply: Claude can't take this folder into its sealed box. ~ is your home folder. In its sealed box, Claude would see all your files and sign-ins. Pick a project folder instead, then Retry. Nothing ran."
        );
        assert_eq!(t.matches("Pick a project folder").count(), 1, "{t}");
        assert!(!t.contains("the computer"), "{t}");
        // A team's folder names the team; the path is shortened under home
        let team = FolderInfo::team(Path::new("/home/oleks/.ssh"), "t_1", "Checkout");
        let t = unsafe_folder_text("Ada", UnsafeKind::SignIns, &team, Some(home));
        assert!(t.contains("~/.ssh holds sign-ins, like your SSH keys"), "{t}");
        assert!(t.ends_with("Pick a project folder for @Checkout instead, then Retry. Nothing ran."), "{t}");
        // kwant's own shared folder: no path at all
        let shared = FolderInfo::team(Path::new("/home/oleks/.kwnt/workspace"), "t_1", "Checkout");
        let t = unsafe_folder_text("Ada", UnsafeKind::SharedFolder, &shared, Some(home));
        assert!(!t.contains("/") && !t.contains('~'), "{t}");
        assert!(t.ends_with("Give @Checkout its own folder, then Retry. Nothing ran."), "{t}");
        let alone = FolderInfo::new(Path::new("/home/oleks/.kwnt/workspace"), FolderFrom::Bot);
        assert!(unsafe_folder_text("Ada", UnsafeKind::SharedFolder, &alone, Some(home)).contains("Give this team its own folder, then Retry."));
        for (kind, words) in [
            (UnsafeKind::Root, "/ holds your home folder."),
            (UnsafeKind::KwantData, "holds kwant's own data, including your sign-ins"),
            (UnsafeKind::Docker, "holds Docker's control socket, which would let Claude out of its sealed box."),
        ] {
            let f = FolderInfo::new(Path::new(if kind == UnsafeKind::Root { "/" } else { "/srv/x" }), FolderFrom::Bot);
            let t = unsafe_folder_text("Ada", kind, &f, Some(home));
            assert!(t.contains(words), "{t}");
            assert_eq!(t.matches("Pick a project folder").count(), 1, "{t}");
        }
        assert_eq!(tilde("/home/oleksandr/x", Some(home)), "/home/oleksandr/x", "only whole folder names");
        assert_eq!(tilde("/home/oleks/site", Some(Path::new("/home/oleks/"))), "~/site");
        assert_eq!(tilde("/srv", None), "/srv");
    }

    #[test]
    fn a_reason_without_a_folder_sends_no_folder_fields() {
        let m = IsolationFailed::new("Ada", "claude", "docker_unavailable").to_meta();
        for absent in ["folder", "folder_from", "unsafe_kind"] {
            assert!(m.get(absent).is_none(), "{absent}");
        }
    }

    #[test]
    fn web_tools_are_merged_into_kwnts_own_disallowed_list() {
        let os = |v: &[&str]| v.iter().map(std::ffi::OsString::from).collect::<Vec<_>>();
        let merged = with_disallowed_tools(os(&["-p", "--disallowedTools", "CronCreate,WebFetch", "--model", "x"]));
        assert_eq!(merged, os(&["-p", "--disallowedTools", "CronCreate,WebFetch,WebSearch", "--model", "x"]));
        let added = with_disallowed_tools(os(&["-p"]));
        assert_eq!(added, os(&["-p", "--disallowedTools", "WebSearch,WebFetch"]));
        let [flag, list] = crate::drivers::cli::claude_disallowed_args();
        let real = with_disallowed_tools(os(&[&flag, &list]));
        assert_eq!(real.iter().filter(|a| *a == "--disallowedTools").count(), 1);
        let v = real[1].to_string_lossy();
        assert!(v.starts_with(list.as_str()) && v.contains("WebSearch") && v.contains("WebFetch"), "{v}");
    }

    #[test]
    fn broker_bodies_name_the_way_forward() {
        assert!(body::SERVER_TOOL.contains("browser tools"));
        assert_eq!(
            body::request_cap(200),
            "This reply reached its limit of 200 requests to Claude. kwant paused it, and the person can continue it from the chat."
        );
    }
}
