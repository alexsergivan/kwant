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

/// The chat text saved with a `turn_isolation_failed` notice that stopped Claude before it
/// started (Mira's notices 1, 2, 2b, 2c, 4, 6 and the unknown-code row). The card is drawn from
/// the meta; this is what a client without the card, and the chat history, show.
pub fn start_text(bot: &str, reason: &str, detail: &str) -> String {
    const SAFETY: &str = "While Run Claude inside the VM is on, Claude never runs outside it, so nothing ran on this computer.";
    let couldnt_start = format!("{bot} didn't reply: Claude couldn't start inside the VM.");
    let not_ready = format!("{bot} didn't reply: Claude's VM isn't ready yet.");
    match reason {
        "docker_unavailable" => format!("{couldnt_start} Docker isn't running on this computer. Start Docker, then Retry. {SAFETY}"),
        "preparing" => format!("{not_ready} The one-time download is still running. Retry when it's done. {SAFETY}"),
        "download_failed" | "download_needed" => {
            format!("{not_ready} The one-time download didn't finish. Try it again in Computer settings, then Retry. {SAFETY}")
        }
        "relay_down" => format!("{couldnt_start} kwant couldn't connect Claude to your sign-in. Restart kwant, then Retry. {SAFETY}"),
        "sign_in_expired" => format!("{bot} didn't reply: your Claude sign-in has expired. Sign in again in Settings › This space › Accounts, then Retry."),
        reason::UNSAFE_FOLDER => format!(
            "{bot} didn't reply: Claude can't use this folder inside the VM. {}. Pick a project folder instead, then Retry. Nothing ran.",
            detail.trim_end_matches('.')
        ),
        code => format!(
            "{couldnt_start} Something went wrong ({code}). Retry, or turn off Run Claude inside the VM to run Claude on this computer. {SAFETY}"
        ),
    }
}

/// The chat text saved with notice 7 (`folder_used_by_isolated_bot`). `engine`: `codex` or `grok`.
pub fn folder_used_text(bot: &str, engine: &str) -> String {
    let engine = match engine {
        "codex" => "Codex",
        "grok" => "Grok",
        other => other,
    };
    format!(
        "{bot} didn't reply: {engine} won't run in a folder Claude used inside the VM. A bot inside the VM could have left settings there that {engine} would load on this computer, so kwant keeps {engine} out of it for now. Switch {bot} to Claude, or pick another folder, then Retry. Nothing ran."
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
        assert!(start_text("Ada", "docker_unavailable", "").starts_with("Ada didn't reply: Claude couldn't start inside the VM. Docker isn't running"));
        assert!(start_text("Ada", "preparing", "").starts_with("Ada didn't reply: Claude's VM isn't ready yet."));
        assert!(start_text("Ada", "sign_in_expired", "").contains("Settings › This space › Accounts"));
        assert!(start_text("Ada", "container_failed", "").contains("Something went wrong (container_failed)"));
        for r in ["docker_unavailable", "preparing", "download_failed", "relay_down", "container_failed"] {
            assert!(start_text("Ada", r, "").contains("nothing ran on this computer"), "{r}");
        }
        let used = folder_used_text("Rex", "codex");
        assert!(used.starts_with("Rex didn't reply: Codex won't run in a folder Claude used inside the VM."), "{used}");
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
