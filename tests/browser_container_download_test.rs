//! 20g: a vm-mode bot's download from kwant's own browser container lands in the bot's own
//! `downloads/`, against real Chromium in the `kwnt-browser` image (`BROWSER_IMAGE`). Run with
//! `cargo test --test browser_container_download_test -- --ignored` on a host with Docker and that
//! image. Ignored by default so CI and ordinary test runs never start a container.
//!
//! The container can't reach a server on the host (its network stops at the host firewall), so the
//! pages and files are served from inside the container itself with its own `socat`, on loopback
//! ports only Chrome there can see.

use axum::{
    body::{to_bytes, Body},
    http::{Request, StatusCode},
    Router,
};
use kwnt::{
    broker::PermissionBroker,
    config::Config,
    db::{spaces::SpaceId, Database},
    router::LayaRouter,
    sandbox::service::managed_container_name,
    server::{
        continuity::SelfScope,
        create_router, mcp_runtime,
        routes::{get_or_create_browser_controller, resolve_sandbox_target_info},
        AppState,
    },
    tools::ToolRunner,
};
use serde_json::{json, Value};
use std::io::Write;
use std::process::{Command, Stdio};
use std::time::Duration;
use tower::ServiceExt;

/// 300 KB of bytes no text encoding would leave unchanged, to prove the copy is byte-exact.
fn test_bytes() -> Vec<u8> {
    let mut x: u32 = 0x2545_f491;
    (0..300 * 1024)
        .map(|_| {
            x ^= x << 13;
            x ^= x >> 17;
            x ^= x << 5;
            (x >> 24) as u8
        })
        .collect()
}

/// 450 MB, past the container's 400 MB cap, announced up front in `Content-Length`.
const BIG_BYTES: u64 = 450 * 1024 * 1024;

const PAGE: &str = "<!doctype html><title>dl</title><body>\
<button onclick=\"location.href='http://127.0.0.1:8000/'\">Get the file</button>\
<button onclick=\"location.href='http://127.0.0.1:8002/'\">Get the big file</button>";

fn docker(args: &[&str]) -> Option<String> {
    let out = Command::new("docker").args(args).output().ok()?;
    out.status.success().then(|| String::from_utf8_lossy(&out.stdout).trim().to_string())
}

fn docker_stdin(args: &[&str], input: &[u8]) {
    let mut child = Command::new("docker").args(args).stdin(Stdio::piped()).spawn().expect("docker");
    child.stdin.take().unwrap().write_all(input).unwrap();
    assert!(child.wait().unwrap().success(), "docker {args:?}");
}

/// Removes the container whatever the test's outcome, so a failure leaves nothing running.
struct Cleanup(String);
impl Drop for Cleanup {
    fn drop(&mut self) {
        let _ = docker(&["rm", "-f", &self.0]);
    }
}

/// Serves `PAGE` on 8001, the test file on 8000 and the oversized one on 8002, all inside the
/// container, as `attachment` downloads.
fn serve_inside(container: &str, bytes: &[u8]) {
    docker_stdin(&["exec", "-i", container, "sh", "-c", "cat > /tmp/src.bin"], bytes);
    let scripts = [
        ("page.sh", format!("printf 'HTTP/1.0 200 OK\\r\\nContent-Type: text/html\\r\\nContent-Length: {}\\r\\n\\r\\n'\nprintf '%s' \"{}\"\n", PAGE.len(), PAGE.replace('"', "\\\""))),
        ("file.sh", format!("printf 'HTTP/1.0 200 OK\\r\\nContent-Type: application/octet-stream\\r\\nContent-Disposition: attachment; filename=\"clip.mp4\"\\r\\nContent-Length: {}\\r\\n\\r\\n'\ncat /tmp/src.bin\n", bytes.len())),
        ("big.sh", format!("printf 'HTTP/1.0 200 OK\\r\\nContent-Type: application/octet-stream\\r\\nContent-Disposition: attachment; filename=\"big.mp4\"\\r\\nContent-Length: {BIG_BYTES}\\r\\n\\r\\n'\nhead -c {BIG_BYTES} /dev/zero\n")),
    ];
    for (name, body) in &scripts {
        docker_stdin(&["exec", "-i", container, "sh", "-c", &format!("cat > /tmp/{name}")], body.as_bytes());
    }
    for (port, name) in [(8001, "page.sh"), (8000, "file.sh"), (8002, "big.sh")] {
        docker(&["exec", "-d", container, "socat", &format!("TCP-LISTEN:{port},bind=127.0.0.1,reuseaddr,fork"), &format!("SYSTEM:sh /tmp/{name}")])
            .expect("socat");
    }
}

/// What's in the container's download folder right now (empty when it's gone).
fn container_downloads(container: &str) -> Vec<String> {
    docker(&["exec", container, "ls", "-A", "/tmp/kwnt-dl"])
        .unwrap_or_default()
        .lines()
        .map(str::to_string)
        .filter(|l| !l.is_empty())
        .collect()
}

