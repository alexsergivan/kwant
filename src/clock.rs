//! The one test clock (decisions.md Q-X1). Every time window that a test has to cross, such as
//! the 24 h template Undo, the 1 h suggestion expiry, the 10 min Telegram pairing code and the
//! 7-day show-once step deletion, reads "now" from [`now`]. Nobody adds a second clock.
//!
//! In a release build, [`now`] is `Utc::now()`. In debug and test builds it is moved by two
//! things, added together:
//! - `KWNT_TEST_NOW_OFFSET_SECS`, a signed number of seconds, read on every call, for a whole
//!   server (QA runs and e2e);
//! - [`set_test_offset`], for tests inside one process. It is process-wide, so a test that uses it
//!   lives in its own test binary.

use chrono::{DateTime, Duration, Utc};
use std::sync::atomic::{AtomicI64, Ordering};

/// The environment variable that moves [`now`] in debug and test builds.
pub const TEST_NOW_OFFSET_ENV: &str = "KWNT_TEST_NOW_OFFSET_SECS";

static TEST_OFFSET_S: AtomicI64 = AtomicI64::new(0);

/// "Now" for any expiry or window a test must be able to cross.
pub fn now() -> DateTime<Utc> {
    let real = Utc::now();
    // An offset past chrono's range leaves the real clock rather than panicking
    Duration::try_seconds(offset_secs()).and_then(|d| real.checked_add_signed(d)).unwrap_or(real)
}

/// How far [`now`] is moved, in seconds: always 0 in a release build.
pub fn offset_secs() -> i64 {
    if !cfg!(debug_assertions) {
        return 0;
    }
    combine(std::env::var(TEST_NOW_OFFSET_ENV).ok().as_deref(), TEST_OFFSET_S.load(Ordering::SeqCst))
}

/// The env value (ignored unless it parses as whole seconds) plus the in-process offset.
fn combine(env: Option<&str>, in_process: i64) -> i64 {
    env.and_then(|v| v.trim().parse::<i64>().ok()).unwrap_or(0).saturating_add(in_process)
}

/// Moves [`now`] by `secs` seconds inside this process; 0 puts it back. Ignored in a release
/// build.
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
    fn now_is_utc_now_when_nothing_moved_it() {
        if offset_secs() == 0 {
            let (a, n, b) = (Utc::now(), now(), Utc::now());
            assert!(a <= n && n <= b);
        }
    }
}
