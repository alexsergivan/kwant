//! Shared harness for the batch 2 B/C/D tests: a real kwnt server on a loopback port and a fake
//! claude CLI on PATH. The fake records every invocation (all arguments) and can call the agent's
//! own tools (`kwnt_tools`) over HTTP the way the real CLI does, so a test drives a whole turn:
//!
//! - `CALLTOOL:<tool>:<base64 json args>` in the prompt: call that tool and reply with its text
//!   (the last marker counts, and each marker runs once: later turns see it again in history).
//! - `REPLYTAG:<tag>`: reply "answer <tag>" (after any tool call).
//! - `REPLYTEXT:<base64 text>`: reply exactly that text (say, a markdown table), and nothing else.
//! - `QUOTAONCE:<tag>` / `QUOTAALWAYS`: fail with Claude's usage-limit error, once / every time;
//!   it resets in 2 s, or in `QUOTARESET:<secs>`.
//! - A no-tools turn (`--tools ""`) first prints claude's init line with no tools.
//! - `CAPPED:<tag>` / `CAPALWAYS:<tag>`: end at the model-call cap; the continuation doesn't / does too.
//! - `SLEEPONCE:<tag>:<ms>`: wait that long, printing nothing, before answering; once per tag (a
//!   slow reply, or a stall when it waits past the stall limit).
//! - `FAILTURN`: die with no output. `APIERROR:<tag>` / `APIERRORPLAIN:<tag>`: end with "API Error:
//!   500 test stub" as claude's error result / as plain text, once per tag.
//! - `FAILSTEP:<tag>` / `BLOCKSTEP:<tag>` in the current request (not the history): run a Bash
//!   tool step that fails, then end on a plan ("I'll retry the render…") / on a `Blocked:` line.
//! - `STREAMLINES:<base64 text>`: print that text (claude `stream-json` lines: an init line, tool
//!   calls and their results) before the reply; once per marker.
//!
//! Each test file that uses it is its own binary, because it changes PATH.
#![allow(dead_code)]

mod exe;

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
# FAILTURN: the CLI dies with no output (exit 3), so the turn fails
if "FAILTURN" in prompt:
    sys.exit(3)
# APIERROR:<tag>: the API call fails, and claude ends with the error as its result (exit 1);
# APIERRORPLAIN:<tag>: the same error as plain text, not JSON. Once per tag (later turns see it
# in history).
ae = re.findall(r"APIERROR(PLAIN)?:([A-Za-z0-9_]+)", prompt)
ae_flag = ae and os.path.join(os.path.dirname(out_dir), "apierror-" + ae[-1][1])
if ae and not os.path.exists(ae_flag):
    open(ae_flag, "w").close()
    if ae[-1][0]:
        print("API Error: 500 test stub"); sys.exit(1)
    print(json.dumps({"type": "result", "subtype": "success", "is_error": True, "num_turns": 1,
        "result": "API Error: 500 test stub"}))
    sys.exit(1)
# SLEEPONCE:<tag>:<ms>: wait silently that long first, once per tag
so = re.findall(r"SLEEPONCE:([A-Za-z0-9_]+):([0-9]+)", prompt)
so_flag = so and os.path.join(os.path.dirname(out_dir), "slept-" + so[-1][0])
if so and not os.path.exists(so_flag):
    open(so_flag, "w").close()
    time.sleep(int(so[-1][1]) / 1000.0)
# A no-tools turn (--tools ""): say so first, like claude's init line, or kwnt stops the run
if "--tools" in args:
    print(json.dumps({"type": "system", "subtype": "init", "tools": [], "mcp_servers": [],
        "skills": [], "slash_commands": []}))
