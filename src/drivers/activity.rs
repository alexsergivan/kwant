//! Live activity: what a bot is doing during a turn, as a short list of steps (thinking, a tool
//! run, a wait for an approval or for the person). Each change to a step is broadcast as an
//! `activity` event while the turn runs, and the whole list is kept on the reply's
//! `meta.activity` so it still shows after a reload. See
//! `docs/live-activity.md` for the wire shapes.
//!
//! Titles and details are made for people ("Running `cargo test`", "Opening github.com") and are
//! redacted: known secret values, `key=value` credentials, bearer tokens and provider-looking keys
//! never leave this module.

use super::events::RuntimeEvent;
use serde::Serialize;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::time::Instant;

/// Most steps kept on one reply; later ones are counted in `dropped`.
pub const MAX_STEPS: usize = 200;
/// Most thinking text kept on one reply, over all its thinking steps (characters).
pub const MAX_THINKING_CHARS: usize = 16_000;
/// Longest step title (characters).
pub const MAX_TITLE: usize = 160;
/// Longest step detail (characters).
pub const MAX_DETAIL: usize = 500;

/// One thing the bot did, or is doing, in a turn.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct Step {
    /// `s1`, `s2`, … in the order the steps started; unique within the turn.
    pub id: String,
    /// "thinking", "tool", "approval", "human_step" or "note".
    pub kind: &'static str,
    /// "running", "waiting", "ok", "failed", "done", "declined" or "stopped".
    pub status: &'static str,
    pub title: String,
    /// The tool's own name (`Bash`, `mcp__github__create_issue`), for tool and approval steps.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool: Option<String>,
    /// More about the step: the full command or address, or why a tool failed.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
    /// A thinking step's text, once the step ends (it streams as `thinking_delta` before that).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    /// The approval ticket or human-step id the step waits on.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ref_id: Option<String>,
    pub started_at: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ended_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub duration_ms: Option<u64>,
    #[serde(skip)]
    clock: Option<Instant>,
}

impl Step {
    pub fn to_json(&self) -> Value {
        serde_json::to_value(self).unwrap_or(Value::Null)
    }

    fn is_open(&self) -> bool {
        matches!(self.status, "running" | "waiting")
    }
}

/// The steps of one turn.
#[derive(Debug, Default)]
pub struct ActivityLog {
    steps: Vec<Step>,
    /// Tool call id → index in `steps`.
    tools: HashMap<String, usize>,
    /// The thinking step still taking text.
    thinking: Option<usize>,
    thinking_chars: usize,
    dropped: usize,
    secrets: Vec<String>,
}

fn now() -> String {
    chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
}

impl ActivityLog {
    /// A log that also masks these exact values (the turn's MCP secrets).
    pub fn with_secrets(secrets: Vec<String>) -> Self {
        Self { secrets, ..Default::default() }
    }

    /// Masks these values too, from now on.
    pub fn add_secrets(&mut self, secrets: impl IntoIterator<Item = String>) {
        self.secrets.extend(secrets.into_iter().filter(|s| !s.is_empty()));
    }

    pub fn is_empty(&self) -> bool {
        self.steps.is_empty()
    }

    pub fn steps(&self) -> &[Step] {
        &self.steps
    }

    /// Masks secrets in text shown on a step.
    pub fn redact(&self, text: &str) -> String {
        redact(text, &self.secrets)
    }

    fn push(&mut self, kind: &'static str, status: &'static str, title: String) -> Option<usize> {
        if self.steps.len() >= MAX_STEPS {
            self.dropped += 1;
            return None;
        }
        self.steps.push(Step {
            id: format!("s{}", self.steps.len() + 1),
            kind,
            status,
            title: cut(&self.redact(&title), MAX_TITLE),
            tool: None,
            detail: None,
            text: None,
            ref_id: None,
            started_at: now(),
            ended_at: None,
            duration_ms: None,
            clock: Some(Instant::now()),
        });
        Some(self.steps.len() - 1)
    }

    fn end(&mut self, i: usize, status: &'static str) -> Step {
        let step = &mut self.steps[i];
        step.status = status;
        step.ended_at = Some(now());
        step.duration_ms = step.clock.map(|c| c.elapsed().as_millis() as u64);
        step.clone()
    }

