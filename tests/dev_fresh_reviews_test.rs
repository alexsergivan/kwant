//! Dev mode reviews only fresh work (0.7.7, dev-fresh-reviews/brief.md): a pushed branch or open
//! PR gets a review card only while its last commit (a PR's last update) is at most
//! `KWANT_DEV_REVIEW_DAYS` (default 7) days old. Older work gets no card and no model call, its
//! old card goes stale, and a new push makes it fresh again. Candidates are read newest first under
//! the per-poll cap, and the status says how many were skipped.
//!
//! Git repos are throwaway `git init`s in a temp dir with back-dated commits; nothing touches the
//! network or any remote. CI sets no `KWANT_DEV_REVIEW_DAYS`, so the window is 7 days.

mod common;

use chrono::{DateTime, Duration, Utc};
use common::*;
use kwnt::db::dev_mode::{DevCardRow, DevRepoRow, BY_HAND, REVIEW, STALE};
use kwnt::db::models::NewTeam;
use kwnt::db::spaces::SpaceId;
use kwnt::server::dev::facts::{Check, DevFacts, FileDiff};
use kwnt::server::dev::forge::ForgeKind;
use kwnt::server::dev::git::Repo;
use kwnt::server::dev::poller;
use kwnt::server::dev::source::{Change, Forge, History};
use kwnt::zombie::github::{EtagCache, GhError, GhReview, PostError, ReviewPost};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};

/// git in `dir`, with author and committer dates set to `at` when given.
fn git_at(dir: &Path, args: &[&str], at: Option<DateTime<Utc>>) -> String {
    let mut cmd = std::process::Command::new("git");
    cmd.arg("-C")
        .arg(dir)
        .args(["-c", "user.name=Vega (kwant bot)", "-c", "user.email=vega-bot@users.noreply.example.com", "-c", "commit.gpgsign=false"])
        .args(args)
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GIT_CONFIG_NOSYSTEM", "1");
    if let Some(at) = at {
        let at = format!("{} +0000", at.timestamp());
        cmd.env("GIT_AUTHOR_DATE", &at).env("GIT_COMMITTER_DATE", &at);
    }
    let out = cmd.output().unwrap();
    assert!(out.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&out.stderr));
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

fn days_ago(d: i64) -> DateTime<Utc> {
    Utc::now() - Duration::days(d)
}

fn git(dir: &Path, args: &[&str]) -> String {
    git_at(dir, args, None)
}

/// A commit of `file` dated `days` days back (now when `None`).
fn commit_at(dir: &Path, file: &str, text: &str, message: &str, days: Option<i64>) -> String {
    std::fs::write(dir.join(file), text).unwrap();
    git(dir, &["add", "--", file]);
    git_at(dir, &["commit", "-q", "-m", message], days.map(days_ago));
    git(dir, &["rev-parse", "HEAD"])
}

/// Branch `name` off main with one commit dated `days` days back; returns its head.
fn branch(dir: &Path, name: &str, days: Option<i64>) -> String {
    git(dir, &["checkout", "-q", "-b", name, "main"]);
    let sha = commit_at(dir, &format!("{name}.txt"), &format!("{name}\n"), &format!("Work on {name}"), days);
    git(dir, &["checkout", "-q", "main"]);
    sha
}

/// A team whose folder is a repo on `main` (its first commit 60 days old), watched by Dev mode
/// with no forge: branches get the no-forge review cards.
async fn watched(env: &Env, name: &str) -> (PathBuf, DevRepoRow) {
    let space = SpaceId::default_space();
    let folder = env.tmp.path().join("folders").join(name);
    std::fs::create_dir_all(&folder).unwrap();
    git(&folder, &["init", "-q", "-b", "main"]);
    commit_at(&folder, "README.md", "# App\n", "Start the repo", Some(60));
    let t = NewTeam {
        name: name.into(),
        description: None,
        icon: None,
        lead_kwant_id: env.a.clone(),
        member_kwants: vec![env.a.clone()],
        workspace_path: Some(folder.to_string_lossy().to_string()),
        repo_url: None,
    };
    env.state.db.create_team(&space, &t).await.unwrap();
    let (s, v) = call(&env.app, "PUT", &format!("/api/spaces/{}/dev_mode", space.as_str()), Some(json!({"on": true}))).await;
    assert_eq!(s.as_u16(), 200, "{v}");
    let row = env.state.db.dev_repo(space.as_str(), &format!("local:{name}")).await.unwrap().unwrap();
    (folder, row)
}

