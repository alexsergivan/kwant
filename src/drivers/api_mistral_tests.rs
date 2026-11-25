//! 35a: a fake Mistral server that answers like the real one. Mistral's API refuses (422) any
//! request whose tool call ids aren't exactly 9 letters or digits, so kwant's own `call_<uuid>`
//! ids, or OpenAI-shaped ids from a proxy, must never reach it as they are.

use super::*;
use axum::{extract::State, http::StatusCode, routing::post, Json, Router};
use std::sync::{Arc, Mutex};

type Bodies = Arc<Mutex<Vec<Value>>>;
type Script = fn(usize, &Value) -> Value;

fn mistral_id(id: &str) -> bool {
    id.len() == 9 && id.chars().all(|c| c.is_ascii_alphanumeric())
}

/// Mistral's 422 for a request whose tool call ids it won't take, else `None`. Like the real
/// one it also refuses a tool result whose id answers no earlier call.
fn mistral_refusal(body: &Value) -> Option<Value> {
    let mut called = Vec::new();
    for m in body["messages"].as_array().into_iter().flatten() {
        for call in m["tool_calls"].as_array().into_iter().flatten() {
            let id = call["id"].as_str().unwrap_or_default();
            if !mistral_id(id) {
                return Some(json!({
                    "object": "error",
                    "message": format!("Tool call id was {id} but must be a-z, A-Z, 0-9, with a length of 9."),
                    "type": "invalid_request_message_order", "param": null, "code": "3280"
                }));
            }
            called.push(id.to_string());
        }
        if m["role"] == "tool" {
            let id = m["tool_call_id"].as_str().unwrap_or_default();
            if !mistral_id(id) || !called.iter().any(|c| c == id) {
                return Some(json!({
                    "object": "error",
                    "message": format!("Tool call id was {id} but must be a-z, A-Z, 0-9, with a length of 9."),
                    "type": "invalid_request_message_order", "param": null, "code": "3280"
                }));
            }
        }
    }
    None
}

/// A fake `POST /v1/chat/completions` that keeps every body it gets, refuses like Mistral when
/// `strict`, and otherwise answers call `n` (from 0) with `script(n, body)`.
async fn fake_chat(strict: bool, script: Script) -> (String, Bodies) {
    let bodies: Bodies = Arc::default();
    let app = Router::new()
        .route(
            "/v1/chat/completions",
            post(|State((bodies, strict, script)): State<(Bodies, bool, Script)>, Json(body): Json<Value>| async move {
                let n = {
                    let mut all = bodies.lock().unwrap();
                    all.push(body.clone());
                    all.len() - 1
                };
                if strict {
                    if let Some(error) = mistral_refusal(&body) {
                        return (StatusCode::UNPROCESSABLE_ENTITY, Json(error));
                    }
                }
                (StatusCode::OK, Json(script(n, &body)))
            }),
        )
        .with_state((bodies.clone(), strict, script));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}/v1/chat/completions", listener.local_addr().unwrap());
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    (url, bodies)
}

fn reply_calls(calls: Value) -> Value {
    json!({"choices": [{"message": {"role": "assistant", "content": "", "tool_calls": calls}}],
        "usage": {"prompt_tokens": 10, "completion_tokens": 5}})
}

fn reply_text(text: &str) -> Value {
    json!({"choices": [{"message": {"role": "assistant", "content": text}}],
        "usage": {"prompt_tokens": 10, "completion_tokens": 5}})
}

fn find(query: &str, id: Option<&str>) -> Value {
    let mut call = json!({"type": "function", "function": {"name": FIND_TOOLS, "arguments": json!({"query": query}).to_string()}});
    if let Some(id) = id {
        call["id"] = json!(id);
    }
    call
}

/// The ids of the assistant's tool calls and of the tool results, in the order sent.
fn sent_ids(body: &Value) -> (Vec<String>, Vec<String>) {
    let (mut calls, mut results) = (Vec::new(), Vec::new());
    for m in body["messages"].as_array().unwrap() {
        for c in m["tool_calls"].as_array().into_iter().flatten() {
            calls.push(c["id"].as_str().unwrap().to_string());
        }
        if m["role"] == "tool" {
            results.push(m["tool_call_id"].as_str().unwrap().to_string());
        }
    }
    (calls, results)
}

fn tool_result(body: &Value, id: &str) -> String {
    body["messages"]
        .as_array()
        .unwrap()
        .iter()
        .find(|m| m["role"] == "tool" && m["tool_call_id"] == id)
        .and_then(|m| m["content"].as_str())
        .unwrap_or_else(|| panic!("no tool result for {id}"))
        .to_string()
}

/// A turn of `model` with an MCP session (no servers: the tools here are kwant's own, answered
/// inline) against `url`.
async fn run(url: String, model: &str) -> Result<TurnOutput, DriverError> {
    let driver = ApiDriver::new(Some("test-mistral-key".into()), Some(url));
    let (tx, mut rx) = mpsc::channel(64);
    tokio::spawn(async move { while rx.recv().await.is_some() {} });
    let tmp = tempfile::tempdir().unwrap();
    let mut input = tests_turn("look up my tools");
    input.kwant.preferred_model = "mistral".into();
    input.model = Some(model.into());
    input.mcp = Some(crate::mcp::McpTurnContext {
        servers: vec![],
        approver: Arc::new(crate::mcp::FixedApprover(true)),
        config_dir: tmp.path().to_path_buf(),
        permission_url: None,
        gateway: None,
        max_tool_iterations: 4,
        approvals: crate::mcp::Approvals::Off,
        records: Default::default(),
        kwnt_only: false,
        media: None,
    });
    driver.stream_turn(input, tx).await
}

