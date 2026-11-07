//! Talking to kwant on a server from the app itself: the health check before connecting and at
//! start, and the tray's poll with the window's sign-in cookie.
//!
//! The cookie value lives in memory only ([`Token`] has no `Display` and hides itself in
//! `Debug`), goes only to the saved origin, and never into a log, a file or an error message.

use crate::target::Origin;
use std::time::Duration;

/// kwant's sign-in cookie (src/server/auth.rs `COOKIE`).
pub const COOKIE: &str = "kwnt_token";

/// The most of a health answer the app reads.
const HEALTH_LIMIT: usize = 64 * 1024;
/// The most of a poll answer the app reads (a long list of waiting items).
const POLL_LIMIT: usize = 2 * 1024 * 1024;

/// Why a server didn't pass the check.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Failure {
    Dns,
    Tls,
    Timeout,
    Refused,
    NotKwant,
}

impl Failure {
    pub fn code(self) -> &'static str {
        match self {
            Failure::Dns => "dns",
            Failure::Tls => "tls",
            Failure::Timeout => "timeout",
            Failure::Refused => "refused",
            Failure::NotKwant => "not_kwant",
        }
    }

    pub fn message(self, host: &str) -> String {
        match self {
            Failure::Dns => format!("Couldn't find {host}. Check the spelling, and that this computer is online."),
            Failure::Tls => format!("Couldn't connect safely to {host}. Its security certificate isn't valid."),
            Failure::Timeout => format!("{host} didn't answer in 10 seconds."),
            Failure::Refused => format!("Nothing answered at {host}. Check that kwant is running and the port is right."),
            Failure::NotKwant => format!("{host} answered, but not as kwant."),
        }
    }
}

/// The version from a /api/health answer, when it is kwant's.
pub fn parse_health(body: &str) -> Option<String> {
    let v: serde_json::Value = serde_json::from_str(body).ok()?;
    if v["app"].as_str() != Some("kwnt") {
        return None;
    }
    Some(v["version"].as_str().unwrap_or("").chars().take(40).collect())
}

/// Sorts a connection error by the words in its causes (reqwest, hyper, the OS and the TLS
/// library each word them their own way). Anything else reads as a timeout: "didn't answer" is
/// the honest catch-all, and the raw error is never shown.
pub fn classify(chain: &str, timed_out: bool) -> Failure {
    let text = chain.to_ascii_lowercase();
    let has = |words: &[&str]| words.iter().any(|w| text.contains(w));
    if has(&["dns error", "failed to lookup address", "name or service not known", "nodename nor servname", "no such host", "temporary failure in name resolution"]) {
        Failure::Dns
    } else if has(&["certificate", "tls", "ssl", "handshake", "trust"]) {
        Failure::Tls
    } else if has(&["connection refused", "refused", "connection reset"]) && !timed_out {
        Failure::Refused
    } else {
        Failure::Timeout
    }
}

fn failure(e: &reqwest::Error) -> Failure {
    let mut chain = String::new();
    let mut source: Option<&dyn std::error::Error> = Some(e);
    while let Some(err) = source {
        chain.push_str(&err.to_string());
        chain.push_str(" | ");
        source = err.source();
    }
    classify(&chain, e.is_timeout())
}

/// One client for the app's own calls: no redirects (a server must answer as itself), short
/// timeouts, no cookie store (the one cookie sent is set per request).
pub fn client() -> reqwest::Client {
    reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .connect_timeout(Duration::from_secs(10))
        .timeout(Duration::from_secs(10))
        .user_agent(concat!("kwant-desktop/", env!("CARGO_PKG_VERSION")))
        .build()
        .unwrap_or_default()
}

/// Reads at most `limit` bytes of the answer.
async fn body(mut response: reqwest::Response, limit: usize) -> Result<String, Failure> {
    let mut out = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(|e| failure(&e))? {
        out.extend_from_slice(&chunk);
        if out.len() > limit {
            return Err(Failure::NotKwant);
        }
    }
    Ok(String::from_utf8_lossy(&out).into_owned())
}

/// GET `{origin}/api/health`: kwant answers 200 with `"app":"kwnt"`. Gives its version.
pub async fn check(client: &reqwest::Client, origin: &Origin) -> Result<String, Failure> {
    let url = origin.page("/api/health");
    let response = client.get(url).send().await.map_err(|e| failure(&e))?;
    if response.status() != reqwest::StatusCode::OK {
        return Err(Failure::NotKwant);
    }
    parse_health(&body(response, HEALTH_LIMIT).await?).ok_or(Failure::NotKwant)
}

/// The window's sign-in cookie value for the saved server, in memory only.
#[derive(Clone, PartialEq, Eq)]
pub struct Token(String);

impl std::fmt::Debug for Token {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Token([redacted])")
    }
}

/// kwant's cookie among the window's cookies: the right name, set by exactly the server's host
/// (a cookie for a look-alike or a parent domain isn't taken), for the whole site.
pub fn pick_token<'a>(
    origin: &Origin,
    cookies: impl IntoIterator<Item = (&'a str, &'a str, Option<&'a str>, Option<&'a str>)>,
) -> Option<Token> {
    let host = origin.url().host_str()?.trim_start_matches('[').trim_end_matches(']').to_ascii_lowercase();
    cookies.into_iter().find_map(|(name, value, domain, path)| {
        let domain = domain?.trim_start_matches('.').trim_start_matches('[').trim_end_matches(']');
        let fits = name == COOKIE
            && domain.eq_ignore_ascii_case(&host)
            && path.is_none_or(|p| p == "/")
            && !value.is_empty()
            && value.bytes().all(|b| b.is_ascii_graphic() && b != b';' && b != b',');
        fits.then(|| Token(value.to_string()))
    })
}

