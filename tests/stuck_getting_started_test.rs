//! stuck-getting-started (0.9.51): a team chat said "Quant is getting started · 1:25:45" with a Stop
//! button while nothing ran. Quant's work-ahead turn started (20:04) and ended (its "While you were
//! away" post, 20:07), but the app never heard `turn_ended`: the live stream (`GET /api/events`)
//! silently skipped every event it had fallen behind on and stayed open, so the app had no reason to
//! read the running turns again. Notes: autonomy/stuck-getting-started/impl.md.

use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use futures::StreamExt;
use kwnt::{
    broker::PermissionBroker,
    config::Config,
    db::Database,
    drivers::events::RuntimeEvent,
    router::LayaRouter,
    server::{create_router, AppState},
    tools::ToolRunner,
};
use std::time::Duration;
use tempfile::tempdir;
use tower::ServiceExt;

async fn setup(tmp: &std::path::Path) -> AppState {
    let db = Database::init(tmp).await.unwrap();
    let config = Config { data_dir: tmp.join("data"), ..Config::default() };
    std::fs::create_dir_all(&config.data_dir).unwrap();
    AppState::new(config.clone(), db, ToolRunner::new(tmp.join("workspace")), PermissionBroker::new(), LayaRouter::new(config.laya_url.clone()))
}

fn started(turn: &str) -> RuntimeEvent {
    RuntimeEvent::TurnStarted { kwant_id: "quant".into(), context_id: Some("team-chat".into()), turn_id: Some(turn.into()), thread_id: None, audit: None }
}

fn ended(turn: &str) -> RuntimeEvent {
    RuntimeEvent::TurnEnded {
        kwant_id: "quant".into(),
        context_id: Some("team-chat".into()),
        outcome: "completed".into(),
        turn_id: Some(turn.into()),
        thread_id: None,
        quiet: true,
        reply: None,
    }
}

#[tokio::test]
async fn a_stream_that_lost_turn_ended_closes_so_the_app_reads_the_running_turns_again() {
    let tmp = tempdir().unwrap();
    let state = setup(tmp.path()).await;
    let app = create_router(state.clone());
    let resp = app.oneshot(Request::builder().uri("/api/events").body(Body::empty()).unwrap()).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let mut body = resp.into_body().into_data_stream();

    // The app sees the turn start: "Quant is getting started"
    let _ = state.event_sender.send(started("wa-1"));
    let first = tokio::time::timeout(Duration::from_secs(5), body.next()).await.expect("turn_started reaches the app").unwrap().unwrap();
    assert!(String::from_utf8_lossy(&first).contains("turn_started"), "{first:?}");

    // Then the app stops reading for a while (asleep, in the background, a slow link) while the turn
    // ends and other bots keep streaming: more events than the server keeps for one reader
    let _ = state.event_sender.send(ended("wa-1"));
    for _ in 0..5000 {
        let _ = state.event_sender.send(RuntimeEvent::MemoryChanged { kwant_id: "backtester".into() });
    }

    // When it reads again, it must either get turn_ended or lose the connection (the app reconnects
    // and re-reads GET /api/turns on every connect). Skipping it on an open stream leaves the chat
    // "getting started" with no end
    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    let (mut closed, mut got_end) = (false, false);
    while !closed && !got_end {
        match tokio::time::timeout_at(deadline, body.next()).await {
            Ok(None) | Ok(Some(Err(_))) => closed = true,
            Ok(Some(Ok(chunk))) => got_end = String::from_utf8_lossy(&chunk).contains("\"turn_ended\""),
            Err(_) => break,
        }
    }
    assert!(
        closed || got_end,
        "the stream skipped turn_ended and stayed open: the app keeps showing the turn as running and never re-reads /api/turns"
    );
}
