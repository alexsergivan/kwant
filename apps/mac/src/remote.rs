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

/// Sorts a connection error. The kinds come first: reqwest's own timeout flag, then the OS
/// error kind of an `io::Error` among the causes. Only then the words of the causes (hyper, the
/// OS resolver and the TLS library each word them their own way), with every copy of the host
/// taken out first: the URL and the host are in some of those texts, and a host may well be
/// called `trust.example.com` or `refused.example.com`. Anything else reads as a timeout:
/// "didn't answer" is the honest catch-all, and the raw error is never shown.
pub fn classify(chain: &str, timed_out: bool, host: &str) -> Failure {
    if timed_out {
        return Failure::Timeout;
    }
    let text = without_addresses(&chain.to_ascii_lowercase(), &host.to_ascii_lowercase());
    let has = |words: &[&str]| words.iter().any(|w| text.contains(w));
    if has(&["dns error", "failed to lookup address", "name or service not known", "nodename nor servname", "no such host", "temporary failure in name resolution"]) {
        Failure::Dns
    } else if has(&["certificate", "tls", "ssl", "handshake", "trust"]) {
        Failure::Tls
    } else if has(&["connection refused", "refused", "connection reset"]) {
        Failure::Refused
    } else {
        Failure::Timeout
    }
}

/// `text` with every web address and every copy of the host (bare, or as `host:port`) blanked.
fn without_addresses(text: &str, host: &str) -> String {
    let edge = |c: char| c.is_whitespace() || matches!(c, '(' | ')' | '"' | '\'' | '“' | '”' | '<' | '>' | ',');
    let mut out = String::with_capacity(text.len());
    for word in text.split_inclusive(edge) {
        if word.contains("://") {
            out.push(' ');
            out.extend(word.chars().last().filter(|&c| edge(c)));
        } else {
            out.push_str(word);
        }
    }
    let host = host.trim_start_matches('[').trim_end_matches(']');
    if host.is_empty() {
        out
    } else {
        out.replace(host, " ")
    }
}

/// The kind of the first `io::Error` among the causes, if it settles the case.
fn io_kind(e: &reqwest::Error) -> Option<Failure> {
    let mut source = std::error::Error::source(e);
    while let Some(err) = source {
        if let Some(io) = err.downcast_ref::<std::io::Error>() {
            use std::io::ErrorKind::*;
            match io.kind() {
                TimedOut => return Some(Failure::Timeout),
                ConnectionRefused | ConnectionReset | ConnectionAborted => return Some(Failure::Refused),
                _ => {}
            }
        }
        source = err.source();
    }
    None
}

