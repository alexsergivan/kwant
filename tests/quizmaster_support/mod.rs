//! Shared by the pa3 tests: a stub source server, a kwant with the bot's own tools endpoint, and
//! the stub writer for "make a pub-quiz app for Friday".
#![allow(dead_code)]

use axum::{
    body::{to_bytes, Body},
    extract::{Path, State},
    http::{Request, StatusCode},
    response::Html,
    routing::get,
    Router,
};
use kwnt::{
    broker::PermissionBroker,
    config::Config,
    db::{models::Kwant, spaces::SpaceId, Database},
    quizmaster::source_check::allow_local_http_for_tests,
    router::LayaRouter,
    server::{
        continuity::{SelfScope, TurnOrigin},
        create_router, mcp_runtime, quizmaster_tools, AppState,
    },
    tools::ToolRunner,
};
use serde_json::{json, Value};
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};
use tempfile::{tempdir, TempDir};
use tower::ServiceExt;

/// Pages for the example's four sources, served on 127.0.0.1, counting fetches per page.
#[derive(Clone, Default)]
pub struct Pages {
    pub hits: Arc<Mutex<HashMap<String, usize>>>,
}

pub const PAGES: [(&str, &str); 4] = [
    ("definitely-maybe", "Definitely Maybe is the debut studio album by English rock band Oasis, released on 29 August 1994"),
    ("ballon-dor-1998", "The 1998 Ballon d'Or. Zinedine Zidane won the award[1] ahead of Davor Šuker."),
    ("berlin-wall", "The fall of the Berlin Wall on 9 November 1989 was a pivotal event in world history."),
    ("eiffel-tower", "The Eiffel Tower is a wrought-iron lattice tower on the Champ de Mars in Paris, France."),
];

async fn page(State(p): State<Pages>, Path(slug): Path<String>) -> Result<Html<String>, StatusCode> {
    *p.hits.lock().unwrap().entry(slug.clone()).or_default() += 1;
    let body = PAGES.iter().find(|(s, _)| *s == slug).map(|(_, t)| *t).ok_or(StatusCode::NOT_FOUND)?;
    Ok(Html(format!("<html><head><title>{slug}</title><script>var x = 1;</script></head><body><p>{body}</p></body></html>")))
}

