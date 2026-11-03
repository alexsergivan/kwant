//! Self-improving bots, round 1 (docs/mission-control.md §23): what a bot's earlier tasks taught
//! it reaches its next task's brief, the self-check sentence rides along when there is an approved
//! check, the turn is attributed, a bot with nothing to learn gets the same brief byte for byte,
//! the setting turns it all off, and only the person can read the lines. Its own test binary: it
//! changes PATH and listens on a real port the stand-in CLI calls back on.

#[path = "common/exe.rs"]
mod exe;

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
        create_router, mcp_runtime,
        missions::{lessons, runner},
        AppState,
    },
    tools::ToolRunner,
};
use serde_json::{json, Value};
use std::path::PathBuf;
use std::sync::OnceLock;
use std::time::Duration;
use tempfile::{tempdir, TempDir};
use tower::ServiceExt;

/// On a Mission Control prompt: logs the prompt (with the task it holds) to prompts.log next to
/// itself; when the brief asks for it, runs the task's checks first (task_request_check); then
/// moves the task to review through its kwant tools.
const FAKE_CLAUDE: &str = r#"#!/usr/bin/env python3
import json, os, re, sys, urllib.request
args = sys.argv[1:]
if args[:1] == ["auth"]:
    print('{"loggedIn":true}')
    sys.exit(0)
prompt = args[-1] if args else ""
held = re.search(r"You hold task (\S+) of", prompt)
if held and "--mcp-config" in args:
    tid = held.group(1)
    with open(os.path.join(os.path.dirname(os.path.abspath(__file__)), "prompts.log"), "a") as f:
        f.write(json.dumps({"task": tid, "prompt": prompt}) + "\n")
    with open(args[args.index("--mcp-config") + 1]) as f:
        servers = json.load(f).get("mcpServers", {})
    for s in servers.values():
        if s.get("url", "").endswith("/api/mcp/self"):
            headers = dict(s.get("headers", {}), **{"content-type": "application/json"})
            calls = []
            if "task_request_check" in prompt:
                calls.append(("task_request_check", {"task_id": tid}))
            calls.append(("task_update", {"task_id": tid, "status": "review"}))
            for n, (name, arguments) in enumerate(calls):
                call = {"jsonrpc": "2.0", "id": n + 1, "method": "tools/call", "params": {"name": name, "arguments": arguments}}
                req = urllib.request.Request(s["url"], data=json.dumps(call).encode(), headers=headers)
                urllib.request.urlopen(req, timeout=30).read()
print(json.dumps({"type": "result", "is_error": False, "num_turns": 1, "result": "done, sent for review",
                  "usage": {"input_tokens": 10, "output_tokens": 10}}))
"#;

fn fake_cli() -> PathBuf {
    static DONE: OnceLock<PathBuf> = OnceLock::new();
    DONE.get_or_init(|| {
        let bin = tempfile::tempdir().unwrap().keep().join("bin");
        std::fs::create_dir_all(&bin).unwrap();
        exe::write_executable(bin.join("claude"), FAKE_CLAUDE);
        std::env::set_var("PATH", format!("{}:{}", bin.display(), std::env::var("PATH").unwrap_or_default()));
        bin
    })
    .clone()
}

/// The prompts the stand-in CLI got for `tid`, in order.
fn prompts(tid: &str) -> Vec<String> {
    let log = std::fs::read_to_string(fake_cli().join("prompts.log")).unwrap_or_default();
    log.lines()
        .filter_map(|l| serde_json::from_str::<Value>(l).ok())
        .filter(|v| v["task"] == tid)
        .map(|v| v["prompt"].as_str().unwrap_or("").to_string())
        .collect()
}

struct Env {
    app: Router,
    state: AppState,
    general: String,
    _tmp: TempDir,
}

async fn setup() -> Env {
    fake_cli();
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
    let general = state.db.list_channels(&space).await.unwrap().into_iter().find(|c| c.name == "general").unwrap().id;
    state.db.set_space_approvals(&space, "off").await.unwrap();
    let e = Env { app, state, general, _tmp: tmp };
    // AutoFix off: a failing check fails the task once, with no retry turns
    person(&e, "POST", "/api/settings", Some(json!({"mission_board": true, "autofix": "off"}))).await;
    e
}

