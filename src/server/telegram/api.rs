//! The Telegram Bot API, the few calls kwant makes. The token is in the URL path
//! (`/bot<token>/<method>`), so this is the only place that builds such a URL, and no error that
//! leaves this file carries a URL, a request or the token (test plan G-L1, G-L12).

use serde_json::{json, Value};
use std::time::Duration;
use zeroize::Zeroizing;

/// Where the Bot API is. Release builds always use it (decision Q-G2).
pub const TELEGRAM_API: &str = "https://api.telegram.org";
/// Debug and test builds only: the mock's base URL, e.g. `http://127.0.0.1:9475`.
pub const API_BASE_ENV: &str = "KWNT_TELEGRAM_API_BASE";
/// How long one getUpdates waits for an update (seconds).
pub const POLL_SECS: u64 = 25;
/// The calls that post a new message. Sent twice, the person sees the message (or an ask card
/// with live buttons) twice, so a lost answer to one of these is `MaybeSent`, never `Network`.
const POSTS_A_MESSAGE: &[&str] = &["sendMessage"];

/// A Telegram call that didn't work. Its text never has the URL or the token.
#[derive(Debug, Clone, PartialEq)]
pub enum TgError {
    /// 401: Telegram doesn't know this token (wrong, or revoked in BotFather).
    Unauthorized,
    /// 409: another getUpdates (or a webhook) holds this bot.
    Conflict,
    /// 429, with Telegram's `retry_after` seconds.
    TooMany { retry_after: u64 },
    /// 403: the person blocked the bot, or the chat can't be written to.
    Forbidden,
    /// 400 with Telegram's description ("message is too long", "chat not found", …).
    BadRequest(String),
    /// No answer: offline, refused, timed out, or not JSON. For a call that posts a message
    /// (`sendMessage`), only when the request never left (refused, DNS, TLS, connect timeout).
    Network,
    /// 5xx, or `ok:false` with another code. Never below 500 for a call that posts a message.
    Server(u16),
    /// A call that posts a message (`sendMessage`) lost its answer after the request may have
    /// reached Telegram: a timeout, a reset, an unreadable answer or a 5xx. The message may be in
    /// the chat already, so it is never sent again (telegram-send-once).
    MaybeSent,
}

impl std::fmt::Display for TgError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TgError::Unauthorized => write!(f, "Telegram didn't accept the token"),
            TgError::Conflict => write!(f, "another program is reading this bot's messages"),
            TgError::TooMany { retry_after } => write!(f, "Telegram asked to slow down for {retry_after} s"),
            TgError::Forbidden => write!(f, "Telegram won't deliver to this chat"),
            TgError::BadRequest(d) => write!(f, "Telegram refused the message: {d}"),
            TgError::Network => write!(f, "couldn't reach Telegram"),
            TgError::Server(c) => write!(f, "Telegram answered with an error ({c})"),
            TgError::MaybeSent => write!(f, "Telegram's answer was lost; the message may have been posted"),
        }
    }
}

/// The API base: the override only in a build with the test seams (`cargo test`), and only an
/// http(s) URL. A release build, a `make run` debug build and Docker always talk to api.telegram.org,
/// so no setting can send the bot token anywhere else (Q-G2).
pub fn api_base() -> String {
    base_for(crate::clock::TEST_SEAMS, std::env::var(API_BASE_ENV).ok())
}

/// The override counts only with the test seams built in (`seams`), and only as an http(s) address.
fn base_for(seams: bool, override_base: Option<String>) -> String {
    if seams {
        if let Some(b) = override_base {
            let b = b.trim().trim_end_matches('/');
            if b.starts_with("http://") || b.starts_with("https://") {
                return b.to_string();
            }
        }
    }
    TELEGRAM_API.to_string()
}

/// Whether `token` looks like a BotFather token: digits, a colon, then 30–50 of `A–Z a–z 0–9 _ -`.
pub fn token_shape_ok(token: &str) -> bool {
    let Some((id, secret)) = token.split_once(':') else { return false };
    !id.is_empty()
        && id.len() <= 20
        && id.bytes().all(|b| b.is_ascii_digit())
        && (30..=50).contains(&secret.len())
        && secret.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
}

