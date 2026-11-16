//! 21d: on the claude engine, the CLI's own tools (Bash, Read, WebFetch…) stop repeating a call
//! that keeps failing.
//!
//! kwnt only sees a built-in call's permission prompt, never its result, and claude allows some
//! calls (Read, Grep…) without asking at all. So the turn's one `--settings` gets three command
//! hooks that post claude's hook payload to [`hook_url`] for this turn's token:
//! - `PreToolUse`: kwnt checks the turn's [`RepeatGuard`]; on a stop it answers
//!   `permissionDecision: "deny"` with the same [`stop_text`](super::repeat_guard::stop_text) as
//!   kwnt's own tools (18a), so the model reads the same words. Claude runs PreToolUse before its
//!   permission check, so a stopped call never shows a card.
//! - `PostToolUse` (claude 2.1.325: a call that worked) clears the call. 22t: when that call
//!   changed something ([`changes_something`]: an edit, or a shell command that isn't stopped),
//!   every stopped call gets its tries again ([`RepeatGuard::changed`]), so a fixed test runs.
//! - `PostToolUseFailure` (a call that failed, with its `error` text, e.g. `"Exit code 1\n…"`)
//!   counts one more identical failure. A call claude or the person denied fires neither event.
//!
//! The hook never blocks a turn on its own error: `curl -sf -m 2 … || true` exits 0 with no output
//! when kwnt is unreachable, slow, or answers with an error, and claude then runs the call as usual.
//! The URL (it carries the turn's token) sits in a 0600 curl config file, never in argv.

use super::cli_config::{create_private_dir, ConfigFile};
use super::repeat_guard::{is_stop_text, RepeatGuard};
use serde_json::{json, Value};
use std::io::Write;
use std::path::Path;

/// The hook events kwnt asks claude to post.
pub const HOOK_EVENTS: [&str; 3] = ["PreToolUse", "PostToolUse", "PostToolUseFailure"];

/// Only claude's own tools: MCP tools (`mcp__…`) are left to their own guards (kwnt's own tools
/// have 18a's). claude's matcher is a JavaScript regular expression.
pub const BUILTIN_MATCHER: &str = "^(?!mcp__)";

/// Seconds claude waits for the hook before it runs the call anyway (curl itself gives up after 2).
const HOOK_TIMEOUT_SECS: u64 = 5;

/// Where the hooks post for the turn whose permission endpoint is `permission_url`.
pub fn hook_url(permission_url: &str) -> String {
    format!("{}/hook", permission_url.trim_end_matches('/'))
}

fn sh_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}

/// The hook command: posts claude's payload (stdin) to the URL in `curl_config`. Silent and
/// exit 0 whatever happens, so a failure never blocks the call (exit 2 would).
pub fn hook_command(curl_config: &Path) -> String {
    format!(
        "curl -sf -m 2 -H 'Content-Type: application/json' --data-binary @- -K {} || true",
        sh_quote(&curl_config.to_string_lossy())
    )
}

/// The `hooks` object merged into the turn's one `--settings`.
pub fn settings(curl_config: &Path) -> Value {
    let entry = json!([{
        "matcher": BUILTIN_MATCHER,
        "hooks": [{"type": "command", "command": hook_command(curl_config), "timeout": HOOK_TIMEOUT_SECS}],
    }]);
    let mut hooks = serde_json::Map::new();
    for event in HOOK_EVENTS {
        hooks.insert(event.to_string(), entry.clone());
    }
    json!({ "hooks": hooks })
}

/// Writes the curl config (`url = "…"`, mode 0600) for this turn's hooks next to its MCP config.
/// Removed when the returned guard drops.
pub fn write_curl_config(config_dir: &Path, permission_url: &str) -> std::io::Result<ConfigFile> {
    create_private_dir(config_dir)?;
    let path = config_dir.join(format!("claude-hook-{}.curl", uuid::Uuid::new_v4().simple()));
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(&path)?;
    let guard = ConfigFile { path };
    let url = hook_url(permission_url).replace('\\', "\\\\").replace('"', "\\\"");
    file.write_all(format!("url = \"{url}\"\n").as_bytes())?;
    file.sync_all()?;
    Ok(guard)
}

