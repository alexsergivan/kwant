//! Dev mode on the existing setup (plan dev-mode/plan/any-git.md S1–S4): the switch finds the repo
//! in a team's folder with no question asked, turns on without a GitHub connection or any repo,
//! and no token in a folder's git remote reaches a reply, the database or a log line.
//!
//! The team folders are throwaway `git init` checkouts; GitHub is the shared mock on loopback.
//! Nothing here touches the network.

mod common;
#[path = "support/mock_github.rs"]
mod mock_github;

use common::*;
use kwnt::db::models::{NewKwant, NewTeam};
use kwnt::db::dev_mode::{LANDED, READY, REVIEW};
use kwnt::db::spaces::{NewSpace, SpaceId};
use kwnt::server::dev::poller;
use mock_github::*;
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};

const PORTS: [u16; 2] = [9495, 9496];
const REPO: &str = "demo-org/demo-app";

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

fn git(dir: &Path, args: &[&str]) {
    let out = std::process::Command::new("git").arg("-C").arg(dir).args(args).env("GIT_TERMINAL_PROMPT", "0").output().unwrap();
    assert!(out.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&out.stderr));
}

/// A checkout at `dir` on `main` with `origin` at `url` (when given).
fn checkout(dir: &Path, url: Option<&str>) -> PathBuf {
    std::fs::create_dir_all(dir).unwrap();
    git(dir, &["init", "-q", "-b", "main"]);
    if let Some(u) = url {
        git(dir, &["remote", "add", "origin", u]);
    }
    dir.to_path_buf()
}

async fn team(env: &Env, name: &str, folder: Option<&Path>) {
    team_in(env, &SpaceId::default_space(), name, folder).await
}

async fn team_in(env: &Env, space: &SpaceId, name: &str, folder: Option<&Path>) {
    let lead = if space.is_default() {
        env.a.clone()
    } else {
        let k = NewKwant {
            name: format!("{name} lead"),
            title: "Lead".into(),
            persona_prompt: "Test persona".into(),
            soul: String::new(),
            avatar_sprite: "s".into(),
            driver_type: "api".into(),
            preferred_model: "claude".into(),
        };
        env.state.db.create_kwant(space, &k).await.unwrap().id
    };
    let t = NewTeam {
        name: name.into(),
        description: None,
        icon: None,
        lead_kwant_id: lead.clone(),
        member_kwants: vec![lead],
        workspace_path: folder.map(|f| f.to_string_lossy().to_string()),
        repo_url: None,
    };
    env.state.db.create_team(space, &t).await.unwrap();
}

fn uri() -> String {
    uri_of(&SpaceId::default_space())
}

fn uri_of(space: &SpaceId) -> String {
    format!("/api/spaces/{}/dev_mode", space.as_str())
}

/// Every file under `dir` that holds `needle`.
fn files_with(dir: &Path, needle: &[u8], out: &mut Vec<String>) {
    for entry in std::fs::read_dir(dir).into_iter().flatten().flatten() {
        let p = entry.path();
        if p.is_dir() {
            files_with(&p, needle, out);
        } else if std::fs::read(&p).is_ok_and(|b| b.windows(needle.len()).any(|w| w == needle)) {
            out.push(p.display().to_string());
        }
    }
}

#[tokio::test]
async fn an_empty_space_turns_on_with_no_repo_and_the_poller_idles() {
    let env = setup().await;
    let (s, v) = call(&env.app, "GET", &uri(), None).await;
    assert_eq!(s.as_u16(), 200, "{v}");
    assert_eq!((v["status"].as_str(), v["sources"].clone(), v["repo"].clone()), (Some("no_repo"), json!([]), Value::Null), "{v}");

    let (s, v) = call(&env.app, "PUT", &uri(), Some(json!({"on": true}))).await;
    assert_eq!(s.as_u16(), 200, "on with no repo is not a 409: {v}");
    assert_eq!(v["on"], true);
    assert_eq!(v["repo"], Value::Null);
    assert_eq!(v["status"], "no_repo");
    assert!(v.get("check").is_none(), "nothing was asked for: {v}");
    // The floor turns on with the switch, repo or not
    assert!(env.state.db.any_dev_mode_on());
    let mode = env.state.db.dev_mode(SpaceId::default_space().as_str()).await.unwrap().unwrap();
    assert!(mode.on && mode.repo.is_empty());
    // The poller idles: no read, no error
    poller::poll(&env.state, &mode).await.expect("an empty repo polls clean");
    poller::poll_logged(&env.state, mode).await;
    let after = env.state.db.dev_mode(SpaceId::default_space().as_str()).await.unwrap().unwrap();
    assert_eq!(after.poll_error, None);

    // Off and on again keeps working
    let (s, _) = call(&env.app, "PUT", &uri(), Some(json!({"on": false}))).await;
    assert_eq!(s.as_u16(), 200);
    let (s, v) = call(&env.app, "PUT", &uri(), Some(json!({"on": true, "backup_command": "pg_dump demo > /tmp/demo.sql"}))).await;
    assert_eq!((s.as_u16(), v["on"].clone(), v["backup_command"].clone()), (200, json!(true), json!("pg_dump demo > /tmp/demo.sql")), "{v}");
}

