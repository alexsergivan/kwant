//! Shared harness for the batch 2 B/C/D tests: a real kwnt server on a loopback port and a fake
//! claude CLI on PATH. The fake records every invocation (all arguments) and can call the agent's
//! own tools (`kwnt_tools`) over HTTP the way the real CLI does, so a test drives a whole turn:
//!
//! - `CALLTOOL:<tool>:<base64 json args>` in the prompt: call that tool and reply with its text
//!   (the last marker counts, and each marker runs once: later turns see it again in history).
//! - `REPLYTAG:<tag>`: reply "answer <tag>" (after any tool call).
//! - `QUOTAONCE:<tag>` / `QUOTAALWAYS`: fail with Claude's usage-limit error, once / every time.
//! - `CAPPED:<tag>` / `CAPALWAYS:<tag>`: end at the model-call cap; the continuation doesn't / does too.
//!
//! Each test file that uses it is its own binary, because it changes PATH.
#![allow(dead_code)]

use axum::{
    body::{to_bytes, Body},
    http::{Request, StatusCode},
    Router,
};
use kwnt::{
    broker::PermissionBroker, config::Config, db::Database, router::LayaRouter, server::create_router,
    server::AppState, tools::ToolRunner,
};
use serde_json::{json, Value};
use std::path::PathBuf;
use std::sync::OnceLock;
use std::time::Duration;
use tower::ServiceExt;

const FAKE_CLAUDE: &str = r#"#!/usr/bin/env python3
import base64, json, os, re, sys, time, urllib.request
args = sys.argv[1:]
if args and args[0] == "auth":
    print('{"loggedIn":true}'); sys.exit(0)
prompt = args[-1] if args else ""
out_dir = os.environ["KWNT_TEST_PROMPTS"]
with open(os.path.join(out_dir, "%d-%d" % (os.getpid(), time.time_ns())), "w") as f:
    f.write("\n".join(args))
# QUOTAONCE:<tag>: the first call answers with Claude's usage-limit error (resets in 2 s);
# QUOTAALWAYS: every call does
q = re.findall(r"QUOTAONCE:([A-Za-z0-9_]+)", prompt)
flag = q and os.path.join(os.path.dirname(out_dir), "quota-" + q[-1])
if "QUOTAALWAYS" in prompt or (flag and not os.path.exists(flag)):
    if flag:
        open(flag, "w").close()
    print(json.dumps({"type": "result", "is_error": True, "num_turns": 1,
        "result": "Claude AI usage limit reached|%d" % (int(time.time()) + 2)}))
    sys.exit(1)
# CAPPED:<tag>: end like an API turn that ran out of model calls; the automatic continuation
# answers "continued <tag>" (CAPALWAYS: and runs out again)
cap = re.findall(r"CAP(?:PED|ALWAYS):([A-Za-z0-9_]+)", prompt)
if cap:
    note = "\n\n(Stopped: reached the limit of 25 model calls for MCP tools this turn.)"
    if "[kwnt: your last turn stopped at the limit" in prompt:
        text = "continued " + cap[-1] + (note if "CAPALWAYS" in prompt else "")
    else:
        text = "partial " + cap[-1] + note
    print(json.dumps({"type": "result", "is_error": False, "num_turns": 1, "result": text}))
    sys.exit(0)
reply = []
m = ([None] + list(re.finditer(r"CALLTOOL:([a-z_]+):([A-Za-z0-9+/=]+)", prompt)))[-1]
done = None
if m:
    import hashlib
    done = os.path.join(os.path.dirname(out_dir), "called-" + hashlib.sha1(m.group(0).encode()).hexdigest())
    if os.path.exists(done):
        m = None
if m and "--mcp-config" in args:
    open(done, "w").close()
    cfg = json.load(open(args[args.index("--mcp-config") + 1]))
    srv = cfg["mcpServers"]["kwnt_tools"]
    body = json.dumps({"jsonrpc": "2.0", "id": 1, "method": "tools/call",
        "params": {"name": m.group(1), "arguments": json.loads(base64.b64decode(m.group(2)))}}).encode()
    req = urllib.request.Request(srv["url"], data=body, method="POST",
        headers={"content-type": "application/json", **srv.get("headers", {})})
    try:
        res = json.load(urllib.request.urlopen(req, timeout=600))
        parts = res["result"]["content"]
        reply.append("TOOL[" + ";".join(p.get("text", p.get("type", "")) for p in parts) + "]")
        if res["result"].get("isError"):
            reply.append("TOOLERROR")
    except Exception as e:
        reply.append("TOOLFAILED " + type(e).__name__)
