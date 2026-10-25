//! Web Push (RFC 8030) to the person's installed app, so a bot that needs them reaches their
//! phone with kwant closed. The browser subscribes with our VAPID public key (RFC 8292) and hands
//! us its push service endpoint; every notification is encrypted for that browser (RFC 8291,
//! aes128gcm), so the push service (Google's, Apple's, Mozilla's) only relays ciphertext.
//!
//! The VAPID key is made on first use and kept in `<data_dir>/vapid_key` (0600, PKCS#8); only
//! its public half ever leaves this machine. Subscriptions live in `<data_dir>/push.json` (0600):
//! an endpoint is a capability URL, so it is never logged beyond its host.
//!
//! Endpoints must be https and resolve to public addresses only, checked again on every send
//! through our own resolver (no proxy), so a subscription can't make kwnt call into its own
//! network. A 404 or 410 from the push service drops the subscription; nothing is retried.

use base64::engine::general_purpose::URL_SAFE_NO_PAD as B64;
use base64::Engine;
use chrono::{DateTime, Utc};
use ring::{aead, agreement, hmac, rand::SecureRandom, signature};
use serde::{Deserialize, Serialize};
use std::net::{IpAddr, SocketAddr};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

const KEY_FILE: &str = "vapid_key";
const SUBS_FILE: &str = "push.json";
/// The VAPID `sub` claim: who the push service can write to about our traffic.
const VAPID_SUBJECT: &str = "mailto:contact@kwant.si";
/// VAPID tokens are good for this long (RFC 8292 allows up to 24 hours).
const JWT_SECS: i64 = 12 * 60 * 60;
/// Subscriptions kept at once; a new one past this drops the oldest.
pub const MAX_SUBSCRIPTIONS: usize = 20;
const MAX_ENDPOINT_LEN: usize = 2048;
const MAX_LABEL_CHARS: usize = 200;
/// Longest notification body in a push.
pub const PUSH_BODY_CHARS: usize = 160;
/// Pushes we send stay well under the 4 KB push services accept.
pub const MAX_PAYLOAD_BYTES: usize = 3 * 1024;
/// Record size in the aes128gcm header; one record always holds our payload.
const RECORD_SIZE: u32 = 4096;
const SEND_TIMEOUT: Duration = Duration::from_secs(10);
const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);

const QUERY_VALUE: &percent_encoding::AsciiSet = &percent_encoding::NON_ALPHANUMERIC.remove(b'-').remove(b'_').remove(b'.');

/// A browser that asked for pushes.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Subscription {
    pub endpoint: String,
    /// The browser's P-256 public key, base64url (65 bytes uncompressed).
    pub p256dh: String,
    /// The browser's auth secret, base64url (16 bytes).
    pub auth: String,
    #[serde(default)]
    pub label: String,
    /// The sign-in session that subscribed, so signing that browser out stops its pushes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    pub created_at: DateTime<Utc>,
}

/// What a push carries, as the service worker reads it.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Payload {
    pub title: String,
    pub body: String,
    pub kind: String,
    pub space_id: Option<String>,
    pub channel_id: Option<String>,
    pub kwant_id: String,
    /// Relative: `/?space=<id>&channel=<id>`, or `&kwant=<id>` without a channel.
    pub url: String,
    pub tag: String,
}

impl Payload {
    pub fn new(
        kind: &str,
        title: &str,
        body: &str,
        space_id: Option<String>,
        channel_id: Option<String>,
        kwant_id: String,
    ) -> Self {
        let enc = |s: &str| percent_encoding::utf8_percent_encode(s, QUERY_VALUE).to_string();
        let mut query = Vec::new();
        if let Some(s) = &space_id {
            query.push(format!("space={}", enc(s)));
        }
        match &channel_id {
            Some(c) => query.push(format!("channel={}", enc(c))),
            None => query.push(format!("kwant={}", enc(&kwant_id))),
        }
        let tag = format!(
            "kwnt-{}-{kind}-{}",
            space_id.as_deref().unwrap_or("none"),
            channel_id.as_deref().unwrap_or(&kwant_id)
        );
        Self {
            title: clip(title, 120),
            body: clip(body, PUSH_BODY_CHARS),
            kind: kind.into(),
            space_id,
            channel_id,
            kwant_id,
            url: format!("/?{}", query.join("&")),
            tag,
        }
    }

    /// The person is waiting on these: kept a day and sent at high urgency.
    pub fn needs_person(&self) -> bool {
        matches!(self.kind.as_str(), "approval_needed" | "question_asked" | "human_step")
    }

    pub fn ttl(&self) -> u32 {
        if self.needs_person() {
            86_400
        } else {
            3_600
        }
    }

