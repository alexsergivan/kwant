//! `kwnt run-check`: reads back what bots ran and points out what the approvals log should
//! never show. Rule 1, "ran with no card": a tool step that should have gone through kwnt's
//! approval gate, with no approvals row for it. Rule 2, "asked again after a no": a card (or a
//! call that ran without one) for a bot in a chat whose kind of call ([`families`]) the person
//! declined there before, with no message from the person in between. That is the rule 27b's
//! gate keeps; this shows whether it held.
//!
//! Read only: the database is opened read-only, nothing is written, and there are no model
//! calls and no network. [`check`] is pure: the loader ([`load`]) and the printing
//! ([`render_text`], [`render_json`]) sit around it.
//!
//! How a step is judged:
//! - kwnt's own tools (`mcp__kwnt_tools__*`) run without a card by design;
//! - claude's look-ups (web search, to-do list, tool search) and file tools run without a card by
//!   design (file tools only inside the bot's folder; the step keeps no path to check that). A
//!   WebFetch of this computer or the local network is the exception: kwnt asks for it, so it
//!   needs a row (judged from the saved address, with no name lookups);
//! - another engine's own tools (codex, grok) have no card hook: their sandbox holds them;
//! - everything else (Bash, MCP tools, any tool on an API engine) needs an approvals row in the
//!   step's time window, for the same bot, tool and chat. A read-only command claude didn't ask
//!   about, a late Allow used by the resumed turn (within the grant's 30 minutes) and a tool the
//!   person chose "Always allow" for earlier in the same chat don't count as findings.
//!
//! [`families`]: crate::mcp::declined::families

use crate::drivers::activity::redact;
use crate::mcp::builtin_tools::{self, ToolGroup};
use crate::mcp::{declined, BuiltinCall, ToolApprovalRequest};
use chrono::{DateTime, Duration, NaiveDateTime, Utc};
use serde::Serialize;
use serde_json::Value;
use std::collections::HashMap;
use std::path::Path;

/// Longest step title in a finding (characters), after redaction.
pub const MAX_TITLE: usize = 120;
/// Approvals rows keep whole seconds; steps keep milliseconds.
const SLACK_SECS: i64 = 2;
/// A late Allow lets the resumed turn run the call once, within the broker's grant lifetime.
const GRANT_MINUTES: i64 = 30;

/// One tool, approval or other step of a reply, as saved in `meta.activity.steps`.
#[derive(Debug, Clone, Default)]
pub struct Step {
    pub kind: String,
    pub status: String,
    pub tool: Option<String>,
    pub title: String,
    pub detail: Option<String>,
    pub ref_id: Option<String>,
    pub started_at: Option<DateTime<Utc>>,
    pub ended_at: Option<DateTime<Utc>>,
}

/// One bot reply with the steps it ran.
#[derive(Debug, Clone)]
pub struct Reply {
    pub id: String,
    pub at: DateTime<Utc>,
    pub bot_id: String,
    pub bot: String,
    /// `thread:<id>`, `channel:<id>` or `direct:<bot id>`.
    pub chat: Option<String>,
    /// How the chat reads in the output ("chat with Forge", "#launch").
    pub chat_label: String,
    /// `meta.ran_on.engine`: "claude", "codex", "grok", "anthropic_api", …
    pub engine: Option<String>,
    pub steps: Vec<Step>,
}

/// One message the person wrote.
#[derive(Debug, Clone)]
pub struct PersonMessage {
    pub chat: String,
    pub at: DateTime<Utc>,
}

/// One approvals row.
#[derive(Debug, Clone)]
pub struct Approval {
    pub id: String,
    pub turn_id: Option<String>,
    pub bot_id: String,
    pub tool: String,
    pub parameters: Value,
    pub status: String,
    pub at: DateTime<Utc>,
    /// From `resume`; rows saved with no `resume` take it from a row of the same turn.
    pub chat: Option<String>,
    /// How the chat reads in the output; filled like `chat`.
    pub chat_label: Option<String>,
    /// The bot's name.
    pub bot: String,
    /// A card the person was shown: saved with a `resume` chat. Rows kwnt closed itself (ran
    /// without asking, refused, a customer's refused call) have none.
    pub card: bool,
}

