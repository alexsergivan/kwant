//! A running turn's live activity (see [`crate::drivers::activity`]): the turn's events feed its
//! log, and every changed step goes out as a scoped `activity` event. Turns are registered by id
//! so code that only knows the turn id (approval cards, human steps) can add a waiting step.

use crate::drivers::activity::{ActivityLog, Step};
use crate::drivers::events::{RuntimeEvent, TurnTag};
use serde_json::Value;
use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock, Weak};
use tokio::sync::broadcast;

/// The activity of one running turn.
pub struct TurnActivity {
    log: Mutex<ActivityLog>,
    sender: broadcast::Sender<RuntimeEvent>,
    tag: TurnTag,
    /// The id (`s1`, …) of the step for kwnt's own open of the request's link, if it made one
    /// (28g): it carries [`LINK_OPEN_REF`] wherever it goes out.
    link_step: Mutex<Option<String>>,
}

fn turns() -> &'static Mutex<HashMap<String, Weak<TurnActivity>>> {
    static TURNS: OnceLock<Mutex<HashMap<String, Weak<TurnActivity>>>> = OnceLock::new();
    TURNS.get_or_init(Default::default)
}

impl TurnActivity {
    /// Starts the turn's log and registers it under its turn id until it is dropped.
    pub fn start(sender: broadcast::Sender<RuntimeEvent>, tag: TurnTag) -> Arc<Self> {
        let this = Arc::new(Self { log: Mutex::new(ActivityLog::default()), sender, tag, link_step: Mutex::new(None) });
        let mut all = turns().lock().unwrap_or_else(|e| e.into_inner());
        all.retain(|_, w| w.strong_count() > 0);
        all.insert(this.tag.turn_id.clone(), Arc::downgrade(&this));
        this
    }

    /// The running turn with this id, if any.
    pub fn find(turn_id: &str) -> Option<Arc<Self>> {
        turns().lock().unwrap_or_else(|e| e.into_inner()).get(turn_id).and_then(Weak::upgrade)
    }

    fn with_log<T>(&self, f: impl FnOnce(&mut ActivityLog) -> T) -> T {
        f(&mut self.log.lock().unwrap_or_else(|e| e.into_inner()))
    }

    fn send(&self, steps: impl IntoIterator<Item = Step>) {
        let link = self.link_step.lock().unwrap_or_else(|e| e.into_inner()).clone();
        for mut step in steps {
            if link.as_deref() == Some(step.id.as_str()) {
                step.ref_id = Some(LINK_OPEN_REF.to_string());
            }
            let _ = self.sender.send(RuntimeEvent::Scoped {
                tag: self.tag.clone(),
                event: Box::new(RuntimeEvent::Activity { step: step.to_json() }),
            });
        }
    }

    /// Masks these values on every step from now on (the turn's MCP secrets).
    pub fn add_secrets(&self, secrets: Vec<String>) {
        self.with_log(|log| log.add_secrets(secrets));
    }

    /// Masks the turn's secrets, credentials and home folders in text saved for others to read.
    pub fn redact(&self, text: &str) -> String {
        crate::drivers::activity::hide_home_paths(&self.with_log(|log| log.redact(text)))
    }

    /// `event` as it may be broadcast: thinking text, tool arguments and tool results with the
    /// turn's secrets and credentials masked (see [`ActivityLog::redact_event`]).
    pub fn redact_event(&self, event: &RuntimeEvent) -> RuntimeEvent {
        self.with_log(|log| log.redact_event(event))
    }

    fn send_events(&self, events: impl IntoIterator<Item = RuntimeEvent>) {
        for event in events {
            let _ = self.sender.send(RuntimeEvent::Scoped { tag: self.tag.clone(), event: Box::new(event) });
        }
    }

    /// Feeds a driver event to the log and broadcasts it, masked, with the steps it changed: any
    /// thinking text held back is sent before the step it belongs to ends, and a thinking chunk
    /// goes out without its last two words until the next chunk (see [`ActivityLog::relay`]).
    pub fn relay(&self, event: &RuntimeEvent) {
        let (held, steps, out) = self.with_log(|log| log.relay(event));
        self.send_events(held);
        self.send(steps);
        self.send_events(out);
    }

    pub fn observe(&self, event: &RuntimeEvent) {
        let steps = self.with_log(|log| log.observe(event));
        self.send(steps);
    }

    pub fn note(&self, title: &str) {
        let step = self.with_log(|log| log.note(title));
        self.send(step);
    }

    pub fn wait(&self, kind: &'static str, ref_id: &str, title: &str, tool: Option<&str>) {
        let (held, steps) = self.with_log(|log| (log.flush_held(), log.wait(kind, ref_id, title, tool)));
        self.send_events(held);
        self.send(steps);
    }

    pub fn settle(&self, ref_id: &str, status: &'static str) {
        let step = self.with_log(|log| log.settle(ref_id, status));
        self.send(step);
    }

    /// 28g: kwnt starts opening the link in the person's message in the bot browser, before the
    /// turn. The step reads as a `browser_open` ("Opening <host>", the address as its detail) and
    /// carries [`LINK_OPEN_REF`], so code can tell it from the bot's own opens. End it with
    /// [`Self::link_open_end`].
    pub fn link_open_start(&self, url: &str) {
        let (pending, _) = link_open_events(url, &LinkOpen::Opened(String::new()));
        let steps = self.with_log(|log| {
            let steps = log.observe(&pending);
            if let Some(step) = steps.iter().rfind(|s| s.kind == "tool") {
                *self.link_step.lock().unwrap_or_else(|e| e.into_inner()) = Some(step.id.clone());
            }
            steps
        });
        self.send(steps);
    }

    /// Ends the step [`Self::link_open_start`] began: `ok` with the page's title, or `failed` with
    /// the reason (redacted and cut like any failed step's).
    pub fn link_open_end(&self, outcome: &LinkOpen) {
        let (_, completed) = link_open_events("", outcome);
        let steps = self.with_log(|log| log.observe(&completed));
        self.send(steps);
    }

    /// Marks the link-open step in saved or listed steps (`{"steps": [...]}`).
    fn mark_link_step(&self, v: &mut Value) {
        let Some(id) = self.link_step.lock().unwrap_or_else(|e| e.into_inner()).clone() else { return };
        let step = v["steps"].as_array_mut().and_then(|steps| steps.iter_mut().find(|s| s["id"] == id.as_str()));
        if let Some(obj) = step.and_then(Value::as_object_mut) {
            obj.insert("ref_id".into(), Value::String(LINK_OPEN_REF.into()));
        }
    }

    /// Sends any thinking text still held back, ends the turn's open steps and returns what to save on the reply as `meta.activity`
    /// (`None` when nothing happened).
    pub fn finish(&self) -> Option<Value> {
        let (held, steps, saved) = self.with_log(|log| {
            let held = log.flush_held();
            let steps = log.finish();
            (held, steps, (!log.is_empty()).then(|| log.to_json()))
        });
        self.send_events(held);
        self.send(steps);
        saved.map(|mut v| {
            self.mark_link_step(&mut v);
            v
        })
    }
}

/// The `ref_id` of the step for kwnt's own open of the link in the person's message before the
/// turn (13a, 28g). Its tool is plain `browser_open`, so the chat's page trail and the inbox's
/// "what changed" read it as a page open. It is never behind an approval card: kwnt opens the
/// person's own link itself.
pub const LINK_OPEN_REF: &str = "kwnt-link-open";

