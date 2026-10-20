//! Saved website passwords, kept where no bot's tools look: `<data_dir>/kwnt-vault/<owner>.bin`,
//! each file AES-256-GCM encrypted with the key in `<data_dir>/kwnt-vault.key` (both owner-only,
//! 0600 files in 0700 folders). An owner is the team, channel or bot whose browser uses them.
//!
//! [`CredentialStore`] is the seam for where passwords live: [`EncryptedFileStore`] is the
//! built-in one, [`super::bitwarden::BitwardenStore`] the person's Bitwarden vault, and
//! [`super::vault::PasswordVault`] picks between them.
//!
//! A bot running as the same OS user could still read both files with approvals off; kwnt
//! refuses tool calls that name them ([`mentions_kwnt_secrets`]), which raises the bar but is
//! not a wall. Bitwarden is: its keys only ever live in kwnt's memory.

use ring::aead::{Aad, LessSafeKey, Nonce, UnboundKey, AES_256_GCM, NONCE_LEN};
use ring::rand::{SecureRandom, SystemRandom};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

pub const VAULT_DIR: &str = "kwnt-vault";
pub const KEY_FILE: &str = "kwnt-vault.key";
/// The legacy plaintext file browsers used to keep in a bot's folder.
pub const LEGACY_FILE: &str = ".saved_passwords.json";

#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    #[error("credential store I/O failed: {0}")]
    Io(#[from] std::io::Error),
    #[error("credential store could not be decrypted")]
    Crypto,
    #[error("invalid origin: {0}")]
    Origin(String),
    #[error("the password vault is locked")]
    Locked,
    #[error("the password manager failed: {0}")]
    Remote(String),
}

/// What the app may see about a saved password: never the password itself.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CredentialMeta {
    pub id: String,
    /// `scheme://host[:port]`; the password is only filled on exactly this origin.
    pub origin: String,
    pub username: String,
    pub created_at: String,
    pub last_used: String,
}

/// A saved password. `Debug` never prints the password.
#[derive(Clone, Serialize, Deserialize, PartialEq)]
pub struct Credential {
    #[serde(flatten)]
    pub meta: CredentialMeta,
    pub password: String,
}

impl std::fmt::Debug for Credential {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Credential")
            .field("meta", &self.meta)
            .field("password", &"[redacted]")
            .finish()
    }
}

/// Where saved passwords live. Every call names its owner (team, channel or bot id).
#[async_trait::async_trait]
pub trait CredentialStore: Send + Sync {
    async fn list(&self, owner: &str) -> Result<Vec<CredentialMeta>, StoreError>;
    async fn get(&self, owner: &str, id: &str) -> Result<Option<Credential>, StoreError>;
    /// The credential to fill on a page whose origin is exactly `origin`.
    async fn for_origin(&self, owner: &str, origin: &str) -> Result<Option<Credential>, StoreError>;
    /// Adds a credential, or replaces the password of the same origin + username.
    async fn save(&self, owner: &str, origin: &str, username: &str, password: &str) -> Result<CredentialMeta, StoreError>;
    async fn delete(&self, owner: &str, id: &str) -> Result<bool, StoreError>;
    /// Removes everything kept for `owner` once its team, channel or bot is deleted. Only the
    /// built-in store keeps per-owner files; other stores keep nothing to remove.
    async fn forget_owner(&self, _owner: &str) -> Result<(), StoreError> {
        Ok(())
    }
}

/// `scheme://host[:port]` of an http(s) URL, or of a bare host (taken as https).
pub fn normalize_origin(input: &str) -> Result<String, StoreError> {
    let raw = input.trim();
    let with_scheme = if raw.contains("://") { raw.to_string() } else { format!("https://{raw}") };
    let url = url::Url::parse(&with_scheme).map_err(|_| StoreError::Origin(raw.to_string()))?;
    if !matches!(url.scheme(), "http" | "https") || url.host_str().is_none_or(str::is_empty) {
        return Err(StoreError::Origin(raw.to_string()));
    }
    Ok(url.origin().ascii_serialization())
}