    pub fn urgency(&self) -> &'static str {
        if self.needs_person() {
            "high"
        } else {
            "normal"
        }
    }

    /// The JSON the browser gets, shortened to fit `MAX_PAYLOAD_BYTES`.
    fn to_bytes(&self) -> Vec<u8> {
        let mut p = self.clone();
        loop {
            let bytes = serde_json::to_vec(&p).unwrap_or_default();
            if bytes.len() <= MAX_PAYLOAD_BYTES || (p.body.is_empty() && p.title.chars().count() <= 40) {
                return bytes;
            }
            if !p.body.is_empty() {
                p.body.clear();
            } else {
                p.title = clip(&p.title, 40);
            }
        }
    }
}

fn clip(text: &str, max: usize) -> String {
    let flat = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if flat.chars().count() <= max {
        return flat;
    }
    let mut out: String = flat.chars().take(max.saturating_sub(1)).collect();
    out.push('…');
    out
}

/// Why a subscription was refused (400).
#[derive(Debug, PartialEq, Eq)]
pub struct Invalid(pub &'static str);

fn decode(s: &str) -> Option<Vec<u8>> {
    // Browsers send base64url without padding; tolerate padding and plain base64 too
    let s = s.trim().trim_end_matches('=').replace('+', "-").replace('/', "_");
    B64.decode(s).ok()
}

/// Addresses on the public internet; everything else is this machine or its network.
pub fn is_public_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => {
            let [a, b, ..] = v4.octets();
            !(v4.is_private()
                || v4.is_loopback()
                || v4.is_link_local()
                || v4.is_broadcast()
                || v4.is_documentation()
                || v4.is_unspecified()
                || v4.is_multicast()
                || a == 0
                || a >= 240
                || (a == 100 && (64..128).contains(&b)) // shared (CGNAT), 100.64/10
                || (a == 192 && b == 0 && v4.octets()[2] == 0) // IETF protocol assignments
                || (a == 198 && (b == 18 || b == 19))) // benchmarking
        }
        IpAddr::V6(v6) => {
            if let Some(v4) = v6.to_ipv4_mapped() {
                return is_public_ip(IpAddr::V4(v4));
            }
            let seg = v6.segments();
            !(v6.is_loopback()
                || v6.is_unspecified()
                || v6.is_multicast()
                || (seg[0] & 0xfe00) == 0xfc00 // unique local
                || (seg[0] & 0xffc0) == 0xfe80 // link local
                || (seg[0] == 0x2001 && seg[1] == 0x0db8) // documentation
                || (seg[0] == 0x0064 && seg[1] == 0xff9b) // NAT64 to anything, incl. private v4
                || (seg[0] == 0 && seg[1] == 0 && seg[2] == 0 && seg[3] == 0 && seg[4] == 0 && seg[5] == 0)) // v4-compatible
        }
    }
}

/// Checks an endpoint before it is stored: https, a host name or public address, no user info.
/// Names are resolved (and checked) again on each send.
pub fn check_endpoint(raw: &str) -> Result<reqwest::Url, Invalid> {
    if raw.len() > MAX_ENDPOINT_LEN {
        return Err(Invalid("endpoint is too long"));
    }
    let url = reqwest::Url::parse(raw).map_err(|_| Invalid("endpoint is not a valid URL"))?;
    if url.scheme() != "https" {
        return Err(Invalid("endpoint must be an https:// URL"));
    }
    if !url.username().is_empty() || url.password().is_some() {
        return Err(Invalid("endpoint must not contain a user name or password"));
    }
    match url.host() {
        None => Err(Invalid("endpoint has no host")),
        Some(url::Host::Ipv4(ip)) if !is_public_ip(IpAddr::V4(ip)) => Err(Invalid("endpoint must be on the public internet")),
        Some(url::Host::Ipv6(ip)) if !is_public_ip(IpAddr::V6(ip)) => Err(Invalid("endpoint must be on the public internet")),
        Some(url::Host::Domain(d)) => {
            let d = d.trim_end_matches('.').to_ascii_lowercase();
            if d == "localhost" || d.ends_with(".localhost") || !d.contains('.') {
                Err(Invalid("endpoint must be on the public internet"))
            } else {
                Ok(url)
            }
        }
        Some(_) => Ok(url),
    }
}

/// Checks the browser's keys: a 65-byte uncompressed P-256 point and a 16-byte auth secret.
pub fn check_keys(p256dh: &str, auth: &str) -> Result<(), Invalid> {
    match decode(p256dh) {
        Some(k) if k.len() == 65 && k[0] == 4 => {}
        _ => return Err(Invalid("keys.p256dh must be a base64url P-256 public key")),
    }
    match decode(auth) {
        Some(a) if a.len() == 16 => Ok(()),
        _ => Err(Invalid("keys.auth must be a base64url 16-byte secret")),
    }
}

fn hmac_sha256(key: &[u8], parts: &[&[u8]]) -> [u8; 32] {
    let key = hmac::Key::new(hmac::HMAC_SHA256, key);
    let mut ctx = hmac::Context::with_key(&key);
    for p in parts {
        ctx.update(p);
    }
    let mut out = [0u8; 32];
    out.copy_from_slice(ctx.sign().as_ref());
    out
}

