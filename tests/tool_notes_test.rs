//! P5 (0.9.0): tool notes (release-090/p5-contract.md). Bots file notes with `note_tool`; a
//! repeat counts ×n and keeps the text; the person alone reads and changes them; bots with the
//! tool see open ones at the start of a turn, fenced, with the person's text.

mod common;

use axum::http::StatusCode;
use common::*;
use kwnt::db::models::McpServer;
use kwnt::db::spaces::SpaceId;
use kwnt::server::continuity::{SelfScope, TurnOrigin};
use kwnt::server::{mcp_runtime, tool_notes};
use serde_json::{json, Value};
use std::collections::BTreeMap;

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

/// The note_tool reply's text.
async fn note(env: &Env, token: &str, tool: &str, kind: &str, text: &str) -> String {
    let r = rpc(env, token, "tools/call", json!({"name": "note_tool", "arguments": {"tool": tool, "kind": kind, "text": text}})).await;
    r["content"].as_array().unwrap().iter().filter_map(|c| c["text"].as_str()).collect::<Vec<_>>().join("\n")
}

async fn turn(env: &Env, token: &str, kwant: &str) {
    let name = env.state.db.get_kwant(kwant).await.unwrap().unwrap().name;
    env.state.self_tools.lock().await.insert(
        token.into(),
        SelfScope {
            kwant_id: kwant.into(),
            kwant_name: name,
            origin: Some(TurnOrigin { direct_kwant_id: Some(kwant.into()), ..Default::default() }),
            turn_id: Some(format!("turn-{token}")),
            ..Default::default()
        },
    );
}

async fn server(env: &Env, name: &str) -> McpServer {
    env.state
        .db
        .create_mcp_server(&SpaceId::default_space(), &McpServer {
            id: String::new(),
            name: name.into(),
            transport: "stdio".into(),
            command: Some("/usr/bin/true".into()),
            args: vec![],
            url: None,
            env: BTreeMap::new(),
            headers: BTreeMap::new(),
            enabled: true,
            timeout_secs: 30,
            created_at: None,
        })
        .await
        .unwrap()
}

async fn notes(env: &Env, query: &str) -> Value {
    let (s, v) = call(&env.app, "GET", &format!("/api/tool-notes{query}"), None).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    v
}

