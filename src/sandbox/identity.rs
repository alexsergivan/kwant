//! What the bots' Chrome tells sites it is, on every page, popup and worker.
//!
//! Windowless Chrome says "HeadlessChrome" in its user agent. kwant used to replace the string
//! with `--user-agent`, which also empties the high-entropy client hints a real Chrome always
//! fills (`sec-ch-ua-full-version-list`, `navigator.userAgentData.getHighEntropyValues`): Google
//! asks for them and refused the sign-in. Now Chrome starts with its own user agent, and a keeper
//! on each browser sets `Network.setUserAgentOverride` with the same user agent minus the
//! headless token **and** the browser's own client hints (read from the browser, so the version,
//! platform version, architecture and bitness are the real ones), for every target as it is
//! created, with auto-attach, before it runs. Nothing is injected into pages: it says what the
//! browser really is. A Chrome with a window already says that, so it is left alone.
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::{LazyLock, Mutex};
use std::time::Duration;

/// The user agent and client hints a target is given.
#[derive(Debug, Clone, PartialEq)]
pub struct Identity {
    pub user_agent: String,
    /// CDP `Emulation.UserAgentMetadata`.
    pub metadata: Value,
}

/// The user agent with the headless token gone: what the same Chrome sends with a window.
pub fn headed(user_agent: &str) -> String {
    user_agent.replace("HeadlessChrome/", "Chrome/")
}

/// Whether a user agent says the browser runs without a window.
pub fn is_headless(user_agent: &str) -> bool {
    user_agent.contains("HeadlessChrome")
}

/// The full version in a `Browser.getVersion` product ("Chrome/154.0.8037.92",
/// "HeadlessChrome/131.0.6778.86").
pub fn product_version(product: &str) -> Option<&str> {
    let version = product.rsplit('/').next()?.trim();
    let ok = version.split('.').count() == 4 && version.split('.').all(|n| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()));
    ok.then_some(version)
}

/// Chrome's GREASE brand for `major` (`GetGreasedUserAgentBrandVersion`): a made-up brand that
/// changes with the version so sites don't match brand lists exactly.
fn grease(major: u32) -> (String, &'static str) {
    const CHARS: [&str; 11] = [" ", "(", ":", "-", ".", "/", ")", ";", "=", "?", "_"];
    const VERSIONS: [&str; 3] = ["8", "99", "24"];
    let brand = format!("Not{}A{}Brand", CHARS[major as usize % 11], CHARS[(major as usize + 1) % 11]);
    (brand, VERSIONS[major as usize % 3])
}

/// The `brands` and `fullVersionList` Chrome `full_version` sends, in Chrome's order
/// (`GenerateBrandVersionList`, seeded with the major version): the GREASE brand, "Chromium",
/// and `product` ("Google Chrome") when there is one.
pub fn brand_lists(full_version: &str, product: Option<&str>) -> Option<(Value, Value)> {
    let major_text = full_version.split('.').next()?;
    let major: u32 = major_text.parse().ok()?;
    let (grease_brand, grease_major) = grease(major);
    let mut entries = vec![(grease_brand, grease_major.to_string(), format!("{grease_major}.0.0.0"))];
    entries.push(("Chromium".into(), major_text.into(), full_version.into()));
    if let Some(product) = product {
        entries.push((product.into(), major_text.into(), full_version.into()));
    }
    let order: &[usize] = if entries.len() == 3 {
        const ORDERS: [[usize; 3]; 6] = [[0, 1, 2], [0, 2, 1], [1, 0, 2], [1, 2, 0], [2, 0, 1], [2, 1, 0]];
        &ORDERS[major as usize % 6]
    } else {
        const ORDERS: [[usize; 2]; 2] = [[0, 1], [1, 0]];
        &ORDERS[major as usize % 2]
    };
    // entry i goes to position order[i]
    let mut placed = vec![None; entries.len()];
    for (i, entry) in entries.into_iter().enumerate() {
        placed[order[i]] = Some(entry);
    }
    let placed: Vec<_> = placed.into_iter().flatten().collect();
    let brands = placed.iter().map(|(b, major, _)| json!({"brand": b, "version": major})).collect();
    let full = placed.iter().map(|(b, _, full)| json!({"brand": b, "version": full})).collect();
    Some((Value::Array(brands), Value::Array(full)))
}

