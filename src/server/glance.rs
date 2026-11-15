//! The desktop island's one call (0.9.13): `GET /api/glance` says, across every space that isn't
//! archived, what needs the person, what is working now and what just finished, in one cheap
//! response the app polls every 5 s per place with `If-None-Match`. `POST /api/glance/ask` sends a
//! quick ask into the person's chat with a bot, as if typed there. Both are the person's only.
//!
//! Needs you is exactly `GET /api/away`'s `waiting` list (the same builder), so the island and the
//! nudge agree. Each approval says whether the island may offer Allow once inline
//! ([`can_allow_inline`]): never for money, the production or push floor, a tier-forced ask, a
//! risky step or anything else whose only safe answer is the full card. Text from bots is
//! untrusted: titles are one masked line, a working bot's step is its activity log's own redacted
//! title, and no tool output, message body or argument value is ever sent.

use super::away::{self, mask_words, rfc3339};
use super::AppState;
use crate::db::spaces::{Space, SpaceId};
use axum::{
    extract::State,
    http::{header, HeaderMap, HeaderValue, StatusCode},
    response::{IntoResponse, Response},
    Json,
};
use chrono::{DateTime, Duration, Utc};
use serde::Deserialize;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::HashMap;

/// Most Needs-you items in one glance (the counts say how many there are in all).
pub const NEEDS_YOU_MAX: usize = 20;
/// Most running turns listed under Working now.
pub const WORKING_MAX: usize = 12;
/// Just finished: at most this many, from the last [`FINISHED_HOURS`].
pub const FINISHED_MAX: usize = 5;
pub const FINISHED_HOURS: i64 = 24;
/// Most bots offered for a quick ask.
pub const ASK_TARGETS_MAX: i64 = 8;
/// Most answer choices a question shows inline; a question with more says so (`choices_total`).
pub const CHOICES_MAX: usize = 6;
const CHOICE_CHARS: usize = 60;
const STEP_CHARS: usize = 80;
/// Longest quick ask. The chat composer has no cap of its own (only the 2 MB request limit); a
/// quick ask is one line from a small box, so it gets a generous fixed one.
pub const ASK_MAX_CHARS: usize = 8_000;

fn error(status: StatusCode, code: &str, message: &str) -> Response {
    (status, Json(json!({ "error": message, "code": code }))).into_response()
}

fn internal() -> Response {
    error(StatusCode::INTERNAL_SERVER_ERROR, "internal", "Couldn't read what's happening. Try again.")
}

/// One line of at most `max` characters, with key-like words masked.
fn line(text: &str, max: usize) -> String {
    let one = mask_words(&text.split_whitespace().collect::<Vec<_>>().join(" "));
    match one.chars().count() > max {
        true => format!("{}…", one.chars().take(max - 1).collect::<String>().trim_end()),
        false => one,
    }
}

/// Percent-encodes everything but RFC 3986 unreserved characters (a path segment or query value),
/// as the desktop app's nudge does.
fn encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => out.push(b as char),
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

/// Where an item opens in the app (the nudge's `target_path`): its channel, its bot's
/// conversation, else its bot's chat, each with `?card=<id>` when a card should show; with no bot,
/// the space's Inbox.
pub fn app_path(space: &str, link: &Value, kwant_id: &str, card: Option<&str>) -> String {
    let base = format!("/s/{}", encode(space));
    let card = card.filter(|c| !c.is_empty()).map(|c| format!("?card={}", encode(c))).unwrap_or_default();
    if let Some(c) = link["channel_id"].as_str().filter(|c| !c.is_empty()) {
        return format!("{base}/c/{}{card}", encode(c));
    }
    let bot = link["direct_kwant_id"].as_str().filter(|b| !b.is_empty()).unwrap_or(kwant_id);
    if bot.is_empty() {
        return format!("{base}/inbox");
    }
    match link["thread_id"].as_str().filter(|t| !t.is_empty()) {
        Some(t) => format!("{base}/b/{}/t/{}{card}", encode(bot), encode(t)),
        None => format!("{base}/b/{}{card}", encode(bot)),
    }
}

/// Word runs of a tool name ([`words`]) that merge, push or change a branch or a release without a
/// file write [`mcp_writes`] would see. Every word but the last matches whole; the last also matches
/// as a prefix, so `merges`, `pushed`, `releases`, `update_refs` and `update_reference` count. Whole
/// words since 20d (they were parts before, so `GITHUB_GET_A_RELEASE` "pushed code"), and a name
/// with a [`READ_VERBS`] word only reads ([`ships_by_name`]).
///
/// [`mcp_writes`]: super::dev::prepush::mcp_writes
const PUSH_WORDS: &[&[&str]] = &[
    &["merge"],
    &["push"],
    &["release"],
    &["update", "branch"],
    &["update", "ref"],
    &["update", "a", "ref"],
    &["create", "a", "ref"],
    &["delete", "a", "ref"],
];

/// Whole words of a tool name ([`words`]) that make a commit or a branch: `commit_files`,
/// `create_commit`, `create_branch`, `GITHUB_CREATE_A_BRANCH` (18c, Rigel's B55–B56). Whole words, so
/// `list_branches` and `list_commits` stay out, and a name with a [`READ_VERBS`] word
/// (`get_commit`, `GITHUB_GET_A_BRANCH`) only reads.
const COMMIT_NAME_WORDS: &[&str] = &["commit", "branch"];

/// Whole words of a tool name that only read; they keep [`COMMIT_NAME_WORDS`] from refusing.
const READ_VERBS: &[&str] = &["get", "list", "search", "read", "view", "show", "describe", "find", "compare"];

/// Word starts of a tool or server name ([`words`]) that mark a tool taking free text to run: a
/// command, a script, code or a query. A one-line peek can't show what that text does, and a list of
/// bad commands is never complete, so all of them need the full card. Prefixes of whole words since
/// 20d, so `execute`, `evaluate`, `commands`, `scripts`, `runner` and `shellExec` count while
/// `truncate`, `description`, `encode` and `geocode` don't. See [`runs_by_name`].
const RUNS_TEXT_NAMES: &[&str] = &["run", "exec", "command", "cmd", "shell", "bash", "terminal", "ssh", "script", "eval", "code", "sql"];

/// Whole words of a tool or server name that run free text but hide a [`RUNS_TEXT_NAMES`] word
/// inside one word, which the old part match caught and a prefix match would miss (20d): SQL
/// dialects, shells and script languages written as one word, and code editors.
const RUNS_TEXT_NAME_WORDS: &[&str] = &[
    "mysql", "mssql", "psql", "pgsql", "plsql", "tsql", "nosql", "postgresql", //
    "powershell", "subshell", "subcommand", "autoexec", "rerun", "autorun", //
    "javascript", "typescript", "applescript", "vbscript", "jscript", //
    "vscode", "xcode", "opencode", "claudecode",
];

/// Argument keys (any depth, any case) that carry free text to run.
const RUNS_TEXT_KEYS: &[&str] = &["command", "cmd", "argv", "args", "script", "code", "shell", "query", "sql"];

/// Parts of an argument key (any depth; lowercased with `_`, `-`, `.` and spaces dropped) that
/// carry free text to run, matched by containment: `command_line`, `commandLine`, `shell_command`,
/// `cmdline`, `bash_command`, `exec_args` (0.9.14, Rigel's hole 1).
const RUNS_TEXT_KEY_PARTS: &[&str] = &["command", "cmd", "shell", "exec", "argv"];

/// Whole words of an argument key (split at `_`, `-`, camelCase) that carry a script. Not by
/// containment, which would catch `description`, `subscription` and `transcript`.
///
/// 18c (Rigel's B10–B13, B17, B52–B53, H06–H08): a command typed under `input`, `line`,
/// `statement`, `code_snippet` or `arguments_list` on a tool whose name says nothing.
const RUNS_TEXT_KEY_WORDS: &[&str] = &[
    "script", "scripts", "javascript", "applescript", "stdin", //
    "input", "inputs", "line", "lines", "statement", "statements", "snippet", "snippets", "args", "arguments",
];

