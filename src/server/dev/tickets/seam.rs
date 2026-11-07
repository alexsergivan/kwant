//! Where the brief (Vega's half) meets pickup (Juno's half), jira-tickets api.md 1.3–1.5.
//!
//! - [`site`]: the connected site, built from the space's saved `dev_jira_sites` row each time
//!   (`atlassian::client_for`, its token from the vault). Nothing is kept in memory, so a server
//!   restart, a changed setting or a deleted site is seen at once.
//! - [`set_state`]: the card row `dev_tickets.state` is the ticket's one state.
//! - [`set_asked`]: the Ask on Jira comment, on both rows.
//!
//! Nothing here reaches the network unless a site is connected and On: a space with no saved site
//! has no tickets, and a site turned Off (`on_` false) has no [`site`], so no Jira read or write is
//! made for it (brief.md §1 step 1, spec E: "pickup stops and the cards stay").

use super::source::{TicketSource, TicketWriter};
use super::store::{self, FailCode};
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

/// The site, only when it is the one saved for `space_id` and it is On. `None` too when it is Off
/// ([`is_off`] tells the two apart) or its token can't be read (vault locked, token gone): nothing
/// reads or writes Jira then.
pub async fn site(state: &AppState, space_id: &str, site_id: &str) -> Option<Site> {
    #[cfg(feature = "test-seams")]
    if let Some(s) = test_sites::get(space_id, site_id) {
        return Some(s);
    }
    let row = saved(state, space_id, site_id).await?;
    if !row.on_ {
        return None;
    }
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

/// Whether `site_id` is the space's saved site and the person turned it Off. Its tickets then wait
/// as they are: [`site`] is `None` for it, but that isn't a failure.
pub async fn is_off(state: &AppState, space_id: &str, site_id: &str) -> bool {
    saved(state, space_id, site_id).await.is_some_and(|r| !r.on_)
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

/// The ticket row states (api.md 1.4): the card row's, which the brief row mirrors.
pub use crate::db::dev_jira::STATES;

/// Whether the card may move `from` → `to` (api.md 1.4): a late `brief_ready` must not overwrite
/// `taken_back`. The one table, `dev_jira::allowed`.
pub fn allowed(from: &str, to: &str) -> bool {
    crate::db::dev_jira::allowed(from, to)
}

/// Moves the ticket to `to` if the card row allows it; false (and nothing written) when it
/// doesn't. The card row (`dev_tickets.state`) is the one truth (D2): the move goes through
/// `store::set_state` (so `dev_ticket_events` gets its row and `dev_ticket` is sent) and the
/// brief row's `state` is mirrored in the same call. `failed` goes through [`fail`].
pub async fn set_state(state: &AppState, space_id: &str, key: &str, to: &str) -> bool {
    if to == "failed" {
        return fail(state, space_id, key, FailCode::Failed).await;
    }
    let Ok(Some(card)) = state.db.dev_ticket(space_id, key).await else { return false };
    if !allowed(&card.state, to) {
        tracing::info!(%key, from = %card.state, %to, "dev: ticket move refused");
        return false;
    }
    match store::set_state(state, space_id, key, to, None).await {
        Ok(true) => {
            tracing::info!(%key, from = %card.state, %to, "dev: ticket moved");
            true
        }
        Ok(false) => false,
        Err(e) => {
            tracing::warn!(%key, from = %card.state, %to, error = %e, "dev: could not move a ticket");
            false
        }
    }
}

/// Fails the ticket with `code` (card row, its event and `dev_ticket`; the brief row mirrors it).
pub async fn fail(state: &AppState, space_id: &str, key: &str, code: FailCode) -> bool {
    store::fail(state, space_id, key, code, None).await.unwrap_or_else(|e| {
        tracing::warn!(%key, error = %e, "dev: could not fail a ticket");
        false
    })
}

/// Remembers the Ask on Jira comment on both rows (D3): the card's `asked_comment_id` (pickup
/// watches for the reporter's reply after it) and the brief's (the reply that answers it).
pub async fn set_asked(state: &AppState, space_id: &str, key: &str, comment_id: &str) -> bool {
    let card = state.db.set_dev_ticket_asked(space_id, key, comment_id).await.unwrap_or(false);
    let brief = state.db.set_ticket_brief_field(space_id, key, "asked_comment_id", Some(comment_id)).await.is_ok();
    card && brief
}

#[cfg(test)]
mod tests {
    use super::*;

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
        assert!(!allowed("working", "working"), "no move to where it is");
    }
}
