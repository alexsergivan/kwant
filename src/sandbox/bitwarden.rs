//! Saved passwords in the person's Bitwarden vault (bitwarden.com, bitwarden.eu, or a
//! self-hosted Bitwarden or Vaultwarden), through Bitwarden's HTTP API with its crypto done
//! here, in kwnt's own process.
//!
//! Why not the `bw` CLI: it keeps its unlock secret (`BW_SESSION`) in the environment or on the
//! command line of every `bw` process, both readable through `/proc` by any process of the same
//! user, and the protected user key in its `data.json`; with both a bot reads every password.
//! `bw serve` and `rbw`'s agent hand passwords to any same-user client of a socket.
//!
//! Here nothing that unlocks the vault leaves kwnt's memory: the person sends the master
//! password once (a person-only route), kwnt derives the keys and keeps the user key and
//! tokens in memory only (kwnt is non-dumpable, see [`crate::harden_process`]), and forgets them
//! on lock or restart. Only non-secret settings (server, email, device id, API client id) are
//! saved, in `<data_dir>/kwnt-vault/bitwarden.json`.
//!
//! kwnt only uses login items in a folder named `kwnt` that carry a hidden `kwnt-owner` field
//! naming the team, channel or bot. Other items' passwords are never decrypted. Organization
//! items are ignored.

use super::credentials::{normalize_origin, private_dir, write_private, Credential, CredentialMeta, CredentialStore, StoreError};
use aes::cipher::{block_padding::Pkcs7, BlockDecryptMut, BlockEncryptMut, KeyIvInit};
use base64::{
    engine::general_purpose::{STANDARD as B64, URL_SAFE_NO_PAD},
    Engine,
};
use ring::rand::{SecureRandom, SystemRandom};
use ring::{hkdf, hmac, pbkdf2};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::num::NonZeroU32;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};
use zeroize::{Zeroize, Zeroizing};

pub const CONFIG_FILE: &str = "bitwarden.json";
pub const FOLDER_NAME: &str = "kwnt";
pub const OWNER_FIELD: &str = "kwnt-owner";
const CLIENT_NAME: &str = "cli";
const CLIENT_VERSION: &str = "2026.10.0";
#[cfg(target_os = "macos")]
const DEVICE_TYPE: &str = "24"; // MacOsCLI
#[cfg(not(target_os = "macos"))]
const DEVICE_TYPE: &str = "25"; // LinuxCLI
/// The cached vault is fetched again when older than this.
const RESYNC: Duration = Duration::from_secs(300);
/// Wrong master passwords in a row (on reveal or forget) before kwnt locks the vault.
pub const MAX_WRONG_PASSWORDS: u32 = 5;

#[derive(Debug, thiserror::Error)]
pub enum UnlockError {
    #[error("wrong email or master password")]
    Wrong,
    #[error("Bitwarden asks for a two-step login code")]
    TwoFactor(Vec<String>),
    #[error("Bitwarden emailed a code to confirm this new device")]
    NewDeviceOtp,
    #[error("another Bitwarden account is set up; forget it first")]
    OtherAccount,
    #[error("{0}")]
    Invalid(String),
    #[error("Bitwarden could not be reached: {0}")]
    Unreachable(String),
    #[error("Bitwarden refused: {0}")]
    Server(String),
    #[error("this vault uses something kwnt does not support yet: {0}")]
    Unsupported(String),
}

#[derive(Debug)]
struct CryptoFail;

impl From<CryptoFail> for StoreError {
    fn from(_: CryptoFail) -> Self {
        StoreError::Crypto
    }
}

struct Len(usize);

impl hkdf::KeyType for Len {
    fn len(&self) -> usize {
        self.0
    }
}

/// A Bitwarden symmetric key: 32 bytes for AES-256-CBC, then 32 for HMAC-SHA256.
pub struct SymKey(Zeroizing<[u8; 64]>);

impl SymKey {
    pub fn from_slice(bytes: &[u8]) -> Option<Self> {
        let mut key = Zeroizing::new([0u8; 64]);
        (bytes.len() == 64).then(|| {
            key.copy_from_slice(bytes);
            SymKey(key)
        })
    }

    /// The key a master key unlocks the account key with (HKDF-Expand "enc" and "mac").
    pub fn stretch(master: &[u8; 32]) -> Self {
        let prk = hkdf::Prk::new_less_safe(hkdf::HKDF_SHA256, master);
        let mut key = Zeroizing::new([0u8; 64]);
        for (info, out) in [(b"enc", 0..32), (b"mac", 32..64)] {
            let info: [&[u8]; 1] = [info];
            prk.expand(&info, Len(32))
                .and_then(|okm| okm.fill(&mut key[out]))
                .expect("32 bytes is a valid HKDF-SHA256 length");
        }
        SymKey(key)
    }

    fn enc(&self) -> &[u8] {
        &self.0[..32]
    }

    fn mac(&self) -> hmac::Key {
        hmac::Key::new(hmac::HMAC_SHA256, &self.0[32..])
    }

    /// Opens a type-2 EncString (`2.iv|data|mac`, AES-256-CBC + HMAC-SHA256).
    pub fn decrypt(&self, enc_string: &str) -> Result<Zeroizing<Vec<u8>>, StoreError> {
        let rest = enc_string.strip_prefix("2.").ok_or(StoreError::Crypto)?;
        let mut parts = rest.split('|');
        let mut next = || parts.next().and_then(|p| B64.decode(p).ok()).ok_or(CryptoFail);
        let (iv, data, tag) = (next()?, next()?, next()?);
        if iv.len() != 16 {
            return Err(StoreError::Crypto);
        }
        let mut signed = iv.clone();
        signed.extend_from_slice(&data);
        hmac::verify(&self.mac(), &signed, &tag).map_err(|_| CryptoFail)?;
        let dec = cbc::Decryptor::<aes::Aes256>::new_from_slices(self.enc(), &iv).map_err(|_| CryptoFail)?;
        dec.decrypt_padded_vec_mut::<Pkcs7>(&data).map(Zeroizing::new).map_err(|_| StoreError::Crypto)
    }

    pub fn decrypt_string(&self, enc_string: &str) -> Result<Zeroizing<String>, StoreError> {
        let bytes = self.decrypt(enc_string)?;
        std::str::from_utf8(&bytes).map(|s| Zeroizing::new(s.to_string())).map_err(|_| StoreError::Crypto)
    }

