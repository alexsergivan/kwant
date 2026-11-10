//! Dev mode (docs/dev-mode.md, plan 0.7): a space watches one GitHub repo. It reads it, and posts
//! a review card to its PR only when the person presses Post (`post`, 0.7.1). The poller
//! writes a pending card per merge to the watched branch and per open PR head, with its DevFacts;
//! `generate` turns them into cards. The production floor lives in `crate::mcp::prod_floor`.
//!
//! Lin owns `facts`, `routes`, `poller`, `post`, `repo`, `backup` and `floor`; Vega owns `generate`, `landed`,
//! `review`, `risk`, `tamper`, `notes` and `source`; Juno owns `scan`, `forge` and
//! `atlassian`; `tickets` is split per its api.md §1.1 (Juno: `source`, `pickup`, `store`, `routes`;
//! Vega: `brief*`, `tools`, `links`, `mission`, `prompt`; `hooks` and `seam` are the join).

pub mod atlassian;
pub mod backup;
pub mod facts;
pub mod floor;
pub mod forge;
pub mod git;
pub mod generate;
pub mod landed;
pub mod line_notes;
pub mod notes;
pub mod poller;
pub mod post;
pub mod prepush;
pub mod repo;
pub mod review;
pub mod risk;
pub mod routes;
pub mod scan;
pub mod source;
pub mod tamper;
pub mod tickets;

pub use generate::generate;

use crate::db::dev_mode::{DevCardRow, CALLS_CAP};
use crate::drivers::events::RuntimeEvent;
use crate::server::AppState;

/// Sends `dev_card {space_id, id, kind, status}` on the event stream.
pub fn notify_card(state: &AppState, row: &DevCardRow) {
    if let Some((keys, pr_url)) = tickets::store::card_keys(row) {
        tokio::spawn(tickets::store::on_card(state.clone(), row.clone(), keys, pr_url));
    }
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

/// How many days a pushed branch or open PR counts as fresh, so it gets a review card:
/// `KWANT_DEV_REVIEW_DAYS` (1–90), else [`poller::REVIEW_FRESH_DAYS`] (7).
pub fn review_days() -> u32 {
    static DAYS: std::sync::OnceLock<u32> = std::sync::OnceLock::new();
    *DAYS.get_or_init(|| {
        let raw = std::env::var("KWANT_DEV_REVIEW_DAYS").ok();
        let days = review_days_from(raw.as_deref());
        if raw.is_some() {
            tracing::info!("dev: KWANT_DEV_REVIEW_DAYS read as {days} (whole days from 1 to 90; anything else is {})", poller::REVIEW_FRESH_DAYS);
        }
        days
    })
}

/// [`review_days`] from the variable's text: a whole number from 1 to 90, else the default.
pub fn review_days_from(raw: Option<&str>) -> u32 {
    raw.and_then(|r| r.trim().parse::<u32>().ok()).filter(|d| (1..=90).contains(d)).unwrap_or(poller::REVIEW_FRESH_DAYS)
}

/// The most disk one repo's git mirror may use: `KWANT_DEV_MIRROR_CAP_BYTES` (1 B–1 TB, so a test
/// can use a tiny cap), else [`git::MIRROR_CAP`] (1 GB).
pub fn mirror_cap() -> u64 {
    static CAP: std::sync::OnceLock<u64> = std::sync::OnceLock::new();
    *CAP.get_or_init(|| env_num("KWANT_DEV_MIRROR_CAP_BYTES", 1, 1 << 40).unwrap_or(git::MIRROR_CAP))
}

/// Where the secrets scan downloads gitleaks from: `KWANT_DEV_GITLEAKS_URL` (an http(s) base; the
/// pinned checksums still apply), else GitHub's release page.
pub fn scan_gitleaks_base() -> Option<&'static str> {
    static URL: std::sync::OnceLock<Option<String>> = std::sync::OnceLock::new();
    URL.get_or_init(|| env_url("KWANT_DEV_GITLEAKS_URL")).as_deref()
}

/// Where the package check looks names up: `KWANT_DEV_REGISTRIES_URL` (one http(s) base for all
/// registries, see [`scan::packages::Registries::all_at`]), else npm, PyPI and crates.io.
pub fn scan_registries_base() -> Option<&'static str> {
    static URL: std::sync::OnceLock<Option<String>> = std::sync::OnceLock::new();
    URL.get_or_init(|| env_url("KWANT_DEV_REGISTRIES_URL")).as_deref()
}

fn env_url(name: &str) -> Option<String> {
    let raw = std::env::var(name).ok()?;
    let ok = url::Url::parse(raw.trim()).is_ok_and(|u| matches!(u.scheme(), "http" | "https") && u.host().is_some());
    if ok {
        tracing::info!("dev: {name} is set");
        Some(raw.trim().trim_end_matches('/').to_string())
    } else {
        tracing::warn!("dev: {name} must be an http(s) URL; the default is used");
        None
    }
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
        assert_eq!(review_days(), 7);
        assert!((now() - Utc::now()).num_seconds().abs() <= 1);
    }

    #[test]
    fn the_review_days_variable_is_held_to_1_to_90() {
        assert_eq!(review_days_from(None), 7);
        for (raw, days) in [("1", 1), ("14", 14), (" 30 ", 30), ("90", 90)] {
            assert_eq!(review_days_from(Some(raw)), days, "{raw:?}");
        }
        // Out of range or not a whole number of days: the default, never 0 (which would skip
        // every review) and never more than 90
        for raw in ["0", "91", "365", "-3", "7.5", "", "seven", "4294967296"] {
            assert_eq!(review_days_from(Some(raw)), 7, "{raw:?}");
        }
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
