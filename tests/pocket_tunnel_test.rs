//! pa6 Join from anywhere (Pocket API v1.2): a one-tap Cloudflare quick tunnel to the guest port
//! only, refused for long-lived kits (Secret Santa), shared by the live pockets that asked for it
//! and stopped with the last of them. Brief:
//! /home/maus/shared/ship-software-safely/autonomy/pocket-apps/pa6/brief.md ("Tests").
//!
//! No network: a fake `cloudflared` (a shell script at `<data_dir>/.test-cloudflared`, the
//! test-seams hook) prints a trycloudflare line the way the real one does and then sleeps.

use axum::{
    body::{to_bytes, Body},
    extract::ConnectInfo,
    http::{Request, StatusCode},
    Router,
};
use chrono::{Duration, Utc};
use kwnt::{
    broker::PermissionBroker,
    config::Config,
    db::Database,
    pocket::{self, listener},
    router::LayaRouter,
    server::{
        auth::{ApiToken, Require},
        create_router, AppState,
    },
    tools::ToolRunner,
};
use serde_json::{json, Value};
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use tempfile::{tempdir, TempDir};
use tower::ServiceExt;

const TOKEN: &str = "test-api-token-0123456789";
const LOCAL: &str = "127.0.0.1:50000";

struct T {
    main: Router,
    /// Only the `cfg(unix)` tests read it (they drive a fake `cloudflared` shell script).
    #[cfg_attr(not(unix), allow(dead_code))]
    state: AppState,
    data: PathBuf,
    _tmp: TempDir,
}

async fn setup() -> T {
    let tmp = tempdir().unwrap();
    let data = tmp.path().join("data");
    let db = Database::init(&data).await.unwrap();
    let config = Config { data_dir: data.clone(), pocket_port: Some(0), browser_cdp_url: Some("http://127.0.0.1:9".into()), ..Config::default() };
    let mut state = AppState::new(config.clone(), db, ToolRunner::new(data.join("workspace")), PermissionBroker::new(), LayaRouter::new(config.laya_url.clone()));
    state.api_token = Arc::new(ApiToken::for_tests(TOKEN, Require::Auto, config.port));
    state.db.set_setting(pocket::SETTING, "on").await.unwrap();
    listener::reconcile(&state).await;
    T { main: create_router(state.clone()), state, data, _tmp: tmp }
}

struct Res {
    status: StatusCode,
    body: Value,
}

async fn send(app: &Router, method: &str, uri: &str, body: Option<Value>) -> Res {
    let req = Request::builder().method(method).uri(uri).header("host", "127.0.0.1:8799").header("content-type", "application/json");
    let mut req = req.body(body.map(|b| Body::from(b.to_string())).unwrap_or_else(Body::empty)).unwrap();
    req.extensions_mut().insert(ConnectInfo(LOCAL.parse::<SocketAddr>().unwrap()));
    let resp = app.clone().oneshot(req).await.unwrap();
    let status = resp.status();
    let bytes = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    Res { status, body: serde_json::from_slice(&bytes).unwrap_or_else(|_| Value::String(String::from_utf8_lossy(&bytes).into())) }
}

async fn make(t: &T, kit: &str, spec: Value) -> String {
    let r = send(&t.main, "POST", "/api/pockets", Some(json!({ "kit": kit, "spec": spec }))).await;
    assert_eq!(r.status, StatusCode::CREATED, "{}", r.body);
    r.body["pocket"]["code"].as_str().unwrap().to_string()
}

async fn quiz(t: &T) -> String {
    make(t, "test", json!({ "title": "Pub quiz" })).await
}

/// A Secret Santa 10 days out.
async fn santa(t: &T) -> String {
    let day = (Utc::now() + Duration::days(10)).date_naive().format("%Y-%m-%d").to_string();
    make(t, "santa", json!({ "title": "Family Secret Santa", "gift_on": day, "reveal_at": format!("{day}T18:00:00Z"), "budget": "€25", "host_name": "Oleks" })).await
}

async fn tunnel(t: &T, code: &str, on: bool) -> Res {
    send(&t.main, "POST", &format!("/api/pockets/{code}/tunnel"), Some(json!({ "on": on }))).await
}

async fn join(t: &T, code: &str) -> Value {
    let r = send(&t.main, "GET", &format!("/api/pockets/{code}"), None).await;
    assert_eq!(r.status, StatusCode::OK, "{}", r.body);
    r.body["join"].clone()
}