#[tokio::test]
async fn with_no_connection_it_still_turns_on_and_no_token_leaks() {
    let logs = capture_logs();
    let env = setup().await;
    let token = canary_token();
    let folders = env.tmp.path().join("folders");
    let web = checkout(&folders.join("web"), Some(&format!("https://x-access-token:{token}@github.com/{REPO}.git")));
    let ops = checkout(&folders.join("ops"), Some(&format!("https://deploy:{token}@gitlab.example.com/group/sub/ops.git")));
    git(&ops, &["symbolic-ref", "refs/remotes/origin/HEAD", "refs/remotes/origin/trunk"]);
    team(&env, "Web", Some(&web)).await;
    team(&env, "Ops", Some(&ops)).await;
    let mut replies = Vec::new();

    let (s, v) = call(&env.app, "GET", &uri(), None).await;
    assert_eq!(s.as_u16(), 200, "{v}");
    assert_eq!(v["status"], "not_connected", "{v}");
    assert_eq!(v["repo"], REPO, "the folder's repo is the prefill");
    assert_eq!(
        v["sources"],
        json!([
            {"team": "Web", "repo": REPO, "host": "github.com", "from": "folder"},
            {"team": "Ops", "repo": "group/sub/ops", "host": "gitlab.example.com", "from": "folder"},
        ])
    );
    replies.push(v);

    let (s, v) = call(&env.app, "PUT", &uri(), Some(json!({"on": true}))).await;
    assert_eq!(s.as_u16(), 200, "no connection is not a 409: {v}");
    assert_eq!((v["on"].clone(), v["repo"].clone(), v["status"].clone()), (json!(true), Value::Null, json!("not_connected")), "{v}");
    assert_eq!(v["branch"], "main", "the floor still learns the folder's branch");
    replies.push(v);

    // A repo asked for by name that nothing reads: on anyway, with the check saying why
    let (s, v) = call(&env.app, "PUT", &uri(), Some(json!({"on": true, "repo": format!("https://u:{token}@github.com/{REPO}")}))).await;
    assert_eq!(s.as_u16(), 200, "{v}");
    assert_eq!((v["on"].clone(), v["repo"].clone(), v["check"]["state"].clone(), v["check"]["repo"].clone()), (json!(true), Value::Null, json!("not_connected"), json!(REPO)), "the token is dropped and the repo still read: {v}");
    replies.push(v);
    // ...and one with no connection problem but no such repo name says not_found (Pixel's two reasons)
    let (s, v) = call(&env.app, "PUT", &uri(), Some(json!({"on": true, "repo": "not a repo"}))).await;
    assert_eq!((s.as_u16(), v["on"].clone(), v["repo"].clone(), v["check"]["state"].clone()), (200, json!(true), Value::Null, json!("not_found")), "{v}");
    replies.push(v);
    let (_, v) = call(&env.app, "POST", &format!("{}/check", uri()), Some(json!({}))).await;
    assert_eq!((v["state"].clone(), v["repo"].clone()), (json!("not_connected"), json!(REPO)), "the check reads the folder's repo: {v}");
    replies.push(v);

    // A space whose only remotes are on other hosts says so
    let teams = env.state.db.list_teams(&SpaceId::default_space()).await.unwrap();
    env.state.db.delete_team(&teams.iter().find(|t| t.name == "Web").unwrap().id).await.unwrap();
    let (_, v) = call(&env.app, "GET", &uri(), None).await;
    assert_eq!((v["status"].clone(), v["sources"][0]["host"].clone()), (json!("other_host"), json!("gitlab.example.com")), "{v}");
    replies.push(v);

    let folder = folders.to_string_lossy().to_string();
    for r in &replies {
        let text = r.to_string();
        assert!(!text.contains(&token), "the token reached a reply: {text}");
        assert!(!text.contains(&folder), "a folder path reached a reply: {text}");
    }
    let log = String::from_utf8_lossy(&logs.lock().unwrap()).to_string();
    assert!(!log.contains(&token), "the token reached a log line");
    let mut leaked = Vec::new();
    files_with(&env.tmp.path().join("data"), token.as_bytes(), &mut leaked);
    assert!(leaked.is_empty(), "the token was stored in {leaked:?}");
}

