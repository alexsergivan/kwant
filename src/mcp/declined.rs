//! 27b: a no stays a no. Once the person declines a card, a later call of the same kind in that
//! chat is refused at once, with no card, until the person writes there again. Without it a bot
//! that heard "no" to `gh pr view 12` tried `gh pr view --json title`, then `/usr/bin/gh pr view`,
//! each one a new card (Compass trial, problems B and C).
//!
//! What a no covers is the call's *family* ([`families`]):
//!
//! | Call | Family |
//! |---|---|
//! | Shell | program + first subcommand word (`gh pr`, `git push`); wrappers, env assignments, paths and `cd` left out |
//! | Shell, a URL program (curl, wget, http, xh) | program + host (`curl` + `localhost`, any port or path) |
//! | File tools | write or read + the file's path |
//! | WebFetch, the bot browser's open or navigate | host |
//! | Any other MCP tool | server + tool |
//!
//! A shell line with several commands has a family for each; it is refused when any one was
//! declined. Remembered in memory only, per bot and conversation, and capped
//! ([`MAX_PER_CONVERSATION`]): a restart ends the turn anyway.

use super::ToolApprovalRequest;
use serde_json::Value;
use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex};
use std::time::Instant;

/// Declined families kept per bot and conversation; the oldest goes first.
pub const MAX_PER_CONVERSATION: usize = 32;
/// Conversations with something declined kept at once; the one declined in longest ago goes first.
pub const MAX_CONVERSATIONS: usize = 1024;

/// Programs whose family is the host they reach, not a subcommand.
const URL_PROGRAMS: &[&str] = &["curl", "wget", "http", "https", "xh", "xhs"];
/// Programs that run the rest of the line: skipped, with their options.
const WRAPPERS: &[&str] = &["sudo", "env", "command", "nohup", "time", "exec", "nice", "timeout", "doas", "stdbuf"];
/// The wrappers' options that take the next word.
const WRAPPER_VALUED: &[&str] = &["-u", "-g", "-n", "-s", "-k", "-C", "-D", "-p", "-U", "--user", "--group", "--signal", "--kill-after", "--unset", "--chdir"];
/// Steps around the real command (moving, printing, filtering a pipe): no family of their own,
/// so `cd repo && gh pr view | jq .title` is just `gh pr`.
const PLUMBING: &[&str] = &[
    "cd", "pushd", "popd", "echo", "printf", "true", "false", "sleep", "set", "export", "test", "[", ":", "head", "tail",
    "jq", "grep", "egrep", "fgrep", "sort", "uniq", "wc", "cat", "less", "more", "cut", "tr",
];
const SHELLS: &[&str] = &["bash", "sh", "zsh", "dash", "ksh"];
/// Global options that take the next word, by program, so `git -C repo push` is still `git push`.
/// (A `match`, not a table: tests/host_git_calls_test.rs accepts a git name as a match pattern.)
fn global_valued(program: &str) -> &'static [&'static str] {
    match program {
        "git" => &["-C", "-c", "--git-dir", "--work-tree", "--namespace"],
        "gh" => &["-R", "--repo", "--hostname"],
        "docker" => &["-H", "--host", "--context", "-c", "--config", "-l", "--log-level"],
        "kubectl" => &["-n", "--namespace", "--context", "--cluster", "--kubeconfig", "-s", "--server", "--user"],
        "npm" => &["--prefix", "-w", "--workspace"],
        "cargo" => &["--manifest-path", "-Z", "--config"],
        _ => &[],
    }
}
/// curl's, wget's and httpie's options that take the next word (it is never the address).
const URL_VALUED: &[&str] = &[
    "-o", "--output", "-H", "--header", "-d", "--data", "--data-raw", "--data-binary", "--data-urlencode", "-X", "--request",
    "-u", "--user", "-A", "--user-agent", "-e", "--referer", "-b", "--cookie", "-c", "--cookie-jar", "-w", "--write-out",
    "-T", "--upload-file", "-F", "--form", "-m", "--max-time", "--connect-timeout", "-x", "--proxy", "-K", "--config",
    "-r", "--range", "--retry", "--cacert", "--cert", "--key", "-E", "--resolve", "-O", "--output-document",
    "--output-file", "-P", "--directory-prefix", "-U", "--post-data", "--post-file", "-t", "--tries", "--timeout",
    "--auth", "-a",
];

