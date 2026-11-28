//! pushes-say-what (fix #4 + m5 of Mira's 0.9.57 team-job pass): a person on a phone reads only
//! the push, so each one says what happened. A question's push carries the question and its
//! choices; a turn that only handed work on says who got it, not "replied"; a teammate result
//! that ends `Blocked:` says blocked, not "completed".
//! A fake claude CLI answers by the prompt's marker word; its own test binary: it changes PATH.

#[path = "common/exe.rs"]
mod exe;

use axum::{
    body::{to_bytes, Body},
    http::{Request, StatusCode},
    Router,
};
use kwnt::{
    broker::PermissionBroker,
    config::Config,
    db::{questions::QuestionPlace, questions::QuestionReason, spaces::SpaceId, threads::NewDelegation, Database},
    drivers::events::RuntimeEvent,
    router::LayaRouter,
    server::{continuity, create_router, questions, AppState},
    tools::ToolRunner,
};
use serde_json::{json, Value};
use std::path::PathBuf;
use std::sync::OnceLock;
use std::time::Duration;
use tower::ServiceExt;

/// `BLOCKED` ends with a Blocked line; anything else is a plain answer.
const FAKE_CLAUDE: &str = r#"#!/bin/sh
case "$1" in
  auth) echo '{"loggedIn":true}'; exit 0 ;;
esac
for a; do last="$a"; done
case "$last" in
  *BLOCKED*) printf '%s\n' '{"type":"result","is_error":false,"num_turns":1,"result":"I tried the booking site.\n\nBlocked: it needs a card I don'\''t have. Add one in Settings and tell me to go on."}' ;;
  *) printf '%s\n' '{"type":"result","is_error":false,"num_turns":1,"result":"Three hotels under 30 euros."}' ;;
esac
"#;

fn fake_cli() {
    static DONE: OnceLock<PathBuf> = OnceLock::new();
    DONE.get_or_init(|| {
        let bin = tempfile::tempdir().unwrap().keep().join("bin");
        std::fs::create_dir_all(&bin).unwrap();
        exe::write_executable(bin.join("claude"), FAKE_CLAUDE);
        std::env::set_var("PATH", format!("{}:{}", bin.display(), std::env::var("PATH").unwrap_or_default()));
        bin
    });
}

async fn call(app: &Router, method: &str, uri: &str, body: Option<Value>) -> (StatusCode, Value) {
    let req = Request::builder().method(method).uri(uri).header("content-type", "application/json");
    let body = body.map(|v| Body::from(v.to_string())).unwrap_or_else(Body::empty);
    let resp = app.clone().oneshot(req.body(body).unwrap()).await.unwrap();
    let s = resp.status();
    let bytes = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    (s, serde_json::from_slice(&bytes).unwrap_or_else(|_| Value::String(String::from_utf8_lossy(&bytes).into())))
}

async fn ok(app: &Router, method: &str, uri: &str, body: Option<Value>) -> Value {
    let (s, v) = call(app, method, uri, body).await;
    assert!(s.is_success(), "{method} {uri}: {s} {v}");
    v
}

#[derive(Debug, Clone, PartialEq)]
struct Note {
    kind: String,
    title: String,
    body: String,
}

fn note(kind: &str, title: &str, body: &str) -> Note {
    Note { kind: kind.into(), title: title.into(), body: body.into() }
}

struct Env {
    app: Router,
    state: AppState,
    rx: tokio::sync::broadcast::Receiver<RuntimeEvent>,
    notifier: tokio::task::JoinHandle<()>,
    _tmp: tempfile::TempDir,
}

impl Drop for Env {
    fn drop(&mut self) {
        self.notifier.abort();
    }
}

