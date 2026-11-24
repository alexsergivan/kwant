//! pa3b (pa3 qa F2, and pa2 registered with pa1): two `pocket_open` calls on one Ready quiz at
//! the same moment open one pocket, not two; the quiz kit is a Pocket core kit.
//! Its own test binary: the slow fake core is registered once per process.

mod quizmaster_support;

use async_trait::async_trait;
use kwnt::server::{
    continuity::TurnOrigin,
    quizmaster_tools::{register_pocket_core, PocketCore, PocketOrigin},
    AppState,
};
use quizmaster_support::{ready_friday_quiz, setup, space, start_pages, tool, turn_in};
use serde_json::{json, Value};
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc,
};
use std::time::Duration;

/// A core that takes a moment to open, as pa1 does (create, plan the schedule, save).
#[derive(Default)]
struct SlowCore {
    opens: AtomicUsize,
}

#[async_trait]
impl PocketCore for SlowCore {
    async fn open(&self, _state: &AppState, _kit: &str, _spec: &Value, _origin: &PocketOrigin) -> Result<String, String> {
        let n = self.opens.fetch_add(1, Ordering::SeqCst) + 1;
        tokio::time::sleep(Duration::from_millis(200)).await;
        Ok(format!("PR{n}"))
    }

    async fn close(&self, _state: &AppState, _code: &str) -> Result<(), String> {
        Ok(())
    }

    async fn set_when(&self, _state: &AppState, _code: &str, _when: Option<&str>) -> Result<(), String> {
        Ok(())
    }
}

#[tokio::test]
async fn two_opens_at_once_open_one_pocket() {
    let core = Arc::new(SlowCore::default());
    assert!(register_pocket_core(core.clone()));
    let (pages_url, _) = start_pages().await;
    let e = setup(true).await;
    let origin = TurnOrigin { channel_id: Some("ch-race".into()), ..Default::default() };
    let token = turn_in(&e, &e.bot, "t-race", origin).await;
    let (id, _) = ready_friday_quiz(&e, &token, &pages_url).await;

    let ((a, a_err), (b, b_err)) =
        tokio::join!(tool(&e, &token, "pocket_open", json!({"id": id})), tool(&e, &token, "pocket_open", json!({"id": id})));
    assert_eq!(core.opens.load(Ordering::SeqCst), 1, "one pocket opened: {a} / {b}");
    assert!(a_err != b_err, "one open wins: {a} / {b}");
    let refused = if a_err { &a } else { &b };
    assert!(refused.contains("already open"), "{refused}");
    let row = e.state.db.quiz_draft_get(&space(), &id).await.unwrap().unwrap();
    assert_eq!((row.state.as_str(), row.pocket_code.as_deref()), ("open", Some("PR1")));
}

#[test]
fn the_quiz_kit_is_a_pocket_kit() {
    assert!(kwnt::pocket::kits::kit("quiz").is_some(), "pa2's quiz registered with the Pocket core");
    assert!(kwnt::pocket::kits::kit_ids().contains(&"quiz"));
}
