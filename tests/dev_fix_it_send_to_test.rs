//! "Send to…" (fix-it-send-to brief, Nov 13): who gets a Fix it or line notes before pressing
//! (`GET /api/dev/cards/:id/recipients`), and a chosen bot of the space instead of the rule's pick
//! (`to` on Fix it and on the notes' send).

mod common;

use common::*;
use kwnt::db::dev_mode::{BY_TEAM, REVIEW};
use kwnt::db::models::{Kwant, NewKwant, NewTeam};
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

async fn review(env: &Env, space: &str, head: &str, bot: Option<&str>, branch: &str) -> String {
    let row = env.state.db.insert_dev_card_in(space, REPO, REVIEW, "pr:31", head, &facts(head, bot, branch)).await.unwrap().unwrap();
    assert!(env.state.db.swap_dev_card_json(&row.id, None, &card()).await.unwrap());
    row.id
}

async fn bot(env: &Env, space: &SpaceId, name: &str) -> Kwant {
    let input = NewKwant {
        name: name.into(),
        title: "Engineer".into(),
        persona_prompt: String::new(),
        soul: String::new(),
        avatar_sprite: "s".into(),
        driver_type: "cli".into(),
        preferred_model: "claude".into(),
    };
    env.state.db.create_kwant(space, &input).await.unwrap()
}

/// A team "Edge" in `space`, led by `lead` with `members`, that owns [`REPO`] there.
async fn team_owns_repo(env: &Env, space: &SpaceId, lead: &str, members: &[&str]) {
    let input = NewTeam {
        name: "Edge".into(),
        description: None,
        icon: None,
        lead_kwant_id: lead.into(),
        member_kwants: members.iter().map(|m| m.to_string()).collect(),
        workspace_path: None,
        repo_url: None,
    };
    let team = env.state.db.create_team(space, &input).await.unwrap();
    env.state.db.add_dev_repo(space.as_str(), REPO, Some(&team.id), "oleks/edge", Some("github.com"), "main", None, BY_TEAM).await.unwrap();
}