impl Approval {
    /// The person said no on this card (not a call kwnt refused on its own).
    fn persons_no(&self) -> bool {
        self.card && self.status == "denied" && self.parameters.get("declined_before").is_none()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Rule {
    RanWithNoCard,
    AskedAgainAfterNo,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Finding {
    pub rule: Rule,
    pub at: DateTime<Utc>,
    pub bot: String,
    pub bot_id: String,
    pub chat: String,
    pub tool: String,
    /// Redacted, at most [`MAX_TITLE`] characters. Rule 2: the call's command, address or path.
    pub title: String,
    /// Rule 1: the reply the step is in.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub reply_id: String,
    /// Rule 2: the approvals row asked again, and the earlier card the person said no on.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub approval_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub declined_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub declined_at: Option<DateTime<Utc>>,
}

/// Steps that ran without a card on purpose, by reason.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct ByDesign {
    /// claude's file tools, free inside the bot's own folder.
    pub folder_files: usize,
    /// Read-only commands claude runs without asking.
    pub read_only_commands: usize,
    /// kwnt's own tools.
    pub kwant_tools: usize,
    /// Web search, to-do list, tool search and the like.
    pub lookups: usize,
    /// A tool the person chose "Always allow" for earlier in the same chat.
    pub always_allowed: usize,
    /// codex's and grok's own tools: no card hook, their sandbox holds them.
    pub other_engines: usize,
}

impl ByDesign {
    pub fn total(&self) -> usize {
        self.folder_files + self.read_only_commands + self.kwant_tools + self.lookups + self.always_allowed + self.other_engines
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Report {
    /// The earliest reply looked at, or `None` for every reply.
    pub since: Option<DateTime<Utc>>,
    pub replies: usize,
    /// Tool steps that ran (status ok or failed).
    pub steps: usize,
    /// Of those, the ones an approvals row covers.
    pub with_card: usize,
    /// Cards the person said no on in the window (rule 2 looks after each).
    pub nos: usize,
    pub by_design: ByDesign,
    pub findings: Vec<Finding>,
}

/// Rule 1 over already-loaded rows: the findings only.
pub fn findings(replies: &[Reply], person_messages: &[PersonMessage], approvals: &[Approval]) -> Vec<Finding> {
    check(replies, person_messages, approvals).findings
}

/// One key per tool across engines: `mcp__github__create_issue` (claude), `mcp__kwnt__github__
/// create_issue` (codex, through kwnt's gateway), `github-create_issue` (grok) and
/// `github__create_issue` (API engines) all read `github_create_issue`.
pub fn tool_key(tool: &str) -> String {
    let tool = tool.strip_prefix("mcp__kwnt__").unwrap_or(tool);
    let mut key = String::with_capacity(tool.len());
    for c in tool.chars() {
        let c = if c.is_ascii_alphanumeric() { c.to_ascii_lowercase() } else { '_' };
        if !(c == '_' && key.ends_with('_')) {
            key.push(c);
        }
    }
    let key = key.trim_matches('_');
    key.strip_prefix("mcp_").unwrap_or(key).to_string()
}

/// Why a step ran without a card, or that it needed one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Gate {
    Card,
    FolderFile,
    KwantTool,
    Lookup,
    OtherEngine,
}

fn gate(engine: Option<&str>, tool: &str, key: &str, step: &Step) -> Gate {
    if key.starts_with("kwnt_tools_") {
        return Gate::KwantTool;
    }
    // API engines: kwnt runs every tool itself, through the gate
    if engine.is_some_and(|e| e.ends_with("_api") || e == "ollama") || tool.starts_with("mcp__") {
        return Gate::Card;
    }
    match engine {
        None | Some("claude") => match builtin_tools::group(tool) {
            // kwnt asks before a WebFetch of this computer or the local network (`fetches_kwnt`)
            Some(ToolGroup::Lookup) if tool == "WebFetch" && fetch_address(step).is_some_and(|url| fetch_asks(&url)) => Gate::Card,
            Some(ToolGroup::Lookup) => Gate::Lookup,
            Some(ToolGroup::Read | ToolGroup::Write) => Gate::FolderFile,
            Some(ToolGroup::Shell) | None => Gate::Card,
        },
        // grok names MCP tools `<server>-<tool>`; its own tools have no dash
        Some("grok") if tool.contains('-') => Gate::Card,
        Some(_) => Gate::OtherEngine,
    }
}

/// The address a WebFetch step opened: its `detail` (the URL as the call gave it, redacted and cut),
/// else the host in its title ("Opening 10.0.0.5"), which has lost the port and path. `None` when
/// the step keeps neither ("Opening a page"). A failed step's `detail` is usually the failure reason
/// ("Request failed with status code 404"; drivers/activity.rs `step_end`), so there it counts only
/// while it still reads as a URL.
fn fetch_address(step: &Step) -> Option<String> {
    let still_url = |d: &&str| step.status != "failed" || (d.contains("://") && !d.contains(char::is_whitespace));
    if let Some(url) = step.detail.as_deref().map(str::trim).filter(|d| !d.is_empty()).filter(still_url) {
        return Some(url.to_string());
    }
    let host = step.title.strip_prefix("Opening ")?.trim();
    (!host.is_empty() && host != "a page" && !host.contains(char::is_whitespace)).then(|| format!("http://{host}/"))
}

/// Whether kwnt's WebFetch rule ([`fetch_risk_with`]) asks about `url`, judged offline: no name is
/// ever looked up. A name ending in `.local`, `.internal` or `.home.arpa` is taken as the local
/// network, and every other name as public, so a public name that only resolves to a local address
/// is missed here (on purpose: run-check makes no network calls).
///
/// [`fetch_risk_with`]: crate::mcp::fetch_risk::fetch_risk_with
fn fetch_asks(url: &str) -> bool {
    crate::mcp::fetch_risk::fetch_risk_with(url, &offline_resolve).is_some()
}

/// A stand-in for the name lookup that never touches the network.
fn offline_resolve(name: &str, _port: u16) -> Option<Vec<std::net::IpAddr>> {
    let local = [".local", ".internal", ".home.arpa"].iter().any(|end| name.ends_with(end) || name == &end[1..]);
    // 192.168.0.1 is private; 1.1.1.1 is public. Only the kind of address matters.
    let ip = if local { std::net::Ipv4Addr::new(192, 168, 0, 1) } else { std::net::Ipv4Addr::new(1, 1, 1, 1) };
    Some(vec![ip.into()])
}

/// A step that needs a card, and what covers it.
struct Item {
    reply: usize,
    step: usize,
    key: String,
    start: DateTime<Utc>,
    end: DateTime<Utc>,
    command: Option<String>,
    read_only: bool,
    covered: bool,
}

fn chats_meet(a: Option<&str>, b: Option<&str>) -> bool {
    match (a, b) {
        (Some(a), Some(b)) => a == b,
        _ => true,
    }
}

fn row_command(row: &Approval) -> Option<&str> {
    row.parameters.pointer("/arguments/command").and_then(Value::as_str)
}

/// Both rules over already-loaded rows, with every approvals row looked at. `replies` are the
/// ones to judge; `approvals` should hold every row (earlier "Always allow" choices, late Allows
/// and earlier nos count).
pub fn check(replies: &[Reply], person_messages: &[PersonMessage], approvals: &[Approval]) -> Report {
    check_since(replies, person_messages, approvals, None, None)
}

/// [`check`], with rule 2 judging only the rows at or after `since`, and of bot `bot_id`.
pub fn check_since(
    replies: &[Reply],
    person_messages: &[PersonMessage],
    approvals: &[Approval],
    since: Option<DateTime<Utc>>,
    bot_id: Option<&str>,
) -> Report {
    let mut rows = approvals.to_vec();
    fill_chats(&mut rows);
    let mut used = vec![false; rows.len()];
    let mut by_design = ByDesign::default();
    let mut items: Vec<Item> = Vec::new();
    let mut steps = 0;

    let mut order: Vec<usize> = (0..replies.len()).collect();
    order.sort_by_key(|&i| (replies[i].at, replies[i].id.clone()));
    for &r in &order {
        let reply = &replies[r];
        for (s, step) in reply.steps.iter().enumerate() {
            let Some(tool) = step.tool.as_deref() else { continue };
            if step.kind != "tool" || !matches!(step.status.as_str(), "ok" | "failed") {
                continue;
            }
            steps += 1;
            let key = tool_key(tool);
            match gate(reply.engine.as_deref(), tool, &key, step) {
                Gate::FolderFile => by_design.folder_files += 1,
                Gate::KwantTool => by_design.kwant_tools += 1,
                Gate::Lookup => by_design.lookups += 1,
                Gate::OtherEngine => by_design.other_engines += 1,
                Gate::Card => {
                    let start = step.started_at.unwrap_or(reply.at);
                    let end = step.ended_at.unwrap_or(reply.at).max(start);
                    let command = (tool == "Bash").then(|| step.detail.clone()).flatten();
                    let read_only = command.as_deref().is_some_and(reads_only_for_claude);
                    items.push(Item { reply: r, step: s, key, start, end, command, read_only, covered: false });
                }
            }
        }
    }

    let fits = |item: &Item, row: &Approval| {
        let reply = &replies[item.reply];
        row.bot_id == reply.bot_id && tool_key(&row.tool) == item.key && chats_meet(row.chat.as_deref(), reply.chat.as_deref())
    };
    let slack = Duration::seconds(SLACK_SECS);

    // Pass 0: an approval step names its row; it belongs to the call it waited on
    let by_id: HashMap<&str, usize> = rows.iter().enumerate().map(|(i, row)| (row.id.as_str(), i)).collect();
    for &r in &order {
        for step in replies[r].steps.iter().filter(|s| s.kind == "approval") {
            let Some(&row) = step.ref_id.as_deref().and_then(|id| by_id.get(id)) else { continue };
            if used[row] {
                continue;
            }
            let asked = step.started_at.unwrap_or(replies[r].at);
            let pick = items
                .iter()
                .enumerate()
                .filter(|(_, it)| it.reply == r && !it.covered && fits(it, &rows[row]))
                .min_by_key(|(_, it)| (it.start > asked, (asked - it.start).num_milliseconds().abs()))
                .map(|(i, _)| i);
            if let Some(i) = pick {
                items[i].covered = true;
                used[row] = true;
            }
        }
    }

    // Passes 1 and 2: the nearest row in the step's window, writing calls first
    for read_only in [false, true] {
        let mut pairs: Vec<(i64, usize, usize)> = Vec::new();
        for (i, it) in items.iter().enumerate().filter(|(_, it)| !it.covered && it.read_only == read_only) {
            for (j, row) in rows.iter().enumerate() {
                if !used[j] && fits(it, row) && row.at >= it.start - slack && row.at <= it.end + slack {
                    pairs.push(((row.at - it.start).num_milliseconds().abs(), i, j));
                }
            }
        }
        pairs.sort_unstable();
        for (_, i, j) in pairs {
            if !items[i].covered && !used[j] {
                items[i].covered = true;
                used[j] = true;
            }
        }
    }

    // Pass 3: a late Allow (the card answered after the turn ended) runs once in the resumed turn
    let mut late: Vec<usize> = (0..items.len()).filter(|&i| !items[i].covered).collect();
    late.sort_by_key(|&i| items[i].start);
    for i in late {
        let it = &items[i];
        let pick = rows
            .iter()
            .enumerate()
            .filter(|(j, row)| {
                !used[*j]
                    && matches!(row.status.as_str(), "allowed_once" | "always_allowed")
                    && fits(it, row)
                    && row.at <= it.start + slack
                    && it.start - row.at <= Duration::minutes(GRANT_MINUTES)
                    && match (it.command.as_deref(), row_command(row)) {
                        (Some(a), Some(b)) => a == b,
                        _ => true,
                    }
            })
            .max_by_key(|(_, row)| row.at)
            .map(|(j, _)| j);
        if let Some(j) = pick {
            used[j] = true;
            items[i].covered = true;
        }
    }

    let with_card = items.iter().filter(|it| it.covered).count();
    let mut found = Vec::new();
    for it in items.iter().filter(|it| !it.covered) {
        let reply = &replies[it.reply];
        if it.read_only {
            by_design.read_only_commands += 1;
            continue;
        }
        let always = rows.iter().any(|row| row.status == "always_allowed" && fits(it, row) && row.at <= it.start + slack);
        if always {
            by_design.always_allowed += 1;
            continue;
        }
        let step = &reply.steps[it.step];
        found.push(Finding {
            rule: Rule::RanWithNoCard,
            at: it.start,
            bot: reply.bot.clone(),
            bot_id: reply.bot_id.clone(),
            chat: reply.chat_label.clone(),
            tool: step.tool.clone().unwrap_or_default(),
            title: clean_title(&step.title),
            reply_id: reply.id.clone(),
            approval_id: None,
            declined_id: None,
            declined_at: None,
        });
    }

    let judged = |row: &Approval| since.is_none_or(|c| row.at >= c) && bot_id.is_none_or(|b| row.bot_id == b);
    let nos = rows.iter().filter(|r| r.persons_no() && judged(r)).count();
    found.extend(asked_again(&rows, person_messages).into_iter().filter(|(i, _)| judged(&rows[*i])).map(|(_, f)| f));
    found.sort_by(|a, b| (a.at, &a.bot).cmp(&(b.at, &b.bot)));

    Report {
        since: None,
        replies: replies.len(),
        steps,
        with_card,
        nos,
        by_design,
        findings: found,
    }
}

/// Rule 2 over chat-filled rows: each row (a card, or a call that ran without asking) whose
/// families overlap an earlier no by the person to the same bot in the same chat, with no message
/// from the person in that chat after the no. With the index of the row asked again.
///
/// The no's time is its card's: the table keeps no answer time, so a message the person wrote
/// while the card waited counts as after the no.
fn asked_again(rows: &[Approval], person_messages: &[PersonMessage]) -> Vec<(usize, Finding)> {
    let families: Vec<Vec<String>> = rows.iter().map(row_families).collect();
    let mut out = Vec::new();
    for (i, row) in rows.iter().enumerate() {
        // Rows kwnt refused on its own (27b's own refusal, kwnt's secrets) are the rule holding
        let asked = row.card || row.status == "auto_allowed";
        let Some(chat) = row.chat.as_deref().filter(|_| asked && !families[i].is_empty()) else { continue };
        let no = rows
            .iter()
            .enumerate()
            .filter(|(j, e)| {
                e.persons_no()
                    && e.at < row.at
                    && e.bot_id == row.bot_id
                    && e.chat.as_deref() == Some(chat)
                    && families[*j].iter().any(|f| families[i].contains(f))
                    && !person_messages.iter().any(|m| m.chat == chat && m.at > e.at && m.at <= row.at)
            })
            .max_by_key(|(_, e)| e.at);
        let Some((_, no)) = no else { continue };
        out.push((
            i,
            Finding {
                rule: Rule::AskedAgainAfterNo,
                at: row.at,
                bot: row.bot.clone(),
                bot_id: row.bot_id.clone(),
                chat: row.chat_label.clone().unwrap_or_else(|| "a chat".into()),
                tool: row.tool.clone(),
                title: clean_title(&row_summary(row, &families[i])),
                reply_id: String::new(),
                approval_id: Some(row.id.clone()),
                declined_id: Some(no.id.clone()),
                declined_at: Some(no.at),
            },
        ));
    }
    out
}

/// What a "no" to this row's call covers, as 27b's gate reads it: the [`ToolApprovalRequest`]
/// is rebuilt from the row's tool name and saved parameters.
fn row_families(row: &Approval) -> Vec<String> {
    let p = &row.parameters;
    let text = |k: &str| p.get(k).and_then(Value::as_str).unwrap_or_default().to_string();
    let builtin = (text("source") == "claude_builtin").then(|| BuiltinCall {
        cwd: p.get("cwd").and_then(Value::as_str).map(str::to_string),
        outside_folder: p.get("outside_folder").and_then(Value::as_bool).unwrap_or(false),
    });
    let tool = Some(text("tool")).filter(|t| !t.is_empty()).unwrap_or_else(|| row.tool.clone());
    let req = ToolApprovalRequest {
        kwant_id: row.bot_id.clone(),
        kwant_name: row.bot.clone(),
        turn_id: row.turn_id.clone().unwrap_or_default(),
        tool_name: row.tool.clone(),
        server: if builtin.is_some() { String::new() } else { text("server") },
        tool,
        arguments: p.get("arguments").cloned().unwrap_or(Value::Null),
        builtin,
        smart: None,
        payment: None,
    };
    declined::families(&req)
}

/// A row's call in a few words: its command, address or file, else what a no to it covers.
fn row_summary(row: &Approval, families: &[String]) -> String {
    let args = row.parameters.get("arguments").unwrap_or(&Value::Null);
    let named = ["command", "cmd", "url", "file_path", "path"].iter().find_map(|k| match &args[*k] {
        Value::String(s) => Some(s.clone()),
        Value::Array(words) => Some(words.iter().filter_map(Value::as_str).collect::<Vec<_>>().join(" ")),
        _ => None,
    });
    named.unwrap_or_else(|| families.join(", "))
}

/// What claude runs without asking kwnt: a command that only reads. kwnt's own list
/// ([`shell_risk::reads_only`](crate::mcp::shell_risk::reads_only)) has no `find`, which claude
/// treats as a read; a `find` with none of [`FIND_ACTIONS`] counts here too. This only sorts the
/// report: the approval gate is unchanged.
pub fn reads_only_for_claude(command: &str) -> bool {
    use crate::mcp::shell_risk::reads_only;
    let anywhere = Path::new("/");
    if reads_only(command, anywhere) {
        return true;
    }
    let Some(parts) = split_commands(command) else { return false };
    if !parts.iter().any(|p| plain_find(p)) {
        return false;
    }
    let rest: Vec<&str> = parts.iter().map(|p| if plain_find(p) { "true" } else { p.as_str() }).collect();
    reads_only(&rest.join(" && "), anywhere)
}

/// `find` options that run, delete or write something.
const FIND_ACTIONS: &[&str] = &["-exec", "-execdir", "-ok", "-okdir", "-delete", "-fprint", "-fprint0", "-fprintf", "-fls"];

fn plain_find(part: &str) -> bool {
    let part = part.trim();
    let Some(args) = part.strip_prefix("find").filter(|a| a.is_empty() || a.starts_with(' ')) else { return false };
    let args = args.replace("2>/dev/null", " ").replace(">/dev/null", " ").replace("2>&1", " ");
    !args.contains(['>', '<', '`', '$', '&', '\\'])
        && !args.split_whitespace().map(|w| w.trim_matches(['\'', '"'])).any(|w| FIND_ACTIONS.contains(&w))
}

/// `a && b || c; d | e` as its simple commands, split outside quotes. `None` for a command with
/// a lone `&` (background) or an open quote.
fn split_commands(command: &str) -> Option<Vec<String>> {
    let mut parts = Vec::new();
    let mut cur = String::new();
    let (mut single, mut double) = (false, false);
    let mut chars = command.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\'' if !double => single = !single,
            '"' if !single => double = !double,
            ';' | '\n' | '|' | '&' if !single && !double => {
                if c == '&' && chars.peek() != Some(&'&') {
                    // `2>&1` stays in its command; a lone `&` sends a command to the background
                    if cur.ends_with('>') {
                        cur.push(c);
                        continue;
                    }
                    return None;
                }
                if matches!(c, '&' | '|') && chars.peek() == Some(&c) {
                    chars.next();
                }
                parts.push(std::mem::take(&mut cur));
                continue;
            }
            _ => {}
        }
        cur.push(c);
    }
    if single || double {
        return None;
    }
    parts.push(cur);
    Some(parts.into_iter().map(|p| p.trim().to_string()).filter(|p| !p.is_empty()).collect())
}

/// Rows saved without `resume` (auto-allowed, refused secrets) take the chat of a row of the
/// same turn.
fn fill_chats(rows: &mut [Approval]) {
    let mut by_turn: HashMap<String, String> = HashMap::new();
    for row in rows.iter() {
        if let (Some(turn), Some(chat)) = (&row.turn_id, &row.chat) {
            by_turn.entry(turn.clone()).or_insert_with(|| chat.clone());
        }
    }
    let mut labels: HashMap<String, String> = HashMap::new();
    for row in rows.iter() {
        if let (Some(chat), Some(label)) = (&row.chat, &row.chat_label) {
            labels.entry(chat.clone()).or_insert_with(|| label.clone());
        }
    }
    for row in rows.iter_mut().filter(|r| r.chat.is_none()) {
        row.chat = row.turn_id.as_ref().and_then(|t| by_turn.get(t)).cloned();
    }
    for row in rows.iter_mut().filter(|r| r.chat_label.is_none()) {
        row.chat_label = row.chat.as_ref().and_then(|c| labels.get(c)).cloned();
    }
}

/// A title as the output shows it: redacted, on one line, cut to [`MAX_TITLE`] characters.
pub fn clean_title(title: &str) -> String {
    let one_line: String = redact(title, &[]).split_whitespace().collect::<Vec<_>>().join(" ");
    if one_line.chars().count() <= MAX_TITLE {
        return one_line;
    }
    let mut cut: String = one_line.chars().take(MAX_TITLE - 1).collect();
    cut.push('…');
    cut
}

// ---------------------------------------------------------------------------------------------
// Since, loading and printing

/// How far back to look: `24h`, `7d`, or `all`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Since {
    All,
    Last(Duration),
}

