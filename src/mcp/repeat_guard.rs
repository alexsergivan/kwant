//! 15c: a turn stops sending a call that keeps failing the same way.
//!
//! [`McpToolSession`](super::session::McpToolSession) holds one guard per turn. After a call,
//! [`RepeatGuard::after`] counts identical failures (same tool, same arguments, same error text);
//! before a call, [`RepeatGuard::check`] refuses a call that already failed [`REPEAT_LIMIT`]
//! times the same way, so the turn tries another way instead of spending its call cap on it.
//! Successful calls (polling a status) and calls whose error changes never stop. Only `ok` and
//! `error` outcomes count: a denied, unanswered or rehearsed call was the person's decision.
//!
//! 22t: when another call changed something (an edit, another shell command), the earlier
//! failures may be stale, so [`RepeatGuard::changed`] gives every call its [`REPEAT_LIMIT`] tries
//! again: a test run that failed 3 times can run once the file is fixed. The turn still stops the
//! call after [`REPEAT_CEILING`] identical failures, whatever changed in between.
//!
//! 23a: "the same error" ignores the error's numbers (durations, timestamps, PIDs, ports, counts)
//! and long hex ids (shas, uuids), so a test run that prints "finished in 3.21s" each time is
//! still the same failure. The stop text quotes the latest real error.

use serde_json::Value;
use sha2::{Digest, Sha256};
use std::sync::Mutex;

/// Identical failures after which the same call is no longer sent.
pub const REPEAT_LIMIT: u32 = 3;
/// 22t: identical failures in one turn after which the call stops even when something changed in
/// between ([`RepeatGuard::changed`] doesn't lower this count), so edit/retry can't loop forever.
pub const REPEAT_CEILING: u32 = 2 * REPEAT_LIMIT;
/// Calls the guard remembers per turn; the least recently failed one is dropped first.
pub const MAX_KEYS: usize = 64;
/// How much of the repeated error the stop text quotes.
const QUOTE_MAX_CHARS: usize = 200;

struct Entry {
    key: [u8; 32],
    /// The latest real error text: what the stop text quotes.
    error: String,
    /// 23a: the error's [`shape`]: what "the same error" compares.
    shape: String,
    /// Identical failures since the last change ([`RepeatGuard::changed`] sets it to 0).
    count: u32,
    /// Identical failures in the turn: only an `ok` or a different error resets it.
    turn: u32,
}

/// One turn's failing calls, oldest first. Interior mutability: calls may run concurrently.
#[derive(Default)]
pub struct RepeatGuard {
    entries: Mutex<Vec<Entry>>,
}

/// 18a: `SelfScope` derives `Debug`; the entries hold hashes and error texts, so only their count shows.
impl std::fmt::Debug for RepeatGuard {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RepeatGuard").field("entries", &self.lock().len()).finish()
    }
}

/// What the guard says before a call.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Stopped {
    /// The text the model gets instead of the call's result.
    pub text: String,
    /// How many identical failures came before ([`REPEAT_LIMIT`]).
    pub count: u32,
}

