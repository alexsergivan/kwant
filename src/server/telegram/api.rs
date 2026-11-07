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
    /// No answer: offline, refused, timed out, or not JSON.
    Network,
    /// 5xx, or `ok:false` with another code.
    Server(u16),
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
        }
    }
}

/// The API base: the override only in debug and test builds, and only an http(s) URL.
pub fn api_base() -> String {
    base_for(cfg!(debug_assertions), std::env::var(API_BASE_ENV).ok())
}

/// The override counts only in a debug or test build (`debug`), and only as an http(s) address.
fn base_for(debug: bool, override_base: Option<String>) -> String {
    if debug {
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
    pub fn new(token: Zeroizing<String>) -> Self {
        let http = reqwest::Client::builder()
            // A redirect could carry the token's URL to another host
            .redirect(reqwest::redirect::Policy::none())
            .connect_timeout(Duration::from_secs(10))
            .timeout(Duration::from_secs(POLL_SECS + 10))
            .build()
            .unwrap_or_default();
        Client { http, base: api_base(), token: std::sync::Arc::new(token) }
    }

    /// Strips the token from a text Telegram sent back (it never should, but it costs nothing).
    fn scrub(&self, text: &str) -> String {
        let t: String = text.chars().take(200).collect();
        if self.token.is_empty() {
            t
        } else {
            t.replace(self.token.as_str(), "[redacted]")
        }
    }

    /// One call. `wait` bounds this call alone (getUpdates waits longer than a send).
    pub async fn call(&self, method: &str, body: &Value, wait: Duration) -> Result<Value, TgError> {
        // The only place the token meets a URL
        let url = Zeroizing::new(format!("{}/bot{}/{method}", self.base, self.token.as_str()));
        let resp = self.http.post(url.as_str()).timeout(wait).json(body).send().await.map_err(|_| TgError::Network)?;
        let status = resp.status().as_u16();
        let v: Value = resp.json().await.map_err(|_| if status >= 500 { TgError::Server(status) } else { TgError::Network })?;
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
        let c = Client::new(Zeroizing::new(fake()));
        assert!(!format!("{c:?}").contains(&fake()));
        assert_eq!(c.scrub(&format!("bad {}", fake())), "bad [redacted]");
        for e in [TgError::Unauthorized, TgError::Conflict, TgError::TooMany { retry_after: 3 }, TgError::Forbidden, TgError::BadRequest("x".into()), TgError::Network, TgError::Server(502)] {
            let s = format!("{e} {e:?}");
            assert!(!s.contains("http") && !s.contains("/bot"), "{s}");
        }
    }

    #[test]
    fn release_builds_ignore_the_override() {
        let mock = Some("http://127.0.0.1:9/".to_string());
        assert_eq!(base_for(false, mock.clone()), "https://api.telegram.org");
        assert_eq!(base_for(true, mock), "http://127.0.0.1:9");
        assert_eq!(base_for(true, Some("file:///etc".into())), "https://api.telegram.org");
        assert_eq!(base_for(true, None), "https://api.telegram.org");
    }
}