/// What a "no" to `req` covers: one family per part of the call, empty when it covers nothing
/// kwant can name (a shell line it can't read, a fetch with no address, one of claude's other
/// tools), so the next such call asks again.
pub fn families(req: &ToolApprovalRequest) -> Vec<String> {
    let name = if req.builtin.is_some() || req.server.is_empty() { req.tool_name.as_str() } else { req.tool.as_str() };
    let lower = name.rsplit("__").next().unwrap_or(name).to_ascii_lowercase();
    let mut out = Vec::new();
    if crate::drivers::activity::is_shell_tool(&req.tool_name) || crate::drivers::activity::is_shell_tool(&req.tool) {
        if let Some(line) = shell_line(&req.arguments) {
            shell_families(&line, 0, &mut out);
        }
    } else if let Some(kind) = file_kind(&lower) {
        let cwd = req.builtin.as_ref().and_then(|b| b.cwd.as_deref());
        if let Some(path) = arg(&req.arguments, &["file_path", "path", "notebook_path", "filePath", "target_file"]) {
            out.push(format!("file:{kind}:{}", normal_path(path, cwd)));
        }
    } else if matches!(lower.as_str(), "webfetch" | "fetch" | "web_fetch") || crate::drivers::activity::opens_page(&lower) {
        if let Some(host) = arg(&req.arguments, &["url", "href", "uri"]).and_then(host_of) {
            out.push(format!("web:{host}"));
        }
    } else if req.builtin.is_none() && !req.server.is_empty() {
        out.push(format!("mcp:{}/{}", req.server, req.tool));
    }
    out.dedup();
    out
}

fn arg<'a>(args: &'a Value, keys: &[&str]) -> Option<&'a str> {
    keys.iter().find_map(|k| args[*k].as_str()).map(str::trim).filter(|s| !s.is_empty())
}

/// `write` (Write, Edit, MultiEdit, NotebookEdit, kwnt's `write_file`…) or `read`.
fn file_kind(tool: &str) -> Option<&'static str> {
    match tool {
        "write" | "write_file" | "create_file" | "edit" | "multiedit" | "edit_file" | "str_replace" | "str_replace_based_edit_tool"
        | "notebookedit" => Some("write"),
        "read" | "read_file" | "notebookread" => Some("read"),
        _ => None,
    }
}

/// The command a shell tool runs: `command` or `cmd`, as a string, or as words (`["bash", "-lc",
/// "gh pr view"]` is its last word).
fn shell_line(args: &Value) -> Option<String> {
    let v = ["command", "cmd"].iter().map(|k| &args[*k]).find(|v| !v.is_null())?;
    if let Some(s) = v.as_str() {
        return Some(s.to_string());
    }
    let words: Vec<&str> = v.as_array()?.iter().filter_map(Value::as_str).collect();
    match words.as_slice() {
        [sh, flag, line] if SHELLS.contains(&base(sh)) && flag.starts_with('-') && flag.contains('c') => Some(line.to_string()),
        // Plain words: quote each one so the lexer reads them back as they are
        _ => Some(words.iter().map(|w| format!("'{}'", w.replace('\'', "'\\''"))).collect::<Vec<_>>().join(" ")),
    }
}

fn base(program: &str) -> &str {
    program.rsplit('/').next().unwrap_or(program)
}