/// What one authenticated GET gave.
#[derive(Debug, PartialEq, Eq)]
pub enum Answer {
    Ok(String),
    /// The cookie is missing, expired or revoked.
    SignedOut,
    Failed,
}

/// GET one of kwant's API paths on the server with the sign-in cookie.
pub async fn get(client: &reqwest::Client, origin: &Origin, token: Option<&Token>, path: &str) -> Answer {
    let mut request = client.get(origin.page(path));
    if let Some(Token(value)) = token {
        match reqwest::header::HeaderValue::from_str(&format!("{COOKIE}={value}")) {
            Ok(mut header) => {
                header.set_sensitive(true);
                request = request.header(reqwest::header::COOKIE, header);
            }
            Err(_) => return Answer::SignedOut,
        }
    }
    let Ok(response) = request.send().await else {
        return Answer::Failed;
    };
    match response.status().as_u16() {
        200 => body(response, POLL_LIMIT).await.map(Answer::Ok).unwrap_or(Answer::Failed),
        401 | 403 => Answer::SignedOut,
        _ => Answer::Failed,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::target::parse_address;

    fn origin(s: &str) -> Origin {
        parse_address(s).unwrap().origin
    }

    #[test]
    fn only_kwant_s_health_answer_passes() {
        assert_eq!(parse_health(r#"{"status":"ok","app":"kwnt","version":"0.7.5"}"#).as_deref(), Some("0.7.5"));
        assert_eq!(parse_health(r#"{"status":"ok","app":"kwnt"}"#).as_deref(), Some(""));
        assert_eq!(parse_health(r#"{"status":"ok","app":"other"}"#), None);
        assert_eq!(parse_health("<html>ok</html>"), None);
        assert_eq!(parse_health(""), None);
    }

    #[test]
    fn connection_errors_are_sorted_by_their_causes() {
        let dns = "error sending request | client error (Connect) | dns error: failed to lookup address information: Name or service not known";
        assert_eq!(classify(dns, false), Failure::Dns);
        assert_eq!(classify("client error (Connect) | The certificate was not trusted.", false), Failure::Tls);
        assert_eq!(classify("error trying to connect: invalid peer certificate: UnknownIssuer", false), Failure::Tls);
        assert_eq!(classify("tcp connect error: Connection refused (os error 111)", false), Failure::Refused);
        assert_eq!(classify("operation timed out", false), Failure::Timeout);
        assert_eq!(classify("error sending request", true), Failure::Timeout);
        assert_eq!(classify("connection reset by peer", false), Failure::Refused);
        assert_eq!(classify("something else", false), Failure::Timeout);
        assert_eq!(Failure::NotKwant.message("kwant.example.com"), "kwant.example.com answered, but not as kwant.");
        assert_eq!(Failure::Timeout.code(), "timeout");
    }

    #[test]
    fn the_cookie_is_taken_only_from_the_server_s_own_host() {
        let o = origin("https://kwant.example.com");
        let pick = |c: Vec<(&'static str, &'static str, Option<&'static str>, Option<&'static str>)>| pick_token(&o, c);
        let t = pick(vec![("kwnt_token", "kd1.TESTONLY", Some("kwant.example.com"), Some("/"))]).unwrap();
        assert_eq!(format!("{t:?}"), "Token([redacted])");
        assert!(pick(vec![("kwnt_token", "kd1.TESTONLY", Some(".kwant.example.com"), None)]).is_some());
        assert!(pick(vec![("kwnt_token", "kd1.TESTONLY", Some("KWANT.example.com"), Some("/"))]).is_some());
        // a look-alike, a parent domain, another name, another path, odd values
        assert!(pick(vec![("kwnt_token", "kd1.TESTONLY", Some("kwant.example.com.evil.example"), Some("/"))]).is_none());
        assert!(pick(vec![("kwnt_token", "kd1.TESTONLY", Some("example.com"), Some("/"))]).is_none());
        assert!(pick(vec![("session", "kd1.TESTONLY", Some("kwant.example.com"), Some("/"))]).is_none());
        assert!(pick(vec![("kwnt_token", "kd1.TESTONLY", Some("kwant.example.com"), Some("/api"))]).is_none());
        assert!(pick(vec![("kwnt_token", "", Some("kwant.example.com"), Some("/"))]).is_none());
        assert!(pick(vec![("kwnt_token", "a\r\nX: y", Some("kwant.example.com"), Some("/"))]).is_none());
        assert!(pick(vec![("kwnt_token", "kd1.TESTONLY", None, Some("/"))]).is_none());
        // an SSH tunnel: the webview keeps the IP (or [::1] without brackets) as the domain
        let tunnel = origin("http://127.0.0.1:2222");
        assert!(pick_token(&tunnel, vec![("kwnt_token", "kd1.TESTONLY", Some("127.0.0.1"), Some("/"))]).is_some());
        let v6 = origin("http://[::1]:2222");
        assert!(pick_token(&v6, vec![("kwnt_token", "kd1.TESTONLY", Some("::1"), Some("/"))]).is_some());
    }
}
