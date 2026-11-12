//! Fix it and line notes on a PR a person wrote (fix-it-any-pr brief, Nov 12): the author bot gets
//! it, else the lead of the team that owns the card's repo, else nobody with a line that says what
//! to do.

mod common;

use common::*;
use kwnt::db::dev_mode::{BY_TEAM, REVIEW};
use kwnt::db::models::NewTeam;
use kwnt::db::spaces::{NewSpace, SpaceId};
use kwnt::server::dev::routes;
use serde_json::{json, Value};

const REPO: &str = "github.com/oleks/edge";

const PATCH: &str = "@@ -1,3 +1,4 @@\n package live\n+// funding accrual\n a\n b";

fn facts(head: &str, bot: Option<&str>, branch: &str) -> String {
    json!({
        "repo": "oleks/edge", "kind": "review", "pr": 31, "title": "Live edge finder", "body": "",
        "head_sha": head, "base_branch": "main", "merged_at": null, "merged_by": null,
        "author": {"login": "oleks", "name": "Oleks", "email": null,
                   "bot": bot.map(|id| json!({"id": id, "name": "x", "engine": "claude", "model": "x"}))},
        "files": [{"path": "internal/edgefinder/live/live.go", "status": "modified", "additions": 1, "deletions": 0, "patch": PATCH, "previous_path": null}],
        "cut": null, "checks": [], "ci": "green", "task": null, "first_edit_files": [],
        "head_branch": branch, "links": {"pr": "https://github.com/oleks/edge/pull/31", "commit": null, "ci": null}
    })
    .to_string()
}

fn card() -> String {
    json!({"findings": [{
        "id": "f1", "severity": "high", "title": "Funding accrual is not in the PR summary's stated scope",
        "file": "internal/edgefinder/live/live.go", "line": 1, "detail": "d", "fix": "x", "source": "review", "state": "open"
    }]})
    .to_string()
}

/// A review card in `space` for [`REPO`], with one open finding.
async fn review(env: &Env, space: &str, head: &str, bot: Option<&str>, branch: &str) -> String {
    let row = env.state.db.insert_dev_card_in(space, REPO, REVIEW, "pr:31", head, &facts(head, bot, branch)).await.unwrap().unwrap();
    assert!(env.state.db.swap_dev_card_json(&row.id, None, &card()).await.unwrap());
    row.id
}

/// A team in the default space, led by `lead`, that owns [`REPO`] in `space`.
async fn team_owns_repo(env: &Env, space: &str, lead: &str) -> kwnt::db::models::Team {
    let input = NewTeam { name: "Edge".into(), description: None, icon: None, lead_kwant_id: lead.into(), member_kwants: vec![], workspace_path: None, repo_url: None };
    let team = env.state.db.create_team(&SpaceId::default_space(), &input).await.unwrap();
    env.state.db.add_dev_repo(space, REPO, Some(&team.id), "oleks/edge", Some("github.com"), "main", None, BY_TEAM).await.unwrap();
    team
}

/// Presses Fix it, then delivers at once (the route's own delivery waits out the Undo window).
async fn fix_it(env: &Env, card: &str) -> Value {
    let (s, v) = call(&env.app, "POST", &format!("/api/dev/cards/{card}/findings/f1"), Some(json!({"action": "fix"}))).await;
    assert_eq!(s.as_u16(), 200, "{v}");
    routes::deliver_fix(&env.state, card, "f1").await;
    finding(env, card).await
}

async fn finding(env: &Env, card: &str) -> Value {
    let row = env.state.db.dev_card(card).await.unwrap().unwrap();
    let c: Value = serde_json::from_str(row.card.as_deref().unwrap()).unwrap();
    c["findings"][0].clone()
}

async fn task_of(env: &Env, f: &Value) -> kwnt::db::missions::Task {
    env.state.db.get_task(f["task_id"].as_str().unwrap_or_else(|| panic!("no task: {f}"))).await.unwrap().unwrap()
}

async fn name_of(env: &Env, id: &str) -> String {
    env.state.db.get_kwant(id).await.unwrap().unwrap().name
}

