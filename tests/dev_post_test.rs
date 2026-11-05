//! Post to GitHub (0.7.1, plan/post-to-github.md) against the shared mock GitHub: one COMMENT
//! review pinned to the card's head, inline comments only on diff lines, the 422 retry with
//! everything in the body, no secret in anything sent, stale heads, read-only tokens, one review
//! per card however often Post is pressed, and only the person may press it.
//!
//! Nothing here reaches a real GitHub: every request goes to the mock on a loopback `api_base`.

mod common;
#[path = "support/mock_github.rs"]
mod mock_github;

use common::*;
use kwnt::db::dev_mode::{READY, RELEASE_NOTES, REVIEW, STALE};
use kwnt::db::spaces::SpaceId;
use kwnt::server::continuity::{SelfScope, TurnOrigin};
use kwnt::server::mcp_runtime;
use mock_github::*;
use serde_json::{json, Value};

const REPO: &str = "demo-org/demo-app";
const PULL: &str = "/repos/demo-org/demo-app/pulls/24";
const REVIEWS: &str = "/repos/demo-org/demo-app/pulls/24/reviews";
const PORTS: [u16; 4] = [9495, 9496, 9497, 9498];

fn head() -> String {
    "c0ffee".repeat(7)[..40].to_string()
}

/// A turn token of bot `a`, as its CLI holds it.
async fn turn_token(env: &Env) -> String {
    let bot = env.state.db.get_kwant(&env.a).await.unwrap().unwrap();
    let scope = SelfScope {
        kwant_id: bot.id.clone(),
        kwant_name: bot.name.clone(),
        origin: Some(TurnOrigin { direct_kwant_id: Some(bot.id.clone()), ..Default::default() }),
        cancel: Some(tokio_util::sync::CancellationToken::new()),
        turn_id: Some("turn-dev-post".into()),
        sandbox_target: Some(bot.id.clone()),
        ..Default::default()
    };
    mcp_runtime::prepare_turn_with_self(&env.state, &bot, Some("turn-dev-post".into()), None, Some(scope)).await.unwrap().1
}

/// A GitHub connection on the mock, and the token it holds.
async fn connect(env: &Env, gh: &MockGithub) -> String {
    let space = SpaceId::default_space();
    env.state.db.set_space_approvals(&space, "ask").await.unwrap();
    let token = canary_token();
    gh.require_token(&token);
    // Saving a connection tests it with `GET /user`
    gh.json("/user", user("octocat"));
    let (s, v) = call(&env.app, "POST", "/api/zombie/sources", Some(json!({"kind": "github", "label": "GitHub", "config": {"api_base": gh.base}, "secret": token}))).await;
    assert_eq!(s.as_u16(), 201, "{v}");
    gh.clear_hits();
    token
}

/// The facts of PR 24 at [`head`]: `src/app.rs` has new-side lines 10–13 in its diff.
fn facts() -> Value {
    let mut f: Value = serde_json::from_str(&std::fs::read_to_string(format!("{}/src/server/dev/fixtures/pr301.json", env!("CARGO_MANIFEST_DIR"))).unwrap()).unwrap();
    f["repo"] = json!(REPO);
    f["kind"] = json!("review");
    f["pr"] = json!(24);
    f["head_sha"] = json!(head());
    f["files"] = json!([
        {"path": "src/app.rs", "status": "modified", "additions": 2, "deletions": 1, "patch": "@@ -10,3 +10,4 @@\n ctx\n-old\n+new\n+newer\n ctx", "previous_path": null},
        {"path": "config/prod.env", "status": "added", "additions": 1, "deletions": 0, "patch": null, "previous_path": null},
    ]);
    f
}

fn finding(id: &str, source: &str, file: &str, line: u64, title: &str, detail: &str) -> Value {
    json!({"id": id, "severity": "high", "title": title, "file": file, "line": line, "detail": detail, "fix": "Do it right",
           "source": source, "state": "open", "sent_at": null, "dismissed": null})
}

