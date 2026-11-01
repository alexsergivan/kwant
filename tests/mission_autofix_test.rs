//! Mission Control Phase B3: AutoFix. A failure is recorded with a class and redacted
//! diagnostics; with `autofix` on, a retryable one gets a bounded attempt (its own strategy, the
//! task's attempts, a recovery budget), and a permanent change is only ever proposed until the
//! person approves it. These are the contract's 10 acceptance tests (docs/mission-control.md §13),
//! numbered as there. Its own test binary: it changes PATH and listens on a real port the
//! stand-in CLI calls back on.

#[path = "common/exe.rs"]
mod exe;

use axum::{
    body::{to_bytes, Body},
    http::{Request, StatusCode},
    Router,
};
use chrono::{DateTime, Utc};
use kwnt::{
    broker::PermissionBroker,
    config::Config,
    db::{autofix::NewAttempt, continuity::NewRoutine, models::Kwant, spaces::SpaceId, Database},
    router::LayaRouter,
    server::{
        continuity::{SelfScope, TurnOrigin},
        create_router, mcp_runtime,
        missions::{autofix::classify, listener, runner},
        AppState,
    },
    tools::ToolRunner,
};
use serde_json::{json, Value};
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use std::sync::OnceLock;
use std::time::Duration;
use tempfile::{tempdir, TempDir};
use tower::ServiceExt;

/// On a Mission Control prompt: logs the prompt (with the task it holds) to prompts.log next to
/// itself, streams its usage so far (10 input, 10 output), moves the task to review through its
/// kwant tools, then reports the same final usage: 20 fresh tokens a turn, all known at hand-in.
const FAKE_CLAUDE: &str = r#"#!/usr/bin/env python3
import json, os, re, sys, urllib.request
args = sys.argv[1:]
if args[:1] == ["auth"]:
    print('{"loggedIn":true}')
    sys.exit(0)
prompt = args[-1] if args else ""
held = re.search(r"You hold task (\S+) of", prompt)
if held and "--mcp-config" in args:
    with open(os.path.join(os.path.dirname(os.path.abspath(__file__)), "prompts.log"), "a") as f:
        f.write(json.dumps({"task": held.group(1), "prompt": prompt}) + "\n")
    start = {"type": "message_start", "message": {"id": "msg_1", "usage": {"input_tokens": 10, "output_tokens": 1}}}
    print(json.dumps({"type": "stream_event", "event": start}), flush=True)
    print(json.dumps({"type": "stream_event", "event": {"type": "message_delta", "usage": {"output_tokens": 10}}}), flush=True)
    with open(args[args.index("--mcp-config") + 1]) as f:
        servers = json.load(f).get("mcpServers", {})
    for s in servers.values():
        if s.get("url", "").endswith("/api/mcp/self"):
            call = {"jsonrpc": "2.0", "id": 1, "method": "tools/call",
                    "params": {"name": "task_update", "arguments": {"task_id": held.group(1), "status": "review"}}}
            headers = dict(s.get("headers", {}), **{"content-type": "application/json"})
            req = urllib.request.Request(s["url"], data=json.dumps(call).encode(), headers=headers)
            urllib.request.urlopen(req, timeout=30).read()
print(json.dumps({"type": "result", "is_error": False, "num_turns": 1, "result": "done, sent for review",
                  "usage": {"input_tokens": 10, "output_tokens": 10}}))
"#;

/// Fresh tokens of one stand-in CLI turn.
const PER_TURN: i64 = 20;

