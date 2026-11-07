//! The one clock seam (next-077 decisions Q-X1): "now", shifted by `KWNT_TEST_NOW_OFFSET_SECS`
//! seconds in debug and test builds so tests can cross a time limit (Telegram's 10-minute pairing
//! code) without waiting. A release build never reads the variable.

use chrono::{DateTime, Duration, Utc};

/// The variable tests set; read on every call so a test can move the clock mid-run.
pub const OFFSET_ENV: &str = "KWNT_TEST_NOW_OFFSET_SECS";

/// The current time, plus the test offset in debug and test builds.
pub fn now() -> DateTime<Utc> {
    Utc::now() + offset()
}

fn offset() -> Duration {
    if !cfg!(debug_assertions) {
        return Duration::zero();
    }
    std::env::var(OFFSET_ENV)
        .ok()
        .and_then(|v| v.trim().parse::<i64>().ok())
        // At most ten years either way: a typo can't overflow chrono
        .map(|s| Duration::seconds(s.clamp(-315_360_000, 315_360_000)))
        .unwrap_or_else(Duration::zero)
}
