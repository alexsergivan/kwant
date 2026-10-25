//! Signing a browser in without pasting the API token. A one-time link (printed at startup, by
//! `kwnt signin-link`, or made on a signed-in device) is exchanged once for a session: a random
//! secret in the `kwnt_token` cookie. The server keeps only its SHA-256 hash, in
//! `<data_dir>/sessions.json` (0600), so a signed-in browser never holds the API token and can be
//! listed and signed out on its own. The API token itself still works everywhere, as the fallback.
//!
//! A new browser can also ask to be let in (`POST /api/auth/device`): it keeps a secret
//! `device_code` and shows a short `user_code`; the person approves that code from a device that
//! is already signed in, and the new browser's next poll gets its session.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::net::IpAddr;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// How long a one-time link works.
pub const LINK_TTL: Duration = Duration::from_secs(15 * 60);
/// How long a session lasts (the cookie's Max-Age).
pub const SESSION_SECS: u64 = 31_536_000;
/// Live links kept at once; making another drops the oldest.
const MAX_LINKS: usize = 20;
/// Used or expired links are remembered this long, to say "already used" instead of "wrong".
const SPENT_MEMORY: Duration = Duration::from_secs(60 * 60);
/// `last_seen` is written to disk at most this often per session.
const LAST_SEEN_PERSIST: chrono::TimeDelta = chrono::TimeDelta::hours(1);
pub const LINK_PREFIX: &str = "kl1.";
const SESSION_PREFIX: &str = "ks1.";
const SESSIONS_FILE: &str = "sessions.json";
/// How long a device request waits for approval.
pub const DEVICE_TTL: Duration = Duration::from_secs(10 * 60);
/// How often the waiting browser should poll.
pub const DEVICE_INTERVAL_SECS: u64 = 2;
/// Device requests waiting at once, from everyone.
const MAX_PENDING: usize = 10;
/// New device requests per minute from one address, and from everyone.
const PER_IP_PER_MIN: usize = 5;
const ALL_PER_MIN: usize = 30;
const DEVICE_PREFIX: &str = "kd1.";
/// User codes: no vowels (no words), no 0/O/1/I; 28^8, about 38 bits.
const USER_CODE_ALPHABET: &[u8] = b"BCDFGHJKLMNPQRSTVWXZ23456789";

/// How a session was made.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Via {
    /// The API token, pasted or from `#kwnt_token=<token>`.
    Token,
    /// A one-time link.
    Link,
    /// Approved from a signed-in device.
    Device,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Session {
    pub id: String,
    /// SHA-256 of the cookie secret, hex.
    hash: String,
    pub browser: String,
    pub ip: String,
    pub via: Via,
    pub created_at: DateTime<Utc>,
    pub expires_at: DateTime<Utc>,
    pub last_seen: DateTime<Utc>,
    #[serde(skip)]
    persisted_seen: Option<DateTime<Utc>>,
}

impl Session {
    pub fn is_hash(&self, hash: &str) -> bool {
        self.hash == hash
    }
}

/// Who is signing in, for the sessions list and the approval card.
#[derive(Debug, Clone, Default)]
pub struct Client {
    pub browser: String,
    pub ip: String,
}

/// What redeeming a one-time link found.
#[derive(Debug, PartialEq, Eq)]
pub enum Redeem {
    /// The link was live; it is spent now. Holds the new session's cookie secret.
    Ok(String),
    /// The link was already used or ran out.
    Spent,
    /// Not a link this server made (or forgotten since).
    Unknown,
}

