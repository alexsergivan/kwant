//! 26c: Ollama that comes up after kwant (a restart) is found without anyone pressing Re-check.
//! A turn that would say "no account" looks at Ollama once more first, a bot pinned to Ollama
//! that still can't reach it says so (not "add an account"), a failed check looks again on its own,
//! and a ready Ollama counts in the space's accounts. Its own test binary: it changes PATH, HOME
//! and the process env.

use axum::{
    body::{to_bytes, Body},
    extract::State,
    http::{Request, StatusCode},
    routing::{get, post},
    Json, Router,
};
use kwnt::{
    broker::PermissionBroker,
    config::Config,
    db::Database,
    drivers::{
        detect_available_models,
        engines::{EngineId, EngineRegistry, EngineStatus, Probe, ProbeEnv},
        RuntimeEvent,
    },
    router::LayaRouter,
    server::{create_router, AppState},
    tools::ToolRunner,
};
use serde_json::{json, Value};
use std::collections::HashSet;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tower::ServiceExt;

const MODEL: &str = "qwen-home:latest";

/// A fake Ollama: counts `/api/tags` looks and chat calls.
#[derive(Clone, Default)]
struct Ollama {
    tags: Arc<AtomicUsize>,
    chats: Arc<AtomicUsize>,
}

async fn tags(State(o): State<Ollama>) -> Json<Value> {
    o.tags.fetch_add(1, Ordering::SeqCst);
    // Slow enough that two looks at once overlap
    tokio::time::sleep(Duration::from_millis(150)).await;
    Json(json!({ "models": [{ "name": MODEL }] }))
}

async fn chat(State(o): State<Ollama>, Json(_): Json<Value>) -> Json<Value> {
    o.chats.fetch_add(1, Ordering::SeqCst);
    Json(
        json!({ "choices": [{ "message": { "role": "assistant", "content": "lights are off" } }], "usage": { "total_tokens": 7 } }),
    )
}

/// A free port with nothing listening on it yet: Ollama "not started".
async fn closed_port() -> u16 {
    let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    l.local_addr().unwrap().port()
}

/// Starts the fake Ollama on `port` (Ollama "came up").
async fn start_ollama(port: u16) -> Ollama {
    let o = Ollama::default();
    let app = Router::new()
        .route("/api/tags", get(tags))
        .route(
            "/api/version",
            get(|| async { Json(json!({ "version": "0.9.1" })) }),
        )
        .route("/v1/chat/completions", post(chat))
        .with_state(o.clone());
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", port))
        .await
        .unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    o
}

fn env_at(port: u16) -> ProbeEnv {
    ProbeEnv {
        ollama_root: format!("http://127.0.0.1:{port}"),
        ollama_installed: true,
        ollama_configured: true,
        ..ProbeEnv::default()
    }
}

/// What a failed startup check leaves behind.
fn failed() -> Probe {
    Probe {
        status: EngineStatus::Error,
        detail: Some("Ollama isn't running".into()),
        version: None,
        models: Vec::new(),
    }
}

fn offers_ollama(models: &[kwnt::drivers::ModelDescriptor]) -> bool {
    models.iter().any(|m| m.id == "local_ollama")
}