async fn press(env: &Env, card: &str, body: Value) -> (u16, Value) {
    let (s, v) = call(&env.app, "POST", &format!("/api/dev/cards/{card}/findings/f1"), Some(body)).await;
    (s.as_u16(), v)
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

async fn recipients(env: &Env, card: &str) -> Value {
    let (s, v) = call(&env.app, "GET", &format!("/api/dev/cards/{card}/recipients"), None).await;
    assert_eq!(s.as_u16(), 200, "{v}");
    v
}

fn ids(v: &Value) -> Vec<String> {
    v["bots"].as_array().unwrap().iter().map(|b| b["id"].as_str().unwrap().to_string()).collect()
}

#[tokio::test]
async fn recipients_name_the_rule_s_pick_first_then_the_owning_team() {
    let env = setup().await;
    let space = SpaceId::default_space();
    let all = env.state.db.list_kwants(&space).await.unwrap().len();

    // No team owns the repo and a person wrote it: no default, the nobody line, every bot
    let card = review(&env, space.as_str(), "aaa1111", None, "oleks/live").await;
    let v = recipients(&env, &card).await;
    assert_eq!(v["default"], Value::Null, "{v}");
    assert_eq!(v["nobody"], "No bot wrote this PR and no team owns oleks/edge. Add the repo to a team (Manage team › Git Repo), then press Fix it again.");
    assert_eq!(ids(&v).len(), all, "{v}");

    // The repo joins a team led by b with member m: b is the default, m right after
    let m = bot(&env, &space, "Zara").await;
    team_owns_repo(&env, &space, &env.b, &[&m.id]).await;
    let v = recipients(&env, &card).await;
    assert_eq!(v["default"], json!({"id": env.b, "name": name_of(&env, &env.b).await, "via": "team_lead", "team": "Edge"}), "{v}");
    assert_eq!(v["nobody"], Value::Null);
    let order = ids(&v);
    assert_eq!((order[0].as_str(), order[1].as_str()), (env.b.as_str(), m.id.as_str()), "{v}");
    assert_eq!(order.len(), all + 1);
    assert_eq!((v["bots"][0]["team"].clone(), v["bots"][1]["team"].clone()), (json!("Edge"), json!("Edge")), "{v}");
    let rest: Vec<&Value> = v["bots"].as_array().unwrap()[2..].iter().collect();
    assert!(rest.iter().all(|b| b.get("team") != Some(&json!("Edge"))), "only Edge's bots say Edge: {v}");
    let names: Vec<String> = rest.iter().map(|b| b["name"].as_str().unwrap().to_lowercase()).collect();
    let mut sorted = names.clone();
    sorted.sort();
    assert_eq!(names, sorted, "the rest by name");

    // A bot's PR: the author bot is the default, with no team line
    let card = review(&env, space.as_str(), "bbb1111", Some(&env.a), "pixel/x").await;
    let v = recipients(&env, &card).await;
    assert_eq!(v["default"], json!({"id": env.a, "name": name_of(&env, &env.a).await, "via": "author"}), "{v}");
    assert_eq!(ids(&v)[0], env.a);

    // Unknown card: 404; a bot's token: refused like the card itself
    let (s, _) = call(&env.app, "GET", "/api/dev/cards/dvc_nope/recipients", None).await;
    assert_eq!(s.as_u16(), 404);
    let (s, _) = call_with(&env.app, "GET", &format!("/api/dev/cards/{card}/recipients"), None, &[("authorization", "Bearer not-the-person")]).await;
    assert_eq!(s.as_u16(), 403);
}

#[tokio::test]
async fn fix_it_to_a_chosen_bot_goes_there_and_tells_it_who_wrote_the_pr() {
    let env = setup().await;
    let space = SpaceId::default_space();
    let picked = bot(&env, &space, "Juno").await;
    // The author bot (a) wrote it; the person sends it to Juno
    let card = review(&env, space.as_str(), "ccc1111", Some(&env.a), "pixel/x").await;
    let (s, v) = press(&env, &card, json!({"action": "fix", "to": picked.id})).await;
    assert_eq!(s, 200, "{v}");
    let waiting = finding(&env, &card).await;
    assert_eq!(
        (waiting["state"].clone(), waiting["sent_to"].clone(), waiting["sent_via"].clone(), waiting["sent_to_id"].clone()),
        (json!("sent"), json!("Juno"), json!("chosen"), json!(picked.id)),
        "{waiting}"
    );
    assert!(waiting.get("sent_team").is_none() && waiting.get("task_id").is_none(), "{waiting}");
    routes::deliver_fix(&env.state, &card, "f1").await;
    let f = finding(&env, &card).await;
    assert_eq!((f["sent_to"].clone(), f["sent_via"].clone(), f["sent_to_id"].clone()), (json!("Juno"), json!("chosen"), json!(picked.id)), "{f}");
    let t = task_of(&env, &f).await;
    assert_eq!(t.direct_kwant_id.as_deref(), Some(picked.id.as_str()));
    let a = &t.acceptance;
    let author = name_of(&env, &env.a).await;
    assert!(a.contains(&format!("You were picked for this fix. {author} wrote this PR, not you.\n")), "{a}");
    assert!(a.contains("Push the fix as a new commit on the PR's branch. Never force-push or rewrite their commits.\n"), "{a}");
    assert!(a.contains("open a draft PR that links PR #31"), "{a}");

    // Undo after it went out: the task is cancelled and all four are forgotten
    let (s, v) = press(&env, &card, json!({"action": "undo_fix"})).await;
    assert_eq!(s, 200, "{v}");
    let back = finding(&env, &card).await;
    assert_eq!(back["state"], "open");
    for k in ["task_id", "sent_to", "sent_via", "sent_team", "sent_to_id"] {
        assert!(back.get(k).is_none(), "{k}: {back}");
    }
    assert_eq!(env.state.db.get_task(&t.id).await.unwrap().unwrap().state, "cancelled");

    // A person's PR, chosen bot: named by the person's login
    let card = review(&env, space.as_str(), "ccc2222", None, "oleks/live").await;
    press(&env, &card, json!({"action": "fix", "to": picked.id})).await;
    routes::deliver_fix(&env.state, &card, "f1").await;
    let t = task_of(&env, &finding(&env, &card).await).await;
    assert!(t.acceptance.contains("You were picked for this fix. oleks wrote this PR, not you."), "{}", t.acceptance);
}

#[tokio::test]
async fn choosing_the_author_bot_adds_no_lines_but_is_still_chosen() {
    let env = setup().await;
    let space = SpaceId::default_space();
    let card = review(&env, space.as_str(), "ddd1111", Some(&env.a), "pixel/x").await;
    let (s, v) = press(&env, &card, json!({"action": "fix", "to": env.a})).await;
    assert_eq!(s, 200, "{v}");
    routes::deliver_fix(&env.state, &card, "f1").await;
    let f = finding(&env, &card).await;
    assert_eq!((f["sent_via"].clone(), f["sent_to_id"].clone()), (json!("chosen"), json!(env.a)), "{f}");
    let t = task_of(&env, &f).await;
    assert_eq!(t.direct_kwant_id.as_deref(), Some(env.a.as_str()));
    for line in ["not you", "picked", "force-push", "fork"] {
        assert!(!t.acceptance.contains(line), "{line}: {}", t.acceptance);
    }
    assert!(t.acceptance.ends_with("Done when the fix is pushed to the PR's branch and its tests pass."));
}

#[tokio::test]
async fn a_bot_of_another_space_or_no_bot_is_a_400_and_the_finding_stays_open() {
    let env = setup().await;
    let space = SpaceId::default_space();
    let lab = env.state.db.create_space(&NewSpace { name: "Lab".into(), icon: "L".into(), color: None, this_machine_enabled: true }).await.unwrap();
    let elsewhere = bot(&env, &SpaceId::new(&lab.id), "Elsewhere").await;
    let card = review(&env, space.as_str(), "eee1111", Some(&env.a), "pixel/x").await;
    for to in [elsewhere.id.as_str(), "kwant-nope"] {
        let (s, v) = press(&env, &card, json!({"action": "fix", "to": to})).await;
        assert_eq!((s, v["code"].clone(), v["error"].clone()), (400, json!("bad_recipient"), json!("That bot isn't in this space")), "{v}");
        let f = finding(&env, &card).await;
        assert_eq!((f["state"].clone(), f.get("sent_at").cloned().unwrap_or(Value::Null)), (json!("open"), Value::Null), "{f}");
        assert!(f.get("sent_to").is_none(), "{f}");
    }
    // An empty `to` is no choice: today's rule
    let (s, v) = press(&env, &card, json!({"action": "fix", "to": " "})).await;
    assert_eq!(s, 200, "{v}");
    let f = finding(&env, &card).await;
    assert_eq!(f["sent_via"], "author", "{f}");
    assert!(f.get("sent_to_id").is_none(), "{f}");
}

#[tokio::test]
async fn redelivery_after_a_restart_goes_to_the_chosen_bot_and_a_gone_one_says_so() {
    let env = setup().await;
    let space = SpaceId::default_space();
    let picked = bot(&env, &space, "Juno").await;
    let card = review(&env, space.as_str(), "fff1111", Some(&env.a), "pixel/x").await;
    press(&env, &card, json!({"action": "fix", "to": picked.id})).await;
    // As after a restart: the finding is still waiting; the poller redelivers it
    let row = env.state.db.dev_card(&card).await.unwrap().unwrap();
    let c: Value = serde_json::from_str(row.card.as_deref().unwrap()).unwrap();
    assert_eq!(routes::undelivered(&c), vec!["f1".to_string()]);
    routes::deliver_fix(&env.state, &card, "f1").await;
    let t = task_of(&env, &finding(&env, &card).await).await;
    assert_eq!(t.direct_kwant_id.as_deref(), Some(picked.id.as_str()), "the chosen bot, not the author bot the rule picks");

    // Chosen, then the bot is deleted before the window ends: back to open, with what to do
    let gone = bot(&env, &space, "Rex").await;
    let card = review(&env, space.as_str(), "fff2222", Some(&env.a), "pixel/x").await;
    press(&env, &card, json!({"action": "fix", "to": gone.id})).await;
    assert!(env.state.db.delete_kwant_and_history(&gone.id, &env.a).await.unwrap());
    routes::deliver_fix(&env.state, &card, "f1").await;
    let f = finding(&env, &card).await;
    assert_eq!((f["state"].clone(), f["sent_at"].clone()), (json!("open"), Value::Null), "{f}");
    assert_eq!(f["send_error"], "Rex isn't in this space any more. Pick another bot and press Fix it again.");
    for k in ["task_id", "sent_to", "sent_via", "sent_to_id"] {
        assert!(f.get(k).is_none(), "{k}: {f}");
    }
}

#[tokio::test]
async fn line_notes_to_a_chosen_bot_reach_its_chat_with_the_lines() {
    let env = setup().await;
    let space = SpaceId::default_space();
    let picked = bot(&env, &space, "Juno").await;
    let card = review(&env, space.as_str(), "ggg1111", Some(&env.a), "pixel/x").await;
    let notes = format!("/api/dev/cards/{card}/notes");
    let note = json!({"path": "internal/edgefinder/live/live.go", "start": {"side": "new", "line": 2}, "end": {"side": "new", "line": 2}, "head_sha": "ggg1111", "text": "Move this to its own PR."});
    let (s, v) = call(&env.app, "POST", &notes, Some(note)).await;
    assert_eq!(s.as_u16(), 201, "{v}");

    // A bot that isn't in this space: 400, the draft stays
    let (s, v) = call(&env.app, "POST", &format!("{notes}/send"), Some(json!({"send_id": "n-1", "to": "kwant-nope"}))).await;
    assert_eq!((s.as_u16(), v["code"].clone()), (400, json!("bad_recipient")), "{v}");

    let (s, v) = call(&env.app, "POST", &format!("{notes}/send"), Some(json!({"send_id": "n-2", "to": picked.id}))).await;
    assert_eq!(s.as_u16(), 200, "{v}");
    assert_eq!((v["kwant_id"].clone(), v["kwant_name"].clone(), v["sent_via"].clone(), v["sent_team"].clone()), (json!(picked.id), json!("Juno"), json!("chosen"), Value::Null), "{v}");
    let (_, msgs) = call(&env.app, "GET", &format!("/api/channels/{}/messages", picked.id), None).await;
    let m = msgs.as_array().unwrap().iter().find(|m| m["text"].as_str().is_some_and(|t| t.contains("from the review"))).unwrap_or_else(|| panic!("{msgs}"));
    let text = m["text"].as_str().unwrap();
    let author = name_of(&env, &env.a).await;
    assert!(text.contains(&format!("You were picked for these notes. {author} wrote this PR, not you.")), "{text}");
    assert!(text.contains("never force-push") && text.contains("open a draft PR that links PR #31"), "{text}");
}