# QUOTAONCE:<tag>: the first call answers with Claude's usage-limit error (resets in 2 s);
# QUOTAALWAYS: every call does. QUOTARESET:<secs> sets a later reset.
reset = int((re.findall(r"QUOTARESET:([0-9]+)", prompt) or ["2"])[-1])
q = re.findall(r"QUOTAONCE:([A-Za-z0-9_]+)", prompt)
flag = q and os.path.join(os.path.dirname(out_dir), "quota-" + q[-1])
if "QUOTAALWAYS" in prompt or (flag and not os.path.exists(flag)):
    if flag:
        open(flag, "w").close()
    print(json.dumps({"type": "result", "is_error": True, "num_turns": 1,
        "result": "Claude AI usage limit reached|%d" % (int(time.time()) + reset)}))
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
# FAILSTEP:<tag> / BLOCKSTEP:<tag> in the current request: a Bash step that fails, then a reply
# that ends on a plan / on a Blocked: line
req = prompt.rsplit("Current User Request:\n", 1)[-1]
fs = re.findall(r"(FAIL|BLOCK)STEP:([A-Za-z0-9_]+)", req)
if fs:
    kind, tag = fs[-1]
    tid = "toolu_" + tag
    print(json.dumps({"type": "assistant", "message": {"id": "msg_" + tag, "content": [
        {"type": "tool_use", "id": tid, "name": "Bash", "input": {"command": "render.sh", "description": "render the intro"}}]}}))
    print(json.dumps({"type": "user", "message": {"content": [
        {"type": "tool_result", "tool_use_id": tid, "is_error": True, "content": "Permission to use Bash has been denied."}]}}))
    end = ("I'll retry the render with a smaller size next." if kind == "FAIL"
        else "Blocked: terminal commands are off for this bot. Allow Terminal for this team, then say continue.")
    print(json.dumps({"type": "result", "is_error": False, "num_turns": 2,
        "result": "The render command was refused (" + tag + ").\n\n" + end}))
    sys.exit(0)
# STREAMLINES:<base64>: print those stream-json lines first (tool calls and results), once per marker
sl = re.findall(r"STREAMLINES:([A-Za-z0-9+/=]+)", prompt)
if sl:
    import hashlib
    sl_flag = os.path.join(os.path.dirname(out_dir), "streamed-" + hashlib.sha1(sl[-1].encode()).hexdigest())
    if not os.path.exists(sl_flag):
        open(sl_flag, "w").close()
        sys.stdout.write(base64.b64decode(sl[-1]).decode())
        sys.stdout.flush()
# REPLYTEXT:<base64>: reply exactly that text
rt = re.findall(r"REPLYTEXT:([A-Za-z0-9+/=]+)", prompt)
if rt:
    print(json.dumps({"type": "result", "is_error": False, "num_turns": 1,
        "result": base64.b64decode(rt[-1]).decode()}))
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

/// Dev mode's secrets and package checks never reach GitHub's releases or a real registry from a
/// test: both point at a closed port, so a poll's scan reads `not_run`.
pub fn no_network_scans() {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| {
        std::env::set_var("KWANT_DEV_GITLEAKS_URL", "http://127.0.0.1:9");
        std::env::set_var("KWANT_DEV_REGISTRIES_URL", "http://127.0.0.1:9");
    });
}

pub fn prompts_dir() -> &'static PathBuf {
    static DIR: OnceLock<PathBuf> = OnceLock::new();
    DIR.get_or_init(|| {
        let root = tempfile::tempdir().unwrap().keep();
        let bin = root.join("bin");
        let prompts = root.join("prompts");
        std::fs::create_dir_all(&bin).unwrap();
        std::fs::create_dir_all(&prompts).unwrap();
        let fake = bin.join("claude");
        exe::write_executable(&fake, FAKE_CLAUDE);
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

/// A prompt marker that makes the fake CLI print these `stream-json` lines before its reply.
pub fn stream_lines_marker(lines: &[Value]) -> String {
    use base64::Engine;
    let text: String = lines.iter().map(|l| format!("{l}\n")).collect();
    format!("STREAMLINES:{}", base64::engine::general_purpose::STANDARD.encode(text))
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
    setup_with(|_| {}).await
}

/// [`setup`], with a chance to change the state (say, point it at a fake service) before the
/// router is built.
pub async fn setup_with(change: impl FnOnce(&mut AppState)) -> Env {
    prompts_dir();
    no_network_scans();
    let tmp = tempfile::tempdir().unwrap();
    let data = tmp.path().join("data");
    let db = Database::init(&data).await.unwrap();
    // These chains predate keep-going turns; a stalled lead isn't nudged here.
    db.set_setting("keep_going_turns", "0").await.unwrap();
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
    let mut state = AppState::new(
        config.clone(),
        db,
        ToolRunner::new(data.join("workspace")),
        PermissionBroker::new(),
        LayaRouter::new(config.laya_url.clone()),
    );
    change(&mut state);
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
