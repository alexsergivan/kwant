//! Which kwant the window shows: the one on this computer (`Local`) or one on a server
//! (`Remote`), saved in the data dir as `desktop-target.json`.
//!
//! Only an origin is ever saved: scheme, host and port. Never a token, a path, a query, a
//! fragment or a user name. A missing, unreadable or odd file means `Local`, which is how the
//! app has always started.

use std::path::{Path, PathBuf};
use tauri::Url;

pub const TARGET_FILE: &str = "desktop-target.json";

/// A server address the app may open: `https://host[:port]`, or `http://` on this computer's
/// loopback names (an SSH tunnel). Built only by [`parse_address`], so it never holds a path,
/// a query, a fragment or userinfo.
#[derive(Clone, PartialEq, Eq)]
pub struct Origin(Url);

impl Origin {
    /// `https://kwant.example.com` (no trailing slash; the default port left out).
    pub fn as_str(&self) -> String {
        self.0.origin().ascii_serialization()
    }

    /// The host as people read it: `kwant.example.com`, `127.0.0.1:2222`. IDN hosts stay in
    /// their xn-- form, so a look-alike name can't pass for a familiar one.
    pub fn host(&self) -> String {
        let host = self.0.host_str().unwrap_or_default();
        match self.0.port() {
            Some(port) => format!("{host}:{port}"),
            None => host.to_string(),
        }
    }

    pub fn url(&self) -> &Url {
        &self.0
    }

    /// One of kwant's pages on this server (`/s/spc_default/inbox`). Anything that would land
    /// on another origin (`@evil.example`, `//evil.example`) gives the server's front page.
    pub fn page(&self, path: &str) -> Url {
        let front = self.0.clone();
        if !path.starts_with('/') || path.starts_with("//") {
            return front;
        }
        match front.join(path) {
            Ok(url) if same_origin(&url, &self.0) => url,
            _ => front,
        }
    }

    /// Whether `url` is exactly this origin: the same scheme, host and port, and no userinfo.
    pub fn is(&self, url: &Url) -> bool {
        same_origin(url, &self.0)
    }
}

impl std::fmt::Debug for Origin {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Origin({})", self.as_str())
    }
}

fn same_origin(a: &Url, b: &Url) -> bool {
    a.scheme() == b.scheme()
        && a.host() == b.host()
        && a.port_or_known_default() == b.port_or_known_default()
        && a.username().is_empty()
        && a.password().is_none()
        && matches!(a.scheme(), "http" | "https")
}

/// A pasted one-time sign-in link (`https://host/#kwnt_token=…`). Held in memory only: it has
/// no `Display`, its `Debug` hides the token, and it is never written anywhere.
#[derive(Clone)]
pub struct SignInLink(Url);

impl SignInLink {
    /// The address the window opens once, so the server can set its cookie.
    pub fn url(&self) -> &Url {
        &self.0
    }
}

impl std::fmt::Debug for SignInLink {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("SignInLink([redacted])")
    }
}

/// What the person typed, once it passed the rules.
#[derive(Debug, Clone)]
pub struct Address {
    pub origin: Origin,
    pub sign_in: Option<SignInLink>,
}

/// Why an address was refused before anything was contacted.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Refusal {
    Empty,
    NotAnAddress,
    HttpNotAllowed,
    Userinfo,
}

impl Refusal {
    pub fn code(self) -> &'static str {
        match self {
            Refusal::Empty => "empty",
            Refusal::NotAnAddress => "not_an_address",
            Refusal::HttpNotAllowed => "http_not_allowed",
            Refusal::Userinfo => "userinfo",
        }
    }

    pub fn message(self) -> &'static str {
        match self {
            Refusal::Empty => "Type the address of your kwant server, for example kwant.example.com.",
            Refusal::NotAnAddress => "That isn't a web address. Try something like kwant.example.com.",
            Refusal::HttpNotAllowed => {
                "Use https:// for a server. Plain http:// works only for this computer (127.0.0.1 or localhost), for example through an SSH tunnel."
            }
            Refusal::Userinfo => {
                "Leave out the user name and password. kwant asks you to sign in after it connects."
            }
        }
    }
}

