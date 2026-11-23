//! pa3b: the Quizmaster on the real Pocket core. `pocket_open` makes a real quiz pocket (pa1, with
//! pa7's origin and schedule), `pocket_update`'s `when` moves it, `pocket_close` ends it; a guest's
//! near miss is judged on arrival, off the request path, once; a reply that isn't a verdict isn't
//! asked again (qa F1); the calls used survive a restart (qa F3); with the switch off nothing runs.
//! Its own test binary: the core and the stub judge model are registered once per process.

mod quizmaster_support;

use async_trait::async_trait;
use kwnt::db::models::NewChannel;
use kwnt::db::pocket::{ENDED, OPEN};
use kwnt::db::spaces::SpaceId;
use kwnt::pocket::{self, kits::Viewer, JoinRequest};
use kwnt::quizmaster::budget::Budget;
use kwnt::quizmaster::judge::{JudgeModel, Outcome};
use kwnt::server::{
    continuity::TurnOrigin,
    quizmaster_live::{self as live, CoreAdapter},
    quizmaster_tools::{self, register_pocket_core},
};
use quizmaster_support::{card, ready_friday_quiz, setup, space, start_pages, tool, turn_in, Env};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

/// The judge model every test shares: its verdict and its call count are per guess, so each test
/// uses guesses of its own.
#[derive(Default)]
struct StubJudge {
    calls: Mutex<HashMap<String, usize>>,
}

/// The guess in the judge prompt's data block.
fn guess_in(prompt: &str) -> String {
    let start = prompt.find("<guest_data>").expect("data block") + "<guest_data>".len();
    let end = prompt.find("</guest_data>").expect("data block end");
    let data: Value = serde_json::from_str(prompt[start..end].trim()).expect("the data block is JSON");
    data["guess"].as_str().unwrap().to_string()
}

#[async_trait]
impl JudgeModel for StubJudge {
    async fn reply(&self, prompt: String) -> Result<String, String> {
        let guess = guess_in(&prompt);
        *self.calls.lock().unwrap().entry(guess.clone()).or_default() += 1;
        Ok(match guess.as_str() {
            "Pariss" | "Pariis" => "yes",
            "Parris" => "maybe so", // not one of the three words: NoVerdict
            _ => "no",
        }
        .to_string())
    }
}

impl StubJudge {
    fn calls(&self, guess: &str) -> usize {
        self.calls.lock().unwrap().get(guess).copied().unwrap_or(0)
    }
}

fn judge_model() -> &'static Arc<StubJudge> {
    static MODEL: OnceLock<Arc<StubJudge>> = OnceLock::new();
    MODEL.get_or_init(|| {
        assert!(register_pocket_core(Arc::new(CoreAdapter)), "the real core registers");
        let m = Arc::new(StubJudge::default());
        live::use_judge_model_for_tests(m.clone());
        m
    })
}

/// A real chat (pa7 refuses an origin whose channel doesn't exist).
async fn chat(e: &Env, name: &str) -> TurnOrigin {
    let input = NewChannel {
        name: name.into(),
        description: None,
        lead_kwant_id: e.bot.id.clone(),
        member_kwants: vec![e.bot.id.clone()],
        routing_mode: "auto".into(),
    };
    let c = e.state.db.create_channel(&SpaceId::default_space(), &input).await.unwrap();
    TurnOrigin { channel_id: Some(c.id), thread_id: None, ..Default::default() }
}

