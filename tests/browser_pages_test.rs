//! One browser page per job (0.9.3). Two jobs of one team, each in its own chat, get their own
//! pages in the team's browser: a sign-in the person is asked to do on one job's page holds only
//! that page, while the other job opens and acts on another site; both pages share the team's
//! sign-ins (cookies). A team keeps at most `MAX_JOB_PAGES_PER_TEAM` job pages (the least recently
//! used idle one makes room, never one a step holds) and an idle page is closed, never a held one.
//!
//! The browser parts run a real windowless Chrome, as the team's container browser (an external
//! DevTools endpoint, where a job page is a new page in the team's browser context) and in Local
//! mode (a new window of the person's Chrome). Skipped when no Chrome is installed.

#[path = "support/skip.rs"]
mod skip;

#[path = "support/chrome_guard.rs"]
mod chrome_guard;
use axum::{
    body::{to_bytes, Body},
    http::{Request, StatusCode},
    Router,
};
use chrome_guard::ChromeGuard;
use kwnt::{
    broker::PermissionBroker,
    config::Config,
    db::{human_steps::HumanStepPlace, models::Kwant, spaces::SpaceId, Database},
    router::LayaRouter,
    sandbox::{computer, BrowserController},
    server::{
        browser_pages::{self, job_page_key, MAX_JOB_PAGES_PER_TEAM},
        continuity::{SelfScope, TurnOrigin},
        create_router, mcp_runtime,
        routes::{get_or_create_browser_controller, PageRefusal, BROWSER_FULL, HELD_FOR_BOT, PAGES_FULL},
        AppState,
    },
    tools::ToolRunner,
};
use serde_json::{json, Value};
use std::{path::Path, time::Duration};
use tempfile::{tempdir, TempDir};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tower::ServiceExt;

/// A sign-in page that sets the site's cookie, and another site's page with a button over all of
/// it that sets its title when clicked.
async fn serve() -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    tokio::spawn(async move {
        while let Ok((mut stream, _)) = listener.accept().await {
            tokio::spawn(async move {
                let mut buf = [0u8; 4096];
                let n = stream.read(&mut buf).await.unwrap_or(0);
                let req = String::from_utf8_lossy(&buf[..n]).to_string();
                let (extra, page) = match req.split_whitespace().nth(1).unwrap_or("/") {
                    "/signin" => ("set-cookie: kwnt_session=signed-in; Max-Age=86400; Path=/\r\n", "<!doctype html><title>sign in</title>"),
                    "/other" => ("", "<!doctype html><title>other</title><body style=margin:0>\
                        <button onclick=\"document.title='clicked'\" style='position:fixed;inset:0'>Go</button></body>"),
                    _ => ("", "<!doctype html><title>plain</title>"),
                };
                let reply = format!("HTTP/1.1 200 OK\r\ncontent-type: text/html\r\n{extra}content-length: {}\r\nconnection: close\r\n\r\n{page}", page.len());
                let _ = stream.write_all(reply.as_bytes()).await;
            });
        }
    });
    format!("http://127.0.0.1:{port}")
}

async fn start_chrome(profile: &Path) -> Option<(String, ChromeGuard)> {
    if computer::local_chrome_path().is_none() {
        skip::skip_or_fail("Chrome or Chromium", skip::CHROME_FIX);
        return None;
    }
    let (endpoint, _) = computer::ensure_local_chrome(profile, true).await.expect("Chrome starts");
    Some((endpoint, ChromeGuard(profile.to_path_buf())))
}

/// kwnt whose bot browser is the container browser at `endpoint`.
async fn container_state(tmp: &TempDir, endpoint: String) -> AppState {
    std::env::set_var("KWNT_COMPUTER_MODE", "vm");
    let db = Database::init(tmp.path()).await.unwrap();
    let config = Config { data_dir: tmp.path().to_path_buf(), browser_cdp_url: Some(endpoint), ..Config::default() };
    AppState::new(config.clone(), db, ToolRunner::new(tmp.path().join("workspace")), PermissionBroker::new(), LayaRouter::new(config.laya_url))
}

async fn first_bot(state: &AppState) -> Kwant {
    state.db.list_kwants(&SpaceId::default_space()).await.unwrap().remove(0)
}

/// A turn of `bot` in its thread `thread`, on the team browser `team`; returns its tool token.
async fn turn_in_thread(state: &AppState, bot: &Kwant, turn_id: &str, thread: &str, team: &str) -> String {
    let scope = SelfScope {
        kwant_id: bot.id.clone(),
        kwant_name: bot.name.clone(),
        origin: Some(TurnOrigin { thread_id: Some(thread.into()), ..Default::default() }),
        cancel: Some(tokio_util::sync::CancellationToken::new()),
        turn_id: Some(turn_id.into()),
        sandbox_target: Some(team.into()),
        ..Default::default()
    };
    mcp_runtime::prepare_turn_with_self(state, bot, Some(turn_id.into()), None, Some(scope)).await.unwrap().1
}

/// Calls a self tool in the background, as the CLI would while the turn waits.
fn call(app: &Router, token: &str, name: &str, args: Value) -> tokio::task::JoinHandle<(String, bool)> {
    let (app, token, name) = (app.clone(), token.to_string(), name.to_string());
    tokio::spawn(async move {
        let body = json!({"jsonrpc": "2.0", "id": 1, "method": "tools/call", "params": {"name": name, "arguments": args}});
        let req = Request::builder()
            .method("POST")
            .uri("/api/mcp/self")
            .header("content-type", "application/json")
            .header("authorization", format!("Bearer {token}"))
            .body(Body::from(body.to_string()))
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        let v: Value = serde_json::from_slice(&to_bytes(resp.into_body(), usize::MAX).await.unwrap()).unwrap();
        (v["result"]["content"][0]["text"].as_str().unwrap_or("").to_string(), v["result"]["isError"].as_bool().unwrap_or(false))
    })
}

