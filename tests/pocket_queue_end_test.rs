//! queue-end-summary (Mira's queue UX pass #4): a bakery queue that served 3 ends with the kit's
//! own line, "Tine's Bakery has ended. 1 day · 3 served · average 1 min.", in the chat it came
//! from, the notification and the Inbox Done row, never "2 players took part.", and without the
//! "Not checked" strip (a count, not a result to verify). Other kits keep their copy (the quiz's
//! "N players took part." is pinned in pocket_schedule_test, Santa's "people" in
//! pocket_santa_ends_test). Brief: /home/maus/shared/ship-software-safely/autonomy/queue-end-summary/brief.md.

use axum::{
    body::{to_bytes, Body},
    extract::ConnectInfo,
    http::{Request, StatusCode},
    Router,
};
use chrono::Utc;
use kwnt::{
    broker::PermissionBroker,
    config::Config,
    db::{
        models::{Kwant, Message, NewKwant},
        spaces::SpaceId,
        Database,
    },
    drivers::events::RuntimeEvent,
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
        create_router, AppState,
    },
    tools::ToolRunner,
};
use serde_json::{json, Value};
use std::net::SocketAddr;
use std::sync::Arc;
use tempfile::{tempdir, TempDir};
use tokio::sync::broadcast::Receiver;
use tower::ServiceExt;

const TOKEN: &str = "test-api-token-0123456789";

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

async fn bot(t: &T) -> Kwant {
    let input = NewKwant {
        name: "Ben".into(),
        title: "Shop helper".into(),
        persona_prompt: String::new(),
        soul: String::new(),
        avatar_sprite: "s".into(),
        driver_type: "cli".into(),
        preferred_model: "claude".into(),
    };
    t.state.db.create_kwant(&SpaceId::default_space(), &input).await.unwrap()
}

async fn host(t: &T, code: &str, name: &str, body: Value) {
    pocket::act(&t.state, code, &Viewer::Host, name, &body).await.unwrap_or_else(|e| panic!("host {name}: {}", e.code));
}

/// Mira's day at Tine's Bakery: 5 tickets, 3 served, 1 left, 1 still waiting, then End.
async fn bakery_day(t: &T, k: &Kwant) -> String {
    let origin = Some(Origin { kwant_id: k.id.clone(), channel_id: None, thread_id: None });
    let spec = json!({ "title": "Tine's Bakery", "line": "Pick up at the side counter", "call_minutes": 3 });
    let (s, _) = pocket::create(&t.state, CreateRequest { kit: "queue".into(), spec, origin, ..Default::default() }).await.unwrap();
    let code = s.row.code.clone();
    let mut guests = Vec::new();
    for _ in 0..5 {
        let (p, _) = pocket::join(&t.state, &code, JoinRequest::default()).await.unwrap_or_else(|e| panic!("join: {}", e.code));
        pocket::act(&t.state, &code, &Viewer::Guest(p.id.clone()), "take", &json!({})).await.unwrap_or_else(|e| panic!("take: {}", e.code));
        guests.push(p.id);
    }
    host(t, &code, "call_next", json!({})).await;
    host(t, &code, "call_next", json!({ "served": 1 })).await;
    host(t, &code, "call_next", json!({ "served": 2 })).await;
    host(t, &code, "serve", json!({ "ticket": 3 })).await;
    pocket::act(&t.state, &code, &Viewer::Guest(guests[3].clone()), "leave", &json!({})).await.unwrap();
    host(t, &code, "end", json!({})).await;
    code
}

/// The "has ended" posts in the bot's chat.
async fn ended_posts(t: &T, k: &Kwant, code: &str) -> Vec<Message> {
    let all = t.state.db.list_messages(&k.id, 200).await.unwrap();
    all.into_iter().filter(|m| m.meta.as_deref().and_then(|s| serde_json::from_str::<Value>(s).ok()).is_some_and(|v| v["reason"] == "results" && v["code"] == json!(code))).collect()
}

/// Every Inbox Done item of kind pocket.
async fn pocket_rows(t: &T) -> Vec<Value> {
    let mut req = Request::builder().method("GET").uri("/api/inbox").header("host", "127.0.0.1:8799").body(Body::empty()).unwrap();
    req.extensions_mut().insert(ConnectInfo("127.0.0.1:50000".parse::<SocketAddr>().unwrap()));
    let resp = t.main.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body: Value = serde_json::from_slice(&to_bytes(resp.into_body(), usize::MAX).await.unwrap()).unwrap();
    body["items"].as_array().unwrap().iter().filter(|i| i["kind"] == "pocket").cloned().collect()
}

