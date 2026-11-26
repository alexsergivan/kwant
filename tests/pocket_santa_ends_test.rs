//! santa-ends-by-itself: a Secret Santa ends on its own a day after the reveal (or, with the
//! reveal skipped or off, a day after the gift day), so the minute run posts the "has ended"
//! line once, with "people", and the Inbox Done row appears, long before the data expires.
//! Brief: /home/maus/shared/ship-software-safely/autonomy/santa-ends-by-itself/brief.md.

use axum::{
    body::{to_bytes, Body},
    extract::ConnectInfo,
    http::{Request, StatusCode},
    Router,
};
use chrono::{DateTime, Duration, Utc};
use kwnt::{
    broker::PermissionBroker,
    config::Config,
    db::{
        models::{Kwant, Message, NewKwant},
        spaces::SpaceId,
        Database,
    },
    pocket::{
        self,
        kits::{spec_date, Viewer},
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
use tower::ServiceExt;

const TOKEN: &str = "test-api-token-0123456789";
const PIN: &str = "4821";
const SIX: [&str; 6] = ["Ana", "Ben", "Cleo", "Dev", "Eli", "Fay"];

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
        name: "Santa helper".into(),
        title: "Helper".into(),
        persona_prompt: String::new(),
        soul: String::new(),
        avatar_sprite: "s".into(),
        driver_type: "cli".into(),
        preferred_model: "claude".into(),
    };
    t.state.db.create_kwant(&SpaceId::default_space(), &input).await.unwrap()
}

/// A Santa 10 days out: gifts that day, the reveal at 19:00 UTC on it, plus `extra`.
fn spec(extra: Value) -> Value {
    let day = (Utc::now() + Duration::days(10)).date_naive().format("%Y-%m-%d").to_string();
    let mut s = json!({ "title": "Office Santa", "gift_on": day, "reveal_at": format!("{day}T19:00:00Z") });
    for (k, v) in extra.as_object().unwrap() {
        s[k] = v.clone();
    }
    s
}

/// Makes the Santa from the bot's chat, lets `names` in and draws: the pocket's code.
async fn drawn(t: &T, k: &Kwant, spec: Value, names: &[&str]) -> String {
    let origin = Some(Origin { kwant_id: k.id.clone(), channel_id: None, thread_id: None });
    let (s, _) = pocket::create(&t.state, CreateRequest { kit: "santa".into(), spec, origin, ..Default::default() }).await.unwrap();
    let code = s.row.code.clone();
    for n in names {
        pocket::join(&t.state, &code, JoinRequest { name: n.to_string(), avatar: None, team: None, pin: Some(PIN.into()) }).await.unwrap();
    }
    pocket::act(&t.state, &code, &Viewer::Host, "draw_now", &json!({})).await.unwrap();
    code
}

/// The supervisor's minute at `at`: pa7's run, then the sweep (which ticks unwatched Santas).
/// Returns (ended by the run, results posted by the run).
async fn minute(t: &T, at: DateTime<Utc>) -> (usize, Vec<String>) {
    let ran = schedule::run_once(&t.state, at).await;
    pocket::sweep(&t.state, at).await;
    (ran.ended, ran.results)
}

async fn kit_state(t: &T, code: &str) -> Value {
    serde_json::from_str(&t.state.db.pocket_get(code).await.unwrap().unwrap().kit_state).unwrap()
}

async fn pocket_state(t: &T, code: &str) -> String {
    t.state.db.pocket_get(code).await.unwrap().unwrap().state
}

/// The "has ended" posts in the bot's chat.
async fn ended_posts(t: &T, k: &Kwant, code: &str) -> Vec<Message> {
    let all = t.state.db.list_messages(&k.id, 200).await.unwrap();
    all.into_iter().filter(|m| m.meta.as_deref().and_then(|s| serde_json::from_str::<Value>(s).ok()).is_some_and(|v| v["reason"] == "results" && v["code"] == json!(code))).collect()
}

async fn done_rows(t: &T, code: &str) -> Vec<Value> {
    let mut req = Request::builder().method("GET").uri("/api/inbox").header("host", "127.0.0.1:8799").body(Body::empty()).unwrap();
    req.extensions_mut().insert(ConnectInfo("127.0.0.1:50000".parse::<SocketAddr>().unwrap()));
    let resp = t.main.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body: Value = serde_json::from_slice(&to_bytes(resp.into_body(), usize::MAX).await.unwrap()).unwrap();
    body["items"].as_array().unwrap().iter().filter(|i| i["kind"] == "pocket" && i["id"] == json!(format!("pocket:{code}"))).cloned().collect()
}

fn at(spec: &Value, field: &str) -> DateTime<Utc> {
    spec_date(&spec[field]).unwrap()
}

