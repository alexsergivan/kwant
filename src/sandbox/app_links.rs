//! Links that open another app (`tiktok://`, `snssdk1233://`, `intent:`, `itms-apps:`...).
//!
//! Chrome answers one with an "Open <app>?" prompt that is tab-modal: while it is up the tab
//! ignores every click and key, yet scripts, animations and screenshots carry on. A headless
//! browser draws the prompt nowhere the live view can see, and DevTools has no way to answer
//! it. It closes only on a cross-site navigation, so even a reload leaves the page dead.
//!
//! kwnt keeps it from opening in the first place: [`GUARD_JS`] runs in an isolated world of every
//! document and cancels a navigation to such a link before it reaches Chrome. What it can't
//! cancel (a frame whose first address is the link) is caught by [`AppLinkWatch`], which tells
//! the live view to step the tab off the page and back, the one thing that closes the prompt.
//! A link that fired while no live view was open is found when one opens: [`PROBE_ARM_JS`] and
//! [`PROBE_READ_JS`] check that the page still gets a mouse move, and a tab that doesn't is
//! stepped off and back the same way.
use std::{collections::VecDeque, time::Duration};
use tokio::time::Instant;

/// The isolated world the guard runs in: the page's own scripts can't see or undo it.
pub const WORLD: &str = "kwnt-applinks";
/// Called from [`WORLD`] with the scheme of a link the guard stopped.
pub const BINDING: &str = "kwntAppLink";

/// Schemes a tab opens itself; everything else is handed to the operating system.
const IN_BROWSER: &[&str] = &[
    "http", "https", "about", "data", "blob", "javascript", "file", "filesystem", "ws", "wss",
    "chrome", "chrome-error", "chrome-extension", "chrome-untrusted", "devtools", "view-source",
];
/// Handed to the operating system without a prompt, so they can't freeze the tab.
const NO_PROMPT: &[&str] = &["mailto", "news", "snews"];

/// Chrome's console line when it refuses a link outright (no user gesture) instead of asking.
pub const REFUSED_LOG: &str = "Not allowed to launch";

/// Cancels navigations to another app in this document. Runs in [`WORLD`], once per world.
pub const GUARD_JS: &str = r#"(() => {
  if (self.__kwntAppLinks || !self.navigation) return;
  self.__kwntAppLinks = true;
  const inBrowser = new Set(__IN_BROWSER__);
  navigation.addEventListener('navigate', (event) => {
    let scheme;
    try { scheme = new URL(event.destination.url).protocol.slice(0, -1).toLowerCase(); } catch (e) { return; }
    if (inBrowser.has(scheme) || !event.cancelable) return;
    event.preventDefault();
    try { self.__BINDING__(scheme); } catch (e) {}
  });
})()"#;

/// [`GUARD_JS`] with its scheme list and binding filled in.
pub fn guard_js() -> String {
    GUARD_JS
        .replace("__IN_BROWSER__", &serde_json::to_string(IN_BROWSER).unwrap_or_default())
        .replace("__BINDING__", BINDING)
}

/// The scheme of `url` when it opens another app, lowercased. A link the tab opens itself, or
/// one that isn't a URL, gives `None`.
pub fn app_scheme(url: &str) -> Option<String> {
    let (scheme, _) = url.trim_start().split_once(':')?;
    let mut chars = scheme.chars();
    if !chars.next()?.is_ascii_alphabetic() || !chars.all(|c| c.is_ascii_alphanumeric() || "+-.".contains(c)) {
        return None;
    }
    let scheme = scheme.to_ascii_lowercase();
    (!IN_BROWSER.contains(&scheme.as_str())).then_some(scheme)
}

/// The scheme of `url` when it is a link to another app a bot must not open: Chrome would hand it
/// to the operating system or freeze the tab behind an "Open <app>?" prompt. `host:port` without
/// a scheme (`localhost:8080`) isn't one, and neither is a Windows drive (`C:\…`, `c:/…`).
pub fn refused_scheme(url: &str) -> Option<String> {
    let scheme = app_scheme(url).filter(|s| s.len() > 1)?;
    let rest = url.trim_start()[scheme.len() + 1..].trim_start_matches('/');
    (!rest.starts_with(|c: char| c.is_ascii_digit())).then_some(scheme)
}

