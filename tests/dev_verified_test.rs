//! Verified end to end (QA V-X1, V-V1): the poller files a PR's review under its own ref, the PR
//! merges (which stales the review), and the landed card takes its reviewer from the review of the
//! commit that merged, compared by bot id. No hand-made refs: every card here is the poller's.
//!
//! The fake engine can't write a review, so each review card is set as a reviewer would leave it
//! (ready, with the reviewer's bot id and no open finding) before the merge.

mod common;
#[path = "support/mock_github.rs"]
mod mock_github;

use common::*;
use kwnt::db::dev_mode::{DevCardRow, LANDED, READY, REVIEW, STALE};
use kwnt::db::spaces::SpaceId;
use kwnt::server::dev::poller;
use mock_github::*;
use serde_json::{json, Value};

const REPO: &str = "demo-org/demo-app";
const R: &str = "/repos/demo-org/demo-app";
const PORTS: [u16; 2] = [9491, 9492];

fn sha(n: u8) -> String {
    format!("{n:x}").repeat(40)
}

fn card(row: &DevCardRow) -> Value {
    serde_json::from_str(row.card.as_deref().unwrap_or("null")).unwrap()
}

/// The landed card of the merge commit `merge`.
async fn landed(env: &Env, merge: &str) -> Value {
    let rows = env.state.db.dev_cards_of_ref(SpaceId::default_space().as_str(), LANDED, merge).await.unwrap();
    assert_eq!(rows.len(), 1, "a landed card for {merge}");
    card(&rows[0])
}

async fn cards_written(env: &Env) {
    wait_for("the cards' text", || async { env.state.db.pending_dev_cards().await.map(|p| p.is_empty()).unwrap_or(false) }).await;
}

#[tokio::test]
async fn a_merged_pr_takes_its_reviewer_from_the_review_of_the_merged_head() {
    let env = setup().await;
    let space = SpaceId::default_space();
    env.state.db.set_space_approvals(&space, "ask").await.unwrap();
    let kwants = env.state.db.list_kwants(&space).await.unwrap();
    let name_of = |id: &str| kwants.iter().find(|k| k.id == id).unwrap().name.clone();
    // Every commit is by bot A; bot B reviews
    let a_name = format!("{} (kwant bot)", name_of(&env.a));
    let person = (a_name.as_str(), "someone@example.com");
    let token = canary_token();
    let gh = MockGithub::start(&PORTS).await;
    gh.require_token(&token);

    let patch = "@@ -1,2 +1,2 @@\n-let retries = 1;\n+let retries = 3;";
    let green = check_runs(&[("rust", "completed", Some("success"))]);
    let direct = commit(&sha(1), "Fix the README", person, &[], "2026-11-05T01:00:00Z");
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
    // Three open PRs: #5 at head 9, #6 at head 7, #7 at head 6
    let open = [(5, "Retry the upload", sha(9)), (6, "Add a cache", sha(7)), (7, "Tidy the logs", sha(6))];
    for (n, title, head) in &open {
        gh.json(&format!("{R}/pulls/{n}/files"), json!([file(&format!("src/f{n}.rs"), "modified", patch)]))
            .json(&format!("{R}/commits/{head}"), commit(head, title, person, &[&sha(1)], "2026-11-05T02:00:00Z"))
            .json(&format!("{R}/commits/{head}/check-runs"), green.clone());
    }
    gh.json(&format!("{R}/pulls"), Value::Array(open.iter().map(|(n, t, h)| pull(*n, t, ("topic", h), "main", "demo-person", None)).collect()));

    let (s, v) = call(&env.app, "POST", "/api/zombie/sources", Some(json!({"kind": "github", "label": "GitHub", "config": {"api_base": gh.base}, "secret": token}))).await;
    assert_eq!(s.as_u16(), 201, "{v}");
    let (s, v) = call(&env.app, "PUT", &format!("/api/spaces/{}/dev_mode", space.as_str()), Some(json!({"on": true, "repo": REPO, "branch": "main"}))).await;
    assert_eq!(s.as_u16(), 200, "{v}");

    // Poll 1: a review card for each open PR, under the poller's own ref
    let mode = env.state.db.dev_repos(space.as_str()).await.unwrap().remove(0);
    poller::poll(&env.state, &mode).await.expect("a clean poll");
    cards_written(&env).await;
    let reviews = env.state.db.dev_cards(space.as_str(), REVIEW, None, 50).await.unwrap();
    assert_eq!(reviews.len(), 3);
    // #5 and #6 reviewed by B, #7 by its own author A; nothing open
    for r in &reviews {
        let pr = serde_json::from_str::<Value>(&r.facts).unwrap()["pr"].as_u64().unwrap();
        let by = if pr == 7 { &env.a } else { &env.b };
        let review = json!({"kind": "review", "pr": pr, "reviewer": {"id": by, "name": name_of(by), "engine": "claude", "model": "auto"}, "findings": []});
        env.state.db.set_dev_card(&r.id, READY, Some(&review.to_string())).await.unwrap();
    }

    // Before the merge, #6 got a new head (8) the poller never saw open. All three merge.
    let merges = [(5, sha(2), sha(9)), (6, sha(4), sha(8)), (7, sha(5), sha(6))];
    let mut list = vec![direct.clone()];
    for (n, merge, head) in &merges {
        let title = open.iter().find(|o| o.0 == *n).unwrap().1;
        let c = commit(merge, &format!("{title} (#{n})"), person, &[&sha(1)], "2026-11-05T03:00:00Z");
        let mut full = c.clone();
        full["files"] = json!([file(&format!("src/f{n}.rs"), "modified", patch)]);
        gh.json(&format!("{R}/commits/{merge}"), full)
            .json(&format!("{R}/commits/{merge}/pulls"), json!([pull(*n, title, ("topic", head), "main", "demo-person", Some((merge, "2026-11-05T03:00:00Z")))]))
            .json(&format!("{R}/commits/{merge}/check-runs"), green.clone());
        list.insert(0, c);
    }
    gh.json(&format!("{R}/commits"), Value::Array(list)).json(&format!("{R}/pulls"), json!([]));

    // Poll 2: three landed cards; the merge stales every review
    let mode = env.state.db.dev_repos(space.as_str()).await.unwrap().remove(0);
    poller::poll(&env.state, &mode).await.expect("a clean poll");
    cards_written(&env).await;
    let reviews = env.state.db.dev_cards(space.as_str(), REVIEW, None, 50).await.unwrap();
    assert!(reviews.iter().all(|r| r.status == STALE), "{:?}", reviews.iter().map(|r| &r.status).collect::<Vec<_>>());

    // #5: B reviewed the head that merged, CI green, nothing open
    let c = landed(&env, &sha(2)).await;
    assert_eq!(c["reviewed_by"], json!(name_of(&env.b)), "{c}");
    assert_eq!(c["verified"], true, "{c}");
    // #6: the review was of an older head
    let c = landed(&env, &sha(4)).await;
    assert_eq!((c["reviewed_by"].clone(), c["verified"].clone()), (Value::Null, json!(false)), "{c}");
    // #7: reviewed by its own author's bot
    let c = landed(&env, &sha(5)).await;
    assert_eq!(c["verified"], false, "{c}");
    assert!(gh.hits().iter().all(|h| h.method == "GET"));
}
