//! Live activity: what a bot is doing during a turn, as a short list of steps (thinking, a tool
//! run, a wait for an approval or for the person). Each change to a step is broadcast as an
//! `activity` event while the turn runs, and the whole list is kept on the reply's
//! `meta.activity` so it still shows after a reload. See
//! `docs/live-activity.md` for the wire shapes.
//!
//! Titles and details are made for people ("Running `cargo test`", "Opening github.com") and are
//! redacted: known secret values, `key=value` credentials, bearer tokens and provider-looking keys
//! never leave this module.

use super::events::{RuntimeEvent, TurnTag};
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
/// Most thinking text held back from the live stream at once (characters): the last two words of
/// what streamed so far, so a secret split across two chunks is masked once it is whole.
pub const MAX_HELD_THINKING: usize = 256;

/// One thing the bot did, or is doing, in a turn.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct Step {
    /// `s1`, `s2`, … in the order the steps started; unique within the turn.
    pub id: String,
    /// "thinking", "tool", "approval", "human_step" or "note".
    pub kind: &'static str,
    /// "running", "waiting", "ok", "failed", "done", "declined", "stopped", "unanswered" (a
    /// tool call whose approval card nobody answered in time: it didn't run) or "rehearsed" (a
    /// Skill Foundry dry run recorded the call instead of running it).
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
    /// The title of the page a bot browser open/navigate step landed on (see [`opens_page`]),
    /// redacted and cut to [`MAX_TITLE`], once the step ends. Missing for every other step, and
    /// on replies saved before 0.9.8.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub page_title: Option<String>,
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
    /// Thinking text not yet streamed, per scope (the `Scoped` tags around it; none for the turn
    /// itself), sent with that scope's next chunk or when the thinking step ends.
    held: Vec<(Vec<TurnTag>, String)>,
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
            page_title: None,
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
                    let (status, why) = step_end(result);
                    if let Some(why) = why {
                        self.steps[i].detail = Some(cut(&self.redact(&why), MAX_DETAIL));
                    }
                    if status == "ok" && self.steps[i].tool.as_deref().is_some_and(opens_page) {
                        self.steps[i].page_title = result["title"]
                            .as_str()
                            .map(str::trim)
                            .filter(|t| !t.is_empty())
                            .map(|t| cut(&self.redact(t), MAX_TITLE));
                    }
                    changed.push(self.end(i, status));
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

    /// Ends the waiting step for `ref_id` with `status` ("ok", "done", "declined", "stopped", or
    /// "unanswered" for an approval card the turn stopped waiting for, still open for a late answer).
    pub fn settle(&mut self, ref_id: &str, status: &'static str) -> Option<Step> {
        let i = self
            .steps
            .iter()
            .rposition(|s| s.ref_id.as_deref() == Some(ref_id) && s.is_open())?;
        if status == "unanswered" {
            self.steps[i].detail = Some(crate::mcp::UNANSWERED_STEP_DETAIL.to_string());
        }
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

    /// The steps so far as `GET /api/turns` lists them for a running turn: the same shape as
    /// `meta.activity`, but with no thinking `text`. A running thinking step's text isn't redacted
    /// until the step ends, and the text that streamed before a load isn't shown anyway.
    pub fn to_view_json(&self) -> Value {
        let mut v = self.to_json();
        if let Some(steps) = v["steps"].as_array_mut() {
            for step in steps.iter_mut().filter_map(Value::as_object_mut) {
                step.remove("text");
            }
        }
        v
    }

    /// Masks secrets in a driver event before it is broadcast: thinking text, a tool call's
    /// arguments and a finished tool's result (every string in them). Reply text passes as is.
    pub fn redact_event(&self, event: &RuntimeEvent) -> RuntimeEvent {
        match event {
            RuntimeEvent::ThinkingDelta(text) => RuntimeEvent::ThinkingDelta(self.redact(text)),
            RuntimeEvent::ToolCallPending { id, name, args } => {
                RuntimeEvent::ToolCallPending { id: id.clone(), name: name.clone(), args: self.redact_value(args) }
            }
            RuntimeEvent::ToolCallCompleted { id, result } => {
                RuntimeEvent::ToolCallCompleted { id: id.clone(), result: self.redact_value(result) }
            }
            RuntimeEvent::Scoped { tag, event } => {
                RuntimeEvent::Scoped { tag: tag.clone(), event: Box::new(self.redact_event(event)) }
            }
            other => other.clone(),
        }
    }

    /// Masks secrets in every string of a JSON value; a field named like a credential
    /// (`api_key`, `password`, `token`, …) loses its value whole, whatever its type, the way
    /// `api_key=…` does (`true` and `null` stay). A plural one (`tokens`, `secrets`) loses it when
    /// it holds any text, so counts like `max_tokens: 100` stay readable.
    fn redact_value(&self, value: &Value) -> Value {
        match value {
            Value::String(s) => Value::String(self.redact(s)),
            Value::Array(items) => Value::Array(items.iter().map(|v| self.redact_value(v)).collect()),
            Value::Object(map) => Value::Object(
                map.iter()
                    .map(|(k, v)| {
                        let v = match credential_field(k) {
                            // `"token": {"value": …}`, `"password": ["…"]`: the whole value
                            Some(Field::Single) if !matches!(v, Value::Bool(_) | Value::Null) => Value::String("[redacted]".into()),
                            // `"tokens": ["…"]`, but `"max_tokens": 100` and `{"input_tokens": 3}` stay
                            Some(Field::Plural) if has_string(v) => Value::String("[redacted]".into()),
                            _ => self.redact_value(v),
                        };
                        (k.clone(), v)
                    })
                    .collect(),
            ),
            other => other.clone(),
        }
    }

    /// Takes one driver event and returns, in the order to broadcast them: thinking text held
    /// back for the live stream that is due now, the steps the event started or changed, and the
    /// event itself as it may go out (redacted; a thinking chunk without its last two words, which
    /// are held until the next chunk, so a secret split across chunks is masked whole).
    pub fn relay(&mut self, event: &RuntimeEvent) -> (Vec<RuntimeEvent>, Vec<Step>, Vec<RuntimeEvent>) {
        let thinking = matches!(event.unscoped(), RuntimeEvent::ThinkingDelta(_));
        let flushed = if thinking { Vec::new() } else { self.flush_held() };
        let steps = self.observe(event);
        // With no thinking step open (all steps used), a chunk is a status line: sent whole
        let out = if thinking && self.thinking.is_some() {
            self.hold_thinking(Vec::new(), event).into_iter().collect()
        } else {
            vec![self.redact_event(event)]
        };
        (flushed, steps, out)
    }

    /// A thinking chunk joined to what its scope held back, minus the new tail it holds back,
    /// redacted; `None` when all of it is held. `scope` is the chain of `Scoped` tags around it.
    fn hold_thinking(&mut self, scope: Vec<TurnTag>, event: &RuntimeEvent) -> Option<RuntimeEvent> {
        match event {
            RuntimeEvent::Scoped { tag, event } => {
                let mut inner_scope = scope;
                inner_scope.push(tag.clone());
                let inner = self.hold_thinking(inner_scope, event)?;
                Some(RuntimeEvent::Scoped { tag: tag.clone(), event: Box::new(inner) })
            }
            RuntimeEvent::ThinkingDelta(text) => {
                let at = match self.held.iter().position(|(s, _)| *s == scope) {
                    Some(at) => at,
                    None => {
                        self.held.push((scope, String::new()));
                        self.held.len() - 1
                    }
                };
                let buf = &mut self.held[at].1;
                buf.push_str(text);
                let cut = hold_from(buf);
                let ready: String = buf.drain(..cut).collect();
                (!ready.is_empty()).then(|| RuntimeEvent::ThinkingDelta(self.redact(&ready)))
            }
            other => Some(self.redact_event(other)),
        }
    }

    /// Everything held back, redacted, as thinking chunks of their own scope; nothing stays held.
    pub fn flush_held(&mut self) -> Vec<RuntimeEvent> {
        std::mem::take(&mut self.held)
            .into_iter()
            .filter(|(_, text)| !text.is_empty())
            .map(|(scope, text)| {
                scope.into_iter().rev().fold(RuntimeEvent::ThinkingDelta(self.redact(&text)), |event, tag| {
                    RuntimeEvent::Scoped { tag, event: Box::new(event) }
                })
            })
            .collect()
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

/// Byte index where the held-back tail of streamed thinking starts: the last two
/// whitespace-separated words, moved back while the text before it ends in a credential whose
/// value is not all there yet (see [`value_pending`]), so `password: <value>` and
/// `Authorization: Bearer <token>` go out whole; at most [`MAX_HELD_THINKING`] characters.
fn hold_from(text: &str) -> usize {
    let mut at = text.len();
    for _ in 0..2 {
        at = text[..at].trim_end().len();
        at = text[..at].rfind(char::is_whitespace).map_or(0, |i| i + text[i..].chars().next().map_or(1, char::len_utf8));
    }
    while let Some(key) = value_pending(&text[..at], &text[at..]).filter(|&k| k < at) {
        at = key;
    }
    let longest = text.char_indices().rev().nth(MAX_HELD_THINKING.saturating_sub(1)).map_or(0, |(i, _)| i);
    at.max(longest)
}

/// Where the credential starts when `head`, the text about to stream, ends before its value is
/// whole, so the key is held with it. The same pairs [`crate::profiles::classify::mask_credentials`]
/// masks: `password:` or `password =` with the value still to come (`next`, the held text, starts
/// it), `Authorization: Basic` (a scheme), a bare `Bearer` before a word that is not all lowercase
/// (so "Bearer tokens expire" is prose), and `password = "` with the closing quote still to come.
/// A credential word on its own ("the password field") is not held.
fn value_pending(head: &str, next: &str) -> Option<usize> {
    use crate::profiles::classify::is_credential_key;
    let is_key = |word: &str| -> bool {
        // `"password"` and `X-Api-Key` count; the name is the trailing run of name characters
        let word = word.strip_suffix(['"', '\'']).unwrap_or(word);
        let start = word
            .char_indices()
            .rev()
            .find(|&(_, c)| !(c.is_ascii_alphanumeric() || c == '_' || c == '-'))
            .map_or(0, |(i, c)| i + c.len_utf8());
        start < word.len() && is_credential_key(&word[start..].to_ascii_lowercase())
    };
    // the last words of `head` with their byte offsets; a held key is held from its word start
    let trimmed = head.trim_end();
    let mut words = trimmed.split_whitespace().rev().map(|w| (w.as_ptr() as usize - head.as_ptr() as usize, w));
    let next_word = next.split_whitespace().next().unwrap_or("");
    if let Some((at, last)) = words.next() {
        let bare = last.trim_end_matches(['"', '\'']);
        // `password:` / `password="`, or `password` before `= …` / `:…`
        if let Some(sep_free) = bare.strip_suffix([':', '=']) {
            if sep_free.is_empty() {
                if let Some((prev_at, prev)) = words.clone().next() {
                    if is_key(prev) {
                        return Some(prev_at);
                    }
                }
            } else if is_key(sep_free) {
                return Some(at);
            }
        } else if next_word.starts_with([':', '=']) && is_key(last) {
            return Some(at);
        }
        let scheme = last.to_ascii_lowercase();
        if ["bearer", "basic", "digest"].contains(&scheme.as_str()) {
            // `Authorization: Basic` with the credential still to come
            if let Some((prev_at, prev)) = words.clone().next() {
                let key = prev.strip_suffix([':', '=']).unwrap_or("");
                if is_key(key) && key.to_ascii_lowercase().trim_end_matches(['"', '\'']).ends_with("authorization") {
                    return Some(prev_at);
                }
            }
            if scheme == "bearer" && !next_word.is_empty() && !next_word.bytes().all(|b| b.is_ascii_lowercase()) {
                return Some(at);
            }
        }
    }
    // `password = "two words` with its closing quote still to come
    for (q, c) in trimmed.char_indices().rev().filter(|(_, c)| *c == '"' || *c == '\'') {
        if trimmed[q + 1..].contains(c) {
            continue;
        }
        let before = trimmed[..q].trim_end();
        let Some(key) = before.strip_suffix([':', '=']) else { continue };
        let key = key.trim_end();
        let word_at = key.char_indices().rev().find(|&(_, c)| c.is_whitespace()).map_or(0, |(i, c)| i + c.len_utf8());
        if is_key(&key[word_at..]) {
            return Some(word_at);
        }
    }
    None
}

enum Field {
    Single,
    Plural,
}

/// Whether a JSON field is named like a credential: `api_key`, `password`, `token` (`Single`), or
/// a plural of one, `tokens`, `passwords` (`Plural`). `key` and `max_tokens` are neither.
fn credential_field(name: &str) -> Option<Field> {
    let name = name.to_ascii_lowercase();
    if crate::profiles::classify::is_credential_key(&name) {
        return Some(Field::Single);
    }
    name.strip_suffix('s')
        .filter(|stem| crate::profiles::classify::is_credential_key(stem))
        .map(|_| Field::Plural)
}

/// Whether any string is inside a JSON value.
fn has_string(value: &Value) -> bool {
    match value {
        Value::String(_) => true,
        Value::Array(items) => items.iter().any(has_string),
        Value::Object(map) => map.values().any(has_string),
        _ => false,
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
/// Own names (after any `mcp__<server>__` prefix) of the bot browser tools that open or move to a
/// page: kwnt's `browser_open` and `browser_navigate`, and the `navigate` / `open_url` of browser
/// tool servers. The same list the chat's page trail counts (`frontend/src/browserPages.ts`).
/// WebFetch isn't one: it never touches the bot's browser.
pub const PAGE_OPEN_TOOLS: [&str; 4] = ["browser_open", "browser_navigate", "navigate", "open_url"];

/// Whether a tool (`browser_open`, `mcp__kwnt__browser_open`, …) opens a page in the bot's browser.
pub fn opens_page(tool: &str) -> bool {
    PAGE_OPEN_TOOLS.contains(&tool.rsplit("__").next().unwrap_or(tool))
}

/// The page title in the text a page-opening browser tool answered with: the `title` of the page
/// JSON it returns (kwnt's `browser_open`), or of the page after a note ("…\n\nThe page now:\n{…}").
/// Drivers put it on the finished tool call's result as `title`, for [`Step::page_title`].
pub fn page_title_in(text: &str) -> Option<String> {
    let text = text.trim();
    let page = serde_json::from_str::<Value>(text).ok().or_else(|| {
        let at = text.rfind("\n{")?;
        serde_json::from_str::<Value>(&text[at + 1..]).ok()
    })?;
    page["title"].as_str().map(str::trim).filter(|t| !t.is_empty()).map(str::to_string)
}

/// The finished tool call's result with the page title on it (`title`), when the tool opens a
/// page and its answer (`text`) names one. Other results pass as they are.
pub fn with_page_title(mut result: Value, tool: &str, text: &str) -> Value {
    if opens_page(tool) && failed_reason(&result).is_none() {
        if let (Some(title), Some(obj)) = (page_title_in(text), result.as_object_mut()) {
            obj.insert("title".into(), Value::String(title));
        }
    }
    result
}

/// A finished tool call's result: `None` when it went fine, `Some(why)` when its step is
/// `failed` (error, failed, denied or cancelled), with the reason it gave, trimmed, if any. The
/// one rule for "a tool failed", shared with the chat's failure line (`server::tool_failures`).
pub fn failed_reason(result: &Value) -> Option<Option<&str>> {
    if !matches!(
        result["status"].as_str(),
        Some("error" | "failed" | "denied" | "cancelled")
    ) {
        return None;
    }
    Some(
        ["error", "content", "message"]
            .iter()
            .find_map(|k| result[*k].as_str().map(str::trim).filter(|s| !s.is_empty())),
    )
}

/// How a finished tool call's step ends, and the detail it shows (26b). A call nobody approved
/// in time is `unanswered` and a dry-run call `rehearsed`, never "ok": by the result's status
/// (kwnt's own tools) or by the refusal kwnt gave the engine (the claude CLI reports both as an
/// error carrying that text). Otherwise `failed` with its reason, as [`failed_reason`], or "ok".
/// The once-per-turn reminder to the model ([`crate::mcp::NO_FALSE_CLAIMS_TEXT`]) never shows.
pub fn step_end(result: &Value) -> (&'static str, Option<String>) {
    let said = |marker: &str| {
        ["error", "content", "message"]
            .iter()
            .any(|k| result[*k].as_str().is_some_and(|t| t.contains(marker)))
    };
    let first_sentence = |text: &'static str| text.split_inclusive('.').next().unwrap_or(text);
    let status = result["status"].as_str().unwrap_or("");
    let refused = matches!(status, "error" | "failed" | "denied");
    if status == "unanswered" || (refused && said(first_sentence(crate::mcp::UNANSWERED_TEXT))) {
        return (
            "unanswered",
            Some(crate::mcp::UNANSWERED_STEP_DETAIL.to_string()),
        );
    }
    if status == "rehearsed" || (refused && said(first_sentence(crate::mcp::REHEARSED_TEXT))) {
        return (
            "rehearsed",
            Some(crate::mcp::REHEARSED_STEP_DETAIL.to_string()),
        );
    }
    // 27b: refused with no card, because the person already said no to one like it
    if refused && said(first_sentence(crate::mcp::DECLINED_BEFORE_TEXT)) {
        return (
            "declined",
            Some(crate::mcp::DECLINED_BEFORE_STEP_DETAIL.to_string()),
        );
    }
    match failed_reason(result) {
        None => ("ok", None),
        Some(why) => (
            "failed",
            why.map(|w| {
                w.replace(crate::mcp::NO_FALSE_CLAIMS_TEXT, "")
                    .replace(crate::mcp::NOT_ANOTHER_WAY_TEXT, "")
                    .trim()
                    .to_string()
            })
            .filter(|w| !w.is_empty()),
        ),
    }
}

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

/// A path separator on any of kwant's platforms: `/`, or `\` on Windows (21c, 22w).
fn is_path_sep(c: char) -> bool {
    c == '/' || c == '\\'
}

/// The last two parts of a path: enough to recognise the file without the whole folder.
/// Splits on `/` and `\` alike and keeps the path's own separators, so `C:\Users\x\acme\a.md`
/// is `…\acme\a.md` and `/srv/app/src/main.rs` is `…/src/main.rs`, as before (22w). A bare drive
/// (`C:\`) stays as it is.
fn short_path(path: &str) -> String {
    let trimmed = path.trim_end_matches(is_path_sep);
    let is_drive = |s: &str| matches!(s.as_bytes(), [d, b':'] if d.is_ascii_alphabetic());
    if is_drive(trimmed) {
        return path.to_string();
    }
    let Some(last) = trimmed.rfind(is_path_sep) else {
        return trimmed.to_string();
    };
    match trimmed[..last].rfind(is_path_sep) {
        // Separators are one byte, so `before` is a char boundary; the slice keeps the separator.
        Some(before) => format!("…{}", &trimmed[before..]),
        None => trimmed.to_string(),
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

/// Whether `name` is a tool that runs a shell command (`Bash`, codex's `exec_command`,
/// `mcp__x__run_shell_command`, an agent's `run_terminal_command`), ignoring an `mcp__server__`
/// prefix and case. The one list of
/// shell tools: [`summarize`] titles them "Running `…`", and the desktop island names only their
/// program (server/activity.rs).
pub fn is_shell_tool(name: &str) -> bool {
    matches!(
        tool_part(name).to_ascii_lowercase().as_str(),
        "bash"
            | "shell"
            | "exec"
            | "exec_command"
            | "command_execution"
            | "run_terminal_cmd"
            | "run_shell_command"
            | "local_shell"
            // 22j: Quant's failed step (2026-11-15), and the other common names for it
            | "run_terminal_command"
            | "execute_command"
            | "run_command"
    )
}

/// The tools whose argument is the bot's own words (a search pattern, a web query, a helper's
/// brief), by kind. The one list of them: [`summarize`] titles them with the words ("Searching the
/// web for “…”"), and the desktop island shows only the kind ([`island_kind_title`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum WordsTool {
    Files,
    Web,
    Helper,
}

impl WordsTool {
    /// `tool` is the name without its `mcp__server__` prefix, lowercased.
    fn of(tool: &str) -> Option<Self> {
        match tool {
            "glob" | "grep" | "search_files" | "list_dir" | "ls" | "find" => Some(Self::Files),
            "websearch" | "web_search" | "search" => Some(Self::Web),
            "task" | "agent" => Some(Self::Helper),
            _ => None,
        }
    }

    /// The title without the words: what the island shows, and the chat's title when the
    /// argument is missing.
    fn bare(self) -> &'static str {
        match self {
            Self::Files => "Searching files",
            Self::Web => "Searching the web",
            Self::Helper => "Starting a helper",
        }
    }
}

/// The desktop island's title for a search or helper tool (`Grep`, `WebSearch`, `Task`,
/// `mcp__x__grep`), ignoring an `mcp__server__` prefix and case: "Searching files", "Searching the
/// web" or "Starting a helper", never the query or brief (19a). `None` for every other tool.
pub fn island_kind_title(name: &str) -> Option<&'static str> {
    WordsTool::of(&tool_part(name).to_ascii_lowercase()).map(WordsTool::bare)
}

/// The tools that read, edit or write one file, by what they do. The one list of them: [`summarize`]
/// titles them with the file's last two parts ("Reading …/src/main.rs"), and the desktop island
/// with only its name ([`island_file_title`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum FileTool {
    Read,
    Edit,
    Write,
}

impl FileTool {
    /// `tool` is the name without its `mcp__server__` prefix, lowercased.
    fn of(tool: &str) -> Option<Self> {
        match tool {
            "read" | "read_file" | "view" | "cat" => Some(Self::Read),
            "edit" | "multiedit" | "edit_file" | "str_replace" | "str_replace_based_edit_tool" | "apply_patch" | "file_change"
            | "notebookedit" => Some(Self::Edit),
            "write" | "write_file" | "create_file" => Some(Self::Write),
            _ => None,
        }
    }

    /// The title's first word, before the path.
    fn verb(self) -> &'static str {
        match self {
            Self::Read => "Reading",
            Self::Edit => "Editing",
            Self::Write => "Writing",
        }
    }

    /// The title when the call names no path.
    fn bare(self) -> &'static str {
        match self {
            Self::Read => "Reading a file",
            Self::Edit => "Editing files",
            Self::Write => "Writing a file",
        }
    }
}

