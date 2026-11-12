//! Dev mode watches every team's repo (0.7.2, plan team-repos.md T1–T9), against the mock GitHub
//! only: two teams with two repos in one space, PR #12 in both, a repo that fails next to one that
//! works, the 10-repo cap, a 0.7.1 row taken over by its team, and the routes' error codes.
//!
//! Each test has a space of its own, so the poller's per-space state (wake list, back-off, tags)
//! is that test's alone. No real GitHub is called: every connection points at the mock.

mod common;
#[path = "support/mock_github.rs"]
mod mock_github;

use common::*;
use kwnt::db::dev_mode::{repo_key, BY_MIGRATION, LANDED, MAX_REPOS, READY, REVIEW};
use kwnt::db::models::{NewKwant, NewTeam};
use kwnt::db::spaces::{NewSpace, SpaceId};
use kwnt::server::dev::{poller, repo};
use mock_github::*;
use serde_json::{json, Value};

const APP: &str = "demo-org/app";
const SITE: &str = "demo-org/site";
const TOOLS: &str = "demo-org/tools";

fn sha(n: u8) -> String {
    format!("{n:0>40}")
}

/// One repo on the mock: a commit on main and open PR #12 at the same head in every repo, so only
/// the repo tells their cards apart.
fn seed(gh: &MockGithub, name: &str) {
    let r = format!("/repos/{name}");
    let person = ("Lin (kwant bot)", "lin-bot@users.noreply.example.com");
    let patch = "@@ -1,2 +1,2 @@\n-let retries = 1;\n+let retries = 3;";
    let green = check_runs(&[("rust", "completed", Some("success"))]);
    let mut first = commit(&sha(1), "Fix the README", person, &[], &hours_ago(3));
    gh.json(&r, repo(name, "main"))
        .json(&format!("{r}/contents"), json!([{"name": "Cargo.toml", "type": "file"}]))
        .json(&format!("{r}/branches/main"), json!({"name": "main"}))
        .json(&format!("{r}/tags"), json!([]))
        .json(&format!("{r}/commits"), json!([first.clone()]));
    first["files"] = json!([file("README.md", "modified", "-old\n+new")]);
    gh.json(&format!("{r}/commits/{}", sha(1)), first)
        .json(&format!("{r}/commits/{}/pulls", sha(1)), json!([]))
        .json(&format!("{r}/commits/{}/check-runs", sha(1)), green.clone())
        .json(&format!("{r}/pulls"), json!([pull(12, "Add a cache", ("cache", &sha(3)), "main", "demo-person", None)]))
        .json(&format!("{r}/pulls/12"), pull(12, "Add a cache", ("cache", &sha(3)), "main", "demo-person", None))
        .json(&format!("{r}/pulls/12/files"), json!([file("src/cache.rs", "modified", patch)]))
        .json(&format!("{r}/commits/{}", sha(3)), commit(&sha(3), "Add a cache", person, &[&sha(1)], &hours_ago(1)))
        .json(&format!("{r}/commits/{}/check-runs", sha(3)), green);
}

struct Space {
    id: SpaceId,
    lead: String,
}

/// A new space, a GitHub connection to the mock, and the token the mock wants.
async fn space(env: &Env, gh: &MockGithub, name: &str) -> Space {
    let s = env.state.db.create_space(&NewSpace { name: name.into(), icon: "S".into(), color: None, this_machine_enabled: true }).await.unwrap();
    let id = SpaceId::new(s.id);
    for s in [&id, &SpaceId::default_space()] {
        env.state.db.set_space_approvals(s, "ask").await.unwrap();
    }
    let k = NewKwant {
        name: format!("{name} lead"),
        title: "Lead".into(),
        persona_prompt: "Test persona".into(),
        soul: String::new(),
        avatar_sprite: "s".into(),
        driver_type: "api".into(),
        preferred_model: "claude".into(),
    };
    let lead = env.state.db.create_kwant(&id, &k).await.unwrap().id;
    let token = canary_token();
    gh.require_token(&token);
    gh.json("/user", user("demo-person"));
    let (s, v) = call(&env.app, "POST", "/api/zombie/sources", Some(json!({"kind": "github", "label": "GitHub", "config": {"api_base": gh.base}, "secret": token}))).await;
    assert_eq!(s.as_u16(), 201, "{v}");
    Space { id, lead }
}

