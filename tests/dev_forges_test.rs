//! Dev mode on GitLab, Gitea and Forgejo, and on repos with no forge (plan dev-mode/plan/any-git.md
//! G4, G5): each forge adapter reads open changes, CI and merged changes from a mock forge on a
//! loopback `api_base`; a Loose ends connection of kind `forge` is told which forge it is by asking
//! the server, one per host; and a repo with no forge gets a review card for each pushed branch
//! that is ahead of the watched one, with no PR, no CI and no Post.
//!
//! Git repos are throwaway `git init`s in a temp dir. Nothing reaches a real forge: every request
//! goes to a mock on 127.0.0.1 and an OS-chosen port, and only GETs are answered.

mod common;
#[path = "support/mock_github.rs"]
mod mock_github;

use common::*;
use kwnt::db::dev_mode::{DevRepoRow, REVIEW, STALE};
use kwnt::db::models::NewTeam;
use kwnt::db::spaces::SpaceId;
use kwnt::server::dev::facts::{CiState, DevFacts};
use kwnt::server::dev::forge::{self, ForgeClient, ForgeConfig, ForgeKind};
use kwnt::server::dev::git::{self, Repo};
use kwnt::server::dev::poller;
use kwnt::server::dev::source::History;
use mock_github::*;
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use zeroize::Zeroizing;

fn git(dir: &Path, args: &[&str]) -> String {
    let out = std::process::Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(["-c", "user.name=Juno (kwant bot)", "-c", "user.email=juno-bot@users.noreply.example.com", "-c", "commit.gpgsign=false"])
        .args(args)
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .output()
        .unwrap();
    assert!(out.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&out.stderr));
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

fn commit(dir: &Path, file: &str, text: &str, message: &str) -> String {
    std::fs::write(dir.join(file), text).unwrap();
    git(dir, &["add", "--", file]);
    git(dir, &["commit", "-q", "-m", message]);
    git(dir, &["rev-parse", "HEAD"])
}

/// A team whose folder is a fresh repo on `main` with one commit, watched by Dev mode. Returns the
/// folder, the root commit and the watched row.
async fn watched(env: &Env, name: &str) -> (PathBuf, String, DevRepoRow) {
    let space = SpaceId::default_space();
    let folder = env.tmp.path().join("folders").join(name);
    std::fs::create_dir_all(&folder).unwrap();
    git(&folder, &["init", "-q", "-b", "main"]);
    let root = commit(&folder, "README.md", "# App\n", "Start the repo");
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
    let key = format!("local:{name}");
    let row = env.state.db.dev_repo(space.as_str(), &key).await.unwrap().unwrap();
    (folder, root, row)
}

/// The repo's review cards, oldest first, with their facts.
async fn reviews(env: &Env, key: &str) -> Vec<(kwnt::db::dev_mode::DevCardRow, DevFacts)> {
    let space = SpaceId::default_space();
    let mut rows = env.state.db.dev_cards_in(space.as_str(), Some(key), REVIEW, None, 50).await.unwrap();
    rows.sort_by(|a, b| a.created_at.cmp(&b.created_at).then(a.id.cmp(&b.id)));
    rows.into_iter().map(|r| {
        let f = serde_json::from_str(&r.facts).unwrap();
        (r, f)
    }).collect()
}

fn live(cards: &[(kwnt::db::dev_mode::DevCardRow, DevFacts)]) -> Vec<&(kwnt::db::dev_mode::DevCardRow, DevFacts)> {
    cards.iter().filter(|(r, _)| r.status != STALE).collect()
}

/// A client for the mock forge, as Loose ends would make it.
fn client(mock: &MockGithub, kind: ForgeKind, token: &str) -> ForgeClient {
    let cfg = ForgeConfig { url: mock.base.clone(), forge: Some(kind) };
    cfg.validate().unwrap();
    ForgeClient::new("src-test", cfg, Zeroizing::new(token.to_string()), Arc::new(git::SystemResolver)).expect("a GitLab or Gitea client")
}

fn every_hit_authorized(mock: &MockGithub) {
    let hits = mock.hits();
    assert!(!hits.is_empty());
    assert!(hits.iter().all(|h| h.method == "GET" && h.authorized), "{:?}", hits.iter().map(|h| (&h.method, &h.path, h.authorized)).collect::<Vec<_>>());
    mock.assert_read_only();
}