fn rpc(method: &str, params: Value) -> Value {
    json!({"jsonrpc": "2.0", "id": 1, "method": method, "params": params})
}

async fn self_tool(app: &Router, token: &str, name: &str, args: Value) -> (Value, bool) {
    let req = Request::builder()
        .method("POST")
        .uri("/api/mcp/self")
        .header("content-type", "application/json")
        .header("authorization", format!("Bearer {token}"))
        .body(Body::from(rpc("tools/call", json!({"name": name, "arguments": args})).to_string()))
        .unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let v: Value = serde_json::from_slice(&to_bytes(resp.into_body(), usize::MAX).await.unwrap()).unwrap_or(Value::Null);
    let is_error = v["result"]["isError"].as_bool().unwrap_or(false);
    let text = v["result"]["content"][0]["text"].as_str().unwrap_or("");
    (serde_json::from_str(text).unwrap_or(Value::Null), is_error)
}

/// Polls `browser_page` until `download.status` is terminal, or panics.
async fn wait_for_download(app: &Router, token: &str) -> Value {
    for _ in 0..200 {
        let (page, is_error) = self_tool(app, token, "browser_page", json!({})).await;
        assert!(!is_error, "{page}");
        if let Some(status) = page["download"]["status"].as_str() {
            if status != "in_progress" {
                return page["download"].clone();
            }
        }
        tokio::time::sleep(Duration::from_millis(150)).await;
    }
    panic!("download never reached a terminal state");
}

