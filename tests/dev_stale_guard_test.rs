//! The stale guard (QA round 3, Rigel's rigel_r3_gap.rs): a review whose PR merges while its text
//! is still being written is staled by the poller, and must stay stale when its `generate` call
//! finally returns. Before the guard, `set_dev_card` wrote `ready`/`failed` over `stale`, and the
//! desk showed a review of a closed PR as live until the next poll.
//!
//! Every card here is the poller's (real refs); the mock GitHub answers on Vega's ports.

mod common;
#[path = "support/mock_github.rs"]
mod mock_github;

use common::*;
use kwnt::db::dev_mode::{DevCardRow, PENDING, READY, REVIEW, STALE};
use kwnt::db::spaces::SpaceId;
use kwnt::drivers::events::RuntimeEvent;
use kwnt::server::dev::generate::generate_with_cap;
use kwnt::server::dev::poller;
use mock_github::*;
use serde_json::{json, Value};

const REPO: &str = "demo-org/demo-app";
const R: &str = "/repos/demo-org/demo-app";
const PORTS: [u16; 2] = [9493, 9494];

fn sha(n: u8) -> String {
    format!("{n:x}").repeat(40)
}

fn card(row: &DevCardRow) -> Value {
    serde_json::from_str(row.card.as_deref().unwrap_or("null")).unwrap()
}

async fn cards_written(env: &Env) {
    wait_for("the cards' text", || async { env.state.db.pending_dev_cards().await.map(|p| p.is_empty()).unwrap_or(false) }).await;
}

#[tokio::test]
async fn a_review_that_finishes_after_its_pr_merged_stays_stale() {
    let env = setup().await;
    let space = SpaceId::default_space();
    env.state.db.set_space_approvals(&space, "ask").await.unwrap();
    let kwants = env.state.db.list_kwants(&space).await.unwrap();
    let name_of = |id: &str| kwants.iter().find(|k| k.id == id).unwrap().name.clone();
    let a_name = format!("{} (kwant bot)", name_of(&env.a));
    let person = (a_name.as_str(), "someone@example.com");
    let token = canary_token();
    let gh = MockGithub::start(&PORTS).await;
    gh.require_token(&token);

    let patch = "@@ -1,2 +1,2 @@\n-let retries = 1;\n+let retries = 3;";
    let green = check_runs(&[("rust", "completed", Some("success"))]);
    let direct = commit(&sha(1), "Fix the README", person, &[], &hours_ago(3));
    gh.json("/user", user("demo-person"))
        .json(R, repo(REPO, "main"))
        .json(&format!("{R}/branches/main"), json!({"name": "main"}))
        .json(&format!("{R}/contents"), json!([{"name": "Cargo.toml", "type": "file"}]))
        .json(&format!("{R}/tags"), json!([]))
        .json(&format!("{R}/commits"), json!([direct]))
        .json(&format!("{R}/commits/{}", sha(1)), {
            let mut c = direct.clone();
            c["files"] = json!([file("README.md", "modified", "-old\n+new")]);
            c
        })
        .json(&format!("{R}/commits/{}/pulls", sha(1)), json!([]))
        .json(&format!("{R}/commits/{}/check-runs", sha(1)), green.clone());
    let head = sha(9);
    gh.json(&format!("{R}/pulls/5/files"), json!([file("src/f5.rs", "modified", patch)]))
        .json(&format!("{R}/commits/{head}"), commit(&head, "Retry the upload", person, &[&sha(1)], &hours_ago(2)))
        .json(&format!("{R}/commits/{head}/check-runs"), green.clone())
        .json(&format!("{R}/pulls"), json!([pull(5, "Retry the upload", ("topic", &head), "main", "demo-person", None)]));

    let (s, v) = call(&env.app, "POST", "/api/zombie/sources", Some(json!({"kind": "github", "label": "GitHub", "config": {"api_base": gh.base}, "secret": token}))).await;
    assert_eq!(s.as_u16(), 201, "{v}");
    let (s, v) = call(&env.app, "PUT", &format!("/api/spaces/{}/dev_mode", space.as_str()), Some(json!({"on": true, "repo": REPO, "branch": "main"}))).await;
    assert_eq!(s.as_u16(), 200, "{v}");

    // Poll 1: the review card for PR #5, under the poller's own ref
    let mode = env.state.db.dev_repos(space.as_str()).await.unwrap().remove(0);
    poller::poll(&env.state, &mode).await.expect("a clean poll");
    cards_written(&env).await;
    let reviews = env.state.db.dev_cards(space.as_str(), REVIEW, None, 50).await.unwrap();
    assert_eq!(reviews.len(), 1);
    let rid = reviews[0].id.clone();
    assert_eq!(reviews[0].ref_, "pr:5");
    // Its text is still being written (a slow engine) when the PR merges
    let writing = json!({"kind": "review", "pr": 5});
    let row = env.state.db.set_dev_card(&rid, PENDING, Some(&writing.to_string())).await.unwrap();
    assert_eq!(row.map(|r| r.status), Some(PENDING.to_string()), "a live card is still written");

    // Poll 2: PR #5 merges, and the poller stales its review
    let merge = sha(2);
    let c = commit(&merge, "Retry the upload (#5)", person, &[&sha(1)], &hours_ago(1));
    let mut full = c.clone();
    full["files"] = json!([file("src/f5.rs", "modified", patch)]);
    gh.json(&format!("{R}/commits/{merge}"), full)
        .json(&format!("{R}/commits/{merge}/pulls"), json!([pull(5, "Retry the upload", ("topic", &head), "main", "demo-person", Some((&merge, &hours_ago(1))))]))
        .json(&format!("{R}/commits/{merge}/check-runs"), green.clone())
        .json(&format!("{R}/commits"), json!([c, direct.clone()]))
        .json(&format!("{R}/pulls"), json!([]));
    let mode = env.state.db.dev_repos(space.as_str()).await.unwrap().remove(0);
    poller::poll(&env.state, &mode).await.expect("a clean poll");
    cards_written(&env).await;
    let before = env.state.db.dev_card(&rid).await.unwrap().unwrap();
    assert_eq!(before.status, STALE);

    // The review's generate() now returns (cap 0: it writes a failed card; a real engine, ready)
    let mut events = env.state.event_sender.subscribe();
    generate_with_cap(env.state.clone(), rid.clone(), 0).await;
    let after = env.state.db.dev_card(&rid).await.unwrap().unwrap();
    assert_eq!(after.status, STALE, "a staled review must not come back to life");
    assert_eq!(card(&after), writing, "nor get new text");
    assert_eq!(after.updated_at, before.updated_at);
    // And nothing tells the desk it changed
    while let Ok(e) = events.try_recv() {
        if let RuntimeEvent::DevCard { id, status, .. } = e {
            assert_ne!(id, rid, "no dev_card event for the stale review (status {status})");
        }
    }

    // The guard only holds a stale card: writing it directly is refused too, ready included
    assert!(env.state.db.set_dev_card(&rid, READY, Some("{}")).await.unwrap().is_none());
    assert_eq!(env.state.db.dev_card(&rid).await.unwrap().unwrap().status, STALE);
    assert!(gh.hits().iter().all(|h| h.method == "GET"));
}
