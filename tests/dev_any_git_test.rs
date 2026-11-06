//! Dev mode reads any git repo (plan dev-mode/plan/any-git.md G1-G3): a team folder that is a plain
//! git checkout with no remote and no forge gets a watched row and landed cards, read with git in
//! place, without touching the folder's working tree, index or refs.
//!
//! The folder is a throwaway `git init` in a temp dir with commits, a branch, a merge and a tag.
//! Nothing here touches the network or any remote.

mod common;

use common::*;
use kwnt::db::dev_mode::{GIT, LANDED};
use kwnt::db::models::NewTeam;
use kwnt::db::spaces::SpaceId;
use kwnt::server::dev::facts::DevFacts;
use kwnt::server::dev::{git, poller};
use serde_json::{json, Value};
use std::path::Path;

fn git(dir: &Path, args: &[&str]) -> String {
    let out = std::process::Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(["-c", "user.name=Lin (kwant bot)", "-c", "user.email=lin-bot@users.noreply.example.com", "-c", "commit.gpgsign=false", "-c", "tag.gpgsign=false"])
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

/// Everything about the folder a read could change: its files, index, HEAD and every ref.
fn snapshot(dir: &Path) -> (String, Vec<u8>, String, String) {
    let index = std::fs::read(dir.join(".git/index")).unwrap();
    let refs = git(dir, &["for-each-ref", "--format=%(refname) %(objectname)"]);
    (git(dir, &["status", "--porcelain", "--ignored"]), index, git(dir, &["rev-parse", "HEAD"]), refs)
}

async fn landed(env: &Env, key: &str) -> Vec<DevFacts> {
    let space = SpaceId::default_space();
    let mut rows = env.state.db.dev_cards_in(space.as_str(), Some(key), LANDED, None, 50).await.unwrap();
    rows.sort_by(|a, b| a.created_at.cmp(&b.created_at).then(a.id.cmp(&b.id)));
    rows.iter().map(|r| serde_json::from_str(&r.facts).unwrap()).collect()
}

#[tokio::test]
async fn a_plain_git_folder_gets_landed_cards_read_in_place() {
    let env = setup().await;
    let space = SpaceId::default_space();
    let folder = env.tmp.path().join("folders/plain");
    std::fs::create_dir_all(&folder).unwrap();
    git(&folder, &["init", "-q", "-b", "main"]);
    let root = commit(&folder, "README.md", "# Plain\n", "Start the repo");
    let second = commit(&folder, "app.rs", "fn main() {}\n", "Add the app\n\nIt does nothing yet.");
    git(&folder, &["tag", "v0.1.0"]);
    let third = commit(&folder, "app.rs", "fn main() {\n    println!(\"hi\");\n}\n", "Say hi");
    // Something the team is in the middle of: kwant must leave it as it is
    std::fs::write(folder.join("wip.txt"), "not committed\n").unwrap();
    std::fs::write(folder.join("README.md"), "# Plain\n\nEdited, not staged.\n").unwrap();
    let before = snapshot(&folder);

    let t = NewTeam {
        name: "Plain".into(),
        description: None,
        icon: None,
        lead_kwant_id: env.a.clone(),
        member_kwants: vec![env.a.clone()],
        workspace_path: Some(folder.to_string_lossy().to_string()),
        repo_url: None,
    };
    env.state.db.create_team(&space, &t).await.unwrap();
    let uri = format!("/api/spaces/{}/dev_mode", space.as_str());

    // Off: turning on would watch it
    let (s, v) = call(&env.app, "GET", &uri, None).await;
    assert_eq!(s.as_u16(), 200, "{v}");
    assert_eq!((v["status"].clone(), v["sources"].clone()), (json!("watching"), json!([{"team": "Plain", "repo": "plain", "host": null, "from": "local"}])), "{v}");

    // On: the team's folder gets a git row, keyed by the team, never by its path
    let (s, v) = call(&env.app, "PUT", &uri, Some(json!({"on": true}))).await;
    assert_eq!(s.as_u16(), 200, "{v}");
    assert_eq!(v["status"], "watching", "{v}");
    assert_eq!((v["repos"][0]["key"].clone(), v["repos"][0]["provider"].clone()), (json!("local:plain"), json!("git")), "{v}");
    assert_eq!((v["repos"][0]["status"].clone(), v["repos"][0]["from"].clone(), v["repos"][0]["branch"].clone()), (json!("watching"), json!("local"), json!("main")), "{v}");
    let path = std::fs::canonicalize(&folder).unwrap().to_string_lossy().to_string();
    assert!(!v.to_string().contains(&path), "the folder path reached a reply: {v}");
    let rows = env.state.db.dev_repos(space.as_str()).await.unwrap();
    assert_eq!(rows.len(), 1);
    let row = &rows[0];
    assert_eq!((row.provider.as_str(), row.repo.as_str(), row.host.as_deref(), row.branch.as_str()), (GIT, "plain", None, "main"));
    assert_eq!(row.git_url.as_deref(), Some(path.as_str()));

    // The first poll backfills what's on the branch, oldest first, from git alone
    poller::poll_space(&env.state, space.as_str()).await;
    let row = env.state.db.dev_repo(space.as_str(), "local:plain").await.unwrap().unwrap();
    assert_eq!(row.poll_error, None);
    assert_eq!(row.last_seen_sha.as_deref(), Some(third.as_str()));
    assert_eq!(poller::last_tag(space.as_str(), "local:plain").as_deref(), Some("v0.1.0"));
    let cards = landed(&env, "local:plain").await;
    assert_eq!(cards.iter().map(|c| c.head_sha.as_str()).collect::<Vec<_>>(), [root.as_str(), second.as_str(), third.as_str()]);
    let add = &cards[1];
    assert_eq!((add.title.as_str(), add.body.as_str(), add.pr, add.repo.as_str(), add.base_branch.as_str()), ("Add the app", "It does nothing yet.", None, "plain", "main"));
    assert_eq!(add.author.name, "Lin (kwant bot)");
    assert_eq!(add.author.email.as_deref(), Some("lin-bot@users.noreply.example.com"));
    assert!(add.merged_at.is_some(), "the commit's date");
    assert_eq!(add.files.len(), 1);
    assert_eq!((add.files[0].path.as_str(), add.files[0].status.as_str(), add.files[0].additions), ("app.rs", "added", 1));
    assert!(add.files[0].patch.as_deref().is_some_and(|p| p.contains("+fn main() {}")), "{:?}", add.files[0].patch);
    assert!(add.checks.is_empty() && add.links.pr.is_none() && add.links.commit.is_none(), "no forge, no host: no PR, CI or web link");
    let hi = &cards[2];
    assert_eq!((hi.files[0].status.as_str(), hi.files[0].additions, hi.files[0].deletions), ("modified", 3, 1));
    assert_eq!(cards[0].files[0].path, "README.md", "the root commit's files");

    // A feature branch merged with a merge commit: one card, with the branch's changes
    git(&folder, &["stash", "-q", "--include-untracked"]);
    git(&folder, &["checkout", "-q", "-b", "feature"]);
    commit(&folder, "lib.rs", "pub fn x() {}\n", "Add lib");
    commit(&folder, "lib.rs", "pub fn x() -> u8 { 1 }\n", "Return one");
    git(&folder, &["checkout", "-q", "main"]);
    git(&folder, &["merge", "-q", "--no-ff", "-m", "Merge feature", "feature"]);
    let merge = git(&folder, &["rev-parse", "HEAD"]);
    git(&folder, &["stash", "pop", "-q"]);
    let before = (before.0, snapshot(&folder).1, merge.clone(), snapshot(&folder).3);
    poller::poll_space(&env.state, space.as_str()).await;
    let cards = landed(&env, "local:plain").await;
    assert_eq!(cards.len(), 4, "only the merge is new on the branch's first-parent line");
    let m = &cards[3];
    assert_eq!((m.head_sha.as_str(), m.title.as_str()), (merge.as_str(), "Merge feature"));
    assert_eq!(m.files.iter().map(|f| (f.path.as_str(), f.status.as_str())).collect::<Vec<_>>(), [("lib.rs", "added")]);
    assert!(m.files[0].patch.as_deref().is_some_and(|p| p.contains("+pub fn x() -> u8 { 1 }")));

    // Nothing new: nothing announced twice
    poller::poll_space(&env.state, space.as_str()).await;
    assert_eq!(landed(&env, "local:plain").await.len(), 4);

    // The desk's rollup: four landed, and no "main is green" from a repo with no CI to read
    let (s, v) = call(&env.app, "GET", &format!("/api/spaces/{}/dev/landed", space.as_str()), None).await;
    assert_eq!(s.as_u16(), 200, "{v}");
    assert_eq!((v["rollup"]["count"].clone(), v["rollup"]["main_green"].clone()), (json!(4), Value::Null), "{v}");

    // The folder is as the team left it: files, index, HEAD, refs (no kwant refs)
    let after = snapshot(&folder);
    assert_eq!(after, before);
    assert!(!after.3.contains("refs/kwant"), "{}", after.3);
    // ...and kwant made no mirror for it
    assert!(!env.tmp.path().join("data/dev").exists() || std::fs::read_dir(env.tmp.path().join("data/dev")).unwrap().flatten().all(|d| std::fs::read_dir(d.path()).map(|mut r| r.next().is_none()).unwrap_or(true)));

    // A git row whose read failed shows its error, never not_connected: it needs no GitHub
    // connection (this space has none), so connecting one couldn't fix it
    assert!(env.state.db.set_dev_repo_error(space.as_str(), "local:plain", "git could not read the repo").await.unwrap());
    let (_, v) = call(&env.app, "GET", &uri, None).await;
    assert_eq!((v["status"].clone(), v["repos"][0]["status"].clone()), (json!("watching"), json!("error")), "{v}");
    // ...and the next good read clears it
    poller::poll_space(&env.state, space.as_str()).await;
    let (_, v) = call(&env.app, "GET", &uri, None).await;
    assert_eq!(v["repos"][0]["status"], "watching", "{v}");

    // The team stops pointing at a git folder: its row goes, its cards stay
    let teams = env.state.db.list_teams(&space).await.unwrap();
    env.state.db.delete_team(&teams.iter().find(|t| t.name == "Plain").unwrap().id).await.unwrap();
    // (the team route does this in the background)
    kwnt::server::dev::repo::reconcile(&env.state, &space).await;
    let (_, v) = call(&env.app, "GET", &uri, None).await;
    assert_eq!((v["status"].clone(), v["repos"].clone()), (json!("no_repo"), Value::Array(vec![])), "{v}");
    assert_eq!(landed(&env, "local:plain").await.len(), 4);
    assert_eq!(snapshot(&folder), before, "dropping the row leaves the folder alone too");
}

/// The forge seam (any-git.md G4): a host with a forge gets its PRs, logins and checks from it and
/// everything git can read from git. A fake forge over a temp repo stands in for GitHub here (the
/// mock GitHub can't serve git), so this is the path every GitHub row takes in 0.7.3.
#[tokio::test]
async fn with_a_forge_prs_and_checks_come_from_it_and_files_from_git() {
    use kwnt::db::dev_mode::{BY_HAND, REVIEW};
    use kwnt::server::dev::facts::{Check, FileDiff};
    use kwnt::server::dev::git::Repo;
    use kwnt::server::dev::source::{Change, Forge, History};
    use kwnt::zombie::github::{EtagCache, GhError};
    use std::sync::Mutex;

    struct Fake {
        merged: Change,
        open: Change,
        asked_files: Mutex<Vec<u64>>,
    }
    #[async_trait::async_trait]
    impl Forge for Fake {
        async fn changes_for_commit(&self, sha: &str) -> Result<Vec<Change>, GhError> {
            Ok(if self.merged.merge_sha.as_deref() == Some(sha) { vec![self.merged.clone()] } else { vec![] })
        }
        async fn open_changes(&self, _base: &str, _cache: &EtagCache) -> Result<Vec<Change>, GhError> {
            Ok(vec![self.open.clone()])
        }
        async fn checks(&self, _sha: &str, _cache: &EtagCache) -> Result<Vec<Check>, GhError> {
            Ok(vec![Check { name: "ci".into(), status: "completed".into(), conclusion: Some("success".into()), url: Some("https://github.com/demo-org/app/actions/runs/1".into()) }])
        }
        async fn change_files(&self, number: u64) -> Result<Vec<FileDiff>, GhError> {
            self.asked_files.lock().unwrap().push(number);
            Ok(vec![])
        }
        fn change_head_ref(&self, number: u64) -> Option<String> {
            Some(format!("refs/pull/{number}/head"))
        }
        async fn commit_login(&self, _sha: &str) -> Option<String> {
            Some("demo-person".into())
        }
    }

    let env = setup().await;
    let space = SpaceId::default_space();
    let dir = env.tmp.path().join("app");
    std::fs::create_dir_all(&dir).unwrap();
    git(&dir, &["init", "-q", "-b", "main"]);
    commit(&dir, "README.md", "# App\n", "Start");
    let pushed = commit(&dir, "app.rs", "fn main() {}\n", "Push straight to main");
    git(&dir, &["checkout", "-q", "-b", "feature"]);
    let head = commit(&dir, "lib.rs", "pub fn x() {}\n", "Add lib");
    git(&dir, &["checkout", "-q", "main"]);
    git(&dir, &["merge", "-q", "--no-ff", "-m", "Merge pull request #7 from demo-org/feature", "feature"]);
    let merge = git(&dir, &["rev-parse", "HEAD"]);
    git(&dir, &["checkout", "-q", "-b", "next"]);
    let open_head = commit(&dir, "next.rs", "pub fn next() {}\n", "Next thing");
    git(&dir, &["checkout", "-q", "main"]);

    let at = chrono::Utc::now();
    let change = |n: u64, title: &str, head_branch: &str, head_sha: &str| Change {
        number: n,
        title: title.into(),
        url: Some(format!("https://github.com/demo-org/app/pull/{n}")),
        author: Some("demo-person".into()),
        head_branch: head_branch.into(),
        head_sha: head_sha.into(),
        base_branch: "main".into(),
        ..Default::default()
    };
    let fake = Fake {
        merged: Change { merged_at: Some(at), merge_sha: Some(merge.clone()), ..change(7, "Add the lib", "feature", &head) },
        open: change(8, "Next thing", "next", &open_head),
        asked_files: Mutex::new(vec![]),
    };
    env.state.db.add_dev_repo(space.as_str(), "github.com/demo-org/app", None, "demo-org/app", Some("github.com"), "main", None, BY_HAND).await.unwrap();
    let row = env.state.db.dev_repo(space.as_str(), "github.com/demo-org/app").await.unwrap().unwrap();
    let h = History::new(Some(Repo::local(&dir, &env.state.config.data_dir).await.unwrap()), dir.to_string_lossy().to_string(), None, None, "main", Some("https://github.com/demo-org/app/commit".into()));
    poller::read(&env.state, &row, &h, Some(&fake)).await.unwrap();

    let cards = landed(&env, "github.com/demo-org/app").await;
    let by = |sha: &str| cards.iter().find(|c| c.head_sha == sha).unwrap_or_else(|| panic!("no card for {sha}: {cards:?}"));
    let m = by(&merge);
    assert_eq!((m.pr, m.title.as_str()), (Some(7), "Add the lib"), "the PR names the merge");
    assert_eq!(m.files.iter().map(|f| f.path.as_str()).collect::<Vec<_>>(), ["lib.rs"], "files from git");
    assert_eq!(m.checks.len(), 1, "checks from the forge");
    assert_eq!(m.links.pr.as_deref(), Some("https://github.com/demo-org/app/pull/7"));
    assert_eq!(m.links.commit, Some(format!("https://github.com/demo-org/app/commit/{merge}")));
    let p = by(&pushed);
    assert_eq!((p.pr, p.title.as_str(), p.author.login.as_deref()), (None, "Push straight to main", Some("demo-person")), "a direct push asks the forge who wrote it");
    assert_eq!(p.files.iter().map(|f| f.path.as_str()).collect::<Vec<_>>(), ["app.rs"]);

    // The open PR's review card: its files from git too (the head is in the repo), never the forge's list
    let reviews = env.state.db.dev_cards_in(space.as_str(), Some("github.com/demo-org/app"), REVIEW, None, 10).await.unwrap();
    assert_eq!(reviews.len(), 1);
    let r: DevFacts = serde_json::from_str(&reviews[0].facts).unwrap();
    assert_eq!((r.pr, r.head_sha.as_str()), (Some(8), open_head.as_str()));
    assert_eq!(r.files.iter().map(|f| f.path.as_str()).collect::<Vec<_>>(), ["next.rs"]);
    assert!(fake.asked_files.lock().unwrap().is_empty(), "git read every diff");
    // ...and the repo read in place has no kwant refs
    assert!(!git(&dir, &["for-each-ref"]).contains("refs/kwant"));
}

/// A team folder at `folders/<name>` on `main` whose `origin` is `remote`.
async fn remote_team(env: &Env, name: &str, remote: &str) -> std::path::PathBuf {
    let folder = env.tmp.path().join("folders").join(name.to_lowercase());
    std::fs::create_dir_all(&folder).unwrap();
    git(&folder, &["init", "-q", "-b", "main"]);
    git(&folder, &["remote", "add", "origin", remote]);
    let t = NewTeam {
        name: name.into(),
        description: None,
        icon: None,
        lead_kwant_id: env.a.clone(),
        member_kwants: vec![env.a.clone()],
        workspace_path: Some(folder.to_string_lossy().to_string()),
        repo_url: None,
    };
    env.state.db.create_team(&SpaceId::default_space(), &t).await.unwrap();
    folder
}

/// Every file under `dir` whose bytes contain `needle`.
fn files_with(dir: &Path, needle: &[u8], out: &mut Vec<String>) {
    for e in std::fs::read_dir(dir).into_iter().flatten().flatten() {
        let p = e.path();
        if p.is_dir() {
            files_with(&p, needle, out);
        } else if std::fs::read(&p).is_ok_and(|b| b.windows(needle.len()).any(|w| w == needle)) {
            out.push(p.display().to_string());
        }
    }
}

/// any-git.md G2: a remote's password never reaches the database, a reply or the data folder.
/// http(s) and git:// lose their userinfo whole; ssh keeps a user name (not a credential), never a
/// password. Nothing is fetched here (the poller isn't running).
#[tokio::test]
async fn a_remotes_password_is_never_stored() {
    let env = setup().await;
    let space = SpaceId::default_space();
    let secret = "s3cr3t-c4n4ry-pw";
    remote_team(&env, "Web", &format!("https://deploy:{secret}@gitlab.example.com/group/web.git")).await;
    remote_team(&env, "Ops", &format!("ssh://deploy:{secret}@gitea.example.org/group/ops.git")).await;
    let uri = format!("/api/spaces/{}/dev_mode", space.as_str());
    let (s, on) = call(&env.app, "PUT", &uri, Some(json!({"on": true}))).await;
    assert_eq!(s.as_u16(), 200, "{on}");
    let (_, v) = call(&env.app, "GET", &uri, None).await;

    let mut rows = env.state.db.dev_repos(space.as_str()).await.unwrap();
    rows.sort_by(|a, b| a.key.cmp(&b.key));
    let urls: Vec<_> = rows.iter().map(|r| (r.key.as_str(), r.git_url.as_deref())).collect();
    assert_eq!(urls, [("gitea.example.org/group/ops", Some("ssh://deploy@gitea.example.org/group/ops.git")), ("gitlab.example.com/group/web", Some("https://gitlab.example.com/group/web.git"))]);
    for r in [on, v] {
        assert!(!r.to_string().contains(secret), "{r}");
    }
    let mut leaked = Vec::new();
    files_with(&env.tmp.path().join("data"), secret.as_bytes(), &mut leaked);
    assert!(leaked.is_empty(), "the password was stored in {leaked:?}");
}

/// any-git.md G1: a mirror over the size cap (1 GB) was deleted after its fetch and left a note.
/// Until a day has passed the repo isn't fetched again, and the row reads `error` with the reason;
/// removing the repo (and adding it again) starts afresh. The cap itself is tested in git.rs with a
/// tiny cap; nothing here touches the network (the note stops the poll before any lookup).
#[tokio::test]
async fn a_repo_too_big_to_copy_reads_as_an_error_until_it_is_added_again() {
    let env = setup().await;
    let space = SpaceId::default_space();
    remote_team(&env, "Big", "https://gitlab.example.com/group/big.git").await;
    let uri = format!("/api/spaces/{}/dev_mode", space.as_str());
    let (s, v) = call(&env.app, "PUT", &uri, Some(json!({"on": true}))).await;
    assert_eq!((s.as_u16(), v["repos"][0]["key"].clone(), v["repos"][0]["branch"].clone()), (200, json!("gitlab.example.com/group/big"), json!("main")), "{v}");

    // The last fetch passed the cap: the mirror went and a note was left beside it
    let dir = git::mirror_dir(&env.state.config.data_dir, space.as_str(), "gitlab.example.com/group/big");
    std::fs::create_dir_all(dir.parent().unwrap()).unwrap();
    std::fs::write(git::too_big_mark(&dir), b"").unwrap();

    poller::poll_space(&env.state, space.as_str()).await;
    let (_, v) = call(&env.app, "GET", &uri, None).await;
    assert_eq!((v["repos"][0]["status"].clone(), v["repos"][0]["poll_error"].clone()), (json!("error"), json!("This repo is too big for Dev mode to read (over 1 GB).")), "{v}");
    assert_eq!(v["repos"][0]["poll_code"], json!("too_big"), "{v}");
    assert_eq!(v["status"], "watching", "{v}");
    assert!(landed(&env, "gitlab.example.com/group/big").await.is_empty());
    assert!(git::too_big_mark(&dir).is_file(), "the note stays for its day");

    // The team stops pointing at it: the row and the note go, so adding it again fetches afresh
    let teams = env.state.db.list_teams(&space).await.unwrap();
    env.state.db.delete_team(&teams.iter().find(|t| t.name == "Big").unwrap().id).await.unwrap();
    kwnt::server::dev::repo::reconcile(&env.state, &space).await;
    assert!(!git::too_big_mark(&dir).exists() && !dir.exists());
}

/// G2 review F2: the folder check runs on every poll, not only when the repo was added. A team
/// folder that read fine is pointed at another team's repo (its `.git` swapped for a symlink): the
/// next poll reads `error` and makes no card from the other repo's commits; put back, it reads
/// again.
#[tokio::test]
async fn a_team_folder_pointed_at_another_repo_reads_as_an_error_on_the_next_poll() {
    let env = setup().await;
    let space = SpaceId::default_space();
    let folder = env.tmp.path().join("folders/ours");
    std::fs::create_dir_all(&folder).unwrap();
    git(&folder, &["init", "-q", "-b", "main"]);
    let ours = commit(&folder, "README.md", "# Ours\n", "Our start");
    let victim = env.tmp.path().join("elsewhere/theirs");
    std::fs::create_dir_all(&victim).unwrap();
    git(&victim, &["init", "-q", "-b", "main"]);
    let theirs = commit(&victim, "s.txt", "SECRET-OTHER-TEAM\n", "other team secret");
    let t = NewTeam {
        name: "Ours".into(),
        description: None,
        icon: None,
        lead_kwant_id: env.a.clone(),
        member_kwants: vec![env.a.clone()],
        workspace_path: Some(folder.to_string_lossy().to_string()),
        repo_url: None,
    };
    env.state.db.create_team(&space, &t).await.unwrap();
    let uri = format!("/api/spaces/{}/dev_mode", space.as_str());
    let (s, v) = call(&env.app, "PUT", &uri, Some(json!({"on": true}))).await;
    assert_eq!(s.as_u16(), 200, "{v}");
    let key = v["repos"][0]["key"].as_str().unwrap().to_string();
    poller::poll_space(&env.state, space.as_str()).await;
    assert_eq!(landed(&env, &key).await.iter().map(|c| c.head_sha.clone()).collect::<Vec<_>>(), std::slice::from_ref(&ours));

    // Someone working in the folder points its .git at the other repo
    std::fs::rename(folder.join(".git"), env.tmp.path().join("ours.git")).unwrap();
    std::os::unix::fs::symlink(victim.join(".git"), folder.join(".git")).unwrap();
    poller::poll_space(&env.state, space.as_str()).await;
    let row = env.state.db.dev_repo(space.as_str(), &key).await.unwrap().unwrap();
    assert_eq!(row.poll_error.as_deref(), Some(git::GitError::Refused.to_string().as_str()));
    let cards = landed(&env, &key).await;
    assert!(cards.iter().all(|c| c.head_sha != theirs && !c.title.contains("other team")), "{cards:?}");
    let (_, v) = call(&env.app, "GET", &uri, None).await;
    assert_eq!(v["repos"][0]["status"], "error", "{v}");
    assert!(!v.to_string().contains("SECRET") && !v.to_string().contains("other team"), "{v}");

    // Put back, it reads again
    std::fs::remove_file(folder.join(".git")).unwrap();
    std::fs::rename(env.tmp.path().join("ours.git"), folder.join(".git")).unwrap();
    poller::poll_space(&env.state, space.as_str()).await;
    assert_eq!(env.state.db.dev_repo(space.as_str(), &key).await.unwrap().unwrap().poll_error, None);
}

/// The second too-big error (one answer from git over the output cap, `GitError::TooBig`) can
/// still happen, so its row gets `poll_code: "too_big"` too, as the 1 GB one does.
#[tokio::test]
async fn both_too_big_errors_read_as_poll_code_too_big() {
    let env = setup().await;
    let space = SpaceId::default_space();
    remote_team(&env, "Wide", "https://gitlab.example.com/group/wide.git").await;
    let uri = format!("/api/spaces/{}/dev_mode", space.as_str());
    let (s, v) = call(&env.app, "PUT", &uri, Some(json!({"on": true}))).await;
    assert_eq!(s.as_u16(), 200, "{v}");
    let key = "gitlab.example.com/group/wide";
    for (e, code) in [(git::GitError::TooBig, json!("too_big")), (git::GitError::RepoTooBig, json!("too_big")), (git::GitError::Unreachable, Value::Null)] {
        assert!(env.state.db.set_dev_repo_error(space.as_str(), key, &e.to_string()).await.unwrap());
        let (_, v) = call(&env.app, "GET", &uri, None).await;
        assert_eq!((v["repos"][0]["status"].clone(), v["repos"][0]["poll_code"].clone()), (json!("error"), code), "{e:?}: {v}");
    }
}
