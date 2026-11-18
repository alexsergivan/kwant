//! 28m home-server-known-first: on the Claude CLI engine the CLI connects to a bot's MCP servers,
//! not kwnt, so a Home Assistant server added without a test is one kwnt never heard from. It
//! still counts for the delivery check when its address is Home Assistant's MCP path, so the
//! first false "Unlocking … now." with no tool call gets the continuation. What a server said it
//! is wins over its address. Full turns on the fake claude (tests/common).

mod common;

use common::*;
use kwnt::{
    db::models::McpServer,
    mcp::home_floor,
    server::mcp_runtime::home_server_names,
};
use serde_json::json;
use std::{collections::BTreeMap, time::Duration};

/// A closed port: nothing answers there, so kwnt never hears from the server.
async fn server(env: &Env, name: &str, url: &str) -> McpServer {
    let space = env.state.db.space_of_kwant(&env.a).await.unwrap().unwrap();
    let server = McpServer {
        id: String::new(),
        name: name.into(),
        transport: "http".into(),
        command: None,
        args: vec![],
        url: Some(url.into()),
        env: BTreeMap::new(),
        headers: BTreeMap::new(),
        enabled: true,
        timeout_secs: 5,
        created_at: None,
    };
    let server = env.state.db.create_mcp_server(&space, &server).await.unwrap();
    env.state.db.set_kwant_mcp_servers(&env.a, std::slice::from_ref(&server.id)).await.unwrap();
    server
}

/// The continuation kwnt sends after `claim` (see `delivery_check::home_change_prompt`).
fn continued(claim: &str) -> String {
    format!("Your reply says \"{claim}\", but no Home Assistant step changed anything in this turn")
}

/// Asks bot A (fake claude) something whose first answer is `claim`, and waits for that reply.
async fn claim_once(env: &Env, tag: &str, claim: &str) {
    plan_replies(tag, &[claim, "Nothing changed: it stays locked."]);
    send_direct(env, &env.a, &format!("unlock it REPLYSEQ:{tag}"), json!({})).await;
    wait_reply(env, &env.a, claim).await;
}

#[tokio::test]
async fn a_never_tested_home_assistant_server_counts_from_the_first_claim() {
    let env = setup().await;
    let ha = server(&env, "house", "http://127.0.0.1:9/api/mcp").await;
    // kwnt never heard from it: no test, no call
    assert_eq!(home_floor::seen(&ha.id), None);
    assert_eq!(env.state.db.mcp_reported_name(&ha.id).await.unwrap(), None);
    assert_eq!(home_server_names(&env.state.db, std::slice::from_ref(&ha)).await, vec!["house".to_string()]);

    let claim = "Unlocking the shed door now.";
    claim_once(&env, "hskf_ha", claim).await;
    let asked = continued(claim);
    wait_for("the home-change continuation", || {
        let n = invocations_with(&asked).len();
        async move { n > 0 }
    })
    .await;
    tokio::time::sleep(Duration::from_millis(500)).await;
    assert_eq!(invocations_with(&asked).len(), 1, "one continuation");
}

#[tokio::test]
async fn a_never_tested_server_elsewhere_is_not_home_assistants() {
    let env = setup().await;
    let other = server(&env, "notes", "http://127.0.0.1:9/mcp").await;
    assert!(home_server_names(&env.state.db, std::slice::from_ref(&other)).await.is_empty());

    let claim = "Unlocking the barn door now.";
    claim_once(&env, "hskf_other", claim).await;
    tokio::time::sleep(Duration::from_millis(1500)).await;
    assert!(invocations_with(&continued(claim)).is_empty(), "no continuation");
}

#[tokio::test]
async fn what_a_server_said_wins_over_its_address() {
    let env = setup().await;
    let ha_path = server(&env, "board", "http://127.0.0.1:9/api/mcp").await;
    // Its test said it is something else: its address doesn't matter any more
    env.state.db.save_mcp_reported_name(&ha_path.id, Some("kwnt-echo")).await.unwrap();
    assert!(home_server_names(&env.state.db, std::slice::from_ref(&ha_path)).await.is_empty());
    // It gave no name at all: not counted either
    env.state.db.save_mcp_reported_name(&ha_path.id, None).await.unwrap();
    assert!(home_server_names(&env.state.db, std::slice::from_ref(&ha_path)).await.is_empty());

    let claim = "Unlocking the cellar door now.";
    claim_once(&env, "hskf_said", claim).await;
    tokio::time::sleep(Duration::from_millis(1500)).await;
    assert!(invocations_with(&continued(claim)).is_empty(), "no continuation");

    // What kwnt heard on a connection in this run wins over what was stored, both ways
    home_floor::note_server_info(&ha_path.id, &json!({"name": "home-assistant"}));
    assert_eq!(home_server_names(&env.state.db, std::slice::from_ref(&ha_path)).await, vec!["board".to_string()]);
    home_floor::note_server_info(&ha_path.id, &json!({"name": "kwnt-echo"}));
    assert!(home_server_names(&env.state.db, std::slice::from_ref(&ha_path)).await.is_empty());
    home_floor::forget(&ha_path.id);
}
