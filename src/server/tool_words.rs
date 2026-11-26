//! What a tool does, in plain words.
//!
//! - [`action`]: the words of an approval ask (33g), after "<bot> wants to": "run a command".
//! - [`doing`]: the "-ing" form (33n), after "in the middle of": "running a command",
//!   "sending a message on Slack".
//!
//! Both mirror `frontend/src/toolLabel.ts`: [`VERBS`] is its `VERBS` table ([`doing`] is form 0,
//! `toolLabel`; [`action`] is form 1, `toolAction`), [`SERVERS`] its `SERVERS`, and a tool it
//! doesn't know falls back to "using {spaced name}" (+ " ({server})"), as there. Keep them in step
//! so the in-app card and strip, the phone push, Telegram and the chat's notes say the same thing.
//! `tests/fixtures/tool_words.json` is the one list of names and words both sides are tested on (33u):
//! the test below and `frontend/tests/toolWordsAgree.test.mjs`.
//! [`action`] covers built-in tools only: an MCP tool's ask keeps naming itself and its server.

/// How a few well-known servers are written (toolLabel.ts `SERVERS`). Others are shown as they come.
const SERVERS: &[(&str, &str)] = &[
    ("github", "GitHub"),
    ("gitlab", "GitLab"),
    ("linear", "Linear"),
    ("slack", "Slack"),
    ("notion", "Notion"),
    ("jira", "Jira"),
    ("stripe", "Stripe"),
    ("gmail", "Gmail"),
    ("googlecalendar", "Google Calendar"),
    ("google_calendar", "Google Calendar"),
    ("googledrive", "Google Drive"),
    ("google_drive", "Google Drive"),
    ("home-assistant", "Home Assistant"),
    ("homeassistant", "Home Assistant"),
];

/// (tool, the "-ing" form, the "wants to" form, whether "on <server>" follows).
type Verb = (&'static str, &'static str, &'static str, bool);

/// toolLabel.ts `VERBS`.
const VERBS: &[Verb] = &[
    ("create_pull_request", "opening a pull request", "open a pull request", true),
    ("merge_pull_request", "merging a pull request", "merge a pull request", true),
    ("create_issue", "opening an issue", "open an issue", true),
    ("add_issue_comment", "commenting on an issue", "comment on an issue", true),
    ("create_branch", "creating a branch", "create a branch", true),
    ("push_files", "pushing files", "push files", true),
    ("create_or_update_file", "changing a file", "change a file", true),
    ("send_message", "sending a message", "send a message", true),
    ("send_email", "sending an email", "send an email", true),
    ("browser_navigate", "opening a page", "open a page", false),
    ("browser_open", "opening a page", "open a page", false),
    ("open_url", "opening a page", "open a page", false),
    ("web_fetch", "opening a page", "open a page", false),
    ("webfetch", "opening a page", "open a page", false),
    ("browser_click", "using the browser", "use the browser", false),
    ("browser_type", "using the browser", "use the browser", false),
    ("browser_act", "using the browser", "use the browser", false),
    ("browser_upload", "uploading a file to a website", "upload a file to a website", false),
    ("browser_snapshot", "reading the browser page", "read the browser page", false),
    ("browser_screenshot", "looking at the browser page", "look at the browser page", false),
    ("ask_user", "asking you a question", "ask you a question", false),
    ("request_human_step", "asking you to do a step in the browser", "ask you to do a step in the browser", false),
    ("web_search", "searching the web", "search the web", false),
    ("websearch", "searching the web", "search the web", false),
    ("bash", "running a command", "run a command", false),
    ("execute_shell", "running a command on your computer", "run a command on your computer", false),
    ("read_file", "reading a file", "read a file", false),
    ("write_file", "writing a file", "write a file", false),
    ("edit_file", "changing a file", "change a file", false),
    // specific-step-titles: Claude Code's and Codex's other built-ins, so no step reads "using ToolSearch"
    ("toolsearch", "finding the right tool", "find the right tool", false),
    ("todowrite", "updating its to-do list", "update its to-do list", false),
    ("update_plan", "updating its plan", "update its plan", false),
    ("exitplanmode", "sharing its plan", "share its plan", false),
    ("search_files", "searching the files", "search the files", false),
    ("find_files", "looking for files", "look for files", false),
    ("list_dir", "listing files", "list files", false),
    ("ask_helper", "asking a helper", "ask a helper", false),
    ("notebookedit", "changing a notebook", "change a notebook", false),
    ("apply_patch", "changing files", "change files", false),
    ("bashoutput", "checking a running command", "check a running command", false),
    ("killshell", "stopping a running command", "stop a running command", false),
    ("killbash", "stopping a running command", "stop a running command", false),
    ("use_skill", "using a skill", "use a skill", false),
];

/// Claude's own built-in names and the key they read as (toolLabel.ts `BUILT_IN_ALIASES`). Only
/// for a built-in: an MCP server's own `read` tool isn't claude's Read.
/// specific-step-titles: `glob`, `grep`, `ls`, `task`, `agent` and `skill` too, names a server's
/// own tool could share.
const BUILT_IN_ALIASES: &[(&str, &str)] = &[
    ("read", "read_file"),
    ("write", "write_file"),
    ("edit", "edit_file"),
    ("multiedit", "edit_file"),
    ("glob", "find_files"),
    ("grep", "search_files"),
    ("ls", "list_dir"),
    ("task", "ask_helper"),
    ("agent", "ask_helper"),
    ("skill", "use_skill"),
];

/// Claude Code's bundled browser tools, `agent_browser_<x>` (seen as
/// `mcp__browser__agent_browser_snapshot`), 33u: `<x>` reads as `browser_<x>` when that key exists,
/// else as the key here, else as `browser_act` ("using the browser"). Never the raw name.
/// toolLabel.ts `AGENT_BROWSER`.
const AGENT_BROWSER: &[(&str, &str)] = &[
    ("back", "browser_open"),
    ("forward", "browser_open"),
    ("reload", "browser_open"),
    ("read", "browser_snapshot"),
    ("get_text", "browser_snapshot"),
    ("get_title", "browser_snapshot"),
    ("get_url", "browser_snapshot"),
];

/// The built-in tools an approval ask names in words ([`action`]).
const BUILT_INS: &[&str] = &[
    "bash", "execute_shell", "read", "read_file", "write", "write_file", "edit", "multiedit", "edit_file", "webfetch", "web_fetch",
    "websearch", "web_search",
];

/// toolLabel.ts `split`: `mcp__github__create_pull_request` → the tool and its server; other
/// names as given. A blank server is no server.
fn split<'a>(name: &'a str, server: Option<&'a str>) -> (&'a str, Option<&'a str>) {
    let server = server.filter(|s| !s.trim().is_empty());
    if let Some(rest) = name.strip_prefix("mcp__") {
        if let Some(i) = rest.find("__").filter(|&i| i > 0 && i + 2 < rest.len()) {
            return (&rest[i + 2..], server.or(Some(&rest[..i])));
        }
    }
    (name, server)
}

