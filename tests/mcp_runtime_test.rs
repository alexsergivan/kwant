//! Turn-time MCP: broker approvals, the claude permission endpoint and the claude config file.

use kwnt::{
    broker::{ApprovalDecision, PermissionBroker},
    config::Config,
    db::{models::McpServer, Database},
    db::models::Kwant,
    drivers::{events::RuntimeEvent, CliDriver, Driver, TurnInput},
    mcp::{cli_config, FixedApprover, GatewayAccess, McpClient, McpTurnContext, ToolApprovalRequest, ToolApprover},
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
        gateway: Some(GatewayAccess { url: "http://127.0.0.1:1/api/mcp/gateway".into(), token: "gw-secret-token".into() }),
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
        builtin: None,
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
        conversation: "conv-test".into(),
        chat: None,
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
async fn unanswered_approval_stays_open_as_waiting() {
    let (state, _tmp) = state().await;
    let (h, ticket) = ask(&state, approver(&state, Duration::from_millis(300)), "echo").await;
    assert!(!h.await.unwrap(), "the call doesn't run");
    assert!(!state.broker.is_pending(&ticket).await, "the turn stopped waiting");
    assert_eq!(state.db.get_approval(&ticket).await.unwrap().unwrap().status, "waiting");
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
            PermissionSession::new("k1".into(), "Ada".into(), "t1".into(), c, None),
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
    // claude's own tools follow the built-in policy: Bash asks the approver (here: yes)
    let bash: Value = serde_json::from_str(&client.call_tool("approve", ask("Bash")).await.unwrap().text).unwrap();
    assert_eq!(bash["behavior"], "allow");
    // Servers the kwant doesn't have are refused without asking
    for tool in ["mcp__other__echo", "mcp__kwnt__approve"] {
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
    assert_eq!(allow_records.len(), 2, "only real approval decisions are recorded");
    assert_eq!(allow_records[0]["name"], "mcp__echo__echo");
    assert_eq!(allow_records[0]["status"], "allowed");
    assert_eq!(allow_records[0]["id"], "tu_1");
    assert_eq!(allow_records[1]["name"], "Bash");
    assert_eq!(sessions["deny-tok"].ctx.take_records()[0]["status"], "denied");
    drop(sessions);

    // Unknown or finished turns are not reachable
    assert!(McpClient::connect(&http_server(format!("{base}/api/mcp/permission/gone")), None).await.is_err());
}

fn turn(mcp: Option<McpTurnContext>) -> TurnInput {
    TurnInput {
        continuity: String::new(),
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
        model: None,
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
    // claude's own auto mode would approve tools without asking kwnt
    assert!(out.contains("--permission-mode default"), "{out}");
    assert!(out.contains("MODE: 600"), "{out}");
    assert!(out.contains("TOOL_TIMEOUT: 330000"), "{out}");
    assert!(!out.contains("sk-very-secret") && !out.contains("hdr-secret"), "secrets leaked into argv: {out}");
    assert_eq!(std::fs::read_dir(tmp.path().join("mcp")).unwrap().count(), 0, "config file left behind");

    // Without servers the invocation is unchanged
    let plain = fake_cli_run("claude", None).await;
    assert!(plain.contains("--strict-mcp-config") && !plain.contains("--mcp-config") && !plain.contains("permission-prompt"), "{plain}");
    assert!(plain.contains("--permission-mode default"), "{plain}");
}

/// The servers the fake codex reports for `codex mcp list --json`, one with a quoted name.
const USER_SERVERS: &str = r#"[{"name":"userdummy","enabled":true},{"name":"user.dot \"q\"","enabled":true}]"#;
/// The same plus a user server named like kwnt's gateway.
const USER_SERVERS_WITH_KWNT: &str =
    r#"[{"name":"userdummy","enabled":true},{"name":"user.dot \"q\"","enabled":true},{"name":"kwnt","enabled":true}]"#;

/// Runs a fake codex/grok that logs its argv and whether it got the gateway token env, and
/// returns (argv, token env, thinking notes, reply). `mcp_list` is what it prints for
/// `mcp list --json`; `None` makes that listing fail.
async fn fake_agent_run(kind: &str, mcp: Option<McpTurnContext>, mcp_list: Option<&str>) -> (String, String, Vec<String>, String) {
    let dir = tempdir().unwrap();
    let script = dir.path().join(kind);
    let log = dir.path().join("argv");
    let env_log = dir.path().join("env");
    let list = dir.path().join("list.json");
    let list_cmd = match mcp_list {
        Some(json) => {
            std::fs::write(&list, json).unwrap();
            format!("cat {}; exit 0", list.display())
        }
        None => "exit 3".to_string(),
    };
    std::fs::write(
        &script,
        format!(
            "#!/bin/sh\nif [ \"$1 $2 $3\" = \"mcp list --json\" ]; then {list_cmd}; fi\necho \"$*\" > {}\necho \"${{KWNT_MCP_GATEWAY_TOKEN:-unset}}\" > {}\n",
            log.display(),
            env_log.display()
        ),
    )
    .unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    let driver = CliDriver::new(Some(script.to_string_lossy().into_owned()), Some(kind.into()));
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
    let out = driver.stream_turn(turn(mcp), tx).await.unwrap();
    let notes = drain.await.unwrap();
    let read = |p: &std::path::Path| std::fs::read_to_string(p).unwrap_or_default().trim().to_string();
    (read(&log), read(&env_log), notes, out.text)
}

#[cfg(target_os = "linux")]
#[tokio::test]
async fn codex_cli_reaches_mcp_only_through_kwnts_gateway() {
    let tmp = tempdir().unwrap();
    let (argv, token, notes, _) =
        fake_agent_run("codex", Some(ctx(tmp.path(), Arc::new(FixedApprover(true)))), Some(USER_SERVERS)).await;
    // The user's own servers are switched off by (quoted) name
    assert!(
        argv.contains(r#"-c mcp_servers={kwnt={url="http://127.0.0.1:1/api/mcp/gateway",bearer_token_env_var="KWNT_MCP_GATEWAY_TOKEN",default_tools_approval_mode="approve",startup_timeout_sec=45,tool_timeout_sec=330},"userdummy"={enabled=false},"user.dot \"q\""={enabled=false}} "#),
        "{argv}"
    );
    assert!(argv.contains(r#"-c shell_environment_policy.set.KWNT_MCP_GATEWAY_TOKEN="""#), "token not blanked for codex's shell: {argv}");
    assert_eq!(token, "gw-secret-token", "token comes through the env");
    for secret in ["gw-secret-token", "sk-very-secret", "hdr-secret", "/usr/bin/true", "mcp.example.test"] {
        assert!(!argv.contains(secret), "{secret} in argv: {argv}");
    }
    assert!(!notes.iter().any(|n| n.contains("can't offer MCP tools")), "{notes:?}");

    // Without servers (none assigned, or the kwant opted out) codex still gets none of the user's
    let (plain, token, _, _) = fake_agent_run("codex", None, Some(USER_SERVERS)).await;
    assert!(plain.contains(r#"-c mcp_servers={"userdummy"={enabled=false},"user.dot \"q\""={enabled=false}} "#), "{plain}");
    // The model's shell is limited to core env on every run, gateway or not
    for run in [&argv, &plain] {
        assert!(
            run.contains(r#"-c shell_environment_policy.inherit="core" -c shell_environment_policy.ignore_default_excludes=false "#),
            "{run}"
        );
    }
    assert!(!plain.contains("shell_environment_policy.set"), "{plain}");
    assert_eq!(token, "unset");

    // And with no user servers either, the override is empty, as before
    let (bare, _, _, _) = fake_agent_run("codex", None, Some("[]")).await;
    assert!(bare.contains("-c mcp_servers={} "), "{bare}");
}

/// A user codex server named `kwnt` would merge with the gateway's entry: without a gateway it is
/// switched off like any other, with one the turn is refused rather than run.
#[cfg(target_os = "linux")]
#[tokio::test]
async fn codex_user_server_named_kwnt_is_disabled_or_refused() {
    let (plain, token, _, _) = fake_agent_run("codex", None, Some(USER_SERVERS_WITH_KWNT)).await;
    assert!(
        plain.contains(r#"-c mcp_servers={"userdummy"={enabled=false},"user.dot \"q\""={enabled=false},"kwnt"={enabled=false}} "#),
        "{plain}"
    );
    assert_eq!(token, "unset");

    let tmp = tempdir().unwrap();
    let (argv, token, _, reply) =
        fake_agent_run("codex", Some(ctx(tmp.path(), Arc::new(FixedApprover(true)))), Some(USER_SERVERS_WITH_KWNT)).await;
    assert_eq!(argv, "", "codex exec ran: {argv}");
    assert_eq!(token, "", "codex exec got the env: {token}");
    assert!(reply.contains("MCP server named `kwnt`") && reply.contains("Rename that server"), "{reply}");
    assert!(!reply.contains("gw-secret-token"), "{reply}");
}

#[cfg(target_os = "linux")]
#[tokio::test]
async fn codex_cli_does_not_run_when_its_servers_cannot_be_listed() {
    let tmp = tempdir().unwrap();
    for mcp in [Some(ctx(tmp.path(), Arc::new(FixedApprover(true)))), None] {
        for list in [None, Some("not json"), Some(r#"[{"enabled":true}]"#)] {
            let (argv, _, _, reply) = fake_agent_run("codex", mcp.clone(), list).await;
            assert_eq!(argv, "", "codex exec ran: {argv}");
            assert!(reply.contains("didn't run codex"), "{reply}");
        }
    }
}

#[cfg(target_os = "linux")]
#[tokio::test]
async fn grok_cli_reaches_mcp_only_through_kwnts_gateway() {
    let tmp = tempdir().unwrap();
    let (argv, token, notes, _) = fake_agent_run("grok", Some(ctx(tmp.path(), Arc::new(FixedApprover(true)))), Some(USER_SERVERS)).await;
    assert_eq!(token, "gw-secret-token", "token comes through the env");
    assert!(argv.contains("--allow MCPTool(kwnt__*) --allow WebFetch --output-format streaming-json"), "{argv}");
    for secret in ["gw-secret-token", "sk-very-secret", "hdr-secret", "/usr/bin/true", "mcp.example.test"] {
        assert!(!argv.contains(secret), "{secret} in argv: {argv}");
    }
    assert!(!notes.iter().any(|n| n.contains("can't offer MCP tools")), "{notes:?}");
}

/// A cancelled turn aborts its driver task; the agent CLI must die with it rather than keep
/// running (and keep calling kwnt's gateway).
#[cfg(target_os = "linux")]
#[tokio::test]
async fn aborting_a_cli_turn_kills_the_cli() {
    let dir = tempdir().unwrap();
    let script = dir.path().join("codex");
    let pid_file = dir.path().join("pid");
    std::fs::write(
        &script,
        format!("#!/bin/sh\nif [ \"$1\" = mcp ]; then echo '[]'; exit 0; fi\necho $$ > {}\nexec sleep 60\n", pid_file.display()),
    )
    .unwrap();
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    let driver = CliDriver::new(Some(script.to_string_lossy().into_owned()), Some("codex".into()));
    let (tx, mut rx) = tokio::sync::mpsc::channel(64);
    tokio::spawn(async move { while rx.recv().await.is_some() {} });
    let task = tokio::spawn(async move { driver.stream_turn(turn(None), tx).await });

    let pid = loop {
        if let Some(pid) = std::fs::read_to_string(&pid_file).ok().filter(|p| p.ends_with('\n')) {
            break pid.trim().to_string();
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    };
    task.abort();
    let _ = task.await;
    let alive = || std::fs::read_to_string(format!("/proc/{pid}/stat")).is_ok_and(|s| !s.contains(") Z "));
    for _ in 0..100 {
        if !alive() {
            return;
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    panic!("codex (pid {pid}) outlived its aborted turn");
}
