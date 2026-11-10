//! CI and review comments back to the author bot (0.9.0 wave 1, `server::dev::fix_it`): Ada's
//! six cases from release-090/plan.md, plus the daily turn cap, Needs you and a stopped chat.
//!
//! The forge is a fake that records what was asked of it (it has no merge, and `post_review`
//! counts calls, which must stay 0); the GitHub client's two new reads run against the shared
//! mock GitHub, which fails the test on any write. Git repos are throwaway `git init`s. PRs are
//! drafts so the poller makes no review cards of its own.

mod common;
#[path = "support/mock_github.rs"]
mod mock_github;

use base64::Engine;
use chrono::{Duration, Utc};
use common::*;
use kwnt::db::dev_fix_it::{FixItemRow, FIX_IT, TURN_HELD, TURN_STARTED, TURN_STOPPED, VERDICT};
use kwnt::db::dev_mode::{DevRepoRow, BY_HAND};
use kwnt::db::models::Message;
use kwnt::db::spaces::SpaceId;
use kwnt::server::dev::facts::{Check, FileDiff};
use kwnt::server::dev::fix_it::{self, TURNS_PER_DAY};
use kwnt::server::dev::forge::ForgeKind;
use kwnt::server::dev::git::Repo;
use kwnt::server::dev::poller;
use kwnt::server::dev::source::{Change, Forge, GithubForge, History, ReviewComment};
use kwnt::zombie::github::{EtagCache, GhError, GhReview, GithubConfig, GithubSource, PostError, ReviewPost};
use mock_github::MockGithub;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use zeroize::Zeroizing;

const REPO: &str = "demo-org/app";
const KEY: &str = "github.com/demo-org/app";
/// Juno's mock GitHub ports (dev_github_test has 9481–9484).
const PORTS: [u16; 2] = [9491, 9492];

fn git_as(dir: &Path, who: &str, args: &[&str]) -> String {
    let out = std::process::Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(["-c", &format!("user.name={who}"), "-c", "user.email=someone@example.com", "-c", "commit.gpgsign=false"])
        .args(args)
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .output()
        .unwrap();
    assert!(out.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&out.stderr));
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

/// A commit of `file` by `who` on `branch` (made off main when new); returns its sha.
fn commit_on(dir: &Path, who: &str, branch: &str, file: &str, text: &str) -> String {
    let exists = git_as(dir, who, &["branch", "--list", branch]);
    if exists.is_empty() {
        git_as(dir, who, &["checkout", "-q", "-b", branch, "main"]);
    } else {
        git_as(dir, who, &["checkout", "-q", branch]);
    }
    if let Some(parent) = dir.join(file).parent() {
        std::fs::create_dir_all(parent).unwrap();
    }
    std::fs::write(dir.join(file), text).unwrap();
    git_as(dir, who, &["add", "--", file]);
    git_as(dir, who, &["commit", "-q", "-m", &format!("Change {file}")]);
    let sha = git_as(dir, who, &["rev-parse", "HEAD"]);
    git_as(dir, who, &["checkout", "-q", "main"]);
    sha
}

#[derive(Default)]
struct Fake {
    open: Mutex<Vec<Change>>,
    checks: Mutex<HashMap<String, Vec<Check>>>,
    comments: Mutex<HashMap<u64, Vec<ReviewComment>>>,
    excerpts: Mutex<HashMap<String, String>>,
    posts: Mutex<usize>,
}

impl Fake {
    fn set_checks(&self, sha: &str, checks: Vec<Check>) {
        self.checks.lock().unwrap().insert(sha.into(), checks);
    }
    fn set_comments(&self, pr: u64, c: Vec<ReviewComment>) {
        self.comments.lock().unwrap().insert(pr, c);
    }
    fn set_head(&self, pr: u64, sha: &str) {
        for p in self.open.lock().unwrap().iter_mut().filter(|p| p.number == pr) {
            p.head_sha = sha.into();
            p.updated_at = Some(Utc::now());
        }
    }
}