/// Why a bot's browser didn't open `url`, a link to another app.
pub fn refused_text(url: &str, scheme: &str) -> String {
    let shown: String = url.trim().chars().take(120).collect();
    format!("{shown} opens another app ({scheme}:), not a web page, so the browser didn't open it: kwnt only opens web pages (http:// and https://). \
             Use a web address instead, or another way on the page such as signing in with a password or code.")
}

/// The scheme of `url` when following it makes Chrome ask whether to open another app.
pub fn prompting_scheme(url: &str) -> Option<String> {
    app_scheme(url).filter(|s| !NO_PROMPT.contains(&s.as_str()))
}

/// A scheme as the person may read it: a scheme the guard reported, checked and cut short.
pub fn shown_scheme(raw: &str) -> Option<String> {
    let scheme = raw.trim().trim_end_matches(':');
    (scheme.len() <= 40).then(|| app_scheme(&format!("{scheme}:"))).flatten()
}

/// How long after a link starts loading Chrome may still refuse it outright (no prompt).
pub const SETTLE: Duration = Duration::from_millis(400);
/// At most this many recoveries in [`WINDOW`]: a page that opens its app again on every load
/// would otherwise be stepped off and back forever.
pub const MAX_RECOVERIES: usize = 3;
pub const WINDOW: Duration = Duration::from_secs(60);
/// The person is told about a stopped link at most this often.
pub const NOTICE_GAP: Duration = Duration::from_secs(5);

/// What the live view should do about a link to another app. The scheme is `None` when the
/// tab was found ignoring input without the live view seeing the link (it fired before it opened).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Verdict {
    /// Chrome is asking: step the tab off the page and back, then tell the person.
    Recover(Option<String>),
    /// Chrome is asking again too often: leave the page as it is and tell the person why.
    GiveUp(Option<String>),
}

/// Marks where a mouse move should land in the page and starts counting the moves that reach
/// it: `[x, y]`, or `null` when there is no point to probe (the page is all frames, or has no
/// size). Runs in the page's main world; the probe removes itself in [`PROBE_READ_JS`].
pub const PROBE_ARM_JS: &str = r#"(() => {
  const w = innerWidth, h = innerHeight, key = Symbol.for('kwnt.inputProbe');
  if (!(w > 32 && h > 32)) return null;
  for (const [x, y] of [[8, 8], [w - 9, h - 9], [8, h - 9], [w - 9, 8], [w >> 1, h >> 1]]) {
    const el = document.elementFromPoint(x, y);
    if (!el || /^(IFRAME|FRAME|EMBED|OBJECT|FENCEDFRAME|PORTAL)$/.test(el.tagName)) continue;
    if (self[key]) removeEventListener('mousemove', self[key].on, true);
    const probe = { n: 0, on: () => { probe.n++; } };
    self[key] = probe;
    addEventListener('mousemove', probe.on, true);
    return [x, y];
  }
  return null;
})()"#;

/// How many probe moves reached the page, or -1 when the probe is gone (a new document).
pub const PROBE_READ_JS: &str = r#"(() => {
  const key = Symbol.for('kwnt.inputProbe'), probe = self[key];
  if (!probe) return -1;
  removeEventListener('mousemove', probe.on, true);
  delete self[key];
  return probe.n;
})()"#;

/// Watches the page's navigations for a link to another app that got past the guard.
#[derive(Debug, Default)]
pub struct AppLinkWatch {
    pending: Option<(String, Instant)>,
    recoveries: VecDeque<Instant>,
    noticed: Option<Instant>,
}

impl AppLinkWatch {
    /// A frame started loading `url`.
    pub fn started(&mut self, url: &str, now: Instant) {
        if self.pending.is_some() { return; }
        if let Some(scheme) = prompting_scheme(url) { self.pending = Some((scheme, now + SETTLE)); }
    }

    /// Chrome logged `text`. A refusal means it didn't ask, so the page still takes input.
    pub fn logged(&mut self, text: &str) {
        if text.starts_with(REFUSED_LOG) { self.pending = None; }
    }

    /// When [`Self::due`] has something to say.
    pub fn deadline(&self) -> Option<Instant> {
        self.pending.as_ref().map(|(_, at)| *at)
    }

    /// What to do once the link has had [`SETTLE`] to be refused.
    pub fn due(&mut self, now: Instant) -> Option<Verdict> {
        let (_, at) = self.pending.as_ref()?;
        if now < *at { return None; }
        let (scheme, _) = self.pending.take()?;
        Some(self.verdict(Some(scheme), now))
    }