/// A team of `space` whose repo link is `name` on the mock (`None`: no link).
async fn team(env: &Env, space: &Space, team: &str, gh: &MockGithub, name: Option<&str>) -> String {
    let t = NewTeam {
        name: team.into(),
        description: None,
        icon: None,
        lead_kwant_id: space.lead.clone(),
        member_kwants: vec![space.lead.clone()],
        workspace_path: None,
        repo_url: name.map(|n| format!("{}/{n}", gh.base)),
    };
    env.state.db.create_team(&space.id, &t).await.unwrap().id
}

fn host(gh: &MockGithub) -> String {
    gh.base.trim_start_matches("http://").split(':').next().unwrap().to_string()
}

fn uri(space: &Space) -> String {
    format!("/api/spaces/{}/dev_mode", space.id.as_str())
}

/// `(repo, team, status, added_by)` of each `repos[]` row.
fn rows(v: &Value) -> Vec<(String, String, String, String)> {
    let s = |x: &Value| x.as_str().unwrap_or("-").to_string();
    v["repos"].as_array().unwrap_or_else(|| panic!("no repos[]: {v}")).iter().map(|r| (s(&r["repo"]), s(&r["team"]), s(&r["status"]), s(&r["added_by"]))).collect()
}

fn row(repo: &str, team: &str, status: &str, by: &str) -> (String, String, String, String) {
    (repo.into(), team.into(), status.into(), by.into())
}

fn enc(key: &str) -> String {
    key.replace('/', "%2F")
}

fn review_card() -> Value {
    json!({
        "kind": "review", "pr": 12, "title": "Add a cache",
        "reviewer": {"id": "kw_v", "name": "Vega", "engine": "codex", "model": "gpt"},
        "risk": {"level": "low", "reasons": []},
        "intent": {"source": "pr", "asked": "A cache", "rows": [], "none": true},
        "tamper": {"flags": [], "unread": []},
        "security": {"secrets": {"state": "pass", "count": 0, "missing": 0}, "packages": {"state": "pass", "count": 0, "missing": 0}, "osv": {"state": "later"}, "semgrep": {"state": "later"}},
        "findings": [],
    })
}

