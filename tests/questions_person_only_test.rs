//! `POST /api/questions/:id/answer` is the person's only (Rigel, #646 advisory 1). The answer is
//! saved as the person's own message and resumes the bot with it, so a bot's turn token gets a 403
//! and the question stays open, whether the bot asked it or another bot did. The person still
//! answers from this machine, or from another one with their token.

use axum::{
    body::{to_bytes, Body},
    extract::ConnectInfo,
    http::{Request, StatusCode},
    Router,
};
use kwnt::{
    broker::PermissionBroker,
    config::Config,
    db::{models::NewKwant, questions::QuestionPlace, spaces::SpaceId, Database},
    router::LayaRouter,
    server::{
        auth::{ApiToken, Require},
        continuity::{SelfScope, TurnOrigin},
        create_router, mcp_runtime, AppState,
    },
    tools::ToolRunner,
};
use serde_json::{json, Value};
use std::net::SocketAddr;
use std::sync::Arc;
use tempfile::{tempdir, TempDir};
use tower::ServiceExt;

const TOKEN: &str = "questions-test-token-0123456789abcdef";

async fn setup() -> (Router, AppState, TempDir) {
    let tmp = tempdir().unwrap();
    let data = tmp.path().to_path_buf();
    let db = Database::init(&data).await.unwrap();
    db.set_setting("keep_going_turns", "0").await.unwrap();
    let config = Config { data_dir: data.clone(), ..Config::default() };
    let mut state = AppState::new(
        config.clone(),
        db,
        ToolRunner::new(data.join("workspace")),
        PermissionBroker::new(),
        LayaRouter::new(config.laya_url.clone()),
    );
    state.api_token = Arc::new(ApiToken::for_tests(TOKEN, Require::Auto, config.port));
    (create_router(state.clone()), state, tmp)
}

/// POST `uri` with `body`, from loopback or (`remote`) from another machine, with an optional bearer.
async fn post(app: &Router, uri: &str, body: Value, bearer: Option<&str>, remote: bool) -> (StatusCode, Value) {
    let host = if remote { "kwnt.example.com" } else { "127.0.0.1:8799" };
    let peer: SocketAddr = if remote { "203.0.113.9:40000" } else { "127.0.0.1:40000" }.parse().unwrap();
    let mut b = Request::builder().method("POST").uri(uri).header("host", host).header("content-type", "application/json");
    if let Some(t) = bearer {
        b = b.header("authorization", format!("Bearer {t}"));
    }
    let mut req = b.body(Body::from(body.to_string())).unwrap();
    req.extensions_mut().insert(ConnectInfo(peer));
    let res = app.clone().oneshot(req).await.unwrap();
    let status = res.status();
    let bytes = to_bytes(res.into_body(), 1 << 20).await.unwrap();
    (status, serde_json::from_slice(&bytes).unwrap_or(Value::Null))
}

/// A live turn token for `bot`, as its own tools get it.
async fn turn_token(state: &AppState, bot: &kwnt::db::models::Kwant) -> String {
    let turn = format!("turn-{}", bot.id);
    let scope = SelfScope {
        kwant_id: bot.id.clone(),
        kwant_name: bot.name.clone(),
        origin: Some(TurnOrigin { direct_kwant_id: Some(bot.id.clone()), ..Default::default() }),
        cancel: Some(tokio_util::sync::CancellationToken::new()),
        turn_id: Some(turn.clone()),
        sandbox_target: Some(bot.id.clone()),
        ..Default::default()
    };
    mcp_runtime::prepare_turn_with_self(state, bot, Some(turn), None, Some(scope)).await.unwrap().1
}

async fn ask(state: &AppState, bot: &kwnt::db::models::Kwant, text: &str) -> String {
    let place = QuestionPlace { kwant_id: bot.id.clone(), direct_kwant_id: Some(bot.id.clone()), ..Default::default() };
    state.db.create_question(&place, text, &["Yes".into(), "No".into()]).await.unwrap().id
}

async fn status(state: &AppState, id: &str) -> String {
    state.db.get_question(id).await.unwrap().unwrap().status
}