/// HTTP verbs that change something, in a `method` argument (any depth, any case).
const WRITE_METHODS: &[&str] = &["DELETE", "PUT", "PATCH", "POST"];

/// HTTP verbs that only read; on an HTTP-shaped call (it has a URL argument) any other `method`
/// value, or one that isn't a plain string, needs the full card.
const READ_METHODS: &[&str] = &["GET", "HEAD", "OPTIONS"];

/// Whole words (in the tool name, any argument key or any string value) that push, merge, publish
/// or release, or move refs and tags.
const PUSH_FAMILY: &[&[&str]] = &[&["push"], &["merge"], &["publish"], &["release"], &["send", "pack"], &["receive", "pack"], &["refs"], &["deploy"], &["tag"]];

/// Parts of a tool name (any case) that act outside kwant or destroy something: publishing,
/// sending, deleting, posting or paying.
const OUTWARD_NAMES: &[&str] = &[
    "PUBLISH", "DEPLOY", "SEND", "EMAIL", "MAIL", "DELETE", "REMOVE", "DESTROY", "DROP", "POST", "PAY",
    // 0.9.14 (Rigel's hole 3)
    "TRANSFER", "SUBMIT", "TWEET", "SHARE", "INVITE", "FORWARD", "UPLOAD", "WIPE", "TERMINATE", "REVOKE", "PURCHASE",
    // 18c (Rigel's B34)
    "BROADCAST",
];

/// Whole words of a tool name ([`words`]) that act outside kwant but sit inside ordinary words as
/// parts (`ship` in `membership`, `relationship`), so they can't join [`OUTWARD_NAMES`]: `ship_it`,
/// `rollout` (18c, Rigel's B54).
const OUTWARD_NAME_WORDS: &[&str] = &["ship", "rollout"];

/// Whole words in the value of an `action`, `operation` or `op` argument ([`is_action_key`]) that
/// change or destroy something (18c, Rigel's H01–H02): the [`WRITE_METHODS`] verbs, plus these.
const ACTION_WRITE_WORDS: &[&str] = &["delete", "put", "patch", "post", "remove", "destroy", "drop", "purge", "wipe", "truncate", "erase"];

/// Argument keys (flattened, [`flat`]) that carry a request body; on an HTTP-shaped call with no
/// method key, such a key means the tool sends it, usually as a POST (18c, Rigel's H04).
const BODY_KEYS: &[&str] = &["body", "data", "json", "payload", "form", "formdata", "requestbody", "postdata", "jsonbody"];

pub const PUSHES: &str = "This pushes code, so it needs the full card.";
pub const RUNS: &str = "This runs a command, so it needs the full card.";
pub const RISKY: &str = "kwant flagged this step as risky.";
pub const OUTWARD: &str = "This acts outside kwant, so it needs the full card.";

/// Every argument key and string value, at any depth, in document order (array items in turn).
fn texts<'a>(v: &'a Value, out: &mut Vec<&'a str>) {
    match v {
        Value::String(s) => out.push(s),
        Value::Array(items) => items.iter().for_each(|i| texts(i, out)),
        Value::Object(map) => map.iter().for_each(|(k, v)| {
            out.push(k);
            texts(v, out);
        }),
        _ => {}
    }
}

/// Whether `f(key, value)` holds for any key at any depth.
fn any_key(v: &Value, f: &dyn Fn(&str, &Value) -> bool) -> bool {
    match v {
        Value::Array(items) => items.iter().any(|i| any_key(i, f)),
        Value::Object(map) => map.iter().any(|(k, v)| f(k, v) || any_key(v, f)),
        _ => false,
    }
}

/// `text` lowercased, letters and digits only: `command_line`, `commandLine` and `Command-Line`
/// all become `commandline`.
fn flat(text: &str) -> String {
    text.chars().filter(|c| c.is_alphanumeric()).flat_map(char::to_lowercase).collect()
}