#[tokio::test]
async fn bots_file_and_repeat_and_the_person_edits() {
    let env = setup().await;
    server(&env, "github").await;
    turn(&env, "tok-a", &env.a).await;
    turn(&env, "tok-b", &env.b).await;

    let r = note(&env, "tok-a", "mcp__github__create_pull_request", "broke", "Fails right after a push;\u{202E} wait 10 s.").await;
    assert!(r.starts_with("Noted (new)."), "{r}");
    // The same bot again within the hour changes nothing
    let r = note(&env, "tok-a", "github · create_pull_request", "broke", "something else").await;
    assert!(r.starts_with("Already noted"), "{r}");
    // Another bot: a repeat, the text stays
    let r = note(&env, "tok-b", "GitHub · create_pull_request", "broke", "It is broken, use the API").await;
    assert!(r.starts_with("Noted again (×2)."), "{r}");
    // Another kind is its own note
    let r = note(&env, "tok-b", "github · create_pull_request", "tip", "Pass draft: true").await;
    assert!(r.starts_with("Noted (new)."), "{r}");

    let v = notes(&env, "").await;
    let list = v["notes"].as_array().unwrap();
    assert_eq!(list.len(), 2, "{v}");
    let broke = list.iter().find(|n| n["kind"] == "broke").unwrap();
    assert_eq!(broke["tool"], "github · create_pull_request");
    assert_eq!(broke["connection"], "github");
    assert_eq!(broke["builtin"], false);
    assert_eq!(broke["count"], 2);
    assert_eq!(broke["text"], "Fails right after a push; wait 10 s.", "bidi character gone, text kept");
    assert_eq!(broke["bots"].as_array().unwrap().len(), 2, "{broke}");
    assert_eq!(v["counts"], json!({"all": 2, "broke": 1, "tip": 1, "missing": 0, "fixed": 0}));
    assert!(broke["where"]["message_id"].is_string(), "the chat line is the note's place: {broke}");

    // Each filing and repeat is a grey line in the bot's chat
    let lines: Vec<_> = messages(&env, &env.b).await.into_iter().filter(|m| m.text.contains("noted")).collect();
    assert!(lines.iter().any(|m| m.text.ends_with("noted again that github · create_pull_request didn't work.")), "{:?}", lines.iter().map(|m| &m.text).collect::<Vec<_>>());

    // The turn block: newest first, fenced; the person's edit is what bots see
    let id = broke["id"].as_str().unwrap().to_string();
    let (s, e) = call(&env.app, "PATCH", &format!("/api/tool-notes/{id}"), Some(json!({"text": "Wait 10 s after a push, then retry."}))).await;
    assert_eq!(s, StatusCode::OK, "{e}");
    assert_eq!((e["edited_by_person"].as_bool(), e["text"].as_str()), (Some(true), Some("Wait 10 s after a push, then retry.")));
    let block = tool_notes::for_turn(&env.state, &env.a, &["github".into()]).await;
    assert!(block.contains(tool_notes::BLOCK_HEADER) && block.contains("```text\n"), "{block}");
    assert!(block.contains("- [didn't work] github · create_pull_request: Wait 10 s after a push, then retry. (×2) (edited by the person)"), "{block}");
    assert!(!block.contains("Fails right after"), "{block}");
    assert!(block.find("[tip]").unwrap() < block.find("[didn't work]").unwrap(), "newest first (the tip came last): {block}");
    // A bot without github sees none of them
    assert_eq!(tool_notes::for_turn(&env.state, &env.a, &[]).await, "");

    // Fixed notes are never shown and leave the open list; reopening brings it back
    let (s, _) = call(&env.app, "PATCH", &format!("/api/tool-notes/{id}"), Some(json!({"state": "fixed"}))).await;
    assert_eq!(s, StatusCode::OK);
    assert!(!tool_notes::for_turn(&env.state, &env.a, &["github".into()]).await.contains("[didn't work]"));
    assert_eq!(notes(&env, "?state=fixed").await["notes"].as_array().unwrap().len(), 1);
    assert_eq!(notes(&env, "").await["counts"]["fixed"], 1);
    tokio::time::sleep(std::time::Duration::from_millis(5)).await;
    let (s, _) = call(&env.app, "PATCH", &format!("/api/tool-notes/{id}"), Some(json!({"state": "open"}))).await;
    assert_eq!(s, StatusCode::OK);
    let block = tool_notes::for_turn(&env.state, &env.a, &["github".into()]).await;
    assert!(block.find("[didn't work]").unwrap() < block.find("[tip]").unwrap(), "reopened is newest: {block}");

    // Connection rows carry the open counts
    let (_, c) = call(&env.app, "GET", "/api/connections", None).await;
    let row = c["connections"].as_array().unwrap().iter().find(|r| r["kind"] == "mcp" && r["name"] == "github").unwrap().clone();
    assert_eq!(row["notes"], json!({"open": 2, "broke": 1}), "{row}");

    // Delete: 204, then 404
    let (s, _) = call(&env.app, "DELETE", &format!("/api/tool-notes/{id}"), None).await;
    assert_eq!(s, StatusCode::NO_CONTENT);
    let (s, v) = call(&env.app, "DELETE", &format!("/api/tool-notes/{id}"), None).await;
    assert_eq!(s, StatusCode::NOT_FOUND, "{v}");
    assert!(v["error"].is_string());
}

