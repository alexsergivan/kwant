//! Each engine check against fake CLIs (signed in, signed out, broken, hanging) and a fake
//! provider API, so every status and reason is pinned without touching real accounts.

#[path = "common/exe.rs"]
mod exe;

use axum::{
    extract::State,
    http::{HeaderMap, StatusCode},
    routing::get,
    Json, Router,
};
use kwnt::drivers::engines::{probe, probe_within, EngineId, EngineStatus, ProbeEnv};
use serde_json::{json, Value};
use std::path::Path;
use std::time::{Duration, Instant};

fn script(dir: &Path, name: &str, body: &str) -> Option<String> {
    let path = dir.join(name);
    exe::write_executable(&path, format!("#!/bin/sh\n{body}\n"));
    Some(path.to_string_lossy().into_owned())
}

#[tokio::test]
async fn cli_checks_report_each_status_with_a_reason() {
    let tmp = tempfile::tempdir().unwrap();
    let d = tmp.path();
    let claude_in = script(d, "claude-in", r#"case "$1" in
  --version) echo "9.9.9 (Claude Code)" ;;
  auth) echo '{"loggedIn":true,"authMethod":"claude.ai","email":"secret-person@example.com"}' ;;
esac"#);
    let claude_out = script(d, "claude-out", r#"case "$1" in
  --version) echo "9.9.9 (Claude Code)" ;;
  auth) echo '{"loggedIn":false,"authMethod":"none"}'; exit 1 ;;
esac"#);
    let claude_broken = script(d, "claude-broken", r#"case "$1" in
  auth) echo "error: unknown command 'auth' sk-ant-api03-abcdefghijklmnopqrstuvwxyz" >&2; exit 2 ;;
esac"#);
    let claude_hang = script(d, "claude-hang", "sleep 30");
    let codex_in = script(d, "codex-in", r#"case "$1" in
  --version) echo "codex-cli 1.2.3" ;;
  login) echo "Logged in using ChatGPT" ;;
  debug) echo '{"models":[{"slug":"gpt-b","display_name":"GPT B","visibility":"list","priority":2},{"slug":"gpt-a","display_name":"GPT A","visibility":"list","priority":1},{"slug":"secret","visibility":"hide","priority":0}]}' ;;
esac"#);
    // Real codex prints this and still exits 0
    let codex_out = script(d, "codex-out", r#"case "$1" in
  login) echo "Not logged in" ;;
esac"#);
    let grok_out = script(d, "grok-out", r#"case "$1" in
  --version) echo "grok 1.0.62 (1474f9cbdd91)" ;;
  models) printf 'You are not authenticated.\n\nDefault model: grok-4.6\n\nAvailable models:\n  * grok-4.6 (default)\n  - grok-4.5\n' ;;
esac"#);
    let grok_in = script(d, "grok-in", r#"case "$1" in
  models) printf 'Default model: grok-4.6\n\nAvailable models:\n  * grok-4.6 (default)\n  - grok-4.5\n' ;;
esac"#);

    let env = |claude: &Option<String>, codex: &Option<String>, grok: &Option<String>| ProbeEnv {
        claude_bin: claude.clone(),
        codex_bin: codex.clone(),
        grok_bin: grok.clone(),
        ..ProbeEnv::default()
    };

    // Claude
    let p = probe(EngineId::Claude, &env(&claude_in, &None, &None)).await;
    assert_eq!(p.status, EngineStatus::Ready);
    assert_eq!(p.version.as_deref(), Some("9.9.9"));
    assert_eq!(p.detail.as_deref(), Some("Signed in (claude.ai)"));
    assert!(p.models.iter().any(|m| m.id == "opus" && m.default));
    let p = probe(EngineId::Claude, &env(&claude_out, &None, &None)).await;
    assert_eq!(p.status, EngineStatus::SignedOut);
    assert_eq!(p.detail.as_deref(), Some("Not signed in: run `claude auth login`"));
    assert!(p.models.is_empty());
    let p = probe(EngineId::Claude, &env(&claude_broken, &None, &None)).await;
    assert_eq!(p.status, EngineStatus::Error);
    let detail = p.detail.unwrap();
    assert!(detail.contains("unknown command"), "{detail}");
    assert!(!detail.contains("abcdefghijklmnop"), "a secret reached the reason: {detail}");
    let p = probe(EngineId::Claude, &env(&None, &None, &None)).await;
    assert_eq!(p.status, EngineStatus::NotInstalled);

    // A hanging CLI is cut off at the limit, not waited on
    let start = Instant::now();
    let p = probe_within(EngineId::Claude, &env(&claude_hang, &None, &None), Duration::from_secs(1)).await;
    assert!(start.elapsed() < Duration::from_secs(5), "took {:?}", start.elapsed());
    assert_eq!(p.status, EngineStatus::Error);
    assert_eq!(p.detail.as_deref(), Some("Check timed out after 1s"));

    // Codex
    let p = probe(EngineId::Codex, &env(&None, &codex_in, &None)).await;
    assert_eq!(p.status, EngineStatus::Ready);
    assert_eq!(p.version.as_deref(), Some("1.2.3"));
    let ids: Vec<_> = p.models.iter().map(|m| m.id.as_str()).collect();
    assert_eq!(ids, ["gpt-a", "gpt-b"]);
    assert!(p.models[0].default);
    let p = probe(EngineId::Codex, &env(&None, &codex_out, &None)).await;
    assert_eq!(p.status, EngineStatus::SignedOut);
    assert_eq!(p.detail.as_deref(), Some("Not signed in: run `codex login`"));

    // Grok
    let p = probe(EngineId::Grok, &env(&None, &None, &grok_out)).await;
    assert_eq!(p.status, EngineStatus::SignedOut);
    assert_eq!(p.version.as_deref(), Some("1.0.62"));
    assert_eq!(p.detail.as_deref(), Some("Not signed in: run `grok login`"));
    let p = probe(EngineId::Grok, &env(&None, &None, &grok_in)).await;
    assert_eq!(p.status, EngineStatus::Ready);
    assert_eq!(p.models.iter().find(|m| m.default).unwrap().id, "grok-4.6");
    assert_eq!(p.models.len(), 2);
}

