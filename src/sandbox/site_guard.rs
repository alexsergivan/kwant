//! Which sites a bot's browser may open: a per-bot allow-list and block-list of domains, and an
//! opt-in block on private and loopback addresses. The idea comes from browser-use's security
//! watchdog (MIT); no code is shared.
//!
//! The rules hold for every top-level page a bot's actions load: `browser_open`, a link it clicks,
//! a redirect, a form it submits, and a new tab its page opens. The person's own live control is
//! never checked. A guard task intercepts top-level document requests over its own DevTools
//! connection ([`run`]); it runs only while the driving bot has rules, so a bot without rules
//! browses exactly as before.
//!
//! What it doesn't cover: subframes (an iframe on an allowed page loads from anywhere), a page's
//! own fetches, images and scripts, and a name whose DNS answer changes between the check and
//! Chrome's own lookup (DNS rebinding). The private-address check resolves names on the computer
//! running kwnt, which in VM mode may answer differently from the browser container.

use futures::{SinkExt, StreamExt};
use serde::Serialize;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::net::IpAddr;
use std::sync::{Arc, Mutex, Weak};
use tokio::sync::RwLock;
use tokio_tungstenite::tungstenite::Message;

/// Most entries one list may hold.
pub const MAX_SITES: usize = 200;

/// Rule names, as the bot's settings call them and errors name them.
pub const ALLOWED: &str = "browser_allowed_sites";
pub const BLOCKED: &str = "browser_blocked_sites";
pub const PRIVATE: &str = "browser_block_private_network";

/// One bot's site rules. All empty and off: no checks at all.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SiteRules {
    /// When not empty, only these sites (and their subdomains) open.
    pub allowed: Vec<String>,
    /// These never open; wins over `allowed`.
    pub blocked: Vec<String>,
    /// Refuse loopback, private, link-local and CGNAT addresses, and names that resolve to them.
    pub block_private: bool,
}

/// Why a page wasn't opened.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Refusal {
    /// [`ALLOWED`], [`BLOCKED`] or [`PRIVATE`].
    pub rule: &'static str,
    pub host: String,
    pub url: String,
    /// The block-list entry that matched.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub entry: Option<String>,
    pub message: String,
}

/// An entry as stored: lowercase host (`example.com`), or `*.example.com` for subdomains only.
/// A pasted URL keeps its host. `Err` says what's wrong with it.
pub fn normalize_site(entry: &str) -> Result<String, String> {
    let raw = entry.trim().to_ascii_lowercase();
    let bad = || format!("\"{}\" isn't a site: use a domain such as example.com or *.example.com", entry.trim());
    let (wild, rest) = match raw.strip_prefix("*.") {
        Some(rest) => (true, rest),
        None => (false, raw.as_str()),
    };
    if rest.is_empty() || rest.contains('*') {
        return Err(bad());
    }
    let with_scheme = if rest.contains("://") { rest.to_string() } else { format!("http://{rest}") };
    let parsed = url::Url::parse(&with_scheme).map_err(|_| bad())?;
    let host = match parsed.host() {
        Some(url::Host::Domain(d)) => d.trim_end_matches('.').to_string(),
        Some(url::Host::Ipv4(ip)) if !wild => ip.to_string(),
        Some(url::Host::Ipv6(ip)) if !wild => format!("[{ip}]"),
        _ => return Err(bad()),
    };
    if host.is_empty() || host.starts_with('.') {
        return Err(bad());
    }
    Ok(if wild { format!("*.{host}") } else { host })
}

/// [`normalize_site`] for a whole list, without duplicates, in the given order.
pub fn normalize_sites(entries: &[String], field: &str) -> Result<Vec<String>, String> {
    if entries.len() > MAX_SITES {
        return Err(format!("{field} holds at most {MAX_SITES} sites"));
    }
    let mut out: Vec<String> = Vec::new();
    for entry in entries {
        let site = normalize_site(entry).map_err(|e| format!("{field}: {e}"))?;
        if !out.contains(&site) {
            out.push(site);
        }
    }
    Ok(out)
}