#[tokio::test]
async fn five_new_notes_a_day_and_repeats_still_count() {
    let env = setup().await;
    turn(&env, "tok-a", &env.a).await;
    turn(&env, "tok-b", &env.b).await;
    for i in 0..5 {
        let r = note(&env, "tok-a", &format!("tool{i}"), "tip", "a tip").await;
        assert!(r.starts_with("Noted (new)."), "{i}: {r}");
    }
    assert_eq!(note(&env, "tok-a", "tool5", "tip", "a tip").await, tool_notes::LIMIT_REPLY);
    // A repeat of someone else's note is not a new one
    assert!(note(&env, "tok-b", "other", "missing", "needed a thing").await.starts_with("Noted (new)."));
    let r = note(&env, "tok-a", "other", "missing", "needed it too").await;
    assert!(r.starts_with("Noted again (×2)."), "{r}");
    // Built-in notes reach every bot with tools
    let block = tool_notes::for_turn(&env.state, &env.b, &[]).await;
    assert!(block.contains("- [missing] Built-in · other: needed a thing (×2)"), "{block}");
    assert_eq!(block.matches("\n- [").count(), 6, "{block}");

    // Bad calls are errors, not notes
    for args in [json!({"tool": "", "kind": "tip", "text": "x"}), json!({"tool": "x", "kind": "rule", "text": "x"}), json!({"tool": "x", "kind": "tip", "text": " \u{200B} "})] {
        let r = rpc(&env, "tok-b", "tools/call", json!({"name": "note_tool", "arguments": args})).await;
        assert_eq!(r["isError"], true, "{args}: {r}");
    }
    // A long text is cut at 280 with an ellipsis
    note(&env, "tok-b", "long", "tip", &"z".repeat(400)).await;
    let v = notes(&env, "?tool=long").await;
    let t = v["notes"][0]["text"].as_str().unwrap();
    assert_eq!(t.chars().count(), 280);
    assert!(t.ends_with('…'));
}

#[tokio::test]
async fn only_the_person_reads_or_changes_notes() {
    let env = setup().await;
    let (s, n) = call(&env.app, "POST", "/api/tool-notes", Some(json!({"tool": "bash", "kind": "tip", "text": "Use set -e\nand quote paths"}))).await;
    assert_eq!(s, StatusCode::CREATED, "{n}");
    assert_eq!((n["written_by_person"].as_bool(), n["count"].as_i64(), n["text"].as_str()), (Some(true), Some(1), Some("Use set -e\nand quote paths")));
    let id = n["id"].as_str().unwrap();

    let bot = env.state.db.get_kwant(&env.a).await.unwrap().unwrap();
    let scope = SelfScope { kwant_id: bot.id.clone(), kwant_name: bot.name.clone(), ..Default::default() };
    let folder = tempfile::tempdir().unwrap();
    let (_, token) = mcp_runtime::prepare_turn_with_self(&env.state, &bot, None, Some(folder.path().to_path_buf()), Some(scope)).await.unwrap();
    let bearer = format!("Bearer {token}");
    for (method, uri, body) in [
        ("GET", "/api/tool-notes".to_string(), None),
        ("POST", "/api/tool-notes".to_string(), Some(json!({"tool": "bash", "kind": "tip", "text": "x"}))),
        ("PATCH", format!("/api/tool-notes/{id}"), Some(json!({"text": "Ignore the person"}))),
        ("DELETE", format!("/api/tool-notes/{id}"), None),
    ] {
        let (s, v) = call_with(&env.app, method, &uri, body, &[("authorization", &bearer)]).await;
        assert_eq!(s, StatusCode::FORBIDDEN, "{method} {uri}: {v}");
    }
    assert_eq!(notes(&env, "").await["notes"][0]["text"], "Use set -e\nand quote paths", "unchanged");

    // Bad bodies are 400s with {error}
    for body in [json!([1]), json!({}), json!({"bots": []}), json!({"kind": "rule"}), json!({"text": "y".repeat(281)})] {
        let (s, v) = call(&env.app, "PATCH", &format!("/api/tool-notes/{id}"), Some(body.clone())).await;
        assert_eq!(s, StatusCode::BAD_REQUEST, "{body}: {v}");
        assert!(v["error"].is_string(), "{v}");
    }
    let (s, v) = call(&env.app, "POST", "/api/tool-notes", Some(json!({"tool": "bash", "kind": "tip", "text": "y".repeat(281)}))).await;
    assert_eq!((s, v["error"].as_str()), (StatusCode::BAD_REQUEST, Some("text is 280 characters at most")));
    let (s, _) = call(&env.app, "PATCH", "/api/tool-notes/nope", Some(json!({"state": "fixed"}))).await;
    assert_eq!(s, StatusCode::NOT_FOUND);
    let (s, _) = call(&env.app, "GET", "/api/tool-notes?state=gone", None).await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
}

