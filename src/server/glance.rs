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

/// Whole words (in the tool name, an argument key or a string value, see [`inline_refusal`] step 1) that push, merge, publish
/// or release, or move refs and tags.
const PUSH_FAMILY: &[&[&str]] = &[&["push"], &["merge"], &["publish"], &["release"], &["send", "pack"], &["receive", "pack"], &["refs"], &["deploy"], &["tag"]];

/// Parts of a tool name (any case) that act outside kwant or destroy something: publishing,
/// sending, deleting or paying. These are verbs: a read verb never lifts them, so `get_deploy_logs`
/// and `list_and_send_emails` still need the full card.
const OUTWARD_NAMES: &[&str] = &[
    "PUBLISH", "DEPLOY", "SEND", "DELETE", "REMOVE", "DESTROY",
    // 0.9.14 (Rigel's hole 3)
    "TRANSFER", "SUBMIT", "TWEET", "INVITE", "FORWARD", "WIPE", "TERMINATE", "REVOKE", "PURCHASE",
    // 18c (Rigel's B34)
    "BROADCAST",
];

/// Parts of a tool name (any case) that act outside kwant as a verb but are just as often the
/// thing a read looks at: mail, posts, payments, shares, uploads, drops. They refuse like
/// [`OUTWARD_NAMES`] unless the name only reads ([`reads_outward_noun`], 21a), so `GMAIL_FETCH_EMAILS`
/// and `get_payment_status` keep Allow once while `create_post` and `pay_invoice` don't.
const OUTWARD_NOUNS: &[&str] = &["EMAIL", "MAIL", "DROP", "POST", "PAY", "SHARE", "UPLOAD"];

/// Whole words of a tool name ([`words`]) that act outside kwant but sit inside ordinary words as
/// parts (`ship` in `membership`, `relationship`), so they can't join [`OUTWARD_NOUNS`]: `ship_it`,
/// `rollout` (18c, Rigel's B54). Lifted like [`OUTWARD_NOUNS`]: `ship_status`, `rollout_status`.
const OUTWARD_NAME_WORDS: &[&str] = &["ship", "rollout"];

/// Whole words that, with the [`READ_VERBS`], say a name with only [`OUTWARD_NOUNS`] in it reads
/// (21a). Local to the outward layer: these must not join [`READ_VERBS`], where they would lift
/// push and commit words too. 22h: `retrieve` (`STRIPE_RETRIEVE_PAYMENT_INTENT`, Rigel's F3) and
/// `check` (`check_payment_status`, which the state route no longer lifts on its own).
const OUTWARD_READ_WORDS: &[&str] = &["fetch", "retrieve", "check"];

/// Last words of a tool name that make it a read of something's state: `rollout_status`,
/// `ship_state` (21a). Only for payments, a ship or a rollout, and only in a short name
/// ([`reads_state`], 22h).
const OUTWARD_READ_LAST_WORDS: &[&str] = &["status", "state"];

/// The one part of [`OUTWARD_NOUNS`] whose state a name may read by ending in
/// [`OUTWARD_READ_LAST_WORDS`] without a read verb (22h), with the [`OUTWARD_NAME_WORDS`]:
/// `payment_status`, `rollout_status`, `ship_status`. A post, share, upload, mail or drop status is
/// as often a write (`post_status`, `share_status`), so those need a read verb.
const STATE_NOUNS: &[&str] = &["PAY"];

/// [`OUTWARD_NOUNS`] and [`OUTWARD_NAME_WORDS`] as whole lowercase words: the outward thing used
/// as a verb. One of these as a name's first word, ahead of its read word, acts: `post_list`,
/// `share_get` (22h). After a Composio app prefix the second word counts (`SLACK_SHARE_GET`), and
/// `DROPBOX_`, `GMAIL_`, `SHAREPOINT_` and `payment` aren't whole outward words, so they don't count.
const OUTWARD_VERB_WORDS: &[&str] = &["email", "mail", "drop", "post", "pay", "share", "upload", "ship", "rollout"];