fn card(findings: Vec<Value>) -> Value {
    json!({
        "kind": "review", "pr": 24, "title": "Sidebar",
        "reviewer": {"id": "kw_v", "name": "Vega", "engine": "codex", "model": "gpt"},
        "risk": {"level": "medium", "reasons": ["Touches config"]},
        "intent": {"source": "pr", "asked": "A sidebar", "rows": [{"state": "ok", "text": "Sidebar added"}], "none": true},
        "tamper": {"flags": [], "unread": []},
        "security": {"secrets": {"state": "hit", "count": 1, "missing": 0}, "packages": {"state": "pass", "count": 0, "missing": 0}, "osv": {"state": "later"}, "semgrep": {"state": "later"}},
        "findings": findings,
    })
}

/// A ready review card for PR 24 at [`head`].
async fn review_card(env: &Env, findings: Vec<Value>) -> String {
    let space = SpaceId::default_space();
    let row = env.state.db.insert_dev_card(space.as_str(), REVIEW, "pr:24", &head(), &facts().to_string()).await.unwrap().unwrap();
    env.state.db.set_dev_card(&row.id, READY, Some(&card(findings).to_string())).await.unwrap();
    row.id
}

fn review_reply(id: u64) -> Value {
    json!({"id": id, "user": user("octocat"), "body": "…", "commit_id": head(), "state": "COMMENTED",
           "html_url": format!("https://github.com/demo-org/demo-app/pull/24#pullrequestreview-{id}")})
}

fn seed(gh: &MockGithub) {
    gh.json(PULL, pull(24, "Sidebar", ("sidebar", &head()), "main", "pixel-bot", None));
    gh.json("/user", user("octocat"));
}

async fn press(env: &Env, id: &str) -> (u16, Value) {
    let (s, v) = call(&env.app, "POST", &format!("/api/dev/cards/{id}/post"), None).await;
    (s.as_u16(), v)
}

fn sent(w: &Write) -> Value {
    serde_json::from_str(&w.body).unwrap()
}

#[tokio::test]
async fn one_press_posts_one_comment_review_with_inline_and_body_findings() {
    let env = setup().await;
    let gh = MockGithub::start(&PORTS).await;
    connect(&env, &gh).await;
    seed(&gh);
    gh.accept_write("POST", REVIEWS, 200, review_reply(1));
    let id = review_card(
        &env,
        vec![finding("f1", "review", "src/app.rs", 11, "Unchecked unwrap", "It panics"), finding("f2", "review", "src/app.rs", 99, "Old helper", "Outside the diff")],
    )
    .await;
    let mut events = env.state.event_sender.subscribe();

    let (s, v) = press(&env, &id).await;
    assert_eq!(s, 200, "{v}");
    assert_eq!(v["post_error"], Value::Null, "{v}");
    assert_eq!(v["posted"]["login"], "octocat");
    assert_eq!(v["posted"]["url"], "https://github.com/demo-org/demo-app/pull/24#pullrequestreview-1");
    assert_eq!(v["posted"]["comments"], 1);
    assert_eq!(v["posted"]["forge"], "github");
    assert!(v["posted"]["at"].is_string(), "{v}");

    let writes = gh.writes();
    assert_eq!(writes.len(), 1, "{writes:?}");
    assert!(writes[0].authorized && writes[0].path == REVIEWS, "{writes:?}");
    let body = sent(&writes[0]);
    assert_eq!(body["event"], "COMMENT", "never APPROVE or REQUEST_CHANGES");
    assert_eq!(body["commit_id"], head().as_str(), "pinned to the card's head");
    assert_eq!(body["comments"].as_array().unwrap().len(), 1, "{body}");
    assert_eq!((body["comments"][0]["path"].as_str(), body["comments"][0]["line"].as_u64(), body["comments"][0]["side"].as_str()), (Some("src/app.rs"), Some(11), Some("RIGHT")));
    assert!(body["comments"][0]["body"].as_str().unwrap().contains("Unchecked unwrap"));
    let text = body["body"].as_str().unwrap();
    for want in ["Risk: Medium", "Touches config", "Done: Sidebar added", "Secrets (gitleaks): 1 possible secret", "Old helper", "`src/app.rs:99`", "Reviewed by Vega (codex) in kwant", &format!("<!-- kwant review card {id} -->")] {
        assert!(text.contains(want), "{want} missing from:\n{text}");
    }
    assert!(!text.contains("Unchecked unwrap"), "an inline finding isn't repeated in the body");

    // The desk hears about it
    let mut heard = false;
    while let Ok(e) = events.try_recv() {
        heard |= serde_json::to_string(&e).unwrap_or_default().contains(&id);
    }
    assert!(heard, "a dev_card event after the post");

    // The card keeps `posted`; the settings say who posts, and where
    let (_, v) = call(&env.app, "GET", &format!("/api/dev/cards/{id}"), None).await;
    assert_eq!(v["posted"]["comments"], 1, "{v}");
    env.state.db.put_dev_mode(SpaceId::default_space().as_str(), false, REPO, "main", None, None).await.unwrap();
    let (_, m) = call(&env.app, "GET", &format!("/api/spaces/{}/dev_mode", SpaceId::default_space().as_str()), None).await;
    assert_eq!((m["forge"].as_str(), m["forge_login"].as_str()), (Some("github"), Some("octocat")), "{m}");
    assert!(m.get("github_login").is_none(), "{m}");
}

