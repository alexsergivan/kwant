//! 19e: a call kwnt's tool endpoint refused in a customer's turn leaves one closed `refused` row
//! per (turn, bot, tool, arguments), however often the model retries it. The refusal itself and
//! the end-to-end trace are tested in telegram_customers_test.rs
//! (`kwants_tools_refuse_a_customer_turns_call_nobody_allowed`).
//!
//! 22a: the row names the chat the call happened in (`resume`, a card's shape), and
//! `GET /api/approvals` returns it as `link`. The chat is not part of "the same call".

mod common;

use common::{call, setup, Env};
use kwnt::db::approval_modes::CUSTOMER_REFUSED;
use kwnt::db::models::{Approval, NewApproval};
use kwnt::db::spaces::SpaceId;
use serde_json::{json, Value};

/// The details `record_customer_refusal` stores for a refused `memory_search`.
fn refused(env: &Env, kwant: &str, turn: &str, query: &str) -> NewApproval {
    NewApproval {
        turn_id: turn.into(),
        kwant_id: kwant.into(),
        tool_name: "mcp__kwnt_tools__memory_search".into(),
        parameters: json!({
            "source": "mcp",
            "server": "kwnt_tools",
            "tool": "memory_search",
            "arguments": {"query": query},
            "kwant_id": kwant,
            "kwant_name": if kwant == env.a { "A" } else { "B" },
            "customer_chat": true,
            "refused": "nobody_allowed",
        }),
    }
}

async fn rows(env: &Env) -> Vec<Approval> {
    env.state.db.list_approvals(&SpaceId::default_space(), Some(CUSTOMER_REFUSED), None).await.unwrap()
}

#[tokio::test]
async fn a_retried_refused_call_leaves_one_row_and_other_arguments_a_second() {
    let env = setup().await;
    let db = &env.state.db;

    // The same call refused 3 times in one turn: 1 row, and every retry gets its id back
    let first = db.save_customer_refused(&refused(&env, &env.a, "turn-1", "delivery times"), None).await.unwrap();
    for _ in 0..2 {
        let again = db.save_customer_refused(&refused(&env, &env.a, "turn-1", "delivery times"), None).await.unwrap();
        assert_eq!(again, first, "a retry returns the first row");
    }
    let list = rows(&env).await;
    assert_eq!(list.len(), 1, "3 identical refusals, 1 row: {list:?}");
    assert_eq!(list[0].id, first);
    assert_eq!(list[0].status, CUSTOMER_REFUSED);

    // The same tool with a changed argument in the same turn: a 2nd row
    let other = db.save_customer_refused(&refused(&env, &env.a, "turn-1", "return policy"), None).await.unwrap();
    assert_ne!(other, first);
    assert_eq!(rows(&env).await.len(), 2, "a changed argument is its own row");
    // ... and its own retry doesn't add a 3rd
    assert_eq!(db.save_customer_refused(&refused(&env, &env.a, "turn-1", "return policy"), None).await.unwrap(), other);
    assert_eq!(rows(&env).await.len(), 2);

    // The same call in the next turn: a new row
    let next = db.save_customer_refused(&refused(&env, &env.a, "turn-2", "delivery times"), None).await.unwrap();
    assert!(next != first && next != other);
    assert_eq!(rows(&env).await.len(), 3, "a later turn is its own row");

    // The list route shows the three, each once
    let (s, v) = call(&env.app, "GET", &format!("/api/approvals?status={CUSTOMER_REFUSED}&kwant_id={}", env.a), None).await;
    assert_eq!(s.as_u16(), 200, "{v}");
    let mut ids: Vec<&str> = v.as_array().unwrap().iter().map(|r| r["id"].as_str().unwrap()).collect();
    ids.sort();
    let mut want = vec![first.as_str(), other.as_str(), next.as_str()];
    want.sort();
    assert_eq!(ids, want, "{v}");
}

#[tokio::test]
async fn only_a_refused_row_of_the_same_bot_counts_as_the_same_call() {
    let env = setup().await;
    let db = &env.state.db;

    // A `denied` row (kwnt's own secret-file refusal) with the very same details is not a refused row
    let same = refused(&env, &env.a, "turn-9", "delivery times");
    db.save_refused(&same).await.unwrap();
    let mine = db.save_customer_refused(&same, None).await.unwrap();
    assert_eq!(rows(&env).await.len(), 1, "the denied row doesn't stand in for the refused one");
    assert_eq!(db.get_approval(&mine).await.unwrap().unwrap().status, CUSTOMER_REFUSED);

    // Another bot's refusal of the same call under the same turn id is its own row
    let theirs = db
        .save_customer_refused(&NewApproval { kwant_id: env.b.clone(), ..refused(&env, &env.a, "turn-9", "delivery times") }, None)
        .await
        .unwrap();
    assert_ne!(theirs, mine);
    assert_eq!(rows(&env).await.len(), 2);
}

