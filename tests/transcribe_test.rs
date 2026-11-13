//! Dictation (roadmap Next 12b): `GET/POST /api/spaces/:id/transcribe`. A stub OpenAI server
//! stands in for api.openai.com (through the `KWNT_OPENAI_API_BASE` test seam): no test calls
//! the real API. The stub answers by key, so the tests can share it while running in parallel.

use axum::{
    body::{to_bytes, Body, Bytes},
    http::{HeaderMap, Request, StatusCode},
    response::{IntoResponse, Response},
    routing::post,
    Json, Router,
};
use kwnt::{
    broker::PermissionBroker,
    config::Config,
    db::{
        models::{NewLlmProfile, ProfileKey},
        spaces::{NewSpace, SpaceId},
        Database,
    },
    drivers::engines::EngineId,
    router::LayaRouter,
    server::{continuity::SelfScope, create_router, transcribe, AppState},
    tools::ToolRunner,
};
use serde_json::{json, Value};
use std::collections::HashSet;
use std::io::Write;
use std::path::Path;
use std::sync::{Arc, Mutex, OnceLock};
use tempfile::{tempdir, TempDir};
use tower::ServiceExt;

/// One request the stub got: (key, model, language, file name, audio bytes).
type Call = (String, String, Option<String>, String, usize);

static CALLS: Mutex<Vec<Call>> = Mutex::new(Vec::new());

fn calls_with(key: &str) -> Vec<Call> {
    CALLS.lock().unwrap().iter().filter(|c| c.0 == key).cloned().collect()
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
    CALLS.lock().unwrap().push((key.clone(), model.clone(), field(&body, "language"), file, size));
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
                let app = Router::new().route("/v1/audio/transcriptions", post(stub)).layer(axum::extract::DefaultBodyLimit::max(64 << 20));
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
    stub_base();
    let tmp = tempdir().unwrap();
    let db = Database::init(tmp.path()).await.unwrap();
    db.set_setting("mission_board", "false").await.unwrap();
    let config = Config { data_dir: tmp.path().to_path_buf(), ..Config::default() };
    let state = AppState::new(
        config.clone(),
        db,
        ToolRunner::new(tmp.path().join("workspace")),
        PermissionBroker::new(),
        LayaRouter::new(config.laya_url.clone()),
    );
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
    let (app, state, _tmp) = setup().await;
    let (code, v) = status(&app, "default").await;
    assert_eq!(code, StatusCode::OK);
    assert_eq!(v, json!({"available": false, "source": null, "why": transcribe::WHY_NO_KEY}));

    let before = CALLS.lock().unwrap().len();
    let (code, v) = call(&app, audio_post("default", "", "audio/webm", vec![1; 1000])).await;
    assert_eq!(code, StatusCode::CONFLICT);
    assert_eq!(v, json!({"error": transcribe::WHY_NO_KEY, "code": "no_transcriber", "why": transcribe::WHY_NO_KEY}));
    assert_eq!(CALLS.lock().unwrap().len(), before, "no upstream call without a key");

    let (code, _) = status(&app, "spc_missing").await;
    assert_eq!(code, StatusCode::NOT_FOUND);
    let (code, _) = call(&app, audio_post("spc_missing", "", "audio/webm", vec![1; 10])).await;
    assert_eq!(code, StatusCode::NOT_FOUND);
    drop(state);
}

#[tokio::test]
async fn the_space_key_transcribes_and_nothing_is_stored() {
    let (app, state, tmp) = setup().await;
    state.db.set_setting("openai_api_key", "sk-host-unused").await.unwrap();
    space_key(&state, &SpaceId::default_space(), "sk-space").await;
    let (_, v) = status(&app, "default").await;
    assert_eq!(v, json!({"available": true, "source": "openai", "why": null}));

    let before = files(tmp.path());
    let audio: Vec<u8> = (0..300_000u32).map(|i| (i % 251) as u8).collect();
    let (code, v) = call(&app, audio_post("default", "?lang=DE", "audio/webm;codecs=opus", audio)).await;
    assert_eq!(code, StatusCode::OK, "{v}");
    assert_eq!(v, json!({"text": "hello from sk-space via gpt-4o-mini-transcribe", "source": "openai"}));
    assert_eq!(
        calls_with("sk-space"),
        vec![("sk-space".into(), "gpt-4o-mini-transcribe".into(), Some("de".into()), "audio.webm".into(), 300_000)],
        "the space key beats the host key, all bytes go up once"
    );
    assert!(calls_with("sk-host-unused").is_empty());
    assert_eq!(files(tmp.path()), before, "no file is left behind");
    let attachments: Vec<String> = state.db.list_attachments_in("default", 10).await.unwrap().into_iter().map(|a| a.id).collect();
    assert!(attachments.is_empty(), "{attachments:?}");
}