/// Adds the families of every simple command in `line`. A line the shell lexer can't read
/// (`$VAR`, a subshell) is read up to its first such character, by whitespace.
fn shell_families(line: &str, depth: u8, out: &mut Vec<String>) {
    let commands = super::shell_risk::simple_commands(line).unwrap_or_else(|| {
        let head = line.split(['$', '`', '(', ')', '|', ';', '&', '<', '>', '\n', '"', '\'']).next().unwrap_or("");
        vec![head.split_whitespace().map(str::to_string).collect()]
    });
    for words in commands {
        if let Some(f) = command_family(&words, depth, out) {
            out.push(f);
        }
    }
}

/// The family of one simple command, or `None` (plumbing, nothing to name). A shell's `-c` line
/// adds its own families to `out` instead.
fn command_family(words: &[String], depth: u8, out: &mut Vec<String>) -> Option<String> {
    let mut i = 0;
    // Env assignments, then wrappers (with their options and env's assignments)
    loop {
        let w = words.get(i)?;
        if super::shell_risk::assignment(w).is_some() {
            i += 1;
            continue;
        }
        let program = base(w);
        if !WRAPPERS.contains(&program) {
            break;
        }
        i += 1;
        let mut positional = usize::from(program == "timeout");
        while let Some(a) = words.get(i) {
            if a.starts_with('-') {
                i += if WRAPPER_VALUED.contains(&a.as_str()) { 2 } else { 1 };
            } else if super::shell_risk::assignment(a).is_some() {
                i += 1;
            } else if positional > 0 {
                positional -= 1;
                i += 1;
            } else {
                break;
            }
        }
    }
    let program = base(words.get(i)?);
    let args = &words[i + 1..];
    if program.is_empty() || PLUMBING.contains(&program) {
        return None;
    }
    if SHELLS.contains(&program) && depth < 2 {
        let at = args.iter().position(|a| a.starts_with('-') && !a.starts_with("--") && a.contains('c'))?;
        shell_families(args.get(at + 1)?, depth + 1, out);
        return None;
    }
    if URL_PROGRAMS.contains(&program) {
        return url_host(args).map(|host| format!("sh:{program}@{host}"));
    }
    let valued = global_valued(program);
    let mut j = 0;
    let first = loop {
        let Some(a) = args.get(j) else { break None };
        if a == "--" {
            break args.get(j + 1);
        }
        if a.starts_with('-') {
            j += if valued.contains(&a.as_str()) { 2 } else { 1 };
            continue;
        }
        break Some(a);
    };
    match first.filter(|a| bare_word(a)) {
        Some(sub) => Some(format!("sh:{program} {sub}")),
        None => Some(format!("sh:{program}")),
    }
}

/// A subcommand-looking word: letters, digits, `-` and `_`, with no `/`, `.`, `:` or `=`.
fn bare_word(word: &str) -> bool {
    !word.is_empty()
        && !word.starts_with('-')
        && word.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
        && word.chars().any(|c| c.is_ascii_alphabetic())
}

/// The host a curl, wget or httpie line reaches: its first address with a scheme, else its first
/// word that looks like `host[:port][/path]` (or httpie's `:port`), skipping option values.
fn url_host(args: &[String]) -> Option<String> {
    let mut positional = Vec::new();
    let mut j = 0;
    while let Some(a) = args.get(j) {
        if a == "--url" {
            if let Some(u) = args.get(j + 1) {
                positional.push(u.as_str());
            }
            j += 2;
        } else if a.starts_with('-') {
            j += if URL_VALUED.contains(&a.as_str()) { 2 } else { 1 };
        } else {
            positional.push(a.as_str());
            j += 1;
        }
    }
    if let Some(host) = positional.iter().filter(|a| a.contains("://")).find_map(|a| host_of(a)) {
        return Some(host);
    }
    positional
        .iter()
        // httpie's method word and its `key=value` / `Header:value` items are not addresses
        .filter(|a| !a.chars().all(|c| c.is_ascii_uppercase()) && !a.contains('='))
        .find_map(|a| {
            let a: &str = a;
            if a.starts_with(':') && a[1..].chars().next().is_some_and(|c| c.is_ascii_digit()) {
                return Some("localhost".to_string());
            }
            let host = host_of(a)?;
            (host == "localhost" || host.contains('.') || a.contains(':')).then_some(host)
        })
}

