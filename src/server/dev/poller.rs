//! The Dev mode poller (plan §3.3, D4, D5, D8; team-repos.md T5): every [`EVERY`] (and right after
//! the switch goes on, or a repo or branch moves) it reads each repo a Dev mode space watches, read
//! only, and writes a pending `dev_cards` row with its [`DevFacts`] per merge to the watched branch
//! and per open PR head, then hands it to [`super::generate`]. Each repo is polled on its own and
//! keeps its own last seen commit, poll error and tag: one failing repo never stops the others.
//! Draft PRs are skipped; diffs are cut at [`DIFF_CHARS`]. Lists go through ETags; a 403/429 rate
//! limit backs the whole space off (it's one token) until GitHub says it may ask again.

use super::facts::{Author, CiState, Check, DevFacts, FileDiff, Kind, KwantRef, Links, DIFF_CHARS};
use super::repo;
use crate::db::dev_mode::{DevCardRow, DevRepoRow, LANDED, PENDING, REVIEW, STALE};
use crate::db::models::Kwant;
use crate::db::spaces::SpaceId;
use crate::server::AppState;
use super::git::{self, Auth, GitError, Repo};
use super::source::{Change, Commit, Forge, GithubForge, History};
use crate::zombie::github::{EtagCache, GhError};
use chrono::{DateTime, Duration, Utc};
use std::collections::{HashMap, HashSet};
use std::sync::{Mutex, OnceLock};
use tokio::sync::Notify;
use tokio_util::sync::CancellationToken;

/// How often each Dev mode space is read, unless `KWANT_DEV_POLL_SECS` says otherwise ([`super::poll_every`]).
pub const EVERY: std::time::Duration = std::time::Duration::from_secs(5 * 60);
/// Merges a space's first poll back-fills, so the desk isn't empty on day one.
pub const BACKFILL: usize = 5;
/// Most new merges one poll turns into cards (a long pause or a force-push takes the newest).
pub const MAX_NEW: usize = 20;
/// Most new review heads one poll reads (each costs a few requests).
pub const MAX_REVIEWS: usize = 10;
/// Files whose history is read for `first_edit_files`.
pub const FIRST_EDIT_FILES: usize = 20;
/// "First edit" means no commit to the file in this many days.
pub const FIRST_EDIT_DAYS: i64 = 90;
/// Cards whose CI was still running are re-read for this long after they were made.
pub const CI_RECHECK_HOURS: i64 = 6;
/// Most cards one poll re-reads CI for.
pub const CI_RECHECK_CARDS: usize = 10;

#[derive(Default)]
struct Shared {
    /// Spaces to poll at the next wake, ahead of the timer.
    soon: HashSet<String>,
    /// A space backed off after a rate limit is left alone until then.
    resume_at: HashMap<String, DateTime<Utc>>,
    /// The newest tag of each watched repo (by [`tag_key`]), and when its commit was made.
    tags: HashMap<String, (String, Option<DateTime<Utc>>)>,
}

fn shared() -> &'static Mutex<Shared> {
    static S: OnceLock<Mutex<Shared>> = OnceLock::new();
    S.get_or_init(Default::default)
}

fn wake() -> &'static Notify {
    static N: OnceLock<Notify> = OnceLock::new();
    N.get_or_init(Notify::new)
}

fn etags() -> &'static EtagCache {
    static C: OnceLock<EtagCache> = OnceLock::new();
    C.get_or_init(EtagCache::default)
}

/// Polls every repo of `space_id` now instead of at the next tick (the switch went on, a repo
/// was added or moved).
pub fn poll_soon(_state: &AppState, space_id: &str) {
    shared().lock().unwrap().soon.insert(space_id.to_string());
    wake().notify_one();
}

/// Whether `space_id` waits for the next wake, ahead of the timer.
pub fn is_due_soon(space_id: &str) -> bool {
    shared().lock().unwrap().soon.contains(space_id)
}

/// Takes `space_id` off the wake list; true when it was on it.
pub fn take_due_soon(space_id: &str) -> bool {
    shared().lock().unwrap().soon.remove(space_id)
}

fn tag_key(space_id: &str, repo_key: &str) -> String {
    format!("{space_id}\n{repo_key}")
}

/// The newest tag the last poll saw on one watched repo (by its key).
pub fn last_tag(space_id: &str, repo_key: &str) -> Option<String> {
    shared().lock().unwrap().tags.get(&tag_key(space_id, repo_key)).map(|(t, _)| t.clone())
}

/// When that tag's commit was made: release notes cover the repo's landed cards after it.
pub fn last_tag_at(space_id: &str, repo_key: &str) -> Option<DateTime<Utc>> {
    shared().lock().unwrap().tags.get(&tag_key(space_id, repo_key)).and_then(|(_, at)| *at)
}

