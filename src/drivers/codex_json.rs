//! What `codex exec --json` prints on stdout, one JSON event per line, turned into events as it
//! arrives: its reasoning summaries (`ThinkingDelta`) and each command, file change, MCP tool
//! call, web search and plan update it starts and finishes. The reply itself still comes from
//! codex's `-o` file. Errors are kept for the refusal check, since `--json` moves the final
//! error from stderr to stdout.

use super::events::RuntimeEvent;
use serde_json::{json, Value};
use std::collections::HashSet;

/// Longest error text kept from codex's events, over all of them.
const ERRORS_MAX: usize = 4000;

#[derive(Default)]
pub struct CodexStream {
    /// Items started and not yet finished, by item id.
    running: HashSet<String>,
    /// Items already finished, so a repeated `item.completed` isn't reported twice.
    finished: HashSet<String>,
    /// It ran a command, changed a file or called a tool.
    pub had_effects: bool,
    /// The turn's errors, one per line, retries left out.
    pub errors: String,
}

/// The tool name and arguments an item is shown with, or `None` for items that aren't tool runs.
fn as_tool(item: &Value) -> Option<(String, Value)> {
    let s = |k: &str| item[k].as_str().unwrap_or_default().to_string();
    Some(match item["type"].as_str()? {
        "command_execution" => ("command_execution".into(), json!({ "command": item["command"] })),
        "file_change" => {
            let path = item["changes"].as_array().and_then(|c| c.first()).map(|c| c["path"].clone());
            ("file_change".into(), json!({ "path": path.unwrap_or(Value::Null) }))
        }
        "mcp_tool_call" => (format!("mcp__{}__{}", s("server"), s("tool")), item["arguments"].clone()),
        "web_search" => ("web_search".into(), json!({ "query": item["query"] })),
        "todo_list" => ("todo_list".into(), json!({})),
        _ => return None,
    })
}

/// How a finished item went: `ok`, or `error` with why.
fn outcome(item: &Value) -> Value {
    let status = item["status"].as_str().unwrap_or_default();
    match item["type"].as_str() {
        Some("command_execution") => match item["exit_code"].as_i64() {
            Some(0) if status != "failed" => json!({ "status": "ok" }),
            Some(code) => json!({ "status": "error", "error": format!("exit code {code}") }),
            None if status == "failed" || status == "declined" => json!({ "status": "error", "error": status }),
            None => json!({ "status": "ok" }),
        },
        _ if status == "failed" || status == "declined" || item["error"].is_object() => {
            let why = item["error"]["message"].as_str().unwrap_or(status);
            json!({ "status": "error", "error": why })
        }
        _ => json!({ "status": "ok" }),
    }
}

impl CodexStream {
    fn keep_error(&mut self, message: &str) {
        let message = message.trim();
        // "Reconnecting... 2/5 (…)" is a retry; the error that ends the turn follows it
        if message.is_empty() || message.starts_with("Reconnecting") || self.errors.len() >= ERRORS_MAX {
            return;
        }
        if !self.errors.lines().any(|l| l == message) {
            self.errors.push_str(message);
            self.errors.push('\n');
        }
    }

