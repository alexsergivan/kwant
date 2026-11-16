//! Dictation (roadmap Next 12b): `GET/POST /api/spaces/:id/transcribe`. A stub OpenAI server
//! stands in for api.openai.com (through the `KWNT_OPENAI_API_BASE` test seam): no test calls
//! the real API. The stub answers by key, so the tests can share it while running in parallel.

use axum::{
    body::{to_bytes, Body, Bytes},
    http::{HeaderMap, Request, StatusCode},
    response::{IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use kwnt::{
    broker::PermissionBroker,
    config::Config,
    db::{
        models::{NewLlmProfile, ProfileKey},
        spaces::{NewSpace, SpaceId, DEFAULT_SPACE_ID},
        Database,
    },
    drivers::engines::{EngineId, EngineStatus, ProbeEnv},
    router::LayaRouter,
    mcp::{FixedApprover, McpTurnContext},
    server::{continuity::SelfScope, create_router, mcp_runtime::PermissionSession, memory_recall, transcribe, AppState},
    tools::ToolRunner,
};
use futures::StreamExt;
use serde_json::{json, Value};
use std::cell::RefCell;
use std::collections::HashSet;
use std::io::Write;
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;
use tempfile::{tempdir, TempDir};

/// The Default space every fresh database starts with (`spc_default`, not "default").
const DEF: &str = DEFAULT_SPACE_ID;
use tower::ServiceExt;

/// One request the stub got: (key, model, language, file name, audio bytes).
type Call = (String, String, Option<String>, String, usize);

static CALLS: Mutex<Vec<Call>> = Mutex::new(Vec::new());
/// Uploads that came without a `Content-Length` matching their size (a chunked upload).
static UNSIZED: AtomicUsize = AtomicUsize::new(0);

/// The stub's log. Tests run in parallel, so a panic in one must not poison it for the rest.
fn calls() -> std::sync::MutexGuard<'static, Vec<Call>> {
    CALLS.lock().unwrap_or_else(|e| e.into_inner())
}

fn calls_with(key: &str) -> Vec<Call> {
    calls().iter().filter(|c| c.0 == key).cloned().collect()
}

/// The value of a multipart text field, or the file part's name and size.
fn field(body: &[u8], name: &str) -> Option<String> {
    let text = String::from_utf8_lossy(body);
    let start = text.find(&format!("name=\"{name}\"\r\n\r\n"))? + name.len() + 11;
    Some(text[start..].split("\r\n").next()?.to_string())
}

fn file_part(body: &[u8]) -> (String, usize) {
    let text = String::from_utf8_lossy(body).to_string();
    let i = text.find("filename=\"").unwrap() + 10;
    let name = text[i..].split('"').next().unwrap().to_string();
    let head = b"Content-Type: application/octet-stream\r\n\r\n";
    let from = body.windows(head.len()).position(|w| w == head).unwrap() + head.len();
    let tail = body.windows(4).rposition(|w| w == b"\r\n--").unwrap();
    (name, tail - from)
}

async fn stub(headers: HeaderMap, body: Bytes) -> Response {
    let key = headers.get("authorization").and_then(|h| h.to_str().ok()).unwrap_or("").trim_start_matches("Bearer ").to_string();
    assert!(headers.get("content-type").unwrap().to_str().unwrap().starts_with("multipart/form-data; boundary="));
    let model = field(&body, "model").unwrap();
    assert_eq!(field(&body, "response_format").as_deref(), Some("json"));
    let (file, size) = file_part(&body);
    let declared = headers.get("content-length").and_then(|h| h.to_str().ok()).and_then(|v| v.parse::<usize>().ok());
    if declared != Some(body.len()) {
        UNSIZED.fetch_add(1, Ordering::SeqCst);
    }
    calls().push((key.clone(), model.clone(), field(&body, "language"), file, size));
    match (key.as_str(), model.as_str()) {
        ("sk-bad", _) => (
            StatusCode::UNAUTHORIZED,
            Json(json!({"error": {"message": "Incorrect API key provided: sk-bad. You can find your API key at https://platform.openai.com/account/api-keys.", "type": "invalid_request_error", "code": "invalid_api_key"}})),
        )
            .into_response(),
        ("sk-limit", _) => (StatusCode::TOO_MANY_REQUESTS, Json(json!({"error": {"message": "Rate limit reached", "code": "rate_limit_exceeded"}}))).into_response(),
        ("sk-old", "gpt-4o-mini-transcribe") => (
            StatusCode::NOT_FOUND,
            Json(json!({"error": {"message": "The model `gpt-4o-mini-transcribe` does not exist", "code": "model_not_found"}})),
        )
            .into_response(),
        (_, m) => Json(json!({"text": format!("  hello from {key} via {m}  ")})).into_response(),
    }
}

/// The OpenAI API engine check: lists models for any key.
async fn models() -> Json<Value> {
    Json(json!({"data": [{"id": "whisper-1"}]}))
}

/// Starts the stub once, on its own thread and runtime (each test has its own runtime), and
/// points kwnt at it.
fn stub_base() -> &'static str {
    static BASE: OnceLock<String> = OnceLock::new();
    BASE.get_or_init(|| {
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let rt = tokio::runtime::Builder::new_multi_thread().worker_threads(1).enable_all().build().unwrap();
            rt.block_on(async move {
                let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
                tx.send(format!("http://{}", listener.local_addr().unwrap())).unwrap();
                let app = Router::new()
                    .route("/v1/audio/transcriptions", post(stub))
                    .route("/v1/models", get(models))
                    .layer(axum::extract::DefaultBodyLimit::max(64 << 20));
                axum::serve(listener, app).await.unwrap();
            });
        });
        let base = rx.recv().unwrap();
        std::env::set_var(transcribe::API_BASE_ENV, &base);
        std::env::remove_var("OPENAI_API_KEY");
        base
    })
}