enum LinkState {
    Live(Instant),
    Spent(Instant),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DeviceState {
    Pending,
    Approved,
    Denied,
}

struct DeviceRequest {
    device_hash: String,
    /// Eight characters, no dash.
    user_code: String,
    client: Client,
    created_at: DateTime<Utc>,
    expires: Instant,
    state: DeviceState,
}

/// A device request the person can approve.
#[derive(Debug, Clone, Serialize)]
pub struct PendingDevice {
    pub user_code: String,
    pub browser: String,
    pub ip: String,
    pub created_at: DateTime<Utc>,
    pub expires_at: DateTime<Utc>,
}

/// What a waiting browser's poll finds.
#[derive(Debug, PartialEq, Eq)]
pub enum Poll {
    Pending,
    /// Approved: the new session's cookie secret. The request is gone now.
    Approved(String),
    Denied,
    /// Ran out, already collected, or never made.
    Expired,
}

#[derive(Default)]
struct Starts {
    by_ip: HashMap<String, Vec<Instant>>,
    all: Vec<Instant>,
}

pub struct SignIn {
    path: Option<PathBuf>,
    sessions: Mutex<Vec<Session>>,
    links: Mutex<HashMap<String, LinkState>>,
    devices: Mutex<Vec<DeviceRequest>>,
    starts: Mutex<Starts>,
}

impl std::fmt::Debug for SignIn {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SignIn").finish_non_exhaustive()
    }
}

