//! Turn-time MCP: broker approvals, the claude permission endpoint and the claude config file.

use kwnt::{
    broker::{ApprovalDecision, PermissionBroker},
    config::Config,
    db::{models::McpServer, Database},
    db::models::Kwant,
    drivers::{events::RuntimeEvent, CliDriver, Driver, TurnInput},
    mcp::{cli_config, FixedApprover, McpClient, McpTurnContext, ToolApprovalRequest, ToolApprover},
    router::LayaRouter,
    server::{create_router, mcp_runtime::{BrokerApprover, PermissionSession}, AppState},
    tools::ToolRunner,
};
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;
use tempfile::{tempdir, TempDir};

fn server(name: &str) -> McpServer {
    McpServer {
        id: format!("id-{name}"),
        name: name.into(),
        transport: "stdio".into(),
        command: Some("/usr/bin/true".into()),
        args: vec!["--flag".into()],
        url: None,
        env: BTreeMap::from([("API_TOKEN".to_string(), "sk-very-secret".to_string())]),
        headers: BTreeMap::new(),
        enabled: true,
        timeout_secs: 30,
        created_at: None,
    }
}

fn ctx(dir: &std::path::Path, approver: Arc<dyn ToolApprover>) -> McpTurnContext {
    let mut http = server("remote");
    http.transport = "http".into();
    http.command = None;
    http.args = vec![];
    http.env = BTreeMap::new();
    http.url = Some("https://mcp.example.test/mcp".into());
    http.headers = BTreeMap::from([("Authorization".to_string(), "Bearer hdr-secret".to_string())]);
    McpTurnContext {
        servers: vec![server("echo"), http],
        approver,
        config_dir: dir.join("mcp"),
        permission_url: Some("http://127.0.0.1:1/api/mcp/permission/tok".into()),
        max_tool_iterations: 8,
        records: Default::default(),
    }
}

async fn state() -> (AppState, TempDir) {
    let tmp = tempdir().unwrap();
    let db = Database::init(tmp.path()).await.unwrap();
    let config = Config { data_dir: tmp.path().to_path_buf(), browser_cdp_url: Some("http://127.0.0.1:9".into()), ..Config::default() };
    let state = AppState::new(
        config.clone(),
        db,
        ToolRunner::new(tmp.path().join("workspace")),
        PermissionBroker::new(),
        LayaRouter::new(config.laya_url.clone()),
    );
    (state, tmp)
}

fn request(tool: &str) -> ToolApprovalRequest {
    ToolApprovalRequest {
        kwant_id: "k1".into(),
        kwant_name: "Ada".into(),
        turn_id: "t1".into(),
        tool_name: format!("mcp__echo__{tool}"),
        server: "echo".into(),
        tool: tool.into(),
        arguments: json!({"text": "hi"}),
    }
}

/// Starts an approval and returns (join handle, ticket id from the ApprovalRequired event).
async fn ask(state: &AppState, approver: Arc<BrokerApprover>, tool: &str) -> (tokio::task::JoinHandle<bool>, String) {
    let mut events = state.event_sender.subscribe();
    let req = request(tool);
    let handle = tokio::spawn(async move { approver.approve(req).await });
    let ticket = loop {
        match tokio::time::timeout(Duration::from_secs(5), events.recv()).await.unwrap().unwrap() {
            RuntimeEvent::ApprovalRequired { id, tool_name, details } => {
                assert_eq!(tool_name, format!("mcp__echo__{tool}"));
                assert_eq!(details["source"], "mcp");
                assert_eq!(details["server"], "echo");
                assert_eq!(details["arguments"]["text"], "hi");
                break id;
            }
            _ => continue,
        }
    };
    (handle, ticket)
}

fn approver(state: &AppState, timeout: Duration) -> Arc<BrokerApprover> {
    Arc::new(BrokerApprover {
        db: state.db.clone(),
        broker: state.broker.clone(),
        events: state.event_sender.clone(),
        timeout,
    })
}