    pub fn line(&mut self, line: &str) -> Vec<RuntimeEvent> {
        let Ok(v) = serde_json::from_str::<Value>(line.trim()) else {
            return Vec::new();
        };
        let mut events = Vec::new();
        let item = &v["item"];
        let id = item["id"].as_str().unwrap_or_default().to_string();
        match v["type"].as_str() {
            Some("error") => self.keep_error(v["message"].as_str().unwrap_or_default()),
            Some("turn.failed") => self.keep_error(v["error"]["message"].as_str().unwrap_or_default()),
            Some("item.started") => {
                if let Some((name, args)) = as_tool(item).filter(|_| !id.is_empty()) {
                    if self.running.insert(id.clone()) {
                        self.had_effects |= item["type"] != "todo_list";
                        events.push(RuntimeEvent::ToolCallPending { id, name, args });
                    }
                }
            }
            Some("item.completed") => match item["type"].as_str() {
                Some("reasoning") => {
                    if let Some(text) = item["text"].as_str().filter(|t| !t.trim().is_empty()) {
                        events.push(RuntimeEvent::ThinkingDelta(format!("{}\n", text.trim_end())));
                    }
                }
                Some("error") => self.keep_error(item["message"].as_str().unwrap_or_default()),
                _ => {
                    if let Some((name, args)) = as_tool(item).filter(|_| !id.is_empty()) {
                        if self.finished.insert(id.clone()) {
                            self.had_effects |= item["type"] != "todo_list";
                            // a file change is often only reported once it's done
                            if !self.running.remove(&id) {
                                events.push(RuntimeEvent::ToolCallPending { id: id.clone(), name, args });
                            }
                            events.push(RuntimeEvent::ToolCallCompleted { id, result: outcome(item) });
                        }
                    }
                }
            },
            _ => {}
        }
        events
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kinds(events: &[RuntimeEvent]) -> Vec<String> {
        events
            .iter()
            .map(|e| match e {
                RuntimeEvent::ThinkingDelta(t) => format!("think {}", t.trim()),
                RuntimeEvent::ToolCallPending { id, name, .. } => format!("start {id} {name}"),
                RuntimeEvent::ToolCallCompleted { id, result } => format!(
                    "end {id} {}{}",
                    result["status"].as_str().unwrap_or(""),
                    result["error"].as_str().map(|e| format!(" ({e})")).unwrap_or_default()
                ),
                other => format!("{other:?}"),
            })
            .collect()
    }

    fn run(fixture: &str) -> (CodexStream, Vec<String>) {
        let mut s = CodexStream::default();
        let events: Vec<RuntimeEvent> = fixture.lines().flat_map(|l| s.line(l)).collect();
        (s, kinds(&events))
    }

    /// A working codex turn, shaped like codex-cli 0.16x `exec --json` output.
    const WORKING: &str = include_str!("fixtures/codex_exec_json.jsonl");
    /// codex with no sign-in: retries, a fallback notice, then the error that ends the turn.
    const SIGNED_OUT: &str = include_str!("fixtures/codex_exec_json_401.jsonl");

    #[test]
    fn codex_json_reports_reasoning_and_each_tool_run() {
        let (s, kinds) = run(WORKING);
        assert_eq!(
            kinds,
            [
                "think **Checking the tests**\n\nI'll run the test suite first.",
                "start item_1 command_execution",
                "end item_1 error (exit code 101)",
                "start item_2 file_change",
                "end item_2 ok",
                "start item_3 mcp__github__create_issue",
                "end item_3 error (forbidden)",
                "start item_4 web_search",
                "end item_4 ok",
            ]
        );
        assert!(s.had_effects);
        assert!(s.errors.is_empty());
    }

    #[test]
    fn codex_json_keeps_the_error_that_ended_the_turn_for_the_refusal_check() {
        let (s, kinds) = run(SIGNED_OUT);
        assert!(kinds.is_empty(), "{kinds:?}");
        assert!(!s.had_effects);
        assert!(!s.errors.contains("Reconnecting"), "{}", s.errors);
        assert!(s.errors.contains("unexpected status 401 Unauthorized"), "{}", s.errors);
        assert_eq!(s.errors.lines().count(), 2, "the fallback notice and the final error, once each: {}", s.errors);
    }

    #[test]
    fn codex_json_titles_read_like_people_wrote_them() {
        let mut s = CodexStream::default();
        let events = s.line(r#"{"type":"item.started","item":{"id":"i","type":"command_execution","command":"bash -lc 'cargo test'","status":"in_progress"}}"#);
        let RuntimeEvent::ToolCallPending { name, args, .. } = &events[0] else { panic!() };
        assert_eq!(crate::drivers::activity::summarize(name, args).0, "Running `cargo test`");
        assert!(s.line("not json").is_empty());
    }
}