/// T1, T3, T5, T6, T8: two teams, two repos, PR #12 in both is two cards; each card posts to its
/// own repo; the card lists filter by repo; a team's link change adds and removes its row.
#[tokio::test]
async fn two_teams_two_repos_and_pr_12_in_both_is_two_cards() {
    let env = setup().await;
    let gh = MockGithub::start(&[0]).await;
    seed(&gh, APP);
    seed(&gh, SITE);
    seed(&gh, TOOLS);
    let sp = space(&env, &gh, "Two teams").await;
    team(&env, &sp, "App", &gh, Some(APP)).await;
    let site_team = team(&env, &sp, "Site", &gh, Some(SITE)).await;
    let h = host(&gh);
    let (app_key, site_key) = (repo_key(&h, APP), repo_key(&h, SITE));

    // Off: nothing is fetched, both show what on would watch
    let (s, v) = call(&env.app, "GET", &uri(&sp), None).await;
    assert_eq!(s.as_u16(), 200, "{v}");
    assert_eq!(rows(&v), vec![row(APP, "App", "watching", "-"), row(SITE, "Site", "watching", "-")], "{v}");
    assert!(env.state.db.dev_repos(sp.id.as_str()).await.unwrap().is_empty(), "off reads only");

    // On: a team row for each, in team order
    let (s, v) = call(&env.app, "PUT", &uri(&sp), Some(json!({"on": true}))).await;
    assert_eq!(s.as_u16(), 200, "{v}");
    assert_eq!(rows(&v), vec![row(APP, "App", "watching", "team"), row(SITE, "Site", "watching", "team")], "{v}");
    assert_eq!((v["repos"][0]["key"].clone(), v["repos"][1]["team_id"].clone()), (json!(app_key), json!(site_team)), "{v}");
    assert_eq!((v["status"].clone(), v["repo"].clone(), v["branch"].clone()), (json!("watching"), json!(APP), json!("main")), "deprecated fields are the first row's: {v}");
    assert!(poller::is_due_soon(sp.id.as_str()));

    // The desk is opened before the first reviews (#357: its own seen time)
    let (s, v) = call(&env.app, "POST", &format!("{}/seen", uri(&sp)), None).await;
    assert_eq!(s.as_u16(), 200, "{v}");

    // One poll of the space reads both: PR #12 in each is its own card
    poller::poll_space(&env.state, sp.id.as_str()).await;
    let reviews = env.state.db.dev_cards_in(sp.id.as_str(), None, REVIEW, None, 50).await.unwrap();
    let mut got: Vec<_> = reviews.iter().map(|c| (c.repo.clone(), c.ref_.clone(), c.head_sha.clone())).collect();
    got.sort();
    assert_eq!(got, vec![(app_key.clone(), "pr:12".into(), sha(3)), (site_key.clone(), "pr:12".into(), sha(3))]);
    assert_eq!(env.state.db.dev_cards_in(sp.id.as_str(), None, LANDED, None, 50).await.unwrap().len(), 2);
    for r in env.state.db.dev_repos(sp.id.as_str()).await.unwrap() {
        assert_eq!((r.last_seen_sha.as_deref(), r.poll_error.as_deref()), (Some(sha(1).as_str()), None), "{}", r.key);
    }
    // A second poll makes nothing new
    poller::poll_space(&env.state, sp.id.as_str()).await;
    assert_eq!(env.state.db.dev_cards_in(sp.id.as_str(), None, REVIEW, None, 50).await.unwrap().len(), 2);

    // The badge is the space's total over every repo; Mark seen (the rollup) leaves it, opening the
    // desk clears it. The cards' own text runs first, so it can't overwrite the status set here
    wait_for("the cards' text", || async { env.state.db.pending_dev_cards().await.map(|p| p.is_empty()).unwrap_or(false) }).await;
    for c in &reviews {
        env.state.db.set_dev_card(&c.id, READY, Some(&review_card().to_string())).await.unwrap().unwrap();
    }
    let badge = |v: &Value| v["new_reviews"].clone();
    let (_, v) = call(&env.app, "GET", &uri(&sp), None).await;
    assert_eq!(badge(&v), json!(2), "PR #12 in both repos: {v}");
    let (s, v) = call(&env.app, "POST", &format!("/api/spaces/{}/dev/landed/seen", sp.id.as_str()), None).await;
    assert_eq!(s.as_u16(), 200, "{v}");
    let (_, v) = call(&env.app, "GET", &uri(&sp), None).await;
    assert_eq!(badge(&v), json!(2), "Mark seen moves only the rollup: {v}");
    let (s, v) = call(&env.app, "POST", &format!("{}/seen", uri(&sp)), None).await;
    assert_eq!(s.as_u16(), 200, "{v}");
    let (_, v) = call(&env.app, "GET", &uri(&sp), None).await;
    assert_eq!(badge(&v), json!(0), "{v}");

    // The lists: every repo, or one with ?repo= (encoded or not)
    let base = format!("/api/spaces/{}/dev", sp.id.as_str());
    let (_, v) = call(&env.app, "GET", &format!("{base}/reviews"), None).await;
    let cards = v["cards"].as_array().unwrap_or_else(|| panic!("{v}")).clone();
    assert_eq!(cards.len(), 2, "{v}");
    for c in &cards {
        let (team, name) = if c["repo"] == json!(app_key) { ("App", APP) } else { ("Site", SITE) };
        assert_eq!((c["team"].clone(), c["repo_name"].clone(), c["pr"].clone()), (json!(team), json!(name), json!(12)), "{c}");
    }
    for q in [enc(&site_key), site_key.clone()] {
        let (s, v) = call(&env.app, "GET", &format!("{base}/reviews?repo={q}"), None).await;
        assert_eq!(s.as_u16(), 200, "{v}");
        let list = v["cards"].as_array().unwrap().clone();
        assert_eq!(list.iter().map(|c| c["repo"].clone()).collect::<Vec<_>>(), vec![json!(site_key)], "{v}");
    }
    let (_, v) = call(&env.app, "GET", &format!("{base}/landed?repo={}", enc(&app_key)), None).await;
    let landed = v["cards"].as_array().or_else(|| v["landed"].as_array()).unwrap_or_else(|| panic!("{v}")).clone();
    assert_eq!(landed.iter().map(|c| c["repo"].clone()).collect::<Vec<_>>(), vec![json!(app_key)], "{v}");
    let (s, v) = call(&env.app, "GET", &format!("{base}/landed?repo=github.com%2Fnobody%2Fhere"), None).await;
    assert_eq!(s.as_u16(), 200, "{v}");
    assert_eq!(v["cards"].as_array().or_else(|| v["landed"].as_array()).map(Vec::len), Some(0), "an unknown repo is an empty list: {v}");

    // T6: posting the Site card writes to Site's PR #12 only
    let site_card = reviews.iter().find(|c| c.repo == site_key).unwrap();
    env.state.db.set_dev_card(&site_card.id, READY, Some(&review_card().to_string())).await.unwrap();
    let to = format!("/repos/{SITE}/pulls/12/reviews");
    gh.accept_write("POST", &to, 200, json!({"id": 7, "user": user("demo-person"), "body": "…", "commit_id": sha(3), "state": "COMMENTED", "html_url": "https://github.example/demo-org/site/pull/12#pullrequestreview-7"}));
    let (s, v) = call(&env.app, "POST", &format!("/api/dev/cards/{}/post", site_card.id), None).await;
    assert_eq!(s.as_u16(), 200, "{v}");
    assert!(v["posted"].is_object() || v["posted"].is_string(), "posted: {v}");
    assert_eq!(gh.writes().iter().map(|w| w.path.clone()).collect::<Vec<_>>(), vec![to.clone()]);
    gh.assert_no_unexpected_writes();

    // A hand repo next to them; DELETE removes only a hand row
    let (s, v) = call(&env.app, "PUT", &uri(&sp), Some(json!({"repo": TOOLS}))).await;
    assert_eq!(s.as_u16(), 200, "{v}");
    assert_eq!(rows(&v).last().unwrap(), &row(TOOLS, "-", "watching", "hand"), "{v}");
    let tools_key = repo_key(&h, TOOLS);
    let (s, v) = call(&env.app, "DELETE", &format!("{}/repos/{}", uri(&sp), enc(&app_key)), None).await;
    assert_eq!((s.as_u16(), v["code"].clone()), (409, json!("team_repo")), "{v}");
    let (s, v) = call(&env.app, "DELETE", &format!("{}/repos/{}", uri(&sp), enc("github.com/nobody/here")), None).await;
    assert_eq!((s.as_u16(), v["code"].clone()), (404, json!("unknown_repo")), "{v}");
    let (s, v) = call(&env.app, "DELETE", &format!("{}/repos/{}", uri(&sp), tools_key), None).await;
    assert_eq!(s.as_u16(), 200, "raw slashes work too: {v}");
    assert_eq!(rows(&v).len(), 2, "{v}");

    // T3 remove: clearing Site's link stops watching it; its cards stay with their repo
    let (s, v) = call(&env.app, "PUT", &uri(&sp), Some(json!({"repo": "", "team": "Site"}))).await;
    assert_eq!(s.as_u16(), 200, "{v}");
    assert_eq!(rows(&v), vec![row(APP, "App", "watching", "team")], "{v}");
    assert_eq!(env.state.db.dev_cards_in(sp.id.as_str(), Some(&site_key), REVIEW, None, 50).await.unwrap().len(), 1);
    let (_, v) = call(&env.app, "GET", &format!("/api/dev/cards/{}", site_card.id), None).await;
    assert_eq!((v["repo_name"].clone(), v["team"].clone()), (json!(SITE), Value::Null), "a removed repo keeps its label: {v}");

    // T3 keep: App's row is the same row, its place kept
    let before = env.state.db.dev_repo(sp.id.as_str(), &app_key).await.unwrap().unwrap();
    // T3 add, by the team's id, with userinfo that is never stored or returned
    let secret = secret_like("ghp_", 36);
    let (s, v) = call(&env.app, "PUT", &uri(&sp), Some(json!({"repo": format!("http://demo:{secret}@{}/{SITE}", gh.base.trim_start_matches("http://")), "team": site_team}))).await;
    assert_eq!(s.as_u16(), 200, "{v}");
    assert_eq!(rows(&v), vec![row(APP, "App", "watching", "team"), row(SITE, "Site", "watching", "team")], "{v}");
    assert!(!v.to_string().contains(&secret));
    let teams = env.state.db.list_teams(&sp.id).await.unwrap();
    let link = teams.iter().find(|t| t.id == site_team).unwrap().repo_url.clone().unwrap();
    assert!(!link.contains(&secret) && !link.contains('@'), "{link}");
    assert_eq!(env.state.db.dev_repo(sp.id.as_str(), &app_key).await.unwrap().unwrap(), before, "a kept row is untouched");

    // Deleting a team stops watching its repo (in the background: the route doesn't wait)
    let (s, _) = call(&env.app, "DELETE", &format!("/api/teams/{site_team}"), None).await;
    assert_eq!(s.as_u16(), 200);
    wait_for("the Site row to go", || async { env.state.db.dev_repo(sp.id.as_str(), &site_key).await.unwrap().is_none() }).await;
    gh.assert_no_unexpected_writes();
}

