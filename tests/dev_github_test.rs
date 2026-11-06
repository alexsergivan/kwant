//! Dev mode's GitHub reads against the shared mock (`tests/support/mock_github.rs`, QA plan §2.1):
//! ETags and 304, every error GitHub sends and what kwant makes of it, a `Link: next` to another
//! host that is never followed, a lost base commit, and only GETs, always with the token.
//!
//! The canary (QA plan §2.5): a GitHub token built at runtime goes through a whole poll, its
//! failures included, and appears nowhere but the vault: not in logs, errors, the event stream,
//! card facts, the database, model prompts, or any other file under the data folder.

mod common;
#[path = "support/mock_github.rs"]
mod mock_github;

use common::*;
use kwnt::db::dev_mode::{LANDED, REVIEW};
use kwnt::db::spaces::SpaceId;
use kwnt::server::dev::poller;
use kwnt::zombie::github::{EtagCache, GhError, GithubConfig, GithubSource};
use mock_github::*;
use serde_json::{json, Value};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};
use zeroize::Zeroizing;

const REPO: &str = "demo-org/demo-app";
const R: &str = "/repos/demo-org/demo-app";
const PORTS: [u16; 2] = [9481, 9482];
const OTHER_PORTS: [u16; 2] = [9483, 9484];

fn sha(n: u8) -> String {
    format!("{n:x}").repeat(40)
}

fn client(gh: &MockGithub, token: &str) -> GithubSource {
    GithubSource::for_dev("src_demo", GithubConfig { api_base: gh.base.clone() }, Zeroizing::new(token.to_string()))
}

#[tokio::test]
async fn an_unchanged_answer_is_a_304_and_reads_from_the_cache() {
    let gh = MockGithub::start(&PORTS).await;
    let token = canary_token();
    gh.require_token(&token).json(&format!("{R}/tags"), json!([{"name": "v1.0.0", "commit": {"sha": sha(1)}}]));
    let c = client(&gh, &token);
    let cache = EtagCache::default();

    let first = c.dev_latest_tag(REPO, &cache).await.unwrap();
    let second = c.dev_latest_tag(REPO, &cache).await.unwrap();
    assert_eq!(first, Some(("v1.0.0".into(), Some(sha(1)))));
    assert_eq!(second, first, "a 304 answers from the cache");
    let hits = gh.hits_of(&format!("{R}/tags"));
    assert_eq!(hits.iter().map(|h| h.not_modified).collect::<Vec<_>>(), [false, true], "{hits:?}");

    gh.json(&format!("{R}/tags"), json!([{"name": "v1.1.0", "commit": {"sha": sha(2)}}]));
    assert_eq!(c.dev_latest_tag(REPO, &cache).await.unwrap(), Some(("v1.1.0".into(), Some(sha(2)))), "a changed ETag is read again");
    assert!(gh.hits().iter().all(|h| h.authorized && h.method == "GET"));
}

#[tokio::test]
async fn each_github_error_is_told_apart() {
    let gh = MockGithub::start(&PORTS).await;
    let token = canary_token();
    gh.require_token(&token).json(R, repo(REPO, "main"));
    let c = client(&gh, &token);
    assert_eq!(c.dev_repo(REPO).await.unwrap().default_branch, "main");

    let reset = chrono::Utc::now().timestamp() as u64 + 600;
    type Expect = fn(&GhError) -> bool;
    let cases: Vec<(Fault, Expect)> = vec![
        (Fault::Unauthorized, |e| *e == GhError::Auth),
        (Fault::Forbidden, |e| *e == GhError::Forbidden),
        (Fault::RateLimited { reset_epoch: reset }, |e| matches!(e, GhError::Limited { retry_after_s } if (590..=600).contains(retry_after_s))),
        (Fault::SecondaryLimit { retry_after: 120 }, |e| *e == GhError::Limited { retry_after_s: 120 }),
        (Fault::TooMany { retry_after: Some(30) }, |e| *e == GhError::Limited { retry_after_s: 30 }),
        (Fault::TooMany { retry_after: None }, |e| *e == GhError::Limited { retry_after_s: 60 }),
        (Fault::Server(500), |e| *e == GhError::Other("GitHub answered 500".into())),
        (Fault::Server(502), |e| *e == GhError::Other("GitHub answered 502".into())),
        (Fault::Server(503), |e| *e == GhError::Other("GitHub answered 503".into())),
        (Fault::MissingBase, |e| *e == GhError::NotFound),
    ];
    for (fault, expected) in cases {
        gh.fault(R, fault.clone());
        let e = c.dev_repo(REPO).await.unwrap_err();
        assert!(expected(&e), "{fault:?} gave {e:?}");
        assert!(!e.to_string().contains(&token) && !format!("{e:?}").contains(&token), "{fault:?}: the error carries no token");
    }

    gh.heal(R);
    assert_eq!(client(&gh, &canary_token()).dev_repo(REPO).await.unwrap_err(), GhError::Auth, "another token is refused");
    assert!(gh.hits().last().unwrap().authorized);

    gh.fault(R, Fault::Slow(Duration::from_millis(1500)));
    let t = Instant::now();
    assert_eq!(c.dev_repo(REPO).await.unwrap().full_name, REPO, "a slow answer still counts");
    assert!(t.elapsed() >= Duration::from_millis(1500));
    assert!(gh.hits().iter().all(|h| h.method == "GET"));
}

