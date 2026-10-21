//! The Loose ends owner: a due letter deadline or a review starts one turn for the space's
//! owner bot (the #general lead unless one is picked) that prepares drafts and next steps.
//! Nothing is sent, the case stays as it was, and a daily cap bounds the turns.

mod common;

use chrono::Utc;
use common::*;
use kwnt::db::zombie::CaseRow;
use kwnt::server::loose_ends_owner::{on_event, Event};
use serde_json::json;
use std::time::Duration;

fn letter(id: &str, space: Option<&str>) -> CaseRow {
    let now = Utc::now();
    CaseRow {
        id: id.into(),
        dedupe_key: format!("document:d:{id}"),
        source: "document".into(),
        source_id: "documents".into(),
        source_ref: id.into(),
        kind: "document_request".into(),
        conv_kind: "document".into(),
        title: format!("Letter from Northfield Water {id}"),
        counterpart: None,
        counterpart_name: Some("Northfield Water".into()),
        reference: Some("NW-448812".into()),
        link: None,
        why: "Asks to pay an overdue bill".into(),
        evidence: "[]".into(),
        draft: None,
        status: "open".into(),
        snoozed_until: None,
        waiting_since: None,
        follow_ups: 0,
        first_seen: now,
        last_seen: now,
        last_activity_at: now,
        status_changed_at: now,
        resolution: None,
        dismiss_reason: None,
        deadline: Some(now.date_naive().to_string()),
        reply_to_message_id: None,
        space_id: space.map(Into::into),
        redacted: false,
        kwant_id: None,
    }
}

async fn owner_of(env: &Env) -> String {
    let (s, v) = call(&env.app, "GET", "/api/zombie/owner", None).await;
    assert!(s.is_success(), "{v}");
    v["owner"]["id"].as_str().expect("the space lead owns it by default").to_string()
}

#[tokio::test]
async fn a_due_deadline_starts_the_owners_turn_and_sends_nothing() {
    let env = setup().await;
    let owner = owner_of(&env).await;
    let c = letter("due1", None);
    env.state.db.zombie_put_case(&c).await.unwrap();
    let now = Utc::now();
    env.state.db.document_set_reminders("due1", &[(0, now)]).await.unwrap();

    let fired = kwnt::documents::reminders::run_once(&env.state, now).await;
    assert_eq!(fired.total(), 1);
    wait_for("the owner's turn", || async { !invocations_with("id: due1").is_empty() }).await;
    let prompt = invocations_with("id: due1").pop().unwrap();
    assert!(prompt.contains("a letter deadline that is today"), "{prompt}");
    assert!(prompt.contains("don't send, submit, pay"), "{prompt}");
    assert!(prompt.contains("reference: NW-448812"));

    // In the owner's own chat, with a note saying nothing goes out without the person
    let msgs = env.state.db.list_messages(&owner, 20).await.unwrap();
    assert!(msgs.iter().any(|m| m.sender_role == "system" && m.text.contains("Nothing is sent without you")), "{msgs:?}");
    tokio::time::sleep(Duration::from_millis(500)).await;
    // Nothing sent: no approval card, the case is still open and untouched
    let cards = env.state.db.list_approvals(&kwnt::db::spaces::SpaceId::default_space(), None, None).await.unwrap();
    assert!(cards.is_empty(), "{cards:?}");
    let after = env.state.db.zombie_case("due1").await.unwrap().unwrap();
    assert_eq!((after.status.as_str(), after.follow_ups, after.draft.is_none()), ("open", 0, true));
    let (_, v) = call(&env.app, "GET", "/api/zombie/owner", None).await;
    assert_eq!(v["used_today"], 1);
}

#[tokio::test]
async fn a_picked_owner_gets_the_review_and_the_daily_cap_holds() {
    let env = setup().await;
    let (s, v) = call(&env.app, "PUT", "/api/zombie/owner", Some(json!({"enabled": true, "kwant_id": env.b, "daily_cap": 1}))).await;
    assert!(s.is_success(), "{v}");
    assert_eq!(v["owner"]["id"], env.b.as_str());

    let started = on_event(&env.state, Event::Review, vec![letter("rev1", None), letter("rev2", None)]).await;
    assert_eq!(started, vec![env.b.clone()], "one turn for both cases");
    wait_for("the review turn", || async { !invocations_with("id: rev2").is_empty() }).await;
    let prompt = invocations_with("id: rev1").pop().unwrap();
    assert!(prompt.contains("a Loose ends review that picked 2 case(s)") && prompt.contains("id: rev2"), "{prompt}");

    // Over the cap: the person is still notified elsewhere, but no second turn
    assert!(on_event(&env.state, Event::Review, vec![letter("rev3", None)]).await.is_empty());
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert!(invocations_with("id: rev3").is_empty());
}

#[tokio::test]
async fn switched_off_or_a_bad_owner_starts_nothing() {
    let env = setup().await;
    let (s, _) = call(&env.app, "PUT", "/api/zombie/owner", Some(json!({"kwant_id": "no-such-bot"}))).await;
    assert_eq!(s.as_u16(), 400, "the owner must be a bot in the space");
    let (s, _) = call(&env.app, "PUT", "/api/zombie/owner", Some(json!({"daily_cap": 99}))).await;
    assert_eq!(s.as_u16(), 400);
    let (s, v) = call(&env.app, "PUT", "/api/zombie/owner", Some(json!({"enabled": false}))).await;
    assert!(s.is_success(), "{v}");
    assert!(on_event(&env.state, Event::Deadline(1), vec![letter("off1", None)]).await.is_empty());
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert!(invocations_with("id: off1").is_empty());
}