async fn setup() -> (Router, AppState, TempDir) {
    setup_with(|_| {}).await
}

/// [`setup`], with a change to the state before the router is made.
async fn setup_with(change: impl FnOnce(&mut AppState)) -> (Router, AppState, TempDir) {
    install_log_capture();
    stub_base();
    let tmp = tempdir().unwrap();
    let db = Database::init(tmp.path()).await.unwrap();
    db.set_setting("mission_board", "false").await.unwrap();
    let config = Config { data_dir: tmp.path().to_path_buf(), ..Config::default() };
    let mut state = AppState::new(
        config.clone(),
        db,
        ToolRunner::new(tmp.path().join("workspace")),
        PermissionBroker::new(),
        LayaRouter::new(config.laya_url.clone()),
    );
    change(&mut state);
    (create_router(state.clone()), state, tmp)
}

async fn space_key(state: &AppState, space: &SpaceId, key: &str) {
    let p = state
        .db
        .create_profile(
            space,
            &NewLlmProfile {
                engine: "codex".into(),
                name: "OpenAI key".into(),
                kind: "api_key".into(),
                home_dir: None,
                priority: 0,
                enabled: true,
                is_default: false,
            },
        )
        .await
        .unwrap();
    state.db.set_profile_key(&p.id, &ProfileKey(key.into())).await.unwrap();
}

async fn call(app: &Router, req: Request<Body>) -> (StatusCode, Value) {
    let resp = app.clone().oneshot(req).await.unwrap();
    let status = resp.status();
    let bytes = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    (status, serde_json::from_slice(&bytes).unwrap_or(Value::Null))
}

async fn status(app: &Router, space: &str) -> (StatusCode, Value) {
    call(app, Request::get(format!("/api/spaces/{space}/transcribe")).body(Body::empty()).unwrap()).await
}

fn audio_post(space: &str, query: &str, content_type: &str, audio: Vec<u8>) -> Request<Body> {
    Request::post(format!("/api/spaces/{space}/transcribe{query}"))
        .header("content-type", content_type)
        .header("content-length", audio.len().to_string())
        .body(Body::from(audio))
        .unwrap()
}

fn files(root: &Path) -> HashSet<String> {
    let mut out = HashSet::new();
    let mut dirs = vec![root.to_path_buf()];
    while let Some(d) = dirs.pop() {
        for e in std::fs::read_dir(&d).unwrap().flatten() {
            let p = e.path();
            if p.is_dir() {
                dirs.push(p);
            } else {
                out.insert(p.strip_prefix(root).unwrap().display().to_string());
            }
        }
    }
    out
}