/// The token from what the person pasted: trimmed, and a pasted `…/bot<token>/…` URL or a
/// `bot<token>` prefix taken apart (test plan G-S4). The URL itself is never kept.
pub fn clean_token(pasted: &str) -> Option<Zeroizing<String>> {
    let t = pasted.trim();
    let t = match t.find("/bot") {
        Some(i) if t.starts_with("http") => t[i + 4..].split('/').next().unwrap_or(""),
        _ => t,
    };
    let t = t.strip_prefix("bot").filter(|rest| token_shape_ok(rest)).unwrap_or(t);
    token_shape_ok(t).then(|| Zeroizing::new(t.to_string()))
}

/// The bot behind a token (`getMe`).
#[derive(Debug, Clone, PartialEq)]
pub struct BotInfo {
    pub id: i64,
    pub username: String,
    pub name: String,
}

#[derive(Clone)]
pub struct Client {
    http: reqwest::Client,
    base: String,
    token: std::sync::Arc<Zeroizing<String>>,
}

impl std::fmt::Debug for Client {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("telegram::Client { token: [redacted] }")
    }
}

impl Client {
    /// A client that never follows a redirect. If that client can't be built there is none: never
    /// a default one, which would follow redirects (Lin's L2).
    pub fn new(token: Zeroizing<String>) -> Result<Self, TgError> {
        Self::with_base(token, api_base())
    }

    /// A client for a local stub (unit tests only; `api_base` stays the one way in for the rest).
    #[cfg(test)]
    pub(crate) fn for_stub(token: Zeroizing<String>, base: &str) -> Self {
        Self::with_base(token, base.to_string()).expect("a test client")
    }

    fn with_base(token: Zeroizing<String>, base: String) -> Result<Self, TgError> {
        let http = reqwest::Client::builder()
            // A redirect could carry the token's URL to another host
            .redirect(reqwest::redirect::Policy::none())
            .connect_timeout(Duration::from_secs(10))
            .timeout(Duration::from_secs(POLL_SECS + 10))
            .build()
            .map_err(|_| TgError::Network)?;
        Ok(Client { http, base, token: std::sync::Arc::new(token) })
    }

    /// Strips the token from a text Telegram sent back (it never should, but it costs nothing),
    /// then cuts it to 200 characters. In that order, so a token across the cut can't leave half.
    fn scrub(&self, text: &str) -> String {
        let t = if self.token.is_empty() { text.to_string() } else { text.replace(self.token.as_str(), "[redacted]") };
        t.chars().take(200).collect()
    }

    /// One call. `wait` bounds this call alone (getUpdates waits longer than a send).
    pub async fn call(&self, method: &str, body: &Value, wait: Duration) -> Result<Value, TgError> {
        // The only place the token meets a URL
        let url = Zeroizing::new(format!("{}/bot{}/{method}", self.base, self.token.as_str()));
        let posts = POSTS_A_MESSAGE.contains(&method);
        let resp = match self.http.post(url.as_str()).timeout(wait).json(body).send().await {
            Ok(r) => r,
            // Never connected (refused, DNS, TLS, connect timeout) or never built: nothing left
            Err(e) if e.is_connect() || e.is_builder() => return Err(TgError::Network),
            // A timeout or a reset after the request may have been written
            Err(_) if posts => return Err(TgError::MaybeSent),
            Err(_) => return Err(TgError::Network),
        };
        let status = resp.status().as_u16();
        let v: Value = match resp.json().await {
            Ok(v) => v,
            // Something answered, so the request arrived; it may have been posted
            Err(_) if posts => return Err(TgError::MaybeSent),
            Err(_) => return Err(if status >= 500 { TgError::Server(status) } else { TgError::Network }),
        };
        if v["ok"] == true {
            return Ok(v["result"].clone());
        }
        let code = v["error_code"].as_u64().map(|c| c as u16).unwrap_or(status);
        let desc = self.scrub(v["description"].as_str().unwrap_or(""));
        Err(match code {
            401 => TgError::Unauthorized,
            // A token Telegram doesn't know at all answers 404 Not Found
            404 if desc.contains("Not Found") => TgError::Unauthorized,
            409 => TgError::Conflict,
            429 => TgError::TooMany { retry_after: v["parameters"]["retry_after"].as_u64().unwrap_or(5).clamp(1, 3600) },
            403 => TgError::Forbidden,
            400 => TgError::BadRequest(desc),
            // Telegram (or a proxy in front of it) failed after it had the request
            c if posts && c >= 500 => TgError::MaybeSent,
            c => TgError::Server(c),
        })
    }