/// T5: a repo that fails records its own error, and the next one is still read.
#[tokio::test]
async fn one_failing_repo_does_not_stop_the_other() {
    let env = setup().await;
    let gh = MockGithub::start(&[0]).await;
    seed(&gh, APP);
    seed(&gh, SITE);
    let sp = space(&env, &gh, "One fails").await;
    team(&env, &sp, "App", &gh, Some(APP)).await;
    team(&env, &sp, "Site", &gh, Some(SITE)).await;
    let h = host(&gh);
    let (app_key, site_key) = (repo_key(&h, APP), repo_key(&h, SITE));
    let (s, v) = call(&env.app, "PUT", &uri(&sp), Some(json!({"on": true}))).await;
    assert_eq!(s.as_u16(), 200, "{v}");
    assert_eq!(env.state.db.dev_repos(sp.id.as_str()).await.unwrap().first().map(|r| r.key.clone()), Some(app_key.clone()), "App is read first");

    for fault in [Fault::Server(502), Fault::Forbidden, Fault::Unauthorized] {
        gh.fault(&format!("/repos/{APP}/commits"), fault.clone());
        poller::poll_space(&env.state, sp.id.as_str()).await;
        let app = env.state.db.dev_repo(sp.id.as_str(), &app_key).await.unwrap().unwrap();
        let site = env.state.db.dev_repo(sp.id.as_str(), &site_key).await.unwrap().unwrap();
        assert!(app.poll_error.is_some(), "{fault:?}: App records its error");
        assert_eq!((site.poll_error.as_deref(), site.last_seen_sha.as_deref()), (None, Some(sha(1).as_str())), "{fault:?}: Site is still read");
        assert!(site.polled_at.is_some());
    }
    assert_eq!(env.state.db.dev_cards_in(sp.id.as_str(), Some(&site_key), REVIEW, None, 50).await.unwrap().len(), 1);
    assert!(env.state.db.dev_cards_in(sp.id.as_str(), Some(&app_key), LANDED, None, 50).await.unwrap().is_empty());

    // The desk: App says error with why, Site watching; the space is still watching
    let (_, v) = call(&env.app, "GET", &uri(&sp), None).await;
    assert_eq!(rows(&v), vec![row(APP, "App", "error", "team"), row(SITE, "Site", "watching", "team")], "{v}");
    assert!(v["repos"][0]["poll_error"].is_string() && v["repos"][1]["poll_error"].is_null(), "{v}");
    assert_eq!(v["status"], "watching");
    assert!(!v.to_string().contains(&gh.base), "no mock URL in the reply: {v}");

    // Healed: the error clears on the next poll
    gh.heal(&format!("/repos/{APP}/commits"));
    poller::poll_space(&env.state, sp.id.as_str()).await;
    let app = env.state.db.dev_repo(sp.id.as_str(), &app_key).await.unwrap().unwrap();
    assert_eq!((app.poll_error.as_deref(), app.last_seen_sha.as_deref()), (None, Some(sha(1).as_str())));

    // A rate limit is the token's, so it backs off the whole space (both repos wait)
    gh.fault(&format!("/repos/{APP}/commits"), Fault::TooMany { retry_after: Some(120) });
    let site_before = env.state.db.dev_repo(sp.id.as_str(), &site_key).await.unwrap().unwrap().polled_at;
    poller::poll_space(&env.state, sp.id.as_str()).await;
    assert!(env.state.db.dev_repo(sp.id.as_str(), &app_key).await.unwrap().unwrap().poll_error.is_some());
    assert_eq!(env.state.db.dev_repo(sp.id.as_str(), &site_key).await.unwrap().unwrap().polled_at, site_before, "Site waits out the limit too");
    gh.assert_read_only();
}