/// RFC 8291 section 3.4 and RFC 8188: one aes128gcm record for `plaintext`, given the ECDH
/// secret already agreed. Split out so the RFC's test vector can be checked without its private key.
pub fn encrypt_with_secret(
    ecdh_secret: &[u8],
    auth_secret: &[u8],
    ua_public: &[u8],
    as_public: &[u8],
    salt: &[u8; 16],
    plaintext: &[u8],
) -> Result<Vec<u8>, &'static str> {
    if as_public.len() != 65 {
        return Err("bad server key");
    }
    if plaintext.len() + 1 + 16 > RECORD_SIZE as usize - 86 {
        return Err("payload too large");
    }
    // PRK_key = HMAC(auth_secret, ecdh_secret); IKM = HKDF-Expand(PRK_key, key_info, 32)
    let prk_key = hmac_sha256(auth_secret, &[ecdh_secret]);
    let ikm = hmac_sha256(&prk_key, &[b"WebPush: info\0", ua_public, as_public, &[1]]);
    let prk = hmac_sha256(salt, &[&ikm]);
    let cek = hmac_sha256(&prk, &[b"Content-Encoding: aes128gcm\0", &[1]]);
    let nonce = hmac_sha256(&prk, &[b"Content-Encoding: nonce\0", &[1]]);

    let key = aead::LessSafeKey::new(aead::UnboundKey::new(&aead::AES_128_GCM, &cek[..16]).map_err(|_| "bad key")?);
    let nonce = aead::Nonce::try_assume_unique_for_key(&nonce[..12]).map_err(|_| "bad nonce")?;
    // The last (and only) record ends with the 0x02 delimiter and no padding
    let mut record = Vec::with_capacity(plaintext.len() + 17);
    record.extend_from_slice(plaintext);
    record.push(2);
    key.seal_in_place_append_tag(nonce, aead::Aad::empty(), &mut record)
        .map_err(|_| "encryption failed")?;

    let mut out = Vec::with_capacity(86 + record.len());
    out.extend_from_slice(salt);
    out.extend_from_slice(&RECORD_SIZE.to_be_bytes());
    out.push(as_public.len() as u8);
    out.extend_from_slice(as_public);
    out.extend_from_slice(&record);
    Ok(out)
}

/// The browser's side of `encrypt_with_secret` (one record), for tests and the fake push
/// service: the plaintext, or `None` when the body doesn't open.
pub fn decrypt_with_secret(ecdh_secret: &[u8], auth_secret: &[u8], ua_public: &[u8], body: &[u8]) -> Option<Vec<u8>> {
    if body.len() < 86 + 17 || u32::from_be_bytes(body[16..20].try_into().ok()?) != RECORD_SIZE || body[20] != 65 {
        return None;
    }
    let (salt, as_public) = (&body[..16], &body[21..86]);
    let prk_key = hmac_sha256(auth_secret, &[ecdh_secret]);
    let ikm = hmac_sha256(&prk_key, &[b"WebPush: info\0", ua_public, as_public, &[1]]);
    let prk = hmac_sha256(salt, &[&ikm]);
    let cek = hmac_sha256(&prk, &[b"Content-Encoding: aes128gcm\0", &[1]]);
    let nonce = hmac_sha256(&prk, &[b"Content-Encoding: nonce\0", &[1]]);
    let key = aead::LessSafeKey::new(aead::UnboundKey::new(&aead::AES_128_GCM, &cek[..16]).ok()?);
    let nonce = aead::Nonce::try_assume_unique_for_key(&nonce[..12]).ok()?;
    let mut record = body[86..].to_vec();
    let plain = key.open_in_place(nonce, aead::Aad::empty(), &mut record).ok()?;
    match plain.split_last() {
        Some((2, rest)) => Some(rest.to_vec()),
        _ => None,
    }
}

/// Encrypts `plaintext` for one browser with a fresh server key pair and salt.
pub fn encrypt(ua_public: &[u8], auth_secret: &[u8], plaintext: &[u8]) -> Result<Vec<u8>, &'static str> {
    let rng = ring::rand::SystemRandom::new();
    let private = agreement::EphemeralPrivateKey::generate(&agreement::ECDH_P256, &rng).map_err(|_| "rng")?;
    let as_public = private.compute_public_key().map_err(|_| "rng")?;
    let mut salt = [0u8; 16];
    rng.fill(&mut salt).map_err(|_| "rng")?;
    let peer = agreement::UnparsedPublicKey::new(&agreement::ECDH_P256, ua_public);
    let secret = agreement::agree_ephemeral(private, &peer, |s| s.to_vec()).map_err(|_| "bad browser key")?;
    encrypt_with_secret(&secret, auth_secret, ua_public, as_public.as_ref(), &salt, plaintext)
}

/// The VAPID key pair (ECDSA P-256).
pub struct Vapid {
    pair: signature::EcdsaKeyPair,
}