/// kwant pages PR files on its own base (`?page=N`) and never follows a `Link: next`, so a link
/// to another host is never fetched and never gets the token.
#[tokio::test]
async fn a_next_link_to_another_host_is_never_followed() {
    let gh = MockGithub::start(&PORTS).await;
    let elsewhere = MockGithub::start(&OTHER_PORTS).await;
    let token = canary_token();
    gh.require_token(&token);
    let files = |from: usize, n: usize| Value::Array((from..from + n).map(|i| file(&format!("src/f{i}.rs"), "modified", "+a\n-b")).collect());
    let page1 = format!("{R}/pulls/7/files?per_page=100&page=1");
    gh.json(&page1, files(0, 100))
        .json(&format!("{R}/pulls/7/files?per_page=100&page=2"), files(100, 3))
        .link_next(&page1, &format!("{}{R}/pulls/7/files?per_page=100&page=2", elsewhere.base));

    let got = client(&gh, &token).dev_pull_files(REPO, 7).await.unwrap();
    assert_eq!(got.len(), 103, "page 2 came from GitHub's own base");
    assert!(elsewhere.hits().is_empty(), "the other host was never asked: {:?}", elsewhere.hits());
    let paths: Vec<String> = gh.hits().into_iter().map(|h| h.path).collect();
    assert_eq!(paths, [page1, format!("{R}/pulls/7/files?per_page=100&page=2")]);
}

/// A force push took the last seen commit off the branch: the list doesn't hold it, so kwant takes
/// the newest commits only, and reading the lost commit is a plain not-found.
#[tokio::test]
async fn a_lost_base_commit_falls_back_to_the_newest() {
    let gh = MockGithub::start(&PORTS).await;
    let token = canary_token();
    gh.require_token(&token);
    let list: Vec<Value> = (2..=4).rev().map(|n| commit(&sha(n), &format!("Change {n}"), ("Demo Person", "demo@example.com"), &[], "2026-11-05T01:00:00Z")).collect();
    gh.json(&format!("{R}/commits"), Value::Array(list)).fault(&format!("{R}/commits/{}", sha(1)), Fault::MissingBase);
    let c = client(&gh, &token);

    let (commits, found) = c.dev_commits_since(REPO, "main", Some(&sha(1)), &EtagCache::default()).await.unwrap();
    assert!(!found);
    assert_eq!(commits.iter().map(|c| c.sha.clone()).collect::<Vec<_>>(), [sha(2), sha(3), sha(4)], "oldest first");
    assert_eq!(poller::window(kwnt::server::dev::source::all(commits), false, found).len(), 3, "at most MAX_NEW of them");
    assert_eq!(c.dev_commit(REPO, &sha(1)).await.unwrap_err(), GhError::NotFound);

    let (commits, found) = c.dev_commits_since(REPO, "main", Some(&sha(3)), &EtagCache::default()).await.unwrap();
    assert!(found);
    assert_eq!(commits.iter().map(|c| c.sha.clone()).collect::<Vec<_>>(), [sha(4)]);
}

#[tokio::test]
async fn a_write_to_the_mock_fails_the_test() {
    let gh = MockGithub::start(&OTHER_PORTS).await;
    let res = reqwest::Client::new().post(format!("{}{R}/merges", gh.base)).body("{}").send().await.unwrap();
    assert_eq!(res.status().as_u16(), 405);
    assert_eq!(gh.hits()[0].method, "POST");
    let dropped = std::panic::catch_unwind(std::panic::AssertUnwindSafe(move || drop(gh)));
    assert!(dropped.is_err(), "dropping a mock that saw a write panics");
}

// ---- the canary -------------------------------------------------------------------------------

static LOG: OnceLock<Arc<Mutex<Vec<u8>>>> = OnceLock::new();

