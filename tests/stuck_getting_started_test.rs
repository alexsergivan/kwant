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

// ---- The server's safety net (src/server/lost_turns.rs) and Stop ----

use kwnt::db::continuity::RunningTurn;
use kwnt::db::models::{NewChannel, NewKwant};
use kwnt::db::spaces::SpaceId;
use kwnt::server::lost_turns::{self, Lost};
use kwnt::server::turns::ActiveTurn;
use std::collections::HashSet;

fn listed(turn_id: &str, context_id: &str, minutes_ago: i64) -> ActiveTurn {
    ActiveTurn {
        turn_id: turn_id.into(),
        kwant_id: "quant".into(),
        context_id: context_id.into(),
        thread_id: None,
        state: "running".into(),
        since: chrono::Utc::now() - chrono::Duration::minutes(minutes_ago),
        purpose: Some("work_ahead".into()),
        audit: None,
    }
}

/// A team chat led by a real bot in the default space: `create_channel` refuses a lead that isn't one
/// (the API fills an empty lead from the first member before it gets there).
async fn team_chat(state: &AppState) -> String {
    let space = SpaceId::default_space();
    let quant = NewKwant {
        name: "Quant".into(),
        title: "Quant researcher".into(),
        persona_prompt: "Backtests trading ideas.".into(),
        soul: String::new(),
        avatar_sprite: "ben".into(),
        driver_type: "api".into(),
        preferred_model: "auto".into(),
    };
    let lead = state.db.create_kwant(&space, &quant).await.unwrap().id;
    let input = NewChannel {
        name: "Binance Trading Lab".into(),
        description: None,
        lead_kwant_id: lead.clone(),
        member_kwants: vec![lead],
        routing_mode: "lead".into(),
    };
    state.db.create_channel(&space, &input).await.unwrap().id
}

/// The `turn_ended` events of `turn_id` waiting in `rx`.
fn ends_of(rx: &mut tokio::sync::broadcast::Receiver<RuntimeEvent>, turn_id: &str) -> usize {
    let mut n = 0;
    while let Ok(e) = rx.try_recv() {
        if let RuntimeEvent::TurnEnded { turn_id: Some(t), .. } = &e {
            n += usize::from(t == turn_id);
        }
    }
    n
}

const SIX_MIN: Duration = Duration::from_secs(360);

#[tokio::test]
async fn the_sweep_ends_a_listed_turn_no_job_holds() {
    let tmp = tempdir().unwrap();
    let state = setup(tmp.path()).await;
    let mut rx = state.event_sender.subscribe();
    // Quant's work-ahead turn, listed 85 minutes after its work went away
    state.turns.set_turn(listed("wa-1", "team-chat", 85));
    // A fresh one no job holds yet is left alone
    state.turns.set_turn(listed("wa-2", "team-chat", 0));

    let ended = lost_turns::sweep(&state, &mut HashSet::new(), chrono::Utc::now(), SIX_MIN).await;
    assert_eq!(ended, vec![("wa-1".to_string(), Lost::Orphan)]);
    assert!(!state.turns.has_turn("wa-1"));
    assert!(state.turns.has_turn("wa-2"));
    assert_eq!(ends_of(&mut rx, "wa-1"), 1, "every app hears it ended");
}

#[tokio::test]
async fn the_sweep_stops_a_silent_turn_with_a_line_in_its_chat_then_unlists_it() {
    let tmp = tempdir().unwrap();
    let state = setup(tmp.path()).await;
    let chat = team_chat(&state).await;
    let mut rx = state.event_sender.subscribe();
    let job = state.turns.start_job([chat.clone(), "t-silent".to_string()]);
    state.turns.set_turn(listed("t-silent", &chat, 90));
    state
        .db
        .add_running_turn(&RunningTurn {
            turn_id: "t-silent".into(),
            kwant_id: "quant".into(),
            kwant_name: "Quant".into(),
            channel_id: Some(chat.clone()),
            started_at: chrono::Utc::now() - chrono::Duration::minutes(90),
            ..Default::default()
        })
        .await
        .unwrap();

    let mut cancelled = HashSet::new();
    let ended = lost_turns::sweep(&state, &mut cancelled, chrono::Utc::now(), SIX_MIN).await;
    assert_eq!(ended, vec![("t-silent".to_string(), Lost::Silent)]);
    assert!(job.token.is_cancelled(), "its work is told to stop");
    let line = state.db.list_messages(&chat, 10).await.unwrap().into_iter().find(|m| m.sender_role == "system").expect("a line in the chat");
    assert_eq!(line.text, "Quant got stuck for 6 minutes, so the work was stopped. Ask again if you still need it.");
    assert!(line.meta.as_deref().unwrap_or_default().contains("\"turn_lost\""), "{:?}", line.meta);

    // Its work never winds down (a hung engine): the next sweep unlists it, without a second line
    let ended = lost_turns::sweep(&state, &mut cancelled, chrono::Utc::now(), SIX_MIN).await;
    assert_eq!(ended, vec![("t-silent".to_string(), Lost::Silent)]);
    assert!(!state.turns.has_turn("t-silent"));
    assert_eq!(ends_of(&mut rx, "t-silent"), 1);
    let lines = state.db.list_messages(&chat, 10).await.unwrap().into_iter().filter(|m| m.sender_role == "system").count();
    assert_eq!(lines, 1);
    drop(job);
}

#[tokio::test]
async fn the_sweep_leaves_a_quiet_turn_that_waits_for_its_slot_or_is_recent() {
    let tmp = tempdir().unwrap();
    let state = setup(tmp.path()).await;
    let _job = state.turns.start_job(["t-recent".to_string(), "t-queued".to_string()]);
    state.turns.set_turn(listed("t-recent", "team-chat", 2));
    state.turns.set_turn(ActiveTurn { state: "queued".into(), ..listed("t-queued", "team-chat", 90) });
    let ended = lost_turns::sweep(&state, &mut HashSet::new(), chrono::Utc::now(), SIX_MIN).await;
    assert!(ended.is_empty(), "{ended:?}");
    assert!(state.turns.has_turn("t-recent") && state.turns.has_turn("t-queued"));
}

#[tokio::test]
async fn stop_clears_a_chat_whose_turn_nothing_runs() {
    let tmp = tempdir().unwrap();
    let state = setup(tmp.path()).await;
    let mut rx = state.event_sender.subscribe();
    state.turns.set_turn(listed("wa-1", "team-chat", 85));
    state.turns.set_turn(listed("other", "another-chat", 85));
    let app = create_router(state.clone());
    let resp = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/chat/cancel")
                .header("content-type", "application/json")
                .body(Body::from(r#"{"channel_id":"team-chat"}"#))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    assert!(!state.turns.has_turn("wa-1"), "Stop leaves the chat 'getting started'");
    assert!(state.turns.has_turn("other"), "another chat's Stop is its own");
    assert_eq!(ends_of(&mut rx, "wa-1"), 1);
}