    pub fn encrypt(&self, plain: &[u8], rng: &SystemRandom) -> Result<String, StoreError> {
        let mut iv = [0u8; 16];
        rng.fill(&mut iv).map_err(|_| CryptoFail)?;
        let enc = cbc::Encryptor::<aes::Aes256>::new_from_slices(self.enc(), &iv).map_err(|_| CryptoFail)?;
        let data = enc.encrypt_padded_vec_mut::<Pkcs7>(plain);
        let mut signed = iv.to_vec();
        signed.extend_from_slice(&data);
        let tag = hmac::sign(&self.mac(), &signed);
        Ok(format!("2.{}|{}|{}", B64.encode(iv), B64.encode(&data), B64.encode(tag.as_ref())))
    }
}

/// How an account turns its master password into the master key.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kdf {
    Pbkdf2 { iterations: u32 },
    Argon2id { iterations: u32, memory_mib: u32, parallelism: u32 },
}

impl Kdf {
    /// From a prelogin or token response, within limits that keep a hostile server from making
    /// kwnt spend minutes or gigabytes on one unlock.
    pub fn from_json(v: &Value) -> Result<Self, UnlockError> {
        let num = |name: &str| field(v, name).and_then(Value::as_u64).map(|n| n.min(u32::MAX as u64) as u32);
        let iterations = num("kdfIterations").ok_or_else(|| UnlockError::Server("no KDF settings".into()))?;
        let kdf = match num("kdf").unwrap_or(0) {
            0 => Kdf::Pbkdf2 { iterations },
            1 => Kdf::Argon2id {
                iterations,
                memory_mib: num("kdfMemory").unwrap_or(64),
                parallelism: num("kdfParallelism").unwrap_or(4),
            },
            other => return Err(UnlockError::Unsupported(format!("KDF type {other}"))),
        };
        let ok = match kdf {
            Kdf::Pbkdf2 { iterations } => (5_000..=10_000_000).contains(&iterations),
            Kdf::Argon2id { iterations, memory_mib, parallelism } => {
                (1..=20).contains(&iterations) && (15..=1024).contains(&memory_mib) && (1..=16).contains(&parallelism)
            }
        };
        if ok {
            Ok(kdf)
        } else {
            Err(UnlockError::Unsupported(format!("KDF settings out of range: {kdf:?}")))
        }
    }
}

/// The account's master key from its master password and email.
pub fn master_key(password: &str, email: &str, kdf: Kdf) -> Result<Zeroizing<[u8; 32]>, UnlockError> {
    let salt = email.trim().to_lowercase();
    let mut key = Zeroizing::new([0u8; 32]);
    match kdf {
        Kdf::Pbkdf2 { iterations } => {
            let n = NonZeroU32::new(iterations).ok_or_else(|| UnlockError::Unsupported("0 iterations".into()))?;
            pbkdf2::derive(pbkdf2::PBKDF2_HMAC_SHA256, n, salt.as_bytes(), password.as_bytes(), &mut key[..]);
        }
        Kdf::Argon2id { iterations, memory_mib, parallelism } => {
            use sha2::{Digest, Sha256};
            let salt = Sha256::digest(salt.as_bytes());
            let params = argon2::Params::new(memory_mib * 1024, iterations, parallelism, Some(32))
                .map_err(|e| UnlockError::Unsupported(e.to_string()))?;
            argon2::Argon2::new(argon2::Algorithm::Argon2id, argon2::Version::V0x13, params)
                .hash_password_into(password.as_bytes(), &salt, &mut key[..])
                .map_err(|e| UnlockError::Unsupported(e.to_string()))?;
        }
    }
    Ok(key)
}

/// What the server checks at login: PBKDF2(master key, salt = master password, 1 round).
pub fn auth_hash(master: &[u8; 32], password: &str) -> Zeroizing<String> {
    let mut out = Zeroizing::new([0u8; 32]);
    pbkdf2::derive(pbkdf2::PBKDF2_HMAC_SHA256, NonZeroU32::MIN, password.as_bytes(), master, &mut out[..]);
    Zeroizing::new(B64.encode(&out[..]))
}

/// Compares two secrets in time that depends only on their lengths.
fn same_secret(a: &[u8], b: &[u8]) -> bool {
    a.len() == b.len() && a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

/// A JSON member by name, ignoring case (Bitwarden answers in camelCase, older servers in
/// PascalCase).
fn field<'a>(v: &'a Value, name: &str) -> Option<&'a Value> {
    let obj = v.as_object()?;
    obj.get(name)
        .or_else(|| obj.iter().find(|(k, _)| k.eq_ignore_ascii_case(name)).map(|(_, v)| v))
        .filter(|v| !v.is_null())
}

fn field_str<'a>(v: &'a Value, name: &str) -> Option<&'a str> {
    field(v, name).and_then(Value::as_str)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Endpoints {
    pub api: String,
    pub identity: String,
}

/// The server's base address as saved, and its API and identity endpoints. bitwarden.com and
/// bitwarden.eu use their own API hosts; anything else is self-hosted (`<base>/api`,
/// `<base>/identity`). Plain http only for this machine.
pub fn endpoints(server: &str) -> Result<(String, Endpoints), UnlockError> {
    let raw = server.trim();
    let raw = if raw.is_empty() { "https://vault.bitwarden.com" } else { raw };
    let with_scheme = if raw.contains("://") { raw.to_string() } else { format!("https://{raw}") };
    let bad = || UnlockError::Invalid("the Bitwarden server must be an https address".into());
    let url = url::Url::parse(&with_scheme).map_err(|_| bad())?;
    let host = url.host_str().ok_or_else(bad)?.to_ascii_lowercase();
    let loopback = host == "localhost" || host.parse::<std::net::IpAddr>().is_ok_and(|ip| ip.is_loopback())
        || host.trim_matches(['[', ']']).parse::<std::net::IpAddr>().is_ok_and(|ip| ip.is_loopback());
    match url.scheme() {
        "https" => {}
        "http" if loopback => {}
        _ => return Err(bad()),
    }
    if url.query().is_some() || url.fragment().is_some() || !url.username().is_empty() || url.password().is_some() {
        return Err(bad());
    }
    for cloud in ["bitwarden.com", "bitwarden.eu"] {
        if host == cloud || host == format!("vault.{cloud}") || host == format!("api.{cloud}") || host == format!("identity.{cloud}") {
            return Ok((
                format!("https://vault.{cloud}"),
                Endpoints { api: format!("https://api.{cloud}"), identity: format!("https://identity.{cloud}") },
            ));
        }
    }
    let base = url.as_str().trim_end_matches('/').to_string();
    Ok((base.clone(), Endpoints { api: format!("{base}/api"), identity: format!("{base}/identity") }))
}

/// The settings kwnt keeps on disk. Nothing here unlocks anything.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct BitwardenConfig {
    pub server_url: String,
    pub email: String,
    /// The personal API key's client id (`user.…`), when the person logs in with it.
    #[serde(default)]
    pub client_id: Option<String>,
    pub device_id: String,
}