#[tokio::test]
async fn with_no_key_it_is_unavailable_and_says_why() {
    let (app, _state, _tmp) = setup().await;
    let (code, v) = status(&app, DEF).await;
    assert_eq!(code, StatusCode::OK);
    assert_eq!(v, json!({"available": false, "source": null, "why": transcribe::WHY_NO_KEY}));

    // Other tests call the shared stub at the same time, so a total count races. This
    // recording's size (1001 bytes) is used by no other test: no stub call may carry it.
    let (code, v) = call(&app, audio_post(DEF, "", "audio/webm", vec![1; 1001])).await;
    assert_eq!(code, StatusCode::CONFLICT);
    assert_eq!(v, json!({"error": transcribe::WHY_NO_KEY, "code": "no_transcriber", "why": transcribe::WHY_NO_KEY}));
    assert!(calls().iter().all(|c| c.4 != 1001), "no upstream call without a key");

    let (code, _) = status(&app, "spc_missing").await;
    assert_eq!(code, StatusCode::NOT_FOUND);
    let (code, v) = call(&app, audio_post("spc_missing", "", "audio/webm", vec![1; 10])).await;
    assert_eq!((code, v["code"].clone()), (StatusCode::NOT_FOUND, json!("not_found")));
}

#[tokio::test]
async fn the_space_key_transcribes_and_nothing_is_stored() {
    let (app, state, tmp) = setup().await;
    state.db.set_setting("openai_api_key", "sk-host-unused").await.unwrap();
    space_key(&state, &SpaceId::default_space(), "sk-space").await;
    let (_, v) = status(&app, DEF).await;
    assert_eq!(v, json!({"available": true, "source": "openai", "why": null}));

    let before = files(tmp.path());
    let audio: Vec<u8> = (0..300_000u32).map(|i| (i % 251) as u8).collect();
    let (code, v) = call(&app, audio_post(DEF, "?lang=DE", "audio/webm;codecs=opus", audio)).await;
    assert_eq!(code, StatusCode::OK, "{v}");
    assert_eq!(v, json!({"text": "hello from sk-space via gpt-4o-mini-transcribe", "source": "openai"}));
    assert_eq!(
        calls_with("sk-space"),
        vec![("sk-space".into(), "gpt-4o-mini-transcribe".into(), Some("de".into()), "audio.webm".into(), 300_000)],
        "the space key beats the host key, all bytes go up once"
    );
    assert!(calls_with("sk-host-unused").is_empty());
    assert_eq!(files(tmp.path()), before, "no file is left behind");
    let attachments: Vec<String> = state.db.list_attachments_in(DEF, 10).await.unwrap().into_iter().map(|a| a.id).collect();
    assert!(attachments.is_empty(), "{attachments:?}");
}

#[tokio::test]
async fn the_host_key_is_used_unless_this_machine_is_off() {
    let (app, state, _tmp) = setup().await;
    state.db.set_setting("openai_api_key", "sk-host").await.unwrap();
    let (code, v) = call(&app, audio_post(DEF, "", "audio/mp4", vec![7; 2048])).await;
    assert_eq!(code, StatusCode::OK, "{v}");
    assert_eq!(v["text"], "hello from sk-host via gpt-4o-mini-transcribe");
    assert!(calls_with("sk-host").iter().any(|c| c.3 == "audio.mp4" && c.4 == 2048 && c.2.is_none()));

    let off = state
        .db
        .create_space(&NewSpace { name: "Off".into(), icon: "x".into(), color: None, this_machine_enabled: false })
        .await
        .unwrap();
    let (_, v) = status(&app, &off.id).await;
    assert_eq!(v, json!({"available": false, "source": null, "why": transcribe::WHY_HOST_OFF}));
    let (code, v) = call(&app, audio_post(&off.id, "", "audio/mp4", vec![7; 10])).await;
    assert_eq!((code, v["code"].clone()), (StatusCode::CONFLICT, json!("no_transcriber")));

    // Its own key works there
    space_key(&state, &SpaceId::new(&off.id), "sk-offspace").await;
    let (code, v) = call(&app, audio_post(&off.id, "", "audio/ogg", vec![7; 10])).await;
    assert_eq!(code, StatusCode::OK, "{v}");
    assert_eq!(v["text"], "hello from sk-offspace via gpt-4o-mini-transcribe");

    // The OpenAI API engine turned off: nobody's key is used
    state.engines.set_disabled(HashSet::from([EngineId::OpenaiApi]));
    let (_, v) = status(&app, &off.id).await;
    assert_eq!(v["why"], transcribe::WHY_ENGINE_OFF);
}