/// Whether a tool call's text names kwnt's own secret files (the vault, its key, the API token).
pub fn mentions_kwnt_secrets(text: &str) -> bool {
    let t = text.to_ascii_lowercase();
    t.contains(VAULT_DIR) || t.contains("api-token") || t.contains(LEGACY_FILE) || t.contains("kwnt token")
}

/// Whether a tool call's arguments name kwnt's own secret files: in any text, or through a path
/// argument that resolves there (a symlink, `..`), relative to `cwd`.
pub fn call_touches_kwnt_secrets(arguments: &serde_json::Value, cwd: Option<&Path>) -> bool {
    use serde_json::Value;
    if mentions_kwnt_secrets(&arguments.to_string()) {
        return true;
    }
    let resolves_there = |s: &str| {
        let p = Path::new(s);
        let p = match (p.is_absolute(), cwd) {
            (true, _) => p.to_path_buf(),
            (false, Some(cwd)) => cwd.join(p),
            (false, None) => return false,
        };
        std::fs::canonicalize(&p).is_ok_and(|real| mentions_kwnt_secrets(&real.to_string_lossy()))
    };
    fn walk(v: &Value, f: &dyn Fn(&str) -> bool) -> bool {
        match v {
            Value::String(s) => f(s),
            Value::Array(a) => a.iter().any(|v| walk(v, f)),
            Value::Object(m) => m.values().any(|v| walk(v, f)),
            _ => false,
        }
    }
    walk(arguments, &|s: &str| s.len() < 4096 && !s.contains('\n') && resolves_there(s.trim()))
}

pub struct EncryptedFileStore {
    dir: PathBuf,
    key: LessSafeKey,
    rng: SystemRandom,
    lock: std::sync::Mutex<()>,
}

impl std::fmt::Debug for EncryptedFileStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EncryptedFileStore").field("dir", &self.dir).finish_non_exhaustive()
    }
}

#[cfg(unix)]
pub(crate) fn private_dir(path: &Path) -> std::io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::create_dir_all(path)?;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700))
}

#[cfg(not(unix))]
pub(crate) fn private_dir(path: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(path)
}

/// Writes `bytes` to `path` as a new 0600 file, via a temp file and a rename.
pub(crate) fn write_private(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    use std::io::Write;
    let tmp = path.with_extension(format!("tmp-{}", uuid::Uuid::new_v4().simple()));
    let mut opts = std::fs::OpenOptions::new();
    opts.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        opts.mode(0o600);
    }
    let result = (|| {
        let mut f = opts.open(&tmp)?;
        f.write_all(bytes)?;
        f.sync_all()?;
        std::fs::rename(&tmp, path)
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    result
}

fn load_or_create_key(path: &Path, rng: &SystemRandom) -> Result<[u8; 32], StoreError> {
    if let Ok(bytes) = std::fs::read(path) {
        if let Ok(key) = <[u8; 32]>::try_from(bytes.as_slice()) {
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600));
            }
            return Ok(key);
        }
        // A key of the wrong size would make every saved password unreadable: refuse to replace it
        return Err(StoreError::Crypto);
    }
    let mut key = [0u8; 32];
    rng.fill(&mut key).map_err(|_| StoreError::Crypto)?;
    write_private(path, &key)?;
    Ok(key)
}

/// The owner each `*.bin` file in `dir` is named after, live or not: every file the built-in
/// store holds. A hashed name (`h-…`) can't be opened by its stem, so it is counted, not moved.
pub fn stored_owners(dir: &Path) -> Vec<String> {
    let Ok(entries) = std::fs::read_dir(dir) else { return Vec::new() };
    let mut owners: Vec<String> = entries
        .flatten()
        .filter_map(|e| e.file_name().to_str().and_then(|n| n.strip_suffix(".bin")).map(str::to_string))
        .filter(|o| !o.is_empty())
        .collect();
    owners.sort();
    owners
}

/// A file name for an owner id; ids are uuids, anything else is hashed.
fn owner_file(owner: &str) -> String {
    let plain = !owner.is_empty() && owner.len() <= 64 && owner.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_');
    if plain {
        format!("{owner}.bin")
    } else {
        use sha2::{Digest, Sha256};
        let digest = Sha256::digest(owner.as_bytes());
        format!("h-{}.bin", digest.iter().take(16).map(|b| format!("{b:02x}")).collect::<String>())
    }
}