/// Polls `f` for up to 5 s.
async fn until<F, Fut>(what: &str, mut f: F)
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = bool>,
{
    for _ in 0..100 {
        if f().await {
            return;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    panic!("timed out waiting for: {what}");
}

/// A Ready Friday quiz, opened in `channel`; returns (env, token, draft id, pocket code).
async fn opened(channel: &str) -> (Env, String, String, String) {
    judge_model();
    let (pages_url, _) = start_pages().await;
    let e = setup(true).await;
    let origin = chat(&e, channel).await;
    let token = turn_in(&e, &e.bot, &format!("t-{channel}"), origin).await;
    let (id, _) = ready_friday_quiz(&e, &token, &pages_url).await;
    let (text, err) = tool(&e, &token, "pocket_open", json!({"id": id})).await;
    assert!(!err, "{text}");
    let code = card(&text)["pocket"].as_str().expect("the card shows the pocket's code").to_string();
    (e, token, id, code)
}

/// Clears the quiz's time (Friday's lobby opens 30 minutes before it, so guests couldn't join now).
async fn open_now(e: &Env, token: &str, id: &str) {
    let (text, err) = tool(e, token, "pocket_update", json!({"id": id, "when": null})).await;
    assert!(!err, "{text}");
}

async fn join(e: &Env, code: &str, name: &str) -> String {
    let req = JoinRequest { name: name.into(), avatar: None, team: Some("Quizzy Rascals".into()), pin: None };
    pocket::join(&e.state, code, req).await.unwrap_or_else(|err| panic!("{name} joins: {}", err.message)).0.id
}

async fn kit_state(e: &Env, code: &str) -> Value {
    pocket::snapshot(&e.state, code).await.unwrap().unwrap().kit_state.clone()
}

/// The host presses Next until question `q` is open.
async fn host_to_question(e: &Env, code: &str, q: u64) {
    pocket::act(&e.state, code, &Viewer::Host, "start", &json!({})).await.unwrap();
    for _ in 0..20 {
        let s = kit_state(e, code).await;
        if s["current"] == json!(q) && s["phase"] == json!("question") {
            return;
        }
        pocket::act(&e.state, code, &Viewer::Host, "next", &json!({})).await.unwrap();
    }
    panic!("question {q} never opened: {}", kit_state(e, code).await);
}

/// Q4 ("Paris", 1 point) is the example's fourth question (index 3).
const PARIS: u64 = 3;

async fn answer(e: &Env, code: &str, player: &str, guess: &str) {
    pocket::act(&e.state, code, &Viewer::Guest(player.into()), "answer", &json!({"q": PARIS, "value": guess})).await.unwrap();
}

async fn pending(e: &Env, code: &str) -> usize {
    let s = pocket::snapshot(&e.state, code).await.unwrap().unwrap();
    kwnt::pocket::quiz_kit::QuizPocket::pending(&s.spec, &s.kit_state).as_array().unwrap().len()
}

async fn judge_calls(e: &Env, id: &str) -> u32 {
    let row = e.state.db.quiz_draft_get(&space(), id).await.unwrap().unwrap();
    serde_json::from_str::<Budget>(&row.budget).unwrap().judge_calls
}

#[tokio::test]
async fn a_ready_quiz_opens_a_real_pocket_moves_and_closes() {
    let (e, token, id, code) = opened("ch-open").await;
    let s = pocket::snapshot(&e.state, &code).await.unwrap().expect("the pocket exists");
    assert_eq!((s.row.kit.as_str(), s.row.state.as_str(), s.row.title.as_str()), ("quiz", OPEN, "Friday Pub Quiz"));
    assert_eq!(s.row.owner_kwant_id.as_deref(), Some(e.bot.id.as_str()));
    let sched = e.state.db.pocket_schedule(&code).await.unwrap().expect("pa7 planned it");
    let origin: Value = serde_json::from_str(sched.origin.as_deref().expect("pa7 keeps the chat")).unwrap();
    assert_eq!(origin["kwant_id"], json!(e.bot.id), "{origin}");
    assert!(origin["channel_id"].as_str().is_some_and(|c| !c.is_empty()), "the turn's chat is the origin: {origin}");
    assert_eq!(sched.starts_at.map(|t| t.to_rfc3339()), Some("2026-11-27T20:00:00+00:00".into()));

    // `when` moves the pocket itself (pa7 set_when), not just the card
    let (text, err) = tool(&e, &token, "pocket_update", json!({"id": id, "when": "2026-11-27T21:30:00+01:00"})).await;
    assert!(!err, "{text}");
    let sched = e.state.db.pocket_schedule(&code).await.unwrap().unwrap();
    assert_eq!(sched.starts_at.map(|t| t.to_rfc3339()), Some("2026-11-27T20:30:00+00:00".into()));
    assert!(pocket::snapshot(&e.state, &code).await.unwrap().unwrap().spec["when"].as_str().unwrap().starts_with("2026-11-27T21:30:00"));
    // A time that isn't RFC 3339 is refused in words, and nothing moves
    let (why, err) = tool(&e, &token, "pocket_update", json!({"id": id, "when": "Friday 8pm"})).await;
    assert!(err, "{why}");
    assert_eq!(e.state.db.pocket_schedule(&code).await.unwrap().unwrap().starts_at.map(|t| t.to_rfc3339()), Some("2026-11-27T20:30:00+00:00".into()));

    // Close ends the pocket
    let (text, err) = tool(&e, &token, "pocket_close", json!({"id": id})).await;
    assert!(!err, "{text}");
    assert_eq!(card(&text)["state"], json!("closed"));
    assert_eq!(pocket::snapshot(&e.state, &code).await.unwrap().unwrap().row.state, ENDED);
    // pa7 refuses to move an ended pocket; the tool says the quiz is closed first
    let (why, err) = tool(&e, &token, "pocket_update", json!({"id": id, "when": "2026-11-28T20:00:00+00:00"})).await;
    assert!(err && why.contains("closed"), "{why}");
}

#[tokio::test]
async fn two_opens_at_once_make_one_pocket() {
    judge_model();
    let (pages_url, _) = start_pages().await;
    let e = setup(true).await;
    let origin = chat(&e, "ch-twice").await;
    let token = turn_in(&e, &e.bot, "t-twice", origin).await;
    let (id, _) = ready_friday_quiz(&e, &token, &pages_url).await;
    let ((a, a_err), (b, b_err)) =
        tokio::join!(tool(&e, &token, "pocket_open", json!({"id": id})), tool(&e, &token, "pocket_open", json!({"id": id})));
    assert!(a_err != b_err, "exactly one open wins: {a} / {b}");
    let refused = if a_err { &a } else { &b };
    assert!(refused.contains("already open"), "{refused}");
    assert_eq!(e.state.db.pocket_list().await.unwrap().len(), 1, "one pocket");
}

#[tokio::test]
async fn a_failed_save_after_open_closes_the_pocket_again() {
    judge_model();
    let (pages_url, _) = start_pages().await;
    let e = setup(true).await;
    let origin = chat(&e, "ch-save").await;
    let token = turn_in(&e, &e.bot, "t-save", origin).await;
    let (id, _) = ready_friday_quiz(&e, &token, &pages_url).await;
    quizmaster_tools::fail_next_open_save_for_tests(&id);
    let (why, err) = tool(&e, &token, "pocket_open", json!({"id": id})).await;
    assert!(err && why.contains("closed again"), "{why}");
    let pockets = e.state.db.pocket_list().await.unwrap();
    assert_eq!(pockets.len(), 1);
    assert_eq!(pockets[0].state, ENDED, "no open pocket the card doesn't know about");
    let row = e.state.db.quiz_draft_get(&space(), &id).await.unwrap().unwrap();
    assert_eq!((row.state.as_str(), row.pocket_code), ("ready", None));
    // It opens on the next try
    let (text, err) = tool(&e, &token, "pocket_open", json!({"id": id})).await;
    assert!(!err, "{text}");
}

#[tokio::test]
async fn a_near_miss_is_judged_once_on_arrival_and_the_score_moves() {
    let model = judge_model();
    let (e, token, id, code) = opened("ch-judge").await;
    open_now(&e, &token, &id).await;
    let ann = join(&e, &code, "Ann").await;
    let _bob = join(&e, &code, "Bob").await;
    host_to_question(&e, &code, PARIS).await;

    // The guest's request doesn't wait for the judge; the judge settles it soon after
    answer(&e, &code, &ann, "Pariss").await;
    until("the near miss is judged", || async { pending(&e, &code).await == 0 }).await;
    assert_eq!(model.calls("Pariss"), 1);
    // Nothing waiting: no call
    assert!(live::judge_now(&e.state, &code).await.is_empty());
    assert_eq!(model.calls("Pariss"), 1, "judged once");
    until("the call count is saved", || async { judge_calls(&e, &id).await == 1 }).await;

    // At the reveal the point counts
    let s = pocket::snapshot(&e.state, &code).await.unwrap().unwrap();
    assert_eq!(live_score(&s, &ann), 0, "not revealed yet");
    pocket::act(&e.state, &code, &Viewer::Host, "next", &json!({})).await.unwrap(); // lock
    pocket::act(&e.state, &code, &Viewer::Host, "next", &json!({})).await.unwrap(); // reveal
    let s = pocket::snapshot(&e.state, &code).await.unwrap().unwrap();
    assert_eq!(s.kit_state["phase"], json!("reveal"));
    let before_q4: i64 = 0; // Ann answered nothing before Q4
    assert_eq!(live_score(&s, &ann), before_q4 + 1, "Ann's judged Paris scores its point");
}

/// Ann's score from the saved game.
fn live_score(s: &pocket::Snapshot, player: &str) -> i64 {
    let spec = kwnt::kits::quiz::validate::parse(&s.spec.to_string()).unwrap();
    let g = kwnt::kits::quiz::engine::QuizGame::restore(spec, &s.kit_state).unwrap();
    g.score(player)
}

#[tokio::test]
async fn an_answer_without_a_verdict_is_not_judged_again() {
    let model = judge_model();
    let (e, token, id, code) = opened("ch-f1").await;
    open_now(&e, &token, &id).await;
    let ann = join(&e, &code, "Ann").await;
    let bob = join(&e, &code, "Bob").await;
    let cy = join(&e, &code, "Cy").await;
    host_to_question(&e, &code, PARIS).await;

    answer(&e, &code, &ann, "Parris").await;
    until("the first judging round", || async { model.calls("Parris") == 1 }).await;
    until("its count is saved", || async { judge_calls(&e, &id).await == 1 }).await;
    assert_eq!(pending(&e, &code).await, 1, "no verdict: the host decides");

    // The next guests' answers (one right, one wrong) don't send Ann's near miss again
    answer(&e, &code, &bob, "Paris").await;
    answer(&e, &code, &cy, "Rome").await;
    tokio::time::sleep(Duration::from_millis(300)).await;
    let again = live::judge_now(&e.state, &code).await;
    assert!(again.is_empty(), "{again:?}");
    assert_eq!(model.calls("Parris"), 1, "asked once");
    assert_eq!(pending(&e, &code).await, 1);
    assert_eq!(judge_calls(&e, &id).await, 1);
}

#[tokio::test]
async fn the_judge_count_survives_a_restart() {
    let model = judge_model();
    let (e, token, id, code) = opened("ch-f3").await;
    open_now(&e, &token, &id).await;
    // A small allowance, so the restored count shows: 2 calls
    let mut row = e.state.db.quiz_draft_get(&space(), &id).await.unwrap().unwrap();
    let mut budget: Budget = serde_json::from_str(&row.budget).unwrap();
    budget.judge_allowance = 2;
    row.budget = serde_json::to_string(&budget).unwrap();
    e.state.db.quiz_draft_save(&row).await.unwrap();
    let ann = join(&e, &code, "Ann").await;
    let bob = join(&e, &code, "Bob").await;
    let cy = join(&e, &code, "Cy").await;
    let _dee = join(&e, &code, "Dee").await;
    host_to_question(&e, &code, PARIS).await;

    answer(&e, &code, &ann, "Paaris").await;
    until("Ann is judged", || async { pending(&e, &code).await == 0 && judge_calls(&e, &id).await == 1 }).await;

    // Restart: the next judge starts from the saved count (1 of 2 used), so it saves 2, not 1
    live::forget_judge_for_tests(&code);
    answer(&e, &code, &bob, "Pariis").await;
    until("Bob is judged", || async { pending(&e, &code).await == 0 && judge_calls(&e, &id).await == 2 }).await;

    // Restart again: the allowance is used up, so Cy's near miss waits for the host, no call
    live::forget_judge_for_tests(&code);
    answer(&e, &code, &cy, "Pparis").await;
    tokio::time::sleep(Duration::from_millis(300)).await;
    let out = live::judge_now(&e.state, &code).await;
    assert!(out.iter().all(|o| matches!(o, Outcome::BudgetOut { .. })), "{out:?}");
    assert_eq!(model.calls("Pparis"), 0);
    assert_eq!(pending(&e, &code).await, 1);
    assert_eq!(judge_calls(&e, &id).await, 2);
}

#[tokio::test]
async fn with_the_switch_off_nothing_opens_or_judges() {
    let model = judge_model();
    // Off from the start: the tools refuse and no pocket is made
    let (pages_url, _) = start_pages().await;
    let off = setup(true).await;
    let origin = chat(&off, "ch-off").await;
    let token = turn_in(&off, &off.bot, "t-off", origin).await;
    let (id, _) = ready_friday_quiz(&off, &token, &pages_url).await;
    off.state.db.set_setting(quizmaster_tools::SETTING, "off").await.unwrap();
    let (why, err) = tool(&off, &token, "pocket_open", json!({"id": id})).await;
    assert!(err, "{why}");
    assert!(off.state.db.pocket_list().await.unwrap().is_empty());

    // Turned off during a quiz: a near miss isn't sent to the model
    let (e, token, id, code) = opened("ch-off-live").await;
    open_now(&e, &token, &id).await;
    let ann = join(&e, &code, "Ann").await;
    let _bob = join(&e, &code, "Bob").await;
    host_to_question(&e, &code, PARIS).await;
    e.state.db.set_setting(quizmaster_tools::SETTING, "off").await.unwrap();
    let _ = pocket::act(&e.state, &code, &Viewer::Guest(ann.clone()), "answer", &json!({"q": PARIS, "value": "Parisx"})).await;
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert!(live::judge_now(&e.state, &code).await.is_empty());
    assert_eq!(model.calls("Parisx"), 0);
    assert_eq!(judge_calls(&e, &id).await, 0);
}