async fn wait_until_empty(container: &str) -> Vec<String> {
    for _ in 0..40 {
        let left = container_downloads(container);
        if left.is_empty() {
            return left;
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
    container_downloads(container)
}

fn files_in(dir: &std::path::Path) -> Vec<String> {
    std::fs::read_dir(dir)
        .map(|d| d.flatten().map(|e| e.file_name().to_string_lossy().to_string()).collect())
        .unwrap_or_default()
}

fn app_state(data_dir: &std::path::Path, db: Database, cdp_url: Option<String>) -> AppState {
    let config = Config { data_dir: data_dir.to_path_buf(), browser_cdp_url: cdp_url, ..Config::default() };
    AppState::new(config.clone(), db, ToolRunner::new(data_dir.join("workspace")), PermissionBroker::new(), LayaRouter::new(config.laya_url.clone()))
}

#[tokio::test]
#[ignore = "needs Docker and the kwnt-browser image (BROWSER_IMAGE); starts a real container"]
async fn a_vm_bots_download_from_the_browser_container_lands_in_its_own_folder() {
    std::env::set_var(kwnt::sandbox::computer::MODE_ENV, "vm");
    std::env::remove_var("KWNT_BROWSER_CDP_URL");
    let tmp = tempfile::tempdir().unwrap();
    let container = managed_container_name(tmp.path());
    let _cleanup = Cleanup(container.clone());
    let db = Database::init(tmp.path()).await.unwrap();
    let state = app_state(tmp.path(), db, None);
    let app = create_router(state.clone());
    let kwants = state.db.list_kwants(&SpaceId::default_space()).await.unwrap();
    let (bot_a, bot_b) = (&kwants[0], &kwants[1]);
    let scope_a = SelfScope { kwant_id: bot_a.id.clone(), kwant_name: bot_a.name.clone(), sandbox_target: Some(bot_a.id.clone()), ..Default::default() };
    let scope_b = SelfScope { kwant_id: bot_b.id.clone(), kwant_name: bot_b.name.clone(), sandbox_target: Some(bot_b.id.clone()), ..Default::default() };
    let (_ctx_a, token_a) = mcp_runtime::prepare_turn_with_self(&state, bot_a, None, None, Some(scope_a)).await.unwrap();
    let (_ctx_b, token_b) = mcp_runtime::prepare_turn_with_self(&state, bot_b, None, None, Some(scope_b)).await.unwrap();

    // B's session first, so its own DevTools connection is listening before A's download begins
    get_or_create_browser_controller(&state, &bot_b.id).await.expect("kwant starts its browser container");
    assert_eq!(docker(&["inspect", "--format", "{{.State.Running}}", &container]).as_deref(), Some("true"));
    let bytes = test_bytes();
    serve_inside(&container, &bytes);
    let (page, is_error) = self_tool(&app, &token_b, "browser_open", json!({"url": "http://127.0.0.1:8001/"})).await;
    assert!(!is_error, "{page}");
    let (page, is_error) = self_tool(&app, &token_a, "browser_open", json!({"url": "http://127.0.0.1:8001/"})).await;
    assert!(!is_error, "{page}");

    // 1. A's download lands in A's own folder, byte for byte, with the usual report
    let (page, is_error) = self_tool(&app, &token_a, "browser_act", json!({"action": "click_text", "text": "Get the file"})).await;
    assert!(!is_error, "{page}");
    let report = wait_for_download(&app, &token_a).await;
    eprintln!("A's report: {report}");
    assert_eq!(report["status"], "done", "{report}");
    assert_eq!(report["path"], "downloads/clip.mp4");
    assert!(report["note"].as_str().unwrap().starts_with("Downloaded clip.mp4 (300.0 KB) to downloads/clip.mp4"), "{report}");
    let (_, workspace_a) = resolve_sandbox_target_info(&state, &bot_a.id).await.expect("bot a workspace");
    let landed = std::fs::read(workspace_a.join("downloads/clip.mp4")).expect("the file landed in A's folder");
    assert_eq!(landed.len(), bytes.len());
    assert!(landed == bytes, "the landed bytes match the served ones");
    eprintln!("landed {} bytes at {}", landed.len(), workspace_a.join("downloads/clip.mp4").display());
    assert_eq!(wait_until_empty(&container).await, Vec::<String>::new(), "the container copy is removed");
    let staging = tmp.path().join("attachments").join("download-tmp");
    assert_eq!(files_in(&staging), Vec::<String>::new(), "nothing is left in staging");

    // 2. B, in the same container, gets nothing
    let (_, workspace_b) = resolve_sandbox_target_info(&state, &bot_b.id).await.expect("bot b workspace");
    assert_eq!(files_in(&workspace_b.join("downloads")), Vec::<String>::new(), "nothing landed in B's folder");
    let (page_b, is_error) = self_tool(&app, &token_b, "browser_page", json!({})).await;
    assert!(!is_error, "{page_b}");
    assert!(page_b["download"].is_null(), "B's page never started a download: {page_b}");

    // 3. Too big for the container: a plain failure, and nothing on the tmpfs
    let (page, is_error) = self_tool(&app, &token_a, "browser_open", json!({"url": "http://127.0.0.1:8001/"})).await;
    assert!(!is_error, "{page}");
    let (page, is_error) = self_tool(&app, &token_a, "browser_act", json!({"action": "click_text", "text": "Get the big file"})).await;
    assert!(!is_error, "{page}");
    let report = wait_for_download(&app, &token_a).await;
    eprintln!("A's big report: {report}");
    assert_eq!(report["status"], "cancelled", "{report}");
    assert!(report["note"].as_str().unwrap().contains("over 400 MB"), "{report}");
    assert_eq!(wait_until_empty(&container).await, Vec::<String>::new(), "nothing is left on the tmpfs");
    let tmp_used = docker(&["exec", &container, "du", "-sm", "/tmp"]).unwrap_or_default();
    eprintln!("container /tmp after the big one: {tmp_used}");
    assert_eq!(files_in(&workspace_a.join("downloads")), vec!["clip.mp4".to_string()]);
    assert_eq!(files_in(&staging), Vec::<String>::new());

    mcp_runtime::finish_turn(&state, &token_a).await;
    mcp_runtime::finish_turn(&state, &token_b).await;

    // 4. The same Chrome reached as a browser on another computer (KWNT_BROWSER_CDP_URL): kwant
    // has no Docker access to it, so the download is refused with the clearer message
    let binding = docker(&["port", &container, "9222/tcp"]).expect("port");
    let port = binding.lines().next().and_then(|l| l.rsplit(':').next()).unwrap().to_string();
    let other = tempfile::tempdir().unwrap();
    let other_db = Database::init(other.path()).await.unwrap();
    let remote = app_state(other.path(), other_db, Some(format!("http://127.0.0.1:{port}")));
    let remote_app = create_router(remote.clone());
    let bots = remote.db.list_kwants(&SpaceId::default_space()).await.unwrap();
    let bot = &bots[0];
    let scope = SelfScope { kwant_id: bot.id.clone(), kwant_name: bot.name.clone(), sandbox_target: Some(bot.id.clone()), ..Default::default() };
    let (_ctx, token) = mcp_runtime::prepare_turn_with_self(&remote, bot, None, None, Some(scope)).await.unwrap();
    let (page, is_error) = self_tool(&remote_app, &token, "browser_open", json!({"url": "http://127.0.0.1:8001/"})).await;
    assert!(!is_error, "{page}");
    let (page, is_error) = self_tool(&remote_app, &token, "browser_act", json!({"action": "click_text", "text": "Get the file"})).await;
    assert!(!is_error, "{page}");
    let report = wait_for_download(&remote_app, &token).await;
    eprintln!("remote report: {report}");
    assert_eq!(report["status"], "failed", "{report}");
    assert!(report["note"].as_str().unwrap().contains("This browser runs on another computer"), "{report}");
    assert!(!report["note"].as_str().unwrap().contains("for now"), "{report}");
    let (_, workspace) = resolve_sandbox_target_info(&remote, &bot.id).await.expect("workspace");
    assert_eq!(files_in(&workspace.join("downloads")), Vec::<String>::new());
    assert_eq!(container_downloads(&container), Vec::<String>::new(), "a denied download writes nothing");
    mcp_runtime::finish_turn(&remote, &token).await;
    assert!(docker(&["inspect", &managed_container_name(other.path())]).is_none(), "an outside browser starts no container");
}