/// The lowercased words of `text`. Anything but letters and digits (spaces, tabs, `_`, `-`, `/`,
/// quotes, dots) is a break, and so is a camelCase hump: `pushToOrigin` is push, to, origin, and
/// `HTTPPush` is http, push. A word in one case throughout (`Pushkin`, `PUSHKIN`) stays whole.
fn words(text: &str) -> Vec<String> {
    let chars: Vec<char> = text.chars().collect();
    let mut out = Vec::new();
    let mut cur = String::new();
    for (i, &c) in chars.iter().enumerate() {
        if !c.is_alphanumeric() {
            if !cur.is_empty() {
                out.push(std::mem::take(&mut cur));
            }
            continue;
        }
        if c.is_uppercase() && !cur.is_empty() {
            // cur is non-empty, so chars[i - 1] is a letter or digit of this word
            let prev = chars[i - 1];
            let next_lower = chars.get(i + 1).is_some_and(|n| n.is_lowercase());
            if prev.is_lowercase() || prev.is_numeric() || (prev.is_uppercase() && next_lower) {
                out.push(std::mem::take(&mut cur));
            }
        }
        cur.extend(c.to_lowercase());
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    out
}

/// Whether an argument key carries free text to run: one of [`RUNS_TEXT_KEYS`], a key holding a
/// [`RUNS_TEXT_KEY_PARTS`] part once flattened, or one with a [`RUNS_TEXT_KEY_WORDS`] word.
fn is_run_key(k: &str) -> bool {
    RUNS_TEXT_KEYS.iter().any(|r| k.eq_ignore_ascii_case(r))
        || RUNS_TEXT_KEY_PARTS.iter().any(|p| flat(k).contains(p))
        || words(k).iter().any(|w| RUNS_TEXT_KEY_WORDS.contains(&w.as_str()))
}

/// Whether any key at any depth carries free text to run ([`is_run_key`]).
fn has_run_key(v: &Value) -> bool {
    any_key(v, &|k, _| is_run_key(k))
}

/// Whether `text` holds a [`PUSH_FAMILY`] word as a whole word ([`words`]).
fn has_push_word(text: &str) -> bool {
    let words = words(text);
    PUSH_FAMILY.iter().any(|phrase| words.windows(phrase.len()).any(|w| w.iter().zip(phrase.iter()).all(|(a, b)| a == b)))
}

/// Whether `name` (already split by [`words`]) has a [`READ_VERBS`] word, so it only reads.
fn reads(name: &[String]) -> bool {
    name.iter().any(|w| READ_VERBS.contains(&w.as_str()))
}

/// Whether a tool name ships code by its words (20d: one place for Telegram and the island): a
/// [`PUSH_WORDS`] run or a [`COMMIT_NAME_WORDS`] word, in a name with no [`READ_VERBS`] word.
/// `GITHUB_GET_A_RELEASE`, `list_merge_requests` and `get_commit` only read; `GITHUB_MERGE_A_PULL_REQUEST`,
/// `GITHUB_CREATE_A_RELEASE`, `push_files` and `GITHUB_UPDATE_A_PULL_REQUEST_BRANCH` ship.
fn ships_by_name(tool: &str) -> bool {
    let name = words(tool);
    let push = PUSH_WORDS.iter().any(|run| {
        let Some((last, head)) = run.split_last() else { return false };
        name.windows(run.len()).any(|w| w[..head.len()].iter().zip(head.iter()).all(|(a, b)| a == b) && w[head.len()].starts_with(last))
    });
    let commit = name.iter().any(|w| COMMIT_NAME_WORDS.contains(&w.as_str()));
    (push || commit) && !reads(&name)
}

/// Whether a tool or its server is named for running free text (20d: one place for Telegram and the
/// island): a word of either name starting with a [`RUNS_TEXT_NAMES`] word, or one of
/// [`RUNS_TEXT_NAME_WORDS`]. A read verb doesn't lift it: `run_query` and `get_shell` still run.
fn runs_by_name(server: &str, tool: &str) -> bool {
    words(server).iter().chain(words(tool).iter()).any(|w| RUNS_TEXT_NAMES.iter().any(|r| w.starts_with(r)) || RUNS_TEXT_NAME_WORDS.contains(&w.as_str()))
}

/// Whether a flattened argument key names an HTTP verb: `method`, `http_method`, `httpMethod`,
/// `verb`, `request_verb` (18c, H03), or an `X-HTTP-Method-Override` header (18c, H05).
fn is_method_key(f: &str) -> bool {
    f.ends_with("method") || f.ends_with("verb") || f.contains("methodoverride")
}

/// Whether a flattened argument key names what a generic tool does: `action`, `operation`, `op`
/// (18c, H01–H02). Exact, so `transaction` and `cooperation` stay out.
fn is_action_key(f: &str) -> bool {
    matches!(f, "action" | "operation" | "op")
}

/// Whether the arguments ask for an HTTP verb that changes something. A method key at any depth
/// ([`is_method_key`]) whose value is one of [`WRITE_METHODS`] always does. On an HTTP-shaped call
/// (a `url`, `uri` or `endpoint` key anywhere) a method that isn't a plain [`READ_METHODS`] string
/// does too: `CONNECT`, a made-up verb, a list or a number. An action key ([`is_action_key`]) whose
/// value has an [`ACTION_WRITE_WORDS`] word does (`delete`, `DELETE`, `remove_item`). And an
/// HTTP-shaped call with a body ([`BODY_KEYS`]) but no method key at all does, since such tools
/// send it as a POST.
fn writes_by_method(args: &Value) -> bool {
    let http = any_key(args, &|k, _| {
        let f = flat(k);
        f == "uri" || f == "endpoint" || f.ends_with("url")
    });
    let by_key = any_key(args, &|k, v| {
        let f = flat(k);
        if is_action_key(&f) {
            let mut all = Vec::new();
            texts(v, &mut all);
            return words(&all.join(" ")).iter().any(|w| ACTION_WRITE_WORDS.contains(&w.as_str()));
        }
        if !is_method_key(&f) {
            return false;
        }
        match v.as_str() {
            Some(s) => {
                let verb = s.trim().to_ascii_uppercase();
                WRITE_METHODS.contains(&verb.as_str()) || (http && !READ_METHODS.contains(&verb.as_str()))
            }
            None => http && !v.is_null(),
        }
    });
    by_key
        || (http
            && !any_key(args, &|k, _| is_method_key(&flat(k)))
            && any_key(args, &|k, v| !v.is_null() && BODY_KEYS.contains(&flat(k).as_str())))
}

/// Why an approval may only be answered on its full card, from its stored details alone; `None`
/// when the island may offer Allow once. Starts from Telegram's rule (money, the production floor,
/// a tier-forced ask, a turn that read outside text or that kwant started on its own, a shell
/// command, kwant's own tools, wrappers that run other actions, file tools outside the bot's
/// folder), then adds what a one-line peek can't show safely: a risky step (`smart_reason`), the
/// push floor (a prepush grant, a GitHub write, a merge) and details it can't read.
///
/// It fails closed in four layers, each of which refuses on its own (Ada, #646 round 2; 0.9.14):
/// 1. a push-family word ([`PUSH_FAMILY`]) in the tool name (unless a [`READ_VERBS`] word says it
///    only reads), an argument key or a string value, as a whole word, camelCase humps included
///    ([`words`]);
/// 2. a tool that runs free text: its tool or server name says so ([`runs_by_name`]) or an
///    argument key at any depth does ([`is_run_key`]: exact, by containment, or by word);
/// 3. a tool whose name acts outside kwant or destroys something ([`OUTWARD_NAMES`], or a whole
///    word of [`OUTWARD_NAME_WORDS`]);
/// 4. an HTTP verb that changes something in a method or action argument, or a body sent to a URL
///    with no method ([`writes_by_method`]).
pub fn inline_refusal(details: &Value) -> Option<&'static str> {
    if !details.is_object() {
        return Some("Open it to see what it does.");
    }
    if let Some(why) = super::telegram::link_only_reason(details) {
        return Some(match why {
            "This kind of step can't be approved from Telegram." => "This kind of step needs the full card.",
            other => other,
        });
    }
    if let Some(why) = ships(details) {
        return Some(why);
    }
    let tool = details["tool"].as_str().unwrap_or_default();
    let server = details["server"].as_str().unwrap_or_default();
    let args = &details["arguments"];
    let upper = tool.to_ascii_uppercase();
    // 1. A push-family word wherever it hides: the name (unless it only reads, 20d), a key, a
    //    value, an argv item
    let mut all = Vec::new();
    texts(args, &mut all);
    if has_push_word(&all.join(" ")) || (has_push_word(tool) && !reads(&words(tool))) {
        return Some(PUSHES);
    }
    // 2. Free text to run, whatever it says
    if runs_by_name(server, tool) || has_run_key(args) {
        return Some(RUNS);
    }
    // 3. Acts outside kwant or destroys something
    if OUTWARD_NAMES.iter().any(|w| upper.contains(w)) || words(tool).iter().any(|w| OUTWARD_NAME_WORDS.contains(&w.as_str())) {
        return Some(OUTWARD);
    }
    // 4. An HTTP call that changes something, whatever the tool is called
    if writes_by_method(args) {
        return Some(OUTWARD);
    }
    None
}

/// A risky step or the push floor, from the details alone: a step smart approvals flagged
/// ([`RISKY`]); a prepush grant, a strict ask, a GitHub write [`mcp_writes`] sees, or a tool
/// named for a push, a merge, a release, a commit or a branch that doesn't only read
/// ([`ships_by_name`], [`PUSHES`]). The first layers of [`inline_refusal`], in its
/// order.
///
/// [`mcp_writes`]: super::dev::prepush::mcp_writes
fn ships(details: &Value) -> Option<&'static str> {
    let set = |k: &str| details.get(k).is_some_and(|v| !v.is_null() && v != false);
    if set("smart_reason") {
        return Some(RISKY);
    }
    if set("prepush_grant") || set("strict_ask") {
        return Some(PUSHES);
    }
    let tool = details["tool"].as_str().unwrap_or_default();
    if !super::dev::prepush::mcp_writes(tool, &details["arguments"]).is_empty() {
        return Some(PUSHES);
    }
    if ships_by_name(tool) {
        return Some(PUSHES);
    }
    None
}

/// Argument keys (any depth, any case) that Telegram's floor reads as free text to run: the
/// brief's four, plus `sql` (the Postgres server's `query` tool takes `{"sql": …}`). Narrower than
/// [`is_run_key`] on purpose: `query`, `args` and `code` sit on many search and sign-in tools that
/// Telegram keeps allowable (18b).
const TELEGRAM_RUN_KEYS: &[&str] = &["command", "cmd", "argv", "script", "sql"];

/// What Telegram's Allow once adds to its own floor (18b, Rigel's push-floor advisory): a step
/// that ships code or runs a command, or one kwant flagged as risky. [`ships`], then a tool that
/// runs free text: a server or tool name word for it ([`runs_by_name`]), or an argument key that
/// is exactly one of [`TELEGRAM_RUN_KEYS`] ([`RUNS`]). Whatever it refuses,
/// [`inline_refusal`] refuses too; the island's other layers (outward names, HTTP verbs, push words
/// inside values, the broad run keys) stay off Telegram on purpose, so an email that says
/// "press release" keeps Allow once there. Since 18c [`ships`] also refuses a tool named for a
/// commit or a branch (`create_commit`, `create_branch`), so Telegram refuses those too.
pub fn ships_or_runs(details: &Value) -> Option<&'static str> {
    if let Some(why) = ships(details) {
        return Some(why);
    }
    let tool = details["tool"].as_str().unwrap_or_default();
    let server = details["server"].as_str().unwrap_or_default();
    if runs_by_name(server, tool) || any_key(&details["arguments"], &|k, _| TELEGRAM_RUN_KEYS.iter().any(|r| k.eq_ignore_ascii_case(r))) {
        return Some(RUNS);
    }
    None
}