/// Starts the poller, after picking up what a restart left: pending cards go to `generate`
/// again, and a Fix it whose task never went out is sent.
pub fn spawn(state: AppState, shutdown: CancellationToken) {
    tokio::spawn(async move {
        recover(&state).await;
        let mut next_all = Utc::now();
        loop {
            let now = Utc::now();
            let all = now >= next_all;
            if all {
                next_all = now + Duration::from_std(super::poll_every()).unwrap_or(Duration::minutes(5));
            }
            let soon = std::mem::take(&mut shared().lock().unwrap().soon);
            match state.db.dev_mode_on_spaces().await {
                Ok(rows) => {
                    for row in rows.into_iter().filter(|r| all || soon.contains(&r.space_id)) {
                        if shutdown.is_cancelled() {
                            return;
                        }
                        poll_space(&state, &row.space_id).await;
                    }
                }
                Err(e) => tracing::warn!("dev: could not list Dev mode spaces: {e}"),
            }
            let wait = (next_all - Utc::now()).to_std().unwrap_or_default();
            tokio::select! {
                _ = shutdown.cancelled() => return,
                _ = wake().notified() => {}
                _ = tokio::time::sleep(wait) => {}
            }
        }
    });
}

async fn recover(state: &AppState) {
    match state.db.pending_dev_cards().await {
        Ok(rows) => {
            for r in rows {
                super::generate(state.clone(), r.id).await;
            }
        }
        Err(e) => tracing::warn!("dev: could not read pending cards: {e}"),
    }
    let Ok(spaces) = state.db.dev_mode_on_spaces().await else { return };
    for m in spaces {
        let Ok(cards) = state.db.dev_cards(&m.space_id, REVIEW, None, 200).await else { continue };
        for c in cards {
            let Some(card) = c.card.as_deref().and_then(|j| serde_json::from_str::<serde_json::Value>(j).ok()) else { continue };
            for f in card["findings"].as_array().into_iter().flatten() {
                if f["state"] == "sent" && !f["task_id"].is_string() {
                    if let Some(fid) = f["id"].as_str() {
                        let (state, card_id, fid) = (state.clone(), c.id.clone(), fid.to_string());
                        tokio::spawn(async move {
                            tokio::time::sleep(std::time::Duration::from_secs(super::routes::FIX_UNDO_SECS)).await;
                            super::routes::deliver_fix(&state, &card_id, &fid).await;
                        });
                    }
                }
            }
        }
    }
}

/// Polls every repo `space_id` watches, one after another. A repo that fails records its error
/// and the next one is still read (T5).
pub async fn poll_space(state: &AppState, space_id: &str) {
    match state.db.dev_repos(space_id).await {
        Ok(rows) => {
            for row in rows {
                poll_logged(state, row).await;
            }
        }
        Err(e) => tracing::warn!(space = %space_id, "dev: could not list the watched repos: {e}"),
    }
}

/// [`poll`] with its outcome recorded: the poll error on the repo's row, and a back-off of the
/// space after a rate limit. Public for the leak tests (`tests/dev_github_test.rs`).
pub async fn poll_logged(state: &AppState, row: DevRepoRow) {
    let space = row.space_id.clone();
    if shared().lock().unwrap().resume_at.get(&space).is_some_and(|t| *t > Utc::now()) {
        return;
    }
    let result = poll(state, &row).await;
    let error = match &result {
        Ok(()) => None,
        Err(GhError::Limited { retry_after_s }) => {
            shared().lock().unwrap().resume_at.insert(space.clone(), Utc::now() + Duration::seconds(*retry_after_s as i64));
            Some(GhError::Limited { retry_after_s: *retry_after_s }.to_string())
        }
        Err(e) => Some(e.to_string()),
    };
    if let Some(e) = &error {
        tracing::info!(space = %space, repo = %row.key, "dev: poll failed: {e}");
        let _ = state.db.dev_repo_polled(&space, &row.key, None, Some(e)).await;
    }
}

