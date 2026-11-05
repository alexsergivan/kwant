//! Dev mode (docs/dev-mode.md, plan 0.7): a space watches one GitHub repo, read only. The poller
//! writes a pending card per merge to the watched branch and per open PR head, with its DevFacts;
//! `generate` turns them into cards. The production floor lives in `crate::mcp::prod_floor`.
//!
//! Lin owns `facts`, `routes`, `poller`, `repo`, `backup` and `floor`; Vega owns `generate`, `landed`,
//! `review`, `risk`, `tamper` and `notes`.

pub mod backup;
pub mod facts;
pub mod floor;
pub mod generate;
pub mod landed;
pub mod notes;
pub mod poller;
pub mod repo;
pub mod review;
pub mod risk;
pub mod routes;
pub mod tamper;

pub use generate::generate;

use crate::db::dev_mode::{DevCardRow, CALLS_CAP};
use crate::drivers::events::RuntimeEvent;
use crate::server::AppState;

/// Sends `dev_card {space_id, id, kind, status}` on the event stream.
pub fn notify_card(state: &AppState, row: &DevCardRow) {
    let _ = state.event_sender.send(RuntimeEvent::DevCard {
        space_id: row.space_id.clone(),
        id: row.id.clone(),
        kind: row.kind.clone(),
        status: row.status.clone(),
    });
}

/// Sends `dev_mode {space_id, on}` on the event stream.
pub fn notify_mode(state: &AppState, space_id: &str, on: bool) {
    let _ = state.event_sender.send(RuntimeEvent::DevMode { space_id: space_id.into(), on });
}

// Test seams (QA plan §2.2), read here and nowhere else. Unset, Dev mode runs as documented.

/// How often each Dev mode space is read: `KWANT_DEV_POLL_SECS` (5–86400), else 300.
pub fn poll_every() -> std::time::Duration {
    static SECS: std::sync::OnceLock<u64> = std::sync::OnceLock::new();
    std::time::Duration::from_secs(*SECS.get_or_init(|| env_num("KWANT_DEV_POLL_SECS", 5, 86_400).unwrap_or(poller::EVERY.as_secs())))
}

/// Most model calls Dev mode makes per space per UTC day: `KWANT_DEV_CALL_CAP` (0–1000), else 30.
pub fn call_cap() -> i64 {
    static CAP: std::sync::OnceLock<i64> = std::sync::OnceLock::new();
    *CAP.get_or_init(|| env_num("KWANT_DEV_CALL_CAP", 0, 1000).map_or(CALLS_CAP, |v| v as i64))
}

fn env_num(name: &str, min: u64, max: u64) -> Option<u64> {
    let raw = std::env::var(name).ok()?;
    let v = raw.trim().parse::<u64>().ok().filter(|v| (min..=max).contains(v));
    match v {
        Some(v) => tracing::info!("dev: {name}={v}"),
        None => tracing::warn!("dev: {name} must be a whole number from {min} to {max}; the default is used"),
    }
    v
}

static CLOCK_SKEW_S: std::sync::atomic::AtomicI64 = std::sync::atomic::AtomicI64::new(0);

/// Dev mode's clock: when a backup was taken, whether it is still fresh (10 minutes), and the
/// daily cap's day. The real clock unless a test moved it with [`set_clock_skew`].
pub fn now() -> chrono::DateTime<chrono::Utc> {
    chrono::Utc::now() + chrono::Duration::seconds(CLOCK_SKEW_S.load(std::sync::atomic::Ordering::SeqCst))
}

/// Moves [`now`] `skew_s` seconds from the real clock; 0 puts it back. For tests only, and
/// process-wide: a test that uses it lives in its own test binary (see tests/dev_seams_test.rs).
#[doc(hidden)]
pub fn set_clock_skew(skew_s: i64) {
    CLOCK_SKEW_S.store(skew_s, std::sync::atomic::Ordering::SeqCst);
}

/// The daily cap's day: the UTC calendar day of `now`, `YYYY-MM-DD`, wherever the server is.
pub fn utc_day(now: chrono::DateTime<chrono::Utc>) -> String {
    now.format("%Y-%m-%d").to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{DateTime, Utc};

    fn at(rfc3339: &str) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339(rfc3339).unwrap().with_timezone(&Utc)
    }

    #[test]
    fn the_cap_day_is_the_utc_day() {
        assert_eq!(utc_day(at("2026-11-05T23:59:59Z")), "2026-11-05");
        assert_eq!(utc_day(at("2026-11-06T00:00:00Z")), "2026-11-06");
        // Late evening in California is already tomorrow in UTC; early morning in India, yesterday
        assert_eq!(utc_day(at("2026-11-05T17:30:00-08:00")), "2026-11-06");
        assert_eq!(utc_day(at("2026-11-06T03:00:00+05:30")), "2026-11-05");
    }

    #[test]
    fn unset_seams_keep_the_defaults() {
        // CI sets neither variable
        assert_eq!(poll_every(), std::time::Duration::from_secs(300));
        assert_eq!(call_cap(), 30);
        assert!((now() - Utc::now()).num_seconds().abs() <= 1);
    }

    #[tokio::test]
    async fn the_daily_cap_resets_at_midnight_utc() {
        let tmp = tempfile::tempdir().unwrap();
        let db = crate::db::Database::init(tmp.path()).await.unwrap();
        let late = at("2026-11-05T23:59:59Z");
        assert!(db.take_dev_call_at("s1", late, 2).await.unwrap());
        assert!(db.take_dev_call_at("s1", late, 2).await.unwrap());
        assert!(!db.take_dev_call_at("s1", late, 2).await.unwrap(), "the cap holds for the rest of the UTC day");
        // 16:00 in California on the 5th is midnight UTC: a new day
        let midnight = at("2026-11-05T16:00:00-08:00");
        assert!(db.take_dev_call_at("s1", midnight, 2).await.unwrap());
        assert_eq!(db.dev_calls_on("s1", "2026-11-05").await.unwrap(), 2);
        assert_eq!(db.dev_calls_on("s1", "2026-11-06").await.unwrap(), 1);
        assert!(db.take_dev_call_at("s2", late, 2).await.unwrap(), "each space has its own cap");
    }
}
