//! Show it once: the recorder script in a real Chrome/Chromium, typed into with real key events
//! (`Input.dispatchKeyEvent`, the in-app viewer's path), its payloads read by the server's own
//! parser. Covers S-F2: a field a script filled and the person then edited is masked. Skipped
//! when no Chrome is installed.
#[path = "support/chrome_guard.rs"]
mod chrome_guard;
use chrome_guard::ChromeGuard;
use kwnt::sandbox::{
    computer::{self, ComputerMode},
    service::BrowserService,
    BrowserController,
};
use kwnt::server::show_once::capture;
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

/// One field per case. `upper` reformats in the page's own input handler (an input mask).
const PAGE: &str = "<!doctype html><title>form</title><body>\
<input id=plain name=plain>\
<input id=filled name=filled>\
<input id=prefilled name=prefilled value=Prefilled>\
<input id=appended name=appended>\
<input id=upper name=upper oninput=\"this.value=this.value.toUpperCase()\">\
<input id=events name=events>\
<input id=pw name=pw type=password>\
<textarea id=area name=area></textarea>";

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
    format!("http://127.0.0.1:{port}/form")
}

/// Runs `expr` in the page's own world (what the page's scripts can do).
async fn page_js(ctrl: &BrowserController, expr: &str) -> Value {
    let r = ctrl.send_cdp_command("Runtime.evaluate", json!({"expression": expr, "returnByValue": true})).await.unwrap();
    r["result"]["value"].clone()
}

/// The person typing `text`, one real key event pair per character.
async fn person_types(ctrl: &BrowserController, text: &str) {
    for ch in text.chars() {
        let key = ch.to_string();
        let code = if ch.is_ascii_alphabetic() { format!("Key{}", ch.to_ascii_uppercase()) } else { String::new() };
        let vk = ch.to_ascii_uppercase() as u32;
        let down = json!({"type": "keyDown", "key": key, "code": code, "text": key, "unmodifiedText": key, "windowsVirtualKeyCode": vk});
        ctrl.send_cdp_command("Input.dispatchKeyEvent", down).await.unwrap();
        let up = json!({"type": "keyUp", "key": key, "code": code, "windowsVirtualKeyCode": vk});
        ctrl.send_cdp_command("Input.dispatchKeyEvent", up).await.unwrap();
        // the recorder's after-the-page update runs between keys, as it does for a person
        tokio::time::sleep(Duration::from_millis(30)).await;
    }
}

async fn focus(ctrl: &BrowserController, id: &str) {
    page_js(ctrl, &format!("document.getElementById('{id}').focus()")).await;
}