/// Whether `entry` covers `host`: `example.com` covers it and its subdomains, `*.example.com`
/// only the subdomains.
pub fn site_matches(entry: &str, host: &str) -> bool {
    match entry.strip_prefix("*.") {
        Some(base) => host.ends_with(&format!(".{base}")),
        // An address is only itself; a name covers its subdomains
        None => host == entry || (!entry.starts_with('[') && entry.parse::<IpAddr>().is_err() && host.ends_with(&format!(".{entry}"))),
    }
}

impl SiteRules {
    pub fn is_empty(&self) -> bool {
        self.allowed.is_empty() && self.blocked.is_empty() && !self.block_private
    }

    /// Whether the bot may open `url` as a page. Only web addresses are checked; `about:blank`,
    /// `data:` and the like reach no site.
    pub fn check(&self, url: &str) -> Result<(), Refusal> {
        self.check_with(url, &system_resolve)
    }

    /// [`Self::check`] with the name lookup supplied (tests).
    pub fn check_with(&self, url: &str, resolve: &dyn Fn(&str, u16) -> Option<Vec<IpAddr>>) -> Result<(), Refusal> {
        if self.is_empty() {
            return Ok(());
        }
        let Ok(parsed) = url::Url::parse(url.trim()) else {
            return Ok(());
        };
        if !matches!(parsed.scheme(), "http" | "https" | "ws" | "wss") {
            return Ok(());
        }
        let host = match parsed.host() {
            Some(url::Host::Domain(d)) => d.trim_end_matches('.').to_ascii_lowercase(),
            Some(url::Host::Ipv4(ip)) => ip.to_string(),
            Some(url::Host::Ipv6(ip)) => format!("[{ip}]"),
            None => String::new(),
        };
        let refuse = |rule, entry: Option<String>, message: String| Refusal {
            rule,
            host: host.clone(),
            url: url.to_string(),
            entry,
            message,
        };
        if let Some(entry) = self.blocked.iter().find(|e| site_matches(e, &host)) {
            return Err(refuse(
                BLOCKED,
                Some(entry.clone()),
                format!("Not opened: {host} is on this bot's blocked sites ({BLOCKED} has \"{entry}\")."),
            ));
        }
        if !self.allowed.is_empty() && !self.allowed.iter().any(|e| site_matches(e, &host)) {
            return Err(refuse(
                ALLOWED,
                None,
                format!(
                    "Not opened: {host} isn't on this bot's allowed sites ({ALLOWED}: {}). Work within those sites, or ask the person to add this one.",
                    self.allowed.join(", ")
                ),
            ));
        }
        if self.block_private && matches!(parsed.scheme(), "http" | "https") {
            if let Err(why) = crate::mcp::fetch_risk::public_only_with(url, resolve) {
                return Err(refuse(
                    PRIVATE,
                    None,
                    format!("Not opened: {why}, and this bot may not open local or private network addresses ({PRIVATE} is on)."),
                ));
            }
        }
        Ok(())
    }
}

fn system_resolve(name: &str, port: u16) -> Option<Vec<IpAddr>> {
    use std::net::ToSocketAddrs;
    let (tx, rx) = std::sync::mpsc::channel();
    let target = format!("{name}:{port}");
    std::thread::spawn(move || {
        let found = target.to_socket_addrs().map(|a| a.map(|s| s.ip()).collect::<Vec<_>>()).ok();
        let _ = tx.send(found);
    });
    rx.recv_timeout(std::time::Duration::from_secs(2)).ok().flatten()
}

/// Who caused the browser's latest action: the bot (checked) or the person (never checked).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Actor {
    #[default]
    Bot,
    Person,
}

