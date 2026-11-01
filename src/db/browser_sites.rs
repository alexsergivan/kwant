//! Each bot's browser site rules (see [`crate::sandbox::site_guard`]): `browser_allowed_sites`
//! and `browser_blocked_sites` (JSON lists of normalized entries) and
//! `browser_block_private_network`. Empty lists and off, the default, change nothing.

use super::{Database, DbError};
use crate::sandbox::site_guard::SiteRules;
use serde::{Deserialize, Serialize};

/// A bot's site rules as the bot routes show and take them.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct BrowserSites {
    #[serde(default)]
    pub browser_allowed_sites: Vec<String>,
    #[serde(default)]
    pub browser_blocked_sites: Vec<String>,
    #[serde(default)]
    pub browser_block_private_network: bool,
}

impl BrowserSites {
    pub fn rules(&self) -> SiteRules {
        SiteRules {
            allowed: self.browser_allowed_sites.clone(),
            blocked: self.browser_blocked_sites.clone(),
            block_private: self.browser_block_private_network,
        }
    }
}

fn list(json: &str) -> Vec<String> {
    serde_json::from_str(json).unwrap_or_default()
}

impl Database {
    pub(super) async fn migrate_browser_sites(&self) -> Result<(), DbError> {
        for column in [
            "browser_allowed_sites TEXT NOT NULL DEFAULT '[]'",
            "browser_blocked_sites TEXT NOT NULL DEFAULT '[]'",
            "browser_block_private_network INTEGER NOT NULL DEFAULT 0",
        ] {
            let _ = sqlx::query(&format!("ALTER TABLE kwants ADD COLUMN {column};")).execute(&self.pool).await;
        }
        Ok(())
    }

    /// The bot's site rules; a missing bot has none.
    pub async fn browser_sites(&self, kwant_id: &str) -> Result<BrowserSites, DbError> {
        let row: Option<(String, String, bool)> = sqlx::query_as(
            "SELECT browser_allowed_sites, browser_blocked_sites, browser_block_private_network FROM kwants WHERE id = ?",
        )
        .bind(kwant_id)
        .fetch_optional(&self.pool)
        .await?;
        Ok(row
            .map(|(allowed, blocked, private)| BrowserSites {
                browser_allowed_sites: list(&allowed),
                browser_blocked_sites: list(&blocked),
                browser_block_private_network: private,
            })
            .unwrap_or_default())
    }

    /// Sets the given fields (already normalized); `None` leaves one as it is.
    pub async fn set_browser_sites(
        &self,
        kwant_id: &str,
        allowed: Option<&[String]>,
        blocked: Option<&[String]>,
        block_private: Option<bool>,
    ) -> Result<(), DbError> {
        if let Some(allowed) = allowed {
            sqlx::query("UPDATE kwants SET browser_allowed_sites = ? WHERE id = ?")
                .bind(serde_json::to_string(allowed).unwrap_or_else(|_| "[]".into()))
                .bind(kwant_id)
                .execute(&self.pool)
                .await?;
        }
        if let Some(blocked) = blocked {
            sqlx::query("UPDATE kwants SET browser_blocked_sites = ? WHERE id = ?")
                .bind(serde_json::to_string(blocked).unwrap_or_else(|_| "[]".into()))
                .bind(kwant_id)
                .execute(&self.pool)
                .await?;
        }
        if let Some(block_private) = block_private {
            sqlx::query("UPDATE kwants SET browser_block_private_network = ? WHERE id = ?")
                .bind(block_private)
                .bind(kwant_id)
                .execute(&self.pool)
                .await?;
        }
        Ok(())
    }
}
