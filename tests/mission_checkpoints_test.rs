//! Mission Control Phase B2: checkpoints and restore. A task's runs are snapshotted before and
//! after, a restore writes back only the paths this task's runs changed (after a safety
//! checkpoint), and every tool call of a task turn is in the ledger with its effect. These are the
//! contract's 9 acceptance tests (docs/mission-control.md §12), numbered as there, plus two B1
//! gaps: the retry `Idempotency-Key` and the drift scan's limits. Its own test binary: it changes
//! PATH and listens on a real port the stand-in CLI calls back on.

use axum::{
    body::{to_bytes, Body},
    http::{Request, StatusCode},
    Router,
};
use kwnt::{
    broker::PermissionBroker,
    config::Config,
    db::{models::Kwant, spaces::SpaceId, Database},
    router::LayaRouter,
    server::{
        continuity::{SelfScope, TurnOrigin},
        create_router, mcp_runtime, AppState,
    },
    tools::ToolRunner,
};
use serde_json::{json, Value};
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::time::Duration;
use tempfile::{tempdir, TempDir};
use tower::ServiceExt;

/// On a Mission Control prompt: asks kwant's permission tool (the `--permission-prompt-tool`
/// server in its `--mcp-config`) for a Write of a.txt and writes it when allowed, asks for a Bash
/// `echo` and runs it, then moves the task it holds to review through its kwant tools.
const FAKE_CLAUDE: &str = r#"#!/usr/bin/env python3
import json, re, subprocess, sys, urllib.request
args = sys.argv[1:]
if args[:1] == ["auth"]:
    print('{"loggedIn":true}')
    sys.exit(0)
prompt = args[-1] if args else ""
held = re.search(r"You hold task (\S+) of", prompt)

def post(s, name, arguments):
    call = {"jsonrpc": "2.0", "id": 1, "method": "tools/call", "params": {"name": name, "arguments": arguments}}
    headers = dict(s.get("headers", {}), **{"content-type": "application/json"})
    req = urllib.request.Request(s["url"], data=json.dumps(call).encode(), headers=headers)
    return json.loads(urllib.request.urlopen(req, timeout=30).read() or b"null")

def allowed(r):
    try:
        return json.loads(r["result"]["content"][0]["text"]).get("behavior") == "allow"
    except Exception:
        return False

if held and "--mcp-config" in args:
    with open(args[args.index("--mcp-config") + 1]) as f:
        servers = json.load(f).get("mcpServers", {})
    perm = next((s for s in servers.values() if "/api/mcp/permission/" in s.get("url", "")), None)
    me = next((s for s in servers.values() if s.get("url", "").endswith("/api/mcp/self")), None)
    if perm and allowed(post(perm, "approve", {"tool_name": "Write", "input": {"file_path": "a.txt", "content": "changed"}, "tool_use_id": "t1"})):
        with open("a.txt", "w") as f:
            f.write("changed")
    if perm and allowed(post(perm, "approve", {"tool_name": "Bash", "input": {"command": "echo hi", "description": "say hi"}, "tool_use_id": "t2"})):
        subprocess.run(["echo", "hi"], stdout=subprocess.DEVNULL)
    if me:
        post(me, "task_update", {"task_id": held.group(1), "status": "review"})
print(json.dumps({"type": "result", "is_error": False, "num_turns": 1, "result": "done, sent for review",
                  "usage": {"input_tokens": 10, "output_tokens": 10}}))
"#;

fn fake_cli() {
    static DONE: OnceLock<PathBuf> = OnceLock::new();
    DONE.get_or_init(|| {
        let bin = tempfile::tempdir().unwrap().keep().join("bin");
        std::fs::create_dir_all(&bin).unwrap();
        let fake = bin.join("claude");
        std::fs::write(&fake, FAKE_CLAUDE).unwrap();
        std::fs::set_permissions(&fake, std::fs::Permissions::from_mode(0o755)).unwrap();
        std::env::set_var("PATH", format!("{}:{}", bin.display(), std::env::var("PATH").unwrap_or_default()));
        bin
    });
}

struct Env {
    app: Router,
    state: AppState,
    bot: Kwant,
    general: String,
    _tmp: TempDir,
}

/// kwant on a real port (the stand-in CLI calls back), with the mission board on.
async fn setup() -> Env {
    let tmp = tempdir().unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let db = Database::init(tmp.path()).await.unwrap();
    let config = Config {
        data_dir: tmp.path().to_path_buf(),
        host: "127.0.0.1".into(),
        port,
        laya_url: "http://127.0.0.1:9".into(),
        browser_cdp_url: Some("http://127.0.0.1:9".into()),
        ..Config::default()
    };
    let state = AppState::new(
        config.clone(),
        db,
        ToolRunner::new(tmp.path().join("workspace")),
        PermissionBroker::new(),
        LayaRouter::new(config.laya_url.clone()),
    );
    let app = create_router(state.clone());
    let served = app.clone();
    tokio::spawn(async move { axum::serve(listener, served).await.unwrap() });
    let space = SpaceId::default_space();
    let bot = state.db.list_kwants(&space).await.unwrap()[0].clone();
    let general = state.db.list_channels(&space).await.unwrap().into_iter().find(|c| c.name == "general").unwrap().id;
    let e = Env { app, state, bot, general, _tmp: tmp };
    person(&e, "POST", "/api/settings", Some(json!({"mission_board": true}))).await;
    e
}

async fn send(app: &Router, method: &str, uri: &str, auth: Option<&str>, key: Option<&str>, body: Option<Value>) -> (StatusCode, Value, bool) {
    let mut req = Request::builder().method(method).uri(uri).header("content-type", "application/json");
    if let Some(token) = auth {
        req = req.header("authorization", format!("Bearer {token}"));
    }
    if let Some(k) = key {
        req = req.header("idempotency-key", k);
    }
    let body = body.map(|b| Body::from(b.to_string())).unwrap_or_else(Body::empty);
    let resp = app.clone().oneshot(req.body(body).unwrap()).await.unwrap();
    let status = resp.status();
    let replayed = resp.headers().get("idempotent-replayed").is_some_and(|v| v == "true");
    let bytes = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    (status, serde_json::from_slice(&bytes).unwrap_or(Value::Null), replayed)
}