#[tokio::test]
async fn the_recorder_masks_any_field_whose_text_is_not_all_the_persons_own_typing() {
    std::env::remove_var("DISPLAY");
    std::env::remove_var("WAYLAND_DISPLAY");
    if computer::local_chrome_path().is_none() {
        eprintln!("skipped: no Chrome or Chromium installed");
        return;
    }
    let tmp = tempfile::tempdir().unwrap();
    let _chrome = ChromeGuard(tmp.path().join("local-chrome"));
    let service = BrowserService::new(tmp.path().to_path_buf(), None);
    service.set_mode(ComputerMode::Local).await;
    let ctrl = service.session("bot", tmp.path()).await.expect("local session");
    let url = serve_page().await;
    ctrl.navigate(&url).await.unwrap();

    // The recorder in its own world, as kwant installs it; `kwntRecord` there collects what it
    // sends (in the app it's the CDP binding)
    let tree = ctrl.send_cdp_command("Page.getFrameTree", json!({})).await.unwrap();
    let frame = tree["frameTree"]["frame"]["id"].as_str().unwrap().to_string();
    let world = ctrl.send_cdp_command("Page.createIsolatedWorld", json!({"frameId": frame, "worldName": capture::WORLD})).await.unwrap();
    let ctx = world["executionContextId"].clone();
    let in_world = |expr: String| {
        let ctrl = &ctrl;
        let ctx = ctx.clone();
        async move {
            let r = ctrl.send_cdp_command("Runtime.evaluate", json!({"expression": expr, "contextId": ctx, "returnByValue": true})).await.unwrap();
            assert!(r.get("exceptionDetails").is_none(), "{r}");
            r["result"]["value"].clone()
        }
    };
    in_world(format!("globalThis.__sent = []; globalThis.{} = function (s) {{ __sent.push(s); }}; true", capture::BINDING)).await;
    in_world(capture::recorder_js()).await;

    // plain: the person types into an empty field
    focus(&ctrl, "plain").await;
    person_types(&ctrl, "abc").await;
    // filled (S-F2): a script sets the value with no event, then the person types one character
    page_js(&ctrl, "document.getElementById('filled').value = 'Script text'").await;
    focus(&ctrl, "filled").await;
    person_types(&ctrl, "x").await;
    // prefilled: the page's own value attribute, then one character
    focus(&ctrl, "prefilled").await;
    person_types(&ctrl, "x").await;
    // appended: the person types, a script adds to it silently, the person types again
    focus(&ctrl, "appended").await;
    person_types(&ctrl, "ab").await;
    page_js(&ctrl, "document.getElementById('appended').value += 'Z'").await;
    person_types(&ctrl, "c").await;
    // upper: the page's input mask reformats every keystroke; still the person's own text
    focus(&ctrl, "upper").await;
    person_types(&ctrl, "ab").await;
    // events: a script fills with an input event (untrusted), then the person types
    page_js(&ctrl, "var e = document.getElementById('events'); e.value = 'From script'; e.dispatchEvent(new Event('input', {bubbles: true})); true").await;
    focus(&ctrl, "events").await;
    person_types(&ctrl, "y").await;
    // a password field the person types
    focus(&ctrl, "pw").await;
    person_types(&ctrl, "hunter").await;
    // a textarea the person types, left focused: it comes from the flush at the stop
    focus(&ctrl, "area").await;
    person_types(&ctrl, "note").await;

    let all = in_world(format!("JSON.stringify(__sent.concat({}))", capture::FLUSH)).await;
    let payloads: Vec<String> = serde_json::from_str(all.as_str().unwrap()).unwrap();
    let mut steps: BTreeMap<String, (Option<String>, Option<String>)> = BTreeMap::new();
    for p in &payloads {
        let c = capture::parse(p).unwrap_or_else(|| panic!("the server reads every payload: {p}"));
        if c.step.kind == "type" {
            steps.insert(c.step.label.clone().unwrap_or_default(), (c.step.value.clone(), c.step.masked.clone()));
        }
    }
    let raw = payloads.join("\n");
    for secret in ["Script text", "Prefilled", "From script", "abZ", "hunter"] {
        assert!(!raw.contains(secret), "{secret:?} left the page: {raw}");
    }
    let saved = |v: &str| (None::<String>, Some(v.to_string()));
    assert_eq!(steps.get("plain"), Some(&(Some("abc".to_string()), None)), "{steps:?}");
    assert_eq!(steps.get("filled"), Some(&saved("saved_login")), "S-F2: script fills, the person types one character");
    assert_eq!(steps.get("prefilled"), Some(&saved("saved_login")), "a prefilled value isn't the person's");
    assert_eq!(steps.get("appended"), Some(&saved("saved_login")), "a silent script change in the middle");
    assert_eq!(steps.get("upper"), Some(&(Some("AB".to_string()), None)), "the page's own input mask is still the person's text");
    assert_eq!(steps.get("events"), Some(&saved("saved_login")), "an untrusted input event");
    assert_eq!(steps.get("pw"), Some(&saved("password")));
    assert_eq!(steps.get("area"), Some(&(Some("note".to_string()), None)), "the pending edit at the stop");
}