/// T4: at most 10 repos a space; the 11th team's repo shows `too_many`, and a hand repo past the
/// cap is refused with 409 `too_many`.
#[tokio::test]
async fn a_space_watches_at_most_ten_repos() {
    let env = setup().await;
    let gh = MockGithub::start(&[0]).await;
    let sp = space(&env, &gh, "Many").await;
    let names: Vec<String> = (0..=MAX_REPOS).map(|i| format!("demo-org/r{i:02}")).collect();
    for (i, n) in names.iter().enumerate() {
        seed(&gh, n);
        team(&env, &sp, &format!("Team {i:02}"), &gh, Some(n)).await;
    }
    seed(&gh, TOOLS);
    let (s, v) = call(&env.app, "PUT", &uri(&sp), Some(json!({"on": true}))).await;
    assert_eq!(s.as_u16(), 200, "{v}");
    let got = rows(&v);
    assert_eq!(got.len(), MAX_REPOS + 1, "{v}");
    assert_eq!(got.iter().filter(|r| r.2 == "watching" && r.3 == "team").count(), MAX_REPOS, "{v}");
    assert_eq!(got.last().unwrap(), &row(&names[MAX_REPOS], &format!("Team {MAX_REPOS:02}"), "too_many", "-"), "{v}");
    assert_eq!(env.state.db.dev_repos(sp.id.as_str()).await.unwrap().len(), MAX_REPOS);

    let (s, v) = call(&env.app, "PUT", &uri(&sp), Some(json!({"repo": TOOLS}))).await;
    assert_eq!((s.as_u16(), v["code"].clone()), (409, json!("too_many")), "{v}");
    assert_eq!(env.state.db.dev_repos(sp.id.as_str()).await.unwrap().len(), MAX_REPOS);

    // Clearing one team's link makes room for the 11th at the next reconcile
    let (s, v) = call(&env.app, "PUT", &uri(&sp), Some(json!({"repo": "", "team": "Team 00"}))).await;
    assert_eq!(s.as_u16(), 200, "{v}");
    let got = rows(&v);
    assert_eq!(got.len(), MAX_REPOS, "{v}");
    assert!(got.iter().all(|r| r.2 == "watching"), "{v}");
    gh.assert_read_only();
}

