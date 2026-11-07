//! Where the brief (Vega's half) meets pickup (Juno's half), jira-tickets api.md 1.3–1.5.
//!
//! - [`site`]: the connected site, built from the space's saved `dev_jira_sites` row each time
//!   (`atlassian::client_for`, its token from the vault). Nothing is kept in memory, so a server
//!   restart, a changed setting or a deleted site is seen at once.
//! - [`set_state`]: the card row `dev_tickets.state` is the ticket's one state.
//! - [`set_asked`]: the Ask on Jira comment, on both rows.
//!
//! Nothing here reaches the network unless a site is connected: a space with no saved site has
//! no tickets, so with Jira off nothing changes.

use super::source::{TicketSource, TicketWriter};
use crate::db::dev_jira::JiraSiteRow;
use crate::server::AppState;
use std::sync::Arc;

/// One connected Jira site, as the brief needs it.
#[derive(Clone)]
pub struct Site {
    pub site_id: String,
    pub space_id: String,
    /// Scoped to the site's projects and Confluence spaces (api.md 1.2).
    pub source: Arc<dyn TicketSource>,
    /// Only person-only routes use it (Ask on Jira).
    pub writer: Option<Arc<dyn TicketWriter>>,
    /// "Wait for my OK before building" (J10).
    pub wait_for_ok: bool,
    /// Confluence search is on for this site.
    pub confluence: bool,
    /// The saved Jira address and, on Data Center, Confluence's own ([`super::links::origins`]):
    /// every link kwant keeps or shows is on one of them (api-decisions 18).
    pub origins: Vec<url::Url>,
}

/// The space's saved site row, only when it is `site_id` (a ticket of a deleted and re-made
/// site finds nothing).
pub async fn saved(state: &AppState, space_id: &str, site_id: &str) -> Option<JiraSiteRow> {
    state.db.dev_jira_site(space_id).await.ok().flatten().filter(|r| r.id == site_id)
}

/// The site, only when it is the one saved for `space_id`. `None` too when its token can't be
/// read (vault locked, token gone): nothing reads Jira then.
pub async fn site(state: &AppState, space_id: &str, site_id: &str) -> Option<Site> {
    #[cfg(feature = "test-seams")]
    if let Some(s) = test_sites::get(space_id, site_id) {
        return Some(s);
    }
    let row = saved(state, space_id, site_id).await?;
    let client = match crate::server::dev::atlassian::client_for(state, &row).await {
        Ok(c) => c,
        Err(e) => {
            tracing::warn!(%site_id, error = %e, "dev: the Jira site can't be read now");
            return None;
        }
    };
    Some(Site {
        site_id: row.id.clone(),
        space_id: space_id.to_string(),
        source: client.clone(),
        writer: Some(client),
        wait_for_ok: row.wait_for_ok,
        confluence: !row.space_list().is_empty(),
        origins: super::links::origins(&row.jira_url, row.confluence_url.as_deref()),
    })
}

/// Whether the site is connected, and its Confluence search is on: from the saved row alone,
/// without reading the token. `None` when the site is gone.
pub async fn confluence(state: &AppState, space_id: &str, site_id: &str) -> Option<bool> {
    #[cfg(feature = "test-seams")]
    if let Some(s) = test_sites::get(space_id, site_id) {
        return Some(s.confluence);
    }
    saved(state, space_id, site_id).await.map(|r| !r.space_list().is_empty())
}

/// The site's origins ([`Site::origins`]), from the saved row alone (no token read); none when
/// the site is gone, so no link passes.
pub async fn origins(state: &AppState, space_id: &str, site_id: &str) -> Vec<url::Url> {
    #[cfg(feature = "test-seams")]
    if let Some(s) = test_sites::get(space_id, site_id) {
        return s.origins;
    }
    saved(state, space_id, site_id).await.map(|r| super::links::origins(&r.jira_url, r.confluence_url.as_deref())).unwrap_or_default()
}

/// Test-only sites (the `test-seams` feature, never in a shipped build): an in-process source
/// stands in for a saved row, so the brief's tests run without an Atlassian mock.
#[cfg(feature = "test-seams")]
mod test_sites {
    use super::Site;
    use std::collections::HashMap;
    use std::sync::{LazyLock, Mutex};

    static SITES: LazyLock<Mutex<HashMap<String, Site>>> = LazyLock::new(Default::default);

    pub fn get(space_id: &str, site_id: &str) -> Option<Site> {
        SITES.lock().unwrap_or_else(|e| e.into_inner()).get(site_id).filter(|s| s.space_id == space_id).cloned()
    }

    pub fn put(site: Site) {
        SITES.lock().unwrap_or_else(|e| e.into_inner()).insert(site.site_id.clone(), site);
    }

    pub fn drop_site(site_id: &str) {
        SITES.lock().unwrap_or_else(|e| e.into_inner()).remove(site_id);
    }
}

/// Test-only: serve `site` as if it were saved (`test-seams`).
#[cfg(feature = "test-seams")]
pub fn register_for_test(site: Site) {
    test_sites::put(site);
}

