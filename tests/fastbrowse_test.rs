//! fastbrowse (a browser agent with its own model) as an optional connection: kwnt never lets it
//! take an irreversible step itself (`--allow-authorize` and `authorize` never reach it), its
//! pause before one becomes an approval card, and a task that pays gets the payment card first,
//! in every approval mode.

use axum::{
    body::{to_bytes, Body},
    http::{Request, StatusCode},
    Router,
};
use kwnt::{
    broker::PermissionBroker,
    config::Config,
    db::{models::McpServer, spaces::SpaceId, Database},
    mcp::{
        cli_config,
        session::{Caller, McpToolSession},
        ApprovalOutcome, GatewayAccess, McpClient, McpTurnContext, SmartVerdict, ToolApprovalRequest, ToolApprover,
    },
    router::LayaRouter,
    server::{
        create_router,
        mcp_runtime::{self, PermissionSession},
        AppState,
    },
    tools::ToolRunner,
};
use serde_json::{json, Value};
use std::{
    collections::BTreeMap,
    path::PathBuf,
    sync::{Arc, Mutex},
    time::Duration,
};
use tempfile::{tempdir, TempDir};
use tower::ServiceExt;

/// A stand-in for `fastbrowse-mcp`: writes its argv and every call's arguments to `calls.jsonl`
/// next to itself, and stops before an irreversible step like the real one does without
/// `--allow-authorize`.
const FAKE: &str = r#"#!/usr/bin/env python3
import json, os, sys
log = os.path.join(os.path.dirname(os.path.abspath(__file__)), "calls.jsonl")
def note(entry):
    with open(log, "a") as f:
        f.write(json.dumps(entry) + "\n")
note({"argv": sys.argv[1:], "key": bool(os.environ.get("OPENROUTER_API_KEY"))})
for line in sys.stdin:
    msg = json.loads(line)
    method, mid = msg.get("method"), msg.get("id")
    if mid is None:
        continue
    if method == "initialize":
        result = {"protocolVersion": "2025-06-18", "capabilities": {"tools": {}}, "serverInfo": {"name": "fastbrowse", "version": "test"}}
    elif method == "tools/list":
        result = {"tools": [{"name": "browse", "description": "Run a browser task",
                  "inputSchema": {"type": "object", "properties": {"task": {"type": "string"}, "start": {"type": "string"},
                  "authorize": {"type": "boolean"}}, "required": ["task"]}}]}
    elif method == "tools/call":
        args = msg["params"].get("arguments", {})
        note({"call": args})
        stop = "submit" in args.get("task", "")
        out = {"status": "needs_confirmation" if stop else "done",
               "answer": "Stopped before pressing Submit" if stop else "Top story: Example",
               "final_url": "https://example.com/form", "steps": 3, "dollars": 0.01}
        result = {"content": [{"type": "text", "text": json.dumps(out)}]}
    else:
        result = {}
    sys.stdout.write(json.dumps({"jsonrpc": "2.0", "id": mid, "result": result}) + "\n")
    sys.stdout.flush()
"#;

struct Fake {
    dir: TempDir,
    command: PathBuf,
}

impl Fake {
    fn new() -> Self {
        let dir = tempdir().unwrap();
        let command = dir.path().join("fastbrowse-mcp");
        std::fs::write(&command, FAKE).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&command, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        Self { dir, command }
    }

    fn server(&self, args: &[&str]) -> McpServer {
        McpServer {
            id: "fb".into(),
            name: "fastbrowse".into(),
            transport: "stdio".into(),
            command: Some(self.command.display().to_string()),
            args: args.iter().map(|a| a.to_string()).collect(),
            url: None,
            env: BTreeMap::from([
                ("OPENROUTER_API_KEY".to_string(), "sk-or-test-secret".to_string()),
                // Any program stands in for Chrome; the fake never starts it
                ("FASTBROWSE_CHROME".to_string(), self.command.display().to_string()),
            ]),
            headers: BTreeMap::new(),
            enabled: true,
            timeout_secs: 30,
            created_at: None,
        }
    }

    fn log(&self) -> Vec<Value> {
        std::fs::read_to_string(self.dir.path().join("calls.jsonl"))
            .unwrap_or_default()
            .lines()
            .map(|l| serde_json::from_str(l).unwrap())
            .collect()
    }
}

/// Allows the call itself, answers fastbrowse's stop with `answer`, and remembers what it was asked.
struct Recording {
    answer: bool,
    asked: Mutex<Vec<ToolApprovalRequest>>,
}

#[async_trait::async_trait]
impl ToolApprover for Recording {
    async fn approve(&self, request: ToolApprovalRequest) -> bool {
        let answer = request.tool != "confirm_step" || self.answer;
        self.asked.lock().unwrap().push(request);
        answer
    }
}