async fn call(app: &Router, method: &str, uri: &str, auth: Option<&str>, body: Option<Value>) -> (StatusCode, Value) {
    let (s, v, _) = send(app, method, uri, auth, None, body).await;
    (s, v)
}

async fn person(e: &Env, method: &str, uri: &str, body: Option<Value>) -> Value {
    let (status, v) = call(&e.app, method, uri, None, body).await;
    assert!(status.is_success(), "{method} {uri}: {status} {v}");
    v
}

async fn ok(e: &Env, token: &str, name: &str, args: Value) -> Value {
    let rpc = json!({"jsonrpc": "2.0", "id": 1, "method": "tools/call", "params": {"name": name, "arguments": args}});
    let (status, v) = call(&e.app, "POST", "/api/mcp/self", Some(token), Some(rpc)).await;
    assert_eq!(status, StatusCode::OK, "{v}");
    let text = v["result"]["content"][0]["text"].as_str().unwrap_or("").to_string();
    assert!(!v["result"]["isError"].as_bool().unwrap_or(false), "{name} {args} failed: {text}");
    serde_json::from_str(&text).unwrap_or(Value::Null)
}

/// A turn of `bot` in #general with its own tools; returns the turn token.
async fn turn(e: &Env, bot: &Kwant, turn_id: &str) -> String {
    let origin = TurnOrigin { channel_id: Some(e.general.clone()), ..Default::default() };
    let scope = SelfScope {
        kwant_id: bot.id.clone(),
        kwant_name: bot.name.clone(),
        origin: Some(origin),
        turn_id: Some(turn_id.into()),
        ..Default::default()
    };
    mcp_runtime::prepare_turn_with_self(&e.state, bot, Some(turn_id.into()), None, Some(scope)).await.unwrap().1
}

fn id(v: &Value) -> String {
    v["id"].as_str().unwrap().to_string()
}

async fn task(e: &Env, id: &str) -> Value {
    person(e, "GET", &format!("/api/tasks/{id}"), None).await
}

async fn state_of(e: &Env, id: &str) -> String {
    task(e, id).await["task"]["state"].as_str().unwrap().to_string()
}

/// A mission led by `lead` with `tasks`; returns the mission id and task ids sorted by title.
async fn mission(e: &Env, lead: &str, tasks: Value) -> (String, Vec<String>) {
    let body = json!({"title": "Ship it", "goal": "a release", "lead_kwant_id": lead, "budget_tokens": 900_000,
                      "channel_id": e.general, "tasks": tasks});
    let m = person(e, "POST", "/api/missions", Some(body)).await;
    let detail = person(e, "GET", &format!("/api/missions/{}", id(&m)), None).await;
    let mut listed = detail["tasks"].as_array().unwrap().clone();
    listed.sort_by_key(|t| t["title"].as_str().unwrap().to_string());
    (id(&m), listed.iter().map(id).collect())
}

async fn folder(e: &Env, mission_id: &str) -> PathBuf {
    let m = e.state.db.get_mission(mission_id).await.unwrap().unwrap();
    let p = PathBuf::from(m.workspace_path);
    std::fs::create_dir_all(&p).unwrap();
    p
}

/// The bot claims `tid`, makes `writes` in the folder (`None` deletes) and hands it in.
async fn run(e: &Env, mid: &str, tid: &str, writes: &[(&str, Option<&str>)]) {
    let dir = folder(e, mid).await;
    let token = turn(e, &e.bot, &format!("t-{tid}-{}", uuid::Uuid::new_v4())).await;
    ok(e, &token, "task_claim", json!({"task_id": tid})).await;
    for (p, body) in writes {
        match body {
            Some(b) => std::fs::write(dir.join(p), b).unwrap(),
            None => std::fs::remove_file(dir.join(p)).unwrap(),
        }
    }
    ok(e, &token, "task_update", json!({"task_id": tid, "status": "review"})).await;
}

async fn checkpoints(e: &Env, tid: &str) -> Vec<Value> {
    person(e, "GET", &format!("/api/tasks/{tid}/checkpoints"), None).await.as_array().unwrap().clone()
}

fn of_kind<'a>(list: &'a [Value], kind: &str) -> Vec<&'a Value> {
    list.iter().filter(|c| c["kind"] == kind).collect()
}

async fn preview(e: &Env, tid: &str, cid: &str) -> Value {
    person(e, "GET", &format!("/api/tasks/{tid}/checkpoints/{cid}/restore-preview"), None).await
}

async fn restore(e: &Env, tid: &str, cid: &str, key: &str, sha: &Value) -> (StatusCode, Value, bool) {
    let uri = format!("/api/tasks/{tid}/checkpoints/{cid}/restore");
    send(&e.app, "POST", &uri, None, Some(key), Some(json!({"preview_sha256": sha}))).await
}

/// A bot that runs on the stand-in claude CLI, with approvals off for the space.
async fn cli_bot(e: &Env) -> String {
    fake_cli();
    e.state.db.set_space_approvals(&SpaceId::default_space(), "off").await.unwrap();
    let bot = json!({"name": "Ann", "title": "Ann the tester", "persona_prompt": "Test persona",
                     "avatar_sprite": "s", "driver_type": "cli", "preferred_model": "claude"});
    id(&person(e, "POST", "/api/kwants", Some(bot)).await)
}