/// A request to kwnt's API; the status and the JSON answer.
async fn request(app: &Router, method: &str, uri: &str, body: Option<Value>) -> (StatusCode, Value) {
    let mut req = Request::builder().method(method).uri(uri);
    if body.is_some() {
        req = req.header("content-type", "application/json");
    }
    let body = body.map_or_else(Body::empty, |b| Body::from(b.to_string()));
    let resp = app.clone().oneshot(req.body(body).unwrap()).await.unwrap();
    let status = resp.status();
    let bytes = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    (status, serde_json::from_slice(&bytes).unwrap_or(Value::Null))
}

async fn eval(ctrl: &BrowserController, expression: &str) -> Value {
    let v = ctrl.send_cdp_command("Runtime.evaluate", json!({"expression": expression, "returnByValue": true})).await.unwrap();
    v["result"]["value"].clone()
}

/// The page's title, once it is `want` (or after 5 s).
async fn title_becomes(ctrl: &BrowserController, want: &str) -> String {
    let mut title = String::new();
    for _ in 0..50 {
        title = eval(ctrl, "document.title").await.as_str().unwrap_or_default().to_string();
        if title == want {
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    title
}

async fn controller(state: &AppState, page: &str) -> BrowserController {
    state.browser_controllers.lock().await.get(page).cloned().unwrap_or_else(|| panic!("no controller for {page}"))
}

/// How many TCP connections on this host are `ESTABLISHED` to local port `port` (either side):
/// a page's own DevTools connection, plus one each for its download, site guard and popups tasks,
/// all to the same container browser port (#753).
async fn established_connections(port: u16) -> usize {
    let hex_port = format!("{port:04X}");
    let mut n = 0;
    for path in ["/proc/net/tcp", "/proc/net/tcp6"] {
        let Ok(text) = tokio::fs::read_to_string(path).await else { continue };
        for line in text.lines().skip(1) {
            let cols: Vec<&str> = line.split_whitespace().collect();
            let (Some(local), Some(remote), Some(st)) = (cols.get(1), cols.get(2), cols.get(3)) else { continue };
            let ends_in_port = |addr: &str| addr.rsplit_once(':').is_some_and(|(_, p)| p.eq_ignore_ascii_case(&hex_port));
            if *st == "01" && (ends_in_port(local) || ends_in_port(remote)) {
                n += 1;
            }
        }
    }
    n
}

/// [`established_connections`] once it holds still: the same count for half a second, read every
/// 50 ms, or the last count at 10 s. A page's download and popups connections come up in their own
/// tasks after the page opens, so a single read can miss them on a busy runner (31z).
async fn settled_connections(port: u16) -> usize {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    let mut last = established_connections(port).await;
    let mut steady = 0;
    while steady < 10 && tokio::time::Instant::now() < deadline {
        tokio::time::sleep(Duration::from_millis(50)).await;
        let n = established_connections(port).await;
        steady = if n == last { steady + 1 } else { 0 };
        last = n;
    }
    last
}

/// Reads [`established_connections`] every 50 ms until it is at most `bound`, up to 10 s; returns
/// the last count. A closed page's sockets go in their own time (its tasks are aborted, then the
/// browser closes its ends), so one read right after the close can still see them (31z); a leaked
/// connection never goes, so it is still over `bound` at the deadline.
async fn connections_drop_to(port: u16, bound: usize) -> usize {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    loop {
        let n = established_connections(port).await;
        if n <= bound || tokio::time::Instant::now() >= deadline {
            return n;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

#[tokio::test]
async fn a_sign_in_on_one_jobs_page_holds_only_that_page_and_the_teams_sign_ins_are_shared() {
    let tmp = tempdir().unwrap();
    let Some((endpoint, _chrome)) = start_chrome(&tmp.path().join("chrome")).await else { return };
    let state = container_state(&tmp, endpoint).await;
    let app = create_router(state.clone());
    let base = serve().await;
    let bot = first_bot(&state).await;
    let team = bot.id.clone();
    let chat_a = state.db.create_thread(&bot.id, "TikTok", "chat", None, "user").await.unwrap().id;
    let chat_b = state.db.create_thread(&bot.id, "Google Flow", "chat", None, "user").await.unwrap().id;
    let page_a = job_page_key(&team, &bot.id, &chat_a).unwrap();
    let page_b = job_page_key(&team, &bot.id, &chat_b).unwrap();

    // Job A opens the sign-in page on its own page (the site sets its cookie) and asks the person
    let token_a = turn_in_thread(&state, &bot, "turn-a", &chat_a, &team).await;
    let (text, is_error) = call(&app, &token_a, "browser_open", json!({"url": format!("{base}/signin")})).await.unwrap();
    assert!(!is_error, "{text}");
    let ctrl_a = controller(&state, &page_a).await;
    assert_eq!(title_becomes(&ctrl_a, "sign in").await, "sign in");
    let waiting = call(&app, &token_a, "request_human_step", json!({"kind": "sign_in", "reason": "Sign in to the test site"}));
    let mut step = None;
    for _ in 0..100 {
        step = state.db.list_human_steps(Some("open"), None, None).await.unwrap().pop();
        if step.is_some() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    let step = step.expect("a step was raised");
    assert_eq!(step.page(), Some(page_a.as_str()), "the step holds job A's page");
    assert_eq!(step.sandbox_target.as_deref(), Some(team.as_str()));
    let card = state.db.get_message(step.message_id.as_deref().unwrap()).await.unwrap().unwrap();
    let meta: Value = serde_json::from_str(card.meta.as_deref().unwrap()).unwrap();
    assert_eq!(meta["page_key"], json!(page_a), "the card points at the step's page");
    assert!(!waiting.is_finished(), "job A waits for the person");
    assert!(ctrl_a.is_held_for_person());

    // Job A's own browser calls are told to stop while the person signs in
    let a_route = format!("/api/sandbox/{team}/bot/{}/chat/{chat_a}", bot.id);
    let (status, body) = request(&app, "POST", &format!("{a_route}/navigate"), Some(json!({"url": format!("{base}/plain")}))).await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(body["error"], HELD_FOR_BOT);

    // Job B, same team, other chat: opens another site and acts on it meanwhile
    let b_route = format!("/api/sandbox/{team}/bot/{}/chat/{chat_b}", bot.id);
    let (status, body) = request(&app, "POST", &format!("{b_route}/navigate"), Some(json!({"url": format!("{base}/other")}))).await;
    assert_eq!(status, StatusCode::OK, "job B isn't held by job A's sign-in: {body}");
    let ctrl_b = controller(&state, &page_b).await;
    assert_eq!(title_becomes(&ctrl_b, "other").await, "other");
    let (status, body) = request(&app, "POST", &format!("{b_route}/act"), Some(json!({"action": "click", "text": "Go"}))).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(title_becomes(&ctrl_b, "clicked").await, "clicked", "job B's click landed on its own page");
    assert_eq!(title_becomes(&ctrl_a, "sign in").await, "sign in", "job A's page is untouched");
    assert!(!kwnt::server::human_steps::browser_is_held(&state, &team).await, "the team page isn't held either");
    // Each page is shown, none is a background tab Chrome doesn't paint: the team page, opened
    // first, still answers screenshots and navigations (they hung, 502, before job pages got
    // windows of their own in VM mode)
    let (status, body) = request(&app, "POST", &format!("/api/sandbox/{team}/navigate"), Some(json!({"url": format!("{base}/plain")}))).await;
    assert_eq!(status, StatusCode::OK, "the team page answers: {body}");
    let ctrl_team = controller(&state, &team).await;
    for (page, ctrl) in [("team", &ctrl_team), ("A", &ctrl_a), ("B", &ctrl_b)] {
        assert_eq!(eval(ctrl, "document.visibilityState").await, json!("visible"), "{page}'s page is painted");
    }

    // The live view and the tab strip show job A's page while the person has a step there
    let (status, pages) = request(&app, "GET", &format!("/api/sandbox/{team}/pages"), None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(pages["live"], json!(page_a), "{pages}");
    assert_eq!(pages["max_job_pages"], json!(MAX_JOB_PAGES_PER_TEAM));
    let listed: Vec<&Value> = pages["pages"].as_array().unwrap().iter().filter(|p| p["job"] == true).collect();
    assert_eq!(listed.len(), 2, "{pages}");
    let a = listed.iter().find(|p| p["page"] == json!(page_a)).unwrap();
    assert_eq!((a["held"].clone(), a["kwant_id"].clone(), a["chat_id"].clone()), (json!(true), json!(bot.id), json!(chat_a)));
    let b = listed.iter().find(|p| p["page"] == json!(page_b)).unwrap();
    assert_eq!(b["held"], json!(false));
    assert!(b["url"].as_str().unwrap().ends_with("/other"), "{b}");

    // One browser context per team: job A's sign-in cookie is on job B's page too
    request(&app, "POST", &format!("{b_route}/navigate"), Some(json!({"url": format!("{base}/plain")}))).await;
    assert_eq!(title_becomes(&ctrl_b, "plain").await, "plain");
    assert_eq!(eval(&ctrl_b, "document.cookie").await, json!("kwnt_session=signed-in"), "sign-ins are shared by the team's pages");

    // The person presses Done: job A carries on on its own page
    let (status, _) = request(&app, "POST", &format!("/api/human-steps/{}/done", step.id), None).await;
    assert_eq!(status, StatusCode::OK);
    let (text, is_error) = tokio::time::timeout(Duration::from_secs(20), waiting).await.expect("job A resumes").unwrap();
    assert!(!is_error && text.contains("done"), "{text}");
    assert!(!ctrl_a.is_held_for_person());
    let (status, body) = request(&app, "POST", &format!("{a_route}/navigate"), Some(json!({"url": format!("{base}/plain")}))).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(title_becomes(&ctrl_a, "plain").await, "plain");
    assert_eq!(title_becomes(&ctrl_b, "plain").await, "plain");
}

/// Opens a job page for chat `chat` of team `team`.
async fn open_page(state: &AppState, team: &str, chat: &str) -> Option<BrowserController> {
    get_or_create_browser_controller(state, &job_page_key(team, "k", chat).unwrap()).await
}

/// An open step on `page` (as a bot's sign-in there would leave it); its id.
async fn hold(state: &AppState, team: &str, page: &str, turn: &str) -> String {
    let place = HumanStepPlace {
        kwant_id: "k".into(),
        turn_id: Some(turn.into()),
        sandbox_target: Some(team.into()),
        page_key: Some(page.into()),
        ..Default::default()
    };
    state.db.create_human_step_with_card(&place, "other", "a step", None, None, "bot", |_| None).await.unwrap().0.id
}

async fn open_pages(state: &AppState) -> Vec<String> {
    let mut pages: Vec<String> = state.browser_controllers.lock().await.keys().filter(|k| browser_pages::is_job_page(k)).cloned().collect();
    pages.sort();
    pages
}

#[tokio::test]
async fn a_team_keeps_at_most_four_job_pages_and_never_closes_a_held_one_to_make_room() {
    let tmp = tempdir().unwrap();
    let Some((endpoint, _chrome)) = start_chrome(&tmp.path().join("chrome")).await else { return };
    let state = container_state(&tmp, endpoint).await;
    let team = first_bot(&state).await.id;
    assert_eq!(MAX_JOB_PAGES_PER_TEAM, 4);
    let key = |c: &str| job_page_key(&team, "k", c).unwrap();
    for chat in ["c0", "c1", "c2", "c3"] {
        open_page(&state, &team, chat).await.expect("a job page opens");
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    // c0 is the oldest but the person has a step there
    hold(&state, &team, &key("c0"), "t0").await;
    open_page(&state, &team, "c4").await.expect("a fifth job opens a page");
    assert_eq!(open_pages(&state).await, vec![key("c0"), key("c2"), key("c3"), key("c4")], "the oldest idle page made room");
    assert_eq!(state.browser_service.job_pages(&team).await.len(), MAX_JOB_PAGES_PER_TEAM);

    // Using a page again keeps it: c2 is used, so c3 is now the oldest idle one
    open_page(&state, &team, "c2").await.unwrap();
    open_page(&state, &team, "c5").await.unwrap();
    assert_eq!(open_pages(&state).await, vec![key("c0"), key("c2"), key("c4"), key("c5")]);

    // Every page held: no room, and none is closed
    for (chat, turn) in [("c2", "t2"), ("c4", "t4"), ("c5", "t5")] {
        hold(&state, &team, &key(chat), turn).await;
    }
    assert!(open_page(&state, &team, "c6").await.is_none(), "no page is closed while its step is open");
    assert_eq!(open_pages(&state).await, vec![key("c0"), key("c2"), key("c4"), key("c5")]);
    // The team page is never counted, and still opens
    assert!(get_or_create_browser_controller(&state, &team).await.is_some());
}

#[tokio::test]
async fn an_idle_job_page_is_closed_but_never_one_with_an_open_step() {
    let tmp = tempdir().unwrap();
    let Some((endpoint, _chrome)) = start_chrome(&tmp.path().join("chrome")).await else { return };
    let state = container_state(&tmp, endpoint).await;
    let team = first_bot(&state).await.id;
    let key = |c: &str| job_page_key(&team, "k", c).unwrap();
    open_page(&state, &team, "idle").await.unwrap();
    open_page(&state, &team, "held").await.unwrap();
    get_or_create_browser_controller(&state, &team).await.unwrap();
    hold(&state, &team, &key("held"), "t").await;

    assert_eq!(browser_pages::sweep_idle(&state, browser_pages::JOB_PAGE_IDLE).await, 0, "pages just used stay open");
    assert_eq!(browser_pages::sweep_idle(&state, Duration::ZERO).await, 1);
    assert_eq!(open_pages(&state).await, vec![key("held")], "the held page stays");
    assert!(state.browser_controllers.lock().await.contains_key(&team), "the team page is never swept");
    assert_eq!(state.browser_service.job_pages(&team).await.len(), 1);

    // A closed job page opens again on its next use
    open_page(&state, &team, "idle").await.expect("reopens");
    assert_eq!(open_pages(&state).await, vec![key("held"), key("idle")]);
}

/// A turn of bot `k` running in chat `chat` on `team`'s browser, until the guard drops.
fn running_turn(state: &AppState, team: &str, chat: &str) -> impl Drop {
    let scope = SelfScope {
        kwant_id: "k".into(),
        origin: Some(TurnOrigin { thread_id: Some(chat.into()), ..Default::default() }),
        sandbox_target: Some(team.into()),
        ..Default::default()
    };
    state.human_steps.track_turn(team, scope)
}

/// A job waiting minutes between two browser calls (a render) is the least recently used page,
/// but while its turn runs neither the cap nor the idle sweep closes it; once the turn ends both do.
#[tokio::test]
async fn a_page_whose_job_still_has_a_turn_running_is_never_closed() {
    let tmp = tempdir().unwrap();
    let Some((endpoint, _chrome)) = start_chrome(&tmp.path().join("chrome")).await else { return };
    let state = container_state(&tmp, endpoint).await;
    let team = first_bot(&state).await.id;
    let key = |c: &str| job_page_key(&team, "k", c).unwrap();

    // The cap: "busy" is the oldest page, its turn still runs, so the next oldest makes room
    let turn = running_turn(&state, &team, "busy");
    for chat in ["busy", "c1", "c2", "c3"] {
        open_page(&state, &team, chat).await.expect("a job page opens");
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    open_page(&state, &team, "c4").await.expect("a fifth job opens a page");
    assert_eq!(open_pages(&state).await, vec![key("busy"), key("c2"), key("c3"), key("c4")], "the running job's page is kept");
    // The turn ends: its page is now the oldest one that can close
    drop(turn);
    open_page(&state, &team, "c5").await.unwrap();
    assert_eq!(open_pages(&state).await, vec![key("c2"), key("c3"), key("c4"), key("c5")], "closed by the cap once its turn ended");

    // The idle sweep: a running job's page stays however long it was unused
    let turn = running_turn(&state, &team, "busy");
    open_page(&state, &team, "busy").await.unwrap();
    assert_eq!(browser_pages::sweep_idle(&state, Duration::ZERO).await, 3, "the other jobs' idle pages close");
    assert_eq!(open_pages(&state).await, vec![key("busy")]);
    drop(turn);
    assert_eq!(browser_pages::sweep_idle(&state, Duration::ZERO).await, 1, "closed once its turn ended");
    assert!(open_pages(&state).await.is_empty());
}

/// Every job page held: a fifth job's bot call waits for a page to free up, gets one as soon as
/// a step ends, and when none frees up in time is told plainly that the pages are in use (not
/// that the browser is broken), through the HTTP route and the bot's browser tool alike.
#[tokio::test]
async fn a_job_that_finds_every_page_in_use_waits_for_one_then_is_told_plainly() {
    let tmp = tempdir().unwrap();
    let Some((endpoint, _chrome)) = start_chrome(&tmp.path().join("chrome")).await else { return };
    let state = container_state(&tmp, endpoint).await;
    let app = create_router(state.clone());
    let base = serve().await;
    let bot = first_bot(&state).await;
    let team = bot.id.clone();
    let key = |c: &str| job_page_key(&team, "k", c).unwrap();
    let mut steps = Vec::new();
    for chat in ["c0", "c1", "c2", "c3"] {
        open_page(&state, &team, chat).await.expect("a job page opens");
        steps.push(hold(&state, &team, &key(chat), chat).await);
    }
    let navigate = format!("/api/sandbox/{}/navigate", key("c4").replace('#', "%23"));
    let body = json!({"url": format!("{base}/plain")});

    // No page frees up in time: a clear 503, after the wait
    state.job_pages.set_wait(Duration::from_millis(1500));
    let started = std::time::Instant::now();
    let (status, v) = request(&app, "POST", &navigate, Some(body.clone())).await;
    assert!(started.elapsed() >= Duration::from_millis(1400), "it waited for a page first");
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(v["code"], PAGES_FULL);
    assert_eq!(v["error"], browser_pages::pages_full_message());
    assert!(v["error"].as_str().unwrap().contains("nothing is wrong with the browser"));
    assert_eq!(open_pages(&state).await.len(), MAX_JOB_PAGES_PER_TEAM, "nothing was closed");
    assert!(state.browser_service.job_pages(&team).await.iter().all(|(k, _)| *k != key("c4")), "the refused page isn't counted");

    // The bot's own browser tool says the same
    let token = turn_in_thread(&state, &bot, "turn-9", "t9", &team).await;
    let (text, is_error) = call(&app, &token, "browser_open", json!({"url": format!("{base}/plain")})).await.unwrap();
    assert!(is_error);
    assert_eq!(text, browser_pages::pages_full_message());

    // A step ends while the call waits: the page that freed up makes room, and the call goes on
    state.job_pages.set_wait(Duration::from_secs(20));
    let waiting = {
        let (app, navigate, body) = (app.clone(), navigate.clone(), body.clone());
        tokio::spawn(async move { request(&app, "POST", &navigate, Some(body)).await })
    };
    tokio::time::sleep(Duration::from_millis(1200)).await;
    assert!(!waiting.is_finished(), "still waiting while every page is held");
    assert!(state.db.close_human_step(&steps[0], "done").await.unwrap());
    let (status, v) = tokio::time::timeout(Duration::from_secs(10), waiting).await.expect("answers soon after").unwrap();
    assert_eq!(status, StatusCode::OK, "{v}");
    assert_eq!(open_pages(&state).await, vec![key("c1"), key("c2"), key("c3"), key("c4")], "the freed page made room");
}

/// Six jobs opening pages at the same moment never leave more than four open: making room and
/// opening are one step per team.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn jobs_opening_pages_at_once_never_pass_the_cap() {
    let tmp = tempdir().unwrap();
    let Some((endpoint, _chrome)) = start_chrome(&tmp.path().join("chrome")).await else { return };
    let state = container_state(&tmp, endpoint.clone()).await;
    let team = first_bot(&state).await.id;
    get_or_create_browser_controller(&state, &team).await.expect("the team page opens");
    let page_targets = || async {
        let targets: Value = reqwest::get(format!("{endpoint}/json/list")).await.unwrap().json().await.unwrap();
        targets.as_array().unwrap().iter().filter(|t| t["type"] == "page").count()
    };
    let before = page_targets().await;
    let opens: Vec<_> = (0..6)
        .map(|i| {
            let (state, team) = (state.clone(), team.clone());
            tokio::spawn(async move { open_page(&state, &team, &format!("j{i}")).await.is_some() })
        })
        .collect();
    for open in opens {
        assert!(open.await.unwrap(), "each job gets a page (an older idle one makes room)");
    }
    assert_eq!(open_pages(&state).await.len(), MAX_JOB_PAGES_PER_TEAM);
    assert_eq!(state.browser_service.job_pages(&team).await.len(), MAX_JOB_PAGES_PER_TEAM);
    assert_eq!(page_targets().await, before + MAX_JOB_PAGES_PER_TEAM, "four job pages in the browser, the others closed");
}

/// 31b: the browser's pages, across every team, stay under the container's process budget. A job
/// opening a page that would pass it first closes the least recently used idle job page of any
/// team (never a team page, a held one, or one in use); when nothing can close, the job is told
/// plainly the browser is full, through the HTTP route and the bot's browser tool alike, instead
/// of Chrome failing to start the page's processes.
#[tokio::test]
async fn a_page_past_the_browsers_process_budget_closes_the_oldest_idle_job_page_of_any_team() {
    let tmp = tempdir().unwrap();
    let Some((endpoint, _chrome)) = start_chrome(&tmp.path().join("chrome")).await else { return };
    let state = container_state(&tmp, endpoint).await;
    let app = create_router(state.clone());
    let bot = first_bot(&state).await;
    // two teams: each bot is a sandbox target with its own browser context
    let b = state.db.list_kwants(&SpaceId::default_space()).await.unwrap().remove(1).id;
    let a = bot.id.clone();
    assert_ne!(a, b);
    let key = |team: &str, c: &str| job_page_key(team, "k", c).unwrap();
    // room for two teams' pages and two job pages
    state.job_pages.set_page_budget(2 * browser_pages::TEAM_PAGE_PIDS + 2 * browser_pages::JOB_PAGE_PIDS);
    open_page(&state, &a, "a1").await.expect("a job page opens, with its team's page");
    tokio::time::sleep(Duration::from_millis(20)).await;
    open_page(&state, &b, "b1").await.expect("another team's job page opens");
    tokio::time::sleep(Duration::from_millis(20)).await;
    let mut both = vec![key(&a, "a1"), key(&b, "b1")];
    both.sort();
    assert_eq!(open_pages(&state).await, both);

    // A third job page would pass the budget: the oldest idle job page closes, though it's
    // another team's, and well under that team's own cap of four
    open_page(&state, &b, "b2").await.expect("a third job page opens");
    assert_eq!(open_pages(&state).await, vec![key(&b, "b1"), key(&b, "b2")], "team a's idle page made room");
    // A job page's team page is the browser service's (its context), not a kwant controller:
    // check it where the budget counts it, as a live page in the browser
    let team_pages_kept = || async {
        let live: Vec<String> = state.browser_service.live_pages().await.into_iter().map(|(k, _)| k).collect();
        live.contains(&a) && live.contains(&b)
    };
    assert!(team_pages_kept().await, "team pages are never closed to make room");

    // Every job page held: nothing closes, and the job is told the browser is full
    hold(&state, &b, &key(&b, "b1"), "t1").await;
    hold(&state, &b, &key(&b, "b2"), "t2").await;
    let refused = kwnt::server::routes::open_page(&state, &key(&a, "a2"), Duration::from_millis(300)).await;
    assert_eq!(refused.err(), Some(PageRefusal::BrowserFull));
    state.job_pages.set_wait(Duration::from_millis(300));
    let navigate = format!("/api/sandbox/{}/navigate", key(&a, "a3").replace('#', "%23"));
    let (status, v) = request(&app, "POST", &navigate, Some(json!({"url": "about:blank"}))).await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(v["code"], BROWSER_FULL);
    assert_eq!(v["error"], browser_pages::browser_full_message());
    let token = turn_in_thread(&state, &bot, "turn-9", "t9", &a).await;
    let (text, is_error) = call(&app, &token, "browser_open", json!({"url": "about:blank"})).await.unwrap();
    assert!(is_error);
    assert_eq!(text, browser_pages::browser_full_message());
    assert_eq!(open_pages(&state).await, vec![key(&b, "b1"), key(&b, "b2")], "nothing was closed");
    assert!(team_pages_kept().await, "nor a team page");
    assert!(state.browser_service.job_pages(&a).await.is_empty(), "the refused pages aren't counted");
}

/// The repro for #753: before the fix, closing a job page never dropped its controller from
/// [`kwnt::sandbox::service::BrowserService`]'s own cache, so its DevTools, download and popups
/// connections to the container browser outlived the page and piled up, one finished job at a
/// time, forever. With the fix, closing a page drops its connections with it.
#[tokio::test]
async fn finished_jobs_dont_leave_their_browser_connections_behind() {
    let tmp = tempdir().unwrap();
    let Some((endpoint, _chrome)) = start_chrome(&tmp.path().join("chrome")).await else { return };
    let port: u16 = endpoint.rsplit(':').next().unwrap().parse().unwrap();
    let state = container_state(&tmp, endpoint).await;
    let team = first_bot(&state).await.id;
    let base = serve().await;

    get_or_create_browser_controller(&state, &team).await.expect("the team page opens");
    let before = settled_connections(port).await;

    // Ten jobs, one after another, each opening its own page (its own DevTools connection, plus
    // a download and a popups connection started the same way a live job's would be) and then
    // closed the way a finished job's page is: never held, never watched.
    for i in 0..10 {
        let chat = format!("c{i}");
        let ctrl = open_page(&state, &team, &chat).await.expect("a job page opens");
        ctrl.navigate(&format!("{base}/plain")).await.unwrap();
        assert_eq!(title_becomes(&ctrl, "plain").await, "plain");
        browser_pages::close_page(&state, &job_page_key(&team, "k", &chat).unwrap()).await;
    }
    let after = connections_drop_to(port, before + 3).await;
    assert!(
        after <= before + 3,
        "ten finished jobs left the browser with {after} connections 10 s after the last closed, started with {before}: \
         closing a page should drop its connections, not add to them forever"
    );
}

/// 31z: a page closed while its job still holds its controller (as the #753 repro above does,
/// and a bot's call in flight would) lets go of every connection once the job lets go of it. The
/// browser ends the page's own connections at once; its download task then went back to retrying
/// the closed page every 3 s forever, never seeing its job gone, and its copy of the controller
/// kept the popups watcher's browser connection open with it: one connection and one task left
/// behind for good, the 14-for-10 flake in CI.
#[tokio::test]
async fn a_page_closed_while_its_job_holds_it_drops_its_connections_once_the_job_lets_go() {
    let tmp = tempdir().unwrap();
    let Some((endpoint, _chrome)) = start_chrome(&tmp.path().join("chrome")).await else { return };
    let port: u16 = endpoint.rsplit(':').next().unwrap().parse().unwrap();
    let state = container_state(&tmp, endpoint).await;
    let team = first_bot(&state).await.id;
    let base = serve().await;
    get_or_create_browser_controller(&state, &team).await.expect("the team page opens");
    let before = settled_connections(port).await;

    let ctrl = open_page(&state, &team, "c").await.expect("a job page opens");
    ctrl.navigate(&format!("{base}/plain")).await.unwrap();
    assert_eq!(title_becomes(&ctrl, "plain").await, "plain");
    browser_pages::close_page(&state, &job_page_key(&team, "k", "c").unwrap()).await;
    // The browser closes the page's own two connections (its DevTools and download ones) while
    // the job still holds it; only the popups watcher's browser connection is left
    let closing = connections_drop_to(port, before + 2).await;
    assert!(closing <= before + 2, "the closed page's own connections stay open: {closing}, started with {before}");
    // and the download task sees its connection end before the job lets go
    tokio::time::sleep(Duration::from_millis(500)).await;

    drop(ctrl);
    let after = connections_drop_to(port, before).await;
    assert!(after <= before, "a closed page left {after} connections behind after its job let go, started with {before}");
}

/// An idle page's connections close; one that's watched (a live view open on it) or held (a step
/// open for the person) never does, even past the same idle time.
#[tokio::test]
async fn an_idle_pages_connections_close_but_never_one_watched_or_held() {
    let tmp = tempdir().unwrap();
    let Some((endpoint, _chrome)) = start_chrome(&tmp.path().join("chrome")).await else { return };
    let state = container_state(&tmp, endpoint).await;
    let team = first_bot(&state).await.id;
    let key = |c: &str| job_page_key(&team, "k", c).unwrap();
    open_page(&state, &team, "idle").await.unwrap();
    open_page(&state, &team, "held").await.unwrap();
    let watched = open_page(&state, &team, "watched").await.unwrap();
    hold(&state, &team, &key("held"), "t").await;
    let _watching = watched.watch();

    assert_eq!(browser_pages::close_idle_connections(&state, Duration::ZERO).await, 1);
    let cached = state.browser_controllers.lock().await;
    assert!(!cached.contains_key(&key("idle")), "the idle page's connections are gone");
    assert!(cached.contains_key(&key("held")), "a page held for the person keeps its connections");
    assert!(cached.contains_key(&key("watched")), "a watched page keeps its connections");
}

/// Closing a page's connections keeps the page itself, its place, and its sign-in; the next use
/// reconnects lazily to the very same page, not a fresh one.
#[tokio::test]
async fn closing_a_pages_connections_keeps_its_sign_in_and_reconnects_the_same_page() {
    let tmp = tempdir().unwrap();
    let Some((endpoint, _chrome)) = start_chrome(&tmp.path().join("chrome")).await else { return };
    let state = container_state(&tmp, endpoint).await;
    let team = first_bot(&state).await.id;
    let key = job_page_key(&team, "k", "c").unwrap();
    let base = serve().await;
    let ctrl = open_page(&state, &team, "c").await.unwrap();
    ctrl.navigate(&format!("{base}/signin")).await.unwrap();
    assert_eq!(title_becomes(&ctrl, "sign in").await, "sign in");

    assert_eq!(browser_pages::close_idle_connections(&state, Duration::ZERO).await, 1);
    assert!(!state.browser_controllers.lock().await.contains_key(&key), "its connections are gone from the cache");

    let ctrl2 = open_page(&state, &team, "c").await.expect("its next use reconnects");
    assert_eq!(eval(&ctrl2, "document.cookie").await, json!("kwnt_session=signed-in"), "its sign-in survived");
    assert_eq!(eval(&ctrl2, "document.title").await, json!("sign in"), "the same page, not a fresh one");
}

#[tokio::test]
async fn in_local_mode_a_job_page_is_a_window_of_the_same_chrome_with_its_sign_ins() {
    if computer::local_chrome_path().is_none() {
        skip::skip_or_fail("Chrome or Chromium", skip::CHROME_FIX);
        return;
    }
    let tmp = tempdir().unwrap();
    let db = Database::init(tmp.path()).await.unwrap();
    db.set_setting(computer::SETTING_KEY, "local").await.unwrap();
    let config = Config { data_dir: tmp.path().to_path_buf(), ..Config::default() };
    let state = AppState::new(config.clone(), db, ToolRunner::new(tmp.path().join("workspace")), PermissionBroker::new(), LayaRouter::new(config.laya_url.clone()));
    let _chrome = ChromeGuard(tmp.path().join("local-chrome"));
    let base = serve().await;
    let team = first_bot(&state).await.id;
    let a = open_page(&state, &team, "a").await.expect("local Chrome job page");
    let b = open_page(&state, &team, "b").await.expect("a second job page");
    a.navigate(&format!("{base}/signin")).await.unwrap();
    b.navigate(&format!("{base}/other")).await.unwrap();
    assert_eq!(title_becomes(&a, "sign in").await, "sign in");
    assert_eq!(title_becomes(&b, "other").await, "other", "each job has its own page");
    assert_eq!(eval(&b, "document.cookie").await, json!("kwnt_session=signed-in"), "the same Chrome, so the same sign-ins");
}

// ---- 31z PROBE (never merges): the #753 repro's connection counts over time, on a CI runner ----

/// Each connection to `port` on this host, by side: `k` = kwnt's end (remote is the browser
/// port), `c` = the browser's end; with its TCP state (01 ESTABLISHED, 08 CLOSE_WAIT, ...).
fn probe_sockets(port: u16) -> String {
    let mut out: Vec<String> = vec![];
    for path in ["/proc/net/tcp", "/proc/net/tcp6"] {
        let Ok(text) = std::fs::read_to_string(path) else { continue };
        for line in text.lines().skip(1) {
            let cols: Vec<&str> = line.split_whitespace().collect();
            let (Some(local), Some(remote), Some(st)) = (cols.get(1), cols.get(2), cols.get(3)) else { continue };
            let p = |addr: &str| addr.rsplit_once(':').and_then(|(_, p)| u16::from_str_radix(p, 16).ok()).unwrap_or(0);
            if p(local) == port {
                out.push(format!("c{}:{st}", p(remote)));
            } else if p(remote) == port {
                out.push(format!("k{}:{st}", p(local)));
            }
        }
    }
    out.sort();
    out.join(" ")
}

/// Runs of equal values in a timeline as `value@ms`.
fn probe_runs(samples: &[(u128, usize)]) -> String {
    let mut out = vec![];
    let mut last = None;
    for &(ms, n) in samples {
        if last != Some(n) {
            out.push(format!("{n}@{ms}"));
            last = Some(n);
        }
    }
    out.join(" ")
}

async fn probe_once(run: usize, load: bool) -> String {
    let stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let spinners: Vec<_> = (0..if load { 4 } else { 0 })
        .map(|_| {
            let stop = stop.clone();
            std::thread::spawn(move || {
                let mut x = 0u64;
                while !stop.load(std::sync::atomic::Ordering::Relaxed) {
                    x = x.wrapping_mul(6364136223846793005).wrapping_add(1);
                }
                x
            })
        })
        .collect();
    let tmp = tempdir().unwrap();
    let Some((endpoint, _chrome)) = start_chrome(&tmp.path().join("chrome")).await else { return "no chrome".into() };
    let port: u16 = endpoint.rsplit(':').next().unwrap().parse().unwrap();
    let state = container_state(&tmp, endpoint).await;
    let team = first_bot(&state).await.id;
    let base = serve().await;

    get_or_create_browser_controller(&state, &team).await.expect("the team page opens");
    let t = std::time::Instant::now();
    let mut before_line = vec![];
    let mut at200 = None;
    let mut sockets200 = String::new();
    while t.elapsed() < Duration::from_millis(3000) {
        let ms = t.elapsed().as_millis();
        let n = established_connections(port).await;
        if at200.is_none() && ms >= 200 {
            at200 = Some(n);
            sockets200 = probe_sockets(port);
        }
        before_line.push((ms, n));
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    let before200 = at200.unwrap();
    let settled = established_connections(port).await;
    let settled_sockets = probe_sockets(port);

    let mut open_counts = vec![];
    for i in 0..10 {
        let chat = format!("c{i}");
        let ctrl = open_page(&state, &team, &chat).await.expect("a job page opens");
        ctrl.navigate(&format!("{base}/plain")).await.unwrap();
        assert_eq!(title_becomes(&ctrl, "plain").await, "plain");
        open_counts.push(established_connections(port).await);
        browser_pages::close_page(&state, &job_page_key(&team, "k", &chat).unwrap()).await;
    }
    let t = std::time::Instant::now();
    let mut after_line = vec![];
    let mut at300 = None;
    let mut sockets300 = String::new();
    while t.elapsed() < Duration::from_millis(6000) {
        let ms = t.elapsed().as_millis();
        let n = established_connections(port).await;
        if at300.is_none() && ms >= 300 {
            at300 = Some(n);
            sockets300 = probe_sockets(port);
        }
        after_line.push((ms, n));
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    let after300 = at300.unwrap();
    let final_n = established_connections(port).await;
    let final_sockets = probe_sockets(port);
    stop.store(true, std::sync::atomic::Ordering::Relaxed);
    for s in spinners {
        let _ = s.join();
    }
    let old_test = if after300 <= before200 + 3 { "pass" } else { "FAIL" };
    format!(
        "run {run} load={load}: old test {old_test} (before@200={before200}, after@300={after300}); settled before@3s={settled}, final after@6s={final_n}\n\
         \x20 before: {}\n\x20 open counts: {open_counts:?}\n\x20 after:  {}\n\
         \x20 sockets before@200: {sockets200}\n\x20 sockets before@3s:  {settled_sockets}\n\
         \x20 sockets after@300:  {sockets300}\n\x20 sockets after@6s:   {final_sockets}",
        probe_runs(&before_line),
        probe_runs(&after_line),
    )
}

#[tokio::test]
async fn zz_31z_probe_connection_counts_over_time() {
    let mut report = vec![];
    for run in 0..20 {
        report.push(probe_once(run, run >= 10).await);
    }
    panic!("31z PROBE REPORT (deliberate failure, never merges)\n{}", report.join("\n"));
}