fn verb(tool: &str, built_in: bool) -> Option<&'static Verb> {
    let lower = tool.trim().to_ascii_lowercase();
    let find = |key: &str| VERBS.iter().find(|v| v.0 == key);
    if let Some((_, key)) = BUILT_IN_ALIASES.iter().find(|(a, _)| built_in && *a == lower) {
        return find(key);
    }
    if let Some(v) = find(&lower) {
        return Some(v);
    }
    let x = lower.strip_prefix("agent_browser_")?;
    find(&format!("browser_{x}"))
        .or_else(|| AGENT_BROWSER.iter().find(|(a, _)| *a == x).and_then(|(_, key)| find(key)))
        .or_else(|| find("browser_act"))
}

fn server_name(s: &str) -> String {
    SERVERS.iter().find(|(k, _)| k.eq_ignore_ascii_case(s)).map(|(_, v)| v.to_string()).unwrap_or_else(|| plain(s, false))
}

/// A name as plain text in a chat note: toolLabel.ts `spaced` (`_` and `-` read as spaces) for a
/// tool, and only letters, digits, `-`, `_` and `.` kept, so a strange name can't bring markdown
/// (a backtick, a link) into the note.
fn plain(s: &str, spaced: bool) -> String {
    let kept: String = s
        .chars()
        .map(|c| match c {
            '_' | '-' if spaced => ' ',
            c if c.is_alphanumeric() || matches!(c, '-' | '_' | '.') => c,
            _ => ' ',
        })
        .collect();
    kept.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// The plain words after "<bot> wants to" for built-in tool `tool` (matched case-insensitively,
/// like toolLabel.ts): `Bash` → "run a command". `None` for an MCP tool (the card's `source` is
/// "mcp", a `server`, or an `mcp__<server>__<tool>` name) and for a tool kwant has no words for:
/// the caller names it. `source` is the card's `details.source`, so an MCP tool named `Bash` with
/// no server never reads as claude's Bash (33u).
pub fn action(tool: &str, server: Option<&str>, source: Option<&str>) -> Option<&'static str> {
    if source == Some("mcp") || server.is_some_and(|s| !s.trim().is_empty()) || tool.starts_with("mcp__") {
        return None;
    }
    if !BUILT_INS.contains(&tool.trim().to_ascii_lowercase().as_str()) {
        return None;
    }
    verb(tool, true).map(|v| v.2)
}

