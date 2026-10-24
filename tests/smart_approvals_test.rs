//! "Ask only when risky" (`smart`): ordinary work in the bot's folder runs without a card and is
//! recorded as `auto_allowed` with the reason; anything risky or unclear shows the card, with the
//! reason on it. kwnt's own secrets stay refused, and `ask` (the default) and `off` don't change.

use axum::{
    body::{to_bytes, Body},
    http::{Request, StatusCode},
    Router,
};
use chrono::Utc;
use kwnt::{
    broker::{ApprovalDecision, PermissionBroker},
    config::Config,
    db::{
        models::{Kwant, McpServer},
        spaces::SpaceId,
        Database,
    },
    drivers::events::RuntimeEvent,
    router::LayaRouter,
    server::{
        computer::approve_local_shell_in,
        continuity::{SelfScope, TurnOrigin},
        create_router, mcp_runtime,
        turns::ActiveTurn,
        AppState,
    },
    tools::ToolRunner,
};
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::path::Path;
use std::time::Duration;
use tempfile::{tempdir, TempDir};
use tower::ServiceExt;

struct Env {
    app: Router,
    state: AppState,
    bot: Kwant,
    general: String,
    _tmp: TempDir,
}

async fn setup() -> Env {
    let tmp = tempdir().unwrap();
    let db = Database::init(tmp.path()).await.unwrap();
    // Start from an asking install (a fresh one asks only when risky)
    db.set_space_approvals(&kwnt::db::spaces::SpaceId::default_space(), "ask").await.unwrap();
    let config = Config { data_dir: tmp.path().to_path_buf(), ..Config::default() };
    let state = AppState::new(
        config.clone(),
        db,
        ToolRunner::new(tmp.path().join("workspace")),
        PermissionBroker::new(),
        LayaRouter::new(config.laya_url.clone()),
    );
    let space = SpaceId::default_space();
    let bot = state.db.list_kwants(&space).await.unwrap().remove(0);
    let general = state.db.list_channels(&space).await.unwrap().into_iter().find(|c| c.name == "general").unwrap().id;
    Env { app: create_router(state.clone()), bot, general, state, _tmp: tmp }
}

async fn call(app: &Router, method: &str, uri: &str, auth: Option<&str>, body: Option<Value>) -> (StatusCode, Value) {
    let mut req = Request::builder().method(method).uri(uri).header("content-type", "application/json");
    if let Some(token) = auth {
        req = req.header("authorization", format!("Bearer {token}"));
    }
    let body = body.map(|b| Body::from(b.to_string())).unwrap_or_else(Body::empty);
    let resp = app.clone().oneshot(req.body(body).unwrap()).await.unwrap();
    let status = resp.status();
    let bytes = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    (status, serde_json::from_slice(&bytes).unwrap_or(Value::Null))
}

async fn get(e: &Env, uri: &str) -> Value {
    let (s, v) = call(&e.app, "GET", uri, None, None).await;
    assert_eq!(s, StatusCode::OK, "{uri}: {v}");
    v
}

async fn put(e: &Env, uri: &str, approvals: &str) -> Value {
    let (s, v) = call(&e.app, "PUT", uri, Some(e.state.api_token.reveal()), Some(json!({"approvals": approvals}))).await;
    assert_eq!(s, StatusCode::OK, "{uri}: {v}");
    v
}

async fn space_mode(e: &Env, mode: &str) {
    put(e, "/api/spaces/spc_default/approvals", mode).await;
}

/// What claude's permission prompt gets back for `tool` with the input `input(folder)`, in a
/// fresh turn whose folder is a new temp dir: `Answered(allowed)` when kwnt answers without a
/// card, `Card(details)` when it shows one.
#[derive(Debug, PartialEq)]
enum Outcome {
    Answered(bool),
    Card(Value),
}