#[tokio::test]
async fn a_refused_key_is_a_502_without_the_key() {
    let (app, state, _tmp) = setup().await;
    space_key(&state, &SpaceId::default_space(), "sk-bad").await;
    // Everything this test's thread logs
    let (log, _capture) = capture_logs();

    // Another thread reaches the same `dictation failed` line first, as a parallel test can (a
    // 429 in `other_upstream_failures_and_the_fallback_model`). With a thread-scoped subscriber
    // that registered the line against the other thread's empty default and this test's log
    // stayed empty (CI #673, #686). The capture must not depend on which thread logs first.
    let limited = state
        .db
        .create_space(&NewSpace { name: "Elsewhere".into(), icon: "x".into(), color: None, this_machine_enabled: true })
        .await
        .unwrap();
    space_key(&state, &SpaceId::new(&limited.id), "sk-limit").await;
    let (other, req, rt) = (app.clone(), audio_post(&limited.id, "", "audio/webm", vec![3; 100]), tokio::runtime::Handle::current());
    let (code, _) = tokio::task::spawn_blocking(move || rt.block_on(call(&other, req))).await.unwrap();
    assert_eq!(code, StatusCode::BAD_GATEWAY, "the other thread logged `dictation failed` too");
    let early = String::from_utf8(log.lock().unwrap().clone()).unwrap();
    assert!(!early.contains("dictation failed"), "another thread's log isn't this test's: {early}");

    let (code, v) = call(&app, audio_post(DEF, "", "audio/wav", vec![3; 500])).await;
    assert_eq!(code, StatusCode::BAD_GATEWAY);
    assert_eq!(v, json!({"error": "The OpenAI key was refused.", "code": "transcriber_failed"}));
    assert_eq!(calls_with("sk-bad").len(), 1, "a refused key isn't retried with the other model");
    let log = String::from_utf8(log.lock().unwrap().clone()).unwrap();
    assert_eq!(log.matches("dictation failed").count(), 1, "{log}");
    assert!(!log.contains("sk-bad") && !v.to_string().contains("sk-bad"), "{log}");
    assert!(!log.contains("Incorrect API key"), "OpenAI's own text isn't passed on: {log}");
}

thread_local! {
    /// Where this thread's log goes while a test captures it ([`capture_logs`]).
    static CAPTURE: RefCell<Option<Arc<Mutex<Vec<u8>>>>> = const { RefCell::new(None) };
}

/// One global subscriber for the whole binary, installed by [`setup_with`] before any test can
/// reach a server log line. It writes only what a capturing thread logs.
///
/// Not `tracing::subscriber::set_default` per test: while that is the only dispatcher ever made,
/// tracing-core (0.1.36) asks the *registering thread's* default whether a log line is wanted. A
/// line first reached by another test's thread (no subscriber) is cached as "never", and the
/// capturing test's own event is dropped: an empty log at random. The filter is dynamic for the
/// same reason: `filter_fn` caches its answer per line too.
fn install_log_capture() {
    static ONCE: OnceLock<()> = OnceLock::new();
    ONCE.get_or_init(|| {
        use tracing_subscriber::prelude::*;
        let capturing = tracing_subscriber::filter::dynamic_filter_fn(|_, _| CAPTURE.try_with(|c| c.borrow().is_some()).unwrap_or(false));
        let layer = tracing_subscriber::fmt::layer().with_ansi(false).with_writer(|| Capture).with_filter(capturing);
        tracing::subscriber::set_global_default(tracing_subscriber::registry().with(layer)).unwrap();
    });
}

/// Captures everything this thread logs (at every level) until the guard drops.
fn capture_logs() -> (Arc<Mutex<Vec<u8>>>, CaptureGuard) {
    install_log_capture();
    let log = Arc::new(Mutex::new(Vec::new()));
    CAPTURE.with(|c| *c.borrow_mut() = Some(log.clone()));
    (log, CaptureGuard)
}

