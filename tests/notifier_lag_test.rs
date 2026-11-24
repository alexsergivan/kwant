//! notifier-lag-catches-up (0.9.51): when the event bus outruns the push/webhook notifier, a
//! person who is away still hears that a bot is waiting for their approval: once, and never for
//! a card already answered.

use kwnt::{
    broker::PermissionBroker,
    config::Config,
    db::{models::NewApproval, spaces::SpaceId, Database},
    drivers::events::RuntimeEvent,
    router::LayaRouter,
    server::{continuity, AppState},
    tools::ToolRunner,
};
use serde_json::json;
use std::time::Duration;
use tempfile::{tempdir, TempDir};

/// More events than any bus size kwant uses (128 on main, 1024 after #1181), so the notifier lags.
const FLOOD: usize = 4096;

async fn setup() -> (AppState, TempDir, String) {
    let tmp = tempdir().unwrap();
    let db = Database::init(tmp.path()).await.unwrap();
    db.set_setting("mission_board", "false").await.unwrap();
    db.set_setting("notify_on", r#"["approval_needed"]"#).await.unwrap();
    let config = Config {
        data_dir: tmp.path().to_path_buf(),
        ..Config::default()
    };
    let state = AppState::new(
        config.clone(),
        db,
        ToolRunner::new(tmp.path().join("workspace")),
        PermissionBroker::new(),
        LayaRouter::new(config.laya_url.clone()),
    );
    let k = state.db.list_kwants(&SpaceId::default_space()).await.unwrap().remove(0);
    (state, tmp, k.id)
}

/// A pending card of `kwant_id`, as the MCP runtime stores it before it sends its event.
async fn card(state: &AppState, kwant_id: &str) -> (String, serde_json::Value) {
    let details = json!({"kwant_id": kwant_id, "kwant_name": "Laya", "source": "mcp"});
    let id = state
        .db
        .save_approval(&NewApproval {
            turn_id: "turn-1".into(),
            kwant_id: kwant_id.into(),
            tool_name: "mcp__github__create_issue".into(),
            parameters: details.clone(),
        })
        .await
        .unwrap()
        .id;
    (id, details)
}

fn asked(id: &str, details: &serde_json::Value) -> RuntimeEvent {
    RuntimeEvent::ApprovalRequired {
        id: id.into(),
        tool_name: "mcp__github__create_issue".into(),
        details: details.clone(),
    }
}

/// Events nobody notifies about, sent without yielding: the parked notifier falls behind.
fn flood(state: &AppState) {
    for _ in 0..FLOOD {
        let _ = state.event_sender.send(RuntimeEvent::TextDelta("x".into()));
    }
}

/// The `approval_needed` notices (their card ids) seen on `rx` within `wait`.
async fn approval_notices(rx: &mut tokio::sync::broadcast::Receiver<RuntimeEvent>, wait: Duration) -> Vec<String> {
    let mut out = Vec::new();
    let deadline = tokio::time::Instant::now() + wait;
    while tokio::time::Instant::now() < deadline {
        match tokio::time::timeout(Duration::from_millis(100), rx.recv()).await {
            Ok(Ok(RuntimeEvent::Notify { kind, card_id, .. })) if kind == "approval_needed" => out.push(card_id.unwrap_or_default()),
            Ok(Err(tokio::sync::broadcast::error::RecvError::Closed)) => break,
            _ => {}
        }
    }
    out
}

/// The card's event is lost to a lag: its notice still goes out, exactly once.
#[tokio::test]
async fn an_approval_lost_to_a_lag_is_notified_once() {
    let (state, _tmp, k) = setup().await;
    let notifier = continuity::spawn_notifier(state.clone());
    tokio::task::yield_now().await;
    let (id, details) = card(&state, &k).await;
    // Its event, then a flood: the notifier reads Lagged, never the event
    let _ = state.event_sender.send(asked(&id, &details));
    flood(&state);
    let mut rx = state.event_sender.subscribe();
    let got = approval_notices(&mut rx, Duration::from_secs(3)).await;
    notifier.abort();
    assert_eq!(got, vec![id], "the approval lost to the lag is notified exactly once");
}

/// A card answered while the notifier lagged gets no notice; one still pending does.
#[tokio::test]
async fn an_approval_answered_during_the_lag_is_not_notified() {
    let (state, _tmp, k) = setup().await;
    let notifier = continuity::spawn_notifier(state.clone());
    tokio::task::yield_now().await;
    let (answered, a_details) = card(&state, &k).await;
    let (pending, p_details) = card(&state, &k).await;
    state.db.update_approval_status(&answered, "allowed_once").await.unwrap();
    let _ = state.event_sender.send(asked(&answered, &a_details));
    let _ = state.event_sender.send(asked(&pending, &p_details));
    flood(&state);
    let mut rx = state.event_sender.subscribe();
    let got = approval_notices(&mut rx, Duration::from_secs(3)).await;
    notifier.abort();
    assert!(!got.contains(&answered), "an answered card is never notified after a lag: {got:?}");
    assert_eq!(got, vec![pending], "the card still pending is notified once");
}

/// The live event arrives after the catch-up already sent its notice (it was still in the bus
/// past the lag): one notice, not two.
#[tokio::test]
async fn a_catch_up_and_the_live_event_never_notify_twice() {
    let (state, _tmp, k) = setup().await;
    let notifier = continuity::spawn_notifier(state.clone());
    tokio::task::yield_now().await;
    let (id, details) = card(&state, &k).await;
    flood(&state);
    // Kept by the bus after the lag: the notifier reads it after its catch-up
    let _ = state.event_sender.send(asked(&id, &details));
    let mut rx = state.event_sender.subscribe();
    let got = approval_notices(&mut rx, Duration::from_secs(3)).await;
    notifier.abort();
    assert_eq!(got, vec![id], "one notice for the card");
}

/// No lag: each live card is notified once, as before; a card created before the notifier
/// started (another run's) is not dug up by anything.
#[tokio::test]
async fn without_a_lag_live_cards_are_notified_as_before() {
    let (state, _tmp, k) = setup().await;
    let (old, _) = card(&state, &k).await;
    let notifier = continuity::spawn_notifier(state.clone());
    tokio::task::yield_now().await;
    let mut rx = state.event_sender.subscribe();
    let (a, a_details) = card(&state, &k).await;
    let (b, b_details) = card(&state, &k).await;
    let _ = state.event_sender.send(asked(&a, &a_details));
    let _ = state.event_sender.send(asked(&b, &b_details));
    let got = approval_notices(&mut rx, Duration::from_secs(2)).await;
    notifier.abort();
    assert_eq!(got, vec![a, b]);
    assert!(!got.contains(&old));
}
