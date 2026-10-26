//! API-agent MCP tool loop against a local mock OpenAI endpoint and the echo MCP server.
//! No network and no API keys: the mock listens on 127.0.0.1.

use axum::{extract::State, routing::post, Json, Router};
use kwnt::{
    db::models::{Kwant, McpServer},
    drivers::{events::RuntimeEvent, ApiDriver, Driver, TurnInput, TurnOutput},
    mcp::{FixedApprover, McpTurnContext, ToolApprover, SECRET_MASK},
};
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

const KWNT: &str = env!("CARGO_BIN_EXE_kwnt");

#[derive(Clone, Default)]
struct Mock {
    /// Replies in order; the last one repeats.
    replies: Arc<Vec<Value>>,
    requests: Arc<Mutex<Vec<Value>>>,
}

async fn completions(State(mock): State<Mock>, Json(body): Json<Value>) -> Json<Value> {
    let n = {
        let mut r = mock.requests.lock().unwrap();
        r.push(body);
        r.len()
    };
    Json(mock.replies[(n - 1).min(mock.replies.len() - 1)].clone())
}

async fn start_mock(replies: Vec<Value>) -> (String, Mock) {
    let mock = Mock { replies: Arc::new(replies), ..Default::default() };
    let app = Router::new()
        .route("/v1/chat/completions", post(completions))
        .route("/v1/messages", post(completions))
        .with_state(mock.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    (format!("http://{addr}/v1/chat/completions"), mock)
}

/// `start_mock` as an Anthropic Messages endpoint.
async fn start_anthropic_mock(replies: Vec<Value>) -> (String, Mock) {
    let (url, mock) = start_mock(replies).await;
    (url.replace("/v1/chat/completions", "/v1/messages"), mock)
}

fn tool_use(id: &str, name: &str, input: Value) -> Value {
    json!({"content": [{"type": "text", "text": "Let me check."}, {"type": "tool_use", "id": id, "name": name, "input": input}],
        "stop_reason": "tool_use", "usage": {"input_tokens": 6, "output_tokens": 4}})
}

fn anthropic_answer(text: &str) -> Value {
    json!({"content": [{"type": "text", "text": text}], "stop_reason": "end_turn", "usage": {"input_tokens": 5, "output_tokens": 2}})
}

fn tool_call(id: &str, name: &str, args: Value) -> Value {
    json!({"choices": [{"message": {"role": "assistant", "content": null, "tool_calls": [
        {"id": id, "type": "function", "function": {"name": name, "arguments": args.to_string()}}]}}],
        "usage": {"total_tokens": 10}})
}

fn answer(text: &str) -> Value {
    json!({"choices": [{"message": {"role": "assistant", "content": text}}], "usage": {"total_tokens": 7}})
}

fn echo_server(timeout_secs: i64) -> McpServer {
    McpServer {
        id: "s1".into(),
        name: "echo".into(),
        transport: "stdio".into(),
        command: Some(KWNT.into()),
        args: vec!["mcp-echo-server".into()],
        url: None,
        env: BTreeMap::from([("ECHO_SECRET".to_string(), "hunter2-secret".to_string())]),
        headers: BTreeMap::new(),
        enabled: true,
        timeout_secs,
        created_at: None,
    }
}

fn ctx(approver: Arc<dyn ToolApprover>, timeout_secs: i64, max: usize) -> McpTurnContext {
    McpTurnContext {
        servers: vec![echo_server(timeout_secs)],
        approver,
        config_dir: std::env::temp_dir(),
        permission_url: None,
        gateway: None,
        max_tool_iterations: max,
        smart_approvals: false,
        records: Default::default(),
    }
}

fn input(mcp: Option<McpTurnContext>) -> TurnInput {
    TurnInput {
        continuity: String::new(),
        skills: String::new(),
        person: Default::default(),
        store: None,
        browser_api_url: None,
        thread_id: "t1".into(),
        kwant: Kwant {
            space_id: kwnt::db::spaces::DEFAULT_SPACE_ID.into(),
            id: "k1".into(),
            name: "Ada".into(),
            title: "Lead".into(),
            persona_prompt: "p".into(),
            soul: String::new(),
            avatar_sprite: "s".into(),
            driver_type: "api".into(),
            preferred_model: "mock-model".into(),
            temperature: 0.2,
            max_tokens: 256,
            level: 1,
            turns_completed: 0,
            created_at: None,
        },
        prompt: "use the tool".into(),
        history: vec![],
        model_override: None,
        model: None,
        effort_level: Some("low".into()),
        team_context: None,
        workspace_dir: None,
        sandbox_id: None,
        teammates: vec![],
        mcp,
    }
}

async fn run(url: String, input: TurnInput) -> (TurnOutput, Vec<RuntimeEvent>) {
    let driver = ApiDriver::new(Some("test-key-not-real".into()), Some(url));
    let (tx, mut rx) = tokio::sync::mpsc::channel(256);
    let collect = tokio::spawn(async move {
        let mut events = Vec::new();
        while let Some(e) = rx.recv().await {
            events.push(e);
        }
        events
    });
    let out = driver.stream_turn(input, tx).await.unwrap();
    (out, collect.await.unwrap())
}

/// The tool message the model received in its second request.
fn tool_message(mock: &Mock, request: usize) -> Value {
    let requests = mock.requests.lock().unwrap();
    requests[request]["messages"].as_array().unwrap().iter().rev().find(|m| m["role"] == "tool").cloned().unwrap()
}

#[tokio::test]
async fn model_lists_and_calls_mcp_tools() {
    let (url, mock) = start_mock(vec![tool_call("call_1", "mcp__echo__echo", json!({"text": "hello"})), answer("The tool said hello.")]).await;
    let c = ctx(Arc::new(FixedApprover(true)), 10, 8);
    let (out, events) = run(url, input(Some(c.clone()))).await;

    assert_eq!(out.text, "The tool said hello.");
    assert_eq!(out.tokens_used, 17);
    let requests = mock.requests.lock().unwrap().clone();
    assert_eq!(requests.len(), 2);
    let names: Vec<_> = requests[0]["tools"].as_array().unwrap().iter().map(|t| t["function"]["name"].as_str().unwrap()).collect();
    assert_eq!(names, ["mcp__echo__echo", "mcp__echo__add", "mcp__echo__sleep", "mcp__echo__env", "mcp__echo__crash"]);
    assert_eq!(requests[0]["tools"][0]["function"]["parameters"]["required"], json!(["text"]));
    // Second request carries the assistant tool call and its result
    let msgs = requests[1]["messages"].as_array().unwrap();
    assert_eq!(msgs[msgs.len() - 2]["tool_calls"][0]["id"], "call_1");
    assert_eq!(msgs[msgs.len() - 1], json!({"role": "tool", "tool_call_id": "call_1", "content": "hello"}));

    assert!(events.iter().any(|e| matches!(e, RuntimeEvent::ToolCallPending { id, name, .. } if id == "call_1" && name == "mcp__echo__echo")));
    assert!(events.iter().any(|e| matches!(e, RuntimeEvent::ToolCallCompleted { id, result } if id == "call_1" && result["status"] == "ok")));
    let records = c.take_records();
    assert_eq!(records.len(), 1);
    assert_eq!(records[0]["status"], "ok");
    assert_eq!(records[0]["result"], "hello");
    assert_eq!(records[0]["server"], "echo");
    assert_eq!(records[0]["arguments"], json!({"text": "hello"}));
}

#[tokio::test]
async fn denied_call_is_not_run_and_model_is_told() {
    let (url, mock) = start_mock(vec![tool_call("call_1", "mcp__echo__add", json!({"a": 1, "b": 2})), answer("ok, no tool")]).await;
    let c = ctx(Arc::new(FixedApprover(false)), 10, 8);
    let (out, _) = run(url, input(Some(c.clone()))).await;
    assert_eq!(out.text, "ok, no tool");
    assert_eq!(tool_message(&mock, 1)["content"], "The user denied this tool call.");
    let records = c.take_records();
    assert_eq!(records[0]["status"], "denied");
    assert!(records[0].get("result").is_none());
}

#[tokio::test]
async fn server_crash_mid_call_becomes_a_tool_error() {
    let (url, mock) = start_mock(vec![
        tool_call("c1", "mcp__echo__crash", json!({})),
        tool_call("c2", "mcp__echo__echo", json!({"text": "again"})),
        answer("server is gone"),
    ])
    .await;
    let c = ctx(Arc::new(FixedApprover(true)), 30, 8);
    let start = std::time::Instant::now();
    let (out, _) = run(url, input(Some(c.clone()))).await;
    assert!(start.elapsed() < std::time::Duration::from_secs(10));
    assert_eq!(out.text, "server is gone");
    assert!(tool_message(&mock, 1)["content"].as_str().unwrap().contains("failed"));
    let records = c.take_records();
    assert_eq!(records[0]["status"], "error");
    assert_eq!(records[1]["status"], "error", "later calls to a dead server fail fast");
}

#[tokio::test]
async fn slow_tool_times_out() {
    let (url, mock) = start_mock(vec![tool_call("c1", "mcp__echo__sleep", json!({"ms": 10_000})), answer("too slow")]).await;
    let c = ctx(Arc::new(FixedApprover(true)), 1, 8);
    let start = std::time::Instant::now();
    let (out, _) = run(url, input(Some(c.clone()))).await;
    assert!(start.elapsed() < std::time::Duration::from_secs(6));
    assert_eq!(out.text, "too slow");
    assert!(tool_message(&mock, 1)["content"].as_str().unwrap().contains("timed out"), "{}", tool_message(&mock, 1));
    assert_eq!(c.take_records()[0]["status"], "error");
}

#[tokio::test]
async fn secrets_in_tool_output_are_masked() {
    let (url, mock) = start_mock(vec![tool_call("c1", "mcp__echo__env", json!({"name": "ECHO_SECRET"})), answer("done")]).await;
    let c = ctx(Arc::new(FixedApprover(true)), 10, 8);
    let (_, events) = run(url, input(Some(c.clone()))).await;
    assert_eq!(tool_message(&mock, 1)["content"], SECRET_MASK);
    let records = c.take_records();
    assert_eq!(records[0]["result"], SECRET_MASK);
    let all = format!("{events:?} {records:?} {:?}", mock.requests.lock().unwrap());
    assert!(!all.contains("hunter2-secret"));
}

#[tokio::test]
async fn tool_loop_stops_at_max_iterations() {
    let (url, mock) = start_mock(vec![tool_call("c", "mcp__echo__add", json!({"a": 1, "b": 1}))]).await;
    let c = ctx(Arc::new(FixedApprover(true)), 10, 3);
    let (out, _) = run(url, input(Some(c.clone()))).await;
    assert_eq!(mock.requests.lock().unwrap().len(), 3);
    assert!(out.text.contains("limit of 3 model calls"), "{}", out.text);
    assert_eq!(c.take_records().len(), 2, "the last round's calls are not run");
}

#[tokio::test]
async fn unknown_tools_and_bad_arguments_are_errors() {
    let (url, mock) = start_mock(vec![
        json!({"choices": [{"message": {"role": "assistant", "content": "", "tool_calls": [
            {"id": "a", "type": "function", "function": {"name": "mcp__other__x", "arguments": "{}"}},
            {"id": "b", "type": "function", "function": {"name": "mcp__echo__echo", "arguments": "not json"}},
            {"id": "c", "type": "function", "function": {"name": "mcp__echo__add", "arguments": {"a": 2, "b": 3}}}]}}]}),
        answer("fine"),
    ])
    .await;
    let c = ctx(Arc::new(FixedApprover(true)), 10, 8);
    let (out, _) = run(url, input(Some(c.clone()))).await;
    assert_eq!(out.text, "fine");
    let records = c.take_records();
    let statuses: Vec<_> = records.iter().map(|r| r["status"].as_str().unwrap()).collect();
    assert_eq!(statuses, ["error", "error", "ok"]);
    assert_eq!(records[2]["result"], "5", "object-form arguments (Ollama) are accepted");
    let second = mock.requests.lock().unwrap()[1].clone();
    let assistant = second["messages"].as_array().unwrap().iter().find(|m| m["role"] == "assistant").unwrap().clone();
    assert!(assistant["tool_calls"][2]["function"]["arguments"].is_string());
}

#[tokio::test]
async fn kwant_without_servers_makes_a_single_plain_call() {
    let (url, mock) = start_mock(vec![answer("hi there")]).await;
    let (out, _) = run(url, input(None)).await;
    assert_eq!(out.text, "hi there");
    let requests = mock.requests.lock().unwrap();
    assert_eq!(requests.len(), 1);
    assert!(requests[0].get("tools").is_none());
}

#[tokio::test]
async fn anthropic_bots_call_mcp_tools_in_the_messages_format() {
    let (url, mock) =
        start_anthropic_mock(vec![tool_use("toolu_1", "mcp__echo__echo", json!({"text": "hello"})), anthropic_answer("The tool said hello.")]).await;
    let c = ctx(Arc::new(FixedApprover(true)), 10, 8);
    let (out, events) = run(url, input(Some(c.clone()))).await;

    assert_eq!(out.text, "The tool said hello.");
    let requests = mock.requests.lock().unwrap().clone();
    assert_eq!(requests.len(), 2);
    let tools = requests[0]["tools"].as_array().unwrap();
    assert_eq!(tools[0]["name"], "mcp__echo__echo");
    assert_eq!(tools[0]["input_schema"]["required"], json!(["text"]));
    assert!(tools.iter().all(|t| t.get("function").is_none() && t["input_schema"]["type"] == "object"));
    // The second request replays the model's tool_use and answers it with a tool_result
    let msgs = requests[1]["messages"].as_array().unwrap();
    let (asked, answered) = (&msgs[msgs.len() - 2], &msgs[msgs.len() - 1]);
    assert_eq!(asked["role"], "assistant");
    assert_eq!(asked["content"][0], json!({"type": "text", "text": "Let me check."}));
    assert_eq!(asked["content"][1], json!({"type": "tool_use", "id": "toolu_1", "name": "mcp__echo__echo", "input": {"text": "hello"}}));
    assert_eq!(answered["role"], "user");
    assert_eq!(answered["content"][0]["type"], "tool_result");
    assert_eq!(answered["content"][0]["tool_use_id"], "toolu_1");
    assert_eq!(answered["content"][0]["content"], "hello");
    // The tool round is cached up to its result, and the system prompt behind the tools
    assert_eq!(answered["content"][0]["cache_control"], json!({"type": "ephemeral"}));
    assert!(requests[1]["system"].as_array().unwrap().iter().any(|b| b.get("cache_control").is_some()));
    assert!(requests.iter().all(|r| r.get("tool_choice").is_none() && r["messages"][0]["role"] == "user"));

    assert!(events.iter().any(|e| matches!(e, RuntimeEvent::ToolCallPending { id, name, .. } if id == "toolu_1" && name == "mcp__echo__echo")));
    assert!(!events.iter().any(|e| matches!(e, RuntimeEvent::ThinkingDelta(t) if t.contains("aren't offered"))));
    let records = c.take_records();
    assert_eq!((records.len(), records[0]["status"].as_str()), (1, Some("ok")));
}

#[tokio::test]
async fn anthropic_tool_calls_go_through_the_approver() {
    let (url, mock) = start_anthropic_mock(vec![tool_use("toolu_1", "mcp__echo__add", json!({"a": 1, "b": 2})), anthropic_answer("ok, no tool")]).await;
    let c = ctx(Arc::new(FixedApprover(false)), 10, 8);
    let (out, _) = run(url, input(Some(c.clone()))).await;
    assert_eq!(out.text, "ok, no tool");
    let requests = mock.requests.lock().unwrap().clone();
    let last = requests[1]["messages"].as_array().unwrap().last().unwrap().clone();
    assert_eq!(last["content"][0]["content"], "The user denied this tool call.");
    assert_eq!(c.take_records()[0]["status"], "denied");
}

/// The system prompt of the model's first request.
fn system_prompt(mock: &Mock) -> String {
    let requests = mock.requests.lock().unwrap();
    requests[0]["messages"].as_array().unwrap().iter().filter(|m| m["role"] == "system").map(|m| m["content"].to_string()).collect()
}

fn with_app_tools(server: McpServer) -> TurnInput {
    let mut c = ctx(Arc::new(FixedApprover(true)), 10, 8);
    c.servers = vec![McpServer { name: kwnt::mcp::SELF_SERVER_NAME.into(), ..server }];
    let mut i = input(Some(c));
    i.continuity = format!("\n\n[Your notes]\n- likes tea{}\n- `routine_create` schedules it", kwnt::drivers::APP_TOOLS_MARKER);
    i
}

#[tokio::test]
async fn app_tools_that_fail_to_start_are_not_promised() {
    let broken = McpServer { command: Some("/nonexistent/kwnt-tools".into()), ..echo_server(5) };
    let (url, mock) = start_mock(vec![answer("I can't schedule that right now.")]).await;
    let (out, events) = run(url, with_app_tools(broken)).await;
    assert_eq!(out.text, "I can't schedule that right now.");
    let system = system_prompt(&mock);
    assert!(!system.contains("routine_create"), "{system}");
    assert!(system.contains("likes tea") && system.contains("Your kwant app tools didn't start this turn"), "{system}");
    assert!(system.contains("You have no kwant tools this turn"), "{system}");
    assert!(mock.requests.lock().unwrap()[0].get("tools").is_none_or(|t| t.as_array().is_none_or(|a| a.is_empty())));
    assert!(events.iter().any(|e| matches!(e, RuntimeEvent::ThinkingDelta(t) if t.contains("MCP server unavailable"))));

    // A server that starts keeps the instructions
    let (url, mock) = start_mock(vec![answer("ok")]).await;
    run(url, with_app_tools(echo_server(5))).await;
    let system = system_prompt(&mock);
    assert!(system.contains("routine_create") && !system.contains("didn't start this turn"), "{system}");
}