fn mr(iid: u64, title: &str, head: (&str, &str), base: &str, draft: bool) -> Value {
    json!({
        "iid": iid, "title": title, "description": "What it does.", "draft": draft, "state": "opened",
        "web_url": format!("https://gitlab.example.com/group/app/-/merge_requests/{iid}"),
        "author": {"username": "ana"}, "source_branch": head.0, "sha": head.1, "target_branch": base,
        "updated_at": mock_github::recent(), "merged_at": null, "merge_commit_sha": null,
    })
}

fn pr(number: u64, title: &str, head: (&str, &str), base: &str) -> Value {
    json!({
        "number": number, "title": title, "body": "What it does.", "state": "open",
        "html_url": format!("https://codeberg.example.com/org/app/pulls/{number}"),
        "user": {"login": "ana"}, "head": {"ref": head.0, "sha": head.1}, "base": {"ref": base},
        "merged": false, "merged_at": null, "merge_commit_sha": null, "updated_at": mock_github::recent(),
    })
}

#[tokio::test]
async fn gitlab_merge_requests_get_review_cards_with_pipelines() {
    let env = setup().await;
    let (folder, root, row) = watched(&env, "gl").await;
    git(&folder, &["checkout", "-q", "-b", "feature"]);
    let head = commit(&folder, "lib.rs", "pub fn x() {}\n", "Add lib");
    git(&folder, &["checkout", "-q", "main"]);

    let mock = MockGithub::start(&[]).await;
    let token = canary_token();
    mock.require_token(&token);
    let p = "/api/v4/projects/group%2Fapp";
    mock.json(
        &format!("{p}/merge_requests?state=opened&target_branch=main&order_by=updated_at&sort=desc&per_page=50"),
        json!([
            mr(7, "Add lib", ("feature", &head), "main", false),
            mr(8, "Draft: half done", ("wip", &head), "main", true),
            mr(9, "Into another branch", ("other", &head), "release", false),
        ]),
    );
    // A retried job (the newer status wins) and a failure that's allowed to fail
    mock.json(
        &format!("{p}/repository/commits/{head}/statuses?all=true&per_page=100"),
        json!([
            {"id": 1, "name": "test", "status": "failed", "target_url": "https://gitlab.example.com/group/app/-/jobs/1"},
            {"id": 3, "name": "test", "status": "success", "target_url": "https://gitlab.example.com/group/app/-/jobs/3"},
            {"id": 2, "name": "lint", "status": "failed", "allow_failure": true, "target_url": "http://not-https.example.com/2"},
        ]),
    );
    // The root commit came from a merged MR
    let mut merged = mr(5, "Start the repo", ("start", &root), "main", false);
    merged["state"] = json!("merged");
    merged["merged_at"] = json!("2026-11-05T09:00:00Z");
    merged["merge_commit_sha"] = json!(root);
    merged["merge_user"] = json!({"username": "lin"});
    mock.json(&format!("{p}/repository/commits/{root}/merge_requests"), json!([merged]));
    mock.json(&format!("{p}/merge_requests/7"), mr(7, "Add lib", ("feature", &head), "main", false));
    mock.json("/api/v4/user", json!({"username": "juno-bot", "id": 4}));

    let c = client(&mock, ForgeKind::Gitlab, &token);
    let f = c.forge_for("group/app");
    assert_eq!(f.kind(), ForgeKind::Gitlab);
    assert_eq!(f.change_head_ref(7).as_deref(), Some("refs/merge-requests/7/head"));
    assert_eq!(f.login().await.unwrap(), "juno-bot");
    let one = f.change(7).await.unwrap().unwrap();
    assert_eq!((one.number, one.head_sha.as_str(), one.base_branch.as_str(), one.head_branch.as_str()), (7, head.as_str(), "main", "feature"));
    assert!(f.change(70).await.unwrap().is_none(), "a 404 is no such change");

    let repo = Repo::local(&folder, &env.state.config.data_dir).await.unwrap();
    let h = History::new(Some(repo), String::new(), None, None, "main", None);
    poller::read(&env.state, &row, &h, Some(f.as_ref())).await.unwrap();

    // What landed carries the MR it came from
    let space = SpaceId::default_space();
    let landed = env.state.db.dev_cards_in(space.as_str(), Some(&row.key), kwnt::db::dev_mode::LANDED, None, 10).await.unwrap();
    let lf: DevFacts = serde_json::from_str(&landed[0].facts).unwrap();
    assert_eq!((lf.pr, lf.forge.as_deref(), lf.no_forge), (Some(5), Some("gitlab"), false), "{lf:?}");

    // One review card: the draft and the MR into another branch get none
    let cards = reviews(&env, &row.key).await;
    assert_eq!(cards.len(), 1, "{:?}", cards.iter().map(|c| &c.0.ref_).collect::<Vec<_>>());
    let (card, facts) = &cards[0];
    assert_eq!((card.ref_.as_str(), card.head_sha.as_str()), ("pr:7", head.as_str()));
    assert_eq!((facts.pr, facts.forge.as_deref(), facts.no_forge, facts.title.as_str()), (Some(7), Some("gitlab"), false, "Add lib"));
    assert_eq!(facts.files.iter().map(|f| (f.path.as_str(), f.status.as_str())).collect::<Vec<_>>(), [("lib.rs", "added")], "the diff from git");
    let checks: Vec<_> = facts.checks.iter().map(|c| (c.name.as_str(), c.status.as_str(), c.conclusion.as_deref(), c.url.as_deref())).collect();
    assert_eq!(checks, [("lint", "completed", Some("neutral"), None), ("test", "completed", Some("success"), Some("https://gitlab.example.com/group/app/-/jobs/3"))]);
    assert_eq!(facts.ci, CiState::Green);
    assert_eq!(facts.links.pr.as_deref(), Some("https://gitlab.example.com/group/app/-/merge_requests/7"));

    // The card says which forge it's on; Post refuses until posting to GitLab comes (G7)
    let (s, v) = call(&env.app, "GET", &format!("/api/dev/cards/{}", card.id), None).await;
    assert_eq!(s.as_u16(), 200, "{v}");
    assert_eq!((v["forge"].clone(), v["no_forge"].clone()), (json!("gitlab"), json!(false)), "{v}");

    // No statuses: the commit's newest pipeline is the check
    mock.json(&format!("{p}/repository/commits/{root}/statuses?all=true&per_page=100"), json!([]));
    mock.json(
        &format!("{p}/pipelines?sha={root}&per_page=20"),
        json!([{"id": 11, "status": "failed", "web_url": "https://gitlab.example.com/p/11"}, {"id": 12, "status": "running", "web_url": "https://gitlab.example.com/p/12"}]),
    );
    let checks = f.checks(&root, &Default::default()).await.unwrap();
    assert_eq!(checks.len(), 1);
    assert_eq!((checks[0].name.as_str(), checks[0].status.as_str(), checks[0].conclusion.as_deref()), ("pipeline", "in_progress", None));

    // A token the server refuses says so, and where to fix it
    let bad = client(&mock, ForgeKind::Gitlab, "glpat-wrong");
    let e = bad.forge_for("group/app").open_changes("main", &Default::default()).await.unwrap_err().to_string();
    assert!(e.contains("GitLab refused the token (401)"), "{e}");
    assert!(!format!("{bad:?}").contains("glpat-wrong"), "the token stays out of Debug");
    every_hit_authorized(&mock);
}