/// The tool call id kwnt's own link open uses in the turn's log; drivers never send it.
const LINK_OPEN_CALL: &str = "kwnt-link-open";

/// How kwnt's open of the request's link before the turn went (28g).
#[derive(Debug, Clone, PartialEq)]
pub enum LinkOpen {
    /// The page loaded; its title (a bot wall or captcha page loaded too).
    Opened(String),
    /// It didn't: the page couldn't be reached, a site rule refused it, or the browser couldn't
    /// move (held by the person, timed out…). The reason as kwnt has it.
    Failed(String),
}

impl LinkOpen {
    /// What the bot browser's navigate answered. A site rule's refusal is its message; a page that
    /// couldn't be reached is its network reason (`net::ERR_NAME_NOT_RESOLVED`).
    pub fn of(navigated: &Result<crate::sandbox::NavigationResult, crate::sandbox::BrowserError>) -> Self {
        use crate::sandbox::BrowserError;
        match navigated {
            Ok(nav) => Self::Opened(nav.title.clone()),
            Err(BrowserError::Navigation(reason)) => Self::Failed(reason.clone()),
            Err(BrowserError::SiteRule(refusal)) => Self::Failed(refusal.message.clone()),
            Err(other) => Self::Failed(other.to_string()),
        }
    }
}

/// The tool call pair kwnt's own link open feeds the turn's log: a `browser_open` of `url`, and
/// its end (`{"status": "ok", "title"}` or `{"status": "error", "error"}`), as a driver reports
/// the bot's own `browser_open`.
fn link_open_events(url: &str, outcome: &LinkOpen) -> (RuntimeEvent, RuntimeEvent) {
    let pending = RuntimeEvent::ToolCallPending {
        id: LINK_OPEN_CALL.into(),
        name: "browser_open".into(),
        args: serde_json::json!({ "url": url }),
    };
    let result = match outcome {
        LinkOpen::Opened(title) => serde_json::json!({ "status": "ok", "title": title }),
        LinkOpen::Failed(reason) => serde_json::json!({ "status": "error", "error": reason }),
    };
    (pending, RuntimeEvent::ToolCallCompleted { id: LINK_OPEN_CALL.into(), result })
}

/// A turn's saved activity without kwnt's own link-open step ([`LINK_OPEN_REF`]): what the bot's
/// own tools did. Follow-through (13a, 0.9.4) reads this, so a link that didn't open counts only
/// through `page_failed`, exactly as before the step was saved. `None` when no other step is left.
pub fn without_link_open(activity: Option<&Value>) -> Option<Value> {
    let mut activity = activity?.clone();
    if let Some(steps) = activity["steps"].as_array_mut() {
        steps.retain(|s| s["ref_id"] != LINK_OPEN_REF);
        if steps.is_empty() && activity.get("dropped").is_none() {
            return None;
        }
    }
    Some(activity)
}

/// An approval card for `turn_id` is waiting on the person.
pub fn approval_waiting(turn_id: &str, ticket: &str, tool_name: &str) {
    if let Some(turn) = TurnActivity::find(turn_id) {
        let (title, _) = crate::drivers::activity::summarize(tool_name, &Value::Null);
        turn.wait("approval", ticket, &format!("Waiting for your approval: {title}"), Some(tool_name));
    }
}

/// kwnt refused a call to `tool_name` in turn `turn_id` on its own, before the engine heard (28k):
/// that tool's next failed step shows `words` (as [`crate::mcp::PREPUSH_BLOCKED_STEP_DETAIL`]),
/// not the text the bot was told. Nobody pressed Deny, so it never reads as the person's no.
pub fn refused(turn_id: &str, tool_name: &str, words: &'static str) {
    if let Some(turn) = TurnActivity::find(turn_id) {
        turn.with_log(|log| log.refused(tool_name, words));
    }
}

/// An approval card for `turn_id` was answered ("ok" allowed, "declined") or stopped waiting.
pub fn approval_settled(turn_id: &str, ticket: &str, status: &'static str) {
    if let Some(turn) = TurnActivity::find(turn_id) {
        turn.settle(ticket, status);
    }
}

/// A running turn as `GET /api/turns` lists it, with its steps so far under `activity` (as
/// `meta.activity`), so a page that loads or reconnects mid-turn shows what the bot is doing now
/// instead of starting from no steps. Queued turns and turns with no steps yet have none. Thinking
/// text is left out: a running thinking step's text isn't redacted until it ends.
pub fn turn_view(turn: &super::turns::ActiveTurn) -> Value {
    let mut v = serde_json::to_value(turn).unwrap_or(Value::Null);
    let steps = TurnActivity::find(&turn.turn_id).and_then(|t| {
        let mut steps = t.with_log(|log| (!log.is_empty()).then(|| log.to_view_json()))?;
        t.mark_link_step(&mut steps);
        Some(steps)
    });
    if let (Some(steps), Some(obj)) = (steps, v.as_object_mut()) {
        obj.insert("activity".into(), steps);
    }
    v
}