/// One poll of one watched repo. Merges are recorded as `last_seen_sha` one by one, so a failure
/// part way keeps the ones done. History comes from git ([`History`]: kwant's mirror of the repo,
/// or a team folder's own `.git`) and, on GitHub, from the API when git can't answer; pull
/// requests and checks come from the repo's [`Forge`] (GitHub only, until G4).
pub async fn poll(state: &AppState, mode: &DevRepoRow) -> Result<(), GhError> {
    if mode.repo.trim().is_empty() {
        return Ok(());
    }
    if mode.is_git() {
        return poll_git(state, mode).await;
    }
    // A row whose branch isn't known yet (added while off): reconcile fills it at switch-on
    if mode.branch.trim().is_empty() {
        return Ok(());
    }
    let (client, _) = repo::client_for(state, &mode.repo).await?;
    let mut git_repo = None;
    let (mut url, mut auth) = (String::new(), None);
    if git::available().await {
        if let Ok((u, header)) = client.dev_git_remote(&mode.repo) {
            if let Some(a) = Auth::header(&u, header) {
                git_repo = Repo::mirror(&state.config.data_dir, &mode.space_id, &mode.key).await.ok();
                (url, auth) = (u, Some(a));
            }
        }
    }
    let commit_base = Some(format!("{}/{}/commit", client.dev_web(), mode.repo.trim_matches('/')));
    let mut h = History::new(git_repo, url, auth, Some((&client, mode.repo.as_str())), &mode.branch, commit_base);
    h.fetch().await?;
    let forge = GithubForge { client: &client, repo: &mode.repo };
    read(state, mode, &h, Some(&forge)).await
}

/// A repo GitHub's API can't read (GitLab, Gitea, a bare git server, a team folder with no
/// remote): git only, so only landed cards, without PRs or checks.
async fn poll_git(state: &AppState, mode: &DevRepoRow) -> Result<(), GhError> {
    if !git::available().await {
        return Err(GitError::NoGit.into());
    }
    let from = mode.git_url.as_deref().unwrap_or_default();
    let (g, url) = if from.starts_with('/') {
        (Repo::local(std::path::Path::new(from), &state.config.data_dir).await?, String::new())
    } else {
        let url = git::remote_url(from).filter(|u| u == from).ok_or(GitError::Refused)?;
        (Repo::mirror(&state.config.data_dir, &mode.space_id, &mode.key).await?, url)
    };
    let mut mode = mode.clone();
    if mode.branch.trim().is_empty() {
        mode.branch = g.default_branch(&url, None).await?;
        state.db.set_dev_repo_branch(&mode.space_id, &mode.key, &mode.branch).await.map_err(|e| GhError::Other(e.to_string()))?;
    }
    let base = mode.host.as_deref().and_then(|h| git::commit_base(h, &mode.repo));
    let mut h = History::new(Some(g), url, None, None, &mode.branch, base);
    h.fetch().await?;
    if mode.deploy_hint.is_none() {
        if let Some(hint) = h.root_names().await.as_deref().and_then(repo::deploy_hint) {
            let _ = state.db.set_dev_repo_hint(&mode.space_id, &mode.key, Some(hint)).await;
        }
    }
    read(state, &mode, &h, None).await
}

/// Reads what landed, the open changes and the CI that was still running, and makes their cards:
/// the poll's body over any [`History`] and, when the host has one, its [`Forge`] (the seam a GitLab
/// or Gitea forge plugs into, any-git.md G4). With no forge, only landed cards.
pub async fn read(state: &AppState, mode: &DevRepoRow, h: &History<'_>, forge: Option<&dyn Forge>) -> Result<(), GhError> {
    let space = SpaceId::new(&mode.space_id);
    let kwants = state.db.list_kwants(&space).await.unwrap_or_default();
    let cache = etags();

    // The newest tag, for release notes
    update_tag(h, mode).await;

    // What landed
    let (commits, found) = h.commits_since(mode.last_seen_sha.as_deref(), cache).await?;
    let mut last = None;
    let mut skipped: Option<String> = None;
    for c in window(commits, mode.last_seen_sha.is_none(), found) {
        // A rebase-merge's earlier commits are skipped; the PR's card shows what they all changed
        let base = skipped.take();
        if !landed(state, h, forge, mode, &kwants, &c, base.as_deref()).await? {
            skipped = base.or_else(|| c.parents.first().cloned());
        }
        last = Some(c.sha.clone());
    }
    state.db.dev_repo_polled(&mode.space_id, &mode.key, last.as_deref(), None).await.map_err(|e| GhError::Other(e.to_string()))?;

    let Some(forge) = forge else { return Ok(()) };
    // Open PRs
    let pulls = forge.open_changes(&mode.branch, cache).await?;
    reviews(state, h, forge, mode, &kwants, &pulls).await?;

    // CI that was still running
    recheck_ci(state, forge, mode).await;
    Ok(())
}

async fn update_tag(h: &History<'_>, mode: &DevRepoRow) {
    let key = tag_key(&mode.space_id, &mode.key);
    let known = shared().lock().unwrap().tags.get(&key).cloned();
    match h.latest_tag(known.as_ref(), etags()).await {
        Ok(Some(t)) => {
            shared().lock().unwrap().tags.insert(key, t);
        }
        Ok(None) => {
            shared().lock().unwrap().tags.remove(&key);
        }
        Err(_) => {}
    }
}