impl EncryptedFileStore {
    /// Opens (or creates) the store under `data_dir`.
    pub fn open(data_dir: &Path) -> Result<Self, StoreError> {
        private_dir(data_dir)?;
        let dir = data_dir.join(VAULT_DIR);
        private_dir(&dir)?;
        let rng = SystemRandom::new();
        let key = load_or_create_key(&data_dir.join(KEY_FILE), &rng)?;
        let key = LessSafeKey::new(UnboundKey::new(&AES_256_GCM, &key).map_err(|_| StoreError::Crypto)?);
        Ok(Self { dir, key, rng, lock: std::sync::Mutex::new(()) })
    }

    fn path(&self, owner: &str) -> PathBuf {
        self.dir.join(owner_file(owner))
    }

    fn read(&self, owner: &str) -> Result<Vec<Credential>, StoreError> {
        let bytes = match std::fs::read(self.path(owner)) {
            Ok(b) => b,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(e) => return Err(e.into()),
        };
        if bytes.len() < NONCE_LEN {
            return Err(StoreError::Crypto);
        }
        let (nonce, sealed) = bytes.split_at(NONCE_LEN);
        let nonce = Nonce::try_assume_unique_for_key(nonce).map_err(|_| StoreError::Crypto)?;
        let mut sealed = sealed.to_vec();
        // The owner is the associated data: a file copied to another owner's name won't open
        let plain = self
            .key
            .open_in_place(nonce, Aad::from(owner.as_bytes()), &mut sealed)
            .map_err(|_| StoreError::Crypto)?;
        serde_json::from_slice(plain).map_err(|_| StoreError::Crypto)
    }

    fn write(&self, owner: &str, creds: &[Credential]) -> Result<(), StoreError> {
        let path = self.path(owner);
        if creds.is_empty() {
            return match std::fs::remove_file(&path) {
                Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e.into()),
                _ => Ok(()),
            };
        }
        let mut nonce = [0u8; NONCE_LEN];
        self.rng.fill(&mut nonce).map_err(|_| StoreError::Crypto)?;
        let mut data = serde_json::to_vec(creds).map_err(|_| StoreError::Crypto)?;
        self.key
            .seal_in_place_append_tag(Nonce::assume_unique_for_key(nonce), Aad::from(owner.as_bytes()), &mut data)
            .map_err(|_| StoreError::Crypto)?;
        let mut out = nonce.to_vec();
        out.extend_from_slice(&data);
        write_private(&path, &out)?;
        Ok(())
    }

    fn guard(&self) -> std::sync::MutexGuard<'_, ()> {
        self.lock.lock().unwrap_or_else(|p| p.into_inner())
    }
}

#[async_trait::async_trait]
impl CredentialStore for EncryptedFileStore {
    async fn list(&self, owner: &str) -> Result<Vec<CredentialMeta>, StoreError> {
        let _g = self.guard();
        Ok(self.read(owner)?.into_iter().map(|c| c.meta).collect())
    }

    async fn get(&self, owner: &str, id: &str) -> Result<Option<Credential>, StoreError> {
        let _g = self.guard();
        Ok(self.read(owner)?.into_iter().find(|c| c.meta.id == id))
    }

    async fn for_origin(&self, owner: &str, origin: &str) -> Result<Option<Credential>, StoreError> {
        let Ok(origin) = normalize_origin(origin) else { return Ok(None) };
        let _g = self.guard();
        Ok(self.read(owner)?.into_iter().find(|c| c.meta.origin == origin))
    }

