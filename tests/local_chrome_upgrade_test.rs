//! Upgrading kwant with its windowless Chrome still running: that Chrome was started without
//! kwant's user agent and tells sites it is "HeadlessChrome", so kwant restarts it once, and never
//! again after. Skipped when no Chrome is installed.
#[path = "support/chrome_guard.rs"]
mod chrome_guard;
use chrome_guard::ChromeGuard;
use kwnt::sandbox::{
    computer::{self, ComputerMode},
    service::BrowserService,
};
use std::time::Duration;

async fn page_agent(endpoint: &str) -> String {
    let version: serde_json::Value = reqwest::get(format!("{endpoint}/json/version")).await.unwrap().json().await.unwrap();
    version["User-Agent"].as_str().unwrap_or_default().to_string()
}

#[tokio::test]
async fn an_older_kwants_headless_chrome_is_restarted_once() {
    std::env::remove_var("DISPLAY");
    std::env::remove_var("WAYLAND_DISPLAY");
    let Some(chrome) = computer::local_chrome_path() else {
        eprintln!("skipped: no Chrome or Chromium installed");
        return;
    };
    let tmp = tempfile::tempdir().unwrap();
    let profile = tmp.path().join("local-chrome");
    std::fs::create_dir_all(&profile).unwrap();
    let _chrome = ChromeGuard(profile.clone());

    // what an older kwant left running: windowless, its own user agent, no launch record
    let mut old = std::process::Command::new(&chrome)
        .args(computer::chrome_args(&profile, true, None))
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .unwrap();
    let old_pid = old.id();
    std::thread::spawn(move || { let _ = old.wait(); });
    let mut endpoint = None;
    for _ in 0..80 {
        if let Some((e, _)) = computer::running_local_chrome(&profile).await { endpoint = Some(e); break; }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
    let endpoint = endpoint.expect("the older Chrome answers");
    assert!(page_agent(&endpoint).await.contains("HeadlessChrome"));
    assert_eq!(computer::launched_headless(&profile), None);

    let service = BrowserService::new(tmp.path().to_path_buf(), None);
    service.set_mode(ComputerMode::Local).await;
    service.session("bot", tmp.path()).await.expect("local session");
    let new_pid = computer::profile_owner(&profile).expect("a Chrome holds the profile");
    assert_ne!(new_pid, old_pid, "the older Chrome is restarted");
    let (endpoint, headless) = computer::running_local_chrome(&profile).await.expect("chrome running");
    assert!(headless);
    assert!(!page_agent(&endpoint).await.contains("Headless"));
    assert_eq!(computer::launched_headless(&profile), Some(true));

    // later calls keep it
    service.session("bot", tmp.path()).await.unwrap();
    service.session("other", tmp.path()).await.unwrap();
    assert_eq!(computer::profile_owner(&profile), Some(new_pid), "restarted only once");

    service.set_mode(ComputerMode::Vm).await;
    computer::wait_local_chrome_gone(&profile).await.expect("windowless Chrome closed");
}