/// Whole words that add a second action to a read name, so the read verb can't vouch for it:
/// `find_and_share_file`, `get_then_pay`, `list_or_create_posts` (via `create`), `fetch_and_reply` (21a).
/// 22h (Rigel's F1 on #692): `change_post_status`, `retry_payment_status`, `MAILCHIMP_SUBSCRIBE_TO_LIST`,
/// `join_mailing_list`, `MAILCHIMP_ARCHIVE_MEMBER_FROM_LIST`.
const SECOND_ACTION_WORDS: &[&str] = &[
    "and", "then", "create", "update", "add", "set", "put", "write", "edit", "modify", "reply", "save", "schedule", "upsert", "insert",
    // 22h
    "change", "toggle", "retry", "capture", "refund", "subscribe", "unsubscribe", "join", "leave", "archive",
];

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

/// Whole words of an argument key that make its value a git thing or a mode, so a push word in it
/// counts however long the value is (21b): `ref`, `branch`, `tag`/`tag_name`, `remote`,
/// `target_commitish`, `head`, `base`, `sha`, `commit`, and `mode`/`strategy` (`merge_mode`).
const GIT_KEY_WORDS: &[&str] = &["ref", "refs", "branch", "branches", "tag", "tags", "remote", "commitish", "head", "base", "sha", "commit", "mode", "strategy"];

/// Whole words ([`words`]) in a value that make a push word in it a git action, not prose (21b):
/// `push to origin main`, `gh release create`, `git merge --force`. `HEAD` (upper case only) and
/// `refs/` are checked in [`git_signal`].
const GIT_SIGNAL_WORDS: &[&str] = &["git", "gh", "origin", "upstream", "remote", "branch", "force"];

/// Whether a value carries a git signal: a [`GIT_SIGNAL_WORDS`] word, `refs/`, or `HEAD`.
fn git_signal(s: &str) -> bool {
    words(s).iter().any(|w| GIT_SIGNAL_WORDS.contains(&w.as_str())) || s.contains("refs/") || s.split(|c: char| !c.is_alphanumeric()).any(|t| t == "HEAD")
}

/// Whether a value under this key is an action, a mode, a command or a git thing, so any push word
/// in it counts (21b): a run key ([`is_run_key`]), an action or method key ([`is_action_key`],
/// [`is_method_key`]) or a [`GIT_KEY_WORDS`] key.
fn strict_key(k: &str) -> bool {
    let f = flat(k);
    is_run_key(k) || is_action_key(&f) || is_method_key(&f) || words(k).iter().any(|w| GIT_KEY_WORDS.contains(&w.as_str()))
}

/// Whether a push word in one string value counts (21b). Always under a [`strict_key`] or as an
/// item of a list (argv-style). Otherwise only when the value reads as an action or a ref, not a
/// sentence: one word (`merge`, `receive-pack`, `forcePush`); two words where the push word comes
/// first (`Push it`, `merge main`) or the other word has a digit (`v1 TAG`); or any length with a
/// [`git_signal`] (`push to origin main`). "Our press release", "press release" and "Release
/// notes for 0.9" don't count.
fn push_value_counts(s: &str, strict: bool) -> bool {
    if !has_push_word(s) {
        return false;
    }
    let toks: Vec<&str> = s.split_whitespace().collect();
    strict
        || toks.len() <= 1
        || git_signal(s)
        || (toks.len() == 2 && (has_push_word(toks[0]) || toks.iter().any(|t| t.chars().any(|c| c.is_ascii_digit()))))
}