/// Every review card of the repo, oldest first.
async fn cards(env: &Env, key: &str) -> Vec<DevCardRow> {
    let mut rows = env.state.db.dev_cards_in(SpaceId::default_space().as_str(), Some(key), REVIEW, None, 50).await.unwrap();
    rows.sort_by(|a, b| a.created_at.cmp(&b.created_at).then(a.id.cmp(&b.id)));
    rows
}

/// The refs of the repo's live (not stale) review cards, sorted.
async fn live(env: &Env, key: &str) -> Vec<String> {
    let mut refs: Vec<String> = cards(env, key).await.into_iter().filter(|c| c.status != STALE).map(|c| c.ref_).collect();
    refs.sort();
    refs
}

async fn status(env: &Env) -> Value {
    let (s, v) = call(&env.app, "GET", &format!("/api/spaces/{}/dev_mode", SpaceId::default_space().as_str()), None).await;
    assert_eq!(s.as_u16(), 200, "{v}");
    v
}

async fn poll(env: &Env) {
    poller::poll_space(&env.state, SpaceId::default_space().as_str()).await;
}

/// Oleks's desk, 2026-11-08: every unmerged branch of a GitLab repo watched with plain git got a
/// review, however old, until the day's 30 model calls were gone. Now a 30-day-old branch gets no
/// card (so no model call), a 1-day-old one does, the status says one was skipped, and a new push
/// to the old branch makes it fresh.
#[tokio::test]
async fn an_old_branch_gets_no_review_until_someone_pushes_to_it() {
    let env = setup().await;
    let (folder, row) = watched(&env, "fresh-a").await;
    let old = branch(&folder, "kc-8125", Some(30));
    let new = branch(&folder, "kc-9001", Some(1));
    // Six days and a bit is still inside the window
    branch(&folder, "kc-9000", Some(6));

    poll(&env).await;
    let all = cards(&env, &row.key).await;
    assert!(all.iter().all(|c| c.ref_ != "branch:kc-8125" && c.head_sha != old), "no card at all for the old branch, so no generate: {all:?}");
    assert_eq!(live(&env, &row.key).await, ["branch:kc-9000", "branch:kc-9001"]);
    assert!(all.iter().any(|c| c.head_sha == new));

    // The status says the window and how many it skipped, per repo and at the top
    let v = status(&env).await;
    assert_eq!((v["repos"][0]["fresh_days"].clone(), v["repos"][0]["old_reviews_skipped"].clone()), (json!(7), json!(1)), "{v}");
    assert_eq!((v["fresh_days"].clone(), v["old_reviews_skipped"].clone()), (json!(7), json!(1)), "{v}");

    // Still old on the next poll: still nothing
    poll(&env).await;
    assert!(cards(&env, &row.key).await.iter().all(|c| c.ref_ != "branch:kc-8125"));

    // Someone pushes to the old branch: it's fresh, so it gets its card, and nothing is skipped
    git(&folder, &["checkout", "-q", "kc-8125"]);
    let pushed = commit_at(&folder, "kc-8125.txt", "kc-8125\nmore\n", "Pick KC-8125 up again", None);
    git(&folder, &["checkout", "-q", "main"]);
    poll(&env).await;
    let all = cards(&env, &row.key).await;
    let c = all.iter().find(|c| c.ref_ == "branch:kc-8125").expect("a card after the push");
    assert_eq!(c.head_sha, pushed);
    let f: DevFacts = serde_json::from_str(&c.facts).unwrap();
    assert_eq!(f.files.iter().map(|x| x.path.as_str()).collect::<Vec<_>>(), ["kc-8125.txt"], "the whole branch against main, old commit included");
    assert_eq!(live(&env, &row.key).await, ["branch:kc-8125", "branch:kc-9000", "branch:kc-9001"]);
    assert_eq!(status(&env).await["repos"][0]["old_reviews_skipped"], 0);
}

/// The 27 cards on the desk drop by themselves after the upgrade: a review card made before, on a
/// branch with no commit in 7 days, goes stale on the next poll, with no new card and no migration.
#[tokio::test]
async fn a_card_on_a_branch_that_went_quiet_goes_stale() {
    let env = setup().await;
    let (folder, row) = watched(&env, "fresh-b").await;
    let old = branch(&folder, "kc-7000", Some(30));
    let fresh = branch(&folder, "kc-7001", Some(2));
    // Cards as 0.7.6 left them, for both heads
    let space = SpaceId::default_space();
    for (r, sha) in [("branch:kc-7000", &old), ("branch:kc-7001", &fresh)] {
        env.state.db.insert_dev_card_in(space.as_str(), &row.key, REVIEW, r, sha, "{}").await.unwrap().unwrap();
    }
    assert_eq!(live(&env, &row.key).await, ["branch:kc-7000", "branch:kc-7001"]);

    poll(&env).await;
    assert_eq!(live(&env, &row.key).await, ["branch:kc-7001"], "the old branch's card is stale; the fresh one keeps its card");
    let all = cards(&env, &row.key).await;
    assert_eq!(all.len(), 2, "no new card for either: {all:?}");
    assert_eq!(all.iter().find(|c| c.ref_ == "branch:kc-7000").unwrap().status, STALE);
    assert_eq!(status(&env).await["repos"][0]["old_reviews_skipped"], 1);
}