/// Which of the new commits (oldest first) become cards: the newest [`BACKFILL`] on a space's
/// first poll, else up to [`MAX_NEW`] (the newest, when there were more or `last_seen` was lost).
pub fn window(commits: Vec<Commit>, first: bool, _found: bool) -> Vec<Commit> {
    let keep = if first { BACKFILL } else { MAX_NEW };
    let skip = commits.len().saturating_sub(keep);
    commits.into_iter().skip(skip).collect()
}

/// The merged PR a commit on the branch is the merge (or squash) commit of. `Err(())` when the
/// commit is one of several a rebase-merge put on the branch and isn't the last: the PR's card
/// is made from its merge commit, so this one is skipped.
pub(crate) fn merged_pr<'a>(pulls: &'a [Change], sha: &str, branch: &str) -> Result<Option<&'a Change>, ()> {
    let merged: Vec<&Change> = pulls.iter().filter(|p| p.merged_at.is_some() && p.base_branch == branch).collect();
    if let Some(p) = merged.iter().find(|p| p.merge_sha.as_deref() == Some(sha)) {
        return Ok(Some(p));
    }
    if merged.is_empty() {
        Ok(None)
    } else {
        Err(())
    }
}

/// Makes the landed card of commit `c`. False when `c` is one of a rebase-merge's earlier commits
/// and was skipped; `base` is then where the next card's files start (the commit before the first
/// skipped one), so the PR's card shows all it changed.
async fn landed(state: &AppState, h: &History<'_>, forge: Option<&dyn Forge>, mode: &DevRepoRow, kwants: &[Kwant], c: &Commit, base: Option<&str>) -> Result<bool, GhError> {
    if state.db.dev_cards_of_ref_in(&mode.space_id, &mode.key, LANDED, &c.sha).await.map(|v| !v.is_empty()).unwrap_or(false) {
        return Ok(true);
    }
    let pulls = match forge {
        Some(f) => f.changes_for_commit(&c.sha).await.unwrap_or_default(),
        None => Vec::new(),
    };
    let Ok(pr) = merged_pr(&pulls, &c.sha, &mode.branch) else { return Ok(false) };
    let mut full = h.commit(&c.sha).await?;
    let files = match (h.has_git(), pr, forge) {
        (true, _, _) => match base {
            Some(b) => h.landed_files(Some(b), &c.sha).await.unwrap_or_else(|| full.files.clone()),
            None => full.files.clone(),
        },
        (false, Some(p), Some(f)) => f.change_files(p.number).await?,
        _ => full.files.clone(),
    };
    // Git knows no forge accounts: a direct push asks the forge who wrote it
    if full.login.is_none() && pr.is_none() && h.has_git() {
        if let Some(f) = forge {
            full.login = f.commit_login(&c.sha).await;
        }
    }
    let checks = match forge {
        Some(f) => f.checks(&c.sha, etags()).await.unwrap_or_default(),
        None => Vec::new(),
    };
    let until = pr.and_then(|p| p.merged_at).or(full.committer.date).unwrap_or_else(Utc::now) - Duration::seconds(1);
    let first = first_edits(h, &files, until).await;
    let facts = landed_facts(mode, &full, pr, files, checks, kwants, first);
    insert(state, mode, LANDED, &c.sha, &c.sha, &facts).await;
    Ok(true)
}

async fn reviews(state: &AppState, h: &History<'_>, forge: &dyn Forge, mode: &DevRepoRow, kwants: &[Kwant], pulls: &[Change]) -> Result<(), GhError> {
    let mut open = HashSet::new();
    let mut read = 0;
    for p in pulls {
        let ref_ = format!("pr:{}", p.number);
        open.insert(ref_.clone());
        if p.draft {
            // Drafts are skipped (D8); a PR that went back to draft loses its card
            stale(state, mode, &ref_, None).await;
            continue;
        }
        let have = state.db.dev_cards_of_ref_in(&mode.space_id, &mode.key, REVIEW, &ref_).await.unwrap_or_default();
        if have.iter().any(|c| c.head_sha == p.head_sha) {
            stale(state, mode, &ref_, Some(&p.head_sha)).await;
            continue;
        }
        if read >= MAX_REVIEWS {
            continue;
        }
        read += 1;
        let head_ref = forge.change_head_ref(p.number);
        let mut files: Vec<FileDiff> = match h.change_files(head_ref.as_deref(), p.number, &p.head_sha, &p.base_branch).await {
            Some(f) => f,
            None => forge.change_files(p.number).await?,
        };
        let head = h.commit(&p.head_sha).await.ok();
        let checks: Vec<Check> = forge.checks(&p.head_sha, etags()).await.unwrap_or_default();
        let first = first_edits(h, &files, Utc::now()).await;
        // The whole diff is scanned before the 60k cut, and secret lines are hidden before
        // anything is stored or shown (Juno, plan §3.4)
        let capped = files.len() >= crate::zombie::github::MAX_PR_FILES;
        let security = super::scan::for_review(&state.config.data_dir, h, &p.head_sha, &mut files, capped).await;
        let mut facts = review_facts(mode, p, head.as_ref(), files, checks, kwants, first);
        facts.security = Some(security);
        insert(state, mode, REVIEW, &ref_, &p.head_sha, &facts).await;
        stale(state, mode, &ref_, Some(&p.head_sha)).await;
    }
    // A PR of this repo that closed or merged: its review cards go stale
    if let Ok(cards) = state.db.dev_cards_in(&mode.space_id, Some(&mode.key), REVIEW, None, 200).await {
        let gone: HashSet<String> = cards.into_iter().filter(|c| c.status != STALE && !open.contains(&c.ref_)).map(|c| c.ref_).collect();
        for r in gone {
            stale(state, mode, &r, None).await;
        }
    }
    Ok(())
}

