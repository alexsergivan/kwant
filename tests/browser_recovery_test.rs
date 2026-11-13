//! 0.9.5 browser recovery: a VM browser that stops answering for a while (overloaded, out of
//! memory) is waited for, not given up on; a browser that isn't there is reported at once; and
//! the panel's status never opens a page.
use axum::{extract::{Path, State, WebSocketUpgrade, ws::Message}, routing::get, Json, Router};
use futures::StreamExt;
use kwnt::{broker::PermissionBroker, config::Config, db::Database, router::LayaRouter, server::{create_router, AppState}, tools::ToolRunner};
use kwnt::sandbox::service::{BROWSER_BUSY, BROWSER_BUSY_CODE};
use serde_json::{json, Value};
use std::{sync::{atomic::{AtomicUsize, Ordering}, Arc}, time::{Duration, Instant}};
use tokio::{net::TcpListener, sync::Mutex};

/// A DevTools endpoint that can stop answering for a while, the way an overloaded Chrome does:
/// its `/json` requests hang and its sockets take commands without replying.
#[derive(Clone, Default)]
struct Mock {
    port: u16,
    targets: Arc<Mutex<Vec<Value>>>,
    commands: Arc<Mutex<Vec<Value>>>,
    contexts: Arc<AtomicUsize>,
    stall_until: Arc<std::sync::Mutex<Option<Instant>>>,
}

impl Mock {
    fn stalled(&self) -> bool { self.stall_until.lock().unwrap().is_some_and(|until| Instant::now() < until) }
    fn stall_for(&self, d: Duration) { *self.stall_until.lock().unwrap() = Some(Instant::now() + d); }
    /// Holds a `/json` answer (or a socket opening) while stalled: longer than any client waits.
    async fn hang_while_stalled(&self) { if self.stalled() { tokio::time::sleep(Duration::from_secs(60)).await; } }
    async fn methods(&self) -> Vec<String> {
        self.commands.lock().await.iter().map(|c| c["method"].as_str().unwrap_or_default().to_string()).collect()
    }
}

async fn cdp(State(mock): State<Mock>, Path(_id): Path<String>, ws: WebSocketUpgrade) -> axum::response::Response {
    mock.hang_while_stalled().await;
    ws.on_upgrade(move |mut socket| async move {
        while let Some(Ok(Message::Text(text))) = socket.next().await {
            let cmd: Value = serde_json::from_str(&text).unwrap();
            mock.commands.lock().await.push(cmd.clone());
            // an overloaded browser reads the command and never gets round to answering it
            if mock.stalled() { continue; }
            let result = match cmd["method"].as_str().unwrap() {
                "Target.createBrowserContext" => json!({"browserContextId": format!("context-{}", mock.contexts.fetch_add(1, Ordering::SeqCst))}),
                "Target.createTarget" => {
                    let id = format!("page-{}", mock.contexts.load(Ordering::SeqCst));
                    mock.targets.lock().await.push(json!({"targetId": id, "browserContextId": cmd["params"]["browserContextId"]}));
                    json!({"targetId": id})
                }
                "Target.getTargets" => json!({"targetInfos": mock.targets.lock().await.clone()}),
                "Browser.getVersion" => json!({"product": "HeadlessChrome/156"}),
                "Runtime.evaluate" => json!({"result": {"value": {"url": "https://example.com/", "title": "Example"}}}),
                "Page.captureScreenshot" => json!({"data": "aGVsbG8="}),
                _ => json!({}),
            };
            if socket.send(Message::Text(json!({"id": cmd["id"], "result": result}).to_string())).await.is_err() { break; }
        }
    })
}

async fn mock() -> (Mock, tokio::task::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let mock = Mock { port: listener.local_addr().unwrap().port(), ..Mock::default() };
    let app = Router::new()
        .route("/json/version", get(|State(m): State<Mock>| async move {
            m.hang_while_stalled().await;
            Json(json!({"webSocketDebuggerUrl": "ws://browser:9223/devtools/browser/root"}))
        }))
        .route("/json/list", get(|State(m): State<Mock>| async move {
            m.hang_while_stalled().await;
            Json(m.targets.lock().await.iter().map(|t| json!({"type": "page", "id": t["targetId"],
                "webSocketDebuggerUrl": format!("ws://browser:9223/devtools/page/{}", t["targetId"].as_str().unwrap())})).collect::<Vec<_>>())
        }))
        .route("/devtools/browser/:id", get(cdp))
        .route("/devtools/page/:id", get(cdp))
        .with_state(mock.clone());
    let task = tokio::spawn(async move { axum::serve(listener, app).await.unwrap(); });
    (mock, task)
}

