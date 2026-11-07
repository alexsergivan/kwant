//! The one test clock (decisions.md Q-X1). Every time window that a test has to cross, such as
//! the 24 h template Undo, the 1 h suggestion expiry, the 10 min Telegram pairing code and the
//! 7-day show-once step deletion, reads "now" from [`now`]. Nobody adds a second clock.
//!
//! Without the `test-seams` feature, [`now`] is `Utc::now()` and nothing can move it: a release
//! build, a `make run` debug build and the Docker image all ignore both seams below. Only
//! `cargo test` turns the feature on (Cargo.toml's dev-dependency on this crate), or a QA build made
//! with `--features test-seams`. There it is moved by two things, added together:
//! - `KWNT_TEST_NOW_OFFSET_SECS`, a signed number of seconds, read on every call, for a whole
//!   server (QA runs and e2e);
//! - [`set_test_offset`], for tests inside one process. It is process-wide, so a test that uses it
//!   lives in its own test binary.

use chrono::{DateTime, Duration, Utc};
use std::sync::atomic::{AtomicI64, Ordering};

/// The environment variable that moves [`now`] when the test seams are built in.
pub const TEST_NOW_OFFSET_ENV: &str = "KWNT_TEST_NOW_OFFSET_SECS";

/// Whether this build has the test seams (the `test-seams` feature). False in every shipped build.
pub const TEST_SEAMS: bool = cfg!(feature = "test-seams");

static TEST_OFFSET_S: AtomicI64 = AtomicI64::new(0);

/// "Now" for any expiry or window a test must be able to cross.
pub fn now() -> DateTime<Utc> {
    let real = Utc::now();
    // An offset past chrono's range leaves the real clock rather than panicking
    Duration::try_seconds(offset_secs()).and_then(|d| real.checked_add_signed(d)).unwrap_or(real)
}

/// How far [`now`] is moved, in seconds: always 0 without the test seams.
pub fn offset_secs() -> i64 {
    seam_offset(TEST_SEAMS, std::env::var(TEST_NOW_OFFSET_ENV).ok().as_deref(), TEST_OFFSET_S.load(Ordering::SeqCst))
}

/// The offset a build with (`seams`) or without the test seams uses.
fn seam_offset(seams: bool, env: Option<&str>, in_process: i64) -> i64 {
    if seams {
        combine(env, in_process)
    } else {
        0
    }
}

/// The env value (ignored unless it parses as whole seconds) plus the in-process offset.
fn combine(env: Option<&str>, in_process: i64) -> i64 {
    env.and_then(|v| v.trim().parse::<i64>().ok()).unwrap_or(0).saturating_add(in_process)
}

/// Moves [`now`] by `secs` seconds inside this process; 0 puts it back. Ignored without the test
/// seams.
#[doc(hidden)]
pub fn set_test_offset(secs: i64) {
    TEST_OFFSET_S.store(secs, Ordering::SeqCst);
}

#[cfg(test)]
mod tests {
    use super::*;

    // Pure on purpose: the clock is process-wide, and other unit tests in this binary read it.
    #[test]
    fn the_env_and_the_in_process_offset_add_up() {
        assert_eq!(combine(None, 0), 0);
        assert_eq!(combine(Some(" 60 "), 3600), 3660);
        assert_eq!(combine(Some("-90"), 0), -90);
        assert_eq!(combine(Some("not a number"), 3600), 3600, "a bad value is ignored");
        assert_eq!(combine(Some("1.5"), 0), 0, "whole seconds only");
        assert_eq!(combine(Some(&i64::MAX.to_string()), 1), i64::MAX, "saturates");
        assert!(Duration::try_seconds(i64::MAX).is_none(), "so now() falls back to the real clock");
    }

    #[test]
    fn a_build_without_the_test_seams_ignores_both_offsets() {
        assert_eq!(seam_offset(false, Some("601"), 3600), 0);
        assert_eq!(seam_offset(true, Some("601"), 3600), 4201);
    }

    #[test]
    fn the_test_seams_are_never_on_by_default() {
        // Only `cargo test` (the dev-dependency) or an explicit QA build may turn them on
        let toml = include_str!("../Cargo.toml");
        let features = toml.split("[features]").nth(1).and_then(|s| s.split("\n[").next()).unwrap_or("");
        let default = features.lines().find(|l| l.trim_start().starts_with("default")).unwrap_or("");
        assert!(!default.contains("test-seams"), "test-seams must not be a default feature: {default}");
        assert!(toml.contains("kwnt = { path = \".\", features = [\"test-seams\"] }"), "tests turn it on through the dev-dependency");
        const { assert!(TEST_SEAMS, "this test binary was built with the seams") };
    }

    #[test]
    fn now_is_utc_now_when_nothing_moved_it() {
        if offset_secs() == 0 {
            let (a, n, b) = (Utc::now(), now(), Utc::now());
            assert!(a <= n && n <= b);
        }
    }
}