async fn stale(state: &AppState, mode: &DevRepoRow, ref_: &str, keep: Option<&str>) {
    for id in state.db.stale_dev_reviews_in(&mode.space_id, &mode.key, ref_, keep).await.unwrap_or_default() {
        if let Ok(Some(row)) = state.db.dev_card(&id).await {
            super::notify_card(state, &row);
        }
    }
}

async fn insert(state: &AppState, mode: &DevRepoRow, kind: &str, ref_: &str, head: &str, facts: &DevFacts) {
    let space_id = &mode.space_id;
    let Ok(json) = serde_json::to_string(facts) else { return };
    match state.db.insert_dev_card_in(space_id, &mode.key, kind, ref_, head, &json).await {
        Ok(Some(row)) => {
            super::notify_card(state, &row);
            tokio::spawn(super::generate(state.clone(), row.id));
        }
        Ok(None) => {}
        Err(e) => tracing::warn!(space = %space_id, "dev: could not save a card: {e}"),
    }
}

/// Landed and review cards made in the last [`CI_RECHECK_HOURS`] whose CI was pending: read the
/// checks again, and when they changed, store them and run `generate` again.
async fn recheck_ci(state: &AppState, forge: &dyn Forge, mode: &DevRepoRow) {
    let since = Utc::now() - Duration::hours(CI_RECHECK_HOURS);
    let mut rows: Vec<DevCardRow> = Vec::new();
    for kind in [LANDED, REVIEW] {
        rows.extend(state.db.dev_cards_in(&mode.space_id, Some(&mode.key), kind, None, 50).await.unwrap_or_default());
    }
    let mut done = 0;
    for row in rows.into_iter().filter(|r| r.created_at >= since && r.status != STALE && r.status != PENDING) {
        if done >= CI_RECHECK_CARDS {
            break;
        }
        let Ok(mut facts) = serde_json::from_str::<DevFacts>(&row.facts) else { continue };
        if facts.ci != CiState::Pending {
            continue;
        }
        done += 1;
        let Ok(checks) = forge.checks(&row.head_sha, etags()).await else { continue };
        let checks = to_checks(checks);
        if checks == facts.checks {
            continue;
        }
        facts.ci = CiState::of(&checks);
        facts.links.ci = ci_link(&checks);
        facts.checks = checks;
        let Ok(json) = serde_json::to_string(&facts) else { continue };
        if let Ok(Some(r)) = state.db.set_dev_card_facts(&row.id, &json).await {
            super::notify_card(state, &r);
            tokio::spawn(super::generate(state.clone(), r.id));
        }
    }
}

/// The changed files (not new ones) nobody committed to in the last [`FIRST_EDIT_DAYS`] before
/// `until`, reading at most [`FIRST_EDIT_FILES`] histories.
async fn first_edits(h: &History<'_>, files: &[FileDiff], until: DateTime<Utc>) -> Vec<String> {
    let since = until - Duration::days(FIRST_EDIT_DAYS);
    let mut out = Vec::new();
    for f in files.iter().filter(|f| f.status != "added").take(FIRST_EDIT_FILES) {
        let path = f.previous_path.as_deref().unwrap_or(&f.path);
        if let Ok(0) = h.path_commits(path, since, until).await {
            out.push(f.path.clone());
        }
    }
    out
}