/// The client-hint platform for a Rust `std::env::consts::OS`.
pub fn hint_platform(os: &str) -> &'static str {
    match os {
        "macos" => "macOS",
        "windows" => "Windows",
        _ => "Linux",
    }
}

/// The client-hint architecture and bitness for a Rust `std::env::consts::ARCH`.
pub fn hint_arch(arch: &str) -> (&'static str, &'static str) {
    match arch {
        "aarch64" => ("arm", "64"),
        "arm" => ("arm", "32"),
        "x86" => ("x86", "32"),
        _ => ("x86", "64"),
    }
}

/// The identity of the Chrome that answered `Browser.getVersion` with `product` and
/// `user_agent`, built from that and this computer: used when the browser's own client hints
/// can't be read. `google` for a Google Chrome (brand "Google Chrome"), else Chromium.
pub fn from_version(product: &str, user_agent: &str, google: bool, os: &str, arch: &str, platform_version: &str) -> Option<Identity> {
    let full = product_version(product)?;
    let (brands, full_list) = brand_lists(full, google.then_some("Google Chrome"))?;
    let (architecture, bitness) = hint_arch(arch);
    Some(Identity {
        user_agent: headed(user_agent),
        metadata: json!({
            "brands": brands,
            "fullVersionList": full_list,
            "fullVersion": full,
            "platform": hint_platform(os),
            "platformVersion": platform_version,
            "architecture": architecture,
            "bitness": bitness,
            "model": "",
            "mobile": false,
            "wow64": false,
        }),
    })
}

/// The identity from what the browser itself reports (`navigator.userAgentData` with its
/// high-entropy values, read on a `chrome://` page): kept as it is, except that a "HeadlessChrome"
/// brand is the product it stands for, and a list missing that product is rebuilt in Chrome's
/// order. `google` says which product this Chrome is when the report doesn't.
pub fn from_report(user_agent: &str, report: &Value, google: bool) -> Option<Identity> {
    let list = |key: &str| report[key].as_array().cloned();
    let (brands, full_list) = (list("brands")?, list("fullVersionList")?);
    let names = |l: &[Value]| l.iter().filter_map(|b| b["brand"].as_str().map(str::to_string)).collect::<Vec<_>>();
    let product = if google || names(&brands).iter().any(|b| b == "Google Chrome" || b == "HeadlessChrome") {
        Some("Google Chrome")
    } else {
        None
    };
    let full = full_list
        .iter()
        .find(|b| b["brand"] == "Chromium" || b["brand"] == "Google Chrome" || b["brand"] == "HeadlessChrome")
        .and_then(|b| b["version"].as_str())
        .filter(|v| v.split('.').count() == 4)?
        .to_string();
    let rename = |l: Vec<Value>| -> Vec<Value> {
        l.into_iter()
            .map(|mut b| {
                if b["brand"] == "HeadlessChrome" {
                    b["brand"] = json!(product.unwrap_or("Chromium"));
                }
                b
            })
            .collect()
    };
    let (mut brands, mut full_list) = (rename(brands), rename(full_list));
    let complete = |l: &[Value]| {
        let n = names(l);
        n.iter().any(|b| b == "Chromium") && product.is_none_or(|p| n.iter().any(|b| b == p))
    };
    if !complete(&brands) || !complete(&full_list) {
        let (b, f) = brand_lists(&full, product)?;
        brands = b.as_array().cloned().unwrap_or_default();
        full_list = f.as_array().cloned().unwrap_or_default();
    }
    let text = |key: &str| report[key].as_str().unwrap_or_default().to_string();
    Some(Identity {
        user_agent: headed(user_agent),
        metadata: json!({
            "brands": brands,
            "fullVersionList": full_list,
            "fullVersion": full,
            "platform": text("platform"),
            "platformVersion": text("platformVersion"),
            "architecture": text("architecture"),
            "bitness": text("bitness"),
            "model": text("model"),
            "mobile": report["mobile"].as_bool().unwrap_or(false),
            "wow64": report["wow64"].as_bool().unwrap_or(false),
        }),
    })
}