fn tests_turn(prompt: &str) -> TurnInput {
    TurnInput {
        continuity: String::new(),
        skills: String::new(),
        person: Default::default(),
        store: None,
        browser_api_url: None,
        thread_id: "t1".into(),
        kwant: crate::db::models::Kwant {
            space_id: crate::db::spaces::DEFAULT_SPACE_ID.into(),
            id: "k1".into(),
            name: "Vega".into(),
            title: "Backend".into(),
            persona_prompt: "You build backends.".into(),
            soul: String::new(),
            avatar_sprite: "s".into(),
            driver_type: "api".into(),
            preferred_model: "mistral".into(),
            temperature: 0.7,
            max_tokens: 1000,
            level: 1,
            turns_completed: 0,
            created_at: None,
        },
        prompt: prompt.into(),
        history: vec![],
        model_override: None,
        model: None,
        effort_level: None,
        team_context: None,
        workspace_dir: None,
        sandbox_id: None,
        teammates: vec![],
        mcp: None,
        no_tools: false,
        read_only: false,
        delivery: None,
        fresh_eyes: false,
        expects: String::new(),
    }
}

/// Two tool calls whose ids Mistral won't take (one without an id, which kwant names
/// `call_<uuid>`, one OpenAI-shaped from a proxy), then a read-back of the first result by the id
/// the model saw, then the answer. Each request goes out with 9-character ids, the same ones
/// every time, and the read-back finds the result.
#[tokio::test]
async fn openai_shaped_tool_call_ids_reach_mistral_as_nine_characters() {
    fn script(n: usize, body: &Value) -> Value {
        match n {
            0 => reply_calls(json!([find("calendar", None), find("mail", Some("call_Qx7aB2mN4pLr8sTuVw"))])),
            1 => {
                // The model reads back its first result by the id it was given
                let (calls, _) = sent_ids(body);
                reply_calls(json!([{"id": "rEad00001", "type": "function",
                    "function": {"name": READ_RESULT, "arguments": json!({"id": calls[0]}).to_string()}}]))
            }
            _ => reply_text("Done."),
        }
    }
    let (url, bodies) = fake_chat(true, script).await;
    let out = run(url, "mistral-small-latest").await.expect("Mistral took every request");
    assert_eq!(out.text, "Done.");

    let bodies = bodies.lock().unwrap().clone();
    assert_eq!(bodies.len(), 3, "no request was refused");
    let (calls, results) = sent_ids(&bodies[1]);
    assert_eq!(calls.len(), 2);
    assert!(calls.iter().all(|id| mistral_id(id)), "{calls:?}");
    assert_ne!(calls[0], calls[1]);
    assert_eq!(results, calls, "each result answers its call");

    // Stable for the whole turn: the next request sends the same ids, and Mistral's own stays
    let (later, later_results) = sent_ids(&bodies[2]);
    assert_eq!(later, [calls[0].clone(), calls[1].clone(), "rEad00001".into()]);
    assert_eq!(later_results, later);
    // The read-back by the id the model saw returns the first result in full
    let first = tool_result(&bodies[1], &calls[0]);
    assert_eq!(tool_result(&bodies[2], "rEad00001"), first);
    assert!(!first.starts_with("No tool result"), "{first}");
}

/// Ids Mistral made itself already fit, and go back exactly as it sent them.
#[tokio::test]
async fn mistrals_own_ids_go_back_unchanged() {
    fn script(n: usize, _: &Value) -> Value {
        match n {
            0 => reply_calls(json!([find("calendar", Some("aB3dE5gH7")), find("mail", Some("Zz9Yy8Xx7"))])),
            _ => reply_text("Done."),
        }
    }
    let (url, bodies) = fake_chat(true, script).await;
    let out = run(url, "mistral-large-latest").await.expect("Mistral took every request");
    assert_eq!(out.text, "Done.");
    let bodies = bodies.lock().unwrap().clone();
    let (calls, results) = sent_ids(&bodies[1]);
    assert_eq!(calls, ["aB3dE5gH7", "Zz9Yy8Xx7"]);
    assert_eq!(results, calls);
}

/// Only Mistral gets short ids: an OpenAI turn's ids go out as they came.
#[tokio::test]
async fn other_providers_keep_their_ids() {
    fn script(n: usize, _: &Value) -> Value {
        match n {
            0 => reply_calls(json!([find("calendar", Some("call_Qx7aB2mN4pLr8sTuVw"))])),
            _ => reply_text("Done."),
        }
    }
    let (url, bodies) = fake_chat(false, script).await;
    let out = run(url, "gpt-5").await.unwrap();
    assert_eq!(out.text, "Done.");
    let bodies = bodies.lock().unwrap().clone();
    assert_eq!(sent_ids(&bodies[1]).0, ["call_Qx7aB2mN4pLr8sTuVw"]);
}