#[tokio::test]
async fn after_a_failed_check_a_second_look_offers_ollama_again_and_shares_one_check() {
    let tmp = tempfile::tempdir().unwrap();
    let db = Database::init(&tmp.path().join("data")).await.unwrap();
    let none = HashSet::new();
    let reg = EngineRegistry::default();
    let port = closed_port().await;
    let env = env_at(port);

    // Down: the look fails and Ollama stays out
    assert_eq!(
        reg.recheck_joined(EngineId::Ollama, &env, Duration::from_secs(3))
            .await,
        EngineStatus::Error
    );
    assert!(!offers_ollama(
        &detect_available_models(&db, &reg, &none, &none).await
    ));

    // Up: one look and it is offered again
    let ollama = start_ollama(port).await;
    assert_eq!(
        reg.recheck_joined(EngineId::Ollama, &env, Duration::from_secs(3))
            .await,
        EngineStatus::Ready
    );
    let models = detect_available_models(&db, &reg, &none, &none).await;
    assert!(offers_ollama(&models), "{models:?}");
    assert_eq!(ollama.tags.load(Ordering::SeqCst), 1);

    // Three turns at once after a failure share one check
    reg.store(vec![(EngineId::Ollama, failed())]);
    let limit = Duration::from_secs(3);
    let (a, b, c) = tokio::join!(
        reg.recheck_joined(EngineId::Ollama, &env, limit),
        reg.recheck_joined(EngineId::Ollama, &env, limit),
        reg.recheck_joined(EngineId::Ollama, &env, limit),
    );
    assert_eq!(
        (a, b, c),
        (
            EngineStatus::Ready,
            EngineStatus::Ready,
            EngineStatus::Ready
        )
    );
    assert_eq!(
        ollama.tags.load(Ordering::SeqCst),
        2,
        "one more look, not three"
    );

    // Off in Settings: never looked at
    reg.set_enabled(EngineId::Ollama, false);
    assert_eq!(
        reg.recheck_joined(EngineId::Ollama, &env, limit).await,
        EngineStatus::Disabled
    );
    assert_eq!(ollama.tags.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn a_second_look_never_holds_a_turn_past_its_limit() {
    // Accepts connections and never answers: a hung Ollama
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    tokio::spawn(async move {
        let mut held = Vec::new();
        while let Ok((s, _)) = listener.accept().await {
            held.push(s);
        }
    });
    let reg = EngineRegistry::default();
    reg.store(vec![(EngineId::Ollama, failed())]);
    let started = Instant::now();
    let status = reg
        .recheck_joined(EngineId::Ollama, &env_at(port), Duration::from_millis(400))
        .await;
    assert!(
        started.elapsed() < Duration::from_secs(2),
        "{:?}",
        started.elapsed()
    );
    assert_eq!(
        status,
        EngineStatus::Error,
        "past the limit the old status stands"
    );
}

#[tokio::test]
async fn the_watcher_finds_ollama_once_it_comes_up_with_no_call_from_the_app() {
    let port = closed_port().await;
    let reg = Arc::new(EngineRegistry::default());
    reg.store(vec![(EngineId::Ollama, failed())]);
    let delays = [Duration::from_millis(40)];
    let watcher = {
        let reg = reg.clone();
        tokio::spawn(async move {
            reg.watch_ollama(&delays, || async move { env_at(port) })
                .await
        })
    };
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert_eq!(reg.status(EngineId::Ollama), EngineStatus::Error);

    let ollama = start_ollama(port).await;
    tokio::time::timeout(Duration::from_secs(10), async {
        while reg.status(EngineId::Ollama) != EngineStatus::Ready {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("the watcher never found Ollama");
    // Ready: no more looks
    let looks = ollama.tags.load(Ordering::SeqCst);
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert_eq!(ollama.tags.load(Ordering::SeqCst), looks);
    watcher.abort();

    // Never set up (no binary, no address) and not running: the watcher leaves it alone
    let reg = Arc::new(EngineRegistry::default());
    reg.store(vec![(
        EngineId::Ollama,
        Probe {
            status: EngineStatus::NotInstalled,
            detail: None,
            version: None,
            models: Vec::new(),
        },
    )]);
    let watcher = {
        let reg = reg.clone();
        let env = ProbeEnv {
            ollama_configured: false,
            ollama_installed: false,
            ..env_at(port)
        };
        tokio::spawn(async move {
            reg.watch_ollama(&delays, || {
                let env = env.clone();
                async move { env }
            })
            .await
        })
    };
    tokio::time::sleep(Duration::from_millis(300)).await;
    watcher.abort();
    assert_eq!(reg.status(EngineId::Ollama), EngineStatus::NotInstalled);
    assert_eq!(ollama.tags.load(Ordering::SeqCst), looks);
}

async fn call(app: &Router, method: &str, uri: &str, body: Option<Value>) -> (StatusCode, Value) {
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method(method)
                .uri(uri)
                .header("content-type", "application/json")
                .body(
                    body.map(|v| Body::from(v.to_string()))
                        .unwrap_or_else(Body::empty),
                )
                .unwrap(),
        )
        .await
        .unwrap();
    let s = resp.status();
    let bytes = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    (s, serde_json::from_slice(&bytes).unwrap_or(Value::Null))
}

/// Sends a prompt; the text the turn streamed and how long it took.
async fn turn(app: &Router, state: &AppState, kwant_id: &str) -> (String, Duration) {
    let mut rx = state.event_sender.subscribe();
    let started = Instant::now();
    let (s, v) = call(
        app,
        "POST",
        "/api/chat/send",
        Some(json!({"kwant_id": kwant_id, "prompt": "lights off", "sender_id": "user"})),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    let mut text = String::new();
    tokio::time::timeout(Duration::from_secs(60), async {
        while let Ok(e) = rx.recv().await {
            match e.unscoped() {
                RuntimeEvent::TextDelta(t) => text.push_str(t),
                RuntimeEvent::TurnEnded { kwant_id: k, .. } if k == kwant_id => break,
                _ => {}
            }
        }
    })
    .await
    .expect("turn did not end");
    (text, started.elapsed())
}

/// The newest saved reply of the bot: its text and parsed meta.
async fn last_reply(state: &AppState, kwant_id: &str) -> (String, Value) {
    let msgs = state.db.list_messages(kwant_id, 50).await.unwrap();
    let m = msgs
        .iter()
        .rev()
        .find(|m| m.sender_role == "kwant")
        .expect("a saved reply");
    (
        m.text.clone(),
        m.meta
            .as_deref()
            .and_then(|s| serde_json::from_str(s).ok())
            .unwrap_or(Value::Null),
    )
}

async fn ready_accounts(app: &Router) -> u64 {
    let (s, v) = call(app, "GET", "/api/spaces", None).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    let spaces = v
        .as_array()
        .or_else(|| v["spaces"].as_array())
        .expect("a list of spaces")
        .clone();
    let default = spaces
        .iter()
        .find(|s| s["is_default"] == true)
        .unwrap_or(&spaces[0]);
    default["counts"]["ready_accounts"]
        .as_u64()
        .expect("counts.ready_accounts")
}

#[tokio::test]
async fn a_bot_pinned_to_ollama_runs_once_ollama_is_back_and_the_space_counts_it() {
    let tmp = tempfile::tempdir().unwrap();
    let bin = tmp.path().join("bin");
    std::fs::create_dir_all(&bin).unwrap();
    // No CLI and no key: Ollama is the only engine, as in the tester's Home space
    std::env::set_var("PATH", format!("{}:/usr/bin:/bin", bin.display()));
    std::env::set_var("HOME", tmp.path());
    for var in [
        "ANTHROPIC_API_KEY",
        "OPENAI_API_KEY",
        "XAI_API_KEY",
        "OLLAMA_MODEL",
        "CLAUDE_CONFIG_DIR",
    ] {
        std::env::remove_var(var);
    }
    let port = closed_port().await;
    let root = format!("http://127.0.0.1:{port}");
    std::env::set_var("OLLAMA_URL", &root);

    let data = tmp.path().join("data");
    let db = Database::init(&data).await.unwrap();
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

    // kwant restarted while Ollama was still down: the startup check failed
    let (s, _) = call(&app, "POST", "/api/engines/recheck", None).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(
        state.engines.status(EngineId::Ollama),
        EngineStatus::NotInstalled
    );
    assert_eq!(ready_accounts(&app).await, 0);

    let (_, kwants) = call(&app, "GET", "/api/kwants", None).await;
    let k = kwants[0].clone();
    let kid = k["id"].as_str().unwrap().to_string();
    let mut body = k.clone();
    body.as_object_mut().unwrap().retain(|key, _| {
        ["name", "title", "persona_prompt", "soul", "avatar_sprite"].contains(&key.as_str())
    });
    body["preferred_model"] = json!(format!("ollama/{MODEL}"));
    let (s, r) = call(&app, "PUT", &format!("/api/kwants/{kid}"), Some(body)).await;
    assert_eq!(s, StatusCode::OK, "{r}");

    // Still down: the bot says Ollama isn't answering, with Re-check, never "add an account"
    let (text, took) = turn(&app, &state, &kid).await;
    assert!(
        text.contains(&format!(
            "Ollama isn't answering at {root}. Start it, then try again."
        )),
        "{text}"
    );
    assert!(
        !text.contains("No account can run bots") && !text.contains("Accounts"),
        "{text}"
    );
    assert!(took < Duration::from_secs(10), "{took:?}");
    let (_, meta) = last_reply(&state, &kid).await;
    assert_eq!(
        meta,
        json!({ "kind": "setup_needed", "reason": "ollama_down", "url": root })
    );

    // Ollama comes up. The cache still says it failed, and nobody presses Re-check
    let ollama = start_ollama(port).await;
    state.engines.store(vec![(EngineId::Ollama, failed())]);
    let (text, _) = turn(&app, &state, &kid).await;
    assert!(text.contains("lights are off"), "{text}");
    assert_eq!(ollama.chats.load(Ordering::SeqCst), 1);
    let (reply, meta) = last_reply(&state, &kid).await;
    assert!(
        reply.contains("lights are off") && meta["kind"] != "setup_needed",
        "{reply} {meta}"
    );

    // Ready Ollama counts in the space, as the sidebar footer says
    assert_eq!(state.engines.status(EngineId::Ollama), EngineStatus::Ready);
    assert_eq!(ready_accounts(&app).await, 1);
    // Off in Settings: not counted
    state.engines.set_enabled(EngineId::Ollama, false);
    assert_eq!(ready_accounts(&app).await, 0);
    state.engines.set_enabled(EngineId::Ollama, true);

    // A bot not pinned to Ollama, with Ollama down and nothing else: the usual setup card
    let port2 = closed_port().await;
    std::env::set_var("OLLAMA_URL", format!("http://127.0.0.1:{port2}"));
    state.engines.store(vec![(EngineId::Ollama, failed())]);
    let mut body = k.clone();
    body.as_object_mut().unwrap().retain(|key, _| {
        ["name", "title", "persona_prompt", "soul", "avatar_sprite"].contains(&key.as_str())
    });
    body["preferred_model"] = json!("auto");
    let (s, r) = call(&app, "PUT", &format!("/api/kwants/{kid}"), Some(body)).await;
    assert_eq!(s, StatusCode::OK, "{r}");
    let (text, _) = turn(&app, &state, &kid).await;
    assert!(text.contains("No account can run bots here yet"), "{text}");
    let (_, meta) = last_reply(&state, &kid).await;
    assert_eq!(meta["reason"], "no_account");
}