/// A kwnt server whose VM browser is at `endpoint`, and the id of its first team.
async fn server(endpoint: String, tmp: &tempfile::TempDir) -> (AppState, String, u16, tokio::task::JoinHandle<()>) {
    let config = Config { data_dir: tmp.path().into(), browser_cdp_url: Some(endpoint), ..Config::default() };
    let db = Database::init(tmp.path()).await.unwrap();
    let id = db.list_kwants(&kwnt::db::spaces::SpaceId::default_space()).await.unwrap()[0].id.clone();
    let state = AppState::new(config.clone(), db, ToolRunner::new(tmp.path().join("workspace")), PermissionBroker::new(), LayaRouter::new(config.laya_url));
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let app = create_router(state.clone());
    let task = tokio::spawn(async move { axum::serve(listener, app).await.unwrap(); });
    (state, id, port, task)
}

async fn api_get(port: u16, path: &str) -> (reqwest::StatusCode, Value) {
    let response = reqwest::get(format!("http://127.0.0.1:{port}{path}")).await.unwrap();
    let status = response.status();
    (status, response.json().await.unwrap_or(Value::Null))
}

#[tokio::test]
async fn a_browser_that_stalls_for_ten_seconds_is_waited_for() {
    let (mock, cdp_task) = mock().await;
    let tmp = tempfile::tempdir().unwrap();
    let (_state, id, port, app_task) = server(format!("http://127.0.0.1:{}", mock.port), &tmp).await;

    // the bot's page is open and works
    let (status, body) = api_get(port, &format!("/api/sandbox/{id}/screenshot")).await;
    assert_eq!(status, 200, "{body}");

    // then the browser stops answering for 10 s (the Nov 12 incident: Target.getTargets and the
    // page's own commands time out); the bot's next step waits it out instead of failing
    mock.stall_for(Duration::from_secs(10));
    let started = Instant::now();
    let (status, body) = api_get(port, &format!("/api/sandbox/{id}/screenshot")).await;
    let waited = started.elapsed();
    assert_eq!(status, 200, "a 10 s stall must not fail the step: {body}");
    assert_eq!(body["screenshot"], "aGVsbG8=");
    assert!(waited >= Duration::from_secs(9), "the answer came after the stall, not before ({waited:?})");
    assert!(waited < Duration::from_secs(45), "inside the retry window ({waited:?})");
    assert_eq!(mock.contexts.load(Ordering::SeqCst), 1, "the same page is used again, not a new one");

    app_task.abort();
    cdp_task.abort();
}

#[tokio::test]
async fn a_browser_that_stays_stalled_is_reported_busy_and_retryable() {
    let (mock, cdp_task) = mock().await;
    let tmp = tempfile::tempdir().unwrap();
    let (state, id, port, app_task) = server(format!("http://127.0.0.1:{}", mock.port), &tmp).await;
    state.job_pages.set_retry_window(Duration::from_secs(4));
    mock.stall_for(Duration::from_secs(120));

    let (status, body) = api_get(port, &format!("/api/sandbox/{id}/screenshot")).await;
    assert_eq!(status, 503);
    assert_eq!(body["error"], BROWSER_BUSY, "temporary, retry in a minute; never the Docker text");
    assert_eq!(body["code"], BROWSER_BUSY_CODE);
    assert_eq!(body["retryable"], true);
    assert!(!body["error"].as_str().unwrap().contains("Docker"));

    // status says the same, quickly, and opens nothing
    let started = Instant::now();
    let (status, body) = api_get(port, &format!("/api/sandbox/{id}/status")).await;
    assert_eq!(status, 200);
    assert_eq!(body["is_running"], true, "overloaded, not gone");
    assert_eq!(body["responding"], false);
    assert_eq!(body["code"], BROWSER_BUSY_CODE);
    assert_eq!(body["retryable"], true);
    assert!(started.elapsed() < Duration::from_secs(8), "status doesn't wait out a whole command timeout");
    assert!(!mock.methods().await.iter().any(|m| m == "Target.createTarget"));

    app_task.abort();
    cdp_task.abort();
}

