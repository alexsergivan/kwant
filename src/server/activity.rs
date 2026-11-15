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
}

fn turns() -> &'static Mutex<HashMap<String, Weak<TurnActivity>>> {
    static TURNS: OnceLock<Mutex<HashMap<String, Weak<TurnActivity>>>> = OnceLock::new();
    TURNS.get_or_init(Default::default)
}

impl TurnActivity {
    /// Starts the turn's log and registers it under its turn id until it is dropped.
    pub fn start(sender: broadcast::Sender<RuntimeEvent>, tag: TurnTag) -> Arc<Self> {
        let this = Arc::new(Self { log: Mutex::new(ActivityLog::default()), sender, tag });
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
        for step in steps {
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
        saved
    }
}

/// An approval card for `turn_id` is waiting on the person.
pub fn approval_waiting(turn_id: &str, ticket: &str, tool_name: &str) {
    if let Some(turn) = TurnActivity::find(turn_id) {
        let (title, _) = crate::drivers::activity::summarize(tool_name, &Value::Null);
        turn.wait("approval", ticket, &format!("Waiting for your approval: {title}"), Some(tool_name));
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
    let steps = TurnActivity::find(&turn.turn_id).and_then(|t| t.with_log(|log| (!log.is_empty()).then(|| log.to_view_json())));
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

}