async fn send(app: &Router, method: &str, uri: &str, auth: Option<&str>, body: Option<Value>) -> (StatusCode, Value) {
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

async fn person(e: &Env, method: &str, uri: &str, body: Option<Value>) -> Value {
    let (status, v) = send(&e.app, method, uri, None, body).await;
    assert!(status.is_success(), "{method} {uri}: {status} {v}");
    v
}

async fn ok(e: &Env, token: &str, name: &str, args: Value) -> Value {
    let rpc = json!({"jsonrpc": "2.0", "id": 1, "method": "tools/call", "params": {"name": name, "arguments": args}});
    let (status, v) = send(&e.app, "POST", "/api/mcp/self", Some(token), Some(rpc)).await;
    assert_eq!(status, StatusCode::OK, "{v}");
    let text = v["result"]["content"][0]["text"].as_str().unwrap_or("").to_string();
    assert!(!v["result"]["isError"].as_bool().unwrap_or(false), "{name} {args} failed: {text}");
    serde_json::from_str(&text).unwrap_or(Value::Null)
}

/// A turn of `bot` in #general with its own tools; returns the turn token.
async fn turn(e: &Env, bot: &Kwant) -> String {
    let turn_id = format!("t-{}", uuid::Uuid::new_v4());
    let origin = TurnOrigin { channel_id: Some(e.general.clone()), ..Default::default() };
    let scope = SelfScope {
        kwant_id: bot.id.clone(),
        kwant_name: bot.name.clone(),
        origin: Some(origin),
        turn_id: Some(turn_id.clone()),
        ..Default::default()
    };
    mcp_runtime::prepare_turn_with_self(&e.state, bot, Some(turn_id), None, Some(scope)).await.unwrap().1
}

fn id(v: &Value) -> String {
    v["id"].as_str().unwrap().to_string()
}

async fn task(e: &Env, id: &str) -> Value {
    person(e, "GET", &format!("/api/tasks/{id}"), None).await["task"].clone()
}

async fn wait_state(e: &Env, tid: &str, want: &str) -> Value {
    for _ in 0..300 {
        let t = task(e, tid).await;
        if t["state"] == want {
            return t;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    panic!("{tid} never got to {want}: {}", task(e, tid).await);
}

/// A bot on the stand-in claude CLI.
async fn cli_bot(e: &Env, name: &str) -> Kwant {
    let bot = json!({"name": name, "title": format!("{name} the tester"), "persona_prompt": "Test persona",
                     "avatar_sprite": "s", "driver_type": "cli", "preferred_model": "claude"});
    let bid = id(&person(e, "POST", "/api/kwants", Some(bot)).await);
    e.state.db.get_kwant(&bid).await.unwrap().unwrap()
}

/// A person task of `bot` outside any mission, claimed by `bot` and handed in; its checks run.
async fn handed_in(e: &Env, bot: &Kwant, title: &str, checks: Value) -> String {
    let body = json!({"title": title, "acceptance": "works", "owner_kwant_id": bot.id, "checks": checks});
    let tid = id(&person(e, "POST", "/api/tasks", Some(body)).await);
    let row = e.state.db.get_task(&tid).await.unwrap().unwrap();
    std::fs::create_dir_all(kwnt::server::missions::task_folder(&e.state, &row).await.unwrap()).unwrap();
    let token = turn(e, bot).await;
    ok(e, &token, "task_claim", json!({"task_id": tid})).await;
    ok(e, &token, "task_update", json!({"task_id": tid, "status": "review"})).await;
    tid
}

fn passing_check() -> Value {
    json!([{"kind": "command", "spec": {"command": "true"}}])
}

/// A mission of one task for `bot` (title "Ship it", task "Publish the notes" with a passing
/// check) that is not held out unless `held` says so; returns its ids.
async fn mission(e: &Env, bot: &Kwant, held: bool) -> (String, String) {
    for _ in 0..60 {
        let body = json!({"title": "Ship it", "goal": "a release", "lead_kwant_id": bot.id, "budget_tokens": 900_000,
                          "channel_id": e.general, "tasks": [{"title": "Publish the notes", "owner": bot.id, "checks": passing_check()}]});
        let mid = id(&person(e, "POST", "/api/missions", Some(body)).await);
        let detail = person(e, "GET", &format!("/api/missions/{mid}"), None).await;
        let tid = id(&detail["tasks"][0]);
        if lessons::held_out(&tid) == held {
            let row = e.state.db.get_mission(&mid).await.unwrap().unwrap();
            std::fs::create_dir_all(&row.workspace_path).unwrap();
            return (mid, tid);
        }
        // The other arm: leave it unrun
        person(e, "POST", &format!("/api/missions/{mid}/cancel"), None).await;
    }
    panic!("no task id landed on held_out == {held} in 60 tries");
}

/// Runs the mission's one task to the end of its turn; returns the prompt the CLI got.
async fn run(e: &Env, mid: &str, tid: &str) -> String {
    person(e, "POST", &format!("/api/missions/{mid}/run"), Some(json!({"max_tasks": 1}))).await;
    wait_state(e, tid, "verified").await;
    for _ in 0..300 {
        if !runner::is_running(&e.state, mid) {
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    tokio::time::sleep(Duration::from_millis(200)).await;
    let p = prompts(tid);
    assert_eq!(p.len(), 1, "{p:?}");
    p[0].clone()
}

/// The task's events of `kind`, as their detail JSON.
async fn events(e: &Env, tid: &str, kind: &str) -> Vec<Value> {
    e.state.db.task_events(tid, 500).await.unwrap().into_iter().filter(|ev| ev.kind == kind).map(|ev| ev.detail_value()).collect()
}

/// The lessons block of a brief: from its header to the end.
fn block_of(prompt: &str) -> Option<&str> {
    prompt.find("[Lessons from earlier tasks]").map(|i| &prompt[i..])
}

#[tokio::test]
async fn a_failed_check_and_a_send_back_reach_the_next_brief_and_are_attributed() {
    let e = setup().await;
    let ann = cli_bot(&e, "Ann").await;

    // A: an approved check fails at hand-in
    let a = handed_in(&e, &ann, "Export the sales CSV", json!([{"kind": "command", "spec": {"command": "echo expected 42 got 41; exit 1"}}])).await;
    for _ in 0..300 {
        if !e.state.db.failures_of(&a).await.unwrap().is_empty() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert_eq!(e.state.db.failures_of(&a).await.unwrap().len(), 1);
    // B: the person sends it back with a reason
    let b = handed_in(&e, &ann, "Write the release note", json!([])).await;
    wait_state(&e, &b, "review").await;
    person(&e, "POST", &format!("/api/tasks/{b}/reject"), Some(json!({"reason": "Too long, keep it under five lines"}))).await;

    // C: the next task's brief carries both, after the brief, with the self-check sentence
    let (mid, c) = mission(&e, &ann, false).await;
    let p = run(&e, &mid, &c).await;
    assert!(p.contains(runner::SELF_CHECK), "{p}");
    let block = block_of(&p).unwrap_or_else(|| panic!("no lessons block: {p}"));
    println!("RENDERED BLOCK >>>\n{block}\n<<< RENDERED BLOCK");
    assert!(p.find(runner::SELF_CHECK).unwrap() < p.find(block).unwrap(), "the block comes after the brief");
    assert!(p.contains(&format!("\n\n{}", lessons::HEADER)), "{p}");
    let lines: Vec<&str> = block.lines().filter(|l| l.starts_with("- ")).collect();
    assert_eq!(lines.len(), 2, "{block}");
    let check = lines.iter().find(|l| l.contains("Export the sales CSV")).unwrap_or_else(|| panic!("{block}"));
    assert!(check.contains("echo expected 42 got 41; exit 1"), "the approved check's command: {check}");
    assert!(check.contains("expected 42 got 41"), "{check}");
    let review = lines.iter().find(|l| l.contains("Write the release note")).unwrap_or_else(|| panic!("{block}"));
    assert!(review.contains("Too long, keep it under five lines"), "{review}");

    // Attribution and the self-check, both logged by kwant
    let shown = events(&e, &c, "lessons_shown").await;
    assert_eq!(shown.len(), 1, "{shown:?}");
    assert_eq!(shown[0]["keys"].as_array().unwrap().len(), 2, "{shown:?}");
    assert!(shown[0]["keys"].as_array().unwrap().iter().any(|k| k.as_str().unwrap().starts_with("check_failed:")), "{shown:?}");
    assert!(shown[0]["keys"].as_array().unwrap().iter().any(|k| k.as_str().unwrap().starts_with("rejected:")), "{shown:?}");
    assert_eq!(shown[0]["bytes"].as_u64(), Some(block.len() as u64 + 2), "{shown:?}");
    assert!(events(&e, &c, "lessons_held_out").await.is_empty());
    let checked = events(&e, &c, "self_checked").await;
    assert_eq!(checked.len(), 1, "{checked:?}");
    assert_eq!(checked[0]["checks"].as_array().unwrap().len(), 1, "{checked:?}");

    // A held-out task gets no block, but the keys it would have shown are logged
    let (hmid, h) = mission(&e, &ann, true).await;
    let hp = run(&e, &hmid, &h).await;
    assert!(block_of(&hp).is_none(), "{hp}");
    assert!(hp.contains(runner::SELF_CHECK), "{hp}");
    let held = events(&e, &h, "lessons_held_out").await;
    assert_eq!(held.len(), 1, "{held:?}");
    assert_eq!(held[0]["keys"].as_array().unwrap().len(), 2, "{held:?}");
    assert!(held[0].get("bytes").is_none(), "{held:?}");
    assert!(events(&e, &h, "lessons_shown").await.is_empty());

    // The person reads the same lines
    let v = person(&e, "GET", &format!("/api/kwants/{}/lessons", ann.id), None).await;
    assert_eq!((v["enabled"].as_bool(), v["window_days"].as_i64()), (Some(true), Some(30)), "{v}");
    let got = v["lines"].as_array().unwrap();
    assert_eq!(got.len(), 2, "{v}");
    let mut sources: Vec<&str> = got.iter().map(|l| l["source"].as_str().unwrap()).collect();
    sources.sort();
    assert_eq!(sources, ["check", "review"], "{v}");
    for l in got {
        assert!(l["key"].is_string() && l["text"].is_string() && l["last_at"].is_i64(), "{l}");
        assert_eq!(l["count"], 1, "{l}");
        assert!(lines.iter().any(|b| b[2..] == *l["text"].as_str().unwrap()), "{l} in {block}");
    }
    assert_eq!(got.iter().find(|l| l["source"] == "check").unwrap()["task_id"], a.as_str(), "{v}");
    assert_eq!(got.iter().find(|l| l["source"] == "review").unwrap()["task_id"], b.as_str(), "{v}");

    // Only the person, and only in the bot's own space
    let token = turn(&e, &ann).await;
    let (s, v) = send(&e.app, "GET", &format!("/api/kwants/{}/lessons", ann.id), Some(&token), None).await;
    assert_eq!(s, StatusCode::FORBIDDEN, "{v}");
    let (s, v) = send(&e.app, "POST", "/api/spaces", None, Some(json!({"name": "Other", "icon": "🧪", "this_machine_enabled": false}))).await;
    assert_eq!(s, StatusCode::CREATED, "{v}");
    let other = id(&v);
    let (s, v) = send(&e.app, "GET", &format!("/api/kwants/{}/lessons?space={other}", ann.id), None, None).await;
    assert_eq!(s, StatusCode::NOT_FOUND, "{v}");
    let (s, v) = send(&e.app, "GET", "/api/kwants/no-such-bot/lessons", None, None).await;
    assert_eq!(s, StatusCode::NOT_FOUND, "{v}");

    // Off: no block, no attribution, no self-check event; the route still answers
    person(&e, "POST", "/api/settings", Some(json!({"lessons": "off"}))).await;
    assert_eq!(person(&e, "GET", "/api/settings", None).await["lessons"], "off");
    let (omid, o) = mission(&e, &ann, false).await;
    let op = run(&e, &omid, &o).await;
    assert!(block_of(&op).is_none(), "{op}");
    assert!(op.contains(runner::SELF_CHECK), "the self-check sentence doesn't depend on the setting: {op}");
    for kind in ["lessons_shown", "lessons_held_out", "self_checked"] {
        assert!(events(&e, &o, kind).await.is_empty(), "{kind}");
    }
    let v = person(&e, "GET", &format!("/api/kwants/{}/lessons", ann.id), None).await;
    assert_eq!(v["enabled"], false, "{v}");
    assert_eq!(v["lines"].as_array().unwrap().len(), 2, "{v}");
    let (s, v) = send(&e.app, "POST", "/api/settings", None, Some(json!({"lessons": "maybe"}))).await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "{v}");
}

#[tokio::test]
async fn a_bot_with_nothing_to_learn_gets_the_same_brief_byte_for_byte() {
    let e = setup().await;
    let bob = cli_bot(&e, "Bob").await;
    assert_eq!(person(&e, "GET", "/api/settings", None).await["lessons"], "on", "on by default");

    let (mid, on) = mission(&e, &bob, false).await;
    let with = run(&e, &mid, &on).await.replace(&on, "TASK");
    assert!(events(&e, &on, "lessons_shown").await.is_empty(), "nothing shown, nothing logged");
    assert!(events(&e, &on, "lessons_held_out").await.is_empty());
    assert_eq!(events(&e, &on, "self_checked").await.len(), 1, "the self-check is logged with the setting on");

    person(&e, "POST", "/api/settings", Some(json!({"lessons": "off"}))).await;
    let (mid, off) = mission(&e, &bob, false).await;
    let without = run(&e, &mid, &off).await.replace(&off, "TASK");
    // The brief, from its first line (the conversation history before it grows with each mission)
    let brief = |p: &str| p[p.find("[Mission Control]").unwrap()..].to_string();
    assert_eq!(brief(&with), brief(&without));
    assert!(block_of(&with).is_none(), "{with}");
    let v = person(&e, "GET", &format!("/api/kwants/{}/lessons", bob.id), None).await;
    assert_eq!(v["lines"], json!([]), "{v}");
}

/// A verified task of `bot` with a file check, its "Keep this true" goal made and met once.
async fn kept(e: &Env, bot: &Kwant, title: &str) -> kwnt::db::goals::Goal {
    let body = json!({"title": title, "acceptance": "the file is there", "owner_kwant_id": bot.id,
                      "checks": [{"kind": "file_exists", "spec": {"path": "ok.txt"}}]});
    let tid = id(&person(e, "POST", "/api/tasks", Some(body)).await);
    let row = e.state.db.get_task(&tid).await.unwrap().unwrap();
    let folder = kwnt::server::missions::task_folder(&e.state, &row).await.unwrap();
    std::fs::create_dir_all(&folder).unwrap();
    std::fs::write(folder.join("ok.txt"), "x").unwrap();
    let token = turn(e, bot).await;
    ok(e, &token, "task_claim", json!({"task_id": tid})).await;
    ok(e, &token, "task_update", json!({"task_id": tid, "status": "review"})).await;
    for _ in 0..300 {
        let t = task(e, &tid).await;
        if t["state"] == "verified" {
            break;
        }
        if t["state"] == "review" && e.state.db.list_checks(&tid).await.unwrap().iter().all(|c| c.status == "passed") {
            person(e, "POST", &format!("/api/tasks/{tid}/verify"), Some(json!({}))).await;
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    wait_state(e, &tid, "verified").await;
    let v = person(e, "POST", "/api/goals", Some(json!({"from_task": tid}))).await;
    let g = e.state.db.get_goal(v["goal"]["id"].as_str().unwrap()).await.unwrap().unwrap();
    assert_eq!(run_goal(e, &g.id).await.status, "met");
    g
}

async fn run_goal(e: &Env, gid: &str) -> kwnt::db::goals::Goal {
    let g = e.state.db.lease_goal(gid, chrono::Utc::now().timestamp_millis(), false).await.unwrap().expect("a lease");
    kwnt::server::goals::runner::run_goal(&e.state, g).await.expect("a finished run")
}

/// The goal's file goes missing and the loop confirms it (two runs in a row).
async fn lapse(e: &Env, g: &kwnt::db::goals::Goal) {
    let _ = std::fs::remove_file(std::path::Path::new(&g.folder).join("ok.txt"));
    run_goal(e, &g.id).await;
    assert_eq!(run_goal(e, &g.id).await.status, "not_met");
}

async fn route_lines(e: &Env, bot: &Kwant) -> Vec<Value> {
    person(e, "GET", &format!("/api/kwants/{}/lessons", bot.id), None).await["lines"].as_array().unwrap().clone()
}

#[tokio::test]
async fn a_goal_that_stopped_holding_reaches_the_brief_and_the_route_until_it_holds_again() {
    let e = setup().await;
    let ann = cli_bot(&e, "Ann").await;
    let cid = cli_bot(&e, "Cid").await;
    let g = kept(&e, &ann, "Publish the price list").await;
    let theirs = kept(&e, &cid, "Refresh the store hours").await;
    assert!(route_lines(&e, &ann).await.is_empty(), "a goal that holds is no lesson");
    lapse(&e, &g).await;
    lapse(&e, &theirs).await;

    // The route: one goal line, only the bot's own
    let lines = route_lines(&e, &ann).await;
    assert_eq!(lines.len(), 1, "{lines:?}");
    let l = &lines[0];
    assert_eq!((l["source"].as_str(), l["key"].as_str()), (Some("goal"), Some(format!("goal:{}", g.id).as_str())), "{l}");
    let today = chrono::Utc::now().format("%Y-%m-%d").to_string();
    assert_eq!(
        l["text"].as_str().unwrap(),
        format!("A goal you worked on, \"Publish the price list\", is not met since {today}: check it before saying a task is done.")
    );
    assert!(!l["text"].as_str().unwrap().contains("ok.txt"), "no check spec: {l}");
    let mine = route_lines(&e, &cid).await;
    assert_eq!(mine.len(), 1, "{mine:?}");
    assert!(mine[0]["text"].as_str().unwrap().contains("Refresh the store hours"), "{mine:?}");

    // Every turn and the next task's brief carry it, attributed by its key
    let turn_block = lessons::for_turn(&e.state, &ann.id).await;
    assert!(turn_block.contains("\"Publish the price list\", is not met since"), "{turn_block}");
    assert!(!turn_block.contains("Refresh the store hours"), "{turn_block}");
    let (mid, t) = mission(&e, &ann, false).await;
    let p = run(&e, &mid, &t).await;
    let block = block_of(&p).unwrap_or_else(|| panic!("no lessons block: {p}"));
    println!("GOAL BLOCK >>>\n{block}\n<<< GOAL BLOCK");
    assert!(block.contains(l["text"].as_str().unwrap()), "{block}");
    assert!(!block.contains("Refresh the store hours"), "another bot's lapse: {block}");
    let shown = events(&e, &t, "lessons_shown").await;
    assert_eq!(shown.len(), 1, "{shown:?}");
    assert_eq!(shown[0]["keys"], json!([format!("goal:{}", g.id)]), "{shown:?}");

    // Met again: no line anywhere
    std::fs::write(std::path::Path::new(&g.folder).join("ok.txt"), "x").unwrap();
    assert_eq!(run_goal(&e, &g.id).await.status, "met");
    assert!(route_lines(&e, &ann).await.is_empty());
    assert_eq!(lessons::for_turn(&e.state, &ann.id).await, "");
    let (mid, t) = mission(&e, &ann, false).await;
    assert!(block_of(&run(&e, &mid, &t).await).is_none());
    assert!(events(&e, &t, "lessons_shown").await.is_empty());
    // The other bot's goal still hasn't recovered
    assert_eq!(route_lines(&e, &cid).await.len(), 1);
}