/// T2 + T3: a space's 0.7.1 repo, migrated as `github.com/…`, is taken over by the team that points
/// at it on another GitHub host: re-keyed with its cards, its place kept, nothing announced again.
#[tokio::test]
async fn a_migrated_repo_becomes_its_teams_row_and_nothing_is_announced_again() {
    let env = setup().await;
    let gh = MockGithub::start(&[0]).await;
    seed(&gh, APP);
    let sp = space(&env, &gh, "Migrated").await;
    let s = sp.id.as_str();
    // As the migration leaves a 0.7.1 space: one row with its place, and its old card
    env.state.db.set_dev_mode(s, false, None, None).await.unwrap();
    let old = repo_key("github.com", APP);
    env.state.db.add_dev_repo(s, &old, None, APP, Some("github.com"), "main", None, BY_MIGRATION).await.unwrap();
    env.state.db.dev_repo_polled(s, &old, Some(&sha(1)), None).await.unwrap();
    env.state.db.insert_dev_card_in(s, &old, LANDED, &sha(1), &sha(1), &json!({"repo": APP}).to_string()).await.unwrap().unwrap();
    assert_eq!(env.state.db.dev_repo(s, &old).await.unwrap().unwrap().added_by_shown(), "hand", "until a reconcile it reads as hand");

    // The team was there in 0.7.1 too (its link is what the space watched); the first read settles it
    let team_id = team(&env, &sp, "App", &gh, Some(APP)).await;
    let (_, v) = call(&env.app, "GET", &uri(&sp), None).await;
    assert_eq!(rows(&v), vec![row(APP, "App", "watching", "team")], "{v}");
    let (s2, v) = call(&env.app, "PUT", &uri(&sp), Some(json!({"on": true}))).await;
    assert_eq!(s2.as_u16(), 200, "{v}");
    assert_eq!(rows(&v), vec![row(APP, "App", "watching", "team")], "{v}");
    let new = repo_key(&host(&gh), APP);
    let r = env.state.db.dev_repo(s, &new).await.unwrap().expect("re-keyed to the team's host");
    assert_eq!((r.team.as_deref(), r.last_seen_sha.as_deref()), (Some(team_id.as_str()), Some(sha(1).as_str())));
    assert!(env.state.db.dev_repo(s, &old).await.unwrap().is_none());
    assert_eq!(env.state.db.dev_cards_in(s, Some(&new), LANDED, None, 10).await.unwrap().len(), 1, "its cards moved with it");

    poller::poll_space(&env.state, s).await;
    assert_eq!(env.state.db.dev_cards_in(s, None, LANDED, None, 10).await.unwrap().len(), 1, "the commit it had seen isn't announced again");
    assert_eq!(env.state.db.dev_cards_in(s, None, REVIEW, None, 10).await.unwrap().len(), 1, "the open PR is new");
    // A plain reconcile later changes nothing
    let done = repo::reconcile(&env.state, &sp.id).await;
    assert!(done.added.is_empty() && done.removed.is_empty() && done.too_many.is_empty());
    gh.assert_read_only();
}

