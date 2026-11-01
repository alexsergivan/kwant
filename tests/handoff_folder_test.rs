//! A hand-off card in a team works in the team's folder, where its owner wrote the files: its file
//! checks and its audit turns look there, not in the owner's or the challenger's own folder
//! (promo footage S7, 2026-10-30: Frontend wrote index.html in the team folder, QA's audit looked
//! in its own folder and found nothing). A fake claude CLI on PATH records the folder each audit
//! turn runs in. Its own test binary: it changes PATH.

#[path = "common/exe.rs"]
mod exe;

use kwnt::{
    broker::PermissionBroker,
    config::Config,
    db::{
        missions::{NewCheck, NewTask, Task},
        models::{Kwant, NewKwant, NewTeam},
        spaces::SpaceId,
        threads::NewDelegation,
        Database,
    },
    router::LayaRouter,
    server::{missions, missions::auditor, AppState},
    tools::ToolRunner,
};
use serde_json::json;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::time::Duration;

/// Writes the folder it runs in to `$KWNT_TEST_CWD/<pid>` for an audit prompt, and answers.
const FAKE_CLAUDE: &str = r#"#!/bin/sh
case "$1" in
  auth) echo '{"loggedIn":true}'; exit 0 ;;
esac
for a; do last="$a"; done
case "$last" in *"[Swarm Auditor]"*) pwd -P > "$KWNT_TEST_CWD/$$" ;; esac
echo '{"type":"result","is_error":false,"num_turns":1,"result":"done"}'
"#;

fn cwd_dir() -> &'static PathBuf {
    static DIR: OnceLock<PathBuf> = OnceLock::new();
    DIR.get_or_init(|| {
        let root = tempfile::tempdir().unwrap().keep();
        let (bin, cwd) = (root.join("bin"), root.join("cwd"));
        std::fs::create_dir_all(&bin).unwrap();
        std::fs::create_dir_all(&cwd).unwrap();
        let fake = bin.join("claude");
        exe::write_executable(&fake, FAKE_CLAUDE);
        std::env::set_var("KWNT_TEST_CWD", &cwd);
        std::env::set_var("PATH", format!("{}:{}", bin.display(), std::env::var("PATH").unwrap_or_default()));
        cwd
    })
}

struct Env {
    state: AppState,
    team_dir: PathBuf,
    owner: Kwant,
    _tmp: tempfile::TempDir,
}

async fn bot(state: &AppState, name: &str) -> Kwant {
    let input = NewKwant {
        name: name.into(),
        title: "Tester".into(),
        persona_prompt: String::new(),
        soul: String::new(),
        avatar_sprite: "s".into(),
        driver_type: "cli".into(),
        preferred_model: "claude".into(),
    };
    state.db.create_kwant(&SpaceId::default_space(), &input).await.unwrap()
}

/// A team with its own folder (like `/tmp/coffee-site` in the footage): Lead, Frontend and a
/// challenger named to sort first.
async fn setup() -> Env {
    cwd_dir();
    let tmp = tempfile::tempdir().unwrap();
    let data = tmp.path().join("data");
    let db = Database::init(&data).await.unwrap();
    let config = Config {
        data_dir: data.clone(),
        laya_url: "http://127.0.0.1:9".into(),
        browser_cdp_url: Some("http://127.0.0.1:9".into()),
        ..Config::default()
    };
    let state = AppState::new(
        config.clone(),
        db,
        ToolRunner::new(data.join("workspace")),
        PermissionBroker::new(),
        LayaRouter::new(config.laya_url.clone()),
    );
    let (lead, owner, qa) = (bot(&state, "Lead").await, bot(&state, "Frontend").await, bot(&state, "Aaa QA").await);
    let team_dir = tmp.path().join("coffee-site");
    std::fs::create_dir_all(&team_dir).unwrap();
    state
        .db
        .create_team(
            &SpaceId::default_space(),
            &NewTeam {
                name: "web".into(),
                description: None,
                icon: None,
                lead_kwant_id: lead.id.clone(),
                member_kwants: vec![lead.id.clone(), owner.id.clone(), qa.id.clone()],
                workspace_path: Some(team_dir.to_string_lossy().into()),
                repo_url: None,
            },
        )
        .await
        .unwrap();
    Env { state, team_dir, owner, _tmp: tmp }
}