/// The person's unlock request. Not `Debug`: it holds the master password.
#[derive(Deserialize, Default)]
pub struct UnlockRequest {
    #[serde(default)]
    pub server_url: Option<String>,
    #[serde(default)]
    pub email: Option<String>,
    pub master_password: String,
    #[serde(default)]
    pub client_id: Option<String>,
    #[serde(default)]
    pub client_secret: Option<String>,
    #[serde(default)]
    pub two_factor_provider: Option<u32>,
    #[serde(default)]
    pub two_factor_token: Option<String>,
    #[serde(default)]
    pub new_device_otp: Option<String>,
}

impl Drop for UnlockRequest {
    fn drop(&mut self) {
        self.master_password.zeroize();
        if let Some(s) = self.client_secret.as_mut() {
            s.zeroize();
        }
    }
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct BitwardenStatus {
    pub server_url: String,
    pub email: String,
    /// `"password"` or `"api_key"`.
    pub login: &'static str,
    pub client_id: Option<String>,
    pub unlocked: bool,
    pub unlocked_at: Option<String>,
    pub item_count: Option<usize>,
    pub last_sync: Option<String>,
}

/// One kwnt login item from the vault, decrypted. Its password is wiped when dropped.
struct Item {
    owner: String,
    cred: Credential,
    /// The item's own key as stored (encrypted with the user key), if it has one.
    key_enc: Option<String>,
    key: Option<SymKey>,
}

impl Drop for Item {
    fn drop(&mut self) {
        self.cred.password.zeroize();
    }
}

struct Session {
    cfg: BitwardenConfig,
    ep: Endpoints,
    access: Zeroizing<String>,
    refresh: Option<Zeroizing<String>>,
    client_secret: Option<Zeroizing<String>>,
    expires: Instant,
    user_key: SymKey,
    kdf: Kdf,
    auth_hash: Zeroizing<String>,
    folder_id: Option<String>,
    /// The account's id, which new items name as `encryptedFor`.
    user_id: Option<String>,
    items: Vec<Item>,
    synced: Instant,
    synced_at: String,
    unlocked_at: String,
    wrong_passwords: u32,
}

pub struct BitwardenStore {
    config_path: PathBuf,
    config: std::sync::RwLock<Option<BitwardenConfig>>,
    session: tokio::sync::Mutex<Option<Session>>,
    http: reqwest::Client,
    rng: SystemRandom,
}

impl std::fmt::Debug for BitwardenStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BitwardenStore").field("config", &self.config()).finish_non_exhaustive()
    }
}

/// Errors from an authenticated API call.
enum ApiError {
    /// The session is no longer valid: lock.
    Auth,
    Other(String),
}

impl From<ApiError> for StoreError {
    fn from(e: ApiError) -> Self {
        match e {
            ApiError::Auth => StoreError::Locked,
            ApiError::Other(m) => StoreError::Remote(m),
        }
    }
}

fn short(s: &str) -> String {
    s.chars().take(200).collect()
}

impl BitwardenStore {
    /// The store with its saved settings from `vault_dir`, locked.
    pub fn open(vault_dir: &Path) -> Self {
        let config_path = vault_dir.join(CONFIG_FILE);
        let config = std::fs::read(&config_path).ok().and_then(|b| serde_json::from_slice(&b).ok());
        Self {
            config_path,
            config: std::sync::RwLock::new(config),
            session: tokio::sync::Mutex::new(None),
            http: reqwest::Client::builder()
                .timeout(Duration::from_secs(30))
                .redirect(reqwest::redirect::Policy::none())
                .build()
                .unwrap_or_default(),
            rng: SystemRandom::new(),
        }
    }

    pub fn config(&self) -> Option<BitwardenConfig> {
        self.config.read().unwrap_or_else(|e| e.into_inner()).clone()
    }

    pub fn is_configured(&self) -> bool {
        self.config().is_some()
    }

    pub async fn is_unlocked(&self) -> bool {
        self.session.lock().await.is_some()
    }

    pub async fn status(&self) -> Option<BitwardenStatus> {
        let cfg = self.config()?;
        let session = self.session.lock().await;
        Some(BitwardenStatus {
            server_url: cfg.server_url.clone(),
            email: cfg.email.clone(),
            login: if cfg.client_id.is_some() { "api_key" } else { "password" },
            client_id: cfg.client_id.clone(),
            unlocked: session.is_some(),
            unlocked_at: session.as_ref().map(|s| s.unlocked_at.clone()),
            item_count: session.as_ref().map(|s| s.items.len()),
            last_sync: session.as_ref().map(|s| s.synced_at.clone()),
        })
    }

    /// Forgets the unlocked vault (keys, tokens, decrypted items). Saved settings stay.
    pub async fn lock(&self) {
        *self.session.lock().await = None;
    }