impl std::str::FromStr for Since {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let s = s.trim();
        if s.eq_ignore_ascii_case("all") {
            return Ok(Since::All);
        }
        let bad = || format!("`{s}` isn't a time span: use something like 24h, 7d or all");
        let unit = s.chars().last().ok_or_else(bad)?;
        let n: i64 = s[..s.len() - unit.len_utf8()].parse().map_err(|_| bad())?;
        match unit {
            'h' if (1..=24 * 3650).contains(&n) => Ok(Since::Last(Duration::hours(n))),
            'd' if (1..=3650).contains(&n) => Ok(Since::Last(Duration::days(n))),
            _ => Err(bad()),
        }
    }
}

impl Since {
    pub fn cutoff(self, now: DateTime<Utc>) -> Option<DateTime<Utc>> {
        match self {
            Since::All => None,
            Since::Last(d) => Some(now - d),
        }
    }
}

/// Everything [`check`] reads, from the database.
#[derive(Debug, Clone, Default)]
pub struct Loaded {
    pub replies: Vec<Reply>,
    pub person_messages: Vec<PersonMessage>,
    pub approvals: Vec<Approval>,
    /// Bot id → name.
    pub bots: Vec<(String, String)>,
}

/// SQLite timestamps (`2026-11-06 18:33:23`) and step times (RFC 3339).
fn parse_time(s: &str) -> Option<DateTime<Utc>> {
    if let Ok(t) = DateTime::parse_from_rfc3339(s) {
        return Some(t.with_timezone(&Utc));
    }
    ["%Y-%m-%d %H:%M:%S%.f", "%Y-%m-%d %H:%M:%S", "%Y-%m-%dT%H:%M:%S%.f"]
        .iter()
        .find_map(|f| NaiveDateTime::parse_from_str(s, f).ok())
        .map(|t| t.and_utc())
}

