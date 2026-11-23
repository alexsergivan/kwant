//! pa4c-server B: a pocket whose results were posted (pa7) shows once in Inbox › Done for 7
//! days, in the same list and unseen count as the other Done items. Names and scores only; a
//! deleted pocket drops out. Brief:
//! /home/maus/shared/ship-software-safely/autonomy/pocket-apps/pa4c-server/brief.md.

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
    db::{
        models::{Kwant, NewKwant},
        spaces::SpaceId,
        Database,
    },
    pocket::{
        self,
        kits::Viewer,
        listener,
        schedule::{self, Origin},
        CreateRequest, JoinRequest,
    },
    router::LayaRouter,
    server::{
        auth::{ApiToken, Require},
        away, create_router, AppState,
    },
    tools::ToolRunner,
};
use serde_json::{json, Value};
use std::net::SocketAddr;
use std::sync::Arc;
use tempfile::{tempdir, TempDir};
use tower::ServiceExt;

const TOKEN: &str = "test-api-token-0123456789";
/// A guest name the Done item must never carry.
const CANARY: &str = "Zyxqv Canary";

struct T {
    main: Router,
    state: AppState,
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
    state.db.set_setting("timezone", "UTC").await.unwrap();
    listener::reconcile(&state).await;
    T { main: create_router(state.clone()), state, _tmp: tmp }
}

async fn bot(t: &T, name: &str) -> Kwant {
    let input = NewKwant {
        name: name.into(),
        title: "Quizmaster".into(),
        persona_prompt: String::new(),
        soul: String::new(),
        avatar_sprite: "s".into(),
        driver_type: "cli".into(),
        preferred_model: "claude".into(),
    };
    t.state.db.create_kwant(&SpaceId::default_space(), &input).await.unwrap()
}

async fn send(app: &Router, method: &str, uri: &str) -> (StatusCode, Value) {
    let req = Request::builder().method(method).uri(uri).header("host", "127.0.0.1:8799");
    let mut req = req.body(Body::empty()).unwrap();
    req.extensions_mut().insert(ConnectInfo("127.0.0.1:50000".parse::<SocketAddr>().unwrap()));
    let resp = app.clone().oneshot(req).await.unwrap();
    let status = resp.status();
    let bytes = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    (status, serde_json::from_slice(&bytes).unwrap_or(Value::Null))
}

async fn make(t: &T, title: &str, k: &Kwant) -> String {
    let origin = Some(Origin { kwant_id: k.id.clone(), channel_id: None, thread_id: None });
    let (s, _) = pocket::create(&t.state, CreateRequest { kit: "test".into(), spec: json!({ "title": title }), origin, ..Default::default() }).await.unwrap();
    s.row.code.clone()
}

/// Joins and taps `taps` times.
async fn play(t: &T, code: &str, name: &str, taps: usize) {
    let (g, _) = pocket::join(&t.state, code, JoinRequest { name: name.into(), avatar: None, team: None, pin: None }).await.unwrap();
    for _ in 0..taps {
        pocket::act(&t.state, code, &Viewer::Guest(g.id.clone()), "tap", &json!({})).await.unwrap();
    }
}

/// Ends the pocket and posts its results at `at` (pa7's sweep).
async fn finish(t: &T, code: &str, at: chrono::DateTime<Utc>) {
    pocket::end(&t.state, code).await.unwrap();
    assert_eq!(schedule::run_once(&t.state, at).await.results, vec![code.to_string()]);
}

async fn inbox(t: &T) -> Value {
    let (status, body) = send(&t.main, "GET", "/api/inbox").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    body
}

fn pockets(body: &Value) -> Vec<Value> {
    body["items"].as_array().unwrap().iter().filter(|i| i["kind"] == "pocket").cloned().collect()
}

/// The results post pa7 left in the bot's chat.
async fn results_post(t: &T, k: &Kwant, code: &str) -> kwnt::db::models::Message {
    t.state
        .db
        .list_messages(&k.id, 100)
        .await
        .unwrap()
        .into_iter()
        .find(|m| m.meta.as_deref().and_then(|s| serde_json::from_str::<Value>(s).ok()).is_some_and(|v| v["reason"] == "results" && v["code"] == json!(code)))
        .expect("the results post")
}

