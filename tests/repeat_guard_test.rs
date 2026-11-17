//! 15c: a turn stops sending a call that keeps failing the same way. A fake http MCP server
//! whose `fail` tool always errors counts the calls that reach it; a mock OpenAI endpoint plays
//! the model. No network beyond 127.0.0.1 and no API keys.

use async_trait::async_trait;
use axum::{extract::State, routing::post, Json, Router};
use kwnt::{
    db::models::{Kwant, McpServer},
    drivers::{events::RuntimeEvent, ApiDriver, Driver, TurnInput},
    mcp::{session::Caller, session::McpToolSession, McpTurnContext, ToolApprovalRequest, ToolApprover},
};
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

const FAIL_TEXT: &str = "invalid_request: the board 'roadmap' does not exist";

/// `tools/call` hits per tool name.
#[derive(Clone, Default)]
struct Hits(Arc<Mutex<BTreeMap<String, usize>>>);

impl Hits {
    fn of(&self, tool: &str) -> usize {
        self.0.lock().unwrap().get(tool).copied().unwrap_or(0)
    }
}

/// An http MCP server with `fail` (always the same error), `flaky` (a new error each time) and
/// `status` (always works, may change things). `lookup` (marked `readOnlyHint`) and `get_board`
/// (a reader by its name) always work and only read.
async fn fake_mcp() -> (String, Hits) {
    async fn mcp(State(hits): State<Hits>, Json(msg): Json<Value>) -> axum::response::Response {
        use axum::response::IntoResponse;
        let id = msg["id"].clone();
        let tool = |name: &str| json!({"name": name, "inputSchema": {"type": "object", "properties": {"board": {"type": "string"}}}});
        let result = match msg["method"].as_str().unwrap_or("") {
            "initialize" => json!({"protocolVersion": "2025-06-18", "capabilities": {"tools": {}},
                                   "serverInfo": {"name": "fake-fail", "version": "1.0.0"}}),
            "tools/list" => {
                let mut lookup = tool("lookup");
                lookup["annotations"] = json!({"readOnlyHint": true});
                json!({"tools": [tool("fail"), tool("flaky"), tool("status"), lookup, tool("get_board")]})
            }
            "tools/call" => {
                let name = msg["params"]["name"].as_str().unwrap_or("").to_string();
                let n = {
                    let mut h = hits.0.lock().unwrap();
                    let n = h.entry(name.clone()).or_default();
                    *n += 1;
                    *n
                };
                match name.as_str() {
                    "fail" => json!({"content": [{"type": "text", "text": FAIL_TEXT}], "isError": true}),
                    "flaky" => json!({"content": [{"type": "text", "text": format!("attempt {n} failed")}], "isError": true}),
                    _ => json!({"content": [{"type": "text", "text": "running"}]}),
                }
            }
            _ => return axum::http::StatusCode::ACCEPTED.into_response(),
        };
        Json(json!({"jsonrpc": "2.0", "id": id, "result": result})).into_response()
    }
    let hits = Hits::default();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let app = Router::new().route("/mcp", post(mcp)).with_state(hits.clone());
    tokio::spawn(async move { axum::serve(listener, app).await });
    (format!("{base}/mcp"), hits)
}

/// Allows (or denies) every call and counts the cards it was asked.
struct Counting {
    allow: AtomicBool,
    cards: AtomicUsize,
}

#[async_trait]
impl ToolApprover for Counting {
    async fn approve(&self, _request: ToolApprovalRequest) -> bool {
        self.cards.fetch_add(1, Ordering::SeqCst);
        self.allow.load(Ordering::SeqCst)
    }
}

fn ctx(url: &str, approver: Arc<dyn ToolApprover>, max: usize) -> McpTurnContext {
    McpTurnContext {
        servers: vec![McpServer {
            id: "s-board".into(),
            name: "board".into(),
            transport: "http".into(),
            command: None,
            args: vec![],
            url: Some(url.to_string()),
            env: BTreeMap::new(),
            headers: BTreeMap::new(),
            enabled: true,
            timeout_secs: 10,
            created_at: None,
        }],
        approver,
        config_dir: std::env::temp_dir(),
        permission_url: None,
        gateway: None,
        max_tool_iterations: max,
        approvals: kwnt::mcp::Approvals::Ask,
        records: Default::default(),
        kwnt_only: false,
    }
}

fn caller() -> Caller {
    Caller { kwant_id: "k1".into(), kwant_name: "Ada".into(), turn_id: "t1".into() }
}

fn counting(allow: bool) -> Arc<Counting> {
    Arc::new(Counting { allow: AtomicBool::new(allow), cards: AtomicUsize::new(0) })
}