    /// The tab was found ignoring input when the live view opened: a link fired before it did.
    /// Counts against the same cap as a link the live view saw.
    pub fn stuck(&mut self, now: Instant) -> Verdict {
        self.verdict(None, now)
    }

    fn verdict(&mut self, scheme: Option<String>, now: Instant) -> Verdict {
        while self.recoveries.front().is_some_and(|t| now.duration_since(*t) >= WINDOW) { self.recoveries.pop_front(); }
        if self.recoveries.len() >= MAX_RECOVERIES { return Verdict::GiveUp(scheme); }
        self.recoveries.push_back(now);
        Verdict::Recover(scheme)
    }

    /// Whether the person may be told about another stopped link now.
    pub fn may_notice(&mut self, now: Instant) -> bool {
        if self.noticed.is_some_and(|t| now.duration_since(t) < NOTICE_GAP) { return false; }
        self.noticed = Some(now);
        true
    }
}

/// Said when the guard stopped a link to another app.
pub fn stopped_text(scheme: &str) -> String {
    format!("This page tried to open another app ({scheme}:), which can't open from here, so kwnt stopped it. Use another way on the page, such as signing in with a password or code.")
}

/// Said after kwnt stepped the tab off the page and back to close Chrome's prompt.
pub fn recovered_text(scheme: Option<&str>) -> String {
    match scheme {
        Some(scheme) => format!("This page tried to open another app ({scheme}:), and the browser stopped taking clicks and keys while it waited. kwnt reloaded the page so you can use it again."),
        None => "This page had stopped taking clicks and keys, most likely because it tried to open another app before you opened the view. kwnt reloaded the page so you can use it again.".into(),
    }
}