    /// Ends the open thinking step, if any.
    fn close_thinking(&mut self) -> Option<Step> {
        let i = self.thinking.take()?;
        if let Some(text) = self.steps[i].text.take() {
            self.steps[i].text = Some(self.redact(&text));
        }
        Some(self.end(i, "ok"))
    }

    /// Takes one event a driver sent and returns the steps it started or changed, ready to
    /// broadcast. Thinking text only starts a step: the text itself streams as `thinking_delta`
    /// and is kept on the step when it ends.
    pub fn observe(&mut self, event: &RuntimeEvent) -> Vec<Step> {
        let mut changed = Vec::new();
        match event.unscoped() {
            RuntimeEvent::ThinkingDelta(text) => {
                if text.trim().is_empty() && self.thinking.is_none() {
                    return changed;
                }
                let i = match self.thinking {
                    Some(i) => i,
                    None => {
                        let Some(i) = self.push("thinking", "running", "Thinking".into()) else {
                            return changed;
                        };
                        self.thinking = Some(i);
                        changed.push(self.steps[i].clone());
                        i
                    }
                };
                let room = MAX_THINKING_CHARS.saturating_sub(self.thinking_chars);
                if room > 0 {
                    let kept: String = text.chars().take(room).collect();
                    self.thinking_chars += kept.chars().count();
                    self.steps[i].text.get_or_insert_with(String::new).push_str(&kept);
                }
            }
            RuntimeEvent::TextDelta(text) if !text.trim().is_empty() => {
                changed.extend(self.close_thinking());
            }
            RuntimeEvent::ToolCallPending { id, name, args } => {
                changed.extend(self.close_thinking());
                let (title, detail) = summarize(name, args);
                if let Some(i) = self.push("tool", "running", title) {
                    self.steps[i].tool = Some(name.clone());
                    self.steps[i].detail = detail.map(|d| cut(&self.redact(&d), MAX_DETAIL));
                    self.tools.insert(id.clone(), i);
                    changed.push(self.steps[i].clone());
                }
            }
            RuntimeEvent::ToolCallCompleted { id, result } => {
                changed.extend(self.close_thinking());
                if let Some(i) = self.tools.remove(id) {
                    let failed = matches!(result["status"].as_str(), Some("error" | "failed" | "denied" | "cancelled"));
                    if failed {
                        let why = ["error", "content", "message"]
                            .iter()
                            .find_map(|k| result[*k].as_str().filter(|s| !s.trim().is_empty()));
                        if let Some(why) = why {
                            self.steps[i].detail = Some(cut(&self.redact(why.trim()), MAX_DETAIL));
                        }
                    }
                    changed.push(self.end(i, if failed { "failed" } else { "ok" }));
                }
            }
            _ => {}
        }
        changed
    }

    /// A note from kwnt itself, such as which engine can't show its thinking.
    pub fn note(&mut self, title: &str) -> Option<Step> {
        let i = self.push("note", "ok", title.to_string())?;
        Some(self.end(i, "ok"))
    }

    /// A step that waits on the person: an approval card (`kind` "approval") or a human step.
    /// Returns the steps to broadcast: the thinking step it ended, if any, then the new one.
    pub fn wait(&mut self, kind: &'static str, ref_id: &str, title: &str, tool: Option<&str>) -> Vec<Step> {
        let mut changed: Vec<Step> = self.close_thinking().into_iter().collect();
        if let Some(i) = self.push(kind, "waiting", title.to_string()) {
            self.steps[i].ref_id = Some(ref_id.to_string());
            self.steps[i].tool = tool.map(str::to_string);
            changed.push(self.steps[i].clone());
        }
        changed
    }

    /// Ends the waiting step for `ref_id` with `status` ("ok", "done", "declined", "stopped").
    pub fn settle(&mut self, ref_id: &str, status: &'static str) -> Option<Step> {
        let i = self
            .steps
            .iter()
            .rposition(|s| s.ref_id.as_deref() == Some(ref_id) && s.is_open())?;
        Some(self.end(i, status))
    }