async fn models(State(()): State<()>, headers: HeaderMap) -> (StatusCode, Json<Value>) {
    let key = headers
        .get("x-api-key")
        .or(headers.get("authorization"))
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default()
        .trim_start_matches("Bearer ")
        .to_string();
    match key.as_str() {
        "good" => (
            StatusCode::OK,
            Json(json!({"data": [
                {"id": "claude-opus-5-5", "display_name": "Claude Opus 5.5", "created": 2},
                {"id": "gpt-6-sol", "created": 3},
                {"id": "grok-4.6", "created": 4},
                {"id": "text-embedding-3-large", "created": 9}
            ]})),
        ),
        "broke" => (
            StatusCode::TOO_MANY_REQUESTS,
            Json(json!({"error": {"message": "You exceeded your current quota"}})),
        ),
        _ => (
            StatusCode::UNAUTHORIZED,
            Json(json!({"error": {"type": "authentication_error", "message": "invalid x-api-key"}})),
        ),
    }
}

async fn serve(app: Router) -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    url
}

#[tokio::test]
async fn api_keys_are_checked_against_the_free_model_list() {
    let base = serve(
        Router::new()
            .route("/v1/models", get(models))
            .route("/api/tags", get(|| async { Json(json!({"models": [{"name": "llama3.2"}, {"name": "qwen3"}]})) }))
            .route("/api/version", get(|| async { Json(json!({"version": "0.9.1"})) }))
            .with_state(()),
    )
    .await;
    let env = |key: &str| ProbeEnv {
        anthropic_key: Some(key.into()),
        openai_key: Some(key.into()),
        xai_key: Some(key.into()),
        anthropic_base: base.clone(),
        openai_base: base.clone(),
        xai_base: base.clone(),
        ollama_root: base.clone(),
        ..ProbeEnv::default()
    };

    let p = probe(EngineId::AnthropicApi, &env("good")).await;
    assert_eq!(p.status, EngineStatus::Ready);
    assert_eq!(p.models.len(), 1);
    assert_eq!(p.models[0].label, "Claude Opus 5.5");
    let p = probe(EngineId::OpenaiApi, &env("good")).await;
    assert_eq!(p.models.iter().map(|m| m.id.as_str()).collect::<Vec<_>>(), ["gpt-6-sol"]);
    let p = probe(EngineId::XaiApi, &env("good")).await;
    assert_eq!(p.models[0].id, "grok-4.6");

    for id in [EngineId::AnthropicApi, EngineId::OpenaiApi, EngineId::XaiApi] {
        let p = probe(id, &env("sk-wrong-key-123")).await;
        assert_eq!(p.status, EngineStatus::SignedOut, "{id:?}");
        assert_eq!(p.detail.as_deref(), Some("API key rejected (401)"));
        let p = probe(id, &env("broke")).await;
        assert_eq!(p.status, EngineStatus::Error);
        assert_eq!(p.detail.as_deref(), Some("Quota exceeded (429)"));
    }
    let p = probe(EngineId::OpenaiApi, &ProbeEnv::default()).await;
    assert_eq!(p.status, EngineStatus::NotInstalled);
    assert_eq!(p.detail.as_deref(), Some("No API key: add one in Settings"));

    // Ollama: up with models, then down
    let p = probe(EngineId::Ollama, &env("good")).await;
    assert_eq!(p.status, EngineStatus::Ready);
    assert_eq!(p.version.as_deref(), Some("0.9.1"));
    assert_eq!(p.models.len(), 2);
    let down = ProbeEnv { ollama_root: "http://127.0.0.1:9".into(), ..ProbeEnv::default() };
    let p = probe(EngineId::Ollama, &down).await;
    assert_eq!(p.status, EngineStatus::NotInstalled);
    let p = probe(EngineId::Ollama, &ProbeEnv { ollama_installed: true, ..down }).await;
    assert_eq!(p.status, EngineStatus::Error);
    assert!(p.detail.unwrap().contains("ollama serve"));
}