/// [`inline_refusal`], plus the space's production marks (as Telegram's [`link_only_in`]).
///
/// [`link_only_in`]: super::telegram::link_only_in
pub async fn can_allow_inline(state: &AppState, space: &str, details: &Value) -> Result<(), &'static str> {
    if let Some(why) = inline_refusal(details) {
        return Err(why);
    }
    match super::telegram::link_only_in(state, space, details).await {
        None => Ok(()),
        Some("This is a production connection.") => Err("This is a production connection."),
        Some(_) => Err("This kind of step needs the full card."),
    }
}

/// One Needs-you item: `/api/away`'s waiting item in the island's shape.
async fn needs_you_item(state: &AppState, space: &Space, v: &Value) -> Value {
    let kind = match (v["kind"].as_str().unwrap_or(""), v.get("human_step_id").is_some_and(|h| !h.is_null())) {
        ("stopped", true) => "human_step",
        (k, _) => k,
    };
    let id = v["id"].as_str().unwrap_or("");
    let kwant_id = v["kwant_id"].as_str().unwrap_or("");
    let card = matches!(kind, "approval" | "question").then_some(id);
    let mut out = json!({
        "kind": kind,
        "id": id,
        "space_id": space.id,
        "space_name": line(&space.name, 60),
        "kwant_id": kwant_id,
        "kwant_name": line(v["kwant_name"].as_str().unwrap_or("A bot"), 60),
        "title": line(v["title"].as_str().unwrap_or(""), 120),
        "since": v["timestamp"],
        "path": app_path(&space.id, &v["link"], kwant_id, card),
        "approval": Value::Null,
        "question": Value::Null,
    });
    if kind == "approval" {
        // The stored details, unmasked, decide; none of them is sent
        let details = match state.db.get_approval(id).await {
            Ok(Some(a)) => serde_json::from_str(&a.parameters).unwrap_or(Value::Null),
            _ => Value::Null,
        };
        let verdict = can_allow_inline(state, &space.id, &details).await;
        out["approval"] = json!({
            "risk": if verdict.is_ok() { "normal" } else { "high" },
            "can_allow_inline": verdict.is_ok(),
            "why_open": verdict.err(),
        });
    }
    if kind == "question" {
        let all: Vec<&str> = v["choices"].as_array().map(|c| c.iter().filter_map(Value::as_str).collect()).unwrap_or_default();
        let shown: Vec<String> = all.iter().take(CHOICES_MAX).map(|c| line(c, CHOICE_CHARS)).collect();
        out["question"] = json!({ "choices": shown, "choices_total": all.len(), "allow_text": true });
    }
    out
}

/// A running turn, as Working now shows it.
struct Working {
    since: DateTime<Utc>,
    value: Value,
}

async fn working(state: &AppState, spaces: &HashMap<String, &Space>, names: &mut HashMap<String, (String, String)>) -> Vec<Working> {
    let mut out = Vec::new();
    for t in state.turns.active_turns() {
        // The hidden decide-first round isn't work anyone asked for; a queued turn isn't working
        if t.purpose.as_deref() == Some("decide") || t.state != "running" {
            continue;
        }
        if !names.contains_key(&t.kwant_id) {
            let Ok(Some(k)) = state.db.get_kwant(&t.kwant_id).await else { continue };
            names.insert(t.kwant_id.clone(), (k.name, k.space_id));
        }
        let (name, space_id) = names[&t.kwant_id].clone();
        if !spaces.contains_key(&space_id) {
            continue;
        }
        let link = match &t.thread_id {
            Some(thread) => json!({"thread_id": thread}),
            None => match state.db.get_channel(&t.context_id).await {
                Ok(Some(_)) => json!({"channel_id": t.context_id}),
                _ => json!({"direct_kwant_id": t.context_id}),
            },
        };
        let (step, step_at) = super::activity::current_step(&t.turn_id).unwrap_or_else(|| ("Getting started".into(), rfc3339(t.since)));
        out.push(Working {
            since: t.since,
            value: json!({
                "turn_id": t.turn_id,
                "kwant_id": t.kwant_id,
                "kwant_name": line(&name, 60),
                "space_id": space_id,
                "step": line(&step, STEP_CHARS),
                "step_started_at": step_at,
                "started_at": rfc3339(t.since),
                "path": app_path(&space_id, &link, &t.kwant_id, None),
            }),
        });
    }
    out
}

/// One Just-finished item, with its done-check line when the item has one.
async fn finished_item(state: &AppState, space: &Space, row: crate::db::away::AwayRow, unseen: bool) -> Value {
    let facts = super::inbox_done::away_facts(state, &row.kind, &row.id).await.unwrap_or_default();
    let check = (!facts.tasks.is_empty()).then(|| {
        let c = super::inbox_done::checked(&facts.tasks, facts.message.as_ref().map(super::inbox_done::meta_of).as_ref());
        let status = match c["state"].as_str() {
            Some("checked") => "verified",
            Some("failed") => "failed",
            _ => "not_checked",
        };
        json!({ "status": status, "line": line(c["line"].as_str().unwrap_or(""), 120) })
    });
    let at = row.at;
    let v = away::item(row);
    let kwant_id = v["kwant_id"].as_str().unwrap_or("");
    let result_link = facts.message.as_ref().map(|m| json!({"thread_id": m.thread_id, "channel_id": m.channel_id, "direct_kwant_id": m.direct_kwant_id}));
    json!({
        "kind": v["kind"],
        "id": v["id"],
        "space_id": space.id,
        "kwant_id": kwant_id,
        "kwant_name": line(v["kwant_name"].as_str().unwrap_or("A bot"), 60),
        "title": line(v["title"].as_str().unwrap_or(""), 120),
        "status": v["status"],
        "finished_at": rfc3339(at),
        "unseen": unseen,
        "check": check,
        "path": app_path(&space.id, result_link.as_ref().unwrap_or(&v["link"]), kwant_id, None),
    })
}

/// The bots a quick ask can go to: the person's most recent direct chats in live spaces, then the
/// rest by name. There are no server-side pins (the sidebar's pins live in the browser), so the
/// app puts its own pins first.
async fn ask_targets(state: &AppState) -> Result<Vec<Value>, crate::db::DbError> {
    let rows = state.db.ask_targets(ASK_TARGETS_MAX).await?;
    Ok(rows.into_iter().map(|(id, name, space)| json!({"kwant_id": id, "name": line(&name, 60), "space_id": space})).collect())
}

/// A bot's saved `avatar_sprite` (as the bot's own settings return it), or `null` when the item
/// names no bot, the bot is gone, or its sprite is empty.
pub fn avatar_of(avatars: &HashMap<String, String>, kwant_id: Option<&str>) -> Value {
    match kwant_id.filter(|k| !k.is_empty()).and_then(|k| avatars.get(k)) {
        Some(sprite) if !sprite.trim().is_empty() => Value::String(sprite.clone()),
        _ => Value::Null,
    }
}