#[derive(Default)]
struct GuardState {
    /// The bot whose rules apply: the last one to act in this browser.
    driver: Option<String>,
    rules: Option<Arc<SiteRules>>,
    actor: Actor,
    refusal: Option<Refusal>,
    task: Option<tokio::task::JoinHandle<()>>,
}

/// A browser's site rules and the task that enforces them; see the module docs.
#[derive(Default)]
pub struct SiteGuard {
    state: Mutex<GuardState>,
}

impl Drop for SiteGuard {
    fn drop(&mut self) {
        if let Some(task) = self.lock().task.take() {
            task.abort();
        }
    }
}

impl SiteGuard {
    fn lock(&self) -> std::sync::MutexGuard<'_, GuardState> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }

    pub fn driver(&self) -> Option<String> {
        self.lock().driver.clone()
    }

    pub fn set_driver(&self, kwant_id: &str) {
        self.lock().driver = Some(kwant_id.to_string());
    }

    pub fn rules(&self) -> Option<Arc<SiteRules>> {
        self.lock().rules.clone()
    }

    pub fn actor(&self) -> Actor {
        self.lock().actor
    }

    pub fn set_actor(&self, actor: Actor) {
        self.lock().actor = actor;
    }

    /// The last page the guard refused, taken (each refusal is reported once).
    pub fn take_refusal(&self) -> Option<Refusal> {
        self.lock().refusal.take()
    }

    fn refused(&self, refusal: Refusal) {
        self.lock().refusal = Some(refusal);
    }

    /// Sets the rules; `true` when the guard task must (re)start for them.
    pub(crate) fn set_rules(&self, rules: SiteRules) -> bool {
        let mut s = self.lock();
        if rules.is_empty() {
            s.rules = None;
            if let Some(task) = s.task.take() {
                task.abort();
            }
            return false;
        }
        s.rules = Some(Arc::new(rules));
        s.task.as_ref().is_none_or(|t| t.is_finished())
    }

    pub(crate) fn set_task(&self, task: tokio::task::JoinHandle<()>) {
        if let Some(old) = self.lock().task.replace(task) {
            old.abort();
        }
    }

    /// Whether the guard task is intercepting now.
    pub fn is_running(&self) -> bool {
        self.lock().task.as_ref().is_some_and(|t| !t.is_finished())
    }

    /// The bot's rules on `url`, with the name lookup off the async runtime.
    pub async fn check(&self, url: &str) -> Result<(), Refusal> {
        let Some(rules) = self.rules() else {
            return Ok(());
        };
        let url = url.to_string();
        match tokio::task::spawn_blocking(move || rules.check(&url)).await {
            Ok(result) => result,
            Err(_) => Ok(()),
        }
    }
}

type WsStream = tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;
type Sink = futures::stream::SplitSink<WsStream, Message>;

const DOCUMENTS: &str = r#"{"patterns":[{"urlPattern":"*","resourceType":"Document","requestStage":"Request"}]}"#;

async fn send(sink: &tokio::sync::Mutex<Sink>, id: u64, method: &str, params: Value, session: Option<&str>) -> bool {
    let mut msg = json!({"id": id, "method": method, "params": params});
    if let Some(s) = session {
        msg["sessionId"] = json!(s);
    }
    sink.lock().await.send(Message::Text(msg.to_string())).await.is_ok()
}

