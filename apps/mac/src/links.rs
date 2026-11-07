//! Where a link goes. Each place's window only ever shows its own kwant (its own page on
//! loopback for This Mac, or exactly the server's origin) and the bundled pages ("Starting
//! kwant…", "Add a server"). A link to another listed place opens (or focuses) that place's
//! window (spec P3); any other web or mail address opens in the person's default browser, so a
//! sign-in page can never take over a window with no way back.

use crate::target::Home;
use tauri::Url;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Link {
    /// Load it in the kwant window.
    Stay,
    /// Cancel it here and open it in the default browser.
    Browser,
    /// Cancel it and open nothing (file:, javascript:, other apps' schemes).
    Drop,
    /// Cancel it here and open it in another place's window: the index in `others`.
    Place(usize),
}

/// The bundled pages (tauri://localhost on macOS, http://tauri.localhost elsewhere): exactly
/// that origin, with no port and no user name or password. Tauri's own access check compares
/// only the scheme and the host, so `tauri://localhost:8080/` or `http://u@tauri.localhost/`
/// must never count as the app's own page here.
pub fn is_bundled(url: &Url) -> bool {
    let host = match url.scheme() {
        "tauri" => "localhost",
        "http" => "tauri.localhost",
        _ => return false,
    };
    url.host_str() == Some(host) && url.port().is_none() && url.username().is_empty() && url.password().is_none()
}

/// kwant's server itself, which the browser can reach too (not the bundled pages): loopback on
/// exactly this port in Local mode, exactly the saved origin in Remote mode.
fn is_kwant_server(url: &Url, home: &Home) -> bool {
    match home {
        Home::Local { port } => {
            url.scheme() == "http"
                && matches!(url.host_str(), Some("127.0.0.1" | "localhost" | "[::1]"))
                && url.port() == Some(*port)
        }
        Home::Remote { origin } => origin.is(url),
    }
}

/// A top-level navigation in a place's window. `others` are the homes of the other places the
/// app lists (This Mac and the servers), in the order the caller keeps.
pub fn navigation(url: &Url, home: &Home, others: &[Home]) -> Link {
    if is_bundled(url) || is_kwant_server(url, home) {
        return Link::Stay;
    }
    if let Some(i) = others.iter().position(|h| is_kwant_server(url, h)) {
        return Link::Place(i);
    }
    match url.scheme() {
        // blank pages and in-page content kwant makes itself (downloads, previews)
        "about" | "blob" | "data" => Link::Stay,
        "http" | "https" | "mailto" => Link::Browser,
        _ => Link::Drop,
    }
}

/// A window.open() or target=_blank request. A place never opens a second window of its own:
/// real addresses, its own kwant's included, go to the browser (sandbox pages and the like); a
/// blank popup is refused, so the page's window.open() gets null and falls back to a navigation
/// that `navigation` sends on. A link to another place opens that place's window.
pub fn new_window(url: &Url, home: &Home, others: &[Home]) -> Link {
    match navigation(url, home, others) {
        Link::Stay if is_kwant_server(url, home) => Link::Browser,
        Link::Stay => Link::Drop,
        other => other,
    }
}

/// The program that hands an address to the default browser: `open` on macOS, `xdg-open`
/// (looked up on PATH) on Linux desktops.
pub const fn browser_opener() -> &'static str {
    if cfg!(target_os = "macos") {
        "/usr/bin/open"
    } else {
        "xdg-open"
    }
}