/// The host of an address, lowercased, without scheme, user, port or path. Every loopback
/// address (`127.0.0.1`, `::1`, `0.0.0.0`) is `localhost`, so a port probe is one family.
pub fn host_of(url: &str) -> Option<String> {
    let rest = url.trim().split_once("://").map_or(url.trim(), |(_, r)| r);
    let authority = rest.split(['/', '?', '#']).next()?;
    let authority = authority.rsplit_once('@').map_or(authority, |(_, h)| h);
    let host = if let Some(v6) = authority.strip_prefix('[') {
        v6.split(']').next()?
    } else {
        match authority.rsplit_once(':') {
            Some((h, port)) if port.chars().all(|c| c.is_ascii_digit()) => h,
            _ => authority,
        }
    };
    let host = host.trim_end_matches('.').to_ascii_lowercase();
    if host.is_empty() || !host.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_' | ':')) {
        return None;
    }
    let loopback = host == "localhost" || host.ends_with(".localhost") || host.starts_with("127.") || host == "::1" || host == "0.0.0.0";
    Some(if loopback { "localhost".into() } else { host })
}

/// `path` joined to `cwd` when relative, with `.` and `..` resolved by name (nothing is read
/// from disk) and no trailing `/`.
fn normal_path(path: &str, cwd: Option<&str>) -> String {
    let joined = match cwd {
        Some(c) if !path.starts_with('/') && !path.starts_with('~') => format!("{}/{path}", c.trim_end_matches('/')),
        _ => path.to_string(),
    };
    let mut parts: Vec<&str> = Vec::new();
    for part in joined.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                parts.pop();
            }
            p => parts.push(p),
        }
    }
    let lead = if joined.starts_with('/') { "/" } else { "" };
    format!("{lead}{}", parts.join("/"))
}

#[derive(Debug, Clone)]
struct Entry {
    family: String,
    ticket: String,
}

#[derive(Debug, Default)]
struct Inner {
    by_chat: HashMap<(String, String), VecDeque<Entry>>,
    /// When each conversation last had something declined, for [`MAX_CONVERSATIONS`].
    last: HashMap<(String, String), Instant>,
}

/// What the person declined, per bot and conversation, until they write there again.
#[derive(Debug, Clone, Default)]
pub struct Declines(Arc<Mutex<Inner>>);

