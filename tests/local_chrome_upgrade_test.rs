//! Upgrading kwant with its windowless Chrome still running: kwant 0.9.4 started it with a made-up
//! `--user-agent` and marked automated (`navigator.webdriver`), which Google refuses to sign in,
//! so kwant restarts it once as Chrome, and never again after. Skipped when no Chrome is installed.
#[path = "support/chrome_guard.rs"]
mod chrome_guard;
use chrome_guard::ChromeGuard;
use kwnt::sandbox::{
    computer::{self, ComputerMode},
    service::BrowserService,
};
use std::time::Duration;

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

    // what kwant 0.9.4 left running: windowless, a made-up user agent, no launch record
    let old_args = [
        format!("--user-data-dir={}", profile.display()),
        "--remote-debugging-port=0".into(),
        "--remote-debugging-address=127.0.0.1".into(),
        "--no-first-run".into(),
        "--no-default-browser-check".into(),
        "--headless=new".into(),
        "--user-agent=Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/141.0.0.0 Safari/537.36".into(),
        "about:blank".into(),
    ];
    let mut old = std::process::Command::new(&chrome)
        .args(old_args)
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
    endpoint.expect("the older Chrome answers");
    assert_eq!(computer::launched_headless(&profile), None);
    assert!(computer::started_by_older_kwant(&profile));

    let service = BrowserService::new(tmp.path().to_path_buf(), None);
    service.set_mode(ComputerMode::Local).await;
    service.session("bot", tmp.path()).await.expect("local session");
    let new_pid = computer::profile_owner(&profile).expect("a Chrome holds the profile");
    assert_ne!(new_pid, old_pid, "the older Chrome is restarted");
    let (_, headless) = computer::running_local_chrome(&profile).await.expect("chrome running");
    assert!(headless);
    assert!(!computer::started_by_older_kwant(&profile), "started as Chrome now");
    assert_eq!(computer::launched_headless(&profile), Some(true));

    // later calls keep it
    service.session("bot", tmp.path()).await.unwrap();
    service.session("other", tmp.path()).await.unwrap();
    assert_eq!(computer::profile_owner(&profile), Some(new_pid), "restarted only once");

    service.set_mode(ComputerMode::Vm).await;
    computer::wait_local_chrome_gone(&profile).await.expect("windowless Chrome closed");
}