/// Lead, Researcher and Writer on the fake claude; the notifier runs (as `serve` starts it).
async fn setup() -> Env {
    fake_cli();
    let tmp = tempfile::tempdir().unwrap();
    let db = Database::init(tmp.path()).await.unwrap();
    db.set_setting("keep_going_turns", "0").await.unwrap();
    let config = Config {
        data_dir: tmp.path().to_path_buf(),
        laya_url: "http://127.0.0.1:9".into(),
        browser_cdp_url: Some("http://127.0.0.1:9".into()),
        ..Config::default()
    };
    let state = AppState::new(
        config.clone(),
        db,
        ToolRunner::new(tmp.path().join("workspace")),
        PermissionBroker::new(),
        LayaRouter::new(config.laya_url.clone()),
    );
    let app = create_router(state.clone());
    // The seeded @helpers go (Assistant last, as a space keeps one bot)
    let seeded = state.db.list_kwants(&SpaceId::default_space()).await.unwrap();
    for k in seeded.iter().filter(|k| k.name != "Assistant") {
        ok(&app, "DELETE", &format!("/api/kwants/{}", k.id), None).await;
    }
    for name in ["Lead", "Researcher", "Writer"] {
        let bot = json!({"name": name, "title": format!("{name} the tester"), "persona_prompt": "Test persona",
            "avatar_sprite": "s", "driver_type": "cli", "preferred_model": "claude"});
        ok(&app, "POST", "/api/kwants", Some(bot)).await;
    }
    for k in seeded.iter().filter(|k| k.name == "Assistant") {
        ok(&app, "DELETE", &format!("/api/kwants/{}", k.id), None).await;
    }
    let rx = state.event_sender.subscribe();
    let notifier = continuity::spawn_notifier(state.clone());
    Env { app, state, rx, notifier, _tmp: tmp }
}

impl Env {
    async fn kwant_id(&self, name: &str) -> String {
        let kwants = ok(&self.app, "GET", "/api/kwants", None).await;
        kwants.as_array().unwrap().iter().find(|k| k["name"] == name).unwrap()["id"].as_str().unwrap().to_string()
    }

    /// Notifications until one of `kind` arrives (60 s at most), oldest first.
    async fn notes_until(&mut self, kind: &str) -> Vec<Note> {
        let mut notes = Vec::new();
        let deadline = tokio::time::Instant::now() + Duration::from_secs(60);
        while tokio::time::Instant::now() < deadline {
            let Ok(Ok(event)) = tokio::time::timeout(Duration::from_millis(200), self.rx.recv()).await else { continue };
            if let RuntimeEvent::Notify { kind: k, title, body, .. } = event.unscoped().clone() {
                let done = k == kind;
                notes.push(Note { kind: k, title, body });
                if done {
                    return notes;
                }
            }
        }
        panic!("no {kind} notification; got {notes:?}");
    }

    /// A hand-off from Lead's direct chat to `teammate`, started by Lead's turn `turn_id`.
    async fn handed(&self, teammate: &str, title: &str, turn_id: &str) -> String {
        let lead = self.kwant_id("Lead").await;
        let (_, d) = self
            .state
            .db
            .create_delegation(&NewDelegation {
                lead_kwant_id: lead.clone(),
                specialist_kwant_id: self.kwant_id(teammate).await,
                origin_channel_id: None,
                origin_kwant_id: Some(lead),
                origin_thread_id: None,
                team_id: None,
                task: format!("{title}, please"),
                title: title.into(),
                continue_lead: true,
                depth: 1,
                follow_up_at: None,
            })
            .await
            .unwrap();
        self.state.db.set_delegation_lead_turn(&d.id, turn_id).await.unwrap();
        d.id
    }

    /// Lead's turn `turn_id` in its direct chat ends completed with `reply`; its notification.
    async fn lead_turn_ends(&mut self, turn_id: &str, reply: &str) -> Note {
        let lead = self.kwant_id("Lead").await;
        let _ = self.state.event_sender.send(RuntimeEvent::TurnEnded {
            kwant_id: lead.clone(),
            context_id: Some(lead),
            outcome: "completed".into(),
            turn_id: Some(turn_id.into()),
            thread_id: None,
            quiet: false,
            reply: Some(reply.into()),
        });
        self.notes_until("turn_finished").await.pop().unwrap()
    }
}

/// "Assistant replied" 1 s after the ask read as "done already?": the turn only handed the job on.
#[tokio::test]
async fn a_turn_that_only_handed_on_says_who_got_the_task() {
    let mut env = setup().await;
    env.handed("Researcher", "Find hotels in Ljubljana", "t-one").await;
    assert_eq!(
        env.lead_turn_ends("t-one", "I've asked Researcher to look into it.").await,
        note("turn_finished", "Lead handed “Find hotels in Ljubljana” to Researcher", "Results come back to your direct chat.")
    );
}