impl Vapid {
    fn from_pkcs8(der: &[u8]) -> Option<Self> {
        let rng = ring::rand::SystemRandom::new();
        signature::EcdsaKeyPair::from_pkcs8(&signature::ECDSA_P256_SHA256_FIXED_SIGNING, der, &rng)
            .ok()
            .map(|pair| Self { pair })
    }

    fn generate() -> Option<(Self, Vec<u8>)> {
        let rng = ring::rand::SystemRandom::new();
        let der = signature::EcdsaKeyPair::generate_pkcs8(&signature::ECDSA_P256_SHA256_FIXED_SIGNING, &rng).ok()?;
        let der = der.as_ref().to_vec();
        Some((Self::from_pkcs8(&der)?, der))
    }

    /// The uncompressed public key, base64url: the browser's `applicationServerKey`.
    pub fn public_key(&self) -> String {
        use signature::KeyPair;
        B64.encode(self.pair.public_key().as_ref())
    }

    /// `Authorization: vapid t=<jwt>, k=<public key>` for `endpoint`'s push service.
    pub fn authorization(&self, endpoint: &reqwest::Url, now: DateTime<Utc>) -> Option<String> {
        let aud = endpoint.origin().ascii_serialization();
        let header = B64.encode(br#"{"typ":"JWT","alg":"ES256"}"#);
        let claims = serde_json::json!({ "aud": aud, "exp": now.timestamp() + JWT_SECS, "sub": VAPID_SUBJECT });
        let claims = B64.encode(serde_json::to_vec(&claims).ok()?);
        let signing_input = format!("{header}.{claims}");
        let rng = ring::rand::SystemRandom::new();
        let sig = self.pair.sign(&rng, signing_input.as_bytes()).ok()?;
        Some(format!("vapid t={signing_input}.{}, k={}", B64.encode(sig.as_ref()), self.public_key()))
    }
}

/// Resolves push service names and keeps only public addresses, so a name pointing into this
/// network (or re-pointed after subscribing) is never connected to.
struct PublicOnly;

impl reqwest::dns::Resolve for PublicOnly {
    fn resolve(&self, name: reqwest::dns::Name) -> reqwest::dns::Resolving {
        let host = name.as_str().to_string();
        Box::pin(async move {
            let addrs: Vec<SocketAddr> = tokio::net::lookup_host((host.as_str(), 0))
                .await?
                .filter(|a| is_public_ip(a.ip()))
                .collect();
            if addrs.is_empty() {
                return Err("push endpoint does not resolve to a public address".into());
            }
            Ok(Box::new(addrs.into_iter()) as reqwest::dns::Addrs)
        })
    }
}

/// What one send found.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Sent {
    Ok,
    /// 404 or 410: the browser unsubscribed; drop it.
    Gone,
    Failed,
}

pub struct Push {
    dir: Option<PathBuf>,
    vapid: Mutex<Option<Arc<Vapid>>>,
    subs: Mutex<Vec<Subscription>>,
    client: reqwest::Client,
    /// Tests send to a local http endpoint.
    allow_local: bool,
}

impl std::fmt::Debug for Push {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Push").finish_non_exhaustive()
    }
}

impl Push {
    /// Subscriptions from `<data_dir>/push.json`; the key is read or made on first use.
    pub fn load(data_dir: &Path) -> Self {
        let subs = match std::fs::read_to_string(data_dir.join(SUBS_FILE)) {
            Ok(s) => serde_json::from_str(&s).unwrap_or_else(|e| {
                tracing::warn!(error = %e, "push.json is unreadable; devices have to turn notifications on again");
                Vec::new()
            }),
            Err(_) => Vec::new(),
        };
        Self::build(Some(data_dir.to_path_buf()), subs, false)
    }

    /// Sends to `http://127.0.0.1` too, for tests with a fake push service. Never used by `serve`.
    pub fn for_tests_allowing_local(data_dir: &Path) -> Self {
        Self::build(Some(data_dir.to_path_buf()), Vec::new(), true)
    }

    fn build(dir: Option<PathBuf>, subs: Vec<Subscription>, allow_local: bool) -> Self {
        let mut builder = reqwest::Client::builder()
            .timeout(SEND_TIMEOUT)
            .connect_timeout(CONNECT_TIMEOUT)
            .redirect(reqwest::redirect::Policy::none())
            // A proxy would resolve the name itself, past our check
            .no_proxy();
        if !allow_local {
            builder = builder.dns_resolver(Arc::new(PublicOnly));
        }
        Self {
            dir,
            vapid: Mutex::new(None),
            subs: Mutex::new(subs),
            client: builder.build().unwrap_or_default(),
            allow_local,
        }
    }