/// The 4th identical failing call never reaches the server, nor asks a card; the record says so.
#[tokio::test]
async fn the_fourth_identical_failure_is_not_sent() {
    let (url, hits) = fake_mcp().await;
    let approver = counting(true);
    let c = ctx(&url, approver.clone(), 8);
    let (session, problems) = McpToolSession::open(&c, None).await;
    assert!(problems.is_empty(), "{problems:?}");
    // Key order doesn't matter: these are the same call
    let args = [r#"{"board": "roadmap", "limit": 5}"#, r#"{"limit": 5, "board": "roadmap"}"#];
    for i in 0..3 {
        let o = session.call(&caller(), &format!("c{i}"), "mcp__board__fail", args[i % 2]).await;
        assert_eq!((o.status, o.text.as_str()), ("error", FAIL_TEXT));
        assert!(o.record.get("repeat_stopped").is_none());
    }
    assert_eq!(hits.of("fail"), 3);
    assert_eq!(approver.cards.load(Ordering::SeqCst), 3);
    for i in 3..5 {
        let o = session.call(&caller(), &format!("c{i}"), "mcp__board__fail", args[i % 2]).await;
        assert_eq!(o.status, "error");
        assert!(o.text.starts_with("kwnt didn't send this call: the same call (same tool, same arguments) already failed 3 times in this turn"), "{}", o.text);
        assert!(o.text.contains(&format!("\"{FAIL_TEXT}\"")), "{}", o.text);
        assert!(o.text.contains("Blocked: <what is blocked>. <what would unblock it>"));
        assert_eq!(o.record["repeat_stopped"], 3);
        assert_eq!(o.record["name"], "mcp__board__fail");
    }
    assert_eq!(hits.of("fail"), 3, "the 4th and 5th calls never reached the server");
    assert_eq!(approver.cards.load(Ordering::SeqCst), 3, "and asked no card");
    // Other arguments are another call
    let o = session.call(&caller(), "d1", "mcp__board__fail", r#"{"board": "backlog"}"#).await;
    assert_eq!(o.text, FAIL_TEXT);
    assert_eq!(hits.of("fail"), 4);
    // The trail has every call, the stopped ones included
    let records = c.take_records();
    assert_eq!(records.len(), 6);
    assert_eq!(records.iter().filter(|r| r.get("repeat_stopped").is_some()).count(), 2);
    session.shutdown().await;
}

/// Polling that works, and failures whose text changes, are never stopped; denied calls don't count.
#[tokio::test]
async fn working_polls_changing_errors_and_denials_are_not_stopped() {
    let (url, hits) = fake_mcp().await;
    let approver = counting(false);
    let c = ctx(&url, approver.clone(), 8);
    let (session, _) = McpToolSession::open(&c, None).await;
    for i in 0..5 {
        let o = session.call(&caller(), &format!("d{i}"), "mcp__board__fail", r#"{"board": "roadmap"}"#).await;
        assert_eq!(o.status, "denied");
    }
    assert_eq!(approver.cards.load(Ordering::SeqCst), 5, "every denied call still asked");
    approver.allow.store(true, Ordering::SeqCst);
    for i in 0..6 {
        let o = session.call(&caller(), &format!("s{i}"), "mcp__board__status", r#"{"board": "roadmap"}"#).await;
        assert_eq!((o.status, o.text.as_str()), ("ok", "running"));
        let o = session.call(&caller(), &format!("f{i}"), "mcp__board__flaky", r#"{"board": "roadmap"}"#).await;
        assert_eq!(o.text, format!("attempt {} failed", i + 1));
    }
    assert_eq!((hits.of("status"), hits.of("flaky")), (6, 6));
    // After 5 denials the person allows: the call runs 3 times before it is stopped
    for _ in 0..4 {
        session.call(&caller(), "x", "mcp__board__fail", r#"{"board": "roadmap"}"#).await;
    }
    assert_eq!(hits.of("fail"), 3);
    session.shutdown().await;
}

/// 22t: on the API engines, a call that worked and may have changed something gives the stopped
/// call 3 more tries, up to 6 identical failures in the turn; a read changes nothing.
#[tokio::test]
async fn a_call_that_changed_something_lets_the_stopped_call_run_again() {
    let (url, hits) = fake_mcp().await;
    let c = ctx(&url, counting(true), 32);
    let (session, problems) = McpToolSession::open(&c, None).await;
    assert!(problems.is_empty(), "{problems:?}");
    macro_rules! fail {
        () => {
            session.call(&caller(), "f", "mcp__board__fail", r#"{"board": "roadmap"}"#).await
        };
    }
    for _ in 0..4 {
        fail!();
    }
    assert_eq!(hits.of("fail"), 3);
    // Reads, by the server's claim or by name, don't reopen it
    for tool in ["mcp__board__lookup", "mcp__board__get_board"] {
        let o = session.call(&caller(), "r", tool, r#"{"board": "roadmap"}"#).await;
        assert_eq!(o.status, "ok");
        assert!(fail!().record.get("repeat_stopped").is_some(), "{tool} reopened it");
    }
    assert_eq!(hits.of("fail"), 3);
    // A call that isn't marked read-only worked: 3 more tries reach the server
    assert_eq!(session.call(&caller(), "w", "mcp__board__status", r#"{"board": "roadmap"}"#).await.status, "ok");
    for i in 4..=6 {
        let o = fail!();
        assert_eq!((o.status, o.text.as_str()), ("error", FAIL_TEXT), "try {i} sent");
    }
    // 6 in the turn is the ceiling: another change doesn't send a 7th
    session.call(&caller(), "w", "mcp__board__status", r#"{"board": "roadmap"}"#).await;
    let o = fail!();
    assert_eq!(o.record["repeat_stopped"], 6);
    assert!(o.text.contains("already failed 6 times in this turn"), "{}", o.text);
    assert_eq!(hits.of("fail"), 6);
    session.shutdown().await;
}

// ---- One API-engine turn with a stub model (no paid call) ----

#[derive(Clone, Default)]
struct Mock {
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

fn tool_call(id: &str, name: &str, args: Value) -> Value {
    json!({"choices": [{"message": {"role": "assistant", "content": null, "tool_calls": [
        {"id": id, "type": "function", "function": {"name": name, "arguments": args.to_string()}}]}}],
        "usage": {"total_tokens": 10}})
}

fn input(mcp: McpTurnContext) -> TurnInput {
    TurnInput {
        fresh_eyes: false,
        expects: String::new(),
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
        prompt: "add a card to the roadmap board".into(),
        history: vec![],
        model_override: None,
        model: None,
        effort_level: Some("low".into()),
        team_context: None,
        workspace_dir: None,
        sandbox_id: None,
        teammates: vec![],
        mcp: Some(mcp),
        no_tools: false,
        read_only: false,
        delivery: None,
    }
}

/// A stub model that would retry forever: the guard answers its 4th try, and the model (scripted
/// as a well-behaved one would) ends with a Blocked line instead of spending the turn's cap.
#[tokio::test]
async fn an_api_turn_hits_the_guard_and_ends_blocked() {
    let (mcp_url, hits) = fake_mcp().await;
    let call = || tool_call("c", "mcp__board__fail", json!({"board": "roadmap"}));
    let blocked = "I couldn't add the card: the roadmap board doesn't exist on this server.\n\n\
                   Blocked: the \"roadmap\" board is missing. Create it, or tell me which board to use.";
    let replies = vec![
        call(),
        call(),
        call(),
        call(),
        json!({"choices": [{"message": {"role": "assistant", "content": blocked}}], "usage": {"total_tokens": 7}}),
    ];
    let mock = Mock { replies: Arc::new(replies), ..Default::default() };
    let app = Router::new().route("/v1/chat/completions", post(completions)).with_state(mock.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}/v1/chat/completions", listener.local_addr().unwrap());
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });

    let c = ctx(&mcp_url, counting(true), 12);
    let driver = ApiDriver::new(Some("test-key-not-real".into()), Some(url));
    let (tx, mut rx) = tokio::sync::mpsc::channel(256);
    let events = tokio::spawn(async move {
        let mut events = Vec::new();
        while let Some(e) = rx.recv().await {
            events.push(e);
        }
        events
    });
    let out = driver.stream_turn(input(c.clone()), tx).await.unwrap();
    let events = events.await.unwrap();

    assert_eq!(hits.of("fail"), 3, "the 4th try never reached the server");
    let requests = mock.requests.lock().unwrap().clone();
    assert_eq!(requests.len(), 5, "4 tool rounds and the answer, not the cap of 12");
    let told = requests[4]["messages"].as_array().unwrap().iter().rev().find(|m| m["role"] == "tool").unwrap()["content"]
        .as_str()
        .unwrap()
        .to_string();
    assert!(told.starts_with("kwnt didn't send this call"), "{told}");
    assert_eq!(out.text, blocked);
    // The chat's activity marks the stopped call, so the failure line skips it
    let stopped: Vec<_> = events
        .iter()
        .filter_map(|e| match e {
            RuntimeEvent::ToolCallCompleted { result, .. } if result.get("repeat_stopped").is_some() => Some(result.clone()),
            _ => None,
        })
        .collect();
    assert_eq!(stopped.len(), 1);
    assert_eq!(stopped[0]["status"], "error");
    let records = c.take_records();
    assert_eq!(records.len(), 4);
    assert_eq!(records[3]["repeat_stopped"], 3);
    // The replay quoted in impl.md
    println!("--- replay ---");
    for (i, r) in records.iter().enumerate() {
        println!("call {}: {} {} -> {}: {}", i + 1, r["name"], r["arguments"], r["status"], r["result"].as_str().unwrap_or(""));
    }
    println!("model's reply:\n{}", out.text);
}
