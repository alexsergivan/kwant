//! Threads, concurrent turns and delegation (batch 2 / A). A fake claude CLI on PATH records every
//! prompt and holds a "LONGTASK:<file>" turn until that file exists, so a test can keep a delegated
//! turn running while it talks to the same agent elsewhere. Its own test binary: it changes PATH.

use axum::{
    body::{to_bytes, Body},
    http::{Request, StatusCode},
    Router,
};
use kwnt::{
    broker::PermissionBroker,
    config::Config,
    db::Database,
    router::LayaRouter,
    server::{continuity, create_router, delegation, AppState},
    tools::ToolRunner,
    RuntimeEvent,
};
use serde_json::{json, Value};
use std::path::PathBuf;
use std::sync::OnceLock;
use std::time::Duration;
use tower::ServiceExt;

/// Replies "answer <tag>" for a prompt with "REPLYTAG:<tag>", followed by "@<name>" for one with
/// "MENTION:<name>"; a "LONGTASK:<file>" prompt waits (up to 60 s) for that file first. Every prompt is saved under `$KWNT_TEST_PROMPTS`.
const FAKE_CLAUDE: &str = r#"#!/bin/sh
case "$1" in
  auth) echo '{"loggedIn":true}'; exit 0 ;;
esac
for a; do last="$a"; done
printf '%s' "$last" > "$KWNT_TEST_PROMPTS/$$-$(date +%s%N)"
release=$(printf '%s\n' "$last" | sed -n 's/.*LONGTASK:\([^ ]*\).*/\1/p' | head -n 1)
if [ -n "$release" ]; then
  i=0
  while [ ! -f "$release" ] && [ $i -lt 600 ]; do sleep 0.1; i=$((i+1)); done
fi
tag=$(printf '%s\n' "$last" | sed -n 's/.*REPLYTAG:\([A-Za-z0-9_]*\).*/\1/p' | tail -n 1)
mention=$(printf '%s\n' "$last" | sed -n 's/.*MENTION:\([A-Za-z0-9_]*\).*/\1/p' | head -n 1)
echo "{\"type\":\"result\",\"is_error\":false,\"num_turns\":1,\"result\":\"answer ${tag:-none}${mention:+ @$mention}\"}"
"#;

fn prompts_dir() -> &'static PathBuf {
    static DIR: OnceLock<PathBuf> = OnceLock::new();
    DIR.get_or_init(|| {
        let root = tempfile::tempdir().unwrap().keep();
        let bin = root.join("bin");
        let prompts = root.join("prompts");
        std::fs::create_dir_all(&bin).unwrap();
        std::fs::create_dir_all(&prompts).unwrap();
        let fake = bin.join("claude");
        std::fs::write(&fake, FAKE_CLAUDE).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&fake, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        std::env::set_var("KWNT_TEST_PROMPTS", &prompts);
        std::env::set_var("PATH", format!("{}:{}", bin.display(), std::env::var("PATH").unwrap_or_default()));
        prompts
    })
}

/// Prompts the fake CLI got that contain `marker`.
fn prompts_with(marker: &str) -> Vec<String> {
    std::fs::read_dir(prompts_dir())
        .unwrap()
        .filter_map(|e| std::fs::read_to_string(e.unwrap().path()).ok())
        .filter(|p| p.contains(marker))
        .collect()
}

async fn call(app: &Router, method: &str, uri: &str, body: Option<Value>) -> (StatusCode, Value) {
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method(method)
                .uri(uri)
                .header("content-type", "application/json")
                .body(body.map(|v| Body::from(v.to_string())).unwrap_or_else(Body::empty))
                .unwrap(),
        )
        .await
        .unwrap();
    let s = resp.status();
    let bytes = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    (s, serde_json::from_slice(&bytes).unwrap_or(Value::Null))
}

struct Env {
    app: Router,
    state: AppState,
    lead: String,
    specialist: String,
    tmp: tempfile::TempDir,
}

impl Env {
    fn release_file(&self, name: &str) -> PathBuf {
        self.tmp.path().join(name)
    }
}

async fn setup() -> Env {
    prompts_dir();
    let tmp = tempfile::tempdir().unwrap();
    let data = tmp.path().join("data");
    let db = Database::init(&data).await.unwrap();
    // Laya is unreachable: turns use the kwant's preferred model, the fake claude
    let config = Config {
        data_dir: data.clone(),
        laya_url: "http://127.0.0.1:9".into(),
        browser_cdp_url: Some("http://127.0.0.1:9".into()),
        ..Config::default()
    };
    let state = AppState::new(
        config.clone(),
        db,
        ToolRunner::new(data.join("workspace")),
        PermissionBroker::new(),
        LayaRouter::new(config.laya_url.clone()),
    );
    let app = create_router(state.clone());
    let (_, kwants) = call(&app, "GET", "/api/kwants", None).await;
    let mut ids = Vec::new();
    for k in kwants.as_array().unwrap().iter().take(2) {
        let id = k["id"].as_str().unwrap().to_string();
        let mut body = k.clone();
        body.as_object_mut()
            .unwrap()
            .retain(|key, _| ["name", "title", "persona_prompt", "soul", "avatar_sprite"].contains(&key.as_str()));
        body["preferred_model"] = json!("claude");
        let (s, v) = call(&app, "PUT", &format!("/api/kwants/{id}"), Some(body)).await;
        assert_eq!(s, StatusCode::OK, "{v}");
        ids.push(id);
    }
    Env { app, state, lead: ids[0].clone(), specialist: ids[1].clone(), tmp }
}