/// What a bot was doing with a tool, after "in the middle of" (toolLabel.ts form 0): `Bash` →
/// "running a command", `mcp__slack__send_message` → "sending a message on Slack", an unknown
/// `mcp__srv__foo_bar` → "using foo bar (srv)". Never empty, never a backtick.
pub fn doing(tool: &str, server: Option<&str>) -> String {
    let (name, server) = split(tool, server);
    if let Some(&(_, ing, _, on_server)) = verb(name, server.is_none()) {
        return match server {
            Some(s) if on_server => format!("{ing} on {}", server_name(s)),
            _ => ing.to_string(),
        };
    }
    let name = match plain(name, true) {
        n if n.is_empty() => "a tool".to_string(),
        n => n,
    };
    match server.map(|s| plain(s, false)).filter(|s| !s.is_empty()) {
        Some(s) => format!("using {name} ({s})"),
        None => format!("using {name}"),
    }
}

#[cfg(test)]
mod tests {
    use super::{action, doing, BUILT_INS};
    use serde_json::Value;

    #[test]
    fn built_in_tools_read_as_what_they_do() {
        for (tool, words) in [
            ("Bash", "run a command"),
            ("bash", "run a command"),
            ("execute_shell", "run a command on your computer"),
            ("Read", "read a file"),
            ("Write", "write a file"),
            ("write_file", "write a file"),
            ("Edit", "change a file"),
            ("MultiEdit", "change a file"),
            ("edit_file", "change a file"),
            ("WebFetch", "open a page"),
            ("web_fetch", "open a page"),
            ("WebSearch", "search the web"),
            ("websearch", "search the web"),
        ] {
            assert_eq!(action(tool, None, None), Some(words), "{tool}");
            assert_eq!(action(tool, Some(""), None), Some(words), "{tool}: a blank server is no server");
            assert_eq!(action(tool, None, Some("claude_builtin")), Some(words), "{tool}");
        }
    }

    #[test]
    fn mcp_and_unknown_tools_keep_their_names() {
        assert_eq!(action("Foo", None, None), None);
        assert_eq!(action("", None, None), None);
        assert_eq!(action("create_issue", Some("github"), None), None);
        // A shell tool on an MCP server is that server's, not claude's Bash
        assert_eq!(action("bash", Some("shell"), None), None);
        assert_eq!(action("mcp__shell__bash", None, None), None);
        // 33u (33g QA Finding 3): the card's source decides too, so an MCP card named `Bash` with
        // an empty or no server isn't claude's Bash either
        assert_eq!(action("Bash", Some(""), Some("mcp")), None);
        assert_eq!(action("Bash", None, Some("mcp")), None);
        assert_eq!(action("Write", None, Some("mcp")), None);
        // A local shell card is a command, as 33g's Telegram tests pin
        assert_eq!(action("Bash", None, Some("local_shell")), Some("run a command"));
        // toolLabel.ts knows these, but an approval ask names only built-ins in words
        assert_eq!(action("send_message", None, None), None);
        assert_eq!(action("browser_navigate", None, None), None);
    }