/// The guard task: one browser-level DevTools connection, attached to the bot's page (`page`)
/// and, paused before they load anything, to every tab that page (or one of its tabs) opens.
/// Each pauses top-level document requests; the driving bot's rules decide them while the
/// person isn't in control (`control` write-locked, or the person acted last). `ready` gets
/// `true` once interception is on. Returns when the connection or the page goes away.
pub(crate) async fn run(
    guard: Weak<SiteGuard>,
    browser_ws: String,
    page: String,
    control: Arc<RwLock<()>>,
    ready: tokio::sync::oneshot::Sender<bool>,
) {
    let Ok(mut ws) = super::browser::connect_cdp(&browser_ws).await else {
        tracing::warn!("site guard: couldn't connect to the browser");
        let _ = ready.send(false);
        return;
    };
    let documents: Value = serde_json::from_str(DOCUMENTS).unwrap_or_default();
    // Set up in order, keeping the events that arrive meanwhile for the loop
    let mut early: Vec<Value> = Vec::new();
    let mut id = 0u64;
    let mut call = |method: &str, params: Value, session: Option<&str>| {
        id += 1;
        let mut msg = json!({"id": id, "method": method, "params": params});
        if let Some(s) = session {
            msg["sessionId"] = json!(s);
        }
        (id, msg)
    };
    let (attach, msg) = call("Target.attachToTarget", json!({"targetId": page, "flatten": true}), None);
    let Some(reply) = request(&mut ws, attach, msg, &mut early).await else {
        let _ = ready.send(false);
        return;
    };
    let Some(main) = reply["result"]["sessionId"].as_str().map(str::to_string) else {
        tracing::warn!("site guard: couldn't attach to the bot's page: {}", reply["error"]);
        let _ = ready.send(false);
        return;
    };
    let (enable, msg) = call("Fetch.enable", documents.clone(), Some(&main));
    let enabled = request(&mut ws, enable, msg, &mut early).await.is_some_and(|r| r.get("error").is_none());
    let (auto, msg) = call("Target.setAutoAttach", json!({"autoAttach": true, "waitForDebuggerOnStart": true, "flatten": true}), None);
    let auto_on = request(&mut ws, auto, msg, &mut early).await.is_some_and(|r| r.get("error").is_none());
    let _ = ready.send(enabled && auto_on);
    if !(enabled && auto_on) {
        tracing::warn!("site guard: the browser refused interception");
        return;
    }

    let (sink, stream) = ws.split();
    let sink = Arc::new(tokio::sync::Mutex::new(sink));
    let ids = Arc::new(std::sync::atomic::AtomicU64::new(id + 1));
    let next = || ids.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    // session id -> target id, for the bot's page and the tabs it opened
    let mut ours: HashMap<String, String> = HashMap::from([(main.clone(), page.clone())]);
    let later = stream.filter_map(|msg| async move {
        match msg {
            Ok(Message::Text(text)) => Some(serde_json::from_str::<Value>(&text).unwrap_or_default()),
            Ok(_) => Some(Value::Null),
            Err(_) => None,
        }
    });
    let mut messages = futures::stream::iter(early).chain(later).boxed();
    while let Some(m) = messages.next().await {
        let params = &m["params"];
        let session = m["sessionId"].as_str();
        match m["method"].as_str().unwrap_or_default() {
            "Target.attachedToTarget" => {
                let info = &params["targetInfo"];
                let (Some(child), Some(target)) = (params["sessionId"].as_str(), info["targetId"].as_str()) else { continue };
                if child == main {
                    continue;
                }
                let opener = info["openerId"].as_str().unwrap_or_default();
                let opened_by_ours = !opener.is_empty() && ours.values().any(|t| t == opener);
                if info["type"] == "page" && opened_by_ours {
                    ours.insert(child.to_string(), target.to_string());
                    let _ = send(&sink, next(), "Fetch.enable", documents.clone(), Some(child)).await;
                    let _ = send(&sink, next(), "Runtime.runIfWaitingForDebugger", json!({}), Some(child)).await;
                } else {
                    // Another bot's tab, a worker, the browser's own UI or a second view of the
                    // bot's page: let it go untouched
                    if params["waitingForDebugger"] == json!(true) {
                        let _ = send(&sink, next(), "Runtime.runIfWaitingForDebugger", json!({}), Some(child)).await;
                    }
                    let _ = send(&sink, next(), "Target.detachFromTarget", json!({"sessionId": child}), None).await;
                }
            }
            "Target.detachedFromTarget" => {
                if let Some(s) = params["sessionId"].as_str() {
                    if s == main {
                        return;
                    }
                    ours.remove(s);
                }
            }
            "Fetch.requestPaused" => {
                let Some(session) = session.map(str::to_string) else { continue };
                let request_id = params["requestId"].as_str().unwrap_or_default().to_string();
                let url = params["request"]["url"].as_str().unwrap_or_default().to_string();
                let top = ours.get(&session).is_some_and(|t| params["frameId"] == json!(t));
                // A tab the bot's page opened, not the page itself
                let opened = (session != main).then(|| ours.get(&session).cloned()).flatten();
                let Some(g) = guard.upgrade() else { return };
                let person = g.actor() == Actor::Person || control.try_read().is_err();
                let rules = if top && !person { g.rules() } else { None };
                drop(g);
                        let (sink, guard, id, page) = (sink.clone(), guard.clone(), next(), page.clone());
                let (close_id, front_id) = (next(), next());
                tokio::spawn(async move {
                    let verdict = match rules {
                        Some(rules) => {
                            let u = url.clone();
                            tokio::task::spawn_blocking(move || rules.check(&u)).await.unwrap_or(Ok(()))
                        }
                        None => Ok(()),
                    };
                    match verdict {
                        Ok(()) => {
                            send(&sink, id, "Fetch.continueRequest", json!({"requestId": request_id}), Some(&session)).await;
                        }
                        Err(refusal) => {
                            tracing::info!(rule = refusal.rule, host = %refusal.host, "site guard refused a page");
                            if let Some(g) = guard.upgrade() {
                                g.refused(refusal);
                            }
                            send(&sink, id, "Fetch.failRequest", json!({"requestId": request_id, "errorReason": "BlockedByClient"}), Some(&session)).await;
                            // A refused new tab would only show an error, and in front it stops the
                            // bot's page (now in the background) from drawing: close it
                            if let Some(tab) = opened {
                                send(&sink, close_id, "Target.closeTarget", json!({"targetId": tab}), None).await;
                                send(&sink, front_id, "Target.activateTarget", json!({"targetId": page}), None).await;
                            }
                        }
                    }
                });
            }
            _ => {}
        }
    }
}