async fn wait_for<F, Fut>(what: &str, mut check: F)
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = bool>,
{
    for _ in 0..300 {
        if check().await {
            return;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    panic!("timed out waiting for {what}");
}

async fn delegation_status(env: &Env, id: &str) -> String {
    let (_, d) = call(&env.app, "GET", &format!("/api/delegations/{id}"), None).await;
    d["status"].as_str().unwrap_or("").to_string()
}

async fn delegate(env: &Env, task: &str) -> Value {
    let (s, d) = call(
        &env.app,
        "POST",
        "/api/delegations",
        Some(json!({
            "lead_kwant_id": env.lead,
            "specialist_kwant_id": env.specialist,
            "task": task,
            "title": "Deep research",
            "kwant_id": env.lead,
            "continue_lead": false,
        })),
    )
    .await;
    assert_eq!(s, StatusCode::ACCEPTED, "{d}");
    d
}

fn meta_kind(m: &kwnt::db::models::Message) -> Option<String> {
    let meta: Value = serde_json::from_str(m.meta.as_deref()?).ok()?;
    meta["kind"].as_str().map(str::to_string)
}

/// The X-post scenario: while a long delegated task runs in the specialist's thread, the user
/// chats with the specialist directly and gets an answer; then the task reports back to the lead.
#[tokio::test]
async fn specialist_answers_direct_chat_while_a_delegated_turn_runs() {
    let env = setup().await;
    let release = env.release_file("research-done");
    let d = delegate(&env, &format!("Research the topic LONGTASK:{} REPLYTAG:research", release.display())).await;
    let id = d["id"].as_str().unwrap().to_string();
    let thread_id = d["thread_id"].as_str().unwrap().to_string();
    assert_eq!(d["status"], "queued");
    wait_for("the delegated turn to run", || async { delegation_status(&env, &id).await == "running" }).await;

    // The specialist's own chat is not blocked: a direct question gets its answer meanwhile
    let mut rx = env.state.event_sender.subscribe();
    let (s, v) = call(
        &env.app,
        "POST",
        "/api/chat/send",
        Some(json!({"kwant_id": env.specialist, "prompt": "How much disk space is left? REPLYTAG:disk DIRECTMARK", "sender_id": "user"})),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    let specialist = env.specialist.clone();
    tokio::time::timeout(Duration::from_secs(30), async {
        loop {
            if let Ok(RuntimeEvent::TurnEnded { kwant_id, context_id, outcome, .. }) = rx.recv().await {
                if kwant_id == specialist && context_id.as_deref() == Some(&specialist) {
                    assert_eq!(outcome, "completed");
                    break;
                }
            }
        }
    })
    .await
    .expect("the direct turn finished while the delegated one was still running");
    let direct = env.state.db.list_messages(&env.specialist, 20).await.unwrap();
    assert!(direct.iter().any(|m| m.sender_role == "kwant" && m.text == "answer disk"), "{direct:?}");
    assert_eq!(delegation_status(&env, &id).await, "running");
    // Two turns of one agent at once, each tagged with its own chat
    let (_, turns) = call(&env.app, "GET", "/api/turns", None).await;
    assert!(turns["turns"].as_array().unwrap().iter().any(|t| t["thread_id"] == thread_id.as_str() && t["state"] == "running"));

    // Engine sessions never mix: neither prompt holds the other chat's text
    assert!(prompts_with("DIRECTMARK").iter().all(|p| !p.contains("LONGTASK")));
    assert!(prompts_with("LONGTASK").iter().all(|p| !p.contains("DIRECTMARK")));

    // The task finishes and reports back to the lead's chat, linking the thread
    std::fs::write(&release, "").unwrap();
    // `delegation_updated` goes out once the result message is linked
    wait_for("the delegation to report", || async {
        let (_, d) = call(&env.app, "GET", &format!("/api/delegations/{id}"), None).await;
        d["status"] == "completed" && d["result_message_id"].is_string()
    })
    .await;
    let lead_chat = env.state.db.list_messages(&env.lead, 20).await.unwrap();
    let started = lead_chat.iter().find(|m| meta_kind(m).as_deref() == Some("delegation_started")).expect("started note");
    assert!(started.text.contains("Deep research"));
    let result = lead_chat.iter().find(|m| meta_kind(m).as_deref() == Some("delegation_result")).expect("result message");
    assert_eq!(result.text, "answer research");
    let meta: Value = serde_json::from_str(result.meta.as_deref().unwrap()).unwrap();
    assert_eq!(meta["thread_id"], thread_id.as_str());
    assert_eq!(meta["status"], "completed");
    let (_, d) = call(&env.app, "GET", &format!("/api/delegations/{id}"), None).await;
    assert_eq!(d["result_message_id"], result.id.as_str());

    // The thread keeps the whole exchange; the specialist's direct chat never shows it
    let (s, msgs) = call(&env.app, "GET", &format!("/api/threads/{thread_id}/messages"), None).await;
    assert_eq!(s, StatusCode::OK);
    let texts: Vec<&str> = msgs.as_array().unwrap().iter().filter_map(|m| m["text"].as_str()).collect();
    assert!(texts[0].contains("Research the topic"), "{texts:?}");
    assert!(texts.contains(&"answer research"));
    assert!(direct.iter().all(|m| !m.text.contains("answer research")));
}

/// A turn over the per-agent cap waits in a queue (visible over SSE and /api/turns), never fails.
#[tokio::test]
async fn excess_turns_queue_instead_of_failing() {
    let env = setup().await;
    let (s, v) = call(&env.app, "POST", "/api/settings", Some(json!({"max_turns_per_agent": 1}))).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    let (s, _) = call(&env.app, "POST", "/api/settings", Some(json!({"max_turns_per_agent": 0}))).await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
    let (_, settings) = call(&env.app, "GET", "/api/settings", None).await;
    assert_eq!(settings["max_turns_per_agent"], 1);
    assert_eq!(settings["delegation_follow_up_minutes"], 15);

    let release = env.release_file("queue-release");
    let d = delegate(&env, &format!("Long one LONGTASK:{} REPLYTAG:long", release.display())).await;
    let id = d["id"].as_str().unwrap().to_string();
    wait_for("the delegated turn to run", || async { delegation_status(&env, &id).await == "running" }).await;

    let mut rx = env.state.event_sender.subscribe();
    let (s, v) = call(
        &env.app,
        "POST",
        "/api/chat/send",
        Some(json!({"kwant_id": env.specialist, "prompt": "quick one REPLYTAG:quick", "sender_id": "user"})),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "never rejected: {v}");
    let queued = tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            if let Ok(e) = rx.recv().await {
                if let RuntimeEvent::TurnQueued { .. } = e.unscoped() {
                    return e.to_wire();
                }
            }
        }
    })
    .await
    .expect("a turn_queued event");
    assert_eq!(queued["type"], "turn_queued");
    assert_eq!(queued["data"]["kwant_id"], env.specialist.as_str());
    assert_eq!(queued["data"]["reason"], "agent_busy");
    assert_eq!(queued["data"]["position"], 1);
    let (_, turns) = call(&env.app, "GET", "/api/turns", None).await;
    assert!(turns["turns"].as_array().unwrap().iter().any(|t| t["state"] == "queued" && t["kwant_id"] == env.specialist.as_str()));

    // Once the long turn ends, the queued one runs on its own
    std::fs::write(&release, "").unwrap();
    let specialist = env.specialist.clone();
    wait_for("the queued turn to answer", || {
        let state = env.state.clone();
        let specialist = specialist.clone();
        async move { state.db.list_messages(&specialist, 20).await.unwrap().iter().any(|m| m.text == "answer quick") }
    })
    .await;
    wait_for("the delegation to complete", || async { delegation_status(&env, &id).await == "completed" }).await;
}