#[tokio::test]
async fn a_github_checkout_is_watched_with_no_repo_given() {
    let _logs = capture_logs();
    let env = setup().await;
    env.state.db.set_space_approvals(&SpaceId::default_space(), "ask").await.unwrap();
    let token = canary_token();
    let gh = MockGithub::start(&PORTS).await;
    gh.require_token(&token);
    let r = format!("/repos/{REPO}");
    gh.json("/user", user("demo-person"))
        .json(&r, repo(REPO, "main"))
        .json(&format!("{r}/contents"), json!([{"name": "fly.toml", "type": "file"}]));
    let (s, v) = call(&env.app, "POST", "/api/zombie/sources", Some(json!({"kind": "github", "label": "GitHub", "config": {"api_base": gh.base}, "secret": token}))).await;
    assert_eq!(s.as_u16(), 201, "{v}");

    // The team's folder is a checkout of the repo on the connection's host, its URL holding a
    // token of its own; a second team with no git folder comes first
    let folder_token = canary_token();
    let host = gh.base.trim_start_matches("http://");
    let web = checkout(&env.tmp.path().join("folders/web"), Some(&format!("http://bot:{folder_token}@{host}/{REPO}.git")));
    let empty = env.tmp.path().join("folders/plain");
    std::fs::create_dir_all(&empty).unwrap();
    team(&env, "Plain", Some(&empty)).await;
    team(&env, "Web", Some(&web)).await;

    let (_, v) = call(&env.app, "GET", &uri(), None).await;
    assert_eq!((v["on"].clone(), v["status"].clone(), v["repo"].clone()), (json!(false), json!("watching"), json!(REPO)), "off, it says what on would watch: {v}");
    let (s, v) = call(&env.app, "PUT", &uri(), Some(json!({"on": true}))).await;
    assert_eq!(s.as_u16(), 200, "{v}");
    assert_eq!(v["repo"], REPO, "{v}");
    assert_eq!(v["branch"], "main");
    assert_eq!(v["deploy_hint"], "fly");
    assert_eq!(v["status"], "watching");
    assert_eq!(v["sources"], json!([{"team": "Web", "repo": REPO, "host": "127.0.0.1", "from": "folder"}]));
    let (_, got) = call(&env.app, "GET", &uri(), None).await;
    assert_eq!((got["status"].clone(), got["repo"].clone()), (json!("watching"), json!(REPO)));
    for text in [v.to_string(), got.to_string()] {
        assert!(!text.contains(&folder_token) && !text.contains(&token), "a token reached a reply: {text}");
    }

    // A watched repo stays watched when GitHub stops answering
    gh.fault(&r, Fault::Server(502));
    let (s, v) = call(&env.app, "PUT", &uri(), Some(json!({"on": true}))).await;
    assert_eq!((s.as_u16(), v["repo"].clone(), v["branch"].clone()), (200, json!(REPO), json!("main")), "{v}");
    gh.heal(&r);
    gh.assert_read_only();
}

fn sha(n: u8) -> String {
    format!("{n:0>40}")
}