/// T8: the route errors carry their codes.
#[tokio::test]
async fn the_routes_say_what_went_wrong() {
    let env = setup().await;
    let gh = MockGithub::start(&[0]).await;
    seed(&gh, APP);
    let sp = space(&env, &gh, "Codes").await;
    team(&env, &sp, "App", &gh, Some(APP)).await;
    let u = uri(&sp);
    for (body, status, code) in [
        (json!({"repo": "not a repo at all"}), 400, "bad_repo"),
        (json!({"repo": APP, "team": "Nobody"}), 400, "unknown_team"),
        (json!({"repo": "nope", "team": "App"}), 400, "bad_repo"),
        (json!({"branch": "bad..branch"}), 400, "bad_branch"),
        (json!({"backup_command": "x".repeat(2001)}), 400, "backup_too_long"),
    ] {
        let (s, v) = call(&env.app, "PUT", &u, Some(body.clone())).await;
        assert_eq!((s.as_u16(), v["code"].clone()), (status, json!(code)), "{body}: {v}");
        assert!(v["error"].is_string());
    }
    let (s, v) = call(&env.app, "POST", &format!("/api/spaces/{}/dev/release_notes", sp.id.as_str()), None).await;
    assert_eq!((s.as_u16(), v["code"].clone()), (409, json!("off")), "{v}");
    let (s, v) = call(&env.app, "PUT", &u, Some(json!({"on": true}))).await;
    assert_eq!(s.as_u16(), 200, "{v}");
    let (s, v) = call(&env.app, "PUT", &u, Some(json!({"repo": APP, "branch": "no-such-branch"}))).await;
    assert_eq!((s.as_u16(), v["code"].clone()), (400, json!("no_branch")), "{v}");
    gh.assert_read_only();
}

// ---- QA 0.7.2 (qa-072.md) h, c, g ------------------------------------------------------------

/// QA h: a GitHub connection deleted after a poll leaves the rows reading `not_connected` (Needs
/// GitHub, Connect), not an error about a refused token; connecting again recovers to watching.
#[tokio::test]
async fn a_lost_connection_reads_as_not_connected_and_reconnecting_recovers() {
    let env = setup().await;
    let gh = MockGithub::start(&[0]).await;
    seed(&gh, APP);
    seed(&gh, SITE);
    let sp = space(&env, &gh, "Lost").await;
    team(&env, &sp, "App", &gh, Some(APP)).await;
    team(&env, &sp, "Site", &gh, Some(SITE)).await;
    let (s, v) = call(&env.app, "PUT", &uri(&sp), Some(json!({"on": true}))).await;
    assert_eq!(s.as_u16(), 200, "{v}");
    poller::poll_space(&env.state, sp.id.as_str()).await;
    let (_, v) = call(&env.app, "GET", &uri(&sp), None).await;
    assert_eq!(rows(&v), vec![row(APP, "App", "watching", "team"), row(SITE, "Site", "watching", "team")], "{v}");

    // The connection goes; the next poll can't read either repo
    let (_, list) = call(&env.app, "GET", "/api/zombie/sources", None).await;
    let ids: Vec<String> = list["sources"].as_array().unwrap_or_else(|| panic!("{list}")).iter().filter(|s| s["kind"] == "github").map(|s| s["id"].as_str().unwrap().to_string()).collect();
    assert_eq!(ids.len(), 1, "{list}");
    let (s, v) = call(&env.app, "DELETE", &format!("/api/zombie/sources/{}", ids[0]), None).await;
    assert_eq!(s.as_u16(), 200, "{v}");
    poller::poll_space(&env.state, sp.id.as_str()).await;
    for r in env.state.db.dev_repos(sp.id.as_str()).await.unwrap() {
        let e = r.poll_error.unwrap_or_else(|| panic!("{}: the failed poll is recorded", r.key));
        assert!(!e.contains("401") && !e.contains("refused the token"), "no connection isn't a refused token: {e}");
    }
    let (_, v) = call(&env.app, "GET", &uri(&sp), None).await;
    assert_eq!(rows(&v), vec![row(APP, "App", "not_connected", "team"), row(SITE, "Site", "not_connected", "team")], "{v}");
    assert_eq!(v["status"], "not_connected", "{v}");

    // Connecting again: watching after the next poll, errors cleared
    let token = canary_token();
    gh.require_token(&token);
    let (s, v) = call(&env.app, "POST", "/api/zombie/sources", Some(json!({"kind": "github", "label": "GitHub", "config": {"api_base": gh.base}, "secret": token}))).await;
    assert_eq!(s.as_u16(), 201, "{v}");
    poller::poll_space(&env.state, sp.id.as_str()).await;
    let (_, v) = call(&env.app, "GET", &uri(&sp), None).await;
    assert_eq!(rows(&v), vec![row(APP, "App", "watching", "team"), row(SITE, "Site", "watching", "team")], "{v}");
    assert_eq!(v["status"], "watching", "{v}");
    assert!(v["repos"].as_array().unwrap().iter().all(|r| r["poll_error"].is_null()), "{v}");
    gh.assert_read_only();
}