/// The runner starts `tid` on the stand-in CLI; waits until the turn hands it in.
async fn cli_run(e: &Env, mid: &str, tid: &str) {
    // An earlier runner turn of this mission may still be ending after its hand-in
    for _ in 0..200 {
        if !kwnt::server::missions::runner::is_running(mid) {
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    let r = person(e, "POST", &format!("/api/missions/{mid}/run"), Some(json!({"max_tasks": 1}))).await;
    assert_eq!(r["started"][0]["task_id"], tid, "{r}");
    for _ in 0..300 {
        if state_of(e, tid).await == "review" {
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert_eq!(state_of(e, tid).await, "review", "{}", task(e, tid).await);
    // The after_run checkpoint is taken in the move to review; the turn may still be ending
    for _ in 0..100 {
        if e.state.db.open_claim(tid).await.unwrap().is_none() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

fn sha(bytes: &[u8]) -> String {
    let d = <sha2::Sha256 as sha2::Digest>::digest(bytes);
    d.iter().map(|b| format!("{b:02x}")).collect()
}

fn blobs(e: &Env) -> PathBuf {
    e.state.config.data_dir.join("checkpoints").join("blobs")
}

async fn events(e: &Env, tid: &str, kind: &str) -> Vec<Value> {
    e.state.db.event_details(tid, kind).await.unwrap()
}

/// A mission on the stand-in CLI whose folder holds a.txt ("original") and keep.txt, run once.
async fn cli_mission(e: &Env) -> (String, String, PathBuf) {
    let ann = cli_bot(e).await;
    let (mid, ids) = mission(e, &ann, json!([{"title": "A", "owner": ann}])).await;
    let dir = folder(e, &mid).await;
    std::fs::write(dir.join("a.txt"), "original").unwrap();
    std::fs::write(dir.join("keep.txt"), b"untouched \x00\x01 bytes").unwrap();
    cli_run(e, &mid, &ids[0]).await;
    (mid, ids[0].clone(), dir)
}

// 1
#[tokio::test]
async fn a_task_run_is_checkpointed_and_its_tool_calls_are_in_the_ledger() {
    let e = setup().await;
    let (_mid, tid, dir) = cli_mission(&e).await;
    assert_eq!(std::fs::read_to_string(dir.join("a.txt")).unwrap(), "changed", "the turn wrote a.txt");
    let list = checkpoints(&e, &tid).await;
    let before = of_kind(&list, "before_run");
    let after = of_kind(&list, "after_run");
    assert_eq!((before.len(), after.len()), (1, 1), "{list:?}");
    assert!(before[0]["restorable"].as_bool().unwrap() && after[0]["restorable"].as_bool().unwrap(), "{list:?}");
    assert!(before[0]["files"].as_i64().unwrap() >= 2, "{list:?}");
    let calls = e.state.db.tool_calls_of(&tid).await.unwrap();
    let effect = |name: &str| calls.iter().find(|c| c.tool_name == name).map(|c| c.effect.clone());
    assert_eq!(effect("Write").as_deref(), Some("workspace"), "{calls:?}");
    assert_eq!(effect("Bash").as_deref(), Some("external"), "D6: Bash is always external: {calls:?}");
    assert!(calls.iter().all(|c| c.decision.contains("allowed")), "approvals are off: {calls:?}");
    // The list shows what happened after the before_run checkpoint
    assert_eq!(before[0]["effects_after"]["workspace"], 1, "{list:?}");
    assert_eq!(before[0]["effects_after"]["external"][0]["tool_name"], "Bash", "{list:?}");
    let t = task(&e, &tid).await;
    assert_eq!(t["task"]["checkpoint_count"], 2, "{t}");
}

// 2
#[tokio::test]
async fn a_restore_brings_back_the_tasks_files_and_can_itself_be_undone() {
    let e = setup().await;
    let (_mid, tid, dir) = cli_mission(&e).await;
    let keep_before = std::fs::read(dir.join("keep.txt")).unwrap();
    let list = checkpoints(&e, &tid).await;
    let cid = id(of_kind(&list, "before_run")[0]);
    let p = preview(&e, &tid, &cid).await;
    assert_eq!(p["changes"], json!([{"path": "a.txt", "action": "overwrite"}]), "{p}");
    assert_eq!(p["task_state_after"], "ready", "{p}");
    assert!(p["refused"].is_null(), "{p}");
    let external = p["external_not_undone"].as_array().unwrap();
    assert!(external.iter().any(|x| x["tool_name"] == "Bash"), "{p}");
    assert!(p["not_undone_note"].as_str().unwrap().contains("can't undo"), "{p}");

    let (s, r, _) = restore(&e, &tid, &cid, "restore-1", &p["preview_sha256"]).await;
    assert_eq!(s, StatusCode::OK, "{r}");
    assert_eq!(std::fs::read_to_string(dir.join("a.txt")).unwrap(), "original");
    assert_eq!(std::fs::read(dir.join("keep.txt")).unwrap(), keep_before, "an untouched file stays byte-identical");
    assert_eq!(r["restored"], json!(["a.txt"]), "{r}");
    assert!(r["external_not_undone"].as_array().unwrap().iter().any(|x| x["tool_name"] == "Bash"), "{r}");
    assert_eq!(r["task"]["state"], "ready", "{r}");
    let safety = r["before_restore_checkpoint_id"].as_str().unwrap().to_string();
    let list = checkpoints(&e, &tid).await;
    assert_eq!(of_kind(&list, "before_restore").len(), 1, "{list:?}");
    assert_eq!(events(&e, &tid, "restored").await.len(), 1);

    // The restore is undone from its own safety checkpoint
    let p = preview(&e, &tid, &safety).await;
    assert_eq!(p["changes"], json!([{"path": "a.txt", "action": "overwrite"}]), "{p}");
    let (s, r, _) = restore(&e, &tid, &safety, "restore-2", &p["preview_sha256"]).await;
    assert_eq!(s, StatusCode::OK, "{r}");
    assert_eq!(std::fs::read_to_string(dir.join("a.txt")).unwrap(), "changed");
    assert_eq!(std::fs::read(dir.join("keep.txt")).unwrap(), keep_before);
}

#[tokio::test]
async fn a_force_path_the_preview_does_not_list_is_refused_and_writes_nothing() {
    let e = setup().await;
    let (mid, ids) = mission(&e, &e.bot.id, json!([{"title": "A", "owner": e.bot.id}])).await;
    let (tid, dir) = (ids[0].clone(), folder(&e, &mid).await);
    std::fs::write(dir.join("a.txt"), "original").unwrap();
    std::fs::write(dir.join("keep.txt"), "untouched").unwrap();
    run(&e, &mid, &tid, &[("a.txt", Some("changed"))]).await;
    let cid = id(of_kind(&checkpoints(&e, &tid).await, "before_run")[0]);
    let p = preview(&e, &tid, &cid).await;
    assert_eq!(p["changes"], json!([{"path": "a.txt", "action": "overwrite"}]), "{p}");
    let uri = format!("/api/tasks/{tid}/checkpoints/{cid}/restore");
    let body = |force: Value| Some(json!({"preview_sha256": p["preview_sha256"], "force_paths": force}));

    // keep.txt is in the folder but not in the preview; nope.txt is nowhere
    let (s, r, _) = send(&e.app, "POST", &uri, None, Some("force-1"), body(json!(["a.txt", "keep.txt", "nope.txt"]))).await;
    assert_eq!(s, StatusCode::UNPROCESSABLE_ENTITY, "{r}");
    assert_eq!(r["code"], "unknown_force_path", "{r}");
    let msg = r["error"].as_str().unwrap();
    assert!(msg.contains("keep.txt") && msg.contains("nope.txt") && !msg.contains("a.txt"), "{r}");
    assert_eq!(std::fs::read_to_string(dir.join("a.txt")).unwrap(), "changed", "nothing is written");
    assert!(of_kind(&checkpoints(&e, &tid).await, "before_restore").is_empty(), "no safety checkpoint either");
    assert!(events(&e, &tid, "restored").await.is_empty());

    // The refusal isn't stored under the key: the same key with a listed path restores
    let (s, r, replayed) = send(&e.app, "POST", &uri, None, Some("force-1"), body(json!(["a.txt"]))).await;
    assert_eq!(s, StatusCode::OK, "{r}");
    assert!(!replayed);
    assert_eq!(std::fs::read_to_string(dir.join("a.txt")).unwrap(), "original");
}

// 3
#[tokio::test]
async fn a_verified_task_forks_instead_and_a_running_one_is_refused() {
    let e = setup().await;
    let plan = json!([{"title": "A", "owner": e.bot.id, "checks": [{"kind": "file_exists", "spec": {"path": "a.txt"}}]},
                      {"title": "B", "owner": e.bot.id}]);
    let (mid, ids) = mission(&e, &e.bot.id, plan).await;
    run(&e, &mid, &ids[0], &[("a.txt", Some("v1"))]).await;
    assert_eq!(state_of(&e, &ids[0]).await, "verified");
    let cid = id(of_kind(&checkpoints(&e, &ids[0]).await, "before_run")[0]);
    let p = preview(&e, &ids[0], &cid).await;
    assert_eq!(p["refused"], "fork_instead", "{p}");
    let (s, v, _) = restore(&e, &ids[0], &cid, "k-verified", &p["preview_sha256"]).await;
    assert_eq!((s, v["code"].as_str()), (StatusCode::CONFLICT, Some("fork_instead")), "{v}");
    assert_eq!(std::fs::read_to_string(folder(&e, &mid).await.join("a.txt")).unwrap(), "v1", "nothing written");

    let token = turn(&e, &e.bot, "t-running").await;
    ok(&e, &token, "task_claim", json!({"task_id": ids[1]})).await;
    assert_eq!(state_of(&e, &ids[1]).await, "working");
    let cid = id(of_kind(&checkpoints(&e, &ids[1]).await, "before_run")[0]);
    let p = preview(&e, &ids[1], &cid).await;
    assert_eq!(p["refused"], "running", "{p}");
    let (s, v, _) = restore(&e, &ids[1], &cid, "k-running", &p["preview_sha256"]).await;
    assert_eq!((s, v["code"].as_str()), (StatusCode::CONFLICT, Some("running")), "{v}");
    assert!(of_kind(&checkpoints(&e, &ids[1]).await, "before_restore").is_empty(), "no safety checkpoint for a refused restore");
}

// 4
#[tokio::test]
async fn a_stale_preview_is_refused_and_a_repeated_key_writes_nothing() {
    let e = setup().await;
    let (mid, ids) = mission(&e, &e.bot.id, json!([{"title": "A", "owner": e.bot.id}])).await;
    let dir = folder(&e, &mid).await;
    std::fs::write(dir.join("a.txt"), "v0").unwrap();
    run(&e, &mid, &ids[0], &[("a.txt", Some("v1"))]).await;
    assert_eq!(state_of(&e, &ids[0]).await, "review");
    let cid = id(of_kind(&checkpoints(&e, &ids[0]).await, "before_run")[0]);
    let stale = preview(&e, &ids[0], &cid).await;
    std::fs::write(dir.join("a.txt"), "v2, edited after the preview").unwrap();
    let (s, v, _) = restore(&e, &ids[0], &cid, "k-stale", &stale["preview_sha256"]).await;
    assert_eq!((s, v["code"].as_str()), (StatusCode::CONFLICT, Some("stale_preview")), "{v}");
    assert_eq!(std::fs::read_to_string(dir.join("a.txt")).unwrap(), "v2, edited after the preview");

    let fresh = preview(&e, &ids[0], &cid).await;
    assert_ne!(fresh["preview_sha256"], stale["preview_sha256"]);
    let (s1, first, replay1) = restore(&e, &ids[0], &cid, "k-once", &fresh["preview_sha256"]).await;
    assert_eq!(s1, StatusCode::OK, "{first}");
    assert!(!replay1);
    assert_eq!(std::fs::read_to_string(dir.join("a.txt")).unwrap(), "v0");
    std::fs::write(dir.join("a.txt"), "v3, after the restore").unwrap();
    let (s2, second, replay2) = restore(&e, &ids[0], &cid, "k-once", &fresh["preview_sha256"]).await;
    assert_eq!(s2, StatusCode::OK, "{second}");
    assert!(replay2, "the same key replays the first response");
    assert_eq!(first, second);
    assert_eq!(std::fs::read_to_string(dir.join("a.txt")).unwrap(), "v3, after the restore", "a replay writes nothing");
    assert_eq!(of_kind(&checkpoints(&e, &ids[0]).await, "before_restore").len(), 1);
    assert_eq!(events(&e, &ids[0], "restored").await.len(), 1);
    // The key is required
    let uri = format!("/api/tasks/{}/checkpoints/{cid}/restore", ids[0]);
    let (s, _, _) = send(&e.app, "POST", &uri, None, None, Some(json!({"preview_sha256": fresh["preview_sha256"]}))).await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
}

// 5
#[tokio::test]
async fn unsafe_manifest_paths_and_symlinks_are_skipped_and_never_followed() {
    let e = setup().await;
    let (mid, ids) = mission(&e, &e.bot.id, json!([{"title": "A", "owner": e.bot.id}])).await;
    let dir = folder(&e, &mid).await;
    let outside = dir.parent().unwrap().join("outside.txt");
    std::fs::write(&outside, "outside, never touched").unwrap();
    std::os::unix::fs::symlink(&outside, dir.join("link.txt")).unwrap();
    std::fs::write(dir.join("s.txt"), "s v0").unwrap();
    run(&e, &mid, &ids[0], &[("s.txt", Some("s v1")), ("a.txt", Some("a v1"))]).await;
    let list = checkpoints(&e, &ids[0]).await;
    let before = of_kind(&list, "before_run")[0].clone();
    let cid = id(&before);
    // The snapshot didn't follow the symlink
    let m_path = e.state.config.data_dir.join("checkpoints").join(&ids[0]).join(format!("{cid}.json"));
    let mut m: Value = serde_json::from_slice(&std::fs::read(&m_path).unwrap()).unwrap();
    assert!(m["files"].as_array().unwrap().iter().all(|f| f["path"] != "link.txt"), "{m}");
    let skip = m["skipped"].as_array().unwrap().iter().find(|s| s["path"] == "link.txt").cloned();
    assert_eq!(skip.unwrap()["why"], "symlink, not followed", "{m}");

    // A hand-edited manifest with an escaping and an absolute path
    let blob = sha(b"s v0");
    let escape_abs = dir.parent().unwrap().join("abs-escape.txt");
    for p in ["../escape.txt".to_string(), escape_abs.display().to_string()] {
        m["files"].as_array_mut().unwrap().push(json!({"path": p, "sha256": blob, "bytes": 4, "mode": 420}));
    }
    std::fs::write(&m_path, serde_json::to_vec(&m).unwrap()).unwrap();
    // s.txt, which the run changed, is now a symlink out of the folder
    std::fs::remove_file(dir.join("s.txt")).unwrap();
    std::os::unix::fs::symlink(&outside, dir.join("s.txt")).unwrap();

    let p = preview(&e, &ids[0], &cid).await;
    let skipped = p["skipped"].as_array().unwrap();
    let why = |path: &str| skipped.iter().find(|s| s["path"] == path).map(|s| s["why"].as_str().unwrap().to_string());
    assert_eq!(why("../escape.txt").as_deref(), Some("unsafe path"), "{p}");
    assert_eq!(why(&escape_abs.display().to_string()).as_deref(), Some("unsafe path"), "{p}");
    assert_eq!(why("s.txt").as_deref(), Some("symlink, not followed"), "{p}");
    assert_eq!(p["changes"], json!([{"path": "a.txt", "action": "delete"}]), "{p}");
    let (s, r, _) = restore(&e, &ids[0], &cid, "k-unsafe", &p["preview_sha256"]).await;
    assert_eq!(s, StatusCode::OK, "{r}");
    assert_eq!(std::fs::read_to_string(&outside).unwrap(), "outside, never touched", "nothing written through a symlink");
    assert!(!dir.parent().unwrap().join("escape.txt").exists() && !escape_abs.exists(), "nothing written outside the folder");
    assert!(!dir.join("a.txt").exists(), "the run's new file is gone");
}

// 6
#[tokio::test]
async fn large_files_are_skipped_and_old_checkpoints_and_orphan_blobs_are_pruned() {
    let e = setup().await;
    person(&e, "POST", "/api/settings", Some(json!({"checkpoint_keep": 2}))).await;
    let (mid, ids) = mission(&e, &e.bot.id, json!([{"title": "A", "owner": e.bot.id}])).await;
    let dir = folder(&e, &mid).await;
    std::fs::File::create(dir.join("big.bin")).unwrap().set_len(25 * 1024 * 1024).unwrap();
    std::fs::write(dir.join("u.txt"), "u0").unwrap();
    run(&e, &mid, &ids[0], &[]).await;
    let first = id(of_kind(&checkpoints(&e, &ids[0]).await, "before_run")[0]);

    std::fs::write(blobs(&e).join("f".repeat(64)), "orphan").unwrap();
    for n in 1..=4 {
        std::fs::write(dir.join("u.txt"), format!("u{n}")).unwrap();
        let c = person(&e, "POST", &format!("/api/tasks/{}/checkpoints", ids[0]), None).await;
        assert_eq!(c["kind"], "manual");
        assert_eq!(c["skipped"], 1, "{c}");
        let m_path = e.state.config.data_dir.join("checkpoints").join(&ids[0]).join(format!("{}.json", id(&c)));
        let m: Value = serde_json::from_slice(&std::fs::read(m_path).unwrap()).unwrap();
        assert_eq!(m["skipped"], json!([{"path": "big.bin", "why": "over 20 MB"}]), "{m}");
    }
    let list = checkpoints(&e, &ids[0]).await;
    let kinds: Vec<&str> = list.iter().map(|c| c["kind"].as_str().unwrap()).collect();
    assert_eq!(kinds, ["before_run", "manual", "manual"], "keep 2, plus the task's first before_run: {list:?}");
    assert_eq!(id(&list[0]), first);
    assert!(events(&e, &ids[0], "checkpoint_pruned").await.len() >= 3);
    assert!(!blobs(&e).join("f".repeat(64)).exists(), "an orphan blob is removed");
    for (content, kept) in [("u0", true), ("u1", false), ("u2", false), ("u3", true), ("u4", true)] {
        assert_eq!(blobs(&e).join(sha(content.as_bytes())).exists(), kept, "blob of {content}");
    }
    assert!(!blobs(&e).join(sha(&vec![0u8; 25 * 1024 * 1024])).exists(), "the big file was never stored");
    let (s, v) = call(&e.app, "POST", "/api/settings", None, Some(json!({"checkpoint_keep": 0}))).await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "{v}");
}

// 7
#[tokio::test]
async fn no_route_returns_manifest_or_blob_paths_or_file_contents() {
    let e = setup().await;
    let (mid, ids) = mission(&e, &e.bot.id, json!([{"title": "A", "owner": e.bot.id}])).await;
    let dir = folder(&e, &mid).await;
    std::fs::write(dir.join(".env"), "API_KEY=SECRET-CONTENT-4242").unwrap();
    std::fs::write(dir.join("key.pem"), "-----BEGIN PRIVATE KEY----- PEM-CONTENT-4242").unwrap();
    run(&e, &mid, &ids[0], &[("a.txt", Some("BODY-CONTENT-4242"))]).await;
    let tid = &ids[0];
    let created = person(&e, "POST", &format!("/api/tasks/{tid}/checkpoints"), None).await;
    let list = checkpoints(&e, tid).await;
    let cid = id(of_kind(&list, "before_run")[0]);
    let p = preview(&e, tid, &cid).await;
    let (_, r, _) = restore(&e, tid, &cid, "k-leak", &p["preview_sha256"]).await;
    let mut seen = vec![created, json!(list), p, r, task(&e, tid).await, checkpoints(&e, tid).await.into()];
    for uri in [format!("/api/missions/{mid}"), format!("/api/missions/{mid}/graph"), "/api/missions".into(), "/api/settings".into()] {
        seen.push(person(&e, "GET", &uri, None).await);
    }
    // The snapshot holds the secrets (it must), under a 0700 folder
    let root = e.state.config.data_dir.join("checkpoints");
    assert_eq!(std::fs::metadata(&root).unwrap().permissions().mode() & 0o777, 0o700);
    assert!(blobs(&e).join(sha(b"API_KEY=SECRET-CONTENT-4242")).exists());
    let root_s = root.display().to_string();
    for v in seen {
        let s = v.to_string();
        for bad in ["manifest", "blobs", root_s.as_str(), "SECRET-CONTENT-4242", "PEM-CONTENT-4242", "BODY-CONTENT-4242"] {
            assert!(!s.contains(bad), "{bad} leaked in {s}");
        }
    }
}

// 8
#[tokio::test]
async fn a_bot_token_cant_checkpoint_restore_or_fork() {
    let e = setup().await;
    let (mid, ids) = mission(&e, &e.bot.id, json!([{"title": "A", "owner": e.bot.id}])).await;
    run(&e, &mid, &ids[0], &[("a.txt", Some("v1"))]).await;
    let tid = &ids[0];
    let cid = id(of_kind(&checkpoints(&e, tid).await, "before_run")[0]);
    let token = turn(&e, &e.bot, "t-bot").await;
    let (s, _) = call(&e.app, "POST", &format!("/api/tasks/{tid}/checkpoints"), Some(&token), None).await;
    assert_eq!(s, StatusCode::FORBIDDEN);
    let uri = format!("/api/tasks/{tid}/checkpoints/{cid}/restore");
    let (s, _, _) = send(&e.app, "POST", &uri, Some(&token), Some("k-bot"), Some(json!({"preview_sha256": "x"}))).await;
    assert_eq!(s, StatusCode::FORBIDDEN);
    let (s, _, _) = send(&e.app, "POST", &format!("/api/tasks/{tid}/fork"), Some(&token), Some("k-bot-fork"), Some(json!({"from_checkpoint": cid}))).await;
    assert_eq!(s, StatusCode::FORBIDDEN);
    assert_eq!(checkpoints(&e, tid).await.len(), 2, "nothing was taken");
}

// 9
#[tokio::test]
async fn an_unreadable_folder_fails_the_checkpoint_but_not_the_turn() {
    let e = setup().await;
    let ann = cli_bot(&e).await;
    let (mid, ids) = mission(&e, &ann, json!([{"title": "A", "owner": ann}])).await;
    let dir = folder(&e, &mid).await;
    std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o300)).unwrap();
    if std::fs::read_dir(&dir).is_ok() {
        // Running as root: permissions don't stop reads, so there is nothing to test
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o755)).unwrap();
        eprintln!("skipped: the folder stays readable (root?)");
        return;
    }
    let tid = &ids[0];
    cli_run(&e, &mid, tid).await;
    std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o755)).unwrap();
    assert_eq!(std::fs::read_to_string(dir.join("a.txt")).unwrap(), "changed", "the turn still ran");
    assert!(!events(&e, tid, "checkpoint_failed").await.is_empty());
    let list = checkpoints(&e, tid).await;
    assert!(!list.is_empty());
    for c in &list {
        assert_eq!(c["restorable"], false, "{c}");
        assert!(c["error"].is_string(), "{c}");
    }
    let cid = id(&list[0]);
    let (s, v) = call(&e.app, "GET", &format!("/api/tasks/{tid}/checkpoints/{cid}/restore-preview"), None, None).await;
    assert_eq!((s, v["code"].as_str()), (StatusCode::CONFLICT, Some("not_restorable")), "{v}");
}