struct CaptureGuard;

impl Drop for CaptureGuard {
    fn drop(&mut self) {
        let _ = CAPTURE.try_with(|c| *c.borrow_mut() = None);
    }
}

/// Writes to the current thread's capture, if any.
struct Capture;

impl Write for Capture {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        let _ = CAPTURE.try_with(|c| {
            if let Some(log) = c.borrow().as_ref() {
                log.lock().unwrap_or_else(|e| e.into_inner()).extend_from_slice(buf);
            }
        });
        Ok(buf.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

#[tokio::test]
async fn other_upstream_failures_and_the_fallback_model() {
    let (app, state, _tmp) = setup().await;
    space_key(&state, &SpaceId::default_space(), "sk-old").await;
    let (code, v) = call(&app, audio_post(DEF, "", "audio/mpeg", vec![3; 100])).await;
    assert_eq!(code, StatusCode::OK, "{v}");
    assert_eq!(v["text"], "hello from sk-old via whisper-1");
    let models: Vec<String> = calls_with("sk-old").into_iter().map(|c| c.1).collect();
    assert_eq!(models, ["gpt-4o-mini-transcribe", "whisper-1"]);

    let limited = state
        .db
        .create_space(&NewSpace { name: "Limited".into(), icon: "x".into(), color: None, this_machine_enabled: true })
        .await
        .unwrap();
    space_key(&state, &SpaceId::new(&limited.id), "sk-limit").await;
    let (code, v) = call(&app, audio_post(&limited.id, "", "audio/webm", vec![3; 100])).await;
    assert_eq!(code, StatusCode::BAD_GATEWAY);
    assert_eq!(v["code"], "transcriber_failed");
    assert_eq!(v["error"], "OpenAI is limiting this key, or its quota is used up. Try again in a minute.");
}

#[tokio::test]
async fn refusals_come_before_anything_goes_upstream() {
    let (app, state, _tmp) = setup().await;
    space_key(&state, &SpaceId::default_space(), "sk-refusals").await;

    // 26 MB declared: refused before a byte is read
    let req = Request::post(format!("/api/spaces/{DEF}/transcribe"))
        .header("content-type", "audio/webm")
        .header("content-length", (26 * 1024 * 1024).to_string())
        .body(Body::from(vec![0u8; 10]))
        .unwrap();
    let (code, v) = call(&app, req).await;
    assert_eq!(code, StatusCode::PAYLOAD_TOO_LARGE);
    assert_eq!(v["code"], "too_large");

    // No length declared: refused at the byte that passes 25 MB
    let chunks = (0..26).map(|_| Ok::<_, std::io::Error>(Bytes::from(vec![0u8; 1024 * 1024])));
    let req = Request::post(format!("/api/spaces/{DEF}/transcribe"))
        .header("content-type", "audio/webm")
        .body(Body::from_stream(futures::stream::iter(chunks)))
        .unwrap();
    let (code, v) = call(&app, req).await;
    assert_eq!((code, v["code"].clone()), (StatusCode::PAYLOAD_TOO_LARGE, json!("too_large")));

    // Exactly 25 MB goes through
    let (code, v) = call(&app, audio_post(DEF, "", "audio/webm", vec![0u8; transcribe::MAX_AUDIO_BYTES as usize])).await;
    assert_eq!(code, StatusCode::OK, "{v}");

    for (ct, query, want_code, want) in [
        ("text/plain", "", StatusCode::UNSUPPORTED_MEDIA_TYPE, "bad_audio_type"),
        ("video/webm", "", StatusCode::UNSUPPORTED_MEDIA_TYPE, "bad_audio_type"),
        ("audio/webm", "?lang=english", StatusCode::BAD_REQUEST, "bad_lang"),
    ] {
        let (code, v) = call(&app, audio_post(DEF, query, ct, vec![1; 10])).await;
        assert_eq!((code, v["code"].as_str().unwrap()), (want_code, want), "{ct} {query}");
    }
    let (code, v) = call(&app, audio_post(DEF, "", "audio/webm", Vec::new())).await;
    assert_eq!((code, v["code"].clone()), (StatusCode::BAD_REQUEST, json!("empty_audio")));

    // A bot's turn token is refused: dictation is the person's
    state.self_tools.lock().await.insert("turn-token".into(), SelfScope::default());
    let mut req = audio_post(DEF, "", "audio/webm", vec![1; 10]);
    req.headers_mut().insert("authorization", "Bearer turn-token".parse().unwrap());
    let (code, v) = call(&app, req).await;
    assert_eq!(code, StatusCode::FORBIDDEN);
    assert_eq!(v, json!({"error": "only the person can dictate", "code": "forbidden"}));

    assert_eq!(calls_with("sk-refusals").len(), 1, "only the 25 MB recording went up");
    assert_eq!(calls_with("sk-refusals")[0].4, transcribe::MAX_AUDIO_BYTES as usize);
}

/// How many rows `table` has, read straight from the database file.
async fn rows(tmp: &TempDir, table: &str) -> i64 {
    let url = format!("sqlite://{}?mode=ro", tmp.path().join("kwnt.db").display());
    let pool = sqlx::SqlitePool::connect(&url).await.unwrap();
    let n: i64 = sqlx::query_scalar(&format!("SELECT COUNT(*) FROM {table}")).fetch_one(&pool).await.unwrap();
    pool.close().await;
    n
}

#[tokio::test]
async fn a_transcription_writes_no_chat_message_and_no_memory() {
    let (app, state, tmp) = setup().await;
    space_key(&state, &SpaceId::default_space(), "sk-nowrite").await;
    let (messages, memories) = (rows(&tmp, "messages").await, rows(&tmp, "kwant_memories").await);
    let (code, v) = call(&app, audio_post(DEF, "", "audio/webm", vec![5; 4096])).await;
    assert_eq!(code, StatusCode::OK, "{v}");
    assert_eq!(calls_with("sk-nowrite").len(), 1);
    assert_eq!(rows(&tmp, "messages").await, messages, "no chat message");
    assert_eq!(rows(&tmp, "kwant_memories").await, memories, "no memory");
}

#[tokio::test]
async fn an_archived_space_or_the_engine_off_is_refused_before_anything_goes_up() {
    let (app, state, _tmp) = setup().await;
    let gone = state
        .db
        .create_space(&NewSpace { name: "Gone".into(), icon: "x".into(), color: None, this_machine_enabled: true })
        .await
        .unwrap();
    space_key(&state, &SpaceId::new(&gone.id), "sk-archived").await;
    state.db.archive_space(&SpaceId::new(&gone.id)).await.unwrap();
    let (code, v) = call(&app, audio_post(&gone.id, "", "audio/webm", vec![1; 10])).await;
    assert_eq!(code, StatusCode::CONFLICT);
    assert_eq!(v["code"], "archived", "{v}");

    // The OpenAI API engine off: the space's own key isn't used either
    space_key(&state, &SpaceId::default_space(), "sk-engine-off").await;
    state.engines.set_disabled(HashSet::from([EngineId::OpenaiApi]));
    let (code, v) = call(&app, audio_post(DEF, "", "audio/webm", vec![1; 10])).await;
    assert_eq!(code, StatusCode::CONFLICT);
    assert_eq!(v, json!({"error": transcribe::WHY_ENGINE_OFF, "code": "no_transcriber", "why": transcribe::WHY_ENGINE_OFF}));

    assert!(calls_with("sk-archived").is_empty() && calls_with("sk-engine-off").is_empty());
}

#[tokio::test]
async fn an_mcp_session_token_is_refused() {
    let (app, state, _tmp) = setup().await;
    space_key(&state, &SpaceId::default_space(), "sk-mcp").await;
    let ctx = McpTurnContext {
        servers: vec![],
        approver: Arc::new(FixedApprover(true)),
        config_dir: std::env::temp_dir(),
        permission_url: None,
        gateway: None,
        max_tool_iterations: 8,
        approvals: kwnt::mcp::Approvals::Ask,
        records: Default::default(),
        kwnt_only: false,
    };
    state.mcp_sessions.lock().await.insert("mcp-turn".into(), PermissionSession::new("k1".into(), "Ada".into(), "t1".into(), ctx, None));
    let mut req = audio_post(DEF, "", "audio/webm", vec![1; 10]);
    req.headers_mut().insert("authorization", "Bearer mcp-turn".parse().unwrap());
    let (code, v) = call(&app, req).await;
    assert_eq!(code, StatusCode::FORBIDDEN);
    assert_eq!(v, json!({"error": "only the person can dictate", "code": "forbidden"}));
    assert!(calls_with("sk-mcp").is_empty());
}

/// A body that notes when it is first read.
fn watched_body(read: Arc<AtomicBool>) -> Body {
    Body::from_stream(futures::stream::once(async move {
        read.store(true, Ordering::SeqCst);
        Ok::<_, std::io::Error>(Bytes::from(vec![9u8; 1000]))
    }))
}

fn watched_post(read: &Arc<AtomicBool>) -> Request<Body> {
    Request::post(format!("/api/spaces/{DEF}/transcribe")).header("content-type", "audio/webm").body(watched_body(read.clone())).unwrap()
}

#[tokio::test]
async fn while_two_are_held_the_next_waits_unread_then_gets_busy_or_its_turn() {
    // Its own slots, so the other tests running beside it keep theirs
    let (app, state, _tmp) = setup_with(|s| s.dictation = Arc::new(transcribe::Slots::with_wait(Duration::from_millis(800)))).await;
    space_key(&state, &SpaceId::default_space(), "sk-busy").await;
    let one = state.dictation.at_once.acquire().await.unwrap();
    let two = state.dictation.at_once.acquire().await.unwrap();

    // Both held past the wait: busy, and not a byte of the body was read
    let read = Arc::new(AtomicBool::new(false));
    let (code, v) = call(&app, watched_post(&read)).await;
    assert_eq!(code, StatusCode::TOO_MANY_REQUESTS);
    assert_eq!(v["code"], "busy", "{v}");
    assert!(!read.load(Ordering::SeqCst), "a busy request's body is never read");

    // While it waits for its turn it hasn't read its body either; once a slot frees, it goes
    let read = Arc::new(AtomicBool::new(false));
    let waiting = tokio::spawn({
        let app = app.clone();
        let req = watched_post(&read);
        async move { call(&app, req).await }
    });
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert!(!read.load(Ordering::SeqCst), "a waiting request holds no audio");
    drop(one);
    let (code, v) = waiting.await.unwrap();
    assert_eq!(code, StatusCode::OK, "{v}");
    assert!(read.load(Ordering::SeqCst));
    assert_eq!(calls_with("sk-busy").len(), 1, "only the one that got its turn went up");
    drop(two);
}

#[tokio::test]
async fn a_stalled_upload_is_interrupted_and_gives_its_slot_back() {
    // Its own slots, and a short stall instead of 15 s
    let (app, state, _tmp) = setup_with(|s| {
        s.dictation = Arc::new(transcribe::Slots { read_stall: Duration::from_secs(1), ..Default::default() })
    })
    .await;
    space_key(&state, &SpaceId::default_space(), "sk-stall").await;
    // Two uploads send 1 KB, then nothing, and never end: both slots are taken
    let stalled = || {
        let body = Body::from_stream(futures::stream::once(async { Ok::<_, std::io::Error>(Bytes::from(vec![1u8; 1000])) }).chain(futures::stream::pending()));
        Request::post(format!("/api/spaces/{DEF}/transcribe")).header("content-type", "audio/webm").body(body).unwrap()
    };
    let held: Vec<_> = (0..2)
        .map(|_| {
            let (app, req) = (app.clone(), stalled());
            tokio::spawn(async move { call(&app, req).await })
        })
        .collect();
    let taken = tokio::time::timeout(Duration::from_secs(5), async {
        while state.dictation.at_once.available_permits() > 0 {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    });
    taken.await.expect("both stalled uploads hold a slot while they read");

    // After the stall each one is refused as interrupted, nothing went upstream, and both slots are back
    for h in held {
        let (code, v) = tokio::time::timeout(Duration::from_secs(5), h).await.expect("a stalled upload ends").unwrap();
        assert_eq!(code, StatusCode::BAD_REQUEST, "{v}");
        assert_eq!(v["code"], "interrupted", "{v}");
    }
    assert_eq!(state.dictation.at_once.available_permits(), transcribe::MAX_AT_ONCE);
    assert!(calls_with("sk-stall").is_empty(), "a stalled upload never goes to OpenAI");

    // A normal recording gets a slot and its text
    let (code, v) = call(&app, audio_post(DEF, "", "audio/webm", vec![1; 100])).await;
    assert_eq!(code, StatusCode::OK, "{v}");
    assert_eq!(calls_with("sk-stall").len(), 1);
}

#[tokio::test]
async fn every_upload_declares_its_length() {
    let (app, state, _tmp) = setup().await;
    space_key(&state, &SpaceId::default_space(), "sk-sized").await;
    // No length from the browser; the upload to OpenAI still has one (the stub counts any without)
    let chunks = (0..3).map(|_| Ok::<_, std::io::Error>(Bytes::from(vec![4u8; 70_000])));
    let req = Request::post(format!("/api/spaces/{DEF}/transcribe")).header("content-type", "audio/webm").body(Body::from_stream(futures::stream::iter(chunks))).unwrap();
    let (code, v) = call(&app, req).await;
    assert_eq!(code, StatusCode::OK, "{v}");
    assert_eq!(calls_with("sk-sized")[0].4, 210_000);
    assert_eq!(UNSIZED.load(Ordering::SeqCst), 0, "every upload had a Content-Length matching its size");
}

/// Hybrid recall's embeddings and dictation read one key order (`server::openai_key`): the same
/// key in each case, and neither when OpenAI is out.
#[tokio::test]
async fn recall_and_dictation_pick_the_same_key() {
    let (_app, state, _tmp) = setup().await;
    let new_space = |name: &str, this_machine_enabled: bool| {
        let db = state.db.clone();
        let name = name.to_string();
        async move { SpaceId::new(&db.create_space(&NewSpace { name, icon: "x".into(), color: None, this_machine_enabled }).await.unwrap().id) }
    };
    state.db.set_setting("openai_api_key", " sk-order-host ").await.unwrap();
    let both = new_space("Both", true).await;
    space_key(&state, &both, "sk-order-space").await;
    let host = new_space("Host", true).await;
    let off = new_space("Off", false).await;
    let off_own = new_space("Off own", false).await;
    space_key(&state, &off_own, "sk-order-own").await;

    let keys = |state: AppState, space: SpaceId| async move {
        let recall = match memory_recall::source_for_space(&state, &space).await {
            Some(memory_recall::EmbedSource::OpenAi { key, .. }) => Some(key),
            _ => None,
        };
        let dictation = match transcribe::transcribe_source_for_space(&state, &space).await {
            Ok(transcribe::TranscribeSource::OpenAi { key, .. }) => Some(key),
            Err(_) => None,
        };
        (recall, dictation)
    };
    let s = |k: &str| Some(k.to_string());
    // The one difference, on purpose (#630): until the OpenAI API engine's check has passed,
    // recall leaves the host's key alone (a revoked key isn't retried every minute in the
    // background); dictation, which the person starts, still tries it. A space's own key is
    // used by both either way.
    assert_ne!(state.engines.report(EngineId::OpenaiApi).status, EngineStatus::Ready);
    assert_eq!(keys(state.clone(), host.clone()).await, (None, s("sk-order-host")), "host key before the check");
    assert_eq!(keys(state.clone(), both.clone()).await, (s("sk-order-space"), s("sk-order-space")), "space key before the check");
    let env = ProbeEnv { openai_key: s("sk-order-host"), openai_base: stub_base().to_string(), ..ProbeEnv::default() };
    state.engines.recheck_one(EngineId::OpenaiApi, &env).await;
    assert_eq!(state.engines.report(EngineId::OpenaiApi).status, EngineStatus::Ready);
    for (space, want) in [(&both, s("sk-order-space")), (&host, s("sk-order-host")), (&off, None), (&off_own, s("sk-order-own"))] {
        assert_eq!(keys(state.clone(), space.clone()).await, (want.clone(), want), "{space}");
    }
    state.engines.set_disabled(HashSet::from([EngineId::OpenaiApi]));
    for space in [&both, &host, &off, &off_own] {
        assert_eq!(keys(state.clone(), space.clone()).await, (None, None), "engine off: {space}");
    }
}