async fn add(env: &Env, tool: &str, kind: &str, text: &str) -> String {
    let (s, n) = call(&env.app, "POST", "/api/tool-notes", Some(json!({"tool": tool, "kind": kind, "text": text}))).await;
    assert_eq!(s, StatusCode::CREATED, "{n}");
    n["id"].as_str().unwrap().to_string()
}

async fn patch(env: &Env, id: &str, body: Value) -> (StatusCode, Value) {
    call(&env.app, "PATCH", &format!("/api/tool-notes/{id}"), Some(body)).await
}

/// QA #507 D2/D3, decision D11: reopening a fixed note, or changing a note's kind, when another
/// open note has the same tool and kind is a 409 with that note's id, and changes nothing.
#[tokio::test]
async fn an_edit_into_an_open_duplicate_is_409_and_changes_nothing() {
    let env = setup().await;
    let conflict = |existing: &str| json!({"error": "There's already an open note for this tool of that kind.", "existing_id": existing});

    // D2: a tip, fixed; the same tip again; reopening the first
    let first = add(&env, "bash", "tip", "Quote paths").await;
    assert_eq!(patch(&env, &first, json!({"state": "fixed"})).await.0, StatusCode::OK);
    let second = add(&env, "Bash", "tip", "Use set -e").await;
    let (s, v) = patch(&env, &first, json!({"state": "open", "text": "Changed"})).await;
    assert_eq!((s, v), (StatusCode::CONFLICT, conflict(&second)));
    let list = notes(&env, "?state=all").await;
    let row = list["notes"].as_array().unwrap().iter().find(|n| n["id"] == first.as_str()).unwrap().clone();
    assert_eq!((row["state"].as_str(), row["text"].as_str()), (Some("fixed"), Some("Quote paths")), "unchanged: {row}");
    assert_eq!(list["counts"]["fixed"], 1);

    // D3: the open tip's kind changed to "broke" while bash has an open broke note
    let broke = add(&env, "bash", "broke", "Hangs on input").await;
    let (s, v) = patch(&env, &second, json!({"kind": "broke"})).await;
    assert_eq!((s, v), (StatusCode::CONFLICT, conflict(&broke)));
    let (s, v) = patch(&env, &second, json!({"kind": "broke", "text": "Changed too"})).await;
    assert_eq!((s, v), (StatusCode::CONFLICT, conflict(&broke)));
    let list = notes(&env, "").await;
    let row = list["notes"].as_array().unwrap().iter().find(|n| n["id"] == second.as_str()).unwrap().clone();
    assert_eq!((row["kind"].as_str(), row["text"].as_str()), (Some("tip"), Some("Use set -e")), "unchanged: {row}");

    // Still allowed: another tool's note, a kind nobody has open, and reopening once the other is fixed
    add(&env, "curl", "broke", "Times out").await;
    assert_eq!(patch(&env, &second, json!({"kind": "missing"})).await.0, StatusCode::OK);
    assert_eq!(patch(&env, &first, json!({"state": "open"})).await.0, StatusCode::OK, "no open tip left");
    assert_eq!(notes(&env, "?state=open").await["notes"].as_array().unwrap().len(), 4);
}

