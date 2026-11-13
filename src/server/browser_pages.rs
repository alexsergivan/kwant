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
//!
//! Resting and recovery (0.9.5). In VM mode a page no bot has used and nobody has watched for
//! [`IDLE_FREEZE`] is put to rest: its media paused and the page frozen, so a feed left
//! autoplaying doesn't burn the browser's CPU and memory; a team page idle for [`IDLE_BLANK`] is
//! sent to about:blank (its cookies stay in the jar). A rested page wakes before its next
//! action, screenshot or live view. When the browser stops answering, a bot's call retries with
//! backoff for up to [`RETRY_WINDOW`], first freeing memory by closing or blanking other idle
//! pages ([`relieve`]), never the page the bot is using; if it still can't, the bot is told the
//! problem is temporary ([`crate::sandbox::service::BROWSER_BUSY`]).

use super::continuity::SelfScope;
use super::AppState;
use crate::sandbox::service::Rest;
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
/// VM mode: a page no bot used and nobody watched for this long is frozen (media paused).
pub const IDLE_FREEZE: Duration = Duration::from_secs(10 * 60);
/// VM mode: a team page idle this long is sent to about:blank.
pub const IDLE_BLANK: Duration = Duration::from_secs(2 * 60 * 60);
/// How long a bot's browser call keeps retrying a browser that doesn't answer (VM mode).
pub const RETRY_WINDOW: Duration = Duration::from_secs(45);
/// The first wait between retries; each next one doubles, up to [`RETRY_MAX_DELAY`].
pub const RETRY_FIRST_DELAY: Duration = Duration::from_secs(1);
pub const RETRY_MAX_DELAY: Duration = Duration::from_secs(8);
/// Freeing memory for a stalled browser leaves alone a page used this recently.
const RELIEF_IDLE: Duration = Duration::from_secs(60);
/// Longest one rest or relief step on one page may take (the browser may be stalled).
const REST_STEP: Duration = Duration::from_secs(5);
/// Longest all of one [`relieve`] may take.
const RELIEF_BUDGET: Duration = Duration::from_secs(15);

/// The wait before retry number `attempt` (0-based): 1, 2, 4, 8, 8, ... seconds.
pub fn retry_delay(attempt: u32) -> Duration {
    RETRY_FIRST_DELAY.saturating_mul(1u32 << attempt.min(16)).min(RETRY_MAX_DELAY)
}

