//! Custom API accounts end to end, against a fake server that speaks both the OpenAI chat and the
//! Anthropic Messages APIs under Agent Maestro's paths (`/api/openai/v1`, `/api/anthropic/v1`):
//! the model list, a streamed turn in a space other than Default with "ran on <account>", a
//! rejected key and a server that isn't running.

use axum::{
    body::{to_bytes, Body},
    extract::State,
    http::{HeaderMap, Request, StatusCode},
    response::{IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use kwnt::{
    broker::PermissionBroker,
    config::Config,
    db::Database,
    drivers::RuntimeEvent,
    router::LayaRouter,
    server::{create_router, AppState},
    tools::ToolRunner,
};
use serde_json::{json, Value};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tower::ServiceExt;

const KEY: &str = "sk-custom-SECRET-0123456789";
const HOST_KEY: &str = "sk-host-key-must-not-reach-custom";

type Seen = Arc<Mutex<Vec<(String, HeaderMap, Value)>>>;

#[derive(Clone)]
struct Fake {
    seen: Seen,
    /// Whether `GET …/models` answers (Agent Maestro's OpenAI side may not list).
    lists: bool,
}

fn authorized(headers: &HeaderMap, anthropic: bool) -> bool {
    let got = if anthropic {
        headers.get("x-api-key").and_then(|v| v.to_str().ok()).map(str::to_string)
    } else {
        headers
            .get("authorization")
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.strip_prefix("Bearer "))
            .map(str::to_string)
    };
    got.as_deref() == Some(KEY)
}

fn denied() -> Response {
    (StatusCode::UNAUTHORIZED, Json(json!({"error": {"message": "invalid api key"}}))).into_response()
}

fn sse(events: Vec<String>) -> Response {
    let body: String = events.into_iter().map(|e| format!("{e}\n\n")).collect();
    ([("content-type", "text/event-stream")], body).into_response()
}

async fn models(fake: Fake, headers: HeaderMap, anthropic: bool) -> Response {
    if !fake.lists {
        return StatusCode::NOT_FOUND.into_response();
    }
    if !authorized(&headers, anthropic) {
        return denied();
    }
    if anthropic {
        Json(json!({"data": [{"id": "claude-sonnet-4", "display_name": "Claude Sonnet 4"}]})).into_response()
    } else {
        Json(json!({"object": "list", "data": [{"id": "gpt-4.1"}, {"id": "claude-sonnet-4"}]})).into_response()
    }
}

/// A fake model server. The reply names the model and the API it came through.
async fn fake_server(lists: bool) -> (String, Seen) {
    let seen: Seen = Arc::default();
    let fake = Fake { seen: seen.clone(), lists };
    let app = Router::new()
        .route(
            "/api/openai/v1/models",
            get(|State(f): State<Fake>, h: HeaderMap| async move { models(f, h, false).await }),
        )
        .route(
            "/api/anthropic/v1/models",
            get(|State(f): State<Fake>, h: HeaderMap| async move { models(f, h, true).await }),
        )
        .route(
            "/api/openai/v1/chat/completions",
            post(|State(f): State<Fake>, h: HeaderMap, Json(body): Json<Value>| async move {
                f.seen.lock().unwrap().push(("openai".into(), h.clone(), body.clone()));
                if !authorized(&h, false) {
                    return denied();
                }
                let model = body["model"].as_str().unwrap_or("?").to_string();
                if body["stream"] != true {
                    return Json(json!({"choices": [{"message": {"content": format!("unstreamed {model}")}}]}))
                        .into_response();
                }
                let piece = |t: &str| format!("data: {}", json!({"choices": [{"delta": {"content": t}}]}));
                sse(vec![
                    piece("Hello "),
                    piece(&format!("from {model} ")),
                    piece("over OpenAI"),
                    format!("data: {}", json!({"choices": [], "usage": {"total_tokens": 9}})),
                    "data: [DONE]".into(),
                ])
            }),
        )
        .route(
            "/api/anthropic/v1/messages",
            post(|State(f): State<Fake>, h: HeaderMap, Json(body): Json<Value>| async move {
                f.seen.lock().unwrap().push(("anthropic".into(), h.clone(), body.clone()));
                if !authorized(&h, true) {
                    return denied();
                }
                let model = body["model"].as_str().unwrap_or("?").to_string();
                let delta = |t: &str| {
                    format!(
                        "event: content_block_delta\ndata: {}",
                        json!({"type": "content_block_delta", "index": 0, "delta": {"type": "text_delta", "text": t}})
                    )
                };
                sse(vec![
                    format!("event: message_start\ndata: {}", json!({"type": "message_start", "message": {"usage": {"input_tokens": 5}}})),
                    delta("Hello "),
                    delta(&format!("from {model} ")),
                    delta("over Anthropic"),
                    format!("event: message_delta\ndata: {}", json!({"type": "message_delta", "usage": {"output_tokens": 6}})),
                    format!("event: message_stop\ndata: {}", json!({"type": "message_stop"})),
                ])
            }),
        )
        .with_state(fake);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let root = format!("http://{}", listener.local_addr().unwrap());
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    (root, seen)
}

/// A port nothing listens on.
async fn closed_port() -> u16 {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    listener.local_addr().unwrap().port()
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
    (s, serde_json::from_slice(&bytes).unwrap_or_else(|_| Value::String(String::from_utf8_lossy(&bytes).into())))
}

struct Env {
    app: Router,
    state: AppState,
    _tmp: tempfile::TempDir,
}

async fn setup() -> Env {
    // The host's own keys must never be used for a Custom API turn
    std::env::set_var("OPENAI_API_KEY", HOST_KEY);
    std::env::set_var("ANTHROPIC_API_KEY", HOST_KEY);
    let tmp = tempfile::tempdir().unwrap();
    let data = tmp.path().join("data");
    let db = Database::init(&data).await.unwrap();
    // Laya is unreachable: routing falls back to the bot's pinned model
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
    Env { app: create_router(state.clone()), state, _tmp: tmp }
}

impl Env {
    async fn space(&self, name: &str) -> String {
        let (s, v) = call(
            &self.app,
            "POST",
            "/api/spaces",
            Some(json!({"name": name, "icon": "🧪", "this_machine_enabled": false})),
        )
        .await;
        assert_eq!(s, StatusCode::CREATED, "{v}");
        v["id"].as_str().unwrap().to_string()
    }

    async fn account(&self, space: &str, body: Value) -> (StatusCode, Value) {
        call(&self.app, "POST", &format!("/api/profiles?space={space}"), Some(body)).await
    }

    async fn bot(&self, space: &str, name: &str, model: &str) -> String {
        let (s, v) = call(
            &self.app,
            "POST",
            &format!("/api/kwants?space={space}"),
            Some(json!({
                "name": name,
                "title": "Tester",
                "persona_prompt": "Test persona",
                "avatar_sprite": "s",
                "driver_type": "api",
                "preferred_model": model,
            })),
        )
        .await;
        assert!(s.is_success(), "{s} {v}");
        v["id"].as_str().unwrap().to_string()
    }

    /// Sends a prompt and collects the turn's events up to `turn_ended`.
    async fn turn(&self, kwant_id: &str) -> Vec<RuntimeEvent> {
        let mut rx = self.state.event_sender.subscribe();
        let (s, v) = call(
            &self.app,
            "POST",
            "/api/chat/send",
            Some(json!({"kwant_id": kwant_id, "prompt": "hi", "sender_id": "user"})),
        )
        .await;
        assert_eq!(s, StatusCode::OK, "{v}");
        let mut events = Vec::new();
        tokio::time::timeout(Duration::from_secs(60), async {
            while let Ok(e) = rx.recv().await {
                let end = matches!(&e, RuntimeEvent::TurnEnded { kwant_id: k, .. } if k == kwant_id);
                events.push(e.unscoped().clone());
                if end {
                    break;
                }
            }
        })
        .await
        .expect("turn did not end");
        events
    }

    /// The bot's saved reply and its `meta.ran_on`.
    async fn reply(&self, kwant_id: &str) -> (String, Value) {
        let saved = self.state.db.list_messages(kwant_id, 10).await.unwrap();
        let m = saved.iter().rev().find(|m| m.sender_role == "kwant").expect("a saved reply");
        let meta: Value = m.meta.as_deref().map(|s| serde_json::from_str(s).unwrap()).unwrap_or(Value::Null);
        (m.text.clone(), meta["ran_on"].clone())
    }
}

fn streamed(events: &[RuntimeEvent]) -> (String, usize) {
    let deltas: Vec<&str> = events
        .iter()
        .filter_map(|e| match e {
            RuntimeEvent::TextDelta(t) => Some(t.as_str()),
            _ => None,
        })
        .collect();
    (deltas.concat(), deltas.len())
}

fn thinking(events: &[RuntimeEvent]) -> String {
    events
        .iter()
        .filter_map(|e| match e {
            RuntimeEvent::ThinkingDelta(t) => Some(t.as_str()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("\n")
}

#[tokio::test]
async fn openai_compatible_account_lists_models_and_streams_a_turn_in_its_space() {
    let env = setup().await;
    let (root, seen) = fake_server(true).await;
    let space = env.space("Maestro space").await;

    let (s, acct) = env
        .account(
            &space,
            json!({
                "engine": "custom", "kind": "custom_api", "name": "Copilot via Maestro",
                "protocol": "openai-chat",
                // A pasted endpoint URL is cut back to the API root
                "base_url": format!("{root}/api/openai/v1/chat/completions"),
                "api_key": KEY, "model": "gpt-4.1",
            }),
        )
        .await;
    assert_eq!(s, StatusCode::CREATED, "{acct}");
    let text = acct.to_string();
    assert!(!text.contains(KEY), "the key never comes back: {text}");
    assert_eq!(acct["has_key"], true);
    assert_eq!(acct["signed_in"], true);
    let custom = &acct["custom"];
    assert_eq!(custom["base_url"], format!("{root}/api/openai/v1"));
    assert_eq!(custom["protocol"], "openai-chat");
    assert_eq!(custom["status"], "ready", "{custom}");
    assert_eq!(custom["detail"], "Connected · 2 models");
    assert_eq!(custom["supports_tools"], true);
    let ids: Vec<&str> = custom["models"].as_array().unwrap().iter().map(|m| m["id"].as_str().unwrap()).collect();
    assert_eq!(ids, ["gpt-4.1", "claude-sonnet-4"]);
    assert_eq!(custom["models"][0]["default"], true, "the account's own model is its default");

    // Scoped to its space: Default doesn't list it
    let (_, default_list) = call(&env.app, "GET", "/api/profiles", None).await;
    assert!(!default_list.to_string().contains("Copilot via Maestro"));

    let kid = env.bot(&space, "Quill", "custom").await;
    let events = env.turn(&kid).await;
    let (text, pieces) = streamed(&events);
    assert_eq!(text, "Hello from gpt-4.1 over OpenAI");
    assert!(pieces >= 3, "streamed piece by piece, not as one block: {pieces}");
    let notice = thinking(&events);
    assert!(notice.contains("Running on Custom API · Copilot via Maestro · gpt-4.1"), "{notice}");

    let (saved, ran_on) = env.reply(&kid).await;
    assert_eq!(saved, "Hello from gpt-4.1 over OpenAI");
    assert_eq!(ran_on["account"], "Copilot via Maestro", "{ran_on}");
    assert_eq!(ran_on["engine"], "custom_api");
    assert_eq!(ran_on["engine_name"], "Custom API");
    assert_eq!(ran_on["family"], "custom");
    assert_eq!(ran_on["model"], "gpt-4.1");
    assert_eq!(ran_on["profile_id"], acct["id"]);

    let seen = seen.lock().unwrap();
    let (api, headers, body) = seen.last().unwrap();
    assert_eq!(api, "openai");
    assert_eq!(headers["authorization"], format!("Bearer {KEY}"));
    assert_eq!(body["stream"], true);
    assert_eq!(body["model"], "gpt-4.1");
    assert!(seen.iter().all(|(_, h, _)| !format!("{h:?}").contains(HOST_KEY)), "the host's key leaked");
}

#[tokio::test]
async fn anthropic_compatible_account_streams_the_bots_pinned_model() {
    let env = setup().await;
    let (root, seen) = fake_server(true).await;
    let space = env.space("Claude via Maestro").await;

    // No model on the account: the bot pins one; the base gets its /v1
    let (s, acct) = env
        .account(
            &space,
            json!({
                "engine": "custom", "kind": "custom_api", "name": "Maestro Claude",
                "protocol": "anthropic-messages", "base_url": format!("{root}/api/anthropic"),
                "api_key": KEY,
            }),
        )
        .await;
    assert_eq!(s, StatusCode::CREATED, "{acct}");
    let custom = &acct["custom"];
    assert_eq!(custom["base_url"], format!("{root}/api/anthropic/v1"));
    assert_eq!(custom["status"], "ready", "{custom}");
    assert_eq!(custom["supports_tools"], false);
    assert_eq!(custom["models"][0]["label"], "Claude Sonnet 4");

    let kid = env.bot(&space, "Sage", "custom:claude-sonnet-4").await;
    let events = env.turn(&kid).await;
    let (text, pieces) = streamed(&events);
    assert_eq!(text, "Hello from claude-sonnet-4 over Anthropic");
    assert!(pieces >= 3, "{pieces}");
    let (saved, ran_on) = env.reply(&kid).await;
    assert_eq!(saved, text);
    assert_eq!(ran_on["account"], "Maestro Claude", "{ran_on}");
    assert_eq!(ran_on["model"], "claude-sonnet-4");

    let seen = seen.lock().unwrap();
    let (api, headers, body) = seen.last().unwrap();
    assert_eq!(api, "anthropic");
    assert_eq!(headers["x-api-key"], KEY);
    assert!(headers.contains_key("anthropic-version"));
    assert!(!headers.contains_key("authorization"));
    assert_eq!(body["stream"], true);
    assert_eq!(body["model"], "claude-sonnet-4");
}

#[tokio::test]
async fn a_rejected_key_is_shown_on_the_account_and_refuses_the_turn() {
    let env = setup().await;
    let (root, seen) = fake_server(true).await;
    let space = env.space("Bad key").await;
    let wrong = "sk-wrong-WRONGWRONG-987654";
    let (s, acct) = env
        .account(
            &space,
            json!({
                "engine": "custom", "kind": "custom_api", "name": "OpenRouter",
                "protocol": "openai-chat", "base_url": format!("{root}/api/openai/v1"),
                "api_key": wrong, "model": "gpt-4.1",
            }),
        )
        .await;
    assert_eq!(s, StatusCode::CREATED, "{acct}");
    assert_eq!(acct["custom"]["status"], "key_rejected", "{acct}");
    assert_eq!(acct["signed_in"], false);
    assert!(!acct.to_string().contains(wrong));

    let kid = env.bot(&space, "Rex", "custom").await;
    let events = env.turn(&kid).await;
    let (text, _) = streamed(&events);
    assert!(text.contains("OpenRouter"), "{text}");
    assert!(text.contains("key was rejected"), "{text}");
    assert!(!text.contains(wrong));
    // Never retried on the host's keys or CLIs
    assert!(seen.lock().unwrap().iter().all(|(_, h, _)| !format!("{h:?}").contains(HOST_KEY)));

    // Fixed key: checked again at once, and ready
    let id = acct["id"].as_str().unwrap();
    let (s, _) = call(&env.app, "PUT", &format!("/api/profiles/{id}/key?space={space}"), Some(json!({"api_key": KEY}))).await;
    assert_eq!(s, StatusCode::NO_CONTENT);
    let (s, v) = call(&env.app, "POST", &format!("/api/profiles/{id}/check?space={space}"), None).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(v["custom"]["status"], "ready", "{v}");
    let events = env.turn(&kid).await;
    assert_eq!(streamed(&events).0, "Hello from gpt-4.1 over OpenAI");
}

#[tokio::test]
async fn a_server_that_is_not_running_says_so() {
    let env = setup().await;
    let port = closed_port().await;
    let space = env.space("Offline").await;
    let (s, acct) = env
        .account(
            &space,
            json!({
                "engine": "custom", "kind": "custom_api", "name": "LM Studio",
                "protocol": "openai-chat", "base_url": format!("http://127.0.0.1:{port}/v1"),
                "model": "qwen2.5-7b",
            }),
        )
        .await;
    assert_eq!(s, StatusCode::CREATED, "{acct}");
    let expected = format!("Nothing is answering at http://127.0.0.1:{port}/v1. Is Agent Maestro / the server running?");
    assert_eq!(acct["custom"]["status"], "unreachable", "{acct}");
    assert_eq!(acct["custom"]["detail"], expected.as_str());
    assert_eq!(acct["has_key"], false);

    let kid = env.bot(&space, "Lumen", "custom").await;
    let events = env.turn(&kid).await;
    let (text, _) = streamed(&events);
    assert!(text.contains(&expected), "{text}");
}

#[tokio::test]
async fn a_server_without_a_model_list_runs_the_model_entered_by_hand() {
    let env = setup().await;
    let (root, _) = fake_server(false).await;
    let space = env.space("No list").await;
    let base = json!({
        "engine": "custom", "kind": "custom_api", "name": "Plain",
        "protocol": "openai-chat", "base_url": format!("{root}/api/openai/v1"), "api_key": KEY,
    });
    let (_, acct) = env.account(&space, base.clone()).await;
    assert_eq!(acct["custom"]["status"], "error", "no list and no model: {acct}");
    let id = acct["id"].as_str().unwrap();
    let (s, acct) = call(&env.app, "PATCH", &format!("/api/profiles/{id}?space={space}"), Some(json!({"model": "gpt-4.1"}))).await;
    assert_eq!(s, StatusCode::OK, "{acct}");
    assert_eq!(acct["custom"]["status"], "ready", "{acct}");
    assert_eq!(acct["custom"]["model"], "gpt-4.1");
    let kid = env.bot(&space, "Plainly", "custom").await;
    assert_eq!(streamed(&env.turn(&kid).await).0, "Hello from gpt-4.1 over OpenAI");
}

#[tokio::test]
async fn a_space_without_a_custom_account_never_falls_back_to_the_host() {
    let env = setup().await;
    let space = env.space("Empty").await;
    let kid = env.bot(&space, "Nemo", "custom").await;
    let events = env.turn(&kid).await;
    let (text, _) = streamed(&events);
    assert!(text.contains("No Custom API account is on in this space"), "{text}");
}

#[tokio::test]
async fn bad_account_bodies_are_refused() {
    let env = setup().await;
    let space = env.space("Validation").await;
    let ok = json!({"engine": "custom", "kind": "custom_api", "name": "X", "protocol": "openai-chat", "base_url": "http://127.0.0.1:1/v1"});
    let with = |k: &str, v: Value| {
        let mut b = ok.clone();
        b[k] = v;
        b
    };
    for (body, why) in [
        (with("kind", json!("api_key")), "custom engine with another kind"),
        (json!({"engine": "claude", "kind": "custom_api", "name": "Y"}), "custom kind on another engine"),
        (with("protocol", json!("grpc")), "unknown protocol"),
        (with("base_url", json!("ftp://example.com/v1")), "not http"),
        (with("base_url", json!("http://user:pass@example.com/v1")), "credentials in the URL"),
        (with("base_url", json!("http://example.com/v1?key=abc")), "query in the URL"),
        (with("model", json!("has space")), "model with whitespace"),
        (with("api_key", json!("short")), "too-short key"),
        (json!({"engine": "openai", "kind": "api_key", "name": "Z"}), "unknown engine"),
        (json!({"engine": "claude", "kind": "api_key", "name": "Z", "base_url": "http://x/v1"}), "server fields on a key account"),
    ] {
        let (s, v) = env.account(&space, body).await;
        assert_eq!(s, StatusCode::BAD_REQUEST, "{why}: {v}");
    }
    let mut no_proto = ok.clone();
    no_proto.as_object_mut().unwrap().remove("protocol");
    assert_eq!(env.account(&space, no_proto).await.0, StatusCode::BAD_REQUEST);
    // Server fields can't be patched onto another kind of account
    let (_, list) = call(&env.app, "GET", &format!("/api/profiles?space={space}"), None).await;
    let system = list.as_array().unwrap().iter().find(|p| p["kind"] == "system").unwrap()["id"].as_str().unwrap().to_string();
    let (s, _) = call(&env.app, "PATCH", &format!("/api/profiles/{system}?space={space}"), Some(json!({"base_url": "http://x/v1"}))).await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
    let (s, _) = call(&env.app, "POST", &format!("/api/profiles/{system}/check?space={space}"), None).await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn deleting_an_account_removes_its_server_and_key() {
    let env = setup().await;
    let space = env.space("Delete").await;
    let (_, acct) = env
        .account(
            &space,
            json!({"engine": "custom", "kind": "custom_api", "name": "Gone", "protocol": "openai-chat",
                   "base_url": "http://127.0.0.1:1/v1", "api_key": KEY, "model": "m"}),
        )
        .await;
    let id = acct["id"].as_str().unwrap().to_string();
    assert!(env.state.db.get_profile_endpoint(&id).await.unwrap().is_some());
    let (s, _) = call(&env.app, "DELETE", &format!("/api/profiles/{id}?space={space}"), None).await;
    assert_eq!(s, StatusCode::NO_CONTENT);
    assert!(env.state.db.get_profile_endpoint(&id).await.unwrap().is_none());
    assert!(env.state.db.get_profile_key(&id).await.unwrap().is_none());
}