/// Sends `msg` and waits up to 5 s for its reply, keeping the events that come first.
async fn request(ws: &mut WsStream, id: u64, msg: Value, early: &mut Vec<Value>) -> Option<Value> {
    ws.send(Message::Text(msg.to_string())).await.ok()?;
    let wait = async {
        while let Some(Ok(reply)) = ws.next().await {
            let Message::Text(text) = reply else { continue };
            let Ok(v) = serde_json::from_str::<Value>(&text) else { continue };
            if v["id"].as_u64() == Some(id) {
                return Some(v);
            }
            if v.get("method").is_some() {
                early.push(v);
            }
        }
        None
    };
    tokio::time::timeout(std::time::Duration::from_secs(5), wait).await.ok().flatten()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rules(allowed: &[&str], blocked: &[&str], block_private: bool) -> SiteRules {
        SiteRules {
            allowed: allowed.iter().map(|s| s.to_string()).collect(),
            blocked: blocked.iter().map(|s| s.to_string()).collect(),
            block_private,
        }
    }

    /// `localtest.me` points at 127.0.0.1, `nas.example` at 192.168.1.20, `example.com` is
    /// public, anything else doesn't resolve.
    fn fake(name: &str, _port: u16) -> Option<Vec<IpAddr>> {
        match name {
            "localtest.me" => Some(vec!["127.0.0.1".parse().unwrap()]),
            "nas.example" => Some(vec!["192.168.1.20".parse().unwrap()]),
            "example.com" | "docs.example.com" => Some(vec!["93.184.216.34".parse().unwrap()]),
            _ => None,
        }
    }

    #[test]
    fn entries_are_stored_as_lowercase_hosts() {
        assert_eq!(normalize_site(" Example.COM ").unwrap(), "example.com");
        assert_eq!(normalize_site("https://docs.example.com/a/b?x").unwrap(), "docs.example.com");
        assert_eq!(normalize_site("*.Example.com").unwrap(), "*.example.com");
        assert_eq!(normalize_site("example.com.").unwrap(), "example.com");
        assert_eq!(normalize_site("bücher.de").unwrap(), "xn--bcher-kva.de");
        assert_eq!(normalize_site("10.0.0.1").unwrap(), "10.0.0.1");
        for bad in ["", "*", "*.", "a.*.com", "*.10.0.0.1", "http://"] {
            assert!(normalize_site(bad).is_err(), "{bad:?} should be refused");
        }
        let list = normalize_sites(&["a.com".into(), "A.com".into(), "b.com".into()], ALLOWED).unwrap();
        assert_eq!(list, ["a.com", "b.com"]);
    }

    #[test]
    fn a_domain_covers_its_subdomains_and_a_wildcard_only_those() {
        assert!(site_matches("example.com", "example.com"));
        assert!(site_matches("example.com", "docs.example.com"));
        assert!(!site_matches("example.com", "badexample.com"));
        assert!(site_matches("*.example.com", "a.b.example.com"));
        assert!(!site_matches("*.example.com", "example.com"));
        assert!(!site_matches("10.0.0.1", "x.10.0.0.1"));
    }

    #[test]
    fn no_rules_means_no_checks() {
        let none = SiteRules::default();
        assert!(none.is_empty());
        for url in ["http://127.0.0.1:8080/", "http://169.254.169.254/latest", "https://anything.example/"] {
            assert_eq!(none.check_with(url, &fake), Ok(()));
        }
    }

    #[test]
    fn the_allow_list_refuses_an_off_list_site_and_names_the_rule() {
        let r = rules(&["example.com"], &[], false);
        assert_eq!(r.check_with("https://example.com/", &fake), Ok(()));
        assert_eq!(r.check_with("https://docs.example.com/x", &fake), Ok(()));
        let no = r.check_with("https://evil.test/login", &fake).unwrap_err();
        assert_eq!(no.rule, ALLOWED);
        assert_eq!(no.host, "evil.test");
        assert!(no.message.contains("browser_allowed_sites: example.com"), "{}", no.message);
        // Not a site: nothing to check
        assert_eq!(r.check_with("about:blank", &fake), Ok(()));
    }

    #[test]
    fn the_block_list_wins_over_the_allow_list() {
        let r = rules(&["example.com"], &["ads.example.com"], false);
        assert_eq!(r.check_with("https://docs.example.com/", &fake), Ok(()));
        let no = r.check_with("https://x.ads.example.com/", &fake).unwrap_err();
        assert_eq!((no.rule, no.entry.as_deref()), (BLOCKED, Some("ads.example.com")));
        assert!(no.message.contains("\"ads.example.com\""), "{}", no.message);
        // A block-list alone blocks only what it names
        let r = rules(&[], &["*.example.com"], false);
        assert_eq!(r.check_with("https://example.com/", &fake), Ok(()));
        assert_eq!(r.check_with("https://docs.example.com/", &fake).unwrap_err().rule, BLOCKED);
    }

    #[test]
    fn the_private_network_block_catches_local_and_internal_addresses() {
        let r = rules(&[], &[], true);
        for url in [
            "http://127.0.0.1:3000/",
            "http://10.1.2.3/",
            "http://169.254.169.254/latest/meta-data/",
            "http://[::1]/",
            "http://localtest.me/",
            "http://nas.example/",
            "http://2130706433/",
            "http://unresolvable.invalid/",
        ] {
            let no = r.check_with(url, &fake).unwrap_err();
            assert_eq!(no.rule, PRIVATE, "{url}");
            assert!(no.message.contains("browser_block_private_network"), "{}", no.message);
        }
        assert_eq!(r.check_with("https://example.com/", &fake), Ok(()));
    }
}