    /// Ends the turn: thinking closes, and anything still running or waiting is marked "stopped".
    pub fn finish(&mut self) -> Vec<Step> {
        let mut changed: Vec<Step> = self.close_thinking().into_iter().collect();
        self.tools.clear();
        for i in 0..self.steps.len() {
            if self.steps[i].is_open() {
                changed.push(self.end(i, "stopped"));
            }
        }
        changed
    }

    /// What is saved on the reply as `meta.activity`.
    pub fn to_json(&self) -> Value {
        let mut v = json!({ "steps": self.steps.iter().map(Step::to_json).collect::<Vec<_>>() });
        if self.dropped > 0 {
            v["dropped"] = json!(self.dropped);
        }
        v
    }
}

/// At most `max` characters, with `…` when cut.
fn cut(text: &str, max: usize) -> String {
    match text.char_indices().nth(max.saturating_sub(1)) {
        Some((idx, _)) if text.chars().count() > max => format!("{}…", &text[..idx]),
        _ => text.to_string(),
    }
}

/// Prefixes of provider keys and tokens that are hidden whole wherever they appear.
const TOKEN_PREFIXES: [&str; 12] = [
    "sk-", "xai-", "ghp_", "gho_", "ghu_", "ghs_", "ghr_", "github_pat_", "glpat-", "AIza", "hf_", "npm_",
];

/// Masks secrets in text shown to people: the exact `secrets`, then credentials written as
/// `key=value`, `Bearer …`, URL passwords and provider-looking tokens.
pub fn redact(text: &str, secrets: &[String]) -> String {
    let text = crate::mcp::redact(text, secrets);
    let text = crate::profiles::classify::mask_credentials(&text);
    let mut out = String::with_capacity(text.len());
    let mut word_start = None;
    let is_token_char = |c: char| c.is_ascii_alphanumeric() || "-_".contains(c);
    let flush = |out: &mut String, word: &str| {
        let secret = word.len() >= 16 && TOKEN_PREFIXES.iter().any(|p| word.starts_with(p) && word.len() > p.len() + 8);
        out.push_str(if secret { "[redacted]" } else { word });
    };
    for (i, c) in text.char_indices() {
        match (is_token_char(c), word_start) {
            (true, None) => word_start = Some(i),
            (false, Some(s)) => {
                flush(&mut out, &text[s..i]);
                word_start = None;
                out.push(c);
            }
            (false, None) => out.push(c),
            (true, Some(_)) => {}
        }
    }
    if let Some(s) = word_start {
        flush(&mut out, &text[s..]);
    }
    out
}

/// Replaces home folders in text shown to other people and bots with `~`: this computer's home
/// and anything shaped like `/home/<name>`, `/Users/<name>` or `C:\Users\<name>`, so a username
/// doesn't leak.
pub fn hide_home_paths(text: &str) -> String {
    let mut text = text.to_string();
    // A home elsewhere (`/root`, `/var/lib/kwant`); only whole folders, so `/rooted` stays
    if let Some(home) = dirs::home_dir().and_then(|h| h.to_str().map(|h| h.trim_end_matches('/').to_string())) {
        if home.len() > 1 {
            text = text.replace(&format!("{home}/"), "~/");
        }
    }
    let mut out = String::with_capacity(text.len());
    let mut rest = text.as_str();
    loop {
        let unix = ["/home/", "/Users/"].iter().filter_map(|p| rest.find(p).map(|i| (i, p.len()))).min();
        let Some((at, len)) = [unix, windows_users_folder(rest)].into_iter().flatten().min() else {
            break;
        };
        let quote = rest[..at].chars().next_back().filter(|c| "\"'`".contains(*c));
        let name_len = home_name_len(&rest[at + len..], quote);
        // `/home/` on its own (or inside a longer word like `/x/home/`) isn't someone's home
        let starts_path = at == 0 || !rest[..at].ends_with(|c: char| c.is_alphanumeric() || "_-.".contains(c));
        if name_len == 0 || !starts_path {
            out.push_str(&rest[..at + len]);
            rest = &rest[at + len..];
            continue;
        }
        out.push_str(&rest[..at]);
        out.push('~');
        rest = &rest[at + len + name_len..];
    }
    out.push_str(rest);
    out
}

