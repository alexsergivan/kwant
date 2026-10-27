//! Bots try the fallback and offer the fix instead of "not possible": every turn with kwnt's tools
//! gets `page_source` (a page's HTML, which WebFetch and the browser don't show) and, in a chat,
//! `connection_suggest` (a card offering an app connection); the prompt tells them to use both
//! before calling something impossible.

mod common;

use axum::http::StatusCode;
use common::*;
use kwnt::server::continuity::{SelfScope, TurnOrigin};
use serde_json::{json, Value};

async fn rpc(env: &Env, token: &str, method: &str, params: Value) -> Value {
    let (s, v) = call_with(
        &env.app,
        "POST",
        "/api/mcp/self",
        Some(json!({"jsonrpc": "2.0", "id": 1, "method": method, "params": params})),
        &[("authorization", &format!("Bearer {token}"))],
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    v["result"].clone()
}

async fn self_call(env: &Env, token: &str, tool: &str, args: Value) -> Value {
    rpc(env, token, "tools/call", json!({"name": tool, "arguments": args})).await
}

fn text_of(result: &Value) -> String {
    result["content"].as_array().unwrap().iter().filter_map(|c| c["text"].as_str()).collect::<Vec<_>>().join("\n")
}

async fn turn(env: &Env, token: &str, kwant: &str, in_chat: bool) {
    env.state.self_tools.lock().await.insert(
        token.into(),
        SelfScope {
            kwant_id: kwant.into(),
            kwant_name: "K".into(),
            origin: in_chat.then(|| TurnOrigin { direct_kwant_id: Some(kwant.into()), ..Default::default() }),
            turn_id: Some(format!("turn-{token}")),
            ..Default::default()
        },
    );
}

async fn tool_names(env: &Env, token: &str) -> Vec<String> {
    let r = rpc(env, token, "tools/list", json!({})).await;
    r["tools"].as_array().unwrap().iter().map(|t| t["name"].as_str().unwrap().to_string()).collect()
}

#[tokio::test]
async fn every_turn_can_read_page_source_and_chats_can_offer_a_connection() {
    let env = setup().await;
    turn(&env, "tok-chat", &env.a, true).await;
    turn(&env, "tok-bare", &env.a, false).await;
    let chat = tool_names(&env, "tok-chat").await;
    assert!(chat.contains(&"page_source".to_string()) && chat.contains(&"connection_suggest".to_string()), "{chat:?}");
    let bare = tool_names(&env, "tok-bare").await;
    assert!(bare.contains(&"page_source".to_string()), "{bare:?}");
    assert!(!bare.contains(&"connection_suggest".to_string()), "no chat, no card to post");
}

#[tokio::test]
async fn page_source_reads_public_pages_only() {
    let env = setup().await;
    turn(&env, "tok-ps", &env.a, true).await;
    for url in ["http://127.0.0.1:9/", "http://10.0.0.1/", "http://localhost/", "file:///etc/passwd"] {
        let r = self_call(&env, "tok-ps", "page_source", json!({"url": url})).await;
        assert_eq!(r["isError"], true, "{url}: {r}");
        let t = text_of(&r);
        assert!(t.contains("public pages only") || t.contains("isn't a web address"), "{url}: {t}");
    }
    let r = self_call(&env, "tok-ps", "page_source", json!({})).await;
    assert_eq!(r["isError"], true);
}

#[tokio::test]
async fn a_connection_suggestion_posts_one_card_per_app_with_the_steps() {
    let env = setup().await;
    turn(&env, "tok-cs", &env.a, true).await;
    let args = json!({
        "app": "google_search_console",
        "name": "Google Search Console",
        "reason": "It shows which of your pages Google indexed and why others aren't.",
    });
    let r = self_call(&env, "tok-cs", "connection_suggest", args.clone()).await;
    assert_eq!(r["isError"], false, "{r}");
    assert!(text_of(&r).contains("First the user has to add Composio"), "{}", text_of(&r));
    let cards: Vec<_> = messages(&env, &env.a).await.into_iter().filter(|m| m.text.starts_with("Suggested connection:")).collect();
    assert_eq!(cards.len(), 1);
    assert_eq!(cards[0].sender_role, "kwant");
    assert!(cards[0].text.contains("find Google Search Console and press Connect"), "{}", cards[0].text);
    let meta: Value = serde_json::from_str(cards[0].meta.as_deref().unwrap()).unwrap();
    assert_eq!(meta["kind"], "connection_suggestion");
    assert_eq!(meta["app"], "google_search_console");
    assert_eq!(meta["composio"], "not_added");
    assert_eq!(meta["connect"], "/api/connections/composio/apps/google_search_console/connect");

    // The same app again in the same turn: no second card
    let again = self_call(&env, "tok-cs", "connection_suggest", args).await;
    assert!(text_of(&again).contains("already suggested"), "{again}");
    assert_eq!(messages(&env, &env.a).await.iter().filter(|m| m.text.starts_with("Suggested connection:")).count(), 1);

    // Not a slug, or no reason: refused, nothing posted
    for bad in [
        json!({"app": "../etc", "name": "X", "reason": "y"}),
        json!({"app": "Google Search Console", "name": "X", "reason": "y"}),
        json!({"app": "hubspot", "name": "HubSpot", "reason": " "}),
    ] {
        let r = self_call(&env, "tok-cs", "connection_suggest", bad.clone()).await;
        assert_eq!(r["isError"], true, "{bad}");
    }
    assert_eq!(messages(&env, &env.a).await.iter().filter(|m| m.text.starts_with("Suggested connection:")).count(), 1);
}

#[tokio::test]
async fn with_composio_ready_the_card_goes_straight_to_connect() {
    let env = setup().await;
    let (s, v) = call(
        &env.app,
        "POST",
        "/api/connections/composio",
        Some(json!({"url": "https://backend.composio.dev/v3/mcp/abc/mcp?user_id=u1", "api_key": "ck_test", "enabled": true})),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    turn(&env, "tok-ready", &env.a, true).await;
    let r = self_call(
        &env,
        "tok-ready",
        "connection_suggest",
        json!({"app": "google_analytics", "name": "Google Analytics", "reason": "Traffic per page for the audit."}),
    )
    .await;
    assert_eq!(r["isError"], false, "{r}");
    assert!(!text_of(&r).contains("First the user"), "{}", text_of(&r));
    let card = messages(&env, &env.a).await.into_iter().find(|m| m.text.starts_with("Suggested connection: Google Analytics")).unwrap();
    let meta: Value = serde_json::from_str(card.meta.as_deref().unwrap()).unwrap();
    assert_eq!(meta["composio"], "ready");
    assert!(card.text.contains("To connect it, open Connections"), "{}", card.text);
}

#[tokio::test]
async fn the_prompt_says_to_try_the_fallback_and_offer_the_fix() {
    let env = setup().await;
    send_direct(&env, &env.a, "audit my page REPLYTAG:bo1", json!({})).await;
    wait_reply(&env, &env.a, "answer bo1").await;
    let args = invocations_with("REPLYTAG:bo1").pop().unwrap();
    assert!(args.contains("Before you call anything impossible, try the fallback"), "fallback rule missing");
    assert!(args.contains("`page_source` reads a public page's HTML source"));
    assert!(args.contains("`connection_suggest` posts a one-tap connection card. Post it in the same turn rather than asking"));
    assert!(args.contains("pagespeed.web.dev"));
    assert!(args.contains("not its HTML: for tags and attributes"), "claude's tool paragraph explains WebFetch");
}