/// A delegation to a busy specialist reads "queued" until its turn gets the slot, not "running"
/// while it waits; it turns "running" (with `started_at`) when the turn starts.
#[tokio::test]
async fn a_delegation_waiting_for_a_busy_specialist_reads_queued() {
    let env = setup().await;
    let (s, v) = call(&env.app, "POST", "/api/settings", Some(json!({"max_turns_per_agent": 1}))).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    let release = env.release_file("busy-release");
    let (s, v) = call(
        &env.app,
        "POST",
        "/api/chat/send",
        Some(json!({"kwant_id": env.specialist, "prompt": format!("busy LONGTASK:{} REPLYTAG:busy", release.display()), "sender_id": "user"})),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    let specialist = env.specialist.clone();
    wait_for("the specialist's direct turn to run", || {
        let (app, specialist) = (env.app.clone(), specialist.clone());
        async move {
            let (_, turns) = call(&app, "GET", "/api/turns", None).await;
            turns["turns"].as_array().unwrap().iter().any(|t| t["state"] == "running" && t["kwant_id"] == specialist.as_str())
        }
    })
    .await;

    let mut rx = env.state.event_sender.subscribe();
    let d = delegate(&env, "Waits its turn REPLYTAG:waited").await;
    let id = d["id"].as_str().unwrap().to_string();
    assert_eq!(d["status"], "queued");
    let thread = d["thread_id"].as_str().unwrap().to_string();
    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            if let Ok(e) = rx.recv().await {
                let wire = e.to_wire();
                if wire["type"] == "turn_queued" && wire["data"]["thread_id"] == thread.as_str() {
                    return;
                }
            }
        }
    })
    .await
    .expect("the delegated turn queues behind the direct one");
    tokio::time::sleep(Duration::from_millis(300)).await;
    let (_, waiting) = call(&env.app, "GET", &format!("/api/delegations/{id}"), None).await;
    assert_eq!(waiting["status"], "queued", "still waiting: {waiting}");
    assert!(waiting["started_at"].is_null(), "{waiting}");
    // No delegation_updated said "running" while it waited
    while let Ok(e) = rx.try_recv() {
        let wire = e.to_wire();
        if wire["type"] == "delegation_updated" {
            assert_ne!(wire["data"]["delegation"]["status"], "running", "{wire}");
        }
    }

    std::fs::write(&release, "").unwrap();
    let saw_running = tokio::time::timeout(Duration::from_secs(20), async {
        loop {
            if let Ok(e) = rx.recv().await {
                let wire = e.to_wire();
                if wire["type"] == "delegation_updated" && wire["data"]["delegation"]["id"] == id.as_str() {
                    let status = wire["data"]["delegation"]["status"].as_str().unwrap_or("").to_string();
                    if status != "queued" {
                        return status;
                    }
                }
            }
        }
    })
    .await
    .expect("a delegation_updated once the turn starts");
    assert_eq!(saw_running, "running", "running is reported before the result");
    wait_for("the delegation to complete", || async { delegation_status(&env, &id).await == "completed" }).await;
    let (_, done) = call(&env.app, "GET", &format!("/api/delegations/{id}"), None).await;
    assert!(!done["started_at"].is_null(), "{done}");
}

