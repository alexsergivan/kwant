//! Popups the bot's page opens, followed by the live view and the bot's browser tools.
//!
//! "Continue with Google" (and most OAuth sign-ins) calls `window.open`: Chrome opens the sign-in
//! in a new page target whose `openerId` is the bot's page. Both the live view and the bot's
//! DevTools connection were bound to the bot's page alone, so the popup loaded where nobody could
//! see or reach it and the click seemed to do nothing. [`Popups`] watches the browser's targets
//! and keeps the chain of pages opened from the bot's page: the newest one still open is the page
//! everything acts on, and when it closes (the sign-in posted back to its opener) the page that
//! opened it is in front again. A page opened by anything else (another bot's tab, the browser's
//! own UI) is never followed.
use serde_json::{json, Value};
use std::sync::{Arc, Weak};
use std::time::Duration;
use tokio::sync::watch;

/// The most popups followed at once, so a page that keeps opening windows can't grow the chain.
pub const MAX_OPEN: usize = 8;

/// The bot's page and the pages opened from it, oldest first.
#[derive(Debug, Default, Clone, PartialEq)]
pub struct Chain {
    home: Option<String>,
    open: Vec<String>,
}

impl Chain {
    /// The bot's own page. A different one starts a new chain. `true` when it changed.
    pub fn set_home(&mut self, target: &str) -> bool {
        if self.home.as_deref() == Some(target) {
            return false;
        }
        self.home = Some(target.to_string());
        self.open.clear();
        true
    }

    pub fn home(&self) -> Option<&str> { self.home.as_deref() }

    /// The page in front: the newest popup still open, else the bot's page.
    pub fn active(&self) -> Option<&str> { self.open.last().map(String::as_str).or(self.home.as_deref()) }

    pub fn is_popup(&self, target: &str) -> bool { self.open.iter().any(|t| t == target) }

    /// A page `target` opened by `opener`: followed when the bot's page or one of its popups
    /// opened it. `true` when the page in front changed.
    pub fn opened(&mut self, target: &str, opener: &str) -> bool {
        if opener.is_empty() || self.home.as_deref() == Some(target) || self.is_popup(target) {
            return false;
        }
        let ours = self.home.as_deref() == Some(opener) || self.is_popup(opener);
        if !ours || self.open.len() >= MAX_OPEN {
            return false;
        }
        self.open.push(target.to_string());
        true
    }

    /// A page closed. A popup leaves the chain; the bot's page closing ends it. `true` when the
    /// page in front changed.
    pub fn closed(&mut self, target: &str) -> bool {
        let before = self.active().map(str::to_string);
        if self.home.as_deref() == Some(target) {
            *self = Self::default();
        } else {
            self.open.retain(|t| t != target);
        }
        before.as_deref() != self.active()
    }
}

/// The chain, kept current by a task watching the browser's targets. Every change of the page
/// in front bumps [`Self::changes`].
pub struct Popups {
    chain: std::sync::Mutex<Chain>,
    changed: watch::Sender<u64>,
    watcher: std::sync::Mutex<Option<tokio::task::JoinHandle<()>>>,
}

impl Default for Popups {
    fn default() -> Self {
        Self { chain: Default::default(), changed: watch::channel(0).0, watcher: Default::default() }
    }
}

impl Drop for Popups {
    fn drop(&mut self) {
        if let Some(task) = self.watcher.get_mut().unwrap_or_else(|e| e.into_inner()).take() {
            task.abort();
        }
    }
}

impl Popups {
    fn with<T>(&self, f: impl FnOnce(&mut Chain) -> T) -> T {
        f(&mut self.chain.lock().unwrap_or_else(|e| e.into_inner()))
    }

    pub fn chain(&self) -> Chain { self.with(|c| c.clone()) }

    /// Bumped each time the page in front changes.
    pub fn changes(&self) -> watch::Receiver<u64> { self.changed.subscribe() }

    pub fn generation(&self) -> u64 { *self.changed.borrow() }

    fn bump(&self) { self.changed.send_modify(|g| *g += 1); }

    pub fn set_home(&self, target: &str) {
        if self.with(|c| c.set_home(target)) {
            self.bump();
        }
    }

    fn opened(&self, target: &str, opener: &str) {
        if self.with(|c| c.opened(target, opener)) {
            tracing::info!(target, opener, "following a popup the bot's page opened");
            self.bump();
        }
    }

    fn closed(&self, target: &str) {
        if self.with(|c| c.closed(target)) {
            tracing::info!(target, "a followed popup closed; back to the page that opened it");
            self.bump();
        }
    }

    /// Whether a watch is running (it ends with its browser connection).
    pub fn is_watching(&self) -> bool {
        self.watcher.lock().unwrap_or_else(|e| e.into_inner()).as_ref().is_some_and(|task| !task.is_finished())
    }

    /// Starts watching the browser at `browser_ws` unless a watch is running, and waits briefly
    /// until it has seen the pages already open.
    pub async fn watch(self: &Arc<Self>, browser_ws: String) {
        let ready = {
            let mut slot = self.watcher.lock().unwrap_or_else(|e| e.into_inner());
            if slot.as_ref().is_some_and(|task| !task.is_finished()) {
                return;
            }
            let (tx, rx) = tokio::sync::oneshot::channel();
            *slot = Some(tokio::spawn(run(Arc::downgrade(self), browser_ws, tx)));
            rx
        };
        let _ = tokio::time::timeout(Duration::from_secs(3), ready).await;
    }
}