/// The card a team lead's hand-off to Frontend makes (no mission), in Review, with the file
/// Frontend wrote in the team's folder.
async fn handoff_card(e: &Env) -> Task {
    let space = SpaceId::default_space();
    let team = e.state.db.list_teams(&space).await.unwrap().into_iter().find(|t| t.name == "web").unwrap();
    let (thread, d) = e
        .state
        .db
        .create_delegation(&NewDelegation {
            lead_kwant_id: team.lead_kwant_id.clone(),
            specialist_kwant_id: e.owner.id.clone(),
            origin_channel_id: None,
            origin_kwant_id: Some(team.lead_kwant_id.clone()),
            origin_thread_id: None,
            team_id: Some(team.id.clone()),
            task: "Build the landing page".into(),
            title: "Build the landing page".into(),
            continue_lead: true,
            depth: 1,
            follow_up_at: None,
        })
        .await
        .unwrap();
    std::fs::write(e.team_dir.join("index.html"), "<h1>Coffee</h1>").unwrap();
    e.state
        .db
        .create_task(
            &space,
            &NewTask {
                title: "Build the landing page".into(),
                acceptance: "index.html opens".into(),
                state: "review".into(),
                priority: 2,
                owner_kwant_id: Some(e.owner.id.clone()),
                max_attempts: 3,
                source_kind: "delegation".into(),
                source_id: Some(d.id.clone()),
                thread_id: Some(thread.id.clone()),
                created_by: "kwant".into(),
                ..Default::default()
            },
        )
        .await
        .unwrap()
}

fn same(a: &Path, b: &Path) -> bool {
    std::fs::canonicalize(a).ok() == std::fs::canonicalize(b).ok()
}

#[tokio::test]
async fn a_team_hand_off_card_checks_files_in_the_team_folder() {
    let e = setup().await;
    let t = handoff_card(&e).await;
    let folder = missions::task_folder(&e.state, &t).await.expect("the card has a folder");
    assert!(same(&folder, &e.team_dir), "the card's folder is {}, not the team's {}", folder.display(), e.team_dir.display());

    // A person's file check on the card finds the file Frontend wrote
    e.state
        .db
        .add_check(&NewCheck {
            task_id: t.id.clone(),
            kind: "file_exists".into(),
            spec: json!({"path": "index.html"}),
            required: true,
            approved: true,
            requested_by: "person".into(),
            approved_by: Some("person".into()),
        })
        .await
        .unwrap();
    let check = e.state.db.list_checks(&t.id).await.unwrap().remove(0);
    let ran = missions::checks::run_check(&e.state, &t, &check).await;
    assert_eq!(ran.status, "passed", "index.html in the team folder: {ran:?}");
}

#[tokio::test]
async fn the_auditor_challenges_a_team_hand_off_in_the_team_folder() {
    let e = setup().await;
    let t = handoff_card(&e).await;
    let audit = auditor::start(&e.state, &t, "manual").await.expect("the audit opens");
    let seen_before: Vec<_> = std::fs::read_dir(cwd_dir()).unwrap().map(|f| f.unwrap().path()).collect();
    tokio::time::timeout(Duration::from_secs(60), auditor::drive(&e.state, &audit)).await.expect("the audit ends");
    let ran: Vec<String> = std::fs::read_dir(cwd_dir())
        .unwrap()
        .map(|f| f.unwrap().path())
        .filter(|p| !seen_before.contains(p))
        .map(|p| std::fs::read_to_string(p).unwrap().trim().to_string())
        .collect();
    assert!(!ran.is_empty(), "the challenge turn ran");
    for dir in &ran {
        assert!(same(Path::new(dir), &e.team_dir), "an audit turn ran in {dir}, not the team folder {}", e.team_dir.display());
    }
}