#[tokio::test]
async fn a_422_about_lines_posts_again_with_everything_in_the_body() {
    let env = setup().await;
    let gh = MockGithub::start(&PORTS).await;
    connect(&env, &gh).await;
    seed(&gh);
    gh.accept_write("POST", REVIEWS, 422, json!({"message": "Unprocessable Entity", "errors": ["Line could not be resolved"]}));
    gh.accept_write("POST", REVIEWS, 200, review_reply(2));
    let id = review_card(&env, vec![finding("f1", "review", "src/app.rs", 12, "Unchecked unwrap", "It panics")]).await;

    let (s, v) = press(&env, &id).await;
    assert_eq!(s, 200, "{v}");
    assert_eq!(v["posted"]["comments"], 0, "{v}");
    let writes = gh.writes();
    assert_eq!(writes.len(), 2, "one retry, no more: {writes:?}");
    assert_eq!(sent(&writes[0])["comments"].as_array().map(Vec::len), Some(1));
    let retry = sent(&writes[1]);
    assert!(retry.get("comments").is_none(), "{retry}");
    assert!(retry["body"].as_str().unwrap().contains("Unchecked unwrap"), "{retry}");
    assert_eq!(retry["commit_id"], head().as_str());
}

#[tokio::test]
async fn a_422_on_the_retry_is_a_failed_post_and_try_again_works() {
    let env = setup().await;
    let gh = MockGithub::start(&PORTS).await;
    connect(&env, &gh).await;
    seed(&gh);
    gh.accept_write("POST", REVIEWS, 422, json!({"message": "Unprocessable Entity", "errors": ["Line could not be resolved"]}));
    gh.accept_write("POST", REVIEWS, 422, json!({"message": "Validation Failed", "errors": [{"message": "Line could not be resolved"}]}));
    let id = review_card(&env, vec![finding("f1", "review", "src/app.rs", 12, "Unchecked unwrap", "It panics")]).await;

    let (s, v) = press(&env, &id).await;
    assert_eq!(s, 200, "{v}");
    assert_eq!(v["posted"], Value::Null);
    assert_eq!(v["post_error"]["kind"], "failed", "{v}");
    assert!(v["post_error"]["message"].as_str().unwrap().contains("Line could not be resolved"), "{v}");
    assert_eq!(gh.writes().len(), 2);

    // Try again: it goes through and the error is cleared
    gh.accept_write("POST", REVIEWS, 200, review_reply(3));
    let (_, v) = press(&env, &id).await;
    assert_eq!(v["post_error"], Value::Null, "{v}");
    assert_eq!(v["posted"]["url"], "https://github.com/demo-org/demo-app/pull/24#pullrequestreview-3");
}