/// The length of the user name at the start of `s`, just after a home's `Users/`. A name with
/// spaces (`Oleks Smith`, up to three words) counts only when the path goes on after it or the
/// quote it opened with closes, so `/home/ann and more` stays `~ and more`.
fn home_name_len(s: &str, quote: Option<char>) -> usize {
    let word_end = |s: &str| s.find(|c: char| c == '/' || c == '\\' || c.is_whitespace() || "\"'`)]>:,;".contains(c)).unwrap_or(s.len());
    let first = word_end(s);
    let mut len = first;
    for _ in 0..2 {
        if first == 0 || !s[len..].starts_with(' ') {
            break;
        }
        let word = word_end(&s[len + 1..]);
        if word == 0 {
            break;
        }
        len += 1 + word;
        let next = s[len..].chars().next();
        if matches!(next, Some('/' | '\\')) || (next.is_some() && next == quote) {
            return len;
        }
    }
    first
}

/// Where the first Windows users folder (`C:\Users\`, `c:/users/`, or JSON-escaped
/// `C:\\Users\\`) starts in `text`, and its length up to the user name.
fn windows_users_folder(text: &str) -> Option<(usize, usize)> {
    let b = text.as_bytes();
    let separators = |mut j: usize| {
        while j < b.len() && (b[j] == b'\\' || b[j] == b'/') {
            j += 1;
        }
        j
    };
    let mut from = 0;
    while let Some(off) = text[from..].find(':') {
        let colon = from + off;
        from = colon + 1;
        // A drive letter on its own: `C:`, not the end of a word like `http:`
        if colon == 0 || !b[colon - 1].is_ascii_alphabetic() || (colon >= 2 && (b[colon - 2].is_ascii_alphanumeric() || b[colon - 2] == b'_')) {
            continue;
        }
        let users = separators(colon + 1);
        if users == colon + 1 || !b[users..].get(..5).is_some_and(|w| w.eq_ignore_ascii_case(b"users")) {
            continue;
        }
        let name = separators(users + 5);
        if name == users + 5 {
            continue;
        }
        return Some((colon - 1, name - (colon - 1)));
    }
    None
}

fn arg<'a>(args: &'a Value, keys: &[&str]) -> Option<&'a str> {
    keys.iter().find_map(|k| args.get(*k).and_then(Value::as_str)).filter(|s| !s.trim().is_empty())
}

/// The host of a URL, or the text itself when it isn't one.
fn host_of(url: &str) -> String {
    url::Url::parse(url)
        .ok()
        .and_then(|u| u.host_str().map(|h| h.trim_start_matches("www.").to_string()))
        .unwrap_or_else(|| url.to_string())
}

/// The last two parts of a path: enough to recognise the file without the whole folder.
fn short_path(path: &str) -> String {
    let parts: Vec<&str> = path.trim_end_matches('/').rsplit('/').take(2).collect();
    let short = parts.into_iter().rev().collect::<Vec<_>>().join("/");
    if short.len() < path.trim_end_matches('/').len() {
        format!("…/{short}")
    } else {
        short
    }
}

/// A command as one line: the array form joined, a `bash -lc '…'` wrapper unwrapped.
fn command_line(args: &Value) -> Option<String> {
    let raw = match args.get("command").or_else(|| args.get("cmd")) {
        Some(Value::String(s)) => s.clone(),
        Some(Value::Array(parts)) => {
            let parts: Vec<&str> = parts.iter().filter_map(Value::as_str).collect();
            match parts.as_slice() {
                [shell, flag, script] if shell.ends_with("sh") && flag.starts_with('-') && flag.contains('c') => {
                    script.to_string()
                }
                _ => parts.join(" "),
            }
        }
        _ => return None,
    };
    let line = raw.split_whitespace().collect::<Vec<_>>().join(" ");
    // codex prints its commands as `bash -lc '…'`
    let line = ["bash -lc ", "/bin/bash -lc ", "zsh -lc ", "/bin/zsh -lc ", "sh -c "]
        .iter()
        .find_map(|p| line.strip_prefix(p))
        .map(|rest| rest.trim_matches(|c| c == '\'' || c == '"').to_string())
        .unwrap_or(line);
    (!line.is_empty()).then_some(line)
}