#[tokio::test]
async fn refusals_of_the_same_call_at_once_still_leave_one_row() {
    let env = setup().await;
    let db = env.state.db.clone();
    let same = refused(&env, &env.a, "turn-5", "opening hours");
    let ids = futures::future::join_all((0..8).map(|_| {
        let db = db.clone();
        let one = same.clone();
        async move { db.save_customer_refused(&one, None).await.unwrap() }
    }))
    .await;
    let list = rows(&env).await;
    assert_eq!(list.len(), 1, "8 refusals at once, 1 row: {list:?}");
    assert!(ids.iter().all(|id| *id == list[0].id), "each gets the one row's id: {ids:?}");
    let details: Value = serde_json::from_str(&list[0].parameters).unwrap();
    assert_eq!(details["arguments"]["query"], "opening hours");
}

/// The `GET /api/approvals?status=refused` rows of bot `kwant`.
async fn listed(env: &Env, kwant: &str) -> Vec<Value> {
    let (s, v) = call(&env.app, "GET", &format!("/api/approvals?status={CUSTOMER_REFUSED}&kwant_id={kwant}"), None).await;
    assert_eq!(s.as_u16(), 200, "{v}");
    v.as_array().unwrap().clone()
}

#[tokio::test]
async fn a_refused_call_names_its_chat_and_the_list_links_it() {
    let env = setup().await;
    let db = &env.state.db;

    // A customer thread: stored in a card's shape, listed as a card's link
    let thread = json!({"thread_id": "th-ann"});
    let id = db.save_customer_refused(&refused(&env, &env.a, "turn-1", "delivery times"), Some(&thread)).await.unwrap();
    let row = db.get_approval(&id).await.unwrap().unwrap();
    assert_eq!(serde_json::from_str::<Value>(row.resume.as_deref().unwrap()).unwrap(), thread);
    assert_eq!(db.approval_chat(&id).await.unwrap(), Some(thread.clone()), "the same column a card's chat is read from");
    let rows = listed(&env, &env.a).await;
    assert_eq!(rows.len(), 1, "{rows:?}");
    assert_eq!(rows[0]["link"], json!({"thread_id": "th-ann", "channel_id": null, "direct_kwant_id": null}));
    assert_eq!(rows[0]["status"], CUSTOMER_REFUSED);

    // A channel and a direct chat keep their own kind
    let ch = db.save_customer_refused(&refused(&env, &env.b, "turn-2", "x"), Some(&json!({"channel_id": "c1"}))).await.unwrap();
    let dm = db.save_customer_refused(&refused(&env, &env.b, "turn-3", "x"), Some(&json!({"direct_kwant_id": env.b}))).await.unwrap();
    let rows = listed(&env, &env.b).await;
    let link = |id: &str| rows.iter().find(|r| r["id"] == id).unwrap()["link"].clone();
    assert_eq!(link(&ch), json!({"thread_id": null, "channel_id": "c1", "direct_kwant_id": null}));
    assert_eq!(link(&dm), json!({"thread_id": null, "channel_id": null, "direct_kwant_id": env.b}));

    // It stays closed: nothing to answer, nothing waiting
    let (s, v) = call(&env.app, "POST", &format!("/api/approvals/{id}/resolve"), Some(json!({"decision": "allow_once"}))).await;
    assert_eq!(s.as_u16(), 409, "{v}");
    assert_eq!(db.get_approval(&id).await.unwrap().unwrap().status, CUSTOMER_REFUSED);
    assert!(!db.turn_has_waiting_approval("turn-1").await.unwrap());
}

#[tokio::test]
async fn a_retry_adds_no_row_and_keeps_the_first_chat() {
    let env = setup().await;
    let db = &env.state.db;
    let first = db
        .save_customer_refused(&refused(&env, &env.a, "turn-1", "delivery times"), Some(&json!({"thread_id": "th-1"})))
        .await
        .unwrap();
    // The same call again in the same turn, even with another (or no) chat: the same row, unchanged
    for chat in [Some(json!({"thread_id": "th-1"})), Some(json!({"channel_id": "c9"})), None] {
        let again = db.save_customer_refused(&refused(&env, &env.a, "turn-1", "delivery times"), chat.as_ref()).await.unwrap();
        assert_eq!(again, first, "19e: the chat is not part of the same call");
    }
    let rows = listed(&env, &env.a).await;
    assert_eq!(rows.len(), 1, "{rows:?}");
    assert_eq!(rows[0]["link"]["thread_id"], "th-1", "the first row's chat stays: {rows:?}");
}

#[tokio::test]
async fn a_row_stored_without_a_chat_still_lists() {
    let env = setup().await;
    let db = &env.state.db;
    // As before 22a: no chat stored
    let old = db.save_customer_refused(&refused(&env, &env.a, "turn-1", "delivery times"), None).await.unwrap();
    assert_eq!(db.get_approval(&old).await.unwrap().unwrap().resume, None);
    let rows = listed(&env, &env.a).await;
    assert_eq!(rows.len(), 1, "{rows:?}");
    assert_eq!(rows[0]["id"], old.as_str());
    assert_eq!(rows[0]["link"], Value::Null, "no chat, no link: {rows:?}");
    assert_eq!(rows[0]["details"]["customer_chat"], true);
}