/// The changed files with their patches, cut so the patches add up to at most [`DIFF_CHARS`]: a
/// patch that doesn't fit is dropped whole, and `cut` says how many files weren't read.
pub fn cut_diffs(files: Vec<FileDiff>) -> (Vec<FileDiff>, Option<String>) {
    let mut left = DIFF_CHARS;
    let mut unread = 0;
    let out = files
        .into_iter()
        .map(|f| {
            let patch = match f.patch {
                Some(p) if p.chars().count() <= left => {
                    left -= p.chars().count();
                    Some(p)
                }
                Some(_) => {
                    unread += 1;
                    None
                }
                None => None,
            };
            FileDiff { patch, ..f }
        })
        .collect();
    let cut = (unread > 0).then(|| format!("diff cut, {unread} {} not read", if unread == 1 { "file" } else { "files" }));
    (out, cut)
}

/// The kwant bot that wrote a commit (D7): git author name `Lin` or `Lin (kwant bot)`, or an
/// email whose local part is `lin-bot`, matched case-insensitively to a bot in the space.
pub fn match_bot(name: &str, email: Option<&str>, kwants: &[Kwant]) -> Option<KwantRef> {
    let name = name.trim();
    let bare = name.strip_suffix("(kwant bot)").map(str::trim).unwrap_or(name);
    let local = email.and_then(|e| e.split('@').next()).map(str::to_lowercase);
    let slug = |n: &str| n.trim().to_lowercase().split_whitespace().collect::<Vec<_>>().join("-");
    let hit = kwants
        .iter()
        .find(|k| !k.name.trim().is_empty() && k.name.trim().eq_ignore_ascii_case(bare))
        .or_else(|| kwants.iter().find(|k| local.as_deref().is_some_and(|l| !k.name.trim().is_empty() && l == format!("{}-bot", slug(&k.name)))))?;
    // The engine family and model from the bot's pin, as the reviewer's are (review.rs); the driver
    // type ("cli", "api") isn't an engine, so comparing it never found "another engine"
    let pin = crate::router::parse_pin(&hit.preferred_model);
    let engine = pin.family.unwrap_or_else(|| "auto".into());
    let model = pin.model.unwrap_or_else(|| "auto".into());
    Some(KwantRef { id: hit.id.clone(), name: hit.name.clone(), engine, model })
}

/// The commit message's first line, and the rest.
pub fn split_message(msg: &str) -> (String, String) {
    let mut it = msg.splitn(2, '\n');
    let title = it.next().unwrap_or("").trim().to_string();
    let body = it.next().unwrap_or("").trim().to_string();
    (title, body)
}

fn https(u: Option<&str>) -> Option<String> {
    u.filter(|u| u.starts_with("https://")).map(str::to_string)
}

/// The checks with only https links, sorted by name so a re-read compares equal.
fn to_checks(cs: Vec<Check>) -> Vec<Check> {
    let mut v: Vec<Check> = cs.into_iter().map(|c| Check { url: https(c.url.as_deref()), ..c }).collect();
    v.sort_by(|a, b| a.name.cmp(&b.name));
    v
}

/// The CI link: the first failed check's page, else the first check's.
fn ci_link(checks: &[Check]) -> Option<String> {
    let bad = |c: &&Check| matches!(c.conclusion.as_deref(), Some("failure" | "timed_out" | "cancelled" | "action_required" | "startup_failure"));
    checks.iter().find(bad).or(checks.first()).and_then(|c| c.url.clone())
}

fn author_of(commit: Option<&Commit>, pr: Option<&Change>, kwants: &[Kwant]) -> Author {
    let login = pr.and_then(|p| p.author.clone()).or(commit.and_then(|c| c.login.clone()));
    let name = commit.map(|c| c.author.name.clone()).filter(|n| !n.trim().is_empty()).or_else(|| login.clone()).unwrap_or_default();
    let email = commit.and_then(|c| c.author.email.clone());
    let bot = match_bot(&name, email.as_deref(), kwants);
    Author { login, name, email, bot }
}

pub fn landed_facts(mode: &DevRepoRow, c: &Commit, pr: Option<&Change>, files: Vec<FileDiff>, checks: Vec<Check>, kwants: &[Kwant], first: Vec<String>) -> DevFacts {
    let (msg_title, msg_body) = split_message(&c.message);
    let (files, cut) = cut_diffs(files);
    let checks = to_checks(checks);
    DevFacts {
        repo: mode.repo.clone(),
        kind: Kind::Landed,
        pr: pr.map(|p| p.number),
        title: pr.map(|p| p.title.trim().to_string()).filter(|t| !t.is_empty()).unwrap_or(msg_title),
        body: pr.and_then(|p| p.body.clone()).unwrap_or(msg_body),
        head_sha: c.sha.clone(),
        base_branch: mode.branch.clone(),
        merged_at: pr.and_then(|p| p.merged_at).or(c.committer.date),
        merged_by: pr.and_then(|p| p.merged_by.clone()),
        author: author_of(Some(c), pr, kwants),
        files,
        cut,
        ci: CiState::of(&checks),
        links: Links { pr: https(pr.and_then(|p| p.url.as_deref())), commit: https(c.url.as_deref()), ci: ci_link(&checks) },
        security: None,
        checks,
        task: None,
        first_edit_files: first,
        draft: false,
        head_branch: None,
        pr_head_sha: pr.map(|p| p.head_sha.clone()),
    }
}

