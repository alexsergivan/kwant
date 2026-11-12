//! One browser page per job (0.9.3). A team's bots shared one page, so a sign-in the person was
//! asked to do on it (a held page) stopped every other job of the team. Now a bot turn in a chat
//! gets its own page, keyed by (sandbox target, bot, chat): a new page in the team's existing
//! browser context, so sign-ins and cookies are shared, while a hold for the person's step holds
//! only the page it was asked on. The person's panel and calls that name no chat keep the team
//! page, whose key is the sandbox target itself.
//!
//! Limits: at most [`MAX_JOB_PAGES_PER_TEAM`] job pages per team (the least recently used idle
//! one is closed to make room) and a job page nobody used for [`JOB_PAGE_IDLE`] is closed. A
//! page with an open step, one the person is driving, or one whose job (bot and chat) still has
//! a turn running is never closed. Room is made and a page opened under one lock per team
//! ([`JobPages::team_lock`]), so jobs opening at once never pass the cap. A bot tool that finds
//! no room waits up to [`PAGE_WAIT`] for a page to free up, then gets [`pages_full_message`].

use super::continuity::SelfScope;
use super::AppState;
use axum::{extract::{Path, State}, Json};
use serde_json::{json, Value};
use std::{
    collections::HashMap,
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc,
    },
    time::{Duration, Instant},
};

pub use crate::sandbox::service::{is_job_page, job_page_key, page_team, PAGE_SEP};

/// Most job pages one team keeps open at once.
pub const MAX_JOB_PAGES_PER_TEAM: usize = 4;
/// A job page unused this long is closed (unless a step holds it).
pub const JOB_PAGE_IDLE: Duration = Duration::from_secs(30 * 60);
/// How often idle job pages are looked for.
pub const SWEEP_EVERY: Duration = Duration::from_secs(60);
/// How long a bot's browser call waits for one of its team's pages to free up before it is
/// told every page is in use.
pub const PAGE_WAIT: Duration = Duration::from_secs(60);

/// What a job is told when every job page of its team is in use and none freed up in time.
pub fn pages_full_message() -> String {
    format!(
        "All {MAX_JOB_PAGES_PER_TEAM} of this team's browser pages are in use by other jobs. \
         Try again in a minute; nothing is wrong with the browser."
    )
}

/// Opening job pages: one lock per team, so making room and opening the page happen as one
/// step, and how long a bot call waits for room.
#[derive(Default)]
pub struct JobPages {
    opening: std::sync::Mutex<HashMap<String, Arc<tokio::sync::Mutex<()>>>>,
    /// Tests only: the wait for room in ms (0 = [`PAGE_WAIT`]).
    wait_ms: AtomicU64,
}

impl JobPages {
    /// The lock held while a page of `team` is made room for and opened, or closed.
    pub fn team_lock(&self, team: &str) -> Arc<tokio::sync::Mutex<()>> {
        self.opening.lock().unwrap_or_else(|e| e.into_inner()).entry(team.to_string()).or_default().clone()
    }

    /// Tests: a shorter wait for room than [`PAGE_WAIT`].
    pub fn set_wait(&self, wait: Duration) {
        self.wait_ms.store((wait.as_millis() as u64).max(1), Ordering::Relaxed);
    }

    pub fn wait(&self) -> Duration {
        match self.wait_ms.load(Ordering::Relaxed) {
            0 => PAGE_WAIT,
            ms => Duration::from_millis(ms),
        }
    }
}

/// A job page's team, bot and chat; `None` for the team page.
pub fn job_of(key: &str) -> Option<(&str, &str, &str)> {
    let mut parts = key.splitn(3, PAGE_SEP);
    match (parts.next(), parts.next(), parts.next()) {
        (Some(team), Some(kwant), Some(chat)) => Some((team, kwant, chat)),
        _ => None,
    }
}

/// The page a turn's browser calls use: the job's own page when the turn runs in a chat
/// ([`super::human_steps::chat_key`], the same chat the bot's browser address names), else the
/// team page. `None` when the turn has no bot browser.
pub fn scope_page(scope: &SelfScope) -> Option<String> {
    let team = scope.sandbox_target.as_deref()?;
    Some(
        super::human_steps::chat_key(scope)
            .and_then(|chat| job_page_key(team, &scope.kwant_id, &chat))
            .unwrap_or_else(|| team.to_string()),
    )
}