pub async fn start_pages() -> (String, Pages) {
    allow_local_http_for_tests();
    let pages = Pages::default();
    let app = Router::new().route("/wiki/:slug", get(page)).with_state(pages.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    (format!("http://{addr}/wiki"), pages)
}

pub struct Env {
    pub app: Router,
    pub state: AppState,
    pub bot: Kwant,
    pub other: Kwant,
    pub _tmp: TempDir,
}

pub async fn setup(pockets_on: bool) -> Env {
    let tmp = tempdir().unwrap();
    let db = Database::init(tmp.path()).await.unwrap();
    let config = Config { data_dir: tmp.path().to_path_buf(), ..Config::default() };
    let state = AppState::new(
        config.clone(),
        db,
        ToolRunner::new(tmp.path().join("workspace")),
        PermissionBroker::new(),
        LayaRouter::new(config.laya_url.clone()),
    );
    if pockets_on {
        state.db.set_setting(quizmaster_tools::SETTING, "on").await.unwrap();
    }
    let kwants = state.db.list_kwants(&SpaceId::default_space()).await.unwrap();
    Env { app: create_router(state.clone()), bot: kwants[0].clone(), other: kwants[1].clone(), state, _tmp: tmp }
}

pub async fn rpc(e: &Env, token: &str, method: &str, params: Value) -> Value {
    let body = json!({"jsonrpc": "2.0", "id": 1, "method": method, "params": params});
    let req = Request::builder()
        .method("POST")
        .uri("/api/mcp/self")
        .header("content-type", "application/json")
        .header("authorization", format!("Bearer {token}"))
        .body(Body::from(body.to_string()))
        .unwrap();
    let resp = e.app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    serde_json::from_slice(&to_bytes(resp.into_body(), usize::MAX).await.unwrap()).unwrap()
}

pub async fn tool(e: &Env, token: &str, name: &str, args: Value) -> (String, bool) {
    let v = rpc(e, token, "tools/call", json!({"name": name, "arguments": args})).await;
    (v["result"]["content"][0]["text"].as_str().unwrap_or("").to_string(), v["result"]["isError"].as_bool().unwrap_or(false))
}

pub async fn tool_names(e: &Env, token: &str) -> Vec<String> {
    let v = rpc(e, token, "tools/list", json!({})).await;
    v["result"]["tools"].as_array().unwrap().iter().map(|t| t["name"].as_str().unwrap().to_string()).collect()
}

pub async fn turn(e: &Env, bot: &Kwant, turn_id: &str) -> String {
    let scope = SelfScope {
        kwant_id: bot.id.clone(),
        kwant_name: bot.name.clone(),
        origin: Some(TurnOrigin::default()),
        turn_id: Some(turn_id.into()),
        ..Default::default()
    };
    mcp_runtime::prepare_turn_with_self(&e.state, bot, Some(turn_id.into()), None, Some(scope)).await.unwrap().1
}

/// The card's JSON line.
pub fn card(text: &str) -> Value {
    let line = text.lines().find_map(|l| l.strip_prefix("card: ")).unwrap_or_else(|| panic!("no card in {text}"));
    serde_json::from_str(line).unwrap()
}

/// The stub writer: what a model does with "make a pub-quiz app for Friday" once it has the
/// template. It keeps the template's questions, names the Friday, and points every source at a
/// page it "read" (the stub server).
pub fn write_friday_quiz(template: &str, pages: &str) -> Value {
    let start = template.find("Template (").expect("the template is there");
    let json_start = start + template[start..].find('{').unwrap();
    let json_end = template.rfind('}').unwrap() + 1;
    let mut spec: Value = serde_json::from_str(&template[json_start..json_end]).expect("the template is JSON");
    spec["title"] = json!("Friday Pub Quiz");
    spec["when"] = json!("2026-11-27T20:00:00+00:00");
    let slugs: Vec<&str> = PAGES.iter().map(|(s, _)| *s).collect();
    let mut i = 0;
    for round in spec["rounds"].as_array_mut().unwrap() {
        for q in round["questions"].as_array_mut().unwrap() {
            q["source"]["url"] = json!(format!("{pages}/{}", slugs[i]));
            i += 1;
        }
    }
    assert_eq!(i, 4);
    spec
}


/// A turn of `bot` in a given chat.
pub async fn turn_in(e: &Env, bot: &Kwant, turn_id: &str, origin: TurnOrigin) -> String {
    let scope = SelfScope {
        kwant_id: bot.id.clone(),
        kwant_name: bot.name.clone(),
        origin: Some(origin),
        turn_id: Some(turn_id.into()),
        ..Default::default()
    };
    mcp_runtime::prepare_turn_with_self(&e.state, bot, Some(turn_id.into()), None, Some(scope)).await.unwrap().1
}

/// A Ready "Friday Pub Quiz" made through the tools; returns its id.
pub async fn ready_friday_quiz(e: &Env, token: &str, pages_url: &str) -> (String, Value) {
    let (template, err) = tool(e, token, "pocket_create", json!({})).await;
    assert!(!err, "{template}");
    let spec = write_friday_quiz(&template, pages_url);
    let (text, err) = tool(e, token, "pocket_create", json!({"spec": spec})).await;
    assert!(!err, "{text}");
    let c = card(&text);
    assert_eq!(c["state"], json!("ready"), "{text}");
    (c["id"].as_str().unwrap().to_string(), spec)
}

/// The default space id, as the tools store it.
pub fn space() -> String {
    SpaceId::default_space().as_str().to_string()
}

/// Turns pocket apps on (the tests that start with them off flip them here).
pub async fn pockets_on(e: &Env) {
    e.state.db.set_setting(quizmaster_tools::SETTING, "on").await.unwrap();
}