#[tokio::test]
async fn a_bot_cannot_answer_a_question_and_the_person_can() {
    let (app, state, _tmp) = setup().await;
    let sp = SpaceId::default_space();
    let asker = state.db.list_kwants(&sp).await.unwrap()[0].clone();
    let other = NewKwant {
        name: "Ben".into(),
        title: "Helper".into(),
        persona_prompt: "Helps.".into(),
        soul: String::new(),
        avatar_sprite: "ben".into(),
        driver_type: "api".into(),
        preferred_model: "auto".into(),
    };
    let other = state.db.create_kwant(&sp, &other).await.unwrap();
    let own = ask(&state, &asker, "Ship it tonight?").await;
    let theirs = ask(&state, &other, "Use the blue one?").await;

    // The asking bot's own turn token, from this machine: 403, and the question stays open
    let token = turn_token(&state, &asker).await;
    for (id, what) in [(&own, "its own question"), (&theirs, "another bot's question")] {
        for body in [json!({"choice": 0}), json!({"text": "yes, go"})] {
            let (s, v) = post(&app, &format!("/api/questions/{id}/answer"), body, Some(&token), false).await;
            assert_eq!((s, v["code"].as_str()), (StatusCode::FORBIDDEN, Some("person_only")), "{what}: {v}");
        }
        assert_eq!(status(&state, id).await, "open", "{what}");
    }

    // From another machine with no token: 401; with the person's token: answered
    let (s, _) = post(&app, &format!("/api/questions/{theirs}/answer"), json!({"choice": 1}), None, true).await;
    assert_eq!(s, StatusCode::UNAUTHORIZED);
    let (s, v) = post(&app, &format!("/api/questions/{theirs}/answer"), json!({"choice": 1}), Some(TOKEN), true).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(v["question"]["status"], "answered");

    // The person on this machine (the app's own page): answered
    let (s, v) = post(&app, &format!("/api/questions/{own}/answer"), json!({"text": "yes, go"}), None, false).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(status(&state, &own).await, "answered");
}

/// A second bot in the default space.
async fn other_bot(state: &AppState) -> kwnt::db::models::Kwant {
    let ben = NewKwant {
        name: "Ben".into(),
        title: "Helper".into(),
        persona_prompt: "Helps.".into(),
        soul: String::new(),
        avatar_sprite: "ben".into(),
        driver_type: "api".into(),
        preferred_model: "auto".into(),
    };
    state.db.create_kwant(&SpaceId::default_space(), &ben).await.unwrap()
}

/// `POST /api/questions/:id/cancel` from a bot that didn't ask: 403 `not_the_asker`, from this
/// machine too, and the question stays open; so does a bearer that is no live turn's. The person
/// still dismisses it (#653, 0.9.14).
#[tokio::test]
async fn a_different_bot_cannot_dismiss_a_question() {
    let (app, state, _tmp) = setup().await;
    let asker = state.db.list_kwants(&SpaceId::default_space()).await.unwrap()[0].clone();
    let other = other_bot(&state).await;
    let q = ask(&state, &asker, "Ship it tonight?").await;
    let uri = format!("/api/questions/{q}/cancel");

    let other_token = turn_token(&state, &other).await;
    for (bearer, what) in [(other_token.as_str(), "another bot's turn token"), ("not-a-live-turn-token", "a made-up bearer")] {
        let (s, v) = post(&app, &uri, json!({}), Some(bearer), false).await;
        assert_eq!((s, v["code"].as_str()), (StatusCode::FORBIDDEN, Some("not_the_asker")), "{what}: {v}");
        assert_eq!(status(&state, &q).await, "open", "{what}");
    }

    // The person on this machine, and from another one with their token
    let (s, v) = post(&app, &uri, json!({}), None, false).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(status(&state, &q).await, "cancelled");
    let q2 = ask(&state, &asker, "Use the blue one?").await;
    let (s, v) = post(&app, &format!("/api/questions/{q2}/cancel"), json!({}), Some(TOKEN), true).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(status(&state, &q2).await, "cancelled");
}

/// The bot that asked may take its own question back with its live turn token: 200, cancelled.
#[tokio::test]
async fn the_asking_bot_can_dismiss_its_own_question() {
    let (app, state, _tmp) = setup().await;
    let asker = state.db.list_kwants(&SpaceId::default_space()).await.unwrap()[0].clone();
    let other = other_bot(&state).await;
    let mine = ask(&state, &asker, "Ship it tonight?").await;
    let theirs = ask(&state, &other, "Use the blue one?").await;

    let token = turn_token(&state, &asker).await;
    let (s, v) = post(&app, &format!("/api/questions/{mine}/cancel"), json!({}), Some(&token), false).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(v["status"], "cancelled", "{v}");
    assert_eq!(status(&state, &mine).await, "cancelled");

    // The same token on the other bot's question: 403, still open
    let (s, v) = post(&app, &format!("/api/questions/{theirs}/cancel"), json!({}), Some(&token), false).await;
    assert_eq!((s, v["code"].as_str()), (StatusCode::FORBIDDEN, Some("not_the_asker")), "{v}");
    assert_eq!(status(&state, &theirs).await, "open");
}
