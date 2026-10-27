//! Where a link goes. The window only ever shows kwant: its own page on loopback and the bundled
//! "Starting kwant…" page. Any other web or mail address opens in the person's default browser,
//! so a sign-in page can never take over the window with no way back.

use tauri::Url;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Link {
    /// Load it in the kwant window.
    Stay,
    /// Cancel it here and open it in the default browser.
    Browser,
    /// Cancel it and open nothing (file:, javascript:, other apps' schemes).
    Drop,
}

/// kwant's own page on this port, or the bundled loading page.
fn is_kwant(url: &Url, port: u16) -> bool {
    match url.scheme() {
        // the bundled loading page (tauri://localhost on macOS, http://tauri.localhost elsewhere)
        "tauri" => url.host_str() == Some("localhost"),
        "http" if url.host_str() == Some("tauri.localhost") => true,
        "http" => matches!(url.host_str(), Some("127.0.0.1" | "localhost" | "[::1]")) && url.port() == Some(port),
        _ => false,
    }
}

/// kwant's server itself, which the browser can reach too (not the bundled page).
fn is_kwant_server(url: &Url, port: u16) -> bool {
    url.scheme() == "http" && url.host_str() != Some("tauri.localhost") && is_kwant(url, port)
}

/// A top-level navigation in the kwant window.
pub fn navigation(url: &Url, port: u16) -> Link {
    if is_kwant(url, port) {
        return Link::Stay;
    }
    match url.scheme() {
        // blank pages and in-page content kwant makes itself (downloads, previews)
        "about" | "blob" | "data" => Link::Stay,
        "http" | "https" | "mailto" => Link::Browser,
        _ => Link::Drop,
    }
}

/// A window.open() or target=_blank request. The app never opens a second window: real
/// addresses, kwant's own included, go to the browser; a blank popup is refused, so the page's
/// window.open() gets null and falls back to a navigation that `navigation` sends to the browser.
pub fn new_window(url: &Url, port: u16) -> Link {
    match navigation(url, port) {
        Link::Stay if is_kwant_server(url, port) => Link::Browser,
        Link::Stay => Link::Drop,
        other => other,
    }
}

/// Opens the address in the default browser (macOS `open`). Only http(s) and mailto reach here.
pub fn open_in_browser(url: &Url) {
    let url = url.as_str().to_string();
    std::thread::spawn(
        move || match std::process::Command::new("/usr/bin/open").arg(&url).status() {
            Ok(s) if s.success() => {}
            Ok(s) => tracing::warn!(status = %s, "the browser didn't open the link"),
            Err(e) => tracing::warn!(error = %e, "couldn't open the link in the browser"),
        },
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    const PORT: u16 = 8799;

    fn nav(s: &str) -> Link {
        navigation(&s.parse().unwrap(), PORT)
    }

    fn popup(s: &str) -> Link {
        new_window(&s.parse().unwrap(), PORT)
    }

    #[test]
    fn kwant_pages_stay() {
        assert_eq!(nav("http://127.0.0.1:8799/"), Link::Stay);
        assert_eq!(nav("http://127.0.0.1:8799/api/mcp/oauth/callback?code=x"), Link::Stay);
        assert_eq!(nav("http://localhost:8799/#/missions"), Link::Stay);
        assert_eq!(nav("http://[::1]:8799/"), Link::Stay);
        assert_eq!(nav("tauri://localhost/index.html"), Link::Stay);
        assert_eq!(nav("http://tauri.localhost/index.html"), Link::Stay);
        assert_eq!(nav("about:blank"), Link::Stay);
        assert_eq!(nav("blob:http://127.0.0.1:8799/5f0c"), Link::Stay);
    }

    #[test]
    fn outside_pages_go_to_the_browser() {
        assert_eq!(
            nav("https://accounts.google.com/o/oauth2/auth?client_id=x"),
            Link::Browser
        );
        assert_eq!(nav("https://dash.cloudflare.com/oauth2/auth"), Link::Browser);
        assert_eq!(nav("http://example.com/"), Link::Browser);
        assert_eq!(nav("mailto:contact@kwant.si"), Link::Browser);
        // another port on this machine is another program, not kwant
        assert_eq!(nav("http://127.0.0.1:3000/"), Link::Browser);
        assert_eq!(nav("http://127.0.0.1/"), Link::Browser);
        // https on kwant's port isn't how the app reaches kwant
        assert_eq!(nav("https://127.0.0.1:8799/"), Link::Browser);
        // a look-alike host
        assert_eq!(nav("http://127.0.0.1.evil.example:8799/"), Link::Browser);
        assert_eq!(nav("https://localhost.example/"), Link::Browser);
    }

    #[test]
    fn other_schemes_are_dropped() {
        assert_eq!(nav("file:///etc/passwd"), Link::Drop);
        assert_eq!(nav("javascript:alert(1)"), Link::Drop);
        assert_eq!(nav("vscode://file/x"), Link::Drop);
        assert_eq!(nav("tauri://evil/index.html"), Link::Drop);
    }

    #[test]
    fn new_windows_never_open_in_the_app() {
        assert_eq!(popup("https://accounts.google.com/"), Link::Browser);
        assert_eq!(popup("http://127.0.0.1:8799/sandbox/team-1"), Link::Browser);
        assert_eq!(popup("mailto:a@b.c"), Link::Browser);
        // the blank popup a sign-in pre-opens is refused, so the page falls back to a navigation
        assert_eq!(popup("about:blank"), Link::Drop);
        assert_eq!(popup("tauri://localhost/index.html"), Link::Drop);
        assert_eq!(popup("http://tauri.localhost/index.html"), Link::Drop);
        assert_eq!(popup("file:///tmp/x"), Link::Drop);
    }
}