    async fn save(&self, owner: &str, origin: &str, username: &str, password: &str) -> Result<CredentialMeta, StoreError> {
        let origin = normalize_origin(origin)?;
        let _g = self.guard();
        let mut creds = self.read(owner)?;
        let now = chrono::Utc::now().to_rfc3339();
        let meta = match creds.iter_mut().find(|c| c.meta.origin == origin && c.meta.username == username) {
            Some(existing) => {
                existing.password = password.to_string();
                existing.meta.last_used = now;
                existing.meta.clone()
            }
            None => {
                let meta = CredentialMeta {
                    id: uuid::Uuid::new_v4().to_string(),
                    origin,
                    username: username.to_string(),
                    created_at: now.clone(),
                    last_used: now,
                };
                creds.push(Credential { meta: meta.clone(), password: password.to_string() });
                meta
            }
        };
        self.write(owner, &creds)?;
        Ok(meta)
    }

    async fn delete(&self, owner: &str, id: &str) -> Result<bool, StoreError> {
        let _g = self.guard();
        let mut creds = self.read(owner)?;
        let before = creds.len();
        creds.retain(|c| c.meta.id != id);
        if creds.len() == before {
            return Ok(false);
        }
        self.write(owner, &creds)?;
        Ok(true)
    }

    async fn forget_owner(&self, owner: &str) -> Result<(), StoreError> {
        let _g = self.guard();
        match std::fs::remove_file(self.path(owner)) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e.into()),
            _ => Ok(()),
        }
    }
}

/// Stands in when the store can't be opened (an unreadable key): every call fails, nothing is
/// written, and the key file is left alone for the person to restore.
#[derive(Debug)]
pub struct UnavailableStore;

#[async_trait::async_trait]
impl CredentialStore for UnavailableStore {
    async fn list(&self, _: &str) -> Result<Vec<CredentialMeta>, StoreError> {
        Err(StoreError::Crypto)
    }
    async fn get(&self, _: &str, _: &str) -> Result<Option<Credential>, StoreError> {
        Err(StoreError::Crypto)
    }
    async fn for_origin(&self, _: &str, _: &str) -> Result<Option<Credential>, StoreError> {
        Err(StoreError::Crypto)
    }
    async fn save(&self, _: &str, _: &str, _: &str, _: &str) -> Result<CredentialMeta, StoreError> {
        Err(StoreError::Crypto)
    }
    async fn delete(&self, _: &str, _: &str) -> Result<bool, StoreError> {
        Err(StoreError::Crypto)
    }
}

/// The built-in store under `data_dir`, or [`UnavailableStore`] when it can't be opened.
pub fn open_store(data_dir: &Path) -> std::sync::Arc<dyn CredentialStore> {
    match EncryptedFileStore::open(data_dir) {
        Ok(s) => std::sync::Arc::new(s),
        Err(e) => {
            tracing::error!(error = %e, "saved passwords are unavailable: the vault could not be opened");
            std::sync::Arc::new(UnavailableStore)
        }
    }
}

/// The legacy plaintext entry shape (`.saved_passwords.json`).
#[derive(Deserialize)]
struct LegacyCredential {
    #[serde(default)]
    domain: String,
    #[serde(default)]
    origin: String,
    #[serde(default)]
    username: String,
    #[serde(default)]
    password: String,
}