#[async_trait::async_trait]
impl Forge for Fake {
    async fn changes_for_commit(&self, _sha: &str) -> Result<Vec<Change>, GhError> {
        Ok(vec![])
    }
    async fn open_changes(&self, _base: &str, _cache: &EtagCache) -> Result<Vec<Change>, GhError> {
        Ok(self.open.lock().unwrap().clone())
    }
    async fn checks(&self, sha: &str, _cache: &EtagCache) -> Result<Vec<Check>, GhError> {
        Ok(self.checks.lock().unwrap().get(sha).cloned().unwrap_or_default())
    }
    async fn change_files(&self, _number: u64) -> Result<Vec<FileDiff>, GhError> {
        Ok(vec![])
    }
    fn change_head_ref(&self, _number: u64) -> Option<String> {
        None
    }
    async fn commit_login(&self, _sha: &str) -> Option<String> {
        None
    }
    fn kind(&self) -> ForgeKind {
        ForgeKind::Github
    }
    async fn change(&self, number: u64) -> Result<Option<Change>, GhError> {
        Ok(self.open.lock().unwrap().iter().find(|c| c.number == number).cloned())
    }
    async fn login(&self) -> Result<String, GhError> {
        Ok("demo-bot".into())
    }
    async fn post_review(&self, _number: u64, _post: &ReviewPost) -> Result<GhReview, PostError> {
        *self.posts.lock().unwrap() += 1;
        Err(PostError::Rejected("the fake forge takes no writes".into()))
    }
    async fn review_comments(&self, number: u64) -> Result<Vec<ReviewComment>, GhError> {
        // Newest first, as GitHub sends them
        let mut c = self.comments.lock().unwrap().get(&number).cloned().unwrap_or_default();
        c.reverse();
        Ok(c)
    }
    async fn check_excerpt(&self, sha: &str, name: &str) -> Option<String> {
        self.excerpts.lock().unwrap().get(&format!("{sha}:{name}")).cloned()
    }
}

fn check(name: &str, status: &str, conclusion: Option<&str>) -> Check {
    Check { name: name.into(), status: status.into(), conclusion: conclusion.map(str::to_string), url: None }
}

fn comment(id: u64, path: &str, start: Option<u64>, line: u64, body: &str, mins_ago: i64) -> ReviewComment {
    ReviewComment {
        id,
        path: path.into(),
        line: Some(line),
        start_line: start,
        body: body.into(),
        author: Some("lin".into()),
        created_at: Some(Utc::now() - Duration::minutes(mins_ago)),
        ..Default::default()
    }
}

fn pr(n: u64, branch: &str, head: &str, body: &str) -> Change {
    Change {
        number: n,
        title: format!("PR {n}"),
        body: Some(body.into()),
        draft: true,
        url: Some(format!("https://github.com/{REPO}/pull/{n}")),
        head_branch: branch.into(),
        head_sha: head.into(),
        base_branch: "main".into(),
        updated_at: Some(Utc::now()),
        ..Default::default()
    }
}

/// A message's meta as JSON (null when it has none).
fn meta(m: &Message) -> Value {
    m.meta.as_deref().and_then(|s| serde_json::from_str(s).ok()).unwrap_or(Value::Null)
}

struct World {
    env: Env,
    dir: PathBuf,
    row: DevRepoRow,
    /// The bot `env.a`'s name: its commits say "<name> (kwant bot)".
    bot: String,
}

async fn world() -> World {
    let env = setup().await;
    let space = SpaceId::default_space();
    let dir = env.tmp.path().join("app");
    std::fs::create_dir_all(&dir).unwrap();
    git_as(&dir, "Oleks", &["init", "-q", "-b", "main"]);
    std::fs::write(dir.join("README.md"), "# App\n").unwrap();
    git_as(&dir, "Oleks", &["add", "README.md"]);
    git_as(&dir, "Oleks", &["commit", "-q", "-m", "Start"]);
    env.state.db.add_dev_repo(space.as_str(), KEY, None, REPO, Some("github.com"), "main", None, BY_HAND).await.unwrap();
    let row = env.state.db.dev_repo(space.as_str(), KEY).await.unwrap().unwrap();
    let bot = env.state.db.get_kwant(&env.a).await.unwrap().unwrap().name;
    World { env, dir, row, bot }
}

impl World {
    fn by_bot(&self, branch: &str, file: &str, text: &str) -> String {
        commit_on(&self.dir, &format!("{} (kwant bot)", self.bot), branch, file, text)
    }

