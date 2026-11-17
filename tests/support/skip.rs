//! Missing test preconditions: a test skips on a dev machine and fails in CI.
//!
//! A skip and a pass have the same exit code, and `cargo test` hides the output of passing tests,
//! so a test that returns early in CI looks green having run nothing. Every early return for a
//! missing tool, image or service goes through [`skip_or_fail`]; `scripts/check_test_skips.py`
//! fails CI on a `skipped` line printed anywhere else.
//!
//! Integration tests include this file with `#[path = "support/skip.rs"] mod skip;`; the lib includes
//! it once as `crate::test_skip` (src/lib.rs) and unit tests `use crate::test_skip as skip;`.
#![allow(dead_code)]

/// True in CI (`CI=true`, which GitHub Actions sets) or with `KWNT_TEST_REQUIRE=1`.
/// `KWNT_AGENT_TEST_REQUIRE=1` is the older name and still works.
pub fn strict() -> bool {
    ["CI", "KWNT_TEST_REQUIRE", "KWNT_AGENT_TEST_REQUIRE"].iter().any(|k| on(k))
}

fn on(key: &str) -> bool {
    std::env::var(key).is_ok_and(|v| !matches!(v.trim().to_ascii_lowercase().as_str(), "" | "0" | "false" | "no"))
}

/// Call right before a test returns early because `what` is missing. `fix` says how to provide it
/// (a command to run or a variable to set). In a strict run this panics with both; otherwise it
/// prints `skipped: …` and the test returns.
#[track_caller]
pub fn skip_or_fail(what: &str, fix: &str) {
    if strict() {
        panic!(
            "{what} is missing, and this run is strict (CI=true or KWNT_TEST_REQUIRE=1), so the test fails instead of \
             skipping. To provide it: {fix}. To skip on your own computer, unset CI and KWNT_TEST_REQUIRE."
        );
    }
    eprintln!("skipped: {what} is missing. To run it: {fix}");
}

/// A live test that runs only when `var` is set (a real API key, a server address). Returns its
/// value, or prints `skipped: …` and returns `None`. Never fails, in CI either: these tests are
/// opt-in and listed in `tests/NOT_IN_CI.md`.
pub fn opt_in(var: &str, what: &str) -> Option<String> {
    let v = std::env::var(var).ok().filter(|v| !v.is_empty());
    if v.is_none() {
        eprintln!("skipped: {what} runs only with {var} set (opt-in, see tests/NOT_IN_CI.md)");
    }
    v
}

/// What to install for the browser tests; the fix text every Chrome skip uses.
pub const CHROME_FIX: &str =
    "install Google Chrome or Chromium on PATH, or set KWNT_CHROME_PATH to its binary";

/// Fix text for the tests that need Docker and the agent image.
pub const AGENT_IMAGE_FIX: &str = "start Docker and run `scripts/ci_docker_tests.sh images`";