    /// Logs in to Bitwarden and unlocks the vault. The first unlock sets the account up; after
    /// that only the same server and email are accepted (see [`Self::forget`]).
    pub async fn unlock(&self, req: UnlockRequest) -> Result<BitwardenStatus, UnlockError> {
        let existing = self.config();
        let server = req.server_url.clone().filter(|s| !s.trim().is_empty())
            .or_else(|| existing.as_ref().map(|c| c.server_url.clone()))
            .unwrap_or_default();
        let (base, ep) = endpoints(&server)?;
        let email = req.email.clone().filter(|s| !s.trim().is_empty())
            .or_else(|| existing.as_ref().map(|c| c.email.clone()))
            .unwrap_or_default()
            .trim()
            .to_lowercase();
        if !email.contains('@') || email.len() > 320 {
            return Err(UnlockError::Invalid("the Bitwarden account email is missing".into()));
        }
        if req.master_password.is_empty() {
            return Err(UnlockError::Invalid("the master password is missing".into()));
        }
        if let Some(cfg) = &existing {
            if cfg.server_url != base || cfg.email != email {
                return Err(UnlockError::OtherAccount);
            }
        }
        let client_id = req.client_id.clone().map(|c| c.trim().to_string()).filter(|c| !c.is_empty())
            .or_else(|| existing.as_ref().and_then(|c| c.client_id.clone()));
        let client_secret = req.client_secret.clone().filter(|c| !c.trim().is_empty()).map(Zeroizing::new);
        if client_id.is_some() && client_secret.is_none() {
            return Err(UnlockError::Invalid("the API key's client secret is missing".into()));
        }
        let cfg = BitwardenConfig {
            server_url: base,
            email: email.clone(),
            client_id: client_id.clone(),
            device_id: existing.as_ref().map(|c| c.device_id.clone()).unwrap_or_else(|| uuid::Uuid::new_v4().to_string()),
        };

        let kdf = self.prelogin(&ep, &email).await?;
        let password = Zeroizing::new(req.master_password.clone());
        let (mk, hash) = {
            let (password, email) = (password.clone(), email.clone());
            tokio::task::spawn_blocking(move || {
                let mk = master_key(&password, &email, kdf)?;
                let hash = auth_hash(&mk, &password);
                Ok::<_, UnlockError>((mk, hash))
            })
            .await
            .map_err(|e| UnlockError::Server(e.to_string()))??
        };

        let mut form: Vec<(&str, String)> = self.device_form(&cfg);
        match (&client_id, &client_secret) {
            (Some(id), Some(secret)) => {
                form.extend([
                    ("grant_type", "client_credentials".into()),
                    ("client_id", id.clone()),
                    ("client_secret", secret.to_string()),
                    ("scope", "api".into()),
                ]);
            }
            _ => {
                form.extend([
                    ("grant_type", "password".into()),
                    ("username", email.clone()),
                    ("password", hash.to_string()),
                    ("scope", "api offline_access".into()),
                    ("client_id", CLIENT_NAME.into()),
                ]);
                if let (Some(p), Some(t)) = (req.two_factor_provider, req.two_factor_token.as_deref()) {
                    form.extend([
                        ("twoFactorProvider", p.to_string()),
                        ("twoFactorToken", t.trim().to_string()),
                        ("twoFactorRemember", "0".into()),
                    ]);
                }
                if let Some(otp) = req.new_device_otp.as_deref().filter(|o| !o.trim().is_empty()) {
                    form.push(("newDeviceOtp", otp.trim().to_string()));
                }
            }
        }
        let token = self.token_request(&ep, &email, form).await?;

        let protected = field_str(&token, "Key").ok_or_else(|| UnlockError::Unsupported("the account has no key".into()))?;
        // With an API key the server never saw the master password: a MAC failure here is a wrong one
        let user_key = SymKey::stretch(&mk)
            .decrypt(protected)
            .map_err(|_| if protected.starts_with("2.") { UnlockError::Wrong } else { UnlockError::Unsupported("account key type".into()) })?;
        let user_key = SymKey::from_slice(&user_key).ok_or_else(|| UnlockError::Unsupported("account key size".into()))?;
        let access = field_str(&token, "access_token").ok_or_else(|| UnlockError::Server("no access token".into()))?;
        let expires_in = field(&token, "expires_in").and_then(Value::as_u64).unwrap_or(3600);
        let now = chrono::Utc::now().to_rfc3339();
        let mut session = Session {
            cfg: cfg.clone(),
            ep,
            access: Zeroizing::new(access.to_string()),
            refresh: field_str(&token, "refresh_token").map(|s| Zeroizing::new(s.to_string())),
            client_secret,
            expires: Instant::now() + Duration::from_secs(expires_in),
            user_key,
            kdf,
            auth_hash: hash,
            folder_id: None,
            user_id: None,
            items: Vec::new(),
            synced: Instant::now(),
            synced_at: now.clone(),
            unlocked_at: now,
            wrong_passwords: 0,
        };
        self.sync(&mut session).await.map_err(|e| match e {
            ApiError::Auth => UnlockError::Server("the new session was refused".into()),
            ApiError::Other(m) => UnlockError::Unreachable(m),
        })?;
        if existing.as_ref() != Some(&cfg) {
            self.save_config(&cfg).map_err(|e| UnlockError::Server(format!("could not save the settings: {e}")))?;
        }
        *self.session.lock().await = Some(session);
        tracing::info!(server = %cfg.server_url, "the person unlocked Bitwarden");
        Ok(self.status().await.expect("configured"))
    }

    /// Checks the master password against the unlocked vault. After
    /// [`MAX_WRONG_PASSWORDS`] wrong ones in a row the vault locks.
    pub async fn verify_master_password(&self, password: &str) -> Result<bool, StoreError> {
        let (email, kdf, expected) = {
            let guard = self.session.lock().await;
            let s = guard.as_ref().ok_or(StoreError::Locked)?;
            (s.cfg.email.clone(), s.kdf, s.auth_hash.clone())
        };
        let password = Zeroizing::new(password.to_string());
        let ok = tokio::task::spawn_blocking(move || {
            master_key(&password, &email, kdf)
                .map(|mk| same_secret(auth_hash(&mk, &password).as_bytes(), expected.as_bytes()))
                .unwrap_or(false)
        })
        .await
        .unwrap_or(false);
        let mut guard = self.session.lock().await;
        let s = guard.as_mut().ok_or(StoreError::Locked)?;
        if ok {
            s.wrong_passwords = 0;
            return Ok(true);
        }
        s.wrong_passwords += 1;
        if s.wrong_passwords >= MAX_WRONG_PASSWORDS {
            *guard = None;
            tracing::warn!("Bitwarden locked after {MAX_WRONG_PASSWORDS} wrong master passwords");
            return Err(StoreError::Locked);
        }
        Ok(false)
    }

