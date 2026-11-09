//! Folders an isolated Claude turn has used (agents in the VM, D4). A bot in the container can
//! leave config there (`.claude/settings.json` hooks, `.mcp.json`, `.codex/config.toml`) that a
//! CLI kwnt later starts on this computer would load. The record lives here, never in the folder,
//! so the bot can't make, forge or remove it; it stays until the person clears it.
//!
//! Keyed on the canonical path. A folder counts as used when it is a recorded one, inside one or
//! holds one (a CLI started in a parent or a child can load the planted files too).

use super::{Database, DbError};
use std::path::{Path, PathBuf};

/// Canonical form of `folder` for the record, or the path itself when it doesn't exist (yet).
pub fn canonical(folder: &Path) -> PathBuf {
    folder.canonicalize().unwrap_or_else(|_| folder.to_path_buf())
}

/// Whether `folder` overlaps a recorded folder (equal, inside or containing). Both canonical.
pub fn overlaps(folder: &Path, recorded: &Path) -> bool {
    folder.starts_with(recorded) || recorded.starts_with(folder)
}

/// The error a refused host command records (its code first, for the API and the UI).
pub const HOST_COMMAND_REFUSED: &str = "folder_used_by_isolated_bot: a bot that runs Claude inside the VM used this folder, \
so commands from it don't run on this computer. Nothing ran.";

impl Database {
    pub(super) async fn migrate_isolated_folders(&self) -> Result<(), DbError> {
        // A path that isn't valid UTF-8 is stored lossily and never matches: the container
        // refuses such folders before it starts (see `agent_vm::mount_arg`)
        sqlx::query(
            "CREATE TABLE IF NOT EXISTS isolated_folders (
                path TEXT PRIMARY KEY,
                first_bot_id TEXT NOT NULL,
                first_used_at TIMESTAMP NOT NULL,
                last_used_at TIMESTAMP NOT NULL
            )",
        )
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// Notes that an isolated turn of `bot_id` uses `folder` (canonical). Called before the
    /// container starts, so a turn that fails later is recorded too.
    pub async fn record_isolated_folder(&self, folder: &Path, bot_id: &str) -> Result<(), DbError> {
        let path = folder.to_string_lossy();
        let now = chrono::Utc::now();
        sqlx::query(
            "INSERT INTO isolated_folders (path, first_bot_id, first_used_at, last_used_at) VALUES (?, ?, ?, ?) \
             ON CONFLICT(path) DO UPDATE SET last_used_at = excluded.last_used_at",
        )
        .bind(path.as_ref())
        .bind(bot_id)
        .bind(now)
        .bind(now)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// Every recorded folder.
    pub async fn isolated_folders(&self) -> Result<Vec<PathBuf>, DbError> {
        let rows: Vec<(String,)> = sqlx::query_as("SELECT path FROM isolated_folders ORDER BY path").fetch_all(&self.pool).await?;
        Ok(rows.into_iter().map(|(p,)| PathBuf::from(p)).collect())
    }

    /// The recorded folder `folder` overlaps, if any (`folder` is canonicalized here).
    pub async fn isolated_folder_for(&self, folder: &Path) -> Result<Option<PathBuf>, DbError> {
        let folder = canonical(folder);
        Ok(self.isolated_folders().await?.into_iter().find(|r| overlaps(&folder, r)))
    }

    /// Why a command kwnt would run on this computer in `folder` (a mission's command check, a
    /// Dev backup command) must not, or `None` when it may (D4, C9b/C9c). Such a command runs
    /// with kwnt's own user and environment, so a script an isolated bot left in the folder would
    /// escape the VM. Fails closed: an unreadable record refuses.
    pub async fn host_command_refusal(&self, folder: &Path) -> Option<String> {
        match self.isolated_folder_for(folder).await {
            Ok(None) => None,
            Ok(Some(_)) => Some(HOST_COMMAND_REFUSED.into()),
            Err(e) => {
                tracing::warn!(error = %e, "couldn't read the folders isolated bots used; not running the command");
                Some(HOST_COMMAND_REFUSED.into())
            }
        }
    }

    /// The recorded folder a program kwnt starts on this computer in `cwd` would be near, or `None`
    /// when there's none (D4). With a folder: equal, inside or holding a recorded one. Without one
    /// (or a missing one) the program starts in kwnt's own working folder, often the home folder
    /// that holds every recorded folder: there only a recorded folder it sits in counts (programs
    /// read settings walking up, not down). Fails closed: an unreadable record counts as near.
    pub async fn isolated_folder_near(&self, cwd: Option<&Path>) -> Option<PathBuf> {
        let found = match cwd.filter(|d| d.exists()) {
            Some(d) => self.isolated_folder_for(d).await.map_err(|e| (e, d.to_path_buf())),
            None => {
                let here = std::env::current_dir().map(|d| canonical(&d)).unwrap_or_default();
                self.isolated_folders().await.map(|all| all.into_iter().find(|r| here.starts_with(r))).map_err(|e| (e, here))
            }
        };
        found.unwrap_or_else(|(e, at)| {
            tracing::warn!(error = %e, "couldn't read the folders isolated bots used; treating this one as used");
            Some(at)
        })
    }

    /// Forgets a recorded folder (the person cleared it). Returns whether one was there.
    pub async fn forget_isolated_folder(&self, folder: &Path) -> Result<bool, DbError> {
        let done = sqlx::query("DELETE FROM isolated_folders WHERE path = ?")
            .bind(canonical(folder).to_string_lossy().as_ref())
            .execute(&self.pool)
            .await?;
        Ok(done.rows_affected() > 0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn a_recorded_folder_covers_its_parents_and_children_and_survives_a_restart() {
        let d = tempfile::tempdir().unwrap();
        let data = d.path().join("data");
        let proj = d.path().join("proj");
        std::fs::create_dir_all(proj.join("sub")).unwrap();
        let other = d.path().join("other");
        std::fs::create_dir_all(&other).unwrap();
        #[cfg(unix)]
        let link = d.path().join("link");
        #[cfg(unix)]
        std::os::unix::fs::symlink(&proj, &link).unwrap();

        let db = Database::init(&data).await.unwrap();
        assert_eq!(db.isolated_folder_for(&proj).await.unwrap(), None);
        db.record_isolated_folder(&canonical(&proj), "b1").await.unwrap();
        db.record_isolated_folder(&canonical(&proj), "b2").await.unwrap();
        assert_eq!(db.isolated_folders().await.unwrap().len(), 1, "one record per folder");
        drop(db);

        let db = Database::init(&data).await.unwrap();
        let want = Some(canonical(&proj));
        assert_eq!(db.isolated_folder_for(&proj).await.unwrap(), want);
        assert_eq!(db.isolated_folder_for(&proj.join("sub")).await.unwrap(), want, "child");
        assert_eq!(db.isolated_folder_for(d.path()).await.unwrap(), want, "parent");
        assert_eq!(db.isolated_folder_for(&proj.join("sub/../")).await.unwrap(), want, "dot-dot");
        #[cfg(unix)]
        assert_eq!(db.isolated_folder_for(&link).await.unwrap(), want, "symlink");
        assert_eq!(db.isolated_folder_for(&other).await.unwrap(), None, "a sibling isn't covered");
        // `/x/proj` doesn't cover `/x/project`
        assert!(!overlaps(Path::new("/x/project"), Path::new("/x/proj")));

        assert!(db.forget_isolated_folder(&proj).await.unwrap());
        assert_eq!(db.isolated_folder_for(&proj).await.unwrap(), None);
    }
}
