//! Which store saved passwords live in: the built-in encrypted files by default, or the person's
//! Bitwarden vault once they unlock one ([`BitwardenStore`]). While Bitwarden is set up it is the
//! only store used; locked, nothing is filled and nothing can be read or saved until the person
//! unlocks it again. Forgetting Bitwarden goes back to the built-in store.

use super::bitwarden::BitwardenStore;
use super::credentials::{open_store, Credential, CredentialMeta, CredentialStore, StoreError, VAULT_DIR};
use std::path::Path;
use std::sync::Arc;

pub struct PasswordVault {
    local: Arc<dyn CredentialStore>,
    bitwarden: BitwardenStore,
}

impl std::fmt::Debug for PasswordVault {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PasswordVault").field("bitwarden", &self.bitwarden).finish_non_exhaustive()
    }
}

/// What a move from the built-in store to Bitwarden did.
#[derive(Debug, Default, Clone, serde::Serialize, PartialEq)]
pub struct Migration {
    pub moved: usize,
    pub failed: usize,
}

impl PasswordVault {
    /// The vault under `data_dir`: the built-in store and the saved Bitwarden settings, locked.
    pub fn open(data_dir: &Path) -> Self {
        Self::new(open_store(data_dir), data_dir)
    }

    pub fn new(local: Arc<dyn CredentialStore>, data_dir: &Path) -> Self {
        Self { local, bitwarden: BitwardenStore::open(&data_dir.join(VAULT_DIR)) }
    }

    pub fn bitwarden(&self) -> &BitwardenStore {
        &self.bitwarden
    }

    pub fn local(&self) -> &dyn CredentialStore {
        self.local.as_ref()
    }

    /// `"bitwarden"` once the person set Bitwarden up, else `"local"`.
    pub fn store_name(&self) -> &'static str {
        if self.bitwarden.is_configured() {
            "bitwarden"
        } else {
            "local"
        }
    }

    /// Whether saved passwords can't be used until the person unlocks Bitwarden.
    pub async fn is_locked(&self) -> bool {
        self.bitwarden.is_configured() && !self.bitwarden.is_unlocked().await
    }

    fn active(&self) -> &dyn CredentialStore {
        if self.bitwarden.is_configured() {
            &self.bitwarden
        } else {
            self.local.as_ref()
        }
    }

    /// Moves every built-in saved password of `owners` into the unlocked Bitwarden vault. Each is
    /// deleted locally only after Bitwarden has it; one that fails stays where it was.
    pub async fn move_local_to_bitwarden(&self, owners: &[String]) -> Result<Migration, StoreError> {
        if !self.bitwarden.is_unlocked().await {
            return Err(StoreError::Locked);
        }
        let mut done = Migration::default();
        for owner in owners {
            let Ok(list) = self.local.list(owner).await else {
                continue;
            };
            for meta in list {
                let Ok(Some(c)) = self.local.get(owner, &meta.id).await else {
                    done.failed += 1;
                    continue;
                };
                match self.bitwarden.save(owner, &c.meta.origin, &c.meta.username, &c.password).await {
                    Ok(_) if self.local.delete(owner, &meta.id).await.is_ok() => done.moved += 1,
                    Ok(_) => done.failed += 1,
                    Err(StoreError::Locked) => return Err(StoreError::Locked),
                    Err(e) => {
                        tracing::warn!(error = %e, "a saved password could not be moved to Bitwarden");
                        done.failed += 1;
                    }
                }
            }
        }
        Ok(done)
    }

    /// How many passwords the built-in store holds for `owners`.
    pub async fn local_count(&self, owners: &[String]) -> usize {
        let mut n = 0;
        for owner in owners {
            n += self.local.list(owner).await.map(|l| l.len()).unwrap_or(0);
        }
        n
    }
}

#[async_trait::async_trait]
impl CredentialStore for PasswordVault {
    async fn list(&self, owner: &str) -> Result<Vec<CredentialMeta>, StoreError> {
        self.active().list(owner).await
    }
    async fn get(&self, owner: &str, id: &str) -> Result<Option<Credential>, StoreError> {
        self.active().get(owner, id).await
    }
    async fn for_origin(&self, owner: &str, origin: &str) -> Result<Option<Credential>, StoreError> {
        self.active().for_origin(owner, origin).await
    }
    async fn save(&self, owner: &str, origin: &str, username: &str, password: &str) -> Result<CredentialMeta, StoreError> {
        self.active().save(owner, origin, username, password).await
    }
    async fn delete(&self, owner: &str, id: &str) -> Result<bool, StoreError> {
        self.active().delete(owner, id).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sandbox::bitwarden::{BitwardenConfig, CONFIG_FILE};

    #[tokio::test]
    async fn local_by_default_and_locked_once_bitwarden_is_set_up() {
        let dir = tempfile::tempdir().unwrap();
        let vault = PasswordVault::open(dir.path());
        assert_eq!(vault.store_name(), "local");
        assert!(!vault.is_locked().await);
        vault.save("team-1", "https://example.com", "alice", "DUMMY-PW").await.unwrap();
        assert!(vault.for_origin("team-1", "https://example.com/login").await.unwrap().is_some());

        std::fs::write(
            dir.path().join(VAULT_DIR).join(CONFIG_FILE),
            serde_json::to_vec(&BitwardenConfig {
                server_url: "https://vault.bitwarden.com".into(),
                email: "a@b.c".into(),
                client_id: None,
                device_id: "d".into(),
            })
            .unwrap(),
        )
        .unwrap();
        // After a restart with Bitwarden set up: locked, nothing filled, the local store unused
        let vault = PasswordVault::open(dir.path());
        assert_eq!(vault.store_name(), "bitwarden");
        assert!(vault.is_locked().await);
        assert!(vault.for_origin("team-1", "https://example.com").await.unwrap().is_none());
        assert!(matches!(vault.list("team-1").await, Err(StoreError::Locked)));
        assert!(matches!(vault.save("team-1", "https://example.com", "u", "p").await, Err(StoreError::Locked)));
        assert!(matches!(vault.move_local_to_bitwarden(&["team-1".into()]).await, Err(StoreError::Locked)));
        assert_eq!(vault.local_count(&["team-1".into()]).await, 1, "the local copy stays until moved");
    }
}