#[tokio::test]
async fn a_browser_that_is_not_there_gets_the_docker_text_at_once() {
    // a port nothing listens on: connection refused
    let closed = TcpListener::bind("127.0.0.1:0").await.unwrap().local_addr().unwrap().port();
    let tmp = tempfile::tempdir().unwrap();
    let (_state, id, port, app_task) = server(format!("http://127.0.0.1:{closed}"), &tmp).await;

    let started = Instant::now();
    let (status, body) = api_get(port, &format!("/api/sandbox/{id}/screenshot")).await;
    assert_eq!(status, 503);
    let error = body["error"].as_str().unwrap();
    assert_ne!(error, BROWSER_BUSY);
    assert!(body.get("code").is_none() && body.get("retryable").is_none(), "{body}");
    assert!(started.elapsed() < Duration::from_secs(5), "a refused connection isn't retried for 45 s");

    app_task.abort();
}

#[tokio::test]
async fn status_reports_health_without_opening_a_page() {
    let (mock, cdp_task) = mock().await;
    let tmp = tempfile::tempdir().unwrap();
    let (state, id, port, app_task) = server(format!("http://127.0.0.1:{}", mock.port), &tmp).await;

    for _ in 0..3 {
        let (status, body) = api_get(port, &format!("/api/sandbox/{id}/status")).await;
        assert_eq!(status, 200);
        assert_eq!(body["is_running"], true, "{body}");
        assert_eq!(body["responding"], true);
        assert_eq!(body["page_open"], false);
    }
    // a key that names no team never reaches the browser at all
    let before = mock.commands.lock().await.len();
    let (_, body) = api_get(port, "/api/sandbox/no-such-team/status").await;
    assert_eq!(body["is_running"], false);
    assert_eq!(mock.commands.lock().await.len(), before);

    let methods = mock.methods().await;
    assert!(!methods.iter().any(|m| m == "Target.createTarget" || m == "Target.createBrowserContext"), "{methods:?}");
    assert!(state.browser_controllers.lock().await.is_empty());

    // once the bot opened its page, status says so, still without touching it
    let (status, _) = api_get(port, &format!("/api/sandbox/{id}/screenshot")).await;
    assert_eq!(status, 200);
    let targets = mock.targets.lock().await.len();
    mock.commands.lock().await.clear();
    let (_, body) = api_get(port, &format!("/api/sandbox/{id}/status")).await;
    assert_eq!(body["page_open"], true);
    assert_eq!(mock.targets.lock().await.len(), targets);
    let methods = mock.methods().await;
    assert!(methods.iter().all(|m| m == "Browser.getVersion"), "status only asks the browser for its version: {methods:?}");

    app_task.abort();
    cdp_task.abort();
}

/// A page that keeps the browser busy the way an autoplaying feed does: a canvas animation
/// played as a video, plus a counter that ticks every 100 ms while the page runs.
const AUTOPLAY_PAGE: &str = "<title>autoplay</title><canvas id=c width=1280 height=720></canvas><video id=v autoplay muted playsinline></video><script>\
window.__ticks=0;setInterval(()=>window.__ticks++,100);\
const c=document.getElementById('c'),x=c.getContext('2d');\
(function draw(t){for(let i=0;i<400;i++){x.fillStyle='hsl('+((t/5+i)%360)+',80%,50%)';x.fillRect(Math.random()*1280,Math.random()*720,40,40);}requestAnimationFrame(draw);})(0);\
const v=document.getElementById('v');v.srcObject=c.captureStream(60);v.play();</script>";

/// What a bot drives: a page with a field it types into.
const DRIVEN_PAGE: &str = "<title>driven</title><input id=q><p id=out></p>";

/// A `data:` address for `html` (`%` and `#` escaped: `%360` would otherwise read as `%36`).
fn data_url(html: &str) -> String {
    format!("data:text/html,{}", html.replace('%', "%25").replace('#', "%23"))
}

async fn eval(ctrl: &kwnt::sandbox::BrowserController, expression: &str) -> Value {
    let reply = ctrl.send_cdp_command("Runtime.evaluate", json!({"expression": expression, "returnByValue": true})).await.unwrap();
    reply["result"]["value"].clone()
}