/// The page a call on the bot browser API uses: the chat route's job page, else (the plain route,
/// or a bot route without a chat) the team page.
pub fn call_page(team: &str, kwant: Option<&str>, chat: Option<&str>) -> String {
    match (kwant, chat) {
        (Some(kwant), Some(chat)) => job_page_key(team, kwant, chat).unwrap_or_else(|| team.to_string()),
        _ => team.to_string(),
    }
}

/// Whether a page may be closed now: no open step holds it, its job has no turn still running
/// (a bot waiting minutes on a render between two calls comes back to its page, not a blank
/// one), and nobody (the person, or a bot's action) is using it.
async fn closable(state: &AppState, key: &str) -> bool {
    if state.db.human_step_open_on(key).await.unwrap_or(true) {
        return false;
    }
    if let Some((team, kwant, chat)) = job_of(key) {
        if state.human_steps.has_turn_in(team, kwant, chat) {
            return false;
        }
    }
    match state.browser_controllers.lock().await.get(key) {
        Some(ctrl) => ctrl.control.try_write().is_ok() && !ctrl.is_held_for_person(),
        None => true,
    }
}

/// Closes a job page: its controller first (cookies kept), then the page.
pub async fn close_page(state: &AppState, key: &str) {
    let ctrl = state.browser_controllers.lock().await.remove(key);
    if let Some(ctrl) = ctrl {
        ctrl.persist_cookies().await;
    }
    if let Err(e) = state.browser_service.close_page(key).await {
        tracing::debug!("couldn't close the job page {key}: {e}");
    }
    tracing::info!(page = %key, "closed a job's browser page");
}

/// Before a new job page opens in `team`: makes room under [`MAX_JOB_PAGES_PER_TEAM`] by closing
/// the least recently used page that can be closed. `false` when every page is held or in use.
/// The caller holds the team's [`JobPages::team_lock`] until the new page is open.
pub async fn make_room(state: &AppState, team: &str, key: &str) -> bool {
    let pages = state.browser_service.job_pages(team).await;
    let open: Vec<_> = pages.into_iter().filter(|(k, _)| k != key).collect();
    if open.len() < MAX_JOB_PAGES_PER_TEAM {
        return true;
    }
    let mut to_close = open.len() + 1 - MAX_JOB_PAGES_PER_TEAM;
    for (page, _) in open {
        if to_close == 0 {
            break;
        }
        if closable(state, &page).await {
            close_page(state, &page).await;
            to_close -= 1;
        }
    }
    to_close == 0
}

/// Closes job pages unused for `idle` that no step holds and nobody is using. Returns how many.
pub async fn sweep_idle(state: &AppState, idle: Duration) -> usize {
    let now = Instant::now();
    let teams: std::collections::BTreeSet<String> = state
        .browser_service
        .pages_in_use()
        .into_iter()
        .filter(|(k, _)| is_job_page(k))
        .map(|(k, _)| page_team(&k).to_string())
        .collect();
    let mut closed = 0;
    for team in teams {
        let lock = state.job_pages.team_lock(&team);
        let _opening = lock.lock().await;
        for (page, at) in state.browser_service.job_pages(&team).await {
            if now.duration_since(at) >= idle && closable(state, &page).await {
                close_page(state, &page).await;
                closed += 1;
            }
        }
    }
    closed
}

/// Runs [`sweep_idle`] every [`SWEEP_EVERY`] until `cancel`.
pub fn spawn_sweeper(state: AppState, cancel: tokio_util::sync::CancellationToken) {
    tokio::spawn(async move {
        loop {
            tokio::select! {
                _ = cancel.cancelled() => return,
                _ = tokio::time::sleep(SWEEP_EVERY) => {}
            }
            let closed = sweep_idle(&state, JOB_PAGE_IDLE).await;
            if closed > 0 {
                tracing::info!("browser: closed {closed} idle job pages");
            }
        }
    });
}

/// The page the team's live view shows: the page an open step holds (the person has something
/// to do there), else the page used most recently, else the team page.
pub async fn live_page(state: &AppState, team: &str) -> String {
    if let Ok(steps) = state.db.list_human_steps(Some("open"), None, None).await {
        if let Some(page) = steps.iter().filter(|s| s.sandbox_target.as_deref() == Some(team)).find_map(|s| s.page().map(str::to_string)) {
            return page;
        }
    }
    let prefix = format!("{team}{}", PAGE_SEP);
    state
        .browser_service
        .pages_in_use()
        .into_iter()
        .filter(|(k, _)| k == team || k.starts_with(&prefix))
        .max_by_key(|(_, at)| *at)
        .map(|(k, _)| k)
        .unwrap_or_else(|| team.to_string())
}