impl Declines {
    fn lock(&self) -> std::sync::MutexGuard<'_, Inner> {
        self.0.lock().unwrap_or_else(|p| p.into_inner())
    }

    /// The person said no to the call with `families` on card `ticket`.
    pub fn record(&self, kwant_id: &str, conversation: &str, families: &[String], ticket: &str) {
        if families.is_empty() {
            return;
        }
        let key = (kwant_id.to_string(), conversation.to_string());
        let mut inner = self.lock();
        if !inner.by_chat.contains_key(&key) && inner.by_chat.len() >= MAX_CONVERSATIONS {
            if let Some(oldest) = inner.last.iter().min_by_key(|(_, at)| **at).map(|(k, _)| k.clone()) {
                inner.by_chat.remove(&oldest);
                inner.last.remove(&oldest);
            }
        }
        inner.last.insert(key.clone(), Instant::now());
        let list = inner.by_chat.entry(key).or_default();
        for family in families {
            list.retain(|e| &e.family != family);
            list.push_back(Entry { family: family.clone(), ticket: ticket.to_string() });
        }
        while list.len() > MAX_PER_CONVERSATION {
            list.pop_front();
        }
    }

    /// The card the person declined one of `families` on, in this bot's conversation.
    pub fn check(&self, kwant_id: &str, conversation: &str, families: &[String]) -> Option<String> {
        if families.is_empty() {
            return None;
        }
        let inner = self.lock();
        let list = inner.by_chat.get(&(kwant_id.to_string(), conversation.to_string()))?;
        list.iter().rev().find(|e| families.contains(&e.family)).map(|e| e.ticket.clone())
    }

    /// The person wrote again in this conversation: what they declined there may be asked again.
    pub fn clear(&self, kwant_id: &str, conversation: &str) {
        let key = (kwant_id.to_string(), conversation.to_string());
        let mut inner = self.lock();
        inner.by_chat.remove(&key);
        inner.last.remove(&key);
    }

    /// A deleted bot.
    pub fn forget_kwant(&self, kwant_id: &str) {
        let mut inner = self.lock();
        inner.by_chat.retain(|(k, _), _| k != kwant_id);
        inner.last.retain(|(k, _), _| k != kwant_id);
    }

    /// A deleted thread or channel.
    pub fn forget_conversation(&self, conversation: &str) {
        let mut inner = self.lock();
        inner.by_chat.retain(|(_, c), _| c != conversation);
        inner.last.retain(|(_, c), _| c != conversation);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mcp::BuiltinCall;
    use serde_json::json;

    fn call(tool_name: &str, server: &str, tool: &str, arguments: Value, builtin: bool) -> ToolApprovalRequest {
        ToolApprovalRequest {
            kwant_id: "k1".into(),
            kwant_name: "Forge".into(),
            turn_id: "t1".into(),
            tool_name: tool_name.into(),
            server: server.into(),
            tool: tool.into(),
            arguments,
            builtin: builtin.then(|| BuiltinCall { cwd: Some("/work/repo".into()), outside_folder: true }),
            smart: None,
            payment: None,
        }
    }

    fn bash(command: &str) -> ToolApprovalRequest {
        call("Bash", "", "Bash", json!({"command": command, "description": "x"}), true)
    }

    fn fam(command: &str) -> Vec<String> {
        families(&bash(command))
    }

    fn same(a: &[String], b: &[String]) -> bool {
        !a.is_empty() && a.iter().any(|f| b.contains(f))
    }

    /// The shell rows of the table: program + first subcommand word, whatever is around it.
    #[test]
    fn a_shell_command_is_its_program_and_subcommand() {
        for (cmd, want) in [
            ("gh pr view 12", "sh:gh pr"),
            ("gh pr view --json title", "sh:gh pr"),
            ("/usr/bin/gh pr view", "sh:gh pr"),
            ("GH_PAGER=cat gh pr view 12", "sh:gh pr"),
            ("sudo -u root gh pr view", "sh:gh pr"),
            ("env GH_PAGER= gh pr view", "sh:gh pr"),
            ("command gh pr view", "sh:gh pr"),
            ("cd /work/repo && gh pr view 12", "sh:gh pr"),
            ("gh -R acme/site pr view 12", "sh:gh pr"),
            ("gh pr view 12 --json title | jq .title", "sh:gh pr"),
            ("git push origin main", "sh:git push"),
            ("git push -f", "sh:git push"),
            ("git -C /work/repo push", "sh:git push"),
            ("timeout 30 git push", "sh:git push"),
            ("bash -lc 'gh pr view 12'", "sh:gh pr"),
            ("gh pr view $(git branch --show-current)", "sh:gh pr"),
            ("ls /tmp", "sh:ls"),
            ("./deploy.sh --prod", "sh:deploy.sh"),
        ] {
            assert_eq!(fam(cmd), vec![want.to_string()], "{cmd}");
        }
        // Another subcommand is another family: it still asks
        for (a, b) in [("gh pr view", "gh issue list"), ("git push", "git status"), ("npm publish", "npm test")] {
            assert!(!same(&fam(a), &fam(b)), "{a} vs {b}");
        }
        // Codex's words form
        let codex = call("exec_command", "", "exec_command", json!({"cmd": ["bash", "-lc", "gh pr view 3"]}), true);
        assert_eq!(families(&codex), vec!["sh:gh pr".to_string()]);
        let words = call("shell", "", "shell", json!({"command": ["gh", "pr", "view", "3"]}), true);
        assert_eq!(families(&words), vec!["sh:gh pr".to_string()]);
    }

    /// Compass B: port probes on localhost are one family, whatever the port, path or spelling.
    #[test]
    fn a_url_program_is_its_program_and_host() {
        for cmd in [
            "curl localhost:9/health",
            "curl -s http://localhost:9/",
            "curl -sS -o /dev/null -w '%{http_code}' http://127.0.0.1:3000/api",
            "curl http://localhost:8080",
            "curl -H 'Accept: text/html' --max-time 3 localhost:5173",
            "curl --url http://[::1]:4000/x",
        ] {
            assert_eq!(fam(cmd), vec!["sh:curl@localhost".to_string()], "{cmd}");
        }
        assert_eq!(fam("wget -O out.html https://Example.com/a"), vec!["sh:wget@example.com".to_string()]);
        assert_eq!(fam("http GET :8080/x"), vec!["sh:http@localhost".to_string()]);
        assert_eq!(fam("xh POST api.example.com/v1 name=x"), vec!["sh:xh@api.example.com".to_string()]);
        assert!(!same(&fam("curl https://api.github.com/x"), &fam("curl https://example.com/x")), "another host still asks");
        assert!(fam("curl -s").is_empty(), "no address: nothing to name");
    }

    /// Several commands in one line: each has its family; plumbing has none.
    #[test]
    fn a_line_has_every_command_s_family() {
        assert_eq!(fam("git add . && git commit -m wip && git push"), vec!["sh:git add", "sh:git commit", "sh:git push"]);
        assert_eq!(fam("echo hi; cd x | head -3"), Vec::<String>::new());
        assert!(same(&fam("git status && git push origin main"), &fam("git push")));
    }

    #[test]
    fn a_file_tool_is_its_kind_and_path() {
        let write = |tool: &str, path: &str| families(&call(tool, "", tool, json!({"file_path": path, "content": "x"}), true));
        assert_eq!(write("Write", "/etc/hosts"), vec!["file:write:/etc/hosts".to_string()]);
        // The same file, written another way, or named another way
        for (tool, path) in [("Edit", "/etc/hosts"), ("MultiEdit", "/etc/./hosts"), ("Write", "/etc/x/../hosts"), ("Edit", "../../etc/hosts")] {
            assert_eq!(write(tool, path), vec!["file:write:/etc/hosts".to_string()], "{tool} {path}");
        }
        let nb = families(&call("NotebookEdit", "", "NotebookEdit", json!({"notebook_path": "a.ipynb"}), true));
        assert_eq!(nb, vec!["file:write:/work/repo/a.ipynb".to_string()]);
        let own = families(&call("mcp__kwnt__write_file", "kwnt", "write_file", json!({"path": "/etc/hosts"}), false));
        assert_eq!(own, vec!["file:write:/etc/hosts".to_string()], "kwnt's own file writes");
        assert!(!same(&write("Write", "/etc/hosts"), &write("Write", "/etc/passwd")), "another file still asks");
        let read = families(&call("Read", "", "Read", json!({"file_path": "/etc/hosts"}), true));
        assert!(!same(&read, &write("Write", "/etc/hosts")), "a read isn't a write");
    }

    #[test]
    fn a_page_is_its_host() {
        let fetch = |url: &str| families(&call("WebFetch", "", "WebFetch", json!({"url": url, "prompt": "x"}), true));
        assert_eq!(fetch("https://news.example.com/a?b=1"), vec!["web:news.example.com".to_string()]);
        assert_eq!(fetch("http://news.example.com:8443/other"), vec!["web:news.example.com".to_string()]);
        let open = families(&call("mcp__kwnt__browser_open", "kwnt", "browser_open", json!({"url": "https://news.example.com/"}), false));
        assert_eq!(open, vec!["web:news.example.com".to_string()]);
        let nav = families(&call("mcp__playwright__navigate", "playwright", "navigate", json!({"url": "https://shop.example.org"}), false));
        assert_eq!(nav, vec!["web:shop.example.org".to_string()]);
        assert!(!same(&fetch("https://a.example.com"), &fetch("https://b.example.com")));
    }

    #[test]
    fn another_mcp_tool_is_its_server_and_tool() {
        let merge = |args: Value| families(&call("mcp__github__merge_pull_request", "github", "merge_pull_request", args, false));
        assert_eq!(merge(json!({"pr": 1})), vec!["mcp:github/merge_pull_request".to_string()]);
        assert_eq!(merge(json!({"pr": 2, "method": "squash"})), merge(json!({"pr": 1})));
        let other = families(&call("mcp__github__create_issue", "github", "create_issue", json!({}), false));
        assert!(!same(&other, &merge(json!({}))));
        // One of claude's other tools names nothing: it always asks
        assert!(families(&call("WebSearch", "", "WebSearch", json!({"query": "x"}), true)).is_empty());
    }

    #[test]
    fn a_no_is_kept_per_bot_and_conversation_until_cleared() {
        let d = Declines::default();
        d.record("k1", "th1", &fam("gh pr view 12"), "ap1");
        assert_eq!(d.check("k1", "th1", &fam("/usr/bin/gh pr view --json title")).as_deref(), Some("ap1"));
        assert_eq!(d.check("k1", "th1", &fam("gh issue list")), None, "another family");
        assert_eq!(d.check("k1", "th2", &fam("gh pr view")), None, "another conversation");
        assert_eq!(d.check("k2", "th1", &fam("gh pr view")), None, "another bot");
        assert_eq!(d.check("k1", "th1", &fam("git status && gh pr view")).as_deref(), Some("ap1"), "any part of a line");
        assert_eq!(d.check("k1", "th1", &[]), None);
        d.clear("k1", "th1");
        assert_eq!(d.check("k1", "th1", &fam("gh pr view")), None, "the person wrote again");

        d.record("k1", "th1", &fam("gh pr view"), "ap2");
        d.record("k2", "th1", &fam("gh pr view"), "ap3");
        d.record("k1", "th2", &fam("gh pr view"), "ap4");
        d.forget_kwant("k1");
        assert_eq!(d.check("k1", "th1", &fam("gh pr view")), None);
        assert_eq!(d.check("k2", "th1", &fam("gh pr view")).as_deref(), Some("ap3"));
        d.forget_conversation("th1");
        assert_eq!(d.check("k2", "th1", &fam("gh pr view")), None);
    }

    #[test]
    fn the_memory_is_capped() {
        let d = Declines::default();
        for i in 0..MAX_PER_CONVERSATION + 5 {
            d.record("k1", "th1", &[format!("mcp:s/t{i}")], &format!("ap{i}"));
        }
        assert_eq!(d.check("k1", "th1", &["mcp:s/t0".into()]), None, "the oldest went first");
        assert!(d.check("k1", "th1", &[format!("mcp:s/t{}", MAX_PER_CONVERSATION + 4)]).is_some());
        for i in 0..MAX_CONVERSATIONS + 3 {
            d.record("k1", &format!("c{i}"), &["mcp:s/t".into()], "ap");
        }
        assert!(d.lock().by_chat.len() <= MAX_CONVERSATIONS);
        assert!(d.check("k1", &format!("c{}", MAX_CONVERSATIONS + 2), &["mcp:s/t".into()]).is_some());
    }
}