    async fn poll(&self, fake: &Fake) {
        let h = History::new(Some(Repo::local(&self.dir, &self.env.state.config.data_dir).await.unwrap()), self.dir.to_string_lossy().to_string(), None, None, "main", None);
        poller::read(&self.env.state, &self.row, &h, Some(fake)).await.unwrap();
    }

    async fn items(&self, n: u64) -> Vec<FixItemRow> {
        self.env.state.db.fix_items_of_pr(SpaceId::default_space().as_str(), KEY, n as i64).await.unwrap()
    }

    async fn lines(&self, thread: &str, kind: &str) -> Vec<Message> {
        messages(&self.env, thread).await.into_iter().filter(|m| meta(m)["kind"].as_str() == Some(kind)).collect()
    }

    /// Waits for the Fix-it turn in `thread` to end.
    async fn turn_done(&self, thread: &str) {
        let state = self.env.state.clone();
        let t = thread.to_string();
        wait_for("the fix-it turn to end", || {
            let (state, t) = (state.clone(), t.clone());
            async move { !state.turns.has_job(&t) && !state.turns.active_turns().iter().any(|a| a.thread_id.as_deref() == Some(t.as_str())) }
        })
        .await;
    }
}

/// Ada's case 1 and 3: a failing check on a bot's PR becomes exactly one Fix-it item in that bot's
/// Fix-it chat, with the job name and the tail of its output, and starts one turn of that bot.
/// The next poll of the same head sends nothing again.
#[tokio::test]
async fn a_failing_check_is_one_fix_it_item_for_the_author_bot_once() {
    let w = world().await;
    let head = w.by_bot("juno-fix-11", "a.rs", "fn a() {}\n");
    let fake = Fake::default();
    fake.open.lock().unwrap().push(pr(11, "juno-fix-11", &head, "Fixes a"));
    fake.set_checks(&head, vec![check("build", "completed", Some("success")), check("test", "completed", Some("failure"))]);
    let log: String = (1..=120).map(|i| format!("noise {i}\n")).collect::<String>() + "error[E0425]: cannot find value `x` in this scope\n";
    fake.excerpts.lock().unwrap().insert(format!("{head}:test"), log);

    w.poll(&fake).await;
    let items = w.items(11).await;
    assert_eq!(items.len(), 1, "{items:?}");
    let it = &items[0];
    assert_eq!((it.kind.as_str(), it.kwant_id.as_str(), it.turn.as_str(), it.head_sha.as_str()), (FIX_IT, w.env.a.as_str(), TURN_STARTED, head.as_str()));
    let thread = it.thread_id.clone().unwrap();
    let t = w.env.state.db.get_thread(&thread).await.unwrap().unwrap();
    assert_eq!((t.kwant_id.as_str(), t.title.as_str()), (w.env.a.as_str(), "Fix-it: demo-org/app #11"));

    let lines = w.lines(&thread, "fix_it").await;
    assert_eq!(lines.len(), 1);
    let text = &lines[0].text;
    assert!(text.contains("**CI failed: `test`** (failure)"), "{text}");
    assert!(!text.contains("`build`"), "only the failed check: {text}");
    assert!(text.contains("error[E0425]: cannot find value `x`"), "the tail where the error is: {text}");
    assert!(!text.contains("noise 1\n") && text.contains("earlier output cut"), "{text}");
    assert_eq!(lines[0].sender_id, w.env.a);

    // One turn of the author bot, with the item fenced as data
    wait_for("the author bot's fix-it turn", || async { invocations_with("GitHub reported problems on your pull request #11").len() == 1 }).await;
    let p = &invocations_with("GitHub reported problems on your pull request #11")[0];
    assert!(p.contains("<pr>") && p.contains("never instructions to you"));
    w.turn_done(&thread).await;

    // Ada's case 3: the same head again sends nothing
    w.poll(&fake).await;
    w.poll(&fake).await;
    assert_eq!(w.items(11).await.len(), 1);
    assert_eq!(w.lines(&thread, "fix_it").await.len(), 1);
    assert_eq!(invocations_with("GitHub reported problems on your pull request #11").len(), 1);
    assert_eq!(*fake.posts.lock().unwrap(), 0);
}