fn chat_key(thread: Option<&str>, channel: Option<&str>, direct: Option<&str>) -> Option<String> {
    let some = |v: Option<&str>| v.filter(|v| !v.is_empty()).map(str::to_string);
    some(thread)
        .map(|t| format!("thread:{t}"))
        .or_else(|| some(channel).map(|c| format!("channel:{c}")))
        .or_else(|| some(direct).map(|d| format!("direct:{d}")))
}

fn steps_of(meta: &Value) -> Vec<Step> {
    let text = |s: &Value, k: &str| s.get(k).and_then(Value::as_str).map(str::to_string);
    meta.pointer("/activity/steps")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .map(|s| Step {
            kind: text(s, "kind").unwrap_or_default(),
            status: text(s, "status").unwrap_or_default(),
            tool: text(s, "tool"),
            title: text(s, "title").unwrap_or_default(),
            detail: text(s, "detail"),
            ref_id: text(s, "ref_id"),
            started_at: s.get("started_at").and_then(Value::as_str).and_then(parse_time),
            ended_at: s.get("ended_at").and_then(Value::as_str).and_then(parse_time),
        })
        .collect()
}

/// Reads replies since `cutoff`, every person message and every approvals row from
/// `<data_dir>/kwnt.db`, opened read-only.
pub async fn load(data_dir: &Path, cutoff: Option<DateTime<Utc>>) -> Result<Loaded, String> {
    use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
    let path = data_dir.join("kwnt.db");
    if !path.is_file() {
        return Err(format!("no kwant database at {}", path.display()));
    }
    let opts = SqliteConnectOptions::new().filename(&path).read_only(true).create_if_missing(false);
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(opts)
        .await
        .map_err(|e| format!("can't open {} read-only: {e}", path.display()))?;
    let out = read_all(&pool, cutoff).await.map_err(|e| format!("can't read {}: {e}", path.display()));
    pool.close().await;
    out
}

/// A person message: time, then thread, channel and direct bot.
type PersonRow = (String, Option<String>, Option<String>, Option<String>);
type Row7 = (String, String, String, Option<String>, Option<String>, Option<String>, Option<String>);

