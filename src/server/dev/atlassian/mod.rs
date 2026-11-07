//! A native, read-mostly Atlassian client (jira-tickets spec J2, J3): Jira Cloud (REST v3, ADF)
//! and Data Center (REST v2, wiki markup), Confluence Cloud and DC. Plain HTTP with the forge
//! client's address pinning; no model in the loop. The token lives in the password vault and only
//! here: it is never in a row, a log line, an event or anything a bot can read.
//!
//! - `client`: sites, auth headers (basic, scoped through api.atlassian.com, DC bearer PAT), one
//!   request with status mapping and Retry-After;
//! - `jira` / `confluence`: reads, and the one write (`add_comment`, J9);
//! - `jql`: the pickup rule and Advanced JQL, scoped to the chosen projects; CQL;
//! - `text`: ADF, wiki markup and storage HTML to plain text and back;
//! - `check`: the setup sheet's site check;
//!
//! The mock Atlassian server for tests is `tests/support/mock_atlassian.rs` (its Python twin for
//! screenshots is `site-media-kit/rig/mock_atlassian.py`).

pub mod check;
pub mod client;
pub mod confluence;
pub mod jira;
pub mod jql;
pub mod text;

use std::sync::Arc;

use zeroize::Zeroizing;

use crate::db::dev_jira::JiraSiteRow;
use crate::server::dev::git;
use crate::server::dev::tickets::source::{SourceError, TicketSource, TicketWriter};
use crate::server::AppState;
use client::{AtlassianClient, Auth, Kind, Secrets, Site};

/// A queue order for Jira priorities (lower first): the default scheme's names on Cloud and DC,
/// else the numeric priority id (`1` is the highest in both default schemes), else none (last).
pub fn priority_rank(name: &str, id: Option<&str>) -> Option<u32> {
    let rank = match name.trim().to_ascii_lowercase().as_str() {
        "blocker" => Some(0),
        "highest" | "critical" => Some(1),
        "high" | "major" => Some(2),
        "medium" => Some(3),
        "low" | "minor" => Some(4),
        "lowest" | "trivial" => Some(5),
        _ => None,
    };
    rank.or_else(|| id.and_then(|i| i.trim().parse::<u32>().ok()).map(|n| n.min(99) + 10))
}

/// The vault name of DC Confluence's own PAT (the Jira token is under the site id itself).
pub fn confluence_secret_id(site_id: &str) -> String {
    format!("{site_id}.confluence")
}

/// The vault origin for an address (scheme, host and port).
pub fn vault_origin(url: &str) -> Option<String> {
    crate::sandbox::credentials::normalize_origin(url).ok()
}

/// The [`Site`] a saved row describes.
pub fn site_of(row: &JiraSiteRow) -> Result<Site, SourceError> {
    let unreadable = || SourceError::Invalid("the saved Jira settings are unreadable; set it up again".into());
    Ok(Site {
        kind: Kind::parse(&row.kind).ok_or_else(unreadable)?,
        jira_url: row.jira_url.clone(),
        confluence_url: row.confluence_url.clone(),
        email: row.email.clone(),
        auth: Auth::parse(&row.auth).ok_or_else(unreadable)?,
        cloud_id: row.cloud_id.clone(),
        projects: row.project_list(),
        spaces: row.space_list(),
    })
}

fn vault(e: crate::sandbox::credentials::StoreError) -> SourceError {
    match e {
        crate::sandbox::credentials::StoreError::Locked => SourceError::Unavailable("the password vault is locked".into()),
        _ => SourceError::Unavailable("the password vault is unavailable".into()),
    }
}

/// The saved token(s) for a row. `Unauthorized` when the Jira token is gone from the vault.
pub async fn secrets_of(state: &AppState, row: &JiraSiteRow) -> Result<Secrets, SourceError> {
    let store = state.credentials.as_ref();
    let jira = crate::zombie::hunt::source_secret(store, &row.id, &row.jira_url).await.map_err(vault)?.ok_or(SourceError::Unauthorized)?;
    let confluence = match (row.kind.as_str(), row.confluence_url.as_deref()) {
        ("dc", Some(url)) => crate::zombie::hunt::source_secret(store, &confluence_secret_id(&row.id), url).await.map_err(vault)?,
        _ => None,
    };
    Ok(Secrets { jira, confluence })
}

/// Saves a token under `name` for `url`'s origin, replacing any earlier one.
pub async fn save_secret(state: &AppState, name: &str, url: &str, secret: &Zeroizing<String>) -> Result<(), crate::sandbox::credentials::StoreError> {
    use crate::sandbox::credentials::CredentialStore;
    let store = state.credentials.as_ref();
    let origin = crate::sandbox::credentials::normalize_origin(url)?;
    crate::zombie::hunt::forget_secret(store, name).await?;
    store.save(crate::zombie::VAULT_OWNER, &origin, name, secret).await?;
    Ok(())
}

/// Deletes a site's token(s).
pub async fn forget_secrets(state: &AppState, site_id: &str) -> Result<(), crate::sandbox::credentials::StoreError> {
    let store = state.credentials.as_ref();
    crate::zombie::hunt::forget_secret(store, site_id).await?;
    crate::zombie::hunt::forget_secret(store, &confluence_secret_id(site_id)).await
}

pub fn resolver() -> Arc<dyn git::Resolver> {
    Arc::new(git::SystemResolver)
}

/// A client for a saved site, with its token from the vault.
pub async fn client_for(state: &AppState, row: &JiraSiteRow) -> Result<Arc<AtlassianClient>, SourceError> {
    let secrets = secrets_of(state, row).await?;
    Ok(Arc::new(AtlassianClient::new(site_of(row)?, &secrets, resolver())?))
}

/// The read side for Vega's tools (api.md §1.2), scoped to the row's projects and spaces.
pub async fn source_for(state: &AppState, row: &JiraSiteRow) -> Result<Arc<dyn TicketSource>, SourceError> {
    Ok(client_for(state, row).await?)
}

/// The one write, for person-only routes (Ask on Jira, J9).
pub async fn writer_for(state: &AppState, row: &JiraSiteRow) -> Result<Arc<dyn TicketWriter>, SourceError> {
    Ok(client_for(state, row).await?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn priorities_rank_by_name_then_id() {
        assert_eq!(priority_rank("Blocker", None), Some(0));
        assert_eq!(priority_rank("Highest", Some("1")), Some(1));
        assert_eq!(priority_rank("critical", None), Some(1));
        assert_eq!(priority_rank("Major", None), Some(2));
        assert_eq!(priority_rank(" Medium ", None), Some(3));
        assert_eq!(priority_rank("Minor", None), Some(4));
        assert_eq!(priority_rank("Trivial", None), Some(5));
        assert_eq!(priority_rank("P0 - drop everything", Some("10000")), Some(109));
        assert_eq!(priority_rank("Urgent", Some("2")), Some(12));
        assert_eq!(priority_rank("Whenever", None), None);
    }
}