/// Said when the page keeps opening its app and kwnt stopped reloading it.
pub fn gave_up_text(scheme: Option<&str>) -> String {
    match scheme {
        Some(scheme) => format!("This page keeps trying to open another app ({scheme}:), and the browser ignores clicks and keys while it does. Open another page from the address bar to carry on."),
        None => "This page keeps ignoring clicks and keys, most likely because it keeps trying to open another app. Open another page from the address bar to carry on.".into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn links_to_other_apps_are_told_from_pages_the_tab_opens_itself() {
        for url in ["snssdk1233://login?x=1", "tiktok://user", "TikTok://x", "intent://scan/#Intent;scheme=z;end", "itms-apps://a", "zoommtg:join", "web+app:x"] {
            assert!(prompting_scheme(url).is_some(), "{url}");
        }
        assert_eq!(prompting_scheme("TikTok://x").as_deref(), Some("tiktok"));
        for url in ["https://www.tiktok.com/login", "http://a", "about:blank", "about:srcdoc", "data:text/html,x", "blob:https://a/1", "javascript:void(0)",
            "chrome-error://chromewebdata/", "file:///tmp/a", "", "no scheme", "/relative", "1abc:x", "a b:c"] {
            assert_eq!(prompting_scheme(url), None, "{url}");
        }
        // mail links are handed over without a prompt, so they can't freeze the tab
        assert_eq!(app_scheme("mailto:a@b.c").as_deref(), Some("mailto"));
        assert_eq!(prompting_scheme("mailto:a@b.c"), None);
        assert_eq!(shown_scheme("snssdk1233:").as_deref(), Some("snssdk1233"));
        assert_eq!(shown_scheme("https"), None);
        assert_eq!(shown_scheme(&"a".repeat(41)), None);
        assert_eq!(shown_scheme("<b>x</b>"), None);
    }

    #[test]
    fn the_guard_lets_the_tab_open_what_it_opens_itself() {
        let js = guard_js();
        for scheme in IN_BROWSER { assert!(js.contains(&format!("\"{scheme}\"")), "{scheme}"); }
        assert!(js.contains("self.kwntAppLink(scheme)"));
        assert!(js.contains("event.preventDefault()"));
        assert!(!js.contains("__IN_BROWSER__") && !js.contains("__BINDING__"), "{js}");
    }

    #[test]
    fn a_link_chrome_refuses_outright_needs_no_recovery() {
        let mut watch = AppLinkWatch::default();
        let t = Instant::now();
        watch.started("snssdk1233://login", t);
        assert_eq!(watch.deadline(), Some(t + SETTLE));
        watch.logged("Not allowed to launch 'snssdk1233://login' because a user gesture is required.");
        assert_eq!(watch.due(t + SETTLE), None);
        assert_eq!(watch.deadline(), None);
        // web pages and mail links never start one
        watch.started("https://www.tiktok.com/", t);
        watch.started("mailto:a@b.c", t);
        assert_eq!(watch.deadline(), None);
    }

    #[test]
    fn a_prompt_is_recovered_after_it_settles_a_few_times_a_minute_at_most() {
        let mut watch = AppLinkWatch::default();
        let t = Instant::now();
        for i in 0..MAX_RECOVERIES as u64 {
            let at = t + Duration::from_secs(i);
            watch.started("snssdk1233://login", at);
            watch.started("tiktok://second-in-the-same-burst", at);
            assert_eq!(watch.due(at + SETTLE / 2), None, "Chrome may still refuse it");
            assert_eq!(watch.due(at + SETTLE), Some(Verdict::Recover(Some("snssdk1233".into()))));
            assert_eq!(watch.due(at + SETTLE), None, "once per link");
        }
        watch.started("snssdk1233://login", t + Duration::from_secs(10));
        assert_eq!(watch.due(t + Duration::from_secs(11)), Some(Verdict::GiveUp(Some("snssdk1233".into()))));
        // a minute after the first, one may run again
        watch.started("snssdk1233://login", t + WINDOW);
        assert_eq!(watch.due(t + WINDOW + SETTLE), Some(Verdict::Recover(Some("snssdk1233".into()))));
    }

    #[test]
    fn a_tab_found_stuck_is_recovered_under_the_same_cap() {
        let mut watch = AppLinkWatch::default();
        let t = Instant::now();
        assert_eq!(watch.stuck(t), Verdict::Recover(None));
        for i in 1..MAX_RECOVERIES as u64 {
            watch.started("snssdk1233://login", t + Duration::from_secs(i));
            assert!(matches!(watch.due(t + Duration::from_secs(i) + SETTLE), Some(Verdict::Recover(_))));
        }
        assert_eq!(watch.stuck(t + Duration::from_secs(10)), Verdict::GiveUp(None));
        assert_eq!(watch.stuck(t + WINDOW), Verdict::Recover(None));
        assert!(recovered_text(None).contains("reloaded the page"));
        assert!(!gave_up_text(None).contains("reloaded"));
    }

    #[test]
    fn a_bot_is_refused_links_to_other_apps_but_not_web_pages() {
        for (url, scheme) in [("tiktok://open", "tiktok"), ("snssdk1233://login", "snssdk1233"), ("intent://scan/#Intent;scheme=zxing;end", "intent"),
            ("intent:#Intent;end", "intent"), (" TikTok://x", "tiktok"), ("mailto:a@b.c", "mailto"), ("itms-apps://a", "itms-apps"), ("zoommtg:join", "zoommtg")] {
            assert_eq!(refused_scheme(url).as_deref(), Some(scheme), "{url}");
        }
        for url in ["https://www.tiktok.com/login", "http://a", "about:blank", "data:text/html,x", "javascript:void(0)", "chrome://version",
            "localhost:8080", "example.com:443/x", "127.0.0.1:9381", "www.tiktok.com", "", "/relative", "C:\\Users\\me\\file.txt", "c:/Users/me/file.txt"] {
            assert_eq!(refused_scheme(url), None, "{url}");
        }
        let text = refused_text("tiktok://open", "tiktok");
        assert!(text.contains("another app (tiktok:)") && text.contains("only opens web pages"), "{text}");
        assert!(refused_text(&"x".repeat(500), "x").len() < 400, "a long link is cut short");
    }

    #[test]
    fn the_input_probe_skips_frames_and_cleans_up() {
        assert!(PROBE_ARM_JS.contains("elementFromPoint") && PROBE_ARM_JS.contains("IFRAME"));
        assert!(PROBE_ARM_JS.contains("addEventListener('mousemove'") && PROBE_READ_JS.contains("removeEventListener('mousemove'"));
        assert!(PROBE_READ_JS.contains("return -1"), "a new document is no proof the tab is stuck");
    }

    #[test]
    fn notices_are_spaced_out() {
        let mut watch = AppLinkWatch::default();
        let t = Instant::now();
        assert!(watch.may_notice(t));
        assert!(!watch.may_notice(t + NOTICE_GAP / 2));
        assert!(watch.may_notice(t + NOTICE_GAP));
    }
}