const SIGN_IN_MARK: &str = "kwnt_token=";

/// The loopback names plain http is allowed on.
fn is_loopback_name(host: &str) -> bool {
    matches!(host, "127.0.0.1" | "localhost" | "[::1]")
}

/// Reads an address: `kwant.example.com`, `https://kwant.example.com`, `http://127.0.0.1:2222`
/// or a pasted sign-in link. No scheme means https, or http on a loopback name (an SSH tunnel
/// speaks plain http). Paths and queries are ignored (a pasted
/// page address works); http only on loopback; userinfo never.
pub fn parse_address(input: &str) -> Result<Address, Refusal> {
    let text = input.trim();
    if text.is_empty() {
        return Err(Refusal::Empty);
    }
    if text.chars().any(char::is_whitespace) {
        return Err(Refusal::NotAnAddress);
    }
    let mut url: Url = if has_scheme(text) {
        text.parse().map_err(|_| Refusal::NotAnAddress)?
    } else {
        let secure: Url = format!("https://{text}").parse().map_err(|_| Refusal::NotAnAddress)?;
        // `localhost.:2222` is the loopback name too (the trailing dot goes below)
        match secure.host_str() {
            Some(h) if is_loopback_name(h.strip_suffix('.').unwrap_or(h)) => {
                format!("http://{text}").parse().map_err(|_| Refusal::NotAnAddress)?
            }
            _ => secure,
        }
    };
    if !matches!(url.scheme(), "http" | "https") {
        return Err(Refusal::NotAnAddress);
    }
    if !url.username().is_empty() || url.password().is_some() {
        return Err(Refusal::Userinfo);
    }
    let Some(host) = url.host_str().map(str::to_string) else {
        return Err(Refusal::NotAnAddress);
    };
    // an empty label (`a..b.example.com`, `.example.com`) is no host name
    if host.is_empty() || host.starts_with('.') || host.contains("..") {
        return Err(Refusal::NotAnAddress);
    }
    // `kwant.example.com.` and `kwant.example.com` are one server to a person; save the plain form
    if let Some(trimmed) = host.strip_suffix('.') {
        if trimmed.is_empty() || trimmed.ends_with('.') || url.set_host(Some(trimmed)).is_err() {
            return Err(Refusal::NotAnAddress);
        }
    }
    let host = url.host_str().unwrap_or_default().to_string();
    if url.scheme() == "http" && !is_loopback_name(&host) {
        return Err(Refusal::HttpNotAllowed);
    }
    let fragment = url.fragment().map(str::to_string);
    let mut origin = url.clone();
    origin.set_path("/");
    origin.set_query(None);
    origin.set_fragment(None);
    let sign_in = fragment
        .filter(|f| f.split('&').any(|p| p.starts_with(SIGN_IN_MARK) && p.len() > SIGN_IN_MARK.len()))
        .map(|f| {
            let mut link = origin.clone();
            link.set_fragment(Some(&f));
            SignInLink(link)
        });
    Ok(Address { origin: Origin(origin), sign_in })
}

/// `scheme://…` at the start. `localhost:8080` has no scheme (it is a host and a port).
fn has_scheme(text: &str) -> bool {
    match text.find("://") {
        Some(i) => {
            let scheme = &text[..i];
            !scheme.is_empty()
                && scheme.chars().next().is_some_and(|c| c.is_ascii_alphabetic())
                && scheme.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.'))
        }
        None => false,
    }
}

/// The saved choice.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Saved {
    Local {
        /// The server used last, offered again by the connect page.
        last: Option<Origin>,
    },
    Remote(Origin),
}

pub fn target_path(data_dir: &Path) -> PathBuf {
    data_dir.join(TARGET_FILE)
}

