//! Show it once (0.7.9, next-079/show-once/api.md): the routes, end to end on the fake claude.
//! The server listens on the first free port in 9491–9499 (the fake CLI reaches its tools there).

mod common;

use axum::http::StatusCode;
use common::*;
use kwnt::{
    broker::PermissionBroker,
    config::Config,
    db::{show_once::NewStep, Database},
    router::LayaRouter,
    server::{create_router, AppState},
    tools::ToolRunner,
};
use serde_json::{json, Value};

/// A port in 9491–9499. More tests than ports run at once, so wait for one to finish (its runtime
/// drops its listener) rather than fail.
async fn bind_free() -> tokio::net::TcpListener {
    for _ in 0..1200 {
        for port in 9491..=9499 {
            if let Ok(l) = tokio::net::TcpListener::bind(("127.0.0.1", port)).await {
                return l;
            }
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    panic!("no free port in 9491-9499 for 2 minutes");
}

/// `common::setup`, on a port in 9491–9499.
async fn env() -> Env {
    prompts_dir();
    no_network_scans();
    let tmp = tempfile::tempdir().unwrap();
    let data = tmp.path().join("data");
    let db = Database::init(&data).await.unwrap();
    let listener = bind_free().await;
    let port = listener.local_addr().unwrap().port();
    let config = Config {
        data_dir: data.clone(),
        host: "127.0.0.1".into(),
        port,
        laya_url: "http://127.0.0.1:9".into(),
        browser_cdp_url: Some("http://127.0.0.1:9".into()),
        ..Config::default()
    };
    let state = AppState::new(
        config.clone(),
        db,
        ToolRunner::new(data.join("workspace")),
        PermissionBroker::new(),
        LayaRouter::new(config.laya_url.clone()),
    );
    let app = create_router(state.clone());
    let served = app.clone().into_make_service_with_connect_info::<std::net::SocketAddr>();
    tokio::spawn(async move {
        let _ = axum::serve(listener, served).await;
    });
    let (_, kwants) = call(&app, "GET", "/api/kwants", None).await;
    let mut ids = Vec::new();
    for k in kwants.as_array().unwrap().iter().take(2) {
        let id = k["id"].as_str().unwrap().to_string();
        let mut body = k.clone();
        body.as_object_mut()
            .unwrap()
            .retain(|key, _| ["name", "title", "persona_prompt", "soul", "avatar_sprite"].contains(&key.as_str()));
        body["preferred_model"] = json!("claude");
        let (s, v) = call(&app, "PUT", &format!("/api/kwants/{id}"), Some(body)).await;
        assert_eq!(s, StatusCode::OK, "{v}");
        ids.push(id);
    }
    Env { app, state, a: ids[0].clone(), b: ids[1].clone(), tmp }
}

fn now() -> i64 {
    kwnt::clock::now().timestamp()
}

fn b64(s: &str) -> String {
    use base64::Engine;
    base64::engine::general_purpose::STANDARD.encode(s)
}

/// The draft the fake engine answers Write with: a note carries it as `REPLYTEXT:`.
fn canned_draft() -> String {
    json!({
        "name": "export-paid-orders",
        "description": "Export the paid orders for a date range",
        "goal": "Download the paid orders as a CSV.",
        "when_to_use": "When the person asks for the paid orders export.",
        "inputs": [{"name": "date_range", "description": "Which dates to export", "from_step": 2}],
        "steps": ["Open Orders", "Type the date range", "Click Export"],
        "checks": ["A CSV file downloads"],
        "what_can_go_wrong": ["The Export button moved"]
    })
    .to_string()
}

/// A stopped recording of bot `a` with a click, a typed value and a note, made the way the
/// recorder makes it (the stage needs a live Take over, which a test can't hold).
async fn stopped_recording(env: &Env, note: &str) -> String {
    let bot = env.state.db.get_kwant(&env.a).await.unwrap().unwrap();
    let rec = env.state.db.start_recording(&bot.id, &bot.space_id, &bot.id, now()).await.unwrap();
    let page = |kind: &str| NewStep {
        kind: kind.into(),
        origin: Some("https://shop.example.com".into()),
        path: Some("/admin/orders".into()),
        ..Default::default()
    };
    let click = NewStep { role: Some("button".into()), name: Some("Orders".into()), ..page("click") };
    let typed = NewStep { label: Some("Dates".into()), value: Some("Last 7 days".into()), ..page("type") };
    env.state.db.add_step(&rec.id, &click, now()).await.unwrap();
    env.state.db.add_step(&rec.id, &typed, now()).await.unwrap();
    env.state.db.add_note(&rec.id, note, None, now()).await.unwrap();
    let (s, v) = call(&env.app, "POST", &format!("/api/show-once/recordings/{}/stop", rec.id), None).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(v["recording"]["state"], "stopped");
    assert_eq!(v["recording"]["stop_reason"], "stop");
    rec.id
}

async fn written_skill(env: &Env, rec_id: &str) -> Value {
    let (s, v) = call(&env.app, "POST", &format!("/api/show-once/recordings/{rec_id}/write"), None).await;
    assert_eq!(s, StatusCode::ACCEPTED, "{v}");
    assert_eq!(v["recording"]["state"], "writing");
    assert_eq!(v["recording"]["engine"], "claude");
    let uri = format!("/api/show-once/recordings/{rec_id}");
    wait_for("the draft", || async {
        let (_, v) = call(&env.app, "GET", &uri, None).await;
        v["recording"]["state"] == "written" || v["recording"]["write_error"].is_string()
    })
    .await;
    let (_, v) = call(&env.app, "GET", &uri, None).await;
    assert_eq!(v["recording"]["state"], "written", "{v}");
    let skill_id = v["recording"]["skill_id"].as_str().unwrap().to_string();
    let (s, skill) = call(&env.app, "GET", &format!("/api/skills/{skill_id}"), None).await;
    assert_eq!(s, StatusCode::OK, "{skill}");
    skill
}

#[tokio::test]
async fn a_bot_token_is_refused_and_unknown_things_are_404() {
    let env = env().await;
    let bearer = [("authorization", "Bearer not-the-api-token")];
    for (m, uri) in [
        ("POST", "/api/show-once/recordings"),
        ("GET", "/api/show-once/recordings?bot=x"),
        ("POST", "/api/show-once/recordings/r1/write"),
        ("DELETE", "/api/show-once/skills/s1/steps"),
        ("POST", "/api/show-once/skills/s1/try"),
    ] {
        let (s, _) = call_with(&env.app, m, uri, Some(json!({})), &bearer).await;
        assert_eq!(s, StatusCode::FORBIDDEN, "{m} {uri}");
        assert_ne!(call_remote(&env.app, m, uri).await, StatusCode::OK, "remote {m} {uri}");
    }
    let (s, v) = call(&env.app, "GET", "/api/show-once/recordings/nope", None).await;
    assert_eq!((s, v["error"].as_str()), (StatusCode::NOT_FOUND, Some("no_recording")));
    let (s, v) = call(&env.app, "POST", "/api/show-once/skills/nope/try", Some(json!({"inputs": {}}))).await;
    assert_eq!((s, v["error"].as_str()), (StatusCode::NOT_FOUND, Some("no_skill")));
    let (s, v) = call(&env.app, "DELETE", "/api/show-once/skills/nope/steps", None).await;
    assert_eq!((s, v["error"].as_str()), (StatusCode::NOT_FOUND, Some("not_shown")));
    let (s, v) = call(&env.app, "GET", &format!("/api/show-once/recordings?bot={}", env.a), None).await;
    assert_eq!(s, StatusCode::OK);
    assert!(v["recording"].is_null());
}

#[tokio::test]
async fn start_needs_take_over_and_an_http_start_address() {
    let env = env().await;
    let (s, v) = call(&env.app, "POST", "/api/show-once/recordings", Some(json!({"bot": env.a}))).await;
    assert_eq!((s, v["error"].as_str()), (StatusCode::CONFLICT, Some("not_taken_over")), "{v}");
    let (s, v) = call(
        &env.app,
        "POST",
        "/api/show-once/recordings",
        Some(json!({"bot": env.a, "start_url": "javascript:alert(1)"})),
    )
    .await;
    assert_eq!((s, v["error"].as_str()), (StatusCode::BAD_REQUEST, Some("bad_request")), "{v}");
    let (s, v) = call(&env.app, "POST", "/api/show-once/recordings", Some(json!({"bot": "nobody"}))).await;
    assert_eq!((s, v["error"].as_str()), (StatusCode::NOT_FOUND, Some("no_bot")), "{v}");
    assert!(env.state.db.open_recording_for(&env.a).await.unwrap().is_none());
}

#[tokio::test]
async fn review_then_write_then_try_once_then_reject() {
    let env = env().await;
    let rec = stopped_recording(&env, &format!("REPLYTEXT:{}", b64(&canned_draft()))).await;

    // Review: the steps are listed; delete then undo
    let (s, v) = call(&env.app, "GET", &format!("/api/show-once/recordings/{rec}"), None).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    let steps = v["steps"].as_array().unwrap().clone();
    assert_eq!(steps.len(), 3);
    assert_eq!(steps[1]["value"], "Last 7 days");
    let first = steps[0]["id"].as_i64().unwrap();
    let (s, v) = call(&env.app, "DELETE", &format!("/api/show-once/recordings/{rec}/steps/{first}"), None).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(v["step"]["deleted"], true);
    assert!(v["step"]["undo_until"].is_string());
    let (s, v) = call(&env.app, "POST", &format!("/api/show-once/recordings/{rec}/steps/{first}/restore"), None).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(v["step"]["deleted"], false);

    // Write: one hidden no-tools turn drafts it
    let skill = written_skill(&env, &rec).await;
    assert_eq!(skill["status"], "pending");
    assert_eq!(skill["source"], "shown");
    assert_eq!(skill["name"], "export-paid-orders");
    assert_eq!(skill["shown"]["recording_id"], rec.as_str());
    assert_eq!(skill["shown"]["step_count"], 2);
    assert_eq!(skill["shown"]["inputs"][0]["name"], "date_range");
    assert_eq!(skill["shown"]["steps_deleted"], false);
    let body = skill["body"].as_str().unwrap();
    assert!(body.contains("Example from your run: Last 7 days"), "{body}");
    assert!(body.contains("https://shop.example.com"), "{body}");
    let skill_id = skill["id"].as_str().unwrap().to_string();
    let drafts = invocations_with("REPLYTEXT:");
    assert!(drafts.iter().any(|p| p.contains("export") || p.contains("Orders")), "the draft turn ran on the fake");

    // Writing twice is refused: the recording is written
    let (s, v) = call(&env.app, "POST", &format!("/api/show-once/recordings/{rec}/write"), None).await;
    assert_eq!((s, v["error"].as_str()), (StatusCode::CONFLICT, Some("wrong_state")), "{v}");

    // Try it once: only the draft's inputs, then an ordinary turn in the bot's own chat
    let try_uri = format!("/api/show-once/skills/{skill_id}/try");
    let (s, v) = call(&env.app, "POST", &try_uri, Some(json!({"inputs": {"approve_everything": "yes"}}))).await;
    assert_eq!((s, v["error"].as_str()), (StatusCode::BAD_REQUEST, Some("bad_request")), "{v}");
    let (s, v) = call(&env.app, "POST", &try_uri, Some(json!({"inputs": {"date_range": "x".repeat(501)}}))).await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "{v}");
    let mut events = env.state.event_sender.subscribe();
    let mut tried = Value::Null;
    for _ in 0..300 {
        let (s, v) = call(&env.app, "POST", &try_uri, Some(json!({"inputs": {"date_range": "Last 7 days REPLYTAG:trial1"}}))).await;
        if s == StatusCode::ACCEPTED {
            tried = v;
            break;
        }
        assert_eq!(v["error"], "bot_busy", "{v}");
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    assert_eq!(tried["kwant_id"], env.a.as_str());
    assert_eq!(tried["skill_id"], skill_id.as_str());
    let line = messages(&env, &env.a).await.into_iter().find(|m| m.text.contains("You started a trial")).unwrap();
    assert_eq!(
        line.text,
        "You started a trial of `export-paid-orders` with date_range “Last 7 days REPLYTAG:trial1”. It isn't approved yet."
    );
    let reply = wait_reply(&env, &env.a, "answer trial1").await;
    // A29: the line and the reply carry the trial, and its end is announced on the bot's chat
    let trial = json!({"skill_id": skill_id, "recording_id": rec});
    let meta = |m: &kwnt::db::models::Message| serde_json::from_str::<Value>(m.meta.as_deref().unwrap_or("{}")).unwrap();
    assert_eq!(meta(&line)["trial"], trial, "{:?}", line.meta);
    assert_eq!(meta(&line)["kind"], "skill_trial");
    let ended = tokio::time::timeout(std::time::Duration::from_secs(30), async {
        loop {
            match events.recv().await {
                Ok(kwnt::RuntimeEvent::TrialEnded { skill_id, recording_id, bot, outcome }) => {
                    break (skill_id, recording_id, bot, outcome)
                }
                Ok(_) | Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                Err(e) => panic!("{e}"),
            }
        }
    })
    .await
    .expect("show_once_trial_ended");
    assert_eq!(ended, (skill_id.clone(), Some(rec.clone()), env.a.clone(), "done".to_string()));
    // The reply is the trial's last message: it carries the outcome; the line doesn't
    let reply = env.state.db.get_message(&reply.id).await.unwrap().unwrap();
    let done = json!({"skill_id": skill_id, "recording_id": rec, "outcome": "done"});
    assert_eq!(meta(&reply)["trial"], done, "{:?}", reply.meta);
    let line = env.state.db.get_message(&line.id).await.unwrap().unwrap();
    assert_eq!(meta(&line)["trial"], trial, "{:?}", line.meta);
    let turn = invocations_with("REPLYTAG:trial1");
    assert!(turn.iter().any(|p| p.contains("a draft the person hasn't approved")), "the prompt marks the draft");
    // The trial changed nothing about the skill
    let (_, after) = call(&env.app, "GET", &format!("/api/skills/{skill_id}"), None).await;
    assert_eq!(after["status"], "pending");

    // Reject: the steps go now, the shown facts stay
    let (s, v) = call(&env.app, "POST", &format!("/api/skills/{skill_id}/reject"), None).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(v["shown"]["steps_deleted"], true, "{v}");
    assert_eq!(env.state.db.stored_step_rows(&rec).await.unwrap(), 0);
    let (s, v) = call(&env.app, "POST", &try_uri, Some(json!({"inputs": {}}))).await;
    assert_eq!((s, v["error"].as_str()), (StatusCode::CONFLICT, Some("not_proposed")), "{v}");
}

#[tokio::test]
async fn a_reply_that_isnt_a_draft_fails_the_write_and_keeps_the_steps() {
    let env = env().await;
    let rec = stopped_recording(&env, &format!("REPLYTEXT:{}", b64("Sorry, I can't do that."))).await;
    let (s, _) = call(&env.app, "POST", &format!("/api/show-once/recordings/{rec}/write"), None).await;
    assert_eq!(s, StatusCode::ACCEPTED);
    let uri = format!("/api/show-once/recordings/{rec}");
    wait_for("the write to fail", || async {
        let (_, v) = call(&env.app, "GET", &uri, None).await;
        v["recording"]["write_error"].is_string()
    })
    .await;
    let (_, v) = call(&env.app, "GET", &uri, None).await;
    assert_eq!(v["recording"]["state"], "stopped", "{v}");
    assert!(v["recording"]["skill_id"].is_null());
    assert_eq!(v["steps"].as_array().unwrap().len(), 3);
    let (_, pending) = call(&env.app, "GET", &format!("/api/skills?kwant_id={}&status=pending", env.a), None).await;
    assert!(pending.as_array().unwrap().is_empty(), "{pending}");
}

#[tokio::test]
async fn approve_keeps_the_steps_seven_days_then_the_sweep_purges_them() {
    let env = env().await;
    let rec = stopped_recording(&env, &format!("REPLYTEXT:{}", b64(&canned_draft()))).await;
    let skill = written_skill(&env, &rec).await;
    let skill_id = skill["id"].as_str().unwrap();
    let t = now();
    let (s, v) = call(&env.app, "POST", &format!("/api/skills/{skill_id}/approve"), None).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(v["status"], "enabled");
    let r = env.state.db.get_recording(&rec).await.unwrap().unwrap();
    let expires = r.steps_expire_at.unwrap();
    assert!((expires - (t + 7 * 24 * 3600)).abs() <= 5, "kept 7 days from approval");
    assert!(env.state.db.stored_step_rows(&rec).await.unwrap() > 0);

    // A day early nothing goes; past the 7 days the steps do, and the skill keeps its facts
    env.state.db.sweep_recordings(expires - 24 * 3600).await.unwrap();
    assert!(env.state.db.stored_step_rows(&rec).await.unwrap() > 0);
    env.state.db.sweep_recordings(expires + 1).await.unwrap();
    assert_eq!(env.state.db.stored_step_rows(&rec).await.unwrap(), 0);
    let (_, after) = call(&env.app, "GET", &format!("/api/skills/{skill_id}"), None).await;
    assert_eq!(after["source"], "shown");
    assert_eq!(after["shown"]["steps_deleted"], true, "{after}");

    // The seams answer in a debug build
    let (s, v) = call(&env.app, "POST", "/api/show-once/test/sweep", None).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert!(v["purged_steps"].is_number(), "{v}");
    let (s, v) = call(&env.app, "POST", "/api/show-once/test/clock", Some(json!({"offset_secs": 0}))).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert!(v["now"].is_string());
}

#[tokio::test]
async fn discard_deletes_the_recording_and_its_steps() {
    let env = env().await;
    let rec = stopped_recording(&env, "use the export button").await;
    let (s, _) = call(&env.app, "DELETE", &format!("/api/show-once/recordings/{rec}"), None).await;
    assert_eq!(s, StatusCode::NO_CONTENT);
    assert!(env.state.db.get_recording(&rec).await.unwrap().is_none());
    assert_eq!(env.state.db.stored_step_rows(&rec).await.unwrap(), 0);
    let (s, _) = call(&env.app, "POST", &format!("/api/show-once/recordings/{rec}/write"), None).await;
    assert_eq!(s, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn stop_takes_handed_back_and_a_dropped_socket_is_closed() {
    use kwnt::server::show_once::stage::StageRecorder;
    let env = env().await;
    let bot = env.state.db.get_kwant(&env.a).await.unwrap().unwrap();
    let rec = env.state.db.start_recording(&bot.id, &bot.space_id, &bot.id, now()).await.unwrap();
    let stop = format!("/api/show-once/recordings/{}/stop", rec.id);
    for bad in [json!({"reason": "closed"}), json!({"reason": "limit_time"}), json!({"reason": 1})] {
        let (s, v) = call(&env.app, "POST", &stop, Some(bad.clone())).await;
        assert_eq!((s, v["error"].as_str()), (StatusCode::BAD_REQUEST, Some("bad_request")), "{bad}");
    }
    let (s, v) = call(&env.app, "POST", &stop, Some(json!({"reason": "handed_back"}))).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!((v["recording"]["state"].as_str(), v["recording"]["stop_reason"].as_str()), (Some("stopped"), Some("handed_back")));
    // Stopping twice is a no-op that keeps the first reason
    let (s, v) = call(&env.app, "POST", &stop, Some(json!({}))).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(v["recording"]["stop_reason"], "handed_back");

    // A26: no browser page yet, so nothing to prefill
    let (s, v) = call(&env.app, "GET", &format!("/api/show-once/recordings?bot={}", env.b), None).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert!(v.as_object().unwrap().contains_key("last_url") && v["last_url"].is_null(), "{v}");

    // With Take over held, Start records; the socket going away (tab closed, computer slept) stops
    // it as `closed`, not Hand back
    let mut stage = StageRecorder::new(env.state.clone(), env.b.clone(), 7);
    let id = live_recording(&env, &mut stage).await;
    stage.closed().await;
    let r = env.state.db.get_recording(&id).await.unwrap().unwrap();
    assert_eq!((r.state.as_str(), r.stop_reason.as_deref()), ("stopped", Some("closed")));
    let (_, v) = call(&env.app, "GET", &format!("/api/show-once/recordings?bot={}", env.b), None).await;
    assert_eq!(v["recording"]["id"], id.as_str(), "the review is offered again");
    assert_eq!(v["recording"]["stop_reason"], "closed");
}

/// Claims Take over on `stage`'s socket, starts a recording of bot `b` there and installs it.
async fn live_recording(env: &Env, stage: &mut kwnt::server::show_once::stage::StageRecorder) -> String {
    use kwnt::server::show_once::stage::Cmd;
    stage.claim().await;
    let (s, v) = call(&env.app, "POST", "/api/show-once/recordings", Some(json!({"bot": env.b, "first_note": "watch me"}))).await;
    assert_eq!(s, StatusCode::CREATED, "{v}");
    let id = v["recording"]["id"].as_str().unwrap().to_string();
    let cmd = next_cmd_for(stage, &id).await;
    assert!(matches!(&cmd, Cmd::Begin { recording_id, .. } if *recording_id == id), "{cmd:?}");
    stage.on_cmd(cmd).await;
    assert!(stage.is_live());
    id
}

#[tokio::test]
async fn hand_back_gives_handed_back_and_the_first_reason_wins() {
    use kwnt::server::show_once::stage::{Act, StageRecorder};
    let env = env().await;
    let stopped = |acts: &[Act]| {
        acts.iter()
            .find_map(|a| match a {
                Act::Send(f) if f["type"] == "skill.recording.stopped" => f["reason"].as_str().map(str::to_string),
                _ => None,
            })
    };

    // The explicit release (the Hand back button) gives `handed_back`; the stop the frontend sends
    // after it, and the socket closing later, keep it
    let mut stage = StageRecorder::new(env.state.clone(), env.b.clone(), 8);
    let id = live_recording(&env, &mut stage).await;
    let acts = stage.handed_back().await;
    assert_eq!(stopped(&acts).as_deref(), Some("handed_back"), "{acts:?}");
    let stop = format!("/api/show-once/recordings/{id}/stop");
    let (s, v) = call(&env.app, "POST", &stop, Some(json!({"reason": "handed_back"}))).await;
    assert_eq!((s, v["recording"]["stop_reason"].as_str()), (StatusCode::OK, Some("handed_back")), "{v}");
    stage.closed().await;
    let r = env.state.db.get_recording(&id).await.unwrap().unwrap();
    assert_eq!(r.stop_reason.as_deref(), Some("handed_back"));
    call(&env.app, "DELETE", &format!("/api/show-once/recordings/{id}"), None).await;

    // The race the other way: Hand back's stop lands first, then the release; still `handed_back`,
    // and the frame the stage shows says so
    let mut stage = StageRecorder::new(env.state.clone(), env.b.clone(), 9);
    let id = live_recording(&env, &mut stage).await;
    let (s, v) = call(&env.app, "POST", &format!("/api/show-once/recordings/{id}/stop"), Some(json!({"reason": "handed_back"}))).await;
    assert_eq!((s, v["recording"]["stop_reason"].as_str()), (StatusCode::OK, Some("handed_back")), "{v}");
    let r = env.state.db.get_recording(&id).await.unwrap().unwrap();
    assert_eq!(r.stop_reason.as_deref(), Some("handed_back"));
    let acts = stage.handed_back().await;
    assert!(stopped(&acts).is_none_or(|r| r == "handed_back"), "{acts:?}");
    stage.closed().await;
    let r = env.state.db.get_recording(&id).await.unwrap().unwrap();
    assert_eq!((r.state.as_str(), r.stop_reason.as_deref()), ("stopped", Some("handed_back")));
}

#[tokio::test]
async fn the_test_seams_are_routes_only_with_the_test_seams_feature() {
    use axum::body::Body;
    use axum::http::Request;
    use tower::ServiceExt;
    let env = env().await;
    // `cargo test` builds kwnt with the feature (the dev-dependency in Cargo.toml)
    const { assert!(kwnt::clock::TEST_SEAMS) };
    let post = |uri: &str| Request::post(uri).header("content-type", "application/json").body(Body::from("{}")).unwrap();
    // Without the feature (a release, `make run` or Docker build) the routes don't exist
    let without = kwnt::server::show_once::routes::router_with(false).with_state(env.state.clone());
    for uri in ["/api/show-once/test/clock", "/api/show-once/test/sweep"] {
        let r = without.clone().oneshot(post(uri)).await.unwrap();
        assert_eq!(r.status(), StatusCode::NOT_FOUND, "{uri}");
    }
    let with = kwnt::server::show_once::routes::router_with(true).with_state(env.state.clone());
    let r = with.oneshot(post("/api/show-once/test/sweep")).await.unwrap();
    assert_eq!(r.status(), StatusCode::OK);
    // No show_once seam is gated on debug_assertions any more (#399 review H1)
    for f in std::fs::read_dir(concat!(env!("CARGO_MANIFEST_DIR"), "/src/server/show_once")).unwrap() {
        let path = f.unwrap().path();
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(!text.contains("debug_assertions"), "{}", path.display());
    }
}

#[tokio::test]
async fn a_failed_trial_marks_its_failure_note_before_the_chat_refreshes() {
    let env = env().await;
    let rec = stopped_recording(&env, &format!("REPLYTEXT:{}", b64(&canned_draft()))).await;
    let skill = written_skill(&env, &rec).await;
    let skill_id = skill["id"].as_str().unwrap().to_string();
    let mut events = env.state.event_sender.subscribe();
    let try_uri = format!("/api/show-once/skills/{skill_id}/try");
    let mut tried = false;
    for _ in 0..300 {
        let (s, v) = call(&env.app, "POST", &try_uri, Some(json!({"inputs": {"date_range": "FAILTURN"}}))).await;
        if s == StatusCode::ACCEPTED {
            tried = true;
            break;
        }
        assert_eq!(v["error"], "bot_busy", "{v}");
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    assert!(tried);
    let trial = json!({"skill_id": skill_id, "recording_id": rec});
    let meta = |m: &kwnt::db::models::Message| serde_json::from_str::<Value>(m.meta.as_deref().unwrap_or("{}")).unwrap();
    // The chat refreshes on the turn's error event: the failure note it reads then has the marker
    let mut at_error = None;
    let mut seen = Vec::new();
    let ended = tokio::time::timeout(std::time::Duration::from_secs(60), async {
        loop {
            // The turn's own events come wrapped in its tag
            let e = match events.recv().await {
                Ok(kwnt::RuntimeEvent::Scoped { event, .. }) => Ok(*event),
                e => e,
            };
            if let Ok(e) = &e {
                seen.push(format!("{e:?}").chars().take(160).collect::<String>());
            }
            match e {
                Ok(kwnt::RuntimeEvent::Error { kwant_id: Some(k), .. }) if k == env.a && at_error.is_none() => {
                    let note = messages(&env, &env.a).await.into_iter().rev().find(|m| meta(m)["kind"] == "turn_failed");
                    at_error = Some(note.expect("the failure note is saved before the error event"));
                }
                Ok(kwnt::RuntimeEvent::TrialEnded { outcome, .. }) => break outcome,
                Ok(_) | Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                Err(e) => panic!("{e}"),
            }
        }
    })
    .await
    .expect("show_once_trial_ended");
    assert_eq!(ended, "failed");
    let note = at_error.unwrap_or_else(|| panic!("no error event for the bot: {seen:#?}"));
    assert_eq!(meta(&note)["trial"]["skill_id"], skill_id.as_str(), "{:?}", note.meta);
    assert_eq!(meta(&note)["trial"]["recording_id"], rec.as_str(), "{:?}", note.meta);
    // After the end, the note is the trial's last message: it carries the outcome; the line doesn't
    let note = env.state.db.get_message(&note.id).await.unwrap().unwrap();
    assert_eq!(meta(&note)["trial"]["outcome"], "failed", "{:?}", note.meta);
    assert_eq!(meta(&note)["kind"], "turn_failed");
    let line = messages(&env, &env.a).await.into_iter().find(|m| m.text.contains("You started a trial")).unwrap();
    assert_eq!(meta(&line)["trial"], trial, "{:?}", line.meta);
    // The skill is still waiting for review
    let (_, v) = call(&env.app, "GET", &format!("/api/skills/{skill_id}"), None).await;
    assert_eq!(v["status"], "pending", "{v}");
}

#[tokio::test]
async fn another_window_taking_over_stops_the_recording_as_closed() {
    use kwnt::server::show_once::stage::{Act, Cmd, StageRecorder};
    let env = env().await;
    let mut first = StageRecorder::new(env.state.clone(), env.b.clone(), 21);
    let id = live_recording(&env, &mut first).await;

    assert!(first.on_installed(LOGIN).await.iter().any(is_step));

    // A second window takes Take over on the same stage while Email still has focus in the first:
    // the first window takes that edit (B2), then stops with `closed`
    let mut second = StageRecorder::new(env.state.clone(), env.b.clone(), 22);
    let ((), acts) = tokio::join!(second.claim(), async {
        let cmd = next_cmd_for(&mut first, &id).await;
        assert!(matches!(&cmd, Cmd::Stop { recording_id, reason, .. } if *recording_id == id && reason == "closed"), "{cmd:?}");
        let acts = first.on_cmd(cmd).await;
        drive(&mut first, acts, &[pending("Email", "email", Some("ops@example.com"))]).await
    });
    let r = env.state.db.get_recording(&id).await.unwrap().unwrap();
    assert_eq!((r.state.as_str(), r.stop_reason.as_deref()), ("stopped", Some("closed")));
    assert!(!second.is_live());
    let last = env.state.db.list_steps(&id, now()).await.unwrap().pop().unwrap();
    assert_eq!((last.kind.as_str(), last.value.as_deref()), ("type", Some("ops@example.com")));

    // The first window takes its recorder off and shows `closed`; the End after it changes nothing
    assert!(acts.contains(&Act::Uninstall), "{acts:?}");
    assert!(acts.iter().any(|a| matches!(a, Act::Send(f) if f["type"] == "skill.recording.stopped" && f["reason"] == "closed")), "{acts:?}");
    assert!(!first.is_live());
    let cmd = next_cmd_for(&mut first, &id).await;
    assert_eq!(cmd, Cmd::End { recording_id: id.clone(), reason: "closed".into() });
    assert!(first.on_cmd(cmd).await.is_empty());
    // Its link is gone: one None, then nothing (no busy loop)
    // (skipping other tests' Discards that reached it before its link went)
    let none = loop {
        match tokio::time::timeout(std::time::Duration::from_secs(5), first.next_cmd()).await.unwrap() {
            Some(Cmd::End { recording_id, .. }) if recording_id != id => continue,
            other => break other,
        }
    };
    assert!(none.is_none(), "{none:?}");
    assert!(tokio::time::timeout(std::time::Duration::from_millis(200), first.next_cmd()).await.is_err());
    // The first window closing later changes nothing: the first reason wins and the second keeps the stage
    first.closed().await;
    assert!(kwnt::server::show_once::stage::taken_over(&env.b));
    let r = env.state.db.get_recording(&id).await.unwrap().unwrap();
    assert_eq!(r.stop_reason.as_deref(), Some("closed"));
    let (_, v) = call(&env.app, "GET", &format!("/api/show-once/recordings?bot={}", env.b), None).await;
    assert_eq!(v["recording"]["id"], id.as_str(), "the review is offered again");
    assert_eq!(v["recording"]["stop_reason"], "closed");
    assert_eq!(v["recording"]["step_count"], r.step_count);

    // A stop that was already there wins over a takeover: Hand back, then another window
    let (s, _) = call(&env.app, "DELETE", &format!("/api/show-once/recordings/{id}"), None).await;
    assert_eq!(s, StatusCode::NO_CONTENT);
    second.handed_back().await;
    let mut third = StageRecorder::new(env.state.clone(), env.b.clone(), 23);
    let id = live_recording(&env, &mut third).await;
    third.handed_back().await;
    let mut fourth = StageRecorder::new(env.state.clone(), env.b.clone(), 24);
    fourth.claim().await;
    let r = env.state.db.get_recording(&id).await.unwrap().unwrap();
    assert_eq!(r.stop_reason.as_deref(), Some("handed_back"));
}

/// Where the B1/B2 tests' browser is (Iris's mock shop).
const LOGIN: &str = "http://127.0.0.1:9483/shop/login.html";

fn is_step(a: &kwnt::server::show_once::stage::Act) -> bool {
    matches!(a, kwnt::server::show_once::stage::Act::Send(f) if f["type"] == "skill.recording.step")
}

/// What the recorder's flush returns for a field edited but not committed yet (its focus is still
/// there). A password field's payload carries a value too, which the server must drop.
fn pending(label: &str, input_type: &str, value: Option<&str>) -> String {
    let password = input_type == "password";
    let mut p = json!({
        "kind": "type", "url": LOGIN, "trusted": true, "label": label,
        "masked": if password { json!("password") } else { Value::Null },
        "field": {"type": input_type, "ever_password": password, "autocomplete": "", "name": "",
                  "id": label.to_lowercase(), "labels": [label], "script_filled": false},
    });
    if let Some(v) = value {
        p["value"] = json!(v);
    }
    p.to_string()
}

/// Does what the stream socket does with the stage's acts, without a browser: a flush returns
/// `pending`, a stop stops and fires its `done`. The rest (frames, recorder on/off) comes back.
async fn drive(
    stage: &mut kwnt::server::show_once::stage::StageRecorder,
    acts: Vec<kwnt::server::show_once::stage::Act>,
    pending: &[String],
) -> Vec<kwnt::server::show_once::stage::Act> {
    use kwnt::server::show_once::stage::Act;
    let mut queue: std::collections::VecDeque<Act> = acts.into();
    let mut out = vec![];
    while let Some(act) = queue.pop_front() {
        match act {
            Act::Flush => queue.extend(stage.on_flushed(pending).await),
            Act::Stop { reason, done } => {
                queue.extend(stage.stop_now(&reason).await);
                done.fire();
            }
            other => out.push(other),
        }
    }
    out
}

async fn steps_of(env: &Env, id: &str) -> Vec<(String, Option<String>)> {
    let steps = env.state.db.list_steps(id, now()).await.unwrap();
    steps.into_iter().map(|s| (s.kind, s.path)).collect()
}

#[tokio::test]
async fn a_start_url_is_the_first_page_and_the_page_before_isnt_a_step() {
    use kwnt::server::show_once::stage::{Act, StageRecorder};
    let env = env().await;
    let start = "http://127.0.0.1:9483/";
    let mut stage = StageRecorder::new(env.state.clone(), env.b.clone(), 31);
    stage.claim().await;
    let body = json!({"bot": env.b, "first_note": "watch me", "start_url": start});
    let (s, v) = call(&env.app, "POST", "/api/show-once/recordings", Some(body)).await;
    assert_eq!(s, StatusCode::CREATED, "{v}");
    let id = v["recording"]["id"].as_str().unwrap().to_string();
    let cmd = next_cmd_for(&mut stage, &id).await;
    let acts = stage.on_cmd(cmd).await;
    assert!(acts.contains(&Act::Install) && acts.contains(&Act::Navigate(start.into())), "{acts:?}");

    // B1: the recorder goes on while the browser is still on the orders page: not a step
    assert!(stage.on_installed("http://127.0.0.1:9483/shop/orders.html").await.is_empty());
    // the start page loads: the first page step; then the person's own clicks and pages
    assert!(stage.on_navigated(start).await.iter().any(is_step));
    let click = json!({"kind": "click", "url": start, "trusted": true, "role": "link", "name": "Orders"});
    assert!(stage.on_binding(&click.to_string()).await.iter().any(is_step));
    assert!(stage.on_navigated("http://127.0.0.1:9483/shop/orders.html").await.iter().any(is_step));
    let steps = steps_of(&env, &id).await;
    let kinds: Vec<&str> = steps.iter().map(|(k, _)| k.as_str()).collect();
    assert_eq!(kinds, ["note", "page", "click", "page"], "{steps:?}");
    assert_eq!(steps[1].1.as_deref(), Some("/"));
    assert_eq!(steps[3].1.as_deref(), Some("/shop/orders.html"));
    stage.handed_back().await;
    call(&env.app, "DELETE", &format!("/api/show-once/recordings/{id}"), None).await;

    // Without start_url the page the browser is on is the first page step, as before
    let id = live_recording(&env, &mut stage).await;
    assert!(stage.on_installed("http://127.0.0.1:9483/shop/orders.html").await.iter().any(is_step));
    let steps = steps_of(&env, &id).await;
    assert_eq!(steps.iter().map(|(k, _)| k.as_str()).collect::<Vec<_>>(), ["note", "page"], "{steps:?}");
    assert_eq!(steps[1].1.as_deref(), Some("/shop/orders.html"));
    stage.handed_back().await;
    call(&env.app, "DELETE", &format!("/api/show-once/recordings/{id}"), None).await;

    // A start_url that can't be opened: the page the browser stayed on is the first step after all
    stage.claim().await;
    let body = json!({"bot": env.b, "first_note": "watch me", "start_url": "http://127.0.0.1:9483/gone"});
    let (s, v) = call(&env.app, "POST", "/api/show-once/recordings", Some(body)).await;
    assert_eq!(s, StatusCode::CREATED, "{v}");
    let id = v["recording"]["id"].as_str().unwrap().to_string();
    let cmd = next_cmd_for(&mut stage, &id).await;
    stage.on_cmd(cmd).await;
    assert!(stage.on_installed("http://127.0.0.1:9483/shop/orders.html").await.is_empty());
    assert!(stage.on_start_failed(Some("http://127.0.0.1:9483/shop/orders.html")).await.iter().any(is_step));
    let steps = steps_of(&env, &id).await;
    assert_eq!(steps.iter().map(|(k, _)| k.as_str()).collect::<Vec<_>>(), ["note", "page"], "{steps:?}");
}

#[tokio::test]
async fn every_stop_takes_the_pending_edit_first_masked_and_within_the_cap() {
    use kwnt::server::show_once::stage::{Cmd, Done, StageRecorder};
    let env = env().await;
    let reason_of = |r: &kwnt::db::show_once::Recording| (r.state.clone(), r.stop_reason.clone());

    // The socket drops (tab closed) with "ops@example.com" typed in Email, focus still there
    let mut stage = StageRecorder::new(env.state.clone(), env.b.clone(), 41);
    let id = live_recording(&env, &mut stage).await;
    stage.on_installed(LOGIN).await;
    let acts = stage.on_flushed(&[pending("Email", "email", Some("ops@example.com"))]).await;
    assert!(acts.iter().any(is_step), "{acts:?}");
    stage.closed().await;
    let r = env.state.db.get_recording(&id).await.unwrap().unwrap();
    assert_eq!(reason_of(&r), ("stopped".into(), Some("closed".into())));
    let last = env.state.db.list_steps(&id, now()).await.unwrap().pop().unwrap();
    assert_eq!(
        (last.kind.as_str(), last.label.as_deref(), last.value.as_deref(), last.masked.as_deref(), last.path.as_deref()),
        ("type", Some("Email"), Some("ops@example.com"), None, Some("/shop/login.html"))
    );
    call(&env.app, "DELETE", &format!("/api/show-once/recordings/{id}"), None).await;

    // Stop (the route) with a password typed: the stage takes it, masked with no value, then stops
    let mut stage = StageRecorder::new(env.state.clone(), env.b.clone(), 42);
    let id = live_recording(&env, &mut stage).await;
    stage.on_installed(LOGIN).await;
    let stop = format!("/api/show-once/recordings/{id}/stop");
    let ((s, v), acts) = tokio::join!(call(&env.app, "POST", &stop, None), async {
        let cmd = next_cmd_for(&mut stage, &id).await;
        assert!(matches!(&cmd, Cmd::Stop { reason, .. } if reason == "stop"), "{cmd:?}");
        let acts = stage.on_cmd(cmd).await;
        drive(&mut stage, acts, &[pending("Password", "password", Some("hunter2"))]).await
    });
    assert_eq!((s, v["recording"]["stop_reason"].as_str()), (StatusCode::OK, Some("stop")), "{v}");
    assert!(acts.iter().any(|a| matches!(a, kwnt::server::show_once::stage::Act::Send(f) if f["type"] == "skill.recording.stopped" && f["reason"] == "stop")), "{acts:?}");
    let last = env.state.db.list_steps(&id, now()).await.unwrap().pop().unwrap();
    assert_eq!((last.kind.as_str(), last.value.as_deref(), last.masked.as_deref()), ("type", None, Some("password")));
    assert_eq!(v["recording"]["step_count"], 2, "page + the masked type step");
    stage.closed().await;
    call(&env.app, "DELETE", &format!("/api/show-once/recordings/{id}"), None).await;

    // The 30-minute limit: the edit is still taken (the time limit doesn't refuse it), reason limit_time
    let mut stage = StageRecorder::new(env.state.clone(), env.b.clone(), 43);
    stage.claim().await;
    let bot = env.state.db.get_kwant(&env.b).await.unwrap().unwrap();
    let limit = kwnt::db::show_once::LIMIT_SECS;
    let rec = env.state.db.start_recording(&bot.id, &bot.space_id, &env.b, now() - limit - 5).await.unwrap();
    stage.on_cmd(Cmd::Begin { recording_id: rec.id.clone(), start_url: None }).await;
    assert!(stage.is_live());
    let acts = stage.tick().await;
    assert!(matches!(acts.as_slice(), [kwnt::server::show_once::stage::Act::Flush, kwnt::server::show_once::stage::Act::Stop { reason, .. }] if reason == "limit_time"), "{acts:?}");
    drive(&mut stage, acts, &[pending("Email", "email", Some("ops@example.com"))]).await;
    let r = env.state.db.get_recording(&rec.id).await.unwrap().unwrap();
    assert_eq!(reason_of(&r), ("stopped".into(), Some("limit_time".into())));
    let last = env.state.db.list_steps(&rec.id, now()).await.unwrap().pop().unwrap();
    assert_eq!((last.kind.as_str(), last.value.as_deref()), ("type", Some("ops@example.com")));
    stage.handed_back().await;
    call(&env.app, "DELETE", &format!("/api/show-once/recordings/{}", rec.id), None).await;

    // 200 steps already: the edit would be step 201, so it's dropped and the stop's reason stands
    let mut stage = StageRecorder::new(env.state.clone(), env.b.clone(), 44);
    let id = live_recording(&env, &mut stage).await;
    let click = NewStep { kind: "click".into(), origin: Some("http://127.0.0.1:9483".into()), path: Some("/".into()),
        role: Some("button".into()), name: Some("Next".into()), ..Default::default() };
    while env.state.db.get_recording(&id).await.unwrap().unwrap().step_count < kwnt::db::show_once::LIMIT_STEPS {
        env.state.db.add_step(&id, &click, now()).await.unwrap();
    }
    assert!(stage.on_flushed(&[pending("Email", "email", Some("ops@example.com"))]).await.is_empty());
    stage.closed().await;
    let r = env.state.db.get_recording(&id).await.unwrap().unwrap();
    assert_eq!(reason_of(&r), ("stopped".into(), Some("closed".into())));
    assert_eq!(r.step_count, kwnt::db::show_once::LIMIT_STEPS);

    // A stop for a recording that isn't live on the stage just fires its done
    let (done, wait) = Done::new();
    assert!(stage.on_cmd(Cmd::Stop { recording_id: id, reason: "stop".into(), done }).await.is_empty());
    assert!(wait.await.is_ok());
}

/// The stage's next command about recording `id`. Discard tells every linked stage (tests run in
/// parallel), so commands about other tests' recordings are skipped.
async fn next_cmd_for(stage: &mut kwnt::server::show_once::stage::StageRecorder, id: &str) -> kwnt::server::show_once::stage::Cmd {
    use kwnt::server::show_once::stage::Cmd;
    loop {
        let cmd = tokio::time::timeout(std::time::Duration::from_secs(5), stage.next_cmd()).await.unwrap().unwrap();
        let about = match &cmd {
            Cmd::Begin { recording_id, .. } | Cmd::Stop { recording_id, .. } | Cmd::End { recording_id, .. } => Some(recording_id.as_str()),
            Cmd::Frame(_) => None,
        };
        if about.is_none_or(|r| r == id) {
            return cmd;
        }
    }
}