pub fn hash(secret: &str) -> String {
    hex(&Sha256::digest(secret.as_bytes()))
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// 244 random bits (two v4 UUIDs, from the OS RNG), hex.
pub fn random_secret() -> String {
    format!("{}{}", uuid::Uuid::new_v4().simple(), uuid::Uuid::new_v4().simple())
}

impl SignIn {
    /// Sessions from `<data_dir>/sessions.json`; an unreadable file signs every browser out
    /// (the API token still works) instead of stopping the server.
    pub fn load(data_dir: &Path) -> Self {
        let path = data_dir.join(SESSIONS_FILE);
        let sessions = match std::fs::read_to_string(&path) {
            Ok(s) => serde_json::from_str::<Vec<Session>>(&s).unwrap_or_else(|e| {
                tracing::warn!(error = %e, "sessions.json is unreadable; every browser has to sign in again");
                Vec::new()
            }),
            Err(_) => Vec::new(),
        };
        let now = Utc::now();
        let sessions = sessions
            .into_iter()
            .filter(|s| s.expires_at > now)
            .map(|s| Session { persisted_seen: Some(s.last_seen), ..s })
            .collect();
        Self {
            path: Some(path),
            sessions: Mutex::new(sessions),
            links: Mutex::new(HashMap::new()),
            devices: Mutex::new(Vec::new()),
            starts: Mutex::new(Starts::default()),
        }
    }

    /// Nothing on disk (tests).
    pub fn in_memory() -> Self {
        Self {
            path: None,
            sessions: Mutex::new(Vec::new()),
            links: Mutex::new(HashMap::new()),
            devices: Mutex::new(Vec::new()),
            starts: Mutex::new(Starts::default()),
        }
    }

    fn save(&self, sessions: &mut [Session]) {
        let Some(path) = &self.path else { return };
        let now = Utc::now();
        for s in sessions.iter_mut() {
            s.persisted_seen = Some(s.last_seen);
        }
        let live: Vec<&Session> = sessions.iter().filter(|s| s.expires_at > now).collect();
        let Ok(json) = serde_json::to_string_pretty(&live) else { return };
        let tmp = path.with_extension("json.tmp");
        let res = super::auth::write_private(&tmp, &json).and_then(|_| std::fs::rename(&tmp, path));
        if let Err(e) = res {
            tracing::warn!(error = %e, "could not save sessions; sign-ins since the last save end on restart");
        }
    }

    /// A new session; returns its cookie secret.
    pub fn create_session(&self, via: Via, client: &Client) -> String {
        let secret = format!("{SESSION_PREFIX}{}", random_secret());
        let now = Utc::now();
        let session = Session {
            id: uuid::Uuid::new_v4().simple().to_string()[..12].to_string(),
            hash: hash(&secret),
            browser: client.browser.clone(),
            ip: client.ip.clone(),
            via,
            created_at: now,
            expires_at: now + chrono::TimeDelta::seconds(SESSION_SECS as i64),
            last_seen: now,
            persisted_seen: None,
        };
        let mut sessions = self.sessions.lock().unwrap_or_else(|e| e.into_inner());
        sessions.retain(|s| s.expires_at > now);
        sessions.push(session);
        self.save(&mut sessions);
        secret
    }

    /// Whether `secret` is a live session's; notes when it was last seen.
    pub fn session_ok(&self, secret: &str) -> bool {
        self.session_id(secret).is_some()
    }

    /// The id of the live session `secret` belongs to.
    pub fn session_id(&self, secret: &str) -> Option<String> {
        if !secret.starts_with(SESSION_PREFIX) {
            return None;
        }
        let h = hash(secret);
        let now = Utc::now();
        let mut sessions = self.sessions.lock().unwrap_or_else(|e| e.into_inner());
        let s = sessions.iter_mut().find(|s| s.hash == h && s.expires_at > now)?;
        s.last_seen = now;
        let id = s.id.clone();
        if s.persisted_seen.is_none_or(|p| now - p > LAST_SEEN_PERSIST) {
            self.save(&mut sessions);
        }
        Some(id)
    }

    /// Live sessions, newest first.
    pub fn sessions(&self) -> Vec<Session> {
        let now = Utc::now();
        let sessions = self.sessions.lock().unwrap_or_else(|e| e.into_inner());
        let mut list: Vec<Session> = sessions.iter().filter(|s| s.expires_at > now).cloned().collect();
        list.sort_by_key(|s| std::cmp::Reverse(s.created_at));
        list
    }

    /// Ends one session; false when there was none with that id.
    pub fn revoke(&self, id: &str) -> bool {
        let mut sessions = self.sessions.lock().unwrap_or_else(|e| e.into_inner());
        let before = sessions.len();
        sessions.retain(|s| s.id != id);
        let removed = sessions.len() != before;
        if removed {
            self.save(&mut sessions);
        }
        removed
    }

    /// Ends the session behind a cookie secret (signing out).
    pub fn revoke_secret(&self, secret: &str) {
        if let Some(id) = self.session_id(secret) {
            self.revoke(&id);
        }
    }

    /// A new one-time link code, live for `ttl`.
    pub fn create_link(&self, ttl: Duration) -> String {
        let code = format!("{LINK_PREFIX}{}", random_secret());
        let now = Instant::now();
        let mut links = self.links.lock().unwrap_or_else(|e| e.into_inner());
        sweep(&mut links, now);
        let mut live: Vec<(String, Instant)> = links
            .iter()
            .filter_map(|(h, s)| match s {
                LinkState::Live(exp) => Some((h.clone(), *exp)),
                LinkState::Spent(_) => None,
            })
            .collect();
        if live.len() >= MAX_LINKS {
            live.sort_by_key(|(_, exp)| *exp);
            for (h, _) in live.iter().take(live.len() + 1 - MAX_LINKS) {
                links.insert(h.clone(), LinkState::Spent(now + SPENT_MEMORY));
            }
        }
        links.insert(hash(&code), LinkState::Live(now + ttl));
        code
    }

    /// Spends a one-time link and makes a session for it.
    pub fn redeem_link(&self, code: &str, client: &Client) -> Redeem {
        if !code.starts_with(LINK_PREFIX) {
            return Redeem::Unknown;
        }
        let h = hash(code);
        let now = Instant::now();
        {
            let mut links = self.links.lock().unwrap_or_else(|e| e.into_inner());
            sweep(&mut links, now);
            match links.get(&h) {
                None => return Redeem::Unknown,
                Some(LinkState::Spent(_)) => return Redeem::Spent,
                Some(LinkState::Live(_)) => {
                    links.insert(h, LinkState::Spent(now + SPENT_MEMORY));
                }
            }
        }
        Redeem::Ok(self.create_session(Via::Link, client))
    }
}

impl SignIn {
    /// A new browser asks to be let in: `(device_code, user_code)`, or the seconds to wait when
    /// this address (or everyone) asked too often, or too many requests are already waiting.
    pub fn start_device(&self, client: &Client) -> Result<(String, String), u64> {
        self.start_device_for(client, DEVICE_TTL)
    }

    /// [`Self::start_device`] with another wait (tests).
    pub fn start_device_for(&self, client: &Client, ttl: Duration) -> Result<(String, String), u64> {
        let now = Instant::now();
        {
            let mut starts = self.starts.lock().unwrap_or_else(|e| e.into_inner());
            let minute = Duration::from_secs(60);
            starts.all.retain(|t| now.duration_since(*t) < minute);
            starts.by_ip.retain(|_, v| {
                v.retain(|t| now.duration_since(*t) < minute);
                !v.is_empty()
            });
            let mine = starts.by_ip.get(&client.ip).map_or(0, Vec::len);
            if mine >= PER_IP_PER_MIN || starts.all.len() >= ALL_PER_MIN {
                let oldest = starts.by_ip.get(&client.ip).and_then(|v| v.first()).copied().unwrap_or(now);
                return Err(60u64.saturating_sub(now.duration_since(oldest).as_secs()).max(1));
            }
            starts.all.push(now);
            starts.by_ip.entry(client.ip.clone()).or_default().push(now);
        }
        let mut devices = self.devices.lock().unwrap_or_else(|e| e.into_inner());
        devices.retain(|d| d.expires > now);
        if devices.iter().filter(|d| d.state == DeviceState::Pending).count() >= MAX_PENDING {
            return Err(30);
        }
        let device_code = format!("{DEVICE_PREFIX}{}", random_secret());
        let user_code = loop {
            let c = random_user_code();
            if !devices.iter().any(|d| d.user_code == c) {
                break c;
            }
        };
        devices.push(DeviceRequest {
            device_hash: hash(&device_code),
            user_code: user_code.clone(),
            client: client.clone(),
            created_at: Utc::now(),
            expires: now + ttl,
            state: DeviceState::Pending,
        });
        Ok((device_code, format_user_code(&user_code)))
    }

    /// The waiting browser's poll. An approved or denied answer is given once.
    pub fn poll_device(&self, device_code: &str) -> Poll {
        if !device_code.starts_with(DEVICE_PREFIX) {
            return Poll::Expired;
        }
        let h = hash(device_code);
        let now = Instant::now();
        let mut devices = self.devices.lock().unwrap_or_else(|e| e.into_inner());
        devices.retain(|d| d.expires > now);
        let Some(i) = devices.iter().position(|d| d.device_hash == h) else {
            return Poll::Expired;
        };
        match devices[i].state {
            DeviceState::Pending => Poll::Pending,
            DeviceState::Denied => {
                devices.remove(i);
                Poll::Denied
            }
            DeviceState::Approved => {
                let d = devices.remove(i);
                drop(devices);
                Poll::Approved(self.create_session(Via::Device, &d.client))
            }
        }
    }

    /// Requests waiting for the person, oldest first.
    pub fn pending_devices(&self) -> Vec<PendingDevice> {
        let now = Instant::now();
        let wall = Utc::now();
        let mut devices = self.devices.lock().unwrap_or_else(|e| e.into_inner());
        devices.retain(|d| d.expires > now);
        devices
            .iter()
            .filter(|d| d.state == DeviceState::Pending)
            .map(|d| PendingDevice {
                user_code: format_user_code(&d.user_code),
                browser: d.client.browser.clone(),
                ip: d.client.ip.clone(),
                created_at: d.created_at,
                expires_at: wall + chrono::TimeDelta::from_std(d.expires - now).unwrap_or_default(),
            })
            .collect()
    }

    /// Approves (or denies) the waiting request with this user code; false when there is none.
    pub fn decide_device(&self, user_code: &str, approve: bool) -> bool {
        let code = normalize_user_code(user_code);
        let now = Instant::now();
        let mut devices = self.devices.lock().unwrap_or_else(|e| e.into_inner());
        devices.retain(|d| d.expires > now);
        match devices
            .iter_mut()
            .find(|d| d.state == DeviceState::Pending && !code.is_empty() && d.user_code == code)
        {
            Some(d) => {
                d.state = if approve { DeviceState::Approved } else { DeviceState::Denied };
                true
            }
            None => false,
        }
    }
}

fn random_user_code() -> String {
    let mut n = uuid::Uuid::new_v4().as_u128();
    let base = USER_CODE_ALPHABET.len() as u128;
    (0..8)
        .map(|_| {
            let c = USER_CODE_ALPHABET[(n % base) as usize] as char;
            n /= base;
            c
        })
        .collect()
}

/// `BCDF2345` → `BCDF-2345`.
fn format_user_code(code: &str) -> String {
    format!("{}-{}", &code[..4], &code[4..])
}

/// What the person typed or tapped, as stored: upper case, without dashes or spaces.
fn normalize_user_code(code: &str) -> String {
    code.chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .map(|c| c.to_ascii_uppercase())
        .collect()
}

/// Expired live links become spent; spent ones are forgotten after an hour.
fn sweep(links: &mut HashMap<String, LinkState>, now: Instant) {
    links.retain(|_, s| !matches!(s, LinkState::Spent(forget) if *forget <= now));
    for s in links.values_mut() {
        if let LinkState::Live(exp) = s {
            if *exp <= now {
                *s = LinkState::Spent(now + SPENT_MEMORY);
            }
        }
    }
}

/// "Chrome on Android" from a User-Agent, for people to recognise their own devices.
pub fn browser_label(user_agent: &str) -> String {
    let ua = user_agent;
    let browser = if ua.contains("Edg/") {
        "Edge"
    } else if ua.contains("OPR/") {
        "Opera"
    } else if ua.contains("Firefox/") || ua.contains("FxiOS/") {
        "Firefox"
    } else if ua.contains("Chrome/") || ua.contains("CriOS/") {
        "Chrome"
    } else if ua.contains("Safari/") {
        "Safari"
    } else if ua.contains("curl/") {
        "curl"
    } else {
        "Browser"
    };
    let os = if ua.contains("iPhone") {
        "iPhone"
    } else if ua.contains("iPad") {
        "iPad"
    } else if ua.contains("Android") {
        "Android"
    } else if ua.contains("Mac OS X") || ua.contains("Macintosh") {
        "Mac"
    } else if ua.contains("Windows") {
        "Windows"
    } else if ua.contains("CrOS") {
        "ChromeOS"
    } else if ua.contains("Linux") {
        "Linux"
    } else {
        ""
    };
    if os.is_empty() {
        browser.to_string()
    } else {
        format!("{browser} on {os}")
    }
}

/// The client's address for display and rate limits: behind a reverse proxy, the first
/// `X-Forwarded-For` hop (which a client without a proxy can set to anything), else the peer.
pub fn client_ip(headers: &axum::http::HeaderMap, peer: Option<IpAddr>) -> String {
    headers
        .get("x-forwarded-for")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.split(',').next())
        .map(str::trim)
        .filter(|v| v.parse::<IpAddr>().is_ok())
        .map(str::to_string)
        .or_else(|| peer.map(|p| p.to_canonical().to_string()))
        .unwrap_or_default()
}