/// What a running turn's steps say it waits on right now (H4).
#[derive(Debug, Default, Clone, PartialEq)]
pub struct TurnWaits {
    /// The newest step waiting for the person: its kind ("approval" or "human_step") and start.
    pub waiting: Option<(&'static str, String)>,
    /// The tool of the newest tool step that still runs.
    pub running_tool: Option<String>,
}

pub fn turn_waits(turn_id: &str) -> TurnWaits {
    let Some(turn) = TurnActivity::find(turn_id) else { return TurnWaits::default() };
    turn.with_log(|log| waits_of(log.steps()))
}

fn waits_of(steps: &[Step]) -> TurnWaits {
    TurnWaits {
        waiting: steps
            .iter()
            .rev()
            .find(|s| matches!(s.kind, "approval" | "human_step") && s.status == "waiting")
            .map(|s| (s.kind, s.started_at.clone())),
        running_tool: steps.iter().rev().find(|s| s.kind == "tool" && s.status == "running").and_then(|s| s.tool.clone()),
    }
}

/// What a running turn is doing now, for a glance (the desktop island's "Working now"): the
/// title of its newest step that still runs or waits, and when that step started. Thinking text is
/// never read (a running thinking step isn't redacted yet), only its kind; titles are the log's own
/// one-line, redacted summaries ("Reading acme.com"). `None` when the turn has no steps yet.
pub fn current_step(turn_id: &str) -> Option<(String, String)> {
    let turn = TurnActivity::find(turn_id)?;
    turn.with_log(|log| step_now(log.steps()))
}

fn step_now(steps: &[Step]) -> Option<(String, String)> {
    let open = |s: &&Step| matches!(s.status, "running" | "waiting");
    if let Some(s) = steps.iter().rev().filter(open).find(|s| s.kind != "thinking") {
        return Some((island_title(s), s.started_at.clone()));
    }
    if let Some(s) = steps.iter().rev().find(open) {
        return Some(("Thinking".to_string(), s.started_at.clone()));
    }
    // Between steps: the last one that ended says where it got to
    steps.iter().rev().find(|s| s.kind != "thinking").map(|s| (island_title(s), s.started_at.clone()))
}

/// A step's title as the island shows it (17a, 19a, 21c). A shell step names only its program
/// ("Running git"), never the command, a search or helper step only its kind ("Searching the
/// web"), never the query or brief, and a read, edit or write step only the file's name ("Reading
/// contract.md"), never its folders: the island sits on the person's screen above every app, shared
/// screens and calls included, and redaction only catches known secret shapes. The chat's activity
/// list keeps the full titles. Every other step keeps its title.
fn island_title(s: &Step) -> String {
    let tool = match s.tool.as_deref() {
        Some(tool) if s.kind == "tool" => tool,
        _ => return s.title.clone(),
    };
    if let Some(bare) = crate::drivers::activity::island_kind_title(tool) {
        return bare.to_string();
    }
    if let Some(title) = crate::drivers::activity::island_file_title(tool, &s.title) {
        return title;
    }
    if !crate::drivers::activity::is_shell_tool(tool) {
        return s.title.clone();
    }
    match shell_program(&s.title) {
        Some(program) => format!("Running {program}"),
        None => "Running a command".to_string(),
    }
}

/// The program a shell step runs, when it's a plain word: the file name of the first word of the
/// command in its "Running `…`" title (`/usr/bin/git status` gives `git`). It reads the title, not
/// `detail`, because a failed step's detail becomes the failure reason. `None` for anything else
/// (`FOO=bar make`, a quote, `$(…)`, an empty or missing command): no further parsing, so `sudo …`
/// is `sudo` and `bash -c …` is `bash`.
fn shell_program(title: &str) -> Option<&str> {
    let command = title.strip_prefix("Running `")?;
    let first = command.split_whitespace().next()?;
    // A one-word command ends at the title's closing backtick
    let first = if command.split_whitespace().nth(1).is_none() { first.strip_suffix('`')? } else { first };
    // Only a plain path: no quote, `$`, `=` or backtick anywhere in it
    if !first.bytes().all(|b| b.is_ascii_alphanumeric() || b"._+-/~".contains(&b)) {
        return None;
    }
    let program = first.rsplit('/').next()?;
    let plain = (1..=24).contains(&program.len())
        && program.bytes().all(|b| b.is_ascii_alphanumeric() || b"._+-".contains(&b))
        && program.bytes().any(|b| b.is_ascii_alphanumeric());
    plain.then_some(program)
}

/// Adds `meta.activity` to a reply's meta.
pub fn with_activity(meta: Option<Value>, activity: Option<Value>) -> Option<Value> {
    let Some(activity) = activity else { return meta };
    let mut meta = meta.unwrap_or_else(|| serde_json::json!({}));
    if meta.is_object() {
        meta["activity"] = activity;
    }
    Some(meta)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn current_step_names_the_running_tool_never_thinking_text() {
        let (tx, _rx) = broadcast::channel(16);
        let tag = TurnTag { turn_id: "turn-step-test".into(), kwant_id: "k".into(), context_id: "c".into(), thread_id: None };
        let turn = TurnActivity::start(tx, tag);
        assert_eq!(current_step("turn-step-test"), None);
        turn.observe(&RuntimeEvent::ToolCallPending { id: "1".into(), name: "Bash".into(), args: json!({"command": "ls"}) });
        let (title, started) = current_step("turn-step-test").unwrap();
        assert!(!title.is_empty() && !started.is_empty(), "{title}");
        approval_waiting("turn-step-test", "ap", "Bash");
        assert!(current_step("turn-step-test").unwrap().0.starts_with("Waiting for your approval"));
        assert_eq!(current_step("no-such-turn"), None);
    }

    /// 17a: the island names a shell step's program, never its command; other steps keep their titles.
    #[test]
    fn current_step_names_a_shell_program_never_the_command() {
        let long_pipeline = format!("grep -rn TODO src | {}sort | uniq -c", "sed -e 's/a/b/' | ".repeat(16));
        assert!(long_pipeline.chars().count() >= 300);
        let shell = |cmd: &str| ("Bash", json!({ "command": cmd }));
        let cases: Vec<((&str, Value), &str)> = vec![
            (shell("curl -H \"Authorization: Bearer abc123def456\" https://api.example.com/v1/x"), "Running curl"),
            (shell("FOO=bar make"), "Running a command"),
            (shell("/usr/local/bin/node x.js"), "Running node"),
            (shell(&long_pipeline), "Running grep"),
            (("mcp__x__run_shell_command", json!({"command": "git status"})), "Running git"),
            (("exec_command", json!({"command": ["bash", "-lc", "cargo test -p kwnt"]})), "Running cargo"),
            (shell("sudo apt install jq"), "Running sudo"),
            (shell("bash -c 'echo hi'"), "Running bash"),
            (shell("ls"), "Running ls"),
            (shell("$(which git) status"), "Running a command"),
            (shell("\"/opt/my tool/run\" --fast"), "Running a command"),
            (shell("`which git` log"), "Running a command"),
            (shell(". ~/.profile"), "Running a command"),
            (shell("a_program_name_that_is_far_too_long_to_show --x"), "Running a command"),
            (shell(""), "Running a command"),
            (("Bash", Value::Null), "Running a command"),
        ];
        let (tx, _rx) = broadcast::channel(64);
        for (n, ((tool, args), want)) in cases.into_iter().enumerate() {
            let id = format!("turn-island-{n}");
            let tag = TurnTag { turn_id: id.clone(), kwant_id: "k".into(), context_id: "c".into(), thread_id: None };
            let turn = TurnActivity::start(tx.clone(), tag);
            turn.observe(&RuntimeEvent::ToolCallPending { id: "1".into(), name: tool.into(), args: args.clone() });
            let (title, _) = current_step(&id).unwrap();
            assert_eq!(title, want, "{tool} {args}");
            assert!(!title.contains('`'), "{title}");
            // The chat's activity list keeps the command
            let chat = turn.with_log(|log| log.steps()[0].clone());
            assert_eq!(chat.title.starts_with("Running `"), chat.detail.is_some(), "{}", chat.title);
        }

        // A failed shell step still names its program between steps, though its detail is now the failure
        let tag = TurnTag { turn_id: "turn-island-failed".into(), kwant_id: "k".into(), context_id: "c".into(), thread_id: None };
        let turn = TurnActivity::start(tx.clone(), tag);
        turn.observe(&RuntimeEvent::ToolCallPending { id: "1".into(), name: "Bash".into(), args: json!({"command": "git push origin main"}) });
        turn.observe(&RuntimeEvent::ToolCallCompleted { id: "1".into(), result: json!({"status": "error", "error": "Permission denied (publickey)"}) });
        assert_eq!(current_step("turn-island-failed").unwrap().0, "Running git");

        // A Reading step names its file (21c) and a thinking step reads as it does today; an approval keeps its title
        let tag = TurnTag { turn_id: "turn-island-other".into(), kwant_id: "k".into(), context_id: "c".into(), thread_id: None };
        let turn = TurnActivity::start(tx, tag);
        let read = json!({"file_path": "/tmp/notes.md"});
        turn.observe(&RuntimeEvent::ToolCallPending { id: "1".into(), name: "Read".into(), args: read.clone() });
        assert_eq!(current_step("turn-island-other").unwrap().0, "Reading notes.md");
        turn.observe(&RuntimeEvent::ToolCallCompleted { id: "1".into(), result: json!({"status": "ok"}) });
        turn.observe(&RuntimeEvent::ThinkingDelta("the key is in ~/.env".into()));
        assert_eq!(current_step("turn-island-other").unwrap().0, "Thinking");
        approval_waiting("turn-island-other", "ap", "Bash");
        assert_eq!(current_step("turn-island-other").unwrap().0, "Waiting for your approval: Running a command");
    }

    /// 19a: the island names a search or helper step's kind, never its query or brief; the chat's
    /// activity list keeps them.
    #[test]
    fn current_step_names_a_search_or_helper_kind_never_its_words() {
        let words = "acme invoice 4471";
        let cases: Vec<(&str, Value, &str)> = vec![
            ("Grep", json!({"pattern": words}), "Searching files"),
            ("Glob", json!({"pattern": format!("**/{words}*")}), "Searching files"),
            ("mcp__x__grep", json!({"query": words}), "Searching files"),
            ("WebSearch", json!({"query": words}), "Searching the web"),
            ("mcp__kwnt__web_search", json!({"q": words}), "Searching the web"),
            ("Task", json!({"description": format!("Find {words} in the mail")}), "Starting a helper"),
            ("Agent", json!({"subagent_type": words}), "Starting a helper"),
            ("WebSearch", Value::Null, "Searching the web"),
        ];
        let (tx, _rx) = broadcast::channel(64);
        for (n, (tool, args, want)) in cases.into_iter().enumerate() {
            let id = format!("turn-island-words-{n}");
            let tag = TurnTag { turn_id: id.clone(), kwant_id: "k".into(), context_id: "c".into(), thread_id: None };
            let turn = TurnActivity::start(tx.clone(), tag);
            turn.observe(&RuntimeEvent::ToolCallPending { id: "1".into(), name: tool.into(), args: args.clone() });
            assert_eq!(current_step(&id).unwrap().0, want, "{tool} {args}");
            // The chat's activity list keeps the words, byte for byte
            let chat = turn.with_log(|log| log.steps()[0].clone());
            assert_eq!(chat.title, crate::drivers::activity::summarize(tool, &args).0);
            assert_eq!(chat.title.contains(words), !args.is_null(), "{}", chat.title);
            // Between steps, the finished step still names only its kind
            turn.observe(&RuntimeEvent::ToolCallCompleted { id: "1".into(), result: json!({"status": "ok"}) });
            let (title, _) = current_step(&id).unwrap();
            assert_eq!(title, want);
            assert!(!title.contains("acme") && !title.contains("4471"), "{title}");
        }

        // An approval for a search keeps its (already bare) title; a Reading step names its file (21c)
        let tag = TurnTag { turn_id: "turn-island-words-other".into(), kwant_id: "k".into(), context_id: "c".into(), thread_id: None };
        let turn = TurnActivity::start(tx, tag);
        approval_waiting("turn-island-words-other", "ap", "WebSearch");
        assert_eq!(current_step("turn-island-words-other").unwrap().0, "Waiting for your approval: Searching the web");
        approval_settled("turn-island-words-other", "ap", "ok");
        let read = json!({"file_path": "/tmp/acme.md"});
        turn.observe(&RuntimeEvent::ToolCallPending { id: "2".into(), name: "Read".into(), args: read.clone() });
        assert_eq!(current_step("turn-island-words-other").unwrap().0, "Reading acme.md");
    }

    /// 21c: the island names a read, edit or write step's file, never its folders; the chat's
    /// activity list keeps "…/folder/file".
    #[test]
    fn current_step_names_a_file_never_its_folders() {
        let cases: Vec<(&str, Value, &str)> = vec![
            ("Read", json!({"file_path": "/srv/clients/acme-layoffs/contract.md"}), "Reading contract.md"),
            ("Edit", json!({"file_path": "/tmp/acme-layoffs/acme.md", "old_string": "a", "new_string": "b"}), "Editing acme.md"),
            ("MultiEdit", json!({"file_path": "/srv/acme-layoffs/src/main.rs", "edits": []}), "Editing main.rs"),
            ("Write", json!({"file_path": "/srv/acme-layoffs/notes.txt", "content": "x"}), "Writing notes.txt"),
            ("NotebookEdit", json!({"notebook_path": "/srv/acme-layoffs/model.ipynb"}), "Editing model.ipynb"),
            ("Read", json!({"file_path": "C:\\clients\\acme-layoffs\\a.md"}), "Reading a.md"),
            ("Read", json!({"file_path": "/srv/clients/acme-layoffs/"}), "Reading acme-layoffs"),
            ("Read", json!({"file_path": "contract.md"}), "Reading contract.md"),
            ("Read", Value::Null, "Reading a file"),
        ];
        let (tx, _rx) = broadcast::channel(64);
        for (n, (tool, args, want)) in cases.into_iter().enumerate() {
            let id = format!("turn-island-file-{n}");
            let tag = TurnTag { turn_id: id.clone(), kwant_id: "k".into(), context_id: "c".into(), thread_id: None };
            let turn = TurnActivity::start(tx.clone(), tag);
            turn.observe(&RuntimeEvent::ToolCallPending { id: "1".into(), name: tool.into(), args: args.clone() });
            assert_eq!(current_step(&id).unwrap().0, want, "{tool} {args}");
            // The chat's activity list keeps its "…/folder/file" title, byte for byte
            let chat = turn.with_log(|log| log.steps()[0].clone());
            assert_eq!(chat.title, crate::drivers::activity::summarize(tool, &args).0);
            // Between steps, the finished step still names only its file
            turn.observe(&RuntimeEvent::ToolCallCompleted { id: "1".into(), result: json!({"status": "ok"}) });
            let (title, _) = current_step(&id).unwrap();
            assert_eq!(title, want);
            assert!(!title.contains('/') && !title.contains('\\') && !title.contains("clients"), "{title}");
            if want != "Reading acme-layoffs" {
                assert!(!title.contains("acme-layoffs"), "{title}");
            }
        }
    }

    #[test]
    fn turn_waits_reads_the_waiting_step_and_the_running_tool() {
        let (tx, _rx) = broadcast::channel(16);
        let tag = TurnTag { turn_id: "turn-waits-test".into(), kwant_id: "k".into(), context_id: "c".into(), thread_id: None };
        let turn = TurnActivity::start(tx, tag);
        assert_eq!(turn_waits("turn-waits-test"), TurnWaits::default());
        turn.observe(&RuntimeEvent::ToolCallPending { id: "1".into(), name: "Bash".into(), args: json!({"command": "ls"}) });
        assert_eq!(turn_waits("turn-waits-test").running_tool.as_deref(), Some("Bash"));
        approval_waiting("turn-waits-test", "ap", "Bash");
        assert_eq!(turn_waits("turn-waits-test").waiting.map(|w| w.0), Some("approval"));
        approval_settled("turn-waits-test", "ap", "ok");
        assert_eq!(turn_waits("turn-waits-test").waiting, None);
        assert_eq!(turn_waits("no-such-turn"), TurnWaits::default());
    }

    #[test]
    fn steps_go_out_scoped_and_approvals_find_their_turn() {
        let (tx, mut rx) = broadcast::channel(16);
        let tag = TurnTag { turn_id: "turn-activity-test".into(), kwant_id: "k".into(), context_id: "c".into(), thread_id: None };
        let turn = TurnActivity::start(tx, tag);
        turn.observe(&RuntimeEvent::ToolCallPending { id: "1".into(), name: "Bash".into(), args: json!({"command": "ls"}) });
        approval_waiting("turn-activity-test", "ap", "Bash");
        approval_settled("turn-activity-test", "ap", "ok");
        approval_waiting("another-turn", "ap2", "Bash");
        let wire: Vec<Value> = std::iter::from_fn(|| rx.try_recv().ok()).map(|e| e.to_wire()).collect();
        assert_eq!(wire.len(), 3);
        assert_eq!(wire[0]["type"], "activity");
        assert_eq!(wire[0]["turn_id"], "turn-activity-test");
        assert_eq!(wire[0]["data"]["step"]["title"], "Running `ls`");
        assert_eq!(wire[1]["data"]["step"]["status"], "waiting");
        assert_eq!(wire[2]["data"]["step"]["status"], "ok");
        let saved = turn.finish().unwrap();
        assert_eq!(saved["steps"][0]["status"], "stopped");
        drop(turn);
        assert!(TurnActivity::find("turn-activity-test").is_none());
        assert_eq!(with_activity(None, Some(json!({"steps": []}))).unwrap()["activity"]["steps"], json!([]));
    }

    /// 28k: kwnt's pre-push gate notes its refusal on the turn by id, so the tool step it held is
    /// saved as "Blocked before push; nothing ran". A turn that isn't running is left alone.
    #[test]
    fn a_held_push_is_saved_as_blocked_before_push() {
        let (tx, _rx) = broadcast::channel(16);
        let tag = TurnTag { turn_id: "turn-prepush-held".into(), kwant_id: "k".into(), context_id: "c".into(), thread_id: None };
        let turn = TurnActivity::start(tx, tag);
        turn.observe(&RuntimeEvent::ToolCallPending { id: "1".into(), name: "Bash".into(), args: json!({"command": "git push origin lin-x"}) });
        refused("turn-prepush-held", "Bash", crate::mcp::PREPUSH_BLOCKED_STEP_DETAIL);
        refused("no-such-turn", "Bash", crate::mcp::PREPUSH_BLOCKED_STEP_DETAIL);
        turn.observe(&RuntimeEvent::ToolCallCompleted {
            id: "1".into(),
            result: json!({"status": "error", "error": "kwant checks one branch at a time before a push: push the branch by name."}),
        });
        let saved = turn.finish().unwrap();
        assert_eq!(saved["steps"][0]["status"], "failed");
        assert_eq!(saved["steps"][0]["detail"], "Blocked before push; nothing ran");
    }

    /// 0.9.8 browser panel: the page title of a bot browser open goes out live, is listed on
    /// `GET /api/turns` and is kept on the saved reply; a reply saved before it reads as it did.
    #[test]
    fn a_browser_steps_page_title_streams_and_is_saved() {
        let (tx, mut rx) = broadcast::channel(16);
        let tag = TurnTag { turn_id: "turn-page-title".into(), kwant_id: "k".into(), context_id: "c".into(), thread_id: None };
        let turn = TurnActivity::start(tx, tag);
        turn.observe(&RuntimeEvent::ToolCallPending { id: "1".into(), name: "mcp__kwnt_tools__browser_open".into(), args: json!({"url": "http://roastco.example.com/"}) });
        turn.observe(&RuntimeEvent::ToolCallCompleted { id: "1".into(), result: json!({"status": "ok", "title": "RoastCo · Shop"}) });
        let wire: Vec<Value> = std::iter::from_fn(|| rx.try_recv().ok()).map(|e| e.to_wire()).collect();
        assert_eq!(wire.last().unwrap()["data"]["step"]["page_title"], "RoastCo · Shop");
        let saved = turn.finish().unwrap();
        let meta = with_activity(Some(json!({"model": "m"})), Some(saved)).unwrap();
        // what the reply's meta holds in the DB and what /api sends back after a reload
        let reloaded: Value = serde_json::from_str(&meta.to_string()).unwrap();
        assert_eq!(reloaded["activity"]["steps"][0]["page_title"], "RoastCo · Shop");
        assert_eq!(reloaded["activity"]["steps"][0]["detail"], "http://roastco.example.com/");
        drop(turn);
        // An old row, saved before the field existed, still reads the same
        let old = json!({"activity": {"steps": [{"id": "s1", "kind": "tool", "status": "ok", "title": "Opening example.com", "tool": "browser_open", "started_at": "2026-11-01T10:00:00.000Z"}]}});
        assert_eq!(with_activity(Some(old.clone()), None), Some(old.clone()));
        assert!(old["activity"]["steps"][0].get("page_title").is_none());
    }

    /// Ticket 4: a page that loads mid-turn gets the open step from `GET /api/turns`.
    #[test]
    fn listed_turn_carries_its_steps_so_far() {
        let active = |turn_id: &str| super::super::turns::ActiveTurn {
            turn_id: turn_id.into(),
            kwant_id: "k".into(),
            context_id: "c".into(),
            thread_id: None,
            state: "running".into(),
            since: chrono::Utc::now(),
            purpose: None,
            audit: None,
        };
        let tag = TurnTag { turn_id: "turn-view-test".into(), kwant_id: "k".into(), context_id: "c".into(), thread_id: None };
        let turn = TurnActivity::start(broadcast::channel(16).0, tag);
        assert!(turn_view(&active("turn-view-test")).get("activity").is_none(), "no steps yet: no activity");
        turn.observe(&RuntimeEvent::ToolCallPending { id: "1".into(), name: "Bash".into(), args: json!({"command": "ls"}) });
        let v = turn_view(&active("turn-view-test"));
        assert_eq!(v["turn_id"], "turn-view-test");
        assert_eq!(v["state"], "running");
        assert_eq!(v["activity"]["steps"][0]["title"], "Running `ls`");
        assert_eq!(v["activity"]["steps"][0]["status"], "running");
        assert!(turn_view(&active("some-other-turn")).get("activity").is_none());
        drop(turn);
    }

    /// Rigel's N1 on #284: a running turn's `GET /api/turns` view carries no thinking text and no
    /// secret from a tool call's arguments, and the live stream masks them too.
    #[test]
    fn listed_turn_has_no_thinking_text_or_raw_secrets() {
        let active = super::super::turns::ActiveTurn {
            turn_id: "turn-view-redact".into(),
            kwant_id: "k".into(),
            context_id: "c".into(),
            thread_id: None,
            state: "running".into(),
            since: chrono::Utc::now(),
            purpose: None,
            audit: None,
        };
        let token = "ghp_abcdefghijklmnopqrstuvwxyz0123456789";
        let (tx, mut rx) = broadcast::channel(16);
        let tag = TurnTag { turn_id: "turn-view-redact".into(), kwant_id: "k".into(), context_id: "c".into(), thread_id: None };
        let turn = TurnActivity::start(tx, tag);
        turn.add_secrets(vec!["hunter2-mcp-value".into()]);
        let thinking = RuntimeEvent::ThinkingDelta(format!("I'll use {token} and hunter2-mcp-value"));
        turn.observe(&thinking);
        let v = turn_view(&active);
        let step = &v["activity"]["steps"][0];
        assert_eq!((step["kind"].as_str(), step["status"].as_str(), step["title"].as_str()), (Some("thinking"), Some("running"), Some("Thinking")));
        assert!(step.get("text").is_none(), "no thinking text on /api/turns: {step}");

        let call = RuntimeEvent::ToolCallPending {
            id: "1".into(),
            name: "Bash".into(),
            args: json!({"command": format!("curl -H 'Authorization: Bearer {token}' https://x.dev"), "api_key": "plainvalue123", "env": ["PASSWORD=hunter2-mcp-value"]}),
        };
        turn.observe(&call);
        let v = turn_view(&active).to_string();
        for secret in [token, "hunter2-mcp-value", "plainvalue123"] {
            assert!(!v.contains(secret), "{secret} in {v}");
        }
        assert!(v.contains("\"title\":\"Running `curl"), "the tool step still reads right: {v}");
        assert!(!v.contains("I'll use"), "thinking that ended on the tool call stays out too: {v}");

        // The live stream: thinking, tool args and tool results go out masked
        for event in [&thinking, &call, &RuntimeEvent::ToolCallCompleted { id: "1".into(), result: json!({"status": "ok", "content": format!("token={token}")}) }] {
            let wire = turn.redact_event(event).to_wire().to_string();
            for secret in [token, "hunter2-mcp-value", "plainvalue123"] {
                assert!(!wire.contains(secret), "{secret} streamed live: {wire}");
            }
        }
        assert_eq!(turn.redact_event(&RuntimeEvent::TextDelta("reply".into())), RuntimeEvent::TextDelta("reply".into()));
        while rx.try_recv().is_ok() {}
        drop(turn);
    }

    /// Rigel's N2 on #286: a secret split across two thinking chunks never streams raw, not even
    /// its tail, and the text held back is all sent when the step ends, for the turn and for a
    /// delegated turn alike, and when the turn ends while thinking.
    #[test]
    fn split_thinking_secrets_never_stream_raw() {
        let (tx, mut rx) = broadcast::channel(64);
        let tag = TurnTag { turn_id: "turn-split-redact".into(), kwant_id: "k".into(), context_id: "c".into(), thread_id: None };
        let turn = TurnActivity::start(tx, tag);
        turn.add_secrets(vec!["hunter2-QA-mcp-value".into()]);
        let sub = TurnTag { turn_id: "turn-split-sub".into(), ..Default::default() };
        let scoped = |e: RuntimeEvent| RuntimeEvent::Scoped { tag: sub.clone(), event: Box::new(e) };
        let chunks = [
            "key one: ghp_FAKEFAKEFAKE",
            "FAKEFAKE0123456789abcd and the MCP one is hunter2-",
            "QA-mcp-value, done.",
        ];
        for c in chunks {
            turn.relay(&RuntimeEvent::ThinkingDelta(c.into()));
        }
        for c in chunks {
            turn.relay(&scoped(RuntimeEvent::ThinkingDelta(c.into())));
        }
        turn.relay(&RuntimeEvent::ToolCallPending { id: "1".into(), name: "Bash".into(), args: json!({"command": "ls"}) });
        // a second thinking step the turn ends in the middle of
        turn.relay(&RuntimeEvent::ThinkingDelta("then hunter2-QA".into()));
        turn.relay(&RuntimeEvent::ThinkingDelta("-mcp-value end".into()));
        let saved = turn.finish().unwrap();

        let events: Vec<RuntimeEvent> = std::iter::from_fn(|| rx.try_recv().ok()).collect();
        let wire: Vec<Value> = events.iter().map(RuntimeEvent::to_wire).collect();
        // what streamed as thinking for the turn itself (`None`) or for a delegated turn
        let streamed = |inner: Option<&str>| -> String {
            events
                .iter()
                .filter_map(|e| match e {
                    RuntimeEvent::Scoped { event, .. } => match (event.as_ref(), inner) {
                        (RuntimeEvent::ThinkingDelta(t), None) => Some(t.clone()),
                        (RuntimeEvent::Scoped { tag, event }, Some(id)) if tag.turn_id == id => match event.as_ref() {
                            RuntimeEvent::ThinkingDelta(t) => Some(t.clone()),
                            _ => None,
                        },
                        _ => None,
                    },
                    _ => None,
                })
                .collect()
        };
        let all = serde_json::to_string(&wire).unwrap();
        for raw in ["FAKEFAKE0123456789abcd", "ghp_FAKE", "hunter2-", "QA-mcp-value", "hunter2-QA"] {
            assert!(!all.contains(raw), "{raw} streamed live: {all}");
        }
        // nothing is lost: the turn's and the delegated turn's text each arrive whole, masked
        let whole = "key one: [redacted] and the MCP one is ********, done.";
        assert_eq!(streamed(Some("turn-split-sub")), whole);
        assert_eq!(streamed(None), format!("{whole}then ******** end"));
        // the held text goes out before the thinking step's end, not after it
        let first_end = wire.iter().position(|w| w["type"] == "activity" && w["data"]["step"]["kind"] == "thinking" && w["data"]["step"]["status"] == "ok").unwrap();
        let last_chunk = wire.iter().rposition(|w| w["type"] == "ThinkingDelta" && w["data"].as_str().is_some_and(|d| d.contains("done."))).unwrap();
        assert!(last_chunk < first_end, "held text sent after the step ended: {all}");
        assert!(!saved.to_string().contains("hunter2"));
        drop(turn);
    }

    /// Every thinking chunk the turn streamed live (its own, not delegated), in order.
    fn live_thinking(events: &[RuntimeEvent]) -> Vec<String> {
        events
            .iter()
            .filter_map(|e| match e {
                RuntimeEvent::Scoped { event, .. } => match event.as_ref() {
                    RuntimeEvent::ThinkingDelta(t) => Some(t.clone()),
                    _ => None,
                },
                _ => None,
            })
            .collect()
    }

    /// Streams `chunks` as one thinking step that a tool call ends; returns the live thinking
    /// chunks and, after each input chunk, how much of the text had streamed.
    fn stream_thinking(chunks: &[&str]) -> (Vec<String>, Vec<String>) {
        let (tx, mut rx) = broadcast::channel(8192);
        let tag = TurnTag { turn_id: "turn-n3".into(), kwant_id: "k".into(), context_id: "c".into(), thread_id: None };
        let turn = TurnActivity::start(tx, tag);
        let mut events = Vec::new();
        let mut so_far = Vec::new();
        for c in chunks {
            turn.relay(&RuntimeEvent::ThinkingDelta((*c).into()));
            events.extend(std::iter::from_fn(|| rx.try_recv().ok()));
            so_far.push(live_thinking(&events).concat());
        }
        turn.relay(&RuntimeEvent::ToolCallPending { id: "1".into(), name: "Bash".into(), args: json!({"command": "ls"}) });
        let _ = turn.finish();
        events.extend(std::iter::from_fn(|| rx.try_recv().ok()));
        drop(turn);
        (live_thinking(&events), so_far)
    }

    /// Rigel's N3 on #288: `key: value` credentials with a space between the key and the value
    /// never stream raw, wherever the thinking text is cut: the key is held until its value is
    /// whole (a quoted one until its closing quote), then both go out masked.
    #[test]
    fn spaced_credentials_never_stream_raw() {
        let lines = [
            ("password: FAKEPW1secretvalue", ["FAKEPW1", "secretvalue"]),
            ("Authorization: Basic dXNlcjpwYXNzFAKEBASIC1", ["dXNl", "FAKEBASIC1"]),
            ("Authorization: Bearer FAKEBEARER1234567890abcdef", ["FAKEBEARER", "567890abcdef"]),
            ("password = \"FAKEPW2 with spaces inside\"", ["FAKEPW2", "spaces inside"]),
            // B1 on #294: multibyte names and words before the key
            ("例: résumé: password: FAKEMB2value 日本語", ["FAKEMB2", "MB2value"]),
        ];
        for (line, raws) in lines {
            let text = format!("First I check the setup. Then I send {line} to the server and wait for it.");
            let masked = crate::drivers::activity::redact(&text, &[]);
            assert!(raws.iter().all(|r| !masked.contains(r)), "not masked as a whole: {masked}");
            // cuts at every space and right after it, so at every word boundary and after
            // `:`, `=`, `Bearer` and `Basic`
            let mut cuts: Vec<usize> = text.match_indices(' ').flat_map(|(i, _)| [i, i + 1]).collect();
            cuts.dedup();
            let mut ways: Vec<Vec<String>> = cuts.iter().map(|&k| vec![text[..k].to_string(), text[k..].to_string()]).collect();
            // and all at once: one word, one character and three characters per chunk
            ways.push(text.split_inclusive(' ').map(String::from).collect());
            ways.push(text.chars().map(String::from).collect());
            ways.push(text.chars().collect::<Vec<_>>().chunks(3).map(|c| c.iter().collect()).collect());
            for chunks in ways {
                let chunks: Vec<&str> = chunks.iter().map(String::as_str).collect();
                let (live, _) = stream_thinking(&chunks);
                for delta in &live {
                    for raw in raws {
                        assert!(!delta.contains(raw), "{raw} streamed live in {delta:?}, chunks {chunks:?}");
                    }
                }
                // nothing lost, nothing twice: the live text is the masked text
                assert_eq!(live.concat(), masked, "chunks {chunks:?}");
            }
        }
    }

    /// Prose that only names a credential ("the password field", "Bearer tokens") is held back
    /// no more than the usual last two words.
    #[test]
    fn credential_words_in_prose_are_not_held_longer() {
        let text = "the password field is empty, Bearer tokens expire and my token was old so the secret key rotates now ";
        let words: Vec<&str> = text.split_inclusive(' ').collect();
        let (live, so_far) = stream_thinking(&words);
        for (i, streamed) in so_far.iter().enumerate() {
            let expected: String = words[..(i + 1).saturating_sub(2)].concat();
            assert_eq!(streamed, &expected, "after word {i} ({:?})", words[i]);
        }
        assert_eq!(live.concat(), text);
    }


    // ---- 28g: kwnt's own open of the request's link is a step on the reply ----

    fn link_turn(id: &str) -> (Arc<TurnActivity>, broadcast::Receiver<RuntimeEvent>) {
        let (tx, rx) = broadcast::channel(64);
        let tag = TurnTag { turn_id: id.into(), kwant_id: "k".into(), context_id: "c".into(), thread_id: None };
        (TurnActivity::start(tx, tag), rx)
    }

    /// The link open as the pre-turn block records it, from what navigate answered.
    fn open_link(turn: &TurnActivity, url: &str, navigated: Result<crate::sandbox::NavigationResult, crate::sandbox::BrowserError>) {
        turn.link_open_start(url);
        turn.link_open_end(&LinkOpen::of(&navigated));
    }

    fn unresolved() -> Result<crate::sandbox::NavigationResult, crate::sandbox::BrowserError> {
        Err(crate::sandbox::BrowserError::Navigation("net::ERR_NAME_NOT_RESOLVED".into()))
    }

    /// The steps that went out live, as the chat sees them.
    fn live_steps(rx: &mut broadcast::Receiver<RuntimeEvent>) -> Vec<Value> {
        let mut out = Vec::new();
        while let Ok(RuntimeEvent::Scoped { event, .. }) = rx.try_recv() {
            if let RuntimeEvent::Activity { step } = *event {
                out.push(step);
            }
        }
        out
    }

    fn bot_tool(turn: &TurnActivity, id: &str, name: &str, args: Value, result: Value) {
        turn.observe(&RuntimeEvent::ToolCallPending { id: id.into(), name: name.into(), args });
        turn.observe(&RuntimeEvent::ToolCallCompleted { id: id.into(), result });
    }

    /// Pebble's trial reply (compass-trial row F): the link didn't resolve, and the reply's
    /// activity now says so, live and saved, marked as kwnt's own open.
    #[test]
    fn a_link_that_does_not_resolve_is_one_failed_browser_open_step() {
        let (turn, mut rx) = link_turn("turn-link-unresolved");
        open_link(&turn, "https://tickets.example.invalid/buy", unresolved());
        let saved = turn.finish().expect("the reply has activity");
        let steps = saved["steps"].as_array().unwrap();
        assert_eq!(steps.len(), 1, "{saved}");
        let s = &steps[0];
        assert_eq!(s["kind"], "tool");
        assert_eq!(s["tool"], "browser_open");
        assert_eq!(s["title"], "Opening tickets.example.invalid");
        assert_eq!(s["status"], "failed");
        assert!(s["detail"].as_str().unwrap().contains("ERR_NAME_NOT_RESOLVED"), "{s}");
        assert_eq!(s["ref_id"], LINK_OPEN_REF, "marked as kwnt's own open: {s}");
        assert!(s.get("page_title").is_none(), "{s}");
        // Live: it ran, then failed, both marked
        let live = live_steps(&mut rx);
        let statuses: Vec<_> = live.iter().map(|s| s["status"].as_str().unwrap()).collect();
        assert_eq!(statuses, ["running", "failed"], "{live:?}");
        assert!(live.iter().all(|s| s["ref_id"] == LINK_OPEN_REF && s["tool"] == "browser_open"), "{live:?}");
        assert_eq!(live[0]["detail"], "https://tickets.example.invalid/buy");
    }

    #[test]
    fn a_link_that_opens_is_ok_with_the_page_title() {
        let (turn, _rx) = link_turn("turn-link-opened");
        let nav = crate::sandbox::NavigationResult { url: "https://roast.example/shop".into(), title: "RoastCo · Shop".into(), status_code: 200 };
        open_link(&turn, "https://roast.example/shop", Ok(nav));
        let saved = turn.finish().unwrap();
        let s = &saved["steps"][0];
        assert_eq!((s["status"].as_str(), s["title"].as_str()), (Some("ok"), Some("Opening roast.example")));
        assert_eq!(s["page_title"], "RoastCo · Shop");
        assert_eq!(s["detail"], "https://roast.example/shop");
        assert_eq!(s["ref_id"], LINK_OPEN_REF);
    }

    #[test]
    fn a_site_rule_refusal_or_a_held_browser_is_a_failed_step_with_its_reason() {
        let refusal = crate::sandbox::site_guard::Refusal {
            rule: "blocked",
            host: "casino.example".into(),
            url: "https://casino.example/".into(),
            entry: Some("casino.example".into()),
            message: "casino.example is on this bot's blocked sites.".into(),
        };
        let (turn, _rx) = link_turn("turn-link-refused");
        open_link(&turn, "https://casino.example/", Err(crate::sandbox::BrowserError::SiteRule(refusal)));
        let saved = turn.finish().unwrap();
        let s = &saved["steps"][0];
        assert_eq!((s["status"].as_str(), s["title"].as_str()), (Some("failed"), Some("Opening casino.example")));
        assert_eq!(s["detail"], "casino.example is on this bot's blocked sites.");
        // Errors main dropped silently are a failed step too
        let (turn, _rx) = link_turn("turn-link-held");
        open_link(&turn, "https://roast.example/", Err(crate::sandbox::BrowserError::HumanControl));
        let s = turn.finish().unwrap()["steps"][0].clone();
        assert_eq!(s["status"], "failed");
        assert_eq!(s["detail"], crate::sandbox::BrowserError::HumanControl.to_string());
    }

    #[test]
    fn a_secret_in_the_link_is_masked_in_the_title_and_detail() {
        let url = "https://pebble:hunter2hunter2@tickets.example.invalid/pay?token=abcd1234efgh5678ijkl&api_key=sk-live0123456789abcdefXYZ";
        let (turn, mut rx) = link_turn("turn-link-secret");
        open_link(&turn, url, Err(crate::sandbox::BrowserError::Navigation(format!("net::ERR_NAME_NOT_RESOLVED at {url}"))));
        let saved = turn.finish().unwrap();
        let mut shown: Vec<Value> = live_steps(&mut rx);
        shown.push(saved["steps"][0].clone());
        let started = turn.with_log(|log| log.steps()[0].clone());
        assert_eq!(started.tool.as_deref(), Some("browser_open"));
        for s in &shown {
            let text = format!("{} {}", s["title"], s["detail"]);
            for secret in ["hunter2hunter2", "abcd1234efgh5678ijkl", "sk-live0123456789abcdefXYZ"] {
                assert!(!text.contains(secret), "{secret} shown in {text}");
            }
            assert!(text.contains("tickets.example.invalid"), "{text}");
        }
        assert!(saved["steps"][0]["detail"].as_str().unwrap().contains("ERR_NAME_NOT_RESOLVED"), "{saved}");
    }

    /// Item 2 (13a and follow-through as on main): a link that didn't open and no tool after it
    /// gives the same verdict, continuation, `meta.blocked` and held-turn carry-over as before the
    /// step was saved, when `page_failed` alone said it.
    #[test]
    fn a_failed_link_and_no_later_tool_follows_through_as_before() {
        use crate::server::delivery_check::{check, failed_step_blocked, pushed_in, tool_steps_in, FailedBefore, Seen};
        let (turn, _rx) = link_turn("turn-link-follow");
        open_link(&turn, "https://tickets.example.invalid/buy", unresolved());
        let saved = turn.finish();
        assert_eq!(saved.as_ref().unwrap()["steps"].as_array().unwrap().len(), 1, "the step is saved on the reply");
        // Read raw, the step would be the turn's own failed last tool step
        assert!(tool_steps_in(saved.as_ref()).1.is_some());
        let own = without_link_open(saved.as_ref());
        // Main saved no activity for this turn at all
        let main: Option<Value> = None;
        assert_eq!(own, main);
        assert_eq!(tool_steps_in(own.as_ref()), tool_steps_in(main.as_ref()));
        assert_eq!(tool_steps_in(own.as_ref()), (false, None, false));
        assert_eq!(pushed_in(own.as_ref()), pushed_in(main.as_ref()));
        let seen = |a: Option<&Value>| {
            let (ran_tools, failed_step, declined) = tool_steps_in(a);
            Seen { ran_tools, failed_step, declined, pushed: pushed_in(a), ..Seen::default() }
        };
        let reply = "The address tickets.example.invalid didn't resolve. Send me the correct link and I'll open it.";
        for continued in [false, true] {
            let (ours, before) = (check(reply, None, continued, &seen(own.as_ref())), check(reply, None, continued, &seen(main.as_ref())));
            assert_eq!(ours, before, "continued {continued}");
            assert_eq!(ours.continue_with, None, "a link told \"do not retry\" never gets a continuation");
        }
        let page_failed = FailedBefore { failed: true, declined: false };
        let blocked = failed_step_blocked(reply, own.as_ref(), page_failed);
        assert_eq!(blocked, failed_step_blocked(reply, main.as_ref(), page_failed));
        assert!(blocked.is_some(), "page_failed still blocks the reply that waits on the person");
        assert_eq!(FailedBefore::of_turn(own.as_ref(), page_failed), FailedBefore::of_turn(main.as_ref(), page_failed));
        // A link that opened leaves nothing behind either
        let (turn, _rx) = link_turn("turn-link-follow-ok");
        let nav = crate::sandbox::NavigationResult { url: "https://roast.example/".into(), title: "RoastCo".into(), status_code: 200 };
        open_link(&turn, "https://roast.example/", Ok(nav));
        let own = without_link_open(turn.finish().as_ref());
        assert_eq!(seen(own.as_ref()), seen(None));
        assert_eq!(FailedBefore::of_turn(own.as_ref(), FailedBefore::default()), FailedBefore::default());
    }

    /// ...and the bot's own steps after it still count, failed or not.
    #[test]
    fn a_failed_link_then_a_failed_bot_tool_still_counts_the_bots_step() {
        use crate::server::delivery_check::{failed_step_blocked, tool_steps_in, FailedBefore};
        let (turn, _rx) = link_turn("turn-link-then-bash");
        open_link(&turn, "https://tickets.example.invalid/buy", unresolved());
        bot_tool(&turn, "t1", "Bash", json!({"command": "curl https://tickets.example/buy"}), json!({"status": "error", "error": "curl: (6) Could not resolve host"}));
        let saved = turn.finish();
        assert_eq!(saved.as_ref().unwrap()["steps"].as_array().unwrap().len(), 2);
        let own = without_link_open(saved.as_ref());
        let (ran, failed, declined) = tool_steps_in(own.as_ref());
        let failed = failed.expect("the bot's own failed step counts");
        assert_eq!((ran, failed.tool.as_str(), declined), (true, "Bash", false));
        assert!(failed.detail.contains("Could not resolve host"), "{failed:?}");
        let after = FailedBefore::of_turn(own.as_ref(), FailedBefore { failed: true, declined: false });
        assert!(after.failed);
        assert!(failed_step_blocked("Send me the correct link and I'll try again.", own.as_ref(), FailedBefore::default()).is_some());
        // A bot tool that worked after the failed link: the turn moved on, as on main
        let (turn, _rx) = link_turn("turn-link-then-ok");
        open_link(&turn, "https://tickets.example.invalid/buy", unresolved());
        bot_tool(&turn, "t1", "WebSearch", json!({"query": "tickets example"}), json!({"status": "ok"}));
        let own = without_link_open(turn.finish().as_ref());
        assert_eq!(tool_steps_in(own.as_ref()), (true, None, false));
        assert_eq!(own.unwrap()["steps"].as_array().unwrap().len(), 1, "only the bot's own step");
    }

    /// A running turn listed by `GET /api/turns` shows the step marked too.
    #[test]
    fn the_listed_turn_marks_the_link_step() {
        let (turn, _rx) = link_turn("turn-link-listed");
        turn.link_open_start("https://roast.example/");
        let mut v = turn.with_log(|log| log.to_view_json());
        turn.mark_link_step(&mut v);
        assert_eq!(v["steps"][0]["ref_id"], LINK_OPEN_REF);
        assert_eq!(v["steps"][0]["status"], "running");
        // The bot's own browser_open is never marked
        bot_tool(&turn, "b1", "mcp__kwnt__browser_open", json!({"url": "https://roast.example/a"}), json!({"status": "ok", "title": "A"}));
        let saved = turn.finish().unwrap();
        assert_eq!(saved["steps"][1]["tool"], "mcp__kwnt__browser_open");
        assert!(saved["steps"][1].get("ref_id").is_none(), "{saved}");
    }
}