pub fn review_facts(mode: &DevRepoRow, p: &Change, head: Option<&Commit>, files: Vec<FileDiff>, checks: Vec<Check>, kwants: &[Kwant], first: Vec<String>) -> DevFacts {
    let (files, cut) = cut_diffs(files);
    let checks = to_checks(checks);
    DevFacts {
        repo: mode.repo.clone(),
        kind: Kind::Review,
        pr: Some(p.number),
        title: p.title.trim().to_string(),
        body: p.body.clone().unwrap_or_default(),
        head_sha: p.head_sha.clone(),
        base_branch: p.base_branch.clone(),
        merged_at: None,
        merged_by: None,
        author: author_of(head, Some(p), kwants),
        files,
        cut,
        ci: CiState::of(&checks),
        links: Links { pr: https(p.url.as_deref()), commit: https(head.and_then(|c| c.url.as_deref())), ci: ci_link(&checks) },
        security: None,
        checks,
        task: None,
        first_edit_files: first,
        draft: p.draft,
        head_branch: Some(p.head_branch.clone()),
        pr_head_sha: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn file(name: &str, patch_len: Option<usize>) -> FileDiff {
        FileDiff { path: name.into(), status: "modified".into(), additions: 3, deletions: 1, patch: patch_len.map(|n| "x".repeat(n)), previous_path: None }
    }

    fn kwant(id: &str, name: &str) -> Kwant {
        serde_json::from_value(json!({
            "id": id, "name": name, "title": "Engineer", "persona_prompt": "", "soul": "", "avatar_sprite": "",
            "driver_type": "cli", "preferred_model": "claude/demo-model", "temperature": 0.0, "max_tokens": 0,
            "level": 1, "turns_completed": 0, "created_at": null, "space_id": "s1"
        }))
        .unwrap()
    }

    /// Built from GitHub's JSON, so the conversion is under test too.
    fn pull(n: u64, merge: Option<&str>, merged: bool, base: &str) -> Change {
        serde_json::from_value::<crate::zombie::github::GhPull>(json!({
            "number": n, "title": format!("PR {n}"), "state": if merged { "closed" } else { "open" }, "draft": false,
            "head": {"ref": "feature", "sha": "h1"}, "base": {"ref": base, "sha": "b1"},
            "merged_at": if merged { json!("2026-11-05T01:00:00Z") } else { json!(null) },
            "merge_commit_sha": merge,
        }))
        .unwrap()
        .into()
    }

    fn commit(sha: &str) -> Commit {
        serde_json::from_value::<crate::zombie::github::GhCommit>(json!({
            "sha": sha, "html_url": format!("https://github.com/demo-org/app/commit/{sha}"),
            "commit": {"message": "Fix the thing\n\nLonger words.", "author": {"name": "Lin (kwant bot)", "email": "lin-bot@users.noreply.example.com", "date": "2026-11-05T01:00:00Z"},
                       "committer": {"name": "GitHub", "email": "noreply@example.com", "date": "2026-11-05T01:00:00Z"}},
            "parents": []
        }))
        .unwrap()
        .into()
    }

    #[test]
    fn diffs_are_cut_at_sixty_thousand_characters() {
        let (files, cut) = cut_diffs(vec![file("a.rs", Some(40_000)), file("b.rs", Some(30_000)), file("c.rs", Some(20_000)), file("logo.png", None)]);
        assert!(files[0].patch.is_some());
        assert!(files[1].patch.is_none(), "40k + 30k is past 60k: b is dropped whole");
        assert!(files[2].patch.is_some(), "a later smaller patch still fits");
        assert!(files[3].patch.is_none());
        assert_eq!(cut.as_deref(), Some("diff cut, 1 file not read"));
        let total: usize = files.iter().filter_map(|f| f.patch.as_ref()).map(|p| p.len()).sum();
        assert!(total <= DIFF_CHARS);
        let (_, cut) = cut_diffs(vec![file("a.rs", Some(60_000))]);
        assert_eq!(cut, None, "exactly 60k fits");
        let (_, cut) = cut_diffs(vec![file("a.rs", Some(60_001)), file("b.rs", Some(60_001))]);
        assert_eq!(cut.as_deref(), Some("diff cut, 2 files not read"));
    }

    #[test]
    fn authors_match_bots_by_name_or_bot_email() {
        let ks = [kwant("k1", "Lin"), kwant("k2", "Flix Dev"), kwant("k3", "")];
        assert_eq!(match_bot("Lin (kwant bot)", None, &ks).unwrap().id, "k1");
        assert_eq!(match_bot("lin", None, &ks).unwrap().id, "k1");
        assert_eq!(match_bot("Someone", Some("flix-dev-bot@users.noreply.example.com"), &ks).unwrap().id, "k2");
        let r = match_bot("Lin", None, &ks).unwrap();
        assert_eq!((r.engine.as_str(), r.model.as_str()), ("claude", "demo-model"));
        assert_eq!(match_bot("Oleks Demo", Some("person@example.com"), &ks), None, "a person isn't a bot");
        assert_eq!(match_bot("", Some("-bot@example.com"), &ks), None, "an unnamed bot never matches");
        assert_eq!(match_bot("Linus", Some("linus@example.com"), &ks), None);
    }

    #[test]
    fn the_first_poll_backfills_five_and_later_ones_twenty() {
        let cs: Vec<Commit> = (0..30).map(|i| commit(&format!("c{i:02}"))).collect();
        let w = window(cs.clone(), true, false);
        assert_eq!(w.iter().map(|c| c.sha.as_str()).collect::<Vec<_>>(), ["c25", "c26", "c27", "c28", "c29"]);
        let w = window(cs.clone(), false, true);
        assert_eq!(w.len(), MAX_NEW);
        assert_eq!(w.last().unwrap().sha, "c29", "the newest are kept");
        assert_eq!(window(cs[..3].to_vec(), false, true).len(), 3);
    }

    #[test]
    fn a_merge_commit_finds_its_pr_and_rebased_commits_are_skipped() {
        let squash = [pull(7, Some("m1"), true, "main")];
        assert_eq!(merged_pr(&squash, "m1", "main").unwrap().unwrap().number, 7);
        assert!(merged_pr(&squash, "earlier", "main").is_err(), "an earlier commit of a rebase-merge");
        assert!(merged_pr(&[pull(8, Some("m2"), true, "release")], "m2", "main").unwrap().is_none(), "a PR into another branch");
        assert!(merged_pr(&[pull(9, None, false, "main")], "x", "main").unwrap().is_none(), "an open PR containing the commit");
        assert!(merged_pr(&[], "direct", "main").unwrap().is_none(), "a direct push");
    }

    #[test]
    fn landed_facts_prefer_the_pr_and_keep_links_https() {
        let mode = DevRepoRow {
            space_id: "s1".into(),
            key: "github.com/demo-org/app".into(),
            team: None,
            provider: "github".into(),
            repo: "demo-org/app".into(),
            host: Some("github.com".into()),
            branch: "main".into(),
            deploy_hint: None,
            last_seen_sha: None,
            polled_at: None,
            poll_error: None,
            added_by: "team".into(),
            git_url: None,
            added_at: Utc::now(),
        };
        let c = commit("m1");
        let mut p = pull(7, Some("m1"), true, "main");
        p.url = Some("javascript:alert(1)".into());
        let checks: Vec<Check> = crate::server::dev::source::all(serde_json::from_value::<Vec<crate::zombie::github::GhCheck>>(json!([
            {"name": "rust", "status": "completed", "conclusion": "failure", "html_url": "https://github.com/demo-org/app/actions/runs/1"},
            {"name": "frontend", "status": "completed", "conclusion": "success", "html_url": "https://github.com/demo-org/app/actions/runs/2"}
        ]))
        .unwrap());
        let f = landed_facts(&mode, &c, Some(&p), vec![file("src/a.rs", Some(10))], checks, &[kwant("k1", "Lin")], vec!["src/a.rs".into()]);
        assert_eq!(f.pr, Some(7));
        assert_eq!(f.title, "PR 7");
        assert_eq!(f.ci, CiState::Red);
        assert_eq!(f.links.pr, None, "a non-https link is dropped");
        assert_eq!(f.links.ci.as_deref(), Some("https://github.com/demo-org/app/actions/runs/1"), "the failing check");
        assert_eq!(f.author.bot.unwrap().id, "k1");
        assert_eq!(f.checks[0].name, "frontend", "checks are sorted so a re-read compares equal");
        let f = landed_facts(&mode, &c, None, vec![], vec![], &[], vec![]);
        assert_eq!((f.title.as_str(), f.body.as_str()), ("Fix the thing", "Longer words."));
        assert_eq!(f.ci, CiState::None);
        assert_eq!(f.links.commit.as_deref(), Some("https://github.com/demo-org/app/commit/m1"));
    }
}