async fn read_all(pool: &sqlx::SqlitePool, cutoff: Option<DateTime<Utc>>) -> Result<Loaded, sqlx::Error> {
    let bots: Vec<(String, String)> = sqlx::query_as("SELECT id, name FROM kwants").fetch_all(pool).await?;
    let channels: HashMap<String, String> = sqlx::query_as::<_, (String, String)>("SELECT id, name FROM channels").fetch_all(pool).await?.into_iter().collect();
    // Older databases may have no threads table
    let threads: HashMap<String, String> = sqlx::query_as::<_, (String, String)>("SELECT id, title FROM threads").fetch_all(pool).await.unwrap_or_default().into_iter().collect();
    let name_of: HashMap<&str, &str> = bots.iter().map(|(id, name)| (id.as_str(), name.as_str())).collect();
    let id_of_name: HashMap<&str, &str> = bots.iter().map(|(id, name)| (name.as_str(), id.as_str())).collect();

    let label = |key: &str| -> String {
        let (kind, id) = key.split_once(':').unwrap_or(("", key));
        match kind {
            "thread" => threads.get(id).map(|t| format!("thread \"{}\"", clean_title(t))).unwrap_or_else(|| "a thread".into()),
            "channel" => channels.get(id).map(|n| format!("#{n}")).unwrap_or_else(|| "a channel".into()),
            _ => format!("chat with {}", name_of.get(id).copied().unwrap_or(id)),
        }
    };

    let since = cutoff.map(|c| c.format("%Y-%m-%d %H:%M:%S").to_string()).unwrap_or_default();
    let replies: Vec<Row7> = sqlx::query_as(
        "SELECT id, sender_id, COALESCE(CAST(created_at AS TEXT), ''), thread_id, channel_id, direct_kwant_id, meta FROM messages \
         WHERE sender_role = 'kwant' AND created_at >= ? AND meta LIKE '%\"steps\"%' ORDER BY created_at",
    )
    .bind(&since)
    .fetch_all(pool)
    .await?;
    let replies = replies
        .into_iter()
        .filter_map(|(id, sender, at, thread, channel, direct, meta)| {
            let meta: Value = serde_json::from_str(meta.as_deref()?).ok()?;
            let (bot_id, bot) = match (name_of.get(sender.as_str()), id_of_name.get(sender.as_str())) {
                (Some(name), _) => (sender.clone(), name.to_string()),
                (None, Some(id)) => (id.to_string(), sender.clone()),
                (None, None) => (sender.clone(), sender.clone()),
            };
            let chat = chat_key(thread.as_deref(), channel.as_deref(), direct.as_deref());
            Some(Reply {
                id,
                at: parse_time(&at)?,
                bot_id,
                bot,
                chat_label: chat.as_deref().map(label).unwrap_or_else(|| "a chat".into()),
                chat,
                engine: meta.pointer("/ran_on/engine").and_then(Value::as_str).map(str::to_string),
                steps: steps_of(&meta),
            })
        })
        .collect();

    let people: Vec<PersonRow> =
        sqlx::query_as("SELECT COALESCE(CAST(created_at AS TEXT), ''), thread_id, channel_id, direct_kwant_id FROM messages WHERE sender_role = 'user'").fetch_all(pool).await?;
    let person_messages = people
        .into_iter()
        .filter_map(|(at, thread, channel, direct)| {
            Some(PersonMessage { chat: chat_key(thread.as_deref(), channel.as_deref(), direct.as_deref())?, at: parse_time(&at)? })
        })
        .collect();

    type ApprovalRow = (String, Option<String>, String, String, Option<String>, String, String, Option<String>);
    let rows: Vec<ApprovalRow> =
        sqlx::query_as("SELECT id, turn_id, kwant_id, tool_name, parameters, status, COALESCE(CAST(created_at AS TEXT), ''), resume FROM approvals").fetch_all(pool).await?;
    let approvals = rows
        .into_iter()
        .filter_map(|(id, turn_id, bot_id, tool, parameters, status, at, resume)| {
            let resume: Value = resume.as_deref().and_then(|r| serde_json::from_str(r).ok()).unwrap_or_default();
            let field = |k: &str| resume.get(k).and_then(Value::as_str);
            let chat = chat_key(field("thread_id"), field("channel_id"), field("direct_kwant_id"));
            Some(Approval {
                bot: name_of.get(bot_id.as_str()).map_or_else(|| bot_id.clone(), |n| n.to_string()),
                id,
                turn_id,
                bot_id,
                tool,
                parameters: parameters.as_deref().and_then(|p| serde_json::from_str(p).ok()).unwrap_or_default(),
                card: chat.is_some() && !matches!(status.as_str(), "auto_allowed" | crate::db::approval_modes::CUSTOMER_REFUSED),
                status,
                at: parse_time(&at)?,
                chat_label: chat.as_deref().map(label),
                chat,
            })
        })
        .collect();

    Ok(Loaded { replies, person_messages, approvals, bots })
}

fn when(t: DateTime<Utc>) -> String {
    t.format("%Y-%m-%d %H:%M:%S UTC").to_string()
}

/// The printed report: a count line, what ran without a card on purpose, then the findings, or
/// "Nothing to flag since …".
pub fn render_text(report: &Report) -> String {
    let since = report.since.map(when).unwrap_or_else(|| "the first reply".into());
    let mut out = String::new();
    let plural = |n: usize, one: &str, many: &str| format!("{n} {}", if n == 1 { one } else { many });
    out.push_str(&format!(
        "Checked {} since {since}: {}, {} with a card.\n",
        plural(report.replies, "reply", "replies"),
        plural(report.steps, "tool step", "tool steps"),
        report.with_card,
    ));
    let b = &report.by_design;
    let parts: Vec<String> = [
        (b.folder_files, "file read or edit", "file reads and edits", " in the bot's own folder"),
        (b.read_only_commands, "read-only command", "read-only commands", ""),
        (b.kwant_tools, "kwant tool", "kwant tools", ""),
        (b.lookups, "look-up", "look-ups", ""),
        (b.always_allowed, "call", "calls", " of a tool the person chose Always allow for"),
        (b.other_engines, "step", "steps", " by another engine's own tools (its sandbox, no card)"),
    ]
    .iter()
    .filter(|(n, ..)| *n > 0)
    .map(|(n, one, many, tail)| format!("{}{tail}", plural(*n, one, many)))
    .collect();
    if !parts.is_empty() {
        out.push_str(&format!("No card needed: {}.\n", parts.join(", ")));
    }
    if report.nos > 0 {
        out.push_str(&format!("The person said no on {}.\n", plural(report.nos, "card", "cards")));
    }
    if report.findings.is_empty() {
        out.push_str(&format!("Nothing to flag since {since}.\n"));
        return out;
    }
    let of = |rule: Rule| report.findings.iter().filter(move |f| f.rule == rule);
    let no_card = of(Rule::RanWithNoCard).count();
    if no_card > 0 {
        out.push_str(&format!("\nRan with no card ({no_card})\n"));
        for f in of(Rule::RanWithNoCard) {
            out.push_str(&format!("  {} · {} · {} · {} · {}\n", when(f.at), f.bot, f.chat, f.tool, f.title));
        }
    }
    let again = of(Rule::AskedAgainAfterNo).count();
    if again > 0 {
        out.push_str(&format!("\nAsked again after a no ({again})\n"));
        for f in of(Rule::AskedAgainAfterNo) {
            let no = f.declined_at.map(|t| t.format("%H:%M:%S").to_string()).unwrap_or_default();
            out.push_str(&format!("  {} · {} · {} · {} · {} · no at {no}, nothing from the person since\n", when(f.at), f.bot, f.chat, f.tool, f.title));
        }
    }
    out
}

pub fn render_json(report: &Report) -> String {
    serde_json::to_string_pretty(report).unwrap_or_else(|_| "{}".into())
}

/// `kwnt run-check`'s options.
#[derive(Debug, Clone)]
pub struct Options {
    pub since: Since,
    pub bot: Option<String>,
    pub json: bool,
    pub strict: bool,
}