#[tokio::test]
async fn a_person_s_pr_on_a_team_repo_goes_to_the_team_s_lead() {
    let env = setup().await;
    let space = SpaceId::default_space();
    team_owns_repo(&env, space.as_str(), &env.b).await;
    let card = review(&env, space.as_str(), "aaa1111", None, "oleks/live").await;

    // From the press on, during the Undo window, the card names the lead, never the PR's person
    let (s, v) = call(&env.app, "POST", &format!("/api/dev/cards/{card}/findings/f1"), Some(json!({"action": "fix"}))).await;
    assert_eq!(s.as_u16(), 200, "{v}");
    let waiting = finding(&env, &card).await;
    assert_eq!((waiting["state"].clone(), waiting["sent_to"].clone(), waiting["sent_via"].clone(), waiting["sent_team"].clone()), (json!("sent"), json!(name_of(&env, &env.b).await), json!("team_lead"), json!("Edge")), "{waiting}");
    assert!(waiting.get("task_id").is_none(), "{waiting}");
    routes::deliver_fix(&env.state, &card, "f1").await;
    let f = finding(&env, &card).await;
    assert_eq!(f["state"], "sent", "{f}");
    assert_eq!((f["sent_to"].clone(), f["sent_via"].clone(), f["sent_team"].clone()), (json!(name_of(&env, &env.b).await), json!("team_lead"), json!("Edge")));
    assert!(f.get("send_error").is_none(), "{f}");
    let t = task_of(&env, &f).await;
    assert_eq!(t.direct_kwant_id.as_deref(), Some(env.b.as_str()));
    let a = &t.acceptance;
    assert!(a.contains("A person wrote this PR, not you. You're getting it as the lead of Edge, the team that owns oleks/edge."), "{a}");
    assert!(a.contains("Never force-push or rewrite their commits."), "{a}");
    assert!(a.contains("If you can't push there (the PR comes from a fork, or the push is refused)") && a.contains("open a draft PR that links PR #31"), "{a}");
    assert!(a.contains(routes::MODEL_PROSE_NOTE), "{a}");

    // Undo after it went out cancels the task and forgets who got it
    let (s, v) = call(&env.app, "POST", &format!("/api/dev/cards/{card}/findings/f1"), Some(json!({"action": "undo_fix"}))).await;
    assert_eq!(s.as_u16(), 200, "{v}");
    let back = finding(&env, &card).await;
    assert_eq!(back["state"], "open");
    for k in ["task_id", "sent_to", "sent_via", "sent_team"] {
        assert!(back.get(k).is_none(), "{k}: {back}");
    }
    assert_eq!(env.state.db.get_task(&t.id).await.unwrap().unwrap().state, "cancelled");
}

#[tokio::test]
async fn an_author_bot_s_pr_is_unchanged_and_a_gone_author_falls_back_to_the_lead() {
    let env = setup().await;
    let space = SpaceId::default_space();
    team_owns_repo(&env, space.as_str(), &env.b).await;

    // The author bot is in the space: it gets it, with no lead lines
    let card = review(&env, space.as_str(), "bbb1111", Some(&env.a), "pixel/x").await;
    let f = fix_it(&env, &card).await;
    assert_eq!((f["sent_to"].clone(), f["sent_via"].clone()), (json!(name_of(&env, &env.a).await), json!("author")));
    assert!(f.get("sent_team").is_none(), "{f}");
    let t = task_of(&env, &f).await;
    assert_eq!(t.direct_kwant_id.as_deref(), Some(env.a.as_str()));
    for line in ["not you", "force-push", "fork"] {
        assert!(!t.acceptance.contains(line), "{line}: {}", t.acceptance);
    }

    // The author bot is gone (and the branch names no bot): the team's lead gets it
    let card = review(&env, space.as_str(), "ccc1111", Some("kwant-gone"), "someone/x").await;
    let f = fix_it(&env, &card).await;
    assert_eq!((f["sent_via"].clone(), task_of(&env, &f).await.direct_kwant_id), (json!("team_lead"), Some(env.b.clone())));
}