#[tokio::test]
async fn gitea_pull_requests_get_review_cards_with_statuses() {
    let env = setup().await;
    let (folder, root, row) = watched(&env, "gt").await;
    git(&folder, &["checkout", "-q", "-b", "feature"]);
    let head = commit(&folder, "app.rs", "fn main() {}\n", "Add the app");
    git(&folder, &["checkout", "-q", "main"]);

    let mock = MockGithub::start(&[]).await;
    let token = canary_token();
    mock.require_token(&token);
    let r = "/api/v1/repos/org/app";
    mock.json(
        &format!("{r}/pulls?state=open&sort=recentupdate&limit=50"),
        json!([pr(3, "Add the app", ("feature", &head), "main"), pr(4, "WIP: not yet", ("wip", &head), "main"), pr(6, "Elsewhere", ("x", &head), "dev")]),
    );
    mock.json(
        &format!("{r}/commits/{head}/statuses?limit=50"),
        json!([
            {"id": 1, "context": "ci/build", "status": "pending"},
            {"id": 2, "context": "ci/build", "status": "failure", "target_url": "https://ci.example.com/2"},
            {"id": 3, "context": "lint", "state": "warning"},
        ]),
    );
    let mut merged = pr(2, "Start the repo", ("start", &root), "main");
    merged["merged"] = json!(true);
    merged["merged_at"] = json!("2026-11-05T09:00:00Z");
    merged["merge_commit_sha"] = json!(root);
    merged["merged_by"] = json!({"login": "lin"});
    mock.json(&format!("{r}/commits/{root}/pull"), merged);
    mock.json("/api/v1/user", json!({"login": "juno-bot", "id": 4}));

    let c = client(&mock, ForgeKind::Gitea, &token);
    let f = c.forge_for("org/app");
    assert_eq!((f.kind(), f.change_head_ref(3).as_deref()), (ForgeKind::Gitea, Some("refs/pull/3/head")));
    assert_eq!(f.login().await.unwrap(), "juno-bot");

    let repo = Repo::local(&folder, &env.state.config.data_dir).await.unwrap();
    let h = History::new(Some(repo), String::new(), None, None, "main", None);
    poller::read(&env.state, &row, &h, Some(f.as_ref())).await.unwrap();

    let space = SpaceId::default_space();
    let landed = env.state.db.dev_cards_in(space.as_str(), Some(&row.key), kwnt::db::dev_mode::LANDED, None, 10).await.unwrap();
    let lf: DevFacts = serde_json::from_str(&landed[0].facts).unwrap();
    assert_eq!((lf.pr, lf.forge.as_deref()), (Some(2), Some("gitea")), "{lf:?}");

    let cards = reviews(&env, &row.key).await;
    assert_eq!(cards.iter().map(|c| c.0.ref_.as_str()).collect::<Vec<_>>(), ["pr:3"], "the WIP pull and the one into dev get none");
    let facts = &cards[0].1;
    assert_eq!((facts.forge.as_deref(), facts.no_forge, facts.files.len()), (Some("gitea"), false, 1));
    let checks: Vec<_> = facts.checks.iter().map(|c| (c.name.as_str(), c.conclusion.as_deref())).collect();
    assert_eq!(checks, [("ci/build", Some("failure")), ("lint", Some("neutral"))]);
    assert_eq!(facts.ci, CiState::Red);

    // Pulled files when git can't read the head: Gitea lists them without patches
    mock.json(
        &format!("{r}/pulls/3/files?limit=50&page=1"),
        json!([{"filename": "app.rs", "status": "added", "additions": 1, "deletions": 0}, {"filename": "new.rs", "previous_filename": "old.rs", "status": "renamed"}]),
    );
    let files = f.change_files(3).await.unwrap();
    assert_eq!(files.iter().map(|f| (f.path.as_str(), f.status.as_str(), f.previous_path.as_deref())).collect::<Vec<_>>(), [("app.rs", "added", None), ("new.rs", "renamed", Some("old.rs"))]);
    every_hit_authorized(&mock);
}