#[tokio::test]
async fn the_host_key_is_used_unless_this_machine_is_off() {
    let (app, state, _tmp) = setup().await;
    state.db.set_setting("openai_api_key", "sk-host").await.unwrap();
    let (code, v) = call(&app, audio_post("default", "", "audio/mp4", vec![7; 2048])).await;
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
    let log = Arc::new(Mutex::new(Vec::<u8>::new()));
    let sink = log.clone();
    let subscriber = tracing_subscriber::fmt()
        .with_max_level(tracing::Level::TRACE)
        .with_ansi(false)
        .with_writer(move || Capture(sink.clone()))
        .finish();
    let _guard = tracing::subscriber::set_default(subscriber);

    let (code, v) = call(&app, audio_post("default", "", "audio/wav", vec![3; 500])).await;
    assert_eq!(code, StatusCode::BAD_GATEWAY);
    assert_eq!(v, json!({"error": "The OpenAI key was refused.", "code": "transcriber_failed"}));
    assert_eq!(calls_with("sk-bad").len(), 1, "a refused key isn't retried with the other model");
    let log = String::from_utf8(log.lock().unwrap().clone()).unwrap();
    assert!(log.contains("dictation failed"), "{log}");
    assert!(!log.contains("sk-bad") && !v.to_string().contains("sk-bad"), "{log}");
    assert!(!log.contains("Incorrect API key"), "OpenAI's own text isn't passed on: {log}");
}

struct Capture(Arc<Mutex<Vec<u8>>>);

impl Write for Capture {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(buf);
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
    let (code, v) = call(&app, audio_post("default", "", "audio/mpeg", vec![3; 100])).await;
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
    let req = Request::post("/api/spaces/default/transcribe")
        .header("content-type", "audio/webm")
        .header("content-length", (26 * 1024 * 1024).to_string())
        .body(Body::from(vec![0u8; 10]))
        .unwrap();
    let (code, v) = call(&app, req).await;
    assert_eq!(code, StatusCode::PAYLOAD_TOO_LARGE);
    assert_eq!(v["code"], "too_large");

    // No length declared: refused at the byte that passes 25 MB
    let chunks = (0..26).map(|_| Ok::<_, std::io::Error>(Bytes::from(vec![0u8; 1024 * 1024])));
    let req = Request::post("/api/spaces/default/transcribe")
        .header("content-type", "audio/webm")
        .body(Body::from_stream(futures::stream::iter(chunks)))
        .unwrap();
    let (code, v) = call(&app, req).await;
    assert_eq!((code, v["code"].clone()), (StatusCode::PAYLOAD_TOO_LARGE, json!("too_large")));

    // Exactly 25 MB goes through
    let (code, v) = call(&app, audio_post("default", "", "audio/webm", vec![0u8; transcribe::MAX_AUDIO_BYTES as usize])).await;
    assert_eq!(code, StatusCode::OK, "{v}");

    for (ct, query, want_code, want) in [
        ("text/plain", "", StatusCode::UNSUPPORTED_MEDIA_TYPE, "bad_audio_type"),
        ("video/webm", "", StatusCode::UNSUPPORTED_MEDIA_TYPE, "bad_audio_type"),
        ("audio/webm", "?lang=english", StatusCode::BAD_REQUEST, "bad_lang"),
    ] {
        let (code, v) = call(&app, audio_post("default", query, ct, vec![1; 10])).await;
        assert_eq!((code, v["code"].as_str().unwrap()), (want_code, want), "{ct} {query}");
    }
    let (code, v) = call(&app, audio_post("default", "", "audio/webm", Vec::new())).await;
    assert_eq!((code, v["code"].clone()), (StatusCode::BAD_REQUEST, json!("empty_audio")));

    // A bot's turn token is refused: dictation is the person's
    state.self_tools.lock().await.insert("turn-token".into(), SelfScope::default());
    let mut req = audio_post("default", "", "audio/webm", vec![1; 10]);
    req.headers_mut().insert("authorization", "Bearer turn-token".parse().unwrap());
    let (code, _) = call(&app, req).await;
    assert_eq!(code, StatusCode::FORBIDDEN);

    assert_eq!(calls_with("sk-refusals").len(), 1, "only the 25 MB recording went up");
    assert_eq!(calls_with("sk-refusals")[0].4, transcribe::MAX_AUDIO_BYTES as usize);
}