#[tokio::test]
async fn nobody_owns_the_repo_or_the_lead_is_elsewhere_leaves_it_open_with_what_to_do() {
    let env = setup().await;
    let space = SpaceId::default_space();
    // No team owns the repo: the press itself says what to do (no Undo window, nothing to undo)
    let card = review(&env, space.as_str(), "ddd1111", None, "oleks/live").await;
    let (s, v) = call(&env.app, "POST", &format!("/api/dev/cards/{card}/findings/f1"), Some(json!({"action": "fix"}))).await;
    assert_eq!(s.as_u16(), 200, "{v}");
    let f = finding(&env, &card).await;
    assert_eq!((f["state"].clone(), f["sent_at"].clone()), (json!("open"), Value::Null), "{f}");
    assert!(f.get("sent_to").is_none(), "{f}");
    routes::deliver_fix(&env.state, &card, "f1").await; // the window's end changes nothing
    let f = finding(&env, &card).await;
    assert_eq!(f["state"], "open", "{f}");
    assert!(f.get("task_id").is_none());
    assert_eq!(
        f["send_error"],
        "No bot wrote this PR and no team owns oleks/edge. Add the repo to a team (Manage team › Git Repo), then press Fix it again."
    );

    // The repo's team is in another space: its lead is no bot of the card's space
    let lab = env.state.db.create_space(&NewSpace { name: "Lab".into(), icon: "L".into(), color: None, this_machine_enabled: true }).await.unwrap();
    team_owns_repo(&env, &lab.id, &env.b).await;
    let card = review(&env, &lab.id, "eee1111", None, "oleks/live").await;
    routes::deliver_fix(&env.state, &card, "f1").await; // nothing to do yet: not sent
    let row = env.state.db.dev_card(&card).await.unwrap().unwrap();
    let mut c: Value = serde_json::from_str(row.card.as_deref().unwrap()).unwrap();
    c["findings"][0]["state"] = json!("sent");
    assert!(env.state.db.swap_dev_card_json(&card, row.card.as_deref(), &c.to_string()).await.unwrap());
    // As after a restart: the poller redelivers what's still waiting, by the same rule
    assert_eq!(routes::undelivered(&c), vec!["f1".to_string()]);
    routes::deliver_fix(&env.state, &card, "f1").await;
    let f = finding(&env, &card).await;
    assert_eq!(f["state"], "open", "{f}");
    assert!(f["send_error"].as_str().unwrap().starts_with("No bot wrote this PR and no team owns oleks/edge."), "{f}");
}

#[tokio::test]
async fn line_notes_on_a_person_s_pr_reach_the_team_s_lead() {
    let env = setup().await;
    let space = SpaceId::default_space();
    let note = |head: &str| {
        json!({"path": "internal/edgefinder/live/live.go", "start": {"side": "new", "line": 2}, "end": {"side": "new", "line": 2}, "head_sha": head, "text": "Move this to its own PR."})
    };

    // No team yet: no one to send to, and the line says what to do
    let card = review(&env, space.as_str(), "fff1111", None, "oleks/live").await;
    let (_, ch) = call(&env.app, "GET", &format!("/api/dev/cards/{card}/changes"), None).await;
    assert_eq!(ch["author_bot"], Value::Null, "{ch}");
    let notes = format!("/api/dev/cards/{card}/notes");
    let (s, v) = call(&env.app, "POST", &notes, Some(note("fff1111"))).await;
    assert_eq!(s.as_u16(), 201, "{v}");
    let (s, v) = call(&env.app, "POST", &format!("{notes}/send"), Some(json!({"send_id": "n-1"}))).await;
    assert_eq!((s.as_u16(), v["code"].clone()), (409, json!("no_author")), "{v}");
    assert_eq!(v["error"], "No bot wrote this PR and no team owns oleks/edge. Add the repo to a team (Manage team › Git Repo), then send again.");

    // The repo joins a team: the lead is who the notes are for, and they reach the lead's chat
    team_owns_repo(&env, space.as_str(), &env.b).await;
    let lead = name_of(&env, &env.b).await;
    let (_, ch) = call(&env.app, "GET", &format!("/api/dev/cards/{card}/changes"), None).await;
    assert_eq!((ch["author_bot"]["id"].clone(), ch["notes_via"].clone(), ch["notes_team"].clone()), (json!(env.b), json!("team_lead"), json!("Edge")), "{ch}");
    let (s, v) = call(&env.app, "POST", &format!("{notes}/send"), Some(json!({"send_id": "n-2"}))).await;
    assert_eq!(s.as_u16(), 200, "{v}");
    assert_eq!((v["kwant_id"].clone(), v["kwant_name"].clone(), v["sent_via"].clone(), v["sent_team"].clone()), (json!(env.b), json!(lead), json!("team_lead"), json!("Edge")));
    let (_, msgs) = call(&env.app, "GET", &format!("/api/channels/{}/messages", env.b), None).await;
    let m = msgs.as_array().unwrap().iter().find(|m| m["text"].as_str().is_some_and(|t| t.contains("from the review"))).unwrap_or_else(|| panic!("{msgs}"));
    let text = m["text"].as_str().unwrap();
    assert!(text.contains("A person wrote this PR, not you. You're getting these notes as the lead of Edge"), "{text}");
    assert!(text.contains("never force-push") && text.contains("open a draft PR that links PR #31"), "{text}");
}