/// A short title for a tool call, and the detail to show when the step is opened (the whole
/// command, address or query). Neither is redacted here.
pub fn summarize(name: &str, args: &Value) -> (String, Option<String>) {
    // `mcp__server__tool`: kwnt's own tools read as built-ins, others name their server
    let (server, tool) = match name.strip_prefix("mcp__").and_then(|r| r.split_once("__")) {
        Some(("kwnt" | "kwant", t)) => (None, t),
        Some((s, t)) => (Some(s), t),
        None => (None, name),
    };
    let lower = tool.to_ascii_lowercase();
    let path = || arg(args, &["file_path", "path", "filePath", "notebook_path", "target_file"]).map(short_path);
    let url = arg(args, &["url", "href", "uri"]);
    match lower.as_str() {
        "bash" | "shell" | "exec" | "exec_command" | "command_execution" | "run_terminal_cmd" | "run_shell_command"
        | "local_shell" => match command_line(args) {
            Some(cmd) => (format!("Running `{}`", cut(&cmd, 80)), Some(cmd)),
            None => ("Running a command".into(), None),
        },
        "read" | "read_file" | "view" | "cat" => (path().map_or("Reading a file".into(), |p| format!("Reading {p}")), None),
        "edit" | "multiedit" | "edit_file" | "str_replace" | "str_replace_based_edit_tool" | "apply_patch" | "file_change"
        | "notebookedit" => (path().map_or("Editing files".into(), |p| format!("Editing {p}")), None),
        "write" | "write_file" | "create_file" => (path().map_or("Writing a file".into(), |p| format!("Writing {p}")), None),
        "glob" | "grep" | "search_files" | "list_dir" | "ls" | "find" => {
            match arg(args, &["pattern", "query", "glob", "path"]) {
                Some(p) => (format!("Searching files for `{}`", cut(p, 60)), None),
                None => ("Searching files".into(), None),
            }
        }
        "webfetch" | "fetch" | "web_fetch" | "browser_open" | "browser_navigate" | "navigate" | "open_url" => match url {
            Some(u) => (format!("Opening {}", host_of(u)), Some(u.to_string())),
            None => ("Opening a page".into(), None),
        },
        "websearch" | "web_search" | "search" => match arg(args, &["query", "q"]) {
            Some(q) => (format!("Searching the web for “{}”", cut(q, 80)), None),
            None => ("Searching the web".into(), None),
        },
        "browser_page" | "browser_snapshot" | "browser_read" => ("Reading the browser page".into(), None),
        "browser_screenshot" => ("Looking at the browser page".into(), None),
        "browser_act" | "browser_click" | "browser_type" => ("Using the browser".into(), None),
        "ask_user" => ("Asking you a question".into(), None),
        "request_human_step" => ("Asking you to do a step in the browser".into(), arg(args, &["reason"]).map(str::to_string)),
        "task" | "agent" => match arg(args, &["description", "subagent_type"]) {
            Some(d) => (format!("Starting a helper: {}", cut(d, 80)), None),
            None => ("Starting a helper".into(), None),
        },
        "todowrite" | "todo_list" | "update_plan" => ("Updating the plan".into(), None),
        _ => {
            let shown = match server {
                Some(s) => format!("Using {tool} ({s})"),
                None => format!("Using {tool}"),
            };
            (shown, url.map(str::to_string))
        }
    }
}

