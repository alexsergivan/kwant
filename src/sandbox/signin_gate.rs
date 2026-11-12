//! Sign-in pages that refuse a browser run by a program, and what kwant does about them.
//!
//! Google (and Microsoft and Apple) judge more than the user agent before they let anyone sign in,
//! and refuse a windowless Chrome however it introduces itself: "Couldn't sign you in. This
//! browser or app may not be secure." A Chrome with a window, on the person's own computer, is
//! Chrome's normal mode, so on the desktop apps the sign-in moves there (`set_local_window`), and
//! comes back into the app once it is done. A server has no screen to open one on: the person is
//! told the ways that do work.
use reqwest::Url;

/// Who runs a sign-in page that refuses automated browsers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Provider {
    Google,
    Microsoft,
    Apple,
}

impl Provider {
    pub fn name(self) -> &'static str {
        match self {
            Provider::Google => "Google",
            Provider::Microsoft => "Microsoft",
            Provider::Apple => "Apple",
        }
    }
}

fn parse(url: &str) -> Option<Url> {
    Url::parse(url).ok().filter(|u| u.scheme() == "https" || u.scheme() == "http")
}

/// The provider whose sign-in host `url` is on.
pub fn provider(url: &str) -> Option<Provider> {
    let url = parse(url)?;
    match url.host_str()? {
        "accounts.google.com" => Some(Provider::Google),
        "login.microsoftonline.com" | "login.live.com" => Some(Provider::Microsoft),
        "appleid.apple.com" | "idmsa.apple.com" => Some(Provider::Apple),
        _ => None,
    }
}

/// Whether `url` is a sign-in page of one of those providers (Google's account pages that aren't
/// sign-in, such as its help, don't count).
pub fn is_signin(url: &str) -> bool {
    let Some(provider) = provider(url) else { return false };
    if provider != Provider::Google {
        return true;
    }
    let path = parse(url).map(|u| u.path().to_string()).unwrap_or_default();
    ["/signin", "/v3/signin", "/o/oauth2", "/ServiceLogin", "/AccountChooser", "/InteractiveLogin"]
        .iter()
        .any(|prefix| path.starts_with(prefix))
}

/// Whether `url` is the page Google shows when it refused the browser: "Couldn't sign you in.
/// This browser or app may not be secure."
pub fn is_rejected(url: &str) -> bool {
    provider(url) == Some(Provider::Google)
        && parse(url).is_some_and(|u| u.path().starts_with("/v3/signin/rejected") || u.path().starts_with("/signin/rejected"))
}

/// When the live view says something about a sign-in: always on the refused page, and on any
/// sign-in page of those providers once the person has taken control of it to sign in.
pub fn needs_window(url: &str, person_in_control: bool) -> bool {
    is_rejected(url) || (person_in_control && is_signin(url))
}

/// Where to open the sign-in again in a real window. Not the refused page itself, which only
/// says no: the sign-in it came from, from its `continue` (Google puts the address it was signing
/// in for there), else the page before it in the tab's history. Any other page opens as it is.
pub fn reopen_url(url: &str, previous: Option<&str>) -> Option<String> {
    if !is_rejected(url) {
        return parse(url).map(|_| url.to_string());
    }
    let url = parse(url)?;
    let next = url.query_pairs().find(|(key, _)| key == "continue").map(|(_, value)| value.to_string());
    next.filter(|n| parse(n).is_some())
        .or_else(|| previous.filter(|p| parse(p).is_some() && !is_rejected(p)).map(str::to_string))
}

/// The line the live view shows on such a page. `window` when a real Chrome window can be opened
/// here (the desktop apps); else the plain way out on a server.
pub fn notice(provider: Provider, window: bool) -> String {
    let name = provider.name();
    if window {
        format!("{name} only allows sign-in in a normal Chrome window.")
    } else {
        format!(
            "{name} doesn't allow sign-in from a browser on a server. Sign in on this site with email instead, or connect {name} in Apps & accounts."
        )
    }
}

/// The action next to [`notice`] when a window can be opened.
pub const WINDOW_ACTION: &str = "Sign in in Chrome";

/// What a live view has been told about a sign-in page, so it hears each change once.
#[derive(Debug)]
pub struct LiveNotice {
    /// Whether a real Chrome window can be opened here; `None` when Chrome already is one.
    window: Option<bool>,
    shown: bool,
}

impl LiveNotice {
    pub fn new(window: Option<bool>) -> Self {
        LiveNotice { window, shown: false }
    }