#[tokio::test]
async fn a_hand_off_to_two_teammates_names_both() {
    let mut env = setup().await;
    // One task to two teammates
    env.handed("Researcher", "Plan the trip", "t-two").await;
    env.handed("Writer", "Plan the trip", "t-two").await;
    assert_eq!(env.lead_turn_ends("t-two", "Researcher and Writer are on it.").await.title, "Lead handed “Plan the trip” to Researcher and Writer");
    // Two tasks
    env.handed("Researcher", "Find hotels", "t-three").await;
    env.handed("Writer", "Write the itinerary", "t-three").await;
    assert_eq!(env.lead_turn_ends("t-three", "Both are on it.").await.title, "Lead handed 2 tasks to Researcher and Writer");
    // A long title is clipped
    let long = "Find three hotels near the old town under thirty euros a night with breakfast and free cancellation";
    env.handed("Researcher", long, "t-long").await;
    let title = env.lead_turn_ends("t-long", "On it.").await.title;
    assert!(title.starts_with("Lead handed “Find three hotels") && title.contains("…” to Researcher"), "{title}");
}

/// A turn with its own `Done:` line, or one that handed nothing on, still says "replied".
#[tokio::test]
async fn a_turn_that_did_the_work_still_says_replied() {
    let mut env = setup().await;
    env.handed("Researcher", "Find hotels", "t-done").await;
    assert_eq!(
        env.lead_turn_ends("t-done", "Here is the plan.\n\nDone: 3-day plan saved.").await,
        note("turn_finished", "Lead replied", "Done: 3-day plan saved.")
    );
    assert_eq!(
        env.lead_turn_ends("t-none", "Hello there.").await,
        note("turn_finished", "Lead replied", "Lead finished replying in your direct chat.")
    );
    // A cancelled hand-off doesn't count
    let id = env.handed("Writer", "Write it up", "t-cancelled").await;
    env.state.db.finish_delegation(&id, "cancelled", None, None).await.unwrap();
    assert_eq!(env.lead_turn_ends("t-cancelled", "Never mind.").await.title, "Lead replied");
}

/// m5: a teammate whose result ends Blocked is blocked, not "completed".
#[tokio::test]
async fn a_blocked_teammate_result_says_blocked() {
    let mut env = setup().await;
    let (lead, researcher) = (env.kwant_id("Lead").await, env.kwant_id("Researcher").await);
    ok(
        &env.app,
        "POST",
        "/api/delegations",
        Some(json!({"lead_kwant_id": lead, "specialist_kwant_id": researcher, "kwant_id": lead,
            "task": "BLOCKED book the hotel", "title": "Book the hotel", "continue_lead": false})),
    )
    .await;
    assert_eq!(
        env.notes_until("delegation_finished").await.pop().unwrap(),
        note(
            "delegation_finished",
            "Researcher is blocked on a task for Lead",
            "Blocked: it needs a card I don't have. Add one in Settings and tell me to go on."
        )
    );
    // A plain result keeps today's words
    ok(
        &env.app,
        "POST",
        "/api/delegations",
        Some(json!({"lead_kwant_id": lead, "specialist_kwant_id": researcher, "kwant_id": lead,
            "task": "find hotels", "title": "Find hotels", "continue_lead": false})),
    )
    .await;
    assert_eq!(
        env.notes_until("delegation_finished").await.pop().unwrap(),
        note("delegation_finished", "Researcher finished a task for Lead", "“Find hotels”: completed.")
    );
}

/// "Researcher has a question · Open the chat to answer." made the person unlock and open kwant
/// to see a one-tap €20 / €30 / No limit.
#[tokio::test]
async fn a_question_push_carries_the_question() {
    let mut env = setup().await;
    let kid = env.kwant_id("Researcher").await;
    let place = QuestionPlace { kwant_id: kid.clone(), direct_kwant_id: Some(kid.clone()), ..QuestionPlace::default() };
    let choices: Vec<String> = ["€20", "€30", "No limit"].iter().map(|s| s.to_string()).collect();
    let asked = questions::ask_without_waiter(&env.state, &place, "Researcher", "How much a night for the hotel?", &choices, QuestionReason::default()).await;
    assert!(asked.is_some());
    assert_eq!(
        env.notes_until("question_asked").await.pop().unwrap(),
        note("question_asked", "Researcher has a question", "How much a night for the hotel? · €20 / €30 / No limit")
    );
    // Without choices: the question alone
    let wid = env.kwant_id("Writer").await;
    let place = QuestionPlace { kwant_id: wid.clone(), direct_kwant_id: Some(wid), ..QuestionPlace::default() };
    questions::ask_without_waiter(&env.state, &place, "Writer", "Which city should I start with?", &[], QuestionReason::default()).await;
    assert_eq!(env.notes_until("question_asked").await.pop().unwrap().body, "Which city should I start with?");
}