#[tokio::test]
async fn broker_approver_follows_user_decisions() {
    let (state, _tmp) = state().await;
    let a = approver(&state, Duration::from_secs(10));

    let (h, ticket) = ask(&state, a.clone(), "echo").await;
    assert!(state.broker.resolve(&ticket, ApprovalDecision::Deny).await);
    assert!(!h.await.unwrap(), "denied by broker");

    let (h, ticket) = ask(&state, a.clone(), "echo").await;
    assert_eq!(state.db.get_approval(&ticket).await.unwrap().unwrap().status, "pending");
    state.broker.resolve(&ticket, ApprovalDecision::AllowOnce).await;
    assert!(h.await.unwrap());

    // "Always allow" sticks for this kwant and tool, without another card
    let (h, ticket) = ask(&state, a.clone(), "add").await;
    state.broker.resolve(&ticket, ApprovalDecision::AlwaysAllowSession).await;
    assert!(h.await.unwrap());
    let mut req = request("add");
    req.arguments = json!({"a": 1, "b": 2});
    assert!(tokio::time::timeout(Duration::from_secs(1), a.approve(req)).await.unwrap());
    let mut other_kwant = request("add");
    other_kwant.kwant_id = "k2".into();
    assert!(tokio::time::timeout(Duration::from_millis(300), a.approve(other_kwant)).await.is_err(), "k2 must still ask");
}

#[tokio::test]
async fn unanswered_approval_expires_as_denied() {
    let (state, _tmp) = state().await;
    let (h, ticket) = ask(&state, approver(&state, Duration::from_millis(300)), "echo").await;
    assert!(!h.await.unwrap());
    assert!(!state.broker.is_pending(&ticket).await);
    assert_eq!(state.db.get_approval(&ticket).await.unwrap().unwrap().status, "expired");
}

#[tokio::test]
async fn aborted_turn_withdraws_its_ticket() {
    let (state, _tmp) = state().await;
    let (h, ticket) = ask(&state, approver(&state, Duration::from_secs(30)), "echo").await;
    h.abort();
    let _ = h.await;
    for _ in 0..50 {
        if !state.broker.is_pending(&ticket).await {
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert!(!state.broker.is_pending(&ticket).await);
}

#[tokio::test]
async fn claude_config_file_is_private_and_removed() {
    let tmp = tempdir().unwrap();
    let c = ctx(tmp.path(), Arc::new(FixedApprover(true)));
    let path;
    {
        let file = cli_config::write_claude_config(&c).unwrap();
        path = file.path.clone();
        assert!(path.starts_with(tmp.path().join("mcp")));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(std::fs::metadata(&path).unwrap().permissions().mode() & 0o777, 0o600);
            assert_eq!(std::fs::metadata(tmp.path().join("mcp")).unwrap().permissions().mode() & 0o777, 0o700);
        }
        let doc: Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        let servers = &doc["mcpServers"];
        assert_eq!(servers["echo"]["type"], "stdio");
        assert_eq!(servers["echo"]["command"], "/usr/bin/true");
        assert_eq!(servers["echo"]["env"]["API_TOKEN"], "sk-very-secret");
        assert_eq!(servers["remote"]["type"], "http");
        assert_eq!(servers["remote"]["headers"]["Authorization"], "Bearer hdr-secret");
        assert_eq!(servers["kwnt"]["url"], "http://127.0.0.1:1/api/mcp/permission/tok");
        // A second turn gets its own file
        let second = cli_config::write_claude_config(&c).unwrap();
        assert_ne!(second.path, path);
    }
    assert!(!path.exists(), "config file must be removed after the turn");
}

async fn serve(state: AppState) -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, create_router(state)).await.unwrap() });
    format!("http://{addr}")
}