#[tokio::test]
async fn a_forge_connection_is_asked_which_forge_it_is_one_per_host() {
    let env = setup().await;
    let space = SpaceId::default_space();
    env.state.db.set_space_approvals(&space, "ask").await.unwrap();
    let mock = MockGithub::start(&[]).await;
    let token = canary_token();
    mock.json("/api/v4/version", json!({"version": "17.5.0", "revision": "abc"}));
    mock.json("/api/v4/user", json!({"username": "juno-bot"}));

    // GitHub is its own connection
    let (s, v) = call(&env.app, "POST", "/api/zombie/sources", Some(json!({"kind": "forge", "label": "Ours", "config": {"url": mock.base, "forge": "github"}, "secret": token}))).await;
    assert_eq!(s.as_u16(), 400, "{v}");

    // No forge given: the server is asked, and its answer is kept
    let (s, v) = call(&env.app, "POST", "/api/zombie/sources", Some(json!({"kind": "forge", "label": "Our GitLab", "config": {"url": mock.base}, "secret": token}))).await;
    assert_eq!(s.as_u16(), 201, "{v}");
    assert_eq!((v["test"]["login"].clone(), v["test"]["forge"].clone()), (json!("juno-bot"), json!("gitlab")), "{v}");
    assert!(!v.to_string().contains(&token), "the token never comes back");
    let rows = env.state.db.zombie_sources().await.unwrap();
    let cfg: ForgeConfig = serde_json::from_str(&rows.iter().find(|r| r.kind == "forge").unwrap().config).unwrap();
    assert_eq!(cfg.forge, Some(ForgeKind::Gitlab));
    assert!(mock.hits_of("/api/v1/version").is_empty(), "GitLab answered first");
    // The probe goes without the token; the test with it
    assert!(mock.hits_of("/api/v4/version").iter().all(|h| !h.authorized));
    assert!(!mock.hits_of("/api/v4/user").is_empty() && mock.hits_of("/api/v4/user").iter().all(|h| h.authorized));

    // One connection per host
    let (s, v) = call(&env.app, "POST", "/api/zombie/sources", Some(json!({"kind": "forge", "label": "Again", "config": {"url": format!("{}/", mock.base), "forge": "gitlab"}, "secret": token}))).await;
    assert_eq!(s.as_u16(), 409, "{v}");
    assert!(v.to_string().contains("already has a connection"), "{v}");

    // Dev mode finds it by host, with its token
    let host = mock.base.trim_start_matches("http://").split(':').next().unwrap().to_string();
    let c = forge::client_for_host(&env.state, &host, Arc::new(git::SystemResolver)).await.expect("the connection, by host");
    assert_eq!(c.kind, ForgeKind::Gitlab);
    assert_eq!(forge::connected_hosts(&env.state).await, [(host.clone(), Some(ForgeKind::Gitlab))]);

    // A server that is neither: the person is asked to say which
    let other = MockGithub::start(&[]).await;
    let (s, v) = call(&env.app, "POST", "/api/zombie/sources", Some(json!({"kind": "forge", "label": "What", "config": {"url": other.base}, "secret": token}))).await;
    assert_eq!(s.as_u16(), 400, "{v}");
    assert!(v.to_string().contains("set \\\"forge\\\" to gitlab or gitea") || v.to_string().contains("gitlab or gitea"), "{v}");
    assert_eq!(v["code"], "forge_unknown", "a stable code beside the sentence: {v}");

    // A Gitea answers on /api/v1 only, and its token goes as `token …`
    let gitea = MockGithub::start(&[]).await;
    gitea.json("/api/v1/version", json!({"version": "1.22.0"}));
    gitea.json("/api/v1/user", json!({"login": "juno-bot"}));
    let (s, v) = call(&env.app, "POST", "/api/zombie/sources", Some(json!({"kind": "forge", "label": "Our Forgejo", "config": {"url": gitea.base}, "secret": token}))).await;
    assert_eq!(s.as_u16(), 201, "{v}");
    assert_eq!(v["test"]["forge"], "gitea", "{v}");
    assert!(gitea.hits_of("/api/v1/version").iter().all(|h| !h.authorized));
    assert!(gitea.hits_of("/api/v1/user").iter().all(|h| h.authorized));
    assert!(!gitea.hits_of("/api/forgejo/v1/version").is_empty(), "a bare version is asked once more");

    // A Forgejo says so in its version (7 and later), or answers Forgejo's own version route
    let fj = MockGithub::start(&[]).await;
    fj.json("/api/v1/version", json!({"version": "7.0.0+gitea-1.21.0"}));
    fj.json("/api/v1/user", json!({"login": "juno-bot"}));
    let (s, v) = call(&env.app, "POST", "/api/zombie/sources", Some(json!({"kind": "forge", "label": "Our Forgejo", "config": {"url": fj.base}, "secret": token}))).await;
    assert_eq!((s.as_u16(), v["test"]["forge"].clone()), (201, json!("forgejo")), "{v}");
    let old = MockGithub::start(&[]).await;
    old.json("/api/v1/version", json!({"version": "1.21.11-1"}));
    old.json("/api/forgejo/v1/version", json!({"version": "1.21.11-1+gitea-1.21.11"}));
    old.json("/api/v1/user", json!({"login": "juno-bot"}));
    let (s, v) = call(&env.app, "POST", "/api/zombie/sources", Some(json!({"kind": "forge", "label": "Old Forgejo", "config": {"url": old.base}, "secret": token}))).await;
    assert_eq!((s.as_u16(), v["test"]["forge"].clone()), (201, json!("forgejo")), "{v}");
    let fj_host = fj.base.trim_start_matches("http://").to_string();
    let rows = env.state.db.zombie_sources().await.unwrap();
    let kept = rows.iter().filter_map(|r| serde_json::from_str::<ForgeConfig>(&r.config).ok()).find(|c| c.host().as_deref() == Some(fj_host.as_str())).unwrap();
    assert_eq!(kept.forge, Some(ForgeKind::Forgejo), "kept as Forgejo");
    let c = client(&fj, ForgeKind::Forgejo, &token);
    assert_eq!(c.forge_for("juno/app").kind(), ForgeKind::Forgejo, "its cards say Forgejo");
    let (_, list) = call(&env.app, "GET", "/api/connections", None).await;
    let forgejo = list.to_string().matches("\"kind\":\"forgejo\"").count();
    assert_eq!(forgejo, 2, "Loose ends lists both as Forgejo: {list}");
    for m in [&mock, &other, &gitea, &fj, &old] {
        m.assert_read_only();
    }
}

