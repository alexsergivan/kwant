//! `scripts/perf_budget.py`'s `browser_launch_s` is bimodal on main: a cold local Chrome plus the
//! identity keeper ([`kwnt::sandbox::identity::keep`]) usually takes ~1.3 s, but sometimes stalls
//! for the keeper's own 8 s timeout, landing ~9.3 s. Reproduces that directly (no full `kwnt`
//! binary, no python harness): a fresh profile and a cold windowless Chrome per iteration, timed
//! from the probe starting Chrome through `keep_identity()` returning.

#[path = "support/skip.rs"]
mod skip;

use kwnt::sandbox::{browser::BrowserController, computer};
use std::time::{Duration, Instant};

/// Comfortably under `scripts/perf_budget.json`'s `browser_launch_s` limit (2.4 s), and far under
/// the identity keeper's own 8 s give-up: a launch that needs the keeper's timeout to move on is
/// the bug, not noise.
const BUDGET: Duration = Duration::from_secs(3);

#[tokio::test]
async fn cold_chrome_launches_stay_under_budget_every_time() {
    if computer::local_chrome_path().is_none() {
        skip::skip_or_fail("Chrome or Chromium", skip::CHROME_FIX);
        return;
    }
    let mut durations = Vec::new();
    for i in 0..6 {
        let tmp = tempfile::tempdir().unwrap();
        let profile = tmp.path().join("local-chrome");
        let started = Instant::now();
        let (endpoint, headless) = computer::ensure_local_chrome(&profile, true).await.expect("chrome starts");
        assert!(headless, "run {i}: must start windowless");
        let browser = BrowserController::at_endpoint(&endpoint, None, true);
        browser.keep_identity().await;
        let elapsed = started.elapsed();
        durations.push(elapsed);
        println!("run {i}: {elapsed:?}");
        let _ = browser.send_cdp_command("Browser.close", serde_json::json!({})).await;
        computer::wait_local_chrome_gone(&profile).await.ok();
    }
    assert!(
        durations.iter().all(|d| *d < BUDGET),
        "every cold launch must stay under {BUDGET:?}; got {durations:?}"
    );
}