// B1 gap: a retry with the same Idempotency-Key happens once
#[tokio::test]
async fn an_idempotency_key_retries_once() {
    let e = setup().await;
    let plan = json!([{"title": "A", "owner": e.bot.id, "checks": [{"kind": "file_exists", "spec": {"path": "never.txt"}}]}]);
    let (_mid, ids) = mission(&e, &e.bot.id, plan).await;
    let tid = &ids[0];
    // Its check fails on every attempt, so the last one fails the task
    for n in 0..5 {
        if state_of(&e, tid).await == "failed" {
            break;
        }
        let token = turn(&e, &e.bot, &format!("t-fail-{n}")).await;
        ok(&e, &token, "task_claim", json!({"task_id": tid})).await;
        ok(&e, &token, "task_update", json!({"task_id": tid, "status": "review"})).await;
    }
    assert_eq!(state_of(&e, tid).await, "failed");
    let uri = format!("/api/tasks/{tid}/retry");
    let (s1, first, r1) = send(&e.app, "POST", &uri, None, Some("retry-1"), None).await;
    let (s2, second, r2) = send(&e.app, "POST", &uri, None, Some("retry-1"), None).await;
    assert!(s1.is_success() && s2.is_success(), "{first} {second}");
    assert!(!r1 && r2, "the second call is a replay");
    assert_eq!(first, second);
    assert_eq!(state_of(&e, tid).await, "ready");
    let t = task(&e, tid).await;
    let retries = t["events"].as_array().unwrap().iter().filter(|ev| ev["from_state"] == "failed" && ev["to_state"] == "ready").count();
    assert_eq!(retries, 1, "one retry for one key: {t}");
    // Without a key a second retry is a real one, refused: the task isn't failed any more
    let (s3, _, _) = send(&e.app, "POST", &uri, None, None, None).await;
    assert!(!s3.is_success());
}