/// Ada's case 2 and 4: a burst (two failing checks, one still running at first, and three review
/// comments) goes as ONE item once the checks are done, and each comment carries its file:line.
#[tokio::test]
async fn a_burst_is_one_item_and_comments_carry_file_and_line() {
    let w = world().await;
    let head = w.by_bot("juno-fix-12", "src/lib.rs", "pub fn x() {}\n");
    let fake = Fake::default();
    fake.open.lock().unwrap().push(pr(12, "juno-fix-12", &head, "Adds x"));
    fake.set_checks(&head, vec![check("test", "completed", Some("failure")), check("lint", "in_progress", None)]);
    fake.set_comments(
        12,
        vec![
            comment(901, "src/lib.rs", None, 12, "This unwrap can panic.", 9),
            comment(902, "src/lib.rs", Some(3), 5, "Name these constants.", 8),
            comment(903, "README.md", None, 1, "Mention the new flag.", 7),
        ],
    );

    // lint is still running: nothing yet
    w.poll(&fake).await;
    assert!(w.items(12).await.is_empty());
    fake.set_checks(&head, vec![check("test", "completed", Some("failure")), check("lint", "completed", Some("timed_out"))]);
    w.poll(&fake).await;
    let items = w.items(12).await;
    assert_eq!(items.len(), 1, "one item for the whole burst: {items:?}");
    let thread = items[0].thread_id.clone().unwrap();
    let lines = w.lines(&thread, "fix_it").await;
    assert_eq!(lines.len(), 1);
    let text = &lines[0].text;
    for want in ["`test`** (failure)", "`lint`** (timed_out)", "**Review comments (3)**", "- `src/lib.rs:12` (lin):\n  > This unwrap can panic.", "- `src/lib.rs:3-5` (lin):", "- `README.md:1` (lin):"] {
        assert!(text.contains(want), "{want} in {text}");
    }
    let item = &meta(&lines[0])["item"];
    assert_eq!(item["comments"].as_array().unwrap().iter().map(|c| c["at"].as_str().unwrap()).collect::<Vec<_>>(), ["src/lib.rs:12", "src/lib.rs:3-5", "README.md:1"]);
    assert_eq!(item["comments"][1]["start_line"], 3);
    w.turn_done(&thread).await;

    // A later comment is new: it goes alone; the old ones and the checks aren't sent again
    let mut c = fake.comments.lock().unwrap().get(&12).cloned().unwrap();
    c.push(comment(904, "src/lib.rs", None, 20, "One more thing.", 5));
    fake.set_comments(12, c);
    w.poll(&fake).await;
    let lines = w.lines(&thread, "fix_it").await;
    assert_eq!(lines.len(), 2);
    let text = &lines[1].text;
    assert!(text.contains("`src/lib.rs:20`") && !text.contains("unwrap can panic") && !text.contains("CI failed"), "{text}");
    w.turn_done(&thread).await;
    assert_eq!(*fake.posts.lock().unwrap(), 0);
}

