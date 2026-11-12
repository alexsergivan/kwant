//! One browser page per job (0.9.3). Two jobs of one team, each in its own chat, get their own
//! pages in the team's browser: a sign-in the person is asked to do on one job's page holds only
//! that page, while the other job opens and acts on another site; both pages share the team's
//! sign-ins (cookies). A team keeps at most `MAX_JOB_PAGES_PER_TEAM` job pages (the least recently
//! used idle one makes room, never one a step holds) and an idle page is closed, never a held one.
//!
//! The browser parts run a real windowless Chrome, as the team's container browser (an external
//! DevTools endpoint, where a job page is a new page in the team's browser context) and in Local
//! mode (a new window of the person's Chrome). Skipped when no Chrome is installed.

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
        routes::{get_or_create_browser_controller, HELD_FOR_BOT},
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
        eprintln!("skipped: no Chrome or Chromium installed");
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

/// An open step on `page` (as a bot's sign-in there would leave it).
async fn hold(state: &AppState, team: &str, page: &str, turn: &str) {
    let place = HumanStepPlace {
        kwant_id: "k".into(),
        turn_id: Some(turn.into()),
        sandbox_target: Some(team.into()),
        page_key: Some(page.into()),
        ..Default::default()
    };
    state.db.create_human_step_with_card(&place, "other", "a step", None, None, "bot", |_| None).await.unwrap();
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

#[tokio::test]
async fn in_local_mode_a_job_page_is_a_window_of_the_same_chrome_with_its_sign_ins() {
    if computer::local_chrome_path().is_none() {
        eprintln!("skipped: no Chrome or Chromium installed");
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