/// The owner verifies a task by `a.txt`; returns the mission and task id.
async fn verified(e: &Env) -> (String, String) {
    let plan = json!([{"title": "A", "owner": e.bot.id, "checks": [{"kind": "file_exists", "spec": {"path": "a.txt"}}]}]);
    let (mid, ids) = mission(e, &e.bot.id, plan).await;
    std::fs::write(folder(e, &mid).await.join("a.txt"), "a v1").unwrap();
    let token = turn(e, &e.bot, "t-verify").await;
    ok(e, &token, "task_claim", json!({"task_id": ids[0]})).await;
    ok(e, &token, "task_attach_evidence", json!({"task_id": ids[0], "kind": "file", "label": "a", "path": "a.txt"})).await;
    ok(e, &token, "task_update", json!({"task_id": ids[0], "status": "review"})).await;
    assert_eq!(state_of(e, &ids[0]).await, "verified");
    (mid, ids[0].clone())
}

fn evidence(e: &Env, tid: &str, path: &str, sha256: String) -> kwnt::db::missions::NewEvidence {
    kwnt::db::missions::NewEvidence {
        task_id: tid.into(),
        kind: "file".into(),
        label: path.into(),
        path: Some(path.into()),
        sha256: Some(sha256),
        captured_by: e.bot.id.clone(),
        ..Default::default()
    }
}