/// Stopping a thread stops only that thread: the specialist's direct chat keeps running.
#[tokio::test]
async fn per_thread_cancel_leaves_other_chats_running() {
    let env = setup().await;
    let thread_release = env.release_file("never");
    let d = delegate(&env, &format!("Slow LONGTASK:{} REPLYTAG:slow", thread_release.display())).await;
    let id = d["id"].as_str().unwrap().to_string();
    let thread_id = d["thread_id"].as_str().unwrap().to_string();
    wait_for("the delegated turn to run", || async { delegation_status(&env, &id).await == "running" }).await;

    let direct_release = env.release_file("direct");
    let (s, _) = call(
        &env.app,
        "POST",
        "/api/chat/send",
        Some(json!({"kwant_id": env.specialist, "prompt": format!("also slow LONGTASK:{} REPLYTAG:direct", direct_release.display()), "sender_id": "user"})),
    )
    .await;
    assert_eq!(s, StatusCode::OK);
    wait_for("both turns to run", || async {
        env.state.turns.active_turns().iter().filter(|t| t.state == "running").count() == 2
    })
    .await;

    let (s, v) = call(&env.app, "POST", &format!("/api/threads/{thread_id}/cancel"), None).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(v["cancelled"], 1);
    assert_eq!(delegation_status(&env, &id).await, "cancelled");
    wait_for("the thread's turn to stop", || async { env.state.turns.active_turns().len() == 1 }).await;
    let left = env.state.turns.active_turns();
    assert_eq!(left[0].context_id, env.specialist, "the direct chat's turn is untouched");
    // Reported once in the lead's chat (by the cancel or the runner, whichever ends it first)
    wait_for("the stop note", || async {
        let lead_chat = env.state.db.list_messages(&env.lead, 20).await.unwrap();
        lead_chat.iter().filter(|m| m.text.contains("Stopped") && meta_kind(m).as_deref() == Some("delegation_result")).count() == 1
    })
    .await;

    std::fs::write(&direct_release, "").unwrap();
    wait_for("the direct turn to answer", || {
        let state = env.state.clone();
        let specialist = env.specialist.clone();
        async move { state.db.list_messages(&specialist, 20).await.unwrap().iter().any(|m| m.text == "answer direct") }
    })
    .await;
    // Cancelling again is a no-op; unknown ids are 404
    let (s, d) = call(&env.app, "POST", &format!("/api/delegations/{id}/cancel"), None).await;
    assert_eq!((s, d["status"].as_str()), (StatusCode::OK, Some("cancelled")));
    let (s, _) = call(&env.app, "POST", "/api/delegations/nope/cancel", None).await;
    assert_eq!(s, StatusCode::NOT_FOUND);
    let (s, _) = call(&env.app, "POST", "/api/threads/nope/cancel", None).await;
    assert_eq!(s, StatusCode::NOT_FOUND);
}