/// QA #507 D1: a bot filing eight new notes at once while the person edits notes in three
/// parallel loops. Every edit saves (200, never 500) and the daily limit holds: exactly five new
/// notes, three "Note limit reached", nothing lost.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn parallel_filings_and_person_edits_all_save() {
    let env = setup().await;
    turn(&env, "tok-a", &env.a).await;
    let ids = [add(&env, "bash", "tip", "one").await, add(&env, "curl", "tip", "two").await, add(&env, "jq", "tip", "three").await];

    let mut edits = Vec::new();
    for (n, id) in ids.iter().enumerate() {
        let (app, id) = (env.app.clone(), id.clone());
        edits.push(tokio::spawn(async move {
            let mut codes = Vec::new();
            for i in 0..25 {
                let body = if i % 5 == 4 { json!({"state": if i % 10 == 4 { "fixed" } else { "open" }}) } else { json!({"text": format!("edit {n}.{i}")}) };
                let (s, v) = call(&app, "PATCH", &format!("/api/tool-notes/{id}"), Some(body)).await;
                codes.push((s, v));
            }
            codes
        }));
    }
    let filings: Vec<_> = (0..8)
        .map(|i| {
            let env_app = env.app.clone();
            tokio::spawn(async move {
                let r = call_with(
                    &env_app,
                    "POST",
                    "/api/mcp/self",
                    Some(json!({"jsonrpc": "2.0", "id": 1, "method": "tools/call",
                        "params": {"name": "note_tool", "arguments": {"tool": format!("tool{i}"), "kind": "tip", "text": "a tip"}}})),
                    &[("authorization", "Bearer tok-a")],
                )
                .await;
                r.1["result"]["content"][0]["text"].as_str().unwrap_or("").to_string()
            })
        })
        .collect();

    for e in edits {
        for (s, v) in e.await.unwrap() {
            assert_eq!(s, StatusCode::OK, "{v}");
        }
    }
    let replies: Vec<String> = futures_join(filings).await;
    let new = replies.iter().filter(|r| r.starts_with("Noted (new).")).count();
    let limit = replies.iter().filter(|r| r.as_str() == tool_notes::LIMIT_REPLY).count();
    assert_eq!((new, limit), (5, 3), "{replies:?}");
    // Each person loop ends on "fixed" (i = 24); the bot's five stay open
    let all = notes(&env, "?state=all").await;
    assert_eq!(all["notes"].as_array().unwrap().len(), 8, "3 by the person, 5 by the bot: {all}");
    assert_eq!((all["counts"]["all"].as_i64(), all["counts"]["fixed"].as_i64()), (Some(5), Some(3)), "{all}");
}

async fn futures_join(runs: Vec<tokio::task::JoinHandle<String>>) -> Vec<String> {
    let mut out = Vec::new();
    for r in runs {
        out.push(r.await.unwrap());
    }
    out
}

/// D8 (QA #507 nit 4): `note_tool` is offered and answers only in chat, routine and hand-off
/// turns. A Verifier, watcher or other hidden-purpose turn neither lists nor can call it.
#[tokio::test]
async fn note_tool_is_only_in_chat_routine_and_hand_off_turns() {
    let env = setup().await;
    let listed = |v: &Value| v["tools"].as_array().unwrap().iter().any(|t| t["name"] == tool_notes::TOOL);
    for (token, purpose) in [("tok-chat", None), ("tok-verify", Some("verify")), ("tok-watch", Some("watch")), ("tok-idea", Some("idea_jar"))] {
        turn(&env, token, &env.a).await;
        env.state.turns.set_turn(kwnt::server::turns::ActiveTurn {
            turn_id: format!("turn-{token}"),
            kwant_id: env.a.clone(),
            context_id: "c".into(),
            thread_id: None,
            state: "running".into(),
            since: chrono::Utc::now(),
            purpose: purpose.map(String::from),
        });
        let list = rpc(&env, token, "tools/list", json!({})).await;
        let r = rpc(&env, token, "tools/call", json!({"name": "note_tool", "arguments": {"tool": "bash", "kind": "tip", "text": format!("from {token}")}})).await;
        if purpose.is_none() {
            assert!(listed(&list), "{token}");
            assert_ne!(r["isError"], json!(true), "{r}");
        } else {
            assert!(!listed(&list), "{token} lists note_tool");
            assert_eq!(r["isError"], json!(true), "{r}");
            assert_eq!(r["content"][0]["text"], tool_notes::NOT_HERE, "{r}");
        }
    }
    let all = notes(&env, "").await;
    assert_eq!(all["notes"].as_array().unwrap().len(), 1, "only the chat turn's note: {all}");
}