/// Opens the address in the default browser ([`browser_opener`]). Only http(s) and mailto reach here.
pub fn open_in_browser(url: &Url) {
    let url = url.as_str().to_string();
    std::thread::spawn(
        move || match std::process::Command::new(browser_opener()).arg(&url).status() {
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
    const LOCAL: Home = Home::Local { port: PORT };

    fn nav(s: &str) -> Link {
        navigation(&s.parse().unwrap(), &LOCAL, &[])
    }

    fn popup(s: &str) -> Link {
        new_window(&s.parse().unwrap(), &LOCAL, &[])
    }

    fn remote(s: &str) -> Home {
        Home::Remote { origin: crate::target::parse_address(s).unwrap().origin }
    }

    fn rnav(home: &str, s: &str) -> Link {
        navigation(&s.parse().unwrap(), &remote(home), &[])
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
    fn links_open_with_the_desktop_s_own_opener() {
        #[cfg(target_os = "macos")]
        assert_eq!(browser_opener(), "/usr/bin/open");
        #[cfg(target_os = "linux")]
        assert_eq!(browser_opener(), "xdg-open");
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

    #[test]
    fn remote_mode_keeps_exactly_the_saved_origin() {
        let h = "https://kwant.example.com";
        assert_eq!(rnav(h, "https://kwant.example.com/"), Link::Stay);
        assert_eq!(rnav(h, "https://kwant.example.com/s/spc_default/inbox?x=1#/missions"), Link::Stay);
        assert_eq!(rnav(h, "https://kwant.example.com:443/"), Link::Stay);
        assert_eq!(rnav(h, "https://KWANT.example.com/"), Link::Stay);
        // the bundled pages and in-page content stay too
        assert_eq!(rnav(h, "tauri://localhost/connect.html"), Link::Stay);
        assert_eq!(rnav(h, "http://tauri.localhost/index.html"), Link::Stay);
        assert_eq!(rnav(h, "about:blank"), Link::Stay);
        assert_eq!(rnav(h, "blob:https://kwant.example.com/5f0c"), Link::Stay);
        assert_eq!(rnav(h, "data:text/plain,hi"), Link::Stay);
    }

    /// Rigel's F7: a port or a user name makes it another page, not the app's own.
    #[test]
    fn the_bundled_pages_are_exactly_their_origin() {
        for s in ["tauri://localhost/connect.html", "http://tauri.localhost/index.html", "http://TAURI.localhost/", "tauri://localhost/../../x"] {
            assert!(is_bundled(&s.parse().unwrap()), "{s}");
        }
        for s in [
            "http://tauri.localhost:8080/",
            "http://tauri.localhost:1/x",
            "tauri://localhost:8080/",
            "http://u:p@tauri.localhost/",
            "http://u@tauri.localhost/",
            "tauri://u@localhost/",
            "tauri://u:p@localhost/",
            "https://tauri.localhost/",
            "http://tauri.localhost./",
            "tauri://localhost.evil.example/",
        ] {
            assert!(!is_bundled(&s.parse().unwrap()), "{s}");
            for home in [LOCAL, remote("https://kwant.example.com")] {
                assert_ne!(navigation(&s.parse().unwrap(), &home, &[]), Link::Stay, "{s} in {home:?}");
            }
        }
        assert_eq!(rnav("https://kwant.example.com", "tauri://localhost:8080/"), Link::Drop);
        assert_eq!(rnav("https://kwant.example.com", "http://u:p@tauri.localhost/"), Link::Browser);
    }

    #[test]
    fn remote_mode_sends_everything_else_to_the_browser() {
        let h = "https://kwant.example.com";
        // another port, another scheme
        assert_eq!(rnav(h, "https://kwant.example.com:8443/"), Link::Browser);
        assert_eq!(rnav(h, "http://kwant.example.com/"), Link::Browser);
        // look-alike hosts
        assert_eq!(rnav(h, "https://kwant.example.com.evil.example/"), Link::Browser);
        assert_eq!(rnav(h, "https://evil-kwant.example.com/"), Link::Browser);
        assert_eq!(rnav(h, "https://www.kwant.example.com/"), Link::Browser);
        assert_eq!(rnav(h, "https://example.com/"), Link::Browser);
        // userinfo, the classic `trusted@evil` trick and a name in front of the right host
        assert_eq!(rnav(h, "https://a@kwant.example.com/"), Link::Browser);
        assert_eq!(rnav(h, "https://a:b@kwant.example.com/"), Link::Browser);
        assert_eq!(rnav(h, "https://kwant.example.com@evil.example/"), Link::Browser);
        // a trailing dot is a different origin to the webview (and its cookies)
        assert_eq!(rnav(h, "https://kwant.example.com./"), Link::Browser);
        // loopback is not kwant in Remote mode
        assert_eq!(rnav(h, "http://127.0.0.1:8799/"), Link::Browser);
        assert_eq!(rnav(h, "file:///etc/passwd"), Link::Drop);
        assert_eq!(rnav(h, "javascript:alert(1)"), Link::Drop);
        assert_eq!(rnav(h, "tauri://evil/index.html"), Link::Drop);
    }

    #[test]
    fn remote_mode_compares_idn_hosts_in_punycode() {
        let h = "https://kwänt.example";
        // the same name, typed either way
        assert_eq!(rnav(h, "https://kwänt.example/"), Link::Stay);
        assert_eq!(rnav(h, "https://xn--kwnt-moa.example/"), Link::Stay);
        // a look-alike (Cyrillic а for a) is another host
        assert_eq!(rnav(h, "https://kw\u{430}nt.example/"), Link::Browser);
        assert_eq!(rnav("https://kwant.example.com", "https://kw\u{430}nt.example.com/"), Link::Browser);
        assert_eq!(rnav("https://kwant.example.com", "https://xn--kwnt-5cd.example.com/"), Link::Browser);
    }

    #[test]
    fn a_loopback_tunnel_is_kept_on_its_exact_port() {
        let h = "http://127.0.0.1:2222";
        assert_eq!(rnav(h, "http://127.0.0.1:2222/s/x"), Link::Stay);
        assert_eq!(rnav(h, "http://127.0.0.1:2223/"), Link::Browser);
        assert_eq!(rnav(h, "http://localhost:2222/"), Link::Browser);
        assert_eq!(rnav(h, "https://127.0.0.1:2222/"), Link::Browser);
    }

    #[test]
    fn local_mode_keeps_today_s_loopback_rules() {
        // a remote kwant is just another site in Local mode
        assert_eq!(nav("https://kwant.example.com/"), Link::Browser);
        assert_eq!(nav("http://127.0.0.1:8799/"), Link::Stay);
        assert_eq!(nav("http://localhost:8799/"), Link::Stay);
        assert_eq!(nav("http://127.0.0.1:8798/"), Link::Browser);
        assert_eq!(nav("https://127.0.0.1:8799/"), Link::Browser);
    }

    /// P3: a link to another listed place opens that place's window, with the same exact-origin
    /// rules as a window's own home; everything else still goes to the browser.
    #[test]
    fn a_link_to_another_place_opens_its_window() {
        let a = remote("https://kwant.example.com");
        let b = remote("http://127.0.0.1:2222");
        // This Mac's window, with two servers listed
        let others = [a.clone(), b.clone()];
        let go = |s: &str| navigation(&s.parse().unwrap(), &LOCAL, &others);
        assert_eq!(go("https://kwant.example.com/s/spc_default/inbox"), Link::Place(0));
        assert_eq!(go("https://KWANT.example.com:443/"), Link::Place(0));
        assert_eq!(go("http://127.0.0.1:2222/s/x"), Link::Place(1));
        assert_eq!(go("http://127.0.0.1:8799/"), Link::Stay, "its own kwant");
        for s in [
            "https://kwant.example.com.evil.example/",
            "https://a@kwant.example.com/",
            "https://kwant.example.com:8443/",
            "http://kwant.example.com/",
            "https://kwant.example.com./",
            "http://localhost:2222/",
            "http://127.0.0.1:2223/",
        ] {
            assert_eq!(go(s), Link::Browser, "{s}");
        }
        // a server's window: This Mac and the other server are places too
        let others = [LOCAL, b];
        let go = |s: &str| navigation(&s.parse().unwrap(), &a, &others);
        assert_eq!(go("https://kwant.example.com/x"), Link::Stay);
        assert_eq!(go("http://127.0.0.1:8799/s/spc_default/inbox"), Link::Place(0));
        assert_eq!(go("http://localhost:8799/"), Link::Place(0));
        assert_eq!(go("http://127.0.0.1:2222/"), Link::Place(1));
        assert_eq!(go("http://127.0.0.1:8798/"), Link::Browser);
        assert_eq!(go("file:///etc/passwd"), Link::Drop);
        // a popup to another place opens that place, to its own server goes to the browser
        let pop = |s: &str| new_window(&s.parse().unwrap(), &a, &others);
        assert_eq!(pop("http://127.0.0.1:8799/s/x"), Link::Place(0));
        assert_eq!(pop("https://kwant.example.com/sandbox/team-1"), Link::Browser);
        assert_eq!(pop("about:blank"), Link::Drop);
    }

    #[test]
    fn a_sign_in_link_stays_in_its_own_server_s_window() {
        let a = remote("https://kwant.example.com");
        let go = |s: &str| navigation(&s.parse().unwrap(), &a, &[LOCAL]);
        assert_eq!(go("https://kwant.example.com/#kwnt_token=kl1.TESTONLY"), Link::Stay);
        assert_eq!(go("https://kwant.example.com.evil.example/#kwnt_token=kl1.TESTONLY"), Link::Browser);
        assert_eq!(go("https://a@kwant.example.com/"), Link::Browser);
    }

    #[test]
    fn remote_new_windows_go_to_the_browser() {
        let h = remote("https://kwant.example.com");
        let pop = |s: &str| new_window(&s.parse().unwrap(), &h, &[]);
        assert_eq!(pop("https://kwant.example.com/sandbox/team-1"), Link::Browser);
        assert_eq!(pop("https://evil.example/"), Link::Browser);
        assert_eq!(pop("about:blank"), Link::Drop);
        assert_eq!(pop("tauri://localhost/connect.html"), Link::Drop);
        assert_eq!(pop("file:///tmp/x"), Link::Drop);
    }
}