    pub async fn get_me(&self) -> Result<BotInfo, TgError> {
        let r = self.call("getMe", &json!({}), Duration::from_secs(15)).await?;
        let username = r["username"].as_str().unwrap_or("").to_string();
        if r["is_bot"] != true || username.is_empty() {
            return Err(TgError::BadRequest("not a bot".into()));
        }
        Ok(BotInfo { id: r["id"].as_i64().unwrap_or(0), name: r["first_name"].as_str().unwrap_or(&username).to_string(), username })
    }

    pub async fn get_updates(&self, offset: i64, timeout_secs: u64) -> Result<Vec<Value>, TgError> {
        let body = json!({
            "offset": offset,
            "timeout": timeout_secs,
            "limit": 50,
            "allowed_updates": ["message", "callback_query"],
        });
        let r = self.call("getUpdates", &body, Duration::from_secs(timeout_secs + 10)).await?;
        Ok(r.as_array().cloned().unwrap_or_default())
    }

    /// Sends HTML text; returns the new message's id.
    pub async fn send_message(&self, chat_id: i64, html: &str, markup: Option<&Value>, reply_to: Option<i64>) -> Result<i64, TgError> {
        let mut body = json!({
            "chat_id": chat_id,
            "text": html,
            "parse_mode": "HTML",
            "link_preview_options": {"is_disabled": true},
        });
        if let Some(m) = markup {
            body["reply_markup"] = m.clone();
        }
        if let Some(r) = reply_to {
            body["reply_parameters"] = json!({"message_id": r, "allow_sending_without_reply": true});
        }
        let r = self.call("sendMessage", &body, Duration::from_secs(20)).await?;
        Ok(r["message_id"].as_i64().unwrap_or(0))
    }

    /// Sends text with no formatting (the fallback when Telegram can't parse the HTML).
    pub async fn send_plain(&self, chat_id: i64, text: &str, markup: Option<&Value>) -> Result<i64, TgError> {
        let mut body = json!({"chat_id": chat_id, "text": text, "link_preview_options": {"is_disabled": true}});
        if let Some(m) = markup {
            body["reply_markup"] = m.clone();
        }
        let r = self.call("sendMessage", &body, Duration::from_secs(20)).await?;
        Ok(r["message_id"].as_i64().unwrap_or(0))
    }

    /// Replaces a message's text and buttons (`markup` None removes the buttons).
    pub async fn edit_message(&self, chat_id: i64, message_id: i64, html: &str, markup: Option<&Value>) -> Result<(), TgError> {
        let mut body = json!({
            "chat_id": chat_id,
            "message_id": message_id,
            "text": html,
            "parse_mode": "HTML",
            "link_preview_options": {"is_disabled": true},
        });
        body["reply_markup"] = markup.cloned().unwrap_or_else(|| json!({"inline_keyboard": []}));
        match self.call("editMessageText", &body, Duration::from_secs(20)).await {
            Err(TgError::BadRequest(d)) if d.contains("not modified") => Ok(()),
            other => other.map(|_| ()),
        }
    }