    /// Stops using Bitwarden: forgets the session and the saved settings. The route checks the
    /// master password first ([`Self::verify_master_password`]), so whoever holds only the API
    /// token can't point kwnt at another vault. The items stay in the person's vault.
    pub async fn forget(&self) -> Result<(), StoreError> {
        let mut session = self.session.lock().await;
        match std::fs::remove_file(&self.config_path) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => return Err(e.into()),
            _ => {}
        }
        *session = None;
        *self.config.write().unwrap_or_else(|e| e.into_inner()) = None;
        tracing::info!("the person stopped using Bitwarden for saved passwords");
        Ok(())
    }

    fn save_config(&self, cfg: &BitwardenConfig) -> std::io::Result<()> {
        if let Some(dir) = self.config_path.parent() {
            private_dir(dir)?;
        }
        write_private(&self.config_path, &serde_json::to_vec_pretty(cfg).unwrap_or_default())?;
        *self.config.write().unwrap_or_else(|e| e.into_inner()) = Some(cfg.clone());
        Ok(())
    }

    fn device_form(&self, cfg: &BitwardenConfig) -> Vec<(&'static str, String)> {
        vec![
            ("deviceType", DEVICE_TYPE.into()),
            ("deviceIdentifier", cfg.device_id.clone()),
            ("deviceName", "kwnt".into()),
            ("devicePushToken", String::new()),
        ]
    }

    fn client_headers(rb: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
        rb.header("Device-Type", DEVICE_TYPE)
            .header("Bitwarden-Client-Name", CLIENT_NAME)
            .header("Bitwarden-Client-Version", CLIENT_VERSION)
    }

    async fn prelogin(&self, ep: &Endpoints, email: &str) -> Result<Kdf, UnlockError> {
        let mut last = String::new();
        for url in [format!("{}/accounts/prelogin", ep.identity), format!("{}/accounts/prelogin", ep.api)] {
            let resp = Self::client_headers(self.http.post(&url))
                .json(&json!({ "email": email }))
                .send()
                .await
                .map_err(|e| UnlockError::Unreachable(short(&e.without_url().to_string())))?;
            if resp.status().is_success() {
                let v: Value = resp.json().await.map_err(|_| UnlockError::Server("unreadable KDF settings".into()))?;
                return Kdf::from_json(&v);
            }
            last = resp.status().to_string();
        }
        Err(UnlockError::Server(format!("prelogin failed: {last}")))
    }

    async fn token_request(&self, ep: &Endpoints, email: &str, form: Vec<(&str, String)>) -> Result<Value, UnlockError> {
        let mut rb = Self::client_headers(self.http.post(format!("{}/connect/token", ep.identity))).form(&form);
        if form.iter().any(|(k, v)| *k == "grant_type" && v == "password") {
            rb = rb.header("Auth-Email", URL_SAFE_NO_PAD.encode(email));
        }
        drop(form);
        let resp = rb.send().await.map_err(|e| UnlockError::Unreachable(short(&e.without_url().to_string())))?;
        let status = resp.status();
        let body: Value = resp.json().await.unwrap_or(Value::Null);
        if status.is_success() {
            return Ok(body);
        }
        if let Some(p) = field(&body, "TwoFactorProviders2").and_then(Value::as_object) {
            return Err(UnlockError::TwoFactor(p.keys().cloned().collect()));
        }
        if let Some(p) = field(&body, "TwoFactorProviders").and_then(Value::as_array) {
            return Err(UnlockError::TwoFactor(p.iter().map(|v| v.as_str().map(str::to_string).unwrap_or_else(|| v.to_string())).collect()));
        }
        let message = [
            field_str(&body, "error_description"),
            field(&body, "ErrorModel").and_then(|m| field_str(m, "Message")),
            field_str(&body, "message"),
        ]
        .into_iter()
        .flatten()
        .find(|m| !m.trim().is_empty())
        .unwrap_or_default()
        .to_string();
        if message.to_lowercase().contains("new device") {
            return Err(UnlockError::NewDeviceOtp);
        }
        let error = field_str(&body, "error").unwrap_or_default();
        let lower = message.to_lowercase();
        if matches!(error, "invalid_grant" | "invalid_client")
            || status == reqwest::StatusCode::UNAUTHORIZED
            || lower.contains("password is incorrect")
            || lower.contains("invalid_username_or_password")
        {
            return Err(UnlockError::Wrong);
        }
        Err(UnlockError::Server(short(&format!("{status} {message}"))))
    }

    /// A fresh access token, from the refresh token or (API key login) the client secret.
    async fn renew(&self, s: &mut Session) -> Result<(), ApiError> {
        let mut form = self.device_form(&s.cfg);
        if let Some(refresh) = &s.refresh {
            form.extend([("grant_type", "refresh_token".into()), ("client_id", CLIENT_NAME.into()), ("refresh_token", refresh.to_string())]);
        } else if let (Some(id), Some(secret)) = (&s.cfg.client_id, &s.client_secret) {
            form.extend([
                ("grant_type", "client_credentials".into()),
                ("client_id", id.clone()),
                ("client_secret", secret.to_string()),
                ("scope", "api".into()),
            ]);
        } else {
            return Err(ApiError::Auth);
        }
        let token = self.token_request(&s.ep, &s.cfg.email, form).await.map_err(|e| match e {
            UnlockError::Unreachable(m) => ApiError::Other(m),
            _ => ApiError::Auth,
        })?;
        let access = field_str(&token, "access_token").ok_or(ApiError::Auth)?;
        s.access = Zeroizing::new(access.to_string());
        if let Some(r) = field_str(&token, "refresh_token") {
            s.refresh = Some(Zeroizing::new(r.to_string()));
        }
        s.expires = Instant::now() + Duration::from_secs(field(&token, "expires_in").and_then(Value::as_u64).unwrap_or(3600));
        Ok(())
    }

    async fn api(&self, s: &mut Session, method: reqwest::Method, path: &str, body: Option<&Value>) -> Result<Value, ApiError> {
        if Instant::now() + Duration::from_secs(60) >= s.expires {
            self.renew(s).await?;
        }
        for attempt in 0..2 {
            let mut rb = Self::client_headers(self.http.request(method.clone(), format!("{}{path}", s.ep.api)))
                .bearer_auth(s.access.as_str());
            if let Some(b) = body {
                rb = rb.json(b);
            }
            let resp = rb.send().await.map_err(|e| ApiError::Other(short(&e.without_url().to_string())))?;
            let status = resp.status();
            if status == reqwest::StatusCode::UNAUTHORIZED && attempt == 0 {
                self.renew(s).await?;
                continue;
            }
            if status == reqwest::StatusCode::UNAUTHORIZED {
                return Err(ApiError::Auth);
            }
            let v: Value = resp.json().await.unwrap_or(Value::Null);
            if !status.is_success() {
                let message = field(&v, "message").or_else(|| field(&v, "ErrorModel")).map(|m| m.to_string()).unwrap_or_default();
                return Err(ApiError::Other(short(&format!("{status} {message}"))));
            }
            return Ok(v);
        }
        Err(ApiError::Auth)
    }

    /// Fetches the vault and keeps kwnt's own items, decrypted, in the session.
    async fn sync(&self, s: &mut Session) -> Result<(), ApiError> {
        let v = self.api(s, reqwest::Method::GET, "/sync?excludeDomains=true", None).await?;
        let (folder_id, items) = parse_sync(&s.user_key, &v);
        s.folder_id = folder_id;
        s.user_id = field(&v, "profile").and_then(|p| field_str(p, "id")).map(str::to_string);
        s.items = items;
        s.synced = Instant::now();
        s.synced_at = chrono::Utc::now().to_rfc3339();
        Ok(())
    }

    /// The unlocked session, synced if stale. A refused session locks the vault.
    async fn fresh(&self) -> Result<tokio::sync::MutexGuard<'_, Option<Session>>, StoreError> {
        let mut guard = self.session.lock().await;
        let s = guard.as_mut().ok_or(StoreError::Locked)?;
        if s.synced.elapsed() >= RESYNC {
            match self.sync(s).await {
                Ok(()) => {}
                Err(ApiError::Auth) => {
                    *guard = None;
                    tracing::warn!("Bitwarden refused the session; the vault is locked");
                    return Err(StoreError::Locked);
                }
                // Offline: keep serving what was fetched
                Err(ApiError::Other(m)) => tracing::warn!(error = %m, "could not refresh the Bitwarden vault"),
            }
        }
        Ok(guard)
    }

    async fn ensure_folder(&self, s: &mut Session) -> Result<String, StoreError> {
        if let Some(id) = &s.folder_id {
            return Ok(id.clone());
        }
        let name = s.user_key.encrypt(FOLDER_NAME.as_bytes(), &self.rng)?;
        let v = self.api(s, reqwest::Method::POST, "/folders", Some(&json!({ "name": name }))).await?;
        let id = field_str(&v, "id").ok_or_else(|| StoreError::Remote("the new folder has no id".into()))?.to_string();
        s.folder_id = Some(id.clone());
        Ok(id)
    }

    #[allow(clippy::too_many_arguments)]
    fn cipher_body(&self, s: &Session, item_key: Option<(&SymKey, &str)>, folder: &str, owner: &str, origin: &str, username: &str, password: &str) -> Result<Value, StoreError> {
        let k = item_key.map(|(k, _)| k).unwrap_or(&s.user_key);
        let enc = |t: &str| k.encrypt(t.as_bytes(), &self.rng);
        let host = url::Url::parse(origin).ok().and_then(|u| u.host_str().map(str::to_string)).unwrap_or_else(|| origin.to_string());
        Ok(json!({
            "type": 1,
            "organizationId": null,
            "encryptedFor": s.user_id,
            "folderId": folder,
            "key": item_key.map(|(_, e)| e),
            "name": enc(&format!("kwnt · {host}"))?,
            "notes": null,
            "favorite": false,
            "reprompt": 0,
            "login": {
                "username": if username.is_empty() { Value::Null } else { Value::String(enc(username)?) },
                "password": enc(password)?,
                "uris": [{ "uri": enc(origin)?, "match": null }],
                "totp": null,
            },
            "fields": [{ "type": 1, "name": enc(OWNER_FIELD)?, "value": enc(owner)?, "linkedId": null }],
        }))
    }

    fn meta_of(s: &Session, owner: &str, pred: impl Fn(&Item) -> bool) -> Option<CredentialMeta> {
        s.items.iter().find(|i| i.owner == owner && pred(i)).map(|i| i.cred.meta.clone())
    }
}