/// A saved origin, accepted only when it is already in its exact saved form.
fn saved_origin(value: &serde_json::Value) -> Option<Origin> {
    let text = value.as_str()?;
    let address = parse_address(text).ok()?;
    (address.sign_in.is_none() && address.origin.as_str() == text).then_some(address.origin)
}

impl Saved {
    /// Missing, unreadable or odd means `Local`.
    pub fn load(data_dir: &Path) -> Saved {
        let saved = std::fs::read_to_string(target_path(data_dir))
            .ok()
            .and_then(|s| serde_json::from_str::<serde_json::Value>(&s).ok());
        let Some(v) = saved else {
            return Saved::Local { last: None };
        };
        match (v["mode"].as_str(), saved_origin(&v["origin"])) {
            (Some("remote"), Some(origin)) => Saved::Remote(origin),
            _ => Saved::Local { last: saved_origin(&v["last_origin"]) },
        }
    }

    /// Only the migration tests write the old file now; the app moves it to desktop-places.json.
    #[cfg(test)]
    pub fn save(&self, data_dir: &Path) -> std::io::Result<()> {
        let v = match self {
            Saved::Remote(origin) => serde_json::json!({ "mode": "remote", "origin": origin.as_str() }),
            Saved::Local { last: Some(o) } => serde_json::json!({ "mode": "local", "last_origin": o.as_str() }),
            Saved::Local { last: None } => serde_json::json!({ "mode": "local" }),
        };
        std::fs::create_dir_all(data_dir)?;
        let path = target_path(data_dir);
        let tmp = path.with_extension("json.tmp");
        std::fs::write(&tmp, serde_json::to_vec_pretty(&v).unwrap_or_default())?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o600));
        }
        std::fs::rename(tmp, path)
    }
}

/// Where the window points for this run of the app.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Home {
    /// kwant on this computer, on loopback.
    Local { port: u16 },
    /// kwant on a server.
    Remote { origin: Origin },
}