fn ctx(servers: Vec<McpServer>, approver: Arc<dyn ToolApprover>, gateway: Option<GatewayAccess>) -> McpTurnContext {
    McpTurnContext {
        servers,
        approver,
        config_dir: std::env::temp_dir(),
        permission_url: None,
        gateway,
        max_tool_iterations: 8,
        smart_approvals: false,
        records: Default::default(),
    }
}

fn caller() -> Caller {
    Caller { kwant_id: "k1".into(), kwant_name: "Ada".into(), turn_id: "t1".into() }
}

#[tokio::test]
async fn authorize_never_reaches_fastbrowse_and_its_pause_becomes_a_card() {
    for answer in [true, false] {
        let fake = Fake::new();
        let approver = Arc::new(Recording { answer, asked: Mutex::new(Vec::new()) });
        let c = ctx(vec![fake.server(&["--local"])], approver.clone(), None);
        let (session, problems) = McpToolSession::open(&c, None).await;
        assert!(problems.is_empty(), "{problems:?}");

        let args = json!({"task": "Fill in the form and submit it", "start": "https://example.com/form", "authorize": true});
        let out = session.call(&caller(), "c1", "mcp__fastbrowse__browse", &args.to_string()).await;
        session.shutdown().await;

        // The process started without --allow-authorize, and the call lost `authorize`
        let log = fake.log();
        let argv: Vec<&str> = log[0]["argv"].as_array().unwrap().iter().map(|a| a.as_str().unwrap()).collect();
        assert_eq!(argv, ["--local"]);
        assert_eq!(log[0]["key"], true, "the model key reaches fastbrowse through its env");
        assert!(log[1]["call"].get("authorize").is_none(), "{}", log[1]);
        assert_eq!(log[1]["call"]["task"], "Fill in the form and submit it");

        // Two cards: the call, then fastbrowse's stop, under its own name so "always allow"
        // for browse never covers it
        let asked = approver.asked.lock().unwrap();
        assert_eq!(asked.len(), 2);
        assert!(asked[0].arguments.get("authorize").is_none());
        assert_eq!(asked[1].tool_name, "mcp__fastbrowse__confirm_step");
        assert!(matches!(asked[1].smart, Some(SmartVerdict::Ask(_))), "smart approvals never skip it");
        assert_eq!(asked[1].arguments["stopped_at"], "https://example.com/form");
        assert_eq!(asked[1].arguments["fastbrowse_says"], "Stopped before pressing Submit");

        assert_eq!(out.status, "ok");
        let expected = if answer { "The person approved this step" } else { "did not approve it" };
        assert!(out.text.contains(expected), "{}", out.text);
        assert!(!out.text.contains("sk-or-test-secret"));
    }
}