/// The glance without `now`, which the ETag is taken over.
pub async fn build(state: &AppState, now: DateTime<Utc>) -> Result<Value, crate::db::DbError> {
    let spaces: Vec<Space> = state.db.list_spaces().await?.into_iter().filter(|s| s.archived_at.is_none()).collect();
    let live: HashMap<String, &Space> = spaces.iter().map(|s| (s.id.clone(), s)).collect();
    // Approvals and mission watches are read once for every space, not once per space
    let mut approvals = away::approvals_by_space(state, None).await?;
    let mut watch = crate::server::missions::watch::waiting_items_by_space(state).await;
    let mut waiting: Vec<(&Space, Value)> = Vec::new();
    let mut needs_you_count = 0;
    let mut finished_rows: Vec<(bool, &Space, crate::db::away::AwayRow)> = Vec::new();
    let mut finished_unseen = 0;
    let day_ago = now - Duration::hours(FINISHED_HOURS);
    let stuck_before = now - Duration::minutes(away::STUCK_AFTER_MINUTES);
    for space in &spaces {
        let id = SpaceId::new(space.id.clone());
        let mine = approvals.remove(&id).unwrap_or_default();
        let watching = watch.remove(&space.id).unwrap_or_default();
        let (items, count) = away::waiting_with(state, &id, mine, watching).await?;
        needs_you_count += count;
        waiting.extend(items.into_iter().take(NEEDS_YOU_MAX).map(|v| (space, v)));
        let seen = away::last_seen(state, &id).await?;
        let from = seen.map_or(day_ago, |s| s.max(day_ago));
        finished_unseen += state.db.away_counts(&id, from, stuck_before).await?.0;
        for row in state.db.away_finished(&id, day_ago, FINISHED_MAX as i64).await? {
            finished_rows.push((seen.is_none_or(|s| row.at > s), space, row));
        }
    }
    // Cut first, then build only the items that are sent
    waiting.sort_by(|(_, a), (_, b)| b["timestamp"].as_str().unwrap_or("").cmp(a["timestamp"].as_str().unwrap_or("")).then_with(|| b["id"].as_str().cmp(&a["id"].as_str())));
    waiting.truncate(NEEDS_YOU_MAX);
    let mut needs_you = Vec::with_capacity(waiting.len());
    for (space, v) in &waiting {
        needs_you.push(needs_you_item(state, space, v).await);
    }
    finished_rows.sort_by(|a, b| b.0.cmp(&a.0).then(b.2.at.cmp(&a.2.at)));
    finished_rows.truncate(FINISHED_MAX);
    let mut finished = Vec::with_capacity(finished_rows.len());
    for (unseen, space, row) in finished_rows {
        finished.push(finished_item(state, space, row, unseen).await);
    }

    let mut names = HashMap::new();
    let mut work = working(state, &live, &mut names).await;
    let working_count = work.len();
    work.sort_by(|a, b| b.since.cmp(&a.since).then_with(|| b.value["turn_id"].as_str().cmp(&a.value["turn_id"].as_str())));
    work.truncate(WORKING_MAX);

    // The bot's face for every item that names one (`null` when there's no bot or no sprite).
    // It is part of the body, so a changed face gives a new ETag.
    let avatars: HashMap<String, String> = state.db.avatar_sprites().await?.into_iter().collect();
    let mut work: Vec<Value> = work.into_iter().map(|w| w.value).collect();
    let mut targets = ask_targets(state).await?;
    for v in needs_you.iter_mut().chain(work.iter_mut()).chain(finished.iter_mut()).chain(targets.iter_mut()) {
        let avatar = avatar_of(&avatars, v["kwant_id"].as_str());
        v["avatar"] = avatar;
    }

    Ok(json!({
        "counts": {
            "needs_you": needs_you_count,
            "working": working_count,
            "finished_unseen": finished_unseen,
        },
        "needs_you": needs_you,
        "working": work,
        "finished": finished,
        "ask_targets": targets,
    }))
}

/// A weak ETag over the glance's content (not its `now`).
pub fn etag_of(body: &Value) -> String {
    let digest = Sha256::digest(body.to_string().as_bytes());
    let hex: String = digest.iter().take(12).map(|b| format!("{b:02x}")).collect();
    format!("W/\"g-{hex}\"")
}

/// Whether `If-None-Match` names this ETag (or `*`). Weak comparison, as for a GET.
pub fn matches(header: Option<&str>, etag: &str) -> bool {
    let Some(h) = header else { return false };
    let bare = |t: &str| t.trim().trim_start_matches("W/").to_string();
    h.split(',').any(|t| t.trim() == "*" || bare(t) == bare(etag))
}

/// `GET /api/glance`: see the module docs. 304 with no body when `If-None-Match` names the
/// current ETag.
pub async fn get(State(state): State<AppState>, headers: HeaderMap) -> Response {
    let now = Utc::now();
    let Ok(mut body) = build(&state, now).await else {
        return internal();
    };
    let etag = etag_of(&body);
    let cache = [(header::CACHE_CONTROL, HeaderValue::from_static("private, no-cache"))];
    let etag_header = HeaderValue::from_str(&etag).unwrap_or_else(|_| HeaderValue::from_static("W/\"g\""));
    if matches(headers.get(header::IF_NONE_MATCH).and_then(|h| h.to_str().ok()), &etag) {
        return (StatusCode::NOT_MODIFIED, cache, [(header::ETAG, etag_header)]).into_response();
    }
    body["now"] = json!(rfc3339(now));
    (cache, [(header::ETAG, etag_header)], Json(body)).into_response()
}

#[derive(Debug, Deserialize)]
pub struct AskPayload {
    pub space_id: String,
    pub kwant_id: String,
    pub text: String,
    /// The app's own id for this ask, the same on every retry: sent once (as the composer's).
    #[serde(default)]
    pub client_message_id: Option<String>,
}