/// The real run (0.9.5 brief, "Real"): the repo's browser image under a small memory cap, one
/// autoplaying page and one driven page. Opt-in: `KWNT_REAL_CDP=http://127.0.0.1:<port>` of a
/// kwnt-browser container started for this run (never a shared one).
#[tokio::test]
#[ignore]
async fn real_capped_browser_keeps_the_driven_page_working() {
    let Ok(endpoint) = std::env::var("KWNT_REAL_CDP") else { return };
    let tmp = tempfile::tempdir().unwrap();
    let (state, id, _port, app_task) = server(endpoint, &tmp).await;
    let steady = |key: String| { let state = state.clone(); async move {
        kwnt::server::routes::open_page_steady(&state, &key, Duration::ZERO).await.unwrap_or_else(|_| panic!("open {key}"))
    }};

    // the team page autoplays, then is left alone
    let feed = steady(id.clone()).await;
    feed.send_cdp_command("Page.navigate", json!({"url": data_url(AUTOPLAY_PAGE)})).await.unwrap();
    tokio::time::sleep(Duration::from_secs(2)).await;
    assert_eq!(eval(&feed, "document.getElementById('v').paused").await, false, "the feed plays");
    tokio::time::sleep(Duration::from_secs(4)).await;
    let ticks_before = eval(&feed, "window.__ticks").await.as_i64().unwrap();
    println!("REAL feed playing, ticks={ticks_before}");

    // a bot's job page is driven; the sweep (2 s idle instead of 10 min) rests the feed only
    let job = format!("{id}#bot-1#chat-1");
    let driven = steady(job.clone()).await;
    driven.send_cdp_command("Page.navigate", json!({"url": data_url(DRIVEN_PAGE)})).await.unwrap();
    tokio::time::sleep(Duration::from_secs(3)).await;
    let _ = steady(job.clone()).await; // the bot's next step: just used
    let (frozen, blanked) = kwnt::server::browser_pages::rest_idle(&state, Duration::from_secs(2), Duration::from_secs(3600)).await;
    println!("REAL sweep froze={frozen} blanked={blanked}");
    assert_eq!((frozen, blanked), (1, 0), "the idle feed is frozen, the driven page isn't");

    // the driven page keeps working, step after step, while the feed rests
    let mut slowest = Duration::ZERO;
    let started = Instant::now();
    let mut steps = 0;
    while started.elapsed() < Duration::from_secs(15) {
        let step = Instant::now();
        let ctrl = steady(job.clone()).await;
        eval(&ctrl, &format!("document.getElementById('q').value='step {steps}';document.getElementById('out').textContent=document.getElementById('q').value")).await;
        let shot = ctrl.capture_screenshot().await.expect("the driven page answers");
        assert!(!shot.is_empty());
        slowest = slowest.max(step.elapsed());
        steps += 1;
    }
    assert_eq!(eval(&driven, "document.getElementById('out').textContent").await, json!(format!("step {}", steps - 1)));
    println!("REAL driven page: {steps} steps in 15 s, slowest {} ms", slowest.as_millis());
    assert!(slowest < Duration::from_secs(5), "every step answered promptly");

    // the feed didn't run while frozen, and wakes on its next use, muted video paused
    let ticks_after = eval(&feed, "window.__ticks").await.as_i64().unwrap();
    println!("REAL feed ticks while frozen: {} (it would be ~{} running)", ticks_after - ticks_before, 15 * 10 + 30);
    assert!(ticks_after - ticks_before < 60, "timers stopped while frozen");
    assert_eq!(eval(&feed, "document.visibilityState").await, json!("visible"), "woken pages paint again");
    assert_eq!(eval(&feed, "document.getElementById('v').paused").await, true, "the rested feed stays paused");
    let ticking = eval(&feed, "window.__ticks").await.as_i64().unwrap();
    tokio::time::sleep(Duration::from_secs(1)).await;
    assert!(eval(&feed, "window.__ticks").await.as_i64().unwrap() > ticking + 5, "timers run again after waking");
    assert!(!feed.capture_screenshot().await.unwrap().is_empty());
    // and it still navigates
    feed.send_cdp_command("Page.navigate", json!({"url": data_url(DRIVEN_PAGE)})).await.unwrap();
    tokio::time::sleep(Duration::from_secs(1)).await;
    assert_eq!(eval(&feed, "document.title").await, json!("driven"), "a woken page navigates");
    println!("REAL feed woke: visible, ticking, screenshot ok, navigated");

    // status says the browser is healthy, without creating a page
    let pages = state.browser_service.open_pages().await.len();
    let (_, body) = api_get(_port, &format!("/api/sandbox/{id}/status")).await;
    assert_eq!(body["responding"], true);
    assert_eq!(state.browser_service.open_pages().await.len(), pages);
    println!("REAL status: {body}");

    kwnt::server::browser_pages::close_page(&state, &job).await;
    app_task.abort();
}
