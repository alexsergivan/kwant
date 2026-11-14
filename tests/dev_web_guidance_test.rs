//! Modern web guidance (web-guidance brief W3): a Dev mode space whose team folder has web code
//! is offered the built-in `modern-web` skill once, as one pending skill every bot in the space
//! sees in Skills. Never again after a reject, a delete, or Dev mode going off and on; never for
//! a Rust-only folder; never while Dev mode is off. kwant runs no `npx` here.

mod common;

use common::*;
use kwnt::db::models::NewTeam;
use kwnt::db::skills::clean_skill_text;
use kwnt::db::spaces::SpaceId;
use kwnt::server::dev::web_guidance::{offer_if_web, SKILL_NAME, VERSION};
use serde_json::json;
use std::path::{Path, PathBuf};

fn space() -> SpaceId {
    SpaceId::default_space()
}

async fn team(env: &Env, name: &str, folder: &Path) {
    let t = NewTeam {
        name: name.into(),
        description: None,
        icon: None,
        lead_kwant_id: env.a.clone(),
        member_kwants: vec![env.a.clone()],
        workspace_path: Some(folder.to_string_lossy().to_string()),
        repo_url: None,
    };
    env.state.db.create_team(&space(), &t).await.unwrap();
}

fn folder(env: &Env, name: &str, files: &[&str]) -> PathBuf {
    let dir = env.tmp.path().join("wg-folders").join(name);
    for f in files {
        let p = dir.join(f);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, "x").unwrap();
    }
    dir
}

async fn dev_mode(env: &Env, on: bool) {
    let (s, v) = call(&env.app, "PUT", &format!("/api/spaces/{}/dev_mode", space().as_str()), Some(json!({"on": on}))).await;
    assert_eq!(s.as_u16(), 200, "{v}");
}

/// Every skill in the space called modern-web, any status.
async fn offered(env: &Env) -> Vec<kwnt::db::skills::Skill> {
    let all = env.state.db.list_skills(&space(), None, None).await.unwrap();
    all.into_iter().filter(|s| s.name == SKILL_NAME).collect()
}

async fn marked(env: &Env) -> bool {
    env.state.db.dev_mode(space().as_str()).await.unwrap().is_some_and(|r| r.web_guidance_offered_at.is_some())
}

/// The offer turning on starts runs in its own task; wait for it (at most 5 s).
async fn wait_for_offer(env: &Env) -> Vec<kwnt::db::skills::Skill> {
    for _ in 0..50 {
        let got = offered(env).await;
        if !got.is_empty() {
            return got;
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    offered(env).await
}

#[tokio::test]
async fn a_web_folder_gets_one_space_wide_offer_and_never_another() {
    let env = setup().await;
    let web = folder(&env, "site", &["package.json", "src/App.tsx"]);
    team(&env, "Site", &web).await;
    assert!(offered(&env).await.is_empty(), "nothing before Dev mode is on");

    dev_mode(&env, true).await;
    let got = wait_for_offer(&env).await;
    assert_eq!(got.len(), 1, "one offer");
    let skill = &got[0];
    assert_eq!((skill.status.as_str(), skill.source.as_str()), ("pending", "agent"));
    assert_eq!(skill.kwant_id, None, "shared with every bot in the space: one card, not one per bot");
    assert!(skill.body.contains(&format!("modern-web-guidance@{VERSION} search")));
    assert!(marked(&env).await);

    // Both bots see the same pending skill in their Skills
    for k in [&env.a, &env.b] {
        let (s, v) = call(&env.app, "GET", &format!("/api/skills?kwant_id={k}&status=pending"), None).await;
        assert_eq!(s.as_u16(), 200, "{v}");
        let ids: Vec<&str> = v.as_array().unwrap().iter().filter_map(|x| x["id"].as_str()).collect();
        assert_eq!(ids, vec![skill.id.as_str()], "{v}");
    }

    // A second pass (the poller's) offers nothing more
    offer_if_web(&env.state, &space()).await;
    assert_eq!(offered(&env).await.len(), 1);

    // Rejected: kept for the record, never offered again
    let (s, v) = call(&env.app, "POST", &format!("/api/skills/{}/reject", skill.id), Some(json!({}))).await;
    assert_eq!(s.as_u16(), 200, "{v}");
    offer_if_web(&env.state, &space()).await;
    let got = offered(&env).await;
    assert_eq!(got.iter().map(|s| s.status.as_str()).collect::<Vec<_>>(), vec!["rejected"]);

    // Deleted: still never offered again
    let (s, _) = call(&env.app, "DELETE", &format!("/api/skills/{}", skill.id), None).await;
    assert!(s.is_success(), "{s}");
    offer_if_web(&env.state, &space()).await;
    assert!(offered(&env).await.is_empty());

    // Dev mode off and on again: no new offer
    dev_mode(&env, false).await;
    dev_mode(&env, true).await;
    tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    offer_if_web(&env.state, &space()).await;
    assert!(offered(&env).await.is_empty(), "turning Dev mode off and on doesn't offer it again");
    assert!(marked(&env).await);
}

#[tokio::test]
async fn a_rust_only_folder_gets_no_offer_until_web_code_appears() {
    let env = setup().await;
    let rust = folder(&env, "engine", &["Cargo.toml", "src/main.rs", "tests/a_test.rs", "README.md"]);
    team(&env, "Engine", &rust).await;
    dev_mode(&env, true).await;
    tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    offer_if_web(&env.state, &space()).await;
    assert!(offered(&env).await.is_empty());
    assert!(!marked(&env).await, "no offer made, so none is marked");

    // The team adds a web front end: the next pass offers it
    std::fs::create_dir_all(rust.join("web")).unwrap();
    std::fs::write(rust.join("web/index.html"), "<!doctype html>").unwrap();
    offer_if_web(&env.state, &space()).await;
    assert_eq!(offered(&env).await.len(), 1);
}

#[tokio::test]
async fn nothing_is_offered_while_dev_mode_is_off() {
    let env = setup().await;
    let web = folder(&env, "site", &["index.html"]);
    team(&env, "Site", &web).await;
    offer_if_web(&env.state, &space()).await;
    assert!(offered(&env).await.is_empty());

    dev_mode(&env, true).await;
    dev_mode(&env, false).await;
    tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    // Whatever the switch-on task did, a pass while off adds nothing
    let before = offered(&env).await.len();
    offer_if_web(&env.state, &space()).await;
    assert_eq!(offered(&env).await.len(), before);
}

#[tokio::test]
async fn the_persons_own_modern_web_skill_is_left_alone() {
    let env = setup().await;
    let web = folder(&env, "site", &["package.json"]);
    team(&env, "Site", &web).await;
    let mine = clean_skill_text(SKILL_NAME, "My own web notes", "Use our design system.").unwrap();
    let own = env.state.db.create_user_skill(&space(), None, mine).await.unwrap();

    dev_mode(&env, true).await;
    tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    offer_if_web(&env.state, &space()).await;
    let got = offered(&env).await;
    assert_eq!(got.len(), 1);
    assert_eq!((got[0].id.as_str(), got[0].body.as_str()), (own.id.as_str(), "Use our design system."));
    assert!(marked(&env).await, "marked, so it isn't checked again");
}