/// This computer's OS version as Chrome reports it on a Mac ("15.1.0"). Elsewhere empty: Chrome
/// on Linux sends "" too, and Windows' value can't be worked out from here.
fn local_platform_version() -> String {
    if !cfg!(target_os = "macos") {
        return String::new();
    }
    let out = std::process::Command::new("sw_vers").arg("-productVersion").output();
    let version = out.map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string()).unwrap_or_default();
    let mut parts: Vec<&str> = version.split('.').filter(|p| !p.is_empty()).collect();
    while !parts.is_empty() && parts.len() < 3 {
        parts.push("0");
    }
    parts.join(".")
}

/// The target types given the identity: pages and frames, and every kind of worker, which send
/// their own requests.
fn takes_identity(kind: &str) -> bool {
    matches!(kind, "page" | "iframe" | "worker" | "shared_worker" | "service_worker" | "other")
}

/// Targets that can have frames or workers of their own, which their own auto-attach reaches.
fn has_children(kind: &str) -> bool {
    matches!(kind, "page" | "iframe")
}

static KEEPERS: LazyLock<Mutex<HashMap<String, tokio::task::JoinHandle<()>>>> = LazyLock::new(Default::default);

/// Keeps the identity on every target of the browser at `browser_ws` (its browser-level DevTools
/// address), unless a keeper already runs for it, and waits briefly until the pages already open
/// have it. Ends with the browser's connection; the next call starts it again.
pub async fn keep(browser_ws: String) {
    let ready = {
        let mut keepers = KEEPERS.lock().unwrap_or_else(|e| e.into_inner());
        keepers.retain(|_, task| !task.is_finished());
        if keepers.contains_key(&browser_ws) {
            return;
        }
        let (tx, rx) = tokio::sync::oneshot::channel();
        keepers.insert(browser_ws.clone(), tokio::spawn(run(browser_ws, tx)));
        rx
    };
    let _ = tokio::time::timeout(Duration::from_secs(8), ready).await;
}

/// One CDP connection, with replies picked out of the events it also carries.
struct Conn {
    ws: super::browser::WsStream,
    next: u64,
    /// events that arrived while waiting for a reply, handled afterwards
    queued: Vec<Value>,
}

impl Conn {
    async fn send(&mut self, method: &str, params: Value, session: Option<&str>) -> Option<u64> {
        use futures::SinkExt;
        self.next += 1;
        let mut message = json!({"id": self.next, "method": method, "params": params});
        if let Some(session) = session {
            message["sessionId"] = json!(session);
        }
        let text = tokio_tungstenite::tungstenite::Message::Text(message.to_string());
        self.ws.send(text).await.ok().map(|_| self.next)
    }

    async fn recv(&mut self) -> Option<Value> {
        if !self.queued.is_empty() {
            return Some(self.queued.remove(0));
        }
        self.recv_raw().await
    }

    /// Sends and waits up to `wait` for the reply's result. Events meanwhile are queued.
    async fn call(&mut self, method: &str, params: Value, session: Option<&str>, wait: Duration) -> Option<Value> {
        let id = self.send(method, params, session).await?;
        let deadline = tokio::time::Instant::now() + wait;
        let mut held = Vec::new();
        let reply = loop {
            let Ok(Some(message)) = tokio::time::timeout_at(deadline, self.recv_raw()).await else { break None };
            if message["id"] == id {
                break message.get("result").cloned();
            }
            if message.get("method").is_some() {
                held.push(message);
            }
        };
        self.queued.extend(held);
        reply
    }

    async fn recv_raw(&mut self) -> Option<Value> {
        use futures::StreamExt;
        loop {
            match self.ws.next().await? {
                Ok(tokio_tungstenite::tungstenite::Message::Text(text)) => return serde_json::from_str(&text).ok(),
                Ok(_) => continue,
                Err(_) => return None,
            }
        }
    }
}

const REPORT_SCRIPT: &str = r#"navigator.userAgentData ? navigator.userAgentData.getHighEntropyValues(["architecture","bitness","model","platformVersion","fullVersionList","wow64"]).then(v => JSON.stringify(v)) : """#;