    /// 33n: the "-ing" form, as toolLabel.ts form 0 says it.
    #[test]
    fn doing_reads_as_the_working_strip_says_it() {
        for (tool, server, words) in [
            ("Bash", None, "running a command"),
            ("execute_shell", None, "running a command on your computer"),
            ("Read", None, "reading a file"),
            ("read_file", None, "reading a file"),
            ("Write", None, "writing a file"),
            ("Edit", None, "changing a file"),
            ("MultiEdit", None, "changing a file"),
            ("WebFetch", None, "opening a page"),
            ("WebSearch", None, "searching the web"),
            ("mcp__slack__send_message", None, "sending a message on Slack"),
            ("send_message", Some("slack"), "sending a message on Slack"),
            ("mcp__github__create_pull_request", None, "opening a pull request on GitHub"),
            ("mcp__gmail__send_email", None, "sending an email on Gmail"),
            ("mcp__acme__send_message", None, "sending a message on acme"),
            ("mcp__home-assistant__send_message", None, "sending a message on Home Assistant"),
            ("send_message", None, "sending a message"),
            ("mcp__browser__browser_navigate", None, "opening a page"),
            ("mcp__kwnt_tools__ask_user", None, "asking you a question"),
            // toolLabel.ts matches an MCP tool's name too, as here
            ("mcp__shell__bash", None, "running a command"),
            // Unknown: the spaced name, and the server as it comes
            ("mcp__srv__foo_bar", None, "using foo bar (srv)"),
            // 33u: Claude Code's browser tools read as the browser does, never as their raw name
            ("mcp__browser__agent_browser_snapshot", None, "reading the browser page"),
            ("mcp__browser__agent_browser_open", None, "opening a page"),
            ("mcp__browser__agent_browser_tab_new", None, "using the browser"),
            // specific-step-titles: Claude Code's other built-ins read as what they do too
            ("Task", None, "asking a helper"),
            ("Agent", None, "asking a helper"),
            ("ToolSearch", None, "finding the right tool"),
            ("TodoWrite", None, "updating its to-do list"),
            ("Glob", None, "looking for files"),
            ("Grep", None, "searching the files"),
            ("LS", None, "listing files"),
            ("NotebookEdit", None, "changing a notebook"),
            ("BashOutput", None, "checking a running command"),
            ("KillShell", None, "stopping a running command"),
            ("apply_patch", None, "changing files"),
            ("update_plan", None, "updating its plan"),
            // ...but a server's own `task` or `grep` keeps its name
            ("mcp__linear__task", None, "using task (linear)"),
            ("grep", Some("fs"), "using grep (fs)"),
            ("apply-patch", None, "using apply patch"),
            ("", None, "using a tool"),
            ("mcp__x____", None, "using a tool (x)"),
            // An MCP server's own `read` isn't claude's Read
            ("mcp__fs__read", None, "using read (fs)"),
        ] {
            assert_eq!(doing(tool, server), words, "{tool} {server:?}");
        }
    }

    #[test]
    fn every_built_in_ask_has_its_ing_form() {
        for tool in BUILT_INS {
            assert!(action(tool, None, None).is_some(), "{tool}");
            let ing = doing(tool, None);
            assert!(!ing.starts_with("using "), "{tool} reads as {ing}");
            // The two forms are one verb: "run a command" / "running a command"
            let base = action(tool, None, None).unwrap();
            assert_eq!(ing.split(' ').skip(1).collect::<Vec<_>>(), base.split(' ').skip(1).collect::<Vec<_>>(), "{tool}");
        }
    }

    #[test]
    fn a_strange_name_brings_no_markdown_into_a_note() {
        for tool in ["mcp__x__`rm -rf`", "mcp__[a](http://e)__b", "**bold**", "mcp__s__<b>x</b>"] {
            let words = doing(tool, None);
            assert!(!words.contains(['`', '[', ']', '*', '<', '>']), "{tool} → {words}");
            assert!(words.starts_with("using "), "{tool} → {words}");
        }
        assert_eq!(doing("mcp__x__`rm -rf`", None), "using rm rf (x)");
    }

    /// 33u: the one list of names and words that `frontend/tests/toolWordsAgree.test.mjs` walks
    /// too, so the in-app card and strip (toolLabel.ts) and the push, Telegram and restart notes
    /// (here) say the same step in the same words. `ing` is toolLabel.ts form 0 and [`doing`];
    /// `base` is form 1, and [`action`] says it when `push` is true (built-ins only), else names the tool.
    #[test]
    fn the_shared_list_reads_the_same_here_as_in_the_app() {
        let cases: Vec<Value> = serde_json::from_str(include_str!("../../tests/fixtures/tool_words.json")).unwrap();
        assert!(cases.len() >= 30, "{}", cases.len());
        for c in &cases {
            let tool = c["tool"].as_str().unwrap();
            let server = c["server"].as_str();
            let (ing, base) = (c["ing"].as_str().unwrap(), c["base"].as_str().unwrap());
            assert_eq!(doing(tool, server), ing, "{tool} {server:?}");
            let push = c["push"].as_bool().unwrap();
            assert_eq!(action(tool, server, c["source"].as_str()), push.then_some(base), "{tool} {server:?} {:?}", c["source"]);
        }
    }
}