impl RepeatGuard {
    /// The same call is the same key: tool name plus the arguments' JSON. serde_json here has
    /// no `preserve_order`, so object keys serialize sorted and key order doesn't matter.
    /// Hashed, so 64 large argument objects don't stay in memory.
    pub fn key(name: &str, arguments: &Value) -> [u8; 32] {
        let mut h = Sha256::new();
        h.update(name.as_bytes());
        h.update([0u8]);
        h.update(arguments.to_string().as_bytes());
        h.finalize().into()
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Vec<Entry>> {
        self.entries.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// `Some` when this call already failed [`REPEAT_LIMIT`] times in a row with the same error
    /// since the last change, or [`REPEAT_CEILING`] times in the turn: don't send it. The count
    /// (and the stop text's number) is the turn's.
    pub fn check(&self, key: &[u8; 32]) -> Option<Stopped> {
        let entries = self.lock();
        let e = entries.iter().find(|e| &e.key == key)?;
        (e.count >= REPEAT_LIMIT || e.turn >= REPEAT_CEILING).then(|| Stopped { text: stop_text(e.turn, &e.error), count: e.turn })
    }

    /// 22t: another call changed something (a file edit, a different shell command, a tool that
    /// isn't read-only), so every call that failed gets its [`REPEAT_LIMIT`] tries again. The
    /// error text and the turn's count stay: the same error after this counts on toward
    /// [`REPEAT_CEILING`].
    pub fn changed(&self) {
        for e in self.lock().iter_mut() {
            e.count = 0;
        }
    }

    /// Records a call's outcome: `error` with the same (redacted) text as last time, apart from
    /// its numbers and hex ids ([`shape`]), counts one more, a different text starts again at 1,
    /// `ok` forgets the call; anything else (denied, unanswered, rehearsed) leaves the count as it
    /// is. The entry keeps the latest text for the stop text.
    pub fn after(&self, key: &[u8; 32], status: &str, text: &str) {
        let mut entries = self.lock();
        let at = entries.iter().position(|e| &e.key == key);
        match status {
            "ok" => {
                if let Some(i) = at {
                    entries.remove(i);
                }
            }
            "error" => {
                let text = text.trim();
                let now = shape(text);
                let mut entry = match at {
                    Some(i) => entries.remove(i),
                    None => Entry { key: *key, error: String::new(), shape: String::new(), count: 0, turn: 0 },
                };
                if entry.turn > 0 && entry.shape == now {
                    entry.count += 1;
                    entry.turn += 1;
                } else {
                    entry.shape = now;
                    entry.count = 1;
                    entry.turn = 1;
                }
                entry.error = text.to_string();
                if entries.len() >= MAX_KEYS {
                    entries.remove(0);
                }
                entries.push(entry);
            }
            _ => {}
        }
    }

    #[cfg(test)]
    fn len(&self) -> usize {
        self.lock().len()
    }
}

/// 23a: an error text with its changing parts made the same. First each run of 7 or more hex
/// characters that has a digit (a sha, a short hash, a temp name) becomes one `#`; hex groups of
/// 4 or more joined by single hyphens count as one run, so a uuid is one `#` too. Then each run of
/// ASCII digits (a duration, a timestamp, a PID, a port, a count) becomes one `#`. Words,
/// punctuation and case stay, so an error that differs in a word has a different shape.
fn shape(text: &str) -> String {
    let c: Vec<char> = text.chars().collect();
    let hex_end = |from: usize| (from..c.len()).find(|&j| !c[j].is_ascii_hexdigit()).unwrap_or(c.len());
    let has_digit = |from: usize, to: usize| c[from..to].iter().any(char::is_ascii_digit);
    let mut hexed = String::with_capacity(text.len());
    let mut i = 0;
    while i < c.len() {
        if !c[i].is_ascii_hexdigit() {
            hexed.push(c[i]);
            i += 1;
            continue;
        }
        let first = hex_end(i);
        let mut end = first;
        if first - i >= 4 {
            while end + 1 < c.len() && c[end] == '-' && c[end + 1].is_ascii_hexdigit() {
                let next = hex_end(end + 1);
                if next - (end + 1) < 4 {
                    break;
                }
                end = next;
            }
            if !has_digit(i, end) {
                end = first;
            }
        }
        let hex = c[i..end].iter().filter(|ch| ch.is_ascii_hexdigit()).count();
        if hex >= 7 && has_digit(i, end) {
            hexed.push('#');
        } else {
            hexed.extend(&c[i..end]);
        }
        i = end;
    }
    let mut out = String::with_capacity(hexed.len());
    let mut in_digits = false;
    for ch in hexed.chars() {
        if ch.is_ascii_digit() {
            if !in_digits {
                out.push('#');
            }
            in_digits = true;
        } else {
            out.push(ch);
            in_digits = false;
        }
    }
    out
}

/// How every stop text starts. A result that carries it is a call the guard didn't send (18a: the
/// claude CLI's results reach tool_failures as plain text, without a `repeat_stopped` field).
pub const STOP_PREFIX: &str = "kwnt didn't send this call: the same call (same tool, same arguments)";

/// Whether a call's result text is the guard's stop text.
pub fn is_stop_text(text: &str) -> bool {
    text.contains(STOP_PREFIX)
}

/// What the model reads instead of a stopped call's result. No connection words of its own, so
/// tool_failures doesn't read it as a connection failure (the quoted error may have some; a
/// stopped call is skipped there by its `repeat_stopped` field).
pub fn stop_text(count: u32, error: &str) -> String {
    let error = super::truncate_text(error.trim(), QUOTE_MAX_CHARS);
    format!(
        "{STOP_PREFIX} already failed {count} times in this \
         turn with the same error: \"{error}\". Change the arguments or try another way. If only the person can fix \
         it, end your reply with \"Blocked: <what is blocked>. <what would unblock it>\"."
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn fail(g: &RepeatGuard, k: &[u8; 32], times: u32, error: &str) {
        for _ in 0..times {
            assert!(g.check(k).is_none(), "stopped too early");
            g.after(k, "error", error);
        }
    }

    #[test]
    fn three_same_failures_stop_the_fourth() {
        let g = RepeatGuard::default();
        let k = RepeatGuard::key("mcp__gh__create_issue", &json!({"repo": "a/b", "title": "x"}));
        fail(&g, &k, 3, "422 Validation Failed: title too short");
        let s = g.check(&k).expect("4th call stopped");
        assert_eq!(s.count, 3);
        assert!(s.text.starts_with("kwnt didn't send this call: the same call (same tool, same arguments) already failed 3 times"));
        assert!(s.text.contains("\"422 Validation Failed: title too short\""));
        assert!(s.text.ends_with("end your reply with \"Blocked: <what is blocked>. <what would unblock it>\"."));
        // Still stopped on a 5th try: the stop itself isn't recorded as a new outcome
        assert!(g.check(&k).is_some());
    }

    #[test]
    fn the_stop_text_is_recognised_and_debug_shows_only_a_count() {
        let g = RepeatGuard::default();
        let k = RepeatGuard::key("browser_open", &json!({"url": "https://x.test"}));
        fail(&g, &k, 3, "secret-looking error: connection closed");
        let s = g.check(&k).unwrap();
        assert!(s.text.starts_with(STOP_PREFIX));
        assert!(is_stop_text(&s.text));
        assert!(is_stop_text(&format!("Error: {}", s.text)), "a wrapper in front still reads as a stop");
        assert!(!is_stop_text("secret-looking error: connection closed"));
        assert_eq!(format!("{g:?}"), "RepeatGuard { entries: 1 }");
    }

    #[test]
    fn a_different_error_text_starts_again() {
        let g = RepeatGuard::default();
        let k = RepeatGuard::key("mcp__a__b", &json!({}));
        fail(&g, &k, 2, "boom");
        g.after(&k, "error", "boom, but different");
        assert!(g.check(&k).is_none());
        g.after(&k, "error", "boom, but different");
        assert!(g.check(&k).is_none());
        g.after(&k, "error", "boom, but different");
        assert!(g.check(&k).is_some());
    }

    #[test]
    fn an_ok_forgets_the_call() {
        let g = RepeatGuard::default();
        let k = RepeatGuard::key("mcp__a__status", &json!({"id": 7}));
        fail(&g, &k, 2, "not ready");
        g.after(&k, "ok", "ready");
        fail(&g, &k, 2, "not ready");
        assert!(g.check(&k).is_none());
        // Polling that keeps working never stops
        for _ in 0..20 {
            g.after(&k, "ok", "running");
        }
        assert!(g.check(&k).is_none());
    }

    #[test]
    fn different_arguments_are_a_different_call() {
        let g = RepeatGuard::default();
        let a = RepeatGuard::key("mcp__a__get", &json!({"id": 1}));
        let b = RepeatGuard::key("mcp__a__get", &json!({"id": 2}));
        let c = RepeatGuard::key("mcp__a__other", &json!({"id": 1}));
        fail(&g, &a, 3, "not found");
        assert!(g.check(&a).is_some());
        assert!(g.check(&b).is_none());
        assert!(g.check(&c).is_none());
    }

    #[test]
    fn key_order_in_arguments_does_not_matter() {
        let x: Value = serde_json::from_str(r#"{"b": 1, "a": {"y": 2, "x": 3}}"#).unwrap();
        let y: Value = serde_json::from_str(r#"{"a": {"x": 3, "y": 2}, "b": 1}"#).unwrap();
        assert_eq!(RepeatGuard::key("t", &x), RepeatGuard::key("t", &y));
        assert_ne!(RepeatGuard::key("t", &x), RepeatGuard::key("u", &x));
    }

    #[test]
    fn denied_unanswered_and_rehearsed_do_not_count() {
        let g = RepeatGuard::default();
        let k = RepeatGuard::key("mcp__a__b", &json!({}));
        for status in ["denied", "unanswered", "rehearsed", "denied", "denied"] {
            g.after(&k, status, "The person declined this tool call.");
        }
        assert!(g.check(&k).is_none());
        // and don't reset a count either
        fail(&g, &k, 2, "boom");
        g.after(&k, "denied", "no");
        g.after(&k, "error", "boom");
        assert!(g.check(&k).is_some());
    }

    #[test]
    fn at_most_64_calls_are_remembered_oldest_dropped() {
        let g = RepeatGuard::default();
        let keys: Vec<_> = (0..65).map(|i| RepeatGuard::key("mcp__a__b", &json!({ "i": i }))).collect();
        fail(&g, &keys[0], 3, "boom");
        for k in &keys[1..] {
            g.after(k, "error", "boom");
        }
        assert_eq!(g.len(), MAX_KEYS);
        // The oldest one was dropped, so it is no longer stopped
        assert!(g.check(&keys[0]).is_none());
        fail(&g, &keys[64], 2, "boom");
        assert!(g.check(&keys[64]).is_some());
        assert_eq!(g.len(), MAX_KEYS);
    }

    #[test]
    fn a_change_gives_a_stopped_call_its_tries_again_up_to_the_ceiling() {
        let g = RepeatGuard::default();
        let k = RepeatGuard::key("Bash", &json!({"command": "npm test"}));
        let other = RepeatGuard::key("Read", &json!({"file_path": "/a"}));
        fail(&g, &k, 3, "1 failing");
        g.after(&other, "error", "nope");
        assert!(g.check(&k).is_some());
        g.changed();
        // 3 more tries, then the turn's 6th identical failure stops it for good
        fail(&g, &k, 3, "1 failing");
        let s = g.check(&k).expect("stopped at the ceiling");
        assert_eq!(s.count, REPEAT_CEILING);
        assert_eq!(s.text, stop_text(6, "1 failing"));
        g.changed();
        assert!(g.check(&k).is_some(), "a change doesn't lift the ceiling");
        // Other entries got their tries back too, and a change with nothing stopped is harmless
        assert!(g.check(&other).is_none());
        RepeatGuard::default().changed();
    }

    #[test]
    fn a_change_between_every_failure_stops_at_the_ceiling() {
        let g = RepeatGuard::default();
        let k = RepeatGuard::key("Bash", &json!({"command": "cargo test"}));
        for i in 1..=REPEAT_CEILING {
            assert!(g.check(&k).is_none(), "try {i} runs");
            g.after(&k, "error", "test a ... FAILED");
            g.changed();
        }
        assert_eq!(g.check(&k).map(|s| s.count), Some(REPEAT_CEILING));
        // A different error starts both counts again; an ok forgets the call
        g.after(&k, "error", "test b ... FAILED");
        assert!(g.check(&k).is_none());
        fail(&g, &k, 2, "test b ... FAILED");
        assert_eq!(g.check(&k).map(|s| s.count), Some(3));
        g.after(&k, "ok", "");
        assert!(g.check(&k).is_none());
    }

    // 23a: the same failure whose error changes only in its numbers or hex ids still repeats.

    fn same_failure_three_times(errors: [&str; 3]) -> Stopped {
        let g = RepeatGuard::default();
        let k = RepeatGuard::key("Bash", &json!({"command": "npm test"}));
        for e in errors {
            assert!(g.check(&k).is_none(), "stopped too early, before {e:?}");
            g.after(&k, "error", e);
        }
        g.check(&k).expect("the 4th try is stopped")
    }

    #[test]
    fn a_failure_that_differs_only_in_its_duration_is_stopped_on_the_fourth_try() {
        let run = |t: &str| {
            format!(
                "test tests::adds ... FAILED\n\ntest result: FAILED. 4 passed; 1 failed; 0 ignored; 0 measured; \
                 0 filtered out; finished in {t}s"
            )
        };
        let s = same_failure_three_times([&run("1.23"), &run("4.56"), &run("7.89")]);
        assert_eq!(s.count, 3);
        // The stop text quotes the latest real error, never the shape
        assert_eq!(s.text, stop_text(3, &run("7.89")));
        assert!(!s.text.contains('#'), "{}", s.text);
    }

    #[test]
    fn a_failure_with_a_new_timestamp_pid_and_port_each_time_is_stopped() {
        let s = same_failure_three_times([
            "2026-11-16T09:10:11.123Z ERROR kwnt[4242]: listen on 127.0.0.1:38211 failed: Address already in use (os error 98)",
            "2026-11-16T09:10:42.907Z ERROR kwnt[4311]: listen on 127.0.0.1:41877 failed: Address already in use (os error 98)",
            "2026-11-16T09:11:05.015Z ERROR kwnt[4390]: listen on 127.0.0.1:39020 failed: Address already in use (os error 98)",
        ]);
        assert_eq!(s.count, 3);
        assert!(s.text.contains("\"2026-11-16T09:11:05.015Z ERROR kwnt[4390]: listen on 127.0.0.1:39020 failed"), "{}", s.text);
        assert!(!s.text.contains('#'), "{}", s.text);
    }

    #[test]
    fn a_failure_with_a_new_sha_or_uuid_each_time_is_stopped() {
        let s = same_failure_three_times([
            "fatal: reference is not a tree: 3f786850e387550fdab836ed7e6dc881de23001b",
            "fatal: reference is not a tree: 89e6c98d92887913cadf06b2adb97f26cde4849b",
            "fatal: reference is not a tree: a94a8fe5ccb19ba61c4c0873d391e987982fbbd3",
        ]);
        assert!(s.text.contains("a94a8fe5ccb19ba61c4c0873d391e987982fbbd3"), "{}", s.text);
        let s = same_failure_three_times([
            "session f47ac10b-58cc-4372-a567-0e02b2c3d479 not found (request 1731750611)",
            "session 9b2d4e6a-1c3f-4a8b-bd2e-7f1a3c5e9d04 not found (request 1731750642)",
            "session c0ffee12-ab34-4cd5-8ef6-0123456789ab not found (request 1731750665)",
        ]);
        assert_eq!(s.text, stop_text(3, "session c0ffee12-ab34-4cd5-8ef6-0123456789ab not found (request 1731750665)"));
    }

    #[test]
    fn numbers_only_failures_count_on_to_the_ceiling_after_a_change() {
        let g = RepeatGuard::default();
        let k = RepeatGuard::key("Bash", &json!({"command": "cargo test"}));
        let run = |i: u32| format!("test a ... FAILED (pid {}, finished in 0.{i}s)", 4000 + i);
        for i in 1..=REPEAT_CEILING {
            assert!(g.check(&k).is_none(), "try {i} runs");
            g.after(&k, "error", &run(i));
            g.changed();
        }
        let s = g.check(&k).expect("stopped at the ceiling");
        assert_eq!(s.count, REPEAT_CEILING);
        assert_eq!(s.text, stop_text(REPEAT_CEILING, &run(REPEAT_CEILING)));
    }

    #[test]
    fn an_error_that_differs_in_a_word_still_starts_again() {
        let g = RepeatGuard::default();
        let k = RepeatGuard::key("Bash", &json!({"command": "cargo test"}));
        fail(&g, &k, 2, "test parse_dates ... FAILED (finished in 0.41s)");
        // Another test fails now: a different failure, counted from 1
        g.after(&k, "error", "test parse_times ... FAILED (finished in 0.52s)");
        assert!(g.check(&k).is_none());
        g.after(&k, "error", "test parse_times ... FAILED (finished in 0.63s)");
        assert!(g.check(&k).is_none());
        g.after(&k, "error", "test parse_times ... FAILED (finished in 0.74s)");
        assert_eq!(g.check(&k).map(|s| s.count), Some(3));
        // Case is a difference too
        let g = RepeatGuard::default();
        fail(&g, &k, 2, "Error: not found");
        g.after(&k, "error", "error: not found");
        assert!(g.check(&k).is_none());
    }

    // end of 23a behaviour tests

    #[test]
    fn shape_makes_numbers_and_long_hex_ids_one_hash_and_keeps_the_words() {
        assert_eq!(shape("finished in 12.34s"), "finished in #.#s");
        assert_eq!(shape("Time: 2.3 s, pid 4242, port :38211"), "Time: #.# s, pid #, port :#");
        assert_eq!(shape("2026-11-16T09:10:11.123Z"), "#-#-#T#:#:#.#Z");
        assert_eq!(shape("at 3f786850e387550fdab836ed7e6dc881de23001b"), "at #");
        assert_eq!(shape("short 3f78685 and 3f7868"), "short # and #f#");
        assert_eq!(shape("id f47ac10b-58cc-4372-a567-0e02b2c3d479."), "id #.");
        assert_eq!(shape("/tmp/kwnt-1a2b3c4d/out"), "/tmp/kwnt-#/out");
        // Hex words without a digit, short hex groups and words next to a hyphen stay
        assert_eq!(shape("deadbeef cafe-face Failed"), "deadbeef cafe-face Failed");
        assert_eq!(shape("a-1234567 abcd-12"), "a-# abcd-#");
        assert_eq!(shape("0x1a2b3c4d"), "#x#");
        assert_eq!(shape("Ошибка 42 ✗"), "Ошибка # ✗");
        assert_eq!(shape(""), "");
        // The accepted tradeoff: counts are numbers too
        assert_eq!(shape("3 failed"), shape("2 failed"));
        assert_ne!(shape("test parse_dates failed"), shape("test parse_times failed"));
    }

    #[test]
    fn a_long_error_is_cut_to_200_characters() {
        let long = "e".repeat(500);
        let t = stop_text(3, &long);
        assert!(t.contains(&format!("\"{}… [truncated]\"", "e".repeat(200))));
        assert!(!t.contains(&"e".repeat(201)));
    }
}