/// A user-made thread is its own conversation: own history, own engine prompt, can be archived.
#[tokio::test]
async fn threads_keep_their_own_history() {
    let env = setup().await;
    let (s, _) = call(
        &env.app,
        "POST",
        "/api/chat/send",
        Some(json!({"kwant_id": env.specialist, "prompt": "secret plan SIDECHAT REPLYTAG:side", "sender_id": "user"})),
    )
    .await;
    assert_eq!(s, StatusCode::OK);
    wait_for("the direct reply", || async {
        env.state.db.list_messages(&env.specialist, 20).await.unwrap().iter().any(|m| m.text == "answer side")
    })
    .await;

    let (s, t) = call(&env.app, "POST", &format!("/api/kwants/{}/threads", env.specialist), Some(json!({"title": "  Wifi\nhelp  "}))).await;
    assert_eq!(s, StatusCode::CREATED, "{t}");
    assert_eq!(t["title"], "Wifi help");
    let tid = t["id"].as_str().unwrap().to_string();
    let (s, v) = call(
        &env.app,
        "POST",
        "/api/chat/send",
        Some(json!({"thread_id": tid, "kwant_id": env.lead, "prompt": "wifi question THREADMARK REPLYTAG:wifi", "sender_id": "user"})),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(v["kwant"].as_str(), env.state.db.get_kwant(&env.specialist).await.unwrap().map(|k| k.name).as_deref(), "the thread's owner answers");
    wait_for("the thread reply", || async {
        env.state.db.list_messages(&tid, 20).await.unwrap().iter().any(|m| m.text == "answer wifi")
    })
    .await;
    assert!(prompts_with("THREADMARK").iter().all(|p| !p.contains("SIDECHAT")), "thread prompt saw the direct chat");
    let direct = env.state.db.list_messages(&env.specialist, 20).await.unwrap();
    assert!(direct.iter().all(|m| !m.text.contains("THREADMARK") && m.text != "answer wifi"));
    let lead_direct = env.state.db.list_messages(&env.lead, 20).await.unwrap();
    assert!(lead_direct.iter().all(|m| !m.text.contains("THREADMARK")));

    let (_, list) = call(&env.app, "GET", &format!("/api/kwants/{}/threads", env.specialist), None).await;
    assert_eq!(list.as_array().unwrap().len(), 1);
    let (s, _) = call(&env.app, "PATCH", &format!("/api/threads/{tid}"), Some(json!({"archived": true}))).await;
    assert_eq!(s, StatusCode::OK);
    let (_, list) = call(&env.app, "GET", &format!("/api/kwants/{}/threads", env.specialist), None).await;
    assert!(list.as_array().unwrap().is_empty());
    let (s, _) = call(&env.app, "POST", "/api/chat/send", Some(json!({"thread_id": tid, "prompt": "hi", "sender_id": "user"}))).await;
    assert_eq!(s, StatusCode::CONFLICT, "archived threads take no new messages");
    let (s, _) = call(&env.app, "POST", "/api/chat/send", Some(json!({"thread_id": "missing", "prompt": "hi", "sender_id": "user"}))).await;
    assert_eq!(s, StatusCode::NOT_FOUND);
    let (s, _) = call(&env.app, "GET", "/api/kwants/missing/threads", None).await;
    assert_eq!(s, StatusCode::NOT_FOUND);
}

/// The agent tools act only for the calling lead, within the rules (no self, depth, own records).
#[tokio::test]
async fn delegation_tools_enforce_their_rules() {
    let env = setup().await;
    let call_tool = |token: &'static str, name: &'static str, args: Value| {
        let app = env.app.clone();
        async move {
            let resp = app
                .oneshot(
                    Request::builder()
                        .method("POST")
                        .uri("/api/mcp/self")
                        .header("content-type", "application/json")
                        .header("authorization", format!("Bearer {token}"))
                        .body(Body::from(
                            json!({"jsonrpc": "2.0", "id": 1, "method": "tools/call", "params": {"name": name, "arguments": args}}).to_string(),
                        ))
                        .unwrap(),
                )
                .await
                .unwrap();
            let v: Value = serde_json::from_slice(&to_bytes(resp.into_body(), usize::MAX).await.unwrap()).unwrap();
            (v["result"]["content"][0]["text"].as_str().unwrap_or("").to_string(), v["result"]["isError"] == true)
        }
    };
    let names: Vec<String> = env.state.db.list_kwants(&kwnt::db::spaces::SpaceId::default_space()).await.unwrap().into_iter().map(|k| k.name).collect();
    {
        let mut tools = env.state.self_tools.lock().await;
        tools.insert(
            "lead-token".into(),
            continuity::SelfScope {
                kwant_id: env.lead.clone(),
                kwant_name: names[0].clone(),
                origin: Some(continuity::TurnOrigin { direct_kwant_id: Some(env.lead.clone()), ..Default::default() }),
                ..Default::default()
            },
        );
        tools.insert(
            "deep-token".into(),
            continuity::SelfScope {
                kwant_id: env.lead.clone(),
                kwant_name: names[0].clone(),
                origin: Some(continuity::TurnOrigin { direct_kwant_id: Some(env.lead.clone()), depth: 2, ..Default::default() }),
                ..Default::default()
            },
        );
        tools.insert(
            "other-token".into(),
            continuity::SelfScope { kwant_id: env.specialist.clone(), kwant_name: names[1].clone(), origin: None, ..Default::default() },
        );
    }
    // Offered only to turns that know their chat
    let list = |token: &'static str| {
        let app = env.app.clone();
        async move {
            let resp = app
                .oneshot(
                    Request::builder()
                        .method("POST")
                        .uri("/api/mcp/self")
                        .header("content-type", "application/json")
                        .header("authorization", format!("Bearer {token}"))
                        .body(Body::from(json!({"jsonrpc": "2.0", "id": 1, "method": "tools/list"}).to_string()))
                        .unwrap(),
                )
                .await
                .unwrap();
            let v: Value = serde_json::from_slice(&to_bytes(resp.into_body(), usize::MAX).await.unwrap()).unwrap();
            v["result"]["tools"].as_array().unwrap().iter().map(|t| t["name"].as_str().unwrap().to_string()).collect::<Vec<_>>()
        }
    };
    assert!(list("lead-token").await.contains(&"delegate".to_string()));
    assert!(!list("other-token").await.contains(&"delegate".to_string()));

    let (text, err) = call_tool("lead-token", "delegate", json!({"to": names[0], "task": "x"})).await;
    assert!(err && text.contains("itself"), "{text}");
    let (text, err) = call_tool("lead-token", "delegate", json!({"to": "Nobody", "task": "x"})).await;
    assert!(err && text.contains(&names[1]), "lists who can take it: {text}");
    let (text, err) = call_tool("deep-token", "delegate", json!({"to": names[1], "task": "x"})).await;
    assert!(err && text.contains("delegated further"), "{text}");
    let (text, err) = call_tool("lead-token", "delegate", json!({"to": "", "task": "x".repeat(8001)})).await;
    assert!(err, "{text}");

    let (text, err) = call_tool("lead-token", "delegate", json!({"to": format!("@{}", names[1]), "task": "Check it REPLYTAG:tool", "title": "Tool task"})).await;
    assert!(!err, "{text}");
    let id = text.split('[').nth(1).and_then(|s| s.split(']').next()).unwrap().to_string();
    let (text, err) = call_tool("lead-token", "delegation_status", json!({})).await;
    assert!(!err && text.contains(&id), "{text}");
    // Another agent can't see or stop it
    let (_, err) = call_tool("other-token", "delegation_status", json!({"id": id})).await;
    assert!(err);
    let (_, err) = call_tool("other-token", "delegation_cancel", json!({"id": id})).await;
    assert!(err);
    // Finished delegations from the tool continue the lead's turn in its chat
    wait_for("the tool delegation to complete", || async { delegation_status(&env, &id).await == "completed" }).await;
    wait_for("the lead's continuation turn", || async { !prompts_with("finished the task you delegated (“Tool task”)").is_empty() }).await;
    let (text, _) = call_tool("lead-token", "delegation_cancel", json!({"id": id})).await;
    assert!(text.contains("already finished"), "{text}");
}