/// Two commits on main (one through a merged PR) and one open PR, as the poller reads them.
fn seed(gh: &MockGithub, r: &str) {
    let person = ("Lin (kwant bot)", "lin-bot@users.noreply.example.com");
    let with_files = |mut c: Value, files: Value| {
        c["files"] = files;
        c
    };
    let patch = "@@ -1,2 +1,2 @@\n-let retries = 1;\n+let retries = 3;";
    let merged = commit(&sha(2), "Retry the upload (#5)", person, &[&sha(1)], "2026-11-05T02:00:00Z");
    let direct = commit(&sha(1), "Fix the README", person, &[], "2026-11-05T01:00:00Z");
    gh.json(&format!("{r}/branches/main"), json!({"name": "main"}))
        .json(&format!("{r}/tags"), json!([]))
        .json(&format!("{r}/commits"), json!([merged, direct]))
        .json(&format!("{r}/commits/{}", sha(2)), with_files(merged.clone(), json!([file("src/upload.rs", "modified", patch)])))
        .json(&format!("{r}/commits/{}", sha(1)), with_files(direct.clone(), json!([file("README.md", "modified", "-old\n+new")])))
        .json(&format!("{r}/commits/{}/pulls", sha(2)), json!([pull(5, "Retry the upload", ("retry", &sha(9)), "main", "demo-person", Some((&sha(2), "2026-11-05T02:00:00Z")))]))
        .json(&format!("{r}/commits/{}/pulls", sha(1)), json!([]))
        .json(&format!("{r}/pulls/5/files"), json!([file("src/upload.rs", "modified", patch)]))
        .json(&format!("{r}/commits/{}/check-runs", sha(2)), check_runs(&[("rust", "completed", Some("success"))]))
        .json(&format!("{r}/commits/{}/check-runs", sha(1)), check_runs(&[("rust", "completed", Some("success"))]))
        .json(&format!("{r}/pulls"), json!([pull(8, "Add a cache", ("cache", &sha(3)), "main", "demo-person", None)]))
        .json(&format!("{r}/pulls/8/files"), json!([file("src/cache.rs", "added", "+pub struct Cache;")]))
        .json(&format!("{r}/commits/{}", sha(3)), commit(&sha(3), "Add a cache", person, &[&sha(2)], "2026-11-05T03:00:00Z"))
        .json(&format!("{r}/commits/{}/check-runs", sha(3)), check_runs(&[("rust", "completed", Some("success"))]));
}