    /// The VAPID key, read from the data dir or made (and saved) the first time.
    pub fn vapid(&self) -> Option<Arc<Vapid>> {
        let mut slot = self.vapid.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(v) = slot.as_ref() {
            return Some(v.clone());
        }
        let path = self.dir.as_ref().map(|d| d.join(KEY_FILE));
        let existing = path
            .as_ref()
            .and_then(|p| std::fs::read_to_string(p).ok())
            .and_then(|s| decode(&s))
            .and_then(|der| Vapid::from_pkcs8(&der));
        let vapid = match existing {
            Some(v) => v,
            None => {
                let (v, der) = Vapid::generate()?;
                if let Some(p) = &path {
                    if let Some(d) = p.parent() {
                        let _ = std::fs::create_dir_all(d);
                    }
                    if let Err(e) = super::auth::write_private(p, &B64.encode(&der)) {
                        // Without the file every restart makes a new key and old subscriptions stop working
                        tracing::warn!(error = %e, "could not save the push key");
                    }
                }
                v
            }
        };
        let v = Arc::new(vapid);
        *slot = Some(v.clone());
        Some(v)
    }

    fn check(&self, endpoint: &str) -> Result<(), Invalid> {
        if self.allow_local && endpoint.starts_with("http://127.0.0.1:") {
            return Ok(());
        }
        check_endpoint(endpoint).map(|_| ())
    }

    fn save(&self, subs: &[Subscription]) {
        let Some(dir) = &self.dir else { return };
        let Ok(json) = serde_json::to_string_pretty(subs) else { return };
        let path = dir.join(SUBS_FILE);
        let tmp = path.with_extension("json.tmp");
        let res = super::auth::write_private(&tmp, &json).and_then(|_| std::fs::rename(&tmp, &path));
        if let Err(e) = res {
            tracing::warn!(error = %e, "could not save push subscriptions");
        }
    }

    /// Adds or refreshes a subscription (by endpoint). Past the cap the oldest is dropped.
    pub fn subscribe(
        &self,
        endpoint: &str,
        p256dh: &str,
        auth: &str,
        label: &str,
        session_id: Option<String>,
    ) -> Result<(), Invalid> {
        self.check(endpoint)?;
        check_keys(p256dh, auth)?;
        let sub = Subscription {
            endpoint: endpoint.to_string(),
            p256dh: p256dh.trim().to_string(),
            auth: auth.trim().to_string(),
            label: clip(label, MAX_LABEL_CHARS),
            session_id,
            created_at: Utc::now(),
        };
        let mut subs = self.subs.lock().unwrap_or_else(|e| e.into_inner());
        subs.retain(|s| s.endpoint != sub.endpoint);
        subs.push(sub);
        subs.sort_by_key(|s| s.created_at);
        while subs.len() > MAX_SUBSCRIPTIONS {
            subs.remove(0);
        }
        self.save(&subs);
        Ok(())
    }

    /// Removes one subscription; false when there was none.
    pub fn unsubscribe(&self, endpoint: &str) -> bool {
        self.remove(|s| s.endpoint == endpoint)
    }

    /// Stops pushes to the browsers a signed-out session subscribed.
    pub fn drop_session(&self, session_id: &str) -> bool {
        self.remove(|s| s.session_id.as_deref() == Some(session_id))
    }

    fn remove(&self, f: impl Fn(&Subscription) -> bool) -> bool {
        let mut subs = self.subs.lock().unwrap_or_else(|e| e.into_inner());
        let before = subs.len();
        subs.retain(|s| !f(s));
        let removed = subs.len() != before;
        if removed {
            self.save(&subs);
        }
        removed
    }

    pub fn subscriptions(&self) -> Vec<Subscription> {
        self.subs.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }

    /// Sends `payload` to every subscription at once and drops the ones that are gone. Each
    /// send has its own timeout; failures are logged (host only) and not retried.
    pub async fn send_all(&self, payload: &Payload) {
        let subs = self.subscriptions();
        if subs.is_empty() {
            return;
        }
        let Some(vapid) = self.vapid() else {
            return tracing::warn!("push key unavailable; notification not pushed");
        };
        let bytes = payload.to_bytes();
        let topic = topic(&payload.tag);
        let sends = subs.iter().map(|s| self.send_one(&vapid, s, &bytes, payload, &topic));
        let results = futures::future::join_all(sends).await;
        let gone: Vec<&str> = subs
            .iter()
            .zip(results)
            .filter(|(_, r)| *r == Sent::Gone)
            .map(|(s, _)| s.endpoint.as_str())
            .collect();
        if !gone.is_empty() {
            self.remove(|s| gone.contains(&s.endpoint.as_str()));
        }
    }