/// kwnt's items from a `/sync` response: login items in the `kwnt` folder with a `kwnt-owner`
/// field. Items that fail to decrypt are skipped.
fn parse_sync(user_key: &SymKey, v: &Value) -> (Option<String>, Vec<Item>) {
    let folder_id = field(v, "folders").and_then(Value::as_array).and_then(|folders| {
        folders.iter().find_map(|f| {
            let name = user_key.decrypt_string(field_str(f, "name")?).ok()?;
            (name.as_str() == FOLDER_NAME).then(|| field_str(f, "id").map(str::to_string)).flatten()
        })
    });
    let Some(folder) = folder_id.clone() else { return (None, Vec::new()) };
    let mut items = Vec::new();
    for c in field(v, "ciphers").and_then(Value::as_array).into_iter().flatten() {
        if field(c, "type").and_then(Value::as_u64) != Some(1)
            || field(c, "organizationId").is_some()
            || field(c, "deletedDate").is_some()
            || field_str(c, "folderId") != Some(folder.as_str())
        {
            continue;
        }
        let Some(id) = field_str(c, "id") else { continue };
        match parse_item(user_key, c, id) {
            Some(item) => items.push(item),
            None => tracing::warn!(item = %id, "skipped a Bitwarden item kwnt could not read"),
        }
    }
    (folder_id, items)
}

fn parse_item(user_key: &SymKey, c: &Value, id: &str) -> Option<Item> {
    let key_enc = field_str(c, "key").map(str::to_string);
    let key = match &key_enc {
        Some(e) => Some(SymKey::from_slice(&user_key.decrypt(e).ok()?)?),
        None => None,
    };
    let k = key.as_ref().unwrap_or(user_key);
    let owner = field(c, "fields").and_then(Value::as_array)?.iter().find_map(|f| {
        let name = k.decrypt_string(field_str(f, "name")?).ok()?;
        (name.as_str() == OWNER_FIELD).then(|| k.decrypt_string(field_str(f, "value")?).ok()).flatten()
    })?;
    let login = field(c, "login")?;
    let password = k.decrypt_string(field_str(login, "password")?).ok()?;
    let username = match field_str(login, "username") {
        Some(u) => k.decrypt_string(u).ok()?.to_string(),
        None => String::new(),
    };
    let uri = field(login, "uris").and_then(Value::as_array)?.iter().find_map(|u| field_str(u, "uri"))?;
    let origin = normalize_origin(&k.decrypt_string(uri).ok()?).ok()?;
    let created = field_str(c, "creationDate").unwrap_or_default().to_string();
    let revised = field_str(c, "revisionDate").unwrap_or_default().to_string();
    Some(Item {
        owner: owner.to_string(),
        cred: Credential {
            meta: CredentialMeta { id: id.to_string(), origin, username, created_at: created, last_used: revised },
            password: password.to_string(),
        },
        key_enc,
        key,
    })
}

#[async_trait::async_trait]
impl CredentialStore for BitwardenStore {
    async fn list(&self, owner: &str) -> Result<Vec<CredentialMeta>, StoreError> {
        let guard = self.fresh().await?;
        let s = guard.as_ref().ok_or(StoreError::Locked)?;
        Ok(s.items.iter().filter(|i| i.owner == owner).map(|i| i.cred.meta.clone()).collect())
    }

    async fn get(&self, owner: &str, id: &str) -> Result<Option<Credential>, StoreError> {
        let guard = self.fresh().await?;
        let s = guard.as_ref().ok_or(StoreError::Locked)?;
        Ok(s.items.iter().find(|i| i.owner == owner && i.cred.meta.id == id).map(|i| i.cred.clone()))
    }

    /// Locked: nothing to fill (never an error, so a page just loads unfilled).
    async fn for_origin(&self, owner: &str, origin: &str) -> Result<Option<Credential>, StoreError> {
        let Ok(origin) = normalize_origin(origin) else { return Ok(None) };
        let guard = match self.fresh().await {
            Ok(g) => g,
            Err(StoreError::Locked) => return Ok(None),
            Err(e) => return Err(e),
        };
        Ok(guard
            .as_ref()
            .and_then(|s| s.items.iter().find(|i| i.owner == owner && i.cred.meta.origin == origin))
            .map(|i| i.cred.clone()))
    }