/// Plan user-flow.md F4, F6 and F10, in a space of its own so the poller's wake list is this
/// test's alone.
#[tokio::test]
async fn a_detected_repo_wakes_the_poller_counts_new_reviews_and_never_replaces_a_typed_one() {
    let env = setup().await;
    let space = env.state.db.create_space(&NewSpace { name: "Shop".into(), icon: "S".into(), color: None, this_machine_enabled: true }).await.unwrap();
    let space = SpaceId::new(space.id);
    // An asking install (every space), so the login may be saved here
    for s in [&space, &SpaceId::default_space()] {
        env.state.db.set_space_approvals(s, "ask").await.unwrap();
    }
    let token = canary_token();
    let gh = MockGithub::start(&PORTS).await;
    gh.require_token(&token);
    let r = format!("/repos/{REPO}");
    gh.json("/user", user("demo-person")).json(&r, repo(REPO, "main")).json(&format!("{r}/contents"), json!([{"name": "Cargo.toml", "type": "file"}]));
    seed(&gh, &r);
    let other = "demo-org/typed-app";
    gh.json(&format!("/repos/{other}"), repo(other, "main")).json(&format!("/repos/{other}/contents"), json!([]));
    let (s, v) = call(&env.app, "POST", "/api/zombie/sources", Some(json!({"kind": "github", "label": "GitHub", "config": {"api_base": gh.base}, "secret": token}))).await;
    assert_eq!(s.as_u16(), 201, "{v}");
    let host = gh.base.trim_start_matches("http://").split(':').next().unwrap();
    let web = checkout(&env.tmp.path().join("folders/shop"), Some(&format!("{}/{REPO}.git", gh.base)));
    team_in(&env, &space, "Shop", Some(&web)).await;
    let u = uri_of(&space);

    // F4: switching on with the repo found in the folder wakes the poller for this space at once
    assert!(!poller::is_due_soon(space.as_str()));
    let (s, v) = call(&env.app, "PUT", &u, Some(json!({"on": true}))).await;
    assert_eq!((s.as_u16(), v["repo"].clone(), v["status"].clone()), (200, json!(REPO), json!("watching")), "{v}");
    assert_eq!(v["sources"], json!([{"team": "Shop", "repo": REPO, "host": host, "from": "folder"}]));
    assert!(poller::is_due_soon(space.as_str()), "on with a detected repo wakes the poller");
    // ...and that first poll backfills (both commits are within BACKFILL) plus the open PR
    let mode = env.state.db.dev_mode(space.as_str()).await.unwrap().unwrap();
    poller::poll(&env.state, &mode).await.expect("a clean first poll");
    let landed = env.state.db.dev_cards(space.as_str(), LANDED, None, 50).await.unwrap();
    let reviews = env.state.db.dev_cards(space.as_str(), REVIEW, None, 50).await.unwrap();
    assert_eq!((landed.len(), reviews.len()), (2, 1), "the first poll backfilled");
    assert!(landed.len() <= poller::BACKFILL);

    // F6: never seen counts nothing, so an update shows no surprise badge
    let (_, v) = call(&env.app, "GET", &u, None).await;
    assert_eq!(v["new_reviews"], 0, "{v}");
    let (s, v) = call(&env.app, "POST", &format!("{u}/seen"), None).await;
    assert_eq!(s.as_u16(), 200, "{v}");
    assert!(v["seen_at"].is_string(), "{v}");
    tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    let fresh_review = env.state.db.insert_dev_card(space.as_str(), REVIEW, "pr:41", &sha(41), "{}").await.unwrap().unwrap();
    env.state.db.set_dev_card(&fresh_review.id, READY, Some(&json!({"headline": "Review"}).to_string())).await.unwrap();
    let fresh_landed = env.state.db.insert_dev_card(space.as_str(), LANDED, &sha(42), &sha(42), "{}").await.unwrap().unwrap();
    env.state.db.set_dev_card(&fresh_landed.id, READY, Some(&json!({"headline": "Landed"}).to_string())).await.unwrap();
    let (_, v) = call(&env.app, "GET", &u, None).await;
    assert_eq!(v["new_reviews"], 1, "one review since seen; the landed card doesn't count: {v}");
    let (_, v) = call(&env.app, "PUT", &u, Some(json!({"on": true}))).await;
    assert_eq!(v["new_reviews"], 1, "the PUT reply carries it too: {v}");
    let (s, _) = call(&env.app, "POST", &format!("{u}/seen"), None).await;
    assert_eq!(s.as_u16(), 200);
    let (_, v) = call(&env.app, "GET", &u, None).await;
    assert_eq!(v["new_reviews"], 0, "seen clears the count: {v}");

    // F10: a repo the person typed is kept; the folder's repo never replaces it
    let (s, v) = call(&env.app, "PUT", &u, Some(json!({"on": true, "repo": other}))).await;
    assert_eq!((s.as_u16(), v["repo"].clone()), (200, json!(other)), "{v}");
    let (_, v) = call(&env.app, "PUT", &u, Some(json!({"on": true}))).await;
    assert_eq!((v["repo"].clone(), v["status"].clone()), (json!(other), json!("watching")), "{v}");
    let (_, v) = call(&env.app, "PUT", &u, Some(json!({"on": false}))).await;
    assert_eq!(v["on"], false);
    let (_, v) = call(&env.app, "PUT", &u, Some(json!({"on": true}))).await;
    assert_eq!(v["repo"], json!(other), "off and on again keeps the typed repo: {v}");
    let (_, v) = call(&env.app, "GET", &u, None).await;
    assert_eq!((v["repo"].clone(), v["status"].clone()), (json!(other), json!("watching")), "{v}");
    gh.assert_read_only();
}

