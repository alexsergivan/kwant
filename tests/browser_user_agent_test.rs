//! The user agent for `web_fetch` asks the local Chrome for its version without stalling the async
//! worker it runs on. Its own test binary, since the answer is worked out once per process.
#![cfg(unix)]
#[path = "common/exe.rs"]
mod exe;

use kwnt::sandbox::computer;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;
use std::time::Duration;

#[tokio::test(flavor = "current_thread")]
async fn a_slow_chrome_version_does_not_block_the_worker() {
    // a Chrome whose `--version` takes a second, like a snap wrapper on a cold start
    let dir = tempfile::tempdir().unwrap();
    let chrome = dir.path().join("chrome");
    exe::write_executable(&chrome, "#!/bin/sh\nsleep 1\necho 'Chromium 150.1.2.3'\n");
    std::env::set_var(computer::CHROME_PATH_ENV, &chrome);

    // on a one-thread runtime, this only ticks while the probe leaves the thread free
    let ticks = Arc::new(AtomicU32::new(0));
    let counter = ticks.clone();
    let ticker = tokio::spawn(async move {
        loop {
            tokio::time::sleep(Duration::from_millis(20)).await;
            counter.fetch_add(1, Ordering::SeqCst);
        }
    });
    let agent = computer::browser_user_agent().await;
    ticker.abort();
    assert!(agent.contains(" Chrome/150.0.0.0 "), "{agent}");
    assert!(ticks.load(Ordering::SeqCst) >= 20, "the worker was blocked: {} ticks", ticks.load(Ordering::SeqCst));
}