#[tokio::test]
async fn no_secret_value_is_in_anything_sent() {
    let env = setup().await;
    let gh = MockGithub::start(&PORTS).await;
    let token = connect(&env, &gh).await;
    seed(&gh);
    gh.accept_write("POST", REVIEWS, 422, json!({"message": "Unprocessable Entity", "errors": ["Line could not be resolved"]}));
    gh.accept_write("POST", REVIEWS, 200, review_reply(4));
    let key = secret_like("AKIA", 16).to_uppercase();
    let findings = vec![
        // gitleaks, on a diff line and off it: the rule and the value stay in kwant
        finding("s1", "gitleaks", "src/app.rs", 11, &format!("Possible secret (aws-access-token {key})"), &format!("aws-access-token matched {key}")),
        finding("s2", "gitleaks", "config/prod.env", 1, "Possible secret (aws-access-token)", &format!("value {key}")),
        // A model finding that quotes it: redaction masks it
        finding("f1", "review", "src/app.rs", 12, "Hard-coded key", &format!("The key {key} is in the source")),
    ];
    let id = review_card(&env, findings).await;
    let mut c: Value = serde_json::from_str(&env.state.db.dev_card(&id).await.unwrap().unwrap().card.unwrap()).unwrap();
    c["risk"]["reasons"] = json!([format!("Secret {key} found")]);
    c["tamper"]["flags"] = json!([{"kind": "skip", "file": "tests/a.rs", "line": 4, "excerpt": format!("-let k = \"{key}\";"), "why": "A test was switched off"}]);
    let before = env.state.db.dev_card(&id).await.unwrap().unwrap().card;
    assert!(env.state.db.swap_dev_card_json(&id, before.as_deref(), &c.to_string()).await.unwrap());

    let (s, v) = press(&env, &id).await;
    assert_eq!(s, 200, "{v}");
    assert!(v["posted"].is_object(), "{v}");
    let writes = gh.writes();
    assert_eq!(writes.len(), 2, "the first try and the retry both checked");
    for w in &writes {
        assert!(!w.body.contains(&key) && !w.body.contains(&key[4..]), "the AKIA value left kwant: {}", w.body);
        assert!(!w.body.contains("aws-access-token"), "the gitleaks rule left kwant: {}", w.body);
        assert!(!w.body.contains(&token), "the token is never in a body");
        assert!(w.body.contains("Possible secret in src/app.rs:11") && w.body.contains("Possible secret in config/prod.env:1"), "{}", w.body);
    }
    let first = sent(&writes[0]);
    let s1 = first["comments"].as_array().unwrap().iter().find(|c| c["line"] == 11).unwrap();
    assert_eq!(s1["body"], "Possible secret in src/app.rs:11\n\n<sub>kwant review</sub>");
}

#[tokio::test]
async fn a_moved_head_or_a_stale_card_posts_nothing() {
    let env = setup().await;
    let gh = MockGithub::start(&PORTS).await;
    connect(&env, &gh).await;
    seed(&gh);
    gh.json(PULL, pull(24, "Sidebar", ("sidebar", &"d".repeat(40)), "main", "pixel-bot", None));
    let id = review_card(&env, vec![finding("f1", "review", "src/app.rs", 11, "T", "D")]).await;

    let (s, v) = press(&env, &id).await;
    assert_eq!(s, 200, "{v}");
    assert_eq!((v["post_error"]["kind"].as_str(), v["posted"].clone()), (Some("stale"), Value::Null), "{v}");
    assert!(gh.writes().is_empty());

    // A card the poller already marked stale doesn't even ask GitHub
    gh.clear_hits();
    env.state.db.stale_dev_reviews(SpaceId::default_space().as_str(), "pr:24", None).await.unwrap();
    assert_eq!(env.state.db.dev_card(&id).await.unwrap().unwrap().status, STALE);
    let (_, v) = press(&env, &id).await;
    assert_eq!(v["post_error"]["kind"], "stale", "{v}");
    assert!(gh.hits_of(PULL).is_empty() && gh.writes().is_empty(), "{:?}", gh.hits());
}

#[tokio::test]
async fn a_token_that_cannot_write_is_read_only() {
    let env = setup().await;
    let gh = MockGithub::start(&PORTS).await;
    connect(&env, &gh).await;
    seed(&gh);
    gh.accept_write("POST", REVIEWS, 403, json!({"message": "Resource not accessible by personal access token"}));
    let id = review_card(&env, vec![]).await;

    let (s, v) = press(&env, &id).await;
    assert_eq!(s, 200, "{v}");
    assert_eq!(v["post_error"]["kind"], "read_only", "{v}");
    assert!(v["post_error"]["message"].as_str().unwrap().contains("Pull requests: Read and write"), "{v}");
    assert_eq!(v["posted"], Value::Null);

    // A 404 on the write (a classic token without `repo` on a private repo) is read only too
    gh.accept_write("POST", REVIEWS, 404, json!({"message": "Not Found"}));
    let (_, v) = press(&env, &id).await;
    assert_eq!(v["post_error"]["kind"], "read_only", "{v}");
    assert_eq!(gh.writes().len(), 2);
}