#[tokio::test]
async fn a_repo_with_no_forge_gets_cards_for_branches_ahead_of_the_watched_one() {
    let env = setup().await;
    let (folder, _root, row) = watched(&env, "nf").await;
    // Ahead of main: a card
    git(&folder, &["checkout", "-q", "-b", "ahead"]);
    let first = commit(&folder, "ahead.txt", "one\n", "Start the ahead work\n\nWith a body.");
    // Already merged into main, and level with main: no card
    git(&folder, &["checkout", "-q", "-b", "merged", "main"]);
    commit(&folder, "merged.txt", "in\n", "Merged work");
    git(&folder, &["checkout", "-q", "main"]);
    git(&folder, &["merge", "-q", "--ff-only", "merged"]);
    git(&folder, &["branch", "-q", "same", "main"]);
    let refs_before = git(&folder, &["for-each-ref", "--format=%(refname) %(objectname)"]);

    let space = SpaceId::default_space();
    poller::poll_space(&env.state, space.as_str()).await;
    let row2 = env.state.db.dev_repo(space.as_str(), &row.key).await.unwrap().unwrap();
    assert_eq!(row2.poll_error, None);
    let cards = reviews(&env, &row.key).await;
    assert_eq!(cards.iter().map(|c| (c.0.ref_.as_str(), c.0.head_sha.as_str())).collect::<Vec<_>>(), [("branch:ahead", first.as_str())]);
    let f = &cards[0].1;
    assert_eq!((f.pr, f.no_forge, f.forge.as_deref(), f.title.as_str(), f.body.as_str()), (None, true, None, "Start the ahead work", ""));
    assert_eq!((f.head_branch.as_deref(), f.base_branch.as_str(), f.ci), (Some("ahead"), "main", CiState::None));
    assert!(f.checks.is_empty() && f.links.pr.is_none());
    assert_eq!(f.files.iter().map(|x| x.path.as_str()).collect::<Vec<_>>(), ["ahead.txt"], "only what the branch adds over main");
    assert!(f.security.is_some(), "scanned like any review");
    assert_eq!(git(&folder, &["for-each-ref", "--format=%(refname) %(objectname)"]), refs_before, "the folder's refs are left alone");

    // The card and the repo say there's no forge; Post refuses
    let (s, v) = call(&env.app, "GET", &format!("/api/dev/cards/{}", cards[0].0.id), None).await;
    assert_eq!(s.as_u16(), 200, "{v}");
    assert_eq!((v["no_forge"].clone(), v["pr"].clone()), (json!(true), Value::Null), "{v}");
    let (s, v) = call(&env.app, "GET", &format!("/api/spaces/{}/dev_mode", space.as_str()), None).await;
    assert_eq!(s.as_u16(), 200, "{v}");
    assert_eq!((v["repos"][0]["forge"].clone(), v["repos"][0]["forge_connected"].clone()), (Value::Null, json!(false)), "{v}");

    // Nothing new: the same card
    poller::poll_space(&env.state, space.as_str()).await;
    assert_eq!(reviews(&env, &row.key).await.len(), 1);

    // A new push: a new card, the old one goes stale
    git(&folder, &["checkout", "-q", "ahead"]);
    let second = commit(&folder, "ahead.txt", "one\ntwo\n", "More ahead work");
    git(&folder, &["checkout", "-q", "main"]);
    poller::poll_space(&env.state, space.as_str()).await;
    let cards = reviews(&env, &row.key).await;
    let now = live(&cards);
    assert_eq!(now.iter().map(|c| c.0.head_sha.as_str()).collect::<Vec<_>>(), [second.as_str()]);
    assert_eq!(cards.len(), 2);

    // Merged into main: no open branch, the card goes stale
    git(&folder, &["merge", "-q", "--no-ff", "-m", "Merge ahead", "ahead"]);
    poller::poll_space(&env.state, space.as_str()).await;
    assert!(live(&reviews(&env, &row.key).await).is_empty());

    // A new branch, then deleted before it merged: its card goes stale too
    git(&folder, &["checkout", "-q", "-b", "gone"]);
    commit(&folder, "gone.txt", "x\n", "Short-lived");
    git(&folder, &["checkout", "-q", "main"]);
    poller::poll_space(&env.state, space.as_str()).await;
    assert_eq!(live(&reviews(&env, &row.key).await).len(), 1);
    git(&folder, &["branch", "-q", "-D", "gone"]);
    poller::poll_space(&env.state, space.as_str()).await;
    assert!(live(&reviews(&env, &row.key).await).is_empty());
}