fn failure(e: &reqwest::Error, origin: &Origin) -> Failure {
    if e.is_timeout() {
        return Failure::Timeout;
    }
    if let Some(kind) = io_kind(e) {
        return kind;
    }
    // the causes only: reqwest's own line is "error sending request for url (…)"
    let mut chain = String::new();
    let mut source = std::error::Error::source(e);
    while let Some(err) = source {
        chain.push_str(&err.to_string());
        chain.push_str(" | ");
        source = err.source();
    }
    let host = origin.url().host_str().unwrap_or_default();
    classify(&chain, false, host)
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
async fn body(mut response: reqwest::Response, limit: usize, origin: &Origin) -> Result<String, Failure> {
    let mut out = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(|e| failure(&e, origin))? {
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
    let response = client.get(url).send().await.map_err(|e| failure(&e, origin))?;
    if response.status() != reqwest::StatusCode::OK {
        return Err(Failure::NotKwant);
    }
    parse_health(&body(response, HEALTH_LIMIT, origin).await?).ok_or(Failure::NotKwant)
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

/// How long Remove waits for the server to sign this app out.
pub const SIGN_OUT_TIMEOUT: Duration = Duration::from_secs(5);

/// `DELETE {origin}/api/auth/session` with the server's own sign-in cookie: the server revokes
/// that session (and drops its push subscription), so the app leaves Signed-in devices.
pub fn sign_out_request(
    client: &reqwest::Client,
    origin: &Origin,
    token: &Token,
    limit: Duration,
) -> reqwest::RequestBuilder {
    client
        .delete(origin.page("/api/auth/session"))
        .header(reqwest::header::COOKIE, format!("{COOKIE}={}", token.0))
        .timeout(limit)
}

/// Signs this app out of a server. `true` only for a 2xx answer; no token, an error, another
/// status or the time limit is `false`. Never logs the token or the server's reply.
pub async fn sign_out(client: &reqwest::Client, origin: &Origin, token: Option<&Token>) -> bool {
    sign_out_within(client, origin, token, SIGN_OUT_TIMEOUT).await
}

async fn sign_out_within(client: &reqwest::Client, origin: &Origin, token: Option<&Token>, limit: Duration) -> bool {
    let Some(token) = token else { return false };
    match sign_out_request(client, origin, token, limit).send().await {
        Ok(response) => response.status().is_success(),
        Err(e) => {
            tracing::info!(host = %origin.host(), kind = failure(&e, origin).code(), "couldn't sign out of a server");
            false
        }
    }
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
        200 => body(response, POLL_LIMIT, origin).await.map(Answer::Ok).unwrap_or(Answer::Failed),
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
        let h = "kwant.example.com";
        let dns = "client error (Connect) | dns error: failed to lookup address information: Name or service not known";
        assert_eq!(classify(dns, false, h), Failure::Dns);
        assert_eq!(classify("client error (Connect) | The certificate was not trusted.", false, h), Failure::Tls);
        assert_eq!(classify("error trying to connect: invalid peer certificate: UnknownIssuer", false, h), Failure::Tls);
        assert_eq!(classify("tcp connect error: Connection refused (os error 111)", false, h), Failure::Refused);
        assert_eq!(classify("operation timed out", false, h), Failure::Timeout);
        assert_eq!(classify("error sending request", true, h), Failure::Timeout);
        assert_eq!(classify("connection reset by peer", false, h), Failure::Refused);
        assert_eq!(classify("something else", false, h), Failure::Timeout);
        assert_eq!(Failure::NotKwant.message("kwant.example.com"), "kwant.example.com answered, but not as kwant.");
        assert_eq!(Failure::Timeout.code(), "timeout");
    }

    /// Rigel's QA traps (F1): reqwest puts the URL in its text, the macOS TLS library names the
    /// host, and a host can contain the classifier's words.
    #[test]
    fn a_host_s_name_never_decides_the_failure() {
        for (chain, timed_out, host, want) in [
            ("error sending request for url (https://trust.example.com/api/health) | operation timed out", true, "trust.example.com", Failure::Timeout),
            ("error sending request for url (https://ssl-kwant.example.com/api/health) | operation timed out", true, "ssl-kwant.example.com", Failure::Timeout),
            ("error sending request for url (https://tls.example.com/api/health) | dns error: failed to lookup address information", false, "tls.example.com", Failure::Dns),
            ("error sending request for url (https://refused.example.com/api/health) | operation timed out", true, "refused.example.com", Failure::Timeout),
            ("error sending request for url (https://trust.example.com/api/health) | Connection refused (os error 111)", false, "trust.example.com", Failure::Refused),
            // the same chains without reqwest's timeout flag: the URL and the host are ignored
            ("error sending request for url (https://trust.localhost:9474/api/health) | operation timed out", false, "trust.localhost", Failure::Timeout),
            ("error sending request for url (https://ssl-kwant.example.com/api/health) | something else", false, "ssl-kwant.example.com", Failure::Timeout),
            ("error sending request for url (https://refused.example.com/api/health) | operation timed out", false, "refused.example.com", Failure::Timeout),
            ("client error (Connect) | connection to tls-handshake.example.com:443 timed out", false, "tls-handshake.example.com", Failure::Timeout),
            // macOS words the host into its certificate errors; the real cause still counts
            ("client error (Connect) | The certificate for this server is invalid. You might be connecting to a server that is pretending to be “refused.example.com”", false, "refused.example.com", Failure::Tls),
            ("connecting to [::1]:9474: something", false, "[::1]", Failure::Timeout),
        ] {
            assert_eq!(classify(chain, timed_out, host), want, "{chain}");
        }
    }

    /// The real path: a server that takes the connection and never answers, at a host named
    /// like a TLS word, is a timeout (the hung https server of Rigel's matrix).
    #[tokio::test]
    async fn a_hung_server_at_a_trust_host_is_a_timeout() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let hold = tokio::spawn(async move {
            let mut held = Vec::new();
            loop {
                let (socket, _) = listener.accept().await.unwrap();
                held.push(socket);
            }
        });
        let client = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .timeout(Duration::from_millis(400))
            .resolve("trust.localhost", ([127, 0, 0, 1], port).into())
            .build()
            .unwrap();
        let o = origin(&format!("https://trust.localhost:{port}"));
        assert_eq!(check(&client, &o).await, Err(Failure::Timeout));
        hold.abort();
    }

    /// Nothing listening: the OS kind says refused, whatever the host is called.
    #[tokio::test]
    async fn nothing_listening_at_a_tls_host_is_refused() {
        let port = {
            let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
            l.local_addr().unwrap().port()
        };
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(5))
            .resolve("tls-trust.localhost", ([127, 0, 0, 1], port).into())
            .build()
            .unwrap();
        let o = origin(&format!("https://tls-trust.localhost:{port}"));
        assert_eq!(check(&client, &o).await, Err(Failure::Refused));
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

    #[test]
    fn the_sign_out_request_carries_only_the_server_s_cookie() {
        let o = origin("https://kwant.example.com");
        let t = Token("kd1.TESTONLY".into());
        let req = sign_out_request(&client(), &o, &t, SIGN_OUT_TIMEOUT).build().unwrap();
        assert_eq!(req.method(), reqwest::Method::DELETE);
        assert_eq!(req.url().as_str(), "https://kwant.example.com/api/auth/session");
        let cookies: Vec<_> = req.headers().get_all(reqwest::header::COOKIE).iter().collect();
        assert_eq!(cookies, vec!["kwnt_token=kd1.TESTONLY"]);
        assert_eq!(req.timeout(), Some(&Duration::from_secs(5)));
        assert!(req.body().is_none());
    }

    /// A one-shot server on 127.0.0.1 that answers `status` (or never answers) and hands back
    /// the request it read.
    async fn one_answer(status: Option<&'static str>) -> (Origin, tokio::task::JoinHandle<String>) {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let seen = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut buf = vec![0u8; 4096];
            let mut got = Vec::new();
            while !got.windows(4).any(|w| w == b"\r\n\r\n") {
                let n = socket.read(&mut buf).await.unwrap();
                if n == 0 {
                    break;
                }
                got.extend_from_slice(&buf[..n]);
            }
            match status {
                Some(status) => {
                    let reply = format!("HTTP/1.1 {status}\r\ncontent-length: 0\r\nconnection: close\r\n\r\n");
                    socket.write_all(reply.as_bytes()).await.unwrap();
                }
                None => tokio::time::sleep(Duration::from_secs(3)).await,
            }
            String::from_utf8_lossy(&got).into_owned()
        });
        (origin(&format!("http://127.0.0.1:{port}")), seen)
    }

    #[tokio::test]
    async fn a_2xx_answer_signs_out() {
        for status in ["204 No Content", "200 OK"] {
            let (o, seen) = one_answer(Some(status)).await;
            let t = Token("kd1.TESTONLY".into());
            assert!(sign_out(&client(), &o, Some(&t)).await, "{status}");
            let request = seen.await.unwrap().to_ascii_lowercase();
            assert!(request.starts_with("delete /api/auth/session http/1.1\r\n"), "{request}");
            assert!(request.contains("\r\ncookie: kwnt_token=kd1.testonly\r\n"), "{request}");
        }
    }

    #[tokio::test]
    async fn another_answer_a_timeout_or_no_token_is_not_signed_out() {
        let t = Token("kd1.TESTONLY".into());
        for status in ["401 Unauthorized", "302 Found", "404 Not Found", "500 Internal Server Error"] {
            let (o, seen) = one_answer(Some(status)).await;
            assert!(!sign_out(&client(), &o, Some(&t)).await, "{status}");
            seen.await.unwrap();
        }
        let (o, seen) = one_answer(None).await;
        let started = std::time::Instant::now();
        assert!(!sign_out_within(&client(), &o, Some(&t), Duration::from_millis(300)).await);
        assert!(started.elapsed() < Duration::from_secs(2), "the time limit holds");
        seen.abort();
        // no cookie: nothing is sent
        let (o, seen) = one_answer(Some("204 No Content")).await;
        assert!(!sign_out(&client(), &o, None).await);
        seen.abort();
        // nothing listening
        let port = std::net::TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port();
        assert!(!sign_out(&client(), &origin(&format!("http://127.0.0.1:{port}")), Some(&t)).await);
    }

}