/// Runs the check and prints it. Exit code: 0, or 1 with `--strict` and anything found, or 2 when
/// the database can't be read or the bot isn't known.
pub async fn run_cli(data_dir: &Path, opts: Options) -> i32 {
    let cutoff = opts.since.cutoff(Utc::now());
    let mut loaded = match load(data_dir, cutoff).await {
        Ok(l) => l,
        Err(e) => {
            eprintln!("kwnt run-check: {e}");
            return 2;
        }
    };
    let mut bot_id = None;
    if let Some(want) = opts.bot.as_deref().map(str::trim) {
        let Some((id, _)) = loaded.bots.iter().find(|(id, name)| id == want || name.eq_ignore_ascii_case(want)) else {
            eprintln!("kwnt run-check: no bot named {want}");
            return 2;
        };
        let id = id.clone();
        loaded.replies.retain(|r| r.bot_id == id);
        bot_id = Some(id);
    }
    let mut report = check_since(&loaded.replies, &loaded.person_messages, &loaded.approvals, cutoff, bot_id.as_deref());
    report.since = cutoff.or_else(|| loaded.replies.iter().map(|r| r.at).min());
    if opts.json {
        println!("{}", render_json(&report));
    } else {
        print!("{}", render_text(&report));
    }
    if opts.strict && !report.findings.is_empty() {
        1
    } else {
        0
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn t(s: &str) -> DateTime<Utc> {
        parse_time(s).unwrap()
    }

    fn tool_step(tool: &str, at: &str, title: &str, detail: Option<&str>) -> Step {
        Step {
            kind: "tool".into(),
            status: "ok".into(),
            tool: Some(tool.into()),
            title: title.into(),
            detail: detail.map(str::to_string),
            ref_id: None,
            started_at: Some(t(at)),
            ended_at: Some(t(at) + Duration::milliseconds(80)),
        }
    }

    fn reply(at: &str, engine: &str, steps: Vec<Step>) -> Reply {
        Reply {
            id: format!("r-{at}"),
            at: t(at) + Duration::seconds(3),
            bot_id: "b1".into(),
            bot: "Forge".into(),
            chat: Some("direct:b1".into()),
            chat_label: "chat with Forge".into(),
            engine: Some(engine.into()),
            steps,
        }
    }

    fn row(id: &str, tool: &str, status: &str, at: &str, chat: Option<&str>) -> Approval {
        Approval {
            id: id.into(),
            turn_id: Some(format!("turn-{id}")),
            bot_id: "b1".into(),
            tool: tool.into(),
            parameters: json!({}),
            status: status.into(),
            at: t(at),
            chat: chat.map(str::to_string),
            chat_label: chat.map(|_| "chat with Forge".to_string()),
            bot: "Forge".into(),
            card: chat.is_some() && status != "auto_allowed",
        }
    }

    #[test]
    fn tool_keys_agree_across_engines() {
        let k = tool_key("mcp__github__create_issue");
        assert_eq!(k, "github_create_issue");
        assert_eq!(tool_key("mcp__kwnt__github__create_issue"), k);
        assert_eq!(tool_key("github-create_issue"), k);
        assert_eq!(tool_key("github__create_issue"), k);
        assert_eq!(tool_key("mcp__kwnt__kwnt_tools__ask_user"), "kwnt_tools_ask_user");
        assert_eq!(tool_key("Bash"), "bash");
    }

    #[test]
    fn gate_by_engine_and_tool() {
        let g = |e: Option<&str>, tool: &str| gate(e, tool, &tool_key(tool), &Step::default());
        assert_eq!(g(Some("claude"), "Bash"), Gate::Card);
        assert_eq!(g(Some("claude"), "Write"), Gate::FolderFile);
        assert_eq!(g(None, "Read"), Gate::FolderFile);
        assert_eq!(g(Some("claude"), "ToolSearch"), Gate::Lookup);
        assert_eq!(g(Some("claude"), "SomeNewTool"), Gate::Card);
        assert_eq!(g(Some("claude"), "mcp__kwnt_tools__ask_user"), Gate::KwantTool);
        assert_eq!(g(Some("claude"), "mcp__github__create_issue"), Gate::Card);
        assert_eq!(g(Some("codex"), "command_execution"), Gate::OtherEngine);
        assert_eq!(g(Some("codex"), "mcp__kwnt__github__create_issue"), Gate::Card);
        assert_eq!(g(Some("grok"), "github-create_issue"), Gate::Card);
        assert_eq!(g(Some("grok"), "bash"), Gate::OtherEngine);
        assert_eq!(g(Some("anthropic_api"), "github__create_issue"), Gate::Card);
        assert_eq!(g(Some("ollama"), "kwnt_tools__ask_user"), Gate::KwantTool);
    }

    #[test]
    fn a_step_with_its_row_is_clean_and_one_without_is_flagged() {
        let r = reply(
            "2026-11-06T18:30:00.100Z",
            "claude",
            vec![tool_step("mcp__github__merge", "2026-11-06T18:30:00.100Z", "Merging", None), tool_step("mcp__github__merge", "2026-11-06T18:30:01.500Z", "Merging again", None)],
        );
        let rows = vec![row("a1", "mcp__github__merge", "allowed_once", "2026-11-06 18:30:00", Some("direct:b1"))];
        let report = check(&[r], &[], &rows);
        assert_eq!(report.steps, 2);
        assert_eq!(report.with_card, 1);
        assert_eq!(report.findings.len(), 1);
        assert_eq!(report.findings[0].title, "Merging again");
        assert_eq!(report.findings[0].rule, Rule::RanWithNoCard);
    }

    #[test]
    fn rows_of_another_bot_or_chat_or_time_dont_cover() {
        let r = reply("2026-11-06T18:30:00.100Z", "claude", vec![tool_step("mcp__github__merge", "2026-11-06T18:30:00.100Z", "Merging", None)]);
        let mut other_bot = row("a1", "mcp__github__merge", "allowed_once", "2026-11-06 18:30:00", Some("direct:b1"));
        other_bot.bot_id = "b2".into();
        let other_chat = row("a2", "mcp__github__merge", "allowed_once", "2026-11-06 18:30:00", Some("channel:c1"));
        let too_early = row("a3", "mcp__github__merge", "denied", "2026-11-06 18:20:00", Some("direct:b1"));
        assert_eq!(findings(&[r], &[], &[other_bot, other_chat, too_early]).len(), 1);
    }

    #[test]
    fn a_rowless_auto_allowed_row_takes_its_turns_chat() {
        let r = reply("2026-11-06T18:30:00.100Z", "claude", vec![tool_step("mcp__github__merge", "2026-11-06T18:30:00.100Z", "Merging", None)]);
        let mut auto = row("a1", "mcp__github__merge", "auto_allowed", "2026-11-06 18:30:00", None);
        auto.turn_id = Some("t".into());
        let mut sibling = row("a2", "mcp__other__x", "allowed_once", "2026-11-06 18:29:59", Some("channel:c1"));
        sibling.turn_id = Some("t".into());
        // Its turn ran in another chat: not this step's row
        assert_eq!(findings(std::slice::from_ref(&r), &[], &[auto.clone(), sibling]).len(), 1);
        // With no sibling the chat is unknown, and the row counts
        assert!(findings(&[r], &[], &[auto]).is_empty());
    }

    #[test]
    fn always_allow_earlier_in_the_chat_covers_later_calls() {
        let first = reply("2026-11-06T18:30:00.100Z", "claude", vec![tool_step("mcp__github__merge", "2026-11-06T18:30:00.100Z", "Merging", None)]);
        let later = reply("2026-11-06T19:45:00.000Z", "claude", vec![tool_step("mcp__github__merge", "2026-11-06T19:45:00.000Z", "Merging later", None)]);
        let rows = vec![row("a1", "mcp__github__merge", "always_allowed", "2026-11-06 18:30:00", Some("direct:b1"))];
        let report = check(&[first, later], &[], &rows);
        assert!(report.findings.is_empty(), "{:?}", report.findings);
        assert_eq!(report.by_design.always_allowed, 1);
    }

    #[test]
    fn a_late_allow_covers_one_call_in_the_resumed_turn() {
        let resumed = reply(
            "2026-11-06T18:40:00.000Z",
            "claude",
            vec![
                tool_step("Bash", "2026-11-06T18:40:00.000Z", "Running `gh pr merge 7`", Some("gh pr merge 7")),
                tool_step("Bash", "2026-11-06T18:40:01.000Z", "Running `gh pr merge 7`", Some("gh pr merge 7")),
            ],
        );
        let mut card = row("a1", "Bash", "allowed_once", "2026-11-06 18:31:00", Some("direct:b1"));
        card.parameters = json!({"arguments": {"command": "gh pr merge 7"}});
        let report = check(&[resumed], &[], &[card]);
        assert_eq!(report.with_card, 1);
        assert_eq!(report.findings.len(), 1);
    }

    #[test]
    fn an_approval_step_ties_its_row_to_its_call() {
        let mut ask = tool_step("Bash", "2026-11-06T18:30:00.050Z", "Waiting for your approval", None);
        ask.kind = "approval".into();
        ask.ref_id = Some("a1".into());
        let r = reply(
            "2026-11-06T18:30:00.000Z",
            "claude",
            vec![tool_step("Bash", "2026-11-06T18:30:00.000Z", "Running `rm -r out`", Some("rm -r out")), ask],
        );
        let rows = vec![row("a1", "Bash", "allowed_once", "2026-11-06 18:30:00", Some("direct:b1"))];
        let report = check(&[r], &[], &rows);
        assert!(report.findings.is_empty());
        assert_eq!(report.with_card, 1);
    }

    #[test]
    fn steps_that_need_no_card_are_counted_not_flagged() {
        let r = reply(
            "2026-11-06T18:30:00.000Z",
            "claude",
            vec![
                tool_step("Write", "2026-11-06T18:30:00.000Z", "Writing …/b1/a.txt", None),
                tool_step("ToolSearch", "2026-11-06T18:30:00.100Z", "Looking up tools", None),
                tool_step("mcp__kwnt_tools__ask_user", "2026-11-06T18:30:00.200Z", "Asking you", None),
            ],
        );
        let mut running = r.clone();
        running.steps[0].status = "running".into();
        let report = check(&[r], &[], &[]);
        assert!(report.findings.is_empty());
        assert_eq!((report.by_design.folder_files, report.by_design.lookups, report.by_design.kwant_tools), (1, 1, 1));
        assert_eq!(check(&[running], &[], &[]).steps, 2);
    }

    fn fetch(at: &str, url: Option<&str>, title: &str) -> Step {
        tool_step("WebFetch", at, title, url)
    }

    #[test]
    fn a_rowless_fetch_of_this_computer_or_the_local_network_is_a_finding() {
        // 8799 is always one of kwnt's ports; localhost on another port only asks for /api/
        let steps = vec![
            fetch("2026-11-06T18:30:00.000Z", Some("http://127.0.0.1:8799/api/settings"), "Opening 127.0.0.1"),
            fetch("2026-11-06T18:30:01.000Z", Some("http://localhost/api/spaces"), "Opening localhost"),
            fetch("2026-11-06T18:30:02.000Z", Some("http://192.168.1.20/admin"), "Opening 192.168.1.20"),
            fetch("2026-11-06T18:30:03.000Z", Some("http://my-nas.local/"), "Opening my-nas.local"),
            fetch("2026-11-06T18:30:04.000Z", Some("http://169.254.169.254/latest/meta-data/"), "Opening 169.254.169.254"),
            // An older step with no detail: the title's host is enough for a private IP
            fetch("2026-11-06T18:30:05.000Z", None, "Opening 10.0.0.5"),
        ];
        let report = check(&[reply("2026-11-06T18:30:00.000Z", "claude", steps.clone())], &[], &[]);
        let titles: Vec<&str> = report.findings.iter().map(|f| f.title.as_str()).collect();
        assert_eq!(
            titles,
            ["Opening 127.0.0.1", "Opening localhost", "Opening 192.168.1.20", "Opening my-nas.local", "Opening 169.254.169.254", "Opening 10.0.0.5"]
        );
        assert!(report.findings.iter().all(|f| f.rule == Rule::RanWithNoCard && f.tool == "WebFetch"));
        assert_eq!(report.by_design.lookups, 0);
        // A reply with no engine recorded is judged as claude
        let mut engineless = reply("2026-11-06T18:30:00.000Z", "claude", steps);
        engineless.engine = None;
        assert_eq!(check(&[engineless], &[], &[]).findings.len(), 6);
    }

    #[test]
    fn a_public_fetch_or_the_local_dev_server_stays_a_lookup() {
        let steps = vec![
            fetch("2026-11-06T18:30:00.000Z", Some("https://github.com/alexsergivan/kwant/pull/798"), "Opening github.com"),
            fetch("2026-11-06T18:30:01.000Z", Some("http://localhost:3000/"), "Opening localhost"),
            fetch("2026-11-06T18:30:02.000Z", Some("http://127.0.0.1:5173/index.html"), "Opening 127.0.0.1"),
            // Names are never looked up: a public name stays public
            fetch("2026-11-06T18:30:03.000Z", Some("http://localtest.me:8799/api/x"), "Opening localtest.me"),
            fetch("2026-11-06T18:30:04.000Z", None, "Opening github.com"),
            fetch("2026-11-06T18:30:05.000Z", None, "Opening a page"),
        ];
        let report = check(&[reply("2026-11-06T18:30:00.000Z", "claude", steps)], &[], &[]);
        assert!(report.findings.is_empty(), "{:?}", report.findings);
        assert_eq!(report.by_design.lookups, 6);
        // Another engine's own fetch has no card hook, local or not
        let codex = reply("2026-11-06T18:31:00.000Z", "codex", vec![fetch("2026-11-06T18:31:00.000Z", Some("http://127.0.0.1:8799/api/x"), "Opening 127.0.0.1")]);
        let report = check(&[codex], &[], &[]);
        assert!(report.findings.is_empty());
        assert_eq!(report.by_design.other_engines, 1);
    }

    #[test]
    fn a_failed_fetch_is_judged_by_its_address_not_its_failure_reason() {
        let failed = |at: &str, detail: Option<&str>, title: &str| Step { status: "failed".into(), ..fetch(at, detail, title) };
        let public = vec![
            failed("2026-11-06T18:30:00.000Z", Some("Request failed with status code 404"), "Opening example.com"),
            failed("2026-11-06T18:30:01.000Z", Some("fetch failed"), "Opening example.org"),
            failed("2026-11-06T18:30:02.000Z", Some("timeout"), "Opening example.net"),
        ];
        let report = check(&[reply("2026-11-06T18:30:00.000Z", "claude", public)], &[], &[]);
        assert!(report.findings.is_empty(), "{:?}", report.findings);
        assert_eq!(report.by_design.lookups, 3);
        let local = vec![
            // The reason replaced the URL: the title's host still shows a private IP
            failed("2026-11-06T18:30:00.000Z", Some("Request failed with status code 404"), "Opening 10.0.0.5"),
            // A failure with no reason keeps the URL
            failed("2026-11-06T18:30:01.000Z", Some("http://127.0.0.1:8799/api/settings"), "Opening 127.0.0.1"),
        ];
        let report = check(&[reply("2026-11-06T18:30:00.000Z", "claude", local)], &[], &[]);
        let titles: Vec<&str> = report.findings.iter().map(|f| f.title.as_str()).collect();
        assert_eq!(titles, ["Opening 10.0.0.5", "Opening 127.0.0.1"]);
        assert_eq!(report.by_design.lookups, 0);
        // The lenient limit: a failed loopback fetch whose URL was replaced reads as the dev server
        let s = failed("2026-11-06T18:30:00.000Z", Some("fetch failed"), "Opening 127.0.0.1");
        assert_eq!(fetch_address(&s).as_deref(), Some("http://127.0.0.1/"));
    }

    #[test]
    fn a_local_fetch_with_its_row_is_covered() {
        let r = reply(
            "2026-11-06T18:30:00.100Z",
            "claude",
            vec![
                fetch("2026-11-06T18:30:00.100Z", Some("http://127.0.0.1:8799/api/settings"), "Opening 127.0.0.1"),
                fetch("2026-11-06T18:30:09.000Z", Some("http://10.0.0.5/"), "Opening 10.0.0.5"),
            ],
        );
        let rows = vec![
            row("a1", "WebFetch", "allowed_once", "2026-11-06 18:30:00", Some("direct:b1")),
            row("a2", "WebFetch", "auto_allowed", "2026-11-06 18:30:09", Some("direct:b1")),
        ];
        let report = check(&[r], &[], &rows);
        assert!(report.findings.is_empty(), "{:?}", report.findings);
        assert_eq!(report.with_card, 2);
    }

    #[test]
    fn the_fetch_address_comes_from_the_detail_then_the_title() {
        let at = "2026-11-06T18:30:00.000Z";
        assert_eq!(fetch_address(&fetch(at, Some(" http://10.0.0.5:81/x "), "Opening 10.0.0.5")).as_deref(), Some("http://10.0.0.5:81/x"));
        assert_eq!(fetch_address(&fetch(at, None, "Opening 10.0.0.5")).as_deref(), Some("http://10.0.0.5/"));
        assert_eq!(fetch_address(&fetch(at, Some(""), "Opening a page")), None);
        assert_eq!(fetch_address(&fetch(at, None, "Opening not a url")), None);
        assert!(fetch_asks("http://router.home.arpa/") && fetch_asks("http://metadata.google.internal/") && fetch_asks("ftp://example.com/"));
        assert!(!fetch_asks("https://example.local.example.com/"));
    }

    #[cfg(unix)]
    #[test]
    fn a_read_only_command_is_by_design_and_a_writing_one_is_flagged() {
        let r = reply(
            "2026-11-06T18:30:00.000Z",
            "claude",
            vec![
                tool_step("Bash", "2026-11-06T18:30:00.000Z", "Running `ls -la`", Some("ls -la")),
                tool_step("Bash", "2026-11-06T18:30:01.000Z", "Running `rm -rf build`", Some("rm -rf build")),
            ],
        );
        let report = check(&[r], &[], &[]);
        assert_eq!(report.by_design.read_only_commands, 1);
        assert_eq!(report.findings.len(), 1);
        assert!(report.findings[0].title.contains("rm -rf"));
    }

    #[cfg(unix)]
    #[test]
    fn a_find_that_only_looks_counts_as_a_read() {
        assert!(reads_only_for_claude("ls -la /srv/x && find /srv/x -iname '*missing*' 2>/dev/null"));
        assert!(reads_only_for_claude("find /home -iname 'report*' 2>/dev/null | head -20"));
        assert!(reads_only_for_claude("ls -la"));
        for writes in [
            "find . -name '*.tmp' -delete",
            "find . -exec rm {} \\;",
            "find . -name x -fprint out.txt",
            "find . -name x > list.txt",
            "find $(pwd) -name x",
            "find . -name x & rm -rf y",
            "find . -name x && rm -rf y",
            "find . -name 'x",
            "rm -rf build",
        ] {
            assert!(!reads_only_for_claude(writes), "{writes}");
        }
    }

    #[test]
    fn titles_are_redacted_and_cut() {
        let title = format!("Running `curl -H 'Authorization: Bearer sk-{}' {}`", "a".repeat(30), "x".repeat(200));
        let clean = clean_title(&title);
        assert!(!clean.contains(&"a".repeat(30)), "{clean}");
        assert!(clean.chars().count() <= MAX_TITLE);
        assert!(clean.ends_with('…'));
        assert_eq!(clean_title("Running `ls`\n  now"), "Running `ls` now");
    }

    #[test]
    fn since_parses_hours_days_and_all() {
        assert_eq!("24h".parse::<Since>(), Ok(Since::Last(Duration::hours(24))));
        assert_eq!("7d".parse::<Since>(), Ok(Since::Last(Duration::days(7))));
        assert_eq!("ALL".parse::<Since>(), Ok(Since::All));
        for bad in ["", "d", "0d", "-1d", "7w", "1.5d", "99999d", "7é"] {
            assert!(bad.parse::<Since>().is_err(), "{bad}");
        }
    }

    fn shell_row(id: &str, command: &str, status: &str, at: &str) -> Approval {
        let mut r = row(id, "Bash", status, at, Some("direct:b1"));
        r.parameters = json!({"source": "claude_builtin", "tool": "Bash", "arguments": {"command": command}, "cwd": "/work"});
        r
    }

    fn said(at: &str) -> PersonMessage {
        PersonMessage { chat: "direct:b1".into(), at: t(at) }
    }

    #[test]
    fn a_row_s_families_are_27b_s() {
        assert_eq!(row_families(&shell_row("a", "cd x && /usr/bin/gh pr view 3", "denied", "2026-11-06 18:30:00")), vec!["sh:gh pr"]);
        let mut mcp = row("m", "mcp__github__merge_pr", "denied", "2026-11-06 18:30:00", Some("direct:b1"));
        mcp.parameters = json!({"source": "mcp", "server": "github", "tool": "merge_pr", "arguments": {"pr": 1}});
        assert_eq!(row_families(&mcp), vec!["mcp:github/merge_pr"]);
        let mut write = row("w", "Write", "denied", "2026-11-06 18:30:00", Some("direct:b1"));
        write.parameters = json!({"source": "claude_builtin", "tool": "Write", "arguments": {"file_path": "../etc/hosts"}, "cwd": "/work"});
        assert_eq!(row_families(&write), vec!["file:write:/etc/hosts"]);
        // A row saved before 27b with only the arguments still reads by its tool name
        let mut old = row("o", "Bash", "denied", "2026-11-06 18:30:00", Some("direct:b1"));
        old.parameters = json!({"arguments": {"command": "git push"}});
        assert_eq!(row_families(&old), vec!["sh:git push"]);
    }

    #[test]
    fn asked_again_after_a_no_until_the_person_writes() {
        let rows = vec![
            shell_row("no", "gh pr view 12", "denied", "2026-11-06 18:30:26"),
            shell_row("again", "gh pr view 12 --json title", "denied", "2026-11-06 18:33:23"),
            shell_row("after", "gh pr view 12", "allowed_once", "2026-11-06 18:35:00"),
        ];
        let report = check(&[], &[said("2026-11-06 18:30:23"), said("2026-11-06 18:34:56")], &rows);
        assert_eq!(report.nos, 2);
        assert_eq!(report.findings.len(), 1, "{:?}", report.findings);
        let f = &report.findings[0];
        assert_eq!((f.rule, f.at), (Rule::AskedAgainAfterNo, t("2026-11-06 18:33:23")));
        assert_eq!((f.approval_id.as_deref(), f.declined_id.as_deref()), (Some("again"), Some("no")));
        assert_eq!(f.title, "gh pr view 12 --json title");

        // `since` and the bot pick which rows are judged; an earlier no still counts
        let later = check_since(&[], &[said("2026-11-06 18:34:56")], &rows, Some(t("2026-11-06 18:33:00")), None);
        assert_eq!((later.nos, later.findings.len()), (1, 1));
        assert!(check_since(&[], &[], &rows, None, Some("b2")).findings.is_empty());
    }

    #[test]
    fn only_the_person_s_no_on_a_card_counts_and_only_a_call_is_asked() {
        let no = shell_row("no", "git push", "denied", "2026-11-06 18:30:00");
        // Another bot, another chat, another kind: not the same no
        let mut other_bot = shell_row("b", "git push", "allowed_once", "2026-11-06 18:31:00");
        other_bot.bot_id = "b2".into();
        let mut other_chat = shell_row("c", "git push", "allowed_once", "2026-11-06 18:31:00");
        other_chat.chat = Some("thread:t9".into());
        let other_kind = shell_row("k", "git status", "allowed_once", "2026-11-06 18:31:00");
        // kwnt's own refusal (27b, kwnt's secrets): closed, no `resume`
        let mut refused = row("r", "Bash", "denied", "2026-11-06 18:31:00", None);
        refused.parameters = json!({"arguments": {"command": "git push -f"}, "declined_before": "no"});
        let report = check(&[], &[], &[no.clone(), other_bot, other_chat, other_kind, refused.clone()]);
        assert!(report.findings.is_empty(), "{:?}", report.findings);

        // A refused row is no "no" either; a call that ran without asking after a no is flagged
        let mut ran = shell_row("ran", "git push", "auto_allowed", "2026-11-06 18:32:00");
        ran.card = false;
        assert!(check(&[], &[], &[refused.clone(), ran.clone()]).findings.is_empty());
        let report = check(&[], &[], &[no, ran]);
        assert_eq!(report.findings.len(), 1);
        assert_eq!(report.findings[0].approval_id.as_deref(), Some("ran"));
    }

    #[test]
    fn text_says_nothing_to_flag_or_lists_findings() {
        let mut report = check(&[], &[], &[]);
        report.since = Some(t("2026-11-06 18:00:00"));
        let text = render_text(&report);
        assert!(text.contains("Checked 0 replies since 2026-11-06 18:00:00 UTC"), "{text}");
        assert!(text.ends_with("Nothing to flag since 2026-11-06 18:00:00 UTC.\n"), "{text}");
        let r = reply("2026-11-06T18:30:00.000Z", "claude", vec![tool_step("mcp__github__merge", "2026-11-06T18:30:00.000Z", "Merging", None)]);
        let report = check(&[r], &[], &[]);
        let text = render_text(&report);
        assert!(text.contains("Ran with no card (1)\n  2026-11-06 18:30:00 UTC · Forge · chat with Forge · mcp__github__merge · Merging\n"), "{text}");
    }
}