/// Follows `Target.targetCreated`/`targetInfoChanged`/`targetDestroyed` on a browser-level
/// connection. Ends when the connection or the [`Popups`] goes away; the next connect restarts it.
async fn run(popups: Weak<Popups>, browser_ws: String, ready: tokio::sync::oneshot::Sender<()>) {
    use futures::{SinkExt, StreamExt};
    use tokio_tungstenite::tungstenite::Message;
    let Ok(mut ws) = super::browser::connect_cdp(&browser_ws).await else {
        tracing::debug!("popups: couldn't connect to the browser");
        return;
    };
    let discover = json!({"id": 1, "method": "Target.setDiscoverTargets", "params": {"discover": true}});
    if ws.send(Message::Text(discover.to_string())).await.is_err() {
        return;
    }
    let mut ready = Some(ready);
    while let Some(Ok(message)) = ws.next().await {
        let Message::Text(text) = message else { continue };
        let value: Value = serde_json::from_str(&text).unwrap_or_default();
        let Some(popups) = popups.upgrade() else { return };
        if value["id"] == 1 {
            // the pages already open were announced before this reply
            if let Some(ready) = ready.take() {
                let _ = ready.send(());
            }
            continue;
        }
        let params = &value["params"];
        match value["method"].as_str().unwrap_or_default() {
            "Target.targetCreated" | "Target.targetInfoChanged" => {
                let info = &params["targetInfo"];
                if info["type"] == "page" {
                    popups.opened(info["targetId"].as_str().unwrap_or_default(), info["openerId"].as_str().unwrap_or_default());
                }
            }
            "Target.targetDestroyed" | "Target.targetCrashed" => popups.closed(params["targetId"].as_str().unwrap_or_default()),
            _ => {}
        }
    }
}

/// `ws_url` (a page's DevTools address) pointed at another page of the same browser.
pub fn page_ws_url(ws_url: &str, target: &str) -> String {
    match ws_url.rsplit_once("/devtools/page/") {
        Some((base, _)) => format!("{base}/devtools/page/{target}"),
        None => ws_url.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn chain() -> Chain {
        let mut c = Chain::default();
        c.set_home("bot");
        c
    }

    #[test]
    fn a_popup_the_bots_page_opens_comes_to_the_front_and_its_close_goes_back() {
        let mut c = chain();
        assert_eq!(c.active(), Some("bot"));
        assert!(c.opened("google", "bot"));
        assert_eq!(c.active(), Some("google"));
        assert!(!c.opened("google", "bot"), "announced again (info changed): nothing new");
        assert!(c.closed("google"));
        assert_eq!(c.active(), Some("bot"));
    }

    #[test]
    fn pages_opened_by_anything_else_are_never_followed() {
        let mut c = chain();
        assert!(!c.opened("other-bots-tab", "other-bot"));
        assert!(!c.opened("new-tab", ""), "a tab nobody opened (the browser's own, a bot's new tab)");
        assert_eq!(c.active(), Some("bot"));
        assert!(!c.closed("other-bots-tab"), "its close changes nothing");
    }

    #[test]
    fn a_popup_of_a_popup_is_followed_and_closing_the_middle_one_keeps_the_newest() {
        let mut c = chain();
        assert!(c.opened("sign-in", "bot"));
        assert!(c.opened("consent", "sign-in"));
        assert_eq!(c.active(), Some("consent"));
        assert!(!c.closed("sign-in"), "the newest is still in front");
        assert!(c.closed("consent"));
        assert_eq!(c.active(), Some("bot"));
    }

    #[test]
    fn the_bots_page_closing_or_changing_ends_the_chain() {
        let mut c = chain();
        c.opened("sign-in", "bot");
        assert!(c.closed("bot"));
        assert_eq!(c.active(), None);
        let mut c = chain();
        c.opened("sign-in", "bot");
        assert!(c.set_home("another"));
        assert_eq!(c.active(), Some("another"));
        assert!(!c.set_home("another"));
    }

    #[test]
    fn a_page_that_keeps_opening_windows_is_followed_only_so_far() {
        let mut c = chain();
        for i in 0..MAX_OPEN {
            assert!(c.opened(&format!("w{i}"), "bot"));
        }
        assert!(!c.opened("one-too-many", "bot"));
        assert_eq!(c.active(), Some(format!("w{}", MAX_OPEN - 1).as_str()));
    }

    #[test]
    fn a_page_address_is_pointed_at_another_page() {
        assert_eq!(page_ws_url("ws://localhost:9222/devtools/page/AAA", "BBB"), "ws://localhost:9222/devtools/page/BBB");
        assert_eq!(page_ws_url("ws://localhost:9222/devtools/browser/x", "BBB"), "ws://localhost:9222/devtools/browser/x");
    }

    #[tokio::test]
    async fn every_change_of_the_page_in_front_is_announced() {
        let p = Popups::default();
        let mut changes = p.changes();
        p.set_home("bot");
        p.opened("google", "bot");
        p.opened("elsewhere", "someone");
        p.closed("google");
        assert_eq!(p.generation(), 3);
        assert!(changes.has_changed().unwrap());
        changes.borrow_and_update();
        p.closed("not-ours");
        assert!(!changes.has_changed().unwrap());
    }
}