// B1 gap: the drift scan hashes at most 200 files and skips files over 50 MB
#[tokio::test]
async fn the_scan_stops_at_200_files_and_skips_files_over_50_mb() {
    let e = setup().await;
    let (mid, tid) = verified(&e).await;
    let dir = folder(&e, &mid).await;
    for n in 0..200 {
        let p = format!("f{n:03}.txt");
        std::fs::write(dir.join(&p), "same").unwrap();
        e.state.db.add_evidence(&evidence(&e, &tid, &p, sha(b"same"))).await.unwrap();
    }
    let r = person(&e, "POST", &format!("/api/missions/{mid}/graph/scan"), None).await;
    assert_eq!(r["scanned"], 200, "{r}");
    assert_eq!(r["flagged"], json!([]), "{r}");
    let skipped = r["skipped"].as_array().unwrap();
    assert_eq!(skipped.len(), 1, "{r}");
    assert_eq!(skipped[0]["why"], "over the 200-file scan limit", "{r}");

    let e = setup().await;
    let (mid, tid) = verified(&e).await;
    let big = folder(&e, &mid).await.join("big.bin");
    std::fs::File::create(&big).unwrap().set_len(51 * 1024 * 1024).unwrap();
    e.state.db.add_evidence(&evidence(&e, &tid, "big.bin", "0".repeat(64))).await.unwrap();
    let r = person(&e, "POST", &format!("/api/missions/{mid}/graph/scan"), None).await;
    assert_eq!(r["scanned"], 1, "only a.txt is hashed: {r}");
    assert_eq!(r["flagged"], json!([]), "a skipped file is never 'changed': {r}");
    let skipped = r["skipped"].as_array().unwrap();
    assert_eq!(skipped.len(), 1, "{r}");
    assert_eq!((skipped[0]["path"].as_str(), skipped[0]["why"].as_str()), (Some("big.bin"), Some("over 50 MB")), "{r}");
    assert_eq!(Path::new(&big).metadata().unwrap().len(), 51 * 1024 * 1024);
}