struct Capture(Arc<Mutex<Vec<u8>>>);

impl std::io::Write for Capture {
    fn write(&mut self, b: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(b);
        Ok(b.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

/// Every log line of this test binary, at debug (trace for kwnt itself).
fn capture_logs() -> Arc<Mutex<Vec<u8>>> {
    LOG.get_or_init(|| {
        let buf = Arc::new(Mutex::new(Vec::new()));
        let writer = buf.clone();
        let sub = tracing_subscriber::fmt()
            .with_env_filter(tracing_subscriber::EnvFilter::new("debug,kwnt=trace"))
            .with_ansi(false)
            .with_writer(move || Capture(writer.clone()))
            .finish();
        tracing::subscriber::set_global_default(sub).expect("no other subscriber in this binary");
        buf
    })
    .clone()
}

/// A repo with one merged PR, one direct commit, one open PR, CI and a tag.
fn seed(gh: &MockGithub) {
    let person = ("Lin (kwant bot)", "lin-bot@users.noreply.example.com");
    let with_files = |mut c: Value, files: Value| {
        c["files"] = files;
        c
    };
    let patch = "@@ -1,2 +1,2 @@\n-let retries = 1;\n+let retries = 3;";
    let merged = commit(&sha(2), "Retry the upload (#5)", person, &[&sha(1)], "2026-11-05T02:00:00Z");
    let direct = commit(&sha(1), "Fix the README", person, &[], "2026-11-05T01:00:00Z");
    gh.json("/user", user("demo-person"))
        .json(R, repo(REPO, "main"))
        .json(&format!("{R}/branches/main"), json!({"name": "main"}))
        .json(&format!("{R}/contents"), json!([{"name": "Cargo.toml", "type": "file"}]))
        .json(&format!("{R}/tags"), json!([{"name": "v0.1.0", "commit": {"sha": sha(1)}}]))
        .json(&format!("{R}/commits"), json!([merged, direct]))
        .json(&format!("{R}/commits/{}", sha(2)), with_files(merged.clone(), json!([file("src/upload.rs", "modified", patch)])))
        .json(&format!("{R}/commits/{}", sha(1)), with_files(direct.clone(), json!([file("README.md", "modified", "-old\n+new")])))
        .json(&format!("{R}/commits/{}/pulls", sha(2)), json!([pull(5, "Retry the upload", ("retry", &sha(9)), "main", "demo-person", Some((&sha(2), "2026-11-05T02:00:00Z")))]))
        .json(&format!("{R}/commits/{}/pulls", sha(1)), json!([]))
        .json(&format!("{R}/pulls/5/files"), json!([file("src/upload.rs", "modified", patch)]))
        .json(&format!("{R}/commits/{}/check-runs", sha(2)), check_runs(&[("rust", "completed", Some("success"))]))
        .json(&format!("{R}/commits/{}/check-runs", sha(1)), check_runs(&[("rust", "completed", Some("failure"))]))
        .json(&format!("{R}/pulls"), json!([pull(8, "Add a cache", ("cache", &sha(3)), "main", "demo-person", None)]))
        .json(&format!("{R}/pulls/8/files"), json!([file("src/cache.rs", "added", "+pub struct Cache;")]))
        .json(&format!("{R}/commits/{}", sha(3)), commit(&sha(3), "Add a cache", person, &[&sha(2)], "2026-11-05T03:00:00Z"))
        .json(&format!("{R}/commits/{}/check-runs", sha(3)), check_runs(&[("rust", "in_progress", None)]));
}

/// Every file under `dir` (the vault's folder left out) that holds `needle`.
fn files_with(dir: &std::path::Path, needle: &[u8], vault: &std::path::Path, out: &mut Vec<String>) {
    for entry in std::fs::read_dir(dir).into_iter().flatten().flatten() {
        let p = entry.path();
        if p.starts_with(vault) {
            continue;
        }
        if p.is_dir() {
            files_with(&p, needle, vault, out);
        } else if std::fs::read(&p).is_ok_and(|b| b.windows(needle.len()).any(|w| w == needle)) {
            out.push(p.display().to_string());
        }
    }
}

#[tokio::test]
async fn the_github_token_never_leaves_the_vault() {
    let logs = capture_logs();
    let env = setup().await;
    let space = SpaceId::default_space();
    // An asking install, so the login may be saved here (F-10)
    env.state.db.set_space_approvals(&space, "ask").await.unwrap();
    let token = canary_token();
    let gh = MockGithub::start(&PORTS).await;
    gh.require_token(&token);
    seed(&gh);
    let mut events = env.state.event_sender.subscribe();

    let (s, v) = call(&env.app, "POST", "/api/zombie/sources", Some(json!({"kind": "github", "label": "GitHub", "config": {"api_base": gh.base}, "secret": token}))).await;
    assert_eq!(s.as_u16(), 201, "{v}");
    let source_id = v["source"]["id"].as_str().unwrap().to_string();
    let mut replies = vec![v];
    let (s, v) = call(&env.app, "PUT", &format!("/api/spaces/{}/dev_mode", space.as_str()), Some(json!({"on": true, "repo": REPO, "branch": "main"}))).await;
    assert_eq!(s.as_u16(), 200, "{v}");
    replies.push(v);
    let (_, v) = call(&env.app, "POST", &format!("/api/spaces/{}/dev_mode/check", space.as_str()), Some(json!({"repo": REPO}))).await;
    replies.push(v);

    let mode = env.state.db.dev_repos(space.as_str()).await.unwrap().remove(0);
    poller::poll(&env.state, &mode).await.expect("a clean poll");
    let landed = env.state.db.dev_cards(space.as_str(), LANDED, None, 50).await.unwrap();
    let reviews = env.state.db.dev_cards(space.as_str(), REVIEW, None, 50).await.unwrap();
    assert_eq!((landed.len(), reviews.len()), (2, 1), "both commits and the open PR made cards");

    // Failures, as the poller records them
    let mut errors = Vec::new();
    for fault in [Fault::Server(502), Fault::Unauthorized, Fault::Forbidden] {
        gh.fault(R, fault.clone());
        let mode = env.state.db.dev_repos(space.as_str()).await.unwrap().remove(0);
        errors.push(format!("{:?}", poller::poll(&env.state, &mode).await.unwrap_err()));
        poller::poll_logged(&env.state, mode).await;
        let after = env.state.db.dev_repos(space.as_str()).await.unwrap().remove(0);
        errors.push(after.poll_error.clone().unwrap_or_else(|| panic!("{fault:?} left no poll error")));
    }
    gh.heal(R);
    let (_, v) = call(&env.app, "GET", &format!("/api/spaces/{}/dev_mode", space.as_str()), None).await;
    replies.push(v);
    let (_, v) = call(&env.app, "GET", &format!("/api/spaces/{}/dev/landed", space.as_str()), None).await;
    replies.push(v);
    let (_, v) = call(&env.app, "GET", &format!("/api/spaces/{}/dev/reviews", space.as_str()), None).await;
    replies.push(v);
    let (_, v) = call(&env.app, "GET", "/api/zombie/sources", None).await;
    replies.push(v);

    // Let the cards' text be written (the fake model answers), so its prompts are on disk too
    for _ in 0..100 {
        if env.state.db.pending_dev_cards().await.map(|p| p.is_empty()).unwrap_or(true) {
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }

    let leaks = |what: &str, text: &str| assert!(!text.contains(&token), "the token leaked into {what}");
    for e in &errors {
        leaks("an error", e);
    }
    for r in &replies {
        leaks("an API reply", &r.to_string());
    }
    for row in landed.iter().chain(&reviews).chain(&env.state.db.dev_cards(space.as_str(), LANDED, None, 50).await.unwrap()) {
        leaks("card facts", &row.facts);
        leaks("a card", row.card.as_deref().unwrap_or_default());
    }
    let mut seen = 0;
    while let Ok(ev) = events.try_recv() {
        leaks("the event stream", &serde_json::to_string(&ev).unwrap());
        seen += 1;
    }
    assert!(seen > 0, "the poll sent card events");
    let log = String::from_utf8_lossy(&logs.lock().unwrap()).to_string();
    assert!(log.contains("dev: poll failed"), "the log capture works");
    leaks("the log", &log);
    assert!(invocations_with(&token).is_empty(), "the token reached a model prompt");

    // The database (every table, WAL included) and every other file under the data folder
    let data = env.state.config.data_dir.clone();
    let mut found = Vec::new();
    files_with(&data, token.as_bytes(), &data.join("kwnt-vault"), &mut found);
    assert!(found.is_empty(), "the token is in {found:?}");
    assert!(data.join("kwnt.db").exists());
    let kept = kwnt::zombie::hunt::source_secret(env.state.credentials.as_ref(), &source_id, &gh.base).await.unwrap();
    assert_eq!(kept.as_deref().map(String::as_str), Some(token.as_str()), "the vault holds it, and only the vault");

    // Only reads, each with the token
    assert!(gh.hits().iter().all(|h| h.method == "GET" && h.authorized), "{:?}", gh.hits());
}