/// The browser's own client hints, read on a background `chrome://version` page (a secure page
/// that runs no site's code), which is closed again.
async fn read_report(conn: &mut Conn) -> Option<Value> {
    let wait = Duration::from_secs(5);
    let created = conn.call("Target.createTarget", json!({"url": "chrome://version", "background": true}), None, wait).await?;
    let target = created["targetId"].as_str()?.to_string();
    let report = async {
        let attached = conn.call("Target.attachToTarget", json!({"targetId": target, "flatten": true}), None, wait).await?;
        let session = attached["sessionId"].as_str()?.to_string();
        for _ in 0..20 {
            let params = json!({"expression": REPORT_SCRIPT, "awaitPromise": true, "returnByValue": true});
            let result = conn.call("Runtime.evaluate", params, Some(&session), wait).await;
            let text = result.as_ref().and_then(|r| r["result"]["value"].as_str()).unwrap_or_default().to_string();
            if let Ok(report) = serde_json::from_str::<Value>(&text) {
                return Some(report);
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        None
    }
    .await;
    let _ = conn.call("Target.closeTarget", json!({"targetId": target}), None, wait).await;
    report
}

/// Whether the Chrome kwant starts on this computer is a Google Chrome.
fn local_is_google() -> bool {
    super::computer::local_chrome_path().is_some_and(|p| super::computer::is_google_chrome(&p))
}

async fn identity(conn: &mut Conn, version: &Value) -> Option<Identity> {
    let user_agent = version["userAgent"].as_str().unwrap_or_default().to_string();
    if let Some(report) = read_report(conn).await {
        if let Some(identity) = from_report(&user_agent, &report, false) {
            return Some(identity);
        }
    }
    tracing::debug!("identity: the browser's client hints couldn't be read; building them");
    let google = tokio::task::spawn_blocking(local_is_google).await.unwrap_or(false);
    let platform_version = tokio::task::spawn_blocking(local_platform_version).await.unwrap_or_default();
    from_version(
        version["product"].as_str().unwrap_or_default(),
        &user_agent,
        google,
        std::env::consts::OS,
        std::env::consts::ARCH,
        &platform_version,
    )
}

async fn run(browser_ws: String, ready: tokio::sync::oneshot::Sender<()>) {
    let Ok(ws) = super::browser::connect_cdp(&browser_ws).await else {
        tracing::debug!("identity: couldn't connect to the browser");
        return;
    };
    let mut conn = Conn { ws, next: 0, queued: Vec::new() };
    let Some(version) = conn.call("Browser.getVersion", json!({}), None, Duration::from_secs(5)).await else { return };
    if !is_headless(version["userAgent"].as_str().unwrap_or_default()) {
        // a Chrome with a window already says what it is: nothing to set. Held open, so this
        // browser isn't asked again on every connect.
        let _ = ready.send(());
        while conn.recv().await.is_some() {}
        return;
    }
    let Some(identity) = identity(&mut conn, &version).await else {
        tracing::warn!("identity: no version from the browser; its pages keep Chrome's own user agent");
        let _ = ready.send(());
        while conn.recv().await.is_some() {}
        return;
    };
    tracing::info!(user_agent = %identity.user_agent, "identity: every page gets the browser's own identity without the headless token");
    let attach = json!({"autoAttach": true, "waitForDebuggerOnStart": true, "flatten": true});
    let Some(attach_id) = conn.send("Target.setAutoAttach", attach.clone(), None).await else { return };
    let override_params = json!({"userAgent": identity.user_agent, "userAgentMetadata": identity.metadata});
    let mut ready = Some(ready);
    let mut attached_all = false;
    // overrides sent for the targets already open, not yet answered
    let mut pending: std::collections::HashSet<u64> = Default::default();
    while let Some(message) = conn.recv().await {
        if let Some(id) = message["id"].as_u64() {
            if id == attach_id {
                attached_all = true;
            }
            pending.remove(&id);
            if attached_all && pending.is_empty() {
                if let Some(ready) = ready.take() {
                    let _ = ready.send(());
                }
            }
            continue;
        }
        if message["method"] != "Target.attachedToTarget" {
            continue;
        }
        let params = &message["params"];
        let Some(session) = params["sessionId"].as_str() else { continue };
        let kind = params["targetInfo"]["type"].as_str().unwrap_or_default();
        if takes_identity(kind) {
            if let Some(id) = conn.send("Network.setUserAgentOverride", override_params.clone(), Some(session)).await {
                if !attached_all {
                    pending.insert(id);
                }
            }
        }
        if has_children(kind) {
            let _ = conn.send("Target.setAutoAttach", attach.clone(), Some(session)).await;
        }
        if params["waitingForDebugger"].as_bool().unwrap_or(false) {
            let _ = conn.send("Runtime.runIfWaitingForDebugger", json!({}), Some(session)).await;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(list: &Value) -> Vec<String> {
        list.as_array().unwrap().iter().map(|b| format!("{}={}", b["brand"].as_str().unwrap(), b["version"].as_str().unwrap())).collect()
    }

    const LINUX_HEADLESS_UA: &str = "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) HeadlessChrome/154.0.0.0 Safari/537.36";

    #[test]
    fn the_brand_lists_match_what_chromium_154_sends() {
        // read from Chromium 154.0.8037.92 on the server (header echo and getHighEntropyValues)
        let (brands, full) = brand_lists("154.0.8037.92", None).unwrap();
        assert_eq!(names(&brands), ["Not A(Brand=99", "Chromium=154"]);
        assert_eq!(names(&full), ["Not A(Brand=99.0.0.0", "Chromium=154.0.8037.92"]);
    }

    #[test]
    fn google_chrome_has_three_brands_in_chromes_order() {
        let (brands, full) = brand_lists("131.0.6778.86", Some("Google Chrome")).unwrap();
        // Chrome 131's own header: "Google Chrome";v="131", "Chromium";v="131", "Not_A Brand";v="24"
        assert_eq!(names(&brands), ["Google Chrome=131", "Chromium=131", "Not_A Brand=24"]);
        assert_eq!(names(&full)[0], "Google Chrome=131.0.6778.86");
        let (brands, _) = brand_lists("154.0.8037.92", Some("Google Chrome")).unwrap();
        assert!(names(&brands).contains(&"Google Chrome=154".to_string()));
        assert!(!names(&brands).iter().any(|b| b.contains("Headless")));
    }

    #[test]
    fn built_from_get_version_for_chrome_and_chromium_on_each_platform() {
        let mac = from_version("Chrome/131.0.6778.86", "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) HeadlessChrome/131.0.0.0 Safari/537.36", true, "macos", "aarch64", "15.1.0").unwrap();
        assert_eq!(mac.user_agent, "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/131.0.0.0 Safari/537.36");
        assert_eq!(mac.metadata["platform"], "macOS");
        assert_eq!(mac.metadata["platformVersion"], "15.1.0");
        assert_eq!(mac.metadata["architecture"], "arm");
        assert_eq!(mac.metadata["bitness"], "64");
        assert!(names(&mac.metadata["brands"]).contains(&"Google Chrome=131".to_string()));

        let win = from_version("Chrome/140.0.7339.80", "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) HeadlessChrome/140.0.0.0 Safari/537.36", true, "windows", "x86_64", "").unwrap();
        assert_eq!(win.metadata["platform"], "Windows");
        assert_eq!(win.metadata["architecture"], "x86");
        assert!(!win.user_agent.contains("Headless"));
        assert!(names(&win.metadata["fullVersionList"]).contains(&"Google Chrome=140.0.7339.80".to_string()));

        let linux = from_version("Chrome/154.0.8037.92", LINUX_HEADLESS_UA, false, "linux", "aarch64", "").unwrap();
        assert_eq!(linux.metadata["platform"], "Linux");
        assert_eq!(names(&linux.metadata["brands"]), ["Not A(Brand=99", "Chromium=154"]);
        for identity in [&mac, &win, &linux] {
            assert_eq!(identity.metadata["mobile"], false);
            assert_eq!(identity.metadata["model"], "");
            assert!(!identity.metadata.to_string().contains("Headless"));
        }
        assert_eq!(from_version("HeadlessChrome", LINUX_HEADLESS_UA, false, "linux", "x86_64", ""), None);
    }

    #[test]
    fn a_report_from_the_browser_is_kept_as_it_is() {
        // Chromium 154 windowless on the server, read on chrome://version
        let report = json!({"architecture":"arm","bitness":"64","brands":[{"brand":"Not A(Brand","version":"99"},{"brand":"Chromium","version":"154"}],"fullVersionList":[{"brand":"Not A(Brand","version":"99.0.0.0"},{"brand":"Chromium","version":"154.0.8037.92"}],"mobile":false,"model":"","platform":"Linux","platformVersion":"","wow64":false});
        let identity = from_report(LINUX_HEADLESS_UA, &report, false).unwrap();
        assert_eq!(identity.user_agent, "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/154.0.0.0 Safari/537.36");
        assert_eq!(identity.metadata["brands"], report["brands"]);
        assert_eq!(identity.metadata["fullVersionList"], report["fullVersionList"]);
        assert_eq!(identity.metadata["architecture"], "arm");
        assert_eq!(identity.metadata["fullVersion"], "154.0.8037.92");
    }

    #[test]
    fn a_headless_brand_in_the_report_becomes_google_chrome() {
        let report = json!({"architecture":"arm","bitness":"64","brands":[{"brand":"HeadlessChrome","version":"131"},{"brand":"Chromium","version":"131"},{"brand":"Not_A Brand","version":"24"}],"fullVersionList":[{"brand":"HeadlessChrome","version":"131.0.6778.86"},{"brand":"Chromium","version":"131.0.6778.86"},{"brand":"Not_A Brand","version":"24.0.0.0"}],"mobile":false,"model":"","platform":"macOS","platformVersion":"15.1.0","wow64":false});
        let identity = from_report("Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) HeadlessChrome/131.0.0.0 Safari/537.36", &report, false).unwrap();
        assert_eq!(names(&identity.metadata["brands"]), ["Google Chrome=131", "Chromium=131", "Not_A Brand=24"]);
        assert_eq!(identity.metadata["platformVersion"], "15.1.0");
        assert!(!identity.metadata.to_string().contains("Headless"));
        // a report missing the product it should name is rebuilt in Chrome's order
        let partial = json!({"brands":[{"brand":"Chromium","version":"131"}],"fullVersionList":[{"brand":"Chromium","version":"131.0.6778.86"}],"platform":"macOS"});
        let identity = from_report("HeadlessChrome/131.0.0.0", &partial, true).unwrap();
        assert_eq!(names(&identity.metadata["brands"]), ["Google Chrome=131", "Chromium=131", "Not_A Brand=24"]);
        // no version anywhere: nothing to build from
        assert_eq!(from_report("x", &json!({"brands":[],"fullVersionList":[]}), false), None);
    }

    #[test]
    fn product_versions() {
        assert_eq!(product_version("Chrome/154.0.8037.92"), Some("154.0.8037.92"));
        assert_eq!(product_version("HeadlessChrome/131.0.6778.86"), Some("131.0.6778.86"));
        assert_eq!(product_version("Chrome/154"), None);
        assert!(is_headless(LINUX_HEADLESS_UA));
        assert!(!is_headless(&headed(LINUX_HEADLESS_UA)));
    }

    /// Real check, opt-in: starts Chrome the way local mode does (windowless unless
    /// `KWNT_REAL_HEADED` is set, which needs a display), keeps its identity as a live view's
    /// connection does, then runs `KWNT_REAL_IDENTITY_DRIVER` with the browser's DevTools address
    /// as its last argument, which checks the pages, popups and workers.
    #[tokio::test]
    #[ignore]
    async fn real_chrome_keeps_its_identity() {
        let Ok(driver) = std::env::var("KWNT_REAL_IDENTITY_DRIVER") else { return };
        let headless = std::env::var("KWNT_REAL_HEADED").is_err();
        let profile = tempfile::tempdir().unwrap();
        let (endpoint, windowless) = super::super::computer::ensure_local_chrome(profile.path(), headless).await.unwrap();
        assert_eq!(windowless, headless);
        let browser = super::super::browser::BrowserController::at_endpoint(&endpoint, None, true);
        browser.keep_identity().await;
        let version: Value = reqwest::get(format!("{endpoint}/json/version")).await.unwrap().json().await.unwrap();
        let ws = version["webSocketDebuggerUrl"].as_str().unwrap().to_string();
        let status = tokio::process::Command::new("sh").arg("-c").arg(format!("{driver} '{ws}'")).status().await.unwrap();
        let _ = browser.send_cdp_command("Browser.close", json!({})).await;
        assert!(status.success(), "the driver failed");
    }
}