#[tokio::test]
async fn a_posted_pocket_shows_once_in_done_with_its_winner_and_source() {
    let t = setup().await;
    let k = bot(&t, "Quizmaster").await;
    let code = make(&t, "Pub quiz", &k).await;
    play(&t, &code, CANARY, 3).await;
    play(&t, &code, "Mia", 1).await;
    let before = inbox(&t).await["counts"]["badge"].as_i64().unwrap();
    finish(&t, &code, Utc::now()).await;
    // A second sweep posts nothing and adds nothing
    assert!(schedule::run_once(&t.state, Utc::now()).await.results.is_empty());

    let body = inbox(&t).await;
    let items = pockets(&body);
    assert_eq!(items.len(), 1, "once: {items:?}");
    let i = &items[0];
    assert_eq!(i["id"], json!(format!("pocket:{code}")));
    assert_eq!(i["title"], json!("Pub quiz: Team 1 win"));
    assert_eq!(i["group"], json!("finished"), "the same list as other Done items");
    assert_eq!(i["kwant_id"], json!(k.id));
    assert_eq!(i["meta"]["players"], json!(2));
    assert!(i["meta"]["ended_at"].is_string(), "{i}");
    assert_eq!(i["winner"], json!({ "name": "Team 1", "points": 3 }));
    assert_eq!(i["unread"], json!(true));

    let post = results_post(&t, &k, &code).await;
    let src = &i["source"];
    assert_eq!((src["kind"].as_str(), src["code"].as_str(), src["kwant_id"].as_str()), (Some("pocket"), Some(code.as_str()), Some(k.id.as_str())));
    assert_eq!(src["message_id"], json!(post.id));
    assert_eq!(i["link"]["direct_kwant_id"], json!(k.id), "opens the chat with the results post: {i}");
    assert!(!i.to_string().contains("Zyxqv"), "names and scores only: {i}");

    // Counted like every other Done item, on both badges
    assert_eq!(body["counts"]["badge"].as_i64().unwrap(), before + 1);
    let (_, away_body) = send(&t.main, "GET", "/api/away").await;
    assert_eq!(away_body["unseen"]["pockets"], json!(1));
    let total = away_body["unseen"]["total"].as_i64().unwrap();
    assert_eq!(away::unread_total(&t.state, &SpaceId::default_space()).await.unwrap(), total, "the space list's badge agrees");

    // Seen: still listed for 7 days, no longer counted
    let (status, _) = send(&t.main, "POST", "/api/away/seen").await;
    assert_eq!(status, StatusCode::OK);
    let body = inbox(&t).await;
    let items = pockets(&body);
    assert_eq!(items.len(), 1);
    assert_eq!(items[0]["unread"], json!(false));
    assert_eq!(body["counts"]["badge"].as_i64().unwrap(), before, "seen items aren't counted");
    let (_, away_body) = send(&t.main, "GET", "/api/away").await;
    assert_eq!(away_body["unseen"]["pockets"], json!(0));
}

#[tokio::test]
async fn nobody_joined_reads_as_pa7_says() {
    let t = setup().await;
    let k = bot(&t, "Quizmaster").await;
    let code = make(&t, "Pub quiz", &k).await;
    finish(&t, &code, Utc::now()).await;
    let items = pockets(&inbox(&t).await);
    assert_eq!(items.len(), 1);
    assert_eq!(items[0]["title"], json!("Pub quiz: Nobody joined."));
    assert_eq!(items[0]["meta"]["players"], json!(0));
    assert!(items[0]["winner"].is_null());
}

#[tokio::test]
async fn a_deleted_pocket_is_gone_and_unposted_ones_never_show() {
    let t = setup().await;
    let k = bot(&t, "Quizmaster").await;
    let gone = make(&t, "Old quiz", &k).await;
    let kept = make(&t, "Pub quiz", &k).await;
    let running = make(&t, "Live quiz", &k).await;
    play(&t, &running, "Mia", 1).await;
    for code in [&gone, &kept] {
        play(&t, code, "Mia", 1).await;
        pocket::end(&t.state, code).await.unwrap();
    }
    assert_eq!(schedule::run_once(&t.state, Utc::now()).await.results.len(), 2);
    assert_eq!(pockets(&inbox(&t).await).len(), 2);

    let (status, _) = send(&t.main, "DELETE", &format!("/api/pockets/{gone}")).await;
    assert!(status.is_success(), "{status}");
    let items = pockets(&inbox(&t).await);
    let codes: Vec<&str> = items.iter().map(|i| i["source"]["code"].as_str().unwrap()).collect();
    assert_eq!(codes, vec![kept.as_str()], "the deleted pocket dropped out; the running one never showed");
}

#[tokio::test]
async fn an_eight_day_old_pocket_is_out_of_the_window() {
    let t = setup().await;
    let k = bot(&t, "Quizmaster").await;
    let old = make(&t, "Last month quiz", &k).await;
    play(&t, &old, "Mia", 1).await;
    finish(&t, &old, Utc::now() - Duration::days(8)).await;
    let recent = make(&t, "Pub quiz", &k).await;
    play(&t, &recent, "Mia", 1).await;
    finish(&t, &recent, Utc::now() - Duration::days(6)).await;

    let items = pockets(&inbox(&t).await);
    let codes: Vec<&str> = items.iter().map(|i| i["source"]["code"].as_str().unwrap()).collect();
    assert_eq!(codes, vec![recent.as_str()], "{items:?}");
    // Asking for an older `since` doesn't bring it back: Done keeps 7 days
    let since = (Utc::now() - Duration::days(30)).to_rfc3339().replace('+', "%2B");
    let (_, body) = send(&t.main, "GET", &format!("/api/inbox?since={since}")).await;
    assert_eq!(pockets(&body).len(), 1);
}

#[tokio::test]
async fn switched_off_no_pocket_items() {
    let t = setup().await;
    let k = bot(&t, "Quizmaster").await;
    let code = make(&t, "Pub quiz", &k).await;
    play(&t, &code, "Mia", 1).await;
    finish(&t, &code, Utc::now()).await;
    t.state.db.set_setting(pocket::SETTING, "off").await.unwrap();
    let body = inbox(&t).await;
    assert!(pockets(&body).is_empty());
    let (_, away_body) = send(&t.main, "GET", "/api/away").await;
    assert_eq!(away_body["unseen"]["pockets"], json!(0));
}