    async fn save(&self, owner: &str, origin: &str, username: &str, password: &str) -> Result<CredentialMeta, StoreError> {
        let origin = normalize_origin(origin)?;
        let mut guard = self.fresh().await?;
        let s = guard.as_mut().ok_or(StoreError::Locked)?;
        let existing = s.items.iter().position(|i| i.owner == owner && i.cred.meta.origin == origin && i.cred.meta.username == username);
        let folder = self.ensure_folder(s).await?;
        let result = match existing {
            Some(pos) => {
                let item = &s.items[pos];
                let id = item.cred.meta.id.clone();
                let item_key = match (&item.key, &item.key_enc) {
                    (Some(k), Some(e)) => Some((k, e.as_str())),
                    _ => None,
                };
                let body = self.cipher_body(s, item_key, &folder, owner, &origin, username, password)?;
                self.api(s, reqwest::Method::PUT, &format!("/ciphers/{id}"), Some(&body)).await
            }
            None => {
                let body = self.cipher_body(s, None, &folder, owner, &origin, username, password)?;
                self.api(s, reqwest::Method::POST, "/ciphers", Some(&body)).await
            }
        };
        if let Err(ApiError::Auth) = result {
            *guard = None;
            return Err(StoreError::Locked);
        }
        result?;
        self.sync(s).await?;
        Self::meta_of(s, owner, |i| i.cred.meta.origin == origin && i.cred.meta.username == username)
            .ok_or_else(|| StoreError::Remote("the saved item did not come back from Bitwarden".into()))
    }

    /// Moves the item to the vault's trash, where the person can still restore it.
    async fn delete(&self, owner: &str, id: &str) -> Result<bool, StoreError> {
        let mut guard = self.fresh().await?;
        let s = guard.as_mut().ok_or(StoreError::Locked)?;
        if !s.items.iter().any(|i| i.owner == owner && i.cred.meta.id == id) {
            return Ok(false);
        }
        let path = format!("/ciphers/{}/delete", percent_encoding::utf8_percent_encode(id, percent_encoding::NON_ALPHANUMERIC));
        if let Err(e) = self.api(s, reqwest::Method::PUT, &path, None).await {
            if matches!(e, ApiError::Auth) {
                *guard = None;
            }
            return Err(e.into());
        }
        self.sync(s).await?;
        Ok(true)
    }
}

