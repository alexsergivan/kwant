//! `GET /api/dev/cards/:id` (impl/lin.md): the desk reads one card by id, chiefly a release
//! notes draft once its `dev_card` event says it is ready, since no list returns those.

mod common;

use common::*;
use kwnt::db::dev_mode::{PENDING, READY, RELEASE_NOTES};
use kwnt::db::spaces::SpaceId;
use kwnt::server::continuity::{SelfScope, TurnOrigin};
use kwnt::server::mcp_runtime;
use serde_json::{json, Value};

/// A turn token of bot `a`, as its CLI holds it.
async fn turn_token(env: &Env) -> String {
    let bot = env.state.db.get_kwant(&env.a).await.unwrap().unwrap();
    let scope = SelfScope {
        kwant_id: bot.id.clone(),
        kwant_name: bot.name.clone(),
        origin: Some(TurnOrigin { direct_kwant_id: Some(bot.id.clone()), ..Default::default() }),
        cancel: Some(tokio_util::sync::CancellationToken::new()),
        turn_id: Some("turn-dev-card".into()),
        sandbox_target: Some(bot.id.clone()),
        ..Default::default()
    };
    mcp_runtime::prepare_turn_with_self(&env.state, &bot, Some("turn-dev-card".into()), None, Some(scope)).await.unwrap().1
}

#[tokio::test]
async fn a_release_notes_draft_is_read_by_id() {
    let env = setup().await;
    let space = SpaceId::default_space();
    let facts = json!({"repo": "demo-org/demo-app", "since_tag": "v1.0.0", "cards": []}).to_string();
    let row = env.state.db.insert_dev_card(space.as_str(), RELEASE_NOTES, "since:v1.0.0", "20261105T030000.000", &facts).await.unwrap().unwrap();
    let uri = format!("/api/dev/cards/{}", row.id);

    // Pending: the row's fields only, no facts leak into the card
    let (s, v) = call(&env.app, "GET", &uri, None).await;
    assert_eq!(s.as_u16(), 200, "{v}");
    assert_eq!((v["id"].as_str(), v["kind"].as_str(), v["status"].as_str()), (Some(row.id.as_str()), Some(RELEASE_NOTES), Some(PENDING)));
    assert!(v["created_at"].is_string() && v.get("markdown").is_none() && v.get("since_tag").is_none() && v.get("cards").is_none(), "{v}");
    // 0.7.2: every card names its repo (key, label, team); a 0.7.1 card made with no repo has key ""
    assert_eq!((v["repo"].clone(), v["repo_name"].clone(), v["team"].clone()), (json!(""), json!("demo-org/demo-app"), Value::Null), "{v}");

    // Ready: the draft's text, with the row's id, kind, status and created_at winning
    let card = json!({"since_tag": "v1.0.0", "markdown": "## What's new\n- Faster sign-in", "whats_new": ["Faster sign-in"], "id": "spoof", "status": "spoof"});
    env.state.db.set_dev_card(&row.id, READY, Some(&card.to_string())).await.unwrap();
    let (s, v) = call(&env.app, "GET", &uri, None).await;
    assert_eq!(s.as_u16(), 200, "{v}");
    assert_eq!(v["markdown"], "## What's new\n- Faster sign-in");
    assert_eq!(v["whats_new"], json!(["Faster sign-in"]));
    assert_eq!((v["id"].as_str(), v["status"].as_str(), v["kind"].as_str()), (Some(row.id.as_str()), Some(READY), Some(RELEASE_NOTES)));
}

#[tokio::test]
async fn an_unknown_card_is_a_404() {
    let env = setup().await;
    let (s, v) = call(&env.app, "GET", "/api/dev/cards/dvc_nope", None).await;
    assert_eq!(s.as_u16(), 404, "{v}");
    assert_eq!(v["error"], "no such card");
}

#[tokio::test]
async fn a_bot_cannot_read_a_card() {
    let env = setup().await;
    let space = SpaceId::default_space();
    let row = env.state.db.insert_dev_card(space.as_str(), RELEASE_NOTES, "since:", "20261105T030001.000", "{}").await.unwrap().unwrap();
    env.state.db.set_dev_card(&row.id, READY, Some(&json!({"markdown": "secret diff summary"}).to_string())).await.unwrap();
    let uri = format!("/api/dev/cards/{}", row.id);

    let token = turn_token(&env).await;
    for auth in [format!("Bearer {token}"), "Bearer not-the-person".to_string()] {
        let (s, v) = call_with(&env.app, "GET", &uri, None, &[("authorization", auth.as_str())]).await;
        assert_eq!(s.as_u16(), 403, "{v}");
        assert!(!v.to_string().contains("secret diff summary"), "{v}");
    }
    let (s, _) = call(&env.app, "GET", &uri, None).await;
    assert_eq!(s.as_u16(), 200, "the person still reads it");
    assert!(call_remote(&env.app, "GET", &uri).await.is_client_error(), "a remote client without the API token is refused");
}