#[tokio::test]
async fn a_revealed_santa_ends_once_a_day_after_the_reveal_with_people_and_a_done_row() {
    let t = setup().await;
    let k = bot(&t).await;
    let sp = spec(json!({}));
    let reveal = at(&sp, "reveal_at");
    let code = drawn(&t, &k, sp, &SIX).await;
    // The reveal runs unwatched at its time (the sweep's tick); the Santa stays open
    assert_eq!(minute(&t, reveal).await, (0, vec![]));
    assert_eq!(kit_state(&t, &code).await["phase"], "revealed");
    // Not at reveal + 23:59
    let almost = reveal + Duration::days(1) - Duration::minutes(1);
    assert_eq!(minute(&t, almost).await, (0, vec![]));
    assert_eq!(pocket_state(&t, &code).await, "open");
    assert!(ended_posts(&t, &k, &code).await.is_empty());
    assert!(done_rows(&t, &code).await.is_empty());
    // At reveal + 1 day the tick marks it over; the next minute's run ends it and posts once
    let day_after = reveal + Duration::days(1);
    assert_eq!(minute(&t, day_after).await, (0, vec![]), "the sweep's tick runs after pa7's run");
    assert!(kit_state(&t, &code).await["done_at"].is_string());
    assert_eq!(minute(&t, day_after + Duration::minutes(1)).await, (1, vec![code.clone()]));
    assert_eq!(pocket_state(&t, &code).await, "ended");
    let posts = ended_posts(&t, &k, &code).await;
    assert_eq!(posts.len(), 1);
    assert_eq!(posts[0].text, "Office Santa has ended. 6 people took part.");
    let rows = done_rows(&t, &code).await;
    assert_eq!(rows.len(), 1, "one Inbox Done row: {rows:?}");
    assert_eq!(rows[0]["title"], "Office Santa has ended");
    assert_eq!(rows[0]["meta"]["players"], 6);
    // A later minute posts nothing more, and the data is still kept until it expires
    assert_eq!(minute(&t, day_after + Duration::minutes(2)).await, (0, vec![]));
    assert_eq!(ended_posts(&t, &k, &code).await.len(), 1);
    assert_eq!(done_rows(&t, &code).await.len(), 1);
    let counts = t.state.db.pocket_row_counts(&code).await.unwrap();
    assert!(counts.iter().all(|n| *n > 0), "ending isn't deleting: {counts:?}");
    let expires = t.state.db.pocket_get(&code).await.unwrap().unwrap().expires_at;
    assert_eq!(expires, Some(reveal + Duration::days(7)), "same expiry as before");
}

#[tokio::test]
async fn a_skipped_or_secret_reveal_ends_a_day_after_the_gift_day() {
    let t = setup().await;
    let k = bot(&t).await;
    let skipped_spec = spec(json!({}));
    let gift_end = at(&skipped_spec, "gift_on");
    let skipped = drawn(&t, &k, skipped_spec, &SIX[..3]).await;
    pocket::act(&t.state, &skipped, &Viewer::Host, "skip_reveal", &json!({})).await.unwrap();
    let mut never_spec = spec(json!({ "reveal": false }));
    never_spec.as_object_mut().unwrap().remove("reveal_at");
    let never = drawn(&t, &k, never_spec, &SIX[..4]).await;
    // A day after the planned reveal is not enough without a reveal; nor gift_on + 1 day - 1 minute
    let reveal_plus_day = gift_end.date_naive().and_hms_opt(19, 0, 0).unwrap().and_utc() + Duration::days(1);
    for now in [reveal_plus_day, gift_end + Duration::days(1) - Duration::minutes(1)] {
        minute(&t, now).await;
        minute(&t, now).await;
        assert_eq!((pocket_state(&t, &skipped).await, pocket_state(&t, &never).await), ("open".into(), "open".into()), "{now}");
    }
    let gift_plus_day = gift_end + Duration::days(1);
    minute(&t, gift_plus_day).await;
    let (ended, mut posted) = minute(&t, gift_plus_day + Duration::minutes(1)).await;
    posted.sort();
    let mut both = vec![skipped.clone(), never.clone()];
    both.sort();
    assert_eq!((ended, posted), (2, both));
    assert_eq!(kit_state(&t, &skipped).await["phase"], "drawn", "the matches stay secret");
    assert_eq!(ended_posts(&t, &k, &skipped).await.iter().map(|m| m.text.clone()).collect::<Vec<_>>(), ["Office Santa has ended. 3 people took part."]);
    assert_eq!(ended_posts(&t, &k, &never).await.iter().map(|m| m.text.clone()).collect::<Vec<_>>(), ["Office Santa has ended. 4 people took part."]);
    assert_eq!((done_rows(&t, &skipped).await.len(), done_rows(&t, &never).await.len()), (1, 1));
}

#[tokio::test]
async fn a_santa_that_never_drew_names_stays_open_past_the_gift_day() {
    let t = setup().await;
    let k = bot(&t).await;
    let sp = spec(json!({}));
    let gift_end = at(&sp, "gift_on");
    let origin = Some(Origin { kwant_id: k.id.clone(), channel_id: None, thread_id: None });
    let (s, _) = pocket::create(&t.state, CreateRequest { kit: "santa".into(), spec: sp, origin, ..Default::default() }).await.unwrap();
    let code = s.row.code.clone();
    for n in &SIX[..2] {
        pocket::join(&t.state, &code, JoinRequest { name: n.to_string(), avatar: None, team: None, pin: Some(PIN.into()) }).await.unwrap();
    }
    for now in [gift_end + Duration::days(1), gift_end + Duration::days(2)] {
        assert_eq!(minute(&t, now).await, (0, vec![]), "{now}");
    }
    assert_eq!(pocket_state(&t, &code).await, "open", "the host's End or the expiry sweep end it, as before");
    assert!(kit_state(&t, &code).await.get("done_at").is_none());
    assert!(ended_posts(&t, &k, &code).await.is_empty());
}