/// Registers an account on a Vaultwarden server with signups open, for tests against a
/// throwaway server only.
#[doc(hidden)]
pub async fn register_test_account(server: &str, email: &str, password: &str, iterations: u32) -> Result<(), String> {
    let (_, ep) = endpoints(server).map_err(|e| e.to_string())?;
    let http = reqwest::Client::new();
    let mk = master_key(password, email, Kdf::Pbkdf2 { iterations }).map_err(|e| e.to_string())?;
    let rng = SystemRandom::new();
    let mut user_key = [0u8; 64];
    rng.fill(&mut user_key).map_err(|_| "rng")?;
    let key = SymKey::stretch(&mk).encrypt(&user_key, &rng).map_err(|e| e.to_string())?;
    let token = http
        .post(format!("{}/accounts/register/send-verification-email", ep.identity))
        .json(&json!({ "email": email, "name": "kwnt test", "receiveMarketingEmails": false }))
        .send()
        .await
        .map_err(|e| e.to_string())?
        .text()
        .await
        .map_err(|e| e.to_string())?;
    let resp = http
        .post(format!("{}/accounts/register/finish", ep.identity))
        .json(&json!({
            "email": email,
            "emailVerificationToken": token.trim().trim_matches('"'),
            "masterPasswordHash": auth_hash(&mk, password).as_str(),
            "masterPasswordHint": null,
            "key": key,
            "kdf": 0,
            "kdfIterations": iterations,
        }))
        .send()
        .await
        .map_err(|e| e.to_string())?;
    if resp.status().is_success() {
        Ok(())
    } else {
        Err(format!("{} {}", resp.status(), resp.text().await.unwrap_or_default().chars().take(200).collect::<String>()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Computed by an independent implementation (Node's crypto) for password
    // "DUMMY-master-pw-1", email "Dummy@kwnt.test", PBKDF2 600000.
    const KAT_MASTER_KEY_B64: &str = "CvEI1eQwuZ8tFkKq6EBO4FhDkorFZCPtQWMf4y12pSI=";
    const KAT_AUTH_HASH: &str = "3z9m+sVXUmr2sAFZOh3Thweu9CdxKlBc676HEYlvIwQ=";
    const KAT_STRETCHED_B64: &str = "5CEqupEYQ+Tepc8DaJw/rHJA2Kp2iJSaLR7Voz0W65xSWSU0ErWp9xOtmT8PaZVbkzafVQji/sCyHAZXPkYsZA==";
    const KAT_ENC_STRING: &str = "2.AwMDAwMDAwMDAwMDAwMDAw==|sFVXIU/i9DZag+EJ0B9FzQ50pfSZZg1UqPOraR6668E=|wqpcki+XtofSJgGFdHsAmbCgtWmQL9BKvZn2IuM5pYA=";

    #[test]
    fn matches_an_independent_implementation() {
        let mk = master_key("DUMMY-master-pw-1", " Dummy@kwnt.test ", Kdf::Pbkdf2 { iterations: 600_000 }).unwrap();
        assert_eq!(B64.encode(&mk[..]), KAT_MASTER_KEY_B64);
        assert_eq!(auth_hash(&mk, "DUMMY-master-pw-1").as_str(), KAT_AUTH_HASH);
        let stretched = SymKey::stretch(&mk);
        assert_eq!(B64.encode(&stretched.0[..]), KAT_STRETCHED_B64);
        assert_eq!(stretched.decrypt_string(KAT_ENC_STRING).unwrap().as_str(), "DUMMY-KAT-plaintext");
    }

    #[test]
    fn enc_strings_round_trip_and_reject_tampering() {
        let rng = SystemRandom::new();
        let key = SymKey::from_slice(&[7u8; 64]).unwrap();
        let e = key.encrypt(b"DUMMY-PW", &rng).unwrap();
        assert!(e.starts_with("2."));
        assert_eq!(key.decrypt(&e).unwrap().as_slice(), b"DUMMY-PW");
        let other = SymKey::from_slice(&[8u8; 64]).unwrap();
        assert!(other.decrypt(&e).is_err(), "another key's MAC fails");
        let mut parts: Vec<String> = e.split('|').map(str::to_string).collect();
        parts[1] = B64.encode([0u8; 16]);
        assert!(key.decrypt(&parts.join("|")).is_err(), "changed data fails the MAC");
        assert!(key.decrypt("0.abc|def").is_err());
        assert!(SymKey::from_slice(&[0u8; 32]).is_none());
    }

    #[test]
    fn argon2id_and_kdf_limits() {
        let kdf = Kdf::from_json(&json!({"kdf": 1, "kdfIterations": 3, "kdfMemory": 16, "kdfParallelism": 1})).unwrap();
        let a = master_key("pw", "a@b.c", kdf).unwrap();
        let b = master_key("pw", "A@B.C", kdf).unwrap();
        assert_eq!(a[..], b[..], "the email salt ignores case");
        assert!(Kdf::from_json(&json!({"Kdf": 0, "KdfIterations": 600000})).is_ok(), "PascalCase too");
        assert!(Kdf::from_json(&json!({"kdf": 0, "kdfIterations": 1_000_000_000u64})).is_err());
        assert!(Kdf::from_json(&json!({"kdf": 1, "kdfIterations": 3, "kdfMemory": 100000, "kdfParallelism": 4})).is_err());
        assert!(Kdf::from_json(&json!({"kdf": 9, "kdfIterations": 3})).is_err());
    }

    #[test]
    fn server_addresses() {
        let (base, ep) = endpoints("").unwrap();
        assert_eq!(base, "https://vault.bitwarden.com");
        assert_eq!(ep.api, "https://api.bitwarden.com");
        assert_eq!(endpoints("vault.bitwarden.eu").unwrap().1.identity, "https://identity.bitwarden.eu");
        let (base, ep) = endpoints("https://pw.example.org/vw/").unwrap();
        assert_eq!(base, "https://pw.example.org/vw");
        assert_eq!(ep.api, "https://pw.example.org/vw/api");
        assert!(endpoints("http://127.0.0.1:18990").is_ok(), "plain http on this machine");
        assert!(endpoints("http://pw.example.org").is_err(), "never plain http elsewhere");
        assert!(endpoints("https://u:p@pw.example.org").is_err());
        assert!(endpoints("ftp://pw.example.org").is_err());
    }

    fn sample_vault(user_key: &SymKey, rng: &SystemRandom) -> Value {
        let e = |t: &str| user_key.encrypt(t.as_bytes(), rng).unwrap();
        let item_key_raw = [9u8; 64];
        let item_key = SymKey::from_slice(&item_key_raw).unwrap();
        let ie = |t: &str| item_key.encrypt(t.as_bytes(), rng).unwrap();
        let owner_field = |owner: &str| json!([{"type": 1, "name": e(OWNER_FIELD), "value": e(owner)}]);
        json!({
            "folders": [{"id": "f-other", "name": e("Personal")}, {"id": "f-kwnt", "name": e("kwnt")}],
            "ciphers": [
                {"id": "c1", "type": 1, "folderId": "f-kwnt", "fields": owner_field("team-1"),
                 "login": {"username": e("alice"), "password": e("DUMMY-1"), "uris": [{"uri": e("https://example.com/login")}]}},
                // not in the kwnt folder: never used, even with the owner field
                {"id": "c2", "type": 1, "folderId": "f-other", "fields": owner_field("team-1"),
                 "login": {"username": e("bob"), "password": e("DUMMY-2"), "uris": [{"uri": e("https://example.com")}]}},
                // in the folder but no owner field
                {"id": "c3", "type": 1, "folderId": "f-kwnt",
                 "login": {"username": e("carol"), "password": e("DUMMY-3"), "uris": [{"uri": e("https://example.com")}]}},
                // trashed
                {"id": "c4", "type": 1, "folderId": "f-kwnt", "deletedDate": "2026-01-01T00:00:00Z", "fields": owner_field("team-1"),
                 "login": {"username": e("dave"), "password": e("DUMMY-4"), "uris": [{"uri": e("https://example.com")}]}},
                // an organization item
                {"id": "c5", "type": 1, "folderId": "f-kwnt", "organizationId": "o1", "fields": owner_field("team-1"),
                 "login": {"username": e("erin"), "password": e("DUMMY-5"), "uris": [{"uri": e("https://example.com")}]}},
                // its own item key, PascalCase like older servers
                {"Id": "c6", "Type": 1, "FolderId": "f-kwnt", "Key": e_bytes(user_key, &item_key_raw, rng),
                 "Fields": [{"Type": 1, "Name": ie(OWNER_FIELD), "Value": ie("bot-9")}],
                 "Login": {"Username": ie("frank"), "Password": ie("DUMMY-6"), "Uris": [{"Uri": ie("http://127.0.0.1:8080/x")}]}},
            ],
        })
    }

    fn e_bytes(k: &SymKey, b: &[u8], rng: &SystemRandom) -> String {
        k.encrypt(b, rng).unwrap()
    }

    #[test]
    fn only_kwnt_items_in_the_kwnt_folder_are_used() {
        let rng = SystemRandom::new();
        let user_key = SymKey::from_slice(&[5u8; 64]).unwrap();
        let (folder, items) = parse_sync(&user_key, &sample_vault(&user_key, &rng));
        assert_eq!(folder.as_deref(), Some("f-kwnt"));
        let got: Vec<(&str, &str, &str, &str)> = items
            .iter()
            .map(|i| (i.cred.meta.id.as_str(), i.owner.as_str(), i.cred.meta.origin.as_str(), i.cred.password.as_str()))
            .collect();
        assert_eq!(got, vec![("c1", "team-1", "https://example.com", "DUMMY-1"), ("c6", "bot-9", "http://127.0.0.1:8080", "DUMMY-6")]);
        assert!(items[1].key.is_some() && items[1].key_enc.is_some());
    }

    #[tokio::test]
    async fn a_locked_store_fills_nothing_and_lists_nothing() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join(CONFIG_FILE),
            serde_json::to_vec(&BitwardenConfig {
                server_url: "https://vault.bitwarden.com".into(),
                email: "a@b.c".into(),
                client_id: None,
                device_id: "d".into(),
            })
            .unwrap(),
        )
        .unwrap();
        let store = BitwardenStore::open(dir.path());
        assert!(store.is_configured());
        assert!(!store.is_unlocked().await);
        assert!(store.for_origin("team-1", "https://example.com").await.unwrap().is_none());
        assert!(matches!(store.list("team-1").await, Err(StoreError::Locked)));
        assert!(matches!(store.save("team-1", "https://example.com", "u", "p").await, Err(StoreError::Locked)));
        assert!(matches!(store.verify_master_password("x").await, Err(StoreError::Locked)));
        let status = store.status().await.unwrap();
        assert!(!status.unlocked && status.item_count.is_none());
    }

    #[tokio::test]
    async fn another_account_needs_forget_first() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join(CONFIG_FILE),
            serde_json::to_vec(&BitwardenConfig {
                server_url: "https://vault.bitwarden.com".into(),
                email: "a@b.c".into(),
                client_id: None,
                device_id: "d".into(),
            })
            .unwrap(),
        )
        .unwrap();
        let store = BitwardenStore::open(dir.path());
        let mut req = UnlockRequest::default();
        req.server_url = Some("https://pw.attacker.test".into());
        req.email = Some("x@attacker.test".into());
        req.master_password = "DUMMY".into();
        assert!(matches!(store.unlock(req).await, Err(UnlockError::OtherAccount)));
    }
}