/// The same call is the same key as 18a's (tool name plus arguments), except Bash's
/// `description`: the model writes a new one for the same command, and it changes nothing.
pub fn key(tool: &str, input: &Value) -> [u8; 32] {
    match input {
        Value::Object(map) if tool == "Bash" && map.contains_key("description") => {
            let mut map = map.clone();
            map.remove("description");
            RepeatGuard::key(tool, &Value::Object(map))
        }
        _ => RepeatGuard::key(tool, input),
    }
}

/// 22t: claude's tools that write files. A read (Read, Grep, Glob, LS, WebFetch…) changes nothing.
pub const WRITE_TOOLS: [&str; 4] = ["Write", "Edit", "MultiEdit", "NotebookEdit"];

/// 22t: whether a call that worked may have changed what a stopped call would see: a file write,
/// or a shell command other than a stopped one (Rigel, #693 QA). A shell command can't be read
/// for its effect, so any other one counts; [`REPEAT_CEILING`](super::repeat_guard::REPEAT_CEILING)
/// still ends a loop.
pub fn changes_something(guard: &RepeatGuard, tool: &str, key: &[u8; 32]) -> bool {
    WRITE_TOOLS.contains(&tool) || (tool == "Bash" && guard.check(key).is_none())
}

/// What kwnt does with one hook payload. `Some` is the hook's stdout (a PreToolUse deny);
/// `None` means no output, and claude goes on as usual.
pub fn on_event(guard: &RepeatGuard, msg: &Value) -> Option<Value> {
    let tool = msg["tool_name"].as_str().unwrap_or("");
    if tool.is_empty() || tool.starts_with("mcp__") {
        return None;
    }
    let input = msg.get("tool_input").cloned().unwrap_or_else(|| json!({}));
    let k = key(tool, &input);
    match msg["hook_event_name"].as_str().unwrap_or("") {
        "PreToolUse" => {
            let stopped = guard.check(&k)?;
            tracing::info!(tool = %tool, count = stopped.count, "repeat guard: didn't run a claude tool call that kept failing the same way");
            Some(json!({
                "hookSpecificOutput": {
                    "hookEventName": "PreToolUse",
                    "permissionDecision": "deny",
                    "permissionDecisionReason": stopped.text,
                }
            }))
        }
        "PostToolUse" => {
            let changed = changes_something(guard, tool, &k);
            guard.after(&k, "ok", "");
            if changed {
                guard.changed();
            }
            None
        }
        "PostToolUseFailure" => {
            let error = msg["error"].as_str().unwrap_or("");
            // The person pressing stop isn't a failure; a stop text is the guard's own words
            if msg["is_interrupt"] != json!(true) && !is_stop_text(error) {
                guard.after(&k, "error", error);
            }
            None
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mcp::repeat_guard::{stop_text, REPEAT_CEILING, REPEAT_LIMIT, STOP_PREFIX};

    // Shapes recorded from claude 2.1.325 (Nov 15), see autonomy/cli-tools-repeat-guard/impl.md
    fn pre(tool: &str, input: Value) -> Value {
        json!({"session_id": "s", "hook_event_name": "PreToolUse", "tool_name": tool, "tool_input": input, "tool_use_id": "toolu_1"})
    }
    fn failed(tool: &str, input: Value, error: &str) -> Value {
        json!({"hook_event_name": "PostToolUseFailure", "tool_name": tool, "tool_input": input, "tool_use_id": "toolu_1",
               "error": error, "is_interrupt": false, "duration_ms": 19})
    }
    fn worked(tool: &str, input: Value) -> Value {
        json!({"hook_event_name": "PostToolUse", "tool_name": tool, "tool_input": input, "tool_use_id": "toolu_1",
               "tool_response": {"stdout": "x", "stderr": "", "interrupted": false}})
    }
    const CAT_ERR: &str = "Exit code 1\ncat: /tmp/missing-a.txt: No such file or directory";

    fn bash(description: &str) -> Value {
        json!({"command": "cat /tmp/missing-a.txt", "description": description})
    }

    #[test]
    fn three_failures_deny_the_fourth_with_the_same_stop_text() {
        let g = RepeatGuard::default();
        for i in 0..REPEAT_LIMIT {
            assert_eq!(on_event(&g, &pre("Bash", bash(&format!("try {i}")))), None, "call {} runs", i + 1);
            assert_eq!(on_event(&g, &failed("Bash", bash(&format!("try {i}")), CAT_ERR)), None);
        }
        let out = on_event(&g, &pre("Bash", bash("one more"))).expect("4th call denied");
        let o = &out["hookSpecificOutput"];
        assert_eq!(o["hookEventName"], "PreToolUse");
        assert_eq!(o["permissionDecision"], "deny");
        let reason = o["permissionDecisionReason"].as_str().unwrap();
        assert!(reason.starts_with(STOP_PREFIX), "{reason}");
        assert!(reason.contains("already failed 3 times in this turn"), "{reason}");
        assert!(reason.contains("cat: /tmp/missing-a.txt: No such file or directory"), "{reason}");
        assert_eq!(
            reason,
            crate::mcp::repeat_guard::stop_text(3, CAT_ERR),
            "the same words as kwant's own tools (18a)"
        );
        // A different command isn't stopped
        assert_eq!(on_event(&g, &pre("Bash", json!({"command": "cat /tmp/other"}))), None);
    }

    #[test]
    fn a_different_error_starts_again_and_ok_clears() {
        let g = RepeatGuard::default();
        let input = json!({"file_path": "/tmp/nope.txt"});
        for _ in 0..2 {
            on_event(&g, &failed("Read", input.clone(), "File does not exist."));
        }
        on_event(&g, &failed("Read", input.clone(), "EACCES: permission denied"));
        on_event(&g, &failed("Read", input.clone(), "EACCES: permission denied"));
        assert_eq!(on_event(&g, &pre("Read", input.clone())), None, "2 of the new error so far");
        on_event(&g, &failed("Read", input.clone(), "EACCES: permission denied"));
        assert!(on_event(&g, &pre("Read", input.clone())).is_some(), "3 of the new error");
        // The file appears: an ok clears the call
        on_event(&g, &worked("Read", input.clone()));
        assert_eq!(on_event(&g, &pre("Read", input.clone())), None);
        for _ in 0..2 {
            on_event(&g, &failed("Read", input.clone(), "EACCES: permission denied"));
        }
        assert_eq!(on_event(&g, &pre("Read", input)), None, "counting starts again after an ok");
    }

    #[test]
    fn interrupts_stop_texts_and_mcp_tools_are_not_counted() {
        let g = RepeatGuard::default();
        let input = json!({"url": "https://x.test"});
        for _ in 0..5 {
            let mut m = failed("WebFetch", input.clone(), "Request aborted");
            m["is_interrupt"] = json!(true);
            on_event(&g, &m);
            let stop = crate::mcp::repeat_guard::stop_text(3, "boom");
            on_event(&g, &failed("WebFetch", input.clone(), &format!("PreToolUse:WebFetch hook error: {stop}")));
        }
        assert_eq!(on_event(&g, &pre("WebFetch", input)), None);
        let mcp = json!({"q": 1});
        for _ in 0..5 {
            on_event(&g, &failed("mcp__github__create_issue", mcp.clone(), "422"));
        }
        assert_eq!(on_event(&g, &pre("mcp__github__create_issue", mcp)), None, "left to the MCP guards");
        // kwnt's own ask_user (and its question tool) never pass through these hooks: a question
        // that "fails" (dismissed, timed out) any number of times is never denied or counted
        for tool in ["mcp__kwnt_tools__ask_user", "mcp__kwnt__ask_user", "mcp__kwnt_tools__question"] {
            let ask = json!({"question": "Ship it?", "choices": ["yes", "no"]});
            for _ in 0..5 {
                assert_eq!(on_event(&g, &pre(tool, ask.clone())), None, "{tool}");
                assert_eq!(on_event(&g, &failed(tool, ask.clone(), "The person dismissed the question.")), None, "{tool}");
            }
            assert_eq!(on_event(&g, &pre(tool, ask)), None, "{tool} is never denied");
        }
        // Junk never panics and never denies
        for junk in [json!(null), json!({}), json!({"hook_event_name": "PreToolUse"}), json!({"tool_name": "Bash"})] {
            assert_eq!(on_event(&g, &junk), None);
        }
    }

    // ---- 22t: a stopped call gets its tries again once something changed ----

    const NPM_ERR: &str = "Exit code 1\nFAIL src/sum.test.js\n  expected 3, received 4";

    fn npm(description: &str) -> Value {
        json!({"command": "npm test", "description": description})
    }

    /// `npm test` fails the same way 3 times; the 4th is denied.
    fn npm_fails_three_times(g: &RepeatGuard) {
        for i in 0..REPEAT_LIMIT {
            assert_eq!(on_event(g, &pre("Bash", npm("run tests"))), None, "npm test {} runs", i + 1);
            on_event(g, &failed("Bash", npm("run tests"), NPM_ERR));
        }
        assert!(on_event(g, &pre("Bash", npm("again"))).is_some(), "4th npm test denied");
    }

    #[test]
    fn a_an_edit_that_worked_lets_the_stopped_test_run_again() {
        let g = RepeatGuard::default();
        npm_fails_three_times(&g);
        let edit = json!({"file_path": "/w/src/sum.js", "old_string": "a + b + 1", "new_string": "a + b"});
        assert_eq!(on_event(&g, &pre("Edit", edit.clone())), None);
        on_event(&g, &worked("Edit", edit));
        assert_eq!(on_event(&g, &pre("Bash", npm("after the fix"))), None, "the 4th npm test runs after the edit");
        on_event(&g, &worked("Bash", npm("after the fix")));
        assert_eq!(on_event(&g, &pre("Bash", npm("once more"))), None);
        // Write, MultiEdit and NotebookEdit change files too
        for tool in ["Write", "MultiEdit", "NotebookEdit"] {
            let g = RepeatGuard::default();
            npm_fails_three_times(&g);
            on_event(&g, &worked(tool, json!({"file_path": "/w/x"})));
            assert_eq!(on_event(&g, &pre("Bash", npm("again"))), None, "{tool} reopens it");
        }
        // An edit that failed changed nothing
        let g = RepeatGuard::default();
        npm_fails_three_times(&g);
        on_event(&g, &failed("Edit", json!({"file_path": "/w/x"}), "String to replace not found in file."));
        assert!(on_event(&g, &pre("Bash", npm("again"))).is_some());
    }

    #[test]
    fn b_a_different_shell_command_that_worked_lets_it_run_again() {
        let g = RepeatGuard::default();
        npm_fails_three_times(&g);
        let sed = json!({"command": "sed -i 's/a + b + 1/a + b/' src/sum.js", "description": "fix the sum"});
        assert_eq!(on_event(&g, &pre("Bash", sed.clone())), None);
        on_event(&g, &worked("Bash", sed));
        assert_eq!(on_event(&g, &pre("Bash", npm("after sed"))), None, "the 4th npm test runs after sed");
        // A different shell command that failed changed nothing
        let g = RepeatGuard::default();
        npm_fails_three_times(&g);
        on_event(&g, &failed("Bash", json!({"command": "sed -i 's/x/y/' nope.js"}), "Exit code 2\nsed: can't read nope.js"));
        assert!(on_event(&g, &pre("Bash", npm("again"))).is_some());
    }

    #[test]
    fn c_a_read_does_not_reopen_it_but_a_later_edit_does() {
        let g = RepeatGuard::default();
        npm_fails_three_times(&g);
        for (tool, input) in [
            ("Read", json!({"file_path": "/w/src/sum.js"})),
            ("Grep", json!({"pattern": "a \\+ b", "path": "/w/src"})),
            ("Glob", json!({"pattern": "**/*.test.js"})),
            ("LS", json!({"path": "/w/src"})),
            ("WebFetch", json!({"url": "https://jestjs.io/docs/expect", "prompt": "toBe"})),
            ("TodoWrite", json!({"todos": [{"content": "fix sum", "status": "in_progress"}]})),
        ] {
            assert_eq!(on_event(&g, &pre(tool, input.clone())), None, "{tool} runs");
            on_event(&g, &worked(tool, input));
            let out = on_event(&g, &pre("Bash", npm("again"))).unwrap_or_else(|| panic!("{tool} reopened npm test"));
            assert_eq!(out["hookSpecificOutput"]["permissionDecisionReason"], stop_text(3, NPM_ERR));
        }
        // Then the bot really fixes the file
        on_event(&g, &worked("Edit", json!({"file_path": "/w/src/sum.js", "old_string": "+ 1", "new_string": ""})));
        assert_eq!(on_event(&g, &pre("Bash", npm("after the fix"))), None);
    }

    #[test]
    fn d_with_an_edit_between_every_failure_it_stops_at_the_turn_ceiling() {
        let g = RepeatGuard::default();
        for i in 1..=REPEAT_CEILING {
            assert_eq!(on_event(&g, &pre("Bash", npm(&format!("try {i}")))), None, "npm test {i} runs");
            on_event(&g, &failed("Bash", npm(&format!("try {i}")), NPM_ERR));
            on_event(&g, &worked("Edit", json!({"file_path": "/w/src/sum.js", "old_string": format!("v{i}"), "new_string": format!("v{}", i + 1)})));
        }
        let out = on_event(&g, &pre("Bash", npm("try 7"))).expect("the 7th npm test is denied");
        let reason = out["hookSpecificOutput"]["permissionDecisionReason"].as_str().unwrap();
        assert_eq!(reason, stop_text(REPEAT_CEILING, NPM_ERR));
        assert!(reason.contains("already failed 6 times in this turn"), "{reason}");
        // Neither an edit nor another shell command lifts the ceiling
        on_event(&g, &worked("Write", json!({"file_path": "/w/src/sum.js"})));
        on_event(&g, &worked("Bash", json!({"command": "git stash"})));
        assert!(on_event(&g, &pre("Bash", npm("try 8"))).is_some());
        // 3 in a row, an edit, 3 in a row: the 6th is the ceiling too
        let g = RepeatGuard::default();
        npm_fails_three_times(&g);
        on_event(&g, &worked("Edit", json!({"file_path": "/w/a"})));
        for i in 4..=6 {
            assert_eq!(on_event(&g, &pre("Bash", npm("t"))), None, "npm test {i} runs");
            on_event(&g, &failed("Bash", npm("t"), NPM_ERR));
        }
        on_event(&g, &worked("Edit", json!({"file_path": "/w/b"})));
        assert!(on_event(&g, &pre("Bash", npm("t"))).is_some(), "stopped after 6 in the turn");
    }

    #[test]
    fn e_a_different_error_still_starts_again_at_1() {
        let g = RepeatGuard::default();
        let edit = |i: u32| worked("Edit", json!({"file_path": "/w/src/sum.js", "old_string": format!("v{i}"), "new_string": "x"}));
        // 5 identical failures with edits between: under the ceiling, so each one ran
        for i in 1..REPEAT_CEILING {
            assert_eq!(on_event(&g, &pre("Bash", npm("t"))), None, "npm test {i} runs");
            on_event(&g, &failed("Bash", npm("t"), NPM_ERR));
            on_event(&g, &edit(i));
        }
        // The fix moved the failure on: a new error counts from 1, for both counts
        let other = "Exit code 1\nFAIL src/mul.test.js\n  expected 6, received 5";
        for i in 1..REPEAT_CEILING {
            assert_eq!(on_event(&g, &pre("Bash", npm("t"))), None, "npm test with the new error {i} runs");
            on_event(&g, &failed("Bash", npm("t"), other));
            on_event(&g, &edit(10 + i));
        }
        on_event(&g, &failed("Bash", npm("t"), other));
        let out = on_event(&g, &pre("Bash", npm("t"))).expect("6 of the new error stop it");
        assert_eq!(out["hookSpecificOutput"]["permissionDecisionReason"], stop_text(6, other));
        // Without edits, the new error still stops at 3, as before
        let g = RepeatGuard::default();
        npm_fails_three_times(&g);
        on_event(&g, &edit(1));
        for _ in 0..REPEAT_LIMIT {
            on_event(&g, &failed("Bash", npm("t"), other));
        }
        let out = on_event(&g, &pre("Bash", npm("t"))).expect("3 of the new error in a row");
        assert_eq!(out["hookSpecificOutput"]["permissionDecisionReason"], stop_text(3, other));
    }

    #[test]
    fn the_bash_description_is_not_part_of_the_call() {
        assert_eq!(key("Bash", &bash("a")), key("Bash", &bash("b")));
        assert_eq!(key("Bash", &bash("a")), key("Bash", &json!({"command": "cat /tmp/missing-a.txt"})));
        assert_ne!(key("Bash", &bash("a")), key("Bash", &json!({"command": "cat /tmp/missing-b.txt", "description": "a"})));
        // Other tools keep every argument
        assert_ne!(key("Task", &json!({"description": "a"})), key("Task", &json!({"description": "b"})));
    }

    #[test]
    fn settings_post_three_events_for_builtin_tools_and_fail_open() {
        let path = Path::new("/Users/o/Library/Application Support/kwnt/mcp/claude-hook-1.curl");
        let v = settings(path);
        let hooks = v["hooks"].as_object().unwrap();
        assert_eq!(hooks.len(), 3);
        for event in HOOK_EVENTS {
            let e = &hooks[event][0];
            assert_eq!(e["matcher"], BUILTIN_MATCHER);
            let h = &e["hooks"][0];
            assert_eq!(h["type"], "command");
            assert_eq!(h["timeout"], 5);
            let cmd = h["command"].as_str().unwrap();
            assert!(cmd.starts_with("curl -sf -m 2 "), "{cmd}");
            assert!(cmd.ends_with(" || true"), "never exits 2: {cmd}");
            assert!(cmd.contains("-K '/Users/o/Library/Application Support/kwnt/mcp/claude-hook-1.curl'"), "{cmd}");
            assert!(!cmd.contains("/api/mcp/permission"), "the token stays out of argv: {cmd}");
        }
        assert_eq!(hook_command(Path::new("/a/it's")), "curl -sf -m 2 -H 'Content-Type: application/json' --data-binary @- -K '/a/it'\\''s' || true");
    }

    #[test]
    fn the_curl_config_holds_the_url_privately_and_goes_with_its_guard() {
        let dir = tempfile::tempdir().unwrap();
        let mcp = dir.path().join("mcp");
        let f = write_curl_config(&mcp, "http://127.0.0.1:8790/api/mcp/permission/tok_A-1").unwrap();
        let text = std::fs::read_to_string(&f.path).unwrap();
        assert_eq!(text, "url = \"http://127.0.0.1:8790/api/mcp/permission/tok_A-1/hook\"\n");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(std::fs::metadata(&f.path).unwrap().permissions().mode() & 0o777, 0o600);
        }
        let path = f.path.clone();
        drop(f);
        assert!(!path.exists());
    }
}