    /// The message for the live view after the page moved to `url` (or the person took control
    /// of it): `{"type":"signin","text":…,"action":…}` on such a sign-in page, `{"type":"signin",
    /// "text":null}` once the page left the sign-in host, else nothing.
    pub fn update(&mut self, url: &str, person_in_control: bool) -> Option<serde_json::Value> {
        let window = self.window?;
        let provider = provider(url);
        if let Some(provider) = provider.filter(|_| needs_window(url, person_in_control)) {
            if self.shown { return None; }
            self.shown = true;
            let action = window.then_some(WINDOW_ACTION);
            return Some(serde_json::json!({"type":"signin","text":notice(provider, window),"action":action}));
        }
        if self.shown && provider.is_none() {
            self.shown = false;
            return Some(serde_json::json!({"type":"signin","text":null}));
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const REJECTED: &str = "https://accounts.google.com/v3/signin/rejected?app_domain=https://elevenlabs.io&continue=https://accounts.google.com/o/oauth2/auth?client_id%3Dabc%26redirect_uri%3Dhttps://elevenlabs.io/__/auth/handler&flowName=GeneralOAuthLite";

    #[test]
    fn the_refused_page_is_recognised() {
        assert!(is_rejected(REJECTED));
        assert!(needs_window(REJECTED, false), "a bot reaching it is enough");
        assert!(!is_rejected("https://accounts.google.com/v3/signin/identifier?flowName=GlifWebSignIn"));
        assert!(!is_rejected("https://example.com/v3/signin/rejected"));
        assert!(!is_rejected("not a url"));
    }

    #[test]
    fn other_sign_in_pages_count_once_the_person_takes_control() {
        let identifier = "https://accounts.google.com/v3/signin/identifier?flowName=GlifWebSignIn";
        assert!(!needs_window(identifier, false), "a bot on it may still get through");
        assert!(needs_window(identifier, true));
        assert!(needs_window("https://accounts.google.com/o/oauth2/v2/auth?client_id=x", true));
        assert!(needs_window("https://login.microsoftonline.com/common/oauth2/v2.0/authorize", true));
        assert!(needs_window("https://appleid.apple.com/auth/authorize?client_id=x", true));
        assert!(!needs_window("https://accounts.google.com/TOS", true), "not a sign-in");
        assert!(!needs_window("https://elevenlabs.io/sign-in", true));
        assert!(!needs_window("https://accounts.google.com.evil.example/signin", true));
        assert_eq!(provider("https://login.live.com/x"), Some(Provider::Microsoft));
    }

    #[test]
    fn the_window_opens_at_the_sign_in_before_the_refusal() {
        assert_eq!(
            reopen_url(REJECTED, Some("https://accounts.google.com/v3/signin/identifier")).as_deref(),
            Some("https://accounts.google.com/o/oauth2/auth?client_id=abc&redirect_uri=https://elevenlabs.io/__/auth/handler")
        );
        let no_continue = "https://accounts.google.com/v3/signin/rejected?app_domain=https://elevenlabs.io";
        assert_eq!(reopen_url(no_continue, Some("https://elevenlabs.io/sign-in")).as_deref(), Some("https://elevenlabs.io/sign-in"));
        assert_eq!(reopen_url(no_continue, Some(no_continue)), None);
        assert_eq!(reopen_url(no_continue, None), None);
        let bad_continue = "https://accounts.google.com/v3/signin/rejected?continue=javascript:alert(1)";
        assert_eq!(reopen_url(bad_continue, None), None, "only a web address is opened");
        assert_eq!(reopen_url("https://elevenlabs.io/app", None).as_deref(), Some("https://elevenlabs.io/app"));
    }

    #[test]
    fn the_lines_say_what_to_do() {
        assert_eq!(notice(Provider::Google, true), "Google only allows sign-in in a normal Chrome window.");
        assert_eq!(
            notice(Provider::Google, false),
            "Google doesn't allow sign-in from a browser on a server. Sign in on this site with email instead, or connect Google in Apps & accounts."
        );
        assert!(notice(Provider::Microsoft, true).starts_with("Microsoft only allows"));
    }

    #[test]
    fn a_live_view_hears_each_change_once() {
        let mut live = LiveNotice::new(Some(true));
        assert_eq!(live.update("https://elevenlabs.io/sign-in", false), None);
        let shown = live.update(REJECTED, false).unwrap();
        assert_eq!(shown["text"], "Google only allows sign-in in a normal Chrome window.");
        assert_eq!(shown["action"], "Sign in in Chrome");
        assert_eq!(live.update(REJECTED, true), None, "once");
        assert_eq!(live.update("https://accounts.google.com/v3/signin/identifier", true), None, "still the sign-in");
        assert_eq!(live.update("https://elevenlabs.io/app", false).unwrap()["text"], serde_json::Value::Null);
        assert_eq!(live.update("https://elevenlabs.io/app/home", false), None);

        let mut server = LiveNotice::new(Some(false));
        let shown = server.update("https://accounts.google.com/v3/signin/identifier", true).unwrap();
        assert!(shown["text"].as_str().unwrap().contains("from a browser on a server"));
        assert_eq!(shown["action"], serde_json::Value::Null);

        let mut windowed = LiveNotice::new(None);
        assert_eq!(windowed.update(REJECTED, true), None, "a real window needs no line");
    }
}