/// The desktop island's title for a read, edit or write tool (`Read`, `MultiEdit`, `NotebookEdit`,
/// `apply_patch`, `mcp__x__read_file`), ignoring an `mcp__server__` prefix and case: the step's
/// chat `title` with its path cut to the file's own name, so "Reading …/src/main.rs" is "Reading
/// main.rs" and "Editing C:\work\a.md" is "Editing a.md" (21c). Folders can name a client or a
/// person, and the island sits on shared screens. A title without folders ("Reading a file",
/// "Reading main.rs") is kept; one that doesn't start with the tool's verb, or whose path has no
/// name, falls back to the bare title. `None` for every other tool.
pub fn island_file_title(name: &str, title: &str) -> Option<String> {
    let kind = FileTool::of(&tool_part(name).to_ascii_lowercase())?;
    let is_sep = is_path_sep;
    let Some(path) = title.strip_prefix(kind.verb()).and_then(|r| r.strip_prefix(' ')) else {
        return Some(kind.bare().to_string());
    };
    if path.trim().is_empty() {
        return Some(kind.bare().to_string());
    }
    if !path.contains(is_sep) {
        return Some(title.to_string());
    }
    match path.trim_end_matches(is_sep).rsplit(is_sep).next().map(str::trim).filter(|n| !n.is_empty() && *n != "…") {
        Some(file) => Some(format!("{} {file}", kind.verb())),
        None => Some(kind.bare().to_string()),
    }
}