#[tokio::test]
async fn a_finished_task_gets_no_second_card() {
    let fake = Fake::new();
    let approver = Arc::new(Recording { answer: true, asked: Mutex::new(Vec::new()) });
    let c = ctx(vec![fake.server(&["--local"])], approver.clone(), None);
    let (session, _) = McpToolSession::open(&c, None).await;
    let out = session.call(&caller(), "c1", "mcp__fastbrowse__browse", r#"{"task": "Top story on Hacker News?"}"#).await;
    session.shutdown().await;
    assert!(out.text.contains("Top story: Example") && !out.text.contains("kwnt:"), "{}", out.text);
    assert_eq!(approver.asked.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn a_fastbrowse_server_with_allow_authorize_never_starts() {
    let fake = Fake::new();
    for args in [&["--local", "--allow-authorize"][..], &["--local", "--cloud-profile", "work"], &["--headless"]] {
        let err = McpClient::connect(&fake.server(args), None).await.err().expect("must not start").to_string();
        assert!(err.contains("fastbrowse"), "{args:?}: {err}");
    }
    assert!(fake.log().is_empty(), "the process never ran");
}

#[tokio::test]
async fn missing_runtime_and_key_are_named_before_starting() {
    let fake = Fake::new();
    let mut no_uv = fake.server(&["--from", "fastbrowse", "fastbrowse-mcp", "--local"]);
    no_uv.command = Some("/nonexistent/bin/uvx".into());
    let err = McpClient::connect(&no_uv, None).await.err().unwrap().to_string();
    assert!(err.contains("fastbrowse needs uv") && err.contains("astral.sh/uv/install.sh"), "{err}");

    let mut no_key = fake.server(&["--local"]);
    no_key.env.remove("OPENROUTER_API_KEY");
    let err = McpClient::connect(&no_key, None).await.err().unwrap().to_string();
    assert!(err.contains("a model key") && err.contains("OPENROUTER_API_KEY"), "{err}");

    let mut no_chrome = fake.server(&["--local"]);
    no_chrome.env.insert("FASTBROWSE_CHROME".into(), "/nonexistent/chrome".into());
    let err = McpClient::connect(&no_chrome, None).await.err().unwrap().to_string();
    assert!(err.contains("Chrome at `/nonexistent/chrome`"), "{err}");
}

#[test]
fn claude_reaches_fastbrowse_only_through_kwnts_gateway() {
    let fake = Fake::new();
    let gw = GatewayAccess { url: "http://127.0.0.1:7/api/mcp/gateway".into(), token: "gw-tok".into() };
    let approver: Arc<dyn ToolApprover> = Arc::new(kwnt::mcp::FixedApprover(true));
    let config = cli_config::claude_config(&ctx(vec![fake.server(&["--local"])], approver.clone(), Some(gw)));
    let entry = &config["mcpServers"]["fastbrowse"];
    assert_eq!(entry["type"], "http");
    assert_eq!(entry["url"], "http://127.0.0.1:7/api/mcp/gateway/server/fastbrowse");
    assert_eq!(entry["headers"]["Authorization"], "Bearer gw-tok");
    assert!(!config.to_string().contains("sk-or-test-secret"), "the model key stays with kwnt");

    // Without the gateway claude gets no fastbrowse at all
    let config = cli_config::claude_config(&ctx(vec![fake.server(&["--local"])], approver, None));
    assert!(config["mcpServers"].get("fastbrowse").is_none(), "{config}");
}

// ---- Through the server: the scoped gateway, the payment card and the API ----

struct Env {
    app: Router,
    state: AppState,
    general: String,
    bot_id: String,
    _tmp: TempDir,
}

async fn setup() -> Env {
    let tmp = tempdir().unwrap();
    let db = Database::init(tmp.path()).await.unwrap();
    let config = Config { data_dir: tmp.path().to_path_buf(), ..Config::default() };
    let state = AppState::new(
        config.clone(),
        db,
        ToolRunner::new(tmp.path().join("workspace")),
        PermissionBroker::new(),
        LayaRouter::new(config.laya_url.clone()),
    );
    let space = SpaceId::default_space();
    let bot_id = state.db.list_kwants(&space).await.unwrap()[0].id.clone();
    let general = state.db.list_channels(&space).await.unwrap().into_iter().find(|c| c.name == "general").unwrap().id;
    Env { app: create_router(state.clone()), state, general, bot_id, _tmp: tmp }
}

async fn call(e: &Env, method: &str, uri: &str, body: Option<Value>) -> (StatusCode, Value) {
    let req = Request::builder()
        .method(method)
        .uri(uri)
        .header("content-type", "application/json")
        .header("authorization", format!("Bearer {}", e.state.api_token.reveal()));
    let body = body.map(|b| Body::from(b.to_string())).unwrap_or_else(Body::empty);
    let resp = e.app.clone().oneshot(req.body(body).unwrap()).await.unwrap();
    let status = resp.status();
    let bytes = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    (status, serde_json::from_slice(&bytes).unwrap_or(Value::Null))
}

async fn set_approvals(e: &Env, mode: &str) {
    let (s, v) = call(e, "PUT", &format!("/api/kwants/{}/approvals", e.bot_id), Some(json!({"approvals": mode}))).await;
    assert_eq!(s, StatusCode::OK, "{v}");
}

fn approver(e: &Env) -> mcp_runtime::BrokerApprover {
    mcp_runtime::BrokerApprover {
        db: e.state.db.clone(),
        data_dir: e.state.config.data_dir.clone(),
        broker: e.state.broker.clone(),
        events: e.state.event_sender.clone(),
        timeout: Duration::from_millis(200),
        conversation: e.general.clone(),
        chat: None,
        always_ask: false,
    }
}

fn browse(e: &Env, server: &str, tool: &str, arguments: Value) -> ToolApprovalRequest {
    ToolApprovalRequest {
        kwant_id: e.bot_id.clone(),
        kwant_name: "bot".into(),
        turn_id: "t-fb".into(),
        tool_name: format!("mcp__{server}__{tool}"),
        server: server.into(),
        tool: tool.into(),
        arguments,
        builtin: None,
        smart: Some(SmartVerdict::Allow("fastbrowse looked safe".into())),
        payment: None,
    }
}

#[tokio::test]
async fn a_fastbrowse_task_that_pays_gets_the_payment_card_in_every_mode() {
    let e = setup().await;
    let a = approver(&e);
    let paying = [
        json!({"task": "Buy the Sauce Labs Backpack", "start": "https://www.saucedemo.com"}),
        json!({"task": "Go to checkout and finish the order"}),
        json!({"task": "Pay my electricity bill"}),
        json!({"task": "Fill the shipping form", "start": "https://shop.example.com/checkout"}),
        json!({"task": "Enter my details", "fields": {"card_number": "4242424242424242"}}),
    ];
    for mode in ["off", "smart", "ask"] {
        set_approvals(&e, mode).await;
        for args in &paying {
            assert_eq!(a.decide(browse(&e, "fastbrowse", "browse", args.clone())).await, ApprovalOutcome::Unanswered, "{mode} {args}");
        }
        // A renamed server is still caught by its tool name
        assert_eq!(a.decide(browse(&e, "web-agent", "browse", paying[0].clone())).await, ApprovalOutcome::Unanswered, "{mode}");
        // fastbrowse's own stop on a checkout page asks too
        let stop = json!({"task": "Fill the form", "stopped_at": "https://shop.example.com/checkout/payment"});
        assert_eq!(a.decide(browse(&e, "fastbrowse", "confirm_step", stop)).await, ApprovalOutcome::Unanswered, "{mode}");
    }

    // With approvals off a read runs without a card
    set_approvals(&e, "off").await;
    for task in ["What is the top story on Hacker News?", "Find the booking page for booktun.ing and summarise it"] {
        let req = browse(&e, "fastbrowse", "browse", json!({"task": task, "start": "https://news.ycombinator.com"}));
        assert_eq!(a.decide(req).await, ApprovalOutcome::Allowed, "{task}");
    }

    let (_, rows) = call(&e, "GET", "/api/approvals?status=waiting", None).await;
    let reasons: Vec<&str> = rows.as_array().unwrap().iter().filter_map(|r| r["details"]["payment_reason"].as_str()).collect();
    assert_eq!(reasons.len(), 21, "{rows}");
}

#[tokio::test]
async fn the_api_refuses_a_fastbrowse_server_that_could_authorize_or_use_the_cloud() {
    let e = setup().await;
    for (args, env) in [
        (json!(["fastbrowse-mcp", "--local", "--allow-authorize"]), json!({})),
        (json!(["--from", "fastbrowse", "fastbrowse-mcp", "--local", "--transport=http"]), json!({})),
        (json!(["--from", "fastbrowse", "fastbrowse-mcp"]), json!({})),
        (json!(["--from", "fastbrowse", "fastbrowse-mcp", "--local"]), json!({"BROWSER_USE_API_KEY": "bu-x"})),
    ] {
        let body = json!({"name": "fastbrowse", "transport": "stdio", "command": "uvx", "args": args, "env": env});
        let (s, v) = call(&e, "POST", "/api/mcp/servers", Some(body)).await;
        assert_eq!(s, StatusCode::BAD_REQUEST, "{args} {env}: {v}");
    }
    // The preset's own command line is accepted
    let body = json!({"name": "fastbrowse", "transport": "stdio", "command": "uvx",
        "args": ["--from", "fastbrowse", "fastbrowse-mcp", "--local", "--max-dollars", "0.25", "--max-seconds", "540"],
        "env": {"OPENROUTER_API_KEY": "sk-or-saved"}, "timeout_secs": 600});
    let (s, v) = call(&e, "POST", "/api/mcp/servers", Some(body)).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert!(!v.to_string().contains("sk-or-saved"), "the key is masked: {v}");
}

#[tokio::test]
async fn the_scoped_gateway_offers_one_server_by_its_bare_tool_names() {
    let e = setup().await;
    let fake = Fake::new();
    let approver = Arc::new(Recording { answer: true, asked: Mutex::new(Vec::new()) });
    let c = ctx(vec![fake.server(&["--local"])], approver.clone(), None);
    e.state
        .mcp_sessions
        .lock()
        .await
        .insert("fb-tok".into(), PermissionSession::new("k1".into(), "Ada".into(), "t1".into(), c, None));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let app = e.app.clone();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });

    let gw = GatewayAccess { url: format!("http://{addr}/api/mcp/gateway"), token: "fb-tok".into() };
    let server = McpServer {
        id: "gw".into(),
        name: "fastbrowse".into(),
        transport: "http".into(),
        command: None,
        args: vec![],
        url: Some(gw.server_url("fastbrowse")),
        env: BTreeMap::new(),
        headers: BTreeMap::from([("Authorization".to_string(), "Bearer fb-tok".to_string())]),
        enabled: true,
        timeout_secs: 30,
        created_at: None,
    };
    let mut client = McpClient::connect(&server, None).await.unwrap();
    let names: Vec<String> = client.list_tools().await.unwrap().into_iter().map(|t| t.name).collect();
    assert_eq!(names, ["browse"]);
    let out = client.call_tool("browse", json!({"task": "Fill in the form and submit it", "authorize": true})).await.unwrap();
    client.shutdown().await;
    assert!(out.text.contains("The person approved this step"), "{}", out.text);
    assert!(fake.log().iter().all(|l| l.get("call").is_none_or(|c| c.get("authorize").is_none())));
    assert_eq!(approver.asked.lock().unwrap().len(), 2);
    mcp_runtime::finish_turn(&e.state, "fb-tok").await;
}