/// Test-only: forget a [`register_for_test`] site.
#[cfg(feature = "test-seams")]
pub fn unregister_for_test(site_id: &str) {
    test_sites::drop_site(site_id);
}

/// The ticket row states (api.md 1.4).
pub const STATES: [&str; 11] = ["waiting", "picked_up", "reading", "needs_answer", "brief_ready", "working", "in_review", "landed", "taken_back", "closed", "failed"];
const ACTIVE: [&str; 6] = ["picked_up", "reading", "needs_answer", "brief_ready", "working", "in_review"];

/// Whether the row may move `from` → `to` (api.md 1.4): a late `brief_ready` must not overwrite
/// `taken_back`.
pub fn allowed(from: &str, to: &str) -> bool {
    match to {
        "picked_up" => matches!(from, "waiting" | "taken_back" | "failed"),
        "reading" => matches!(from, "picked_up" | "needs_answer"),
        "needs_answer" => from == "reading",
        "brief_ready" => matches!(from, "reading" | "needs_answer"),
        "working" => matches!(from, "brief_ready" | "picked_up"),
        "in_review" => matches!(from, "working" | "brief_ready"),
        "landed" => matches!(from, "in_review" | "working"),
        "taken_back" => !matches!(from, "closed" | "landed" | "taken_back"),
        "closed" => from != "closed",
        "failed" => ACTIVE.contains(&from),
        _ => false,
    }
}

/// Moves the ticket's row to `to` if allowed; false (and nothing written) when it isn't.
pub async fn set_state(state: &AppState, space_id: &str, key: &str, to: &str) -> bool {
    let Ok(Some(row)) = state.db.ticket_brief(space_id, key).await else { return false };
    if !allowed(&row.state, to) {
        tracing::info!(%key, from = %row.state, %to, "dev: ticket move refused");
        return false;
    }
    let moved = state.db.move_ticket_brief(space_id, key, &[row.state.as_str()], to).await.unwrap_or(false);
    if moved {
        tracing::info!(%key, from = %row.state, %to, "dev: ticket moved");
    }
    moved
}

/// Remembers the Ask on Jira comment, so the reporter's reply after it counts as the answer.
pub async fn set_asked(state: &AppState, space_id: &str, key: &str, comment_id: &str) -> bool {
    state.db.set_ticket_brief_field(space_id, key, "asked_comment_id", Some(comment_id)).await.is_ok()
}

/// The ticket key a branch or PR title carries (`TRAIL-102-orders-export`, "TRAIL-102: Orders
/// export"): the first `PROJECT-123` at its start, upper-case.
pub fn key_in(branch: &str, title: &str) -> Option<String> {
    let at_start = |s: &str| -> Option<String> {
        let s = s.trim().trim_start_matches("refs/heads/");
        let s = s.rsplit('/').next().unwrap_or(s);
        let project: String = s.chars().take_while(|c| c.is_ascii_alphanumeric() || *c == '_').collect();
        if project.is_empty() || !project.chars().next()?.is_ascii_alphabetic() || project.len() > 30 {
            return None;
        }
        let rest = s[project.len()..].strip_prefix('-')?;
        let n: String = rest.chars().take_while(char::is_ascii_digit).collect();
        if n.is_empty() || n.len() > 9 || rest[n.len()..].chars().next().is_some_and(|c| c.is_ascii_alphanumeric()) {
            return None;
        }
        Some(format!("{}-{n}", project.to_ascii_uppercase()))
    };
    at_start(branch).or_else(|| at_start(title))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_at_the_start_of_a_branch_or_title() {
        assert_eq!(key_in("TRAIL-102-orders-export", ""), Some("TRAIL-102".into()));
        assert_eq!(key_in("feature/TRAIL-102-x", ""), Some("TRAIL-102".into()));
        assert_eq!(key_in("main", "TRAIL-7: Orders export"), Some("TRAIL-7".into()));
        assert_eq!(key_in("trail-7-x", ""), Some("TRAIL-7".into()));
        assert_eq!(key_in("fix-TRAIL-7", "Orders"), None, "only at the start");
        assert_eq!(key_in("TRAIL-7a", ""), None);
        assert_eq!(key_in("utf-8-names", ""), Some("UTF-8".into()), "looks like a key; no brief means no section");
        assert_eq!(key_in("", ""), None);
    }

    #[test]
    fn moves_follow_the_table() {
        assert!(allowed("picked_up", "reading"));
        assert!(allowed("needs_answer", "reading"));
        assert!(allowed("reading", "needs_answer"));
        assert!(allowed("needs_answer", "brief_ready"));
        assert!(allowed("brief_ready", "working"));
        assert!(allowed("picked_up", "working"), "resume");
        assert!(!allowed("taken_back", "brief_ready"), "a late brief never overwrites taken back");
        assert!(!allowed("closed", "failed"));
        assert!(allowed("reading", "failed"));
        assert!(!allowed("landed", "taken_back"));
        assert!(!allowed("reading", "nonsense"));
    }
}