/// The stall hook posts one note per delegation; a restart marks open ones interrupted.
#[tokio::test]
async fn follow_ups_and_restarts_report_in_the_origin_chat() {
    let env = setup().await;
    let release = env.release_file("stall");
    let d = delegate(&env, &format!("Stalls LONGTASK:{} REPLYTAG:late", release.display())).await;
    let id = d["id"].as_str().unwrap().to_string();
    assert!(d["follow_up_at"].is_string());
    wait_for("the delegated turn to run", || async { delegation_status(&env, &id).await == "running" }).await;
    assert_eq!(delegation::follow_ups(&env.state, chrono::Utc::now()).await, 0, "not due yet");
    let later = chrono::Utc::now() + chrono::Duration::minutes(16);
    assert_eq!(delegation::follow_ups(&env.state, later).await, 1);
    assert_eq!(delegation::follow_ups(&env.state, later).await, 0, "only once");
    let lead_chat = env.state.db.list_messages(&env.lead, 20).await.unwrap();
    assert!(lead_chat.iter().any(|m| meta_kind(m).as_deref() == Some("delegation_stalled") && m.text.contains("still working")));

    // Turned off: no follow-up time is set
    let (s, _) = call(&env.app, "POST", "/api/settings", Some(json!({"delegation_follow_up_minutes": 0}))).await;
    assert_eq!(s, StatusCode::OK);
    let d2 = delegate(&env, "Another REPLYTAG:two").await;
    assert!(d2["follow_up_at"].is_null());

    std::fs::write(&release, "").unwrap();
    wait_for("both to complete", || async {
        delegation_status(&env, &id).await == "completed"
            && delegation_status(&env, d2["id"].as_str().unwrap()).await == "completed"
    })
    .await;

    // A delegation left open by a stopped kwnt becomes interrupted, with a note
    let (_, open) = env
        .state
        .db
        .create_delegation(&kwnt::db::threads::NewDelegation {
            lead_kwant_id: env.lead.clone(),
            specialist_kwant_id: env.specialist.clone(),
            origin_channel_id: None,
            origin_kwant_id: Some(env.lead.clone()),
            origin_thread_id: None,
            team_id: None,
            task: "t".into(),
            title: "Cut short".into(),
            continue_lead: true,
            depth: 1,
            follow_up_at: None,
        })
        .await
        .unwrap();
    delegation::interrupt_open(&env.state).await;
    assert_eq!(delegation_status(&env, &open.id).await, "interrupted");
    let lead_chat = env.state.db.list_messages(&env.lead, 30).await.unwrap();
    assert!(lead_chat.iter().any(|m| m.text.contains("Cut short") && m.text.contains("restarted")));
}

/// The new columns and tables are additive: rows written the old way still load, and init twice
/// is a no-op.
#[tokio::test]
async fn migration_is_additive_and_idempotent() {
    let tmp = tempfile::tempdir().unwrap();
    let db = Database::init(tmp.path()).await.unwrap();
    drop(db);
    let url = format!("sqlite://{}?mode=rwc", tmp.path().join("kwnt.db").display());
    {
        let pool = sqlx::SqlitePool::connect(&url).await.unwrap();
        // What the previous binary writes: no thread_id, no meta
        sqlx::query("INSERT INTO messages (id, direct_kwant_id, sender_role, sender_id, text) VALUES ('old1', 'k', 'user', 'you', 'from the old binary')")
            .execute(&pool)
            .await
            .unwrap();
        pool.close().await;
    }
    let db = Database::init(tmp.path()).await.unwrap();
    let db = {
        drop(db);
        Database::init(tmp.path()).await.unwrap()
    };
    {
        let msgs = db.list_messages("k", 10).await.unwrap();
        assert_eq!(msgs.len(), 1);
        assert!(msgs[0].thread_id.is_none() && msgs[0].meta.is_none());
    }
    // Tagged SSE events flatten for clients
    let wire = RuntimeEvent::Scoped {
        tag: kwnt::drivers::events::TurnTag {
            turn_id: "t1".into(),
            kwant_id: "k".into(),
            context_id: "th".into(),
            thread_id: Some("th".into()),
        },
        event: Box::new(RuntimeEvent::TextDelta("hi".into())),
    }
    .to_wire();
    assert_eq!(wire["type"], "TextDelta");
    assert_eq!(wire["data"], "hi");
    assert_eq!((wire["turn_id"].as_str(), wire["context_id"].as_str(), wire["thread_id"].as_str()), (Some("t1"), Some("th"), Some("th")));
}

