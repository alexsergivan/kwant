//! pa3 with a Pocket core registered (a fake standing in for pa1/pa7): a Ready quiz opens with the
//! origin of the chat the bot was asked in (never one from the tool arguments), moves through
//! `set_when` while open, and closes. A core that refuses (pa7's `bad_origin`, `ended`) is
//! reported as it is: no silent retry, and the quiz keeps its state.
//! Its own test binary: the core is registered once per process.

mod quizmaster_support;

use async_trait::async_trait;
use kwnt::server::{
    continuity::TurnOrigin,
    quizmaster_tools::{register_pocket_core, PocketCore, PocketOrigin},
    AppState,
};
use quizmaster_support::{card, ready_friday_quiz, setup, space, start_pages, tool, turn_in};
use serde_json::{json, Value};
use std::sync::{Arc, Mutex, OnceLock};

#[derive(Default)]
struct FakeCore {
    opens: Mutex<Vec<(String, String, PocketOrigin)>>,
    moves: Mutex<Vec<(String, Option<String>)>>,
    closes: Mutex<Vec<String>>,
}

#[async_trait]
impl PocketCore for FakeCore {
    async fn open(&self, _state: &AppState, kit: &str, spec: &Value, origin: &PocketOrigin) -> Result<String, String> {
        let mut opens = self.opens.lock().unwrap();
        opens.push((kit.to_string(), spec["title"].as_str().unwrap_or("").to_string(), origin.clone()));
        if origin.channel_id.as_deref() == Some("gone") {
            return Err("bad_origin: that chat doesn't exist any more".into());
        }
        Ok(format!("PQ{}", opens.len()))
    }

    async fn close(&self, _state: &AppState, code: &str) -> Result<(), String> {
        self.closes.lock().unwrap().push(code.to_string());
        Ok(())
    }

    async fn set_when(&self, _state: &AppState, code: &str, when: Option<&str>) -> Result<(), String> {
        self.moves.lock().unwrap().push((code.to_string(), when.map(str::to_string)));
        if when.is_none() {
            return Err("ended: the pocket isn't open any more".into());
        }
        Ok(())
    }
}

fn core() -> &'static Arc<FakeCore> {
    static CORE: OnceLock<Arc<FakeCore>> = OnceLock::new();
    CORE.get_or_init(|| {
        let c = Arc::new(FakeCore::default());
        assert!(register_pocket_core(c.clone()), "the first core registers");
        c
    })
}

fn chat(channel: &str, thread: Option<&str>) -> TurnOrigin {
    TurnOrigin { channel_id: Some(channel.into()), thread_id: thread.map(str::to_string), ..Default::default() }
}

/// One test, so the shared fake's call lists are read in order.
#[tokio::test]
async fn a_ready_quiz_opens_in_its_chat_moves_and_closes() {
    let core = core();
    assert!(!register_pocket_core(Arc::new(FakeCore::default())), "a second core is refused");
    let (pages_url, _) = start_pages().await;
    let e = setup(true).await;
    let token = turn_in(&e, &e.bot, "t-open", chat("ch-friday", Some("th-7"))).await;
    let (id, spec) = ready_friday_quiz(&e, &token, &pages_url).await;

    // Open: the origin is the turn's chat; an `origin` in the arguments is ignored
    let forged = json!({"id": id, "origin": {"kwant_id": e.other.id, "channel_id": "ch-elsewhere"}});
    let (text, err) = tool(&e, &token, "pocket_open", forged).await;
    assert!(!err, "{text}");
    let c = card(&text);
    assert_eq!((c["state"].clone(), c["pocket"].clone()), (json!("open"), json!("PQ1")), "{text}");
    {
        let opens = core.opens.lock().unwrap();
        assert_eq!(opens.len(), 1);
        let (kit, title, origin) = &opens[0];
        assert_eq!((kit.as_str(), title.as_str()), ("quiz", "Friday Pub Quiz"));
        assert_eq!(origin, &PocketOrigin { kwant_id: e.bot.id.clone(), channel_id: Some("ch-friday".into()), thread_id: Some("th-7".into()) });
    }
    let (why, err) = tool(&e, &token, "pocket_open", json!({"id": id})).await;
    assert!(err && why.contains("already open"), "{why}");

    // Open: the questions are fixed, the time moves through the core
    let (why, err) = tool(&e, &token, "pocket_update", json!({"id": id, "spec": spec})).await;
    assert!(err && why.contains("can't change"), "{why}");
    let (text, err) = tool(&e, &token, "pocket_update", json!({"id": id, "when": "2026-11-27T21:00:00+00:00"})).await;
    assert!(!err, "{text}");
    assert_eq!(card(&text)["when"], json!("2026-11-27T21:00:00+00:00"));
    assert_eq!(core.moves.lock().unwrap().last().unwrap(), &("PQ1".to_string(), Some("2026-11-27T21:00:00+00:00".to_string())));

    // The core refuses (ended): said as it is, and the stored time stays
    let (why, err) = tool(&e, &token, "pocket_update", json!({"id": id, "when": null})).await;
    assert!(err && why.contains("ended"), "{why}");
    let row = e.state.db.quiz_draft_get(&space(), &id).await.unwrap().unwrap();
    assert!(row.spec.contains("2026-11-27T21:00:00+00:00"));

    // Close
    let (text, err) = tool(&e, &token, "pocket_close", json!({"id": id})).await;
    assert!(!err, "{text}");
    assert_eq!(card(&text)["state"], json!("closed"));
    assert_eq!(core.closes.lock().unwrap().as_slice(), ["PQ1".to_string()]);
    for (name, args, says) in [
        ("pocket_close", json!({"id": id}), "isn't open"),
        ("pocket_open", json!({"id": id}), "Only a Ready quiz"),
        ("pocket_update", json!({"id": id, "when": "2026-11-28T21:00:00+00:00"}), "closed"),
    ] {
        let (why, err) = tool(&e, &token, name, args).await;
        assert!(err && why.contains(says), "{name}: {why}");
    }

    // A chat the core doesn't accept: one try, the error as it is, still Ready
    let gone = turn_in(&e, &e.bot, "t-gone", chat("gone", None)).await;
    let (id2, _) = ready_friday_quiz(&e, &gone, &pages_url).await;
    let before = core.opens.lock().unwrap().len();
    let (why, err) = tool(&e, &gone, "pocket_open", json!({"id": id2})).await;
    assert!(err && why.contains("bad_origin"), "{why}");
    assert_eq!(core.opens.lock().unwrap().len(), before + 1, "no retry without the origin");
    let row = e.state.db.quiz_draft_get(&space(), &id2).await.unwrap().unwrap();
    assert_eq!((row.state.as_str(), row.pocket_code), ("ready", None));
}