#[cfg(unix)]
/// Waits (5 s at most) until the pocket's `join.tunnel.state` is `want`.
async fn tunnel_until(t: &T, code: &str, want: &str) -> Value {
    let mut last = Value::Null;
    for _ in 0..100 {
        last = join(t, code).await;
        if last["tunnel"]["state"] == want {
            return last;
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
    panic!("the tunnel never got to {want}: {last}");
}

#[cfg(unix)]
const FAKE_URL: &str = "https://bowl-cited-than-amplifier.trycloudflare.com";

/// A fake cloudflared: `--version` answers like the real one; a tunnel writes its arguments and
/// pid next to itself, prints the real banner (stderr) and sleeps. `fail` exits at once instead.
#[cfg(unix)]
fn fake_cloudflared(data: &std::path::Path, fail: bool) {
    use std::os::unix::fs::PermissionsExt;
    let dir = data.display();
    let tunnel = if fail {
        "echo \"2026-11-23T07:32:12Z ERR failed to request quick Tunnel: Post \\\"https://api.trycloudflare.com/tunnel\\\": dial tcp: lookup api.trycloudflare.com: no such host\" >&2\nexit 1".to_string()
    } else {
        format!(
            "echo $$ > \"{dir}/cloudflared-pid\"\n\
             echo '2026-11-23T07:32:12Z INF Requesting new quick Tunnel on trycloudflare.com...' >&2\n\
             echo '2026-11-23T07:32:18Z INF +--------------------------------------------------------------------------------------------+' >&2\n\
             echo '2026-11-23T07:32:18Z INF |  Your quick Tunnel has been created! Visit it at (it may take some time to be reachable):  |' >&2\n\
             echo '2026-11-23T07:32:18Z INF |  {FAKE_URL}                                       |' >&2\n\
             exec sleep 300"
        )
    };
    let script = format!(
        "#!/bin/sh\nif [ \"$1\" = \"--version\" ]; then echo 'cloudflared version 2026.11.1 (built 2026-11-17-15:04 UTC)'; exit 0; fi\n\
         printf '%s\\n' \"$@\" > \"{dir}/cloudflared-args\"\n{tunnel}\n"
    );
    let path = data.join(".test-cloudflared");
    std::fs::write(&path, script).unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
}

#[cfg(unix)]
fn alive(pid: &str) -> bool {
    std::process::Command::new("kill").args(["-0", pid]).stderr(std::process::Stdio::null()).status().is_ok_and(|s| s.success())
}

#[cfg(unix)]
async fn gone(pid: &str) -> bool {
    for _ in 0..100 {
        if !alive(pid) {
            return true;
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
    false
}

#[tokio::test]
async fn a_santa_pocket_never_gets_a_quick_tunnel() {
    let t = setup().await;
    let code = santa(&t).await;
    let r = tunnel(&t, &code, true).await;
    assert_eq!(r.status, StatusCode::CONFLICT, "{}", r.body);
    assert_eq!(r.body["error"], "no_quick_tunnel");
    let j = join(&t, &code).await;
    assert_eq!(j["quick_tunnel"], false);
    assert_eq!(j["tunnel"]["state"], "off");
    assert!(j["anywhere_url"].is_null(), "{j}");
    assert!(!t.data.join("cloudflared-args").exists(), "a Santa pocket never starts cloudflared");
}

#[cfg(unix)]
#[tokio::test]
async fn the_quick_tunnel_points_only_at_the_guest_port() {
    let t = setup().await;
    fake_cloudflared(&t.data, false);
    let code = quiz(&t).await;
    let before = join(&t, &code).await;
    assert_eq!(before["quick_tunnel"], true, "{before}");
    assert_eq!(before["tunnel"]["state"], "off");
    let r = tunnel(&t, &code, true).await;
    assert_eq!(r.status, StatusCode::OK, "{}", r.body);
    let j = tunnel_until(&t, &code, "on").await;
    assert_eq!(j["mode"], "anywhere");
    assert_eq!(j["anywhere_kind"], "quick");
    assert_eq!(j["anywhere_url"], format!("{FAKE_URL}/p/{code}"));
    assert_eq!(j["tunnel"]["url"], FAKE_URL);
    assert!(j["tunnel"]["note"].as_str().is_some_and(|n| n.contains("new address")), "{j}");
    let guest_port = t.state.pocket.listener.addr().expect("the guest listener runs").port();
    assert_ne!(guest_port, t.state.config.port);
    let args = std::fs::read_to_string(t.data.join("cloudflared-args")).unwrap();
    let args: Vec<&str> = args.lines().collect();
    let target = format!("http://127.0.0.1:{guest_port}");
    assert_eq!(args, vec!["tunnel", "--no-autoupdate", "--url", target.as_str()], "only ever the guest port");
}

#[cfg(unix)]
#[tokio::test]
async fn the_tunnel_is_shared_and_stops_with_the_last_pocket_that_uses_it() {
    let t = setup().await;
    fake_cloudflared(&t.data, false);
    let (a, b, c) = (quiz(&t).await, quiz(&t).await, quiz(&t).await);
    assert_eq!(tunnel(&t, &a, true).await.status, StatusCode::OK);
    tunnel_until(&t, &a, "on").await;
    let pid = std::fs::read_to_string(t.data.join("cloudflared-pid")).unwrap().trim().to_string();
    assert_eq!(tunnel(&t, &b, true).await.status, StatusCode::OK);
    tunnel_until(&t, &b, "on").await;
    let again = std::fs::read_to_string(t.data.join("cloudflared-pid")).unwrap().trim().to_string();
    assert_eq!(pid, again, "one tunnel per box: the second pocket shares it");
    assert_eq!(join(&t, &c).await["tunnel"]["state"], "off", "a pocket that didn't ask isn't on it");
    assert!(join(&t, &c).await["anywhere_url"].is_null());

    // Ending A keeps it for B
    assert_eq!(send(&t.main, "POST", &format!("/api/pockets/{a}/end"), None).await.status, StatusCode::OK);
    assert_eq!(join(&t, &a).await["tunnel"]["state"], "off");
    assert_eq!(join(&t, &a).await["quick_tunnel"], false, "an ended pocket can't turn it on again");
    assert_eq!(tunnel(&t, &a, true).await.body["error"], "pocket_ended");
    assert_eq!(join(&t, &b).await["tunnel"]["state"], "on");
    assert!(alive(&pid), "B still uses it");

    // Deleting B, the last user, stops cloudflared
    assert_eq!(send(&t.main, "DELETE", &format!("/api/pockets/{b}"), None).await.status, StatusCode::NO_CONTENT);
    assert!(gone(&pid).await, "the last user deleted: cloudflared stops");
    assert!(!t.state.pocket.tunnel.active());
}

#[cfg(unix)]
#[tokio::test]
async fn turning_it_off_or_the_switch_off_stops_it() {
    let t = setup().await;
    fake_cloudflared(&t.data, false);
    let (a, b) = (quiz(&t).await, quiz(&t).await);
    tunnel(&t, &a, true).await;
    tunnel(&t, &b, true).await;
    tunnel_until(&t, &a, "on").await;
    let pid = std::fs::read_to_string(t.data.join("cloudflared-pid")).unwrap().trim().to_string();
    // A turns it off for itself; B keeps it
    let r = tunnel(&t, &a, false).await;
    assert_eq!(r.status, StatusCode::OK);
    assert_eq!(r.body["join"]["tunnel"]["state"], "off");
    assert_eq!(join(&t, &b).await["tunnel"]["state"], "on");
    assert!(alive(&pid));
    // Pocket apps switched off: everything stops
    t.state.db.set_setting(pocket::SETTING, "off").await.unwrap();
    listener::reconcile(&t.state).await;
    assert!(gone(&pid).await, "the switch off stops cloudflared");
    assert!(!t.state.pocket.tunnel.active());
}

#[cfg(unix)]
#[tokio::test]
async fn a_tunnel_that_cant_start_says_so_plainly_and_can_be_tried_again() {
    let t = setup().await;
    fake_cloudflared(&t.data, true);
    let code = quiz(&t).await;
    assert_eq!(tunnel(&t, &code, true).await.status, StatusCode::OK);
    let j = tunnel_until(&t, &code, "failed").await;
    assert_eq!(j["tunnel"]["note"], pocket::tunnel::FAILED_LINE);
    assert!(j["tunnel"]["url"].is_null());
    assert_ne!(j["mode"], "anywhere", "an error line is never read as the address: {j}");
    // Try again with a cloudflared that works
    fake_cloudflared(&t.data, false);
    assert_eq!(tunnel(&t, &code, true).await.status, StatusCode::OK);
    tunnel_until(&t, &code, "on").await;
    t.state.pocket.tunnel.stop_all();
}

#[tokio::test]
async fn the_tunnel_route_needs_a_pocket_that_is_there_and_a_body() {
    let t = setup().await;
    let r = tunnel(&t, "zzzzzz", true).await;
    assert_eq!(r.status, StatusCode::NOT_FOUND, "{}", r.body);
    let code = quiz(&t).await;
    let r = send(&t.main, "POST", &format!("/api/pockets/{code}/tunnel"), Some(json!({}))).await;
    assert!(r.status.is_client_error(), "no `on`: {}", r.status);
    // Off when it was never on is fine and changes nothing
    let r = tunnel(&t, &code, false).await;
    assert_eq!(r.status, StatusCode::OK, "{}", r.body);
    assert_eq!(r.body["join"]["tunnel"]["state"], "off");
}