/// QA B1: on with a detected repo and no connection, then the person connects GitHub. The repo is
/// bound and the poller wakes with no off and on; a space that is off is left alone.
#[tokio::test]
async fn connecting_github_while_on_binds_the_detected_repo_without_off_and_on() {
    let env = setup().await;
    let mk = |name: &str| NewSpace { name: name.into(), icon: "S".into(), color: None, this_machine_enabled: true };
    let space = SpaceId::new(env.state.db.create_space(&mk("Shop")).await.unwrap().id);
    let off = SpaceId::new(env.state.db.create_space(&mk("Idle")).await.unwrap().id);
    for s in [&space, &off, &SpaceId::default_space()] {
        env.state.db.set_space_approvals(s, "ask").await.unwrap();
    }
    let token = canary_token();
    let gh = MockGithub::start(&PORTS).await;
    gh.require_token(&token);
    let r = format!("/repos/{REPO}");
    gh.json("/user", user("demo-person")).json(&r, repo(REPO, "main")).json(&format!("{r}/contents"), json!([{"name": "fly.toml", "type": "file"}]));
    seed(&gh, &r);
    let web = checkout(&env.tmp.path().join("folders/shop"), Some(&format!("{}/{REPO}.git", gh.base)));
    team_in(&env, &space, "Shop", Some(&web)).await;
    team_in(&env, &off, "Idle", Some(&web)).await;

    // On before any connection: no repo. (The mock's host only counts as GitHub once a connection
    // names it, so here it reads other_host; on github.com it is not_connected.)
    let u = uri_of(&space);
    let (s, v) = call(&env.app, "PUT", &u, Some(json!({"on": true}))).await;
    assert_eq!((s.as_u16(), v["on"].clone(), v["repo"].clone()), (200, json!(true), Value::Null), "{v}");
    let (_, v) = call(&env.app, "GET", &u, None).await;
    assert_eq!(v["repo"], Value::Null, "no connection yet: {v}");
    // The wake from turning on has been taken by a poll, which had nothing to read
    let mode = env.state.db.dev_mode(space.as_str()).await.unwrap().unwrap();
    poller::poll(&env.state, &mode).await.unwrap();
    poller::take_due_soon(space.as_str());
    assert!(!poller::is_due_soon(space.as_str()));

    // The person connects GitHub (Loose ends) and comes back: no off, no on, no typing
    let (s, v) = call(&env.app, "POST", "/api/zombie/sources", Some(json!({"kind": "github", "label": "GitHub", "config": {"api_base": gh.base}, "secret": token}))).await;
    assert_eq!(s.as_u16(), 201, "{v}");
    assert!(!v.to_string().contains(&token));
    assert!(poller::is_due_soon(space.as_str()), "binding wakes the poller");
    let (_, v) = call(&env.app, "GET", &u, None).await;
    assert_eq!((v["on"].clone(), v["repo"].clone(), v["branch"].clone(), v["status"].clone()), (json!(true), json!(REPO), json!("main"), json!("watching")), "{v}");
    assert_eq!(v["deploy_hint"], "fly");
    let mode = env.state.db.dev_mode(space.as_str()).await.unwrap().unwrap();
    poller::poll(&env.state, &mode).await.expect("the bound repo polls");
    assert_eq!(env.state.db.dev_cards(space.as_str(), LANDED, None, 50).await.unwrap().len(), 2, "the first poll backfilled");

    // A space that is off stays off with no repo; its GET only says what on would watch
    assert!(env.state.db.dev_mode(off.as_str()).await.unwrap().is_none());
    let (_, v) = call(&env.app, "GET", &uri_of(&off), None).await;
    assert_eq!((v["on"].clone(), v["status"].clone()), (json!(false), json!("watching")), "{v}");
    assert!(env.state.db.dev_mode(off.as_str()).await.unwrap().is_none(), "a GET never writes an off space");

    // A changed GitHub source binds every space that is on with no repo, with no GET needed
    let changed = SpaceId::new(env.state.db.create_space(&mk("Changed")).await.unwrap().id);
    let read = SpaceId::new(env.state.db.create_space(&mk("Read")).await.unwrap().id);
    for s in [&changed, &read] {
        team_in(&env, s, "Web", Some(&web)).await;
        env.state.db.put_dev_mode(s.as_str(), true, "", "", None, None).await.unwrap();
    }
    let (s, v) = call(&env.app, "GET", "/api/zombie/sources", None).await;
    assert_eq!(s.as_u16(), 200, "{v}");
    let id = v["sources"].as_array().unwrap().iter().find(|x| x["kind"] == "github").unwrap()["id"].as_str().unwrap().to_string();
    let (s, v) = call(&env.app, "PATCH", &format!("/api/zombie/sources/{id}"), Some(json!({"label": "GitHub (work)"}))).await;
    assert_eq!(s.as_u16(), 200, "{v}");
    for s in [&changed, &read] {
        let m = env.state.db.dev_mode(s.as_str()).await.unwrap().unwrap();
        assert_eq!((m.on, m.repo.as_str(), m.branch.as_str()), (true, REPO, "main"), "the source change bound {}", s.as_str());
    }
    // When no source change happened, the GET falls back to the same binding
    let late = SpaceId::new(env.state.db.create_space(&mk("Late")).await.unwrap().id);
    team_in(&env, &late, "Web", Some(&web)).await;
    env.state.db.put_dev_mode(late.as_str(), true, "", "", None, None).await.unwrap();
    let (_, v) = call(&env.app, "GET", &uri_of(&late), None).await;
    assert_eq!((v["repo"].clone(), v["status"].clone()), (json!(REPO), json!("watching")), "the GET fallback bound it: {v}");
    assert!(poller::is_due_soon(late.as_str()));
    gh.assert_read_only();
}