t = re.findall(r"REPLYTAG:([A-Za-z0-9_]+)", prompt)
reply.append("answer " + (t[-1] if t else "none"))
print(json.dumps({"type": "result", "is_error": False, "num_turns": 1, "result": " ".join(reply)}))
"#;

pub fn prompts_dir() -> &'static PathBuf {
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

/// Every CLI invocation (all its arguments, one per line) whose text contains `marker`.
pub fn invocations_with(marker: &str) -> Vec<String> {
    std::fs::read_dir(prompts_dir())
        .unwrap()
        .filter_map(|e| std::fs::read_to_string(e.unwrap().path()).ok())
        .filter(|p| p.contains(marker))
        .collect()
}

/// A prompt marker that makes the fake CLI call `tool` with `args`.
pub fn call_tool_marker(tool: &str, args: Value) -> String {
    use base64::Engine;
    format!("CALLTOOL:{tool}:{}", base64::engine::general_purpose::STANDARD.encode(args.to_string()))
}

pub async fn call(app: &Router, method: &str, uri: &str, body: Option<Value>) -> (StatusCode, Value) {
    call_with(app, method, uri, body, &[]).await
}

pub async fn call_with(
    app: &Router,
    method: &str,
    uri: &str,
    body: Option<Value>,
    headers: &[(&str, &str)],
) -> (StatusCode, Value) {
    let mut b = Request::builder().method(method).uri(uri).header("content-type", "application/json");
    for (k, v) in headers {
        b = b.header(*k, *v);
    }
    let resp = app
        .clone()
        .oneshot(b.body(body.map(|v| Body::from(v.to_string())).unwrap_or_else(Body::empty)).unwrap())
        .await
        .unwrap();
    let s = resp.status();
    let bytes = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    (s, serde_json::from_slice(&bytes).unwrap_or(Value::Null))
}

/// A request as a remote client without the API token: every guarded route must refuse it.
pub async fn call_remote(app: &Router, method: &str, uri: &str) -> StatusCode {
    call_with(app, method, uri, Some(json!({})), &[("host", "kwnt.example.com")]).await.0
}

pub struct Env {
    pub app: Router,
    pub state: AppState,
    /// Two agents, both on the fake claude.
    pub a: String,
    pub b: String,
    pub tmp: tempfile::TempDir,
}

/// A server listening on a loopback port (the CLI reaches its tools there).
pub async fn setup() -> Env {
    prompts_dir();
    let tmp = tempfile::tempdir().unwrap();
    let data = tmp.path().join("data");
    let db = Database::init(&data).await.unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let config = Config {
        data_dir: data.clone(),
        host: "127.0.0.1".into(),
        port,
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
    let served = app.clone().into_make_service_with_connect_info::<std::net::SocketAddr>();
    tokio::spawn(async move {
        let _ = axum::serve(listener, served).await;
    });
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
    Env { app, state, a: ids[0].clone(), b: ids[1].clone(), tmp }
}

pub async fn wait_for<F, Fut>(what: &str, mut check: F)
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

/// Messages of a chat (channel, direct chat by kwant id, or thread).
pub async fn messages(env: &Env, context_id: &str) -> Vec<kwnt::db::models::Message> {
    env.state.db.list_messages(context_id, 100).await.unwrap()
}

/// Waits until the chat has a kwant message containing `needle`, and returns it.
pub async fn wait_reply(env: &Env, context_id: &str, needle: &str) -> kwnt::db::models::Message {
    for _ in 0..300 {
        if let Some(m) =
            messages(env, context_id).await.into_iter().find(|m| m.sender_role == "kwant" && m.text.contains(needle))
        {
            return m;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    panic!("timed out waiting for a reply with {needle}");
}

/// Sends a user message to `kwant`'s direct chat.
pub async fn send_direct(env: &Env, kwant: &str, prompt: &str, extra: Value) -> Value {
    let mut body = json!({"prompt": prompt, "sender_id": "user", "kwant_id": kwant});
    if let (Some(b), Some(e)) = (body.as_object_mut(), extra.as_object()) {
        for (k, v) in e {
            b.insert(k.clone(), v.clone());
        }
    }
    let (s, v) = call(&env.app, "POST", "/api/chat/send", Some(body)).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    v
}