#[tokio::test]
async fn a_second_press_or_a_double_click_makes_one_review() {
    let env = setup().await;
    let gh = MockGithub::start(&PORTS).await;
    connect(&env, &gh).await;
    seed(&gh);
    gh.accept_write("POST", REVIEWS, 200, review_reply(5));
    let id = review_card(&env, vec![finding("f1", "review", "src/app.rs", 11, "T", "D")]).await;

    // A double click: both presses at once
    let (a, b) = tokio::join!(press(&env, &id), press(&env, &id));
    assert_eq!((a.0, b.0), (200, 200));
    assert_eq!(a.1["posted"], b.1["posted"], "both answer with the one review");
    assert_eq!(gh.writes().len(), 1);

    // A later press: the card comes back unchanged, and GitHub isn't asked at all
    gh.clear_hits();
    let (s, v) = press(&env, &id).await;
    assert_eq!(s, 200);
    assert_eq!(v["posted"], a.1["posted"]);
    assert!(gh.hits().is_empty(), "{:?}", gh.hits());
    assert_eq!(gh.writes().len(), 1);
}

#[tokio::test]
async fn a_review_already_on_github_is_found_not_posted_again() {
    let env = setup().await;
    let gh = MockGithub::start(&PORTS).await;
    connect(&env, &gh).await;
    seed(&gh);
    let id = review_card(&env, vec![]).await;
    // GitHub took an earlier post whose answer was lost
    let mut earlier = review_reply(6);
    earlier["body"] = json!(format!("…\n\n<!-- kwant review card {id} -->"));
    gh.json(REVIEWS, json!([earlier]));

    let (_, v) = press(&env, &id).await;
    assert_eq!(v["posted"]["url"], "https://github.com/demo-org/demo-app/pull/24#pullrequestreview-6", "{v}");
    assert!(gh.writes().is_empty());
}

#[tokio::test]
async fn only_the_person_posts_and_only_review_cards() {
    let env = setup().await;
    let gh = MockGithub::start(&PORTS).await;
    connect(&env, &gh).await;
    seed(&gh);
    let id = review_card(&env, vec![]).await;
    let uri = format!("/api/dev/cards/{id}/post");

    let token = turn_token(&env).await;
    for auth in [format!("Bearer {token}"), "Bearer not-the-person".to_string()] {
        let (s, v) = call_with(&env.app, "POST", &uri, None, &[("authorization", auth.as_str())]).await;
        assert_eq!(s.as_u16(), 403, "{v}");
    }
    assert!(call_remote(&env.app, "POST", &uri).await.is_client_error(), "a remote client without the API token is refused");
    assert!(gh.hits().is_empty() && gh.writes().is_empty(), "a refused press never reaches GitHub: {:?}", gh.hits());
    assert_eq!(env.state.db.dev_card(&id).await.unwrap().unwrap().post_error, None);

    let (s, _) = press(&env, "dvc_nope").await;
    assert_eq!(s, 404);
    let notes = env.state.db.insert_dev_card(SpaceId::default_space().as_str(), RELEASE_NOTES, "since:", "20261105T210000.000", "{}").await.unwrap().unwrap();
    let (s, v) = press(&env, &notes.id).await;
    assert_eq!(s, 400, "{v}");
}

#[tokio::test]
async fn no_connection_is_a_failed_post_and_no_forge() {
    let env = setup().await;
    let id = review_card(&env, vec![]).await;
    let (s, v) = press(&env, &id).await;
    assert_eq!(s, 200, "{v}");
    assert_eq!(v["post_error"]["kind"], "failed", "{v}");
    assert!(v["post_error"]["message"].as_str().unwrap().contains("Connect GitHub in Loose ends"), "{v}");
    let (_, m) = call(&env.app, "GET", &format!("/api/spaces/{}/dev_mode", SpaceId::default_space().as_str()), None).await;
    assert_eq!((m["forge"].clone(), m["forge_login"].clone()), (Value::Null, Value::Null), "{m}");
}