/// At most MAX_REVIEWS branches are read per poll, and the freshest go first: name order would
/// have read b00..b09, the oldest ten.
#[tokio::test]
async fn branches_are_read_newest_first_under_the_cap() {
    assert_eq!(poller::MAX_REVIEWS, 10);
    let env = setup().await;
    let (folder, row) = watched(&env, "fresh-c").await;
    // b00 is the oldest (5 days and 11 hours), b11 the newest (5 days)
    for i in 0..12i64 {
        let name = format!("b{i:02}");
        git(&folder, &["checkout", "-q", "-b", &name, "main"]);
        std::fs::write(folder.join(format!("{name}.txt")), "x\n").unwrap();
        git(&folder, &["add", "--", &format!("{name}.txt")]);
        git_at(&folder, &["commit", "-q", "-m", &name], Some(days_ago(5) - Duration::hours(11 - i)));
        git(&folder, &["checkout", "-q", "main"]);
    }
    poll(&env).await;
    let want: Vec<String> = (2..12).map(|i| format!("branch:b{i:02}")).collect();
    assert_eq!(live(&env, &row.key).await, want, "the ten newest");
    // The two oldest wait for the next poll, and none was skipped as old
    assert_eq!(status(&env).await["repos"][0]["old_reviews_skipped"], 0);
    poll(&env).await;
    assert_eq!(live(&env, &row.key).await.len(), 12);
}

struct Fake {
    open: Vec<Change>,
}

#[async_trait::async_trait]
impl Forge for Fake {
    async fn changes_for_commit(&self, _sha: &str) -> Result<Vec<Change>, GhError> {
        Ok(vec![])
    }
    async fn open_changes(&self, _base: &str, _cache: &EtagCache) -> Result<Vec<Change>, GhError> {
        Ok(self.open.clone())
    }
    async fn checks(&self, _sha: &str, _cache: &EtagCache) -> Result<Vec<Check>, GhError> {
        Ok(vec![])
    }
    async fn change_files(&self, _number: u64) -> Result<Vec<FileDiff>, GhError> {
        Ok(vec![])
    }
    fn change_head_ref(&self, number: u64) -> Option<String> {
        Some(format!("refs/pull/{number}/head"))
    }
    async fn commit_login(&self, _sha: &str) -> Option<String> {
        None
    }
    fn kind(&self) -> ForgeKind {
        ForgeKind::Gitlab
    }
    async fn change(&self, number: u64) -> Result<Option<Change>, GhError> {
        Ok(self.open.iter().find(|c| c.number == number).cloned())
    }
    async fn login(&self) -> Result<String, GhError> {
        Ok("demo-bot".into())
    }
    async fn post_review(&self, _number: u64, _post: &ReviewPost) -> Result<GhReview, PostError> {
        unreachable!("nothing posts in this test")
    }
}