// D10
#[tokio::test]
async fn a_restore_holds_the_task_until_the_person_runs_it() {
    let e = setup().await;
    let (mid, tid, dir) = cli_mission(&e).await;
    let cid = id(of_kind(&checkpoints(&e, &tid).await, "before_run")[0]);
    let p = preview(&e, &tid, &cid).await;
    let (s, r, _) = restore(&e, &tid, &cid, "hold-1", &p["preview_sha256"]).await;
    assert_eq!(s, StatusCode::OK, "{r}");
    assert_eq!(r["task"]["state"], "ready", "{r}");
    assert!(r["task"]["held_at"].is_string(), "a restored task is held: {r}");
    assert_eq!(events(&e, &tid, "held").await.len(), 1);

    // The runner (tick and the Run button of the mission) leaves it alone
    kwnt::server::missions::runner::tick(&e.state).await;
    let run = person(&e, "POST", &format!("/api/missions/{mid}/run"), Some(json!({"max_tasks": 1}))).await;
    assert!(run["started"].as_array().unwrap().is_empty(), "{run}");
    assert!(e.state.db.open_claim(&tid).await.unwrap().is_none(), "a held task isn't claimed");
    assert_eq!(state_of(&e, &tid).await, "ready");
    // A bot can't claim it either
    let token = turn(&e, &e.bot, "t-held").await;
    let rpc = json!({"jsonrpc": "2.0", "id": 1, "method": "tools/call", "params": {"name": "task_claim", "arguments": {"task_id": tid}}});
    let (_, v) = call(&e.app, "POST", "/api/mcp/self", Some(&token), Some(rpc)).await;
    assert!(v["result"]["isError"].as_bool().unwrap_or(false), "{v}");
    assert!(v["result"]["content"][0]["text"].as_str().unwrap().contains("waits for the person"), "{v}");

    // Undo works: the task never started, so no 409 running; it stays held
    let safety = r["before_restore_checkpoint_id"].as_str().unwrap().to_string();
    let p = preview(&e, &tid, &safety).await;
    assert!(p["refused"].is_null(), "{p}");
    let (s, r, _) = restore(&e, &tid, &safety, "hold-2", &p["preview_sha256"]).await;
    assert_eq!(s, StatusCode::OK, "{r}");
    assert_eq!(std::fs::read_to_string(dir.join("a.txt")).unwrap(), "changed");
    assert!(r["task"]["held_at"].is_string(), "{r}");

    // The person's Run lets it go, once
    let r = person(&e, "POST", &format!("/api/tasks/{tid}/run"), None).await;
    assert!(r["task"]["held_at"].is_null(), "{r}");
    let (s, v) = call(&e.app, "POST", &format!("/api/tasks/{tid}/run"), None, None).await;
    assert_eq!((s, v["code"].as_str()), (StatusCode::CONFLICT, Some("not_held")), "{v}");
    assert_eq!(events(&e, &tid, "released").await.len(), 1);
    cli_run(&e, &mid, &tid).await;

    // "Restore and run again" doesn't hold it: the runner picks it up
    let cid = id(of_kind(&checkpoints(&e, &tid).await, "before_run").last().unwrap());
    let p = preview(&e, &tid, &cid).await;
    let uri = format!("/api/tasks/{tid}/checkpoints/{cid}/restore");
    let (s, r, _) = send(&e.app, "POST", &uri, None, Some("hold-3"), Some(json!({"preview_sha256": p["preview_sha256"], "rerun": true}))).await;
    assert_eq!(s, StatusCode::OK, "{r}");
    assert!(r["task"]["held_at"].is_null(), "{r}");
    cli_run(&e, &mid, &tid).await;
}