/// The notifications sent so far: (kind, title, body).
fn notes(rx: &mut Receiver<RuntimeEvent>) -> Vec<(String, String, String)> {
    let mut out = Vec::new();
    while let Ok(ev) = rx.try_recv() {
        if let RuntimeEvent::Notify { kind, title, body, .. } = ev {
            out.push((kind, title, body));
        }
    }
    out
}

#[tokio::test]
async fn a_queue_ends_with_served_and_average_in_the_chat_the_notification_and_the_done_row() {
    let t = setup().await;
    let k = bot(&t).await;
    let code = bakery_day(&t, &k).await;
    let mut rx = t.state.event_sender.subscribe();
    let ran = schedule::run_once(&t.state, Utc::now()).await;
    assert_eq!(ran.results, vec![code.clone()], "the ended queue's results are posted once");

    // The card's own Done line; "days" follows the box's clock, so a run across midnight still holds
    let kit_state: Value = serde_json::from_str(&t.state.db.pocket_get(&code).await.unwrap().unwrap().kit_state).unwrap();
    let days = kit_state["days"].as_u64().unwrap();
    let day = if days == 1 { "1 day".to_string() } else { format!("{days} days") };
    let line = format!("{day} · 3 served · average 1 min");

    // The chat: the line Ben's next turn reads
    let posts = ended_posts(&t, &k, &code).await;
    assert_eq!(posts.len(), 1, "{posts:?}");
    assert_eq!(posts[0].text, format!("Tine's Bakery has ended. {line}."));
    assert!(!posts[0].text.contains("player"), "customers aren't players: {}", posts[0].text);

    // The notification: the same words without the title
    let n = notes(&mut rx);
    assert_eq!(n, vec![("pocket_app".to_string(), "Tine's Bakery has ended".to_string(), format!("{line}."))]);

    // Inbox › Done: the same line in meta.summary, no players count, no "Not checked" strip
    let rows = pocket_rows(&t).await;
    assert_eq!(rows.len(), 1, "{rows:?}");
    let row = &rows[0];
    assert_eq!(row["id"], json!(format!("pocket:{code}")));
    assert_eq!(row["title"], "Tine's Bakery has ended");
    assert_eq!(row["meta"]["summary"], json!(line));
    assert!(row["meta"].get("players").is_none(), "no players on a queue's row: {}", row["meta"]);
    assert!(row["meta"]["ended_at"].is_string());
    assert!(row["checked"].is_null(), "no Not checked strip on a count: {}", row["checked"]);
    assert_eq!(row["result_message_id"], json!(posts[0].id), "the chat's post is still the row's result");
}

#[tokio::test]
async fn a_quiz_like_pocket_keeps_players_and_its_check_line() {
    let t = setup().await;
    let k = bot(&t).await;
    let origin = Some(Origin { kwant_id: k.id.clone(), channel_id: None, thread_id: None });
    let (s, _) = pocket::create(&t.state, CreateRequest { kit: "test".into(), spec: json!({ "title": "Pub quiz" }), origin, ..Default::default() }).await.unwrap();
    let code = s.row.code.clone();
    for name in ["Mia", "Noa"] {
        pocket::join(&t.state, &code, JoinRequest { name: name.into(), ..Default::default() }).await.unwrap();
    }
    pocket::end(&t.state, &code).await.unwrap();
    let mut rx = t.state.event_sender.subscribe();
    assert_eq!(schedule::run_once(&t.state, Utc::now()).await.results, vec![code.clone()]);
    let posts = ended_posts(&t, &k, &code).await;
    assert_eq!(posts.iter().map(|m| m.text.as_str()).collect::<Vec<_>>(), ["Pub quiz has ended. 2 players took part."]);
    assert_eq!(notes(&mut rx), vec![("pocket_app".to_string(), "Pub quiz has ended".to_string(), "2 players took part. The results are in your chat.".to_string())]);
    let rows = pocket_rows(&t).await;
    assert_eq!(rows.len(), 1, "{rows:?}");
    assert_eq!(rows[0]["meta"]["players"], 2);
    assert!(rows[0]["meta"].get("summary").is_none());
    assert_eq!(rows[0]["checked"]["state"], "not_checked", "other pockets keep today's check line");
}