/// A tool's name without its `mcp__server__` prefix (`mcp__x__grep` gives `grep`), as
/// [`summarize`] reads it; `name` itself when it has none.
fn tool_part(name: &str) -> &str {
    name.strip_prefix("mcp__").and_then(|r| r.split_once("__")).map_or(name, |(_, t)| t)
}

/// A short title for a tool call, and the detail to show when the step is opened (the whole
/// command, address or query). Neither is redacted here.
pub fn summarize(name: &str, args: &Value) -> (String, Option<String>) {
    if is_shell_tool(name) {
        return match command_line(args) {
            Some(cmd) => (format!("Running `{}`", cut(&cmd, 80)), Some(cmd)),
            None => ("Running a command".into(), None),
        };
    }
    // `mcp__server__tool`: kwnt's own tools read as built-ins, others name their server
    let (server, tool) = match name.strip_prefix("mcp__").and_then(|r| r.split_once("__")) {
        Some(("kwnt" | "kwant", t)) => (None, t),
        Some((s, t)) => (Some(s), t),
        None => (None, name),
    };
    let lower = tool.to_ascii_lowercase();
    let path = || arg(args, &["file_path", "path", "filePath", "notebook_path", "target_file"]).map(short_path);
    let url = arg(args, &["url", "href", "uri"]);
    if let Some(kind) = WordsTool::of(&lower) {
        let title = match kind {
            WordsTool::Files => arg(args, &["pattern", "query", "glob", "path"]).map(|p| format!("Searching files for `{}`", cut(p, 60))),
            WordsTool::Web => arg(args, &["query", "q"]).map(|q| format!("Searching the web for “{}”", cut(q, 80))),
            WordsTool::Helper => arg(args, &["description", "subagent_type"]).map(|d| format!("Starting a helper: {}", cut(d, 80))),
        };
        return (title.unwrap_or_else(|| kind.bare().to_string()), None);
    }
    if let Some(kind) = FileTool::of(&lower) {
        return (path().map_or(kind.bare().into(), |p| format!("{} {p}", kind.verb())), None);
    }
    match lower.as_str() {
        "webfetch" | "fetch" | "web_fetch" | "browser_open" | "browser_navigate" | "navigate" | "open_url" => match url {
            Some(u) => (format!("Opening {}", host_of(u)), Some(u.to_string())),
            None => ("Opening a page".into(), None),
        },
        "browser_page" | "browser_snapshot" | "browser_read" => ("Reading the browser page".into(), None),
        "browser_screenshot" => ("Looking at the browser page".into(), None),
        "browser_cite" => ("Checking a quote on the browser page".into(), None),
        "browser_act" | "browser_click" | "browser_type" => ("Using the browser".into(), None),
        "ask_user" => ("Asking you a question".into(), None),
        "request_human_step" => ("Asking you to do a step in the browser".into(), arg(args, &["reason"]).map(str::to_string)),
        "todowrite" | "todo_list" | "update_plan" => ("Updating the plan".into(), None),
        _ => {
            let shown = match server {
                Some(s) => format!("Using {tool} ({s})"),
                None => format!("Using {tool}"),
            };
            (shown, home_target(tool, args).or_else(|| url.map(str::to_string)))
        }
    }
}