/// With a forge, a PR is fresh by its `updated_at`, else by its head commit's date; an unknown
/// date counts as fresh. Old PRs get no card and their old cards go stale.
#[tokio::test]
async fn forge_prs_are_reviewed_only_while_updated_in_the_window() {
    let env = setup().await;
    let space = SpaceId::default_space();
    let dir = env.tmp.path().join("mr");
    std::fs::create_dir_all(&dir).unwrap();
    git(&dir, &["init", "-q", "-b", "main"]);
    commit_at(&dir, "README.md", "# App\n", "Start", Some(60));
    let h20 = branch(&dir, "quiet", None);
    let h21 = branch(&dir, "busy", Some(40));
    let h22 = branch(&dir, "old-head", Some(30));
    let h23 = branch(&dir, "new-head", Some(1));
    let h24 = branch(&dir, "nothing-known", None);
    let change = |n: u64, head_branch: &str, head_sha: &str, days: Option<i64>| Change {
        number: n,
        title: format!("MR {n}"),
        url: Some(format!("https://gitlab.example.com/webc/app/-/merge_requests/{n}")),
        head_branch: head_branch.into(),
        head_sha: head_sha.into(),
        base_branch: "main".into(),
        updated_at: days.map(days_ago),
        ..Default::default()
    };
    let fake = Fake {
        open: vec![
            // Updated 30 days ago, though its head commit is new: old
            change(20, "quiet", &h20, Some(30)),
            // A comment yesterday on an old commit: fresh
            change(21, "busy", &h21, Some(1)),
            // No updated_at: its head commit decides, 30 days old
            change(22, "old-head", &h22, None),
            // No updated_at, head commit a day old: fresh
            change(23, "new-head", &h23, None),
            // No updated_at and a head git doesn't have: unknown, so fresh (git can't diff it, so the forge's list is asked)
            change(24, "nothing-known", &"e".repeat(40), None),
        ],
    };
    let _ = h24;
    env.state.db.add_dev_repo(space.as_str(), "gitlab.example.com/webc/app", None, "webc/app", Some("gitlab.example.com"), "main", None, BY_HAND).await.unwrap();
    let row = env.state.db.dev_repo(space.as_str(), "gitlab.example.com/webc/app").await.unwrap().unwrap();
    // A card 0.7.6 made for the old MR 20
    env.state.db.insert_dev_card_in(space.as_str(), &row.key, REVIEW, "pr:20", &h20, "{}").await.unwrap().unwrap();
    let h = History::new(Some(Repo::local(&dir, &env.state.config.data_dir).await.unwrap()), dir.to_string_lossy().to_string(), None, None, "main", None);
    poller::read(&env.state, &row, &h, Some(&fake)).await.unwrap();

    assert_eq!(live(&env, &row.key).await, ["pr:21", "pr:23", "pr:24"]);
    let all = cards(&env, &row.key).await;
    assert!(all.iter().all(|c| c.ref_ != "pr:22"), "no card for the MR whose head is 30 days old: {all:?}");
    assert_eq!(all.iter().filter(|c| c.ref_ == "pr:20").map(|c| c.status.as_str()).collect::<Vec<_>>(), [STALE], "the old MR's card went stale and got no new one");
    assert_eq!(poller::old_reviews_skipped(space.as_str(), &row.key), 2);
}

/// Rigel's probe2 on #447 (F-1): a fresh branch that turns a symlink into a file next to a +1
/// edit. git prints two `-p` sections for the one type-changed file, and every patch used to be
/// dropped, so the card said "Couldn't read 2 files (too large to diff)" and went to Medium for a
/// +2 −1 change. Now every file keeps its own patch and nothing is unread.
#[cfg(unix)]
#[tokio::test]
async fn a_type_change_leaves_no_file_unread() {
    let env = setup().await;
    let (folder, row) = watched(&env, "typechange").await;
    std::fs::write(folder.join("a.txt"), "hi\n").unwrap();
    std::os::unix::fs::symlink("a.txt", folder.join("link")).unwrap();
    git(&folder, &["add", "-A"]);
    git_at(&folder, &["commit", "-q", "-m", "Add a.txt and a link to it"], Some(days_ago(60)));
    git(&folder, &["checkout", "-q", "-b", "typechange", "main"]);
    std::fs::remove_file(folder.join("link")).unwrap();
    std::fs::write(folder.join("link"), "real file now\n").unwrap();
    std::fs::write(folder.join("a.txt"), "hi\nmore\n").unwrap();
    git(&folder, &["add", "-A"]);
    git_at(&folder, &["commit", "-q", "-m", "Tiny change"], Some(days_ago(1)));
    git(&folder, &["checkout", "-q", "main"]);

    poll(&env).await;
    let all = cards(&env, &row.key).await;
    assert_eq!(all.len(), 1, "{all:?}");
    let f: DevFacts = serde_json::from_str(&all[0].facts).unwrap();
    let got: Vec<_> = f.files.iter().map(|x| (x.path.as_str(), x.status.as_str(), x.additions, x.deletions, x.patch.is_some())).collect();
    assert_eq!(got, [("a.txt", "modified", 1, 0, true), ("link", "changed", 1, 1, true)]);
    assert!(kwnt::server::dev::risk::unread_files(&f.files).is_empty());
    let r = kwnt::server::dev::risk::assess(&kwnt::server::dev::risk::Inputs { files: &f.files, ..Default::default() });
    assert!(r.reasons.iter().all(|x| !x.starts_with("Couldn't read")), "{:?}", r.reasons);
    // The card itself, once made, says the same
    if let Some(card) = &all[0].card {
        let c: Value = serde_json::from_str(card).unwrap();
        assert!(c["risk"]["reasons"].as_array().into_iter().flatten().all(|x| !x.as_str().unwrap_or("").starts_with("Couldn't read")), "{}", c["risk"]);
        assert_eq!(c["tamper"]["unread"].as_array().map_or(0, Vec::len), 0, "{}", c["tamper"]);
    }
}