fn fake_cli() -> PathBuf {
    static DONE: OnceLock<PathBuf> = OnceLock::new();
    DONE.get_or_init(|| {
        let bin = tempfile::tempdir().unwrap().keep().join("bin");
        std::fs::create_dir_all(&bin).unwrap();
        let fake = bin.join("claude");
        exe::write_executable(&fake, FAKE_CLAUDE);
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
    bot: Kwant,
    general: String,
    tmp: TempDir,
}

/// kwant on a real port (the stand-in CLI calls back), with the mission board on.
async fn setup() -> Env {
    // The CLI lookup is cached per process: put the stand-in first before any test can resolve `claude`
    fake_cli();
    let tmp = tempdir().unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let db = Database::init(tmp.path()).await.unwrap();
    let state = state_on(&tmp, port, db);
    let app = create_router(state.clone());
    let served = app.clone();
    tokio::spawn(async move { axum::serve(listener, served).await.unwrap() });
    let space = SpaceId::default_space();
    let bot = state.db.list_kwants(&space).await.unwrap()[0].clone();
    let general = state.db.list_channels(&space).await.unwrap().into_iter().find(|c| c.name == "general").unwrap().id;
    let e = Env { app, state, bot, general, tmp };
    person(&e, "POST", "/api/settings", Some(json!({"mission_board": true}))).await;
    e
}

fn state_on(tmp: &TempDir, port: u16, db: Database) -> AppState {
    let config = Config {
        data_dir: tmp.path().to_path_buf(),
        host: "127.0.0.1".into(),
        port,
        laya_url: "http://127.0.0.1:9".into(),
        browser_cdp_url: Some("http://127.0.0.1:9".into()),
        ..Config::default()
    };
    AppState::new(
        config.clone(),
        db,
        ToolRunner::new(tmp.path().join("workspace")),
        PermissionBroker::new(),
        LayaRouter::new(config.laya_url.clone()),
    )
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
    person(e, "GET", &format!("/api/tasks/{id}"), None).await["task"].clone()
}

async fn failures(e: &Env, tid: &str) -> Vec<Value> {
    person(e, "GET", &format!("/api/tasks/{tid}/failures"), None).await.as_array().unwrap().clone()
}

async fn autofix(e: &Env, mode: &str, pct: i64) {
    person(e, "POST", "/api/settings", Some(json!({"autofix": mode, "recovery_budget_pct": pct}))).await;
}

/// Waits until `tid` is in `want` (up to ~15 s).
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

/// A person task of the default bot with `checks`, outside any mission (no runner turns start),
/// and its folder.
async fn person_task(e: &Env, checks: Value) -> String {
    let body = json!({"title": "Fix it", "acceptance": "works", "owner_kwant_id": e.bot.id, "checks": checks});
    let t = person(e, "POST", "/api/tasks", Some(body)).await;
    let tid = id(&t);
    let row = e.state.db.get_task(&tid).await.unwrap().unwrap();
    std::fs::create_dir_all(kwnt::server::missions::task_folder(&e.state, &row).await.unwrap()).unwrap();
    tid
}

/// The default bot claims `tid` in a new turn; returns the turn id and its token.
async fn claim(e: &Env, tid: &str) -> (String, String) {
    let turn_id = format!("t-{}", uuid::Uuid::new_v4());
    let token = turn(e, &e.bot, &turn_id).await;
    ok(e, &token, "task_claim", json!({"task_id": tid})).await;
    (turn_id, token)
}

/// Claims `tid` and hands it in; its checks run.
async fn hand_in(e: &Env, tid: &str) {
    let (_, token) = claim(e, tid).await;
    ok(e, &token, "task_update", json!({"task_id": tid, "status": "review"})).await;
}

/// A failing command check whose output says what went wrong.
fn failing_check() -> Value {
    json!([{"kind": "command", "spec": {"command": "echo expected 42 got 41; exit 1"}}])
}

/// A bot that runs on the stand-in claude CLI, with approvals off for the space.
async fn cli_bot(e: &Env) -> String {
    fake_cli();
    e.state.db.set_space_approvals(&SpaceId::default_space(), "off").await.unwrap();
    let bot = json!({"name": "Ann", "title": "Ann the tester", "persona_prompt": "Test persona",
                     "avatar_sprite": "s", "driver_type": "cli", "preferred_model": "claude"});
    id(&person(e, "POST", "/api/kwants", Some(bot)).await)
}

/// A mission of one task for the stand-in CLI bot with the failing check; returns its ids.
async fn cli_mission(e: &Env, budget: i64) -> (String, String) {
    let ann = cli_bot(e).await;
    let body = json!({"title": "Ship it", "goal": "a release", "lead_kwant_id": ann, "budget_tokens": 900_000,
                      "channel_id": e.general, "tasks": [{"title": "A", "owner": ann, "budget_tokens": budget, "checks": failing_check()}]});
    let m = person(e, "POST", "/api/missions", Some(body)).await;
    let mid = id(&m);
    let detail = person(e, "GET", &format!("/api/missions/{mid}"), None).await;
    let tid = id(&detail["tasks"][0]);
    let row = e.state.db.get_mission(&mid).await.unwrap().unwrap();
    std::fs::create_dir_all(&row.workspace_path).unwrap();
    (mid, tid)
}

/// Waits until the mission's runner turn is over and its spend has landed.
async fn runner_idle(mid: &str) {
    for _ in 0..300 {
        if !runner::is_running(mid) {
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    tokio::time::sleep(Duration::from_millis(200)).await;
}

async fn attempts(e: &Env, tid: &str) -> Vec<kwnt::db::autofix::Attempt> {
    e.state.db.attempts_of(tid).await.unwrap()
}

fn kinds(items: &Value) -> Vec<String> {
    items.as_array().unwrap().iter().map(|i| i["kind"].as_str().unwrap().to_string()).collect()
}

fn at(v: &Value) -> DateTime<Utc> {
    v.as_str().unwrap().parse().unwrap()
}

// 1
#[test]
fn classify_maps_every_signal_to_its_class() {
    let rows = [
        // engine drivers (claude CLI, codex, grok, the API driver) and routine runs
        ("turn", "error", "Not logged in · Please run /login", "auth"),
        ("turn", "error", "Not signed in: run `claude auth login` and try again", "auth"),
        ("turn", "error", "OAuth token has expired. Please obtain a new token or refresh your existing token.", "auth"),
        ("turn", "error", "API key rejected (401)", "auth"),
        ("turn", "error", "unexpected status 401 Unauthorized: Reconnecting… timed out", "auth"),
        ("turn", "error", "Credit balance is too low", "quota"),
        ("turn", "error", "Claude AI usage limit reached|1792162800", "quota"),
        ("turn", "error", "You've hit your limit · resets 5pm", "quota"),
        ("turn", "error", "5-hour limit reached ∙ resets 3pm", "quota"),
        ("turn", "error", "API Error: 429 usage limit reached", "quota"),
        ("turn", "error", "Error: insufficient_quota: You exceeded your current quota", "quota"),
        ("turn", "error", "API Error: 529 {\"type\":\"error\",\"error\":{\"type\":\"overloaded_error\"}}", "transient"),
        ("turn", "error", "stream error: rate-limiting (HTTP 429)", "transient"),
        ("routine", "error", "Quota exceeded (429)", "quota"),
        ("handoff", "error", "error sending request: connection refused", "transient"),
        ("turn", "error", "upstream 503 Service Unavailable", "transient"),
        ("turn", "error", "the model said something odd", "unknown"),
        ("turn", "error", "ran 4290 steps", "unknown"),
        // checks
        ("check", "failed", "command echo\nexit code 1\nexpected 42 got 41", "check_failed"),
        ("check", "failed", "command nosuchcmd\nexit code 127\nsh: 1: nosuchcmd: not found", "bad_spec"),
        ("check", "failed", "command x\nexit code 1\nbash: x: command not found", "bad_spec"),
        ("check", "error", "Check timed out after 30s", "transient"),
        ("check", "error", "file out.csv: path escapes the task folder", "bad_spec"),
        // what the board itself saw
        ("lease", "stalled", "", "stalled"),
        ("budget", "error", "over budget: 120 of 100 tokens", "budget"),
        ("turn", "stalled", "", "stalled"),
        ("turn", "denied", "mcp__gmail__send: send to bob", "denied"),
        ("turn", "needs_input", "", "needs_input"),
    ];
    for (source, status, text, want) in rows {
        assert_eq!(classify(source, status, text), want, "{source}/{status}: {text}");
    }
}

// 2
#[tokio::test]
async fn a_failing_check_is_retried_with_its_diagnostics_until_the_attempts_run_out() {
    let e = setup().await;
    autofix(&e, "on", 25).await;
    let (mid, tid) = cli_mission(&e, 50_000).await;
    person(&e, "POST", &format!("/api/missions/{mid}/run"), Some(json!({"max_tasks": 1}))).await;
    // Three runs: the first, then two AutoFix retries; the last failure has no attempt left
    for _ in 0..600 {
        let t = task(&e, &tid).await;
        if t["state"] == "failed" && t["attempts"] == 3 && failures(&e, &tid).await.len() == 3 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    runner_idle(&mid).await;
    let t = task(&e, &tid).await;
    assert_eq!((t["state"].as_str(), t["attempts"].as_i64()), (Some("failed"), Some(3)), "{t}");
    let fs = failures(&e, &tid).await;
    assert_eq!(fs.len(), 3, "{fs:?}");
    assert!(fs.iter().all(|f| f["class"] == "check_failed" && f["source"] == "check"), "{fs:?}");
    assert!(fs[0]["excerpt"].as_str().unwrap().contains("expected 42 got 41"), "{fs:?}");
    let a = attempts(&e, &tid).await;
    let got: Vec<(&str, &str)> = a.iter().map(|a| (a.strategy.as_str(), a.status.as_str())).collect();
    assert_eq!(
        got,
        [("retry_with_diagnostics", "failed"), ("retry_with_diagnostics", "failed"), ("retry_with_diagnostics", "skipped")],
        "{a:?}"
    );
    assert!(a[2].reason.starts_with("no_attempts_left"), "{a:?}");
    assert_eq!(t["autofix_attempts"], 2, "two retries ran; the skipped one doesn't count: {t}");
    let ins = person(&e, "GET", "/api/missions/insights", None).await;
    assert_eq!(ins["failures_by_class_7d"]["check_failed"], 3, "{ins}");
    assert_eq!(ins["autofix_7d"]["attempts"], 2, "{ins}");
    // The retries' prompts carry the check's output and the attempt's idempotency key
    let p = prompts(&tid);
    assert_eq!(p.len(), 3, "{p:?}");
    assert!(!p[0].contains("[AutoFix]"), "{}", p[0]);
    for (n, prompt) in p.iter().enumerate().skip(1) {
        assert!(prompt.contains(&format!("[AutoFix] This is attempt {} of 3", n + 1)), "{prompt}");
        assert!(prompt.contains("expected 42 got 41"), "{prompt}");
        assert!(prompt.contains("KWANT_IDEMPOTENCY_KEY="), "{prompt}");
    }
    assert!(t["needs_you"].as_array().unwrap().iter().all(|i| i["kind"] != "autofix_suggested"), "{t}");
    // No attempt can be scheduled past max_attempts, even in SQL
    let sql = format!(
        "INSERT INTO autofix_attempts (id, task_id, failure_id, strategy, idempotency_key, status, reason, created_at) \
         VALUES ('extra', '{tid}', '{}', 'retry_same', 'extra-key', 'scheduled', 'more', '2026-10-27T00:00:00Z')",
        fs[2]["id"].as_str().unwrap()
    );
    let err = e.state.db.exec_raw_for_test(&sql).await.unwrap_err().to_string();
    assert!(err.contains("within the task's attempts"), "{err}");
}

// 3
#[tokio::test]
async fn off_records_only_and_suggest_waits_for_the_person() {
    let e = setup().await;
    // off (the default): Phase A behaviour, the failure is only recorded
    let off = person_task(&e, failing_check()).await;
    hand_in(&e, &off).await;
    let t = wait_state(&e, &off, "ready").await;
    assert_eq!(t["attempts"], 1, "{t}");
    let fs = failures(&e, &off).await;
    assert_eq!(fs.len(), 1, "{fs:?}");
    assert_eq!(fs[0]["class"], "check_failed");
    assert!(attempts(&e, &off).await.is_empty(), "off plans nothing");
    assert_eq!(t["last_failure"]["class"], "check_failed", "{t}");
    // suggest: one suggested attempt and a Needs-you item; nothing runs until the person asks
    autofix(&e, "suggest", 25).await;
    let sug = person_task(&e, failing_check()).await;
    hand_in(&e, &sug).await;
    wait_state(&e, &sug, "failed").await;
    tokio::time::sleep(Duration::from_millis(300)).await;
    kwnt::server::missions::autofix::run_due(&e.state).await;
    let t = task(&e, &sug).await;
    assert_eq!(t["state"], "failed", "nothing runs on its own: {t}");
    let a = attempts(&e, &sug).await;
    assert_eq!(a.len(), 1, "{a:?}");
    assert_eq!((a[0].strategy.as_str(), a[0].status.as_str()), ("retry_with_diagnostics", "suggested"));
    assert_eq!(t["autofix"]["status"], "suggested", "{t}");
    assert!(kinds(&t["needs_you"]).contains(&"autofix_suggested".to_string()), "{t}");
    let r = person(&e, "POST", &format!("/api/tasks/{sug}/autofix"), None).await;
    assert_eq!(r["status"], "running", "{r}");
    let t = task(&e, &sug).await;
    assert_eq!(t["state"], "ready", "{t}");
    assert!(!kinds(&t["needs_you"]).contains(&"autofix_suggested".to_string()), "{t}");
    // The bot's next claim is the attempt's: the claim note says so
    claim(&e, &sug).await;
    let note = kwnt::server::missions::autofix::retry_note(&e.state, &e.state.db.get_task(&sug).await.unwrap().unwrap()).await.unwrap();
    assert!(note.contains("expected 42 got 41") && note.contains("KWANT_IDEMPOTENCY_KEY="), "{note}");
}

// 4
#[tokio::test]
async fn a_retry_over_the_recovery_budget_is_skipped_and_only_autofix_turns_count_as_recovery() {
    let e = setup().await;
    autofix(&e, "on", 25).await;
    // A budget of 100: 25 for recovery, and each turn spends 20
    let (mid, tid) = cli_mission(&e, 100).await;
    person(&e, "POST", &format!("/api/missions/{mid}/run"), Some(json!({"max_tasks": 1}))).await;
    for _ in 0..600 {
        let a = attempts(&e, &tid).await;
        if a.len() == 2 && a[1].status == "skipped" {
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    runner_idle(&mid).await;
    let t = task(&e, &tid).await;
    // The first turn was a plain run: none of it is recovery; the AutoFix retry's turn is
    assert_eq!(t["spent_tokens"], 2 * PER_TURN, "{t}");
    assert_eq!(t["recovery_spent_tokens"], PER_TURN, "{t}");
    assert_eq!(t["recovery_budget_tokens"], 25, "{t}");
    let a = attempts(&e, &tid).await;
    assert_eq!(a.len(), 2, "{a:?}");
    assert_eq!((a[0].status.as_str(), a[0].tokens_spent), ("failed", PER_TURN), "{a:?}");
    assert_eq!(a[1].status, "skipped", "{a:?}");
    assert!(a[1].reason.starts_with("over_recovery_budget"), "{a:?}");
    assert_eq!(t["state"], "failed", "{t}");
    assert_eq!(t["attempts"], 2, "an attempt is left, but no recovery budget: {t}");
    // The person is told, not just shown a failed task
    let fix: Vec<&Value> = t["needs_you"].as_array().unwrap().iter().filter(|i| i["kind"] == "fix_needed").collect();
    assert_eq!(fix.len(), 1, "{t}");
    assert_eq!(fix[0]["text"], kwnt::server::missions::autofix::RECOVERY_BUDGET_FIX, "{t}");
    // At 10% the person can't push one through either
    autofix(&e, "on", 10).await;
    let (status, v) = send(&e.app, "POST", &format!("/api/tasks/{tid}/autofix"), None, None).await;
    assert_eq!(status, StatusCode::CONFLICT, "{v}");
    assert_eq!(v["code"], "over_recovery_budget", "{v}");
    assert_eq!(task(&e, &tid).await["state"], "failed");
}

// 5
#[tokio::test]
async fn transient_failures_back_off_1_5_25_minutes_and_survive_a_restart() {
    let e = setup().await;
    autofix(&e, "on", 25).await;
    let tid = person_task(&e, json!([])).await;
    e.state.db.exec_raw_for_test(&format!("UPDATE tasks SET max_attempts = 5 WHERE id = '{tid}'")).await.unwrap();
    let mut waits = Vec::new();
    for n in 0..3 {
        let started = Utc::now();
        claim(&e, &tid).await;
        runner::after_turn(&e.state, &tid, Some("API Error: 429 rate_limit_error: This request would exceed your rate limit"), started).await;
        assert_eq!(task(&e, &tid).await["state"], "failed");
        let fs = failures(&e, &tid).await;
        let f = fs.last().unwrap();
        assert_eq!(f["class"], "transient", "{fs:?}");
        let a = &f["autofix"];
        assert_eq!((a["strategy"].as_str(), a["status"].as_str()), (Some("wait_and_retry"), Some("scheduled")), "{a}");
        waits.push((at(&a["not_before"]) - at(&f["created_at"])).num_minutes());
        let aid = a["id"].as_str().unwrap().to_string();
        if n == 0 {
            // A restart: a new pool on the same database. The attempt is still there, once
            let db2 = Database::init(e.tmp.path()).await.unwrap();
            let state2 = state_on(&e.tmp, 9, db2);
            let row = state2.db.get_task(&tid).await.unwrap().unwrap();
            let again = state2.db.attempts_of(&tid).await.unwrap();
            assert_eq!(again.len(), 1, "{again:?}");
            let same_key = NewAttempt {
                task: &row,
                failure_id: &again[0].failure_id,
                strategy: "wait_and_retry",
                idempotency_key: again[0].idempotency_key.clone(),
                status: "scheduled",
                not_before: None,
                reason: "again".into(),
            };
            assert!(state2.db.insert_attempt(&same_key).await.unwrap().is_none(), "the same key plans nothing twice");
            let other = NewAttempt { idempotency_key: "another".into(), ..same_key };
            assert!(state2.db.insert_attempt(&other).await.is_err(), "one live attempt per task");
            // Not due yet: the tick leaves it; once due, the reloaded app runs it
            kwnt::server::missions::autofix::run_due(&state2).await;
            assert_eq!(state2.db.get_attempt(&aid).await.unwrap().unwrap().status, "scheduled");
            state2.db.exec_raw_for_test(&format!("UPDATE autofix_attempts SET not_before = '2026-01-01T00:00:00Z' WHERE id = '{aid}'")).await.unwrap();
            kwnt::server::missions::autofix::run_due(&state2).await;
            assert_eq!(state2.db.get_attempt(&aid).await.unwrap().unwrap().status, "running");
        } else {
            // The person asks now: no waiting
            let r = person(&e, "POST", &format!("/api/tasks/{tid}/autofix"), None).await;
            assert_eq!((r["id"].as_str(), r["status"].as_str()), (Some(aid.as_str()), Some("running")), "{r}");
        }
        assert_eq!(task(&e, &tid).await["state"], "ready");
    }
    assert_eq!(waits, [1, 5, 25]);
}

// 6
#[tokio::test]
async fn auth_budget_and_denied_are_never_retried_and_ask_the_person() {
    let e = setup().await;
    autofix(&e, "on", 25).await;
    // auth
    let auth = person_task(&e, json!([])).await;
    let started = Utc::now();
    claim(&e, &auth).await;
    runner::after_turn(&e.state, &auth, Some("Not logged in · Please run /login"), started).await;
    // a usage limit: a retry before it resets only burns an attempt
    let quota = person_task(&e, json!([])).await;
    let started = Utc::now();
    claim(&e, &quota).await;
    runner::after_turn(&e.state, &quota, Some("Claude AI usage limit reached|1792162800"), started).await;
    // denied
    let denied = person_task(&e, json!([])).await;
    let started = Utc::now();
    let (turn_id, _) = claim(&e, &denied).await;
    e.state
        .db
        .log_tool_call(&turn_id, &e.bot.id, &denied, "mcp__gmail__send", "external", "send to bob@example.com", "denied", None, None)
        .await
        .unwrap();
    runner::after_turn(&e.state, &denied, None, started).await;
    for (tid, class) in [(&auth, "auth"), (&quota, "quota"), (&denied, "denied")] {
        let t = task(&e, tid).await;
        assert_eq!(t["state"], "failed", "{t}");
        assert_eq!(t["last_failure"]["class"], class, "{t}");
        let a = attempts(&e, tid).await;
        assert_eq!(a.len(), 1, "{a:?}");
        assert_eq!((a[0].strategy.as_str(), a[0].status.as_str()), ("none", "skipped"), "{a:?}");
        assert!(kinds(&t["needs_you"]).contains(&"fix_needed".to_string()), "{t}");
        if class == "quota" {
            let texts = t["needs_you"].to_string();
            assert!(texts.contains("Usage limit reached: wait for it to reset or switch account"), "{t}");
        }
        let (status, v) = send(&e.app, "POST", &format!("/api/tasks/{tid}/autofix"), None, None).await;
        assert_eq!((status, v["code"].as_str()), (StatusCode::CONFLICT, Some("not_retryable")), "{v}");
    }
    // budget: over its budget, the task is blocked, and AutoFix never moves a blocked task
    let body = json!({"title": "Small", "owner_kwant_id": e.bot.id, "budget_tokens": 100});
    let budget = id(&person(&e, "POST", "/api/tasks", Some(body)).await);
    let started_ms = Utc::now().timestamp_millis() - 1000;
    let (turn_id, _) = claim(&e, &budget).await;
    kwnt::server::missions::spend(&e.state, &e.bot.id, &e.general, Some(&turn_id), started_ms, 150, 0).await;
    let t = task(&e, &budget).await;
    assert_eq!(t["state"], "blocked", "{t}");
    assert_eq!(t["last_failure"]["class"], "budget", "{t}");
    assert!(kinds(&t["needs_you"]).contains(&"fix_needed".to_string()), "{t}");
    assert!(attempts(&e, &budget).await.iter().all(|a| a.status == "skipped"));
    let (status, v) = send(&e.app, "POST", &format!("/api/tasks/{budget}/autofix"), None, None).await;
    assert_eq!((status, v["code"].as_str()), (StatusCode::CONFLICT, Some("not_retryable")), "{v}");
    kwnt::server::missions::autofix::run_due(&e.state).await;
    assert_eq!(task(&e, &budget).await["state"], "blocked");
}

// 7
#[tokio::test]
async fn a_failed_routine_run_is_retried_as_a_new_linked_run() {
    let e = setup().await;
    autofix(&e, "on", 25).await;
    let routine = e
        .state
        .db
        .create_routine(&NewRoutine {
            kwant_id: e.bot.id.clone(),
            channel_id: Some(e.general.clone()),
            name: "Morning digest".into(),
            instructions: "Post the digest".into(),
            schedule: json!({"type": "interval", "every_minutes": 60}).to_string(),
            enabled: false,
            next_run_at: None,
            thread_id: None,
            context_message_id: None,
            ..Default::default()
        })
        .await
        .unwrap();
    let run = e.state.db.start_routine_run(&routine.id, "schedule", "running", None).await.unwrap();
    listener::on_routine_run(&e.state, &routine.id, &run.id, "running").await;
    let failed = e.state.db.task_by_source("routine", &run.id).await.unwrap().unwrap();
    e.state.db.finish_routine_run(&run.id, "error", Some("stream error: rate-limiting (HTTP 429)"), None).await.unwrap();
    listener::on_routine_run(&e.state, &routine.id, &run.id, "error").await;
    let t = task(&e, &failed.id).await;
    assert_eq!(t["state"], "failed", "{t}");
    assert_eq!(t["last_failure"]["class"], "transient", "{t}");
    let r = person(&e, "POST", &format!("/api/tasks/{}/autofix", failed.id), None).await;
    assert_eq!(r["status"], "running", "{r}");
    let new_id = r["retry_task_id"].as_str().expect("the retry is a new run's task").to_string();
    assert_ne!(new_id, failed.id);
    let new = e.state.db.get_task(&new_id).await.unwrap().unwrap();
    assert_eq!(new.forked_from.as_deref(), Some(failed.id.as_str()));
    assert_eq!(new.source_kind, "routine");
    assert_ne!(new.source_id, failed.source_id, "a new routine run");
    assert_eq!(new.attempts, 1, "the failed run used one attempt");
    assert_eq!(e.state.db.latest_claim_autofix(&new_id).await.unwrap().as_deref(), Some(r["id"].as_str().unwrap()), "its spend is recovery");
    assert_eq!(task(&e, &failed.id).await["state"], "failed", "the failed task stays failed");
}

// 8
#[tokio::test]
async fn workflow_changes_are_applied_once_and_only_by_the_person() {
    let e = setup().await;
    let tid = person_task(&e, json!([])).await;
    let (_, token) = claim(&e, &tid).await;
    let p = ok(&e, &token, "workflow_propose", json!({"target": "task_acceptance", "task_id": tid, "after": "works, with tests", "reason": "the acceptance is vague"})).await;
    let cid = id(&p);
    assert_eq!(p["status"], "proposed");
    // Not even SQL applies a change nobody approved
    let err = e.state.db.exec_raw_for_test(&format!("UPDATE workflow_changes SET status = 'applied' WHERE id = '{cid}'")).await.unwrap_err().to_string();
    assert!(err.contains("only the person decides") || err.contains("only after the person approves"), "{err}");
    let listed = person(&e, "GET", "/api/workflow-changes?status=proposed", None).await;
    assert_eq!(listed[0]["before"], json!({"acceptance": "works"}), "before is read by kwant: {listed}");
    let (status, _) = send(&e.app, "POST", &format!("/api/workflow-changes/{cid}/approve"), Some(&token), None).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    let (status, _) = send(&e.app, "GET", "/api/workflow-changes", Some(&token), None).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    let c = person(&e, "POST", &format!("/api/workflow-changes/{cid}/approve"), None).await;
    assert_eq!(c["status"], "applied", "{c}");
    assert_eq!(e.state.db.get_task(&tid).await.unwrap().unwrap().acceptance, "works, with tests");
    let (status, _) = send(&e.app, "POST", &format!("/api/workflow-changes/{cid}/approve"), None, None).await;
    assert_eq!(status, StatusCode::CONFLICT, "applied once");
    // The target changed after the proposal: stale, nothing written
    let p = ok(&e, &token, "workflow_propose", json!({"target": "max_attempts", "task_id": tid, "after": 5, "reason": "flaky"})).await;
    e.state.db.exec_raw_for_test(&format!("UPDATE tasks SET max_attempts = 4 WHERE id = '{tid}'")).await.unwrap();
    let c = person(&e, "POST", &format!("/api/workflow-changes/{}/approve", id(&p)), None).await;
    assert_eq!(c["status"], "stale", "{c}");
    assert_eq!(e.state.db.get_task(&tid).await.unwrap().unwrap().max_attempts, 4);
    // A rejected one is never applied
    let p = ok(&e, &token, "workflow_propose", json!({"target": "max_attempts", "task_id": tid, "after": 6, "reason": "flaky"})).await;
    let c = person(&e, "POST", &format!("/api/workflow-changes/{}/reject", id(&p)), None).await;
    assert_eq!(c["status"], "rejected", "{c}");
    assert_eq!(e.state.db.get_task(&tid).await.unwrap().unwrap().max_attempts, 4);
}

// 9
#[tokio::test]
async fn a_bad_check_spec_only_proposes_a_change() {
    let e = setup().await;
    autofix(&e, "on", 25).await;
    let spec = json!({"command": "nosuchcmd_kwant_autofix"});
    let tid = person_task(&e, json!([{"kind": "command", "spec": spec}])).await;
    hand_in(&e, &tid).await;
    let t = wait_state(&e, &tid, "failed").await;
    assert_eq!(t["last_failure"]["class"], "bad_spec", "{t}");
    let a = attempts(&e, &tid).await;
    assert_eq!(a.len(), 1, "{a:?}");
    assert_eq!((a[0].strategy.as_str(), a[0].status.as_str()), ("propose_change", "suggested"), "{a:?}");
    let changes = person(&e, "GET", "/api/workflow-changes?status=proposed", None).await;
    let changes = changes.as_array().unwrap();
    assert_eq!(changes.len(), 1, "{changes:?}");
    assert_eq!(changes[0]["target"], "check_spec");
    assert_eq!(changes[0]["task_id"], tid.as_str());
    assert_eq!(changes[0]["after"]["required"], false, "{changes:?}");
    // The board asks for one task's changes only
    let mine = person(&e, "GET", &format!("/api/workflow-changes?status=proposed&task_id={tid}"), None).await;
    assert_eq!(mine.as_array().unwrap().len(), 1, "{mine}");
    let other = person(&e, "GET", "/api/workflow-changes?task_id=no-such-task", None).await;
    assert_eq!(other.as_array().unwrap().len(), 0, "{other}");
    assert!(kinds(&t["needs_you"]).contains(&"workflow_change".to_string()), "{t}");
    // Unchanged until approved, and AutoFix doesn't rerun a check that can't run
    let checks = e.state.db.list_checks(&tid).await.unwrap();
    assert_eq!((checks[0].spec_value(), checks[0].required), (spec.clone(), true));
    let (status, v) = send(&e.app, "POST", &format!("/api/tasks/{tid}/autofix"), None, None).await;
    assert_eq!((status, v["code"].as_str()), (StatusCode::CONFLICT, Some("not_retryable")), "{v}");
    let c = person(&e, "POST", &format!("/api/workflow-changes/{}/approve", id(&changes[0])), None).await;
    assert_eq!(c["status"], "applied", "{c}");
    let checks = e.state.db.list_checks(&tid).await.unwrap();
    assert_eq!((checks[0].spec_value(), checks[0].required), (spec, false));
}

// 10
#[tokio::test]
async fn diagnostics_are_redacted_and_their_path_never_reaches_the_api() {
    let e = setup().await;
    // Suggest: the failure stays put for the person to look at
    autofix(&e, "suggest", 25).await;
    // The secrets are only in the output, never in the spec
    let cmd = "printf 'key sk-ant-%s%s\\n' api03abcdefghij 0123456789 >&2; printf 'GET https://bob:%s%s@example.com/x\\n' hunter2 secretpw >&2; exit 1";
    let tid = person_task(&e, json!([{"kind": "command", "spec": {"command": cmd}}])).await;
    hand_in(&e, &tid).await;
    wait_state(&e, &tid, "failed").await;
    let fs = failures(&e, &tid).await;
    assert_eq!(fs.len(), 1, "{fs:?}");
    let excerpt = fs[0]["excerpt"].as_str().unwrap();
    assert!(excerpt.contains("exit code 1"), "{excerpt}");
    for secret in ["api03abcdefghij0123456789", "hunter2secretpw"] {
        assert!(!excerpt.contains(secret), "{excerpt}");
    }
    // The full diagnostics are on disk only, redacted, 0600 in a 0700 folder
    let dir = e.state.config.data_dir.join("autofix").join(&tid);
    assert_eq!(std::fs::metadata(&dir).unwrap().permissions().mode() & 0o777, 0o700);
    let files: Vec<_> = std::fs::read_dir(&dir).unwrap().map(|f| f.unwrap().path()).collect();
    assert_eq!(files.len(), 1, "{files:?}");
    assert_eq!(std::fs::metadata(&files[0]).unwrap().permissions().mode() & 0o777, 0o600);
    let body = std::fs::read_to_string(&files[0]).unwrap();
    assert!(body.contains("exit code 1") && !body.contains("api03abcdefghij0123456789") && !body.contains("hunter2secretpw"), "{body}");
    // No API response carries the path
    let root = e.state.config.data_dir.join("autofix").to_string_lossy().into_owned();
    let mut seen = Vec::new();
    for uri in [format!("/api/tasks/{tid}"), format!("/api/tasks/{tid}/failures"), "/api/tasks".into(), "/api/missions/insights".into()] {
        let (_, v) = send(&e.app, "GET", &uri, None, None).await;
        seen.push(v.to_string());
    }
    let r = person(&e, "POST", &format!("/api/tasks/{tid}/autofix"), None).await;
    seen.push(r.to_string());
    for s in &seen {
        assert!(!s.contains("diagnostics_path") && !s.contains(&root), "{s}");
        assert!(!s.contains("hunter2secretpw"), "{s}");
    }
    let note = kwnt::server::missions::autofix::retry_note(&e.state, &e.state.db.get_task(&tid).await.unwrap().unwrap()).await.unwrap();
    assert!(!note.contains("api03abcdefghij0123456789") && !note.contains("hunter2secretpw"), "{note}");
}

// Eval 9 follow-ups

#[tokio::test]
async fn a_key_typed_into_a_check_spec_is_masked_wherever_the_spec_is_shown() {
    let e = setup().await;
    autofix(&e, "suggest", 25).await;
    let secret = "Zq8VwK3mT9pL2rX7nB4c";
    // The key is glued to quotes and a redirect, as people type it; the output never prints it
    let cmd = format!("test \"$(echo nope)\" = \"{secret}\">/dev/null; echo checked; exit 1");
    let tid = person_task(&e, json!([{"kind": "command", "spec": {"command": cmd}}])).await;
    hand_in(&e, &tid).await;
    wait_state(&e, &tid, "failed").await;
    // It still ran with the real value: the stored spec is untouched
    let checks = e.state.db.list_checks(&tid).await.unwrap();
    assert!(checks[0].spec_value()["command"].as_str().unwrap().contains(secret));
    let mut seen = Vec::new();
    for uri in [format!("/api/tasks/{tid}"), format!("/api/tasks/{tid}/failures"), "/api/tasks".into()] {
        let (status, v) = send(&e.app, "GET", &uri, None, None).await;
        if status == StatusCode::OK {
            seen.push((uri, v.to_string()));
        }
    }
    assert_eq!(seen.len(), 3, "{seen:?}");
    let t = person(&e, "GET", &format!("/api/tasks/{tid}"), None).await;
    assert!(t["checks"][0]["spec"]["command"].as_str().unwrap().contains("[redacted]"), "{t}");
    for (uri, body) in &seen {
        assert!(!body.contains(secret), "{uri}: {body}");
    }
    for ev in e.state.db.task_events(&tid, 500).await.unwrap() {
        assert!(!ev.detail.contains(secret), "{ev:?}");
    }
    let dir = e.state.config.data_dir.join("autofix").join(&tid);
    for f in std::fs::read_dir(&dir).unwrap() {
        let body = std::fs::read_to_string(f.unwrap().path()).unwrap();
        assert!(body.contains("checked") && !body.contains(secret), "{body}");
    }
    let note = kwnt::server::missions::autofix::retry_note(&e.state, &e.state.db.get_task(&tid).await.unwrap().unwrap()).await;
    assert!(note.is_none_or(|n| !n.contains(secret)));
    // A bot can't copy the masked spec back as a "change"
    let token = turn(&e, &e.bot, &format!("t-{}", uuid::Uuid::new_v4())).await;
    let args = json!({"target": "check_spec", "task_id": tid, "reason": "fix it",
                      "after": {"check_id": checks[0].id, "spec": t["checks"][0]["spec"].clone()}});
    let (status, v) = send(&e.app, "POST", "/api/mcp/self", Some(&token), Some(json!({"jsonrpc": "2.0", "id": 1, "method": "tools/call", "params": {"name": "workflow_propose", "arguments": args}}))).await;
    assert_eq!(status, StatusCode::OK, "{v}");
    assert!(v.to_string().contains("masked values"), "{v}");
}

#[tokio::test]
async fn running_a_held_task_twice_with_one_key_replays_the_first_200() {
    let e = setup().await;
    let tid = person_task(&e, json!([])).await;
    assert!(e.state.db.set_task_hold(&tid, true).await.unwrap());
    let run = |key: Option<&'static str>| {
        let mut req = Request::builder().method("POST").uri(format!("/api/tasks/{tid}/run")).header("content-type", "application/json");
        if let Some(k) = key {
            req = req.header("idempotency-key", k);
        }
        e.app.clone().oneshot(req.body(Body::from("{}")).unwrap())
    };
    let first = run(Some("run-1")).await.unwrap();
    assert_eq!(first.status(), StatusCode::OK);
    let first: Value = serde_json::from_slice(&to_bytes(first.into_body(), usize::MAX).await.unwrap()).unwrap();
    assert!(first["task"]["held_at"].is_null(), "{first}");
    let again = run(Some("run-1")).await.unwrap();
    assert_eq!(again.status(), StatusCode::OK);
    assert_eq!(again.headers().get("idempotent-replayed").map(|v| v.to_str().unwrap()), Some("true"));
    let again: Value = serde_json::from_slice(&to_bytes(again.into_body(), usize::MAX).await.unwrap()).unwrap();
    assert_eq!(again, first);
    // A new key (or none) on a task that isn't held is still a 409
    for key in [Some("run-2"), None] {
        let r = run(key).await.unwrap();
        assert_eq!(r.status(), StatusCode::CONFLICT, "{key:?}");
    }
    let released = e.state.db.task_events(&tid, 500).await.unwrap().iter().filter(|ev| ev.kind == "released").count();
    assert_eq!(released, 1);
}

#[tokio::test]
async fn insights_count_tasks_only_autofix_holds_as_needing_the_person() {
    let e = setup().await;
    autofix(&e, "on", 25).await;
    // A plain ready task waits on nobody
    let _ready = person_task(&e, json!([])).await;
    let insights = person(&e, "GET", "/api/missions/insights", None).await;
    assert_eq!(insights["needs_you_now"], 0, "{insights}");
    // A failed task held only by a fix_needed item sits in the board's Needs-you column, so it counts
    let auth = person_task(&e, json!([])).await;
    let started = Utc::now();
    claim(&e, &auth).await;
    runner::after_turn(&e.state, &auth, Some("Not logged in · Please run /login"), started).await;
    let t = task(&e, &auth).await;
    assert_eq!(t["state"], "failed", "{t}");
    assert_eq!(kinds(&t["needs_you"]), vec!["fix_needed".to_string()], "{t}");
    let insights = person(&e, "GET", "/api/missions/insights", None).await;
    assert_eq!(insights["needs_you_now"], 1, "{insights}");
    // Cancelled, it leaves the board and the count
    e.state.db.exec_raw_for_test(&format!("UPDATE tasks SET state = 'cancelled' WHERE id = '{auth}'")).await.unwrap();
    let insights = person(&e, "GET", "/api/missions/insights", None).await;
    assert_eq!(insights["needs_you_now"], 0, "{insights}");
}