/// Ada's case 5: a green head gets one advisory verdict from a fresh-eyes bot that isn't the
/// author, and nothing is merged or written to the forge; the next poll adds no second verdict.
#[tokio::test]
async fn green_gets_one_advisory_verdict_and_no_merge() {
    let w = world().await;
    let head = w.by_bot("juno-fix-13", "b.rs", "fn b() { todo!() }\n");
    let reply = json!({"verdict": "hold", "reasons": ["b() still panics with todo!()", "No test covers b()"]});
    let body = format!("Adds b. REPLYTEXT:{}", base64::engine::general_purpose::STANDARD.encode(reply.to_string()));
    let fake = Fake::default();
    fake.open.lock().unwrap().push(pr(13, "juno-fix-13", &head, &body));
    fake.set_checks(&head, vec![check("build", "completed", Some("success")), check("test", "completed", Some("success"))]);

    w.poll(&fake).await;
    let items = w.items(13).await;
    assert!(items.iter().all(|i| i.kind != FIX_IT), "nothing to fix on green: {items:?}");
    let s = w.env.state.clone();
    wait_for("the verdict", || {
        let s = s.clone();
        async move { s.db.fix_items_of_pr(SpaceId::default_space().as_str(), KEY, 13).await.unwrap().iter().any(|i| i.kind == VERDICT) }
    })
    .await;
    let items = w.items(13).await;
    assert_eq!(items.len(), 1, "{items:?}");
    let thread = items[0].thread_id.clone().unwrap();
    let lines = w.lines(&thread, "fix_it_verdict").await;
    assert_eq!(lines.len(), 1);
    let m = &lines[0];
    assert_ne!(m.sender_id, w.env.a, "the author never reviews its own PR");
    assert!(m.text.contains(": HOLD**\n- b() still panics with todo!()\n- No test covers b()"), "{}", m.text);
    assert!(m.text.contains("Advisory only: kwant never merges"), "{}", m.text);
    let item = &meta(m)["item"];
    assert_eq!((item["verdict"].clone(), item["label"].clone(), item["advisory"].clone()), (json!("hold"), json!("HOLD"), json!(true)));
    // The reviewer's call had no tools
    let calls = invocations_with("re-checking a pull request with fresh eyes");
    assert_eq!(calls.iter().filter(|c| c.contains("PR #13")).count(), 1);

    w.poll(&fake).await;
    w.poll(&fake).await;
    assert_eq!(w.items(13).await.len(), 1, "one verdict per head");
    assert_eq!(*fake.posts.lock().unwrap(), 0, "nothing written to the forge");
    // No turn of the author bot either
    assert!(invocations_with("GitHub reported problems on your pull request #13").is_empty());
}

/// Ada's case 6: a PR the person opened (no bot wrote its head) gets nothing: no item, no chat,
/// nothing marked sent.
#[tokio::test]
async fn a_pr_not_opened_by_a_bot_is_ignored() {
    let w = world().await;
    let head = commit_on(&w.dir, "Oleks", "oleks-tweak", "c.rs", "fn c() {}\n");
    let fake = Fake::default();
    fake.open.lock().unwrap().push(pr(14, "oleks-tweak", &head, "Mine"));
    fake.set_checks(&head, vec![check("test", "completed", Some("failure"))]);
    fake.set_comments(14, vec![comment(941, "c.rs", None, 1, "Nit.", 30)]);
    w.poll(&fake).await;
    w.poll(&fake).await;
    assert!(w.items(14).await.is_empty());
    assert!(w.env.state.db.fix_seen_keys(SpaceId::default_space().as_str(), KEY, 14).await.unwrap().is_empty());
    let threads = w.env.state.db.list_threads(&w.env.a, true, 50).await.unwrap();
    assert!(threads.iter().all(|t| !t.title.starts_with("Fix-it")), "{threads:?}");
    // Green: no verdict either
    fake.set_checks(&head, vec![check("test", "completed", Some("success"))]);
    w.poll(&fake).await;
    assert!(w.items(14).await.is_empty());
}