    async fn send_one(&self, vapid: &Vapid, sub: &Subscription, bytes: &[u8], payload: &Payload, topic: &str) -> Sent {
        let host = super::continuity::loggable_url(&sub.endpoint);
        if self.check(&sub.endpoint).is_err() {
            tracing::warn!(endpoint = %host, "push endpoint is no longer allowed; skipped");
            return Sent::Failed;
        }
        let Ok(url) = reqwest::Url::parse(&sub.endpoint) else { return Sent::Failed };
        let (Some(ua_public), Some(auth)) = (decode(&sub.p256dh), decode(&sub.auth)) else { return Sent::Failed };
        let body = match encrypt(&ua_public, &auth, bytes) {
            Ok(b) => b,
            Err(e) => {
                tracing::warn!(endpoint = %host, error = e, "could not encrypt a push");
                return Sent::Failed;
            }
        };
        let Some(authorization) = vapid.authorization(&url, Utc::now()) else { return Sent::Failed };
        let res = self
            .client
            .post(url)
            .header(reqwest::header::AUTHORIZATION, authorization)
            .header(reqwest::header::CONTENT_ENCODING, "aes128gcm")
            .header(reqwest::header::CONTENT_TYPE, "application/octet-stream")
            .header("TTL", payload.ttl().to_string())
            .header("Urgency", payload.urgency())
            .header("Topic", topic)
            .body(body)
            .send()
            .await;
        match res {
            Ok(r) if r.status().is_success() => Sent::Ok,
            Ok(r) if r.status() == reqwest::StatusCode::NOT_FOUND || r.status() == reqwest::StatusCode::GONE => {
                tracing::info!(endpoint = %host, status = %r.status(), "push subscription expired; removed");
                Sent::Gone
            }
            Ok(r) => {
                tracing::warn!(endpoint = %host, status = %r.status(), "push service refused the notification");
                Sent::Failed
            }
            // reqwest errors can print the full URL, so only the kind is logged
            Err(e) => {
                tracing::warn!(endpoint = %host, timeout = e.is_timeout(), connect = e.is_connect(), "push service could not be reached");
                Sent::Failed
            }
        }
    }
}

/// RFC 8030 `Topic`: a newer push with the same topic replaces one still waiting on the push
/// service. At most 32 base64url characters, so the tag is hashed.
fn topic(tag: &str) -> String {
    use sha2::Digest;
    B64.encode(sha2::Sha256::digest(tag.as_bytes()))[..32].to_string()
}

// Routes (all person-only, see `auth::person_only`)

#[derive(Debug, Deserialize)]
pub struct SubscribeKeys {
    pub p256dh: String,
    pub auth: String,
}