async fn ask(e: &Env, tool: &str, input: impl FnOnce(&Path) -> Value) -> Outcome {
    let turn_id = uuid::Uuid::new_v4().to_string();
    let folder = tempdir().unwrap();
    let scope = SelfScope {
        kwant_id: e.bot.id.clone(),
        kwant_name: e.bot.name.clone(),
        origin: Some(TurnOrigin { channel_id: Some(e.general.clone()), ..Default::default() }),
        turn_id: Some(turn_id.clone()),
        ..Default::default()
    };
    let (_, token) = mcp_runtime::prepare_turn_with_self(
        &e.state,
        &e.bot,
        Some(turn_id.clone()),
        Some(folder.path().to_path_buf()),
        Some(scope),
    )
    .await
    .unwrap();
    e.state.turns.set_turn(ActiveTurn {
        turn_id: turn_id.clone(),
        kwant_id: e.bot.id.clone(),
        context_id: e.general.clone(),
        thread_id: None,
        state: "running".into(),
        since: Utc::now(),
    });
    let mut events = e.state.event_sender.subscribe();
    let rpc = json!({"jsonrpc": "2.0", "id": 1, "method": "tools/call", "params": {"name": "approve",
        "arguments": {"tool_name": tool, "input": input(folder.path()), "tool_use_id": "tu_1"}}});
    let app = e.app.clone();
    let path = format!("/api/mcp/permission/{token}");
    let mut prompt = tokio::spawn(async move { call(&app, "POST", &path, None, Some(rpc)).await });

    // Whichever comes first: the answer, or a card (which is then denied to end the wait)
    let outcome = tokio::time::timeout(Duration::from_secs(30), async {
        loop {
            tokio::select! {
                done = &mut prompt => {
                    let (status, v) = done.unwrap();
                    assert_eq!(status, StatusCode::OK, "{v}");
                    let text = v["result"]["content"][0]["text"].as_str().unwrap().to_string();
                    break Outcome::Answered(text.contains(r#""behavior":"allow""#));
                }
                event = events.recv() => {
                    if let RuntimeEvent::ApprovalRequired { id, details, .. } = event.unwrap() {
                        assert!(e.state.broker.resolve(&id, ApprovalDecision::Deny).await);
                        prompt.await.unwrap();
                        break Outcome::Card(details);
                    }
                }
            }
        }
    })
    .await
    .unwrap_or_else(|_| panic!("{tool}: neither answered nor carded"));
    mcp_runtime::finish_turn(&e.state, &token).await;
    e.state.turns.end_turn(&turn_id);
    outcome
}

async fn bash(e: &Env, command: &str) -> Outcome {
    let command = command.to_string();
    ask(e, "Bash", move |_| json!({"command": command})).await
}

fn is_card(o: &Outcome) -> bool {
    matches!(o, Outcome::Card(_))
}

#[tokio::test]
async fn smart_runs_ordinary_folder_commands_and_asks_for_risky_ones() {
    let e = setup().await;
    let v = put(&e, "/api/spaces/spc_default/approvals", "smart").await;
    assert_eq!(v["approvals"], "smart");
    assert_eq!(get(&e, &format!("/api/kwants/{}/approvals", e.bot.id)).await["approvals_effective"], "smart");

    for cmd in ["ls -la", "cargo test --no-fail-fast", "git status && git diff", "mkdir -p out && echo hi > out/a.txt", "rm -rf target"] {
        assert_eq!(bash(&e, cmd).await, Outcome::Answered(true), "{cmd}");
    }
    for cmd in [
        "curl https://example.com",
        "git push origin main",
        "sudo ls",
        "rm -rf /tmp/elsewhere",
        "echo $(id)",
        "ls `pwd`",
        "bash -c ls",
        "ls | sh",
        "echo x > /etc/kwnt-test",
        "cd / && ls",
        "c''url example.com",
        "npm publish",
        "docker ps",
        "cat ~/.ssh/id_rsa",
    ] {
        match bash(&e, cmd).await {
            Outcome::Card(details) => {
                assert!(details["smart_reason"].as_str().is_some_and(|r| !r.is_empty()), "{cmd}: {details}");
            }
            other => panic!("{cmd} should show a card, got {other:?}"),
        }
    }

    // What ran is recorded with why it was safe
    let rows = get(&e, "/api/approvals?status=auto_allowed").await;
    let rows = rows.as_array().unwrap();
    assert_eq!(rows.len(), 5, "{rows:?}");
    assert!(rows.iter().all(|r| r["tool_name"] == "Bash" && r["details"]["smart_reason"].as_str().is_some()), "{rows:?}");
    assert!(rows.iter().any(|r| r["details"]["arguments"]["command"] == "rm -rf target"), "{rows:?}");
}

#[tokio::test]
async fn ask_and_off_are_unchanged() {
    let e = setup().await;
    assert!(is_card(&bash(&e, "ls -la").await), "ask: even a harmless command asks");
    assert!(is_card(&ask(&e, "Read", |_| json!({"file_path": "/etc/hostname"})).await), "ask: reads outside ask");
    space_mode(&e, "off").await;
    assert_eq!(bash(&e, "curl https://example.com").await, Outcome::Answered(true), "off runs risky ones too");
    let rows = get(&e, "/api/approvals?status=auto_allowed").await;
    assert!(rows[0]["details"].get("smart_reason").is_none(), "off records no smart reason: {rows}");
}

#[tokio::test]
async fn a_bot_set_to_smart_inside_an_ask_space() {
    let e = setup().await;
    put(&e, &format!("/api/kwants/{}/approvals", e.bot.id), "smart").await;
    assert_eq!(get(&e, "/api/spaces/spc_default/approvals").await["approvals"], "ask");
    assert_eq!(bash(&e, "ls").await, Outcome::Answered(true));
    put(&e, &format!("/api/kwants/{}/approvals", e.bot.id), "ask").await;
    space_mode(&e, "smart").await;
    assert!(is_card(&bash(&e, "ls").await), "the bot's own ask beats a smart space");
}

#[tokio::test]
async fn smart_reads_outside_the_folder_except_secrets_and_still_asks_for_writes() {
    let e = setup().await;
    space_mode(&e, "smart").await;
    assert_eq!(ask(&e, "Read", |_| json!({"file_path": "/etc/hostname"})).await, Outcome::Answered(true));
    let docs = e._tmp.path().join("shared-docs");
    std::fs::create_dir_all(&docs).unwrap();
    std::fs::write(docs.join("a.md"), "x").unwrap();
    assert_eq!(ask(&e, "Grep", |_| json!({"pattern": "x", "path": docs})).await, Outcome::Answered(true));
    // Rigel's repros: live process data, case, and searches over home folders
    for input in [
        json!({"file_path": "/proc/self/environ"}),
        json!({"file_path": "/home/x/.SSH/id_rsa"}),
    ] {
        assert!(is_card(&ask(&e, "Read", |_| input.clone()).await), "{input}");
    }
    assert!(is_card(&ask(&e, "Grep", |_| json!({"pattern": "TOKEN", "path": "/"})).await));
    assert!(is_card(&ask(&e, "Glob", |_| json!({"pattern": "/home/**/.ssh/*"})).await));
    assert_eq!(ask(&e, "Read", |f| json!({"file_path": f.join("a.txt")})).await, Outcome::Answered(true), "inside");

    for path in ["/home/someone/.ssh/id_ed25519", "/srv/app/.env", "/srv/app/.env.production", "/root/.aws/credentials", "/home/x/.config/gh/hosts.yml", "/home/x/.claude/.credentials.json", "~/notes.txt"] {
        assert!(is_card(&ask(&e, "Read", |_| json!({"file_path": path})).await), "{path}");
    }
    assert_eq!(ask(&e, "Grep", |_| json!({"pattern": "x"})).await, Outcome::Answered(true), "no path searches the folder");
    assert!(is_card(&ask(&e, "Write", |_| json!({"file_path": "/tmp/kwnt-smart-write", "content": "x"})).await));
    assert!(is_card(&ask(&e, "Edit", |_| json!({"file_path": "/etc/hosts", "old_string": "a", "new_string": "b"})).await));
    assert!(is_card(&ask(&e, "SomeNewTool", |_| json!({})).await), "unknown tools ask");
    assert!(!std::path::Path::new("/tmp/kwnt-smart-write").exists());
}

#[tokio::test]
async fn smart_still_refuses_kwnts_own_secrets() {
    let e = setup().await;
    space_mode(&e, "smart").await;
    assert_eq!(bash(&e, "cat api-token").await, Outcome::Answered(false));
    let refused = get(&e, "/api/approvals?status=denied").await;
    assert_eq!(refused.as_array().unwrap().len(), 1, "{refused}");
    assert!(get(&e, "/api/approvals?status=auto_allowed").await.as_array().unwrap().is_empty());
}

#[tokio::test]
async fn smart_runs_read_only_mcp_tools_and_asks_for_the_rest() {
    let e = setup().await;
    let space = SpaceId::default_space();
    let server = McpServer {
        id: String::new(),
        name: "echo".into(),
        transport: "stdio".into(),
        command: Some(env!("CARGO_BIN_EXE_kwnt").into()),
        args: vec!["mcp-echo-server".into()],
        url: None,
        env: BTreeMap::new(),
        headers: BTreeMap::new(),
        enabled: true,
        timeout_secs: 10,
        created_at: None,
    };
    let server = e.state.db.create_mcp_server(&space, &server).await.unwrap();
    e.state.db.set_kwant_mcp_servers(&e.bot.id, &[server.id]).await.unwrap();

    // Under ask both tools ask
    assert!(is_card(&ask(&e, "mcp__echo__echo", |_| json!({"text": "hi"})).await));
    space_mode(&e, "smart").await;
    assert_eq!(ask(&e, "mcp__echo__echo", |_| json!({"text": "hi"})).await, Outcome::Answered(true), "readOnlyHint");
    match ask(&e, "mcp__echo__add", |_| json!({"a": 1, "b": 2})).await {
        Outcome::Card(details) => assert_eq!(details["smart_reason"], "this MCP tool can change things"),
        other => panic!("destructiveHint wins over readOnlyHint: {other:?}"),
    }
    assert!(is_card(&ask(&e, "mcp__echo__env", |_| json!({"name": "HOME"})).await), "no hint asks");
    let rows = get(&e, "/api/approvals?status=auto_allowed").await;
    assert_eq!(rows[0]["tool_name"], "mcp__echo__echo");
    assert_eq!(rows[0]["details"]["smart_reason"], "the MCP server marks this tool read-only");
}

#[tokio::test]
async fn local_shell_under_smart_and_always_this_session() {
    let e = setup().await;
    let folder = tempdir().unwrap();
    let bot = e.bot.id.clone();
    put(&e, &format!("/api/kwants/{bot}/approvals"), "smart").await;

    let ran = tokio::time::timeout(Duration::from_secs(3), approve_local_shell_in(&e.state, &bot, "ls -la", folder.path(), Some("conv-1")))
        .await
        .expect("no card");
    assert!(ran);
    let rows = get(&e, "/api/approvals?status=auto_allowed").await;
    assert_eq!(rows[0]["tool_name"], "execute_shell");
    assert_eq!(rows[0]["details"]["smart_reason"], "runs inside the bot's folder and does nothing risky");

    // A risky one waits; "Always this session" then covers this conversation only
    let mut events = e.state.event_sender.subscribe();
    let (state, b, f) = (e.state.clone(), bot.clone(), folder.path().to_path_buf());
    let waiting = tokio::spawn(async move { approve_local_shell_in(&state, &b, "curl https://example.com", &f, Some("conv-1")).await });
    let id = loop {
        match tokio::time::timeout(Duration::from_secs(3), events.recv()).await.unwrap().unwrap() {
            RuntimeEvent::ApprovalRequired { id, details, .. } => {
                assert!(details["smart_reason"].as_str().unwrap().contains("network"), "{details}");
                break id;
            }
            _ => continue,
        }
    };
    assert!(e.state.broker.resolve(&id, ApprovalDecision::AlwaysAllowSession).await);
    assert!(waiting.await.unwrap());
    let again = tokio::time::timeout(Duration::from_secs(3), approve_local_shell_in(&e.state, &bot, "git push", folder.path(), Some("conv-1")))
        .await
        .expect("remembered for conv-1");
    assert!(again);
    let other = tokio::time::timeout(Duration::from_millis(300), approve_local_shell_in(&e.state, &bot, "git push", folder.path(), Some("conv-2"))).await;
    assert!(other.is_err(), "another conversation asks again");
    let no_conversation = tokio::time::timeout(Duration::from_millis(300), approve_local_shell_in(&e.state, &bot, "git push", folder.path(), None)).await;
    assert!(no_conversation.is_err(), "nothing to remember it in");

    // Under ask nothing is remembered, and even `ls` waits
    put(&e, &format!("/api/kwants/{bot}/approvals"), "ask").await;
    let asked = tokio::time::timeout(Duration::from_millis(300), approve_local_shell_in(&e.state, &bot, "ls", folder.path(), Some("conv-1"))).await;
    assert!(asked.is_err());
}

#[tokio::test]
async fn smart_needs_a_folder_for_local_commands() {
    let e = setup().await;
    put(&e, &format!("/api/kwants/{}/approvals", e.bot.id), "smart").await;
    let gone = Path::new("/nonexistent/kwnt-smart-folder");
    let asked = tokio::time::timeout(Duration::from_millis(300), approve_local_shell_in(&e.state, &e.bot.id, "ls", gone, None)).await;
    assert!(asked.is_err(), "a folder that doesn't exist asks");
}

#[tokio::test]
async fn an_older_approvals_table_learns_smart_and_keeps_its_rows() {
    let tmp = tempdir().unwrap();
    drop(Database::init(tmp.path()).await.unwrap());
    // As a release before `smart` left it
    {
        let url = format!("sqlite://{}", tmp.path().join("kwnt.db").display());
        let pool = sqlx::SqlitePool::connect(&url).await.unwrap();
        sqlx::query("DROP TABLE approval_modes").execute(&pool).await.unwrap();
        sqlx::query(
            "CREATE TABLE approval_modes (scope TEXT NOT NULL CHECK (scope IN ('space', 'kwant')), id TEXT NOT NULL,
             mode TEXT NOT NULL CHECK (mode IN ('ask', 'off')), updated_at TIMESTAMP NOT NULL, PRIMARY KEY (scope, id))",
        )
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query("INSERT INTO approval_modes VALUES ('space', 'spc_default', 'off', CURRENT_TIMESTAMP)").execute(&pool).await.unwrap();
        assert!(sqlx::query("INSERT INTO approval_modes VALUES ('kwant', 'k', 'smart', CURRENT_TIMESTAMP)").execute(&pool).await.is_err());
        pool.close().await;
    }
    let db = Database::init(tmp.path()).await.unwrap();
    assert_eq!(db.space_approvals(&SpaceId::default_space()).await.unwrap(), "off", "rows kept");
    db.set_space_approvals(&SpaceId::default_space(), "smart").await.unwrap();
    assert_eq!(db.space_approvals(&SpaceId::default_space()).await.unwrap(), "smart");
    assert!(db.any_approvals_off().await.unwrap(), "smart counts as off for the password store");
    // Running the migration again changes nothing
    drop(db);
    let db = Database::init(tmp.path()).await.unwrap();
    assert_eq!(db.space_approvals(&SpaceId::default_space()).await.unwrap(), "smart");
}