    pub async fn answer_callback(&self, id: &str, text: Option<&str>) -> Result<(), TgError> {
        let mut body = json!({"callback_query_id": id});
        if let Some(t) = text {
            body["text"] = json!(t);
        }
        self.call("answerCallbackQuery", &body, Duration::from_secs(10)).await.map(|_| ())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fake() -> String {
        ["12345", "6789:", &"Ab_-".repeat(9)[..35]].concat()
    }

    #[test]
    fn token_shapes() {
        let t = fake();
        assert!(token_shape_ok(&t));
        for bad in ["", "123", ":abc", "12a:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa", "123:short", "123:has space aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"] {
            assert!(!token_shape_ok(bad), "{bad}");
        }
        assert_eq!(clean_token(&format!("  {t}\n")).as_deref().map(String::as_str), Some(t.as_str()));
        assert_eq!(clean_token(&format!("https://api.telegram.org/bot{t}/getMe")).as_deref().map(String::as_str), Some(t.as_str()));
        assert_eq!(clean_token(&format!("bot{t}")).as_deref().map(String::as_str), Some(t.as_str()));
        assert_eq!(clean_token("bot123:ABC"), None);
    }

    #[test]
    fn errors_never_show_the_token_or_a_url() {
        let c = Client::new(Zeroizing::new(fake())).unwrap();
        assert!(!format!("{c:?}").contains(&fake()));
        assert_eq!(c.scrub(&format!("bad {}", fake())), "bad [redacted]");
        // A token across the 200-character cut: none of it is left
        let long = format!("{}{}", "x".repeat(190), fake());
        let out = c.scrub(&long);
        assert!(out.ends_with("[redacted]") && out.chars().count() == 200, "{out}");
        let (id, _) = fake().split_once(':').map(|(a, b)| (a.to_string(), b.to_string())).unwrap();
        assert!(!out.contains(&format!("{id}:")), "{out}");
        for e in [TgError::Unauthorized, TgError::Conflict, TgError::TooMany { retry_after: 3 }, TgError::Forbidden, TgError::BadRequest("x".into()), TgError::Network, TgError::Server(502), TgError::MaybeSent] {
            let s = format!("{e} {e:?}");
            assert!(!s.contains("http") && !s.contains("/bot"), "{s}");
        }
    }

    fn client(base: &str) -> Client {
        Client::for_stub(Zeroizing::new(fake()), base)
    }

    // telegram-send-once: a request that never left is Network for every call, so it may go again
    #[tokio::test]
    async fn a_request_that_never_connected_is_network() {
        let base = stub::refused_base();
        let c = client(&base);
        assert_eq!(c.send_message(1, "hi", None, None).await, Err(TgError::Network));
        assert_eq!(c.send_plain(1, "hi", None).await, Err(TgError::Network));
        assert_eq!(c.edit_message(1, 2, "hi", None).await, Err(TgError::Network));
    }

    // telegram-send-once: once the request may have arrived, a lost answer to sendMessage is
    // MaybeSent; every other call keeps today's Network/Server
    #[tokio::test]
    async fn a_lost_answer_to_a_send_is_maybe_sent_and_other_calls_keep_todays_errors() {
        use stub::Reply::*;
        let rows = [
            ("reset after the request", Close, TgError::Network),
            ("proxy 502 page", Answer(502, "<html>Bad Gateway</html>"), TgError::Server(502)),
            ("Telegram's own 500", Answer(500, r#"{"ok":false,"error_code":500,"description":"Internal Server Error"}"#), TgError::Server(500)),
            ("200 that isn't JSON", Answer(200, "<html>ok</html>"), TgError::Network),
        ];
        for (why, reply, edit_err) in rows {
            let s = stub::start(vec![reply]).await;
            let c = client(&s.base);
            assert_eq!(c.send_message(1, "hi", None, None).await, Err(TgError::MaybeSent), "{why}");
            assert_eq!(c.send_plain(1, "hi", None).await, Err(TgError::MaybeSent), "{why}");
            assert_eq!(c.edit_message(1, 2, "hi", None).await, Err(edit_err.clone()), "{why}");
            assert_eq!(c.get_me().await, Err(edit_err), "{why}");
            assert_eq!(s.hits("sendMessage"), 2, "{why}");
        }
        // A timeout after the request was written
        let s = stub::start(vec![Hang]).await;
        let c = client(&s.base);
        let short = Duration::from_millis(300);
        assert_eq!(c.call("sendMessage", &json!({"chat_id": 1, "text": "hi"}), short).await, Err(TgError::MaybeSent));
        assert_eq!(c.call("editMessageText", &json!({"chat_id": 1}), short).await, Err(TgError::Network));
        assert_eq!(s.hits("sendMessage"), 1);
    }

    // Telegram's clear refusals stay what they were for a send: 429 waits, 400 falls back, 4xx
    #[tokio::test]
    async fn refusals_to_a_send_are_unchanged() {
        use stub::Reply::*;
        let rows = [
            (Answer(429, r#"{"ok":false,"error_code":429,"description":"Too Many Requests","parameters":{"retry_after":3}}"#), TgError::TooMany { retry_after: 3 }),
            (Answer(400, r#"{"ok":false,"error_code":400,"description":"Bad Request: can't parse entities"}"#), TgError::BadRequest("Bad Request: can't parse entities".into())),
            (Answer(403, r#"{"ok":false,"error_code":403,"description":"Forbidden: bot was blocked by the user"}"#), TgError::Forbidden),
            (Answer(401, r#"{"ok":false,"error_code":401,"description":"Unauthorized"}"#), TgError::Unauthorized),
        ];
        for (reply, want) in rows {
            let s = stub::start(vec![reply]).await;
            assert_eq!(client(&s.base).send_message(1, "hi", None, None).await, Err(want));
        }
        let s = stub::start(vec![Answer(200, r#"{"ok":true,"result":{"message_id":42}}"#)]).await;
        assert_eq!(client(&s.base).send_message(1, "hi", None, None).await, Ok(42));
    }

    #[test]
    fn builds_without_the_test_seams_ignore_the_override() {
        let mock = Some("http://127.0.0.1:9/".to_string());
        assert_eq!(base_for(false, mock.clone()), "https://api.telegram.org");
        assert_eq!(base_for(true, mock), "http://127.0.0.1:9");
        assert_eq!(base_for(true, Some("file:///etc".into())), "https://api.telegram.org");
        assert_eq!(base_for(true, None), "https://api.telegram.org");
    }
}

/// A local Bot API for unit tests: counts the calls per method and answers each connection from
/// a script. No real Telegram, no network beyond 127.0.0.1.
#[cfg(test)]
pub(crate) mod stub {
    use std::collections::HashMap;
    use std::sync::{Arc, Mutex};
    use std::time::Duration;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::{TcpListener, TcpStream};

    #[derive(Clone, Debug)]
    pub enum Reply {
        /// An HTTP status and a body.
        Answer(u16, &'static str),
        /// Reads the whole request, then closes without answering (a reset after it was written).
        Close,
        /// Reads the whole request and never answers (the caller times out).
        Hang,
    }

    pub struct Stub {
        pub base: String,
        hits: Arc<Mutex<HashMap<String, usize>>>,
    }

    impl Stub {
        /// How many requests for `method` arrived in full.
        pub fn hits(&self, method: &str) -> usize {
            self.hits.lock().unwrap().get(method).copied().unwrap_or(0)
        }
    }

    /// A base URL nothing listens on, so every request is refused before it leaves.
    pub fn refused_base() -> String {
        let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = l.local_addr().unwrap();
        drop(l);
        format!("http://{addr}")
    }

    /// Answers the n-th connection with `script[n]`, and every later one with the last line.
    /// Each answer says `connection: close`, so one connection is one request.
    pub async fn start(script: Vec<Reply>) -> Stub {
        assert!(!script.is_empty());
        let l = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", l.local_addr().unwrap());
        let hits = Arc::new(Mutex::new(HashMap::new()));
        let counts = hits.clone();
        tokio::spawn(async move {
            let mut n = 0;
            while let Ok((mut sock, _)) = l.accept().await {
                let reply = script[n.min(script.len() - 1)].clone();
                n += 1;
                let counts = counts.clone();
                tokio::spawn(async move {
                    let Some(method) = read_request(&mut sock).await else { return };
                    *counts.lock().unwrap().entry(method).or_insert(0) += 1;
                    match reply {
                        Reply::Answer(status, body) => {
                            let head = format!("HTTP/1.1 {status} Stub\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n", body.len());
                            let _ = sock.write_all(head.as_bytes()).await;
                            let _ = sock.write_all(body.as_bytes()).await;
                            let _ = sock.shutdown().await;
                        }
                        Reply::Close => drop(sock),
                        Reply::Hang => {
                            tokio::time::sleep(Duration::from_secs(60)).await;
                            drop(sock);
                        }
                    }
                });
            }
        });
        Stub { base, hits }
    }

    /// Reads one request (head and `content-length` body); its method is the path's last part.
    async fn read_request(sock: &mut TcpStream) -> Option<String> {
        let mut buf = Vec::new();
        let mut chunk = [0u8; 4096];
        let head_end = loop {
            let n = sock.read(&mut chunk).await.ok()?;
            if n == 0 {
                return None;
            }
            buf.extend_from_slice(&chunk[..n]);
            if let Some(i) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
                break i + 4;
            }
        };
        let head = String::from_utf8_lossy(&buf[..head_end]).to_string();
        let len = head
            .lines()
            .find_map(|l| l.to_ascii_lowercase().strip_prefix("content-length:").map(|v| v.trim().parse::<usize>().unwrap_or(0)))
            .unwrap_or(0);
        while buf.len() < head_end + len {
            let n = sock.read(&mut chunk).await.ok()?;
            if n == 0 {
                return None;
            }
            buf.extend_from_slice(&chunk[..n]);
        }
        let path = head.split_whitespace().nth(1)?;
        Some(path.rsplit('/').next()?.to_string())
    }
}