/// QA c: at the cap, removing a hand row gives its place to the team repo that was left out right
/// away, not at a read 30 s later.
#[tokio::test]
async fn removing_a_hand_repo_at_the_cap_watches_the_team_repo_left_out() {
    let env = setup().await;
    let gh = MockGithub::start(&[0]).await;
    let sp = space(&env, &gh, "Cap").await;
    let names: Vec<String> = (0..MAX_REPOS).map(|i| format!("demo-org/r{i:02}")).collect();
    for n in &names {
        seed(&gh, n);
    }
    seed(&gh, TOOLS);
    // Nine teams with a repo, and a tenth with none yet
    for (i, n) in names.iter().enumerate() {
        team(&env, &sp, &format!("Team {i:02}"), &gh, (i + 1 < MAX_REPOS).then_some(n.as_str())).await;
    }
    let (s, v) = call(&env.app, "PUT", &uri(&sp), Some(json!({"on": true}))).await;
    assert_eq!(s.as_u16(), 200, "{v}");
    let (s, v) = call(&env.app, "PUT", &uri(&sp), Some(json!({"repo": format!("{}/{TOOLS}", gh.base)}))).await;
    assert_eq!(s.as_u16(), 200, "{v}");
    assert_eq!(env.state.db.dev_repos(sp.id.as_str()).await.unwrap().len(), MAX_REPOS, "{v}");
    // The tenth team links its repo: no place left
    let last = &names[MAX_REPOS - 1];
    let team10 = format!("Team {:02}", MAX_REPOS - 1);
    let (s, v) = call(&env.app, "PUT", &uri(&sp), Some(json!({"repo": format!("{}/{last}", gh.base), "team": team10}))).await;
    assert_eq!(s.as_u16(), 200, "{v}");
    let got = rows(&v);
    assert!(got.contains(&row(last, &team10, "too_many", "-")), "{v}");

    // Removing the hand repo: the tenth team's repo is watched in the same reply
    let tools_key = repo_key(&host(&gh), TOOLS);
    let (s, v) = call(&env.app, "DELETE", &format!("{}/repos/{}", uri(&sp), enc(&tools_key)), None).await;
    assert_eq!(s.as_u16(), 200, "{v}");
    let got = rows(&v);
    assert_eq!(got.len(), MAX_REPOS, "{v}");
    assert!(got.contains(&row(last, &team10, "watching", "team")), "{v}");
    assert!(env.state.db.dev_repo(sp.id.as_str(), &repo_key(&host(&gh), last)).await.unwrap().is_some());
    gh.assert_read_only();
}

/// QA g: a repo added while off that GitHub doesn't have reads as an error after switch-on, not
/// watching with no branch; once it exists, the next reconcile reads its branch and clears it.
#[tokio::test]
async fn a_missing_repo_added_while_off_reads_as_an_error_at_switch_on() {
    let env = setup().await;
    let gh = MockGithub::start(&[0]).await;
    seed(&gh, APP);
    let sp = space(&env, &gh, "Ghost").await;
    team(&env, &sp, "App", &gh, Some(APP)).await;
    const GHOST: &str = "demo-org/ghost";
    let (s, v) = call(&env.app, "PUT", &uri(&sp), Some(json!({"repo": format!("{}/{GHOST}", gh.base)}))).await;
    assert_eq!(s.as_u16(), 200, "off keeps it as given: {v}");
    let key = repo_key(&host(&gh), GHOST);
    assert_eq!(env.state.db.dev_repo(sp.id.as_str(), &key).await.unwrap().map(|r| r.branch), Some(String::new()));

    let (s, v) = call(&env.app, "PUT", &uri(&sp), Some(json!({"on": true}))).await;
    assert_eq!(s.as_u16(), 200, "{v}");
    assert_eq!(rows(&v), vec![row(APP, "App", "watching", "team"), row(GHOST, "-", "error", "hand")], "{v}");
    let ghost = &v["repos"][1];
    assert!(ghost["poll_error"].as_str().is_some_and(|e| e.contains("no such repo")), "{v}");
    assert!(ghost["branch"].is_null(), "{v}");
    assert_eq!(v["status"], "watching", "the other repo is still watched: {v}");
    // A poll leaves the error (the row has no branch to read) and the other repo is read
    poller::poll_space(&env.state, sp.id.as_str()).await;
    assert!(env.state.db.dev_repo(sp.id.as_str(), &key).await.unwrap().unwrap().poll_error.is_some());

    // It appears: the next reconcile fills its branch and clears the error
    seed(&gh, GHOST);
    repo::reconcile(&env.state, &sp.id).await;
    let r = env.state.db.dev_repo(sp.id.as_str(), &key).await.unwrap().unwrap();
    assert_eq!((r.branch.as_str(), r.poll_error.as_deref()), ("main", None));
    gh.assert_read_only();
}