/// `PushSubscription.toJSON()` plus an optional label; `expirationTime` is ignored.
#[derive(Debug, Deserialize)]
pub struct SubscribeBody {
    pub endpoint: String,
    pub keys: SubscribeKeys,
    #[serde(default)]
    pub label: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct UnsubscribeBody {
    pub endpoint: String,
}

fn bad_request(msg: &str) -> axum::response::Response {
    use axum::response::IntoResponse;
    (axum::http::StatusCode::BAD_REQUEST, axum::Json(serde_json::json!({ "error": msg }))).into_response()
}

/// `GET /api/push/key`: `{public_key}`, the browser's `applicationServerKey`.
pub async fn key(axum::extract::State(state): axum::extract::State<super::AppState>) -> axum::response::Response {
    use axum::response::IntoResponse;
    match state.push.vapid() {
        Some(v) => axum::Json(serde_json::json!({ "public_key": v.public_key() })).into_response(),
        None => (axum::http::StatusCode::INTERNAL_SERVER_ERROR, axum::Json(serde_json::json!({ "error": "push key unavailable" }))).into_response(),
    }
}

/// `POST /api/push/subscribe`: stores (or refreshes) this browser's subscription, linked to its
/// sign-in session when it has one. 204; 400 with `{error}` for a bad endpoint or keys.
pub async fn subscribe(
    axum::extract::State(state): axum::extract::State<super::AppState>,
    headers: axum::http::HeaderMap,
    body: Result<axum::Json<SubscribeBody>, axum::extract::rejection::JsonRejection>,
) -> axum::response::Response {
    use axum::response::IntoResponse;
    let Ok(axum::Json(body)) = body else {
        return bad_request("expected {endpoint, keys: {p256dh, auth}}");
    };
    let session = super::auth::cookie_token(&headers).and_then(|c| state.api_token.signin.session_id(c));
    let label = body
        .label
        .or_else(|| headers.get(axum::http::header::USER_AGENT).and_then(|v| v.to_str().ok()).map(super::signin::browser_label))
        .unwrap_or_default();
    match state.push.subscribe(&body.endpoint, &body.keys.p256dh, &body.keys.auth, &label, session) {
        Ok(()) => axum::http::StatusCode::NO_CONTENT.into_response(),
        Err(Invalid(msg)) => bad_request(msg),
    }
}

/// `POST /api/push/unsubscribe` `{endpoint}`: 204, also when it wasn't subscribed.
pub async fn unsubscribe(
    axum::extract::State(state): axum::extract::State<super::AppState>,
    body: Result<axum::Json<UnsubscribeBody>, axum::extract::rejection::JsonRejection>,
) -> axum::response::Response {
    use axum::response::IntoResponse;
    let Ok(axum::Json(body)) = body else {
        return bad_request("expected {endpoint}");
    };
    state.push.unsubscribe(&body.endpoint);
    axum::http::StatusCode::NO_CONTENT.into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn b64(s: &str) -> Vec<u8> {
        B64.decode(s.split_whitespace().collect::<String>()).unwrap()
    }

    /// RFC 8291 section 5 and appendix A.
    #[test]
    fn encrypts_the_rfc_8291_example() {
        let ecdh = b64("kyrL1jIIOHEzg3sM2ZWRHDRB62YACZhhSlknJ672kSs");
        let auth = b64("BTBZMqHH6r4Tts7J_aSIgg");
        let ua_public = b64("BCVxsr7N_eNgVRqvHtD0zTZsEc6-VV-JvLexhqUzORcx aOzi6-AYWXvTBHm4bjyPjs7Vd8pZGH6SRpkNtoIAiw4");
        let as_public = b64("BP4z9KsN6nGRTbVYI_c7VJSPQTBtkgcy27mlmlMoZIIg Dll6e3vCYLocInmYWAmS6TlzAC8wEqKK6PBru3jl7A8");
        let salt: [u8; 16] = b64("DGv6ra1nlYgDCS1FRnbzlw").try_into().unwrap();
        let plaintext = b"When I grow up, I want to be a watermelon";
        let out = encrypt_with_secret(&ecdh, &auth, &ua_public, &as_public, &salt, plaintext).unwrap();
        let header = b64("DGv6ra1nlYgDCS1FRnbzlwAAEABBBP4z 9KsN6nGRTbVYI_c7VJSPQTBtkgcy27ml mlMoZIIgDll6e3vCYLocInmYWAmS6Tlz AC8wEqKK6PBru3jl7A8");
        let ciphertext = b64("8pfeW0KbunFT06SuDKoJH9Ql87S1QUrd irN6GcG7sFz1y1sqLgVi1VhjVkHsUoEs bI_0LpXMuGvnzQ");
        assert_eq!(&out[..86], &header[..]);
        assert_eq!(&out[86..], &ciphertext[..]);
    }

    /// A fresh key pair and salt, decrypted the way a browser does it.
    #[test]
    fn a_browser_can_decrypt_a_push() {
        let rng = ring::rand::SystemRandom::new();
        let ua_private = agreement::EphemeralPrivateKey::generate(&agreement::ECDH_P256, &rng).unwrap();
        let ua_public = ua_private.compute_public_key().unwrap().as_ref().to_vec();
        let auth = [7u8; 16];
        let out = encrypt(&ua_public, &auth, b"{\"title\":\"hi\"}").unwrap();
        assert_eq!(decrypt_for_tests(ua_private, &ua_public, &auth, &out), b"{\"title\":\"hi\"}");
    }

    fn decrypt_for_tests(ua_private: agreement::EphemeralPrivateKey, ua_public: &[u8], auth: &[u8], body: &[u8]) -> Vec<u8> {
        let peer = agreement::UnparsedPublicKey::new(&agreement::ECDH_P256, &body[21..86]);
        let ecdh = agreement::agree_ephemeral(ua_private, &peer, |s| s.to_vec()).unwrap();
        decrypt_with_secret(&ecdh, auth, ua_public, body).unwrap()
    }

    #[test]
    fn vapid_token_is_an_es256_jwt_for_the_push_origin() {
        let (v, _) = Vapid::generate().unwrap();
        let url = reqwest::Url::parse("https://fcm.googleapis.com/fcm/send/abc").unwrap();
        let now = Utc::now();
        let h = v.authorization(&url, now).unwrap();
        let (t, k) = h.strip_prefix("vapid t=").unwrap().split_once(", k=").unwrap();
        assert_eq!(k, v.public_key());
        assert_eq!(b64(k).len(), 65);
        let parts: Vec<&str> = t.split('.').collect();
        assert_eq!(parts.len(), 3);
        let claims: serde_json::Value = serde_json::from_slice(&b64(parts[1])).unwrap();
        assert_eq!(claims["aud"], "https://fcm.googleapis.com");
        assert_eq!(claims["sub"], VAPID_SUBJECT);
        assert_eq!(claims["exp"], now.timestamp() + JWT_SECS);
        let public = signature::UnparsedPublicKey::new(&signature::ECDSA_P256_SHA256_FIXED, b64(k));
        public
            .verify(format!("{}.{}", parts[0], parts[1]).as_bytes(), &b64(parts[2]))
            .expect("signature verifies");
    }

    #[test]
    fn endpoints_must_be_public_https() {
        assert!(check_endpoint("https://fcm.googleapis.com/fcm/send/abc").is_ok());
        assert!(check_endpoint("https://web.push.apple.com/QX").is_ok());
        for bad in [
            "http://fcm.googleapis.com/x",
            "https://localhost/x",
            "https://foo.localhost/x",
            "https://intranet/x",
            "https://127.0.0.1/x",
            "https://10.0.0.5/x",
            "https://192.168.1.1/x",
            "https://169.254.169.254/latest",
            "https://100.64.0.1/x",
            "https://[::1]/x",
            "https://[fd00::1]/x",
            "https://[::ffff:10.0.0.1]/x",
            "https://user:pw@fcm.googleapis.com/x",
            "file:///etc/passwd",
            "not a url",
        ] {
            assert!(check_endpoint(bad).is_err(), "{bad} should be refused");
        }
        assert!(check_endpoint(&format!("https://fcm.googleapis.com/{}", "a".repeat(3000))).is_err());
        assert!(is_public_ip("8.8.8.8".parse().unwrap()));
        assert!(is_public_ip("2607:f8b0::1".parse().unwrap()));
        assert!(!is_public_ip("0.1.2.3".parse().unwrap()));
        assert!(!is_public_ip("64:ff9b::a00:1".parse().unwrap()));
    }

    /// A name that resolves into this machine or its network is never connected to.
    #[tokio::test]
    async fn the_resolver_drops_private_addresses() {
        use reqwest::dns::Resolve;
        let err = PublicOnly.resolve("localhost".parse().unwrap()).await.err().expect("refused");
        assert!(err.to_string().contains("public address"));
        // And the real client fails before sending anything
        let push = Push::build(None, Vec::new(), false);
        let res = push.client.post("https://localhost:9/x").send().await;
        assert!(res.is_err());
    }

    #[test]
    fn keys_are_checked() {
        let p = B64.encode([4u8; 65]);
        let a = B64.encode([1u8; 16]);
        assert!(check_keys(&p, &a).is_ok());
        assert!(check_keys(&format!("{p}="), &format!("{a}==")).is_ok());
        assert!(check_keys(&B64.encode([4u8; 33]), &a).is_err());
        assert!(check_keys(&p, &B64.encode([1u8; 8])).is_err());
        assert!(check_keys("!!", &a).is_err());
    }

    #[test]
    fn subscriptions_upsert_cap_and_persist_privately() {
        let dir = tempfile::tempdir().unwrap();
        let push = Push::load(dir.path());
        let p = B64.encode([4u8; 65]);
        let a = B64.encode([1u8; 16]);
        for i in 0..MAX_SUBSCRIPTIONS + 3 {
            push.subscribe(&format!("https://fcm.googleapis.com/fcm/send/{i}"), &p, &a, "Chrome", None).unwrap();
        }
        assert_eq!(push.subscriptions().len(), MAX_SUBSCRIPTIONS);
        assert!(!push.subscriptions().iter().any(|s| s.endpoint.ends_with("/send/0")));
        // Same endpoint again replaces, with the new session
        push.subscribe("https://fcm.googleapis.com/fcm/send/5", &p, &a, "Pixel", Some("s1".into())).unwrap();
        assert_eq!(push.subscriptions().len(), MAX_SUBSCRIPTIONS);
        assert_eq!(Push::load(dir.path()).subscriptions().len(), MAX_SUBSCRIPTIONS);
        assert!(push.drop_session("s1"));
        assert!(!push.unsubscribe("https://fcm.googleapis.com/fcm/send/5"));
        assert!(push.unsubscribe("https://fcm.googleapis.com/fcm/send/6"));
        assert_eq!(Push::load(dir.path()).subscriptions().len(), MAX_SUBSCRIPTIONS - 2);
        assert_eq!(push.subscribe("https://10.0.0.1/x", &p, &a, "", None), Err(Invalid("endpoint must be on the public internet")));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(dir.path().join(SUBS_FILE)).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o600);
        }
    }

    #[test]
    fn vapid_key_is_made_once_and_kept() {
        let dir = tempfile::tempdir().unwrap();
        let key = Push::load(dir.path()).vapid().unwrap().public_key();
        assert_eq!(Push::load(dir.path()).vapid().unwrap().public_key(), key);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(dir.path().join(KEY_FILE)).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o600);
        }
    }

    #[test]
    fn payload_shape_ttl_and_urgency() {
        let p = Payload::new("approval_needed", "Ada needs approval", &"x".repeat(400), Some("sp 1".into()), Some("c1".into()), "k1".into());
        assert_eq!(p.url, "/?space=sp%201&channel=c1");
        assert_eq!(p.tag, "kwnt-sp 1-approval_needed-c1");
        assert_eq!(p.body.chars().count(), PUSH_BODY_CHARS);
        assert_eq!((p.ttl(), p.urgency()), (86_400, "high"));
        let p = Payload::new("turn_finished", "Laya finished", "done", None, None, "k1".into());
        assert_eq!(p.url, "/?kwant=k1");
        assert_eq!(p.tag, "kwnt-none-turn_finished-k1");
        assert_eq!((p.ttl(), p.urgency()), (3_600, "normal"));
        let huge = Payload { title: "t".repeat(5000), ..p };
        assert!(huge.to_bytes().len() <= MAX_PAYLOAD_BYTES);
        assert_eq!(topic("kwnt-x").len(), 32);
    }
}