fn http_server(url: String) -> McpServer {
    McpServer {
        id: "kwnt".into(),
        name: "kwnt".into(),
        transport: "http".into(),
        command: None,
        args: vec![],
        url: Some(url),
        env: BTreeMap::new(),
        headers: BTreeMap::new(),
        enabled: true,
        timeout_secs: 10,
        created_at: None,
    }
}

#[tokio::test]
async fn permission_endpoint_answers_claude_prompts_over_http() {
    let (state, tmp) = state().await;
    let base = serve(state.clone()).await;

    for (token, allow) in [("allow-tok", true), ("deny-tok", false)] {
        let c = ctx(tmp.path(), Arc::new(FixedApprover(allow)));
        state.mcp_sessions.lock().await.insert(
            token.into(),
            PermissionSession { kwant_id: "k1".into(), kwant_name: "Ada".into(), turn_id: "t1".into(), ctx: c },
        );
    }

    // kwnt's own HTTP MCP client doubles as the streamable-HTTP test client
    let mut client = McpClient::connect(&http_server(format!("{base}/api/mcp/permission/allow-tok")), None).await.unwrap();
    assert_eq!(client.server_info["name"], "kwnt");
    let tools = client.list_tools().await.unwrap();
    assert_eq!(tools.len(), 1);
    assert_eq!(tools[0].name, "approve");

    let ask = |tool: &str| json!({"tool_name": tool, "input": {"text": "hi"}, "tool_use_id": "tu_1"});
    let allowed: Value = serde_json::from_str(&client.call_tool("approve", ask("mcp__echo__echo")).await.unwrap().text).unwrap();
    assert_eq!(allowed, json!({"behavior": "allow", "updatedInput": {"text": "hi"}}));
    // Built-in tools and servers the kwant doesn't have are refused without asking
    for tool in ["Bash", "mcp__other__echo", "mcp__kwnt__approve"] {
        let d: Value = serde_json::from_str(&client.call_tool("approve", ask(tool)).await.unwrap().text).unwrap();
        assert_eq!(d["behavior"], "deny", "{tool}");
    }
    assert!(client.call_tool("nope", json!({})).await.is_err());
    client.shutdown().await;

    let mut denied = McpClient::connect(&http_server(format!("{base}/api/mcp/permission/deny-tok")), None).await.unwrap();
    let d: Value = serde_json::from_str(&denied.call_tool("approve", ask("mcp__echo__add")).await.unwrap().text).unwrap();
    assert_eq!(d["behavior"], "deny");
    denied.shutdown().await;

    let sessions = state.mcp_sessions.lock().await;
    let allow_records = sessions["allow-tok"].ctx.take_records();
    assert_eq!(allow_records.len(), 1, "only real approval decisions are recorded");
    assert_eq!(allow_records[0]["name"], "mcp__echo__echo");
    assert_eq!(allow_records[0]["status"], "allowed");
    assert_eq!(allow_records[0]["id"], "tu_1");
    assert_eq!(sessions["deny-tok"].ctx.take_records()[0]["status"], "denied");
    drop(sessions);

    // Unknown or finished turns are not reachable
    assert!(McpClient::connect(&http_server(format!("{base}/api/mcp/permission/gone")), None).await.is_err());
}

fn turn(mcp: Option<McpTurnContext>) -> TurnInput {
    TurnInput {
        browser_api_url: None,
        thread_id: "t1".into(),
        kwant: Kwant {
            id: "k1".into(),
            name: "Ada".into(),
            title: "Lead".into(),
            persona_prompt: "p".into(),
            soul: String::new(),
            avatar_sprite: "s".into(),
            driver_type: "cli".into(),
            preferred_model: "claude".into(),
            temperature: 0.7,
            max_tokens: 1000,
            level: 1,
            turns_completed: 0,
            created_at: None,
        },
        prompt: "hi".into(),
        history: vec![],
        model_override: None,
        effort_level: Some("low".into()),
        team_context: None,
        workspace_dir: None,
        sandbox_id: None,
        teammates: vec![],
        mcp,
    }
}