/// 26f: what a Home Assistant intent tool (`intent__Hass*`, or `homeassistant__*` other than
/// `GetLiveContext`) acts on, as a step's detail: its `name`, else its `area`, else its `domain`
/// (the first one of a list). Plain text, at most 60 characters.
fn home_target(tool: &str, args: &Value) -> Option<String> {
    let intent = tool.starts_with("intent__Hass")
        || tool.strip_prefix("homeassistant__").is_some_and(|t| t != "GetLiveContext");
    if !intent {
        return None;
    }
    fn text(v: &Value) -> Option<&str> {
        v.as_str().or_else(|| v.as_array()?.first()?.as_str()).map(str::trim).filter(|t| !t.is_empty())
    }
    let target = text(&args["name"]).or_else(|| text(&args["area"])).or_else(|| text(&args["domain"]))?;
    let plain: String = target.chars().map(|c| if c.is_control() { ' ' } else { c }).collect();
    Some(plain.chars().take(60).collect())
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

    /// 26f: a Home Assistant step says what it acts on.
    #[test]
    fn home_assistant_steps_name_what_they_act_on() {
        let detail = |tool: &str, args: Value| summarize(&format!("mcp__house__{tool}"), &args).1;
        assert_eq!(detail("intent__HassTurnOff", json!({"name": "Living room lamp"})).as_deref(), Some("Living room lamp"));
        assert_eq!(detail("intent__HassTurnOn", json!({"area": "Kitchen", "domain": ["light"]})).as_deref(), Some("Kitchen"));
        assert_eq!(detail("intent__HassTurnOff", json!({"domain": ["light"]})).as_deref(), Some("light"));
        assert_eq!(detail("homeassistant__GetLiveContext", json!({"name": "x"})), None);
        assert_eq!(detail("intent__HassTurnOff", json!({"name": "a".repeat(80)})).map(|d| d.chars().count()), Some(60));
        assert_eq!(detail("other_tool", json!({"name": "Living room lamp"})), None);
    }

    /// Rigel's B1 on #294: the hold is cut on char boundaries for any script. Every prefix of
    /// mixed CJK, accented Latin, emoji, NBSP and U+3000 text around `key:`, `password = "a b"`
    /// and `Authorization: Bearer X` gets a hold index without a panic, on a char boundary.
    #[test]
    fn hold_from_cuts_multibyte_text_on_char_boundaries() {
        let samples = [
            "Plan: 例: see résumé: then password: FAKEMB1value and 日本語: done",
            "例:\u{a0}password:\u{3000}FAKEMB3 résumé\u{a0}token=😀x 日本語\u{3000}secret: 値",
            "résumé\u{a0}password = \"a b 😀\" then 例\u{3000}password = \"日本 語\" ok",
            "😀 Authorization: Bearer X 例\u{3000}Authorization:\u{a0}Bearer\u{a0}Ünïcode 日本語",
            "\u{3000}é:\u{a0}例= 😀: \u{a0}Bearer 日本 \"例 api_key:é\u{3000}",
        ];
        for text in samples {
            for (k, _) in text.char_indices().chain([(text.len(), ' ')]) {
                let head = &text[..k];
                let at = hold_from(head);
                assert!(at <= head.len() && head.is_char_boundary(at), "{at} in {head:?}");
            }
        }
    }

    /// The documented trade-off: a bare `Bearer` before an all-lowercase word is prose to the
    /// masker ("Bearer tokens expire"), so it is neither held live nor masked when saved, and the
    /// live text still equals the saved text. `Authorization: Bearer` masks any word.
    #[test]
    fn bare_lowercase_bearer_is_prose_live_and_saved() {
        assert_eq!(value_pending("I send Bearer", "abcdefghij now"), None);
        assert_eq!(redact("I send Bearer abcdefghij now", &[]), "I send Bearer abcdefghij now");
        assert!(value_pending("send Authorization: Bearer", "abcdefghij now").is_some());
        assert!(!redact("send Authorization: Bearer abcdefghij now", &[]).contains("abcdefghij"));
    }

    fn pending(id: &str, name: &str, args: Value) -> RuntimeEvent {
        RuntimeEvent::ToolCallPending { id: id.into(), name: name.into(), args }
    }

    fn done(id: &str, status: &str) -> RuntimeEvent {
        RuntimeEvent::ToolCallCompleted { id: id.into(), result: json!({ "status": status }) }
    }

    /// Rigel's N1 on #286: a credential-named field holding an object or a list goes out masked
    /// whole, while `key`, `max_tokens` and flags stay readable.
    #[test]
    fn credential_fields_lose_objects_and_lists_whole() {
        let log = ActivityLog::default();
        let args = json!({
            "token": {"value": "objsecret111"},
            "tokens": ["arrsecret222"],
            "password": ["pw-in-array-333"],
            "auth": {"api_key": {"nested": ["deepsecret444"]}},
            "key": "Enter",
            "max_tokens": 100,
            "usage": {"input_tokens": 3, "output_tokens": 4},
            "secret": true,
            "api_key": null,
        });
        let RuntimeEvent::ToolCallPending { args: out, .. } = log.redact_event(&pending("1", "mcp__x__y", args)) else {
            panic!("not a tool call")
        };
        let wire = out.to_string();
        for secret in ["objsecret111", "arrsecret222", "pw-in-array-333", "deepsecret444"] {
            assert!(!wire.contains(secret), "{secret} in {wire}");
        }
        assert_eq!(out["token"], "[redacted]");
        assert_eq!(out["tokens"], "[redacted]");
        assert_eq!(out["password"], "[redacted]");
        assert_eq!(out["auth"]["api_key"], "[redacted]");
        assert_eq!(out["key"], "Enter");
        assert_eq!(out["max_tokens"], 100);
        assert_eq!(out["usage"], json!({"input_tokens": 3, "output_tokens": 4}));
        assert_eq!(out["secret"], true);
        assert_eq!(out["api_key"], Value::Null);
        assert!(!crate::profiles::classify::is_credential_key("key"));
        assert!(!crate::profiles::classify::is_credential_key("max_tokens"));
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

    /// 26b: a call nobody approved in time didn't run, so its step is `unanswered`, never "ok"
    /// (and not lumped into "failed"), and says so.
    #[test]
    fn an_unanswered_call_is_never_ok() {
        let mut log = ActivityLog::default();
        log.observe(&pending("a", "mcp__ha__turn_on", json!({})));
        let ended = log.observe(&done("a", "unanswered"));
        assert_eq!(ended[0].status, "unanswered");
        assert_eq!(
            ended[0].detail.as_deref(),
            Some("Waited 5 minutes for your answer; nothing ran")
        );
        assert!(crate::mcp::UNANSWERED_STEP_DETAIL
            .contains(&(crate::mcp::APPROVAL_TIMEOUT_SECS / 60).to_string()));
        // The claude CLI reports kwnt's refusal as an error carrying its text (and the reminder)
        log.observe(&pending("b", "mcp__ha__turn_on", json!({})));
        let refusal = crate::mcp::with_no_false_claims(crate::mcp::UNANSWERED_TEXT);
        let ended = log.observe(&RuntimeEvent::ToolCallCompleted {
            id: "b".into(),
            result: json!({"status": "error", "error": refusal}),
        });
        assert_eq!(ended[0].status, "unanswered");
        assert_eq!(
            ended[0].detail.as_deref(),
            Some(crate::mcp::UNANSWERED_STEP_DETAIL)
        );
    }

    /// 26b: a Skill Foundry dry run recorded the call instead of running it: its own status.
    #[test]
    fn a_rehearsed_call_has_its_own_status() {
        let mut log = ActivityLog::default();
        log.observe(&pending("a", "mcp__mail__send", json!({})));
        let ended = log.observe(&done("a", "rehearsed"));
        assert_eq!(
            (ended[0].status, ended[0].detail.as_deref()),
            ("rehearsed", Some(crate::mcp::REHEARSED_STEP_DETAIL))
        );
        log.observe(&pending("b", "Bash", json!({})));
        let ended = log.observe(&RuntimeEvent::ToolCallCompleted {
            id: "b".into(),
            result: json!({"status": "error", "error": crate::mcp::REHEARSED_TEXT}),
        });
        assert_eq!(ended[0].status, "rehearsed");
    }

    /// 26b: the reminder to the model never shows as a step's reason; a denial stays "failed".
    #[test]
    fn the_reminder_to_the_model_stays_out_of_details() {
        let (status, why) = step_end(
            &json!({"status": "error", "content": crate::mcp::with_no_false_claims("boom: 500")}),
        );
        assert_eq!((status, why.as_deref()), ("failed", Some("boom: 500")));
        let (status, why) = step_end(
            &json!({"status": "error", "error": crate::mcp::with_no_false_claims(crate::mcp::DENIED_TEXT)}),
        );
        // 27b: the "don't try another way" sentence is for the bot too
        assert_eq!(
            (status, why.as_deref()),
            ("failed", Some("The user denied this tool call."))
        );
        assert_eq!(
            step_end(&json!({"status": "ok", "content": "fine"})),
            ("ok", None)
        );
        assert_eq!(
            step_end(&json!({"status": "error", "content": crate::mcp::NO_FALSE_CLAIMS_TEXT})),
            ("failed", None)
        );
    }

    /// 27b: a call refused because the person already said no ends `declined`, with the step
    /// detail for the person, however the engine reports it.
    #[test]
    fn a_call_refused_for_an_earlier_no_shows_as_declined() {
        let want = ("declined", Some(crate::mcp::DECLINED_BEFORE_STEP_DETAIL.to_string()));
        // claude CLI: the permission route's deny message comes back as the error
        assert_eq!(step_end(&json!({"status": "error", "error": crate::mcp::DECLINED_BEFORE_TEXT})), want);
        // API engine: kwnt's own record, status `denied`
        assert_eq!(step_end(&json!({"status": "denied", "content": crate::mcp::DECLINED_BEFORE_TEXT})), want);
        assert_eq!(step_end(&json!({"status": "error", "content": crate::mcp::with_no_false_claims(crate::mcp::DECLINED_BEFORE_TEXT)})), want);
        // An ok result quoting it is still ok
        assert_eq!(step_end(&json!({"status": "ok", "content": crate::mcp::DECLINED_BEFORE_TEXT})).0, "ok");
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
        log.wait(
            "approval",
            "ap2",
            "Waiting for your approval: Bash",
            Some("Bash"),
        );
        let late = log.settle("ap2", "unanswered").unwrap();
        assert_eq!(
            (late.status, late.detail.as_deref()),
            ("unanswered", Some(crate::mcp::UNANSWERED_STEP_DETAIL))
        );
        assert!(log.settle("ap1", "ok").is_none(), "settled once");
        log.observe(&pending("t", "Read", json!({})));
        log.wait("human_step", "hs1", "Waiting for you to sign in", None);
        let stopped = log.finish();
        assert_eq!(stopped.iter().map(|s| s.status).collect::<Vec<_>>(), ["stopped", "stopped"]);
    }

    /// 22w: `short_path` keeps the last two parts of a Windows path as it does a Unix one, with the
    /// path's own separators; every `/` path reads exactly as before.
    #[test]
    fn short_path_shortens_windows_paths_too() {
        let cases = [
            // Unix: unchanged from 0.9.16.
            ("/srv/app/src/main.rs", "…/src/main.rs"),
            ("/home/x/clients/acme/contract.md", "…/acme/contract.md"),
            ("/tmp/acme.md", "…/tmp/acme.md"),
            ("/tmp/acme/", "…/tmp/acme"),
            ("src/main.rs", "src/main.rs"),
            ("a//b/c.md", "…/b/c.md"),
            ("a/b//c.md", "…//c.md"),
            ("/a.md", "/a.md"),
            ("contract.md", "contract.md"),
            ("/", ""),
            ("", ""),
            // Windows: drive letter and backslashes.
            ("C:\\Users\\x\\clients\\acme\\contract.md", "…\\acme\\contract.md"),
            ("C:\\work\\a.md", "…\\work\\a.md"),
            ("C:\\a.md", "C:\\a.md"),
            ("C:\\work\\acme\\", "…\\work\\acme"),
            ("work\\a.md", "work\\a.md"),
            // A bare drive stays as typed.
            ("C:\\", "C:\\"),
            ("d:/", "d:/"),
            // UNC.
            ("\\\\server\\share\\clients\\a.md", "…\\clients\\a.md"),
            ("\\\\server\\a.md", "…\\server\\a.md"),
            // Mixed separators keep their own.
            ("C:/work\\acme\\a.md", "…\\acme\\a.md"),
            ("C:/work/acme\\a.md", "…/acme\\a.md"),
            ("C:\\work\\acme/a.md", "…\\acme/a.md"),
            // Non-ASCII names slice on char boundaries.
            ("C:\\Пользователи\\клиент\\договор.md", "…\\клиент\\договор.md"),
        ];
        for (path, want) in cases {
            assert_eq!(short_path(path), want, "{path}");
        }
    }

    #[test]
    fn summaries_are_short_and_human() {
        let cases = [
            ("Bash", json!({"command": "cargo   test\n --all"}), "Running `cargo test --all`"),
            ("command_execution", json!({"command": ["bash", "-lc", "ls -la"]}), "Running `ls -la`"),
            ("command_execution", json!({"command": "bash -lc 'cargo test'"}), "Running `cargo test`"),
            ("Read", json!({"file_path": "/srv/app/src/main.rs"}), "Reading …/src/main.rs"),
            ("Read", json!({"file_path": "C:\\Users\\x\\clients\\acme\\contract.md"}), "Reading …\\acme\\contract.md"),
            ("Edit", json!({"file_path": "main.rs"}), "Editing main.rs"),
            ("WebFetch", json!({"url": "https://www.github.com/x/y"}), "Opening github.com"),
            ("mcp__kwnt__browser_open", json!({"url": "https://example.com/login"}), "Opening example.com"),
            ("WebSearch", json!({"query": "rust sse"}), "Searching the web for “rust sse”"),
            ("WebSearch", json!({}), "Searching the web"),
            ("Grep", json!({"pattern": "TODO"}), "Searching files for `TODO`"),
            ("mcp__x__grep", json!({"pattern": "TODO"}), "Searching files for `TODO`"),
            ("Glob", json!({}), "Searching files"),
            ("Task", json!({"description": "Find the bug"}), "Starting a helper: Find the bug"),
            ("Agent", json!({"subagent_type": "Explore"}), "Starting a helper: Explore"),
            ("Task", json!({}), "Starting a helper"),
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

    /// 21c: the island's title for a read, edit or write tool is the chat's title with the path cut
    /// to the file's own name; the chat's title keeps "…/folder/file". Every other tool has none.
    #[test]
    fn island_file_titles_name_the_file_only() {
        let cases: Vec<(&str, Value, &str, &str)> = vec![
            ("Read", json!({"file_path": "/home/x/clients/acme/contract.md"}), "Reading …/acme/contract.md", "Reading contract.md"),
            ("Edit", json!({"file_path": "/tmp/acme.md"}), "Editing …/tmp/acme.md", "Editing acme.md"),
            ("MultiEdit", json!({"file_path": "/repo/src/main.rs"}), "Editing …/src/main.rs", "Editing main.rs"),
            ("Write", json!({"file_path": "/srv/clients/acme-layoffs/plan.txt"}), "Writing …/acme-layoffs/plan.txt", "Writing plan.txt"),
            ("NotebookEdit", json!({"notebook_path": "/work/acme/model.ipynb"}), "Editing …/acme/model.ipynb", "Editing model.ipynb"),
            ("apply_patch", json!({"path": "/work/acme/a.rs"}), "Editing …/acme/a.rs", "Editing a.rs"),
            ("mcp__x__read_file", json!({"path": "/work/acme/b.md"}), "Reading …/acme/b.md", "Reading b.md"),
            ("Read", json!({"file_path": "C:\\work\\a.md"}), "Reading …\\work\\a.md", "Reading a.md"),
            ("Edit", json!({"file_path": "C:\\Users\\x\\clients\\acme\\contract.md"}), "Editing …\\acme\\contract.md", "Editing contract.md"),
            ("Read", json!({"file_path": "/tmp/acme/"}), "Reading …/tmp/acme", "Reading acme"),
            ("Read", json!({"file_path": "contract.md"}), "Reading contract.md", "Reading contract.md"),
            ("Read", Value::Null, "Reading a file", "Reading a file"),
            ("Edit", Value::Null, "Editing files", "Editing files"),
            ("Write", Value::Null, "Writing a file", "Writing a file"),
        ];
        for (name, args, chat, island) in cases {
            let title = summarize(name, &args).0;
            assert_eq!(title, chat, "{name} {args}");
            assert_eq!(island_file_title(name, &title).as_deref(), Some(island), "{name} {args}");
        }
        // A title that isn't the tool's own falls back to the bare title; a path with no name too
        assert_eq!(island_file_title("Read", "Reading ").as_deref(), Some("Reading a file"));
        assert_eq!(island_file_title("Read", "Reading …/").as_deref(), Some("Reading a file"));
        assert_eq!(island_file_title("Edit", "Looking at /x/acme/y.md").as_deref(), Some("Editing files"));
        for name in ["Bash", "Grep", "WebFetch", "WebSearch", "mcp__linear__list_issues", "mcp__read"] {
            assert_eq!(island_file_title(name, "Reading …/acme/contract.md"), None, "{name}");
        }
    }

    /// 19a: the island's bare title per search or helper kind, ignoring an mcp prefix and case;
    /// every other tool (shell included) has none.
    #[test]
    fn island_kind_titles_name_the_kind_only() {
        for (name, want) in [
            ("Grep", Some("Searching files")),
            ("Glob", Some("Searching files")),
            ("mcp__x__grep", Some("Searching files")),
            ("mcp__kwnt__SEARCH_FILES", Some("Searching files")),
            ("WebSearch", Some("Searching the web")),
            ("web_search", Some("Searching the web")),
            ("Task", Some("Starting a helper")),
            ("mcp__x__agent", Some("Starting a helper")),
            ("Bash", None),
            ("Read", None),
            ("WebFetch", None),
            ("mcp__linear__list_issues", None),
            ("mcp__grep", None),
        ] {
            assert_eq!(island_kind_title(name), want, "{name}");
            // The island's title is the chat's title without its words
            if let Some(bare) = want {
                assert_eq!(summarize(name, &Value::Null).0, bare, "{name}");
            }
        }
    }

    #[test]
    fn terminal_tools_are_shell_tools() {
        for name in ["run_terminal_command", "mcp__desktop__run_terminal_command", "execute_command", "Run_Command", "Bash", "run_terminal_cmd"] {
            assert!(is_shell_tool(name), "{name}");
        }
        for name in ["Read", "run_tests", "mcp__x__run_query", "command"] {
            assert!(!is_shell_tool(name), "{name}");
        }
        let cmd = json!({"command": "journalctl -u maker --since 17:18"});
        assert_eq!(summarize("run_terminal_command", &cmd).0, "Running `journalctl -u maker --since 17:18`");
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

    fn opened(id: &str, result: Value) -> RuntimeEvent {
        RuntimeEvent::ToolCallCompleted { id: id.into(), result }
    }

    /// 0.9.8 browser panel: a bot browser open/navigate step keeps the title of the page it
    /// landed on, under any MCP prefix, so a saved reply's page trail shows titles after a reload.
    #[test]
    fn a_browser_open_step_keeps_the_page_title() {
        for tool in ["browser_open", "mcp__kwnt__browser_open", "mcp__kwnt_tools__browser_open", "browser_navigate", "mcp__playwright__navigate", "open_url"] {
            let mut log = ActivityLog::default();
            log.observe(&pending("b", tool, json!({"url": "http://roastco.example.com/"})));
            assert!(log.steps()[0].page_title.is_none(), "no title while it runs");
            let ended = log.observe(&opened("b", json!({"status": "ok", "title": "  RoastCo · Shop "})));
            assert_eq!(ended[0].page_title.as_deref(), Some("RoastCo · Shop"), "{tool}");
            assert_eq!(log.to_json()["steps"][0]["page_title"], "RoastCo · Shop", "{tool}: saved on the reply");
        }
    }

    #[test]
    fn other_steps_get_no_page_title() {
        let mut log = ActivityLog::default();
        // WebFetch never touches the bot's browser; a GitHub issue's title isn't a page
        for (id, tool) in [("1", "WebFetch"), ("2", "mcp__github__create_issue"), ("3", "mcp__kwnt__browser_page"), ("4", "Bash")] {
            log.observe(&pending(id, tool, json!({"url": "https://example.com"})));
            let ended = log.observe(&opened(id, json!({"status": "ok", "title": "Some title"})));
            assert_eq!(ended[0].page_title, None, "{tool}");
        }
        // An open that failed, or a page with no title (or a blank one), has none either
        for (id, result) in [
            ("5", json!({"status": "error", "error": "net::ERR_NAME_NOT_RESOLVED", "title": "x"})),
            ("6", json!({"status": "ok"})),
            ("7", json!({"status": "ok", "title": "   "})),
            ("8", json!({"status": "ok", "title": 42})),
        ] {
            log.observe(&pending(id, "mcp__kwnt__browser_open", json!({"url": "https://example.com"})));
            assert_eq!(log.observe(&opened(id, result))[0].page_title, None, "step {id}");
        }
        // Old rows and other steps carry no `page_title` key at all
        let saved = log.to_json();
        assert!(saved["steps"].as_array().unwrap().iter().all(|s| s.get("page_title").is_none()), "{saved}");
    }

    #[test]
    fn a_page_title_is_redacted_and_cut() {
        let mut log = ActivityLog::with_secrets(vec!["hunter2-mcp-value".into()]);
        log.observe(&pending("b", "mcp__kwnt__browser_open", json!({"url": "https://example.com"})));
        let title = "Reset for hunter2-mcp-value with ghp_abcdefghijklmnopqrstuvwxyz0123";
        let ended = log.observe(&opened("b", json!({"status": "ok", "title": title})));
        let shown = ended[0].page_title.clone().unwrap();
        assert!(!shown.contains("hunter2-mcp-value") && !shown.contains("ghp_abc"), "{shown}");
        assert!(shown.starts_with("Reset for "), "{shown}");

        log.observe(&pending("c", "browser_navigate", json!({"url": "https://example.com"})));
        let ended = log.observe(&opened("c", json!({"status": "ok", "title": "é".repeat(MAX_TITLE + 40)})));
        let cut = ended[0].page_title.clone().unwrap();
        assert_eq!(cut.chars().count(), MAX_TITLE);
        assert!(cut.ends_with('…'));
    }

    /// The drivers find the title in the page JSON `browser_open` answers with.
    #[test]
    fn drivers_read_the_title_from_the_page_the_tool_answered_with() {
        let page = json!({"url": "http://roastco.example.com/", "title": "RoastCo · Shop", "text": "Prices"}).to_string();
        assert_eq!(page_title_in(&page).as_deref(), Some("RoastCo · Shop"));
        // A CAPTCHA note, then the page the person left
        let noted = format!("Solved by the person.\n\nThe page now:\n{page}");
        assert_eq!(page_title_in(&noted).as_deref(), Some("RoastCo · Shop"));
        for text in ["", "Browser error (HTTP 502).", r#"{"title": ""}"#, r#"{"url": "x"}"#, "[1,2]", "{\"title\": \"cut off"] {
            assert_eq!(page_title_in(text), None, "{text:?}");
        }
        let ok = with_page_title(json!({"status": "ok"}), "mcp__kwnt__browser_open", &page);
        assert_eq!(ok, json!({"status": "ok", "title": "RoastCo · Shop"}));
        // Not a page-opening tool, or the call failed: the result passes as it is
        assert_eq!(with_page_title(json!({"status": "ok"}), "WebFetch", &page), json!({"status": "ok"}));
        let failed = json!({"status": "error", "error": "x"});
        assert_eq!(with_page_title(failed.clone(), "browser_open", &page), failed);
    }
}