pub fn client(headers: &axum::http::HeaderMap, peer: Option<IpAddr>) -> Client {
    let ua = headers
        .get(axum::http::header::USER_AGENT)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    Client {
        browser: browser_label(ua),
        ip: client_ip(headers, peer),
    }
}

/// A QR code for `text` as terminal lines (two modules per character, with a quiet zone), dark
/// modules drawn as spaces on a light background so it scans in dark and light terminals alike.
pub fn qr_lines(text: &str) -> Vec<String> {
    let Ok(code) = qrcode::QrCode::with_error_correction_level(text.as_bytes(), qrcode::EcLevel::L) else {
        return Vec::new();
    };
    let width = code.width();
    let colors = code.to_colors();
    let quiet = 2usize;
    let size = width + 2 * quiet;
    let dark = |x: usize, y: usize| -> bool {
        if x < quiet || y < quiet || x >= quiet + width || y >= quiet + width {
            return false;
        }
        colors[(y - quiet) * width + (x - quiet)] == qrcode::Color::Dark
    };
    let mut lines = Vec::new();
    let mut y = 0;
    while y < size {
        let mut line = String::new();
        for x in 0..size {
            // Light modules are drawn (█), dark ones left blank: top and bottom half per character
            let top = !dark(x, y);
            let bottom = y + 1 >= size || !dark(x, y + 1);
            line.push(match (top, bottom) {
                (true, true) => '█',
                (true, false) => '▀',
                (false, true) => '▄',
                (false, false) => ' ',
            });
        }
        lines.push(line);
        y += 2;
    }
    lines
}