/// Runs the CLI driver against a fake CLI that prints its argv, the config file's mode and
/// MCP_TOOL_TIMEOUT, so the real invocation can be checked without the real CLI.
async fn fake_cli_run(kind: &str, mcp: Option<McpTurnContext>) -> String {
    let dir = tempdir().unwrap();
    let script = dir.path().join("fake-cli");
    std::fs::write(
        &script,
        "#!/bin/sh\necho \"ARGS: $*\"\nwhile [ $# -gt 0 ]; do\n  if [ \"$1\" = --mcp-config ]; then echo \"MODE: $(stat -c %a \"$2\")\"; fi\n  shift\ndone\necho \"TOOL_TIMEOUT: $MCP_TOOL_TIMEOUT\"\n",
    )
    .unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    let driver = CliDriver::new(Some(script.to_string_lossy().into_owned()), Some(kind.into()));
    let (tx, mut rx) = tokio::sync::mpsc::channel(64);
    let drain = tokio::spawn(async move { while rx.recv().await.is_some() {} });
    let out = driver.stream_turn(turn(mcp), tx).await.unwrap();
    drain.await.unwrap();
    out.text
}

#[cfg(target_os = "linux")]
#[tokio::test]
async fn claude_cli_gets_mcp_config_without_secrets_in_argv() {
    let tmp = tempdir().unwrap();
    let c = ctx(tmp.path(), Arc::new(FixedApprover(true)));
    let out = fake_cli_run("claude", Some(c)).await;
    assert!(out.contains("--strict-mcp-config"), "{out}");
    assert!(out.contains(&format!("--mcp-config {}", tmp.path().join("mcp").display())), "{out}");
    assert!(out.contains("--permission-prompt-tool mcp__kwnt__approve"), "{out}");
    assert!(out.contains("MODE: 600"), "{out}");
    assert!(out.contains("TOOL_TIMEOUT: 330000"), "{out}");
    assert!(!out.contains("sk-very-secret") && !out.contains("hdr-secret"), "secrets leaked into argv: {out}");
    assert_eq!(std::fs::read_dir(tmp.path().join("mcp")).unwrap().count(), 0, "config file left behind");

    // Without servers the invocation is unchanged
    let plain = fake_cli_run("claude", None).await;
    assert!(plain.contains("--strict-mcp-config") && !plain.contains("--mcp-config") && !plain.contains("permission-prompt"), "{plain}");
}

#[cfg(target_os = "linux")]
#[tokio::test]
async fn codex_cli_runs_with_mcp_disabled() {
    let tmp = tempdir().unwrap();
    // codex prints its answer to an -o file, so this fake logs its argv to a file instead
    let dir = tempdir().unwrap();
    let script = dir.path().join("codex");
    let log = dir.path().join("argv");
    std::fs::write(&script, format!("#!/bin/sh\necho \"$*\" > {}\n", log.display())).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    let driver = CliDriver::new(Some(script.to_string_lossy().into_owned()), Some("codex".into()));
    let (tx, mut rx) = tokio::sync::mpsc::channel(64);
    let drain = tokio::spawn(async move {
        let mut notes = Vec::new();
        while let Some(ev) = rx.recv().await {
            if let RuntimeEvent::ThinkingDelta(t) = ev {
                notes.push(t);
            }
        }
        notes
    });
    driver.stream_turn(turn(Some(ctx(tmp.path(), Arc::new(FixedApprover(true))))), tx).await.unwrap();
    let notes = drain.await.unwrap();
    let argv = std::fs::read_to_string(&log).unwrap();
    assert!(argv.contains("-c mcp_servers={}"), "{argv}");
    assert!(notes.iter().any(|n| n.contains("can't offer MCP tools through the codex CLI")), "{notes:?}");
}
