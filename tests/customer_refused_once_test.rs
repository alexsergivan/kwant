//! 19e: a call kwnt's tool endpoint refused in a customer's turn leaves one closed `refused` row
//! per (turn, bot, tool, arguments), however often the model retries it. The refusal itself and
//! the end-to-end trace are tested in telegram_customers_test.rs
//! (`kwants_tools_refuse_a_customer_turns_call_nobody_allowed`).

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
    let first = db.save_customer_refused(&refused(&env, &env.a, "turn-1", "delivery times")).await.unwrap();
    for _ in 0..2 {
        let again = db.save_customer_refused(&refused(&env, &env.a, "turn-1", "delivery times")).await.unwrap();
        assert_eq!(again, first, "a retry returns the first row");
    }
    let list = rows(&env).await;
    assert_eq!(list.len(), 1, "3 identical refusals, 1 row: {list:?}");
    assert_eq!(list[0].id, first);
    assert_eq!(list[0].status, CUSTOMER_REFUSED);

    // The same tool with a changed argument in the same turn: a 2nd row
    let other = db.save_customer_refused(&refused(&env, &env.a, "turn-1", "return policy")).await.unwrap();
    assert_ne!(other, first);
    assert_eq!(rows(&env).await.len(), 2, "a changed argument is its own row");
    // ... and its own retry doesn't add a 3rd
    assert_eq!(db.save_customer_refused(&refused(&env, &env.a, "turn-1", "return policy")).await.unwrap(), other);
    assert_eq!(rows(&env).await.len(), 2);

    // The same call in the next turn: a new row
    let next = db.save_customer_refused(&refused(&env, &env.a, "turn-2", "delivery times")).await.unwrap();
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
    let mine = db.save_customer_refused(&same).await.unwrap();
    assert_eq!(rows(&env).await.len(), 1, "the denied row doesn't stand in for the refused one");
    assert_eq!(db.get_approval(&mine).await.unwrap().unwrap().status, CUSTOMER_REFUSED);

    // Another bot's refusal of the same call under the same turn id is its own row
    let theirs = db
        .save_customer_refused(&NewApproval { kwant_id: env.b.clone(), ..refused(&env, &env.a, "turn-9", "delivery times") })
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
        async move { db.save_customer_refused(&one).await.unwrap() }
    }))
    .await;
    let list = rows(&env).await;
    assert_eq!(list.len(), 1, "8 refusals at once, 1 row: {list:?}");
    assert!(ids.iter().all(|id| *id == list[0].id), "each gets the one row's id: {ids:?}");
    let details: Value = serde_json::from_str(&list[0].parameters).unwrap();
    assert_eq!(details["arguments"]["query"], "opening hours");
}
