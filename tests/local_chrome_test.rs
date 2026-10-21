//! Local mode against a real Chrome/Chromium: it runs without a window, pages render and take
//! input at the viewer's 1280x800 size, and the window switch reopens the same page in a real
//! window and back. Skipped when no Chrome is installed; the window part needs Xvfb on Linux.
use kwnt::sandbox::{
    computer::{self, ComputerMode},
    service::BrowserService,
};
use kwnt::server::browser_stream::BrowserInput;
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

const PAGE: &str = "<!doctype html><title>before</title><body style=margin:0>\
<button id=b style='position:absolute;left:0;top:0;width:400px;height:300px' \
onclick=\"document.title='clicked '+innerWidth+'x'+innerHeight\">press</button>";

/// A one-page HTTP server on 127.0.0.1, so the test needs no network.
async fn serve_page() -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    tokio::spawn(async move {
        while let Ok((mut stream, _)) = listener.accept().await {
            tokio::spawn(async move {
                let mut buf = [0u8; 2048];
                let _ = stream.read(&mut buf).await;
                let reply = format!(
                    "HTTP/1.1 200 OK\r\ncontent-type: text/html\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{PAGE}",
                    PAGE.len()
                );
                let _ = stream.write_all(reply.as_bytes()).await;
            });
        }
    });
    format!("http://127.0.0.1:{port}/page")
}

async fn title_becomes(ctrl: &kwnt::sandbox::BrowserController, want: &str) -> String {
    let mut title = String::new();
    for _ in 0..40 {
        if let Some((_, t)) = ctrl.sync_location().await {
            title = t;
            if title == want { break; }
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    title
}

#[tokio::test]
async fn local_chrome_runs_windowless_and_can_open_as_a_window() {
    std::env::remove_var("DISPLAY");
    std::env::remove_var("WAYLAND_DISPLAY");
    if computer::local_chrome_path().is_none() {
        eprintln!("skipped: no Chrome or Chromium installed");
        return;
    }
    let tmp = tempfile::tempdir().unwrap();
    let profile = tmp.path().join("local-chrome");
    let service = BrowserService::new(tmp.path().to_path_buf(), None);
    service.set_mode(ComputerMode::Local).await;
    let url = serve_page().await;

    // windowless: no display is needed, and the page gets the viewer's viewport
    service.session("other", tmp.path()).await.expect("first session");
    let ctrl = service.session("bot", tmp.path()).await.expect("local session");
    let (endpoint, headless) = computer::running_local_chrome(&profile).await.expect("chrome running");
    assert!(headless, "local Chrome must start without a window");
    ctrl.navigate(&url).await.unwrap();
    assert!(!ctrl.capture_screenshot().await.unwrap().is_empty());
    // the same input path the in-app viewer uses
    ctrl.dispatch_input(BrowserInput::Click { x: 100.0, y: 100.0 }).await.unwrap();
    assert_eq!(title_becomes(&ctrl, "clicked 1280x800").await, "clicked 1280x800");
    // the other bot's page must still paint while this one is in front: a background tab never
    // does once its viewer override is gone, so look at it over a fresh connection
    let sessions: serde_json::Value = serde_json::from_slice(
        &std::fs::read(tmp.path().join("browser-sessions-local.json")).unwrap()).unwrap();
    let other_target = sessions["other"]["target"].as_str().expect("other bot's page").to_string();
    let other = kwnt::sandbox::BrowserController::at_endpoint(&endpoint, Some(other_target), false);
    let shot = tokio::time::timeout(Duration::from_secs(5), other.capture_screenshot()).await;
    assert!(matches!(shot, Ok(Ok(ref s)) if !s.is_empty()), "screenshot of the other bot's page: {shot:?}");

    // the window switch, where a display exists (Xvfb stands in for a desktop)
    let xvfb = std::process::Command::new("Xvfb").args([":97", "-screen", "0", "1440x900x24"])
        .stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null()).spawn();
    if let Ok(mut xvfb) = xvfb {
        tokio::time::sleep(Duration::from_millis(800)).await;
        std::env::set_var("DISPLAY", ":97");
        service.set_local_window(true).await.expect("open as a window");
        assert!(service.local_window());
        let (_, headless) = computer::running_local_chrome(&profile).await.expect("chrome running");
        assert!(!headless, "the window form must be a real window");
        // only the bot's real page is open: Chrome's blank start window is not left on top, and
        // the other bot's never-used blank page is not reopened as a window of its own
        let (endpoint, _) = computer::running_local_chrome(&profile).await.unwrap();
        let browser = kwnt::sandbox::BrowserController::at_endpoint(&endpoint, None, true);
        let targets = browser.send_cdp_command("Target.getTargets", serde_json::json!({})).await.unwrap();
        let pages: Vec<_> = targets["targetInfos"].as_array().unwrap().iter()
            .filter(|t| t["type"] == "page").map(|t| t["url"].as_str().unwrap_or("").to_string()).collect();
        assert_eq!(pages, vec![url.clone()], "pages in the window form");
        // the other bot still gets a page when it asks for one
        service.session("other", tmp.path()).await.expect("other bot's page on demand");
        // the bot's page was reopened there
        let ctrl = service.session("bot", tmp.path()).await.unwrap();
        ctrl.refresh_location().await.unwrap();
        assert_eq!(ctrl.current_url.lock().await.as_deref(), Some(url.as_str()));

        // and back into the app, again with the page
        service.set_local_window(false).await.expect("back to the app");
        let (_, headless) = computer::running_local_chrome(&profile).await.expect("chrome running");
        assert!(headless);
        let ctrl = service.session("bot", tmp.path()).await.unwrap();
        ctrl.refresh_location().await.unwrap();
        assert_eq!(ctrl.current_url.lock().await.as_deref(), Some(url.as_str()));
        let _ = xvfb.kill();
        let _ = xvfb.wait();
    } else {
        eprintln!("window part skipped: no Xvfb");
    }

    // leaving local mode closes the windowless Chrome, since nothing would show it
    service.set_mode(ComputerMode::Vm).await;
    computer::wait_local_chrome_gone(&profile).await.expect("windowless Chrome closed");
}