/// `GET /api/sandbox/:team_id/pages`: the team's pages for a tab strip, newest first. Each has
/// its `page` key (pass it to the live view as `?page=`), the bot and chat of a job page, its
/// address and title, whether a step holds it (`held`), and seconds since its last use. `live`
/// is the page the live view shows by default.
pub async fn list_pages(State(state): State<AppState>, Path(team_id): Path<String>) -> Json<Value> {
    let team = page_team(&team_id).to_string();
    let now = Instant::now();
    let mut pages: Vec<(String, Option<Instant>)> = vec![(team.clone(), state.browser_service.page_used_at(&team))];
    pages.extend(state.browser_service.job_pages(&team).await.into_iter().map(|(k, at)| (k, Some(at))));
    pages.sort_by_key(|(_, at)| std::cmp::Reverse(*at));
    let mut out = Vec::new();
    for (key, at) in pages {
        let ctrl = state.browser_controllers.lock().await.get(&key).cloned();
        let (url, title) = match &ctrl {
            Some(c) => (c.current_url.lock().await.clone(), c.current_title.lock().await.clone()),
            None => (None, None),
        };
        let mut parts = key.splitn(3, PAGE_SEP).skip(1);
        out.push(json!({
            "page": key,
            "team_id": team,
            "kwant_id": parts.next(),
            "chat_id": parts.next(),
            "job": is_job_page(&key),
            "url": url,
            "title": title,
            "held": state.db.human_step_open_on(&key).await.unwrap_or(false),
            "idle_seconds": at.map(|at| now.duration_since(at).as_secs()),
        }));
    }
    Json(json!({
        "team_id": team,
        "live": live_page(&state, &team).await,
        "max_job_pages": MAX_JOB_PAGES_PER_TEAM,
        "pages": out,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_job_page_is_keyed_by_team_bot_and_chat() {
        assert_eq!(job_page_key("t", "k", "c").as_deref(), Some("t#k#c"));
        assert_eq!(job_page_key("t", "k", ""), None);
        assert_eq!(job_page_key("t", "k#x", "c"), None, "a part with the separator can't make a key");
        assert_eq!(page_team("t#k#c"), "t");
        assert_eq!(page_team("t"), "t");
        assert!(is_job_page("t#k#c") && !is_job_page("t"));
        assert_eq!(call_page("t", Some("k"), Some("c")), "t#k#c");
        assert_eq!(call_page("t", Some("k"), None), "t", "a bot route without a chat keeps the team page");
        assert_eq!(call_page("t", None, None), "t", "the person's panel keeps the team page");
    }

    #[test]
    fn a_job_pages_parts_are_read_back_from_its_key() {
        assert_eq!(job_of("t#k#c"), Some(("t", "k", "c")));
        assert_eq!(job_of("t"), None);
        assert!(pages_full_message().starts_with("All 4 of this team's browser pages are in use by other jobs."));
        let pages = JobPages::default();
        assert_eq!(pages.wait(), PAGE_WAIT);
        pages.set_wait(Duration::from_millis(5));
        assert_eq!(pages.wait(), Duration::from_millis(5));
        assert!(Arc::ptr_eq(&pages.team_lock("t"), &pages.team_lock("t")));
        assert!(!Arc::ptr_eq(&pages.team_lock("t"), &pages.team_lock("u")), "one lock per team");
    }

    #[test]
    fn a_turn_in_a_chat_uses_its_own_page_and_one_without_the_team_page() {
        let origin = super::super::continuity::TurnOrigin { thread_id: Some("th".into()), ..Default::default() };
        let scope = SelfScope { kwant_id: "k".into(), origin: Some(origin), sandbox_target: Some("t".into()), ..Default::default() };
        assert_eq!(scope_page(&scope).as_deref(), Some("t#k#th"));
        let no_chat = SelfScope { kwant_id: "k".into(), sandbox_target: Some("t".into()), ..Default::default() };
        assert_eq!(scope_page(&no_chat).as_deref(), Some("t"));
        let no_browser = SelfScope { kwant_id: "k".into(), ..Default::default() };
        assert_eq!(scope_page(&no_browser), None);
    }
}