/// Past TURNS_PER_DAY turns for a PR in a day, a new item waits in the chat and in Needs you
/// instead of starting another turn. Archiving the Fix-it chat stops it: nothing more is posted.
#[tokio::test]
async fn the_daily_turn_cap_holds_items_and_an_archived_chat_stops_them() {
    assert_eq!(TURNS_PER_DAY, 3);
    let w = world().await;
    let fake = Fake::default();
    let mut thread = String::new();
    for i in 0..=TURNS_PER_DAY {
        let head = w.by_bot("juno-fix-15", "d.rs", &format!("fn d() {{ {i} }}\n"));
        if i == 0 {
            fake.open.lock().unwrap().push(pr(15, "juno-fix-15", &head, "Adds d"));
        } else {
            fake.set_head(15, &head);
        }
        fake.set_checks(&head, vec![check("test", "completed", Some("failure"))]);
        w.poll(&fake).await;
        let items = w.items(15).await;
        assert_eq!(items.len(), i + 1, "one item per head");
        thread = items[i].thread_id.clone().unwrap();
        w.turn_done(&thread).await;
    }
    let items = w.items(15).await;
    assert_eq!(items.iter().map(|i| i.turn.as_str()).collect::<Vec<_>>(), [TURN_STARTED, TURN_STARTED, TURN_STARTED, TURN_HELD]);
    assert!(items.iter().all(|i| i.thread_id.as_deref() == Some(thread.as_str())), "one chat per PR");
    assert_eq!(invocations_with("GitHub reported problems on your pull request #15").len(), TURNS_PER_DAY);
    let last = w.lines(&thread, "fix_it").await.pop().unwrap();
    assert!(last.text.contains("Reply here to start it."), "{}", last.text);

    // Needs you lists the held item, with its chat
    let (s, v) = call(&w.env.app, "GET", "/api/waiting", None).await;
    assert_eq!(s.as_u16(), 200, "{v}");
    let held: Vec<&Value> = v["items"].as_array().unwrap().iter().filter(|i| i["kind"] == "dev_fix_it").collect();
    assert_eq!(held.len(), 1, "{v}");
    assert_eq!((held[0]["thread_id"].as_str(), held[0]["kwant_id"].as_str()), (Some(thread.as_str()), Some(w.env.a.as_str())));
    assert!(held[0]["text"].as_str().unwrap().contains("demo-org/app #15"));

    // The person archives the chat: the next failure is marked seen, but nothing is posted
    w.env.state.db.update_thread(&thread, None, Some(true)).await.unwrap();
    let before = messages(&w.env, &thread).await.len();
    let head = w.by_bot("juno-fix-15", "d.rs", "fn d() { 99 }\n");
    fake.set_head(15, &head);
    fake.set_checks(&head, vec![check("test", "completed", Some("failure"))]);
    w.poll(&fake).await;
    let items = w.items(15).await;
    assert_eq!(items.last().map(|i| (i.turn.as_str(), i.thread_id.clone())), Some((TURN_STOPPED, None)));
    assert_eq!(messages(&w.env, &thread).await.len(), before);
    w.poll(&fake).await;
    assert_eq!(w.items(15).await.len(), items.len(), "and not looked at again");
}

/// The GitHub client's two new reads, against the mock: inline review comments with their lines
/// (the original line when the line is gone), and a failed check's output and annotations. Only
/// GETs.
#[tokio::test]
async fn github_reads_review_comments_and_check_output() {
    let gh = MockGithub::start(&PORTS).await;
    let sha = "a".repeat(40);
    let r = format!("/repos/{REPO}");
    gh.json(
        &format!("{r}/pulls/7/comments"),
        json!([
            {"id": 2, "path": "src/a.rs", "line": 12, "start_line": 10, "body": "Range", "user": {"login": "lin"}, "commit_id": sha, "html_url": "https://github.com/x", "created_at": "2026-11-10T06:00:00Z"},
            {"id": 1, "path": "src/b.rs", "line": null, "original_line": 4, "body": "Outdated", "user": {"login": "rigel"}, "created_at": "2026-11-10T05:00:00Z"}
        ]),
    );
    gh.json(
        &format!("{r}/commits/{sha}/check-runs"),
        json!({"total_count": 1, "check_runs": [{"id": 55, "name": "test", "status": "completed", "conclusion": "failure",
            "output": {"title": "2 tests failed", "summary": "fix_it::plan failed", "text": null, "annotations_count": 1}}]}),
    );
    gh.json(&format!("{r}/check-runs/55/annotations"), json!([{"path": "src/a.rs", "start_line": 12, "annotation_level": "failure", "message": "assertion failed"}]));
    let client = GithubSource::for_dev("src_demo", GithubConfig { api_base: gh.base.clone() }, Zeroizing::new("t0ken".to_string()));
    let forge = GithubForge { client: &client, repo: REPO };

    let c = forge.review_comments(7).await.unwrap();
    assert_eq!(c.iter().map(|c| (c.id, c.path.as_str(), c.start_line, c.line, c.author.as_deref())).collect::<Vec<_>>(), [(2, "src/a.rs", Some(10), Some(12), Some("lin")), (1, "src/b.rs", None, Some(4), Some("rigel"))]);
    assert_eq!(fix_it::file_line(&c[0]), "src/a.rs:10-12");
    let x = forge.check_excerpt(&sha, "test").await.unwrap();
    assert_eq!(x, "2 tests failed\nfix_it::plan failed\nsrc/a.rs:12: failure: assertion failed");
    assert_eq!(forge.check_excerpt(&sha, "lint").await, None, "no such run");
    assert!(gh.hits_of(&format!("{r}/pulls/7/comments"))[0].path.contains("sort=created"));
    gh.assert_read_only();
}