/// What an engine can't show of its thinking, as the note opening a turn's steps; `None` when
/// it streams its thinking (claude's thinking blocks, codex's reasoning summaries, grok's
/// thoughts). `engine` is an `EngineId::as_str`.
pub fn thinking_note(engine: &str) -> Option<&'static str> {
    match engine {
        "claude" | "codex" | "grok" => None,
        // the API drivers pass on reasoning only when the server returns it (reasoning_content,
        // reasoning, Anthropic thinking blocks); kwnt doesn't turn on extended thinking itself
        _ => Some("This engine shares its thinking only when its model returns it; otherwise you'll see what it says between steps and each tool it runs."),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pending(id: &str, name: &str, args: Value) -> RuntimeEvent {
        RuntimeEvent::ToolCallPending { id: id.into(), name: name.into(), args }
    }

    fn done(id: &str, status: &str) -> RuntimeEvent {
        RuntimeEvent::ToolCallCompleted { id: id.into(), result: json!({ "status": status }) }
    }

    #[test]
    fn steps_follow_thinking_and_tool_runs() {
        let mut log = ActivityLog::default();
        let started = log.observe(&RuntimeEvent::ThinkingDelta("Let me check ".into()));
        assert_eq!((started[0].kind, started[0].status), ("thinking", "running"));
        // more thinking text changes nothing on the wire
        assert!(log.observe(&RuntimeEvent::ThinkingDelta("the tests.".into())).is_empty());
        let changed = log.observe(&pending("t1", "Bash", json!({"command": "cargo test -q"})));
        assert_eq!(changed.len(), 2, "thinking ends when the tool starts");
        assert_eq!((changed[0].status, changed[0].text.as_deref()), ("ok", Some("Let me check the tests.")));
        assert_eq!(changed[1].title, "Running `cargo test -q`");
        assert_eq!((changed[1].id.as_str(), changed[1].status), ("s2", "running"));
        let ended = log.observe(&done("t1", "error"));
        assert_eq!(ended[0].status, "failed");
        assert!(ended[0].duration_ms.is_some() && ended[0].ended_at.is_some());
        let saved = log.to_json();
        assert_eq!(saved["steps"].as_array().unwrap().len(), 2);
        assert!(saved.get("dropped").is_none());
    }

    #[test]
    fn a_failed_tool_says_why_redacted() {
        let mut log = ActivityLog::with_secrets(vec!["hunter2hunter2".into()]);
        log.observe(&pending("a", "mcp__github__create_issue", json!({})));
        let ended = log.observe(&RuntimeEvent::ToolCallCompleted {
            id: "a".into(),
            result: json!({"status": "error", "content": "401 for token hunter2hunter2"}),
        });
        assert_eq!(ended[0].title, "Using create_issue (github)");
        assert_eq!(ended[0].detail.as_deref(), Some("401 for token ********"));
    }

    #[test]
    fn steps_and_thinking_are_capped() {
        let mut log = ActivityLog::default();
        for i in 0..MAX_STEPS + 5 {
            log.observe(&pending(&i.to_string(), "Read", json!({"file_path": "a.rs"})));
            log.observe(&done(&i.to_string(), "ok"));
        }
        assert_eq!(log.steps().len(), MAX_STEPS);
        assert_eq!(log.to_json()["dropped"], 5);

        let mut log = ActivityLog::default();
        log.observe(&RuntimeEvent::ThinkingDelta("x".repeat(MAX_THINKING_CHARS + 50)));
        let closed = log.finish();
        assert_eq!(closed[0].text.as_ref().unwrap().len(), MAX_THINKING_CHARS);
    }

    #[test]
    fn waits_settle_and_finish_stops_what_is_open() {
        let mut log = ActivityLog::default();
        let w = log.wait("approval", "ap1", "Waiting for your approval: Bash", Some("Bash")).remove(0);
        assert_eq!((w.kind, w.status, w.ref_id.as_deref()), ("approval", "waiting", Some("ap1")));
        assert_eq!(log.settle("ap1", "ok").unwrap().status, "ok");
        assert!(log.settle("ap1", "ok").is_none(), "settled once");
        log.observe(&pending("t", "Read", json!({})));
        log.wait("human_step", "hs1", "Waiting for you to sign in", None);
        let stopped = log.finish();
        assert_eq!(stopped.iter().map(|s| s.status).collect::<Vec<_>>(), ["stopped", "stopped"]);
    }

    #[test]
    fn summaries_are_short_and_human() {
        let cases = [
            ("Bash", json!({"command": "cargo   test\n --all"}), "Running `cargo test --all`"),
            ("command_execution", json!({"command": ["bash", "-lc", "ls -la"]}), "Running `ls -la`"),
            ("command_execution", json!({"command": "bash -lc 'cargo test'"}), "Running `cargo test`"),
            ("Read", json!({"file_path": "/srv/app/src/main.rs"}), "Reading …/src/main.rs"),
            ("Edit", json!({"file_path": "main.rs"}), "Editing main.rs"),
            ("WebFetch", json!({"url": "https://www.github.com/x/y"}), "Opening github.com"),
            ("mcp__kwnt__browser_open", json!({"url": "https://example.com/login"}), "Opening example.com"),
            ("WebSearch", json!({"query": "rust sse"}), "Searching the web for “rust sse”"),
            ("mcp__kwnt__ask_user", json!({}), "Asking you a question"),
            ("mcp__linear__list_issues", json!({}), "Using list_issues (linear)"),
            ("something_else", json!({}), "Using something_else"),
        ];
        for (name, args, want) in cases {
            assert_eq!(summarize(name, &args).0, want, "{name}");
        }
        let long = "x".repeat(300);
        let mut log = ActivityLog::default();
        let s = log.observe(&pending("1", "Bash", json!({"command": long})));
        assert!(s[0].title.chars().count() <= MAX_TITLE);
        assert!(s[0].detail.as_ref().unwrap().chars().count() <= MAX_DETAIL);
    }

    #[test]
    fn home_folders_are_hidden() {
        assert_eq!(
            hide_home_paths("open /home/oleks/.kwnt/data.db failed; see /Users/Oleks Smith/x and /home/ann"),
            "open ~/.kwnt/data.db failed; see ~/x and ~"
        );
        // A name with spaces, whole, when the path goes on or its quotes close
        assert_eq!(hide_home_paths("at /Users/Ann Marie Lee/Library/x"), "at ~/Library/x");
        assert_eq!(hide_home_paths("open '/Users/Oleks Smith' failed"), "open '~' failed");
        assert_eq!(hide_home_paths("/home/ann and then /tmp/x"), "~ and then /tmp/x", "a sentence after a name isn't the name");
        // Windows homes, either slash, any case, and escaped in JSON
        assert_eq!(hide_home_paths(r"C:\Users\Oleks Smith\AppData\kwnt.log"), r"~\AppData\kwnt.log");
        assert_eq!(hide_home_paths("d:/users/oleks/x and C:/Users/ann"), "~/x and ~");
        assert_eq!(hide_home_paths(r#"{"path":"C:\\Users\\oleks\\x"}"#), r#"{"path":"~\\x"}"#);
        // Not a Windows home: another folder, or a URL scheme
        assert_eq!(hide_home_paths(r"C:\Program Files\kwnt and note:\Users\x"), r"C:\Program Files\kwnt and note:\Users\x");
        // Not a home: a bare `/home/`, or `home` inside another path
        assert_eq!(hide_home_paths("/home/ and /srv/home/app"), "/home/ and /srv/home/app");
        assert_eq!(hide_home_paths("no paths here"), "no paths here");
    }

    #[test]
    fn redaction_hides_secrets_in_titles_and_details() {
        let mut log = ActivityLog::with_secrets(vec!["s3cr3t-value-123".into()]);
        let s = log.observe(&pending(
            "1",
            "Bash",
            json!({"command": "curl -H 'Authorization: Bearer abc.def.ghi' https://u:pw@api.x.io/?token=zzz9 && OPENAI_API_KEY=sk-proj-abcdefghijklmnop1234 run --pass s3cr3t-value-123"}),
        ));
        let shown = format!("{} {}", s[0].title, s[0].detail.as_deref().unwrap());
        for leaked in ["abc.def.ghi", ":pw@", "zzz9", "sk-proj", "s3cr3t-value-123"] {
            assert!(!shown.contains(leaked), "{leaked} leaked in {shown}");
        }
        assert!(shown.contains("curl"));
        for (text, want) in [
            ("push with ghp_abcdefghijklmnopqrstuvwxyz0123", "push with [redacted]"),
            ("key xai-AbCdEfGhIjKlMnOpQrSt9 here", "key [redacted] here"),
            ("the sk- prefix and skip-this", "the sk- prefix and skip-this"),
        ] {
            assert_eq!(redact(text, &[]), want);
        }
    }
}