/// `POST /api/glance/ask {space_id, kwant_id, text, client_message_id?}`: sends `text` into the
/// person's chat with that bot, exactly as if typed there (the bot answers there; nothing opens).
/// 200 `{path, user_message_id}`; 400 for an empty or too long text, 404 for a bot not in that
/// space, 409 for an archived space.
pub async fn ask(State(state): State<AppState>, Json(p): Json<AskPayload>) -> Response {
    let text = p.text.trim();
    if text.is_empty() {
        return error(StatusCode::BAD_REQUEST, "empty", "Type something to ask.");
    }
    if text.chars().count() > ASK_MAX_CHARS {
        return error(StatusCode::BAD_REQUEST, "too_long", "That's too long for a quick ask. Open the chat to send it.");
    }
    let kwant = match state.db.get_kwant(&p.kwant_id).await {
        Ok(Some(k)) if k.space_id == p.space_id => k,
        Ok(_) => return error(StatusCode::NOT_FOUND, "not_found", "That bot isn't in this space."),
        Err(_) => return internal(),
    };
    let space = SpaceId::new(p.space_id.clone());
    if let Err(r) = super::spaces::ensure_active(&state, &space).await {
        return r;
    }
    let payload = super::routes::SendChatPayload {
        channel_id: None,
        kwant_id: Some(kwant.id.clone()),
        team_id: None,
        thread_id: None,
        prompt: text.to_string(),
        sender_id: "PlayerHero".into(),
        attachment_ids: Vec::new(),
        reply_to_message_id: None,
        source: None,
        meta: None,
        client_message_id: p.client_message_id,
        message_id: None,
    };
    match super::routes::send_chat_inner(state, payload, true).await {
        Ok(Json(v)) => Json(json!({
            "path": app_path(&p.space_id, &json!({"direct_kwant_id": kwant.id}), &kwant.id, None),
            "user_message_id": v["user_message_id"],
            "deduped": v["deduped"].as_bool().unwrap_or(false),
        }))
        .into_response(),
        Err(StatusCode::CONFLICT) => error(StatusCode::CONFLICT, "archived", "This space is archived."),
        Err(StatusCode::BAD_REQUEST) => error(StatusCode::BAD_REQUEST, "bad_request", "That ask couldn't be sent."),
        Err(_) => internal(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mcp(server: &str, tool: &str, args: Value) -> Value {
        json!({"source": "mcp", "server": server, "tool": tool, "arguments": args})
    }

    #[test]
    fn a_plain_mcp_call_may_be_allowed_inline() {
        assert_eq!(inline_refusal(&mcp("notion", "create_page", json!({"title": "Notes"}))), None);
        let read = json!({"source": "claude_builtin", "tool": "Read", "outside_folder": false, "arguments": {"file_path": "a.txt"}});
        assert_eq!(inline_refusal(&read), None);
    }

    #[test]
    fn money_risk_and_the_floors_need_the_full_card() {
        let with = |k: &str, v: Value| {
            let mut d = mcp("stripe", "create_invoice", json!({}));
            d[k] = v;
            d
        };
        for (k, v) in [
            ("payment_reason", json!("Sends money")),
            ("prod_floor", json!({"server": "db"})),
            ("tier_reason", json!("Tier 3")),
            ("smart_reason", json!("Deletes records")),
            ("prepush_grant", json!({"repo": "x"})),
            ("tainted", json!(true)),
            ("always_ask", json!(true)),
        ] {
            assert!(inline_refusal(&with(k, v)).is_some(), "{k}");
        }
        // A shell command, kwant's own tools, wrappers and files outside the folder
        assert!(inline_refusal(&json!({"source": "local_shell", "tool": "shell", "arguments": {"command": "ls"}})).is_some());
        assert!(inline_refusal(&json!({"source": "claude_builtin", "tool": "Bash", "arguments": {"command": "ls"}})).is_some());
        assert!(inline_refusal(&json!({"source": "claude_builtin", "tool": "Write", "arguments": {}})).is_some());
        assert!(inline_refusal(&mcp("composio", "COMPOSIO_MULTI_EXECUTE_TOOL", json!({}))).is_some());
        assert!(inline_refusal(&Value::Null).is_some(), "unreadable details: false");
        assert!(inline_refusal(&json!({"tool": "x"})).is_some(), "no source: false");
    }

    #[test]
    fn pushes_and_merges_need_the_full_card() {
        let write = mcp("github", "create_or_update_file", json!({"owner": "o", "repo": "r", "path": "a", "content": "x", "message": "m", "branch": "main"}));
        assert!(inline_refusal(&write).is_some());
        assert!(inline_refusal(&mcp("github", "merge_pull_request", json!({"owner": "o", "repo": "r", "pull_number": 1}))).is_some());
        assert!(inline_refusal(&mcp("composio", "GITHUB_MERGE_A_PULL_REQUEST", json!({}))).is_some());
        assert!(inline_refusal(&mcp("runner", "run", json!({"cmd": "cd x && git push origin main"}))).is_some());
        assert!(inline_refusal(&mcp("github", "create_issue", json!({"owner": "o", "repo": "r", "title": "Bug"}))).is_none());
    }

    /// Rigel's round-2 risk-4 rows (#646): fifteen commands and four outward tools that came back
    /// Allow once at 184d8fd0. tests/glance_test.rs reads the same rows back through the API.
    fn risky_rows() -> Vec<(&'static str, Value)> {
        let run = |c: &str| mcp("runner", "run", json!({ "command": c }));
        vec![
            ("git  push (two spaces)", run("git  push origin main")),
            ("git<TAB>push", run("git\tpush origin main")),
            ("git -C dir push", run("git -C /repo push origin main")),
            ("git -c k=v push", run("git -c core.x=1 push")),
            ("argv [git, push]", mcp("runner", "run", json!({"argv": ["git", "push", "origin", "main"]}))),
            ("gh pr merge", run("gh pr merge 5 --squash")),
            ("gh release create", run("gh release create v1")),
            ("gh api -X PUT merge", run("gh api -X PUT repos/o/r/pulls/5/merge")),
            ("git send-pack", run("git send-pack origin main")),
            ("npm publish", run("npm publish")),
            ("cargo publish", run("cargo publish")),
            ("docker push", run("docker push img")),
            ("curl POST git/refs", run("curl -X POST https://api.github.com/repos/o/r/git/refs")),
            ("rm -rf /", run("rm -rf /")),
            ("bitbucket pr_action merge", mcp("bitbucket", "pr_action", json!({"action": "merge"}))),
            ("publish_site", mcp("netlify", "publish_site", json!({}))),
            ("deploy", mcp("vercel", "deploy", json!({}))),
            ("send_email", mcp("gmail", "send_email", json!({}))),
            ("delete_page", mcp("notion", "delete_page", json!({}))),
        ]
    }

    #[test]
    fn every_round_two_row_needs_the_full_card() {
        let rows = risky_rows();
        assert_eq!(rows.len(), 19);
        for (name, d) in rows {
            assert!(inline_refusal(&d).is_some(), "{name}");
        }
    }

    #[test]
    fn each_layer_refuses_on_its_own() {
        // 1. Push-family words, whole words only, in any key or value, at any depth
        let words = |args: Value| inline_refusal(&mcp("notes", "save_note", args));
        assert_eq!(words(json!({"a": {"b": ["x", "Push it"]}})), Some(PUSHES));
        assert_eq!(words(json!({"what_to_merge": "a"})), Some(PUSHES), "a key");
        assert_eq!(words(json!({"t": "receive-pack"})), Some(PUSHES));
        assert_eq!(words(json!({"t": "v1 TAG"})), Some(PUSHES));
        assert_eq!(words(json!({"t": "pushkin, merger, tags, published"})), None, "whole words only");
        assert_eq!(inline_refusal(&mcp("github", "create_tag", json!({}))), Some(PUSHES), "a name word");
        // 2. Free text to run: by tool name, server name or a key at any depth
        assert_eq!(inline_refusal(&mcp("notes", "Execute", json!({}))), Some(RUNS));
        assert_eq!(inline_refusal(&mcp("my-ssh", "connect", json!({}))), Some(RUNS));
        assert_eq!(inline_refusal(&mcp("db", "read_rows", json!({"filter": {"SQL": "select 1"}}))), Some(RUNS));
        assert_eq!(inline_refusal(&mcp("notes", "save_note", json!({"items": [{"code": "x"}]}))), Some(RUNS));
        // 3. Outward or destructive names
        assert_eq!(inline_refusal(&mcp("x", "Remove_Item", json!({}))), Some(OUTWARD));
        assert_eq!(inline_refusal(&mcp("x", "create_post", json!({}))), Some(OUTWARD));
        assert_eq!(inline_refusal(&mcp("x", "pay_invoice", json!({}))), Some(OUTWARD));
    }

    /// The 0.9.14 hardening (Rigel's holes 1–4 on #646): one row per new rule, each on a tool name
    /// no older layer refuses, with the reason it must give.
    fn hardening_rows() -> Vec<(&'static str, Value, &'static str)> {
        let note = |args: Value| mcp("notes", "save_note", args);
        let http = |args: Value| mcp("http", "http_request", args);
        let url = "https://api.example.com/items/1";
        vec![
            // 1. Command keys by containment or word
            ("key command_line", note(json!({"command_line": "ls -la"})), RUNS),
            ("key commandLine, nested", note(json!({"opts": {"commandLine": "ls -la"}})), RUNS),
            ("key shell_command", note(json!({"shell_command": "whoami"})), RUNS),
            ("key cmdline", note(json!({"cmdline": "whoami"})), RUNS),
            ("key bash_command", note(json!({"bash_command": "whoami"})), RUNS),
            ("key exec_args in a list", note(json!({"steps": [{"exec_args": ["x"]}]})), RUNS),
            ("key user_script", note(json!({"user_script": "x()"})), RUNS),
            // 2. camelCase push-family words
            ("key pushToOrigin", note(json!({"pushToOrigin": true})), PUSHES),
            ("key mergeBranch", note(json!({"mergeBranch": "dev"})), PUSHES),
            ("value forcePush", note(json!({"op": "forcePush"})), PUSHES),
            ("value HTTPPush", note(json!({"op": "HTTPPush"})), PUSHES),
            ("tool createTag", mcp("git_ops", "createTag", json!({})), PUSHES),
            ("tool v2Deploy", mcp("site", "v2Deploy", json!({})), PUSHES),
            // 3. Outward names
            ("submit_form", mcp("x", "submit_form", json!({})), OUTWARD),
            ("transfer_funds", mcp("x", "transfer_funds", json!({})), OUTWARD),
            ("tweet", mcp("x", "tweet", json!({})), OUTWARD),
            ("share_page", mcp("x", "share_page", json!({})), OUTWARD),
            ("invite_user", mcp("x", "invite_user", json!({})), OUTWARD),
            ("forward_message", mcp("x", "forward_message", json!({})), OUTWARD),
            ("upload_file", mcp("x", "upload_file", json!({})), OUTWARD),
            ("wipe_disk", mcp("x", "wipe_disk", json!({})), OUTWARD),
            ("terminate_instance", mcp("x", "terminate_instance", json!({})), OUTWARD),
            ("revoke_token", mcp("x", "revoke_token", json!({})), OUTWARD),
            ("purchase_item", mcp("x", "purchase_item", json!({})), OUTWARD),
            // 4. HTTP verbs that change something
            ("method DELETE", http(json!({"url": url, "method": "DELETE"})), OUTWARD),
            ("method PUT", http(json!({"url": url, "method": "PUT"})), OUTWARD),
            ("method PATCH", http(json!({"url": url, "method": "PATCH"})), OUTWARD),
            ("method POST", http(json!({"url": url, "method": "POST"})), OUTWARD),
            ("httpMethod post, nested, no url", mcp("api", "call", json!({"options": {"httpMethod": " post "}})), OUTWARD),
            ("method CONNECT on a URL", http(json!({"url": url, "method": "CONNECT"})), OUTWARD),
            ("method as a list on a URL", http(json!({"url": url, "method": ["GET", "DELETE"]})), OUTWARD),
        ]
    }

    #[test]
    fn every_0914_hardening_row_needs_the_full_card() {
        let rows = hardening_rows();
        assert_eq!(rows.len(), 31);
        for (name, d, why) in rows {
            assert_eq!(inline_refusal(&d), Some(why), "{name}");
        }
    }

    /// What the 0.9.14 rules must not catch: a read, a `description` key (it holds "script"), a
    /// camelCase key with no push word, "Pushkin" in a hump, and a `method` that isn't HTTP.
    #[test]
    fn the_0914_hardening_keeps_ordinary_steps_inline() {
        let url = "https://api.example.com/items/1";
        for (name, d) in [
            ("method GET on a URL", mcp("http", "http_request", json!({"url": url, "method": "get"}))),
            ("method HEAD on a URL", mcp("http", "http_request", json!({"url": url, "method": "HEAD"}))),
            ("a description key", mcp("linear", "create_issue", json!({"title": "Bug", "description": "Steps to reproduce"}))),
            ("a camelCase key", mcp("notion", "create_page", json!({"pageTitle": "Reading list"}))),
            ("Pushkin in a hump", mcp("notion", "create_page", json!({"title": "readPushkin"}))),
            ("a method that isn't HTTP", mcp("stats", "summarize", json!({"method": "median"}))),
        ] {
            assert_eq!(inline_refusal(&d), None, "{name}");
        }
        assert_eq!(words("pushToOrigin HTTPPush v2Deploy Pushkin PUSHKIN git_push"), ["push", "to", "origin", "http", "push", "v2", "deploy", "pushkin", "pushkin", "git", "push"]);
        for (name, d) in realistic_rows() {
            assert_eq!(inline_refusal(&d), None, "{name}");
        }
    }

    /// Realistic steps that still get Allow once, so the rule isn't "always no".
    fn realistic_rows() -> Vec<(&'static str, Value)> {
        vec![
            (
                "a file write inside the bot's folder",
                json!({"source": "claude_builtin", "tool": "Write", "outside_folder": false, "arguments": {"file_path": "notes/todo.md", "content": "Buy milk\nCall Ana"}}),
            ),
            ("a read-only fetch", mcp("fetch", "fetch", json!({"url": "https://example.com/docs", "max_length": 5000}))),
            ("a page that mentions Pushkin", mcp("notion", "create_page", json!({"title": "Pushkin reading list", "parent": {"page_id": "abc"}}))),
        ]
    }

    #[test]
    fn realistic_steps_may_still_be_allowed_inline() {
        for (name, d) in realistic_rows() {
            assert_eq!(inline_refusal(&d), None, "{name}");
        }
    }

    // 18b: Telegram's floor is a subset of the island's, so the island never offers what
    // Telegram refuses
    #[test]
    fn whatever_telegram_s_floor_refuses_the_island_refuses_too() {
        let mut rows: Vec<Value> = risky_rows().into_iter().map(|(_, d)| d).collect();
        rows.extend(hardening_rows().into_iter().map(|(_, d, _)| d));
        rows.extend(realistic_rows().into_iter().map(|(_, d)| d));
        rows.push(mcp("postgres", "query", json!({"sql": "DROP TABLE users"})));
        rows.push(mcp("devtools", "run_command", json!({"cmd": "gh  pr merge 12"})));
        rows.push(mcp("composio", "GITHUB_CREATE_A_RELEASE", json!({"tag_name": "v1"})));
        let mut refused = 0;
        for d in &rows {
            if let Some(why) = ships_or_runs(d) {
                refused += 1;
                assert!([PUSHES, RUNS, RISKY].contains(&why), "{why}");
                assert!(inline_refusal(d).is_some(), "{d}");
            }
        }
        assert!(refused >= 3, "the table has refused rows");
        // And the ordinary ones it keeps
        for d in [
            mcp("composio", "GMAIL_SEND_EMAIL", json!({"recipient_email": "a@b.c", "subject": "Our press release", "body": "The press release is attached."})),
            mcp("composio", "NOTION_SEARCH_NOTION_PAGE", json!({"query": "release notes"})),
        ] {
            assert_eq!(ships_or_runs(&d), None, "{d}");
        }
    }

    /// 20d: Rigel's #667 probe rows (finding 2), ordinary reads that the old part match refused
    /// with a wrong reason on both surfaces. tests/telegram_test.rs reads them back through Telegram.
    fn reads_rows() -> Vec<(&'static str, Value)> {
        vec![
            ("GITHUB_GET_A_RELEASE (RELEASE)", mcp("composio", "GITHUB_GET_A_RELEASE", json!({"id": 1}))),
            ("list_merge_requests (MERGE)", mcp("gitlab", "list_merge_requests", json!({}))),
            ("truncate_text (RUN)", mcp("db", "truncate_text", json!({"t": "x"}))),
            ("get_description (SCRIPT)", mcp("crm", "get_description", json!({"id": 1}))),
            ("url_encode (CODE)", mcp("util", "url_encode", json!({"u": "x"}))),
            ("geocode (CODE)", mcp("maps", "geocode", json!({"q": "Paris"}))),
            // Near misses of the same kind
            ("GITHUB_LIST_RELEASES", mcp("composio", "GITHUB_LIST_RELEASES", json!({"owner": "o", "repo": "r"}))),
            ("get_merge_request", mcp("gitlab", "get_merge_request", json!({"iid": 4}))),
            ("search_retrieval (EVAL)", mcp("docs", "search_retrieval", json!({"q": "x"}))),
            ("get_subscription (SCRIPT)", mcp("crm", "get_subscription", json!({"id": "s"}))),
            ("decode_barcode (CODE)", mcp("util", "decode_barcode", json!({"image": "a.png"}))),
            ("list_trunk_builds (RUN)", mcp("ci", "list_trunk_builds", json!({}))),
        ]
    }

    /// The older Telegram floor's line for a wrapper (`link_only_reason`), which the island shows
    /// first: any tool name with MULTI_EXECUTE, WORKBENCH, BASH or SHELL in it.
    const OTHER: &str = "This step runs other actions inside it.";

    /// 20d: what must still refuse after the switch to whole words, with its reason on each surface
    /// (island, Telegram's `ships_or_runs`).
    fn still_refused_rows() -> Vec<(&'static str, Value, &'static str, &'static str)> {
        vec![
            ("COMPOSIO_MULTI_EXECUTE_TOOL", mcp("composio", "COMPOSIO_MULTI_EXECUTE_TOOL", json!({})), OTHER, RUNS),
            ("run_command", mcp("devtools", "run_command", json!({})), RUNS, RUNS),
            ("execute_sql", mcp("db", "execute_sql", json!({})), RUNS, RUNS),
            ("evaluate_js", mcp("browser", "evaluate_js", json!({})), RUNS, RUNS),
            ("GITHUB_MERGE_A_PULL_REQUEST", mcp("composio", "GITHUB_MERGE_A_PULL_REQUEST", json!({})), PUSHES, PUSHES),
            ("GITHUB_CREATE_A_RELEASE", mcp("composio", "GITHUB_CREATE_A_RELEASE", json!({})), PUSHES, PUSHES),
            ("push_files", mcp("gh", "push_files", json!({})), PUSHES, PUSHES),
            ("GITHUB_UPDATE_A_PULL_REQUEST_BRANCH", mcp("composio", "GITHUB_UPDATE_A_PULL_REQUEST_BRANCH", json!({})), PUSHES, PUSHES),
            // A read verb doesn't lift a run name
            ("run_query", mcp("db", "run_query", json!({})), RUNS, RUNS),
            ("get_shell", mcp("x", "get_shell", json!({})), OTHER, RUNS),
            ("list_commands", mcp("x", "list_commands", json!({})), RUNS, RUNS),
            // Prefixes of whole words, camelCase humps, server names
            ("runner server", mcp("runner", "start", json!({})), RUNS, RUNS),
            ("shellExec", mcp("x", "shellExec", json!({})), OTHER, RUNS),
            ("commands / scripts", mcp("x", "save_scripts", json!({})), RUNS, RUNS),
            ("executeSql", mcp("x", "executeSql", json!({})), RUNS, RUNS),
            ("my-ssh server", mcp("my-ssh", "connect", json!({})), RUNS, RUNS),
            // One-word names the old part match caught and a prefix alone would miss
            ("mysql server", mcp("mysql", "query", json!({})), RUNS, RUNS),
            ("POSTGRESQL server", mcp("POSTGRESQL", "query", json!({})), RUNS, RUNS),
            ("powershell", mcp("win", "powershell", json!({})), OTHER, RUNS),
            ("javascript", mcp("x", "javascript", json!({})), RUNS, RUNS),
            ("vscode server", mcp("vscode", "open_file", json!({})), RUNS, RUNS),
            ("rerun_job", mcp("ci", "rerun_job", json!({})), RUNS, RUNS),
            // Push words: plurals, past tense, camelCase, references
            ("merges_pr", mcp("x", "merges_pr", json!({})), PUSHES, PUSHES),
            ("gitPushed", mcp("x", "gitPushed", json!({})), PUSHES, PUSHES),
            ("GITHUB_UPDATE_A_REFERENCE", mcp("composio", "GITHUB_UPDATE_A_REFERENCE", json!({})), PUSHES, PUSHES),
            ("GITHUB_CREATE_A_REFERENCE", mcp("composio", "GITHUB_CREATE_A_REFERENCE", json!({})), PUSHES, PUSHES),
            ("GITHUB_DELETE_A_REFERENCE", mcp("composio", "GITHUB_DELETE_A_REFERENCE", json!({})), PUSHES, PUSHES),
            ("update_refs", mcp("git", "update_refs", json!({})), PUSHES, PUSHES),
            ("updateBranch", mcp("x", "updateBranch", json!({})), PUSHES, PUSHES),
            ("publish_release", mcp("x", "publish_release", json!({})), PUSHES, PUSHES),
        ]
    }

    #[test]
    fn ordinary_reads_keep_allow_once_on_both_surfaces() {
        let rows = reads_rows();
        assert_eq!(rows.len(), 12);
        for (name, d) in rows {
            assert_eq!(ships_or_runs(&d), None, "telegram: {name}");
            assert_eq!(inline_refusal(&d), None, "island: {name}");
        }
    }

    #[test]
    fn real_pushes_and_runs_still_refuse_on_both_surfaces() {
        let rows = still_refused_rows();
        assert_eq!(rows.len(), 30);
        for (name, d, island, telegram) in rows {
            assert_eq!(inline_refusal(&d), Some(island), "island: {name}");
            assert_eq!(ships_or_runs(&d), Some(telegram), "telegram: {name}");
        }
    }

    /// The other island layers are unchanged (brief, "Don't touch"): a push word in a key or value
    /// still refuses a read tool there, and Telegram, which doesn't read them, keeps Allow once.
    #[test]
    fn a_push_word_in_the_arguments_still_refuses_a_read_on_the_island() {
        let d = mcp("composio", "GITHUB_GET_A_RELEASE", json!({"owner": "o", "repo": "r", "release_id": 1}));
        assert_eq!(inline_refusal(&d), Some(PUSHES));
        assert_eq!(ships_or_runs(&d), None);
        // And a read verb lifts only the name's push words, not an outward one
        assert_eq!(inline_refusal(&mcp("x", "get_deploy_logs", json!({}))), Some(OUTWARD));
    }

    #[test]
    fn avatars_are_the_saved_sprite_or_null() {
        let avatars: HashMap<String, String> = [("a".to_string(), "architect".to_string()), ("b".to_string(), "  ".to_string())].into();
        assert_eq!(avatar_of(&avatars, Some("a")), json!("architect"));
        assert_eq!(avatar_of(&avatars, Some("b")), Value::Null);
        assert_eq!(avatar_of(&avatars, Some("gone")), Value::Null);
        assert_eq!(avatar_of(&avatars, Some("")), Value::Null);
        assert_eq!(avatar_of(&avatars, None), Value::Null);
    }

    #[test]
    fn etags_match_weakly_and_ignore_now() {
        let a = json!({"counts": {"needs_you": 1}});
        let tag = etag_of(&a);
        assert!(tag.starts_with("W/\"g-"));
        assert_eq!(tag, etag_of(&a.clone()));
        assert_ne!(tag, etag_of(&json!({"counts": {"needs_you": 2}})));
        assert!(matches(Some(&tag), &tag));
        assert!(matches(Some(&tag.replace("W/", "")), &tag));
        assert!(matches(Some(&format!("\"other\", {tag}")), &tag));
        assert!(matches(Some("*"), &tag));
        assert!(!matches(Some("W/\"g-0\""), &tag));
        assert!(!matches(None, &tag));
    }

    #[test]
    fn paths_follow_the_nudge() {
        assert_eq!(app_path("s", &json!({"channel_id": "c 1"}), "k", Some("ap")), "/s/s/c/c%201?card=ap");
        assert_eq!(app_path("s", &json!({"thread_id": "t1"}), "k1", Some("ap")), "/s/s/b/k1/t/t1?card=ap");
        assert_eq!(app_path("s", &json!({"direct_kwant_id": "k2"}), "k1", None), "/s/s/b/k2");
        assert_eq!(app_path("s", &json!({}), "", None), "/s/s/inbox");
    }

    #[test]
    fn lines_are_short_one_line_and_masked() {
        assert_eq!(line("a\n  b", 10), "a b");
        assert!(!line("token sk-ant-api03-abcdefghijklmnop1234", 120).contains("sk-ant"));
        assert_eq!(line(&"x".repeat(200), 10).chars().count(), 10);
    }
}