/// `bot` claims `tid` in its own turn; returns the turn id and token.
async fn claim_as(e: &Env, bot: &Kwant, tid: &str) -> (String, String) {
    let turn_id = format!("t-{tid}-{}", uuid::Uuid::new_v4());
    let token = turn(e, bot, &turn_id).await;
    ok(e, &token, "task_claim", json!({"task_id": tid})).await;
    (turn_id, token)
}

/// `bot` writes `rel` in `dir`, through a logged Write call when `logged` (else like a shell would).
async fn write_as(e: &Env, bot: &Kwant, turn_id: &str, dir: &Path, rel: &str, body: &str, logged: bool) {
    if logged {
        let req = kwnt::mcp::ToolApprovalRequest {
            kwant_id: bot.id.clone(),
            kwant_name: bot.name.clone(),
            turn_id: turn_id.into(),
            tool_name: "Write".into(),
            server: String::new(),
            tool: "Write".into(),
            arguments: json!({"file_path": rel, "content": body}),
            builtin: Some(kwnt::mcp::BuiltinCall { cwd: Some(dir.to_string_lossy().into_owned()), outside_folder: false }),
            smart: None,
        };
        kwnt::server::missions::checkpoints::log_call(&e.state.db, &e.state.config.data_dir, &req, "allowed", None).await;
    }
    std::fs::write(dir.join(rel), body).unwrap();
}

// D11
#[tokio::test]
async fn a_conflict_names_another_task_only_when_its_own_calls_wrote_the_path() {
    let e = setup().await;
    let bot = json!({"name": "Bea", "title": "Bea the builder", "persona_prompt": "Test persona",
                     "avatar_sprite": "s", "driver_type": "cli", "preferred_model": "claude"});
    let bea: Kwant = e.state.db.get_kwant(&id(&person(&e, "POST", "/api/kwants", Some(bot)).await)).await.unwrap().unwrap();
    let me = e.bot.id.clone();
    let (mid, ids) = mission(&e, &me, json!([{"title": "A", "owner": me}, {"title": "B", "owner": bea.id}])).await;
    let (a, b) = (ids[0].clone(), ids[1].clone());
    let dir = folder(&e, &mid).await;
    std::fs::write(dir.join("seed.txt"), "seed").unwrap();
    // A and B run at the same time in one folder
    let (ta, ka) = claim_as(&e, &e.bot, &a).await;
    let (tb, kb) = claim_as(&e, &bea, &b).await;
    write_as(&e, &e.bot, &ta, &dir, "a.txt", "by A", true).await;
    write_as(&e, &bea, &tb, &dir, "b.txt", "by B", true).await;
    write_as(&e, &bea, &tb, &dir, "seed.txt", "by a shell", false).await;
    ok(&e, &ka, "task_update", json!({"task_id": a, "status": "review"})).await;
    ok(&e, &kb, "task_update", json!({"task_id": b, "status": "review"})).await;
    let calls = e.state.db.tool_calls_of(&a).await.unwrap();
    assert_eq!(calls.iter().filter_map(|c| c.path.as_deref()).collect::<Vec<_>>(), vec!["a.txt"], "{calls:?}");

    let conflict = |p: &Value, path: &str| p["conflicts"].as_array().unwrap().iter().find(|c| c["path"] == path).cloned();
    let change = |p: &Value, path: &str| p["changes"].as_array().unwrap().iter().any(|c| c["path"] == path);
    // Restoring B: a.txt is A's (A's own Write), seed.txt nobody's own call wrote
    let cb = id(of_kind(&checkpoints(&e, &b).await, "before_run")[0]);
    let pb = preview(&e, &b, &cb).await;
    assert_eq!(conflict(&pb, "a.txt").map(|c| c["changed_by_task"].clone()), Some(json!(a)), "{pb}");
    assert!(change(&pb, "b.txt"), "B's own file is a plain change: {pb}");
    let seed = conflict(&pb, "seed.txt").unwrap_or_else(|| panic!("{pb}"));
    assert!(seed["changed_by_task"].is_null() && seed["why"] == "changed outside this task", "{pb}");
    // Restoring A: b.txt is B's; a.txt, only A's own call wrote, is never shown as B's
    let ca = id(of_kind(&checkpoints(&e, &a).await, "before_run")[0]);
    let pa = preview(&e, &a, &ca).await;
    assert_eq!(conflict(&pa, "b.txt").map(|c| c["changed_by_task"].clone()), Some(json!(b)), "{pa}");
    assert!(change(&pa, "a.txt") && conflict(&pa, "a.txt").is_none(), "{pa}");
    assert!(conflict(&pa, "seed.txt").is_some_and(|c| c["changed_by_task"].is_null()), "{pa}");
    let named: Vec<&Value> = pa["conflicts"].as_array().unwrap().iter().chain(pb["conflicts"].as_array().unwrap()).filter(|c| !c["changed_by_task"].is_null()).collect();
    assert_eq!(named.len(), 2, "only proven writes name a task: {named:?}");
}