impl Home {
    /// One of kwant's pages (`/` or a nudge's item).
    pub fn page(&self, path: &str) -> Option<Url> {
        match self {
            Home::Local { port } => {
                let path = if path.starts_with('/') && !path.starts_with("//") { path } else { "/" };
                format!("http://127.0.0.1:{port}{path}").parse().ok()
            }
            Home::Remote { origin } => Some(origin.page(path)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn origin(s: &str) -> String {
        parse_address(s).unwrap().origin.as_str()
    }

    #[test]
    fn a_bare_host_becomes_https() {
        assert_eq!(origin("kwant.example.com"), "https://kwant.example.com");
        assert_eq!(origin("  kwant.example.com  "), "https://kwant.example.com");
        assert_eq!(origin("KWANT.Example.COM"), "https://kwant.example.com");
        assert_eq!(origin("kwant.example.com:8443"), "https://kwant.example.com:8443");
        assert_eq!(origin("https://kwant.example.com:443"), "https://kwant.example.com");
        // a host and a port is not a scheme; a loopback name without one is an SSH tunnel: http
        assert_eq!(origin("localhost:2222"), "http://localhost:2222");
        assert_eq!(origin("127.0.0.1:2222"), "http://127.0.0.1:2222");
        assert_eq!(origin("[::1]:2222"), "http://[::1]:2222");
        assert_eq!(origin("https://localhost:8443"), "https://localhost:8443");
        assert_eq!(origin("localhost.example.com"), "https://localhost.example.com");
    }

    #[test]
    fn paths_queries_and_other_fragments_are_dropped() {
        let a = parse_address("https://kwant.example.com/s/spc_default/inbox?x=1#/missions").unwrap();
        assert_eq!(a.origin.as_str(), "https://kwant.example.com");
        assert!(a.sign_in.is_none());
        assert_eq!(a.origin.url().as_str(), "https://kwant.example.com/");
    }

    #[test]
    fn http_only_on_loopback() {
        assert_eq!(origin("http://127.0.0.1:2222"), "http://127.0.0.1:2222");
        assert_eq!(origin("http://localhost:2222/"), "http://localhost:2222");
        assert_eq!(origin("http://[::1]:2222"), "http://[::1]:2222");
        assert_eq!(parse_address("http://kwant.example.com").unwrap_err(), Refusal::HttpNotAllowed);
        assert_eq!(parse_address("http://10.0.0.5:8800").unwrap_err(), Refusal::HttpNotAllowed);
        assert_eq!(parse_address("http://127.0.0.1.nip.io").unwrap_err(), Refusal::HttpNotAllowed);
        assert_eq!(parse_address("http://localhost.example.com").unwrap_err(), Refusal::HttpNotAllowed);
    }

    #[test]
    fn userinfo_and_non_web_addresses_are_refused() {
        assert_eq!(parse_address("https://a@kwant.example.com").unwrap_err(), Refusal::Userinfo);
        assert_eq!(parse_address("https://a:b@kwant.example.com").unwrap_err(), Refusal::Userinfo);
        assert_eq!(parse_address("a@kwant.example.com").unwrap_err(), Refusal::Userinfo);
        assert_eq!(parse_address("").unwrap_err(), Refusal::Empty);
        assert_eq!(parse_address("   ").unwrap_err(), Refusal::Empty);
        for bad in ["ftp://kwant.example.com", "file:///etc/passwd", "javascript://alert(1)", "kwant example.com", "https://", "https://."] {
            assert_eq!(parse_address(bad).unwrap_err(), Refusal::NotAnAddress, "{bad}");
        }
    }

    #[test]
    fn idn_hosts_are_saved_in_xn_form_and_a_trailing_dot_is_dropped() {
        assert_eq!(origin("https://kwänt.example"), "https://xn--kwnt-moa.example");
        assert_eq!(parse_address("https://kwänt.example").unwrap().origin.host(), "xn--kwnt-moa.example");
        assert_eq!(origin("https://kwant.example.com."), "https://kwant.example.com");
        assert_eq!(parse_address("https://kwant.example.com..").unwrap_err(), Refusal::NotAnAddress);
    }

    /// Rigel's F8: a trailing dot on a loopback name is the same loopback name, with or
    /// without a scheme.
    #[test]
    fn a_loopback_name_with_a_trailing_dot_is_still_loopback() {
        assert_eq!(origin("localhost.:2222"), "http://localhost:2222");
        assert_eq!(origin("http://localhost.:2222"), "http://localhost:2222");
        assert_eq!(origin("localhost."), "http://localhost");
        assert_eq!(origin("https://localhost.:8443"), "https://localhost:8443");
        assert_eq!(origin("localhost.example.com.:2222"), "https://localhost.example.com:2222");
        assert_eq!(parse_address("http://localhost.example.com.").unwrap_err(), Refusal::HttpNotAllowed);
    }

    /// Rigel's F9: ".." in a host is refused before anything is contacted, as the spec says.
    #[test]
    fn an_empty_label_in_a_host_is_refused() {
        for bad in [
            "https://a..b.example.com",
            "a..b.example.com",
            "https://kwant..example.com:8443/x",
            "https://kwant.example.com../",
            "https://..kwant.example.com",
            "https://.kwant.example.com",
            "http://localhost..:2222",
        ] {
            assert_eq!(parse_address(bad).unwrap_err(), Refusal::NotAnAddress, "{bad}");
        }
    }

    #[test]
    fn a_sign_in_link_keeps_only_the_origin_and_the_fragment() {
        let a = parse_address("https://kwant.example.com/some/path?q=1#kwnt_token=kl1.TESTONLY").unwrap();
        assert_eq!(a.origin.as_str(), "https://kwant.example.com");
        let link = a.sign_in.as_ref().expect("a sign-in link");
        assert_eq!(link.url().as_str(), "https://kwant.example.com/#kwnt_token=kl1.TESTONLY");
        assert_eq!(format!("{link:?}"), "SignInLink([redacted])");
        assert!(!format!("{a:?}").contains("TESTONLY"));
        // a bare host with the mark still gets https
        assert!(parse_address("kwant.example.com/#kwnt_token=kl1.TESTONLY").unwrap().sign_in.is_some());
        // an empty token is no link
        assert!(parse_address("https://kwant.example.com/#kwnt_token=").unwrap().sign_in.is_none());
        // a sign-in link to plain http elsewhere is still refused
        assert_eq!(
            parse_address("http://kwant.example.com/#kwnt_token=kl1.TESTONLY").unwrap_err(),
            Refusal::HttpNotAllowed
        );
    }

    fn dir(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("kwant-target-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn no_file_means_local() {
        let d = dir("none");
        assert_eq!(Saved::load(&d), Saved::Local { last: None });
        let _ = std::fs::remove_dir_all(d);
    }

    #[test]
    fn a_remote_choice_round_trips_as_the_origin_only() {
        let d = dir("round");
        let a = parse_address("https://kwant.example.com/x#kwnt_token=kl1.TESTONLY").unwrap();
        Saved::Remote(a.origin.clone()).save(&d).unwrap();
        let text = std::fs::read_to_string(target_path(&d)).unwrap();
        assert!(!text.contains("TESTONLY") && !text.contains("kwnt_token") && !text.contains("/x"), "{text}");
        let v: serde_json::Value = serde_json::from_str(&text).unwrap();
        assert_eq!(v, serde_json::json!({"mode": "remote", "origin": "https://kwant.example.com"}));
        assert_eq!(Saved::load(&d), Saved::Remote(a.origin.clone()));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(target_path(&d)).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode, 0o600);
        }
        Saved::Local { last: Some(a.origin.clone()) }.save(&d).unwrap();
        assert_eq!(Saved::load(&d), Saved::Local { last: Some(a.origin) });
        let _ = std::fs::remove_dir_all(d);
    }

    #[test]
    fn an_odd_file_means_local() {
        let d = dir("odd");
        for text in [
            "not json",
            r#"{"mode":"remote"}"#,
            r#"{"mode":"remote","origin":"http://kwant.example.com"}"#,
            r#"{"mode":"remote","origin":"https://kwant.example.com/path"}"#,
            r#"{"mode":"remote","origin":"https://kwant.example.com/#kwnt_token=kl1.TESTONLY"}"#,
            r#"{"mode":"remote","origin":"https://a@kwant.example.com"}"#,
            r#"{"mode":"remote","origin":"https://kwant.example.com/"}"#,
            r#"{"mode":"remote","origin":42}"#,
            r#"{"mode":"somewhere","origin":"https://kwant.example.com"}"#,
        ] {
            std::fs::write(target_path(&d), text).unwrap();
            assert!(matches!(Saved::load(&d), Saved::Local { .. }), "{text}");
        }
        let _ = std::fs::remove_dir_all(d);
    }

    #[test]
    fn pages_never_leave_the_origin() {
        let o = parse_address("https://kwant.example.com").unwrap().origin;
        assert_eq!(o.page("/s/spc_default/inbox").as_str(), "https://kwant.example.com/s/spc_default/inbox");
        assert_eq!(o.page("/").as_str(), "https://kwant.example.com/");
        assert_eq!(o.page("@evil.example/").as_str(), "https://kwant.example.com/");
        assert_eq!(o.page("//evil.example/").as_str(), "https://kwant.example.com/");
        assert_eq!(o.page("https://evil.example/").as_str(), "https://kwant.example.com/");
        let local = Home::Local { port: 8123 };
        assert_eq!(local.page("/x").unwrap().as_str(), "http://127.0.0.1:8123/x");
        assert_eq!(local.page("@evil.example").unwrap().as_str(), "http://127.0.0.1:8123/");
    }
}