/// A specialist can't hand its task back to the lead that gave it; its reply already goes back.
#[tokio::test]
async fn a_task_cannot_be_handed_back_to_its_sender() {
    let env = setup().await;
    let release = env.release_file("handback-release");
    let d = delegate(&env, &format!("Hold on LONGTASK:{} REPLYTAG:handback", release.display())).await;
    let thread = d["thread_id"].as_str().unwrap();
    let (s, v) = call(
        &env.app,
        "POST",
        "/api/delegations",
        Some(json!({
            "lead_kwant_id": env.specialist,
            "specialist_kwant_id": env.lead,
            "task": "You do it",
            "thread_id": thread,
        })),
    )
    .await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "{v}");
    assert!(v["error"].as_str().unwrap().contains("don't hand it back"), "{v}");
    std::fs::write(&release, "").unwrap();
    wait_for("the held task to finish", || async { delegation_status(&env, d["id"].as_str().unwrap()).await == "completed" }).await;
}

/// A lead that hands work to two agents at once gets one turn after both end, with both results.
#[tokio::test]
async fn parallel_delegations_resume_the_lead_once_with_every_result() {
    let env = setup().await;
    let (_, kwants) = call(&env.app, "GET", "/api/kwants", None).await;
    let third = kwants.as_array().unwrap().iter().map(|k| k["id"].as_str().unwrap().to_string()).find(|id| *id != env.lead && *id != env.specialist);
    let Some(third) = third else {
        // The default space has only two bots; add one
        let (s, v) = call(&env.app, "POST", "/api/kwants", Some(json!({"name": "Third", "title": "QA", "preferred_model": "claude"}))).await;
        assert!(s.is_success(), "{v}");
        return run_fan_in(&env, v["id"].as_str().unwrap().to_string()).await;
    };
    let mut body = kwants.as_array().unwrap().iter().find(|k| k["id"] == third.as_str()).unwrap().clone();
    body.as_object_mut().unwrap().retain(|key, _| ["name", "title", "persona_prompt", "soul", "avatar_sprite"].contains(&key.as_str()));
    body["preferred_model"] = json!("claude");
    let (s, v) = call(&env.app, "PUT", &format!("/api/kwants/{third}"), Some(body)).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    run_fan_in(&env, third).await;
}

async fn run_fan_in(env: &Env, third: String) {
    let (rel_a, rel_b) = (env.release_file("fan-a"), env.release_file("fan-b"));
    let mut ids = Vec::new();
    for (to, rel, tag, title) in [(&env.specialist, &rel_a, "fanA", "Fan part A"), (&third, &rel_b, "fanB", "Fan part B")] {
        let (s, d) = call(
            &env.app,
            "POST",
            "/api/delegations",
            Some(json!({
                "lead_kwant_id": env.lead,
                "specialist_kwant_id": to,
                "task": format!("Part LONGTASK:{} REPLYTAG:{tag}", rel.display()),
                "title": title,
                "kwant_id": env.lead,
            })),
        )
        .await;
        assert_eq!(s, StatusCode::ACCEPTED, "{d}");
        ids.push(d["id"].as_str().unwrap().to_string());
    }
    std::fs::write(&rel_a, "").unwrap();
    wait_for("part A to complete", || async { delegation_status(env, &ids[0]).await == "completed" }).await;
    // Part B is still open, so the lead isn't resumed yet
    tokio::time::sleep(Duration::from_millis(500)).await;
    assert!(prompts_with("(“Fan part A”)").is_empty(), "no turn for A alone");
    std::fs::write(&rel_b, "").unwrap();
    wait_for("part B to complete", || async { delegation_status(env, &ids[1]).await == "completed" }).await;
    wait_for("the lead's one continuation turn", || async { !prompts_with("All 2 tasks you delegated have ended").is_empty() }).await;
    tokio::time::sleep(Duration::from_millis(500)).await;
    let resumed = prompts_with("All 2 tasks you delegated have ended");
    assert_eq!(resumed.len(), 1, "exactly one resume");
    let p = &resumed[0];
    assert!(p.contains("answer fanA") && p.contains("answer fanB"), "both results: {p}");
    assert!(p.contains("Check each result"), "{p}");
    assert!(prompts_with("finished the task you delegated (“Fan part").is_empty(), "no per-part turns");
}

async fn name(env: &Env, id: &str) -> String {
    let (_, kwants) = call(&env.app, "GET", "/api/kwants", None).await;
    kwants.as_array().unwrap().iter().find(|k| k["id"] == id).unwrap()["name"].as_str().unwrap().to_string()
}

/// A team led by `env.lead` with `env.specialist` in it; returns its channel's id.
async fn team_channel(env: &Env, team: &str) -> String {
    let (s, v) = call(
        &env.app,
        "POST",
        "/api/teams",
        Some(json!({"name": team, "lead_kwant_id": env.lead, "member_kwants": [env.lead, env.specialist]})),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    let (_, channels) = call(&env.app, "GET", "/api/channels", None).await;
    channels
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["name"] == format!("team-{team}").as_str())
        .unwrap_or_else(|| panic!("no team-{team} channel: {channels}"))["id"]
        .as_str()
        .unwrap()
        .to_string()
}

async fn say_in(env: &Env, channel: &str, prompt: &str) {
    let (s, v) = call(&env.app, "POST", "/api/chat/send", Some(json!({"channel_id": channel, "prompt": prompt, "sender_id": "user"}))).await;
    assert_eq!(s, StatusCode::OK, "{v}");
}

async fn channel_messages(env: &Env, channel: &str) -> Vec<kwnt::db::models::Message> {
    env.state.db.list_messages(channel, 100).await.unwrap()
}