/// `kwnt signin-link`: asks the running server (with the API token, from this machine) for a new
/// one-time link and prints it with a QR code.
pub async fn cli_link(config: &crate::config::Config, url: Option<&str>, qr: bool) -> Result<(), String> {
    use super::auth;
    let token = auth::env_token()
        .map(Ok)
        .unwrap_or_else(|| auth::read_existing(&config.data_dir))
        .map_err(|e| format!("{e}. Run it as the user that runs `kwnt serve`, with the same KWNT_DATA_DIR."))?;
    let bound = config.host.trim_matches(['[', ']']);
    let reach = if auth::is_loopback_bind(bound) || bound.is_empty() || bound.parse::<IpAddr>().is_ok_and(|ip| ip.is_unspecified()) {
        "127.0.0.1".to_string()
    } else if bound.parse::<std::net::Ipv6Addr>().is_ok() {
        format!("[{bound}]")
    } else {
        bound.to_string()
    };
    let endpoint = format!("http://{reach}:{}/api/signin/links", config.port);
    let resp = reqwest::Client::builder()
        .no_proxy()
        .timeout(Duration::from_secs(10))
        .build()
        .map_err(|e| e.to_string())?
        .post(&endpoint)
        .bearer_auth(&token)
        .send()
        .await
        .map_err(|e| format!("can't reach kwnt at {endpoint} ({e}). Is `kwnt serve` running?"))?;
    if !resp.status().is_success() {
        return Err(format!("kwnt answered {} at {endpoint}", resp.status()));
    }
    let body: serde_json::Value = resp.json().await.map_err(|e| e.to_string())?;
    let code = body["code"].as_str().ok_or("no code in kwnt's answer")?;
    let base = url.map(str::to_string).or_else(|| {
        body["url"]
            .as_str()
            .and_then(|u| u.split_once("/#").map(|(b, _)| b.to_string()))
    });
    let link = auth::one_time_link(&config.host, config.port, base.as_deref(), code);
    for line in auth::one_time_notice(&link, qr) {
        println!("{line}");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn me() -> Client {
        Client { browser: "Chrome on Android".into(), ip: "203.0.113.9".into() }
    }

    #[test]
    fn a_link_signs_in_once() {
        let s = SignIn::in_memory();
        let code = s.create_link(LINK_TTL);
        assert!(code.starts_with(LINK_PREFIX));
        let Redeem::Ok(secret) = s.redeem_link(&code, &me()) else { panic!("first use") };
        assert!(s.session_ok(&secret));
        assert_eq!(s.redeem_link(&code, &me()), Redeem::Spent);
        assert_eq!(s.sessions().len(), 1);
        assert_eq!(s.sessions()[0].via, Via::Link);
    }

    #[test]
    fn an_expired_link_is_refused() {
        let s = SignIn::in_memory();
        let code = s.create_link(Duration::from_millis(1));
        std::thread::sleep(Duration::from_millis(5));
        assert_eq!(s.redeem_link(&code, &me()), Redeem::Spent);
        assert!(s.sessions().is_empty());
    }

    #[test]
    fn unknown_and_tampered_links_are_refused() {
        let s = SignIn::in_memory();
        let code = s.create_link(LINK_TTL);
        assert_eq!(s.redeem_link("kl1.nope", &me()), Redeem::Unknown);
        assert_eq!(s.redeem_link(&format!("{code}x"), &me()), Redeem::Unknown);
        // A session secret is not a link, and a link is not a session
        assert!(!s.session_ok(&code));
        assert!(matches!(s.redeem_link(&code, &me()), Redeem::Ok(_)));
    }

    #[test]
    fn too_many_links_drop_the_oldest() {
        let s = SignIn::in_memory();
        let first = s.create_link(Duration::from_secs(60));
        for _ in 0..MAX_LINKS {
            s.create_link(LINK_TTL);
        }
        assert_eq!(s.redeem_link(&first, &me()), Redeem::Spent);
    }

    #[test]
    fn sessions_persist_hashed_and_revoke() {
        let dir = tempfile::tempdir().unwrap();
        let s = SignIn::load(dir.path());
        let a = s.create_session(Via::Token, &me());
        let b = s.create_session(Via::Link, &me());
        let file = std::fs::read_to_string(dir.path().join(SESSIONS_FILE)).unwrap();
        assert!(!file.contains(&a) && !file.contains(&b), "secrets on disk");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(dir.path().join(SESSIONS_FILE)).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o600);
        }
        // Survives a restart
        let s2 = SignIn::load(dir.path());
        assert!(s2.session_ok(&a) && s2.session_ok(&b));
        let id = s2.session_id(&a).unwrap();
        assert!(s2.revoke(&id));
        assert!(!s2.revoke(&id));
        assert!(!s2.session_ok(&a));
        s2.revoke_secret(&b);
        assert!(!s2.session_ok(&b));
        assert!(SignIn::load(dir.path()).sessions().is_empty());
    }

    #[test]
    fn a_corrupt_sessions_file_signs_everyone_out() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join(SESSIONS_FILE), "{not json").unwrap();
        assert!(SignIn::load(dir.path()).sessions().is_empty());
    }

    #[test]
    fn a_device_is_let_in_once_after_approval() {
        let s = SignIn::in_memory();
        let (device, user) = s.start_device(&me()).unwrap();
        assert!(device.starts_with(DEVICE_PREFIX));
        assert_eq!(user.len(), 9);
        assert_eq!(&user[4..5], "-");
        assert_eq!(s.poll_device(&device), Poll::Pending);
        let pending = s.pending_devices();
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].user_code, user);
        assert_eq!(pending[0].browser, "Chrome on Android");
        // Typed in lower case without the dash still matches
        assert!(s.decide_device(&user.replace('-', "").to_lowercase(), true));
        assert!(s.pending_devices().is_empty());
        let Poll::Approved(secret) = s.poll_device(&device) else { panic!("approved") };
        assert!(s.session_ok(&secret));
        assert_eq!(s.sessions()[0].via, Via::Device);
        // Single use: the device code and the user code are spent
        assert_eq!(s.poll_device(&device), Poll::Expired);
        assert!(!s.decide_device(&user, true));
        assert_eq!(s.sessions().len(), 1);
    }

    #[test]
    fn a_denied_device_gets_nothing() {
        let s = SignIn::in_memory();
        let (device, user) = s.start_device(&me()).unwrap();
        assert!(s.decide_device(&user, false));
        assert!(!s.decide_device(&user, true), "a decision is final");
        assert_eq!(s.poll_device(&device), Poll::Denied);
        assert_eq!(s.poll_device(&device), Poll::Expired);
        assert!(s.sessions().is_empty());
    }

    #[test]
    fn an_expired_device_request_cannot_be_approved() {
        let s = SignIn::in_memory();
        let (device, user) = s.start_device_for(&me(), Duration::from_millis(1)).unwrap();
        std::thread::sleep(Duration::from_millis(5));
        assert!(s.pending_devices().is_empty());
        assert!(!s.decide_device(&user, true));
        assert_eq!(s.poll_device(&device), Poll::Expired);
        assert!(s.sessions().is_empty());
    }

    #[test]
    fn unknown_codes_match_nothing() {
        let s = SignIn::in_memory();
        let (device, _) = s.start_device(&me()).unwrap();
        assert!(!s.decide_device("BCDF-2345", true));
        assert!(!s.decide_device("", true));
        assert!(!s.decide_device("----", true));
        assert_eq!(s.poll_device("kd1.nope"), Poll::Expired);
        assert_eq!(s.poll_device(&format!("{device}x")), Poll::Expired);
        assert_eq!(s.poll_device(&device), Poll::Pending);
    }

    #[test]
    fn device_requests_are_rate_limited_and_capped() {
        let s = SignIn::in_memory();
        for _ in 0..PER_IP_PER_MIN {
            s.start_device(&me()).unwrap();
        }
        let wait = s.start_device(&me()).unwrap_err();
        assert!((1..=60).contains(&wait), "{wait}");
        // Another address still may, until MAX_PENDING are waiting
        let mut n = PER_IP_PER_MIN;
        let mut ip = 0;
        while n < MAX_PENDING {
            ip += 1;
            let c = Client { browser: "x".into(), ip: format!("198.51.100.{ip}") };
            if s.start_device(&c).is_ok() {
                n += 1;
            }
        }
        let other = Client { browser: "x".into(), ip: "192.0.2.1".into() };
        assert!(s.start_device(&other).is_err(), "cap on waiting requests");
        // Deciding one frees a slot
        let code = s.pending_devices()[0].user_code.clone();
        assert!(s.decide_device(&code, false));
        assert!(s.start_device(&other).is_ok());
    }

    #[test]
    fn user_codes_avoid_lookalikes() {
        for _ in 0..200 {
            let c = random_user_code();
            assert_eq!(c.len(), 8);
            assert!(c.bytes().all(|b| USER_CODE_ALPHABET.contains(&b)), "{c}");
        }
    }

    #[test]
    fn labels() {
        let android = "Mozilla/5.0 (Linux; Android 14; Pixel 8) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/129.0 Mobile Safari/537.36";
        assert_eq!(browser_label(android), "Chrome on Android");
        let iphone = "Mozilla/5.0 (iPhone; CPU iPhone OS 17_5 like Mac OS X) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/17.5 Mobile/15E148 Safari/604.1";
        assert_eq!(browser_label(iphone), "Safari on iPhone");
        let mac_ff = "Mozilla/5.0 (Macintosh; Intel Mac OS X 14.5; rv:131.0) Gecko/20100101 Firefox/131.0";
        assert_eq!(browser_label(mac_ff), "Firefox on Mac");
        assert_eq!(browser_label(""), "Browser");
    }

    #[test]
    fn client_ip_prefers_the_proxy_hop() {
        use axum::http::{HeaderMap, HeaderValue};
        let mut h = HeaderMap::new();
        let peer: Option<IpAddr> = Some("172.18.0.1".parse().unwrap());
        assert_eq!(client_ip(&h, peer), "172.18.0.1");
        h.insert("x-forwarded-for", HeaderValue::from_static("198.51.100.4, 172.18.0.1"));
        assert_eq!(client_ip(&h, peer), "198.51.100.4");
        h.insert("x-forwarded-for", HeaderValue::from_static("not-an-ip"));
        assert_eq!(client_ip(&h, peer), "172.18.0.1");
    }

    #[test]
    fn qr_is_square_text() {
        let lines = qr_lines("https://kwant.example.com/#kwnt_token=kl1.0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcd");
        assert!(lines.len() > 10);
        let w = lines[0].chars().count();
        assert!(lines.iter().all(|l| l.chars().count() == w));
        assert_eq!(lines.len(), w.div_ceil(2));
    }
}