/// Step 1 of [`inline_refusal`] on the arguments (21b): a push word in a key, at any depth, when the
/// tool name doesn't read (`keys_count`), so `release_id` on `GITHUB_GET_A_RELEASE` doesn't count
/// and `what_to_merge` on `save_note` does; or a push word in a string value that counts
/// ([`push_value_counts`]). Everything under a [`strict_key`] is strict, and a string that is an
/// item of a list is too.
fn push_word_in_args(v: &Value, keys_count: bool, strict: bool, in_list: bool) -> bool {
    match v {
        Value::String(s) => push_value_counts(s, strict || in_list),
        Value::Array(items) => items.iter().any(|i| push_word_in_args(i, keys_count, strict, true)),
        Value::Object(map) => {
            map.iter().any(|(k, v)| (keys_count && has_push_word(k)) || push_word_in_args(v, keys_count, strict || strict_key(k), false))
        }
        _ => false,
    }
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

/// Whether a tool name acts outside kwant or destroys something ([`inline_refusal`] step 3): a
/// part of it is one of [`OUTWARD_NAMES`], or it has an outward noun ([`OUTWARD_NOUNS`] as a part,
/// [`OUTWARD_NAME_WORDS`] as a whole word) and doesn't only read ([`reads_outward_noun`]).
fn outward_by_name(tool: &str) -> bool {
    let upper = tool.to_ascii_uppercase();
    if OUTWARD_NAMES.iter().any(|w| upper.contains(w)) {
        return true;
    }
    let name = words(tool);
    let noun = OUTWARD_NOUNS.iter().any(|w| upper.contains(w)) || name.iter().any(|w| OUTWARD_NAME_WORDS.contains(&w.as_str()));
    noun && !reads_outward_noun(tool, &name)
}

/// Whether a name whose only outward words are nouns only reads them (21a; tightened in 22h after
/// Rigel's F1 on #692). It has none of [`SECOND_ACTION_WORDS`], and either:
/// - a read word ([`READ_VERBS`], [`OUTWARD_READ_WORDS`]), and the name doesn't open with an
///   [`OUTWARD_VERB_WORDS`] word ahead of it (the second word after a Composio app prefix):
///   `GMAIL_LIST_THREADS`, `WORDPRESS_GET_POST`, `getPostStatus` read; `post_list`, `share_get` don't;
/// - or it reads a payment's, a ship's or a rollout's state ([`reads_state`]): `rollout_status`.
///
/// `GMAIL_REPLY_TO_THREAD`, `find_and_share_file`, `set_payment_status`, `post_status` and
/// `MAILCHIMP_SUBSCRIBE_TO_LIST` don't read.
fn reads_outward_noun(tool: &str, name: &[String]) -> bool {
    let is_read = |w: &String| READ_VERBS.contains(&w.as_str()) || OUTWARD_READ_WORDS.contains(&w.as_str());
    // The verb's place: the first word, or the second after a Composio app prefix
    let verb_at = usize::from(app_prefixed(tool));
    let read_first = name.iter().position(is_read).is_some_and(|r| r <= verb_at || !OUTWARD_VERB_WORDS.contains(&name[verb_at].as_str()));
    (read_first || reads_state(tool, name)) && !name.iter().any(|w| SECOND_ACTION_WORDS.contains(&w.as_str()))
}

/// Whether a name with no read verb reads a payment's, a ship's or a rollout's state (22h): it ends
/// in [`OUTWARD_READ_LAST_WORDS`], its only [`OUTWARD_NOUNS`] part is a [`STATE_NOUNS`] one, and every
/// word before the last is a payment, ship or rollout word, apart from the app prefix of a
/// Composio-style name ([`app_prefixed`]). `payment_status`, `rollout_status`, `ship_state` and
/// `STRIPE_PAYMENT_STATUS` read; `post_status`, `MASTODON_POST_STATUS`, `cancel_payment_status` and
/// `post_payment_status` don't.
fn reads_state(tool: &str, name: &[String]) -> bool {
    let Some((last, before)) = name.split_last() else { return false };
    if !OUTWARD_READ_LAST_WORDS.contains(&last.as_str()) {
        return false;
    }
    let upper = tool.to_ascii_uppercase();
    if OUTWARD_NOUNS.iter().any(|n| upper.contains(n) && !STATE_NOUNS.contains(n)) {
        return false;
    }
    let skip = usize::from(app_prefixed(tool));
    let state_word = |w: &String| OUTWARD_NAME_WORDS.contains(&w.as_str()) || STATE_NOUNS.iter().any(|n| w.to_ascii_uppercase().contains(n));
    before.len() > skip && before.iter().skip(skip).all(state_word)
}

/// Whether a tool name looks like a Composio one, `APP_ACTION_THING`: no lower-case letter, an
/// underscore, and three words or more, so its first word names the app (`STRIPE_PAYMENT_STATUS`).
fn app_prefixed(tool: &str) -> bool {
    !tool.chars().any(char::is_lowercase) && tool.contains('_') && words(tool).len() >= 3
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
/// 1. a push-family word ([`PUSH_FAMILY`]) in the tool name or an argument key (unless a
///    [`READ_VERBS`] word says the name only reads), or in a string value that is an action, a mode,
///    a command or a ref rather than a sentence ([`push_word_in_args`], 21b), as a whole word,
///    camelCase humps included ([`words`]);
/// 2. a tool that runs free text: its tool or server name says so ([`runs_by_name`]) or an
///    argument key at any depth does ([`is_run_key`]: exact, by containment, or by word);
/// 3. a tool whose name acts outside kwant or destroys something ([`outward_by_name`]: a verb of
///    [`OUTWARD_NAMES`] anywhere, or an outward noun in a name that doesn't only read it);
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
    // 1. A push-family word wherever it hides: the name (unless it only reads, 20d), a key (unless
    //    the name only reads, 21b), a value that is an action, a mode or a ref rather than a
    //    sentence (21b), an argv item
    let name_reads = reads(&words(tool));
    if push_word_in_args(args, !name_reads, !args.is_object(), false) || (has_push_word(tool) && !name_reads) {
        return Some(PUSHES);
    }
    // 2. Free text to run, whatever it says
    if runs_by_name(server, tool) || has_run_key(args) {
        return Some(RUNS);
    }
    // 3. Acts outside kwant or destroys something (an outward noun in a name that only reads it
    //    doesn't count, 21a)
    if outward_by_name(tool) {
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

    /// Step 1 as it was before 21b: a push word anywhere in the joined keys and values, or in a name
    /// that doesn't read.
    fn step1_before_21b(d: &Value) -> bool {
        let tool = d["tool"].as_str().unwrap_or_default();
        let mut all = Vec::new();
        texts(&d["arguments"], &mut all);
        has_push_word(&all.join(" ")) || (has_push_word(tool) && !reads(&words(tool)))
    }

    /// 21b: a push word in a sentence, or in a key of a tool that only reads, doesn't say "This pushes
    /// code". Each row: before (old step 1 refused?), the island's answer after.
    fn prose_rows() -> Vec<(&'static str, Value, bool, Option<&'static str>)> {
        let note = |args: Value| mcp("notes", "save_note", args);
        vec![
            // Allow once now (each said PUSHES before)
            ("GITHUB_GET_A_RELEASE release_id (a key, the name reads)", mcp("composio", "GITHUB_GET_A_RELEASE", json!({"owner": "o", "repo": "r", "release_id": 1})), true, None),
            ("GITHUB_GET_A_RELEASE_BY_TAG_NAME tag v1.0", mcp("composio", "GITHUB_GET_A_RELEASE_BY_TAG_NAME", json!({"owner": "o", "repo": "r", "tag": "v1.0"})), true, None),
            ("K07 (Rigel) gmail create_draft \"press release\"", mcp("gmail", "create_draft", json!({"subject": "press release", "body": "hi"})), true, None),
            ("K07 (brief) notion create_page \"Our press release\"", mcp("notion", "create_page", json!({"title": "Our press release", "content": "The press release is attached."})), true, None),
            ("notion create_page \"Release notes for 0.9\"", mcp("notion", "create_page", json!({"title": "Release notes for 0.9", "parent": {"page_id": "abc"}})), true, None),
            ("save_note \"We will merge the two lists\"", note(json!({"text": "We will merge the two lists"})), true, None),
            ("linear create_issue, launch post title", mcp("linear", "create_issue", json!({"title": "Publish the launch blog post on Monday", "description": "Draft is in the doc"})), true, None),
            // Allow once before and after
            ("save_note \"we merged the two lists\" (merged isn't a push word)", note(json!({"text": "we merged the two lists"})), false, None),
            // Still refused, another layer now answers (step 3, out of scope)
            ("K07 (brief) GMAIL_CREATE_EMAIL_DRAFT \"Our press release\"", mcp("composio", "GMAIL_CREATE_EMAIL_DRAFT", json!({"recipient_email": "a@b.c", "subject": "Our press release", "body": "The press release is attached."})), true, Some(OUTWARD)),
            // Still PUSHES
            ("message \"push to origin main\" (git signal)", note(json!({"message": "push to origin main"})), true, Some(PUSHES)),
            ("branch \"release\" (git key)", note(json!({"branch": "release"})), true, Some(PUSHES)),
            ("action \"merge\" (action key)", note(json!({"action": "merge"})), true, Some(PUSHES)),
            ("cmd \"deploy the site to prod now\" (run key)", note(json!({"cmd": "deploy the site to prod now"})), true, Some(PUSHES)),
            ("strategy \"squash merge\" (mode key)", note(json!({"strategy": "squash merge"})), true, Some(PUSHES)),
            ("opts.mode, a sentence under a mode key", note(json!({"opts": {"mode": "force push it right now"}})), true, Some(PUSHES)),
            ("\"force push\" (git signal)", note(json!({"t": "force push"})), true, Some(PUSHES)),
            ("\"git merge feature into main please\"", note(json!({"t": "git merge feature into main please"})), true, Some(PUSHES)),
            ("\"move the refs/heads/main pointer\" (refs/)", note(json!({"t": "move the refs/heads/main pointer"})), true, Some(PUSHES)),
            ("\"reset HEAD and then push it\" (HEAD)", note(json!({"t": "reset HEAD and then push it"})), true, Some(PUSHES)),
            ("a sentence in a list (argv-style)", note(json!({"items": ["deploy the site now"]})), true, Some(PUSHES)),
            ("\"Release 2.0\" (two words, push word first)", note(json!({"t": "Release 2.0"})), true, Some(PUSHES)),
            ("\"merge main\" (two words, push word first)", note(json!({"t": "merge main"})), true, Some(PUSHES)),
            ("a read with tag \"release\" (a git key's value still counts)", mcp("composio", "GITHUB_GET_A_RELEASE_BY_TAG_NAME", json!({"tag": "release"})), true, Some(PUSHES)),
            ("key what_to_merge on a name that doesn't read", note(json!({"what_to_merge": "a"})), true, Some(PUSHES)),
            ("a bare string argument", note(json!("publish the site now")), true, Some(PUSHES)),
        ]
    }

    #[test]
    fn a_push_word_in_prose_or_a_read_s_key_keeps_allow_once_and_actions_do_not() {
        let rows = prose_rows();
        assert_eq!(rows.len(), 25);
        for (name, d, before, want) in rows {
            assert_eq!(step1_before_21b(&d), before, "before 21b: {name}");
            assert_eq!(inline_refusal(&d), want, "island: {name}");
        }
        // A read verb lifts only the name's and keys' push words, not an outward one
        assert_eq!(inline_refusal(&mcp("x", "get_deploy_logs", json!({}))), Some(OUTWARD));
        // Telegram is unchanged: it never read push words in arguments
        let d = mcp("composio", "GITHUB_GET_A_RELEASE", json!({"owner": "o", "repo": "r", "release_id": 1}));
        assert_eq!(ships_or_runs(&d), None);
    }

    /// Every row the island refused at step 1 before 21b still needs the full card after it, unless
    /// it is one of the [`prose_rows`] the brief lifts on purpose.
    #[test]
    fn only_prose_and_read_keys_are_lifted() {
        let mut rows: Vec<Value> = risky_rows().into_iter().map(|(_, d)| d).collect();
        rows.extend(hardening_rows().into_iter().map(|(_, d, _)| d));
        rows.extend(reads_rows().into_iter().map(|(_, d)| d));
        rows.extend(outward_rows().into_iter().map(|(_, d, _)| d));
        let mut checked = 0;
        for d in &rows {
            if step1_before_21b(d) {
                checked += 1;
                assert!(inline_refusal(d).is_some(), "{d}");
            }
        }
        assert!(checked >= 8, "{checked}");
    }

    /// 21a: the island's outward layer, both directions. A read of mail, files, posts, payments,
    /// uploads or a rollout keeps Allow once; a name that acts on them, or reads and then acts, doesn't.
    fn outward_rows() -> Vec<(&'static str, Value, Option<&'static str>)> {
        vec![
            // Reads: Allow once (each was OUTWARD before 21a)
            ("GMAIL_FETCH_EMAILS (MAIL, EMAIL)", mcp("composio", "GMAIL_FETCH_EMAILS", json!({"max_results": 10})), None),
            ("GMAIL_LIST_THREADS (MAIL)", mcp("composio", "GMAIL_LIST_THREADS", json!({})), None),
            ("DROPBOX_LIST_FILES (DROP)", mcp("composio", "DROPBOX_LIST_FILES", json!({"path": "/"})), None),
            ("SHAREPOINT_GET_FILE (SHARE)", mcp("composio", "SHAREPOINT_GET_FILE", json!({"file_id": "f"})), None),
            ("WORDPRESS_GET_POST (POST)", mcp("composio", "WORDPRESS_GET_POST", json!({"post_id": 7})), None),
            ("get_payment_status (PAY)", mcp("stripe", "get_payment_status", json!({"id": "pi_1"})), None),
            ("list_uploads (UPLOAD)", mcp("files", "list_uploads", json!({})), None),
            ("rollout_status (rollout)", mcp("ci", "rollout_status", json!({})), None),
            ("ship_status (ship)", mcp("ci", "ship_status", json!({})), None),
            ("getPaymentState (camelCase)", mcp("x", "getPaymentState", json!({})), None),
            // Acts on them, or reads and then acts: still OUTWARD
            ("GMAIL_SEND_EMAIL", mcp("composio", "GMAIL_SEND_EMAIL", json!({})), Some(OUTWARD)),
            ("GMAIL_REPLY_TO_THREAD", mcp("composio", "GMAIL_REPLY_TO_THREAD", json!({})), Some(OUTWARD)),
            ("create_post", mcp("x", "create_post", json!({})), Some(OUTWARD)),
            ("SLACK_CHAT_POST_MESSAGE", mcp("composio", "SLACK_CHAT_POST_MESSAGE", json!({})), Some(OUTWARD)),
            ("pay_invoice", mcp("x", "pay_invoice", json!({})), Some(OUTWARD)),
            ("share_page", mcp("x", "share_page", json!({})), Some(OUTWARD)),
            ("find_and_share_file", mcp("x", "find_and_share_file", json!({})), Some(OUTWARD)),
            ("get_and_delete_post", mcp("x", "get_and_delete_post", json!({})), Some(OUTWARD)),
            ("list_and_send_emails", mcp("x", "list_and_send_emails", json!({})), Some(OUTWARD)),
            ("ship_it", mcp("x", "ship_it", json!({})), Some(OUTWARD)),
            ("DROPBOX_DELETE_FILE", mcp("composio", "DROPBOX_DELETE_FILE", json!({})), Some(OUTWARD)),
            ("upload_file", mcp("x", "upload_file", json!({})), Some(OUTWARD)),
            ("get_post sending a POST body (step 4)", mcp("x", "get_post", json!({"url": "https://api.example.com/posts", "method": "POST", "body": {"t": "x"}})), Some(OUTWARD)),
            // Near misses of the lift
            ("set_payment_status (set)", mcp("x", "set_payment_status", json!({})), Some(OUTWARD)),
            ("fetch_then_pay (then)", mcp("x", "fetch_then_pay", json!({})), Some(OUTWARD)),
            ("get_or_create_post (create)", mcp("x", "get_or_create_post", json!({})), Some(OUTWARD)),
            ("GMAIL_FETCH_AND_REPLY (and, reply)", mcp("composio", "GMAIL_FETCH_AND_REPLY", json!({})), Some(OUTWARD)),
            ("schedule_post_status (schedule)", mcp("x", "schedule_post_status", json!({})), Some(OUTWARD)),
            ("get_deploy_logs (a verb isn't lifted)", mcp("x", "get_deploy_logs", json!({})), Some(OUTWARD)),
            ("list_mail_to_forward (a verb isn't lifted)", mcp("x", "list_mail_to_forward", json!({})), Some(OUTWARD)),
            ("GMAILFETCHEMAILS (one word, no read verb)", mcp("x", "GMAILFETCHEMAILS", json!({})), Some(OUTWARD)),
            ("status_of_payment (status not last)", mcp("x", "status_of_payment", json!({})), Some(OUTWARD)),
        ]
    }

    /// Step 3 as it was before 21a: any outward part, or a whole `ship`/`rollout` word.
    fn outward_before_21a(tool: &str) -> bool {
        let upper = tool.to_ascii_uppercase();
        OUTWARD_NAMES.iter().chain(OUTWARD_NOUNS.iter()).any(|w| upper.contains(w)) || words(tool).iter().any(|w| OUTWARD_NAME_WORDS.contains(&w.as_str()))
    }

    #[test]
    fn reads_of_outward_things_keep_allow_once_and_acts_on_them_do_not() {
        let rows = outward_rows();
        assert_eq!(rows.len(), 32);
        for (name, d, want) in rows {
            let tool = d["tool"].as_str().unwrap();
            assert!(outward_before_21a(tool), "{name}: step 3 refused it before 21a");
            assert_eq!(inline_refusal(&d), want, "island: {name}");
            // Telegram's floor doesn't read outward names, before or after
            assert_eq!(ships_or_runs(&d), None, "telegram: {name}");
        }
    }

    #[test]
    fn the_lift_only_touches_outward_nouns() {
        // Steps 1 and 2 answer first, whatever the name reads
        assert_eq!(inline_refusal(&mcp("composio", "GMAIL_FETCH_EMAILS", json!({"query": "label:x"}))), Some(RUNS), "a run key");
        assert_eq!(inline_refusal(&mcp("x", "get_post", json!({"note": "merge it"}))), Some(PUSHES), "a push word in a value");
        // fetch lifts outward nouns only; it doesn't join READ_VERBS for push and commit names
        assert!(!READ_VERBS.contains(&"fetch"));
        assert_eq!(inline_refusal(&mcp("x", "fetch_and_merge", json!({}))), Some(PUSHES));
        assert_eq!(inline_refusal(&mcp("x", "fetch_commit", json!({}))), Some(PUSHES));
    }

    /// Step 3's lift as 21a shipped it (#692 06b3406a, unchanged by 21b): a read verb, `fetch`, or a
    /// last word `status`/`state`, with none of 21a's second-action words.
    fn outward_at_21a(tool: &str) -> bool {
        const SECOND_21A: &[&str] = &["and", "then", "create", "update", "add", "set", "put", "write", "edit", "modify", "reply", "save", "schedule", "upsert", "insert"];
        let upper = tool.to_ascii_uppercase();
        if OUTWARD_NAMES.iter().any(|w| upper.contains(w)) {
            return true;
        }
        let name = words(tool);
        let noun = OUTWARD_NOUNS.iter().any(|w| upper.contains(w)) || name.iter().any(|w| OUTWARD_NAME_WORDS.contains(&w.as_str()));
        let read = reads(&name) || name.iter().any(|w| w == "fetch") || name.last().is_some_and(|w| w == "status" || w == "state");
        noun && !(read && !name.iter().any(|w| SECOND_21A.contains(&w.as_str())))
    }

    /// 22h (Rigel's F1, F3 and F5 on #692): (tool, step 3 at 21a refuses, island now).
    fn writes_named_like_reads_rows() -> Vec<(&'static str, bool, Option<&'static str>)> {
        vec![
            // F1, must stay refused: (a) a post, share or upload status is as often a write
            ("post_status", false, Some(OUTWARD)),
            ("MASTODON_POST_STATUS", false, Some(OUTWARD)),
            ("TWITTER_POST_STATUS", false, Some(OUTWARD)),
            ("facebook_post_status", false, Some(OUTWARD)),
            ("postStatus", false, Some(OUTWARD)),
            ("share_status", false, Some(OUTWARD)),
            ("shareStatus", false, Some(OUTWARD)),
            ("upload_status", false, Some(OUTWARD)),
            ("post_payment_status", false, Some(OUTWARD)),
            // F1 (a): a verb before a payment's state, with or without a 22h second-action word
            ("cancel_payment_status", false, Some(OUTWARD)),
            ("approve_payment_status", false, Some(OUTWARD)),
            // F1 (c): the new second-action words
            ("retry_payment_status", false, Some(OUTWARD)),
            ("capture_payment_status", false, Some(OUTWARD)),
            ("refund_payment_status", false, Some(OUTWARD)),
            ("change_payment_status", false, Some(OUTWARD)),
            ("change_post_status", false, Some(OUTWARD)),
            ("WORDPRESS_CHANGE_POST_STATUS", false, Some(OUTWARD)),
            ("toggle_post_status", false, Some(OUTWARD)),
            ("RETRY_PAYMENT_STATUS", false, Some(OUTWARD)),
            ("MAILCHIMP_SUBSCRIBE_TO_LIST", false, Some(OUTWARD)),
            ("MAILCHIMP_SUBSCRIBE_MEMBER_TO_LIST", false, Some(OUTWARD)),
            ("MAILCHIMP_UNSUBSCRIBE_FROM_LIST", false, Some(OUTWARD)),
            ("MAILCHIMP_ARCHIVE_MEMBER_FROM_LIST", false, Some(OUTWARD)),
            ("MAILCHIMP_BATCH_SUBSCRIBE_LIST", false, Some(OUTWARD)),
            ("GMAIL_UNSUBSCRIBE_FROM_MAILING_LIST", false, Some(OUTWARD)),
            ("subscribe_to_mailing_list", false, Some(OUTWARD)),
            ("join_mailing_list", false, Some(OUTWARD)),
            ("leave_mailing_list", false, Some(OUTWARD)),
            // F1 (b): the outward word is the verb, the read word comes after it
            ("post_list", false, Some(OUTWARD)),
            ("postList", false, Some(OUTWARD)),
            ("share_list", false, Some(OUTWARD)),
            ("share_get", false, Some(OUTWARD)),
            ("post_get", false, Some(OUTWARD)),
            ("post_show", false, Some(OUTWARD)),
            ("upload_get", false, Some(OUTWARD)),
            ("pay_list", false, Some(OUTWARD)),
            ("SLACK_SHARE_GET", false, Some(OUTWARD)),
            // Must allow: the brief's rows
            ("get_payment_status", false, None),
            ("rollout_status", false, None),
            ("ship_status", false, None),
            ("GMAIL_FETCH_EMAILS", false, None),
            ("DROPBOX_LIST_FILES", false, None),
            ("STRIPE_RETRIEVE_PAYMENT_INTENT", true, None),
            ("STRIPE_RETRIEVE_A_PAYMENT_INTENT", true, None),
            // Must allow: reads 21a allowed that sit next to the new rules
            ("ship_state", false, None),
            ("payment_status", false, None),
            ("STRIPE_PAYMENT_STATUS", false, None),
            ("check_payment_status", false, None),
            ("STRIPE_CHECK_PAYMENT_STATUS", false, None),
            ("getPostStatus", false, None),
            ("get_post_status", false, None),
            ("SHAREPOINT_GET_FILE", false, None),
            ("MAILCHIMP_GET_LIST", false, None),
            ("GMAIL_LIST_THREADS", false, None),
            ("list_uploads", false, None),
            // F2 is left as it is: marking mail read keeps Allow once
            ("GMAIL_MARK_EMAIL_AS_READ", false, None),
            ("mark_email_as_read", false, None),
            // F3: check reads too now
            ("check_email", true, None),
        ]
    }

    #[test]
    fn writes_named_like_reads_keep_the_full_card_and_reads_keep_allow_once() {
        let rows = writes_named_like_reads_rows();
        assert_eq!(rows.len(), 58);
        for (tool, refused_at_21a, want) in rows {
            assert!(outward_before_21a(tool), "{tool}: step 3 refused it before 21a");
            assert_eq!(outward_at_21a(tool), refused_at_21a, "{tool}: at 21a");
            let d = mcp("composio", tool, json!({}));
            assert_eq!(inline_refusal(&d), want, "island: {tool}");
            assert_eq!(ships_or_runs(&d), None, "telegram: {tool}");
        }
        // 21a's own rows keep their verdicts, and 21a's helper agrees with them
        for (name, d, want) in outward_rows() {
            let tool = d["tool"].as_str().unwrap();
            if d["arguments"].as_object().is_some_and(|a| a.is_empty()) || want.is_none() {
                assert_eq!(outward_at_21a(tool), want == Some(OUTWARD), "{name}: at 21a");
            }
        }
    }

    #[test]
    fn retrieve_and_check_lift_outward_nouns_only() {
        assert!(!READ_VERBS.contains(&"retrieve") && !READ_VERBS.contains(&"check"));
        assert_eq!(inline_refusal(&mcp("x", "retrieve_and_merge", json!({}))), Some(PUSHES));
        assert_eq!(inline_refusal(&mcp("x", "check_commit", json!({}))), Some(PUSHES));
        // A Composio prefix moves the verb's place to the second word only in an APP_ACTION name
        assert!(app_prefixed("SLACK_SHARE_GET") && app_prefixed("STRIPE_PAYMENT_STATUS"));
        assert!(!app_prefixed("share_get_x") && !app_prefixed("POST_LIST") && !app_prefixed("GMAILFETCHEMAILS"));
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