/// Gap 1: a team lead that hands work on with an @mention in its team channel delegates it. The
/// teammate works in its own thread (the delegation_started chip in the channel), its result is
/// posted back, and the lead is resumed once with it to give the user one final answer; the
/// teammate never takes a handoff turn in the channel itself.
#[tokio::test]
async fn a_leads_mention_in_its_team_channel_is_a_delegation() {
    let env = setup().await;
    let (lead, specialist) = (name(&env, &env.lead).await, name(&env, &env.specialist).await);
    let channel = team_channel(&env, "mention-squad").await;
    say_in(&env, &channel, &format!("@{lead} Ship it LEADMARK1\nREPLYTAG:shipped MENTION:{specialist}")).await;

    wait_for("the delegation from the mention", || async {
        env.state.db.list_delegations(Some(&env.lead), None, 10).await.unwrap().iter().any(|d| d.origin_channel_id.as_deref() == Some(channel.as_str()))
    })
    .await;
    let d = env.state.db.list_delegations(Some(&env.lead), None, 10).await.unwrap().into_iter().find(|d| d.origin_channel_id.as_deref() == Some(channel.as_str())).unwrap();
    assert_eq!(d.specialist_kwant_id, env.specialist);
    assert!(d.continue_lead);
    assert!(d.task.contains("LEADMARK1") && d.task.contains(&format!("@{specialist}")), "the task carries the request and the lead's words: {}", d.task);

    wait_for("the delegation to complete", || async { delegation_status(&env, &d.id).await == "completed" }).await;
    let lead_replies = || async {
        channel_messages(&env, &channel).await.into_iter().filter(|m| m.sender_role == "kwant" && m.sender_id == lead).count()
    };
    wait_for("the lead's final answer", || async { lead_replies().await == 2 }).await;
    // Settle: nothing else arrives
    tokio::time::sleep(Duration::from_millis(1500)).await;
    let msgs = channel_messages(&env, &channel).await;
    let kinds: Vec<Option<String>> = msgs.iter().map(meta_kind).collect();
    assert_eq!(kinds.iter().filter(|k| k.as_deref() == Some("delegation_started")).count(), 1, "one Sent-to chip");
    assert_eq!(kinds.iter().filter(|k| k.as_deref() == Some("delegation_result")).count(), 1, "one result");
    assert_eq!(lead_replies().await, 2, "the lead's handoff reply and one final answer");
    assert!(
        msgs.iter().filter(|m| m.sender_id == specialist).all(|m| meta_kind(m).as_deref() == Some("delegation_result")),
        "the teammate speaks in the channel only through its result"
    );
    let resumes = prompts_with("LEADMARK1").into_iter().filter(|p| p.contains("finished the task you delegated")).count();
    assert_eq!(resumes, 1, "the lead is resumed exactly once");
    assert!(prompts_with("LEADMARK1").iter().all(|p| !p.contains("handed this conversation to you")), "no direct handoff turn");
}

/// A teammate who isn't the lead still hands the turn on directly, as before; and a lead's
/// mention that the delegation guards refuse is said in the channel instead of running.
#[tokio::test]
async fn non_lead_mentions_hand_off_and_refused_mentions_are_reported() {
    let env = setup().await;
    let (lead, specialist) = (name(&env, &env.lead).await, name(&env, &env.specialist).await);
    let channel = team_channel(&env, "plain-squad").await;
    say_in(&env, &channel, &format!("@{specialist} Look at this PLAINMARK REPLYTAG:plain MENTION:{lead}")).await;
    wait_for("the lead's handoff turn", || async {
        prompts_with("PLAINMARK").iter().any(|p| p.contains("handed this conversation to you"))
            && channel_messages(&env, &channel).await.iter().any(|m| m.sender_id == lead)
    })
    .await;
    assert!(
        env.state.db.list_delegations(Some(&env.specialist), None, 10).await.unwrap().is_empty(),
        "a teammate's mention isn't a delegation"
    );

    // The lead already has the most unfinished delegations it may have: the mention is refused
    let release = env.release_file("refused-release");
    let mut held = Vec::new();
    for _ in 0..delegation::MAX_OPEN_PER_LEAD {
        held.push(delegate(&env, &format!("Hold LONGTASK:{} REPLYTAG:held", release.display())).await["id"].as_str().unwrap().to_string());
    }
    // Its own channel: the history above would carry the earlier MENTION into the lead's prompt
    let channel = team_channel(&env, "refused-squad").await;
    say_in(&env, &channel, &format!("@{lead} Again REFUSEDMARK REPLYTAG:again MENTION:{specialist}")).await;
    wait_for("the refusal note", || async {
        channel_messages(&env, &channel).await.iter().any(|m| meta_kind(m).as_deref() == Some("handoff_refused"))
    })
    .await;
    let note = channel_messages(&env, &channel).await.into_iter().find(|m| meta_kind(m).as_deref() == Some("handoff_refused")).unwrap();
    assert!(note.text.contains(&specialist) && note.text.contains("unfinished delegations"), "{}", note.text);
    assert!(prompts_with("REFUSEDMARK").iter().all(|p| !p.contains("handed this conversation to you")), "refused, not handed off directly");
    std::fs::write(&release, "").unwrap();
    for id in held {
        wait_for("a held task to finish", || async { delegation_status(&env, &id).await == "completed" }).await;
    }
}