/// Whether to try once more after waiting `delay`, `elapsed` into a retry `window`: only while
/// the next attempt still starts inside it.
pub fn keep_retrying(elapsed: Duration, delay: Duration, window: Duration) -> bool {
    elapsed + delay < window
}

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
    /// Tests only: the retry window in ms (0 = [`RETRY_WINDOW`]).
    retry_ms: AtomicU64,
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

    /// Tests: a shorter retry window than [`RETRY_WINDOW`].
    pub fn set_retry_window(&self, window: Duration) {
        self.retry_ms.store((window.as_millis() as u64).max(1), Ordering::Relaxed);
    }

    pub fn retry_window(&self) -> Duration {
        match self.retry_ms.load(Ordering::Relaxed) {
            0 => RETRY_WINDOW,
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

/// Whether a page may be put to rest now: [`closable`], and no live view shows it.
async fn restable(state: &AppState, key: &str) -> bool {
    let watched = state.browser_controllers.lock().await.get(key).is_some_and(|c| c.watchers() > 0);
    !watched && closable(state, key).await
}

/// Puts one page to rest (see [`Rest`]) through its controller, or one made for the open page
/// when no bot has used it this run. Bounded by [`REST_STEP`]. Whether it was.
async fn rest_page(state: &AppState, key: &str, rest: Rest) -> bool {
    let ctrl = state.browser_controllers.lock().await.get(key).cloned();
    let ctrl = match ctrl {
        Some(ctrl) => ctrl,
        None => match state.browser_service.existing_page(key).await {
            Some(ctrl) => ctrl,
            None => return false,
        },
    };
    let done = match rest {
        Rest::Frozen => tokio::time::timeout(REST_STEP, ctrl.freeze()).await,
        Rest::Blank => tokio::time::timeout(REST_STEP, ctrl.blank()).await,
    };
    match done {
        Ok(Ok(())) => {
            state.browser_service.mark_rested(key, rest);
            true
        }
        Ok(Err(e)) => {
            tracing::debug!(page = %key, "couldn't put an idle page to rest: {e}");
            false
        }
        Err(_) => {
            tracing::debug!(page = %key, "putting an idle page to rest timed out");
            false
        }
    }
}

/// What the idle sweep does with a page idle for `idle` that was last put to rest as `rested`:
/// nothing before `freeze_after`; a team page past `blank_after` is blanked; any other is
/// frozen. `None` when it already is.
pub fn rest_for(job: bool, idle: Duration, rested: Option<Rest>, freeze_after: Duration, blank_after: Duration) -> Option<Rest> {
    if idle < freeze_after || rested == Some(Rest::Blank) {
        return None;
    }
    let want = if !job && idle >= blank_after { Rest::Blank } else { Rest::Frozen };
    (rested != Some(want)).then_some(want)
}

/// VM mode: puts to rest the pages idle past `freeze_after` (see [`rest_for`]) that no step
/// holds, no turn of their job is running in, and nobody is using or watching. Returns how many
/// were (frozen, blanked).
pub async fn rest_idle(state: &AppState, freeze_after: Duration, blank_after: Duration) -> (usize, usize) {
    if state.browser_service.mode() == crate::sandbox::computer::ComputerMode::Local {
        return (0, 0);
    }
    let now = Instant::now();
    let (mut frozen, mut blanked) = (0, 0);
    for (key, at) in state.browser_service.open_pages().await {
        let rested = state.browser_service.rest_of(&key);
        let Some(rest) = rest_for(is_job_page(&key), now.duration_since(at), rested, freeze_after, blank_after) else { continue };
        if !restable(state, &key).await || !rest_page(state, &key, rest).await {
            continue;
        }
        tracing::info!(page = %key, ?rest, "browser: put an idle page to rest");
        match rest {
            Rest::Frozen => frozen += 1,
            Rest::Blank => blanked += 1,
        }
    }
    (frozen, blanked)
}

/// The browser stopped answering a call for `keep`: frees memory by closing other idle job pages
/// and blanking other idle team pages, least recently used first, never `keep` nor a page used
/// in the last minute, watched, held or in use. Each step is bounded (the browser may be
/// stalled), and all of it by [`RELIEF_BUDGET`]. Returns how many pages were closed or blanked.
pub async fn relieve(state: &AppState, keep: &str) -> usize {
    if state.browser_service.mode() == crate::sandbox::computer::ComputerMode::Local {
        return 0;
    }
    let mut relieved = 0;
    let work = async {
        let now = Instant::now();
        let pages = match tokio::time::timeout(REST_STEP, state.browser_service.open_pages()).await {
            Ok(pages) => pages,
            Err(_) => return,
        };
        for (key, at) in pages {
            if key == keep || now.duration_since(at) < RELIEF_IDLE || !restable(state, &key).await {
                continue;
            }
            if is_job_page(&key) {
                // a page being opened or closed for its team is left to that
                let lock = state.job_pages.team_lock(page_team(&key));
                let Ok(_opening) = lock.try_lock() else { continue };
                if tokio::time::timeout(REST_STEP, close_page(state, &key)).await.is_ok() {
                    relieved += 1;
                }
            } else if state.browser_service.rest_of(&key) != Some(Rest::Blank) && rest_page(state, &key, Rest::Blank).await {
                relieved += 1;
            }
        }
    };
    let _ = tokio::time::timeout(RELIEF_BUDGET, work).await;
    if relieved > 0 {
        tracing::warn!(page = %keep, "browser: not answering; closed or blanked {relieved} other idle pages to free memory");
    }
    relieved
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

/// Runs [`sweep_idle`] and [`rest_idle`] every [`SWEEP_EVERY`] until `cancel`.
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
            let (frozen, blanked) = rest_idle(&state, IDLE_FREEZE, IDLE_BLANK).await;
            if frozen + blanked > 0 {
                tracing::info!("browser: froze {frozen} and blanked {blanked} idle pages");
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
    fn retries_back_off_from_one_second_to_eight_inside_the_window() {
        let delays: Vec<u64> = (0..6).map(|n| retry_delay(n).as_secs()).collect();
        assert_eq!(delays, [1, 2, 4, 8, 8, 8]);
        assert_eq!(retry_delay(40), RETRY_MAX_DELAY, "no overflow on a long outage");
        let s = Duration::from_secs;
        assert!(keep_retrying(s(0), s(1), RETRY_WINDOW));
        assert!(keep_retrying(s(36), s(8), RETRY_WINDOW));
        assert!(!keep_retrying(s(37), s(8), RETRY_WINDOW), "the next try would start after the window");
        assert!(!keep_retrying(s(45), s(1), RETRY_WINDOW));
        // the waits alone fill the window in 1+2+4+8+8+8+8 = 39 s: about 7 tries when each fails fast
        let mut elapsed = Duration::ZERO;
        let mut tries = 1;
        while keep_retrying(elapsed, retry_delay(tries - 1), RETRY_WINDOW) {
            elapsed += retry_delay(tries - 1);
            tries += 1;
        }
        assert_eq!((tries, elapsed.as_secs()), (8, 39));
        let pages = JobPages::default();
        assert_eq!(pages.retry_window(), RETRY_WINDOW);
        pages.set_retry_window(Duration::from_millis(5));
        assert_eq!(pages.retry_window(), Duration::from_millis(5));
    }

    #[test]
    fn idle_pages_are_frozen_after_ten_minutes_and_team_pages_blanked_after_hours() {
        let m = |n: u64| Duration::from_secs(n * 60);
        let rest = |job, idle, rested| rest_for(job, idle, rested, IDLE_FREEZE, IDLE_BLANK);
        assert_eq!(rest(false, m(9), None), None, "used within ten minutes");
        assert_eq!(rest(false, m(10), None), Some(Rest::Frozen));
        assert_eq!(rest(true, m(10), None), Some(Rest::Frozen));
        assert_eq!(rest(false, m(30), Some(Rest::Frozen)), None, "already frozen");
        assert_eq!(rest(false, m(120), Some(Rest::Frozen)), Some(Rest::Blank));
        assert_eq!(rest(false, m(120), None), Some(Rest::Blank));
        assert_eq!(rest(false, m(600), Some(Rest::Blank)), None, "already blank");
        assert_eq!(rest(true, m(600), None), Some(Rest::Frozen), "a job page is never blanked (it is closed)");
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
