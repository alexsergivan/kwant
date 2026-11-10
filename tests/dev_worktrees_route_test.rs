//! H1 worktree routes over HTTP (QA of #486, D2 and N1): a create race for one new branch answers
//! one 201 and refusals only (never 500 `git_failed`), and a removed worktree is 404 to
//! DELETE, done and keep alike.

mod common;

use common::*;
use kwnt::db::models::NewTeam;
use kwnt::db::spaces::SpaceId;
use serde_json::{json, Value};
use std::path::Path;

fn git(dir: &Path, args: &[&str]) -> String {
    let out = std::process::Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(["-c", "user.name=Demo Person", "-c", "user.email=demo@example.com", "-c", "commit.gpgsign=false"])
        .args(args)
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .output()
        .unwrap();
    assert!(out.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&out.stderr));
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

/// Dev mode on, and a team whose folder is a git repo with one commit on main. Its team id.
async fn team_with_repo(env: &Env) -> String {
    let space = SpaceId::default_space();
    let folder = env.tmp.path().join("folders/web");
    std::fs::create_dir_all(&folder).unwrap();
    git(&folder, &["init", "-q", "-b", "main"]);
    std::fs::write(folder.join("README.md"), "# Web\n").unwrap();
    git(&folder, &["add", "README.md"]);
    git(&folder, &["commit", "-q", "-m", "Start"]);
    let t = NewTeam {
        name: "Web".into(),
        description: None,
        icon: None,
        lead_kwant_id: env.a.clone(),
        member_kwants: vec![env.a.clone()],
        workspace_path: Some(folder.to_string_lossy().to_string()),
        repo_url: None,
    };
    let team = env.state.db.create_team(&space, &t).await.unwrap();
    env.state.db.put_dev_mode(space.as_str(), true, "demo-org/web", "main", None, None).await.unwrap();
    team.id
}

/// Rigel's repro (qa-486-493 D2, tools/race.mjs): 6 creates at once for one new branch, 8 rounds.
#[tokio::test]
async fn a_create_race_gets_one_201_and_refusals_only() {
    let env = setup().await;
    let team = team_with_repo(&env).await;
    let uri = format!("/api/spaces/{}/dev/worktrees", SpaceId::default_space().as_str());
    for round in 0..8 {
        let body = json!({"team_id": team, "branch": format!("lin/race-{round}"), "base": "main"});
        let calls = (0..6).map(|_| call(&env.app, "POST", &uri, Some(body.clone())));
        let got: Vec<(u16, Value)> = futures::future::join_all(calls).await.into_iter().map(|(s, v)| (s.as_u16(), v)).collect();
        let made = got.iter().filter(|(s, _)| *s == 201).count();
        assert_eq!(made, 1, "round {round}: {got:?}");
        for (s, v) in got.iter().filter(|(s, _)| *s != 201) {
            assert_eq!(*s, 409, "round {round}: {v}");
            assert!(matches!(v["code"].as_str(), Some("branch_exists" | "folder_taken")), "round {round}: {v}");
        }
    }
    let (s, v) = call(&env.app, "GET", &uri, None).await;
    assert_eq!(s.as_u16(), 200, "{v}");
    let rows = v["repos"][0]["worktrees"].as_array().cloned().unwrap_or_default();
    assert_eq!(rows.len(), 8, "one worktree per round: {v}");
    assert!(v["repos"][0]["other"].as_array().is_none_or(|o| o.is_empty()), "nothing left behind: {v}");
    let branches = git(&env.tmp.path().join("folders/web"), &["for-each-ref", "--format=%(refname:short)", "refs/heads/lin/"]);
    assert_eq!(branches.lines().count(), 8, "{branches}");
}

/// N1: once removed, done and keep answer 404 like DELETE, and nothing is stamped.
#[tokio::test]
async fn a_removed_worktree_is_404_to_done_keep_and_delete() {
    let env = setup().await;
    let team = team_with_repo(&env).await;
    let uri = format!("/api/spaces/{}/dev/worktrees", SpaceId::default_space().as_str());
    let (s, w) = call(&env.app, "POST", &uri, Some(json!({"team_id": team, "branch": "lin/n1", "base": "main"}))).await;
    assert_eq!(s.as_u16(), 201, "{w}");
    let id = w["id"].as_str().unwrap().to_string();
    let one = format!("/api/dev/worktrees/{id}");

    let (s, v) = call(&env.app, "POST", &format!("{one}/done"), Some(json!({}))).await;
    assert_eq!(s.as_u16(), 200, "a live one is marked done: {v}");
    let first = env.state.db.worktree(&id).await.unwrap().unwrap().done_at;
    assert!(first.is_some());

    let (s, v) = call(&env.app, "DELETE", &one, None).await;
    assert_eq!(s.as_u16(), 200, "{v}");
    for (method, uri, body) in [("POST", format!("{one}/done"), Some(json!({}))), ("POST", format!("{one}/keep"), Some(json!({"keep": true}))), ("DELETE", one.clone(), None)] {
        let (s, v) = call(&env.app, method, &uri, body).await;
        assert_eq!(s.as_u16(), 404, "{method} {uri}: {v}");
        assert_eq!(v["code"].as_str(), Some("unknown_worktree"), "{v}");
    }
    let row = env.state.db.worktree(&id).await.unwrap().unwrap();
    assert_eq!(row.done_at, first, "done on a removed row stamps nothing");
    assert!(!row.keep_build, "keep on a removed row changes nothing");
}