/// Moves `<folder>/.saved_passwords.json` into the store for each of `owners`, then deletes the
/// file. Returns how many credentials were moved. A file that can't be parsed is left in place.
pub async fn migrate_legacy(store: &dyn CredentialStore, folder: &Path, owners: &[&str]) -> Result<usize, StoreError> {
    let file = folder.join(LEGACY_FILE);
    let content = match std::fs::read_to_string(&file) {
        Ok(c) => c,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(0),
        Err(e) => return Err(e.into()),
    };
    let Ok(legacy) = serde_json::from_str::<Vec<LegacyCredential>>(&content) else {
        tracing::warn!(file = %file.display(), "a saved-passwords file could not be read; left in place");
        return Ok(0);
    };
    let mut moved = 0;
    for c in &legacy {
        if c.password.is_empty() {
            continue;
        }
        let source = if c.origin.is_empty() { c.domain.as_str() } else { c.origin.as_str() };
        let Ok(origin) = normalize_origin(source) else { continue };
        for owner in owners {
            store.save(owner, &origin, &c.username, &c.password).await?;
        }
        moved += 1;
    }
    std::fs::remove_file(&file)?;
    Ok(moved)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn origins_are_exact() {
        assert_eq!(normalize_origin("example.com").unwrap(), "https://example.com");
        assert_eq!(normalize_origin("https://Example.com/login?x=1").unwrap(), "https://example.com");
        assert_eq!(normalize_origin("http://127.0.0.1:8080/a").unwrap(), "http://127.0.0.1:8080");
        assert!(normalize_origin("file:///etc/passwd").is_err());
        assert!(normalize_origin("javascript:alert(1)").is_err());
    }

    #[tokio::test]
    async fn encrypted_round_trip_and_private_files() {
        let dir = tempfile::tempdir().unwrap();
        let store = EncryptedFileStore::open(dir.path()).unwrap();
        let meta = store.save("team-1", "https://example.com/login", "alice", "DUMMY-PW-1").await.unwrap();
        assert_eq!(meta.origin, "https://example.com");
        let raw = std::fs::read(dir.path().join(VAULT_DIR).join("team-1.bin")).unwrap();
        assert!(!String::from_utf8_lossy(&raw).contains("DUMMY-PW-1"));
        assert!(!String::from_utf8_lossy(&raw).contains("alice"));
        assert_eq!(store.get("team-1", &meta.id).await.unwrap().unwrap().password, "DUMMY-PW-1");
        assert!(store.list("team-2").await.unwrap().is_empty());
        assert!(!format!("{:?}", store.get("team-1", &meta.id).await.unwrap()).contains("DUMMY"));
        // same origin + username replaces
        store.save("team-1", "https://example.com", "alice", "DUMMY-PW-2").await.unwrap();
        assert_eq!(store.list("team-1").await.unwrap().len(), 1);
        // reopen with the same key
        let again = EncryptedFileStore::open(dir.path()).unwrap();
        assert_eq!(again.get("team-1", &meta.id).await.unwrap().unwrap().password, "DUMMY-PW-2");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = |p: PathBuf| std::fs::metadata(p).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode(dir.path().join(KEY_FILE)), 0o600);
            assert_eq!(mode(dir.path().join(VAULT_DIR)), 0o700);
            assert_eq!(mode(dir.path().join(VAULT_DIR).join("team-1.bin")), 0o600);
        }
        assert!(store.delete("team-1", &meta.id).await.unwrap());
        assert!(!dir.path().join(VAULT_DIR).join("team-1.bin").exists());
    }

    #[tokio::test]
    async fn a_file_moved_to_another_owner_does_not_open() {
        let dir = tempfile::tempdir().unwrap();
        let store = EncryptedFileStore::open(dir.path()).unwrap();
        store.save("a", "https://example.com", "u", "DUMMY").await.unwrap();
        let v = dir.path().join(VAULT_DIR);
        std::fs::copy(v.join("a.bin"), v.join("b.bin")).unwrap();
        assert!(matches!(store.list("b").await, Err(StoreError::Crypto)));
    }

    #[tokio::test]
    async fn every_file_is_listed_and_forgetting_an_owner_removes_its_file() {
        let dir = tempfile::tempdir().unwrap();
        let store = EncryptedFileStore::open(dir.path()).unwrap();
        store.save("a", "https://example.com", "u", "DUMMY").await.unwrap();
        store.save("b", "https://example.com", "u", "DUMMY").await.unwrap();
        let v = dir.path().join(VAULT_DIR);
        std::fs::write(v.join("bitwarden.json"), b"{}").unwrap();
        assert_eq!(stored_owners(&v), vec!["a".to_string(), "b".to_string()]);
        store.forget_owner("a").await.unwrap();
        store.forget_owner("never-saved").await.unwrap();
        assert!(!v.join("a.bin").exists());
        assert_eq!(stored_owners(&v), vec!["b".to_string()]);
    }

    #[test]
    fn secret_mentions() {
        assert!(mentions_kwnt_secrets("cat ~/.kwnt/kwnt-vault.key"));
        assert!(mentions_kwnt_secrets("cat /data/API-TOKEN"));
        assert!(mentions_kwnt_secrets("kwnt token"));
        assert!(!mentions_kwnt_secrets("touch x"));
    }
}
